//! Parameter Store: where an AWS vault's records actually live.
//!
//! Every call is the same shape — a signed `POST /` to `ssm.{region}.amazonaws.com` with
//! `X-Amz-Target: AmazonSSM.<Operation>` and a JSON body — so there is one private `call` and a
//! thin typed method per operation on top of it.
//!
//! Values are always read **with decryption** and written as **SecureString**. A plain `String`
//! parameter would be stored in the clear in somebody's AWS account, and that is not a vault.
//! The KMS key is the account's default (`alias/aws/ssm`) unless the vault names one.
//!
//! What each operation returns is only what AWS returns for it. In particular `GetParameter` and
//! `GetParametersByPath` do not report the KMS key or the description; those come from
//! [`SsmClient::describe_parameters`] or from the history.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use zeroize::Zeroizing;

use super::error::{from_json_response, AwsError, AwsResult};
use super::sigv4::{self, Credentials, SigningInput};
use super::SecretString;
use super::{host_header, http_client, now_unix, send, trim_endpoint, validate_region, Response};

/// `GetParametersByPath` returns at most ten per page; asking for the maximum keeps the number of
/// round trips (and of throttling opportunities) down.
const PAGE_SIZE_BY_PATH: u32 = 10;

/// `GetParameterHistory` and `DescribeParameters` allow fifty.
const PAGE_SIZE_HISTORY: u32 = 50;

/// Refuse to sign with credentials this close to expiry: the request could arrive after it.
const EXPIRY_MARGIN_SECONDS: u64 = 30;

/// A safety stop for pagination. Fifty thousand parameters under one path is far past anything a
/// person's vault holds, and a server that keeps handing out tokens forever should not be able to
/// keep us looping.
const MAX_PAGES: usize = 5_000;

/// One parameter, decrypted.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Parameter {
    pub name: String,
    /// `SecureString`, `String` or `StringList`.
    #[serde(rename = "Type")]
    pub kind: String,
    pub value: SecretString,
    pub version: i64,
    /// Unix seconds.
    #[serde(
        rename = "LastModifiedDate",
        default,
        deserialize_with = "epoch_seconds"
    )]
    pub last_modified: Option<i64>,
    #[serde(rename = "ARN", default)]
    pub arn: Option<String>,
}

/// A parameter as `DescribeParameters` sees it: everything but the value.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ParameterMetadata {
    pub name: String,
    #[serde(rename = "Type")]
    pub kind: String,
    #[serde(default)]
    pub key_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub version: i64,
    #[serde(
        rename = "LastModifiedDate",
        default,
        deserialize_with = "epoch_seconds"
    )]
    pub last_modified: Option<i64>,
    #[serde(default)]
    pub last_modified_user: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
}

/// One version of a parameter, decrypted.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ParameterHistoryEntry {
    pub name: String,
    #[serde(rename = "Type")]
    pub kind: String,
    #[serde(default)]
    pub key_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub value: SecretString,
    pub version: i64,
    #[serde(
        rename = "LastModifiedDate",
        default,
        deserialize_with = "epoch_seconds"
    )]
    pub last_modified: Option<i64>,
    /// The ARN of whoever wrote this version: an assumed-role session, for an SSO user.
    #[serde(default)]
    pub last_modified_user: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Tag {
    pub key: String,
    pub value: String,
}

impl Tag {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// What to write. Always a SecureString.
#[derive(Debug, Clone, Default)]
pub struct PutParameter {
    pub name: String,
    pub value: SecretString,
    /// A KMS key id, ARN or alias. `None` means the account's `alias/aws/ssm`.
    pub key_id: Option<String>,
    pub description: Option<String>,
    /// Replace an existing parameter (creating a new version). When `false`, writing a name that
    /// exists fails with [`AwsError::VersionConflict`].
    pub overwrite: bool,
    /// Only allowed when creating: AWS rejects tags together with `Overwrite`. Use
    /// [`SsmClient::add_tags_to_resource`] to tag an existing parameter.
    pub tags: Vec<Tag>,
    /// Use the Advanced tier, needed for values over 4 KB (and billed by AWS).
    pub advanced: bool,
}

/// The body of `PutParameter`, borrowing the value instead of copying it into a `serde_json::Value`
/// that would not be zeroized.
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PutParameterBody<'a> {
    name: &'a str,
    value: &'a str,
    #[serde(rename = "Type")]
    kind: &'static str,
    overwrite: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    #[serde(skip_serializing_if = "<[Tag]>::is_empty")]
    tags: &'a [Tag],
    #[serde(skip_serializing_if = "Option::is_none")]
    tier: Option<&'static str>,
}

/// A Parameter Store client for one region and one set of credentials.
pub struct SsmClient {
    http: reqwest::Client,
    region: String,
    endpoint: String,
    credentials: Credentials,
}

impl std::fmt::Debug for SsmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SsmClient")
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .field("credentials", &self.credentials)
            .finish()
    }
}

impl SsmClient {
    /// A client for `https://ssm.{region}.amazonaws.com`.
    pub fn new(region: &str, credentials: Credentials) -> AwsResult<Self> {
        validate_region(region)?;
        Ok(Self {
            http: http_client()?,
            region: region.to_string(),
            endpoint: format!("https://ssm.{region}.amazonaws.com"),
            credentials,
        })
    }

    /// Send requests somewhere else: a VPC endpoint, a FIPS endpoint, or a fake server in tests.
    /// The region is still the one signed.
    pub fn with_endpoint(mut self, endpoint: &str) -> Self {
        self.endpoint = trim_endpoint(endpoint);
        self
    }

    /// Swap in fresh credentials, after the old ones expired and the role was assumed again.
    pub fn set_credentials(&mut self, credentials: Credentials) {
        self.credentials = credentials;
    }

    pub fn region(&self) -> &str {
        &self.region
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Every parameter under `path`, decrypted, following `NextToken` to the end.
    pub async fn get_parameters_by_path(
        &self,
        path: &str,
        recursive: bool,
    ) -> AwsResult<Vec<Parameter>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Page {
            #[serde(default)]
            parameters: Vec<Parameter>,
            #[serde(default)]
            next_token: Option<String>,
        }

        let mut all = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut body = json!({
                "Path": path,
                "Recursive": recursive,
                "WithDecryption": true,
                "MaxResults": PAGE_SIZE_BY_PATH,
            });
            if let Some(token) = &token {
                body["NextToken"] = json!(token);
            }
            let page: Page = self.call_json("GetParametersByPath", &body).await?;
            all.extend(page.parameters);
            match next_token(page.next_token, &token)? {
                Some(next) => token = Some(next),
                None => return Ok(all),
            }
        }
        Err(too_many_pages())
    }

    /// One parameter, decrypted. `name` may carry a `:version` or `:label` selector.
    pub async fn get_parameter(&self, name: &str) -> AwsResult<Parameter> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Output {
            parameter: Parameter,
        }
        let body = json!({ "Name": name, "WithDecryption": true });
        let output: Output = self.call_json("GetParameter", &body).await?;
        Ok(output.parameter)
    }

    /// Metadata (KMS key, description, last writer) for every parameter under `path`.
    pub async fn describe_parameters(
        &self,
        path: &str,
        recursive: bool,
    ) -> AwsResult<Vec<ParameterMetadata>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Page {
            #[serde(default)]
            parameters: Vec<ParameterMetadata>,
            #[serde(default)]
            next_token: Option<String>,
        }

        let mut all = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut body = json!({
                "ParameterFilters": [{
                    "Key": "Path",
                    "Option": if recursive { "Recursive" } else { "OneLevel" },
                    "Values": [path],
                }],
                "MaxResults": PAGE_SIZE_HISTORY,
            });
            if let Some(token) = &token {
                body["NextToken"] = json!(token);
            }
            let page: Page = self.call_json("DescribeParameters", &body).await?;
            all.extend(page.parameters);
            match next_token(page.next_token, &token)? {
                Some(next) => token = Some(next),
                None => return Ok(all),
            }
        }
        Err(too_many_pages())
    }

    /// Write a SecureString. Returns the new version number.
    pub async fn put_parameter(&self, input: &PutParameter) -> AwsResult<i64> {
        if input.overwrite && !input.tags.is_empty() {
            return Err(AwsError::Invalid(
                "tags can only be set when creating a parameter, not when overwriting it".into(),
            ));
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Output {
            version: i64,
        }

        let body = PutParameterBody {
            name: &input.name,
            value: input.value.expose(),
            kind: "SecureString",
            overwrite: input.overwrite,
            key_id: input.key_id.as_deref(),
            description: input.description.as_deref(),
            tags: &input.tags,
            tier: input.advanced.then_some("Advanced"),
        };
        let output: Output = self.call_json("PutParameter", &body).await?;
        Ok(output.version)
    }

    pub async fn delete_parameter(&self, name: &str) -> AwsResult<()> {
        self.call("DeleteParameter", &json!({ "Name": name }))
            .await?;
        Ok(())
    }

    /// Every version of a parameter, oldest first (AWS's order), decrypted.
    pub async fn get_parameter_history(&self, name: &str) -> AwsResult<Vec<ParameterHistoryEntry>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Page {
            #[serde(default)]
            parameters: Vec<ParameterHistoryEntry>,
            #[serde(default)]
            next_token: Option<String>,
        }

        let mut all = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut body = json!({
                "Name": name,
                "WithDecryption": true,
                "MaxResults": PAGE_SIZE_HISTORY,
            });
            if let Some(token) = &token {
                body["NextToken"] = json!(token);
            }
            let page: Page = self.call_json("GetParameterHistory", &body).await?;
            all.extend(page.parameters);
            match next_token(page.next_token, &token)? {
                Some(next) => token = Some(next),
                None => return Ok(all),
            }
        }
        Err(too_many_pages())
    }

    pub async fn list_tags_for_resource(&self, name: &str) -> AwsResult<Vec<Tag>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Output {
            #[serde(default)]
            tag_list: Vec<Tag>,
        }
        let body = json!({ "ResourceType": "Parameter", "ResourceId": name });
        let output: Output = self.call_json("ListTagsForResource", &body).await?;
        Ok(output.tag_list)
    }

    pub async fn add_tags_to_resource(&self, name: &str, tags: &[Tag]) -> AwsResult<()> {
        let body = json!({ "ResourceType": "Parameter", "ResourceId": name, "Tags": tags });
        self.call("AddTagsToResource", &body).await?;
        Ok(())
    }

    pub async fn remove_tags_from_resource(&self, name: &str, keys: &[String]) -> AwsResult<()> {
        let body = json!({ "ResourceType": "Parameter", "ResourceId": name, "TagKeys": keys });
        self.call("RemoveTagsFromResource", &body).await?;
        Ok(())
    }

    async fn call_json<T: serde::de::DeserializeOwned>(
        &self,
        operation: &str,
        body: &impl Serialize,
    ) -> AwsResult<T> {
        let response = self.call(operation, body).await?;
        serde_json::from_slice(&response.body).map_err(|e| AwsError::Other {
            code: "MalformedResponse".into(),
            message: format!("{operation} returned a body this client cannot read: {e}"),
            status: response.status,
        })
    }

    /// Sign and send one operation; a non-2xx answer comes back as an error.
    async fn call(&self, operation: &str, body: &impl Serialize) -> AwsResult<Response> {
        let now = now_unix();
        if self.credentials.is_expired(now, EXPIRY_MARGIN_SECONDS) {
            return Err(AwsError::SessionExpired(
                "the temporary AWS credentials have expired".into(),
            ));
        }

        // Serialised straight into a buffer that is wiped if anything below fails before the
        // client takes ownership of it.
        let payload =
            Zeroizing::new(serde_json::to_vec(body).map_err(|e| AwsError::Invalid(e.to_string()))?);

        let url = reqwest::Url::parse(&format!("{}/", self.endpoint))
            .map_err(|e| AwsError::Invalid(format!("bad SSM endpoint: {e}")))?;
        let host = host_header(&url)?;
        let target = format!("AmazonSSM.{operation}");
        let payload_hash = sigv4::sha256_hex(&payload);
        let headers = [
            ("content-type", "application/x-amz-json-1.1"),
            ("x-amz-target", target.as_str()),
        ];
        let signed = sigv4::sign(
            &self.credentials,
            &SigningInput {
                method: "POST",
                host: &host,
                path: "/",
                query: &[],
                headers: &headers,
                payload_sha256: &payload_hash,
                region: &self.region,
                service: "ssm",
                unix_time: now,
                double_encode_path: true,
            },
        );

        let mut request = self.http.post(url).body(payload.to_vec());
        for (name, value) in headers.iter() {
            request = request.header(*name, *value);
        }
        for (name, value) in &signed.headers {
            request = request.header(name.as_str(), value.as_str());
        }

        let response = send(request).await?;
        if response.is_success() {
            Ok(response)
        } else {
            Err(from_json_response(
                response.status,
                response.header("x-amzn-errortype"),
                &response.body,
            ))
        }
    }
}

/// The token for the next page, or `None` when this was the last one.
///
/// A server that returns the same token twice would loop forever; that is reported rather than
/// followed.
fn next_token(next: Option<String>, previous: &Option<String>) -> AwsResult<Option<String>> {
    match next {
        Some(next) if next.is_empty() => Ok(None),
        Some(next) if previous.as_deref() == Some(next.as_str()) => Err(AwsError::Other {
            code: "PaginationLoop".into(),
            message: "AWS returned the same NextToken twice".into(),
            status: 200,
        }),
        other => Ok(other),
    }
}

fn too_many_pages() -> AwsError {
    AwsError::Other {
        code: "TooManyPages".into(),
        message: format!("stopped after {MAX_PAGES} pages"),
        status: 200,
    }
}

/// SSM sends dates as fractional Unix seconds (`1700000000.123`).
fn epoch_seconds<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<i64>, D::Error> {
    let value: Option<f64> = Option::deserialize(deserializer)?;
    Ok(value.map(|v| v.floor() as i64))
}

#[cfg(test)]
mod tests {
    use super::super::fake::{assert_signature_verifies, Canned, FakeAws};
    use super::*;

    fn credentials() -> Credentials {
        Credentials::new("ASIAFAKE", "fake-secret", Some("fake-session-token".into()))
    }

    async fn client(responses: Vec<Canned>) -> (SsmClient, FakeAws) {
        let fake = FakeAws::start(responses).await;
        let client = SsmClient::new("sa-east-1", credentials())
            .unwrap()
            .with_endpoint(&fake.base);
        (client, fake)
    }

    fn parameter(name: &str, value: &str, version: i64) -> serde_json::Value {
        json!({
            "Name": name, "Type": "SecureString", "Value": value, "Version": version,
            "LastModifiedDate": 1_700_000_000.5, "ARN": format!("arn:aws:ssm:sa-east-1:123:parameter{name}"),
            "DataType": "text"
        })
    }

    #[tokio::test]
    async fn listing_a_path_follows_next_token_to_the_end() {
        let (client, fake) = client(vec![
            Canned::json(
                200,
                json!({
                    "Parameters": [parameter("/kh/a", "one", 1), parameter("/kh/b", "two", 3)],
                    "NextToken": "page-2"
                }),
            ),
            Canned::json(
                200,
                json!({ "Parameters": [parameter("/kh/c", "three", 1)] }),
            ),
        ])
        .await;

        let all = client.get_parameters_by_path("/kh/", true).await.unwrap();
        let names: Vec<_> = all.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["/kh/a", "/kh/b", "/kh/c"]);
        assert_eq!(all[1].value.expose(), "two");
        assert_eq!(all[1].version, 3);
        assert_eq!(all[0].last_modified, Some(1_700_000_000));

        let requests = fake.requests();
        assert_eq!(requests.len(), 2);
        let first = requests[0].json();
        assert_eq!(first["Path"], "/kh/");
        assert_eq!(first["Recursive"], true);
        assert_eq!(first["WithDecryption"], true);
        assert_eq!(first["MaxResults"], 10);
        assert!(first.get("NextToken").is_none());
        assert_eq!(requests[1].json()["NextToken"], "page-2");
    }

    #[tokio::test]
    async fn every_request_carries_the_target_and_a_well_formed_signature() {
        let (client, fake) = client(vec![Canned::json(
            200,
            json!({
                "Parameter": parameter("/kh/a", "one", 1)
            }),
        )])
        .await;

        client.get_parameter("/kh/a").await.unwrap();

        let request = &fake.requests()[0];
        assert_eq!(request.method, "POST");
        assert_eq!(request.uri, "/");
        assert_eq!(
            request.header("x-amz-target"),
            Some("AmazonSSM.GetParameter")
        );
        assert_eq!(
            request.header("content-type"),
            Some("application/x-amz-json-1.1")
        );
        assert_eq!(
            request.header("x-amz-security-token"),
            Some("fake-session-token")
        );

        let date = request.header("x-amz-date").unwrap();
        assert_eq!(date.len(), 16);
        assert!(date.ends_with('Z'));

        let authorization = request.header("authorization").unwrap();
        let prefix = format!(
            "AWS4-HMAC-SHA256 Credential=ASIAFAKE/{}/sa-east-1/ssm/aws4_request, \
             SignedHeaders=content-type;host;x-amz-date;x-amz-security-token;x-amz-target, \
             Signature=",
            &date[..8]
        );
        assert!(authorization.starts_with(&prefix), "{authorization}");
        let signature = &authorization[prefix.len()..];
        assert_eq!(signature.len(), 64);
        assert!(signature
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));

        assert_signature_verifies(request, &credentials(), "sa-east-1", "ssm");
    }

    #[tokio::test]
    async fn aws_errors_become_the_matching_variants() {
        let (client, _fake) = client(vec![
            Canned::json(
                400,
                json!({
                    "__type": "com.amazonaws.ssm#AccessDeniedException",
                    "message": "User is not authorized to perform ssm:GetParameter"
                }),
            ),
            Canned::json(400, json!({ "__type": "ParameterNotFound" })),
            Canned::json(
                400,
                json!({
                    "__type": "ExpiredTokenException",
                    "message": "The security token included in the request is expired"
                }),
            ),
            Canned::json(
                400,
                json!({ "__type": "ThrottlingException", "message": "Rate exceeded" }),
            ),
            Canned::json(
                400,
                json!({ "__type": "ParameterAlreadyExists", "message": "exists" }),
            ),
            Canned::json(
                400,
                json!({ "__type": "ValidationException", "message": "bad name" }),
            ),
        ])
        .await;

        assert!(matches!(
            client.get_parameter("/x").await,
            Err(AwsError::AccessDenied(m)) if m.contains("ssm:GetParameter")
        ));
        assert!(matches!(
            client.get_parameter("/x").await,
            Err(AwsError::NotFound(_))
        ));
        assert!(matches!(
            client.get_parameter("/x").await,
            Err(AwsError::SessionExpired(_))
        ));
        assert!(matches!(
            client.get_parameter("/x").await,
            Err(AwsError::Throttled(_))
        ));
        assert!(matches!(
            client
                .put_parameter(&PutParameter {
                    name: "/x".into(),
                    ..Default::default()
                })
                .await,
            Err(AwsError::VersionConflict(_))
        ));
        assert_eq!(
            client.delete_parameter("/x").await,
            Err(AwsError::Invalid("bad name".into()))
        );
    }

    #[tokio::test]
    async fn a_put_writes_a_secure_string_with_its_options() {
        let (client, fake) = client(vec![Canned::json(
            200,
            json!({ "Version": 4, "Tier": "Standard" }),
        )])
        .await;

        let version = client
            .put_parameter(&PutParameter {
                name: "/kh/login".into(),
                value: "s3cret".into(),
                key_id: Some("alias/keyhold".into()),
                description: Some("GitHub".into()),
                overwrite: false,
                tags: vec![Tag::new("keyhold", "1")],
                advanced: false,
            })
            .await
            .unwrap();
        assert_eq!(version, 4);

        let body = fake.requests()[0].json();
        assert_eq!(body["Type"], "SecureString");
        assert_eq!(body["Value"], "s3cret");
        assert_eq!(body["KeyId"], "alias/keyhold");
        assert_eq!(body["Description"], "GitHub");
        assert_eq!(body["Overwrite"], false);
        assert_eq!(body["Tags"], json!([{ "Key": "keyhold", "Value": "1" }]));
        assert!(body.get("Tier").is_none());
    }

    #[tokio::test]
    async fn tags_with_overwrite_are_refused_before_anything_is_sent() {
        let (client, fake) = client(vec![]).await;
        let result = client
            .put_parameter(&PutParameter {
                name: "/kh/x".into(),
                overwrite: true,
                tags: vec![Tag::new("a", "b")],
                ..Default::default()
            })
            .await;
        assert!(matches!(result, Err(AwsError::Invalid(_))));
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn expired_credentials_are_refused_before_anything_is_sent() {
        let fake = FakeAws::start(vec![]).await;
        let client = SsmClient::new("sa-east-1", credentials().with_expiry(1))
            .unwrap()
            .with_endpoint(&fake.base);
        assert!(matches!(
            client.get_parameter("/x").await,
            Err(AwsError::SessionExpired(_))
        ));
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn history_is_paginated_and_carries_who_wrote_each_version() {
        let entry = |version: i64, value: &str| {
            json!({
                "Name": "/kh/a", "Type": "SecureString", "KeyId": "alias/aws/ssm",
                "Description": "d", "Value": value, "Version": version,
                "LastModifiedDate": 1_600_000_000.0,
                "LastModifiedUser": "arn:aws:sts::123:assumed-role/Admin/daniel",
                "Labels": []
            })
        };
        let (client, fake) = client(vec![
            Canned::json(
                200,
                json!({ "Parameters": [entry(1, "v1")], "NextToken": "t" }),
            ),
            Canned::json(200, json!({ "Parameters": [entry(2, "v2")] })),
        ])
        .await;

        let history = client.get_parameter_history("/kh/a").await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].value.expose(), "v2");
        assert_eq!(history[0].key_id.as_deref(), Some("alias/aws/ssm"));
        assert!(history[0]
            .last_modified_user
            .as_deref()
            .unwrap()
            .ends_with("/daniel"));

        let requests = fake.requests();
        assert_eq!(
            requests[0].header("x-amz-target"),
            Some("AmazonSSM.GetParameterHistory")
        );
        assert_eq!(requests[0].json()["WithDecryption"], true);
        assert_eq!(requests[1].json()["NextToken"], "t");
    }

    #[tokio::test]
    async fn tags_are_listed_added_and_removed_on_the_parameter() {
        let (client, fake) = client(vec![
            Canned::json(200, json!({ "TagList": [{ "Key": "k", "Value": "v" }] })),
            Canned::json(200, json!({})),
            Canned::json(200, json!({})),
        ])
        .await;

        assert_eq!(
            client.list_tags_for_resource("/kh/a").await.unwrap(),
            vec![Tag::new("k", "v")]
        );
        client
            .add_tags_to_resource("/kh/a", &[Tag::new("x", "y")])
            .await
            .unwrap();
        client
            .remove_tags_from_resource("/kh/a", &["k".into()])
            .await
            .unwrap();

        let requests = fake.requests();
        assert_eq!(requests[0].json()["ResourceType"], "Parameter");
        assert_eq!(
            requests[1].json()["Tags"],
            json!([{ "Key": "x", "Value": "y" }])
        );
        assert_eq!(requests[2].json()["TagKeys"], json!(["k"]));
        assert_eq!(
            requests[2].header("x-amz-target"),
            Some("AmazonSSM.RemoveTagsFromResource")
        );
    }

    #[tokio::test]
    async fn a_server_that_repeats_its_token_does_not_loop_forever() {
        let page = json!({ "Parameters": [], "NextToken": "same" });
        let (client, _fake) = client(vec![
            Canned::json(200, page.clone()),
            Canned::json(200, page),
        ])
        .await;
        assert!(matches!(
            client.get_parameters_by_path("/kh/", true).await,
            Err(AwsError::Other { code, .. }) if code == "PaginationLoop"
        ));
    }

    #[test]
    fn a_decrypted_value_never_prints() {
        let parameter: Parameter =
            serde_json::from_value(parameter("/kh/a", "hunter2", 1)).unwrap();
        assert!(!format!("{parameter:?}").contains("hunter2"));
    }
}
