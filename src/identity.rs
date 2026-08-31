//! How a vault is protected, and what it takes to open one.
//!
//! keyhold offers three independent layers. Each is a genuine choice with a stated cost, and the
//! interface recommends rather than insists — a vault somebody refuses to use because it nags is a
//! vault that ends up as a text file on the desktop.
//!
//! 1. **A master password** — recommended, and optional. With one, the key that opens the vault is
//!    derived from it and exists nowhere else; the file on disk is unreadable without something
//!    only you know. Without one, keyhold falls back to a *device key* stored beside the vault,
//!    and the honest consequence is that anybody who can read your files can open your vault. That
//!    is a real mode for a machine only you use, and the interface says exactly what it costs.
//! 2. **A second factor from an authenticator app** — optional. See `totp.rs` for what it does and
//!    does not protect against.
//! 3. **Auto-lock** — recommended, and switchable off. Lives in `lock.rs`.
//!
//! The key hierarchy is the same in every mode, which is what keeps the modes from multiplying the
//! code paths that touch key material:
//!
//! ```text
//! master password ──Argon2id(salt)──┐
//!                                   ├─> unlock key ──unwrap──> vault key ──> everything
//! device key file ──────────────────┘
//!
//! recovery key ──Argon2id(recovery salt)──> recovery key-encryption key ──unwrap──> vault key
//! ```
//!
//! The vault key is random, generated once, and never derived from anything. That is what lets the
//! master password change without re-encrypting a single record — changing it re-wraps one key.

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::crypto::{
    self, random_bytes, unwrap_key, wrap_key, KdfParams, SecretKey, KEY_LEN, SALT_LEN,
};
use crate::db::Store;
use crate::encoding::base32;

/// Context strings, so a ciphertext from one place cannot be pasted into another.
const AAD_VAULT_KEY: &[u8] = b"keyhold:vault-key:v1";
const AAD_RECOVERY: &[u8] = b"keyhold:recovery:v1";
const AAD_TOTP: &[u8] = b"keyhold:totp-secret:v1";

/// The recovery key, in bytes. 32 bytes is 256 bits of entropy — it is the last way in, so it is
/// generated at full strength and never typed from memory.
const RECOVERY_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protection {
    /// A master password. The key exists only while you are here to type it.
    Password,
    /// A key file beside the vault. Convenient, and readable by anything that can read your files.
    Device,
}

impl Protection {
    fn as_str(self) -> &'static str {
        match self {
            Protection::Password => "password",
            Protection::Device => "device",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "password" => Ok(Protection::Password),
            "device" => Ok(Protection::Device),
            other => bail!("this vault declares an unknown protection mode: {other}"),
        }
    }
}

/// What the interface needs to decide which screen to show. Nothing here is secret.
#[derive(Debug, Serialize)]
pub struct IdentityStatus {
    pub initialised: bool,
    pub protection: Option<Protection>,
    pub second_factor: bool,
    pub recovery_confirmed: bool,
}

/// What somebody chose on the setup screen.
pub struct SetupOptions {
    /// `None` means no master password — the device-key mode, with its cost already accepted.
    pub password: Option<String>,
}

/// Handed back exactly once, at setup.
pub struct SetupOutcome {
    /// Shown once, never stored, and unrecoverable afterwards.
    pub recovery_key: String,
    pub vault_key: SecretKey,
}

/// The credentials offered at unlock.
pub struct UnlockAttempt {
    pub password: Option<String>,
    pub code: Option<String>,
    /// Used instead of the password when somebody has lost it.
    pub recovery_key: Option<String>,
}

pub fn status(vault: &Store) -> Result<IdentityStatus> {
    let row: Option<(String, i64, Option<Vec<u8>>)> = vault
        .conn()
        .query_row(
            "SELECT protection, recovery_confirmed, totp_secret FROM identity WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;

    Ok(match row {
        None => IdentityStatus {
            initialised: false,
            protection: None,
            second_factor: false,
            recovery_confirmed: false,
        },
        Some((protection, confirmed, totp)) => IdentityStatus {
            initialised: true,
            protection: Some(Protection::parse(&protection)?),
            second_factor: totp.is_some(),
            recovery_confirmed: confirmed != 0,
        },
    })
}

/// Create the vault's identity. Refuses to run twice.
///
/// A second setup would orphan every key already wrapped — every record in the vault would become
/// unopenable, silently. So the answer is a refusal, not an overwrite.
pub fn create(vault: &mut Store, options: SetupOptions) -> Result<SetupOutcome> {
    if status(vault)?.initialised {
        bail!("this vault already has an identity; setting it up again would orphan its contents");
    }

    let protection = match &options.password {
        Some(_) => Protection::Password,
        None => Protection::Device,
    };

    let params = KdfParams::default();
    let vault_key = SecretKey::random();

    // The key that wraps the vault key: derived from the password, or read from the device file.
    let (salt, unlock_key) = match &options.password {
        Some(password) => {
            if password.trim().is_empty() {
                bail!("a master password cannot be blank");
            }

            let salt = random_bytes(SALT_LEN);
            let key = crypto::derive_key(password, &salt, params)?;

            (Some(salt), key)
        }
        None => (None, device_key(vault, true)?),
    };

    let wrapped_vault_key = wrap_key(&unlock_key, &vault_key, AAD_VAULT_KEY)?;

    // The recovery key is independent of the password on purpose: forgetting one must not cost the
    // other, and the whole point is to have a way in when the password is gone.
    let recovery_secret = random_bytes(RECOVERY_BYTES);
    let recovery_key = format_recovery_key(&recovery_secret);
    let recovery_salt = random_bytes(SALT_LEN);
    let recovery_kek = crypto::derive_key(&recovery_key, &recovery_salt, params)?;
    let recovery_wrapped = wrap_key(&recovery_kek, &vault_key, AAD_RECOVERY)?;

    vault.conn().execute(
        "INSERT INTO identity (
             id, protection, kdf_params, salt, wrapped_vault_key,
             recovery_salt, recovery_wrapped_vault_key, recovery_confirmed, created_at
         ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, 0, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        params![
            protection.as_str(),
            serde_json::to_string(&params)?,
            salt,
            wrapped_vault_key,
            recovery_salt,
            recovery_wrapped,
        ],
    )?;

    Ok(SetupOutcome { recovery_key, vault_key })
}

/// Open the vault, or fail without saying which part was wrong.
pub fn unlock(vault: &Store, attempt: UnlockAttempt) -> Result<SecretKey> {
    let row: (String, String, Option<Vec<u8>>, Vec<u8>, Vec<u8>, Vec<u8>, Option<Vec<u8>>) = vault
        .conn()
        .query_row(
            "SELECT protection, kdf_params, salt, wrapped_vault_key,
                    recovery_salt, recovery_wrapped_vault_key, totp_secret
             FROM identity WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow!("there is no vault on this machine yet"))?;

    let (protection, kdf_json, salt, wrapped, recovery_salt, recovery_wrapped, totp_secret) = row;

    let protection = Protection::parse(&protection)?;
    let params: KdfParams = serde_json::from_str(&kdf_json).context("unreadable vault header")?;
    params.validate()?;

    // The recovery key is a whole separate door: it bypasses the password, and it must not bypass
    // the second factor.
    let vault_key = if let Some(recovery) = attempt.recovery_key.as_deref() {
        let normalised = normalise_recovery_key(recovery);
        let kek = crypto::derive_key(&normalised, &recovery_salt, params)?;

        unwrap_key(&kek, &recovery_wrapped, AAD_RECOVERY)
            .map_err(|_| anyhow!("that recovery key does not open this vault"))?
    } else {
        let unlock_key = match protection {
            Protection::Password => {
                let password = attempt
                    .password
                    .as_deref()
                    .ok_or_else(|| anyhow!("this vault needs its master password"))?;
                let salt = salt.ok_or_else(|| anyhow!("unreadable vault header"))?;

                crypto::derive_key(password, &salt, params)?
            }
            Protection::Device => device_key(vault, false)?,
        };

        // Nothing here compares a stored hash. The password is right when the key it derives opens
        // the wrapped vault key, and wrong when it does not — there is no other check to fool.
        unwrap_key(&unlock_key, &wrapped, AAD_VAULT_KEY)
            .map_err(|_| anyhow!("that master password does not open this vault"))?
    };

    if let Some(sealed) = totp_secret {
        let mut secret = crypto::open(&vault_key, &sealed, AAD_TOTP)
            .context("this vault's second factor could not be read")?;

        let code = attempt
            .code
            .as_deref()
            .ok_or_else(|| anyhow!("this vault also needs a code from your authenticator app"))?;

        let accepted = crate::totp::verify(&secret, code, now_seconds());
        secret.zeroize();

        if !accepted {
            bail!("that code is not right, or it has already expired");
        }
    }

    Ok(vault_key)
}

/// Turn on the second factor, sealing the new secret under the vault key.
pub fn enable_second_factor(vault: &Store, vault_key: &SecretKey, secret: &[u8]) -> Result<()> {
    let sealed = crypto::seal(vault_key, secret, AAD_TOTP)?;

    vault
        .conn()
        .execute("UPDATE identity SET totp_secret = ?1 WHERE id = 1", params![sealed])?;

    Ok(())
}

pub fn disable_second_factor(vault: &Store) -> Result<()> {
    vault
        .conn()
        .execute("UPDATE identity SET totp_secret = NULL WHERE id = 1", [])?;

    Ok(())
}

/// Record that the recovery key was written down — the interface asks for it back to be sure.
pub fn confirm_recovery(vault: &Store) -> Result<()> {
    vault
        .conn()
        .execute("UPDATE identity SET recovery_confirmed = 1 WHERE id = 1", [])?;

    Ok(())
}

/// Change the master password, or add or remove one.
///
/// Re-wraps one key. Every record in the vault is untouched, which is the whole reason the vault
/// key is random rather than derived.
pub fn change_protection(
    vault: &Store,
    vault_key: &SecretKey,
    new_password: Option<&str>,
) -> Result<()> {
    let params = KdfParams::default();

    let (protection, salt, unlock_key) = match new_password {
        Some(password) => {
            if password.trim().is_empty() {
                bail!("a master password cannot be blank");
            }

            let salt = random_bytes(SALT_LEN);
            let key = crypto::derive_key(password, &salt, params)?;

            (Protection::Password, Some(salt), key)
        }
        None => (Protection::Device, None, device_key(vault, true)?),
    };

    let wrapped = wrap_key(&unlock_key, vault_key, AAD_VAULT_KEY)?;

    vault.conn().execute(
        "UPDATE identity SET protection = ?1, kdf_params = ?2, salt = ?3, wrapped_vault_key = ?4
         WHERE id = 1",
        params![
            protection.as_str(),
            serde_json::to_string(&params)?,
            salt,
            wrapped
        ],
    )?;

    Ok(())
}

/// The key used when there is no master password.
///
/// A random 32-byte file next to the vault, `0600`. This is the mode's whole honest story: the key
/// is on the disk, so the vault is protected by file permissions rather than by cryptography, and
/// a copy of your home directory is a copy of your vault. keyhold offers it because a machine only
/// you use is a real situation, and refuses to pretend it is equivalent to a password.
fn device_key(vault: &Store, create: bool) -> Result<SecretKey> {
    let path = vault.sibling("device.key");

    if path.exists() {
        let bytes = std::fs::read(&path)?;

        if bytes.len() != KEY_LEN {
            bail!("the device key file is damaged: {}", path.display());
        }

        let mut fixed = [0u8; KEY_LEN];
        fixed.copy_from_slice(&bytes);

        return Ok(SecretKey::from_bytes(fixed));
    }

    if !create {
        bail!(
            "this vault is protected by a device key and the file is missing: {}",
            path.display()
        );
    }

    let key = SecretKey::random();
    crate::paths::write_private_file(&path, key.as_bytes())?;

    Ok(key)
}

/// The recovery key as a person sees it.
///
/// **Base32, not base64.** A recovery key is read off a screen and typed back in, sometimes off a
/// piece of paper months later, so its alphabet has to survive that trip: base32 is uppercase-only
/// and drops the characters that get confused for each other, while base64 is case-sensitive —
/// writing it down in capitals would quietly throw away a third of its entropy. Grouped in fours
/// because a 52-character run is unreadable, and the dashes are decoration the parser ignores.
fn format_recovery_key(secret: &[u8]) -> String {
    group(&base32(secret))
}

fn group(encoded: &str) -> String {
    encoded
        .as_bytes()
        .chunks(4)
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect::<Vec<_>>()
        .join("-")
}

/// Accept it back however it was typed — spaces, missing dashes, lower case.
///
/// Somebody entering a recovery key is already having a bad day.
fn normalise_recovery_key(input: &str) -> String {
    let stripped: String = input
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_uppercase() || ('2'..='7').contains(c))
        .collect();

    group(&stripped)
}

pub fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Only used to check the recovery key round-trips through display and back.
#[cfg(test)]
fn recovery_bytes(formatted: &str) -> Option<Vec<u8>> {
    crate::encoding::from_base32(&formatted.replace('-', ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let vault = Store::open(&dir.path().join("vault.db")).unwrap();

        (dir, vault)
    }

    #[test]
    fn a_password_vault_opens_with_its_password_and_nothing_else() {
        let (_dir, mut vault) = scratch();

        create(&mut vault, SetupOptions { password: Some("a long master password".into()) }).unwrap();

        let opened = unlock(
            &vault,
            UnlockAttempt {
                password: Some("a long master password".into()),
                code: None,
                recovery_key: None,
            },
        );
        assert!(opened.is_ok());

        let wrong = unlock(
            &vault,
            UnlockAttempt { password: Some("almost right".into()), code: None, recovery_key: None },
        );
        assert!(wrong.is_err(), "the wrong password opened the vault");
    }

    #[test]
    fn the_recovery_key_opens_a_vault_whose_password_is_gone() {
        let (_dir, mut vault) = scratch();

        let outcome =
            create(&mut vault, SetupOptions { password: Some("forgotten already".into()) }).unwrap();

        let opened = unlock(
            &vault,
            UnlockAttempt {
                password: None,
                code: None,
                recovery_key: Some(outcome.recovery_key.clone()),
            },
        )
        .unwrap();

        assert_eq!(opened.as_bytes(), outcome.vault_key.as_bytes());
    }

    #[test]
    fn a_recovery_key_is_accepted_however_it_was_typed() {
        let (_dir, mut vault) = scratch();
        let outcome = create(&mut vault, SetupOptions { password: Some("whatever".into()) }).unwrap();

        let mangled = outcome.recovery_key.to_lowercase().replace('-', " ");

        assert!(unlock(
            &vault,
            UnlockAttempt { password: None, code: None, recovery_key: Some(mangled) }
        )
        .is_ok());
    }

    #[test]
    fn the_recovery_key_survives_being_written_down() {
        // The value a person copies off the screen has to carry every bit of the secret behind it.
        // Base64 in capitals would not, which is why this test exists.
        let secret = random_bytes(RECOVERY_BYTES);
        let formatted = format_recovery_key(&secret);

        assert_eq!(recovery_bytes(&formatted).unwrap()[..RECOVERY_BYTES], secret[..]);
    }

    #[test]
    fn however_it_is_typed_it_derives_the_same_key() {
        let secret = random_bytes(RECOVERY_BYTES);
        let shown = format_recovery_key(&secret);

        for typed in [
            shown.clone(),
            shown.to_lowercase(),
            shown.replace('-', ""),
            shown.replace('-', " "),
            format!("  {}  ", shown.to_lowercase()),
        ] {
            assert_eq!(normalise_recovery_key(&typed), shown, "failed for {typed:?}");
        }
    }

    #[test]
    fn a_vault_without_a_password_opens_by_itself_and_says_so() {
        let (_dir, mut vault) = scratch();

        create(&mut vault, SetupOptions { password: None }).unwrap();

        assert_eq!(status(&vault).unwrap().protection, Some(Protection::Device));
        assert!(unlock(
            &vault,
            UnlockAttempt { password: None, code: None, recovery_key: None }
        )
        .is_ok());
    }

    #[test]
    fn the_second_factor_is_required_once_it_is_on() {
        let (_dir, mut vault) = scratch();
        let outcome = create(&mut vault, SetupOptions { password: Some("the password".into()) }).unwrap();

        let secret = crate::totp::generate_secret();
        enable_second_factor(&vault, &outcome.vault_key, &secret).unwrap();

        let without = unlock(
            &vault,
            UnlockAttempt { password: Some("the password".into()), code: None, recovery_key: None },
        );
        assert!(without.is_err(), "the password alone opened a vault with a second factor");

        let with = unlock(
            &vault,
            UnlockAttempt {
                password: Some("the password".into()),
                code: Some(crate::totp::code_now(&secret, now_seconds())),
                recovery_key: None,
            },
        );
        assert!(with.is_ok());
    }

    #[test]
    fn the_recovery_key_does_not_bypass_the_second_factor() {
        let (_dir, mut vault) = scratch();
        let outcome = create(&mut vault, SetupOptions { password: Some("the password".into()) }).unwrap();

        let secret = crate::totp::generate_secret();
        enable_second_factor(&vault, &outcome.vault_key, &secret).unwrap();

        let attempt = unlock(
            &vault,
            UnlockAttempt {
                password: None,
                code: None,
                recovery_key: Some(outcome.recovery_key.clone()),
            },
        );

        assert!(attempt.is_err(), "the recovery key walked past the second factor");
    }

    #[test]
    fn changing_the_password_keeps_the_same_vault_key() {
        let (_dir, mut vault) = scratch();
        let outcome = create(&mut vault, SetupOptions { password: Some("the first one".into()) }).unwrap();

        change_protection(&vault, &outcome.vault_key, Some("the second one")).unwrap();

        let reopened = unlock(
            &vault,
            UnlockAttempt { password: Some("the second one".into()), code: None, recovery_key: None },
        )
        .unwrap();

        assert_eq!(reopened.as_bytes(), outcome.vault_key.as_bytes());
        assert!(unlock(
            &vault,
            UnlockAttempt { password: Some("the first one".into()), code: None, recovery_key: None }
        )
        .is_err());
    }

    #[test]
    fn a_second_setup_is_refused_rather_than_orphaning_the_contents() {
        let (_dir, mut vault) = scratch();

        create(&mut vault, SetupOptions { password: Some("first".into()) }).unwrap();

        assert!(create(&mut vault, SetupOptions { password: Some("second".into()) }).is_err());
    }
}
