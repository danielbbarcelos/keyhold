//! A ZIP with one file, encrypted with AES-256 in the WinZip AE-2 format.
//!
//! The only ZIP encryption worth the name: the traditional "ZipCrypto" falls to a known-plaintext
//! attack in minutes, and a file Keyhold writes has a predictable start. AE-2 is what 7-Zip, Keka,
//! The Unarchiver and WinZip open; the archive utilities built into macOS and some versions of
//! Windows do not, and the share dialog says so.
//!
//! Written here rather than pulled in: it is a hundred lines over the primitives already in the
//! build (AES, HMAC-SHA1), and a ZIP library would bring compression, readers and formats this
//! never needs. Stored, not compressed: one small text file.
//!
//! The format (APPNOTE.TXT §7.2, and WinZip's "AES Encryption Information"):
//! - PBKDF2-HMAC-SHA1(password, 16-byte salt, 1000 iterations) gives 66 bytes: the AES key, the
//!   HMAC key and a 2-byte password verifier. 1000 iterations is fixed by the format, which is why
//!   the password is generated, long and random.
//! - AES-256 in CTR mode with a little-endian 128-bit counter starting at 1.
//! - The first 10 bytes of HMAC-SHA1 over the ciphertext follow it. AE-2 writes no CRC: the HMAC
//!   is the integrity check, and a CRC of the plaintext would leak something about it.
//! - The name, sizes and date stay in clear in any ZIP: the file inside has a fixed name.

use aes_gcm::aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
use aes_gcm::aes::Aes256;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha1::Sha1;
use zeroize::Zeroizing;

type HmacSha1 = Hmac<Sha1>;

const ITERATIONS: u32 = 1000;

/// PBKDF2-HMAC-SHA1 to `length` bytes.
fn pbkdf2_sha1(password: &[u8], salt: &[u8], length: usize) -> Zeroizing<Vec<u8>> {
    let prf = <HmacSha1 as Mac>::new_from_slice(password).expect("any key length");
    let mut out = Zeroizing::new(Vec::with_capacity(length));
    let mut block = 1u32;

    while out.len() < length {
        let mut first = prf.clone();
        first.update(salt);
        first.update(&block.to_be_bytes());
        let mut u = first.finalize().into_bytes();
        let mut t = u;

        for _ in 1..ITERATIONS {
            let mut next = prf.clone();
            next.update(&u);
            u = next.finalize().into_bytes();
            for (a, b) in t.iter_mut().zip(u.iter()) {
                *a ^= b;
            }
        }

        out.extend_from_slice(&t);
        block += 1;
    }

    out.truncate(length);
    out
}

/// AES-256-CTR with WinZip's little-endian counter, starting at 1.
fn ctr(key: &[u8], data: &mut [u8]) {
    let cipher = Aes256::new(GenericArray::from_slice(key));
    let mut counter: u128 = 1;

    for chunk in data.chunks_mut(16) {
        let mut block = GenericArray::clone_from_slice(&counter.to_le_bytes());
        cipher.encrypt_block(&mut block);
        for (byte, pad) in chunk.iter_mut().zip(block.iter()) {
            *byte ^= pad;
        }
        counter = counter.wrapping_add(1);
    }
}

/// DOS date and time for the ZIP headers, from Unix seconds (UTC).
fn dos_time(unix: u64) -> (u16, u16) {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u16;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u16;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as u16;

    let time = ((secs / 3600) as u16) << 11 | ((secs % 3600 / 60) as u16) << 5 | ((secs % 60) / 2) as u16;
    let date = (year.saturating_sub(1980)) << 9 | month << 5 | day;
    (time, date)
}

/// A ZIP holding `name` with `content`, encrypted under `password`.
pub fn encrypted_zip(name: &str, content: &[u8], password: &str, unix_time: u64) -> Vec<u8> {
    let mut salt = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut salt);

    let derived = pbkdf2_sha1(password.as_bytes(), &salt, 66);
    let (aes_key, rest) = derived.split_at(32);
    let (auth_key, verifier) = rest.split_at(32);

    let mut ciphertext = content.to_vec();
    ctr(aes_key, &mut ciphertext);

    let mut mac = <HmacSha1 as Mac>::new_from_slice(auth_key).expect("any key length");
    mac.update(&ciphertext);
    let auth = mac.finalize().into_bytes();

    let mut data = Vec::with_capacity(16 + 2 + ciphertext.len() + 10);
    data.extend_from_slice(&salt);
    data.extend_from_slice(verifier);
    data.extend_from_slice(&ciphertext);
    data.extend_from_slice(&auth[..10]);

    // The AES extra field: AE-2, vendor "AE", AES-256, and the real method (0, stored).
    let mut extra = Vec::new();
    extra.extend_from_slice(&0x9901u16.to_le_bytes());
    extra.extend_from_slice(&7u16.to_le_bytes());
    extra.extend_from_slice(&2u16.to_le_bytes());
    extra.extend_from_slice(b"AE");
    extra.push(3);
    extra.extend_from_slice(&0u16.to_le_bytes());

    let (time, date) = dos_time(unix_time);
    let flags: u16 = 1 | 1 << 11; // encrypted, UTF-8 name
    let method: u16 = 99; // AES
    let version: u16 = 51;
    let name_bytes = name.as_bytes();
    let size = data.len() as u32;

    let mut out = Vec::new();

    // Local file header.
    out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&method.to_le_bytes());
    out.extend_from_slice(&time.to_le_bytes());
    out.extend_from_slice(&date.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // CRC: none in AE-2
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&(content.len() as u32).to_le_bytes());
    out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    out.extend_from_slice(name_bytes);
    out.extend_from_slice(&extra);
    out.extend_from_slice(&data);

    // Central directory.
    let central_at = out.len() as u32;
    out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    out.extend_from_slice(&version.to_le_bytes()); // made by
    out.extend_from_slice(&version.to_le_bytes()); // needed
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&method.to_le_bytes());
    out.extend_from_slice(&time.to_le_bytes());
    out.extend_from_slice(&date.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&(content.len() as u32).to_le_bytes());
    out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment
    out.extend_from_slice(&0u16.to_le_bytes()); // disk
    out.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
    out.extend_from_slice(&0u32.to_le_bytes()); // external attributes
    out.extend_from_slice(&0u32.to_le_bytes()); // offset of the local header
    out.extend_from_slice(name_bytes);
    out.extend_from_slice(&extra);
    let central_size = out.len() as u32 - central_at;

    // End of central directory.
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_at.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbkdf2_sha1_matches_rfc_6070() {
        let out = pbkdf2_sha1(b"password", b"salt", 20);
        let hex: String = out.iter().map(|b| format!("{b:02x}")).collect();
        // RFC 6070, c = 1000 is not listed; c = 1 and 2 are, but this module only runs 1000.
        // The c = 4096 vector cannot be used either, so the check is against a value computed with
        // Python's hashlib.pbkdf2_hmac('sha1', b'password', b'salt', 1000, 20).
        assert_eq!(hex, "6e88be8bad7eae9d9e10aa061224034fed48d03f");
    }

    #[test]
    fn dos_time_is_the_civil_date() {
        // 2026-11-17 14:30:20 UTC.
        let (time, date) = dos_time(1_794_925_820);
        assert_eq!(date >> 9, 2026 - 1980);
        assert_eq!((date >> 5) & 0x0f, 11);
        assert_eq!(date & 0x1f, 17);
        assert_eq!(time >> 11, 14);
        assert_eq!((time >> 5) & 0x3f, 30);
    }

    /// The real check: 7-Zip opens it with the password, and refuses without it.
    #[test]
    fn seven_zip_opens_it_with_the_password_and_not_without() {
        let Ok(found) = std::process::Command::new("7z").arg("i").output() else {
            eprintln!("7z not installed; skipping");
            return;
        };
        if !found.status.success() {
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("registro.zip");
        let text = "Postgres produção\nSenha: hunter2\n".repeat(5);
        std::fs::write(&archive, encrypted_zip("registro.txt", text.as_bytes(), "senha-forte-gerada-1234", 1_794_925_820)).unwrap();

        let ok = std::process::Command::new("7z")
            .args(["x", "-y", "-psenha-forte-gerada-1234"])
            .arg(format!("-o{}", dir.path().join("out").display()))
            .arg(&archive)
            .output()
            .unwrap();
        assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stdout));
        assert_eq!(std::fs::read_to_string(dir.path().join("out/registro.txt")).unwrap(), text);

        let wrong = std::process::Command::new("7z")
            .args(["x", "-y", "-perrada"])
            .arg(format!("-o{}", dir.path().join("wrong").display()))
            .arg(&archive)
            .output()
            .unwrap();
        assert!(!wrong.status.success(), "a wrong password opened it");
    }
}
