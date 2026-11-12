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

use anyhow::Result;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

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

/// Where the socket lives: a file in the state directory on Unix, a named pipe on Windows.
pub fn socket_path() -> std::path::PathBuf {
    #[cfg(unix)]
    return crate::paths::state_dir().join("agent.sock");

    #[cfg(windows)]
    return std::path::PathBuf::from(pipe::name());
}

/// Start listening, replacing any socket left behind by a previous run.
///
/// The listener lives in the spawned task for as long as the process does; there is no handle to
/// hold on to, because there is nothing a caller would do with one. Stopping the surface means
/// deleting the socket and letting the accept loop end — see [`stop`].
#[cfg(unix)]
pub fn listen(state: Arc<AppState>) -> Result<()> {
    use anyhow::Context;
    use tokio::net::UnixListener;

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
                    let pid = stream.peer_cred().ok().and_then(|c| c.pid()).unwrap_or(0);
                    let state = state.clone();
                    tokio::spawn(async move {
                        let _ = serve(stream, peer_of(pid), state).await;
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
#[cfg(unix)]
pub fn stop() {
    let _ = std::fs::remove_file(socket_path());
}

/// Whether the socket is there to be connected to.
#[cfg(unix)]
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

/// The agent's door on Windows: a named pipe, per user.
///
/// A pipe has no file to delete, so "stopped" is a switch the accept loop reads: a connection that
/// arrives while it is off is dropped unanswered. Every connection is also checked against the
/// kernel's idea of who the client is ([`crate::procinfo::is_mine`]): the pipe namespace is shared
/// by every user of the machine, and only this user's processes get an answer.
#[cfg(windows)]
pub mod pipe {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use anyhow::{Context, Result};
    use tokio::net::windows::named_pipe::ServerOptions;

    use crate::http::AppState;

    static ON: AtomicBool = AtomicBool::new(false);
    static RUNNING: AtomicBool = AtomicBool::new(false);

    pub fn name() -> String {
        let user: String = std::env::var("USERNAME")
            .unwrap_or_default()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();

        format!(r"\\.\pipe\keyhold-agent-{user}")
    }

    /// The pid at the other end of a pipe, from the kernel.
    pub fn client_pid(handle: std::os::windows::io::RawHandle) -> Option<u32> {
        let mut pid = 0u32;
        // SAFETY: one out-parameter on a handle owned by the caller.
        let ok = unsafe { windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId(handle as _, &mut pid) } != 0;

        ok.then_some(pid)
    }

    pub fn listen(state: Arc<AppState>) -> Result<()> {
        ON.store(true, Ordering::SeqCst);

        if RUNNING.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let name = name();
        // The first instance, so a pipe somebody else created under this name is a refusal here
        // rather than a door that leads to them.
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)
            .with_context(|| format!("could not open the agent pipe {name}"))?;

        tokio::spawn(async move {
            loop {
                if server.connect().await.is_err() {
                    break;
                }

                let connected = server;
                server = match ServerOptions::new().reject_remote_clients(true).create(&name) {
                    Ok(next) => next,
                    Err(_) => break,
                };

                use std::os::windows::io::AsRawHandle;
                let pid = client_pid(connected.as_raw_handle()).unwrap_or(0);

                if !ON.load(Ordering::SeqCst) || !crate::procinfo::is_mine(pid as i32) {
                    drop(connected);
                    continue;
                }

                let state = state.clone();
                tokio::spawn(async move {
                    let _ = super::serve(connected, super::peer_of(pid as i32), state).await;
                });
            }

            RUNNING.store(false, Ordering::SeqCst);
        });

        Ok(())
    }

    pub fn stop() {
        ON.store(false, Ordering::SeqCst);
    }

    pub fn listening() -> bool {
        ON.load(Ordering::SeqCst)
    }
}

#[cfg(windows)]
pub fn listen(state: Arc<AppState>) -> Result<()> {
    pipe::listen(state)
}

#[cfg(windows)]
pub fn stop() {
    pipe::stop()
}

#[cfg(windows)]
pub fn listening() -> bool {
    pipe::listening()
}

/// Who is on the other end, from the kernel rather than from the client.
///
/// The process on the socket is always `keyhold mcp` — it is the translator, and naming it would
/// tell somebody nothing. What is worth showing is what launched it, so the parent is read too:
/// that is `claude`, or whatever else is driving. Both come from the kernel (`procinfo`), never
/// from anything the client says about itself.
fn peer_of(pid: i32) -> Peer {
    let parent_pid = crate::procinfo::parent(pid);

    Peer {
        pid: i64::from(pid),
        process: crate::procinfo::name(pid).unwrap_or_else(|| "desconhecido".into()),
        parent: parent_pid.and_then(crate::procinfo::name),
        parent_pid: parent_pid.map(i64::from),
    }
}

async fn serve<S>(stream: S, peer: Peer, state: Arc<AppState>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
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

/// The vaults an agent is allowed to know about at all: the ones switched on for agents, local or
/// AWS. An AWS vault is read through the same calls the interface makes, with the session that is
/// open in Keyhold; without one, the agent is told to have the person renew it.
fn visible_vaults(state: &AppState) -> Vec<String> {
    let store = state.store.lock().unwrap();

    state
        .lock
        .with_key(|key| {
            vaults::list_vaults(&store, key)
                .map(|found| {
                    found
                        .into_iter()
                        .filter(|vault| {
                            vault.agent_access && !vault.archived
                        })
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
            let mut records = 0;
            for vault in &allowed {
                if crate::http::aws::is_aws_vault(state, vault) {
                    records += crate::http::aws::agent_previews(state, vault).await.map(|found| found.len()).unwrap_or(0);
                } else {
                    let store = state.store.lock().unwrap();
                    records += records::list(&store, vault, false).map(|found| found.len()).unwrap_or(0);
                }
            }

            let store = state.store.lock().unwrap();
            let may_reveal = crate::settings::agent_may_reveal(&store);

            Response::Status {
                unlocked: true,
                vaults: allowed,
                records,
                may_reveal,
                may_copy: crate::clipboard::available(),
            }
        }

        Request::Search { query } => search(state, &query).await,

        Request::Describe { record } => describe(state, &record).await,

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
                    "este cofre não entrega valores a um assistente. Para usar uma senha, chame \
                     keyhold_run com o comando sem a senha: ela vai em uma variável de ambiente. \
                     Para o usuário colar em outro lugar, chame keyhold_copy.",
                );
            }

            // A secret never leaves as a value, whatever is switched on and whatever the person
            // would answer: it would be in the conversation the moment it arrived, and from there
            // in a command line, a log, a summary. The question is not even asked.
            if let Some((kind, _)) = field_of(state, &record, &field).await {
                if records::is_secret_kind(&kind) && kind != "totp" {
                    note_agent(state, peer, audit::action::SECRET_REVEALED, &record, &field, false);

                    return Response::refused(SECRET_NEVER_RETURNED);
                }
            }

            match approved(state, peer, &record, &field, "get").await {
                Err(refusal) => refusal,
                Ok(()) => match value_of(state, &record, &field).await {
                    Some(value) => {
                        note_agent(state, peer, audit::action::SECRET_REVEALED, &record, &field, true);

                        Response::Value { value }
                    }
                    None => Response::refused("o campo não existe neste registro"),
                },
            }
        }

        Request::Copy { record, field } => {
            match approved(state, peer, &record, &field, "copy").await {
                Err(refusal) => refusal,
                Ok(()) => match value_of(state, &record, &field).await {
                    None => Response::refused("o campo não existe neste registro"),
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
                Ok(()) => match value_of(state, &record, &field).await {
                    None => Response::refused("o campo não existe neste registro"),
                    Some(value) => {
                        note_agent(state, peer, audit::action::SECRET_REVEALED, &record, &field, true);

                        run_with(&command, &variable, &value)
                    }
                },
            }
        }
    }
}

async fn search(state: &Arc<AppState>, query: &str) -> Response {
    // Same reason as `reachable`: this takes the store lock, so it runs before we take it here.
    let allowed = visible_vaults(state);
    let needle = query.trim().to_lowercase();
    let matches = |title: &str, subtitle: &str, url: &str| {
        needle.is_empty() || format!("{title} {subtitle} {url}").to_lowercase().contains(&needle)
    };
    let mut found = Vec::new();

    for vault_id in &allowed {
        if crate::http::aws::is_aws_vault(state, vault_id) {
            let name = {
                let store = state.store.lock().unwrap();
                state.lock.with_key(|key| vaults::read_vault(&store, key, vault_id).map(|v| v.name).unwrap_or_default()).unwrap_or_default()
            };
            let Ok(previews) = crate::http::aws::agent_previews(state, vault_id).await else {
                continue;
            };
            for preview in previews {
                if !matches(&preview.title, &preview.subtitle, "") {
                    continue;
                }
                found.push(RecordSummary {
                    id: preview.id,
                    title: preview.title,
                    username: preview.subtitle,
                    url: String::new(),
                    vault: name.clone(),
                    kind: "item".into(),
                });
            }
            continue;
        }

        let store = state.store.lock().unwrap();
        let _ = state.lock.with_key(|key| {
            let name = vaults::read_vault(&store, key, vault_id).map(|vault| vault.name).unwrap_or_default();
            let Ok(previews) = records::list_with_previews(&store, key, vault_id, false) else {
                return;
            };
            for preview in previews {
                if !matches(&preview.title, &preview.subtitle, &preview.url) {
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
        });
    }

    Response::Records { records: found }
}

/// Whether this record is one the agent is allowed to touch.
///
/// The list of allowed vaults is fetched **before** the store is locked, and deliberately so: it
/// takes the same lock, and `std::sync::Mutex` is not reentrant. Taking it twice on one thread does
/// not fail — it waits, forever, for itself.
fn reachable(state: &Arc<AppState>, record_id: &str) -> bool {
    let allowed = visible_vaults(state);

    // An AWS record's id names its vault; whether the parameter is there is AWS's to say.
    if crate::aws::mapping::is_aws_id(record_id) {
        return matches!(
            crate::aws::mapping::decode_id(record_id),
            Some((crate::aws::mapping::IdKind::Record, vault, _)) if allowed.contains(&vault)
        );
    }

    let store = state.store.lock().unwrap();

    match records::header(&store, record_id) {
        Ok(header) => allowed.contains(&header.vault_id) && !header.archived,
        Err(_) => false,
    }
}

/// A record's payload, from the file or from AWS. `Err` carries why an AWS record did not open
/// (an expired session, most often), for the agent to pass on.
async fn payload_of(state: &Arc<AppState>, record_id: &str) -> std::result::Result<records::Payload, String> {
    if crate::aws::mapping::is_aws_id(record_id) {
        return crate::http::aws::agent_payload(state, record_id).await;
    }

    let store = state.store.lock().unwrap();
    state
        .lock
        .with_key(|key| records::open(&store, key, record_id).map(|record| record.payload).map_err(|e| e.to_string()))
        .unwrap_or_else(|| Err("o cofre está trancado".into()))
}

async fn describe(state: &Arc<AppState>, record_id: &str) -> Response {
    if !reachable(state, record_id) {
        return Response::refused("o registro não está disponível para um agente");
    }

    match payload_of(state, record_id).await {
        Err(why) => Response::refused(format!("não foi possível abrir o registro: {why}")),
        Ok(payload) => Response::Fields {
            title: payload.title,
            fields: payload
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

/// What a secret field's `get` answers: no, and the way to do what was wanted.
const SECRET_NEVER_RETURNED: &str = "campos secretos (senha, chave, PIN, cartão) nunca são \
    entregues a um assistente, nem com aprovação. Chame keyhold_run: execute o comando SEM a senha e \
    informe em `variable` o nome da variável que o programa lê (por exemplo MYSQL_PWD para mysql, \
    PGPASSWORD para psql). O Keyhold coloca a senha nessa variável do processo e devolve só a saída. \
    Se o usuário é quem precisa da senha, chame keyhold_copy.";

/// A field's kind and value, from the file or from AWS. `username` is the record's own line.
async fn field_of(state: &Arc<AppState>, record_id: &str, field: &str) -> Option<(String, String)> {
    let payload = payload_of(state, record_id).await.ok()?;

    if field == "username" {
        return Some(("text".to_string(), payload.username())).filter(|(_, value)| !value.is_empty());
    }

    let one = payload.fields.iter().find(|one| one.label.eq_ignore_ascii_case(field))?;

    // A 2FA field hands out the code of the moment, never the secret that makes codes.
    if one.kind == "totp" {
        return crate::totp::code_for_field(&one.value, now_seconds()).map(|(code, _)| ("totp".to_string(), code));
    }

    Some((one.kind.clone(), one.value.clone()))
}

async fn value_of(state: &Arc<AppState>, record_id: &str, field: &str) -> Option<String> {
    field_of(state, record_id, field).await.map(|(_, value)| value)
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

        return Err(Response::refused("o registro não está disponível para um agente"));
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

    let title = payload_of(state, record_id).await.map(|payload| payload.title).unwrap_or_default();

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

            return Err(Response::refused("o pedido foi negado"));
        }

        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    state.agent.pending.lock().unwrap().retain(|one| one.id != id);
    note_agent(state, peer, audit::action::SECRET_REVEALED, record_id, field, false);

    Err(Response::refused("o pedido expirou sem resposta"))
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
        format!("usar {} de “{}” em um comando", request.field, request.record_title)
    } else {
        format!("ler {} de “{}”", request.field, request.record_title)
    };

    format!(
        "{who} (PID {}) pede para {what}.\n\nO pedido expira em {APPROVAL_TIMEOUT_SECONDS} segundos. \
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
    let variable = if variable.trim().is_empty() { "KEYHOLD_SECRET" } else { variable.trim() };

    let valid = variable
        .chars()
        .enumerate()
        .all(|(at, c)| c == '_' || c.is_ascii_alphabetic() || (at > 0 && c.is_ascii_digit()));
    if !valid {
        return Response::refused("o nome da variável de ambiente só pode ter letras, números e _");
    }

    // The value is already in the command's environment; the arguments are the assistant's own text
    // and must not carry it. A command line that contains the value is the leak this tool exists to
    // prevent, so it is refused before anything runs. Where a program takes the secret as an
    // argument, the assistant writes `{{secret}}` and the value is put there only now, here, so the
    // conversation and the command it shows never hold it.
    if !value.is_empty() && command.iter().any(|argument| argument.contains(value)) {
        return Response::refused(
            "o comando contém a senha escrita nele. Execute o comando sem a senha: ela já está na \
             variável de ambiente informada em `variable`. Se o programa exige a senha como \
             argumento, escreva {{secret}} no lugar dela.",
        );
    }

    let arguments: Vec<String> = command.iter().map(|argument| argument.replace("{{secret}}", value)).collect();

    let output = std::process::Command::new(&arguments[0])
        .args(&arguments[1..])
        .env(variable, value)
        .output();

    match output {
        Err(error) => Response::refused(format!("não foi possível executar: {}", redact(&error.to_string(), value))),
        Ok(output) => Response::Ran {
            code: output.status.code().unwrap_or(-1),
            stdout: redact(&String::from_utf8_lossy(&output.stdout), value),
            stderr: redact(&String::from_utf8_lossy(&output.stderr), value),
        },
    }
}

/// What a command printed, without the value it was given. A program that echoes its environment
/// (`printenv`, `env`, a failed login that quotes the password) would otherwise hand the secret to
/// the assistant by the back door. Plain, base64, URL-encoded, JSON-escaped and hex forms go.
pub fn redact(output: &str, value: &str) -> String {
    use crate::encoding::{base64_standard, base64url};

    if value.is_empty() {
        return output.to_string();
    }

    let url_encoded: String = value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect();
    let hex: String = value.bytes().map(|b| format!("{b:02x}")).collect();
    let json_escaped = serde_json::to_string(value).map(|s| s.trim_matches('"').to_string()).unwrap_or_default();

    let mut forms = vec![
        value.to_string(),
        base64_standard(value.as_bytes()),
        base64url(value.as_bytes()),
        url_encoded.to_lowercase(),
        url_encoded,
        hex.to_uppercase(),
        hex,
        json_escaped,
    ];
    forms.retain(|form| !form.is_empty());
    // Longest first, so a form that contains another is not left half replaced.
    forms.sort_by_key(|form| std::cmp::Reverse(form.len()));
    forms.dedup();

    let mut out = output.to_string();
    for form in forms {
        out = out.replace(&form, "[valor do Keyhold]");
    }

    out
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
