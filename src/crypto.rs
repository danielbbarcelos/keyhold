//! The primitives, and the reasoning behind each choice.
//!
//! Everything here is standard, boring and well-reviewed. That is the point: a vault is not the
//! place to be clever, and the interesting decisions in this project are about *where* keys live
//! and *when* they exist, not about inventing a construction.
//!
//! - **Argon2id** turns a master password into a key. Memory-hard, so an attacker with the vault
//!   file cannot trade cheap parallel hardware for time. The parameters are stored in the file and
//!   versioned, so they can be raised later without re-encrypting anything.
//! - **AES-256-GCM** for every payload, always with additional authenticated data. The AAD is what
//!   makes a ciphertext refuse to open in the wrong place — swapping two records, or restoring an
//!   old version of one, fails loudly instead of silently succeeding.
//! - **`OsRng`** for every random byte. Never a seeded PRNG, never `rand::random` with a thread RNG
//!   whose seeding this code did not choose.
//!
//! Key material is wrapped in [`SecretKey`], which zeroizes on drop and refuses to print itself.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, bail, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// AES-256: 32 bytes. GCM nonce: 12 bytes, the size the construction is designed around.
pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;
pub const SALT_LEN: usize = 16;

/// The floor for Argon2id, in the units the crate uses.
///
/// 64 MiB and three passes is the recommendation for an interactive unlock: it costs about a
/// quarter of a second on a laptop and makes a large-scale offline guess expensive. These are the
/// *minimum* keyhold will accept when reading parameters back out of a vault file — a file that
/// asks for less is a file somebody edited.
pub const ARGON2_MEMORY_KIB: u32 = 64 * 1024;
pub const ARGON2_ITERATIONS: u32 = 3;
pub const ARGON2_PARALLELISM: u32 = 1;

/// Key material: zeroized on drop, never printed, never serialised.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretKey([u8; KEY_LEN]);

impl SecretKey {
    pub fn random() -> Self {
        let mut bytes = [0u8; KEY_LEN];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

/// Printing a key by accident is how a well-encrypted vault ends up in a log file.
impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(…)")
    }
}

/// The Argon2id settings a particular vault was built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            memory_kib: ARGON2_MEMORY_KIB,
            iterations: ARGON2_ITERATIONS,
            parallelism: ARGON2_PARALLELISM,
        }
    }
}

impl KdfParams {
    /// Reject anything weaker than the floor.
    ///
    /// Checked on the way *in*, when a vault file is read, rather than only on the way out: the
    /// attack this stops is an edited file that asks keyhold to derive a key cheaply.
    pub fn validate(&self) -> Result<()> {
        if self.memory_kib < ARGON2_MEMORY_KIB
            || self.iterations < ARGON2_ITERATIONS
            || self.parallelism == 0
        {
            bail!(
                "this vault asks for a weaker key derivation than Keyhold accepts \
                 ({} KiB / {} passes); it may have been tampered with",
                self.memory_kib,
                self.iterations
            );
        }

        Ok(())
    }
}

pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

/// Turn a password into a key.
///
/// Deliberately slow. The cost is paid once per unlock by the person, and once per guess by
/// anybody attacking the file — which is the entire trade this function exists to make.
pub fn derive_key(password: &str, salt: &[u8], params: KdfParams) -> Result<SecretKey> {
    params.validate()?;

    let argon = Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(
            params.memory_kib,
            params.iterations,
            params.parallelism,
            Some(KEY_LEN),
        )
        .map_err(|e| anyhow!("invalid key derivation parameters: {e}"))?,
    );

    let mut out = [0u8; KEY_LEN];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| anyhow!("could not derive a key: {e}"))?;

    let key = SecretKey::from_bytes(out);
    out.zeroize();

    Ok(key)
}

/// Encrypt, binding the ciphertext to `aad`.
///
/// The nonce is random and prepended. Random-with-a-fresh-key is the safe pattern here: keyhold
/// never encrypts enough messages under one key for a 96-bit random nonce to be a concern, and a
/// counter would have to be persisted correctly across crashes to be safer.
pub fn seal(key: &SecretKey, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| anyhow!("could not build the cipher: {e}"))?;

    let nonce_bytes = random_bytes(NONCE_LEN);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let mut out = cipher
        .encrypt(nonce, Payload { msg: plaintext, aad })
        .map_err(|_| anyhow!("could not encrypt"))?;

    let mut sealed = nonce_bytes;
    sealed.append(&mut out);

    Ok(sealed)
}

/// Decrypt, requiring the same `aad`.
///
/// A wrong key, a tampered byte or the wrong `aad` all arrive here as the same failure, and that is
/// intentional: the caller gets "this did not open", never a hint about which of the three it was.
pub fn open(key: &SecretKey, sealed: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    if sealed.len() <= NONCE_LEN {
        bail!("this value did not open");
    }

    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| anyhow!("could not build the cipher: {e}"))?;

    let (nonce_bytes, ciphertext) = sealed.split_at(NONCE_LEN);

    cipher
        .decrypt(
            Nonce::from_slice(nonce_bytes),
            Payload { msg: ciphertext, aad },
        )
        .map_err(|_| anyhow!("this value did not open"))
}

/// Wrap one key inside another.
pub fn wrap_key(wrapping: &SecretKey, target: &SecretKey, aad: &[u8]) -> Result<Vec<u8>> {
    seal(wrapping, target.as_bytes(), aad)
}

pub fn unwrap_key(wrapping: &SecretKey, wrapped: &[u8], aad: &[u8]) -> Result<SecretKey> {
    let mut bytes = open(wrapping, wrapped, aad)?;

    if bytes.len() != KEY_LEN {
        bytes.zeroize();
        bail!("this value did not open");
    }

    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&bytes);
    bytes.zeroize();

    Ok(SecretKey::from_bytes(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AAD: &[u8] = b"keyhold:test:1";

    #[test]
    fn the_same_password_and_salt_give_the_same_key() {
        let salt = random_bytes(SALT_LEN);
        let a = derive_key("correct horse", &salt, KdfParams::default()).unwrap();
        let b = derive_key("correct horse", &salt, KdfParams::default()).unwrap();

        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn a_different_salt_gives_a_different_key() {
        let a = derive_key("same password", &random_bytes(SALT_LEN), KdfParams::default()).unwrap();
        let b = derive_key("same password", &random_bytes(SALT_LEN), KdfParams::default()).unwrap();

        assert_ne!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn a_sealed_value_comes_back_intact() {
        let key = SecretKey::random();
        let sealed = seal(&key, b"the secret", AAD).unwrap();

        assert_ne!(sealed.as_slice(), b"the secret");
        assert_eq!(open(&key, &sealed, AAD).unwrap(), b"the secret");
    }

    #[test]
    fn one_flipped_byte_fails_to_open() {
        let key = SecretKey::random();
        let mut sealed = seal(&key, b"the secret", AAD).unwrap();

        let last = sealed.len() - 1;
        sealed[last] ^= 0x01;

        assert!(open(&key, &sealed, AAD).is_err(), "a tampered ciphertext opened");
    }

    #[test]
    fn the_wrong_context_fails_to_open() {
        let key = SecretKey::random();
        let sealed = seal(&key, b"the secret", b"record:1|v1").unwrap();

        // The same key, the same ciphertext, a different place: this is a record moved, or an old
        // version restored, and it must not open.
        assert!(open(&key, &sealed, b"record:2|v1").is_err());
        assert!(open(&key, &sealed, b"record:1|v2").is_err());
    }

    #[test]
    fn the_wrong_password_fails_to_open() {
        let salt = random_bytes(SALT_LEN);
        let right = derive_key("right", &salt, KdfParams::default()).unwrap();
        let wrong = derive_key("wrong", &salt, KdfParams::default()).unwrap();

        let sealed = seal(&right, b"the secret", AAD).unwrap();

        assert!(open(&wrong, &sealed, AAD).is_err());
    }

    #[test]
    fn a_wrapped_key_survives_the_round_trip() {
        let wrapping = SecretKey::random();
        let target = SecretKey::random();

        let wrapped = wrap_key(&wrapping, &target, AAD).unwrap();
        let recovered = unwrap_key(&wrapping, &wrapped, AAD).unwrap();

        assert_eq!(recovered.as_bytes(), target.as_bytes());
    }

    #[test]
    fn weak_parameters_are_refused() {
        let weak = KdfParams { memory_kib: 8, iterations: 1, parallelism: 1 };

        assert!(weak.validate().is_err());
        assert!(derive_key("password", &random_bytes(SALT_LEN), weak).is_err());
    }

    #[test]
    fn a_key_never_prints_itself() {
        let key = SecretKey::random();
        assert_eq!(format!("{key:?}"), "SecretKey(…)");
    }
}
