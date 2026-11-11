//! Vaults, folders and records over HTTP.
//!
//! Every route here needs the master key, and there is exactly one way to get it: `with_key`, which
//! goes through the lock. That is not a convention — it is the reason the auto-lock deadline is
//! real. There is no path in this file that reaches a plaintext payload without the clock having
//! been consulted first, and a request that arrives one second late gets a 423 rather than a secret.
//!
//! **AWS vaults use these same routes (F9).** A handler whose vault lives in Parameter Store, or
//! whose id starts with `aws~`, hands the request to `http::aws` before touching the store, and
//! the answer comes back in the shapes below. That check is the first line of each handler
//! because the store's lock cannot be held across a request to AWS.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use super::aws;
use super::{bad_request, locked, Answer, AppState};
use crate::aws::mapping::is_aws_id;
use crate::audit::{self, Detail};
use crate::attachments;
use crate::health;
use crate::records::{self, Payload};
use crate::transfer;
use crate::vaults::{self, FolderMeta, VaultMeta};

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/vaults", get(list_vaults).post(create_vault))
        .route("/vaults/:id", put(update_vault))
        .route("/vaults/:id", delete(delete_vault))
        .route("/vaults/:id/contents", get(vault_contents))
        .route("/vaults/:id/archive", post(archive_vault))
        .route("/vaults/:id/agent-access", post(agent_access))
        .route("/vaults/:id/health", get(vault_health))
        .route("/vaults/:id/rotate", post(rotate))
        .route("/vaults/:id/export", post(export_vault))
        .route("/vaults/:id/import", post(import_into_vault))
        .route("/vaults/:id/trash", get(list_trash))
        .route("/vaults/:id/trash", delete(empty_trash))
        .route("/records/:id/destroy", post(destroy_record))
        .route("/vaults/:id/folders", get(list_folders).post(create_folder))
        .route("/vaults/:id/records", get(list_records).post(create_record))
        .route("/folders/:id", put(update_folder))
        .route("/folders/:id", delete(delete_folder))
        .route("/folders/:id/move", post(move_folder))
        .route("/records/:id", get(open_record).put(update_record))
        .route("/records/:id/move", post(move_record))
        .route("/records/:id/totp/:index", get(totp_code))
        .route("/records/:id/favourite", post(set_favourite))
        .route("/records/:id/archive", post(archive_record))
        .route("/records/:id/compromised", post(set_compromised))
        .route("/records/:id/rotation", post(set_rotation))
        .route("/records/:id/accesses", post(report_access))
        .route("/records/:id/attachments", get(list_attachments).post(add_attachment))
        .route("/attachments/:id", get(open_attachment))
        .route("/attachments/:id", delete(delete_attachment))
        .route("/records/:id/versions", get(list_versions))
        .route("/records/:id/versions/:version", get(read_version))
        .route("/records/:id/versions/:version/restore", post(restore_version))
}

/// Run something with the master key, or answer 423.
///
/// The closure shape is what keeps the key from escaping: it is borrowed for the length of the call
/// and never returned, so no route can accidentally hold on to it past the moment it was allowed.
macro_rules! with_key {
    ($state:expr, |$key:ident| $body:expr) => {
        $state.lock.with_key(|$key| $body).ok_or_else(locked)?
    };
}

// ── vaults ──────────────────────────────────────────────────────────────────────────────────────

async fn list_vaults(State(state): State<Arc<AppState>>) -> Answer {
    let store = state.store.lock().unwrap();

    let found = with_key!(state, |key| vaults::list_vaults(&store, key)).map_err(bad_request)?;

    Ok(Json(json!({ "vaults": found })))
}

async fn create_vault(State(state): State<Arc<AppState>>, Json(meta): Json<VaultMeta>) -> Answer {
    let store = state.store.lock().unwrap();

    let made = with_key!(state, |key| vaults::create_vault(&store, key, meta))
        .map_err(bad_request)?;

    audit::note(&store, &state.lock, audit::action::VAULT_CREATED, Some(&made.id), Detail::new());

    Ok(Json(json!({ "vault": made })))
}

async fn update_vault(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(meta): Json<VaultMeta>,
) -> Answer {
    let store = state.store.lock().unwrap();

    let updated = with_key!(state, |key| vaults::update_vault(&store, key, &id, meta))
        .map_err(bad_request)?;
    drop(store);

    // Another connection, region or bucket is another view of AWS: nothing cached for the old one
    // may answer for the new one.
    state.aws.invalidate(&id);

    Ok(Json(json!({ "vault": updated })))
}

/// What deleting this vault would take with it, counted before anything is destroyed.
async fn vault_contents(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let store = state.store.lock().unwrap();

    let counted = with_key!(state, |_key| vaults::contents_of(&store, &id)).map_err(bad_request)?;

    Ok(Json(json!({ "contents": counted })))
}

#[derive(Deserialize)]
struct DeleteVault {
    /// The vault's own name, typed back. See `vaults::delete_vault` for why this is not ceremony.
    confirm_name: String,
}

async fn delete_vault(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<DeleteVault>,
) -> Answer {
    let store = state.store.lock().unwrap();

    let destroyed = with_key!(state, |key| vaults::delete_vault(
        &store,
        key,
        &id,
        &body.confirm_name
    ))
    .map_err(bad_request)?;

    audit::note(
        &store,
        &state.lock,
        audit::action::VAULT_DELETED,
        Some(&id),
        Detail::new()
            .with("records", destroyed.records)
            .with("folders", destroyed.folders),
    );
    drop(store);

    // An AWS vault's listing stays in memory for seconds; a deleted vault's must not.
    state.aws.invalidate(&id);

    Ok(Json(json!({ "deleted": destroyed })))
}

#[derive(Deserialize)]
struct Flag {
    #[serde(default)]
    on: bool,
}

async fn archive_vault(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(flag): Json<Flag>,
) -> Answer {
    let store = state.store.lock().unwrap();

    with_key!(state, |_key| vaults::set_archived(&store, &id, flag.on)).map_err(bad_request)?;
    drop(store);

    state.aws.invalidate(&id);

    Ok(Json(json!({ "archived": flag.on })))
}

/// Let a local agent see this vault, or stop it (F7).
///
/// Behind the lock like everything else: deciding what an agent may read is a decision only
/// somebody who can already open the vault gets to make.
async fn agent_access(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(flag): Json<Flag>,
) -> Answer {
    // A19: the agent surface reads the local file only. Saying so here is better than a switch
    // that turns on and then finds nothing.
    if flag.on && aws::is_aws_vault(&state, &id) {
        return Err(bad_request(
            "assistentes ainda não leem cofres AWS. Este cofre fica fora do alcance deles",
        ));
    }

    let store = state.store.lock().unwrap();

    with_key!(state, |_key| vaults::set_agent_access(&store, &id, flag.on)).map_err(bad_request)?;

    Ok(Json(json!({ "agent_access": flag.on })))
}

/// What is wrong with the passwords in this vault (F5).
///
/// Computed on request and never stored: the findings are only true for as long as the records
/// behind them are unchanged, and a cached verdict about a password is a verdict that goes stale
/// silently.
async fn vault_health(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let store = state.store.lock().unwrap();

    let found = with_key!(state, |key| health::examine(&store, key, &id)).map_err(bad_request)?;

    Ok(Json(json!({ "health": found })))
}

/// Give this vault a new key (F5).
async fn rotate(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let mut store = state.store.lock().unwrap();

    let rotated =
        with_key!(state, |key| vaults::rotate_vault(&mut store, key, &id)).map_err(bad_request)?;

    audit::note(
        &store,
        &state.lock,
        audit::action::VAULT_ROTATED,
        Some(&id),
        Detail::new().with("generation", rotated.key_version),
    );

    Ok(Json(json!({ "vault": rotated })))
}

// ── taking it out, and bringing it back (F6) ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ExportRequest {
    passphrase: String,
}

/// Seal this vault into one archive.
///
/// The archive is returned rather than written: keyhold does not choose where a copy of your vault
/// lands on disk. The browser saves it where you say.
async fn export_vault(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<ExportRequest>,
) -> Answer {
    if aws::is_aws_vault(&state, &id) {
        return Err(bad_request(
            "exportar um cofre AWS ainda não é possível: o conteúdo dele fica na AWS",
        ));
    }

    let store = state.store.lock().unwrap();

    let envelope = with_key!(state, |key| transfer::export(&store, key, &id, &body.passphrase))
        .map_err(bad_request)?;

    audit::note(
        &store,
        &state.lock,
        audit::action::EXPORTED,
        Some(&id),
        Detail::new().with("format", transfer::FORMAT),
    );

    Ok(Json(json!({ "archive": envelope })))
}

#[derive(Deserialize)]
#[serde(tag = "from", rename_all = "lowercase")]
enum ImportRequest {
    /// keyhold's own sealed archive.
    Archive { envelope: transfer::Envelope, passphrase: String },
    /// The CSV every other password manager exports.
    Csv { text: String },
}

async fn import_into_vault(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<ImportRequest>,
) -> Answer {
    if aws::is_aws_vault(&state, &id) {
        return Err(bad_request("importar para um cofre AWS ainda não é possível"));
    }

    let store = state.store.lock().unwrap();

    let incoming = match &body {
        ImportRequest::Archive { envelope, passphrase } => {
            transfer::open_archive(envelope, passphrase).map_err(bad_request)?.records
        }
        ImportRequest::Csv { text } => transfer::from_csv(text).map_err(bad_request)?,
    };

    let done = with_key!(state, |key| transfer::import(&store, key, &id, &incoming))
        .map_err(bad_request)?;

    audit::note(
        &store,
        &state.lock,
        audit::action::IMPORTED,
        Some(&id),
        Detail::new()
            .with("records", done.records as i64)
            .with("folders", done.folders as i64)
            .with(
                "from",
                match body {
                    ImportRequest::Archive { .. } => "archive",
                    ImportRequest::Csv { .. } => "csv",
                },
            ),
    );

    Ok(Json(json!({ "imported": done })))
}

// ── attachments (F6) ────────────────────────────────────────────────────────────────────────────

async fn list_attachments(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    if is_aws_id(&id) {
        return aws::list_attachments(&state, &id).await;
    }

    let store = state.store.lock().unwrap();

    let found = with_key!(state, |key| attachments::list(&store, key, &id)).map_err(bad_request)?;

    Ok(Json(json!({ "attachments": found, "max_bytes": attachments::MAX_BYTES })))
}

#[derive(Deserialize)]
struct NewAttachment {
    name: String,
    #[serde(default)]
    mime: String,
    /// The file's bytes, base64url — the same alphabet everything opaque uses here.
    data: String,
}

async fn add_attachment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<NewAttachment>,
) -> Answer {
    let bytes = crate::encoding::from_base64url(&body.data)
        .ok_or_else(|| bad_request("o conteúdo do arquivo não chegou inteiro"))?;

    let mime = if body.mime.is_empty() { "application/octet-stream" } else { &body.mime };

    if is_aws_id(&id) {
        return aws::add_attachment(&state, &id, &body.name, mime, &bytes).await;
    }

    let store = state.store.lock().unwrap();

    let added = with_key!(state, |key| attachments::add(
        &store,
        key,
        &id,
        &body.name,
        mime,
        &bytes
    ))
    .map_err(bad_request)?;

    audit::note(
        &store,
        &state.lock,
        audit::action::RECORD_UPDATED,
        Some(&id),
        Detail::new().with("attachment", added.name.clone()).with("bytes", added.size),
    );

    Ok(Json(json!({ "attachment": added })))
}

/// Hand back one file's bytes.
///
/// Base64 in a JSON body rather than a raw download: a browser download has no way to carry the
/// session token, and a route that authenticated by query string would put a credential in history
/// and in every log that URLs land in. The interface turns this back into a file to save.
async fn open_attachment(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    if is_aws_id(&id) {
        return aws::open_attachment(&state, &id).await;
    }

    let store = state.store.lock().unwrap();

    let file = with_key!(state, |key| attachments::open(&store, key, &id)).map_err(bad_request)?;

    if crate::settings::log_reads(&store) {
        audit::note(
            &store,
            &state.lock,
            audit::action::SECRET_REVEALED,
            Some(&id),
            Detail::new().with("attachment", file.name.clone()),
        );
    }

    Ok(Json(json!({
        "name": file.name,
        "mime": file.mime,
        "data": crate::encoding::base64url(&file.bytes),
    })))
}

async fn delete_attachment(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    if is_aws_id(&id) {
        return aws::delete_attachment(&state, &id).await;
    }

    let store = state.store.lock().unwrap();

    with_key!(state, |_key| attachments::remove(&store, &id)).map_err(bad_request)?;

    Ok(Json(json!({ "deleted": true })))
}

// ── the trash ───────────────────────────────────────────────────────────────────────────────────

async fn list_trash(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let store = state.store.lock().unwrap();

    let found = with_key!(state, |key| records::trash(&store, key, &id)).map_err(bad_request)?;

    Ok(Json(json!({
        "records": found,
        "retention_days": crate::settings::retention_days(&store),
    })))
}

/// Destroy one record for good.
///
/// A POST rather than a DELETE on the record's own path, because `DELETE /records/:id` already
/// means something gentler here — the trash — and two verbs that differ only in permanence is how
/// somebody loses a secret to a misremembered endpoint.
async fn destroy_record(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    if is_aws_id(&id) {
        return aws::delete_record(&state, &id, true).await;
    }

    let store = state.store.lock().unwrap();

    with_key!(state, |_key| records::destroy(&store, &id)).map_err(bad_request)?;

    audit::note(&store, &state.lock, audit::action::RECORD_DESTROYED, Some(&id), Detail::new());

    Ok(Json(json!({ "destroyed": true })))
}

/// Destroy everything in this vault's trash.
///
/// The loop is a named function rather than a closure inside the macro: `with_key!` hands the key
/// to a closure whose return type it does not annotate, and a `?` inside one has nothing to infer
/// from.
fn empty(store: &crate::db::Store, key: &crate::crypto::SecretKey, vault_id: &str) -> anyhow::Result<usize> {
    let waiting = records::trash(store, key, vault_id)?;

    for record in &waiting {
        records::destroy(store, &record.header.id)?;
    }

    Ok(waiting.len())
}

async fn empty_trash(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let store = state.store.lock().unwrap();

    let count = with_key!(state, |key| empty(&store, key, &id)).map_err(bad_request)?;

    audit::note(
        &store,
        &state.lock,
        audit::action::RECORD_DESTROYED,
        Some(&id),
        Detail::new().with("count", count as i64),
    );

    Ok(Json(json!({ "destroyed": count })))
}

#[derive(Deserialize)]
struct Access {
    /// `reveal` or `copy`.
    kind: String,
    /// The field's label, which is not a secret. Its value never leaves the interface.
    #[serde(default)]
    field: String,
}

/// The interface reporting that somebody looked at a secret.
///
/// Reported rather than inferred, because the binary hands over a whole record when a tab opens and
/// cannot tell which field the person actually uncovered. That makes this the one entry in the
/// trail that depends on the interface being honest — which is fine for the interface keyhold
/// ships, and is exactly why the agent surface (F7) will record its own reads on this side instead.
async fn report_access(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Access>,
) -> Answer {
    let store = state.store.lock().unwrap();

    if state.lock.with_key(|_| ()).is_none() {
        return Err(locked());
    }

    if crate::settings::log_reads(&store) {
        audit::note(
            &store,
            &state.lock,
            if body.kind == "copy" {
                audit::action::SECRET_COPIED
            } else {
                audit::action::SECRET_REVEALED
            },
            Some(&id),
            Detail::new().with("field", body.field),
        );
    }

    Ok(Json(json!({ "noted": true })))
}

// ── folders ─────────────────────────────────────────────────────────────────────────────────────

async fn list_folders(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    if aws::is_aws_vault(&state, &id) {
        return aws::list_folders(&state, &id).await;
    }

    let store = state.store.lock().unwrap();

    let found = with_key!(state, |key| vaults::list_folders(&store, key, &id))
        .map_err(bad_request)?;

    Ok(Json(json!({ "folders": found })))
}

#[derive(Deserialize)]
struct NewFolder {
    #[serde(flatten)]
    meta: FolderMeta,
    #[serde(default)]
    parent_id: Option<String>,
}

async fn create_folder(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<NewFolder>,
) -> Answer {
    if aws::is_aws_vault(&state, &id) {
        return aws::create_folder(&state, &id, &body.meta.name, body.parent_id.as_deref()).await;
    }

    let store = state.store.lock().unwrap();

    let made = with_key!(state, |key| vaults::create_folder(
        &store,
        key,
        &id,
        body.parent_id.as_deref(),
        body.meta
    ))
    .map_err(bad_request)?;

    Ok(Json(json!({ "folder": made })))
}

async fn update_folder(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(meta): Json<FolderMeta>,
) -> Answer {
    if is_aws_id(&id) {
        return Err(not_yet_for_aws_folders());
    }

    let store = state.store.lock().unwrap();

    let updated = with_key!(state, |key| vaults::update_folder(&store, key, &id, meta))
        .map_err(bad_request)?;

    Ok(Json(json!({ "folder": updated })))
}

#[derive(Deserialize)]
struct MoveFolder {
    #[serde(default)]
    parent_id: Option<String>,
}

async fn move_folder(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<MoveFolder>,
) -> Answer {
    if is_aws_id(&id) {
        return Err(not_yet_for_aws_folders());
    }

    let store = state.store.lock().unwrap();

    with_key!(state, |_key| vaults::move_folder(&store, &id, body.parent_id.as_deref()))
        .map_err(bad_request)?;

    Ok(Json(json!({ "moved": true })))
}

async fn delete_folder(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    if is_aws_id(&id) {
        return aws::delete_folder(&state, &id).await;
    }

    let store = state.store.lock().unwrap();

    with_key!(state, |_key| vaults::delete_folder(&store, &id)).map_err(bad_request)?;

    Ok(Json(json!({ "deleted": true })))
}

/// Parameter Store has no rename: renaming a folder means rewriting every parameter under it, and
/// that is a bigger operation than a click should start without its own confirmation.
fn not_yet_for_aws_folders() -> super::Failure {
    bad_request("renomear ou mover pastas de um cofre AWS ainda não é possível")
}

// ── records ─────────────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    archived: bool,
}

/// The list: every record's name, and none of their secrets.
///
/// See `records::list_with_previews` for why the titles are opened here. Behind the lock like
/// everything else — the shape of a locked vault is not public either.
async fn list_records(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Answer {
    if aws::is_aws_vault(&state, &id) {
        return aws::list_records(&state, &id).await;
    }

    let store = state.store.lock().unwrap();

    let found = with_key!(state, |key| records::list_with_previews(
        &store,
        key,
        &id,
        query.archived
    ))
    .map_err(bad_request)?;

    Ok(Json(json!({ "records": found })))
}

#[derive(Deserialize)]
struct NewRecord {
    /// Records no longer have a type; older interfaces still send one.
    #[serde(default = "item_kind")]
    kind: String,
    #[serde(default)]
    folder_id: Option<String>,
    payload: Payload,
    /// AWS vaults only: the parameter's name, or its last segment under the folder. Suggested
    /// from the title when absent.
    #[serde(default)]
    name: Option<String>,
}

fn item_kind() -> String {
    "item".into()
}

async fn create_record(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<NewRecord>,
) -> Answer {
    if aws::is_aws_vault(&state, &id) {
        return aws::create_record(
            &state,
            &id,
            body.folder_id.as_deref(),
            &body.payload,
            body.name.as_deref(),
        )
        .await;
    }

    let store = state.store.lock().unwrap();

    let made = with_key!(state, |key| records::create(
        &store,
        key,
        &id,
        body.folder_id.as_deref(),
        &body.kind,
        &body.payload
    ))
    .map_err(bad_request)?;

    audit::note(
        &store,
        &state.lock,
        audit::action::RECORD_CREATED,
        Some(&made.id),
        Detail::new().with("kind", made.kind.clone()),
    );

    Ok(Json(json!({ "record": made })))
}

async fn open_record(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    if is_aws_id(&id) {
        return aws::open_record(&state, &id).await;
    }

    let store = state.store.lock().unwrap();

    let opened = with_key!(state, |key| records::open(&store, key, &id)).map_err(bad_request)?;

    Ok(Json(json!({ "record": opened })))
}

/// The code a record's 2FA field gives right now.
///
/// Computed here rather than in the page: the page may be served over plain HTTP under a name of
/// its own, where the browser withholds the cryptography it would need.
async fn totp_code(
    State(state): State<Arc<AppState>>,
    Path((id, index)): Path<(String, usize)>,
) -> Answer {
    if is_aws_id(&id) {
        return aws::totp_code(&state, &id, index).await;
    }

    let store = state.store.lock().unwrap();
    let opened = with_key!(state, |key| records::open(&store, key, &id)).map_err(bad_request)?;

    let field = opened
        .payload
        .fields
        .get(index)
        .filter(|f| f.kind == "totp")
        .ok_or_else(|| bad_request(anyhow::anyhow!("esse campo não é um código 2FA")))?;

    let (code, remaining) =
        crate::totp::code_for_field(&field.value, crate::identity::now_seconds())
            .ok_or_else(|| bad_request(anyhow::anyhow!("o segredo desse campo não é um 2FA válido")))?;

    Ok(Json(json!({ "code": code, "remaining": remaining })))
}

/// The header an AWS save may carry instead of `expected_version` in the body.
const EXPECTED_VERSION_HEADER: &str = "x-keyhold-expected-version";

/// Save a record.
///
/// The body is the payload itself. For an AWS record it may also carry `expected_version` (or the
/// `x-keyhold-expected-version` header): the version the person opened, so a save on top of
/// somebody else's change is refused with 409 instead of overwriting it (A12). A local record
/// ignores it, since nobody else writes to this file.
async fn update_record(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Answer {
    let expected = body.get("expected_version").and_then(|v| v.as_i64()).or_else(|| {
        headers
            .get(EXPECTED_VERSION_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok())
    });

    let payload: Payload = serde_json::from_value(body).map_err(bad_request)?;

    if is_aws_id(&id) {
        return aws::update_record(&state, &id, &payload, expected).await;
    }

    let mut store = state.store.lock().unwrap();

    let updated = with_key!(state, |key| records::update(&mut store, key, &id, &payload))
        .map_err(bad_request)?;

    audit::note(
        &store,
        &state.lock,
        audit::action::RECORD_UPDATED,
        Some(&id),
        Detail::new().with("version", updated.version),
    );

    Ok(Json(json!({ "record": updated })))
}

#[derive(Deserialize)]
struct MoveRecord {
    #[serde(default)]
    folder_id: Option<String>,
    /// Absent means "wherever it already is". Present and different means a re-encryption.
    #[serde(default)]
    vault_id: Option<String>,
}

/// Move a record: to another folder, or to another vault entirely.
///
/// One route for both because they are one gesture as far as somebody dragging a row is concerned,
/// and because the folder is only meaningful alongside the vault that owns it — sending them apart
/// would allow a request that files a record in a folder belonging to a vault it is not in.
async fn move_record(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<MoveRecord>,
) -> Answer {
    if is_aws_id(&id) {
        return aws::move_record(&state, &id, body.folder_id.as_deref(), body.vault_id.as_deref())
            .await;
    }

    // A17 is not done: a local record does not go into an AWS vault by being dragged there.
    if body.vault_id.as_deref().is_some_and(|to| aws::is_aws_vault(&state, to)) {
        return Err(bad_request(
            "um registro local não vai para um cofre AWS por um arraste. Use \"Copiar para…\": \
             ele copia para o outro cofre e, no fim, oferece apagar o original",
        ));
    }

    let mut store = state.store.lock().unwrap();

    let before = with_key!(state, |_key| records::header(&store, &id)).map_err(bad_request)?;

    match body.vault_id.as_deref() {
        Some(vault_id) => with_key!(state, |key| records::move_to_vault(
            &mut store,
            key,
            &id,
            vault_id,
            body.folder_id.as_deref()
        ))
        .map_err(bad_request)?,
        None => with_key!(state, |_key| records::move_to_folder(
            &store,
            &id,
            body.folder_id.as_deref()
        ))
        .map_err(bad_request)?,
    }

    // Only a change of vault is written down. Dragging a row between folders is arranging, and a
    // trail that logs arranging is a trail nobody reads the important lines of.
    if body.vault_id.as_deref().is_some_and(|to| to != before.vault_id) {
        audit::note(
            &store,
            &state.lock,
            audit::action::RECORD_MOVED,
            Some(&id),
            Detail::new().with("from_vault", before.vault_id),
        );
    }

    Ok(Json(json!({ "moved": true })))
}

async fn set_favourite(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(flag): Json<Flag>,
) -> Answer {
    if is_aws_id(&id) {
        return aws::set_favourite(&state, &id, flag.on).await;
    }

    let store = state.store.lock().unwrap();

    with_key!(state, |_key| records::set_favourite(&store, &id, flag.on)).map_err(bad_request)?;

    Ok(Json(json!({ "favourite": flag.on })))
}

async fn archive_record(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(flag): Json<Flag>,
) -> Answer {
    // A16: deleting an AWS record deletes the parameter. There is no trash to put it in.
    if is_aws_id(&id) {
        return aws::delete_record(&state, &id, flag.on).await;
    }

    let store = state.store.lock().unwrap();

    with_key!(state, |_key| records::archive(&store, &id, flag.on)).map_err(bad_request)?;

    audit::note(
        &store,
        &state.lock,
        if flag.on { audit::action::RECORD_DELETED } else { audit::action::RECORD_RESTORED },
        Some(&id),
        Detail::new(),
    );

    Ok(Json(json!({ "archived": flag.on })))
}

async fn set_compromised(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(flag): Json<Flag>,
) -> Answer {
    if is_aws_id(&id) {
        return aws::set_compromised(&state, &id, flag.on).await;
    }

    let store = state.store.lock().unwrap();

    with_key!(state, |_key| records::set_compromised(&store, &id, flag.on)).map_err(bad_request)?;

    Ok(Json(json!({ "compromised": flag.on })))
}

#[derive(Deserialize)]
struct Rotation {
    #[serde(default)]
    due_at: Option<String>,
}

async fn set_rotation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Rotation>,
) -> Answer {
    if is_aws_id(&id) {
        return aws::set_rotation(&state, &id, body.due_at.as_deref()).await;
    }

    let store = state.store.lock().unwrap();

    with_key!(state, |_key| records::set_rotation_due(&store, &id, body.due_at.as_deref()))
        .map_err(bad_request)?;

    Ok(Json(json!({ "rotation_due_at": body.due_at })))
}

async fn list_versions(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    if is_aws_id(&id) {
        return aws::list_versions(&state, &id).await;
    }

    let store = state.store.lock().unwrap();

    let found = with_key!(state, |_key| records::history(&store, &id)).map_err(bad_request)?;

    Ok(Json(json!({ "versions": found })))
}

async fn read_version(
    State(state): State<Arc<AppState>>,
    Path((id, version)): Path<(String, i64)>,
) -> Answer {
    if is_aws_id(&id) {
        return aws::read_version(&state, &id, version).await;
    }

    let store = state.store.lock().unwrap();

    let payload = with_key!(state, |key| records::read_version(&store, key, &id, version))
        .map_err(bad_request)?;

    Ok(Json(json!({ "payload": payload })))
}

/// Bring an old version back.
///
/// For an AWS record the body may carry `expected_version` (or the `x-keyhold-expected-version`
/// header): the version the person had open, checked like a save (A12). Without one, the check is
/// against the current version as the restore found it. A local record needs neither, and the
/// body may be absent.
async fn restore_version(
    State(state): State<Arc<AppState>>,
    Path((id, version)): Path<(String, i64)>,
    headers: axum::http::HeaderMap,
    body: Option<Json<serde_json::Value>>,
) -> Answer {
    if is_aws_id(&id) {
        let expected = body
            .as_ref()
            .and_then(|Json(body)| body.get("expected_version"))
            .and_then(|v| v.as_i64())
            .or_else(|| {
                headers
                    .get(EXPECTED_VERSION_HEADER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse().ok())
            });

        return aws::restore_version(&state, &id, version, expected).await;
    }

    let mut store = state.store.lock().unwrap();

    let restored =
        with_key!(state, |key| records::restore_version(&mut store, key, &id, version))
            .map_err(bad_request)?;

    Ok(Json(json!({ "record": restored })))
}
