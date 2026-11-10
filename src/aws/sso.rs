//! IAM Identity Center (formerly AWS SSO): logging in without the AWS CLI.
//!
//! The person never gives Keyhold an access key. They type their organisation's start URL, open
//! the link Keyhold shows, confirm a short code in their browser, and AWS hands Keyhold a token
//! that can be traded for short-lived credentials for one account and one role. That is the OAuth
//! 2.0 device authorization grant (RFC 8628), and it has three steps here:
//!
//! 1. **Register a client** with SSO-OIDC (`oidc.{region}.amazonaws.com`). Public, named
//!    `keyhold`, asking only for `sso:account:access`. The registration lasts about 90 days and
//!    can be reused for later logins.
//! 2. **Start a device authorization** for the start URL, which returns the code to show and the
//!    link to open.
//! 3. **Poll `CreateToken`** at the interval AWS asks for, until the person approves, refuses, or
//!    the code expires. Each poll is one call to [`SsoClient::poll_device_login`]; the waiting is
//!    the caller's, so the interface can show progress and let the person cancel.
//!
//! With the token, the SSO portal (`portal.sso.{region}.amazonaws.com`) lists the accounts and
//! roles the person may use and issues role credentials, which are what actually sign SSM and S3
//! requests. Logging out revokes the token on AWS's side, not only in this process.
//!
//! None of these endpoints are signed with SigV4: SSO-OIDC is plain JSON authenticated by the
//! client secret and device code, and the portal takes the bearer token in its own header.
//!
//! Every secret along the way — the client secret, the device code, the access and refresh
//! tokens, the role credentials — is held in a type that zeroizes on drop and prints as `…`.

use serde::{Deserialize, Serialize};
use serde_json::json;
use zeroize::Zeroizing;

use super::error::{bare_code, from_json_response, AwsError, AwsResult};
use super::sigv4::{canonical_query, Credentials};
use super::{http_client, now_unix, send, trim_endpoint, validate_region, Response, SecretString};

/// The OAuth grant type for the device flow.
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// The only scope Keyhold asks for: enough to list accounts and get role credentials.
const SCOPE: &str = "sso:account:access";

/// RFC 8628 says to poll every five seconds unless told otherwise.
const DEFAULT_INTERVAL_SECONDS: u64 = 5;

/// The portal's maximum page size.
const PORTAL_PAGE_SIZE: &str = "100";

/// Treat a token as expired this long before AWS would, so a request does not die in flight.
const EXPIRY_MARGIN_SECONDS: u64 = 60;

/// A safety stop for portal pagination.
const MAX_PAGES: usize = 1_000;

/// The body of `CreateToken`, for both grants, borrowing the secrets rather than copying them into
/// a `serde_json::Value` that would not be wiped.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenRequest<'a> {
    client_id: &'a str,
    client_secret: &'a str,
    grant_type: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_code: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceAuthorizationRequest<'a> {
    client_id: &'a str,
    client_secret: &'a str,
    start_url: &'a str,
}

/// An SSO-OIDC client registration: who Keyhold is, as far as Identity Center is concerned.
#[derive(Clone)]
pub struct ClientRegistration {
    client_id: String,
    client_secret: SecretString,
    /// Unix seconds after which the registration must be renewed.
    expires_at: u64,
}

impl ClientRegistration {
    /// Rebuild a registration that was persisted (sealed) earlier.
    pub fn from_parts(client_id: String, client_secret: SecretString, expires_at: u64) -> Self {
        Self {
            client_id,
            client_secret,
            expires_at,
        }
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn client_secret(&self) -> &SecretString {
        &self.client_secret
    }

    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }

    pub fn is_expired(&self, now: u64) -> bool {
        now.saturating_add(EXPIRY_MARGIN_SECONDS) >= self.expires_at
    }
}

impl std::fmt::Debug for ClientRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientRegistration")
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// A login in progress: what to show the person, and what to poll with.
#[derive(Clone)]
pub struct DeviceLogin {
    client: ClientRegistration,
    device_code: SecretString,
    /// The short code the person confirms in the browser, e.g. `ABCD-EFGH`. Not a secret: it is
    /// useless without the device code, and it is meant to be read off the screen.
    pub user_code: String,
    pub verification_uri: String,
    /// The link with the code already filled in. This is the one to open.
    pub verification_uri_complete: String,
    /// Seconds to wait between polls. Add five after every [`Poll::SlowDown`].
    pub interval: u64,
    /// Unix seconds after which the code is dead and the login must start over.
    pub expires_at: u64,
}

impl DeviceLogin {
    pub fn client(&self) -> &ClientRegistration {
        &self.client
    }
}

impl std::fmt::Debug for DeviceLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceLogin")
            .field("client", &self.client)
            .field("device_code", &self.device_code)
            .field("user_code", &self.user_code)
            .field("verification_uri_complete", &self.verification_uri_complete)
            .field("interval", &self.interval)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// An Identity Center access token, with what is needed to refresh it.
#[derive(Clone)]
pub struct SsoToken {
    access_token: SecretString,
    refresh_token: Option<SecretString>,
    /// Unix seconds.
    expires_at: u64,
    client: ClientRegistration,
}

impl SsoToken {
    /// Rebuild a token that was persisted (sealed) earlier.
    pub fn from_parts(
        access_token: SecretString,
        refresh_token: Option<SecretString>,
        expires_at: u64,
        client: ClientRegistration,
    ) -> Self {
        Self {
            access_token,
            refresh_token,
            expires_at,
            client,
        }
    }

    pub fn access_token(&self) -> &SecretString {
        &self.access_token
    }

    pub fn refresh_token(&self) -> Option<&SecretString> {
        self.refresh_token.as_ref()
    }

    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }

    pub fn client(&self) -> &ClientRegistration {
        &self.client
    }

    /// Whether the token is expired, or will be within a minute.
    pub fn is_expired(&self, now: u64) -> bool {
        now.saturating_add(EXPIRY_MARGIN_SECONDS) >= self.expires_at
    }
}

impl std::fmt::Debug for SsoToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SsoToken")
            .field("access_token", &self.access_token)
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "…"))
            .field("expires_at", &self.expires_at)
            .field("client", &self.client)
            .finish()
    }
}

/// Where a device login stands after one poll.
#[derive(Debug)]
pub enum Poll {
    /// Not approved yet. Wait `interval` seconds and poll again.
    Pending,
    /// Polling too fast. Add five seconds to the interval, then poll again.
    SlowDown,
    /// Approved.
    Done(SsoToken),
    /// The person refused in the browser.
    Denied,
    /// The code expired before anybody approved it. Start over.
    Expired,
}

/// An account the person may use.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub account_id: String,
    #[serde(default)]
    pub account_name: String,
    #[serde(default)]
    pub email_address: String,
}

/// A role (permission set) the person may assume in an account.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Role {
    pub account_id: String,
    pub role_name: String,
}

/// The two Identity Center endpoints for one region.
pub struct SsoClient {
    http: reqwest::Client,
    region: String,
    oidc_endpoint: String,
    portal_endpoint: String,
}

impl std::fmt::Debug for SsoClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SsoClient")
            .field("region", &self.region)
            .field("oidc_endpoint", &self.oidc_endpoint)
            .field("portal_endpoint", &self.portal_endpoint)
            .finish()
    }
}

impl SsoClient {
    /// A client for the Identity Center instance in `region` (the region of the start URL's
    /// directory, which need not be the region the parameters live in).
    pub fn new(region: &str) -> AwsResult<Self> {
        validate_region(region)?;
        Ok(Self {
            http: http_client()?,
            region: region.to_string(),
            oidc_endpoint: format!("https://oidc.{region}.amazonaws.com"),
            portal_endpoint: format!("https://portal.sso.{region}.amazonaws.com"),
        })
    }

    /// Point both endpoints elsewhere, for tests.
    pub fn with_endpoints(mut self, oidc: &str, portal: &str) -> Self {
        self.oidc_endpoint = trim_endpoint(oidc);
        self.portal_endpoint = trim_endpoint(portal);
        self
    }

    pub fn region(&self) -> &str {
        &self.region
    }

    /// Register Keyhold as a public OIDC client. Reusable until it expires (about 90 days).
    pub async fn register_client(&self) -> AwsResult<ClientRegistration> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Output {
            client_id: String,
            client_secret: SecretString,
            client_secret_expires_at: u64,
        }

        let body = json!({
            "clientName": "keyhold",
            "clientType": "public",
            "scopes": [SCOPE],
        });
        let response = self.oidc("/client/register", &body).await?;
        let output: Output = parse(&response, "RegisterClient")?;
        Ok(ClientRegistration {
            client_id: output.client_id,
            client_secret: output.client_secret,
            expires_at: output.client_secret_expires_at,
        })
    }

    /// Start a device authorization with an existing registration.
    pub async fn start_device_authorization(
        &self,
        client: ClientRegistration,
        start_url: &str,
    ) -> AwsResult<DeviceLogin> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Output {
            device_code: SecretString,
            user_code: String,
            verification_uri: String,
            verification_uri_complete: String,
            expires_in: u64,
            #[serde(default)]
            interval: Option<u64>,
        }

        let body = DeviceAuthorizationRequest {
            client_id: &client.client_id,
            client_secret: client.client_secret.expose(),
            start_url,
        };
        let response = self.oidc("/device_authorization", &body).await?;
        let output: Output = parse(&response, "StartDeviceAuthorization")?;
        Ok(DeviceLogin {
            client,
            device_code: output.device_code,
            user_code: output.user_code,
            verification_uri: output.verification_uri,
            verification_uri_complete: output.verification_uri_complete,
            interval: output.interval.unwrap_or(DEFAULT_INTERVAL_SECONDS).max(1),
            expires_at: now_unix().saturating_add(output.expires_in),
        })
    }

    /// Register and start a login in one go: what the interface calls when the person clicks
    /// "log in".
    pub async fn start_device_login(&self, start_url: &str) -> AwsResult<DeviceLogin> {
        let client = self.register_client().await?;
        self.start_device_authorization(client, start_url).await
    }

    /// Ask once whether the person has approved the login.
    pub async fn poll_device_login(&self, login: &DeviceLogin) -> AwsResult<Poll> {
        if now_unix() >= login.expires_at {
            return Ok(Poll::Expired);
        }

        let body = TokenRequest {
            client_id: &login.client.client_id,
            client_secret: login.client.client_secret.expose(),
            grant_type: DEVICE_GRANT,
            device_code: Some(login.device_code.expose()),
            refresh_token: None,
        };
        let response = self.send_oidc("/token", &body).await?;
        if response.is_success() {
            let token = token_from(&response, &login.client, None)?;
            return Ok(Poll::Done(token));
        }

        // RFC 8628 errors come back as `{"error": "authorization_pending"}`; AWS also names them
        // in `x-amzn-ErrorType` as `AuthorizationPendingException` and friends.
        let parsed: serde_json::Value =
            serde_json::from_slice(&response.body).unwrap_or(serde_json::Value::Null);
        let codes = [
            parsed.get("error").and_then(|v| v.as_str()).unwrap_or(""),
            parsed
                .get("__type")
                .and_then(|v| v.as_str())
                .map(bare_code)
                .unwrap_or(""),
            response
                .header("x-amzn-errortype")
                .map(bare_code)
                .unwrap_or(""),
        ];
        for code in codes {
            match code {
                "authorization_pending" | "AuthorizationPendingException" => {
                    return Ok(Poll::Pending)
                }
                "slow_down" | "SlowDownException" => return Ok(Poll::SlowDown),
                "expired_token" | "ExpiredTokenException" => return Ok(Poll::Expired),
                "access_denied" | "AccessDeniedException" => return Ok(Poll::Denied),
                _ => {}
            }
        }
        Err(oidc_error(&response))
    }

    /// Trade the refresh token for a new access token, without asking the person again.
    ///
    /// Fails with [`AwsError::SessionExpired`] when there is no refresh token, the registration
    /// has lapsed, or AWS no longer accepts the refresh token — all three mean "log in again".
    pub async fn refresh(&self, token: &SsoToken) -> AwsResult<SsoToken> {
        let Some(refresh_token) = &token.refresh_token else {
            return Err(AwsError::SessionExpired("no refresh token".into()));
        };
        if token.client.is_expired(now_unix()) {
            return Err(AwsError::SessionExpired(
                "the client registration has expired".into(),
            ));
        }

        let body = TokenRequest {
            client_id: &token.client.client_id,
            client_secret: token.client.client_secret.expose(),
            grant_type: "refresh_token",
            device_code: None,
            refresh_token: Some(refresh_token.expose()),
        };
        let response = self.send_oidc("/token", &body).await?;
        if !response.is_success() {
            return Err(match oidc_error(&response) {
                // On the token endpoint, every one of these means the refresh token is done.
                AwsError::Invalid(m) | AwsError::AccessDenied(m) => AwsError::SessionExpired(m),
                other => other,
            });
        }
        token_from(&response, &token.client, token.refresh_token.clone())
    }

    /// Every account the token can see.
    pub async fn list_accounts(&self, token: &SsoToken) -> AwsResult<Vec<Account>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Page {
            #[serde(default)]
            account_list: Vec<Account>,
            #[serde(default)]
            next_token: Option<String>,
        }

        let mut all = Vec::new();
        let mut next: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut query = vec![("max_result", PORTAL_PAGE_SIZE)];
            if let Some(next) = &next {
                query.push(("next_token", next.as_str()));
            }
            let response = self
                .portal("GET", "/assignment/accounts", &query, token)
                .await?;
            let page: Page = parse(&response, "ListAccounts")?;
            all.extend(page.account_list);
            match page
                .next_token
                .filter(|t| !t.is_empty() && Some(t) != next.as_ref())
            {
                Some(token) => next = Some(token),
                None => return Ok(all),
            }
        }
        Ok(all)
    }

    /// Every role the token may assume in `account_id`.
    pub async fn list_roles(&self, token: &SsoToken, account_id: &str) -> AwsResult<Vec<Role>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Page {
            #[serde(default)]
            role_list: Vec<Role>,
            #[serde(default)]
            next_token: Option<String>,
        }

        let mut all = Vec::new();
        let mut next: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut query = vec![("account_id", account_id), ("max_result", PORTAL_PAGE_SIZE)];
            if let Some(next) = &next {
                query.push(("next_token", next.as_str()));
            }
            let response = self
                .portal("GET", "/assignment/roles", &query, token)
                .await?;
            let page: Page = parse(&response, "ListAccountRoles")?;
            all.extend(page.role_list);
            match page
                .next_token
                .filter(|t| !t.is_empty() && Some(t) != next.as_ref())
            {
                Some(token) => next = Some(token),
                None => return Ok(all),
            }
        }
        Ok(all)
    }

    /// Temporary credentials for one role in one account: what signs SSM and S3 requests.
    pub async fn role_credentials(
        &self,
        token: &SsoToken,
        account_id: &str,
        role_name: &str,
    ) -> AwsResult<Credentials> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RoleCredentials {
            access_key_id: String,
            secret_access_key: SecretString,
            session_token: SecretString,
            /// Unix *milliseconds*, unlike everything else.
            expiration: u64,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Output {
            role_credentials: RoleCredentials,
        }

        let query = [("account_id", account_id), ("role_name", role_name)];
        let response = self
            .portal("GET", "/federation/credentials", &query, token)
            .await?;
        let output: Output = parse(&response, "GetRoleCredentials")?;
        let c = &output.role_credentials;
        Ok(Credentials::new(
            c.access_key_id.clone(),
            c.secret_access_key.expose(),
            Some(c.session_token.expose().to_string()),
        )
        .with_expiry(c.expiration / 1000))
    }

    /// Revoke the token on AWS's side. Credentials already issued with it stay valid until they
    /// expire; that is how Identity Center works, and why they are kept short.
    pub async fn logout(&self, token: &SsoToken) -> AwsResult<()> {
        self.portal("POST", "/logout", &[], token).await?;
        Ok(())
    }

    async fn oidc(&self, path: &str, body: &impl Serialize) -> AwsResult<Response> {
        let response = self.send_oidc(path, body).await?;
        if response.is_success() {
            Ok(response)
        } else {
            Err(oidc_error(&response))
        }
    }

    async fn send_oidc(&self, path: &str, body: &impl Serialize) -> AwsResult<Response> {
        let payload =
            Zeroizing::new(serde_json::to_vec(body).map_err(|e| AwsError::Invalid(e.to_string()))?);
        let request = self
            .http
            .post(format!("{}{path}", self.oidc_endpoint))
            .header("content-type", "application/json")
            .body(payload.to_vec());
        send(request).await
    }

    async fn portal(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        token: &SsoToken,
    ) -> AwsResult<Response> {
        if token.is_expired(now_unix()) {
            return Err(AwsError::SessionExpired(
                "the Identity Center session has expired".into(),
            ));
        }

        let query = canonical_query(query);
        let url = if query.is_empty() {
            format!("{}{path}", self.portal_endpoint)
        } else {
            format!("{}{path}?{query}", self.portal_endpoint)
        };
        let request = match method {
            "POST" => self.http.post(url),
            _ => self.http.get(url),
        }
        .header("x-amz-sso_bearer_token", token.access_token.expose());

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

fn oidc_error(response: &Response) -> AwsError {
    from_json_response(
        response.status,
        response.header("x-amzn-errortype"),
        &response.body,
    )
}

fn parse<T: serde::de::DeserializeOwned>(response: &Response, operation: &str) -> AwsResult<T> {
    serde_json::from_slice(&response.body).map_err(|e| AwsError::Other {
        code: "MalformedResponse".into(),
        message: format!("{operation} returned a body this client cannot read: {e}"),
        status: response.status,
    })
}

/// Build a token from a successful `CreateToken` answer. A refresh answer may omit the refresh
/// token, in which case the old one is kept.
fn token_from(
    response: &Response,
    client: &ClientRegistration,
    previous_refresh: Option<SecretString>,
) -> AwsResult<SsoToken> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Output {
        access_token: SecretString,
        expires_in: u64,
        #[serde(default)]
        refresh_token: Option<SecretString>,
    }

    let output: Output = parse(response, "CreateToken")?;
    Ok(SsoToken {
        access_token: output.access_token,
        refresh_token: output.refresh_token.or(previous_refresh),
        expires_at: now_unix().saturating_add(output.expires_in),
        client: client.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Canned, FakeAws};
    use super::*;

    async fn client(responses: Vec<Canned>) -> (SsoClient, FakeAws) {
        let fake = FakeAws::start(responses).await;
        let client = SsoClient::new("us-east-1")
            .unwrap()
            .with_endpoints(&fake.base, &fake.base);
        (client, fake)
    }

    fn registration() -> ClientRegistration {
        ClientRegistration::from_parts("cid".into(), "csecret".into(), now_unix() + 86_400)
    }

    fn login(expires_at: u64) -> DeviceLogin {
        DeviceLogin {
            client: registration(),
            device_code: "dcode".into(),
            user_code: "ABCD-EFGH".into(),
            verification_uri: "https://device.sso.us-east-1.amazonaws.com/".into(),
            verification_uri_complete:
                "https://device.sso.us-east-1.amazonaws.com/?user_code=ABCD-EFGH".into(),
            interval: 5,
            expires_at,
        }
    }

    fn token() -> SsoToken {
        SsoToken::from_parts(
            "atoken".into(),
            Some("rtoken".into()),
            now_unix() + 3600,
            registration(),
        )
    }

    #[tokio::test]
    async fn starting_a_login_registers_a_public_client_and_returns_what_to_show() {
        let (client, fake) = client(vec![
            Canned::json(200, json!({
                "clientId": "cid", "clientSecret": "csecret",
                "clientIdIssuedAt": 1, "clientSecretExpiresAt": 4_000_000_000u64
            })),
            Canned::json(200, json!({
                "deviceCode": "dcode", "userCode": "ABCD-EFGH",
                "verificationUri": "https://device.sso.us-east-1.amazonaws.com/",
                "verificationUriComplete": "https://device.sso.us-east-1.amazonaws.com/?user_code=ABCD-EFGH",
                "expiresIn": 600, "interval": 1
            })),
        ])
        .await;

        let login = client
            .start_device_login("https://acme.awsapps.com/start")
            .await
            .unwrap();
        assert_eq!(login.user_code, "ABCD-EFGH");
        assert!(login
            .verification_uri_complete
            .ends_with("user_code=ABCD-EFGH"));
        assert_eq!(login.interval, 1);
        assert!(login.expires_at > now_unix() + 590);
        assert_eq!(login.client().client_id(), "cid");

        let requests = fake.requests();
        assert_eq!(requests[0].uri, "/client/register");
        assert_eq!(
            requests[0].json(),
            json!({ "clientName": "keyhold", "clientType": "public", "scopes": ["sso:account:access"] })
        );
        assert_eq!(requests[1].uri, "/device_authorization");
        assert_eq!(
            requests[1].json()["startUrl"],
            "https://acme.awsapps.com/start"
        );
        assert_eq!(requests[1].json()["clientSecret"], "csecret");
    }

    #[tokio::test]
    async fn polling_walks_through_pending_and_slow_down_to_a_token() {
        let (client, fake) = client(vec![
            Canned::json(
                400,
                json!({ "error": "authorization_pending", "error_description": null }),
            ),
            Canned::json(400, json!({ "error": "slow_down" })),
            Canned::json(400, json!({})).with_header(
                "x-amzn-errortype",
                "AuthorizationPendingException:http://internal/",
            ),
            Canned::json(
                200,
                json!({
                    "accessToken": "atoken", "tokenType": "Bearer", "expiresIn": 28_800,
                    "refreshToken": "rtoken"
                }),
            ),
        ])
        .await;
        let login = login(now_unix() + 600);

        assert!(matches!(
            client.poll_device_login(&login).await.unwrap(),
            Poll::Pending
        ));
        assert!(matches!(
            client.poll_device_login(&login).await.unwrap(),
            Poll::SlowDown
        ));
        assert!(matches!(
            client.poll_device_login(&login).await.unwrap(),
            Poll::Pending
        ));
        let Poll::Done(token) = client.poll_device_login(&login).await.unwrap() else {
            panic!("expected a token");
        };
        assert_eq!(token.access_token().expose(), "atoken");
        assert_eq!(token.refresh_token().unwrap().expose(), "rtoken");
        assert!(token.expires_at() > now_unix() + 28_000);

        let body = fake.requests()[0].json();
        assert_eq!(
            body["grantType"],
            "urn:ietf:params:oauth:grant-type:device_code"
        );
        assert_eq!(body["deviceCode"], "dcode");
        assert_eq!(fake.requests()[0].uri, "/token");
    }

    #[tokio::test]
    async fn a_refused_or_expired_login_says_so() {
        let (client, _fake) = client(vec![
            Canned::json(400, json!({ "error": "access_denied" })),
            Canned::json(400, json!({ "error": "expired_token" })),
            Canned::json(
                400,
                json!({ "error": "invalid_client", "error_description": "bad client" }),
            )
            .with_header("x-amzn-errortype", "InvalidClientException"),
        ])
        .await;
        let login = login(now_unix() + 600);

        assert!(matches!(
            client.poll_device_login(&login).await.unwrap(),
            Poll::Denied
        ));
        assert!(matches!(
            client.poll_device_login(&login).await.unwrap(),
            Poll::Expired
        ));
        assert!(matches!(
            client.poll_device_login(&login).await,
            Err(AwsError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn a_code_past_its_expiry_is_not_polled_at_all() {
        let (client, fake) = client(vec![]).await;
        assert!(matches!(
            client.poll_device_login(&login(1)).await.unwrap(),
            Poll::Expired
        ));
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn refreshing_keeps_the_old_refresh_token_when_no_new_one_comes_back() {
        let (client, fake) = client(vec![
            Canned::json(200, json!({ "accessToken": "atoken2", "expiresIn": 3600 })),
            Canned::json(400, json!({ "error": "invalid_grant" }))
                .with_header("x-amzn-errortype", "InvalidGrantException"),
        ])
        .await;

        let refreshed = client.refresh(&token()).await.unwrap();
        assert_eq!(refreshed.access_token().expose(), "atoken2");
        assert_eq!(refreshed.refresh_token().unwrap().expose(), "rtoken");
        let body = fake.requests()[0].json();
        assert_eq!(body["grantType"], "refresh_token");
        assert_eq!(body["refreshToken"], "rtoken");

        assert!(matches!(
            client.refresh(&token()).await,
            Err(AwsError::SessionExpired(_))
        ));

        let without = SsoToken::from_parts("a".into(), None, now_unix() + 3600, registration());
        assert!(matches!(
            client.refresh(&without).await,
            Err(AwsError::SessionExpired(_))
        ));
    }

    #[tokio::test]
    async fn accounts_and_roles_are_paginated_and_sent_with_the_bearer_header() {
        let (client, fake) = client(vec![
            Canned::json(200, json!({
                "accountList": [{ "accountId": "111", "accountName": "prod", "emailAddress": "a@x" }],
                "nextToken": "n1"
            })),
            Canned::json(200, json!({
                "accountList": [{ "accountId": "222", "accountName": "dev", "emailAddress": "b@x" }]
            })),
            Canned::json(200, json!({
                "roleList": [{ "accountId": "111", "roleName": "Admin" }], "nextToken": "r1"
            })),
            Canned::json(200, json!({ "roleList": [{ "accountId": "111", "roleName": "ReadOnly" }] })),
        ])
        .await;

        let accounts = client.list_accounts(&token()).await.unwrap();
        assert_eq!(
            accounts
                .iter()
                .map(|a| a.account_name.as_str())
                .collect::<Vec<_>>(),
            ["prod", "dev"]
        );
        let roles = client.list_roles(&token(), "111").await.unwrap();
        assert_eq!(
            roles
                .iter()
                .map(|r| r.role_name.as_str())
                .collect::<Vec<_>>(),
            ["Admin", "ReadOnly"]
        );

        let requests = fake.requests();
        assert_eq!(requests[0].uri, "/assignment/accounts?max_result=100");
        assert_eq!(
            requests[1].uri,
            "/assignment/accounts?max_result=100&next_token=n1"
        );
        assert_eq!(
            requests[2].uri,
            "/assignment/roles?account_id=111&max_result=100"
        );
        assert_eq!(
            requests[3].uri,
            "/assignment/roles?account_id=111&max_result=100&next_token=r1"
        );
        assert!(requests
            .iter()
            .all(|r| r.header("x-amz-sso_bearer_token") == Some("atoken")));
    }

    #[tokio::test]
    async fn role_credentials_come_back_temporary_and_expiring() {
        let (client, fake) = client(vec![
            Canned::json(
                200,
                json!({ "roleCredentials": {
                    "accessKeyId": "ASIAROLE", "secretAccessKey": "rsecret",
                    "sessionToken": "rsession", "expiration": 1_900_000_000_000u64
                }}),
            ),
            Canned::json(
                401,
                json!({ "message": "Session token not found or invalid" }),
            )
            .with_header("x-amzn-errortype", "UnauthorizedException:http://internal/"),
        ])
        .await;

        let credentials = client
            .role_credentials(&token(), "111", "Admin Role")
            .await
            .unwrap();
        assert_eq!(credentials.access_key_id(), "ASIAROLE");
        assert_eq!(credentials.session_token(), Some("rsession"));
        assert_eq!(credentials.expires_at(), Some(1_900_000_000));
        assert_eq!(
            fake.requests()[0].uri,
            "/federation/credentials?account_id=111&role_name=Admin%20Role"
        );

        assert!(matches!(
            client.role_credentials(&token(), "111", "Admin").await,
            Err(AwsError::SessionExpired(_))
        ));
    }

    #[tokio::test]
    async fn logging_out_posts_to_the_portal() {
        let (client, fake) = client(vec![Canned::empty(200)]).await;
        client.logout(&token()).await.unwrap();
        let request = &fake.requests()[0];
        assert_eq!(request.method, "POST");
        assert_eq!(request.uri, "/logout");
        assert_eq!(request.header("x-amz-sso_bearer_token"), Some("atoken"));
    }

    #[tokio::test]
    async fn an_expired_token_is_not_sent_to_the_portal() {
        let (client, fake) = client(vec![]).await;
        let expired = SsoToken::from_parts("a".into(), None, 10, registration());
        assert!(matches!(
            client.list_accounts(&expired).await,
            Err(AwsError::SessionExpired(_))
        ));
        assert!(fake.requests().is_empty());
    }

    #[test]
    fn nothing_secret_prints() {
        let printed = format!("{:?} {:?}", token(), login(0));
        for secret in ["atoken", "rtoken", "csecret", "dcode"] {
            assert!(!printed.contains(secret), "{secret} leaked into {printed}");
        }
        assert!(printed.contains("ABCD-EFGH"));
    }
}
