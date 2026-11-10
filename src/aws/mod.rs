//! Talking to AWS: the one exception to "nothing leaves the machine", and only when asked.
//!
//! An AWS Parameter Store vault keeps its records as SecureString parameters in somebody's own
//! AWS account (and, optionally, its attachments in an S3 bucket encrypted with KMS). For that,
//! Keyhold has to reach AWS — but only for vaults configured that way, only with credentials the
//! person obtained by logging in through IAM Identity Center, and only to the endpoints of the
//! services involved. A vault that lives in the local file never causes a single request here.
//!
//! This module is the whole client layer, and it is deliberately small:
//!
//! - [`sigv4`] signs requests. Written by hand instead of pulling in the AWS SDK, which would
//!   bring a C toolchain (aws-lc) and several hundred crates into a password manager's build.
//! - [`sso`] is the IAM Identity Center device login: register a client, show the person a code
//!   and a link, poll until they approve it, then trade the token for an account's role
//!   credentials. No AWS CLI, no `~/.aws` files read or written.
//! - [`ssm`] is Parameter Store: read, write, version history and tags.
//! - [`s3`] is the handful of object operations attachments need.
//! - [`error`] folds AWS's many error spellings into the few the interface can explain.
//!
//! On top of the clients, the vault itself:
//!
//! - [`mapping`] is how a record becomes a parameter and back: names, folders, the value format,
//!   title and notes in the description, marks in tags. Pure, and tested on its own.
//! - [`config`] is what is remembered between runs (logins, connections, favourites), sealed.
//! - [`state`] is what is held in memory while the vault is open (tokens, credentials, caches),
//!   and dropped when it locks.
//!
//! The HTTP routes that use all of this are in `http::aws`.
//!
//! **Secrets stay wrapped.** Credentials, tokens, client secrets, device codes and every
//! decrypted value are held in types that zeroize on drop and print as `…`. Response bodies are
//! wiped after parsing. That is best effort — the HTTP stack and TLS buffers have their own copies
//! that this code cannot reach — but it keeps the long-lived copies, the ones that sit in a
//! struct for the length of a session, out of reach of a heap dump and out of every log line.
//!
//! **Every endpoint is a field.** Each client is built with the real AWS endpoint for its region
//! and can be pointed elsewhere with the crate-private `with_endpoint`, which is how the tests run
//! against a fake server on localhost without a single request reaching AWS. Only the tests set
//! one (`AwsState::set_endpoints` exists only under `cfg(test)`), and outside the tests the HTTP
//! client refuses anything that is not HTTPS, so a production build has no way to aim a signed
//! request at a plain-text endpoint.

pub mod config;
pub mod error;
pub mod mapping;
pub mod s3;
pub mod sigv4;
pub mod ssm;
pub mod sso;
pub mod state;

pub use error::{AwsError, AwsResult};
pub use s3::{ObjectSummary, PutObjectOutput, S3Client, S3Object};
pub use sigv4::Credentials;
pub use ssm::{Parameter, ParameterHistoryEntry, ParameterMetadata, PutParameter, SsmClient, Tag};
pub use sso::{Account, ClientRegistration, DeviceLogin, Poll, Role, SsoClient, SsoToken};

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// How long to wait for a TCP and TLS handshake before calling the network down.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a whole request may take, body included. Generous, for large S3 objects on a slow
/// connection; Parameter Store answers in well under a second.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// A string that is a secret: a decrypted parameter value, a token.
///
/// Zeroized on drop and never printed. Read it with [`SecretString::expose`], which is greppable
/// on purpose.
#[derive(Clone, Default, PartialEq, Eq, Zeroize, ZeroizeOnDrop, serde::Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretString(…)")
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// Bytes that are a secret: the body of an S3 object holding an attachment.
#[derive(Clone, Default, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct SecretBytes(Vec<u8>);

impl SecretBytes {
    pub fn new(value: Vec<u8>) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretBytes({} bytes)", self.0.len())
    }
}

/// The HTTP client every AWS client in this module is built on.
///
/// Redirects are **off**: a signed request must not be replayed to a host it was not signed for,
/// and S3 answers a request to the wrong region with a 301 that we want to read, not follow.
/// TLS is rustls on the `ring` provider with compiled-in roots, so nothing about it depends on
/// the system's OpenSSL or certificate store.
///
/// **No proxy, and HTTPS only.** A proxy named in the environment (`HTTPS_PROXY`, `ALL_PROXY`)
/// would see every request's host and timing, and a misconfigured one could terminate TLS; AWS's
/// endpoints are reached directly or not at all. Outside the tests, which talk to a fake on
/// `http://127.0.0.1`, the client refuses any URL that is not `https://`, so an endpoint override
/// can never send a signed request or a decrypted value in the clear.
pub fn http_client() -> AwsResult<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .https_only(!cfg!(test))
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .user_agent(concat!("keyhold/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(AwsError::from)
}

/// Seconds since the Unix epoch.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Refuse a region that could not be one.
///
/// The region goes into hostnames (`ssm.{region}.amazonaws.com`), so anything beyond lowercase
/// letters, digits and hyphens could point a signed request at somebody else's host.
pub fn validate_region(region: &str) -> AwsResult<()> {
    let plausible = !region.is_empty()
        && region.len() <= 32
        && region
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if plausible {
        Ok(())
    } else {
        Err(AwsError::Invalid(format!(
            "'{region}' is not an AWS region"
        )))
    }
}

/// Strip a trailing slash so endpoints concatenate cleanly with paths.
pub(crate) fn trim_endpoint(endpoint: &str) -> String {
    endpoint.trim_end_matches('/').to_string()
}

/// The `Host` header a URL will be sent with: the port is included only when it is not the
/// scheme's default, which is exactly what the HTTP client does and therefore what must be signed.
pub(crate) fn host_header(url: &reqwest::Url) -> AwsResult<String> {
    let host = url
        .host_str()
        .ok_or_else(|| AwsError::Invalid(format!("endpoint has no host: {url}")))?;
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

/// A response read to the end. The body is wiped when this is dropped.
pub(crate) struct Response {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: Zeroizing<Vec<u8>>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

pub(crate) async fn send(request: reqwest::RequestBuilder) -> AwsResult<Response> {
    let response = request.send().await?;
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = Zeroizing::new(Vec::from(response.bytes().await?));
    Ok(Response {
        status,
        headers,
        body,
    })
}

/// Like [`send`], but a successful body larger than `limit` bytes is refused instead of read.
///
/// The `Content-Length` is checked before a single byte of the body is taken, and the body is
/// then read chunk by chunk under the same cap, for a server that sends no length or lies about
/// it. Error bodies (small XML documents) are read as usual.
pub(crate) async fn send_capped(
    request: reqwest::RequestBuilder,
    limit: u64,
) -> AwsResult<Response> {
    let mut response = request.send().await?;
    let status = response.status().as_u16();
    let headers = response.headers().clone();

    if !(200..300).contains(&status) {
        let body = Zeroizing::new(Vec::from(response.bytes().await?));
        return Ok(Response { status, headers, body });
    }

    if let Some(size) = response.content_length().filter(|size| *size > limit) {
        return Err(AwsError::TooLarge { size, limit });
    }

    // Reserved up front so the buffer is not reallocated (and the old copies left unwiped) as
    // the chunks arrive.
    let mut body = Zeroizing::new(Vec::with_capacity(
        response.content_length().unwrap_or(0).min(limit) as usize,
    ));
    while let Some(chunk) = response.chunk().await? {
        let size = (body.len() + chunk.len()) as u64;
        if size > limit {
            return Err(AwsError::TooLarge { size, limit });
        }
        body.extend_from_slice(&chunk);
    }

    Ok(Response { status, headers, body })
}

/// A fake AWS for the tests: a local HTTP server that records every request and answers each
/// with the next canned response, in order.
#[cfg(test)]
pub(crate) mod fake {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::extract::{Request, State};
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum::response::Response;

    #[derive(Debug, Clone)]
    pub struct Recorded {
        pub method: String,
        /// Path and query, exactly as received.
        pub uri: String,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
    }

    impl Recorded {
        pub fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }

        pub fn json(&self) -> serde_json::Value {
            serde_json::from_slice(&self.body).expect("request body is JSON")
        }
    }

    pub struct Canned {
        pub status: u16,
        pub headers: Vec<(&'static str, String)>,
        pub body: Vec<u8>,
    }

    impl Canned {
        pub fn json(status: u16, body: serde_json::Value) -> Self {
            Self {
                status,
                headers: vec![("content-type", "application/x-amz-json-1.1".into())],
                body: body.to_string().into_bytes(),
            }
        }

        pub fn xml(status: u16, body: &str) -> Self {
            Self {
                status,
                headers: vec![("content-type", "application/xml".into())],
                body: body.as_bytes().to_vec(),
            }
        }

        pub fn empty(status: u16) -> Self {
            Self {
                status,
                headers: vec![],
                body: vec![],
            }
        }

        pub fn with_header(mut self, name: &'static str, value: &str) -> Self {
            self.headers.push((name, value.to_string()));
            self
        }
    }

    /// Recompute the signature from what the server actually received — host and port, encoded
    /// path and query, body — and check it is the one the client sent. This is what AWS does on
    /// its side, so a mismatch here is a `SignatureDoesNotMatch` in production.
    pub fn assert_signature_verifies(
        request: &Recorded,
        credentials: &crate::aws::Credentials,
        region: &str,
        service: &str,
    ) {
        use crate::aws::sigv4::{sha256_hex, sign, SigningInput};

        let authorization = request.header("authorization").expect("signed");
        let signed_headers = authorization
            .split("SignedHeaders=")
            .nth(1)
            .and_then(|rest| rest.split(',').next())
            .expect("SignedHeaders");
        let headers: Vec<(&str, &str)> = signed_headers
            .split(';')
            .filter(|h| !matches!(*h, "host" | "x-amz-date" | "x-amz-security-token"))
            .map(|h| (h, request.header(h).expect("signed header was sent")))
            .collect();

        let (path, query) = request.uri.split_once('?').unwrap_or((&request.uri, ""));
        let decoded: Vec<(String, String)> = query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|pair| {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                (percent_decode(k), percent_decode(v))
            })
            .collect();
        let pairs: Vec<(&str, &str)> = decoded
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let payload = sha256_hex(&request.body);
        let signed = sign(
            credentials,
            &SigningInput {
                method: &request.method,
                host: request.header("host").expect("host"),
                path,
                query: &pairs,
                headers: &headers,
                payload_sha256: &payload,
                region,
                service,
                unix_time: parse_amz_date(request.header("x-amz-date").expect("x-amz-date")),
                double_encode_path: service != "s3",
            },
        );
        let expected = &signed
            .headers
            .iter()
            .find(|(n, _)| n == "authorization")
            .unwrap()
            .1;
        assert_eq!(authorization, expected, "signature does not verify");
    }

    fn percent_decode(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    /// `YYYYMMDDTHHMMSSZ` back to Unix seconds (days-from-civil, the inverse of the signer's).
    fn parse_amz_date(stamp: &str) -> u64 {
        let n = |range: std::ops::Range<usize>| stamp[range].parse::<i64>().unwrap();
        let (y, m, d) = (n(0..4), n(4..6), n(6..8));
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        (days * 86_400 + n(9..11) * 3600 + n(11..13) * 60 + n(13..15)) as u64
    }

    #[derive(Clone)]
    struct Shared {
        responses: Arc<Mutex<VecDeque<Canned>>>,
        recorded: Arc<Mutex<Vec<Recorded>>>,
    }

    pub struct FakeAws {
        pub base: String,
        recorded: Arc<Mutex<Vec<Recorded>>>,
    }

    impl FakeAws {
        pub async fn start(responses: Vec<Canned>) -> Self {
            let shared = Shared {
                responses: Arc::new(Mutex::new(responses.into())),
                recorded: Arc::new(Mutex::new(Vec::new())),
            };
            let recorded = shared.recorded.clone();
            let app = axum::Router::new().fallback(handle).with_state(shared);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            Self {
                base: format!("http://{address}"),
                recorded,
            }
        }

        pub fn requests(&self) -> Vec<Recorded> {
            self.recorded.lock().unwrap().clone()
        }
    }

    async fn handle(State(shared): State<Shared>, request: Request) -> Response {
        let (parts, body) = request.into_parts();
        let body = axum::body::to_bytes(body, usize::MAX)
            .await
            .unwrap()
            .to_vec();
        shared.recorded.lock().unwrap().push(Recorded {
            method: parts.method.to_string(),
            uri: parts
                .uri
                .path_and_query()
                .map(|p| p.to_string())
                .unwrap_or_default(),
            headers: parts
                .headers
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect(),
            body,
        });

        let canned = shared.responses.lock().unwrap().pop_front();
        let canned = canned.unwrap_or_else(|| {
            Canned::json(
                500,
                serde_json::json!({
                    "__type": "FakeExhausted", "message": "no canned response left"
                }),
            )
        });
        let mut response = Response::new(Body::from(canned.body));
        *response.status_mut() = StatusCode::from_u16(canned.status).unwrap();
        for (name, value) in canned.headers {
            response.headers_mut().insert(
                HeaderName::from_static(name),
                HeaderValue::from_str(&value).unwrap(),
            );
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plausible_regions_are_accepted() {
        assert!(validate_region("sa-east-1").is_ok());
        assert!(validate_region("us-gov-west-1").is_ok());
        assert!(validate_region("").is_err());
        assert!(validate_region("evil.com/").is_err());
        assert!(validate_region("US-EAST-1").is_err());
    }

    #[test]
    fn secrets_print_as_nothing() {
        assert_eq!(
            format!("{:?}", SecretString::new("hunter2")),
            "SecretString(…)"
        );
        assert_eq!(
            format!("{:?}", SecretBytes::new(vec![1, 2, 3])),
            "SecretBytes(3 bytes)"
        );
    }

    #[test]
    fn the_host_header_carries_a_port_only_when_it_is_not_the_default() {
        let url = reqwest::Url::parse("https://ssm.sa-east-1.amazonaws.com/").unwrap();
        assert_eq!(host_header(&url).unwrap(), "ssm.sa-east-1.amazonaws.com");
        let url = reqwest::Url::parse("http://127.0.0.1:4566/").unwrap();
        assert_eq!(host_header(&url).unwrap(), "127.0.0.1:4566");
    }
}
