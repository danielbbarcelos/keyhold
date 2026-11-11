//! "Copiar para…": records, a folder or a whole vault, into a folder of any vault (§9.1, C1 to C7).
//!
//! A copy never moves anything. The original stays where it was until the person, at the end,
//! chooses to delete it (C1, C7). Only the current version travels (C3), and the trash does not.
//!
//! The work is split in three calls so the interface can show each step and stop between them:
//!
//! 1. `plan` lists what would be written where, finds the names already taken (C4) and checks,
//!    before anything is written, what would stop the copy halfway (C5). It returns titles and
//!    destination names, never a value.
//! 2. `run` copies one record. The interface sends them one at a time, so a failure stops the
//!    copy at a known record and "Continuar de onde parou" resends only what did not arrive.
//! 3. `originals` deletes the originals that were copied: to the trash for a local vault, for good
//!    for an AWS vault, and then only with the vault's name typed (C7).
//!
//! Every route sits behind the session token, like the rest of the interface. The agent socket has
//! no way here: an assistant cannot copy a vault to an AWS account.
//!
//! Writing goes through the same functions the record routes use, AWS included, so a copy inherits
//! every rule a save has: the KMS key is inherited or the write refused, a `String` parameter is
//! never overwritten, and the lock is checked on every step.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use super::aws;
use super::{bad_request, locked, Answer, AppState, Failure};
use crate::attachments;
use crate::audit::{self, Detail};
use crate::aws::mapping::{self, IdKind};
use crate::records::{self, Payload};
use crate::vaults;

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/copy/plan", post(plan))
        .route("/copy/run", post(run))
        .route("/copy/originals", post(remove_originals))
}

/// Seconds a record is expected to take, to say whether an AWS session lasts the whole copy.
const SECONDS_PER_RECORD: u64 = 2;

/// Margin left on a session after the estimated end of the copy.
const SESSION_MARGIN: u64 = 300;

// ── what has been copied ────────────────────────────────────────────────────────────────────────

/// Every record "Copiar para…" wrote since Keyhold started, by where it came from and where it
/// went. Two things lean on it:
///
/// - resuming: a record sent again after a stop is finished (its missing attachments added), or
///   answered as already done, never written a second time. That includes a stop caused by the
///   vault locking, which is why the ledger outlives a lock;
/// - deleting originals: only a record that arrived whole somewhere else, and is still there, may
///   have its original deleted (C7). The server decides that, not the interface.
///
/// Memory only. It holds ids and AWS parameter names, never a value; the one title it keeps (the
/// name a rename settled on, to answer a resend) is dropped when the vault locks.
#[derive(Default)]
pub struct Ledger {
    inner: std::sync::Mutex<LedgerInner>,
}

#[derive(Default)]
struct LedgerInner {
    generation: Option<u64>,
    /// `(source id, destination vault, where in it)` → what was written.
    entries: HashMap<(String, String, String), Copied>,
}

#[derive(Clone)]
struct Copied {
    source_vault: String,
    dest_id: String,
    /// Every attachment arrived. A record without them is not a copy anybody may delete the
    /// original of.
    files_ok: bool,
    title: Option<String>,
    name: Option<String>,
}

type Entries = HashMap<(String, String, String), Copied>;

impl Ledger {
    fn with<T>(&self, generation: u64, f: impl FnOnce(&mut Entries) -> T) -> T {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation != Some(generation) {
            inner.generation = Some(generation);
            for entry in inner.entries.values_mut() {
                entry.title = None;
            }
        }
        f(&mut inner.entries)
    }
}

/// Where in the destination a run writes, for telling a resend from a second copy.
fn place_key(run: &Run) -> (String, String, String) {
    let target = match run.item.action.as_str() {
        "overwrite" => format!("over:{}", run.item.dest_id.clone().unwrap_or_default()),
        _ => format!(
            "{}|{}|{}|{}",
            run.dest_folder.clone().unwrap_or_default(),
            run.item.rel.join("/"),
            run.item.name.clone().unwrap_or_default(),
            run.item.title.clone().unwrap_or_default(),
        ),
    };

    (run.item.source_id.clone(), run.dest_vault.clone(), target)
}

/// What a record id stands for: itself for a local record, the parameter (account, region, name)
/// for an AWS one, so two vault entries over the same account are seen as the same place.
fn identity(state: &AppState, id: &str) -> String {
    aws::parameter_identity(state, id).unwrap_or_else(|| id.to_string())
}

macro_rules! with_key {
    ($state:expr, |$key:ident| $body:expr) => {
        $state.lock.with_key(|$key| $body).ok_or_else(locked)?
    };
}

// ── what is being copied ────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Scope {
    source_vault: String,
    /// Copy this folder (with everything under it, itself included as a folder).
    #[serde(default)]
    folder_id: Option<String>,
    /// Copy these records, loose, straight into the destination folder.
    #[serde(default)]
    record_ids: Option<Vec<String>>,
    dest_vault: String,
    /// The destination folder: a local folder id, or an AWS folder id. `None` is the root.
    #[serde(default)]
    dest_folder: Option<String>,
}

/// One record about to be copied: where it is, and the folders it brings along.
struct Source {
    id: String,
    title: String,
    /// The folders between the copied root and the record, by name.
    rel: Vec<String>,
    /// For an AWS record, its own last segment, kept as is in another AWS vault.
    leaf: Option<String>,
}

fn is_aws(state: &AppState, vault_id: &str) -> bool {
    aws::is_aws_vault(state, vault_id)
}

fn vault_exists(state: &AppState, vault_id: &str) -> Result<vaults::VaultSummary, Failure> {
    let store = state.store.lock().unwrap();
    with_key!(state, |key| vaults::read_vault(&store, key, vault_id))
        .map_err(|_| bad_request("esse cofre não existe"))
}

/// A vault records may be written into: it exists and is not archived.
fn writable_vault(state: &AppState, vault_id: &str) -> Result<vaults::VaultSummary, Failure> {
    let vault = vault_exists(state, vault_id)?;
    if vault.archived {
        return Err(bad_request("esse cofre está arquivado. Restaure-o antes de copiar para ele"));
    }
    Ok(vault)
}

/// The names of a folder and of each folder above it, root first.
fn chain(folders: &HashMap<String, (Option<String>, String)>, mut id: Option<String>) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();

    while let Some(current) = id {
        if !seen.insert(current.clone()) {
            break;
        }
        let Some((parent, name)) = folders.get(&current) else { break };
        names.push(name.clone());
        id = parent.clone();
    }

    names.reverse();
    names
}

/// The ids of a folder and of every folder above it, root first.
fn chain_ids(folders: &HashMap<String, (Option<String>, String)>, mut id: Option<String>) -> Vec<String> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();

    while let Some(current) = id {
        if !seen.insert(current.clone()) {
            break;
        }
        ids.push(current.clone());
        id = folders.get(&current).and_then(|(parent, _)| parent.clone());
    }

    ids.reverse();
    ids
}

fn local_sources(state: &AppState, scope: &Scope) -> Result<Vec<Source>, Failure> {
    let store = state.store.lock().unwrap();

    let (folders, previews) = state.lock.with_key(|key| -> anyhow::Result<_> {
        Ok((
            vaults::list_folders(&store, key, &scope.source_vault)?,
            records::list_with_previews(&store, key, &scope.source_vault, false)?,
        ))
    }).ok_or_else(locked)?
    .map_err(bad_request)?;

    let folders: HashMap<String, (Option<String>, String)> =
        folders.into_iter().map(|f| (f.id, (f.parent_id, f.name))).collect();

    if let Some(ids) = &scope.record_ids {
        let wanted: HashSet<&String> = ids.iter().collect();
        let found: Vec<Source> = previews
            .into_iter()
            .filter(|p| wanted.contains(&p.header.id))
            .map(|p| Source { id: p.header.id, title: p.title, rel: Vec::new(), leaf: None })
            .collect();

        if found.len() != wanted.len() {
            return Err(bad_request("algum registro escolhido não está neste cofre"));
        }

        return Ok(found);
    }

    if let Some(root) = &scope.folder_id {
        if !folders.contains_key(root) {
            return Err(bad_request("essa pasta não está neste cofre"));
        }
    }

    Ok(previews
        .into_iter()
        .filter_map(|p| {
            let ids = chain_ids(&folders, p.header.folder_id.clone());
            let names = chain(&folders, p.header.folder_id.clone());

            let rel = match &scope.folder_id {
                None => names,
                Some(root) => {
                    let at = ids.iter().position(|id| id == root)?;
                    names[at..].to_vec()
                }
            };

            Some(Source { id: p.header.id, title: p.title, rel, leaf: None })
        })
        .collect())
}

async fn aws_sources(state: &AppState, scope: &Scope) -> Result<Vec<Source>, Failure> {
    let Json(listed) = aws::list_records(state, &scope.source_vault).await?;
    let records = listed["records"].as_array().cloned().unwrap_or_default();

    let root = match &scope.folder_id {
        Some(id) => Some(
            aws::folder_path(&scope.source_vault, Some(id))?
                .ok_or_else(|| bad_request("essa pasta não está neste cofre"))?,
        ),
        None => None,
    };
    let wanted: Option<HashSet<&String>> = scope.record_ids.as_ref().map(|ids| ids.iter().collect());

    let mut found = Vec::new();

    for record in records {
        let id = record["id"].as_str().unwrap_or_default().to_string();
        let Some((IdKind::Record, _, name)) = mapping::decode_id(&id) else { continue };
        let title = record["title"].as_str().unwrap_or_default().to_string();
        let leaf = Some(mapping::leaf(&name).to_string());

        if let Some(wanted) = &wanted {
            if wanted.contains(&id) {
                found.push(Source { id, title, rel: Vec::new(), leaf });
            }
            continue;
        }

        let absolute = mapping::absolute(&name);
        let parent = mapping::parent_path(&absolute).unwrap_or_default();

        let rel_path = match &root {
            None => parent.clone(),
            Some(root) => {
                let root = mapping::absolute(root);
                if parent != root && !parent.starts_with(&format!("{root}/")) {
                    continue;
                }
                // The copied folder comes along by name, then whatever is under it.
                format!("/{}{}", mapping::leaf(&root), &parent[root.len()..])
            }
        };

        let rel = rel_path.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect();
        found.push(Source { id, title, rel, leaf });
    }

    if let Some(wanted) = &wanted {
        if found.len() != wanted.len() {
            return Err(bad_request("algum registro escolhido não está neste cofre"));
        }
    }

    Ok(found)
}

async fn sources(state: &AppState, scope: &Scope) -> Result<Vec<Source>, Failure> {
    if is_aws(state, &scope.source_vault) {
        aws_sources(state, scope).await
    } else {
        local_sources(state, scope)
    }
}

// ── where it goes ───────────────────────────────────────────────────────────────────────────────

/// A folder name as a Parameter Store segment.
fn segment(name: &str) -> String {
    let suggested = mapping::suggest_name(name);
    if suggested.is_empty() {
        "pasta".to_string()
    } else {
        suggested
    }
}

/// The full parameter name a record gets in an AWS vault.
fn aws_name(base: Option<&str>, rel: &[String], leaf: &str) -> String {
    let mut path = base.map(|b| b.trim_end_matches('/').to_string()).filter(|b| !b.is_empty());

    for name in rel {
        path = Some(mapping::join(path.as_deref(), &segment(name)));
    }

    mapping::join(path.as_deref(), leaf)
}

fn aws_leaf(title: &str) -> String {
    let suggested = mapping::suggest_name(title);
    if suggested.is_empty() {
        "registro".to_string()
    } else {
        suggested
    }
}

/// A local vault's folders, and the records in each, by title, for finding names already taken.
struct LocalTarget {
    folders: Vec<vaults::FolderNode>,
    /// (folder id, lowercased title) → record id.
    titles: HashMap<(Option<String>, String), String>,
}

fn local_target(state: &AppState, vault_id: &str) -> Result<LocalTarget, Failure> {
    let store = state.store.lock().unwrap();

    let (folders, previews) = state.lock.with_key(|key| -> anyhow::Result<_> {
        Ok((
            vaults::list_folders(&store, key, vault_id)?,
            records::list_with_previews(&store, key, vault_id, false)?,
        ))
    }).ok_or_else(locked)?
    .map_err(bad_request)?;

    let titles = previews
        .into_iter()
        .map(|p| ((p.header.folder_id, p.title.trim().to_lowercase()), p.header.id))
        .collect();

    Ok(LocalTarget { folders, titles })
}

impl LocalTarget {
    /// The existing folder at `rel` under `base`, if every step of it exists.
    fn find(&self, base: Option<&str>, rel: &[String]) -> Option<Option<String>> {
        let mut at = base.map(str::to_string);

        for name in rel {
            let next = self.folders.iter().find(|f| {
                f.parent_id == at && f.name.trim().eq_ignore_ascii_case(name.trim())
            })?;
            at = Some(next.id.clone());
        }

        Some(at)
    }
}

fn check_local_folder(state: &AppState, vault_id: &str, folder: Option<&str>) -> Result<(), Failure> {
    let Some(folder) = folder else { return Ok(()) };
    let store = state.store.lock().unwrap();
    let found = with_key!(state, |key| vaults::list_folders(&store, key, vault_id)).map_err(bad_request)?;

    if found.iter().any(|f| f.id == folder) {
        Ok(())
    } else {
        Err(bad_request("a pasta de destino não está no cofre de destino"))
    }
}

// ── 1. the plan ─────────────────────────────────────────────────────────────────────────────────

fn local_attachment_count(state: &AppState, record_id: &str) -> Result<usize, Failure> {
    let store = state.store.lock().unwrap();
    Ok(with_key!(state, |key| attachments::list(&store, key, record_id)).map_err(bad_request)?.len())
}

async fn attachment_count(state: &AppState, source_aws: bool, record_id: &str) -> Result<Option<usize>, Failure> {
    if !source_aws {
        return local_attachment_count(state, record_id).map(Some);
    }

    match aws::list_attachments(state, record_id).await {
        Ok(Json(listed)) => Ok(Some(listed["attachments"].as_array().map_or(0, Vec::len))),
        // What the person can act on (the vault locked, the session ended, the role cannot read)
        // is said as itself, not as "could not list the attachments".
        Err(failure)
            if matches!(
                failure.0,
                axum::http::StatusCode::LOCKED
                    | axum::http::StatusCode::FAILED_DEPENDENCY
                    | axum::http::StatusCode::FORBIDDEN
                    | axum::http::StatusCode::UNAUTHORIZED
            ) =>
        {
            Err(failure)
        }
        Err(_) => Ok(None),
    }
}

async fn plan(State(state): State<Arc<AppState>>, Json(scope): Json<Scope>) -> Answer {
    let source = vault_exists(&state, &scope.source_vault)?;
    let dest = writable_vault(&state, &scope.dest_vault)?;
    let source_aws = source.kind == vaults::KIND_AWS;
    let dest_aws = dest.kind == vaults::KIND_AWS;

    let base = if dest_aws {
        aws::folder_path(&scope.dest_vault, scope.dest_folder.as_deref())?
    } else {
        check_local_folder(&state, &scope.dest_vault, scope.dest_folder.as_deref())?;
        None
    };

    let found = sources(&state, &scope).await?;

    let mut problems: Vec<String> = Vec::new();
    let mut counts = Vec::with_capacity(found.len());
    let mut unknown_files = 0usize;

    for one in &found {
        let count = attachment_count(&state, source_aws, &one.id).await?;
        if count.is_none() {
            unknown_files += 1;
        }
        counts.push(count);
    }

    if unknown_files > 0 {
        problems.push(format!(
            "não consegui listar os anexos de {unknown_files} registro(s) na origem. Sem saber se \
             eles têm arquivos, a cópia levaria só os campos"
        ));
    }

    let has_files = counts.iter().any(|c| c.unwrap_or(0) > 0);
    let estimate = found.len() as u64 * SECONDS_PER_RECORD;
    let now = crate::identity::now_seconds() as u64;
    let mut expiries = Vec::new();

    if source_aws {
        expiries.push(aws::copy_source(&state, &scope.source_vault).await?);
    }

    let target = if dest_aws {
        let target = aws::copy_target(&state, &scope.dest_vault, has_files).await?;
        problems.extend(target.problems.iter().cloned());
        expiries.push(target.session_expires_at);
        Some(target)
    } else {
        None
    };

    let session_expires_at = expiries.into_iter().flatten().min();
    if let Some(ends) = session_expires_at {
        if ends < now + estimate + SESSION_MARGIN {
            problems.push(
                "a sessão AWS termina antes do tempo previsto para a cópia. Renove o login em \
                 Ajustes, Contas AWS, e planeje de novo"
                    .to_string(),
            );
        }
    }

    let local = if dest_aws { None } else { Some(local_target(&state, &scope.dest_vault)?) };
    let mut planned: HashSet<String> = HashSet::new();

    let items: Vec<Value> = found
        .iter()
        .zip(&counts)
        .map(|(one, count)| {
            let mut error: Option<String> = None;

            let (path, name, conflict) = if let Some(target) = &target {
                let leaf = one.leaf.clone().unwrap_or_else(|| aws_leaf(&one.title));
                let name = aws_name(base.as_deref(), &one.rel, &leaf);

                // A title with "/" is fine: the name is made from it without the slash, and the
                // title itself only goes into the description.
                if let Err(why) = mapping::validate_name(&name) {
                    error = Some(why);
                }

                let conflict = match target.names.get(&name) {
                    Some((secure, version)) => {
                        let dest_id = mapping::record_id(&scope.dest_vault, &name);
                        // The record already there is the one being copied: writing over it and
                        // then deleting the "original" would delete the only one.
                        let itself = identity(&state, &dest_id) == identity(&state, &one.id);
                        Some(json!({
                            "dest_id": dest_id,
                            "read_only": !secure,
                            "version": version,
                            "reason": if itself { "self" } else { "exists" },
                        }))
                    }
                    None if planned.contains(&name) => Some(json!({
                        "dest_id": null, "read_only": false, "reason": "repeated",
                    })),
                    None => None,
                };
                planned.insert(name.clone());

                (name.clone(), Some(name), conflict)
            } else {
                let local = local.as_ref().expect("local target");
                let mut shown: Vec<String> = Vec::new();
                shown.extend(one.rel.iter().cloned());
                shown.push(one.title.clone());

                let key = format!("{}\u{0}{}", one.rel.join("\u{0}"), one.title.trim().to_lowercase());
                let existing = local
                    .find(scope.dest_folder.as_deref(), &one.rel)
                    .and_then(|folder| local.titles.get(&(folder, one.title.trim().to_lowercase())).cloned());

                let conflict = match existing {
                    Some(dest_id) => {
                        let itself = dest_id == one.id;
                        Some(json!({
                            "dest_id": dest_id,
                            "read_only": false,
                            "reason": if itself { "self" } else { "exists" },
                        }))
                    }
                    None if planned.contains(&key) => {
                        Some(json!({ "dest_id": null, "read_only": false, "reason": "repeated" }))
                    }
                    None => None,
                };
                planned.insert(key);

                (shown.join(" / "), None, conflict)
            };

            json!({
                "source_id": one.id,
                "title": one.title,
                "rel": one.rel,
                "path": path,
                "name": name,
                "conflict": conflict,
                "attachments": count,
                "error": error,
            })
        })
        .collect();

    Ok(Json(json!({
        "items": items,
        "source": { "id": source.id, "name": source.name, "kind": source.kind },
        "dest": { "id": dest.id, "name": dest.name, "kind": dest.kind },
        "checks": {
            "ok": problems.is_empty(),
            "problems": problems,
            "session_expires_at": session_expires_at,
            "estimated_seconds": estimate,
        },
        // C6: names and folder paths in Parameter Store are not encrypted.
        "metadata_visible": dest_aws,
    })))
}

// ── 2. copying one record ───────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RunItem {
    source_id: String,
    #[serde(default)]
    rel: Vec<String>,
    /// `create`, `overwrite` or `rename`.
    #[serde(default = "create")]
    action: String,
    /// The new title, for `rename`.
    #[serde(default)]
    title: Option<String>,
    /// The full parameter name in an AWS destination. Derived from the title when absent.
    #[serde(default)]
    name: Option<String>,
    /// The record written over, for `overwrite`.
    #[serde(default)]
    dest_id: Option<String>,
    /// For an AWS overwrite, the version the plan saw. A change since then is a 409, not a
    /// silent overwrite of somebody else's save.
    #[serde(default)]
    expected_version: Option<i64>,
}

fn create() -> String {
    "create".into()
}

#[derive(Deserialize)]
struct Run {
    source_vault: String,
    dest_vault: String,
    #[serde(default)]
    dest_folder: Option<String>,
    item: RunItem,
}

/// A record read from its vault, ready to be written into another.
struct Read {
    payload: Payload,
    favourite: bool,
    files: Vec<(String, String, Vec<u8>)>,
}

fn read_local(state: &AppState, vault_id: &str, id: &str) -> Result<Read, Failure> {
    let store = state.store.lock().unwrap();

    state.lock.with_key(|key| -> Result<Read, Failure> {
        let opened = records::open(&store, key, id).map_err(bad_request)?;

        if opened.header.vault_id != vault_id || opened.header.archived {
            return Err(bad_request("esse registro não está no cofre de origem"));
        }

        let mut files = Vec::new();
        for one in attachments::list(&store, key, id).map_err(bad_request)? {
            let file = attachments::open(&store, key, &one.id).map_err(bad_request)?;
            files.push((file.name, file.mime, file.bytes));
        }

        Ok(Read { payload: opened.payload, favourite: opened.header.favourite, files })
    }).ok_or_else(locked)?
}

async fn read_aws(state: &AppState, vault_id: &str, id: &str) -> Result<Read, Failure> {
    match mapping::decode_id(id) {
        Some((IdKind::Record, owner, _)) if owner == vault_id => {}
        _ => return Err(bad_request("esse registro não está no cofre de origem")),
    }

    let Json(opened) = aws::open_record(state, id).await?;
    let record = &opened["record"];
    let payload: Payload = serde_json::from_value(record["payload"].clone()).map_err(bad_request)?;
    let favourite = record["favourite"].as_bool().unwrap_or(false);

    let Json(listed) = aws::list_attachments(state, id).await?;
    let mut files = Vec::new();

    for one in listed["attachments"].as_array().cloned().unwrap_or_default() {
        let Some(file_id) = one["id"].as_str() else { continue };
        let Json(file) = aws::open_attachment(state, file_id).await?;
        let bytes = file["data"]
            .as_str()
            .and_then(crate::encoding::from_base64url)
            .ok_or_else(|| bad_request("um anexo da origem não chegou inteiro"))?;
        files.push((
            file["name"].as_str().unwrap_or("anexo").to_string(),
            file["mime"].as_str().unwrap_or("application/octet-stream").to_string(),
            bytes,
        ));
    }

    Ok(Read { payload, favourite, files })
}

/// Find or make the folders `rel` under `base` in a local vault.
fn local_folder(state: &AppState, vault_id: &str, base: Option<&str>, rel: &[String]) -> Result<Option<String>, Failure> {
    let store = state.store.lock().unwrap();

    state.lock.with_key(|key| -> Result<Option<String>, Failure> {
        let mut at = base.map(str::to_string);

        for name in rel {
            let existing = vaults::list_folders(&store, key, vault_id).map_err(bad_request)?;
            let found = existing
                .iter()
                .find(|f| f.parent_id == at && f.name.trim().eq_ignore_ascii_case(name.trim()))
                .map(|f| f.id.clone());

            at = Some(match found {
                Some(id) => id,
                None => {
                    vaults::create_folder(
                        &store,
                        key,
                        vault_id,
                        at.as_deref(),
                        vaults::FolderMeta { name: name.clone(), colour: None },
                    )
                    .map_err(bad_request)?
                    .id
                }
            });
        }

        Ok(at)
    }).ok_or_else(locked)?
}

/// What a write into the destination produced.
struct Written {
    dest_id: String,
    warning: Option<String>,
    files_ok: bool,
    /// The title used, when a rename had to move past names already taken.
    title: Option<String>,
    /// The parameter name used in an AWS destination.
    name: Option<String>,
}

/// "Nome (2)" from "Nome", "Nome (3)" from "Nome (2)".
fn bump_title(title: &str) -> String {
    let trimmed = title.trim_end();

    if let Some(open) = trimmed.rfind(" (") {
        let inside = &trimmed[open + 2..];
        if let Some(number) = inside.strip_suffix(')').and_then(|n| n.parse::<u32>().ok()) {
            return format!("{} ({})", &trimmed[..open], number + 1);
        }
    }

    format!("{trimmed} (2)")
}

/// `/a/senha-2` from `/a/senha`, `/a/senha-3` from `/a/senha-2`: the original leaf with a number.
fn bump_name(name: &str) -> String {
    if let Some(dash) = name.rfind('-') {
        let tail = &name[dash + 1..];
        if !tail.is_empty() && !tail.contains('/') {
            if let Ok(number) = tail.parse::<u32>() {
                return format!("{}-{}", &name[..dash], number + 1);
            }
        }
    }

    format!("{name}-2")
}

/// How many names a rename tries before giving up and saying so.
const RENAME_TRIES: u32 = 50;

/// Write into a local vault: the record and its files together, or nothing.
fn write_local(state: &AppState, run: &Run, read: &Read, payload: &Payload) -> Result<Written, Failure> {
    if run.item.action == "overwrite" {
        let id = run.item.dest_id.clone().ok_or_else(|| bad_request("falta o registro a sobrescrever"))?;
        if id == run.item.source_id {
            return Err(bad_request(
                "esse é o próprio registro que está sendo copiado. Escolha renomear ou pular",
            ));
        }

        let mut store = state.store.lock().unwrap();
        let files_ok = state.lock.with_key(|key| -> Result<bool, Failure> {
            let header = records::header(&store, &id).map_err(bad_request)?;
            if header.vault_id != run.dest_vault || header.archived {
                return Err(bad_request("o registro a sobrescrever não está no cofre de destino"));
            }
            records::update(&mut store, key, &id, payload).map_err(bad_request)?;

            let names: Vec<String> = attachments::list(&store, key, &id)
                .map_err(bad_request)?
                .into_iter()
                .map(|a| a.name)
                .collect();

            // A file with the same name is already there; the record is overwritten, its files are
            // added to, never replaced.
            for (name, mime, bytes) in &read.files {
                if !names.contains(name) {
                    attachments::add(&store, key, &id, name, mime, bytes).map_err(bad_request)?;
                }
            }
            if read.favourite {
                let _ = records::set_favourite(&store, &id, true);
            }

            Ok(true)
        }).ok_or_else(locked)??;

        return Ok(Written { dest_id: id, warning: None, files_ok, title: None, name: None });
    }

    let folder = local_folder(state, &run.dest_vault, run.dest_folder.as_deref(), &run.item.rel)?;
    let store = state.store.lock().unwrap();

    let (id, title) = state.lock.with_key(|key| -> Result<(String, String), Failure> {
        let mut payload = payload.clone();

        if run.item.action == "rename" {
            let taken: HashSet<String> = records::list_with_previews(&store, key, &run.dest_vault, false)
                .map_err(bad_request)?
                .into_iter()
                .filter(|p| p.header.folder_id == folder)
                .map(|p| p.title.trim().to_lowercase())
                .collect();

            let mut tries = 0;
            while taken.contains(&payload.title.trim().to_lowercase()) {
                tries += 1;
                if tries > RENAME_TRIES {
                    return Err(bad_request("não achei um nome livre para a cópia. Escolha um à mão"));
                }
                payload.title = bump_title(&payload.title);
            }
        }

        let made = records::create(&store, key, &run.dest_vault, folder.as_deref(), "item", &payload)
            .map_err(bad_request)?;

        // The record and its files arrive together: a file that fails takes the record back out,
        // so a resend after the failure does not leave two of it.
        let files = read
            .files
            .iter()
            .try_for_each(|(name, mime, bytes)| attachments::add(&store, key, &made.id, name, mime, bytes).map(|_| ()));

        if let Err(error) = files {
            let _ = records::archive(&store, &made.id, true).and_then(|_| records::destroy(&store, &made.id));
            return Err(bad_request(format!("um anexo não pôde ser copiado: {error}")));
        }

        if read.favourite {
            let _ = records::set_favourite(&store, &made.id, true);
        }

        Ok((made.id, payload.title))
    }).ok_or_else(locked)??;

    // Outside `with_key`: writing to the trail takes the lock itself.
    audit::note(&store, &state.lock, audit::action::RECORD_CREATED, Some(&id), Detail::new().with("kind", "item"));

    Ok(Written { dest_id: id, warning: None, files_ok: true, title: Some(title), name: None })
}

/// Write into an AWS vault, through the same paths a save takes.
///
/// The record is entered in the ledger as soon as it exists, before its files: a stop in between
/// (the vault locked, a file refused) is then finished by the next resend instead of answered
/// with "já existe" forever.
async fn write_aws(
    state: &AppState,
    run: &Run,
    read: &Read,
    payload: &Payload,
    generation: u64,
) -> Result<Written, Failure> {
    let (id, name) = if run.item.action == "overwrite" {
        let id = run.item.dest_id.clone().ok_or_else(|| bad_request("falta o registro a sobrescrever"))?;
        match mapping::decode_id(&id) {
            Some((IdKind::Record, owner, _)) if owner == run.dest_vault => {}
            _ => return Err(bad_request("o registro a sobrescrever não está no cofre de destino")),
        }
        if identity(state, &id) == identity(state, &run.item.source_id) {
            return Err(bad_request(
                "esse é o próprio parâmetro que está sendo copiado. Escolha renomear ou pular",
            ));
        }

        let Json(written) = aws::update_record(state, &id, payload, run.item.expected_version).await?;
        let warning = written["warning"].as_str().map(str::to_string);
        (id, (None, warning))
    } else {
        let base = aws::folder_path(&run.dest_vault, run.dest_folder.as_deref())?;
        let mut name = match run.item.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            Some(name) => mapping::absolute(name),
            None => aws_name(base.as_deref(), &run.item.rel, &aws_leaf(&payload.title)),
        };

        let mut tries = 0;
        let made = loop {
            match aws::create_record(state, &run.dest_vault, None, payload, Some(&name)).await {
                Ok(Json(made)) => break made,
                // A rename moves past names that are taken, here or by an earlier record of the
                // same copy, instead of failing on each resend.
                Err(failure)
                    if failure.0 == axum::http::StatusCode::CONFLICT
                        && run.item.action == "rename"
                        && tries < RENAME_TRIES =>
                {
                    tries += 1;
                    name = bump_name(&name);
                }
                Err(failure) => return Err(failure),
            }
        };

        // Folders made on the way keep the names they had: the path carries a segment, the
        // marker carries "Bancos (PJ)". Only when the segment changed it, and never fatal.
        let mut folder_warning = None;
        let mut path = base.clone();
        let full = run.item.rel.iter().fold(base.clone(), |at, folder| Some(mapping::join(at.as_deref(), &segment(folder))));
        // A name typed by hand may live elsewhere: then these folders were never made.
        let under = full.as_deref().is_some_and(|f| mapping::absolute(&name).starts_with(&format!("{}/", f.trim_end_matches('/'))));
        for folder in run.item.rel.iter().filter(|_| under) {
            let piece = segment(folder);
            path = Some(mapping::join(path.as_deref(), &piece));
            if piece != *folder {
                if let Err(failure) = aws::ensure_folder_label(state, &run.dest_vault, path.as_deref().unwrap_or_default(), folder).await {
                    folder_warning = Some(format!("a pasta “{folder}” ficou com o nome {piece}: {}", failure.1));
                }
            }
        }

        (made["record"]["id"].as_str().unwrap_or_default().to_string(), (Some(name), folder_warning))
    };
    let (name, warning) = name;

    state.copies.with(generation, |entries| {
        entries.insert(
            place_key(run),
            Copied {
                source_vault: run.source_vault.clone(),
                dest_id: id.clone(),
                files_ok: read.files.is_empty(),
                title: None,
                name: name.clone(),
            },
        );
    });

    if read.favourite {
        let _ = aws::set_favourite(state, &id, true).await;
    }

    let (files_ok, mut warnings) = add_aws_files(state, &id, read, &[]).await?;
    warnings.extend(warning);

    Ok(Written { dest_id: id, warning: join(warnings), files_ok, title: None, name })
}

/// Add to an AWS record the files it does not have yet. A refused file is said, and leaves the
/// record marked as not whole; a lock stops everything.
async fn add_aws_files(
    state: &AppState,
    id: &str,
    read: &Read,
    already: &[String],
) -> Result<(bool, Vec<String>), Failure> {
    let mut warnings = Vec::new();
    let mut files_ok = true;

    for (name, mime, bytes) in &read.files {
        if already.contains(name) {
            continue;
        }
        if let Err(failure) = aws::add_attachment(state, id, name, mime, bytes).await {
            if failure.0 == axum::http::StatusCode::LOCKED {
                return Err(failure);
            }
            files_ok = false;
            warnings.push(format!("o anexo {name} não foi copiado: {}", failure.1));
        }
    }

    Ok((files_ok, warnings))
}

fn join(warnings: Vec<String>) -> Option<String> {
    (!warnings.is_empty()).then(|| warnings.join(". "))
}

fn answer(source_id: &str, written: &Written, already: bool) -> Value {
    let mut answer = json!({
        "source_id": source_id,
        "dest_id": written.dest_id,
        "files_ok": written.files_ok,
        "already": already,
    });
    if let Some(warning) = &written.warning {
        answer["warning"] = json!(warning);
    }
    if let Some(title) = &written.title {
        answer["title"] = json!(title);
    }
    if let Some(name) = &written.name {
        answer["name"] = json!(name);
    }
    answer
}

async fn run(State(state): State<Arc<AppState>>, Json(run): Json<Run>) -> Answer {
    let generation = state.lock.generation().ok_or_else(locked)?;
    let source_aws = is_aws(&state, &run.source_vault);
    let dest_aws = is_aws(&state, &run.dest_vault);
    vault_exists(&state, &run.source_vault)?;
    writable_vault(&state, &run.dest_vault)?;

    if !matches!(run.item.action.as_str(), "create" | "overwrite" | "rename") {
        return Err(bad_request("a ação de cópia é criar, sobrescrever ou renomear"));
    }

    // Sent again after a stop: finish it, or say it is done. Never a second copy.
    let earlier = state.copies.with(generation, |entries| entries.get(&place_key(&run)).cloned());

    if let Some(earlier) = earlier {
        let mut written = Written {
            dest_id: earlier.dest_id.clone(),
            warning: None,
            files_ok: earlier.files_ok,
            title: earlier.title.clone(),
            name: earlier.name.clone(),
        };

        if !earlier.files_ok && dest_aws {
            let read = if source_aws {
                read_aws(&state, &run.source_vault, &run.item.source_id).await?
            } else {
                read_local(&state, &run.source_vault, &run.item.source_id)?
            };
            let already = aws::attachment_names(&state, &earlier.dest_id).await?;
            let (files_ok, warnings) = add_aws_files(&state, &earlier.dest_id, &read, &already).await?;

            written.files_ok = files_ok;
            written.warning = join(warnings);
            state.copies.with(generation, |entries| {
                if let Some(entry) = entries.get_mut(&place_key(&run)) {
                    entry.files_ok = files_ok;
                }
            });
        }

        return Ok(Json(answer(&run.item.source_id, &written, true)));
    }

    let read = if source_aws {
        read_aws(&state, &run.source_vault, &run.item.source_id).await?
    } else {
        read_local(&state, &run.source_vault, &run.item.source_id)?
    };

    let mut payload = read.payload.clone();
    if run.item.action == "rename" {
        let title = run.item.title.as_deref().map(str::trim).filter(|t| !t.is_empty());
        payload.title = title.ok_or_else(|| bad_request("o novo título ficou vazio"))?.to_string();
    }

    let written = if dest_aws {
        write_aws(&state, &run, &read, &payload, generation).await?
    } else {
        write_local(&state, &run, &read, &payload)?
    };

    state.copies.with(generation, |entries| {
        entries.insert(
            place_key(&run),
            Copied {
                source_vault: run.source_vault.clone(),
                dest_id: written.dest_id.clone(),
                files_ok: written.files_ok,
                title: written.title.clone(),
                name: written.name.clone(),
            },
        );
    });

    {
        let store = state.store.lock().unwrap();
        audit::note(
            &store,
            &state.lock,
            audit::action::RECORD_COPIED,
            Some(&written.dest_id),
            Detail::new()
                .with("from_record", run.item.source_id.clone())
                .with("from_vault", run.source_vault.clone())
                .with("to_vault", run.dest_vault.clone())
                .with("action", run.item.action.clone())
                .with("attachments", read.files.len())
                .with("files_ok", written.files_ok),
        );
    }

    Ok(Json(answer(&run.item.source_id, &written, false)))
}

// ── 3. the originals ────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Originals {
    source_vault: String,
    record_ids: Vec<String>,
    /// The vault's name, typed. Required for an AWS vault, where deleting is for good.
    #[serde(default)]
    confirm_name: Option<String>,
}

/// Why an original may not be deleted, or `None` when it may: it was copied whole in this unlock,
/// to somewhere that is not itself, and that copy is still there.
async fn not_deletable(
    state: &AppState,
    source_vault: &str,
    id: &str,
    copies: &[Copied],
    present: &mut HashMap<String, HashSet<String>>,
) -> Result<Option<&'static str>, Failure> {
    let whole: Vec<&Copied> = copies.iter().filter(|c| c.source_vault == source_vault && c.files_ok).collect();

    if whole.is_empty() {
        return Ok(Some("não foi copiado inteiro nesta sessão"));
    }

    let own = identity(state, id);

    for copy in whole {
        if identity(state, &copy.dest_id) == own {
            continue;
        }

        if exists(state, &copy.dest_id, present).await? {
            return Ok(None);
        }
    }

    Ok(Some("a cópia não está mais no destino"))
}

/// Whether a copy is still where it was written.
async fn exists(state: &AppState, id: &str, present: &mut HashMap<String, HashSet<String>>) -> Result<bool, Failure> {
    if let Some((IdKind::Record, vault_id, _)) = mapping::decode_id(id) {
        if !present.contains_key(&vault_id) {
            let Json(listed) = aws::list_records(state, &vault_id).await?;
            let ids = listed["records"]
                .as_array()
                .map(|all| all.iter().filter_map(|r| r["id"].as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            present.insert(vault_id.clone(), ids);
        }
        return Ok(present[&vault_id].contains(id));
    }

    let store = state.store.lock().unwrap();
    Ok(records::header(&store, id).map(|h| !h.archived).unwrap_or(false))
}

async fn remove_originals(State(state): State<Arc<AppState>>, Json(body): Json<Originals>) -> Answer {
    let generation = state.lock.generation().ok_or_else(locked)?;
    let vault = vault_exists(&state, &body.source_vault)?;
    let aws_source = vault.kind == vaults::KIND_AWS;

    if aws_source {
        let typed = body.confirm_name.as_deref().map(str::trim).unwrap_or_default();
        if typed != vault.name.trim() {
            return Err(bad_request(
                "para apagar na AWS, digite o nome do cofre exatamente como ele é. Lá não há lixeira",
            ));
        }
    }

    let ledger: HashMap<String, Vec<Copied>> = state.copies.with(generation, |entries| {
        let mut by_source: HashMap<String, Vec<Copied>> = HashMap::new();
        for ((source, _, _), copied) in entries.iter() {
            by_source.entry(source.clone()).or_default().push(copied.clone());
        }
        by_source
    });

    let mut present = HashMap::new();
    let mut removed = 0usize;
    let mut skipped = 0usize;
    let mut failed: Vec<Value> = Vec::new();

    for id in &body.record_ids {
        let copies = ledger.get(id).cloned().unwrap_or_default();

        if let Some(why) = not_deletable(&state, &body.source_vault, id, &copies, &mut present).await? {
            failed.push(json!({ "id": id, "error": why }));
            continue;
        }

        if aws_source {
            match mapping::decode_id(id) {
                Some((IdKind::Record, owner, _)) if owner == body.source_vault => {}
                _ => {
                    failed.push(json!({ "id": id, "error": "não é um registro deste cofre" }));
                    continue;
                }
            }

            match aws::delete_record(&state, id, true).await {
                Ok(_) => removed += 1,
                Err(failure) if failure.0 == axum::http::StatusCode::LOCKED => return Err(failure),
                Err(failure) => failed.push(json!({ "id": id, "error": failure.1 })),
            }
            continue;
        }

        let store = state.store.lock().unwrap();
        let outcome = state.lock.with_key(|_key| -> anyhow::Result<bool> {
            let header = records::header(&store, id)?;
            if header.vault_id != body.source_vault {
                anyhow::bail!("não é um registro deste cofre");
            }
            // Already in the trash: archiving again would restart its retention clock.
            if header.archived {
                return Ok(false);
            }
            records::archive(&store, id, true)?;
            Ok(true)
        }).ok_or_else(locked)?;

        match outcome {
            Ok(true) => {
                audit::note(&store, &state.lock, audit::action::RECORD_DELETED, Some(id), Detail::new());
                removed += 1;
            }
            Ok(false) => skipped += 1,
            Err(error) => failed.push(json!({ "id": id, "error": error.to_string() })),
        }
    }

    Ok(Json(json!({ "removed": removed, "skipped": skipped, "failed": failed, "trash": !aws_source })))
}

// Tests live in `http::aws::tests`, whose harness has a fake Parameter Store next to local vaults.
