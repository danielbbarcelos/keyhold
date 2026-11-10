//! Time-based one-time passwords (RFC 6238), for the optional second factor.
//!
//! **What this buys, said honestly.** The TOTP secret has to live in the vault file for keyhold to
//! check a code, sealed under the vault key like everything else. So somebody who takes the file
//! and breaks the master password gets the secret too, and can generate codes. A second factor
//! does not add resistance to an *offline* attack on the file, and keyhold's interface says so
//! rather than implying otherwise.
//!
//! What it does buy is real: it stops somebody who has the password but not your phone. A password
//! read over your shoulder, typed into a machine somebody else can reach, reused from somewhere
//! that leaked, or sitting in a password manager they already opened — in all of those the file is
//! not in their hands, and the code is.
//!
//! Implementation notes that matter: HMAC-SHA1 because that is what authenticator apps implement,
//! comparison in constant time, and a ±1 step window so a phone whose clock drifts by a few
//! seconds still works.

use hmac::{Hmac, Mac};
use sha1::Sha1;
use subtle::ConstantTimeEq;

use crate::encoding::base32;

type HmacSha1 = Hmac<Sha1>;

/// The values every authenticator app assumes when the URI does not say otherwise.
pub const STEP_SECONDS: u64 = 30;
pub const DIGITS: u32 = 6;
/// 160 bits — the size RFC 4226 recommends for the shared secret.
pub const SECRET_LEN: usize = 20;

/// How far out of step a code is still accepted, in either direction.
///
/// One step is thirty seconds each way. Wider would be friendlier and would also widen the window
/// in which an observed code is still usable.
const DRIFT_STEPS: u64 = 1;

pub fn generate_secret() -> Vec<u8> {
    crate::crypto::random_bytes(SECRET_LEN)
}

/// The `otpauth://` URI an authenticator app scans.
///
/// The label carries the account name so a person with several vaults can tell them apart in the
/// app's list. Nothing secret is in it except the secret itself, which is the entire purpose.
pub fn provisioning_uri(secret: &[u8], account: &str) -> String {
    format!(
        "otpauth://totp/Keyhold:{}?secret={}&issuer=Keyhold&algorithm=SHA1&digits={}&period={}",
        urlencode(account),
        base32(secret),
        DIGITS,
        STEP_SECONDS
    )
}

fn urlencode(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

/// The code for one counter step.
fn code_at(secret: &[u8], counter: u64) -> String {
    code_at_digits(secret, counter, DIGITS)
}

fn code_at_digits(secret: &[u8], counter: u64, digits: u32) -> String {
    let mut mac = HmacSha1::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&counter.to_be_bytes());

    let digest = mac.finalize().into_bytes();

    // Dynamic truncation, RFC 4226 §5.4.
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary = ((digest[offset] & 0x7f) as u32) << 24
        | (digest[offset + 1] as u32) << 16
        | (digest[offset + 2] as u32) << 8
        | (digest[offset + 3] as u32);

    format!("{:0width$}", binary % 10u32.pow(digits), width = digits as usize)
}

/// The code a record's "Código 2FA" field gives right now, and how many seconds it has left.
///
/// The field holds what a site hands out when you turn on two-factor: either the bare base32
/// secret or the whole `otpauth://` address (whose `digits` and `period` are honoured; SHA-1 only,
/// which is what sites issue). Anything else gives `None`, and the interface says the field is not
/// a 2FA secret instead of showing a wrong code.
pub fn code_for_field(value: &str, unix_seconds: u64) -> Option<(String, u64)> {
    let value = value.trim();
    let (mut secret, mut digits, mut period) = (value.to_string(), DIGITS, STEP_SECONDS);

    if let Some(rest) = value.strip_prefix("otpauth://totp/") {
        let query = rest.split_once('?')?.1;
        secret.clear();
        for pair in query.split('&') {
            let (key, val) = pair.split_once('=').unwrap_or((pair, ""));
            match key.to_ascii_lowercase().as_str() {
                "secret" => secret = val.to_string(),
                "digits" => digits = val.parse().ok().filter(|d| (6..=8).contains(d))?,
                "period" => period = val.parse().ok().filter(|p| *p > 0)?,
                "algorithm" if !val.eq_ignore_ascii_case("sha1") => return None,
                _ => {}
            }
        }
    }

    let cleaned: String =
        secret.chars().filter(|c| !c.is_whitespace() && *c != '=').collect::<String>().to_uppercase();
    let key = crate::encoding::from_base32(&cleaned).filter(|k| !k.is_empty())?;
    let code = code_at_digits(&key, unix_seconds / period, digits);

    Some((code, period - unix_seconds % period))
}

pub fn code_now(secret: &[u8], unix_seconds: u64) -> String {
    code_at(secret, unix_seconds / STEP_SECONDS)
}

/// Whether `candidate` is a code this secret produces around `unix_seconds`.
///
/// Compared in constant time: a code is only six digits, and an oracle that leaks how many of them
/// matched turns a one-in-a-million guess into six one-in-ten guesses.
pub fn verify(secret: &[u8], candidate: &str, unix_seconds: u64) -> bool {
    let candidate = candidate.trim().replace(' ', "");

    if candidate.len() != DIGITS as usize || !candidate.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }

    let current = unix_seconds / STEP_SECONDS;
    let mut matched = false;

    // Every step in the window is checked, without an early return, so that the time this takes
    // does not say which step matched.
    for step in current.saturating_sub(DRIFT_STEPS)..=current + DRIFT_STEPS {
        let expected = code_at(secret, step);
        let hit: bool = expected.as_bytes().ct_eq(candidate.as_bytes()).into();
        matched |= hit;
    }

    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 test vectors, SHA-1, with the seed the appendix uses.
    const RFC_SECRET: &[u8] = b"12345678901234567890";

    #[test]
    fn matches_the_rfc_vectors() {
        assert_eq!(code_now(RFC_SECRET, 59), "287082");
        assert_eq!(code_now(RFC_SECRET, 1111111109), "081804");
        assert_eq!(code_now(RFC_SECRET, 1111111111), "050471");
        assert_eq!(code_now(RFC_SECRET, 1234567890), "005924");
        assert_eq!(code_now(RFC_SECRET, 2000000000), "279037");
    }

    #[test]
    fn the_current_code_verifies() {
        let secret = generate_secret();
        let now = 1_700_000_000;

        assert!(verify(&secret, &code_now(&secret, now), now));
    }

    #[test]
    fn a_clock_off_by_one_step_still_works() {
        let secret = generate_secret();
        let now = 1_700_000_000;

        assert!(verify(&secret, &code_now(&secret, now - STEP_SECONDS), now));
        assert!(verify(&secret, &code_now(&secret, now + STEP_SECONDS), now));
    }

    #[test]
    fn a_code_expires_outside_the_window() {
        let secret = generate_secret();
        let now = 1_700_000_000;

        assert!(!verify(&secret, &code_now(&secret, now - 5 * STEP_SECONDS), now));
    }

    #[test]
    fn rubbish_is_refused_without_ceremony() {
        let secret = generate_secret();
        let now = 1_700_000_000;

        for candidate in ["", "12345", "1234567", "abcdef", "  ", "12 34 56"] {
            assert!(!verify(&secret, candidate, now), "accepted {candidate:?}");
        }
    }

    #[test]
    fn the_provisioning_uri_is_what_an_app_expects() {
        let uri = provisioning_uri(b"12345678901234567890", "daniel@example.com");

        assert!(uri.starts_with("otpauth://totp/Keyhold:"));
        assert!(uri.contains("secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"));
        assert!(uri.contains("issuer=Keyhold"));
        assert!(uri.contains("digits=6"));
        assert!(uri.contains("period=30"));
    }

    #[test]
    fn a_record_field_gives_the_rfc_code_from_a_bare_secret_or_an_address() {
        let bare = "GEZD GNBV GY3T QOJQ GEZD GNBV GY3T QOJQ";
        assert_eq!(code_for_field(bare, 59), Some(("287082".into(), 1)));

        let address = "otpauth://totp/x?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&digits=8&period=30";
        assert_eq!(code_for_field(address, 59).unwrap().0, "94287082");

        assert_eq!(code_for_field("not a secret!", 59), None);
        assert_eq!(code_for_field("otpauth://totp/x?secret=GEZD&algorithm=SHA256", 59), None);
    }
}
