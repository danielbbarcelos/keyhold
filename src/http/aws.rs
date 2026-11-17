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
//!
//! **Fail closed on what decides protection.** A write over an existing parameter has to name its
//! KMS key, or Parameter Store moves it to the account's default key; a new record inherits its
//! folder's key; an attachment is encrypted with its record's key. When the key cannot be read
//! (DescribeParameters denied and history denied too, or the listing's metadata failed), the write
//! is refused with a sentence saying why, never sent with a guess.
//!
//! **A write that half-happened says so.** Some operations are two requests (a move is a put and
//! a delete; a format switch is a put and a tag). When the second fails after the first landed,
//! the answer is a success with a `warning` in Portuguese for the interface to show, and the
//! caches are cleared on every path, errors included, so the next read sees what is really there.

use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

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
use crate::aws::mapping::{self, attachment_id, folder_id, record_id, IdKind};
use crate::aws::state::{Pending, Snapshot, SNAPSHOT_SECONDS, TAGS_SECONDS};
use crate::aws::{
    now_unix, AwsError, Credentials, Parameter, ParameterMetadata, Poll, PutParameter, S3Client,
    SsmClient, SsoToken, Tag,
};
use crate::records::{OpenRecord, Payload, RecordHeader, RecordPreview, VersionEntry};
use crate::tr;
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
        .route("/aws/vaults/:id/trail", get(vault_trail))
        .route("/aws/vaults/:id/audit/retry", post(retry_audit))
}

// ── errors ──────────────────────────────────────────────────────────────────────────────────────

/// An `AwsError` as the interface hears it.
pub(crate) fn aws_failure(error: AwsError) -> Failure {
    let (status, kind, text) = match &error {
        AwsError::SessionExpired(_) => (
            StatusCode::FAILED_DEPENDENCY,
            "session_expired",
            tr!("a sessão da AWS expirou. Renove o login para continuar"),
        ),
        AwsError::AccessDenied(detail) => (
            StatusCode::FORBIDDEN,
            "access_denied",
            tr!("a AWS negou o acesso ao papel desta conta ({detail})", detail = detail),
        ),
        AwsError::NotFound(_) => (
            StatusCode::NOT_FOUND,
            "not_found",
            tr!("o item não existe mais na AWS. Ele pode ter sido excluído"),
        ),
        AwsError::VersionConflict(_) => (
            StatusCode::CONFLICT,
            "conflict",
            tr!("o item foi alterado na AWS depois de aberto. Recarregue antes de salvar"),
        ),
        AwsError::Throttled(_) => (
            StatusCode::TOO_MANY_REQUESTS,
            "throttled",
            tr!("a AWS limitou o ritmo das requisições. Tente novamente em alguns segundos"),
        ),
        AwsError::Network(_) => (
            StatusCode::BAD_GATEWAY,
            "network",
            tr!("a AWS não respondeu. Confira a conexão com a internet"),
        ),
        AwsError::Invalid(detail) => (
            StatusCode::BAD_REQUEST,
            "invalid",
            tr!("a AWS recusou o pedido: {detail}", detail = detail),
        ),
        AwsError::TooLarge { size, limit } => (
            StatusCode::BAD_REQUEST,
            "invalid",
            tr!(
                "o arquivo tem {size} MB, acima do limite de {limit} MB aceito pelo Keyhold",
                size = format!("{:.1}", *size as f64 / (1024.0 * 1024.0)),
                limit = limit / (1024 * 1024)
            ),
        ),
        AwsError::Other { code, .. } => (
            StatusCode::BAD_GATEWAY,
            "other",
            tr!("a AWS respondeu com um erro inesperado ({code})", code = code),
        ),
    };

    Failure(status, text, Some(kind))
}

/// A refusal of ours that is still about AWS, for the interface to show the same way.
fn aws_refusal(status: StatusCode, kind: &'static str, text: impl Into<String>) -> Failure {
    Failure(status, text.into(), Some(kind))
}

fn not_found(text: impl Into<String>) -> Failure {
    aws_refusal(StatusCode::NOT_FOUND, "not_found", text)
}

fn session_expired() -> Failure {
    aws_failure(AwsError::SessionExpired("no token".into()))
}

/// What an S3 error means for a vault's bucket, naming the bucket and the permissions (A18).
fn s3_failure(error: AwsError, bucket: &str, region: &str) -> Failure {
    match error {
        AwsError::AccessDenied(message) => {
            // S3 says which policy refused (an explicit deny in the bucket policy, a missing allow in
            // the role) when the request is from the same account: worth more than our guess.
            eprintln!("  s3      access denied on {bucket}: {message}");
            let mut text = tr!(
                "sem permissão no bucket {bucket}. O papel desta conta precisa de \
                 s3:ListBucket, s3:GetObject, s3:PutObject e s3:DeleteObject nele, e de \
                 kms:GenerateDataKey e kms:Decrypt na chave dos parâmetros", bucket = bucket
            );
            let detail = message.trim();
            if !detail.is_empty() && !detail.eq_ignore_ascii_case("access denied") {
                text = format!("{text}. AWS: {detail}");
            }
            aws_refusal(StatusCode::FORBIDDEN, "access_denied", text)
        }
        AwsError::NotFound(message) if message.to_lowercase().contains("bucket") => aws_refusal(
            StatusCode::NOT_FOUND,
            "not_found",
            tr!(
                "o bucket {bucket} não existe na região {region}. Crie o bucket na AWS, pois o \
                 Keyhold não cria buckets", bucket = bucket, region = region
            ),
        ),
        AwsError::NotFound(_) => not_found(tr!("o arquivo não existe mais no bucket")),
        AwsError::Invalid(message) if message.contains("region") => aws_refusal(
            StatusCode::BAD_REQUEST,
            "invalid",
            tr!("o bucket {bucket} está em outra região, e não em {region}", bucket = bucket, region = region),
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

/// How long before an access token runs out Keyhold trades its refresh token for a new one.
const RENEW_AHEAD_SECONDS: u64 = 10 * 60;

/// Renew, ahead of time, every session about to run out that can be renewed. Called by the
/// overview the interface reads to draw the rail, so a session lives as long as Identity Center
/// allows (its own "session duration") while Keyhold is unlocked, and not a second past a lock:
/// locking drops the tokens, refresh tokens included. A refusal is left alone here: the session
/// ends when the access token does, and the rail says so.
async fn renew_ahead(state: &AppState, config: &AwsConfig) {
    let Some(generation) = state.aws.generation(&state.lock) else { return };
    let now = now_unix();

    for login in &config.logins {
        let due = state
            .aws
            .with_generation(&state.lock, generation, |inner| {
                inner
                    .tokens
                    .get(&login.id)
                    .filter(|t| t.renews() && t.expires_at() <= now.saturating_add(RENEW_AHEAD_SECONDS))
                    .cloned()
            })
            .flatten();

        let Some(held) = due else { continue };
        let Ok(sso) = state.aws.sso(&login.region) else { continue };

        if let Ok(fresh) = sso.refresh(&held).await {
            state.aws.with_generation(&state.lock, generation, |inner| {
                inner.tokens.insert(login.id.clone(), fresh);
            });
        }
    }
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

    renew_ahead(&state, &config).await;

    let sessions: HashMap<String, (u64, bool)> = state
        .aws
        .with(&state.lock, |inner| {
            inner.tokens.iter().map(|(id, token)| (id.clone(), (token.expires_at(), token.renews()))).collect()
        })
        .ok_or_else(locked)?;

    Ok(Json(json!({
        "logins": config
            .logins
            .iter()
            .map(|login| {
                let session = sessions.get(&login.id).copied();
                let mut json = login_json(login, session.map(|(at, _)| at));
                json["renews"] = json!(session.is_some_and(|(_, renews)| renews));
                json
            })
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
        return Err(bad_request(tr!("o endereço do portal precisa começar com https://")));
    }
    crate::aws::validate_region(&region)
        .map_err(|_| bad_request(tr!("'{region}' não é uma região da AWS", region = region)))?;

    let config = load(&state)?;
    let generation = state.aws.generation(&state.lock).ok_or_else(locked)?;

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

    device_login(&state, login, known, generation).await
}

/// Register, start the device authorization, and remember it until it is approved, unless the
/// vault locked while AWS was answering.
async fn device_login(state: &AppState, login: Login, known: bool, generation: u64) -> Answer {
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
        .with_generation(&state.lock, generation, |inner| {
            inner.pending.insert(login.id.clone(), Pending { login, device, known });
        })
        .ok_or_else(locked)?;

    Ok(Json(answer))
}

/// Ask AWS once whether the person approved the login. The interface calls this every `interval`.
async fn poll_login(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let generation = state.aws.generation(&state.lock).ok_or_else(locked)?;
    let waiting = state
        .aws
        .with_generation(&state.lock, generation, |inner| {
            let device = inner.pending.get(&id).map(|p| (p.device.clone(), p.login.clone(), p.known));
            (device, inner.tokens.contains_key(&id))
        })
        .ok_or_else(locked)?;

    let (device, login, known) = match waiting {
        (Some(found), _) => found,
        (None, true) => return Ok(Json(json!({ "state": "done" }))),
        (None, false) => return Err(not_found(tr!("nenhum login em andamento para esta conta"))),
    };

    let sso = state.aws.sso(&login.region).map_err(aws_failure)?;
    let outcome = sso.poll_device_login(&device).await.map_err(aws_failure)?;

    let answer = match outcome {
        Poll::Pending => "pending",
        Poll::SlowDown => {
            state.aws.with_generation(&state.lock, generation, |inner| {
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
            // A token approved for an unlock that is over is dropped here, not kept for the next.
            state
                .aws
                .with_generation(&state.lock, generation, |inner| {
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
    // "Renovar" probes the trail again: a policy or a log group may have been added meanwhile.
    state.aws.with(&state.lock, |inner| {
        inner.audit.clear();
        inner.callers.clear();
        inner.bucket_settings.clear();
    });

    let config = load(&state)?;
    let login = config.login(&id).cloned().ok_or_else(|| not_found(tr!("o login SSO não existe mais")))?;
    let generation = state.aws.generation(&state.lock).ok_or_else(locked)?;

    let held = state
        .aws
        .with_generation(&state.lock, generation, |inner| inner.tokens.get(&id).cloned())
        .ok_or_else(locked)?;

    if let Some(token) = held.filter(|t| t.refresh_token().is_some()) {
        let sso = state.aws.sso(&login.region).map_err(aws_failure)?;

        if let Ok(fresh) = sso.refresh(&token).await {
            let expires_at = fresh.expires_at();
            state
                .aws
                .with_generation(&state.lock, generation, |inner| inner.tokens.insert(id.clone(), fresh))
                .ok_or_else(locked)?;

            return Ok(Json(json!({ "login": login_json(&login, Some(expires_at)), "device": null })));
        }
    }

    device_login(&state, login, true, generation).await
}

/// Log out of Identity Center. The login stays listed, disconnected (A4).
async fn revoke_login(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let config = load(&state)?;
    let login = config.login(&id).cloned().ok_or_else(|| not_found(tr!("o login SSO não existe mais")))?;

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

/// Remove a login. A token still held is revoked on AWS's side first, as `revoke` does: removing
/// the login from the list must not leave a live Identity Center session behind it.
async fn delete_login(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let removed = change(&state, |config, _| {
        let login = config.login(&id).cloned().ok_or_else(|| not_found(tr!("o login SSO não existe mais")))?;
        if config.connections.iter().any(|c| c.login_id == id) {
            return Err(conflict(
                tr!("o login ainda possui contas. Remova as contas antes de remover o login"),
            ));
        }
        config.logins.retain(|l| l.id != id);
        Ok(login)
    })?;

    let token = state.aws.with(&state.lock, |inner| {
        inner.pending.remove(&id);
        inner.tokens.remove(&id)
    });

    if let Some(token) = token.flatten() {
        if let Ok(sso) = state.aws.sso(&removed.region) {
            let _ = sso.logout(&token).await;
        }
    }

    Ok(Json(json!({ "deleted": true })))
}

/// The login's token, refreshed if it ran out and AWS still accepts the refresh token. A refresh
/// that finishes after the vault locked is not stored.
async fn token_for(state: &AppState, login: &Login, generation: u64) -> Result<SsoToken, Failure> {
    let held = state
        .aws
        .with_generation(&state.lock, generation, |inner| inner.tokens.get(&login.id).cloned())
        .ok_or_else(locked)?
        .ok_or_else(session_expired)?;

    if !held.is_expired(now_unix()) {
        return Ok(held);
    }

    let sso = state.aws.sso(&login.region).map_err(aws_failure)?;
    let fresh = sso.refresh(&held).await.map_err(|_| session_expired())?;

    state
        .aws
        .with_generation(&state.lock, generation, |inner| {
            inner.tokens.insert(login.id.clone(), fresh.clone())
        })
        .ok_or_else(locked)?;

    Ok(fresh)
}

/// Every account the login can reach, with the roles in each.
async fn list_accounts(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let config = load(&state)?;
    let login = config.login(&id).cloned().ok_or_else(|| not_found(tr!("o login SSO não existe mais")))?;
    let generation = state.aws.generation(&state.lock).ok_or_else(locked)?;

    let token = Arc::new(token_for(&state, &login, generation).await?);
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
        return Err(bad_request(tr!("o número da conta AWS deve conter apenas dígitos")));
    }
    if role.is_empty() {
        return Err(bad_request(tr!("escolha o papel a usar nesta conta")));
    }

    let made = change(&state, |config, _| {
        if config.login(&body.login_id).is_none() {
            return Err(not_found(tr!("o login SSO não existe mais")));
        }
        // A3: one account, one connection. Another role is an edit of that connection.
        if config.connections.iter().any(|c| c.account_id == account_id) {
            return Err(conflict(
                tr!("esta conta já foi adicionada. Para usar outro papel, edite a conta existente"),
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
        return Err(bad_request(tr!("escolha o papel a usar nesta conta")));
    }

    let (edited, users) = change(&state, |config, vaults| {
        let connection = config
            .connections
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| not_found(tr!("a conta AWS não existe mais")))?;

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
            return Err(not_found(tr!("a conta AWS não existe mais")));
        }
        if !used_by(&id, vaults).is_empty() {
            return Err(conflict(
                tr!("um cofre ainda usa esta conta. Exclua o cofre ou troque a conta dele antes"),
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
    // Which unlock this request belongs to. Credentials fetched below are only kept if the vault
    // is still in this one when they arrive.
    let generation = state.aws.generation(&state.lock).ok_or_else(locked)?;

    let (vault, connection, login) = {
        let store = state.store.lock().unwrap();

        state
            .lock
            .with_key(|key| -> Result<_, Failure> {
                let summary = vaults::read_vault(&store, key, vault_id)
                    .map_err(|_| not_found(tr!("o cofre não existe mais")))?;
                let vault = summary
                    .aws
                    .filter(|_| summary.kind == vaults::KIND_AWS)
                    .ok_or_else(|| bad_request(tr!("o cofre não é um cofre AWS")))?;
                let config = config::load(&store, key).map_err(bad_request)?;
                let connection = config.connection(&vault.connection_id).cloned().ok_or_else(|| {
                    bad_request(tr!("a conta AWS deste cofre não existe mais. Conecte a conta de novo em Contas AWS e crie o cofre outra vez"))
                })?;
                let login = config.login(&connection.login_id).cloned();
                Ok((vault, connection, login))
            })
            .ok_or_else(locked)??
    };

    let credentials = credentials_for(state, &connection, login.as_ref(), generation).await?;
    let ssm = state.aws.ssm(&vault.region, credentials.clone()).map_err(aws_failure)?;

    Ok(Place { vault_id: vault_id.to_string(), vault, connection, ssm: Arc::new(ssm), credentials })
}

/// The role credentials for a connection: the ones held, while they have five minutes left, or new
/// ones from Identity Center.
async fn credentials_for(
    state: &AppState,
    connection: &Connection,
    login: Option<&Login>,
    generation: u64,
) -> Result<Credentials, Failure> {
    let now = now_unix();
    let held = state
        .aws
        .with_generation(&state.lock, generation, |inner| inner.credentials.get(&connection.id).cloned())
        .ok_or_else(locked)?;

    if let Some(held) = held.filter(|c| !c.is_expired(now, CREDENTIALS_MARGIN)) {
        return Ok(held);
    }

    let login = login.ok_or_else(session_expired)?;
    let token = token_for(state, login, generation).await?;
    let sso = state.aws.sso(&login.region).map_err(aws_failure)?;

    let fresh = sso
        .role_credentials(&token, &connection.account_id, &connection.role)
        .await
        .map_err(|error| match error {
            // The portal's answer to a role that is gone or not assigned any more.
            AwsError::AccessDenied(_) | AwsError::NotFound(_) => aws_refusal(
                StatusCode::FORBIDDEN,
                "access_denied",
                tr!(
                    "o login não pode mais usar o papel {role} na conta {account}. Edite a conta",
                    role = connection.role,
                    account = connection.account_id
                ),
            ),
            other => aws_failure(other),
        })?;

    // Credentials for an unlock that is over are dropped here, and the request refused with them.
    state
        .aws
        .with_generation(&state.lock, generation, |inner| {
            inner.credentials.insert(connection.id.clone(), fresh.clone())
        })
        .ok_or_else(locked)?;

    Ok(fresh)
}

/// An S3 client for the vault's bucket that refuses a bucket of any account but the connection's
/// (`x-amz-expected-bucket-owner` on every request).
fn s3_of(state: &AppState, place: &Place) -> Result<S3Client, Failure> {
    state
        .aws
        .s3(&place.vault.region, place.credentials.clone(), &place.connection.account_id)
        .map_err(aws_failure)
}

/// The vault and target of an id of the given kind. Any other kind, or an id that is not AWS's,
/// is "not found": a folder id handed to a record route never reaches a parameter.
fn target_of(id: &str, kind: IdKind, missing: String) -> Result<(String, String), Failure> {
    match mapping::decode_id(id) {
        Some((found, vault, target)) if found == kind => Ok((vault, target)),
        _ => Err(not_found(missing)),
    }
}

fn target_record(id: &str) -> Result<(String, String), Failure> {
    target_of(id, IdKind::Record, tr!("o registro não existe"))
}

fn target_folder(id: &str) -> Result<(String, String), Failure> {
    target_of(id, IdKind::Folder, tr!("a pasta não existe"))
}

fn target_attachment(id: &str) -> Result<(String, String), Failure> {
    target_of(id, IdKind::Attachment, tr!("o anexo não existe"))
}

/// The whole vault, read once: GetParametersByPath from `/` (A6), and the description and key of
/// those parameters, and only those, from DescribeParameters.
///
/// A role that may not call DescribeParameters (one given a single branch of the account) gets
/// the same from each parameter's history instead. When neither can be read there is still a
/// listing, titled by name, with the failure kept in `metadata_error` so that nothing that needs a
/// key decides from an empty map. Such a snapshot is not cached: the next request tries again
/// rather than living with a throttle for ten seconds.
async fn snapshot(state: &AppState, place: &Place) -> Result<Arc<Snapshot>, Failure> {
    let now = now_unix();
    let cached = state
        .aws
        .with(&state.lock, |inner| inner.snapshots.get(&place.vault_id).cloned())
        .ok_or_else(locked)?;

    if let Some(cached) = cached.filter(|s| now.saturating_sub(s.at) < SNAPSHOT_SECONDS) {
        return Ok(cached);
    }

    let path = place.vault.listing_path();
    let parameters = place
        .ssm
        .get_parameters_by_path(path, true)
        .await
        .map_err(|error| listing_failure(error, path))?;

    let wanted: std::collections::HashSet<&str> = parameters.iter().map(|p| p.name.as_str()).collect();
    let (metadata, metadata_error) = match place.ssm.describe_parameters(path, true).await {
        Ok(found) => (
            found
                .into_iter()
                .filter(|m| wanted.contains(m.name.as_str()))
                .map(|m| (m.name.clone(), m))
                .collect(),
            None,
        ),
        // A role given one branch of the account cannot call DescribeParameters at all: AWS only
        // grants it for the whole account. What it can read is each parameter's own history, and
        // the newest entry there carries the same description and key. Without this, such a vault
        // would show parameter names where the titles and the folder labels belong.
        Err(AwsError::AccessDenied(detail)) => {
            let (found, failed) = metadata_from_history(state, place, &parameters).await;

            (found, failed.then(|| AwsError::AccessDenied(detail)))
        }
        Err(error) => (HashMap::new(), Some(error)),
    };

    let complete = metadata_error.is_none();
    let fresh = Arc::new(Snapshot { at: now, parameters, metadata, metadata_error });

    if complete {
        state.aws.with(&state.lock, |inner| {
            inner.snapshots.insert(place.vault_id.clone(), fresh.clone());
        });
    }

    Ok(fresh)
}

/// How many histories are read at once. Enough to open a vault of a few hundred parameters in a
/// couple of seconds, well under what Parameter Store allows an account per second.
const HISTORY_AT_ONCE: usize = 8;

/// The description, key and tier of every parameter of a listing, read from each one's history.
///
/// Only the parameters whose version changed since the last time are asked about: a history is one
/// request per parameter, and a vault is read again every minute. The answer says whether anything
/// could not be read, in which case what was read is still used for titles, and the listing is
/// treated as incomplete by everything that needs a key.
async fn metadata_from_history(
    state: &AppState,
    place: &Place,
    parameters: &[Parameter],
) -> (HashMap<String, ParameterMetadata>, bool) {
    let cache_key = |name: &str| format!("{}|{name}", place.vault_id);

    let known: HashMap<String, ParameterMetadata> = state
        .aws
        .with(&state.lock, |inner| {
            parameters
                .iter()
                .filter_map(|p| {
                    let kept = inner.history_metadata.get(&cache_key(&p.name))?;
                    (kept.version == p.version).then(|| (p.name.clone(), kept.clone()))
                })
                .collect()
        })
        .unwrap_or_default();

    let limit = Arc::new(tokio::sync::Semaphore::new(HISTORY_AT_ONCE));
    let mut reading = tokio::task::JoinSet::new();
    for parameter in parameters.iter().filter(|p| !known.contains_key(&p.name)) {
        let (ssm, limit, name) = (place.ssm.clone(), limit.clone(), parameter.name.clone());
        reading.spawn(async move {
            let _turn = limit.acquire().await;

            ssm.latest_history(&name).await
        });
    }

    let mut found = known;
    let mut fresh = Vec::new();
    let mut failed = false;
    while let Some(read) = reading.join_next().await {
        match read {
            Ok(Ok(Some(entry))) => fresh.push(entry.metadata()),
            // Gone between the listing and now: nothing to say about it.
            Ok(Ok(None)) | Ok(Err(AwsError::NotFound(_))) => {}
            _ => failed = true,
        }
    }

    state.aws.with(&state.lock, |inner| {
        // What is no longer in the vault is forgotten with it.
        let prefix = cache_key("");
        let present: std::collections::HashSet<String> = parameters.iter().map(|p| cache_key(&p.name)).collect();
        inner.history_metadata.retain(|key, _| !key.starts_with(&prefix) || present.contains(key));

        for metadata in &fresh {
            inner.history_metadata.insert(cache_key(&metadata.name), metadata.clone());
        }
    });

    found.extend(fresh.into_iter().map(|metadata| (metadata.name.clone(), metadata)));

    (found, failed)
}

/// A listing AWS refused. The role answered, so this is never "no answer", and renewing the
/// session would not change it: the vault starts somewhere the role may not read. A role given one
/// branch of the account (a client's `/shared`) needs the vault to start there.
fn listing_failure(error: AwsError, path: &str) -> Failure {
    match error {
        AwsError::AccessDenied(detail) => aws_refusal(
            StatusCode::FORBIDDEN,
            "listing_denied",
            if path == "/" {
                tr!(
                    "o papel desta conta não pode listar a conta inteira, a partir de /. Se o seu \
                     acesso é a uma pasta só (como /shared), crie um cofre novo informando essa \
                     pasta em Caminho na AWS ({detail})", detail = detail
                )
            } else {
                tr!(
                    "o papel desta conta não pode listar a partir de {path}. Crie um cofre novo com \
                     o Caminho na AWS que o seu acesso permite ({detail})", path = path, detail = detail
                )
            },
        ),
        other => aws_failure(other),
    }
}

/// Why the key, tier or description of a parameter could not be read, as a refusal.
fn metadata_failure(error: AwsError, name: &str) -> Failure {
    match error {
        AwsError::AccessDenied(_) => aws_refusal(
            StatusCode::FORBIDDEN,
            "access_denied",
            tr!(
                "nada foi gravado. Não foi possível ler a chave KMS de {name}: o papel desta conta \
                 precisa de ssm:DescribeParameters ou de ssm:GetParameterHistory. Sem a chave, \
                 gravar trocaria a proteção do segredo pela chave padrão da conta", name = name
            ),
        ),
        other => aws_failure(other),
    }
}

/// The metadata of one parameter: its description, key and tier.
///
/// DescribeParameters by exact name first; when that is refused or finds nothing, the newest
/// entry of the parameter's history, which carries the same key, description and tier. When
/// neither can be read the answer is an error, never `None`: every caller is about to write, and
/// writing without the key would move the parameter to the account's default key.
async fn metadata_of(place: &Place, name: &str) -> Result<ParameterMetadata, Failure> {
    let described = match place.ssm.describe_parameter(name).await {
        Ok(Some(found)) => return Ok(found),
        Ok(None) => None,
        Err(error) => Some(error),
    };

    match place.ssm.latest_history(name).await {
        Ok(Some(entry)) => Ok(entry.metadata()),
        Ok(None) => Err(not_found(tr!("o registro não existe mais na AWS"))),
        Err(AwsError::NotFound(message)) if described.is_none() => {
            Err(aws_failure(AwsError::NotFound(message)))
        }
        Err(history_error) => Err(metadata_failure(described.unwrap_or(history_error), name)),
    }
}

/// The key an object next to a parameter is encrypted with in S3: the parameter's own, as an ARN, or
/// `None` (S3's default, `aws/s3`) for a parameter on the account's default `alias/aws/ssm`, which S3
/// cannot use.
fn s3_key(place: &Place, metadata: &ParameterMetadata, name: &str) -> Result<Option<String>, Failure> {
    let key = key_of(metadata, name)?;
    if key == mapping::DEFAULT_SSM_KEY || key.ends_with(":alias/aws/ssm") {
        return Ok(None);
    }

    Ok(Some(mapping::kms_arn(&key, &place.vault.region, &place.connection.account_id)))
}

/// The KMS key a SecureString was written with. AWS always reports one; if it did not, the write
/// is refused rather than sent without one.
fn key_of(metadata: &ParameterMetadata, name: &str) -> Result<String, Failure> {
    metadata.key_id.clone().filter(|k| !k.is_empty()).ok_or_else(|| {
        aws_refusal(
            StatusCode::BAD_GATEWAY,
            "other",
            tr!(
                "nada foi gravado. A AWS não informou com qual chave KMS {name} está protegido, e \
                 gravar sem essa informação trocaria a proteção do segredo", name = name
            ),
        )
    })
}

/// Whether Parameter Store keeps this type encrypted.
/// A `String` parameter holding a JSON object reads as one password field per key (read only, so
/// nothing is ever written back in that shape). See [`mapping::plaintext_fields`].
fn split_plaintext(payload: &mut Payload, kind: &str, value: &str) {
    if is_secure(kind) {
        return;
    }
    if let Some(fields) = mapping::plaintext_fields(value) {
        payload.fields = fields;
    }
}

fn is_secure(kind: &str) -> bool {
    kind == "SecureString"
}

/// A `String` or `StringList` parameter is not overwritten: Keyhold only writes SecureString, and
/// changing the type under an application that reads it as plain text could break it.
fn refuse_plaintext(kind: &str, name: &str) -> Result<(), Failure> {
    if is_secure(kind) {
        return Ok(());
    }

    Err(aws_refusal(
        StatusCode::BAD_REQUEST,
        "invalid",
        tr!(
            "{name} é um parâmetro {kind}, armazenado sem criptografia. O Keyhold grava apenas \
             SecureString, e trocar o tipo pode quebrar as aplicações que leem esse parâmetro. \
             Se necessário, mude o tipo na própria AWS", name = name, kind = kind
        ),
    ))
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
        id: record_id(&place.vault_id, name),
        vault_id: place.vault_id.clone(),
        folder_id: mapping::folder_of(name, place.vault.root()).map(|p| folder_id(&place.vault_id, &p)),
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

/// A JSON object with `warning` added when there is one to give.
fn with_warning(mut answer: Value, warning: Option<String>) -> Value {
    if let Some(warning) = warning {
        answer["warning"] = json!(warning);
    }
    answer
}

/// A record, or a preview, with `plaintext` set: whether the parameter is a `String` or
/// `StringList` that Parameter Store keeps unencrypted (and that Keyhold will not overwrite).
fn with_plaintext(record: impl serde::Serialize, kind: &str) -> Value {
    let mut value = serde_json::to_value(record).unwrap_or(Value::Null);
    if let Some(object) = value.as_object_mut() {
        object.insert("plaintext".into(), json!(!is_secure(kind)));
    }
    value
}

/// A parameter, opened into the shape a local record has, with its type.
///
/// Reading is lenient where writing is not: a role that may read the value but neither describe
/// nor read history still sees the record, titled by its name. Saving it then fails closed in
/// [`write_over`], which needs the key.
async fn open_parameter(
    state: &AppState,
    place: &Place,
    name: &str,
) -> Result<(OpenRecord, String), Failure> {
    let parameter = place.ssm.get_parameter(name).await.map_err(aws_failure)?;
    let metadata = metadata_of(place, name).await.ok();
    let tags = tags_of(state, place, name).await;
    let config = load(state)?;

    let mut payload = mapping::payload_from(
        name,
        metadata.as_ref().and_then(|m| m.description.as_deref()),
        parameter.value.expose(),
        tagged_json(tags.as_deref()),
    );
    split_plaintext(&mut payload, &parameter.kind, parameter.value.expose());

    let header = header_for(
        place,
        &config,
        name,
        parameter.version,
        parameter.last_modified,
        tags.as_deref(),
    );

    Ok((OpenRecord { header, payload }, parameter.kind.clone()))
}

// ── folders ─────────────────────────────────────────────────────────────────────────────────────

pub(crate) async fn list_folders(state: &AppState, vault_id: &str) -> Answer {
    let place = place(state, vault_id).await?;
    let snapshot = snapshot(state, &place).await?;

    let folders = mapping::folder_nodes_in(
        vault_id,
        place.vault.root(),
        snapshot.parameters.iter().map(|p| p.name.as_str()),
        &folder_labels(&snapshot),
    );

    Ok(Json(json!({ "folders": folders })))
}

/// The names folder markers carry in their descriptions, path → name. Empty when the metadata
/// could not be read: the tree then shows path segments, which is still the truth.
fn folder_labels(snapshot: &Snapshot) -> HashMap<String, String> {
    snapshot
        .metadata
        .iter()
        .filter(|(name, _)| mapping::is_keep(name))
        .filter_map(|(name, meta)| {
            let path = mapping::parent_path(name)?;
            Some((path, mapping::folder_label(meta.description.as_deref())?))
        })
        .collect()
}

/// The segment a folder gets: the one typed in "Editar nome", or one made from its name.
fn folder_segment(label: &str, segment: Option<&str>) -> Result<String, Failure> {
    let segment = match segment.map(str::trim).filter(|s| !s.is_empty()) {
        Some(typed) => typed.to_string(),
        None => mapping::suggest_name(label),
    };
    mapping::validate_segment(&segment).map_err(bad_request)?;

    Ok(segment)
}

/// A folder's name as its marker's description: one line, no `/`, within what AWS accepts.
fn folder_description(label: &str) -> Result<String, Failure> {
    if label.trim().is_empty() {
        return Err(bad_request(tr!("a pasta precisa de um nome")));
    }
    if label.contains('/') {
        return Err(bad_request(tr!("o nome da pasta não pode conter '/', pois na AWS a barra cria mais uma pasta")));
    }

    mapping::description_for(label).map_err(bad_request)
}

/// Write the folder's name on its marker: over the marker when it exists (same key), or as a new
/// marker under the folder's key otherwise.
async fn write_folder_label(
    state: &AppState,
    place: &Place,
    snapshot: &Snapshot,
    path: &str,
    description: &str,
    exists: bool,
) -> Result<(), Failure> {
    let marker = mapping::join(Some(path), mapping::KEEP);

    let key_id = if exists {
        let metadata = metadata_of(place, &marker).await?;
        Some(key_of(&metadata, &marker)?)
    } else {
        mapping::inherit_key(Some(path), &known_keys(snapshot)?)
    };

    let written = place
        .ssm
        .put_parameter(&PutParameter {
            name: marker.clone(),
            value: mapping::KEEP_VALUE.into(),
            key_id,
            description: Some(description.to_string()),
            overwrite: exists,
            ..PutParameter::default()
        })
        .await;

    state.aws.invalidate(&place.vault_id);
    written.map(|_| ()).map_err(|e| write_failure(e, &marker))
}

/// Give a folder that "Copiar para…" created its name in Keyhold, when the path segment could not
/// carry it ("Bancos (PJ)" at `/bancos-pj`). A marker that already has a name keeps it.
pub(crate) async fn ensure_folder_label(state: &AppState, vault_id: &str, path: &str, label: &str) -> Result<(), Failure> {
    let place = place(state, vault_id).await?;
    let listing = snapshot(state, &place).await?;
    let marker = mapping::join(Some(path), mapping::KEEP);

    if folder_labels(&listing).contains_key(path) {
        return Ok(());
    }

    let exists = listing.parameters.iter().any(|p| mapping::absolute(&p.name) == marker);
    let description = folder_description(label)?;

    write_folder_label(state, &place, &listing, path, &description, exists).await
}

/// What moving a folder's path came to.
enum Relocated {
    Moved { target: String, warning: Option<String> },
    /// Nothing was moved: these cannot be (a `String` parameter, or one with attachments).
    Blocked(String),
}

/// Move every parameter under `path` to `new_path`, keys and tags and description (the folder's
/// name, on its marker) included; each one starts its history over. Checked before anything is
/// written: a `String` parameter is never rewritten, and one with attachments would leave its files
/// behind, so either refuses the whole move. A failure half way stops and says what moved.
async fn relocate_folder(
    state: &AppState,
    place: &Place,
    listing: &Snapshot,
    path: &str,
    new_path: &str,
) -> Result<Relocated, Failure> {
    mapping::validate_name(new_path).map_err(bad_request)?;

    let paths = mapping::folder_paths(listing.parameters.iter().map(|p| p.name.as_str()));
    if paths.contains(new_path) {
        return Err(conflict(tr!("já existe uma pasta {new_path} na AWS", new_path = new_path)));
    }

    let prefix = format!("{}/", path.trim_end_matches('/'));
    let inside: Vec<&crate::aws::Parameter> = listing
        .parameters
        .iter()
        .filter(|p| mapping::absolute(&p.name).starts_with(&prefix))
        .collect();

    let mut blocked = Vec::new();
    for parameter in &inside {
        if !is_secure(&parameter.kind) {
            blocked.push(tr!("{name} (parâmetro String)", name = mapping::leaf(&parameter.name)));
            continue;
        }
        if mapping::is_keep(&parameter.name) {
            continue;
        }
        match attachments_in(state, place, &parameter.name).await {
            Ok(Some((_, _, keys))) if !keys.is_empty() => {
                blocked.push(tr!("{name} (tem anexos)", name = mapping::leaf(&parameter.name)))
            }
            Ok(_) => {}
            Err(Ok(failure)) => return Err(because(&tr!("a pasta não foi movida, pois não foi possível conferir os anexos"), failure)),
            Err(Err(failure)) => return Err(failure),
        }
    }

    if !blocked.is_empty() {
        return Ok(Relocated::Blocked(blocked.join(", ")));
    }

    let mut warning = None;
    let mut moved = 0;
    for parameter in &inside {
        let old = mapping::absolute(&parameter.name);
        let new_name = format!("{new_path}{}", &old[path.trim_end_matches('/').len()..]);
        let old_id = record_id(&place.vault_id, &parameter.name);

        match move_parameter(state, place, &old_id, &parameter.name, &new_name).await {
            Ok(Json(answer)) => {
                moved += 1;
                if let Some(text) = answer.get("warning").and_then(Value::as_str) {
                    warning = Some(text.to_string());
                }
            }
            Err(failure) => {
                state.aws.invalidate(&place.vault_id);
                if moved == 0 {
                    return Err(failure);
                }
                // Half way: both folders exist now. Say so, and name the new one.
                warning = Some(tr!(
                    "{moved} de {total} parâmetros foram movidos para {new_path} antes da interrupção ({why}). O restante permanece em {path}",
                    moved = moved,
                    total = inside.len(),
                    new_path = new_path,
                    why = failure.1,
                    path = path
                ));
                break;
            }
        }
    }

    state.aws.invalidate(&place.vault_id);
    Ok(Relocated::Moved { target: new_path.to_string(), warning })
}

/// Move a folder under another one, or to the root: its path changes, and so does the name of
/// every parameter under it (see [`relocate_folder`]). Its name in Keyhold travels on its marker.
pub(crate) async fn move_folder(state: &AppState, id: &str, parent_id: Option<&str>) -> Answer {
    let (vault_id, path) = target_folder(id)?;
    let parent = folder_path(state, &vault_id, parent_id)?;

    if parent.as_deref().is_some_and(|p| p == path || p.starts_with(&format!("{}/", path.trim_end_matches('/')))) {
        return Err(bad_request(tr!("uma pasta não pode ser movida para dentro dela mesma")));
    }

    let new_path = mapping::join(parent.as_deref(), mapping::leaf(&path));
    let place = place(state, &vault_id).await?;

    state.aws.invalidate(&vault_id);
    let listing = snapshot(state, &place).await?;
    let label = folder_labels(&listing).get(&path).cloned();

    if !mapping::folder_paths(listing.parameters.iter().map(|p| p.name.as_str())).contains(&path) {
        return Err(not_found(tr!("a pasta não existe mais")));
    }

    if mapping::absolute(&new_path) == mapping::absolute(&path) {
        return Ok(Json(json!({ "moved": true, "id": id })));
    }

    match relocate_folder(state, &place, &listing, &path, &new_path).await? {
        Relocated::Blocked(blocked) => Err(conflict(tr!(
            "a pasta não foi movida, pois estes itens não podem mudar de nome na AWS: {blocked}", blocked = blocked
        ))),
        Relocated::Moved { target, warning } => {
            let node = mapping::folder_node(&vault_id, place.vault.root(), &target, label);
            Ok(Json(with_warning(json!({ "moved": true, "id": node.id, "folder": node }), warning)))
        }
    }
}

/// Rename a folder (F9, A7 revisited): its name in Keyhold always, and its path in AWS only when
/// `segment` asks for it.
///
/// The name lives in the marker's description, so renaming it touches one parameter and nothing
/// else. A new path is a move of every parameter under the folder (Parameter Store has no rename),
/// each one losing its history. It is refused before anything is written when a parameter under
/// the folder is a `String` (never rewritten) or has attachments (they live under the old name):
/// then only the name changes, and the answer says why the path did not.
pub(crate) async fn rename_folder(state: &AppState, id: &str, label: &str, segment: Option<&str>) -> Answer {
    let (vault_id, path) = target_folder(id)?;
    let label = label.trim();
    let description = folder_description(label)?;
    let place = place(state, &vault_id).await?;

    state.aws.invalidate(&vault_id);
    let listing = snapshot(state, &place).await?;
    let names: Vec<String> = listing.parameters.iter().map(|p| p.name.clone()).collect();
    let paths = mapping::folder_paths(names.iter().map(String::as_str));

    if !paths.contains(&path) {
        return Err(not_found(tr!("a pasta não existe mais")));
    }

    let mut target = path.clone();
    let mut warning: Option<String> = None;

    let wanted = segment.map(str::trim).filter(|s| !s.is_empty() && *s != mapping::leaf(&path));

    if let Some(wanted) = wanted {
        mapping::validate_segment(wanted).map_err(bad_request)?;
        let new_path = mapping::join(mapping::parent_path(&path).as_deref(), wanted);

        match relocate_folder(state, &place, &listing, &path, &new_path).await? {
            Relocated::Moved { target: moved_to, warning: said } => {
                target = moved_to;
                warning = said;
            }
            Relocated::Blocked(blocked) => {
                warning = Some(tr!(
                    "apenas o nome no Keyhold mudou. O caminho na AWS permanece {path}, pois estes itens não podem ser movidos: {blocked}", path = path, blocked = blocked
                ));
            }
        }
    }

    let fresh = snapshot(state, &place).await?;
    let exists = fresh
        .parameters
        .iter()
        .any(|p| mapping::absolute(&p.name) == mapping::join(Some(&target), mapping::KEEP));
    write_folder_label(state, &place, &fresh, &target, &description, exists).await?;

    let node = mapping::folder_node(&vault_id, place.vault.root(), &target, Some(label.to_string()));

    Ok(Json(with_warning(json!({ "folder": node }), warning)))
}

/// A new folder is a `.keep` marker under its path (A7).
pub(crate) async fn create_folder(
    state: &AppState,
    vault_id: &str,
    label: &str,
    segment: Option<&str>,
    parent_id: Option<&str>,
) -> Answer {
    let label = label.trim();
    let description = folder_description(label)?;
    let name = folder_segment(label, segment)?;

    let parent = folder_path(state, vault_id, parent_id)?;
    let path = mapping::join(parent.as_deref(), &name);
    let marker = mapping::join(Some(&path), mapping::KEEP);
    mapping::validate_name(&path).map_err(bad_request)?;

    let place = place(state, vault_id).await?;
    let snapshot = snapshot(state, &place).await?;

    if mapping::folder_paths(snapshot.parameters.iter().map(|p| p.name.as_str())).contains(&path) {
        return Err(aws_refusal(StatusCode::CONFLICT, "conflict", tr!("a pasta já existe")));
    }

    let key_id = mapping::inherit_key(Some(&path), &known_keys(&snapshot)?);

    let written = place
        .ssm
        .put_parameter(&PutParameter {
            name: marker.clone(),
            value: mapping::KEEP_VALUE.into(),
            key_id,
            description: Some(description),
            overwrite: false,
            ..PutParameter::default()
        })
        .await;

    state.aws.invalidate(vault_id);
    written.map_err(|e| write_failure(e, &marker))?;

    let node = mapping::folder_node(vault_id, place.vault.root(), &path, Some(label.to_string()));

    Ok(Json(json!({ "folder": node })))
}

/// Delete an empty folder's marker. A folder with anything in it stays (A7).
pub(crate) async fn delete_folder(state: &AppState, id: &str) -> Answer {
    let (vault_id, path) = target_folder(id)?;
    let place = place(state, &vault_id).await?;

    // Read fresh: deleting on a listing ten seconds old could miss what somebody just put there.
    state.aws.invalidate(&vault_id);
    let snapshot = snapshot(state, &place).await?;

    if mapping::folder_has_contents(&path, snapshot.parameters.iter().map(|p| p.name.as_str())) {
        return Err(conflict(tr!("a pasta não está vazia. Mova ou exclua o conteúdo antes")));
    }

    let marker = mapping::join(Some(&path), mapping::KEEP);
    let found = snapshot
        .parameters
        .iter()
        .find(|p| mapping::absolute(&p.name) == marker)
        .map(|p| p.name.clone());

    if let Some(name) = found {
        let deleted = place.ssm.delete_parameter(&name).await;
        state.aws.invalidate(&vault_id);
        deleted.map_err(aws_failure)?;
    }

    Ok(Json(json!({ "deleted": true })))
}

/// The path a folder id stands for, or the vault's root when there is none: what "no folder"
/// means in a vault that starts at `/shared` is `/shared`, and in one of the whole account, `/`.
pub(crate) fn folder_path(state: &AppState, vault_id: &str, id: Option<&str>) -> Result<Option<String>, Failure> {
    let root = aws_vault_of(state, vault_id)?.and_then(|vault| vault.root);

    let Some(id) = id else { return Ok(root) };
    let (owner, path) = target_folder(id)?;

    if owner != vault_id {
        return Err(bad_request(tr!("a pasta pertence a outro cofre")));
    }
    if !mapping::under_root(&path, root.as_deref()) {
        return Err(not_found(tr!("a pasta não está neste cofre")));
    }

    Ok(Some(path))
}

/// Every name the listing has, with its key, for [`mapping::inherit_key`]. Refused when the
/// listing's metadata could not be read: an empty map would make every new record inherit the
/// account's default key, silently, in a folder that uses a key of its own.
fn known_keys(snapshot: &Snapshot) -> Result<Vec<(String, Option<String>)>, Failure> {
    if let Some(error) = &snapshot.metadata_error {
        return Err(match error {
            AwsError::AccessDenied(_) => aws_refusal(
                StatusCode::FORBIDDEN,
                "access_denied",
                tr!(
                    "nada foi gravado. Não foi possível ler as chaves KMS desta conta (o papel precisa \
                     de ssm:DescribeParameters), e sem elas não há como saber com qual chave proteger o \
                     que seria criado nesta pasta"
                ),
            ),
            other => aws_failure(other.clone()),
        });
    }

    Ok(snapshot
        .parameters
        .iter()
        .map(|p| (p.name.clone(), snapshot.metadata.get(&p.name).and_then(|m| m.key_id.clone())))
        .collect())
}

/// A write AWS refused, in words that name what was being written (A10).
fn write_failure(error: AwsError, name: &str) -> Failure {
    match error {
        AwsError::AccessDenied(_) => aws_refusal(
            StatusCode::FORBIDDEN,
            "access_denied",
            tr!(
                "o papel desta conta não pode gravar em {name}. Na AWS, ele precisa de \
                 ssm:PutParameter nesse caminho e de kms:Encrypt na chave", name = name
            ),
        ),
        AwsError::VersionConflict(_) => aws_refusal(
            StatusCode::CONFLICT,
            "conflict",
            tr!("já existe um parâmetro chamado {name}. Escolha outro nome", name = name),
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

    let mut previews: Vec<(RecordPreview, &str)> = visible
        .into_iter()
        .map(|parameter| {
            let tags = tags.get(&parameter.name).cloned().flatten();
            let mut payload = mapping::payload_from(
                &parameter.name,
                snapshot.metadata.get(&parameter.name).and_then(|m| m.description.as_deref()),
                parameter.value.expose(),
                tagged_json(tags.as_deref()),
            );
            split_plaintext(&mut payload, &parameter.kind, parameter.value.expose());

            let preview = RecordPreview {
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
            };

            (preview, parameter.kind.as_str())
        })
        .collect();

    previews.sort_by(|a, b| b.0.header.updated_at.cmp(&a.0.header.updated_at));

    let records: Vec<Value> =
        previews.into_iter().map(|(preview, kind)| with_plaintext(preview, kind)).collect();

    let names: Vec<String> = snapshot.parameters.iter().map(|p| p.name.clone()).collect();
    let audit = probe(state, &place, &names).await;

    Ok(Json(json!({ "records": records, "audit": audit })))
}

// ── the audit trail (CloudWatch Logs) ───────────────────────────────────────────────────────────
//
// See `aws::audit` for the rules and `docs/AWS.md` for the account side. A sensitive action
// (revealing, copying, moving, deleting, downloading, copying out) writes its event before it
// happens and does not happen when the write fails for a reason other than "the account has no
// trail here". Viewing and saving queue their events and never wait on CloudWatch.

/// Whether an action waits for its event.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Weight {
    Sensitive,
    Background,
}

fn now_ms() -> i64 {
    (crate::identity::now_seconds() as i64) * 1000
}

/// The administrator's settings on the vault's bucket, read once per unlock.
async fn bucket_settings(state: &AppState, place: &Place) -> crate::aws::audit::BucketSettings {
    use crate::aws::audit::BucketSettings;

    let cached = state.aws.with(&state.lock, |inner| inner.bucket_settings.get(&place.vault_id).cloned()).flatten();
    if let Some(settings) = cached {
        return settings;
    }

    let settings = match (bucket_of(place), s3_of(state, place)) {
        (Ok(bucket), Ok(s3)) => match s3.get_bucket_tagging(&bucket).await {
            Ok(tags) => BucketSettings::from_tags(&tags),
            Err(_) => BucketSettings::unreadable(),
        },
        _ => BucketSettings::unreadable(),
    };

    state.aws.with(&state.lock, |inner| inner.bucket_settings.insert(place.vault_id.clone(), settings.clone()));
    settings
}

/// The person behind the connection's credentials: their audit stream's name.
async fn caller(state: &AppState, place: &Place) -> Result<String, AwsError> {
    let cached = state.aws.with(&state.lock, |inner| inner.callers.get(&place.connection.id).cloned()).flatten();
    if let Some(name) = cached {
        return Ok(name);
    }

    let sts = state.aws.sts(&place.vault.region, place.credentials.clone())?;
    let caller = sts.get_caller_identity().await?;
    let name = caller
        .session_name()
        .map(str::to_string)
        .ok_or_else(|| AwsError::Invalid(tr!("estas credenciais não são de uma sessão do Identity Center")))?;

    state.aws.with(&state.lock, |inner| inner.callers.insert(place.connection.id.clone(), name.clone()));
    Ok(name)
}

fn trail_unavailable(reason: &str) -> Failure {
    aws_refusal(
        StatusCode::SERVICE_UNAVAILABLE,
        "audit_unavailable",
        tr!("a ação não foi feita porque a trilha da AWS não pôde ser gravada: {reason}. Tente de novo", reason = reason),
    )
}

fn trail_required(reason: &str) -> Failure {
    aws_refusal(
        StatusCode::FORBIDDEN,
        "audit_required",
        tr!("este cofre exige a trilha da AWS, e ela não está disponível: {reason}", reason = reason),
    )
}

fn set_status(state: &AppState, key: &str, status: crate::aws::audit::Status) {
    state.aws.with(&state.lock, |inner| inner.audit.insert(key.to_string(), status));
}

/// Write events to one group now. `Ok` when they landed or when the account has no trail here
/// (and does not require one).
async fn write_now(
    state: &AppState,
    place: &Place,
    group: &str,
    stream: &str,
    events: &[(i64, String)],
    required: bool,
) -> Result<(), Failure> {
    use crate::aws::audit::{classify, Failure as Why, Status};

    let key = format!("{}|{group}", place.connection.id);
    let logs = state.aws.logs(&place.vault.region, place.credentials.clone()).map_err(aws_failure)?;

    match logs.append(group, stream, events).await {
        Ok(()) => {
            set_status(state, &key, Status::Audited);
            Ok(())
        }
        Err(AwsError::SessionExpired(message)) => Err(aws_failure(AwsError::SessionExpired(message))),
        Err(error) => match classify(&error, group) {
            Why::Off(reason) => {
                set_status(state, &key, Status::Off(reason.clone()));
                if required { Err(trail_required(&reason)) } else { Ok(()) }
            }
            Why::Unavailable(reason) => Err(trail_unavailable(&reason)),
        },
    }
}

/// Send what is queued for this connection. Never fails the request it rides on: what cannot be
/// sent now stays for the next time, up to `QUEUE_LIMIT`.
async fn flush(state: &AppState, place: &Place) {
    use crate::aws::audit::{classify, Failure as Why, Status, QUEUE_LIMIT};

    let mine: Vec<crate::aws::audit::Queued> = state
        .aws
        .with(&state.lock, |inner| {
            let (mine, rest) = std::mem::take(&mut inner.audit_queue)
                .into_iter()
                .partition(|queued| queued.connection_id == place.connection.id);
            inner.audit_queue = rest;
            mine
        })
        .unwrap_or_default();

    if mine.is_empty() {
        return;
    }

    let Ok(logs) = state.aws.logs(&place.vault.region, place.credentials.clone()) else { return };
    let mut keep = Vec::new();
    let mut batches: Vec<((String, String), Vec<crate::aws::audit::Queued>)> = Vec::new();

    for queued in mine {
        let target = (queued.group.clone(), queued.stream.clone());
        match batches.iter_mut().find(|(at, _)| *at == target) {
            Some((_, list)) => list.push(queued),
            None => batches.push((target, vec![queued])),
        }
    }

    for ((group, stream), list) in batches {
        let events: Vec<(i64, String)> = list.iter().map(|q| (q.at_ms, q.message.clone())).collect();
        let key = format!("{}|{group}", place.connection.id);

        match logs.append(&group, &stream, &events).await {
            Ok(()) => set_status(state, &key, Status::Audited),
            Err(error) => match classify(&error, &group) {
                Why::Off(reason) => set_status(state, &key, Status::Off(reason)),
                Why::Unavailable(_) => keep.extend(list),
            },
        }
    }

    if !keep.is_empty() {
        state.aws.with(&state.lock, |inner| {
            inner.audit_queue.extend(keep);
            let excess = inner.audit_queue.len().saturating_sub(QUEUE_LIMIT);
            inner.audit_queue.drain(..excess);
        });
    }
}

/// Record `action` on a parameter of `place`. See the section's comment for what `weight` decides.
pub(crate) async fn trail(
    state: &AppState,
    place: &Place,
    parameter: &str,
    record_id: &str,
    action: &str,
    detail: Value,
    weight: Weight,
) -> Result<(), Failure> {
    use crate::aws::audit::{log_group, message, Queued, Status};

    let settings = bucket_settings(state, place).await;
    let absolute = mapping::absolute(parameter);
    let group = log_group(settings.prefix(), &absolute);
    let key = format!("{}|{group}", place.connection.id);

    let status = state.aws.with(&state.lock, |inner| inner.audit.get(&key).cloned()).ok_or_else(locked)?;
    if let Some(Status::Off(reason)) = status {
        return if settings.required && weight == Weight::Sensitive { Err(trail_required(&reason)) } else { Ok(()) };
    }

    let stream = match caller(state, place).await {
        Ok(stream) => stream,
        Err(error) if weight == Weight::Sensitive => {
            if let AwsError::SessionExpired(message) = error {
                return Err(aws_failure(AwsError::SessionExpired(message)));
            }
            return Err(trail_unavailable(&error.to_string()));
        }
        Err(_) => return Ok(()),
    };

    let event = (now_ms(), message(action, record_id, &absolute, detail));

    match weight {
        Weight::Sensitive => {
            write_now(state, place, &group, &stream, &[event], settings.required).await?;
            // CloudWatch answered: whatever waited in the queue goes too.
            flush(state, place).await;
            Ok(())
        }
        Weight::Background => {
            state.aws.with(&state.lock, |inner| {
                inner.audit_queue.push(Queued {
                    connection_id: place.connection.id.clone(),
                    region: place.vault.region.clone(),
                    group,
                    stream,
                    at_ms: event.0,
                    message: event.1,
                })
            });
            flush(state, place).await;
            Ok(())
        }
    }
}

/// [`trail`] for a record id, resolving its vault.
pub(crate) async fn trail_record(state: &AppState, id: &str, action: &str, detail: Value, weight: Weight) -> Result<(), Failure> {
    let (vault_id, name) = target_record(id)?;
    let place = place(state, &vault_id).await?;
    trail(state, &place, &name, id, action, detail, weight).await
}

/// Write "session opened" to every log group the listing reaches that has not been tried in this
/// unlock, and say what each came to, for the screen.
async fn probe(state: &AppState, place: &Place, names: &[String]) -> Value {
    use crate::aws::audit::{log_group, message, Status};

    let settings = bucket_settings(state, place).await;
    let mut groups: Vec<String> = names.iter().map(|name| log_group(settings.prefix(), &mapping::absolute(name))).collect();
    if let Some(root) = place.vault.root() {
        groups.push(log_group(settings.prefix(), &format!("{}/x", mapping::absolute(root))));
    }
    groups.sort();
    groups.dedup();

    let stream = caller(state, place).await;
    let mut answer = Vec::new();

    for group in groups {
        let key = format!("{}|{group}", place.connection.id);
        let mut status = state.aws.with(&state.lock, |inner| inner.audit.get(&key).cloned()).flatten();

        if status.is_none() {
            if let Ok(stream) = &stream {
                let event = (now_ms(), message("session.opened", "", &group, json!({})));
                let _ = write_now(state, place, &group, stream, &[event], false).await;
                status = state.aws.with(&state.lock, |inner| inner.audit.get(&key).cloned()).flatten();
            }
        }

        answer.push(match status {
            Some(Status::Audited) => json!({ "group": group, "state": "on" }),
            Some(Status::Off(reason)) => json!({ "group": group, "state": "off", "reason": reason }),
            None => json!({
                "group": group,
                "state": "unknown",
                "reason": match &stream {
                    Err(error) => tr!("não foi possível saber quem é o usuário desta sessão ({error})", error = error),
                    Ok(_) => tr!("o CloudWatch Logs não respondeu"),
                },
            }),
        });
    }

    json!({ "groups": answer, "required": settings.required, "settings_unreadable": settings.unreadable })
}

// ── reading the trail ───────────────────────────────────────────────────────────────────────────

/// How far back a record's activity looks by default, and the vault's trail.
const ACTIVITY_DAYS: i64 = 30;
const TRAIL_DAYS: i64 = 7;
const ACTIVITY_PAGE: usize = 50;
const TRAIL_PAGE: usize = 500;

fn day_ms() -> i64 {
    86_400_000
}

/// One event as the screen shows it. The person is the stream's name and the time CloudWatch's,
/// never what the event's body claims (see `aws::audit`).
fn event_json(event: &crate::aws::LogEvent) -> Value {
    let body: Value = serde_json::from_str(&event.message).unwrap_or_default();
    let mut detail = body.clone();
    if let Some(object) = detail.as_object_mut() {
        for key in ["v", "action", "record", "parameter", "client"] {
            object.remove(key);
        }
    }

    json!({
        "at_ms": event.ingestion_time,
        "person": event.log_stream_name,
        "action": body["action"],
        "record": body["record"],
        "parameter": body["parameter"],
        "detail": detail,
    })
}

fn audit_state_json(status: Option<&crate::aws::audit::Status>, group: &str) -> Value {
    use crate::aws::audit::Status;

    match status {
        Some(Status::Audited) => json!({ "group": group, "state": "on" }),
        Some(Status::Off(reason)) => json!({ "group": group, "state": "off", "reason": reason }),
        None => json!({ "group": group, "state": "unknown" }),
    }
}

/// What the trail of a record's boundary came to in this unlock, without asking AWS.
pub(crate) fn record_audit_state(state: &AppState, place: &Place, name: &str) -> Value {
    let settings = state
        .aws
        .with(&state.lock, |inner| inner.bucket_settings.get(&place.vault_id).cloned())
        .flatten()
        .unwrap_or_default();
    let group = crate::aws::audit::log_group(settings.prefix(), &mapping::absolute(name));
    let key = format!("{}|{group}", place.connection.id);
    let status = state.aws.with(&state.lock, |inner| inner.audit.get(&key).cloned()).flatten();

    audit_state_json(status.as_ref(), &group)
}

/// A record's own events, newest first. `before` (milliseconds) asks for the page before it.
pub(crate) async fn record_activity(state: &AppState, id: &str, before: Option<i64>) -> Answer {
    let (vault_id, name) = target_record(id)?;
    let place = place(state, &vault_id).await?;
    let settings = bucket_settings(state, &place).await;
    let group = crate::aws::audit::log_group(settings.prefix(), &mapping::absolute(&name));
    let logs = state.aws.logs(&place.vault.region, place.credentials.clone()).map_err(aws_failure)?;

    let end = before.unwrap_or_else(|| now_ms() + 60_000);
    let start = end - ACTIVITY_DAYS * day_ms();
    // The record id is base32 and the pattern quotes it: nothing in it can break out of the filter.
    let pattern = format!("{{ $.record = \"{id}\" }}");

    match logs.filter_log_events(&group, Some(&pattern), Some(start), Some(end), ACTIVITY_PAGE * 4).await {
        Ok(mut events) => {
            events.sort_by(|a, b| b.ingestion_time.cmp(&a.ingestion_time));
            let more = events.len() > ACTIVITY_PAGE;
            events.truncate(ACTIVITY_PAGE);

            Ok(Json(json!({
                "source": "aws",
                "group": group,
                "days": ACTIVITY_DAYS,
                "events": events.iter().map(event_json).collect::<Vec<_>>(),
                "more": more,
                "audit": record_audit_state(state, &place, &name),
            })))
        }
        Err(AwsError::AccessDenied(_)) => Ok(Json(json!({ "source": "aws", "group": group, "denied": true, "events": [] }))),
        Err(AwsError::NotFound(_)) => Ok(Json(json!({ "source": "aws", "group": group, "missing": true, "events": [] }))),
        Err(other) => Err(aws_failure(other)),
    }
}

#[derive(serde::Deserialize)]
struct TrailQuery {
    #[serde(default)]
    person: Option<String>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    days: Option<i64>,
}

/// The whole trail of an AWS vault, for whoever may read its log groups.
async fn vault_trail(
    State(state): State<Arc<AppState>>,
    Path(vault_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<TrailQuery>,
) -> Answer {
    let place = place(&state, &vault_id).await?;
    let snapshot = snapshot(&state, &place).await?;
    let settings = bucket_settings(&state, &place).await;
    let logs = state.aws.logs(&place.vault.region, place.credentials.clone()).map_err(aws_failure)?;

    let mut groups: Vec<String> = snapshot
        .parameters
        .iter()
        .map(|p| crate::aws::audit::log_group(settings.prefix(), &mapping::absolute(&p.name)))
        .collect();
    if let Some(root) = place.vault.root() {
        groups.push(crate::aws::audit::log_group(settings.prefix(), &format!("{}/x", mapping::absolute(root))));
    }
    groups.sort();
    groups.dedup();

    let days = query.days.unwrap_or(TRAIL_DAYS).clamp(1, 90);
    let end = now_ms() + 60_000;
    let start = end - days * day_ms();
    // Only the action is filtered by CloudWatch; the action names are ours and fixed.
    let pattern = query
        .action
        .as_deref()
        .filter(|a| a.chars().all(|c| c.is_ascii_lowercase() || c == '.' || c == '_'))
        .map(|action| format!("{{ $.action = \"{action}\" }}"));

    let titles: HashMap<String, String> = snapshot
        .parameters
        .iter()
        .map(|p| {
            let title = mapping::payload_from(
                &p.name,
                snapshot.metadata.get(&p.name).and_then(|m| m.description.as_deref()),
                "",
                None,
            )
            .title;
            (mapping::absolute(&p.name), title)
        })
        .collect();

    let mut events = Vec::new();
    let mut denied = Vec::new();
    let mut missing = Vec::new();

    for group in &groups {
        match logs.filter_log_events(group, pattern.as_deref(), Some(start), Some(end), TRAIL_PAGE).await {
            Ok(found) => events.extend(found),
            Err(AwsError::AccessDenied(_)) => denied.push(group.clone()),
            Err(AwsError::NotFound(_)) => missing.push(group.clone()),
            Err(other) => return Err(aws_failure(other)),
        }
    }

    let mut people: Vec<String> = events.iter().map(|e| e.log_stream_name.clone()).collect();
    people.sort();
    people.dedup();

    if let Some(person) = query.person.as_deref().filter(|p| !p.is_empty()) {
        events.retain(|e| e.log_stream_name == person);
    }
    events.sort_by(|a, b| b.ingestion_time.cmp(&a.ingestion_time));
    events.truncate(TRAIL_PAGE);

    let rows: Vec<Value> = events
        .iter()
        .map(|event| {
            let mut row = event_json(event);
            let title = row["parameter"].as_str().and_then(|p| titles.get(p)).cloned();
            row["title"] = json!(title);
            row
        })
        .collect();

    Ok(Json(json!({
        "groups": groups,
        "denied": denied,
        "missing": missing,
        "days": days,
        "people": people,
        "events": rows,
    })))
}

/// "Tentar de novo" on the badge: forget what this unlock learned about the vault's trail and ask
/// again.
async fn retry_audit(State(state): State<Arc<AppState>>, Path(vault_id): Path<String>) -> Answer {
    let place = place(&state, &vault_id).await?;

    state.aws.with(&state.lock, |inner| {
        let prefix = format!("{}|", place.connection.id);
        inner.audit.retain(|key, _| !key.starts_with(&prefix));
        inner.callers.remove(&place.connection.id);
        inner.bucket_settings.remove(&vault_id);
    });

    let snapshot = snapshot(&state, &place).await?;
    let names: Vec<String> = snapshot.parameters.iter().map(|p| p.name.clone()).collect();

    Ok(Json(json!({ "audit": probe(&state, &place, &names).await })))
}

// ── sharing a record ────────────────────────────────────────────────────────────────────────────
//
// See `crate::share` for what may go and how it is sealed. Here: where the objects go (inside the
// record's permission boundary, `<segment>/.keyhold/shares/<id>/`), the signed URLs, and the
// record of the share kept in the sealed AWS configuration.

#[derive(serde::Deserialize)]
pub(crate) struct ShareBody {
    #[serde(flatten)]
    choice: crate::share::Choice,
    /// Hours the link lasts; ignored for first access.
    #[serde(default)]
    hours: Option<u64>,
    #[serde(default)]
    first_access: bool,
    #[serde(default)]
    password: bool,
    #[serde(default)]
    recipient: Option<String>,
}

/// Five minutes: a link signed with credentials about to end would die before anybody opened it.
const SHARE_MIN_SECONDS: u64 = 300;

fn share_refusal(text: impl Into<String>) -> Failure {
    aws_refusal(StatusCode::FORBIDDEN, "share_refused", text)
}

pub(crate) async fn share_record(state: &AppState, id: &str, body: ShareBody) -> Answer {
    use crate::share::{self, crypto};

    let (vault_id, name) = target_record(id)?;
    let place = place(state, &vault_id).await?;
    let settings = bucket_settings(state, &place).await;

    if settings.share_off {
        return Err(share_refusal(tr!("o compartilhamento está desligado para este cofre (tag keyhold:share=off no bucket)")));
    }
    let bucket = bucket_of(&place)?;
    // A bucket with a dot is reached path-style, on a host shared by every bucket of the region:
    // the recipient's page would share its origin with strangers.
    if bucket.contains('.') {
        return Err(share_refusal(tr!("o compartilhamento precisa de um bucket sem ponto no nome")));
    }

    let absolute = mapping::absolute(&name);
    let segments: Vec<&str> = absolute.trim_start_matches('/').split('/').collect();
    let boundary = if segments.len() > 1 {
        segments[0].to_string()
    } else {
        settings
            .share_prefix
            .clone()
            .ok_or_else(|| share_refusal(tr!("um registro na raiz da conta só pode ser compartilhado com a tag keyhold:share-prefix no bucket")))?
    };

    let now = now_unix();
    let session_end = place.credentials.expires_at().unwrap_or(now + 3600);
    if session_end.saturating_sub(now) < SHARE_MIN_SECONDS {
        return Err(session_expired());
    }
    let expires_at = if body.first_access {
        session_end
    } else {
        (now + body.hours.unwrap_or(1).clamp(1, 12) * 3600).min(session_end)
    };
    if body.first_access && !body.password {
        return Err(bad_request(tr!("o primeiro acesso exige a senha de abertura")));
    }

    let (opened, _) = open_parameter(state, &place, &name).await?;
    let (fields, notes, _left_out) = share::content(&opened.payload, &body.choice).map_err(bad_request)?;
    let recipient = body.recipient.as_deref().map(str::trim).filter(|r| !r.is_empty()).map(str::to_string);

    // The event lands before anything is written to the bucket.
    trail(
        state,
        &place,
        &name,
        id,
        "record.shared",
        json!({
            "recipient": recipient,
            "fields": fields.iter().map(|f| f.label.clone()).collect::<Vec<_>>(),
            "notes": notes.is_some(),
            "first_access": body.first_access,
            "password": body.password,
            "expires_at": expires_at,
        }),
        Weight::Sensitive,
    )
    .await?;

    let sender = caller(state, &place).await.unwrap_or_else(|_| place.connection.title.clone());
    let metadata = metadata_of(&place, &name).await?;
    let key_id = s3_key(&place, &metadata, &name)?;
    let s3 = s3_of(state, &place)?;
    let s3_error = |e| s3_failure(e, &bucket, &place.vault.region);

    let share_id = crate::encoding::base32(&crypto::random32()[..16]).to_lowercase();
    let prefix = format!("{boundary}/.keyhold/shares/{share_id}/");
    let aad = crypto::aad(&share_id, expires_at);
    let link_key = crypto::random32();
    let password = body.password.then(|| crypto::generated_password(20));
    let salt = crypto::random32();
    let ttl = expires_at.saturating_sub(now).max(1);
    let mut objects: Vec<(String, Option<String>)> = Vec::new();

    // The content: the fields, under the link key and the password.
    let content_key = crypto::content_key(
        &link_key,
        password.as_deref().map(|p| (p, &salt[..16], crypto::PASSWORD_ITERATIONS)),
    );
    let sealed = crypto::seal(&content_key, &share::content_json(&fields, notes.as_deref()), &aad).map_err(bad_request)?;
    let content_key_name = format!("{prefix}content.bin");
    let written = s3.put_object(&bucket, &content_key_name, &sealed, key_id.as_deref(), Some("application/octet-stream")).await.map_err(s3_error)?;
    objects.push((content_key_name.clone(), written.version_id.clone()));

    let content_url = s3.presigned_url("GET", &bucket, &content_key_name, &[], ttl).map_err(aws_failure)?;
    let delete_url = match (body.first_access, written.version_id.as_deref()) {
        (true, Some(version)) => Some(s3.presigned_url("DELETE", &bucket, &content_key_name, &[("versionId", version)], ttl).map_err(aws_failure)?),
        (true, None) => Some(s3.presigned_url("DELETE", &bucket, &content_key_name, &[], ttl).map_err(aws_failure)?),
        _ => None,
    };

    let mut markers = serde_json::Map::new();
    for kind in ["opened", "revealed", "wrong"] {
        let (url, fields) = s3.post_target(&bucket, &format!("{prefix}m-{kind}"), key_id.as_deref(), expires_at).map_err(aws_failure)?;
        markers.insert(kind.into(), json!(share::Marker::new(url, fields)));
    }

    // The envelope: what the page shows before "Revelar", under the link key alone.
    let envelope = share::Envelope {
        v: 1,
        title: opened.payload.title.clone(),
        sender,
        expires_at,
        first: body.first_access,
        password: password.as_ref().map(|_| json!({ "salt": crate::encoding::base64url(&salt[..16]), "iterations": crypto::PASSWORD_ITERATIONS })),
        content: content_url,
        delete: delete_url,
        markers,
    };
    let sealed = crypto::seal(&crypto::envelope_key(&link_key), &serde_json::to_vec(&envelope).map_err(bad_request)?, &aad).map_err(bad_request)?;
    let envelope_name = format!("{prefix}envelope.bin");
    let written = s3.put_object(&bucket, &envelope_name, &sealed, key_id.as_deref(), Some("application/octet-stream")).await.map_err(s3_error)?;
    objects.push((envelope_name.clone(), written.version_id));
    let envelope_url = s3.presigned_url("GET", &bucket, &envelope_name, &[], ttl).map_err(aws_failure)?;

    // The page: the same bytes for every share.
    let page_name = format!("{prefix}index.html");
    let written = s3
        .put_object(&bucket, &page_name, share::recipient_page().as_bytes(), key_id.as_deref(), Some("text/html; charset=utf-8"))
        .await
        .map_err(s3_error)?;
    objects.push((page_name.clone(), written.version_id));
    let page_url = s3.presigned_url("GET", &bucket, &page_name, &[], ttl).map_err(aws_failure)?;

    for kind in ["opened", "revealed", "wrong"] {
        objects.push((format!("{prefix}m-{kind}"), None));
    }

    let link = format!("{page_url}{}", share::fragment(&link_key, &envelope_url, &share_id, expires_at));

    let record = crate::aws::config::SharedLink {
        id: share_id.clone(),
        vault_id: vault_id.clone(),
        record_id: id.to_string(),
        bucket: bucket.clone(),
        prefix,
        created_at: now,
        expires_at,
        first_access: body.first_access,
        password: password.is_some(),
        recipient: recipient.clone(),
        objects,
        seen: Vec::new(),
    };
    change(state, |config, _| {
        config.shares.retain(|share| share.expires_at > now);
        config.shares.push(record);
        Ok(())
    })?;

    Ok(Json(json!({
        "link": link,
        "password": password,
        "share": { "id": share_id, "expires_at": expires_at, "first_access": body.first_access, "recipient": recipient },
    })))
}

/// The record's links still open, with what their pages reported. A marker seen for the first
/// time becomes a trail event ("abriu o link", "revelou o conteúdo").
pub(crate) async fn record_shares(state: &AppState, id: &str) -> Answer {
    let now = now_unix();
    let config = load(state)?;
    let mine: Vec<crate::aws::config::SharedLink> =
        config.shares.iter().filter(|share| share.record_id == id && share.expires_at > now).cloned().collect();

    let (vault_id, name) = target_record(id)?;
    let place = place(state, &vault_id).await?;
    // The longest a new link may last: until these credentials end.
    let session_ends_at = place.credentials.expires_at();
    let settings = bucket_settings(state, &place).await;

    if mine.is_empty() {
        return Ok(Json(json!({ "shares": [], "session_ends_at": session_ends_at, "share_off": settings.share_off })));
    }

    let s3 = s3_of(state, &place)?;
    let mut answer = Vec::new();
    let mut newly_seen: Vec<(String, String)> = Vec::new();

    for share in &mine {
        let found = s3.list_objects_v2(&share.bucket, &format!("{}m-", share.prefix)).await.unwrap_or_default();
        let has = |kind: &str| found.iter().any(|o| o.key.ends_with(&format!("m-{kind}")));

        for kind in ["opened", "revealed", "wrong"] {
            if has(kind) && !share.seen.iter().any(|s| s == kind) {
                newly_seen.push((share.id.clone(), kind.to_string()));
                let action = match kind {
                    "opened" => "share.opened",
                    "revealed" => "share.revealed",
                    _ => "share.wrong_password",
                };
                // The person is the sharer's stream; the detail says it was the recipient's page.
                let _ = trail(state, &place, &name, id, action, json!({ "share": share.id, "recipient": share.recipient }), Weight::Background).await;
            }
        }

        answer.push(json!({
            "id": share.id,
            "created_at": share.created_at,
            "expires_at": share.expires_at,
            "first_access": share.first_access,
            "password": share.password,
            "recipient": share.recipient,
            "opened": has("opened"),
            "revealed": has("revealed"),
        }));
    }

    if !newly_seen.is_empty() {
        change(state, |config, _| {
            for (share_id, kind) in &newly_seen {
                if let Some(share) = config.shares.iter_mut().find(|s| &s.id == share_id) {
                    share.seen.push(kind.clone());
                }
            }
            Ok(())
        })?;
    }

    Ok(Json(json!({ "shares": answer, "session_ends_at": session_ends_at, "share_off": settings.share_off })))
}

/// Revoke a link: delete its objects, versions and all, so nothing of it stays in the bucket.
pub(crate) async fn revoke_share(state: &AppState, share_id: &str) -> Answer {
    let config = load(state)?;
    let share = config
        .shares
        .iter()
        .find(|share| share.id == share_id)
        .cloned()
        .ok_or_else(|| not_found(tr!("o link não existe mais")))?;

    let place = place(state, &share.vault_id).await?;
    let s3 = s3_of(state, &place)?;
    let mut failed = Vec::new();

    for (key, version) in &share.objects {
        let outcome = match version {
            Some(version) => s3.delete_object_version(&share.bucket, key, version).await,
            None => s3.delete_object(&share.bucket, key).await,
        };
        if let Err(error) = outcome {
            if !matches!(error, AwsError::NotFound(_)) {
                failed.push(mapping::leaf(key).to_string());
            }
        }
    }

    if !failed.is_empty() {
        return Err(share_refusal(tr!(
            "o link não foi revogado por completo: {names} não foi apagado do bucket. Tente de novo",
            names = failed.join(", ")
        )));
    }

    change(state, |config, _| {
        config.shares.retain(|s| s.id != share.id);
        Ok(())
    })?;

    let (_, name) = target_record(&share.record_id)?;
    trail(state, &place, &name, &share.record_id, "share.revoked", json!({ "share": share.id }), Weight::Background).await?;

    Ok(Json(json!({ "revoked": true })))
}

/// The record a body describes, checked against what AWS can hold: the description (the title
/// alone), the value (fields, icon and notes), and whether that value is Keyhold's JSON.
fn prepared(payload: &Payload) -> Result<(Payload, String, String, bool), Failure> {
    payload.check().map_err(bad_request)?;
    let payload = payload.clone().normalise();

    let description = mapping::description_for(&payload.title).map_err(bad_request)?;

    // A picture is the one thing in a record that can be left out and found again: when the value
    // does not fit with it, the record is written without it, keeping the icon's name, colour and
    // its place in the library. This is what lets a record copied from a local vault, where the
    // picture is as large as it likes, land in a parameter at all.
    let (payload, (value, json)) = match mapping::encode_value(&payload) {
        Ok(encoded) => (payload, encoded),
        Err(too_large) => {
            let has_picture = payload.icon.as_ref().is_some_and(|icon| icon.image.is_some());
            if !has_picture {
                return Err(bad_request(too_large));
            }

            let mut lighter = payload;
            if let Some(icon) = lighter.icon.as_mut() {
                icon.image = None;
            }
            let encoded = mapping::encode_value(&lighter).map_err(|_| bad_request(too_large))?;

            (lighter, encoded)
        }
    };

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
    let folder = folder_path(state, vault_id, folder_id)?;

    let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(full) if full.starts_with('/') => full.to_string(),
        Some(leaf) => mapping::join(folder.as_deref(), leaf),
        None => mapping::join(folder.as_deref(), &mapping::suggest_name(&payload.title)),
    };
    mapping::validate_name(&name).map_err(bad_request)?;

    let place = place(state, vault_id).await?;
    if let Some(root) = place.vault.root().filter(|root| !mapping::under_root(&name, Some(root))) {
        return Err(bad_request(tr!("o nome na AWS precisa começar com {root}/, o caminho deste cofre", root = root)));
    }
    let snapshot = snapshot(state, &place).await?;
    let key_id =
        mapping::inherit_key(mapping::parent_path(&name).as_deref(), &known_keys(&snapshot)?);

    let mut tags = Vec::new();
    if json {
        tags.push(Tag::new(mapping::TAG_FORMAT, mapping::TAG_FORMAT_JSON));
    }

    let written = place
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
        .await;

    state.aws.invalidate(vault_id);
    let version = written.map_err(|e| write_failure(e, &name))?;

    let config = load(state)?;
    let header = header_for(&place, &config, &name, version, None, Some(&tags));
    note(state, audit::action::RECORD_CREATED, &header.id, Detail::new().with("kind", "item"));

    let fields: Vec<&str> = payload.fields.iter().map(|field| field.label.as_str()).collect();
    trail(state, &place, &name, &header.id, "record.created", json!({ "fields": fields }), Weight::Background).await?;

    Ok(Json(json!({ "record": header })))
}

pub(crate) async fn open_record(state: &AppState, id: &str) -> Answer {
    let mut record = record_json(state, id).await?;
    trail_record(state, id, "record.viewed", json!({}), Weight::Background).await?;

    // What goes to the page: secrets sealed (see `records::sealed_json`).
    let payload: Payload = serde_json::from_value(record["payload"].clone()).map_err(|e| Failure(StatusCode::BAD_GATEWAY, e.to_string(), None))?;
    record["payload"] = crate::records::sealed_json(&payload);

    let (vault_id, name) = target_record(id)?;
    let place = place(state, &vault_id).await?;
    record["audit"] = record_audit_state(state, &place, &name);

    Ok(Json(json!({ "record": record })))
}

/// A record read fresh from Parameter Store, secrets and all: for the binary's own use (copying
/// between vaults, the agent surface), never for the page.
pub(crate) async fn record_json(state: &AppState, id: &str) -> Result<Value, Failure> {
    let (vault_id, name) = target_record(id)?;
    let place = place(state, &vault_id).await?;
    let (opened, kind) = open_parameter(state, &place, &name).await?;
    let mut record = with_plaintext(opened, &kind);
    if let Some(object) = record.as_object_mut() {
        // The parameter's name, for "Na AWS: …" and for copying into an application's config.
        object.insert("parameter".into(), json!(mapping::absolute(&name)));
    }

    Ok(record)
}

/// What a write over an existing parameter produced: the new header, and a sentence for the person
/// when something around the write did not go as planned.
struct Written {
    header: RecordHeader,
    warning: Option<String>,
}

/// Write a new version over an existing parameter, keeping its key and tier, and switching the
/// format tag when the value changed format (A8). With `expected`, refuse if AWS has moved on (A12).
///
/// The order is what keeps the gaps small and harmless:
///
/// 1. Everything slow is read first (metadata, tags), so only `GetParameter` sits between the
///    version check and the write. Parameter Store has no conditional put; this is as close as it
///    gets, and step 4 reports the race it cannot prevent.
/// 2. Going to JSON, the `keyhold:formato=json` tag is added **before** the put: if the put then
///    fails, a plain value with the tag still reads as plain (the reader falls back by shape),
///    while a JSON value without the tag would read as one plain field.
/// 3. The put, with the key and description the parameter had (fail closed when unknown).
/// 4. If the new version is past `expected + 1`, somebody else wrote in between: the save stands,
///    and the answer carries a warning.
/// 5. Going to plain, the tag is removed **after** the put; a failure there is a warning, since the
///    value reads correctly either way.
///
/// The caches are cleared on every path past the first write, errors included.
async fn write_over(
    state: &AppState,
    place: &Place,
    name: &str,
    payload: &Payload,
    expected: Option<i64>,
) -> Result<Written, Failure> {
    let (_, description, value, json) = prepared(payload)?;

    let metadata = metadata_of(place, name).await?;
    refuse_plaintext(&metadata.kind, name)?;
    let key_id = key_of(&metadata, name)?;
    let tags = place.ssm.list_tags_for_resource(name).await.ok();
    let was_json = tagged_json(tags.as_deref());

    let current = place.ssm.get_parameter(name).await.map_err(aws_failure)?;
    refuse_plaintext(&current.kind, name)?;

    if let Some(expected) = expected.filter(|v| *v != current.version) {
        return Err(aws_refusal(
            StatusCode::CONFLICT,
            "conflict",
            tr!(
                "o registro foi salvo na AWS durante a edição (a versão aberta era a {expected}, e \
                 a atual é a {current}). Recarregue para visualizar a mudança antes de salvar",
                expected = expected,
                current = current.version
            ),
        ));
    }
    let baseline = expected.unwrap_or(current.version);

    let mut tags = tags.unwrap_or_default();
    let mut warnings: Vec<String> = Vec::new();

    let outcome: Result<i64, Failure> = async {
        if json && was_json != Some(true) {
            let format = Tag::new(mapping::TAG_FORMAT, mapping::TAG_FORMAT_JSON);
            place
                .ssm
                .add_tags_to_resource(name, std::slice::from_ref(&format))
                .await
                .map_err(|e| write_failure(e, name))?;
            tags.retain(|t| t.key != mapping::TAG_FORMAT);
            tags.push(format);
        }

        let version = place
            .ssm
            .put_parameter(&PutParameter {
                name: name.to_string(),
                advanced: value.len() > mapping::STANDARD_LIMIT
                    || metadata.tier.as_deref() == Some("Advanced"),
                value: value.into(),
                // Overwriting without a key would move the parameter to the account's default key.
                key_id: Some(key_id),
                description: Some(description),
                overwrite: true,
                tags: Vec::new(),
            })
            .await
            .map_err(|e| write_failure(e, name))?;

        if version > baseline + 1 {
            warnings.push(tr!(
                "outra versão de {name} foi salva na AWS durante esta gravação: a sua passou a ser a \
                 versão {version}, e não a {expected}. Abra o histórico para conferir se nada foi perdido",
                name = name,
                version = version,
                expected = baseline + 1
            ));
        }

        if !json && was_json != Some(false) {
            match place
                .ssm
                .remove_tags_from_resource(name, &[mapping::TAG_FORMAT.to_string()])
                .await
            {
                Ok(()) => tags.retain(|t| t.key != mapping::TAG_FORMAT),
                // Tags that could not even be read are not worth a warning when they cannot be
                // removed either; a tag known to be there is.
                Err(_) if was_json == Some(true) => warnings.push(tr!(
                    "o registro foi salvo, mas não foi possível remover a marca {tag}={value} de {name}. O \
                     Keyhold continua lendo o valor corretamente pelo formato. Remova a marca na AWS \
                     se ela confundir quem lê o parâmetro",
                    tag = mapping::TAG_FORMAT,
                    value = mapping::TAG_FORMAT_JSON,
                    name = name
                )),
                Err(_) => {}
            }
        }

        Ok(version)
    }
    .await;

    state.aws.invalidate(&place.vault_id);
    let version = outcome?;

    let config = load(state)?;
    let header = header_for(place, &config, name, version, None, Some(&tags));
    let warning = (!warnings.is_empty()).then(|| warnings.join(". "));

    Ok(Written { header, warning })
}

/// Save a record. `expected` is the version the person opened, from the body's
/// `expected_version` or the `x-keyhold-expected-version` header; without it the save is not
/// checked (A12 needs to know what was opened to know whether it changed), though a race with
/// another writer is still reported as a `warning`.
pub(crate) async fn update_record(
    state: &AppState,
    id: &str,
    payload: &Payload,
    expected: Option<i64>,
) -> Answer {
    let (vault_id, name) = target_record(id)?;
    let place = place(state, &vault_id).await?;

    // Secrets the page never had come back sealed; put the stored ones in their place. Read
    // fresh, like any open: what the page saw may be older than what is in AWS now, and the
    // version check in `write_over` is what refuses that case.
    let mut payload = payload.clone();
    let (stored, _) = open_parameter(state, &place, &name).await?;
    crate::records::unseal(&mut payload, &stored.payload).map_err(|error| Failure(StatusCode::BAD_REQUEST, error.to_string(), None))?;

    let written = write_over(state, &place, &name, &payload, expected).await?;

    let changed = crate::aws::audit::changed_fields(&stored.payload, &payload);
    trail(state, &place, &name, id, "record.updated", json!({ "fields": changed }), Weight::Background).await?;

    note(
        state,
        audit::action::RECORD_UPDATED,
        id,
        Detail::new().with("version", written.header.version),
    );

    Ok(Json(with_warning(json!({ "record": written.header }), written.warning)))
}

/// The object keys of a record's own attachments (one level under its prefix), with the bucket and
/// a client, or `None` when the vault has no bucket or the parameter cannot have attachments.
async fn attachments_in(
    state: &AppState,
    place: &Place,
    name: &str,
) -> Result<Option<(S3Client, String, Vec<String>)>, Result<Failure, Failure>> {
    let (Some(bucket), Some(prefix)) = (place.vault.bucket.clone(), mapping::attachment_prefix(name))
    else {
        return Ok(None);
    };

    let s3 = s3_of(state, place).map_err(Err)?;
    let keys = s3
        .list_objects_v2(&bucket, &prefix)
        .await
        .map_err(|e| Ok(s3_failure(e, &bucket, &place.vault.region)))?
        .into_iter()
        .filter(|o| mapping::is_attachment_of(&prefix, &o.key))
        .map(|o| o.key)
        .collect();

    Ok(Some((s3, bucket, keys)))
}

/// A refusal that explains why the bucket was needed before saying what went wrong in it.
fn because(why: &str, failure: Failure) -> Failure {
    Failure(failure.0, format!("{why}: {}", failure.1), failure.2)
}

/// Move a parameter to another folder: write it under the new name with its tags and key, then
/// delete the old one. Parameter Store has no rename, and this is the order that never loses it.
///
/// What a move costs, said in the answer: the new parameter starts its history at version 1
/// (`history_lost: true`, for the interface to warn before and after). A record with attachments
/// is not moved at all, since its files live under the old name's prefix and would be orphaned.
/// If the old name cannot be deleted after the copy landed, the move still answers success, with
/// a `warning` that both copies exist.
pub(crate) async fn move_record(
    state: &AppState,
    id: &str,
    folder_id: Option<&str>,
    to_vault: Option<&str>,
    leaf: Option<&str>,
) -> Answer {
    let (vault_id, name) = target_record(id)?;

    if to_vault.is_some_and(|to| to != vault_id) {
        return Err(bad_request(tr!(
            "um registro não muda de cofre AWS ao ser arrastado. Use “Copiar para outro local”: \
             ele copia para o outro cofre e, ao final, oferece excluir o original"
        )));
    }

    let folder = folder_path(state, &vault_id, folder_id)?;
    // A new last segment renames as well as moves: "Mudar o nome na AWS também".
    let leaf = match leaf.map(str::trim).filter(|l| !l.is_empty()) {
        Some(typed) => {
            mapping::validate_segment(typed).map_err(bad_request)?;
            typed.to_string()
        }
        None => mapping::leaf(&name).to_string(),
    };
    let new_name = mapping::join(folder.as_deref(), &leaf);
    mapping::validate_name(&new_name).map_err(bad_request)?;

    if mapping::absolute(&new_name) == mapping::absolute(&name) {
        return Ok(Json(json!({ "moved": true, "id": id, "history_lost": false })));
    }

    let place = place(state, &vault_id).await?;
    trail(
        state,
        &place,
        &name,
        id,
        "record.moved",
        json!({ "from": mapping::absolute(&name), "to": mapping::absolute(&new_name) }),
        Weight::Sensitive,
    )
    .await?;
    let outcome = move_parameter(state, &place, id, &name, &new_name).await;
    state.aws.invalidate(&vault_id);

    outcome
}

async fn move_parameter(
    state: &AppState,
    place: &Place,
    id: &str,
    name: &str,
    new_name: &str,
) -> Answer {
    // No metadata, no move: the copy would land on the default key, without its title.
    let metadata = metadata_of(place, name).await?;
    refuse_plaintext(&metadata.kind, name)?;
    let key_id = key_of(&metadata, name)?;

    let why = tr!("o registro não foi movido, pois não foi possível conferir se ele tem anexos no bucket");
    match attachments_in(state, place, name).await {
        Ok(Some((_, bucket, keys))) if !keys.is_empty() => {
            return Err(aws_refusal(
                StatusCode::CONFLICT,
                "conflict",
                tr!(
                    "o registro tem {count} anexo(s) no bucket {bucket}, armazenados sob o nome atual, \
                     e mover deixaria os arquivos para trás. Baixe e remova os anexos, mova o \
                     registro e anexe os arquivos novamente",
                    count = keys.len(),
                    bucket = bucket
                ),
            ));
        }
        Ok(_) => {}
        Err(Ok(failure)) => return Err(because(&why, failure)),
        Err(Err(failure)) => return Err(failure),
    }

    let parameter = place.ssm.get_parameter(name).await.map_err(aws_failure)?;
    let tags: Vec<Tag> = place
        .ssm
        .list_tags_for_resource(name)
        .await
        .map_err(aws_failure)?
        .into_iter()
        .filter(|t| !t.key.starts_with("aws:"))
        .collect();

    place
        .ssm
        .put_parameter(&PutParameter {
            name: new_name.to_string(),
            advanced: parameter.value.expose().len() > mapping::STANDARD_LIMIT
                || metadata.tier.as_deref() == Some("Advanced"),
            value: parameter.value.clone(),
            key_id: Some(key_id),
            description: metadata.description.clone(),
            overwrite: false,
            tags,
        })
        .await
        .map_err(|e| write_failure(e, new_name))?;

    // From here the new name exists, so the favourite follows it whatever happens to the old one.
    let account = place.connection.account_id.clone();
    let _ = change(state, |config, _| {
        if config.is_favourite(&account, name) {
            config.set_favourite(&account, name, false);
            config.set_favourite(&account, new_name, true);
        }
        Ok(())
    });

    let warning = match place.ssm.delete_parameter(name).await {
        Ok(()) => None,
        Err(error) => Some(tr!(
            "o registro foi copiado para {new_name}, mas não foi possível excluir {name} ({why}). As \
             duas cópias existem na AWS agora. Exclua a antiga quando possível",
            new_name = new_name,
            name = name,
            why = aws_failure(error).1
        )),
    };

    let new_id = record_id(&place.vault_id, new_name);
    note(state, audit::action::RECORD_MOVED, &new_id, Detail::new().with("from", id));

    Ok(Json(with_warning(
        json!({ "moved": true, "id": new_id, "history_lost": true }),
        warning,
    )))
}

/// Delete for good (A16). There is no trash in Parameter Store, and pretending otherwise would
/// mean keeping a copy here.
///
/// The record's attachments go first. If the vault has a bucket and it cannot be listed, or one
/// file cannot be deleted, the record stays and the answer says why: deleting the parameter anyway
/// would leave files in the bucket that nothing in Keyhold points at any more.
pub(crate) async fn delete_record(state: &AppState, id: &str, on: bool) -> Answer {
    if !on {
        return Err(bad_request(tr!("cofres AWS não têm lixeira, e o que foi excluído não pode ser restaurado")));
    }

    let (vault_id, name) = target_record(id)?;
    let place = place(state, &vault_id).await?;
    trail(state, &place, &name, id, "record.deleted", json!({}), Weight::Sensitive).await?;
    let outcome = delete_parameter_and_files(state, &place, id, &name).await;
    state.aws.invalidate(&vault_id);

    outcome
}

async fn delete_parameter_and_files(
    state: &AppState,
    place: &Place,
    id: &str,
    name: &str,
) -> Answer {
    let mut files = 0i64;

    match attachments_in(state, place, name).await {
        Ok(Some((s3, bucket, keys))) => {
            for key in keys {
                s3.delete_object(&bucket, &key).await.map_err(|e| {
                    because(
                        &tr!(
                            "o registro não foi excluído, pois o anexo {name} não foi removido do bucket",
                            name = mapping::leaf(&key)
                        ),
                        s3_failure(e, &bucket, &place.vault.region),
                    )
                })?;
                files += 1;
            }
        }
        Ok(None) => {}
        Err(Ok(failure)) => {
            return Err(because(
                &tr!(
                    "o registro não foi excluído, pois ele pode ter anexos no bucket e não foi possível \
                     listá-los. Excluir agora deixaria os arquivos para trás"
                ),
                failure,
            ))
        }
        Err(Err(failure)) => return Err(failure),
    }

    place.ssm.delete_parameter(name).await.map_err(aws_failure)?;

    let account = place.connection.account_id.clone();
    let _ = change(state, |config, _| {
        config.set_favourite(&account, name, false);
        Ok(())
    });

    note(state, audit::action::RECORD_DESTROYED, id, Detail::new().with("attachments", files));

    Ok(Json(json!({ "archived": true, "destroyed": true })))
}

/// Favourites are personal, so they stay here (A14).
pub(crate) async fn set_favourite(state: &AppState, id: &str, on: bool) -> Answer {
    let (vault_id, name) = target_record(id)?;
    let vault = aws_vault_of(state, &vault_id)?.ok_or_else(|| not_found(tr!("o cofre não existe mais")))?;

    change(state, |config, _| {
        let account = config
            .connection(&vault.connection_id)
            .map(|c| c.account_id.clone())
            .ok_or_else(|| bad_request(tr!("a conta AWS deste cofre não existe mais")))?;
        config.set_favourite(&account, &name, on);
        Ok(())
    })?;

    Ok(Json(json!({ "favourite": on })))
}

async fn set_tag(state: &AppState, id: &str, key: &str, value: Option<String>) -> Result<(), Failure> {
    let (vault_id, name) = target_record(id)?;
    let place = place(state, &vault_id).await?;

    let done = match value {
        Some(value) => place.ssm.add_tags_to_resource(&name, &[Tag::new(key, value)]).await,
        None => place.ssm.remove_tags_from_resource(&name, &[key.to_string()]).await,
    };

    state.aws.invalidate(&vault_id);
    done.map_err(|e| write_failure(e, &name))
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
    let (vault_id, name) = target_record(id)?;
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

/// An old version's payload, and the current version number at the time history was read. Tags
/// are not versioned, so the format is read from the value's shape.
async fn version_payload(place: &Place, name: &str, version: i64) -> Result<(Payload, i64), Failure> {
    let entries = history(place, name).await?;
    let current = entries.last().map(|e| e.version).unwrap_or(version);

    let entry = entries
        .into_iter()
        .find(|e| e.version == version)
        .ok_or_else(|| not_found(tr!("a versão não existe mais na AWS")))?;

    let payload = mapping::payload_from(name, entry.description.as_deref(), entry.value.expose(), None);

    Ok((payload, current))
}

pub(crate) async fn read_version(state: &AppState, id: &str, version: i64) -> Answer {
    let (vault_id, name) = target_record(id)?;
    let place = place(state, &vault_id).await?;
    let (payload, _) = version_payload(&place, &name, version).await?;

    Ok(Json(json!({ "payload": crate::records::sealed_json(&payload) })))
}

/// One secret field of an AWS record, read fresh from Parameter Store (a `GetParameter` of its own
/// in CloudTrail) and noted in the trail before it is handed over.
pub(crate) async fn reveal_secret(
    state: &AppState,
    id: &str,
    index: usize,
    copying: bool,
    version: Option<i64>,
) -> Answer {
    let (vault_id, name) = target_record(id)?;
    let place = place(state, &vault_id).await?;

    let payload = match version {
        Some(version) => version_payload(&place, &name, version).await?.0,
        None => open_parameter(state, &place, &name).await?.0.payload,
    };

    let secret = crate::records::secret_of(&payload, index).map_err(|error| Failure(StatusCode::BAD_REQUEST, error.to_string(), None))?;

    // The event lands before the value leaves the binary, or the value does not leave.
    trail(
        state,
        &place,
        &name,
        id,
        if copying { "field.copied" } else { "field.revealed" },
        json!({ "field": secret["label"], "version": version }),
        Weight::Sensitive,
    )
    .await?;

    // The local trail follows the same switch as a local vault's reads. The AWS trail (CloudWatch)
    // does not ask: there, the account decides.
    let log_reads = crate::settings::log_reads(&state.store.lock().unwrap());
    if log_reads {
        note(
            state,
            if copying { audit::action::SECRET_COPIED } else { audit::action::SECRET_REVEALED },
            id,
            Detail::new().with("field", secret["label"].as_str().unwrap_or_default().to_string()),
        );
    }

    Ok(Json(json!({ "secret": secret })))
}

/// Bring an old version back by writing it as a new one: AWS's history only grows.
///
/// Checked like a save (A12): against `expected`, the version the person had open, or, when the
/// interface does not send one, against the current version as history reported it when this
/// request started. A restore never lands silently on top of somebody else's write.
pub(crate) async fn restore_version(
    state: &AppState,
    id: &str,
    version: i64,
    expected: Option<i64>,
) -> Answer {
    let (vault_id, name) = target_record(id)?;
    let place = place(state, &vault_id).await?;
    let (payload, current) = version_payload(&place, &name, version).await?;
    let written = write_over(state, &place, &name, &payload, expected.or(Some(current))).await?;

    note(
        state,
        audit::action::RECORD_UPDATED,
        id,
        Detail::new().with("version", written.header.version).with("restored", version),
    );

    Ok(Json(with_warning(json!({ "record": written.header }), written.warning)))
}

pub(crate) async fn totp_code(state: &AppState, id: &str, index: usize) -> Answer {
    let (vault_id, name) = target_record(id)?;
    let place = place(state, &vault_id).await?;
    let (opened, _) = open_parameter(state, &place, &name).await?;

    let field = opened
        .payload
        .fields
        .get(index)
        .filter(|f| f.kind == "totp")
        .ok_or_else(|| bad_request(tr!("o campo não é um código 2FA")))?;

    let (code, remaining) =
        crate::totp::code_for_field(&field.value, crate::identity::now_seconds())
            .ok_or_else(|| bad_request(tr!("o segredo do campo não é um 2FA válido")))?;

    Ok(Json(json!({ "code": code, "remaining": remaining })))
}

// ── attachments (A18) ───────────────────────────────────────────────────────────────────────────

fn bucket_of(place: &Place) -> Result<String, Failure> {
    place.vault.bucket.clone().ok_or_else(|| {
        bad_request(tr!("este cofre AWS não possui um bucket S3 para anexos. Defina um nas configurações do cofre"))
    })
}

/// Where a parameter's files go, or a refusal for a parameter whose name has no leading `/`
/// (whose prefix would be shared with the parameter of the same name with one).
fn prefix_for(name: &str) -> Result<String, Failure> {
    mapping::attachment_prefix(name).ok_or_else(|| {
        bad_request(tr!(
            "anexos funcionam apenas em parâmetros cujo nome começa com '/'. Mova o registro para \
             uma pasta para anexar arquivos"
        ))
    })
}

fn attachment_of(vault_id: &str, record_id: &str, key: &str, size: u64, at: &str) -> Attachment {
    let name = mapping::leaf(key).to_string();

    Attachment {
        id: attachment_id(vault_id, key),
        record_id: record_id.to_string(),
        mime: mime_guess::from_path(&name).first_or_octet_stream().to_string(),
        name,
        size: size as i64,
        created_at: at.to_string(),
    }
}

pub(crate) async fn list_attachments(state: &AppState, record_id: &str) -> Answer {
    let (vault_id, name) = target_record(record_id)?;
    let place = place(state, &vault_id).await?;

    let (Some(bucket), Some(prefix)) = (place.vault.bucket.clone(), mapping::attachment_prefix(&name))
    else {
        return Ok(Json(json!({ "attachments": [], "max_bytes": crate::attachments::MAX_BYTES })));
    };

    let s3 = s3_of(state, &place)?;

    let found: Vec<Attachment> = s3
        .list_objects_v2(&bucket, &prefix)
        .await
        .map_err(|e| s3_failure(e, &bucket, &place.vault.region))?
        .into_iter()
        .filter(|o| mapping::is_attachment_of(&prefix, &o.key))
        .map(|o| attachment_of(&vault_id, record_id, &o.key, o.size, &o.last_modified))
        .collect();

    Ok(Json(json!({ "attachments": found, "max_bytes": crate::attachments::MAX_BYTES })))
}

/// Upload a file next to its parameter, encrypted with the parameter's own key (A18). A parameter
/// on the account's default key (`alias/aws/ssm`, which S3 cannot use) gets S3's default, `aws/s3`.
/// When the parameter's key cannot be read, nothing is uploaded: guessing would mean `aws/s3` for
/// a file that belongs under a customer key.
pub(crate) async fn add_attachment(
    state: &AppState,
    record_id: &str,
    file_name: &str,
    mime: &str,
    bytes: &[u8],
) -> Answer {
    let file_name = file_name.trim();
    if file_name.is_empty() {
        return Err(bad_request(tr!("o arquivo precisa de um nome")));
    }
    if file_name.contains('/') || file_name == "." || file_name == ".." {
        return Err(bad_request(tr!("o nome do arquivo não pode conter '/'")));
    }
    if bytes.is_empty() {
        return Err(bad_request(tr!("o arquivo está vazio")));
    }
    if bytes.len() > crate::attachments::MAX_BYTES {
        return Err(bad_request(tr!(
            "o arquivo tem {size} MB, e o limite é {limit} MB",
            size = format!("{:.1}", bytes.len() as f64 / (1024.0 * 1024.0)),
            limit = crate::attachments::MAX_BYTES / (1024 * 1024)
        )));
    }

    let (vault_id, name) = target_record(record_id)?;
    let prefix = prefix_for(&name)?;
    let place = place(state, &vault_id).await?;
    let bucket = bucket_of(&place)?;

    let metadata = metadata_of(&place, &name).await?;
    let key_id = s3_key(&place, &metadata, &name)?;

    let s3 = s3_of(state, &place)?;
    let key = format!("{prefix}{file_name}");

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

/// Hand back one file, refusing one past the attachment limit before its body is read: a file
/// somebody put in the bucket by hand can be any size.
pub(crate) async fn open_attachment(state: &AppState, id: &str) -> Answer {
    let (vault_id, key) = target_attachment(id)?;
    let place = place(state, &vault_id).await?;
    let bucket = bucket_of(&place)?;
    let s3 = s3_of(state, &place)?;

    let object = s3
        .get_object(&bucket, &key, crate::attachments::MAX_BYTES as u64)
        .await
        .map_err(|e| s3_failure(e, &bucket, &place.vault.region))?;

    let name = mapping::leaf(&key).to_string();

    if let Some(parameter) = mapping::attachment_record(&key) {
        let record_id = mapping::record_id(&vault_id, &mapping::absolute(&parameter));
        trail(state, &place, &parameter, &record_id, "attachment.downloaded", json!({ "attachment": name }), Weight::Sensitive)
            .await?;
    }

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
    let (vault_id, key) = target_attachment(id)?;
    let place = place(state, &vault_id).await?;
    let bucket = bucket_of(&place)?;
    let s3 = s3_of(state, &place)?;

    s3.delete_object(&bucket, &key)
        .await
        .map_err(|e| s3_failure(e, &bucket, &place.vault.region))?;

    Ok(Json(json!({ "deleted": true })))
}

// ── copying between vaults (§9.1) ───────────────────────────────────────────────────────────────

/// What a copy into an AWS vault needs to know before writing anything (C5): the names already
/// there (and whether each is a `SecureString`), what would stop the copy, and when the session
/// ends.
pub(crate) struct CopyTarget {
    /// Each name already there: whether it is a `SecureString`, and its version.
    pub names: HashMap<String, (bool, i64)>,
    pub problems: Vec<String>,
    pub session_expires_at: Option<u64>,
}

/// When the SSO session behind a vault ends, if Keyhold holds one.
fn session_expiry(state: &AppState, place: &Place) -> Option<u64> {
    state
        .aws
        .with(&state.lock, |inner| inner.tokens.get(&place.connection.login_id).map(|t| t.expires_at()))
        .flatten()
}

/// The checks before copying into `vault_id`. A vault whose keys cannot be read is a problem, not
/// an error: nothing would be created there (`known_keys` fails closed), and the person should hear
/// it before the first record rather than at it.
pub(crate) async fn copy_target(state: &AppState, vault_id: &str, need_bucket: bool) -> Result<CopyTarget, Failure> {
    let place = place(state, vault_id).await?;
    let snapshot = snapshot(state, &place).await?;
    let mut problems = Vec::new();

    if let Err(failure) = known_keys(&snapshot) {
        problems.push(failure.1);
    }

    let names = snapshot
        .parameters
        .iter()
        .filter(|p| !mapping::is_keep(&p.name))
        .map(|p| (p.name.clone(), (is_secure(&p.kind), p.version)))
        .collect();

    if need_bucket {
        match place.vault.bucket.as_deref() {
            None => problems.push(tr!(
                "há anexos para copiar, e o cofre de destino não possui bucket. Defina um bucket \
                 nas configurações do cofre ou remova esses registros da cópia"
            )),
            Some(bucket) => {
                let s3 = s3_of(state, &place)?;
                if let Err(error) = s3.head_bucket(bucket).await {
                    problems.push(s3_failure(error, bucket, &place.vault.region).1);
                }
            }
        }
    }

    Ok(CopyTarget { names, problems, session_expires_at: session_expiry(state, &place) })
}

/// Which parameter an AWS record id stands for, across vault entries: the account, the region and
/// the name. Two vaults on connections to the same account and region see the same parameters, so
/// a copy between them can land on its own source. `None` for anything that is not an AWS record.
pub(crate) fn parameter_identity(state: &AppState, id: &str) -> Option<String> {
    let (IdKind::Record, vault_id, name) = mapping::decode_id(id)? else { return None };
    let vault = aws_vault_of(state, &vault_id).ok()??;
    let config = load(state).ok()?;
    let connection = config.connections.iter().find(|c| c.id == vault.connection_id)?;

    Some(format!("{}|{}|{}", connection.account_id, vault.region, mapping::absolute(&name)))
}

/// The names of a record's attachments in an AWS vault, to finish a copy that stopped halfway.
pub(crate) async fn attachment_names(state: &AppState, record_id: &str) -> Result<Vec<String>, Failure> {
    let Json(listed) = list_attachments(state, record_id).await?;

    Ok(listed["attachments"]
        .as_array()
        .map(|all| all.iter().filter_map(|a| a["name"].as_str().map(str::to_string)).collect())
        .unwrap_or_default())
}

/// When the session of a vault being copied from ends. Fails as any read of the vault would.
pub(crate) async fn copy_source(state: &AppState, vault_id: &str) -> Result<Option<u64>, Failure> {
    let place = place(state, vault_id).await?;
    Ok(session_expiry(state, &place))
}

// ── health check ────────────────────────────────────────────────────────────────────────────────

/// Can this vault read its parameters, and reach its bucket? For the vault's settings page.
async fn check_vault(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    require_unlocked(&state)?;
    let place = place(&state, &id).await?;

    let path = place.vault.listing_path();
    let (parameters, parameters_message) = match place.ssm.get_parameters_by_path(path, false).await {
        Ok(_) => (true, None),
        Err(AwsError::AccessDenied(_)) => (
            false,
            Some(tr!("o papel desta conta não pode ler parâmetros a partir de {path} (falta ssm:GetParametersByPath)", path = path)),
        ),
        Err(other) => return Err(aws_failure(other)),
    };

    let (bucket, bucket_message) = match place.vault.bucket.as_deref() {
        None => (Value::Null, None),
        Some(bucket) => {
            let s3 = s3_of(&state, &place)?;
            match s3.head_bucket(bucket).await {
                Ok(()) => (json!("ok"), None),
                Err(AwsError::NotFound(_)) => (
                    json!("missing"),
                    Some(tr!(
                        "o bucket {bucket} não existe na região {region}. Crie o bucket na AWS, pois o Keyhold não cria buckets",
                        bucket = bucket,
                        region = place.vault.region
                    )),
                ),
                Err(AwsError::AccessDenied(_)) | Err(AwsError::Invalid(_)) => (
                    json!("denied"),
                    Some(tr!(
                        "sem acesso ao bucket {bucket}: ele está em outra região ou não pertence à \
                         conta {account}. O papel precisa de s3:ListBucket, s3:GetObject, s3:PutObject e \
                         s3:DeleteObject",
                        bucket = bucket,
                        account = place.connection.account_id
                    )),
                ),
                Err(other) => return Err(aws_failure(other)),
            }
        }
    };

    let message = match (parameters_message, bucket_message) {
        (None, None) => tr!("conexão verificada: o Keyhold lê os parâmetros desta conta"),
        (Some(a), None) | (None, Some(a)) => a,
        (Some(a), Some(b)) => tr!("{a}. Além disso, {b}", a = a, b = b),
    };

    Ok(Json(json!({ "parameters": parameters, "bucket": bucket, "message": message })))
}

// ── forgetting on lock ──────────────────────────────────────────────────────────────────────────

/// Sweep the AWS state every `every`, so a vault that auto-locks with nobody at the screen drops
/// its tokens, credentials and caches within seconds, not at the next request (L1).
///
/// The lock itself has no timer: it notices it ran out when somebody asks (`Lock::generation`).
/// This is the somebody. It holds the app state weakly, so it ends with the server.
pub(crate) fn start_sweeper(state: &Arc<AppState>, every: Duration) -> tokio::task::JoinHandle<()> {
    let weak: Weak<AppState> = Arc::downgrade(state);

    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            ticks.tick().await;
            let Some(state) = weak.upgrade() else { return };
            state.aws.sweep(&state.lock);
        }
    })
}

#[cfg(test)]
mod tests;

// ── for the agent ───────────────────────────────────────────────────────────────────────────────

/// A record of an AWS vault as an agent's search lists it: nothing secret.
pub(crate) struct AgentPreview {
    pub id: String,
    pub title: String,
    pub subtitle: String,
}

/// The records of an AWS vault, for an agent, through the same listing the interface reads.
pub(crate) async fn agent_previews(state: &AppState, vault_id: &str) -> Result<Vec<AgentPreview>, String> {
    let Json(listing) = list_records(state, vault_id).await.map_err(|failure| failure.1)?;

    Ok(listing["records"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| AgentPreview {
                    id: row["id"].as_str().unwrap_or_default().to_string(),
                    title: row["title"].as_str().unwrap_or_default().to_string(),
                    subtitle: row["subtitle"].as_str().unwrap_or_default().to_string(),
                })
                .collect()
        })
        .unwrap_or_default())
}

/// An AWS record's payload, for an agent that was allowed to read it. The reason it did not open
/// (an expired session, a denied parameter) comes back in words.
pub(crate) async fn agent_payload(state: &AppState, record_id: &str) -> Result<Payload, String> {
    let record = record_json(state, record_id).await.map_err(|failure| {
        if failure.2 == Some("session_expired") {
            "a sessão da AWS expirou. Peça que ela seja renovada no Keyhold".to_string()
        } else {
            failure.1
        }
    })?;

    serde_json::from_value(record["payload"].clone()).map_err(|e| e.to_string())
}
