//! The token that says a request came from the window keyhold opened.
//!
//! keyhold binds to loopback, and loopback is not a boundary: every process running as this user
//! can reach the port, and a web page in any tab can try to talk to it. So the HTTP surface starts
//! from "no" and the token is what turns it into "yes".
//!
//! Two decisions about how it travels:
//!
//!  - **It is handed over in the URL fragment**, never the query string. Everything after `#` stays
//!    in the browser: it is not in the request line, not in a `Referer`, not in any server log —
//!    including the logs of a page the user visits next.
//!  - **It is compared in constant time.** A byte-at-a-time comparison against a value an attacker
//!    can submit repeatedly is a value an attacker can learn one byte at a time.
//!
//! The token is generated per run and never written to disk. Restarting keyhold invalidates every
//! open tab, which is the correct behaviour: a tab from the previous run is a tab whose provenance
//! this process cannot vouch for.

use rand::RngCore;
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// 32 bytes of entropy, URL-safe, no padding.
const TOKEN_BYTES: usize = 32;

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SessionToken {
    value: String,
}

impl SessionToken {
    pub fn generate() -> Self {
        let mut bytes = [0u8; TOKEN_BYTES];
        rand::rngs::OsRng.fill_bytes(&mut bytes);

        let value = base64_url(&bytes);
        bytes.zeroize();

        Self { value }
    }

    /// Only for putting into the URL keyhold opens. Never logged, never persisted.
    pub fn expose(&self) -> &str {
        &self.value
    }

    pub fn matches(&self, candidate: &str) -> bool {
        // Length is not secret — a mismatch there is answered immediately — but the bytes are.
        candidate.len() == self.value.len()
            && candidate.as_bytes().ct_eq(self.value.as_bytes()).into()
    }
}

/// URL-safe base64 without padding, written out rather than pulled in as a dependency.
///
/// A dependency in the path that handles key-adjacent bytes is a dependency whose next release can
/// read them. This is twenty lines and has no reason to ever change.
fn base64_url(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    let mut out = String::with_capacity((input.len() * 4 + 2) / 3);

    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;

        out.push(ALPHABET[(n >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6 & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_differ_between_runs() {
        assert_ne!(
            SessionToken::generate().expose(),
            SessionToken::generate().expose()
        );
    }

    #[test]
    fn token_is_long_enough_to_be_unguessable() {
        // 32 bytes in base64url, unpadded.
        assert_eq!(SessionToken::generate().expose().len(), 43);
    }

    #[test]
    fn only_the_exact_token_matches() {
        let token = SessionToken::generate();
        let value = token.expose().to_string();

        assert!(token.matches(&value));
        assert!(!token.matches(""));
        assert!(!token.matches(&value[..value.len() - 1]));
        assert!(!token.matches(&format!("{value}x")));
    }
}
