//! The agent surface: a local assistant looking things up, with the person still deciding.
//!
//! # The shape of it
//!
//! ```text
//! Claude Code ──stdio(MCP)──> `keyhold mcp` ──unix socket 0600──> the running keyhold
//! ```
//!
//! The middle process holds no key and no state; it translates. The daemon is the only thing that
//! can open anything, and it is where every decision is made.
//!
//! # What actually protects you
//!
//! Telling a model not to leak a secret is not a security control. These are:
//!
//! 1. **Scope by marking, not by permission.** A vault an agent may see is marked as such, off by
//!    default. What is not marked is not listed — there is nothing for the agent to ask about,
//!    rather than something for it to be refused.
//! 2. **Approval per request.** Every value handed over is a question answered in the interface,
//!    naming the process and the PID that asked. The yes carries a window — once, while that
//!    process lives, a fixed span, or no end — and covers one field of one record. See
//!    [`crate::grants`] for why those windows can outlive this process, and what keeps that from
//!    being a decision made once and forgotten.
//! 3. **A locked vault is a dead integration.** Nothing works locked, and the socket never accepts
//!    a master password — unlocking happens in the interface, by hand.
//! 4. **The trail records the agent as its own actor**, including refusals.
//! 5. **`run` beats `get`.** A command executed with the secret in a child process's environment
//!    keeps the value out of the model's context entirely. It is the path the tool descriptions
//!    push towards.
//!
//! And the thing the interface has to say out loud: **a value handed to an assistant leaves this
//! machine.** That is the opposite of everything else keyhold promises, which is why it takes a
//! deliberate yes.

pub mod protocol;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::audit::{self, Detail};
use crate::http::AppState;
use crate::identity::now_seconds;
use crate::records;
use crate::vaults;
use protocol::{FieldSummary, Peer, RecordSummary, Request, Response};

/// How long a request waits for somebody to answer it.
///
/// Long enough to notice the notification and read it; short enough that an agent left running
/// overnight does not hold a question open until morning.
const APPROVAL_TIMEOUT_SECONDS: u64 = 90;

/// A question waiting for an answer, as the interface sees it.
#[derive(Clone, Debug, Serialize)]
pub struct PendingRequest {
    pub id: String,
    pub peer: Peer,
    pub record_id: String,
    pub record_title: String,
    pub field: String,
    /// `get` or `run` — what will happen to the value if this is allowed.
    pub intent: String,
    pub asked_at: u64,
}

/// The questions in flight, and the answers to them. Only that.
///
/// A yes that outlives the question is not here — it is a row in the vault, sealed, listed and
/// revocable. See [`crate::grants`]. This holds what cannot outlive the process anyway: a question
/// nobody has answered yet, and an answer nobody has collected yet.
#[derive(Default)]
pub struct AgentState {
    pending: Mutex<Vec<PendingRequest>>,
    answers: Mutex<HashMap<String, bool>>,
    /// The interface's own URL, token included, so a desktop question can put the window back.
    /// Set once by the binary; absent in tests, where nothing should open a browser.
    door: OnceLock<String>,
    /// When the interface last polled while somebody was actually looking at it, in seconds.
    looked_at: AtomicU64,
    /// One desktop question at a time. A burst of requests is one knock; the interface lists them all.
    asking: AtomicBool,
}

impl AgentState {
    /// Where the interface lives, token included. `None` until the binary sets it, and in tests.
    pub fn door(&self) -> Option<&str> {
        self.door.get().map(String::as_str)
    }

    /// Tell the agent surface where the interface lives, so it can open it when it has to ask.
    pub fn set_door(&self, url: String) {
        let _ = self.door.set(url);
    }

    /// The interface polled with its window focused and visible.
    pub fn looked(&self) {
        self.looked_at.store(now_seconds(), Ordering::Relaxed);
    }

    /// Whether somebody is looking at the interface right now. The interface polls every second,
    /// so three seconds of silence means the tab is in the background, minimised, or closed.
    fn somebody_looking(&self) -> bool {
        now_seconds().saturating_sub(self.looked_at.load(Ordering::Relaxed)) <= 3
    }

    pub fn waiting(&self) -> Vec<PendingRequest> {
        self.pending.lock().unwrap().clone()
    }

    /// The interface answering one question. Returns what was asked, so the caller can turn a yes
    /// into a standing permission — which is a decision about the vault, and so belongs in it.
    pub fn answer(&self, id: &str, allowed: bool) -> Option<PendingRequest> {
        let asked = {
            let mut pending = self.pending.lock().unwrap();
            let at = pending.iter().position(|one| one.id == id);

            at.map(|at| pending.remove(at))
        };

        self.answers.lock().unwrap().insert(id.to_string(), allowed);

        asked
    }

    /// Drop every question still waiting. Standing permissions are revoked separately, in the
    /// vault, because that is where they live.
    pub fn forget_pending(&self) {
        self.pending.lock().unwrap().clear();
    }
}

/// Where the socket lives.
pub fn socket_path() -> std::path::PathBuf {
    crate::paths::state_dir().join("agent.sock")
}

/// Start listening, replacing any socket left behind by a previous run.
///
/// The listener lives in the spawned task for as long as the process does; there is no handle to
/// hold on to, because there is nothing a caller would do with one. Stopping the surface means
/// deleting the socket and letting the accept loop end — see [`stop`].
pub fn listen(state: Arc<AppState>) -> Result<()> {
    let path = socket_path();

    crate::paths::ensure_private_dir(&crate::paths::state_dir())?;

    // A socket file from a crashed run would make bind fail. Removing it is safe: if another
    // keyhold were actually listening, the bind below would still be the one that succeeds and the
    // other process's socket is already unreachable through this path.
    let _ = std::fs::remove_file(&path);

    let listener = UnixListener::bind(&path)
        .with_context(|| format!("could not open the agent socket at {}", path.display()))?;

    restrict(&path)?;

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let state = state.clone();
                    tokio::spawn(async move {
                        let _ = serve(stream, state).await;
                    });
                }
                Err(_) => break,
            }
        }
    });

    Ok(())
}

/// Take the socket away.
///
/// Removing the file is what makes the integration stop existing rather than start refusing: a new
/// connection has nowhere to land. Connections already open finish what they were doing, which is
/// at most one question that the vault will answer or refuse on its own terms.
pub fn stop() {
    let _ = std::fs::remove_file(socket_path());
}

/// Whether the socket is there to be connected to.
pub fn listening() -> bool {
    socket_path().exists()
}

#[cfg(unix)]
fn restrict(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)?;

    Ok(())
}

/// Who is on the other end, from the kernel rather than from the client.
///
/// The process on the socket is always `keyhold mcp` — it is the translator, and naming it would
/// tell somebody nothing. What is worth showing is what launched it, so the parent is read too:
/// that is `claude`, or whatever else is driving. Both come from `/proc`, never from anything the
/// client says about itself.
fn peer_of(stream: &UnixStream) -> Peer {
    let credentials = stream.peer_cred().ok();
    let pid = credentials.and_then(|c| c.pid()).unwrap_or(0) as i64;

    let name_of = |pid: i64| {
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
    };

    // Field 4 of /proc/<pid>/stat is the parent's pid. The comm field before it can contain spaces
    // and parentheses, so the parse starts after its closing one.
    let parent_pid = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok().and_then(|stat| {
        let after = stat.rfind(')')?;

        stat[after + 1..].split_whitespace().nth(1)?.parse::<i64>().ok()
    });

    Peer {
        pid,
        process: name_of(pid).unwrap_or_else(|| "desconhecido".into()),
        parent: parent_pid.and_then(name_of),
        parent_pid,
    }
}

async fn serve(stream: UnixStream, state: Arc<AppState>) -> Result<()> {
    let peer = peer_of(&stream);
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => handle(request, &state, &peer).await,
            Err(_) => Response::refused("pedido malformado"),
        };

        writer.write_all(serde_json::to_string(&response)?.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
    }

    Ok(())
}

/// The vaults an agent is allowed to know about at all.
fn visible_vaults(state: &AppState) -> Vec<String> {
    let store = state.store.lock().unwrap();

    state
        .lock
        .with_key(|key| {
            vaults::list_vaults(&store, key)
                .map(|found| {
                    found
                        .into_iter()
                        .filter(|vault| vault.agent_access && !vault.archived)
                        .map(|vault| vault.id)
                        .collect()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

/// The same path a socket connection takes, for tests that would otherwise have to spawn a client.
///
/// The peer is invented here because the kernel is not involved; everything else — the scope check,
/// the approval, the trail — is exactly what a real connection goes through.
#[doc(hidden)]
pub async fn handle_for_tests(request: Request, state: &Arc<AppState>) -> Response {
    handle(
        request,
        state,
        &Peer {
            pid: std::process::id() as i64,
            process: "teste".into(),
            parent: None,
            parent_pid: None,
        },
    )
    .await
}

async fn handle(request: Request, state: &Arc<AppState>, peer: &Peer) -> Response {
    if state.lock.with_key(|_| ()).is_none() {
        return Response::refused("o cofre está trancado");
    }

    match request {
        Request::Status => {
            let allowed = visible_vaults(state);
            let store = state.store.lock().unwrap();

            let records = allowed
                .iter()
                .filter_map(|vault| records::list(&store, vault, false).ok())
                .map(|found| found.len())
                .sum();

            let may_reveal = crate::settings::agent_may_reveal(&store);

            Response::Status {
                unlocked: true,
                vaults: allowed,
                records,
                may_reveal,
                may_copy: crate::clipboard::available(),
            }
        }

        Request::Search { query } => search(state, &query),

        Request::Describe { record } => describe(state, &record),

        Request::Get { record, field } => {
            // The refusal that matters: a value an agent never receives is a value it cannot print
            // into a conversation, however it was asked and whatever it was told.
            let allowed = {
                let store = state.store.lock().unwrap();
                crate::settings::agent_may_reveal(&store)
            };

            if !allowed {
                note_agent(state, peer, audit::action::SECRET_REVEALED, &record, &field, false);

                return Response::refused(
                    "este cofre não entrega valores a um assistente. Use keyhold_copy para pôr o \
                     valor na área de transferência da pessoa, ou keyhold_run para executar um \
                     comando com ele.",
                );
            }

            match approved(state, peer, &record, &field, "get").await {
                Err(refusal) => refusal,
                Ok(()) => match value_of(state, &record, &field) {
                    Some(value) => {
                        note_agent(state, peer, audit::action::SECRET_REVEALED, &record, &field, true);

                        Response::Value { value }
                    }
                    None => Response::refused("esse campo não existe neste registro"),
                },
            }
        }

        Request::Copy { record, field } => {
            match approved(state, peer, &record, &field, "copy").await {
                Err(refusal) => refusal,
                Ok(()) => match value_of(state, &record, &field) {
                    None => Response::refused("esse campo não existe neste registro"),
                    Some(value) => match crate::clipboard::copy_for_a_while(&value) {
                        Err(error) => Response::refused(error.to_string()),
                        Ok(seconds) => {
                            note_agent(
                                state,
                                peer,
                                audit::action::SECRET_COPIED,
                                &record,
                                &field,
                                true,
                            );

                            Response::Copied { field, seconds }
                        }
                    },
                },
            }
        }

        Request::Run { record, field, command, variable } => {
            if command.is_empty() {
                return Response::refused("nenhum comando para executar");
            }

            match approved(state, peer, &record, &field, "run").await {
                Err(refusal) => refusal,
                Ok(()) => match value_of(state, &record, &field) {
                    None => Response::refused("esse campo não existe neste registro"),
                    Some(value) => {
                        note_agent(state, peer, audit::action::SECRET_REVEALED, &record, &field, true);

                        run_with(&command, &variable, &value)
                    }
                },
            }
        }
    }
}

fn search(state: &Arc<AppState>, query: &str) -> Response {
    // Same reason as `reachable`: this takes the store lock, so it runs before we take it here.
    let allowed = visible_vaults(state);
    let store = state.store.lock().unwrap();
    let needle = query.trim().to_lowercase();

    let found = state.lock.with_key(|key| {
        let mut found = Vec::new();

        for vault_id in &allowed {
            let name = vaults::read_vault(&store, key, vault_id)
                .map(|vault| vault.name)
                .unwrap_or_default();

            let Ok(previews) = records::list_with_previews(&store, key, vault_id, false) else {
                continue;
            };

            for preview in previews {
                let haystack = format!("{} {} {}", preview.title, preview.subtitle, preview.url)
                    .to_lowercase();

                if !needle.is_empty() && !haystack.contains(&needle) {
                    continue;
                }

                found.push(RecordSummary {
                    id: preview.header.id,
                    title: preview.title,
                    username: preview.subtitle,
                    url: preview.url,
                    vault: name.clone(),
                    kind: preview.header.kind,
                });
            }
        }

        found
    });

    Response::Records { records: found.unwrap_or_default() }
}

/// Whether this record is one the agent is allowed to touch.
///
/// The list of allowed vaults is fetched **before** the store is locked, and deliberately so: it
/// takes the same lock, and `std::sync::Mutex` is not reentrant. Taking it twice on one thread does
/// not fail — it waits, forever, for itself.
fn reachable(state: &Arc<AppState>, record_id: &str) -> bool {
    let allowed = visible_vaults(state);
    let store = state.store.lock().unwrap();

    match records::header(&store, record_id) {
        Ok(header) => allowed.contains(&header.vault_id) && !header.archived,
        Err(_) => false,
    }
}

fn describe(state: &Arc<AppState>, record_id: &str) -> Response {
    if !reachable(state, record_id) {
        return Response::refused("esse registro não está ao alcance de um agente");
    }

    let store = state.store.lock().unwrap();

    let opened = state
        .lock
        .with_key(|key| records::open(&store, key, record_id).ok())
        .flatten();

    match opened {
        None => Response::refused("esse registro não abriu"),
        Some(record) => Response::Fields {
            title: record.payload.title,
            fields: record
                .payload
                .fields
                .iter()
                .map(|field| FieldSummary {
                    label: field.label.clone(),
                    kind: field.kind.clone(),
                    secret: records::is_secret_kind(&field.kind),
                })
                .collect(),
        },
    }
}

fn value_of(state: &Arc<AppState>, record_id: &str, field: &str) -> Option<String> {
    let store = state.store.lock().unwrap();

    state
        .lock
        .with_key(|key| {
            let record = records::open(&store, key, record_id).ok()?;

            if field == "username" {
                return Some(record.payload.subtitle()).filter(|s| !s.is_empty());
            }

            let one = record.payload.fields.iter().find(|one| one.label.eq_ignore_ascii_case(field))?;

            // A 2FA field hands out the code of the moment, never the secret that makes codes.
            if one.kind == "totp" {
                return crate::totp::code_for_field(&one.value, now_seconds())
                    .map(|(code, _)| code);
            }

            Some(one.value.clone())
        })
        .flatten()
}

/// Ask the person, unless they already said yes for this field.
async fn approved(
    state: &Arc<AppState>,
    peer: &Peer,
    record_id: &str,
    field: &str,
    intent: &str,
) -> std::result::Result<(), Response> {
    if !reachable(state, record_id) {
        note_agent(state, peer, audit::action::SECRET_REVEALED, record_id, field, false);

        return Err(Response::refused("esse registro não está ao alcance de um agente"));
    }

    // A standing permission answers before anybody is asked. Read fresh every time rather than
    // cached: revoking one has to take effect at once, and the clock moves.
    let standing = {
        let store = state.store.lock().unwrap();

        state
            .lock
            .with_key(|key| crate::grants::allows(&store, key, record_id, field))
            .unwrap_or(false)
    };

    if standing {
        return Ok(());
    }

    let title = {
        let store = state.store.lock().unwrap();

        state
            .lock
            .with_key(|key| {
                records::open(&store, key, record_id)
                    .map(|record| record.payload.title)
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    };

    let id = vaults::new_id();

    state.agent.pending.lock().unwrap().push(PendingRequest {
        id: id.clone(),
        peer: peer.clone(),
        record_id: record_id.to_string(),
        record_title: title,
        field: field.to_string(),
        intent: intent.to_string(),
        asked_at: now_seconds(),
    });

    // The window the dialog lives in may be closed, behind another one, or on another workspace.
    // The desktop question goes away with this function: answered, refused or expired.
    let _knock = DesktopKnock::start(state.clone(), id.clone());

    // Poll rather than push: the answer comes from an HTTP request on another task, and a channel
    // between them would have to be cleaned up on every path this can end on — including the one
    // where nobody ever answers.
    let deadline = now_seconds() + APPROVAL_TIMEOUT_SECONDS;

    while now_seconds() < deadline {
        if let Some(allowed) = state.agent.answers.lock().unwrap().remove(&id) {
            if allowed {
                return Ok(());
            }

            note_agent(state, peer, audit::action::SECRET_REVEALED, record_id, field, false);

            return Err(Response::refused("a pessoa negou este pedido"));
        }

        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    state.agent.pending.lock().unwrap().retain(|one| one.id != id);
    note_agent(state, peer, audit::action::SECRET_REVEALED, record_id, field, false);

    Err(Response::refused("ninguém respondeu ao pedido a tempo"))
}

/// The question, asked on the desktop as well, for as long as it is open.
///
/// A browser notification only reaches the desktop while the tab exists and was allowed to notify,
/// and a tab in the background is throttled anyway. The process asking is this one, which is
/// always running while an agent can ask anything, so it asks the way a second start does: a
/// native dialog on top of whatever has focus. It answers nothing by itself. "Abrir o Keyhold"
/// only brings the interface forward, where the decision is made with the record and the window
/// in front of you; a yes from a dialog that cannot show either would be a yes to less.
///
/// Dropping it takes the dialog back, so a question that was settled in the interface, or that
/// expired, does not stay on screen asking to be answered.
struct DesktopKnock(Option<tokio::task::JoinHandle<()>>);

impl DesktopKnock {
    fn start(state: Arc<AppState>, id: String) -> Self {
        let Some(door) = state.agent.door.get().cloned() else {
            return Self(None);
        };

        let task = tokio::spawn(async move {
            // A focused interface polls within a second and shows the dialog itself. Give it that
            // second before deciding nobody is looking.
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

            let still_waiting = state.agent.waiting().into_iter().find(|one| one.id == id);
            let Some(request) = still_waiting else { return };

            if state.agent.somebody_looking() {
                return;
            }

            if state.agent.asking.swap(true, Ordering::SeqCst) {
                return;
            }

            let _release = Release(&state.agent.asking);

            if crate::instance::ask_to_open(&knock_text(&request)).await {
                crate::instance::open_browser(&door);
            }
        });

        Self(Some(task))
    }
}

impl Drop for DesktopKnock {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            // Aborting drops the dialog's child process, which is spawned to die with it.
            task.abort();
        }
    }
}

/// Clears the "already asking" flag however the knock ends, including by being aborted.
struct Release<'a>(&'a AtomicBool);

impl Drop for Release<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// What the desktop is told. The record's title and the field's label, never a value — the same
/// two things the dialog in the interface names before anybody decides anything.
fn knock_text(request: &PendingRequest) -> String {
    let who = request.peer.parent.as_deref().unwrap_or(&request.peer.process);
    let what = if request.intent == "run" {
        format!("usar {} de \"{}\" num comando", request.field, request.record_title)
    } else {
        format!("ler {} de \"{}\"", request.field, request.record_title)
    };

    format!(
        "{who} (PID {}) quer {what}.\n\nO pedido expira em {APPROVAL_TIMEOUT_SECONDS} segundos. \
         Abra o Keyhold para permitir ou negar.",
        request.peer.parent_pid.unwrap_or(request.peer.pid)
    )
}

/// Run a command with the secret in the child's environment.
///
/// The value goes in through the environment rather than the command line: `/proc/*/cmdline` is
/// readable by every process of this user, and `/proc/*/environ` is not. It never comes back —
/// only what the command printed, which is the whole point of preferring this over handing the
/// value to a model.
fn run_with(command: &[String], variable: &str, value: &str) -> Response {
    let variable = if variable.trim().is_empty() { "KEYHOLD_SECRET" } else { variable };

    let output = std::process::Command::new(&command[0])
        .args(&command[1..])
        .env(variable, value)
        .output();

    match output {
        Err(error) => Response::refused(format!("não foi possível executar: {error}")),
        Ok(output) => Response::Ran {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        },
    }
}

fn note_agent(
    state: &Arc<AppState>,
    peer: &Peer,
    action: &str,
    record_id: &str,
    field: &str,
    allowed: bool,
) {
    let store = state.store.lock().unwrap();

    let _ = audit::record(
        &store,
        &state.lock,
        audit::Actor::Agent { process: peer.process.clone(), pid: peer.pid },
        action,
        Some(record_id),
        if allowed { audit::Outcome::Allowed } else { audit::Outcome::Denied },
        Detail::new().with("field", field).with("asked_by", peer.describe()),
    );
}
