//! CloudWatch Logs: where an AWS vault's audit trail is written.
//!
//! The same shape as Parameter Store: a signed `POST /` to `logs.{region}.amazonaws.com` with
//! `X-Amz-Target: Logs_20140328.<Operation>` and a JSON body. Only what the trail needs: create a
//! stream, put events into it, and filter a log group's events.
//!
//! Each person writes to a stream named after them, and the IAM policy pins that name to their
//! Identity Center user (see `docs/AWS.md`), so the stream's name, not anything in an event's body,
//! is who did it.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::error::{from_json_response, AwsError, AwsResult};
use super::sigv4::{self, Credentials, SigningInput};
use super::{host_header, http_client, now_unix, send, trim_endpoint, validate_region, Response};

/// Refuse to sign with credentials this close to expiry: the request could arrive after it.
const EXPIRY_MARGIN_SECONDS: u64 = 30;

/// One event read back from a log group.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEvent {
    /// The stream it was written to: who wrote it.
    #[serde(default)]
    pub log_stream_name: String,
    /// When the writer said it happened, in milliseconds. Not trusted for display.
    #[serde(default)]
    pub timestamp: i64,
    /// When CloudWatch received it, in milliseconds: the time a reader is shown.
    #[serde(default)]
    pub ingestion_time: i64,
    #[serde(default)]
    pub message: String,
}

/// A CloudWatch Logs client for one region and one set of credentials.
pub struct LogsClient {
    http: reqwest::Client,
    region: String,
    endpoint: String,
    credentials: Credentials,
}

impl LogsClient {
    /// A client for `https://logs.{region}.amazonaws.com`.
    pub fn new(region: &str, credentials: Credentials) -> AwsResult<Self> {
        validate_region(region)?;
        Ok(Self {
            http: http_client()?,
            region: region.to_string(),
            endpoint: format!("https://logs.{region}.amazonaws.com"),
            credentials,
        })
    }

    /// Send requests somewhere else; only the tests can (see `SsmClient::with_endpoint`).
    pub(crate) fn with_endpoint(mut self, endpoint: &str) -> Self {
        self.endpoint = trim_endpoint(endpoint);
        self
    }

    /// Create `stream` in `group`. A stream that already exists is not an error.
    pub async fn create_log_stream(&self, group: &str, stream: &str) -> AwsResult<()> {
        match self.call("CreateLogStream", &json!({ "logGroupName": group, "logStreamName": stream })).await {
            Ok(_) => Ok(()),
            Err(AwsError::Other { code, .. }) if code == "ResourceAlreadyExistsException" => Ok(()),
            Err(other) => Err(other),
        }
    }

    /// Put `messages` into `stream`, each with the writer's own time in milliseconds.
    pub async fn put_log_events(&self, group: &str, stream: &str, messages: &[(i64, String)]) -> AwsResult<()> {
        let events: Vec<_> = messages
            .iter()
            .map(|(timestamp, message)| json!({ "timestamp": timestamp, "message": message }))
            .collect();

        let response = self
            .call("PutLogEvents", &json!({ "logGroupName": group, "logStreamName": stream, "logEvents": events }))
            .await?;

        // Events CloudWatch accepted the call for but did not store (too old, too far ahead) come
        // back in `rejectedLogEventsInfo` with a 200. Those are failures to the trail.
        let parsed: serde_json::Value = serde_json::from_slice(&response.body).unwrap_or_default();
        if parsed.get("rejectedLogEventsInfo").is_some_and(|info| !info.is_null()) {
            return Err(AwsError::Invalid(format!("CloudWatch Logs rejected events: {}", parsed["rejectedLogEventsInfo"])));
        }

        Ok(())
    }

    /// [`LogsClient::put_log_events`], creating the stream first when CloudWatch says it is not there.
    pub async fn append(&self, group: &str, stream: &str, messages: &[(i64, String)]) -> AwsResult<()> {
        match self.put_log_events(group, stream, messages).await {
            Err(AwsError::NotFound(_)) => {
                // Either the stream or the log group is missing. Creating the stream tells which:
                // a missing log group answers NotFound again, and that is what the caller sees.
                self.create_log_stream(group, stream).await?;
                self.put_log_events(group, stream, messages).await
            }
            other => other,
        }
    }

    /// Events of `group` matching `pattern` (CloudWatch's filter syntax), newest last, between
    /// `start` and `end` in milliseconds, following `nextToken` up to `max` events.
    pub async fn filter_log_events(
        &self,
        group: &str,
        pattern: Option<&str>,
        start: Option<i64>,
        end: Option<i64>,
        max: usize,
    ) -> AwsResult<Vec<LogEvent>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Page {
            #[serde(default)]
            events: Vec<LogEvent>,
            #[serde(default)]
            next_token: Option<String>,
        }

        let mut all = Vec::new();
        let mut token: Option<String> = None;

        loop {
            let mut body = json!({ "logGroupName": group, "limit": 1000 });
            if let Some(pattern) = pattern {
                body["filterPattern"] = json!(pattern);
            }
            if let Some(start) = start {
                body["startTime"] = json!(start);
            }
            if let Some(end) = end {
                body["endTime"] = json!(end);
            }
            if let Some(token) = &token {
                body["nextToken"] = json!(token);
            }

            let response = self.call("FilterLogEvents", &body).await?;
            let page: Page = serde_json::from_slice(&response.body).map_err(|e| AwsError::Other {
                code: "MalformedResponse".into(),
                message: format!("FilterLogEvents returned a body this client cannot read: {e}"),
                status: response.status,
            })?;

            all.extend(page.events);

            match page.next_token {
                Some(next) if !next.is_empty() && Some(&next) != token.as_ref() && all.len() < max => token = Some(next),
                _ => break,
            }
        }

        all.truncate(max);
        Ok(all)
    }

    /// Sign and send one operation; a non-2xx answer comes back as an error.
    async fn call(&self, operation: &str, body: &impl Serialize) -> AwsResult<Response> {
        let now = now_unix();
        if self.credentials.is_expired(now, EXPIRY_MARGIN_SECONDS) {
            return Err(AwsError::SessionExpired("the temporary AWS credentials have expired".into()));
        }

        let payload = serde_json::to_vec(body).map_err(|e| AwsError::Invalid(e.to_string()))?;
        let url = reqwest::Url::parse(&format!("{}/", self.endpoint))
            .map_err(|e| AwsError::Invalid(format!("bad CloudWatch Logs endpoint: {e}")))?;
        let host = host_header(&url)?;
        let target = format!("Logs_20140328.{operation}");
        let payload_hash = sigv4::sha256_hex(&payload);
        let headers = [("content-type", "application/x-amz-json-1.1"), ("x-amz-target", target.as_str())];
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
                service: "logs",
                unix_time: now,
                double_encode_path: true,
            },
        );

        let mut request = self.http.post(url).body(payload);
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
            Err(from_json_response(response.status, response.header("x-amzn-errortype"), &response.body))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{assert_signature_verifies, Canned, FakeAws};
    use super::*;

    fn credentials() -> Credentials {
        Credentials::new("ASIAFAKE", "fake-secret", Some("fake-session-token".into()))
    }

    async fn client(responses: Vec<Canned>) -> (LogsClient, FakeAws) {
        let fake = FakeAws::start(responses).await;
        let client = LogsClient::new("us-east-2", credentials()).unwrap().with_endpoint(&fake.base);
        (client, fake)
    }

    #[tokio::test]
    async fn appending_creates_the_stream_when_it_is_missing() {
        let missing = Canned::json(400, json!({ "__type": "ResourceNotFoundException", "message": "The specified log stream does not exist." }));
        let (client, fake) = client(vec![missing, Canned::json(200, json!({})), Canned::json(200, json!({}))]).await;

        client.append("/keyhold/audit/team", "ana", &[(1, "{}".into())]).await.unwrap();

        let requests = fake.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].header("x-amz-target"), Some("Logs_20140328.PutLogEvents"));
        assert_eq!(requests[1].header("x-amz-target"), Some("Logs_20140328.CreateLogStream"));
        assert_eq!(requests[1].json()["logStreamName"], "ana");
        assert_signature_verifies(&requests[2], &credentials(), "us-east-2", "logs");
    }

    #[tokio::test]
    async fn a_missing_log_group_is_reported_as_not_found() {
        let missing = || Canned::json(400, json!({ "__type": "ResourceNotFoundException", "message": "The specified log group does not exist." }));
        let (client, _fake) = client(vec![missing(), missing()]).await;

        let failed = client.append("/keyhold/audit/team", "ana", &[(1, "{}".into())]).await;

        assert!(matches!(failed, Err(AwsError::NotFound(_))), "{failed:?}");
    }

    #[tokio::test]
    async fn an_existing_stream_is_fine_and_a_denial_is_a_denial() {
        let exists = Canned::json(400, json!({ "__type": "ResourceAlreadyExistsException", "message": "exists" }));
        let denied = Canned::json(400, json!({ "__type": "AccessDeniedException", "message": "no" }));
        let (client, _fake) = client(vec![exists, denied]).await;

        client.create_log_stream("/g", "ana").await.unwrap();
        let refused = client.put_log_events("/g", "ana", &[(1, "{}".into())]).await;

        assert!(matches!(refused, Err(AwsError::AccessDenied(_))), "{refused:?}");
    }

    #[tokio::test]
    async fn rejected_events_are_a_failure() {
        let (client, _fake) =
            client(vec![Canned::json(200, json!({ "rejectedLogEventsInfo": { "tooOldLogEventEndIndex": 0 } }))]).await;

        assert!(client.put_log_events("/g", "ana", &[(1, "{}".into())]).await.is_err());
    }

    #[tokio::test]
    async fn filtering_follows_next_token_and_keeps_the_stream_and_ingestion_time() {
        let page = |token: Option<&str>, stream: &str| {
            Canned::json(
                200,
                json!({
                    "events": [{ "logStreamName": stream, "timestamp": 5, "ingestionTime": 9, "message": "{\"a\":1}" }],
                    "nextToken": token,
                }),
            )
        };
        let (client, fake) = client(vec![page(Some("t2"), "ana"), page(None, "bia")]).await;

        let events = client.filter_log_events("/g", Some("{ $.record = \"r1\" }"), None, None, 100).await.unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(events[1].log_stream_name, "bia");
        assert_eq!(events[0].ingestion_time, 9);
        assert_eq!(fake.requests()[1].json()["nextToken"], "t2");
    }
}
