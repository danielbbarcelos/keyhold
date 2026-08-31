//! Base64url and base32, written out rather than pulled in.
//!
//! Both sit directly in the path that handles key material: base64url carries the session token
//! and the recovery key, base32 carries the TOTP secret. A dependency in that path is a dependency
//! whose next release can read it, and these two are eighty lines that have no reason to ever
//! change again.

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// URL-safe base64, unpadded.
pub fn base64url(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);

    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;

        out.push(B64[(n >> 18 & 63) as usize] as char);
        out.push(B64[(n >> 12 & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64[(n >> 6 & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64[(n & 63) as usize] as char);
        }
    }

    out
}

pub fn from_base64url(input: &str) -> Option<Vec<u8>> {
    let mut bits = 0u32;
    let mut held = 0u8;
    let mut out = Vec::with_capacity(input.len() * 3 / 4);

    for byte in input.bytes() {
        let value = B64.iter().position(|c| *c == byte)? as u32;

        bits = (bits << 6) | value;
        held += 6;

        if held >= 8 {
            held -= 8;
            out.push((bits >> held) as u8);
        }
    }

    Some(out)
}

/// RFC 4648 base32, unpadded and uppercase — the alphabet authenticator apps expect.
pub fn base32(input: &[u8]) -> String {
    let mut bits = 0u32;
    let mut held = 0u8;
    let mut out = String::new();

    for byte in input {
        bits = (bits << 8) | *byte as u32;
        held += 8;

        while held >= 5 {
            held -= 5;
            out.push(B32[((bits >> held) & 31) as usize] as char);
        }
    }

    if held > 0 {
        out.push(B32[((bits << (5 - held)) & 31) as usize] as char);
    }

    out
}

/// Standard base64, padded — the alphabet a `data:` URL is read with.
///
/// Separate from [`base64url`] on purpose rather than parameterised: the two are used for opposite
/// reasons (one must survive a URL, the other must be decoded by a browser) and a shared function
/// with a flag is a function somebody eventually calls with the wrong flag.
pub fn base64_standard(input: &[u8]) -> String {
    const B64S: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);

    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;

        out.push(B64S[(n >> 18 & 63) as usize] as char);
        out.push(B64S[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { B64S[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64S[(n & 63) as usize] as char } else { '=' });
    }

    out
}

pub fn from_base32(input: &str) -> Option<Vec<u8>> {
    let mut bits = 0u32;
    let mut held = 0u8;
    let mut out = Vec::with_capacity(input.len() * 5 / 8);

    for byte in input.bytes() {
        let value = B32.iter().position(|c| *c == byte)? as u32;

        bits = (bits << 5) | value;
        held += 5;

        if held >= 8 {
            held -= 8;
            out.push((bits >> held) as u8);
        }
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_round_trips() {
        for len in 0..40 {
            let bytes = crate::crypto::random_bytes(len);
            let encoded = base64url(&bytes);

            assert_eq!(from_base64url(&encoded).unwrap(), bytes, "failed at {len} bytes");
        }
    }

    #[test]
    fn base64url_avoids_characters_that_break_a_url() {
        let encoded = base64url(&crate::crypto::random_bytes(256));

        assert!(!encoded.contains('+') && !encoded.contains('/') && !encoded.contains('='));
    }

    #[test]
    fn base32_matches_the_rfc_vectors() {
        // RFC 4648 §10, minus the padding authenticator apps do not use.
        assert_eq!(base32(b"f"), "MY");
        assert_eq!(base32(b"fo"), "MZXQ");
        assert_eq!(base32(b"foo"), "MZXW6");
        assert_eq!(base32(b"foob"), "MZXW6YQ");
        assert_eq!(base32(b"fooba"), "MZXW6YTB");
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
    }

    #[test]
    fn base32_round_trips() {
        for len in 0..40 {
            let bytes = crate::crypto::random_bytes(len);
            let encoded = base32(&bytes);

            assert_eq!(from_base32(&encoded).unwrap(), bytes, "failed at {len} bytes");
        }
    }

    #[test]
    fn base32_says_the_same_thing_in_any_case_it_is_written() {
        // The alphabet is uppercase only, which is the property that lets a person copy a recovery
        // key off a screen without the case mattering.
        assert!(base32(&crate::crypto::random_bytes(32))
            .chars()
            .all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)));
    }

    #[test]
    fn rubbish_is_rejected_rather_than_guessed_at() {
        assert!(from_base64url("not base64 !!").is_none());
    }
}
