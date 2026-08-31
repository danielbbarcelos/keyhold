//! Vaults, folders and records over HTTP.
//!
//! Every route here needs the master key, and there is exactly one way to get it: `with_key`, which
//! goes through the lock. That is not a convention — it is the reason the auto-lock deadline is
//! real. There is no path in this file that reaches a plaintext payload without the clock having
//! been consulted first, and a request that arrives one second late gets a 423 rather than a secret.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use super::{bad_request, locked, Answer, AppState};
use crate::health;
use crate::records::{self, Payload};
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
        .route("/records/:id/favourite", post(set_favourite))
        .route("/records/:id/archive", post(archive_record))
        .route("/records/:id/compromised", post(set_compromised))
        .route("/records/:id/rotation", post(set_rotation))
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

    Ok(Json(json!({ "vault": rotated })))
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
    let store = state.store.lock().unwrap();

    with_key!(state, |_key| records::destroy(&store, &id)).map_err(bad_request)?;

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

    Ok(Json(json!({ "destroyed": count })))
}

// ── folders ─────────────────────────────────────────────────────────────────────────────────────

async fn list_folders(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
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
    let store = state.store.lock().unwrap();

    with_key!(state, |_key| vaults::move_folder(&store, &id, body.parent_id.as_deref()))
        .map_err(bad_request)?;

    Ok(Json(json!({ "moved": true })))
}

async fn delete_folder(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let store = state.store.lock().unwrap();

    with_key!(state, |_key| vaults::delete_folder(&store, &id)).map_err(bad_request)?;

    Ok(Json(json!({ "deleted": true })))
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
    kind: String,
    #[serde(default)]
    folder_id: Option<String>,
    payload: Payload,
}

async fn create_record(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<NewRecord>,
) -> Answer {
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

    Ok(Json(json!({ "record": made })))
}

async fn open_record(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let store = state.store.lock().unwrap();

    let opened = with_key!(state, |key| records::open(&store, key, &id)).map_err(bad_request)?;

    Ok(Json(json!({ "record": opened })))
}

async fn update_record(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<Payload>,
) -> Answer {
    let mut store = state.store.lock().unwrap();

    let updated = with_key!(state, |key| records::update(&mut store, key, &id, &payload))
        .map_err(bad_request)?;

    Ok(Json(json!({ "record": updated })))
}

#[derive(Deserialize)]
struct MoveRecord {
    #[serde(default)]
    folder_id: Option<String>,
}

async fn move_record(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<MoveRecord>,
) -> Answer {
    let store = state.store.lock().unwrap();

    with_key!(state, |_key| records::move_to_folder(&store, &id, body.folder_id.as_deref()))
        .map_err(bad_request)?;

    Ok(Json(json!({ "moved": true })))
}

async fn set_favourite(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(flag): Json<Flag>,
) -> Answer {
    let store = state.store.lock().unwrap();

    with_key!(state, |_key| records::set_favourite(&store, &id, flag.on)).map_err(bad_request)?;

    Ok(Json(json!({ "favourite": flag.on })))
}

async fn archive_record(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(flag): Json<Flag>,
) -> Answer {
    let store = state.store.lock().unwrap();

    with_key!(state, |_key| records::archive(&store, &id, flag.on)).map_err(bad_request)?;

    Ok(Json(json!({ "archived": flag.on })))
}

async fn set_compromised(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(flag): Json<Flag>,
) -> Answer {
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
    let store = state.store.lock().unwrap();

    with_key!(state, |_key| records::set_rotation_due(&store, &id, body.due_at.as_deref()))
        .map_err(bad_request)?;

    Ok(Json(json!({ "rotation_due_at": body.due_at })))
}

async fn list_versions(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Answer {
    let store = state.store.lock().unwrap();

    let found = with_key!(state, |_key| records::history(&store, &id)).map_err(bad_request)?;

    Ok(Json(json!({ "versions": found })))
}

async fn read_version(
    State(state): State<Arc<AppState>>,
    Path((id, version)): Path<(String, i64)>,
) -> Answer {
    let store = state.store.lock().unwrap();

    let payload = with_key!(state, |key| records::read_version(&store, key, &id, version))
        .map_err(bad_request)?;

    Ok(Json(json!({ "payload": payload })))
}

async fn restore_version(
    State(state): State<Arc<AppState>>,
    Path((id, version)): Path<(String, i64)>,
) -> Answer {
    let mut store = state.store.lock().unwrap();

    let restored =
        with_key!(state, |key| records::restore_version(&mut store, key, &id, version))
            .map_err(bad_request)?;

    Ok(Json(json!({ "record": restored })))
}
