//! AWS Parameter Store vaults over HTTP (F9).
//!
//! Two halves. The `/aws` routes manage what an AWS vault stands on: Identity Center logins,
//! connections (an account and a role), and a health check. The rest are the **same routes a
//! local vault uses**, answered from AWS instead of from the file: `contents.rs` looks at the
//! vault's kind or at the id it was given, and hands AWS ones to the functions here. The JSON
//! that comes back has the shapes the interface already draws, so an AWS vault is a vault.
//!
//! **Where things come from.** Every record is one parameter; every folder is a path; the vault is
//! the whole account from `/` (A5). The rules for turning one into the other are in
//! `aws::mapping`, and nothing here second-guesses them.
//!
//! **Nothing is copied to disk** (A13). Reads go to AWS every time, with two caches of a few
//! seconds in memory (`aws::state`) so that one screen asking for folders and then records does not
//! read the account twice. Writes clear them.
//!
//! **The store's lock is never held across a request to AWS.** It is a `std::sync::Mutex`, and a
//! request can take seconds; every function here takes it, reads or writes what it needs, and lets
//! go before awaiting anything. The lock of `AwsState` is the same, and is never taken while the
//! key is borrowed (`Lock::with_key`), because both go through the vault's lock.
//!
//! **Errors say what to do.** An `AwsError` becomes `{ "error": "<frase>", "aws": "<kind>" }`
//! with a status the interface can switch on: 424 when the AWS session needs renewing (never 401,
//! which belongs to Keyhold's own session), 403, 404, 409, 429, 502 and 400.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{bad_request, conflict, locked, Answer, AppState, Failure};
use crate::attachments::Attachment;
use crate::audit::{self, Detail};
use crate::aws::config::{self, AwsConfig, Connection, Login};
use crate::aws::mapping::{self, encode_id};
use crate::aws::state::{Pending, Snapshot, SNAPSHOT_SECONDS, TAGS_SECONDS};
use crate::aws::{
    now_unix, AwsError, Credentials, Parameter, ParameterMetadata, Poll, PutParameter, SsmClient,
    SsoToken, Tag,
};
use crate::records::{OpenRecord, Payload, RecordHeader, RecordPreview, VersionEntry};
use crate::vaults::{self, AwsVault};

/// How many tag requests run at once while a list is drawn. Parameter Store throttles
/// `ListTagsForResource` at a few per second per account; eight in flight stays under it for any
/// list a person scrolls through.
const TAG_CONCURRENCY: usize = 8;

/// Refresh role credentials this long before they expire.
const CREDENTIALS_MARGIN: u64 = 300;

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/aws", get(overview))
        .route("/aws/logins", post(start_login))
        .route("/aws/logins/:id", delete(delete_login))
        .route("/aws/logins/:id/poll", post(poll_login))
        .route("/aws/logins/:id/renew", post(renew_login))
        .route("/aws/logins/:id/revoke", post(revoke_login))
        .route("/aws/logins/:id/accounts", get(list_accounts))
        .route("/aws/connections", post(create_connection))
        .route("/aws/connections/:id", put(update_connection).delete(delete_connection))
        .route("/aws/vaults/:id/check", post(check_vault))
}

// ── errors ──────────────────────────────────────────────────────────────────────────────────────

/// An `AwsError` as the interface hears it.
pub(crate) fn aws_failure(error: AwsError) -> Failure {
    let (status, kind, text) = match &error {
        AwsError::SessionExpired(_) => (
            StatusCode::FAILED_DEPENDENCY,
            "session_expired",
            "a sessão da AWS expirou. Renove o login para continuar".to_string(),
        ),
        AwsError::AccessDenied(detail) => (
            StatusCode::FORBIDDEN,
            "access_denied",
            format!("a AWS negou o acesso para a função desta conexão ({detail})"),
        ),
        AwsError::NotFound(_) => (
            StatusCode::NOT_FOUND,
            "not_found",
            "isso não existe mais na AWS. Talvez alguém tenha apagado".to_string(),
        ),
        AwsError::VersionConflict(_) => (
            StatusCode::CONFLICT,
            "conflict",
            "alguém mudou isso na AWS depois que você abriu. Recarregue antes de salvar".to_string(),
        ),
        AwsError::Throttled(_) => (
            StatusCode::TOO_MANY_REQUESTS,
            "throttled",
            "a AWS pediu para ir mais devagar. Tente de novo em alguns segundos".to_string(),
        ),
        AwsError::Network(_) => (
            StatusCode::BAD_GATEWAY,
            "network",
            "não consegui falar com a AWS. Confira a conexão com a internet".to_string(),
        ),
        AwsError::Invalid(detail) => (
            StatusCode::BAD_REQUEST,
            "invalid",
            format!("a AWS recusou o pedido: {detail}"),
        ),
        AwsError::Other { code, .. } => (
            StatusCode::BAD_GATEWAY,
            "other",
            format!("a AWS respondeu com um erro inesperado ({code})"),
        ),
    };

    Failure(status, text, Some(kind))
}

/// A refusal of ours that is still about AWS, for the interface to show the same way.
fn aws_refusal(status: StatusCode, kind: &'static str, text: impl Into<String>) -> Failure {
    Failure(status, text.into(), Some(kind))
}

fn not_found(text: &str) -> Failure {
    aws_refusal(StatusCode::NOT_FOUND, "not_found", text)
}

fn session_expired() -> Failure {
    aws_failure(AwsError::SessionExpired("no token".into()))
}

/// What an S3 error means for a vault's bucket, naming the bucket and the permissions (A18).
fn s3_failure(error: AwsError, bucket: &str, region: &str) -> Failure {
    match error {
        AwsError::AccessDenied(_) => aws_refusal(
            StatusCode::FORBIDDEN,
            "access_denied",
            format!(
                "sem permissão no bucket {bucket}. A função desta conexão precisa de \
                 s3:ListBucket, s3:GetObject, s3:PutObject e s3:DeleteObject nele, e de \
                 kms:GenerateDataKey e kms:Decrypt na chave dos parâmetros"
            ),
        ),
        AwsError::NotFound(message) if message.to_lowercase().contains("bucket") => aws_refusal(
            StatusCode::NOT_FOUND,
            "not_found",
            format!(
                "o bucket {bucket} não existe na região {region}. Crie-o na AWS: o Keyhold não \
                 cria buckets"
            ),
        ),
        AwsError::NotFound(_) => not_found("esse arquivo não existe mais no bucket"),
        AwsError::Invalid(message) if message.contains("region") => aws_refusal(
            StatusCode::BAD_REQUEST,
            "invalid",
            format!("o bucket {bucket} está em outra região, não em {region}"),
        ),
        other => aws_failure(other),
    }
}

// ── the configuration ───────────────────────────────────────────────────────────────────────────

fn load(state: &AppState) -> Result<AwsConfig, Failure> {
    let store = state.store.lock().unwrap();

    state
        .lock
        .with_key(|key| config::load(&store, key))
        .ok_or_else(locked)?
        .map_err(bad_request)
}

/// Change the configuration and save it, all under one hold of the store.
fn change<T>(
    state: &AppState,
    f: impl FnOnce(&mut AwsConfig, &[vaults::VaultSummary]) -> Result<T, Failure>,
) -> Result<T, Failure> {
    let store = state.store.lock().unwrap();

    state
        .lock
        .with_key(|key| -> Result<T, Failure> {
            let mut config = config::load(&store, key).map_err(bad_request)?;
            let vaults = vaults::list_vaults(&store, key).map_err(bad_request)?;
            let out = f(&mut config, &vaults)?;
            config::save(&store, key, &config).map_err(bad_request)?;
            Ok(out)
        })
        .ok_or_else(locked)?
}

fn require_unlocked(state: &AppState) -> Result<(), Failure> {
    state.lock.with_key(|_| ()).ok_or_else(locked)
}

fn note(state: &AppState, action: &str, scope: &str, detail: Detail) {
    let store = state.store.lock().unwrap();
    audit::note(&store, &state.lock, action, Some(scope), detail);
}

fn login_json(login: &Login, expires_at: Option<u64>) -> Value {
    let now = now_unix();

    json!({
        "id": login.id,
        "start_url": login.start_url,
        "region": login.region,
        "label": login.label,
        "connected": expires_at.is_some_and(|at| at > now),
        "expires_at": expires_at,
    })
}

fn connection_json(connection: &Connection, used_by: Vec<String>) -> Value {
    json!({
        "id": connection.id,
        "login_id": connection.login_id,
        "account_id": connection.account_id,
        "account_name": connection.account_name,
        "role": connection.role,
        "title": connection.title,
        "note": connection.note,
        "used_by": used_by,
    })
}

fn used_by(connection_id: &str, vaults: &[vaults::VaultSummary]) -> Vec<String> {
    vaults
        .iter()
        .filter(|v| v.aws.as_ref().is_some_and(|aws| aws.connection_id == connection_id))
        .map(|v| v.id.clone())
        .collect()
}

// ── /aws ────────────────────────────────────────────────────────────────────────────────────────

/// Logins (with whether each has a live session) and connections (with the vaults using each).
async fn overview(State(state): State<Arc<AppState>>) -> Answer {
    let (config, vaults) = {
        let store = state.store.lock().unwrap();
        state
            .lock
            .with_key(|key| -> Result<_, Failure> {
                Ok((
                    config::load(&store, key).map_err(bad_request)?,
                    vaults::list_vaults(&store, key).map_err(bad_request)?,
                ))
            })
            .ok_or_else(locked)??
    };

    let sessions: HashMap<String, u64> = state
        .aws
        .with(&state.lock, |inner| {
            inner.tokens.iter().map(|(id, token)| (id.clone(), token.expires_at())).collect()
        })
        .ok_or_else(locked)?;

    Ok(Json(json!({
        "logins": config
            .logins
            .iter()
            .map(|login| login_json(login, sessions.get(&login.id).copied()))
            .collect::<Vec<_>>(),
        "connections": config
            .connections
            .iter()
            .map(|c| connection_json(c, used_by(&c.id, &vaults)))
            .collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct NewLogin {
    start_url: String,
    region: String,
    #[serde(default)]
    label: Option<String>,
}

/// Start an Identity Center device login and hand back what to show the person.
async fn start_login(State(state): State<Arc<AppState>>, Json(body): Json<NewLogin>) -> Answer {
    let start_url = body.start_url.trim().to_string();
    let region = body.region.trim().to_string();

    if !start_url.starts_with("https://") {
        return Err(bad_request("o endereço de login precisa começar com https://"));
    }
    crate::aws::validate_region(&region)
        .map_err(|_| bad_request(format!("'{region}' não é uma região da AWS")))?;

    let config = load(&state)?;

    // The same organisation and region is the same login: logging in again renews it rather than
    // listing it twice.
    let (login, known) = match config
        .logins
        .iter()
        .find(|l| l.start_url == start_url && l.region == region)
    {
        Some(existing) => (existing.clone(), true),
        None => (
            Login {
                id: vaults::new_id(),
                label: body
                    .label
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .unwrap_or_else(|| start_url.clone()),
                start_url,
                region,
            },
            false,
        ),
    };

    device_login(&state, login, known).await
}

/// Register, start the device authorization, and remember it until it is approved.
async fn device_login(state: &AppState, login: Login, known: bool) -> Answer {
    let sso = state.aws.sso(&login.region).map_err(aws_failure)?;
    let device = sso.start_device_login(&login.start_url).await.map_err(aws_failure)?;

    let answer = json!({
        "login": login_json(&login, None),
        "device": {
            "user_code": device.user_code,
            "verification_uri_complete": device.verification_uri_complete,
            "interval": device.interval,
            "expires_at": device.expires_at,
        },
    });

    state
        .aws
        .with(&state.lock, |inner| {
            inner.pending.insert(login.id.clone(), Pending { login, device, known });
        })
        .ok_or_else(locked)?;

    Ok(Json(answer))
}

/// Ask AWS once whether the person approved the login. The interface calls this every `interval`.
async fn poll_login(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let waiting = state
        .aws
        .with(&state.lock, |inner| {
            let device = inner.pending.get(&id).map(|p| (p.device.clone(), p.login.clone(), p.known));
            (device, inner.tokens.contains_key(&id))
        })
        .ok_or_else(locked)?;

    let (device, login, known) = match waiting {
        (Some(found), _) => found,
        (None, true) => return Ok(Json(json!({ "state": "done" }))),
        (None, false) => return Err(not_found("não há login em andamento para essa conta")),
    };

    let sso = state.aws.sso(&login.region).map_err(aws_failure)?;
    let outcome = sso.poll_device_login(&device).await.map_err(aws_failure)?;

    let answer = match outcome {
        Poll::Pending => "pending",
        Poll::SlowDown => {
            state.aws.with(&state.lock, |inner| {
                if let Some(pending) = inner.pending.get_mut(&id) {
                    pending.device.interval += 5;
                }
            });
            "slow_down"
        }
        Poll::Denied => {
            state.aws.with(&state.lock, |inner| inner.pending.remove(&id));
            "denied"
        }
        Poll::Expired => {
            state.aws.with(&state.lock, |inner| inner.pending.remove(&id));
            "expired"
        }
        Poll::Done(token) => {
            state
                .aws
                .with(&state.lock, |inner| {
                    inner.pending.remove(&id);
                    inner.tokens.insert(id.clone(), token);
                    // A new token can see different roles; credentials are fetched again.
                    inner.credentials.clear();
                })
                .ok_or_else(locked)?;

            if !known {
                change(&state, |config, _| {
                    if config.login(&login.id).is_none() {
                        config.logins.push(login.clone());
                    }
                    Ok(())
                })?;
            }

            "done"
        }
    };

    Ok(Json(json!({ "state": answer })))
}

/// Renew a login: quietly with the refresh token when AWS still accepts it, otherwise a new
/// device login, answered exactly like `POST /aws/logins`.
async fn renew_login(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let config = load(&state)?;
    let login = config.login(&id).cloned().ok_or_else(|| not_found("essa conta AWS não existe"))?;

    let held = state
        .aws
        .with(&state.lock, |inner| inner.tokens.get(&id).cloned())
        .ok_or_else(locked)?;

    if let Some(token) = held.filter(|t| t.refresh_token().is_some()) {
        let sso = state.aws.sso(&login.region).map_err(aws_failure)?;

        if let Ok(fresh) = sso.refresh(&token).await {
            let expires_at = fresh.expires_at();
            state.aws.with(&state.lock, |inner| inner.tokens.insert(id.clone(), fresh));

            return Ok(Json(json!({ "login": login_json(&login, Some(expires_at)), "device": null })));
        }
    }

    device_login(&state, login, true).await
}

/// Log out of Identity Center. The login stays listed, disconnected (A4).
async fn revoke_login(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let config = load(&state)?;
    let login = config.login(&id).cloned().ok_or_else(|| not_found("essa conta AWS não existe"))?;

    let token = state
        .aws
        .with(&state.lock, |inner| {
            inner.pending.remove(&id);
            for connection in config.connections.iter().filter(|c| c.login_id == id) {
                inner.credentials.remove(&connection.id);
            }
            inner.snapshots.clear();
            inner.tokens.remove(&id)
        })
        .ok_or_else(locked)?;

    // Revoked on AWS's side when possible. If AWS cannot be reached the token is still forgotten
    // here, which is what the person asked for; it expires on its own.
    if let Some(token) = token {
        if let Ok(sso) = state.aws.sso(&login.region) {
            let _ = sso.logout(&token).await;
        }
    }

    Ok(Json(json!({ "revoked": true })))
}

async fn delete_login(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    change(&state, |config, _| {
        if config.login(&id).is_none() {
            return Err(not_found("essa conta AWS não existe"));
        }
        if config.connections.iter().any(|c| c.login_id == id) {
            return Err(conflict(
                "esse login ainda tem conexões. Remova as conexões antes de remover o login",
            ));
        }
        config.logins.retain(|l| l.id != id);
        Ok(())
    })?;

    state.aws.with(&state.lock, |inner| {
        inner.tokens.remove(&id);
        inner.pending.remove(&id);
    });

    Ok(Json(json!({ "deleted": true })))
}

/// The login's token, refreshed if it ran out and AWS still accepts the refresh token.
async fn token_for(state: &AppState, login: &Login) -> Result<SsoToken, Failure> {
    let held = state
        .aws
        .with(&state.lock, |inner| inner.tokens.get(&login.id).cloned())
        .ok_or_else(locked)?
        .ok_or_else(session_expired)?;

    if !held.is_expired(now_unix()) {
        return Ok(held);
    }

    let sso = state.aws.sso(&login.region).map_err(aws_failure)?;
    let fresh = sso.refresh(&held).await.map_err(|_| session_expired())?;

    state.aws.with(&state.lock, |inner| inner.tokens.insert(login.id.clone(), fresh.clone()));

    Ok(fresh)
}

/// Every account the login can reach, with the roles in each.
async fn list_accounts(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let config = load(&state)?;
    let login = config.login(&id).cloned().ok_or_else(|| not_found("essa conta AWS não existe"))?;

    let token = Arc::new(token_for(&state, &login).await?);
    let sso = Arc::new(state.aws.sso(&login.region).map_err(aws_failure)?);

    let mut accounts = sso.list_accounts(&token).await.map_err(aws_failure)?;
    accounts.sort_by_key(|a| a.account_name.to_lowercase());

    let gate = Arc::new(tokio::sync::Semaphore::new(TAG_CONCURRENCY));
    let mut tasks = tokio::task::JoinSet::new();

    for (index, account) in accounts.iter().enumerate() {
        let (sso, token, gate) = (sso.clone(), token.clone(), gate.clone());
        let account_id = account.account_id.clone();

        tasks.spawn(async move {
            let _permit = gate.acquire_owned().await;
            (index, sso.list_roles(&token, &account_id).await)
        });
    }

    let mut roles: Vec<Vec<String>> = vec![Vec::new(); accounts.len()];
    while let Some(done) = tasks.join_next().await {
        let Ok((index, found)) = done else { continue };
        let mut names: Vec<String> =
            found.map_err(aws_failure)?.into_iter().map(|r| r.role_name).collect();
        names.sort();
        roles[index] = names;
    }

    Ok(Json(json!({
        "accounts": accounts
            .iter()
            .zip(roles)
            .map(|(account, roles)| json!({
                "account_id": account.account_id,
                "account_name": account.account_name,
                "roles": roles,
            }))
            .collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct NewConnection {
    login_id: String,
    account_id: String,
    #[serde(default)]
    account_name: String,
    role: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

fn title_or_default(title: Option<String>, account_id: &str, role: &str) -> String {
    title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| Connection::default_title(account_id, role))
}

async fn create_connection(
    State(state): State<Arc<AppState>>,
    Json(body): Json<NewConnection>,
) -> Answer {
    let account_id = body.account_id.trim().to_string();
    let role = body.role.trim().to_string();

    if account_id.is_empty() || !account_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad_request("o número da conta AWS só tem dígitos"));
    }
    if role.is_empty() {
        return Err(bad_request("escolha a função a usar nessa conta"));
    }

    let made = change(&state, |config, _| {
        if config.login(&body.login_id).is_none() {
            return Err(not_found("esse login AWS não existe"));
        }
        // A3: one account, one connection. Another role is an edit of that connection.
        if config.connections.iter().any(|c| c.account_id == account_id) {
            return Err(conflict(
                "essa conta já tem uma conexão. Para usar outra função, edite a conexão existente",
            ));
        }

        let connection = Connection {
            id: vaults::new_id(),
            login_id: body.login_id.clone(),
            title: title_or_default(body.title.clone(), &account_id, &role),
            account_name: body.account_name.trim().to_string(),
            note: body.note.clone().unwrap_or_default(),
            account_id: account_id.clone(),
            role: role.clone(),
        };
        config.connections.push(connection.clone());
        Ok(connection)
    })?;

    Ok(Json(json!({ "connection": connection_json(&made, Vec::new()) })))
}

#[derive(Deserialize)]
struct EditConnection {
    role: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

async fn update_connection(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<EditConnection>,
) -> Answer {
    let role = body.role.trim().to_string();
    if role.is_empty() {
        return Err(bad_request("escolha a função a usar nessa conta"));
    }

    let (edited, users) = change(&state, |config, vaults| {
        let connection = config
            .connections
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| not_found("essa conexão AWS não existe"))?;

        connection.role = role.clone();
        connection.title = title_or_default(body.title.clone(), &connection.account_id, &role);
        connection.note = body.note.clone().unwrap_or_default();

        Ok((connection.clone(), used_by(&id, vaults)))
    })?;

    // Another role means other credentials, and perhaps another view of the account.
    state.aws.with(&state.lock, |inner| {
        inner.credentials.remove(&id);
        inner.snapshots.clear();
        inner.tags.clear();
    });

    Ok(Json(json!({ "connection": connection_json(&edited, users) })))
}

async fn delete_connection(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    change(&state, |config, vaults| {
        if config.connection(&id).is_none() {
            return Err(not_found("essa conexão AWS não existe"));
        }
        if !used_by(&id, vaults).is_empty() {
            return Err(conflict(
                "um cofre ainda usa essa conexão. Apague o cofre ou troque a conexão dele antes",
            ));
        }
        config.connections.retain(|c| c.id != id);
        Ok(())
    })?;

    state.aws.with(&state.lock, |inner| inner.credentials.remove(&id));

    Ok(Json(json!({ "deleted": true })))
}

// ── one vault's place in AWS ────────────────────────────────────────────────────────────────────

/// Everything a request to an AWS vault needs: where it is, and a client signed for it.
pub(crate) struct Place {
    pub vault_id: String,
    pub vault: AwsVault,
    pub connection: Connection,
    pub ssm: Arc<SsmClient>,
    pub credentials: Credentials,
}

/// The AWS place of a vault, or `None` for a local one. Answers 423 while locked.
pub(crate) fn aws_vault_of(state: &AppState, vault_id: &str) -> Result<Option<AwsVault>, Failure> {
    let store = state.store.lock().unwrap();

    let found = state
        .lock
        .with_key(|key| vaults::read_vault(&store, key, vault_id).ok())
        .ok_or_else(locked)?;

    Ok(found.and_then(|vault| vault.aws.filter(|_| vault.kind == vaults::KIND_AWS)))
}

/// Whether a vault lives in AWS. Anything that goes wrong reading it says no, and the local route
/// that runs instead reports the problem the way it always has.
pub(crate) fn is_aws_vault(state: &AppState, vault_id: &str) -> bool {
    matches!(aws_vault_of(state, vault_id), Ok(Some(_)))
}

async fn place(state: &AppState, vault_id: &str) -> Result<Place, Failure> {
    let (vault, connection, login) = {
        let store = state.store.lock().unwrap();

        state
            .lock
            .with_key(|key| -> Result<_, Failure> {
                let summary = vaults::read_vault(&store, key, vault_id)
                    .map_err(|_| not_found("esse cofre não existe"))?;
                let vault = summary
                    .aws
                    .filter(|_| summary.kind == vaults::KIND_AWS)
                    .ok_or_else(|| bad_request("esse cofre não é um cofre AWS"))?;
                let config = config::load(&store, key).map_err(bad_request)?;
                let connection = config.connection(&vault.connection_id).cloned().ok_or_else(|| {
                    bad_request("a conexão AWS deste cofre não existe mais. Escolha outra nas configurações do cofre")
                })?;
                let login = config.login(&connection.login_id).cloned();
                Ok((vault, connection, login))
            })
            .ok_or_else(locked)??
    };

    let credentials = credentials_for(state, &connection, login.as_ref()).await?;
    let ssm = state.aws.ssm(&vault.region, credentials.clone()).map_err(aws_failure)?;

    Ok(Place { vault_id: vault_id.to_string(), vault, connection, ssm: Arc::new(ssm), credentials })
}

/// The role credentials for a connection: the ones held, while they have five minutes left, or new
/// ones from Identity Center.
async fn credentials_for(
    state: &AppState,
    connection: &Connection,
    login: Option<&Login>,
) -> Result<Credentials, Failure> {
    let now = now_unix();
    let held = state
        .aws
        .with(&state.lock, |inner| inner.credentials.get(&connection.id).cloned())
        .ok_or_else(locked)?;

    if let Some(held) = held.filter(|c| !c.is_expired(now, CREDENTIALS_MARGIN)) {
        return Ok(held);
    }

    let login = login.ok_or_else(session_expired)?;
    let token = token_for(state, login).await?;
    let sso = state.aws.sso(&login.region).map_err(aws_failure)?;

    let fresh = sso
        .role_credentials(&token, &connection.account_id, &connection.role)
        .await
        .map_err(|error| match error {
            // The portal's answer to a role that is gone or not assigned any more.
            AwsError::AccessDenied(_) | AwsError::NotFound(_) => aws_refusal(
                StatusCode::FORBIDDEN,
                "access_denied",
                format!(
                    "o login não pode mais usar a função {} na conta {}. Edite a conexão",
                    connection.role, connection.account_id
                ),
            ),
            other => aws_failure(other),
        })?;

    state
        .aws
        .with(&state.lock, |inner| inner.credentials.insert(connection.id.clone(), fresh.clone()));

    Ok(fresh)
}

/// The vault and parameter (or path, or object key) an AWS id points at.
fn target(id: &str) -> Result<(String, String), Failure> {
    mapping::decode_id(id).ok_or_else(|| not_found("esse registro não existe"))
}

/// The whole vault, read once: GetParametersByPath from `/` (A6), and the description and key of
/// those parameters, and only those, from DescribeParameters.
async fn snapshot(state: &AppState, place: &Place) -> Result<Arc<Snapshot>, Failure> {
    let now = now_unix();
    let cached = state
        .aws
        .with(&state.lock, |inner| inner.snapshots.get(&place.vault_id).cloned())
        .ok_or_else(locked)?;

    if let Some(cached) = cached.filter(|s| now.saturating_sub(s.at) < SNAPSHOT_SECONDS) {
        return Ok(cached);
    }

    let parameters = place.ssm.get_parameters_by_path("/", true).await.map_err(aws_failure)?;

    // Descriptions are titles. A role that may read values but not describe them still gets a
    // list, titled by name.
    let wanted: std::collections::HashSet<&str> = parameters.iter().map(|p| p.name.as_str()).collect();
    let metadata = match place.ssm.describe_parameters("/", true).await {
        Ok(found) => found
            .into_iter()
            .filter(|m| wanted.contains(m.name.as_str()))
            .map(|m| (m.name.clone(), m))
            .collect(),
        Err(_) => HashMap::new(),
    };

    let fresh = Arc::new(Snapshot { at: now, parameters, metadata });
    state.aws.with(&state.lock, |inner| {
        inner.snapshots.insert(place.vault_id.clone(), fresh.clone());
    });

    Ok(fresh)
}

/// The metadata of one parameter: its description, key and tier.
async fn metadata_of(place: &Place, name: &str) -> Option<ParameterMetadata> {
    let folder = mapping::parent_path(name).unwrap_or_else(|| "/".into());

    place
        .ssm
        .describe_parameters(&folder, false)
        .await
        .ok()?
        .into_iter()
        .find(|m| m.name == name)
}

/// Tags for many parameters, from the cache or with a few requests at a time. A parameter whose
/// tags could not be read maps to `None`, and is shown without marks rather than not at all.
async fn tags_for(
    state: &AppState,
    place: &Place,
    names: Vec<String>,
) -> HashMap<String, Option<Vec<Tag>>> {
    let now = now_unix();
    let mut found: HashMap<String, Option<Vec<Tag>>> = HashMap::new();
    let mut missing = Vec::new();

    state.aws.with(&state.lock, |inner| {
        for name in names {
            match inner.tags.get(&format!("{}|{name}", place.vault_id)) {
                Some((at, tags)) if now.saturating_sub(*at) < TAGS_SECONDS => {
                    found.insert(name, Some(tags.clone()));
                }
                _ => missing.push(name),
            }
        }
    });

    let gate = Arc::new(tokio::sync::Semaphore::new(TAG_CONCURRENCY));
    let mut tasks = tokio::task::JoinSet::new();

    for name in missing {
        let (ssm, gate) = (place.ssm.clone(), gate.clone());
        tasks.spawn(async move {
            let _permit = gate.acquire_owned().await;
            let tags = ssm.list_tags_for_resource(&name).await.ok();
            (name, tags)
        });
    }

    let mut fetched = Vec::new();
    while let Some(done) = tasks.join_next().await {
        if let Ok((name, tags)) = done {
            fetched.push((name, tags));
        }
    }

    state.aws.with(&state.lock, |inner| {
        for (name, tags) in &fetched {
            if let Some(tags) = tags {
                inner.tags.insert(format!("{}|{name}", place.vault_id), (now, tags.clone()));
            }
        }
    });

    found.extend(fetched);
    found
}

async fn tags_of(state: &AppState, place: &Place, name: &str) -> Option<Vec<Tag>> {
    tags_for(state, place, vec![name.to_string()]).await.remove(name).flatten()
}

fn tag<'a>(tags: Option<&'a [Tag]>, key: &str) -> Option<&'a str> {
    tags?.iter().find(|t| t.key == key).map(|t| t.value.as_str())
}

/// What the tags say about the format: see `mapping::decode_value`.
fn tagged_json(tags: Option<&[Tag]>) -> Option<bool> {
    tags.map(|tags| {
        tags.iter()
            .any(|t| t.key == mapping::TAG_FORMAT && t.value == mapping::TAG_FORMAT_JSON)
    })
}

fn header_for(
    place: &Place,
    config: &AwsConfig,
    name: &str,
    version: i64,
    last_modified: Option<i64>,
    tags: Option<&[Tag]>,
) -> RecordHeader {
    let when = mapping::iso_from_unix(last_modified.unwrap_or(now_unix() as i64));

    RecordHeader {
        id: encode_id(&place.vault_id, name),
        vault_id: place.vault_id.clone(),
        folder_id: mapping::parent_path(name).map(|p| encode_id(&place.vault_id, &p)),
        kind: "item".into(),
        version,
        favourite: config.is_favourite(&place.connection.account_id, name),
        rotation_due_at: tag(tags, mapping::TAG_ROTATION).map(str::to_string),
        compromised_at: tag(tags, mapping::TAG_COMPROMISED).map(str::to_string),
        archived: false,
        archived_at: None,
        created_at: when.clone(),
        updated_at: when,
    }
}

/// A parameter, opened into the shape a local record has.
async fn open_parameter(
    state: &AppState,
    place: &Place,
    name: &str,
) -> Result<(OpenRecord, Option<ParameterMetadata>, Option<Vec<Tag>>), Failure> {
    let parameter = place.ssm.get_parameter(name).await.map_err(aws_failure)?;
    let metadata = metadata_of(place, name).await;
    let tags = tags_of(state, place, name).await;
    let config = load(state)?;

    let payload = mapping::payload_from(
        name,
        metadata.as_ref().and_then(|m| m.description.as_deref()),
        parameter.value.expose(),
        tagged_json(tags.as_deref()),
    );

    let header = header_for(
        place,
        &config,
        name,
        parameter.version,
        parameter.last_modified,
        tags.as_deref(),
    );

    Ok((OpenRecord { header, payload }, metadata, tags))
}

// ── folders ─────────────────────────────────────────────────────────────────────────────────────

pub(crate) async fn list_folders(state: &AppState, vault_id: &str) -> Answer {
    let place = place(state, vault_id).await?;
    let snapshot = snapshot(state, &place).await?;

    let folders =
        mapping::folder_nodes(vault_id, snapshot.parameters.iter().map(|p| p.name.as_str()));

    Ok(Json(json!({ "folders": folders })))
}

/// A new folder is a `.keep` marker under its path (A7).
pub(crate) async fn create_folder(
    state: &AppState,
    vault_id: &str,
    name: &str,
    parent_id: Option<&str>,
) -> Answer {
    let name = name.trim();
    mapping::validate_title(name).map_err(bad_request)?;
    mapping::validate_segment(name).map_err(bad_request)?;

    let parent = folder_path(vault_id, parent_id)?;
    let path = mapping::join(parent.as_deref(), name);
    let marker = mapping::join(Some(&path), mapping::KEEP);
    mapping::validate_name(&path).map_err(bad_request)?;

    let place = place(state, vault_id).await?;
    let snapshot = snapshot(state, &place).await?;

    if mapping::folder_paths(snapshot.parameters.iter().map(|p| p.name.as_str())).contains(&path) {
        return Err(aws_refusal(StatusCode::CONFLICT, "conflict", "essa pasta já existe"));
    }

    let key_id = mapping::inherit_key(Some(&path), &known_keys(&snapshot));

    place
        .ssm
        .put_parameter(&PutParameter {
            name: marker.clone(),
            value: mapping::KEEP_VALUE.into(),
            key_id,
            description: Some("Pasta criada pelo Keyhold".into()),
            overwrite: false,
            ..PutParameter::default()
        })
        .await
        .map_err(|e| write_failure(e, &marker))?;

    state.aws.invalidate(vault_id);

    let node = mapping::folder_nodes(vault_id, [marker.as_str()])
        .into_iter()
        .find(|n| n.id == encode_id(vault_id, &path))
        .ok_or_else(|| bad_request("a pasta não foi criada"))?;

    Ok(Json(json!({ "folder": node })))
}

/// Delete an empty folder's marker. A folder with anything in it stays (A7).
pub(crate) async fn delete_folder(state: &AppState, folder_id: &str) -> Answer {
    let (vault_id, path) = target(folder_id)?;
    let place = place(state, &vault_id).await?;

    // Read fresh: deleting on a listing ten seconds old could miss what somebody just put there.
    state.aws.invalidate(&vault_id);
    let snapshot = snapshot(state, &place).await?;

    if mapping::folder_has_contents(&path, snapshot.parameters.iter().map(|p| p.name.as_str())) {
        return Err(conflict("essa pasta não está vazia. Mova ou apague o que está nela antes"));
    }

    let marker = mapping::join(Some(&path), mapping::KEEP);
    let exists = snapshot.parameters.iter().any(|p| mapping::absolute(&p.name) == marker);

    if exists {
        let name = snapshot
            .parameters
            .iter()
            .find(|p| mapping::absolute(&p.name) == marker)
            .map(|p| p.name.clone())
            .unwrap_or(marker);
        place.ssm.delete_parameter(&name).await.map_err(aws_failure)?;
    }

    state.aws.invalidate(&vault_id);

    Ok(Json(json!({ "deleted": true })))
}

fn folder_path(vault_id: &str, folder_id: Option<&str>) -> Result<Option<String>, Failure> {
    let Some(folder_id) = folder_id else { return Ok(None) };
    let (owner, path) = target(folder_id)?;

    if owner != vault_id {
        return Err(bad_request("essa pasta é de outro cofre"));
    }

    Ok(Some(path))
}

fn known_keys(snapshot: &Snapshot) -> Vec<(String, Option<String>)> {
    snapshot
        .parameters
        .iter()
        .map(|p| (p.name.clone(), snapshot.metadata.get(&p.name).and_then(|m| m.key_id.clone())))
        .collect()
}

/// A write AWS refused, in words that name what was being written (A10).
fn write_failure(error: AwsError, name: &str) -> Failure {
    match error {
        AwsError::AccessDenied(_) => aws_refusal(
            StatusCode::FORBIDDEN,
            "access_denied",
            format!(
                "a função desta conexão não pode escrever em {name}. Na AWS, ela precisa de \
                 ssm:PutParameter nesse caminho e de kms:Encrypt na chave"
            ),
        ),
        AwsError::VersionConflict(_) => aws_refusal(
            StatusCode::CONFLICT,
            "conflict",
            format!("já existe um parâmetro chamado {name}. Escolha outro nome"),
        ),
        other => aws_failure(other),
    }
}

// ── records ─────────────────────────────────────────────────────────────────────────────────────

pub(crate) async fn list_records(state: &AppState, vault_id: &str) -> Answer {
    let place = place(state, vault_id).await?;
    let snapshot = snapshot(state, &place).await?;
    let config = load(state)?;

    let visible: Vec<&Parameter> =
        snapshot.parameters.iter().filter(|p| !mapping::is_keep(&p.name)).collect();
    let tags = tags_for(state, &place, visible.iter().map(|p| p.name.clone()).collect()).await;

    let mut previews: Vec<RecordPreview> = visible
        .into_iter()
        .map(|parameter| {
            let tags = tags.get(&parameter.name).cloned().flatten();
            let payload = mapping::payload_from(
                &parameter.name,
                snapshot.metadata.get(&parameter.name).and_then(|m| m.description.as_deref()),
                parameter.value.expose(),
                tagged_json(tags.as_deref()),
            );

            RecordPreview {
                header: header_for(
                    &place,
                    &config,
                    &parameter.name,
                    parameter.version,
                    parameter.last_modified,
                    tags.as_deref(),
                ),
                subtitle: payload.subtitle(),
                url: payload.first_url(),
                title: payload.title,
                icon: payload.icon,
            }
        })
        .collect();

    previews.sort_by(|a, b| b.header.updated_at.cmp(&a.header.updated_at));

    Ok(Json(json!({ "records": previews })))
}

/// The record a body describes, checked against what AWS can hold.
fn prepared(payload: &Payload) -> Result<(Payload, String, String, bool), Failure> {
    payload.check().map_err(bad_request)?;
    let payload = payload.clone().normalise();

    let description = mapping::join_description(&payload.title, &payload.notes).map_err(bad_request)?;
    let (value, json) = mapping::encode_value(&payload).map_err(bad_request)?;

    Ok((payload, description, value, json))
}

pub(crate) async fn create_record(
    state: &AppState,
    vault_id: &str,
    folder_id: Option<&str>,
    payload: &Payload,
    name: Option<&str>,
) -> Answer {
    let (_, description, value, json) = prepared(payload)?;
    let folder = folder_path(vault_id, folder_id)?;

    let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(full) if full.starts_with('/') => full.to_string(),
        Some(leaf) => mapping::join(folder.as_deref(), leaf),
        None => mapping::join(folder.as_deref(), &mapping::suggest_name(&payload.title)),
    };
    mapping::validate_name(&name).map_err(bad_request)?;

    let place = place(state, vault_id).await?;
    let snapshot = snapshot(state, &place).await?;
    let key_id = mapping::inherit_key(mapping::parent_path(&name).as_deref(), &known_keys(&snapshot));

    let mut tags = Vec::new();
    if json {
        tags.push(Tag::new(mapping::TAG_FORMAT, mapping::TAG_FORMAT_JSON));
    }

    let version = place
        .ssm
        .put_parameter(&PutParameter {
            name: name.clone(),
            advanced: value.len() > mapping::STANDARD_LIMIT,
            value: value.into(),
            key_id,
            description: Some(description),
            overwrite: false,
            tags: tags.clone(),
        })
        .await
        .map_err(|e| write_failure(e, &name))?;

    state.aws.invalidate(vault_id);

    let config = load(state)?;
    let header = header_for(&place, &config, &name, version, None, Some(&tags));
    note(state, audit::action::RECORD_CREATED, &header.id, Detail::new().with("kind", "item"));

    Ok(Json(json!({ "record": header })))
}

pub(crate) async fn open_record(state: &AppState, id: &str) -> Answer {
    let (vault_id, name) = target(id)?;
    let place = place(state, &vault_id).await?;
    let (opened, _, _) = open_parameter(state, &place, &name).await?;

    Ok(Json(json!({ "record": opened })))
}

/// Write a new version over an existing parameter, keeping its key and tier, and switching the
/// format tag when the value changed format (A8). With `expected`, refuse if AWS has moved on (A12).
async fn write_over(
    state: &AppState,
    place: &Place,
    name: &str,
    payload: &Payload,
    expected: Option<i64>,
) -> Result<RecordHeader, Failure> {
    let (_, description, value, json) = prepared(payload)?;

    let current = place.ssm.get_parameter(name).await.map_err(aws_failure)?;

    if let Some(expected) = expected.filter(|v| *v != current.version) {
        return Err(aws_refusal(
            StatusCode::CONFLICT,
            "conflict",
            format!(
                "alguém salvou esse registro na AWS enquanto você editava (você abriu a versão \
                 {expected}, e a atual é a {}). Recarregue para ver a mudança antes de salvar",
                current.version
            ),
        ));
    }

    let metadata = metadata_of(place, name).await;
    let tags = place.ssm.list_tags_for_resource(name).await.ok();
    let was_json = tagged_json(tags.as_deref()).unwrap_or(false);

    let version = place
        .ssm
        .put_parameter(&PutParameter {
            name: name.to_string(),
            advanced: value.len() > mapping::STANDARD_LIMIT
                || metadata.as_ref().and_then(|m| m.tier.as_deref()) == Some("Advanced"),
            value: value.into(),
            // Overwriting without a key would move the parameter to the account's default key.
            key_id: metadata.as_ref().and_then(|m| m.key_id.clone()),
            description: Some(description),
            overwrite: true,
            tags: Vec::new(),
        })
        .await
        .map_err(|e| write_failure(e, name))?;

    let mut tags = tags.unwrap_or_default();
    if json && !was_json {
        let format = Tag::new(mapping::TAG_FORMAT, mapping::TAG_FORMAT_JSON);
        place.ssm.add_tags_to_resource(name, std::slice::from_ref(&format)).await.map_err(aws_failure)?;
        tags.push(format);
    } else if !json && was_json {
        place
            .ssm
            .remove_tags_from_resource(name, &[mapping::TAG_FORMAT.to_string()])
            .await
            .map_err(aws_failure)?;
        tags.retain(|t| t.key != mapping::TAG_FORMAT);
    }

    state.aws.invalidate(&place.vault_id);

    let config = load(state)?;
    Ok(header_for(place, &config, name, version, None, Some(&tags)))
}

/// Save a record. `expected` is the version the person opened, from the body's
/// `expected_version` or the `x-keyhold-expected-version` header; without it the save is not
/// checked (A12 needs to know what was opened to know whether it changed).
pub(crate) async fn update_record(
    state: &AppState,
    id: &str,
    payload: &Payload,
    expected: Option<i64>,
) -> Answer {
    let (vault_id, name) = target(id)?;
    let place = place(state, &vault_id).await?;
    let header = write_over(state, &place, &name, payload, expected).await?;

    note(state, audit::action::RECORD_UPDATED, id, Detail::new().with("version", header.version));

    Ok(Json(json!({ "record": header })))
}

/// Move a parameter to another folder: write it under the new name with its tags and key, then
/// delete the old one. Parameter Store has no rename, and this is the order that never loses it.
pub(crate) async fn move_record(
    state: &AppState,
    id: &str,
    folder_id: Option<&str>,
    to_vault: Option<&str>,
) -> Answer {
    let (vault_id, name) = target(id)?;

    if to_vault.is_some_and(|to| to != vault_id) {
        return Err(bad_request(
            "mover registros entre cofres AWS e locais ainda não é possível. Crie o registro no \
             outro cofre e apague este",
        ));
    }

    let folder = folder_path(&vault_id, folder_id)?;
    let new_name = mapping::join(folder.as_deref(), mapping::leaf(&name));
    mapping::validate_name(&new_name).map_err(bad_request)?;

    if mapping::absolute(&new_name) == mapping::absolute(&name) {
        return Ok(Json(json!({ "moved": true, "id": id })));
    }

    let place = place(state, &vault_id).await?;
    let parameter = place.ssm.get_parameter(&name).await.map_err(aws_failure)?;
    let metadata = metadata_of(&place, &name).await;
    let tags: Vec<Tag> = place
        .ssm
        .list_tags_for_resource(&name)
        .await
        .map_err(aws_failure)?
        .into_iter()
        .filter(|t| !t.key.starts_with("aws:"))
        .collect();

    place
        .ssm
        .put_parameter(&PutParameter {
            name: new_name.clone(),
            advanced: parameter.value.expose().len() > mapping::STANDARD_LIMIT
                || metadata.as_ref().and_then(|m| m.tier.as_deref()) == Some("Advanced"),
            value: parameter.value.clone(),
            key_id: metadata.as_ref().and_then(|m| m.key_id.clone()),
            description: metadata.as_ref().and_then(|m| m.description.clone()),
            overwrite: false,
            tags,
        })
        .await
        .map_err(|e| write_failure(e, &new_name))?;

    place.ssm.delete_parameter(&name).await.map_err(aws_failure)?;

    let account = place.connection.account_id.clone();
    change(state, |config, _| {
        if config.is_favourite(&account, &name) {
            config.set_favourite(&account, &name, false);
            config.set_favourite(&account, &new_name, true);
        }
        Ok(())
    })?;

    state.aws.invalidate(&vault_id);

    let new_id = encode_id(&vault_id, &new_name);
    note(state, audit::action::RECORD_MOVED, &new_id, Detail::new().with("from", id));

    Ok(Json(json!({ "moved": true, "id": new_id })))
}

/// Delete for good (A16). There is no trash in Parameter Store, and pretending otherwise would
/// mean keeping a copy here.
pub(crate) async fn delete_record(state: &AppState, id: &str, on: bool) -> Answer {
    if !on {
        return Err(bad_request("cofres AWS não têm lixeira: o que foi apagado não volta"));
    }

    let (vault_id, name) = target(id)?;
    let place = place(state, &vault_id).await?;

    place.ssm.delete_parameter(&name).await.map_err(aws_failure)?;

    let account = place.connection.account_id.clone();
    change(state, |config, _| {
        config.set_favourite(&account, &name, false);
        Ok(())
    })?;

    state.aws.invalidate(&vault_id);
    note(state, audit::action::RECORD_DESTROYED, id, Detail::new());

    Ok(Json(json!({ "archived": true, "destroyed": true })))
}

/// Favourites are personal, so they stay here (A14).
pub(crate) async fn set_favourite(state: &AppState, id: &str, on: bool) -> Answer {
    let (vault_id, name) = target(id)?;
    let vault = aws_vault_of(state, &vault_id)?.ok_or_else(|| not_found("esse cofre não existe"))?;

    change(state, |config, _| {
        let account = config
            .connection(&vault.connection_id)
            .map(|c| c.account_id.clone())
            .ok_or_else(|| bad_request("a conexão AWS deste cofre não existe mais"))?;
        config.set_favourite(&account, &name, on);
        Ok(())
    })?;

    Ok(Json(json!({ "favourite": on })))
}

async fn set_tag(state: &AppState, id: &str, key: &str, value: Option<String>) -> Result<(), Failure> {
    let (vault_id, name) = target(id)?;
    let place = place(state, &vault_id).await?;

    match value {
        Some(value) => place.ssm.add_tags_to_resource(&name, &[Tag::new(key, value)]).await,
        None => place.ssm.remove_tags_from_resource(&name, &[key.to_string()]).await,
    }
    .map_err(|e| write_failure(e, &name))?;

    state.aws.invalidate(&vault_id);

    Ok(())
}

pub(crate) async fn set_compromised(state: &AppState, id: &str, on: bool) -> Answer {
    let when = on.then(|| mapping::iso_from_unix(now_unix() as i64));
    set_tag(state, id, mapping::TAG_COMPROMISED, when).await?;

    Ok(Json(json!({ "compromised": on })))
}

pub(crate) async fn set_rotation(state: &AppState, id: &str, due: Option<&str>) -> Answer {
    let due = due
        .filter(|d| !d.trim().is_empty())
        .map(|d| mapping::rotation_date(d.trim()))
        .transpose()
        .map_err(bad_request)?;

    set_tag(state, id, mapping::TAG_ROTATION, due.clone()).await?;

    Ok(Json(json!({ "rotation_due_at": due })))
}

// ── history (A15) ───────────────────────────────────────────────────────────────────────────────

async fn history(place: &Place, name: &str) -> Result<Vec<crate::aws::ParameterHistoryEntry>, Failure> {
    let mut entries = place.ssm.get_parameter_history(name).await.map_err(aws_failure)?;
    entries.sort_by_key(|e| e.version);

    // Parameter Store keeps the last hundred; so does this, whatever a fake or a future API says.
    let excess = entries.len().saturating_sub(100);
    entries.drain(..excess);

    Ok(entries)
}

/// The versions before the current one, newest first, the way the local history lists them.
pub(crate) async fn list_versions(state: &AppState, id: &str) -> Answer {
    let (vault_id, name) = target(id)?;
    let place = place(state, &vault_id).await?;
    let mut entries = history(&place, &name).await?;

    entries.pop();
    let versions: Vec<VersionEntry> = entries
        .into_iter()
        .rev()
        .map(|e| VersionEntry {
            version: e.version,
            created_at: mapping::iso_from_unix(e.last_modified.unwrap_or(0)),
        })
        .collect();

    Ok(Json(json!({ "versions": versions })))
}

/// An old version's payload. Tags are not versioned, so the format is read from the value's shape.
async fn version_payload(place: &Place, name: &str, version: i64) -> Result<Payload, Failure> {
    let entry = history(place, name)
        .await?
        .into_iter()
        .find(|e| e.version == version)
        .ok_or_else(|| not_found("essa versão não existe mais na AWS"))?;

    Ok(mapping::payload_from(name, entry.description.as_deref(), entry.value.expose(), None))
}

pub(crate) async fn read_version(state: &AppState, id: &str, version: i64) -> Answer {
    let (vault_id, name) = target(id)?;
    let place = place(state, &vault_id).await?;
    let payload = version_payload(&place, &name, version).await?;

    Ok(Json(json!({ "payload": payload })))
}

/// Bring an old version back by writing it as a new one: AWS's history only grows.
pub(crate) async fn restore_version(state: &AppState, id: &str, version: i64) -> Answer {
    let (vault_id, name) = target(id)?;
    let place = place(state, &vault_id).await?;
    let payload = version_payload(&place, &name, version).await?;
    let header = write_over(state, &place, &name, &payload, None).await?;

    note(
        state,
        audit::action::RECORD_UPDATED,
        id,
        Detail::new().with("version", header.version).with("restored", version),
    );

    Ok(Json(json!({ "record": header })))
}

pub(crate) async fn totp_code(state: &AppState, id: &str, index: usize) -> Answer {
    let (vault_id, name) = target(id)?;
    let place = place(state, &vault_id).await?;
    let (opened, _, _) = open_parameter(state, &place, &name).await?;

    let field = opened
        .payload
        .fields
        .get(index)
        .filter(|f| f.kind == "totp")
        .ok_or_else(|| bad_request("esse campo não é um código 2FA"))?;

    let (code, remaining) =
        crate::totp::code_for_field(&field.value, crate::identity::now_seconds())
            .ok_or_else(|| bad_request("o segredo desse campo não é um 2FA válido"))?;

    Ok(Json(json!({ "code": code, "remaining": remaining })))
}

// ── attachments (A18) ───────────────────────────────────────────────────────────────────────────

/// Where a parameter's files go: `<name without the leading slash>/`.
fn prefix_of(name: &str) -> String {
    format!("{}/", name.trim_start_matches('/'))
}

fn bucket_of(place: &Place) -> Result<String, Failure> {
    place.vault.bucket.clone().ok_or_else(|| {
        bad_request("este cofre AWS não tem um bucket S3 para anexos. Defina um nas configurações do cofre")
    })
}

fn attachment_of(vault_id: &str, record_id: &str, key: &str, size: u64, at: &str) -> Attachment {
    let name = mapping::leaf(key).to_string();

    Attachment {
        id: encode_id(vault_id, key),
        record_id: record_id.to_string(),
        mime: mime_guess::from_path(&name).first_or_octet_stream().to_string(),
        name,
        size: size as i64,
        created_at: at.to_string(),
    }
}

pub(crate) async fn list_attachments(state: &AppState, record_id: &str) -> Answer {
    let (vault_id, name) = target(record_id)?;
    let place = place(state, &vault_id).await?;

    let Some(bucket) = place.vault.bucket.clone() else {
        return Ok(Json(json!({ "attachments": [], "max_bytes": crate::attachments::MAX_BYTES })));
    };

    let s3 = state.aws.s3(&place.vault.region, place.credentials.clone()).map_err(aws_failure)?;
    let prefix = prefix_of(&name);

    let found: Vec<Attachment> = s3
        .list_objects_v2(&bucket, &prefix)
        .await
        .map_err(|e| s3_failure(e, &bucket, &place.vault.region))?
        .into_iter()
        .filter(|o| o.key.len() > prefix.len() && !o.key[prefix.len()..].contains('/'))
        .map(|o| attachment_of(&vault_id, record_id, &o.key, o.size, &o.last_modified))
        .collect();

    Ok(Json(json!({ "attachments": found, "max_bytes": crate::attachments::MAX_BYTES })))
}

/// Upload a file next to its parameter, encrypted with the parameter's own key (A18). A parameter
/// on the account's default key (`alias/aws/ssm`, which S3 cannot use) gets S3's default, `aws/s3`.
pub(crate) async fn add_attachment(
    state: &AppState,
    record_id: &str,
    file_name: &str,
    mime: &str,
    bytes: &[u8],
) -> Answer {
    let file_name = file_name.trim();
    if file_name.is_empty() {
        return Err(bad_request("o arquivo precisa de um nome"));
    }
    if file_name.contains('/') || file_name == "." || file_name == ".." {
        return Err(bad_request("o nome do arquivo não pode ter '/'"));
    }
    if bytes.is_empty() {
        return Err(bad_request("o arquivo está vazio"));
    }
    if bytes.len() > crate::attachments::MAX_BYTES {
        return Err(bad_request(format!(
            "o arquivo tem {:.1} MB; o limite é {} MB",
            bytes.len() as f64 / (1024.0 * 1024.0),
            crate::attachments::MAX_BYTES / (1024 * 1024)
        )));
    }

    let (vault_id, name) = target(record_id)?;
    let place = place(state, &vault_id).await?;
    let bucket = bucket_of(&place)?;

    let key_id = metadata_of(&place, &name)
        .await
        .and_then(|m| m.key_id)
        .filter(|k| k != mapping::DEFAULT_SSM_KEY);

    let s3 = state.aws.s3(&place.vault.region, place.credentials.clone()).map_err(aws_failure)?;
    let key = format!("{}{file_name}", prefix_of(&name));

    s3.put_object(&bucket, &key, bytes, key_id.as_deref(), Some(mime))
        .await
        .map_err(|e| s3_failure(e, &bucket, &place.vault.region))?;

    let added = attachment_of(
        &vault_id,
        record_id,
        &key,
        bytes.len() as u64,
        &mapping::iso_from_unix(now_unix() as i64),
    );

    note(
        state,
        audit::action::RECORD_UPDATED,
        record_id,
        Detail::new().with("attachment", added.name.clone()).with("bytes", added.size),
    );

    Ok(Json(json!({ "attachment": added })))
}

pub(crate) async fn open_attachment(state: &AppState, id: &str) -> Answer {
    let (vault_id, key) = target(id)?;
    let place = place(state, &vault_id).await?;
    let bucket = bucket_of(&place)?;
    let s3 = state.aws.s3(&place.vault.region, place.credentials.clone()).map_err(aws_failure)?;

    let object = s3
        .get_object(&bucket, &key)
        .await
        .map_err(|e| s3_failure(e, &bucket, &place.vault.region))?;

    let name = mapping::leaf(&key).to_string();

    let log_reads = {
        let store = state.store.lock().unwrap();
        crate::settings::log_reads(&store)
    };
    if log_reads {
        note(state, audit::action::SECRET_REVEALED, id, Detail::new().with("attachment", name.clone()));
    }

    Ok(Json(json!({
        "name": name,
        "mime": object
            .content_type
            .unwrap_or_else(|| mime_guess::from_path(&name).first_or_octet_stream().to_string()),
        "data": crate::encoding::base64url(object.body.expose()),
    })))
}

pub(crate) async fn delete_attachment(state: &AppState, id: &str) -> Answer {
    let (vault_id, key) = target(id)?;
    let place = place(state, &vault_id).await?;
    let bucket = bucket_of(&place)?;
    let s3 = state.aws.s3(&place.vault.region, place.credentials.clone()).map_err(aws_failure)?;

    s3.delete_object(&bucket, &key)
        .await
        .map_err(|e| s3_failure(e, &bucket, &place.vault.region))?;

    Ok(Json(json!({ "deleted": true })))
}

// ── health check ────────────────────────────────────────────────────────────────────────────────

/// Can this vault read its parameters, and reach its bucket? For the vault's settings page.
async fn check_vault(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    require_unlocked(&state)?;
    let place = place(&state, &id).await?;

    let (parameters, parameters_message) = match place.ssm.get_parameters_by_path("/", false).await {
        Ok(_) => (true, None),
        Err(AwsError::AccessDenied(_)) => (
            false,
            Some("a função desta conexão não pode ler parâmetros (falta ssm:GetParametersByPath)".to_string()),
        ),
        Err(other) => return Err(aws_failure(other)),
    };

    let (bucket, bucket_message) = match place.vault.bucket.as_deref() {
        None => (Value::Null, None),
        Some(bucket) => {
            let s3 = state.aws.s3(&place.vault.region, place.credentials.clone()).map_err(aws_failure)?;
            match s3.head_bucket(bucket).await {
                Ok(()) => (json!("ok"), None),
                Err(AwsError::NotFound(_)) => (
                    json!("missing"),
                    Some(format!(
                        "o bucket {bucket} não existe na região {}. Crie-o na AWS: o Keyhold não cria buckets",
                        place.vault.region
                    )),
                ),
                Err(AwsError::AccessDenied(_)) | Err(AwsError::Invalid(_)) => (
                    json!("denied"),
                    Some(format!(
                        "sem acesso ao bucket {bucket} (ou ele está em outra região). A função \
                         precisa de s3:ListBucket, s3:GetObject, s3:PutObject e s3:DeleteObject"
                    )),
                ),
                Err(other) => return Err(aws_failure(other)),
            }
        }
    };

    let message = match (parameters_message, bucket_message) {
        (None, None) => "tudo certo: o Keyhold lê os parâmetros desta conta".to_string(),
        (Some(a), None) | (None, Some(a)) => a,
        (Some(a), Some(b)) => format!("{a}. Além disso, {b}"),
    };

    Ok(Json(json!({ "parameters": parameters, "bucket": bucket, "message": message })))
}

#[cfg(test)]
mod tests;
