//! S3: the handful of object operations an AWS vault's attachments need.
//!
//! Parameter Store caps a value at 8 KB, which is plenty for a password and nowhere near enough
//! for a PDF. Attachments therefore go to a bucket the person chooses, each object written with
//! `x-amz-server-side-encryption: aws:kms` so that S3 never holds it unencrypted at rest, under
//! the vault's KMS key when it names one.
//!
//! Requests go to the virtual-hosted endpoint (`https://{bucket}.s3.{region}.amazonaws.com`),
//! which is what AWS wants every new client to use. A bucket with dots in its name is the one
//! exception: the wildcard certificate `*.s3.{region}.amazonaws.com` does not cover
//! `my.bucket.s3…`, so those go path-style to `https://s3.{region}.amazonaws.com/{bucket}`. An
//! overridden endpoint (a fake server in tests, an S3-compatible store) is always path-style.
//!
//! **Object keys are encoded segment by segment and signed exactly as sent** — S3 is the one
//! service where SigV4 does not double-encode the path — so a key with spaces, `+`, `%`, `?` or
//! accents arrives as the same key. Keys with `.` or `..` segments are refused, because URL
//! parsing would quietly resolve them into a different key.
//!
//! The response bodies Keyhold needs from S3 are tiny XML documents (a listing, an error), so
//! they are read with a few lines of string search instead of an XML crate.

use super::error::{classify, AwsError, AwsResult};
use super::sigv4::{self, Credentials, SigningInput};
use super::{
    host_header, http_client, now_unix, send, send_capped, trim_endpoint, validate_region, Response,
    SecretBytes,
};

const EXPIRY_MARGIN_SECONDS: u64 = 30;

/// A safety stop for pagination: a million objects at a thousand per page.
const MAX_PAGES: usize = 1_000;

/// What `PutObject` reports back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PutObjectOutput {
    pub etag: Option<String>,
    /// Present when the bucket has versioning enabled.
    pub version_id: Option<String>,
}

/// An object, read whole.
#[derive(Debug, Clone)]
pub struct S3Object {
    pub body: SecretBytes,
    pub etag: Option<String>,
    pub content_type: Option<String>,
    /// The `Last-Modified` header, as S3 sends it (an HTTP date).
    pub last_modified: Option<String>,
    pub version_id: Option<String>,
}

/// One entry of a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectSummary {
    pub key: String,
    pub size: u64,
    /// ISO 8601, as S3 sends it.
    pub last_modified: String,
    pub etag: Option<String>,
}

/// An S3 client for one region and one set of credentials.
pub struct S3Client {
    http: reqwest::Client,
    region: String,
    /// When set, every request goes path-style to this endpoint instead of to AWS.
    endpoint: Option<String>,
    credentials: Credentials,
    /// The account the bucket must belong to, sent as `x-amz-expected-bucket-owner`.
    expected_owner: Option<String>,
}

impl std::fmt::Debug for S3Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Client")
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .field("credentials", &self.credentials)
            .field("expected_owner", &self.expected_owner)
            .finish()
    }
}

impl S3Client {
    pub fn new(region: &str, credentials: Credentials) -> AwsResult<Self> {
        validate_region(region)?;
        Ok(Self {
            http: http_client()?,
            region: region.to_string(),
            endpoint: None,
            credentials,
            expected_owner: None,
        })
    }

    /// Send every request path-style to `endpoint` instead.
    ///
    /// Crate-private on purpose: only `AwsState` calls it, with an endpoint only the tests can set,
    /// and outside the tests the HTTP client refuses anything that is not HTTPS.
    pub(crate) fn with_endpoint(mut self, endpoint: &str) -> Self {
        self.endpoint = Some(trim_endpoint(endpoint));
        self
    }

    /// Refuse to talk to a bucket that is not owned by `account_id`.
    ///
    /// Bucket names are global. If the vault's bucket is ever deleted and somebody else creates
    /// one with the same name, every attachment written afterwards would land in *their* account,
    /// encrypted with a key they choose to honour. With this header (signed, like every other)
    /// S3 answers 403 instead of serving or accepting anything from a bucket of another account.
    pub fn with_expected_owner(mut self, account_id: &str) -> Self {
        self.expected_owner = Some(account_id.to_string());
        self
    }

    pub fn set_credentials(&mut self, credentials: Credentials) {
        self.credentials = credentials;
    }

    pub fn region(&self) -> &str {
        &self.region
    }

    /// Check that the bucket exists, is in this region, and these credentials may use it.
    pub async fn head_bucket(&self, bucket: &str) -> AwsResult<()> {
        let response = self
            .request("HEAD", bucket, None, &[], &[], Vec::new())
            .await?;
        self.check(&response)
    }

    /// Write an object, encrypted at rest with KMS (`kms_key_id` or the account's
    /// `aws/s3` key).
    pub async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        body: &[u8],
        kms_key_id: Option<&str>,
        content_type: Option<&str>,
    ) -> AwsResult<PutObjectOutput> {
        let mut headers = vec![("x-amz-server-side-encryption", "aws:kms")];
        if let Some(kms_key_id) = kms_key_id {
            headers.push(("x-amz-server-side-encryption-aws-kms-key-id", kms_key_id));
        }
        if let Some(content_type) = content_type {
            headers.push(("content-type", content_type));
        }
        let response = self
            .request("PUT", bucket, Some(key), &[], &headers, body.to_vec())
            .await?;
        self.check(&response)?;
        Ok(PutObjectOutput {
            etag: response.header("etag").map(str::to_string),
            version_id: response.header("x-amz-version-id").map(str::to_string),
        })
    }

    /// Read an object whole, refusing one larger than `max_bytes` before its body is read
    /// ([`AwsError::TooLarge`]).
    pub async fn get_object(&self, bucket: &str, key: &str, max_bytes: u64) -> AwsResult<S3Object> {
        let mut response = self
            .request_capped("GET", bucket, Some(key), &[], &[], Vec::new(), Some(max_bytes))
            .await?;
        self.check(&response)?;
        let header = |name: &str| response.header(name).map(str::to_string);
        let (etag, content_type, last_modified, version_id) = (
            header("etag"),
            header("content-type"),
            header("last-modified"),
            header("x-amz-version-id"),
        );
        Ok(S3Object {
            body: SecretBytes::new(std::mem::take(&mut *response.body)),
            etag,
            content_type,
            last_modified,
            version_id,
        })
    }

    /// Delete an object. S3 answers the same whether or not it existed.
    pub async fn delete_object(&self, bucket: &str, key: &str) -> AwsResult<()> {
        let response = self
            .request("DELETE", bucket, Some(key), &[], &[], Vec::new())
            .await?;
        self.check(&response)
    }

    /// Every object whose key starts with `prefix`, following continuation tokens to the end.
    pub async fn list_objects_v2(
        &self,
        bucket: &str,
        prefix: &str,
    ) -> AwsResult<Vec<ObjectSummary>> {
        let mut all = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut query = vec![("list-type", "2"), ("prefix", prefix)];
            if let Some(token) = &token {
                query.push(("continuation-token", token.as_str()));
            }
            let response = self
                .request("GET", bucket, None, &query, &[], Vec::new())
                .await?;
            self.check(&response)?;

            let xml = std::str::from_utf8(&response.body).map_err(|_| AwsError::Other {
                code: "MalformedResponse".into(),
                message: "ListObjectsV2 returned a body that is not UTF-8".into(),
                status: response.status,
            })?;
            for block in xml_blocks(xml, "Contents") {
                all.push(ObjectSummary {
                    key: xml_unescape(xml_tag(block, "Key").unwrap_or_default()),
                    size: xml_tag(block, "Size")
                        .and_then(|s| s.trim().parse().ok())
                        .unwrap_or(0),
                    last_modified: xml_tag(block, "LastModified")
                        .unwrap_or_default()
                        .to_string(),
                    etag: xml_tag(block, "ETag").map(xml_unescape),
                });
            }

            let truncated = xml_tag(xml, "IsTruncated").map(str::trim) == Some("true");
            let next = xml_tag(xml, "NextContinuationToken").map(xml_unescape);
            match next {
                Some(next) if truncated && !next.is_empty() && Some(&next) != token.as_ref() => {
                    token = Some(next)
                }
                _ => return Ok(all),
            }
        }
        Err(AwsError::Other {
            code: "TooManyPages".into(),
            message: format!("stopped after {MAX_PAGES} pages"),
            status: 200,
        })
    }

    /// Turn a non-2xx response into an error.
    fn check(&self, response: &Response) -> AwsResult<()> {
        if response.is_success() {
            return Ok(());
        }

        // A bucket in another region answers 301 (with an empty body to a HEAD) and names its
        // region in a header. That is a configuration mistake to explain, not a redirect to follow.
        if let Some(region) = response.header("x-amz-bucket-region") {
            if (300..400).contains(&response.status) || region != self.region {
                return Err(AwsError::Invalid(format!(
                    "the bucket is in region {region}, not {}",
                    self.region
                )));
            }
        }

        let body = std::str::from_utf8(&response.body).unwrap_or("");
        let code = xml_tag(body, "Code").unwrap_or("");
        let message = xml_tag(body, "Message")
            .map(xml_unescape)
            .unwrap_or_default();
        Err(classify(response.status, code, &message))
    }

    /// Build, sign and send one request. The status is not checked here.
    async fn request(
        &self,
        method: &str,
        bucket: &str,
        key: Option<&str>,
        query: &[(&str, &str)],
        headers: &[(&str, &str)],
        body: Vec<u8>,
    ) -> AwsResult<Response> {
        self.request_capped(method, bucket, key, query, headers, body, None).await
    }

    /// [`S3Client::request`], reading at most `cap` bytes of a successful body.
    #[allow(clippy::too_many_arguments)]
    async fn request_capped(
        &self,
        method: &str,
        bucket: &str,
        key: Option<&str>,
        query: &[(&str, &str)],
        headers: &[(&str, &str)],
        body: Vec<u8>,
        cap: Option<u64>,
    ) -> AwsResult<Response> {
        validate_bucket(bucket)?;
        if let Some(key) = key {
            validate_key(key)?;
        }
        let now = now_unix();
        if self.credentials.is_expired(now, EXPIRY_MARGIN_SECONDS) {
            return Err(AwsError::SessionExpired(
                "the temporary AWS credentials have expired".into(),
            ));
        }

        let (origin, path) = self.locate(bucket, key);
        let payload_hash = sigv4::sha256_hex(&body);
        let mut all_headers: Vec<(&str, &str)> = headers.to_vec();
        all_headers.push(("x-amz-content-sha256", &payload_hash));
        // Signed with the rest, so it cannot be stripped on the way without breaking the request.
        if let Some(owner) = self.expected_owner.as_deref() {
            all_headers.push(("x-amz-expected-bucket-owner", owner));
        }

        let probe = reqwest::Url::parse(&format!("{origin}{path}"))
            .map_err(|e| AwsError::Invalid(format!("bad S3 endpoint: {e}")))?;
        let host = host_header(&probe)?;

        let signed = sigv4::sign(
            &self.credentials,
            &SigningInput {
                method,
                host: &host,
                path: &path,
                query,
                headers: &all_headers,
                payload_sha256: &payload_hash,
                region: &self.region,
                service: "s3",
                unix_time: now,
                double_encode_path: false,
            },
        );

        let url = if signed.query.is_empty() {
            format!("{origin}{path}")
        } else {
            format!("{origin}{path}?{}", signed.query)
        };
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| AwsError::Invalid(format!("bad method {method}")))?;
        let mut request = self.http.request(method, url).body(body);
        for (name, value) in all_headers {
            request = request.header(name, value);
        }
        for (name, value) in &signed.headers {
            request = request.header(name.as_str(), value.as_str());
        }
        match cap {
            Some(limit) => send_capped(request, limit).await,
            None => send(request).await,
        }
    }

    /// The origin (`scheme://host[:port]`) and the wire path for a bucket and optional key.
    fn locate(&self, bucket: &str, key: Option<&str>) -> (String, String) {
        let key_path = key.map(sigv4::encode_path).unwrap_or_default();
        match &self.endpoint {
            Some(endpoint) => (endpoint.clone(), format!("/{bucket}{key_path}")),
            None if bucket.contains('.') => (
                format!("https://s3.{}.amazonaws.com", self.region),
                format!("/{bucket}{key_path}"),
            ),
            None => (
                format!("https://{bucket}.s3.{}.amazonaws.com", self.region),
                if key_path.is_empty() {
                    "/".to_string()
                } else {
                    key_path
                },
            ),
        }
    }
}

/// S3's naming rules, enforced before a bucket name goes into a hostname.
fn validate_bucket(bucket: &str) -> AwsResult<()> {
    let bytes = bucket.as_bytes();
    let ok = (3..=63).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'.' || *b == b'-')
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && !bucket.contains("..");
    if ok {
        Ok(())
    } else {
        Err(AwsError::Invalid(format!(
            "'{bucket}' is not a valid bucket name"
        )))
    }
}

fn validate_key(key: &str) -> AwsResult<()> {
    if key.is_empty() || key.len() > 1024 {
        return Err(AwsError::Invalid(
            "an object key must be 1 to 1024 bytes".into(),
        ));
    }
    if key
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(AwsError::Invalid(
            "an object key cannot contain '.' or '..' segments".into(),
        ));
    }
    Ok(())
}

/// The text of the first `<name>…</name>` in `xml`.
fn xml_tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}

/// The inner text of every `<name>…</name>` in `xml`, in order.
fn xml_blocks<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(&open) {
        let start = at + open.len();
        let Some(len) = rest[start..].find(&close) else {
            break;
        };
        out.push(&rest[start..start + len]);
        rest = &rest[start + len + close.len()..];
    }
    out
}

/// Undo the five XML entities and numeric character references.
fn xml_unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at..];
        let Some(end) = after.find(';') else {
            out.push_str(after);
            return out;
        };
        let entity = &after[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => out.push(c),
            None => out.push_str(&after[..=end]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::super::fake::{assert_signature_verifies, Canned, FakeAws};
    use super::*;

    fn credentials() -> Credentials {
        Credentials::new("ASIAFAKE", "fake-secret", Some("fake-session-token".into()))
    }

    async fn client(responses: Vec<Canned>) -> (S3Client, FakeAws) {
        let fake = FakeAws::start(responses).await;
        let client = S3Client::new("sa-east-1", credentials())
            .unwrap()
            .with_endpoint(&fake.base);
        (client, fake)
    }

    #[test]
    fn real_endpoints_are_virtual_hosted_except_for_dotted_buckets() {
        let client = S3Client::new("sa-east-1", credentials()).unwrap();
        assert_eq!(
            client.locate("keyhold-files", Some("a b/c.pdf")),
            (
                "https://keyhold-files.s3.sa-east-1.amazonaws.com".to_string(),
                "/a%20b/c.pdf".to_string()
            )
        );
        assert_eq!(
            client.locate("keyhold-files", None),
            (
                "https://keyhold-files.s3.sa-east-1.amazonaws.com".to_string(),
                "/".to_string()
            )
        );
        assert_eq!(
            client.locate("my.bucket", Some("k")),
            (
                "https://s3.sa-east-1.amazonaws.com".to_string(),
                "/my.bucket/k".to_string()
            )
        );
    }

    #[tokio::test]
    async fn a_put_is_encrypted_with_kms_and_signed_for_s3() {
        let (client, fake) = client(vec![Canned::empty(200)
            .with_header("etag", "\"abc\"")
            .with_header("x-amz-version-id", "v1")])
        .await;

        let output = client
            .put_object(
                "vault-bucket",
                "attachments/my file+1 ç.pdf",
                b"%PDF-1.7",
                Some("alias/keyhold"),
                Some("application/pdf"),
            )
            .await
            .unwrap();
        assert_eq!(output.etag.as_deref(), Some("\"abc\""));
        assert_eq!(output.version_id.as_deref(), Some("v1"));

        let request = &fake.requests()[0];
        assert_eq!(request.method, "PUT");
        assert_eq!(
            request.uri,
            "/vault-bucket/attachments/my%20file%2B1%20%C3%A7.pdf"
        );
        assert_eq!(request.body, b"%PDF-1.7");
        assert_eq!(
            request.header("x-amz-server-side-encryption"),
            Some("aws:kms")
        );
        assert_eq!(
            request.header("x-amz-server-side-encryption-aws-kms-key-id"),
            Some("alias/keyhold")
        );
        assert_eq!(
            request.header("x-amz-content-sha256"),
            Some(sigv4::sha256_hex(b"%PDF-1.7").as_str())
        );
        let authorization = request.header("authorization").unwrap();
        assert!(
            authorization.contains("/sa-east-1/s3/aws4_request, "),
            "{authorization}"
        );
        assert!(authorization.contains(
            "SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date;\
             x-amz-security-token;x-amz-server-side-encryption;\
             x-amz-server-side-encryption-aws-kms-key-id, "
        ));
        assert_signature_verifies(request, &credentials(), "sa-east-1", "s3");
    }

    #[tokio::test]
    async fn a_get_returns_the_body_as_a_secret() {
        let (client, fake) = client(vec![Canned {
            status: 200,
            headers: vec![
                ("etag", "\"e\"".into()),
                ("content-type", "text/plain".into()),
            ],
            body: b"hello".to_vec(),
        }])
        .await;

        let object = client.get_object("vault-bucket", "k", 1024).await.unwrap();
        assert_eq!(object.body.expose(), b"hello");
        assert_eq!(object.etag.as_deref(), Some("\"e\""));
        assert_eq!(object.content_type.as_deref(), Some("text/plain"));
        assert!(!format!("{object:?}").contains("hello"));
        assert_eq!(
            fake.requests()[0].header("x-amz-content-sha256"),
            Some(sigv4::EMPTY_SHA256)
        );
    }

    #[tokio::test]
    async fn a_listing_follows_continuation_tokens_and_decodes_keys() {
        let page1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>vault-bucket</Name><Prefix>att/</Prefix><KeyCount>2</KeyCount><MaxKeys>1000</MaxKeys>
  <IsTruncated>true</IsTruncated>
  <Contents><Key>att/a&amp;b.pdf</Key><LastModified>2026-11-01T10:00:00.000Z</LastModified>
    <ETag>&quot;e1&quot;</ETag><Size>10</Size><StorageClass>STANDARD</StorageClass></Contents>
  <Contents><Key>att/c.txt</Key><LastModified>2026-11-02T10:00:00.000Z</LastModified>
    <ETag>&quot;e2&quot;</ETag><Size>20</Size><StorageClass>STANDARD</StorageClass></Contents>
  <NextContinuationToken>1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=</NextContinuationToken>
</ListBucketResult>"#;
        let page2 = r#"<ListBucketResult><IsTruncated>false</IsTruncated>
  <Contents><Key>att/d.txt</Key><LastModified>2026-11-03T10:00:00.000Z</LastModified><Size>30</Size></Contents>
</ListBucketResult>"#;
        let (client, fake) = client(vec![Canned::xml(200, page1), Canned::xml(200, page2)]).await;

        let objects = client
            .list_objects_v2("vault-bucket", "att/")
            .await
            .unwrap();
        assert_eq!(
            objects.iter().map(|o| o.key.as_str()).collect::<Vec<_>>(),
            ["att/a&b.pdf", "att/c.txt", "att/d.txt"]
        );
        assert_eq!(objects[1].size, 20);
        assert_eq!(objects[0].etag.as_deref(), Some("\"e1\""));
        assert_eq!(objects[2].last_modified, "2026-11-03T10:00:00.000Z");

        let requests = fake.requests();
        assert_eq!(requests[0].uri, "/vault-bucket?list-type=2&prefix=att%2F");
        assert_eq!(
            requests[1].uri,
            "/vault-bucket?continuation-token=1ueGcxLPRx1Tr%2FXYExHnhbYLgveDs2J%2Fwm36Hy4vbOwM%3D\
             &list-type=2&prefix=att%2F"
        );
        for request in &requests {
            assert_signature_verifies(request, &credentials(), "sa-east-1", "s3");
        }
    }

    #[tokio::test]
    async fn s3_error_documents_become_the_matching_variants() {
        let no_bucket = "<Error><Code>NoSuchBucket</Code>\
            <Message>The specified bucket does not exist</Message>\
            <BucketName>vault-bucket</BucketName></Error>";
        let denied = "<?xml version=\"1.0\"?><Error><Code>AccessDenied</Code>\
            <Message>Access Denied</Message><RequestId>X</RequestId></Error>";
        let expired = "<Error><Code>ExpiredToken</Code>\
            <Message>The provided token has expired.</Message></Error>";
        let (client, _fake) = client(vec![
            Canned::xml(404, no_bucket),
            Canned::xml(403, denied),
            Canned::xml(400, expired),
            Canned::xml(
                503,
                "<Error><Code>SlowDown</Code><Message>Reduce</Message></Error>",
            ),
        ])
        .await;

        assert_eq!(
            client.list_objects_v2("vault-bucket", "").await,
            Err(AwsError::NotFound(
                "The specified bucket does not exist".into()
            ))
        );
        assert_eq!(
            client.get_object("vault-bucket", "k", 1024).await.unwrap_err(),
            AwsError::AccessDenied("Access Denied".into())
        );
        assert!(matches!(
            client
                .put_object("vault-bucket", "k", b"x", None, None)
                .await,
            Err(AwsError::SessionExpired(_))
        ));
        assert!(matches!(
            client.delete_object("vault-bucket", "k").await,
            Err(AwsError::Throttled(_))
        ));
    }

    #[tokio::test]
    async fn a_head_without_a_body_is_classified_by_its_status() {
        let (client, fake) = client(vec![
            Canned::empty(200),
            Canned::empty(404),
            Canned::empty(403),
            Canned::empty(301).with_header("x-amz-bucket-region", "us-east-1"),
        ])
        .await;

        client.head_bucket("vault-bucket").await.unwrap();
        assert!(matches!(
            client.head_bucket("vault-bucket").await,
            Err(AwsError::NotFound(_))
        ));
        assert!(matches!(
            client.head_bucket("vault-bucket").await,
            Err(AwsError::AccessDenied(_))
        ));
        assert!(matches!(
            client.head_bucket("vault-bucket").await,
            Err(AwsError::Invalid(m)) if m.contains("us-east-1")
        ));
        assert_eq!(fake.requests()[0].method, "HEAD");
        assert_eq!(fake.requests()[0].uri, "/vault-bucket");
    }

    #[tokio::test]
    async fn bad_names_are_refused_before_anything_is_sent() {
        let (client, fake) = client(vec![]).await;
        assert!(matches!(
            client.head_bucket("Bad_Bucket").await,
            Err(AwsError::Invalid(_))
        ));
        assert!(matches!(
            client.head_bucket("evil.com/x").await,
            Err(AwsError::Invalid(_))
        ));
        assert!(matches!(
            client.get_object("vault-bucket", "a/../b", 1024).await,
            Err(AwsError::Invalid(_))
        ));
        assert!(matches!(
            client.get_object("vault-bucket", "", 1024).await,
            Err(AwsError::Invalid(_))
        ));
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn every_request_names_the_account_that_must_own_the_bucket_and_signs_it() {
        let list = "<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>";
        let fake = FakeAws::start(vec![
            Canned::empty(200),
            Canned::empty(200),
            Canned::empty(200),
            Canned::empty(204),
            Canned::xml(200, list),
        ])
        .await;
        let client = S3Client::new("sa-east-1", credentials())
            .unwrap()
            .with_endpoint(&fake.base)
            .with_expected_owner("123456789012");

        client.head_bucket("vault-bucket").await.unwrap();
        client.put_object("vault-bucket", "k", b"x", None, None).await.unwrap();
        client.get_object("vault-bucket", "k", 10).await.unwrap();
        client.delete_object("vault-bucket", "k").await.unwrap();
        client.list_objects_v2("vault-bucket", "p/").await.unwrap();

        let requests = fake.requests();
        assert_eq!(requests.len(), 5);
        for request in &requests {
            assert_eq!(request.header("x-amz-expected-bucket-owner"), Some("123456789012"));
            let authorization = request.header("authorization").unwrap();
            assert!(
                authorization.contains("x-amz-expected-bucket-owner"),
                "the owner is signed: {authorization}"
            );
            assert_signature_verifies(request, &credentials(), "sa-east-1", "s3");
        }
    }

    #[tokio::test]
    async fn an_object_past_the_limit_is_refused_before_its_body_is_kept() {
        let (client, _fake) = client(vec![
            Canned { status: 200, headers: vec![], body: vec![7u8; 64] },
            Canned { status: 200, headers: vec![], body: vec![7u8; 8] },
        ])
        .await;

        assert_eq!(
            client.get_object("vault-bucket", "k", 16).await.unwrap_err(),
            AwsError::TooLarge { size: 64, limit: 16 }
        );
        assert_eq!(client.get_object("vault-bucket", "k", 16).await.unwrap().body.len(), 8);
    }

    #[test]
    fn xml_entities_are_decoded() {
        assert_eq!(
            xml_unescape("a&amp;b &lt;c&gt; &#233;&#x4E2D; &bogus; &"),
            "a&b <c> é中 &bogus; &"
        );
    }
}
