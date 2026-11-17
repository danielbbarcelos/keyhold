//! STS, for one question: who are these credentials?
//!
//! The audit trail writes to a stream named after the person, and the IAM policy only lets each
//! person write the stream named by their Identity Center user (`${aws:PrincipalTag/keyhold-user}`,
//! from `${path:userName}`). Identity Center names the role session after that same user, so the
//! session name in `GetCallerIdentity`'s ARN is the stream name the policy will accept.
//!
//! `GetCallerIdentity` needs no permission: IAM cannot deny it.

use super::error::{classify, AwsError, AwsResult};
use super::sigv4::{self, Credentials, SigningInput};
use super::{host_header, http_client, now_unix, send, trim_endpoint, validate_region};

const EXPIRY_MARGIN_SECONDS: u64 = 30;

const BODY: &str = "Action=GetCallerIdentity&Version=2011-06-15";

pub struct StsClient {
    http: reqwest::Client,
    region: String,
    endpoint: String,
    credentials: Credentials,
}

/// Who a set of credentials belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub account: String,
    pub arn: String,
}

impl Caller {
    /// The role session's name: the Identity Center user for a role assumed through the portal
    /// (`arn:aws:sts::123:assumed-role/AWSReservedSSO_Team_abc/ana.souza`).
    pub fn session_name(&self) -> Option<&str> {
        let rest = self.arn.split(":assumed-role/").nth(1)?;
        let (_, session) = rest.split_once('/')?;
        (!session.is_empty()).then_some(session)
    }
}

impl StsClient {
    /// A client for the regional endpoint, `https://sts.{region}.amazonaws.com`.
    pub fn new(region: &str, credentials: Credentials) -> AwsResult<Self> {
        validate_region(region)?;
        Ok(Self {
            http: http_client()?,
            region: region.to_string(),
            endpoint: format!("https://sts.{region}.amazonaws.com"),
            credentials,
        })
    }

    pub(crate) fn with_endpoint(mut self, endpoint: &str) -> Self {
        self.endpoint = trim_endpoint(endpoint);
        self
    }

    pub async fn get_caller_identity(&self) -> AwsResult<Caller> {
        let now = now_unix();
        if self.credentials.is_expired(now, EXPIRY_MARGIN_SECONDS) {
            return Err(AwsError::SessionExpired("the temporary AWS credentials have expired".into()));
        }

        let url = reqwest::Url::parse(&format!("{}/", self.endpoint))
            .map_err(|e| AwsError::Invalid(format!("bad STS endpoint: {e}")))?;
        let host = host_header(&url)?;
        let payload_hash = sigv4::sha256_hex(BODY.as_bytes());
        let headers = [("content-type", "application/x-www-form-urlencoded; charset=utf-8")];
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
                service: "sts",
                unix_time: now,
                double_encode_path: true,
            },
        );

        let mut request = self.http.post(url).body(BODY);
        for (name, value) in headers.iter() {
            request = request.header(*name, *value);
        }
        for (name, value) in &signed.headers {
            request = request.header(name.as_str(), value.as_str());
        }

        let response = send(request).await?;
        let body = std::str::from_utf8(&response.body).unwrap_or("");

        if !response.is_success() {
            return Err(classify(response.status, tag(body, "Code").unwrap_or(""), tag(body, "Message").unwrap_or("")));
        }

        match (tag(body, "Account"), tag(body, "Arn")) {
            (Some(account), Some(arn)) => Ok(Caller { account: account.trim().to_string(), arn: arn.trim().to_string() }),
            _ => Err(AwsError::Other {
                code: "MalformedResponse".into(),
                message: "GetCallerIdentity returned no Arn".into(),
                status: response.status,
            }),
        }
    }
}

fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}

#[cfg(test)]
mod tests {
    use super::super::fake::{assert_signature_verifies, Canned, FakeAws};
    use super::*;

    #[test]
    fn the_session_name_of_an_identity_center_role_is_the_user() {
        let caller = Caller {
            account: "123".into(),
            arn: "arn:aws:sts::123:assumed-role/AWSReservedSSO_ParameterStoreReadWrite_0a1b/ana.souza".into(),
        };
        assert_eq!(caller.session_name(), Some("ana.souza"));

        let user = Caller { account: "123".into(), arn: "arn:aws:iam::123:user/deploy".into() };
        assert_eq!(user.session_name(), None);
    }

    #[tokio::test]
    async fn asks_with_a_signed_form_and_reads_the_arn() {
        let credentials = Credentials::new("ASIAFAKE", "fake-secret", Some("token".into()));
        let xml = "<GetCallerIdentityResponse><GetCallerIdentityResult><Arn>arn:aws:sts::123:assumed-role/R/ana</Arn>\
                   <UserId>AROA:ana</UserId><Account>123</Account></GetCallerIdentityResult></GetCallerIdentityResponse>";
        let fake = FakeAws::start(vec![Canned::xml(200, xml)]).await;
        let client = StsClient::new("us-east-2", credentials.clone()).unwrap().with_endpoint(&fake.base);

        let caller = client.get_caller_identity().await.unwrap();

        assert_eq!(caller.session_name(), Some("ana"));
        assert_eq!(caller.account, "123");
        assert_signature_verifies(&fake.requests()[0], &credentials, "us-east-2", "sts");
    }
}
