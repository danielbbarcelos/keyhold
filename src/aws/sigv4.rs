//! AWS Signature Version 4, by hand, with the `hmac` and `sha2` crates already in the tree.
//!
//! The AWS SDK would do this for us and bring along a few hundred crates, a C toolchain for
//! aws-lc, and a retry and credential-discovery machinery whose behaviour we would then have to
//! audit. SigV4 itself is small and completely specified: build a canonical form of the request,
//! hash it, and HMAC it with a key derived from the secret, the date, the region and the service.
//! Doing it here keeps the one piece of code that touches the secret access key short enough to
//! read in one sitting.
//!
//! The two services Keyhold talks to disagree on one detail, and it is the classic SigV4 bug:
//!
//! - **Every service except S3** expects the path in the canonical request to be URI-encoded
//!   *twice* — once for the wire, once again for signing. (For SSM and the JSON services the path
//!   is always `/`, so this never bites in practice, but the rule is implemented anyway.)
//! - **S3** expects it encoded exactly once, as it went on the wire.
//!
//! Query strings follow the same rule everywhere: each key and value encoded with the strict
//! RFC 3986 unreserved set (spaces as `%20`, never `+`), then sorted by key and value.
//!
//! The secret access key never leaves [`Credentials`] except to seed the HMAC chain, and every
//! intermediate key in that chain is zeroized as soon as the next one exists.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

type HmacSha256 = Hmac<Sha256>;

/// The algorithm name, as it appears at the start of every `Authorization` header.
pub const ALGORITHM: &str = "AWS4-HMAC-SHA256";

/// The SHA-256 of an empty body, which every `GET`, `HEAD` and `DELETE` signs.
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// An AWS access key pair, optionally temporary.
///
/// Temporary credentials (the kind IAM Identity Center hands out) also carry a session token
/// that must travel with every request, and an expiry after which AWS will refuse them. Keyhold
/// only ever holds temporary credentials in practice, but long-lived ones are representable so
/// the signer can be tested against AWS's published examples.
///
/// Zeroized on drop. Never printed: the `Debug` form shows the first four characters of the
/// access key id (enough to tell `AKIA` from `ASIA`) and nothing else.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Credentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
    /// Unix seconds after which AWS refuses these credentials, when they are temporary.
    expires_at: Option<u64>,
}

impl Credentials {
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        session_token: Option<String>,
    ) -> Self {
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            session_token,
            expires_at: None,
        }
    }

    /// The same credentials, marked as expiring at `unix_seconds`.
    pub fn with_expiry(mut self, unix_seconds: u64) -> Self {
        self.expires_at = Some(unix_seconds);
        self
    }

    pub fn access_key_id(&self) -> &str {
        &self.access_key_id
    }

    /// The secret half. Exposed for persisting the pair sealed in a vault, and for nothing else.
    pub fn secret_access_key(&self) -> &str {
        &self.secret_access_key
    }

    pub fn session_token(&self) -> Option<&str> {
        self.session_token.as_deref()
    }

    pub fn expires_at(&self) -> Option<u64> {
        self.expires_at
    }

    /// Whether these credentials are past (or within `margin` seconds of) their expiry.
    ///
    /// A request signed a few seconds before expiry can still arrive after it, so callers check
    /// with a margin rather than waiting for AWS to say no.
    pub fn is_expired(&self, now: u64, margin: u64) -> bool {
        self.expires_at
            .map(|at| now.saturating_add(margin) >= at)
            .unwrap_or(false)
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let prefix: String = self.access_key_id.chars().take(4).collect();
        f.debug_struct("Credentials")
            .field("access_key_id", &format_args!("{prefix}…"))
            .field("secret_access_key", &format_args!("…"))
            .field("session_token", &self.session_token.as_ref().map(|_| "…"))
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Everything about one request that goes into its signature.
#[derive(Debug, Clone)]
pub struct SigningInput<'a> {
    pub method: &'a str,
    /// The `Host` header exactly as it will be sent, with the port when it is not the default.
    pub host: &'a str,
    /// The path as it goes on the wire: already percent-encoded once, see [`encode_path`].
    pub path: &'a str,
    /// Query parameters, *not* encoded. Order does not matter; they are sorted here.
    pub query: &'a [(&'a str, &'a str)],
    /// Headers to sign and send besides `host`, `x-amz-date` and `x-amz-security-token`, which
    /// are added here. Names in any case.
    pub headers: &'a [(&'a str, &'a str)],
    /// Lowercase hex SHA-256 of the body ([`EMPTY_SHA256`] for none).
    pub payload_sha256: &'a str,
    pub region: &'a str,
    pub service: &'a str,
    /// The signing time, in Unix seconds.
    pub unix_time: u64,
    /// `true` for every service except S3. See the module documentation.
    pub double_encode_path: bool,
}

/// What signing produces: the headers to add, and the query string to put on the URL.
#[derive(Clone)]
pub struct Signed {
    /// `x-amz-date`, `x-amz-security-token` (when there is one) and `authorization`, to be sent
    /// alongside the headers that were passed in. `host` is not here: the HTTP client sets it from
    /// the URL, which is why the URL must be built from the same host that was signed.
    pub headers: Vec<(String, String)>,
    /// The canonical query string, which is also a correct wire encoding. Empty for none.
    pub query: String,
    /// The signature alone, for tests and diagnostics. Not secret: it is in the header anyway, and
    /// it is only valid for this exact request for a few minutes.
    pub signature: String,
}

impl std::fmt::Debug for Signed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The security token rides in `headers`, so the headers are not printed.
        f.debug_struct("Signed")
            .field("query", &self.query)
            .field("signature", &self.signature)
            .finish_non_exhaustive()
    }
}

/// Sign a request.
pub fn sign(credentials: &Credentials, input: &SigningInput<'_>) -> Signed {
    let (amz_date, date) = amz_date(input.unix_time);

    let mut headers: Vec<(String, String)> = input
        .headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), canonical_header_value(value)))
        .collect();
    headers.push(("host".into(), input.host.to_string()));
    headers.push(("x-amz-date".into(), amz_date.clone()));
    if let Some(token) = credentials.session_token() {
        headers.push(("x-amz-security-token".into(), token.to_string()));
    }
    headers.sort_by(|a, b| a.0.cmp(&b.0));

    // Duplicate names are joined with commas, in the order they were given.
    let mut merged: Vec<(String, String)> = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        match merged.last_mut() {
            Some((last, joined)) if *last == name => {
                joined.push(',');
                joined.push_str(&value);
            }
            _ => merged.push((name, value)),
        }
    }

    let canonical_headers: String = merged
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect();
    let signed_headers = merged
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");

    let query = canonical_query(input.query);
    let canonical_uri = if input.path.is_empty() {
        "/".to_string()
    } else if input.double_encode_path {
        // The second pass: the `%` of every escape from the first becomes `%25`.
        uri_encode(input.path, true)
    } else {
        input.path.to_string()
    };

    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload}",
        method = input.method,
        payload = input.payload_sha256,
    );

    let scope = format!("{date}/{}/{}/aws4_request", input.region, input.service);
    let string_to_sign = format!(
        "{ALGORITHM}\n{amz_date}\n{scope}\n{}",
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );

    let key = signing_key(
        &credentials.secret_access_key,
        &date,
        input.region,
        input.service,
    );
    let signature = hex(&hmac(&key, string_to_sign.as_bytes()));

    let authorization = format!(
        "{ALGORITHM} Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        credentials.access_key_id
    );

    let mut out = vec![("x-amz-date".to_string(), amz_date)];
    if let Some(token) = credentials.session_token() {
        out.push(("x-amz-security-token".into(), token.to_string()));
    }
    out.push(("authorization".into(), authorization));

    Signed {
        headers: out,
        query,
        signature,
    }
}

/// A request signed in its query string (a "presigned URL"), so whoever holds the URL can make
/// that one request, to that one object, until it expires.
pub struct PresignInput<'a> {
    pub method: &'a str,
    pub host: &'a str,
    /// The path as it goes on the wire, already encoded (`encode_path`).
    pub path: &'a str,
    /// Parameters signed along with it (`versionId`, `x-amz-expected-bucket-owner`).
    pub query: &'a [(&'a str, &'a str)],
    pub region: &'a str,
    pub service: &'a str,
    pub unix_time: u64,
    pub expires_in: u64,
}

/// The full query string of a presigned request, signature included. Only `host` is signed as a
/// header and the payload is `UNSIGNED-PAYLOAD`, as S3 expects of a URL handed to someone else.
pub fn presign(credentials: &Credentials, input: &PresignInput<'_>) -> String {
    let (amz_date, date) = amz_date(input.unix_time);
    let scope = format!("{date}/{}/{}/aws4_request", input.region, input.service);
    let credential = format!("{}/{scope}", credentials.access_key_id);
    let expires = input.expires_in.to_string();

    let mut params: Vec<(&str, &str)> = input.query.to_vec();
    params.push(("X-Amz-Algorithm", ALGORITHM));
    params.push(("X-Amz-Credential", &credential));
    params.push(("X-Amz-Date", &amz_date));
    params.push(("X-Amz-Expires", &expires));
    params.push(("X-Amz-SignedHeaders", "host"));
    if let Some(token) = credentials.session_token() {
        params.push(("X-Amz-Security-Token", token));
    }

    let query = canonical_query(&params);
    let canonical_request = format!(
        "{method}\n{path}\n{query}\nhost:{host}\n\nhost\nUNSIGNED-PAYLOAD",
        method = input.method,
        path = input.path,
        host = input.host,
    );
    let string_to_sign = format!(
        "{ALGORITHM}\n{amz_date}\n{scope}\n{}",
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );

    let key = signing_key(&credentials.secret_access_key, &date, input.region, input.service);
    let signature = hex(&hmac(&key, string_to_sign.as_bytes()));

    format!("{query}&X-Amz-Signature={signature}")
}

/// The signed fields of a browser POST to S3: the policy and what signs it.
pub struct PostPolicy {
    pub policy: String,
    pub credential: String,
    pub date: String,
    pub signature: String,
    pub security_token: Option<String>,
}

/// Sign an S3 POST policy. `conditions` are the policy's own; the algorithm, credential, date and
/// token conditions are added here, since the signature covers them.
pub fn post_policy(
    credentials: &Credentials,
    region: &str,
    unix_time: u64,
    expires_at: u64,
    mut conditions: Vec<serde_json::Value>,
) -> PostPolicy {
    let (signed_at, date) = amz_date(unix_time);
    let credential = format!("{}/{date}/{region}/s3/aws4_request", credentials.access_key_id);

    conditions.push(serde_json::json!({ "x-amz-algorithm": ALGORITHM }));
    conditions.push(serde_json::json!({ "x-amz-credential": credential }));
    conditions.push(serde_json::json!({ "x-amz-date": signed_at }));
    if let Some(token) = credentials.session_token() {
        conditions.push(serde_json::json!({ "x-amz-security-token": token }));
    }

    let (expiry, _) = amz_date(expires_at);
    // `20261117T143020Z` to the ISO form the policy wants.
    let iso = format!(
        "{}-{}-{}T{}:{}:{}Z",
        &expiry[0..4],
        &expiry[4..6],
        &expiry[6..8],
        &expiry[9..11],
        &expiry[11..13],
        &expiry[13..15]
    );
    let document = serde_json::json!({ "expiration": iso, "conditions": conditions }).to_string();
    let policy = crate::encoding::base64_standard(document.as_bytes());

    let key = signing_key(&credentials.secret_access_key, &date, region, "s3");
    let signature = hex(&hmac(&key, policy.as_bytes()));

    PostPolicy {
        policy,
        credential,
        signature,
        date: signed_at,
        security_token: credentials.session_token().map(str::to_string),
    }
}

/// `kSigning = HMAC(HMAC(HMAC(HMAC("AWS4" + secret, date), region), service), "aws4_request")`,
/// with each link of the chain wiped as soon as the next exists.
fn signing_key(secret: &str, date: &str, region: &str, service: &str) -> Zeroizing<Vec<u8>> {
    let mut seed = Zeroizing::new(Vec::with_capacity(4 + secret.len()));
    seed.extend_from_slice(b"AWS4");
    seed.extend_from_slice(secret.as_bytes());

    let k_date = Zeroizing::new(hmac(&seed, date.as_bytes()));
    let k_region = Zeroizing::new(hmac(&k_date, region.as_bytes()));
    let k_service = Zeroizing::new(hmac(&k_region, service.as_bytes()));
    Zeroizing::new(hmac(&k_service, b"aws4_request"))
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts a key of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Lowercase hex, which is what every hash in SigV4 is written as.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Lowercase hex SHA-256 of a body.
pub fn sha256_hex(body: &[u8]) -> String {
    hex(&Sha256::digest(body))
}

/// Trim, and collapse runs of spaces into one, as the canonical form requires.
fn canonical_header_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Percent-encode with the RFC 3986 unreserved set, the only encoding SigV4 accepts.
///
/// `A-Z a-z 0-9 - _ . ~` pass through; everything else, byte by byte of its UTF-8 form, becomes
/// `%XX` in uppercase. With `keep_slash` the `/` passes through too, for paths.
pub fn uri_encode(input: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if unreserved || (keep_slash && byte == b'/') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Encode a decoded path for the wire, segment by segment, keeping the `/` between segments.
///
/// This is how an S3 object key becomes a URL path: `reports/2026 Q1.pdf` becomes
/// `/reports/2026%20Q1.pdf`, and a key that contains `%` or `?` stays a key instead of becoming
/// an escape or a query string.
pub fn encode_path(decoded: &str) -> String {
    let encoded = decoded
        .split('/')
        .map(|segment| uri_encode(segment, false))
        .collect::<Vec<_>>()
        .join("/");
    if encoded.starts_with('/') {
        encoded
    } else {
        format!("/{encoded}")
    }
}

/// The canonical query string: each key and value encoded, sorted by key then value, joined
/// with `&`. A key with an empty value still gets its `=`.
pub fn canonical_query(params: &[(&str, &str)]) -> String {
    let mut encoded: Vec<(String, String)> = params
        .iter()
        .map(|(k, v)| (uri_encode(k, false), uri_encode(v, false)))
        .collect();
    encoded.sort();
    encoded
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// `(YYYYMMDDTHHMMSSZ, YYYYMMDD)` for a Unix time, in UTC.
///
/// Done by hand (Howard Hinnant's days-to-civil) rather than by adding a date crate for one
/// format string.
pub fn amz_date(unix_time: u64) -> (String, String) {
    let days = (unix_time / 86_400) as i64;
    let secs = unix_time % 86_400;
    let (year, month, day) = civil_from_days(days);
    let date = format!("{year:04}{month:02}{day:02}");
    let stamp = format!(
        "{date}T{:02}{:02}{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    );
    (stamp, date)
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_presigned_url_matches_the_example_in_the_s3_documentation() {
        // "Authenticating Requests: Using Query Parameters (AWS Signature Version 4)", example GET.
        let credentials = Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", None);
        let query = presign(
            &credentials,
            &PresignInput {
                method: "GET",
                host: "examplebucket.s3.amazonaws.com",
                path: "/test.txt",
                query: &[],
                region: "us-east-1",
                service: "s3",
                unix_time: 1_369_353_600, // 2013-05-24T00:00:00Z
                expires_in: 86_400,
            },
        );

        assert!(
            query.ends_with("X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"),
            "{query}"
        );
    }

    #[test]
    fn a_post_policy_carries_its_own_signing_conditions() {
        let credentials = Credentials::new("AKID", "secret", Some("token".into()));
        let signed = post_policy(&credentials, "us-east-2", 1_794_925_820, 1_794_929_420, vec![serde_json::json!({ "bucket": "b" })]);

        let document: serde_json::Value =
            serde_json::from_slice(
                &crate::encoding::from_base64url(signed.policy.replace('+', "-").replace('/', "_").trim_end_matches('=')).unwrap(),
            )
            .unwrap();
        assert_eq!(document["expiration"], "2026-11-17T15:30:20Z");
        let conditions = document["conditions"].to_string();
        assert!(conditions.contains("x-amz-security-token") && conditions.contains(&signed.credential));
        assert_eq!(signed.signature.len(), 64);
    }

    use super::*;

    /// The credentials every case in AWS's SigV4 test suite is signed with.
    fn suite_credentials() -> Credentials {
        Credentials::new(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            None,
        )
    }

    /// 20150830T123600Z, the suite's fixed signing time.
    const SUITE_TIME: u64 = 1_440_938_160;

    fn suite_input<'a>(
        method: &'a str,
        query: &'a [(&'a str, &'a str)],
        headers: &'a [(&'a str, &'a str)],
        payload: &'a str,
    ) -> SigningInput<'a> {
        SigningInput {
            method,
            host: "example.amazonaws.com",
            path: "/",
            query,
            headers,
            payload_sha256: payload,
            region: "us-east-1",
            service: "service",
            unix_time: SUITE_TIME,
            double_encode_path: true,
        }
    }

    fn authorization(signed: &Signed) -> &str {
        &signed
            .headers
            .iter()
            .find(|(n, _)| n == "authorization")
            .unwrap()
            .1
    }

    #[test]
    fn the_signing_time_formats_as_aws_writes_it() {
        assert_eq!(
            amz_date(SUITE_TIME),
            ("20150830T123600Z".into(), "20150830".into())
        );
        assert_eq!(amz_date(1_369_353_600).0, "20130524T000000Z");
        assert_eq!(amz_date(951_782_400).1, "20000229");
    }

    #[test]
    fn matches_the_suite_for_get_vanilla() {
        let signed = sign(
            &suite_credentials(),
            &suite_input("GET", &[], &[], EMPTY_SHA256),
        );
        assert_eq!(
            authorization(&signed),
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
             SignedHeaders=host;x-amz-date, \
             Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    #[test]
    fn matches_the_suite_for_get_vanilla_query_order_key_case() {
        let query = [("Param2", "value2"), ("Param1", "value1")];
        let signed = sign(
            &suite_credentials(),
            &suite_input("GET", &query, &[], EMPTY_SHA256),
        );
        assert_eq!(signed.query, "Param1=value1&Param2=value2");
        assert_eq!(
            signed.signature,
            "b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500"
        );
    }

    #[test]
    fn matches_the_suite_for_post_x_www_form_urlencoded() {
        let body = b"Param1=value1";
        let payload = sha256_hex(body);
        let headers = [("Content-Type", "application/x-www-form-urlencoded")];
        let signed = sign(
            &suite_credentials(),
            &suite_input("POST", &[], &headers, &payload),
        );
        assert_eq!(
            authorization(&signed),
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
             SignedHeaders=content-type;host;x-amz-date, \
             Signature=ff11897932ad3f4e8b18135d722051e5ac45fc38421b1da7b9d196a0fe09473a"
        );
    }

    /// The examples in the S3 documentation ("Signature Calculations for the Authorization
    /// Header"), all signed on 2013-05-24 for `examplebucket`.
    fn s3_doc_credentials() -> Credentials {
        Credentials::new(
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            None,
        )
    }

    fn s3_doc_input<'a>(
        method: &'a str,
        path: &'a str,
        query: &'a [(&'a str, &'a str)],
        headers: &'a [(&'a str, &'a str)],
        payload: &'a str,
    ) -> SigningInput<'a> {
        SigningInput {
            method,
            host: "examplebucket.s3.amazonaws.com",
            path,
            query,
            headers,
            payload_sha256: payload,
            region: "us-east-1",
            service: "s3",
            unix_time: 1_369_353_600,
            double_encode_path: false,
        }
    }

    #[test]
    fn matches_the_s3_documentation_for_get_object() {
        let headers = [
            ("Range", "bytes=0-9"),
            ("x-amz-content-sha256", EMPTY_SHA256),
        ];
        let path = encode_path("test.txt");
        let signed = sign(
            &s3_doc_credentials(),
            &s3_doc_input("GET", &path, &[], &headers, EMPTY_SHA256),
        );
        assert_eq!(
            authorization(&signed),
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn matches_the_s3_documentation_for_put_object_with_an_encoded_key() {
        let body = b"Welcome to Amazon S3.";
        let payload = sha256_hex(body);
        assert_eq!(
            payload,
            "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
        let headers = [
            ("Date", "Fri, 24 May 2013 00:00:00 GMT"),
            ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
            ("x-amz-content-sha256", payload.as_str()),
        ];
        let path = encode_path("test$file.text");
        assert_eq!(path, "/test%24file.text");
        let signed = sign(
            &s3_doc_credentials(),
            &s3_doc_input("PUT", &path, &[], &headers, &payload),
        );
        assert_eq!(
            signed.signature,
            "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
    }

    #[test]
    fn matches_the_s3_documentation_for_list_objects() {
        let query = [("max-keys", "2"), ("prefix", "J")];
        let headers = [("x-amz-content-sha256", EMPTY_SHA256)];
        let signed = sign(
            &s3_doc_credentials(),
            &s3_doc_input("GET", "/", &query, &headers, EMPTY_SHA256),
        );
        assert_eq!(signed.query, "max-keys=2&prefix=J");
        assert_eq!(
            signed.signature,
            "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
        );
    }

    #[test]
    fn a_session_token_is_signed_and_sent() {
        let credentials = suite_credentials();
        let credentials = Credentials::new(
            credentials.access_key_id(),
            credentials.secret_access_key(),
            Some("TOKEN".into()),
        );
        let signed = sign(&credentials, &suite_input("GET", &[], &[], EMPTY_SHA256));
        assert!(signed
            .headers
            .iter()
            .any(|(n, v)| n == "x-amz-security-token" && v == "TOKEN"));
        assert!(
            authorization(&signed).contains("SignedHeaders=host;x-amz-date;x-amz-security-token,")
        );
    }

    #[test]
    fn s3_paths_are_encoded_once_and_other_services_twice() {
        let path = encode_path("a b/ç%");
        assert_eq!(path, "/a%20b/%C3%A7%25");

        let mut input = s3_doc_input("GET", &path, &[], &[], EMPTY_SHA256);
        let once = sign(&s3_doc_credentials(), &input);
        input.double_encode_path = true;
        let twice = sign(&s3_doc_credentials(), &input);
        assert_ne!(once.signature, twice.signature);
        assert_eq!(uri_encode(&path, true), "/a%2520b/%25C3%25A7%2525");
    }

    #[test]
    fn query_values_use_percent_twenty_and_sort_after_encoding() {
        assert_eq!(
            canonical_query(&[("b", "x y"), ("a", "1+1=2"), ("a", "0")]),
            "a=0&a=1%2B1%3D2&b=x%20y"
        );
    }

    #[test]
    fn credentials_never_print_their_secrets() {
        let credentials =
            Credentials::new("ASIAEXAMPLE", "SECRETSECRET", Some("TOKENTOKEN".into()));
        let printed = format!("{credentials:?}");
        assert!(printed.contains("ASIA"));
        assert!(!printed.contains("EXAMPLE"));
        assert!(!printed.contains("SECRETSECRET"));
        assert!(!printed.contains("TOKENTOKEN"));
    }

    #[test]
    fn expiry_is_checked_with_a_margin() {
        let credentials = suite_credentials().with_expiry(1000);
        assert!(!credentials.is_expired(900, 60));
        assert!(credentials.is_expired(950, 60));
        assert!(!suite_credentials().is_expired(u64::MAX, 60));
    }
}
