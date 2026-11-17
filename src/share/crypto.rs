//! The two layers of a shared record, and the key derivations the recipient's page repeats.
//!
//! - **The link key** (`K`, 32 random bytes) lives only in the link's fragment, which browsers never
//!   send. Whoever reads the bucket without the link holds ciphertext.
//! - **The envelope** opens with `K` alone: title, sender, expiry and the signed URLs the page
//!   needs. Nothing about the record is in clear in S3.
//! - **The content** (the fields) opens with `K` and, when the share has an opening password, with
//!   PBKDF2-HMAC-SHA256 of it too: whoever holds the link but not the password cannot open it, and
//!   whoever reads the bucket cannot even start guessing, for want of `K`.
//!
//! Everything here is what the page does with WebCrypto, step for step: HKDF-SHA256 with an empty
//! salt, PBKDF2-HMAC-SHA256, AES-256-GCM with a 12-byte nonce in front of the ciphertext and the
//! share id and expiry as additional data. The test at the bottom pins the derivations.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, Result};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use zeroize::Zeroizing;

type HmacSha256 = Hmac<Sha256>;

/// Iterations of PBKDF2 for the opening password: OWASP's 2023 figure for HMAC-SHA256.
pub const PASSWORD_ITERATIONS: u32 = 600_000;

pub const ENVELOPE_INFO: &[u8] = b"keyhold-share-envelope-v1";
pub const CONTENT_INFO: &[u8] = b"keyhold-share-content-v1";

/// HKDF-SHA256 (RFC 5869) with an empty salt, 32 bytes out: what `crypto.subtle.deriveKey`
/// produces from `{ name: 'HKDF', hash: 'SHA-256', salt: new Uint8Array(), info }`.
pub fn hkdf32(ikm: &[u8], info: &[u8]) -> Zeroizing<[u8; 32]> {
    // Extract with a zero-length salt, which RFC 5869 says is a string of zeros of hash length.
    let mut extract = <HmacSha256 as Mac>::new_from_slice(&[0u8; 32]).expect("any key length");
    extract.update(ikm);
    let mut prk = Zeroizing::new([0u8; 32]);
    prk.copy_from_slice(&extract.finalize().into_bytes());

    let mut expand = <HmacSha256 as Mac>::new_from_slice(prk.as_ref()).expect("any key length");
    expand.update(info);
    expand.update(&[1u8]);

    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&expand.finalize().into_bytes());
    out
}

/// PBKDF2-HMAC-SHA256, 32 bytes out.
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> Zeroizing<[u8; 32]> {
    let prf = <HmacSha256 as Mac>::new_from_slice(password).expect("any key length");

    let mut first = prf.clone();
    first.update(salt);
    first.update(&1u32.to_be_bytes());
    let mut u = first.finalize().into_bytes();

    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&u);

    for _ in 1..iterations {
        let mut next = prf.clone();
        next.update(&u);
        u = next.finalize().into_bytes();
        for (o, b) in out.iter_mut().zip(u.iter()) {
            *o ^= b;
        }
    }

    out
}

/// The content key: from `K`, and from the password when there is one.
pub fn content_key(link_key: &[u8; 32], password: Option<(&str, &[u8], u32)>) -> Zeroizing<[u8; 32]> {
    let mut ikm = Zeroizing::new(link_key.to_vec());
    if let Some((password, salt, iterations)) = password {
        ikm.extend_from_slice(pbkdf2_sha256(password.as_bytes(), salt, iterations).as_ref());
    }

    hkdf32(&ikm, CONTENT_INFO)
}

pub fn envelope_key(link_key: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    hkdf32(link_key, ENVELOPE_INFO)
}

/// The additional data both layers are bound to: the share and when it ends.
pub fn aad(share_id: &str, expires_at: u64) -> Vec<u8> {
    format!("{share_id}|{expires_at}").into_bytes()
}

/// AES-256-GCM, nonce first.
pub fn seal(key: &[u8; 32], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| anyhow!("bad key length"))?;
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);

    let sealed = cipher
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad })
        .map_err(|_| anyhow!("encryption failed"))?;

    let mut out = nonce.to_vec();
    out.extend_from_slice(&sealed);
    Ok(out)
}

pub fn open(key: &[u8; 32], sealed: &[u8], aad: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if sealed.len() < 12 + 16 {
        return Err(anyhow!("too short"));
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| anyhow!("bad key length"))?;
    let (nonce, body) = sealed.split_at(12);

    cipher
        .decrypt(Nonce::from_slice(nonce), Payload { msg: body, aad })
        .map(Zeroizing::new)
        .map_err(|_| anyhow!("does not open"))
}

pub fn random32() -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0u8; 32]);
    rand::rngs::OsRng.fill_bytes(out.as_mut());
    out
}

/// A password nobody has to type twice: base32 without the letters people misread, in groups.
pub fn generated_password(length: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789abcdefghjkmnpqrstuvwxyz";
    let mut bytes = vec![0u8; length];
    rand::rngs::OsRng.fill_bytes(&mut bytes);

    bytes.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn hkdf_matches_rfc_5869_with_an_empty_salt() {
        // RFC 5869, test case 3: IKM 22 x 0x0b, salt and info empty, L = 42 (first 32 bytes).
        let okm = hkdf32(&[0x0b; 22], b"");
        assert_eq!(hex(okm.as_ref()), "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d");
    }

    #[test]
    fn pbkdf2_matches_the_published_vectors() {
        // RFC 7914 §11: PBKDF2-HMAC-SHA256, P = "passwd", S = "salt", c = 1 (first 32 bytes).
        let out = pbkdf2_sha256(b"passwd", b"salt", 1);
        assert_eq!(hex(out.as_ref()), "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc");
        // P = "Password", S = "NaCl", c = 80000.
        let out = pbkdf2_sha256(b"Password", b"NaCl", 80000);
        assert_eq!(hex(out.as_ref()), "4ddcd8f60b98be21830cee5ef22701f9641a4418d04c0414aeff08876b34ab56");
    }

    #[test]
    fn a_layer_opens_only_with_its_key_and_its_share() {
        let key = random32();
        let k = envelope_key(&key);
        let sealed = seal(&k, b"envelope", &aad("abc", 10)).unwrap();

        assert_eq!(open(&k, &sealed, &aad("abc", 10)).unwrap().as_slice(), b"envelope");
        assert!(open(&k, &sealed, &aad("abc", 11)).is_err(), "another expiry");
        assert!(open(&content_key(&key, None), &sealed, &aad("abc", 10)).is_err(), "the other layer's key");
    }

    #[test]
    fn the_password_changes_the_content_key() {
        let key = random32();
        let salt = [7u8; 16];
        let with = content_key(&key, Some(("senha", &salt, 1000)));

        assert_ne!(with.as_ref(), content_key(&key, None).as_ref());
        assert_ne!(with.as_ref(), content_key(&key, Some(("outra", &salt, 1000))).as_ref());
        assert_eq!(with.as_ref(), content_key(&key, Some(("senha", &salt, 1000))).as_ref());
    }
}
