//! The HTTP surface: the interface, and nothing else.
//!
//! Loopback is not a security boundary. Every process running as this user can connect to the
//! port, and any page in any tab can try to as well. So this module is written as if the network
//! were hostile, because locally it is:
//!
//!  - **Everything under `/api` requires the session token**, compared in constant time. The only
//!    routes that answer without one are `/health` and the static bundle.
//!  - **A strict Content-Security-Policy on every response**, because the page that renders secrets
//!    is the page an injected script would want. `default-src 'none'` and no CDN anywhere: every
//!    byte the browser runs is compiled into this binary.
//!  - **The bundle is embedded**, not read from disk at runtime. There is no directory an attacker
//!    can drop a file into and have keyhold serve it, and no path traversal to get wrong, because
//!    there is no filesystem lookup at all.
//!  - **No secret is ever in a response** except the value the person just asked for, and the two
//!    values that exist for exactly one moment: the recovery key at setup, and the second-factor
//!    secret while it is being scanned.

pub(crate) mod aws;
mod contents;
mod copy;

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::extract::Path;
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::audit::{self, Detail};
use crate::db::Store;
use crate::generator::{self, Recipe};
use crate::identity::{self, SetupOptions, UnlockAttempt};
use crate::lock::{Lock, LockPolicy};
use crate::session::SessionToken;
use crate::settings;
use crate::totp;

/// The header the interface sends its token in.
///
/// A custom header rather than a cookie, deliberately: a cookie is attached by the browser to
/// every request to this origin, including ones another page caused. A header has to be set by
/// code that is already running on our page.
pub const TOKEN_HEADER: &str = "x-keyhold-token";

#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

pub struct AppState {
    pub token: SessionToken,
    /// Everything the agent surface remembers, which is nothing that survives a restart (F7).
    pub agent: crate::agent::AgentState,
    pub store: std::sync::Arc<std::sync::Mutex<Store>>,
    pub lock: Lock,
    /// Failed unlocks since the last success, for the one attack a local app can slow down: a
    /// person at an open laptop guessing. It does nothing about an attack on the file itself —
    /// see `docs/THREAT_MODEL.md`, which says so rather than implying this is more than it is.
    pub failures: std::sync::Mutex<u32>,
    /// AWS sessions, credentials and caches for Parameter Store vaults (F9). Memory only, and
    /// dropped when the vault locks.
    pub aws: crate::aws::state::AwsState,
    /// What "Copiar para…" wrote in this unlock, so deleting originals only ever touches records
    /// that really arrived somewhere else (§9.1, C7). Memory only.
    pub copies: copy::Ledger,
}

/// How often the AWS state is swept for a lock that happened on its own (see `start_aws_sweeper`).
const AWS_SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// Start the background task that drops AWS tokens, credentials and caches within seconds of the
/// vault locking, auto-lock included, even when nothing is asking the server anything. Call once,
/// from inside the runtime, after the state is built.
pub fn start_aws_sweeper(state: &Arc<AppState>) {
    aws::start_sweeper(state, AWS_SWEEP_EVERY);
}

/// After this many wrong answers, keyhold stops accepting them for a while.
const MAX_FAILURES: u32 = 5;

pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/setup", post(setup))
        .route("/setup/recovery-confirmation", post(confirm_recovery))
        .route("/unlock", post(unlock))
        .route("/lock", post(lock_now))
        .route("/heartbeat", post(heartbeat))
        .route("/visibility", post(visibility))
        .route("/settings/lock", put(set_lock_policy))
        .route("/settings/appearance", put(set_appearance))
        .route("/settings/retention", put(set_retention))
        .route("/settings/log-reads", put(set_log_reads))
        .route("/settings/generator", put(set_generator))
        .route("/settings/layout", put(set_layout))
        .route("/alerts", get(alert_presets))
        .route("/alerts", put(set_alert_presets))
        .route("/audit", get(read_audit))
        .route("/audit/integrity", get(audit_integrity))
        .route("/agent", get(agent_status))
        .route("/agent", post(set_agent_surface))
        .route("/agent/requests", get(agent_requests))
        .route("/agent/requests/:id", post(answer_agent_request))
        .route("/agent/grants", get(agent_grants))
        .route("/agent/grants", delete(revoke_agent_grants))
        .route("/agent/grants/:id", delete(revoke_agent_grant))
        .route("/agent/reveal", put(set_agent_may_reveal))
        .route("/instances", get(instances))
        .route("/instances/:pid/close", post(close_instance))
        .route("/protection", put(change_protection))
        .route("/second-factor/begin", post(begin_second_factor))
        .route("/second-factor", post(enable_second_factor))
        .route("/second-factor", delete(disable_second_factor))
        .route("/password", post(make_password))
        .merge(contents::routes())
        .merge(aws::routes())
        .merge(copy::routes())
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));

    Router::new()
        .route("/health", get(health))
        .route("/open-window", post(open_window))
        .nest("/api", api)
        .fallback(serve_asset)
        // An attachment travels as base64 inside JSON: 25 MB of file is about 34 MB of body.
        .layer(axum::extract::DefaultBodyLimit::max(crate::attachments::MAX_BYTES / 3 * 4 + 1024 * 1024))
        .layer(middleware::from_fn(security_headers))
        // Outermost, so it also covers the routes that answer without a token.
        .layer(middleware::from_fn(only_this_machine))
        .with_state(state)
}

/// Refuse a request that reached this port under somebody else's name.
///
/// The attack this closes is DNS rebinding: a page on `evil.example` whose name is made to resolve
/// to `127.0.0.1`, so the browser treats this server as that page's own origin and lets it read
/// what comes back. Binding to loopback does not help — the connection really is from loopback —
/// and neither does the absence of CORS, because after rebinding there is nothing cross about it.
///
/// What gives it away is the `Host` header: the browser sends the name that was typed, and that
/// name is not one of ours. A vault reached by any name but its own is not being reached by its
/// own interface.
///
/// It was never the whole defence — every route that matters wants a session token, and a rebound
/// page has no way to learn one — but "they would have needed the token anyway" is the argument
/// that stops being true the first time something is added that does not.
async fn only_this_machine(request: Request<Body>, next: Next) -> Response {
    let host = host_of(request.headers(), request.uri());
    let host = host.as_str();

    // Port and all, or nothing: the name is what is checked, and an address is allowed to carry the
    // port it was reached on. Anything with credentials, a path, or another name is not.
    let name = host.rsplit_once(':').map(|(name, _)| name).unwrap_or(host);

    if matches!(name, "127.0.0.1" | "localhost" | "[::1]" | "::1") || is_own_name(name) {
        return next.run(request).await;
    }

    (
        StatusCode::MISDIRECTED_REQUEST,
        Json(json!({ "error": "this vault answers to 127.0.0.1 and nothing else" })),
    )
        .into_response()
}

/// The name a request was addressed to.
///
/// HTTP/1.1 carries it in `Host`. HTTP/2 — what a browser negotiates over HTTPS — carries it in the
/// `:authority` pseudo-header, which arrives as the URI's authority and not as a header at all.
/// Reading only `Host` turned every HTTPS request into "no name", refused.
fn host_of(headers: &HeaderMap, uri: &Uri) -> String {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or_else(|| uri.authority().map(|authority| authority.as_str().to_string()))
        .unwrap_or_default()
}

static OWN_NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// The names `sudo keyhold set-host` gave this machine, read from `/etc/hosts` once at start.
///
/// Only those, and only names that cannot exist on the internet (see `hosts`), so this widens the
/// check above by exactly the name somebody chose on purpose.
pub fn allow_names(names: Vec<String>) {
    let _ = OWN_NAMES.set(names);
}

fn is_own_name(name: &str) -> bool {
    OWN_NAMES.get().map(|names| names.iter().any(|own| own == name)).unwrap_or(false)
}

/// "Abrir o Keyhold", from a page that has no token: a bookmark, or a tab opened by hand.
///
/// The token never comes back in the answer. Like a second start choosing "Abrir e continuar", this
/// makes the running keyhold hand its own URL to the desktop, and the browser receives it from
/// there. A request from another site is refused by its `Origin`; the worst a local process that
/// forges one can do is open a tab, and only once every few seconds.
async fn open_window(State(state): State<Arc<AppState>>, uri: Uri, headers: HeaderMap) -> Response {
    static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let host = host_of(&headers, &uri);
    let origin = headers.get(header::ORIGIN).and_then(|value| value.to_str().ok()).unwrap_or_default();

    if origin != format!("http://{host}") && origin != format!("https://{host}") {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "apenas a própria página do Keyhold pode fazer este pedido" })))
            .into_response();
    }

    let now = identity::now_seconds();

    if now.saturating_sub(LAST.swap(now, std::sync::atomic::Ordering::SeqCst)) < 3 {
        return (StatusCode::TOO_MANY_REQUESTS, Json(json!({ "error": "o Keyhold acabou de ser aberto. Confira as abas do navegador" })))
            .into_response();
    }

    match state.agent.door() {
        Some(door) => {
            crate::instance::open_browser(door);
            Json(json!({ "opened": true })).into_response()
        }
        None => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "não foi possível abrir o Keyhold a partir desta página" })))
            .into_response(),
    }
}

/// A failure the interface can show without the server having explained too much.
///
/// The third part is set only for failures that came from (or are about) AWS: one of
/// `session_expired`, `access_denied`, `not_found`, `conflict`, `throttled`, `network`, `invalid`
/// or `other`, sent as `"aws"` next to the message so the interface can offer the right way out
/// (renewing the login, above all).
pub(crate) struct Failure(pub StatusCode, pub String, pub Option<&'static str>);

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        match self.2 {
            Some(kind) => (self.0, Json(json!({ "error": self.1, "aws": kind }))).into_response(),
            None => (self.0, Json(json!({ "error": self.1 }))).into_response(),
        }
    }
}

pub(crate) type Answer = Result<Json<Value>, Failure>;

pub(crate) fn bad_request(error: impl std::fmt::Display) -> Failure {
    Failure(StatusCode::BAD_REQUEST, error.to_string(), None)
}

pub(crate) fn conflict(error: impl std::fmt::Display) -> Failure {
    Failure(StatusCode::CONFLICT, error.to_string(), None)
}

pub(crate) fn locked() -> Failure {
    Failure(StatusCode::LOCKED, "the vault is locked".into(), None)
}

/// Liveness only. Says that something is listening and what version it is — never whether a vault
/// exists, and never anything about its contents.
async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

/// Everything the interface needs to choose a screen. No secrets, in any state.
async fn status(State(state): State<Arc<AppState>>) -> Answer {
    state.aws.sweep(&state.lock);
    let store = state.store.lock().unwrap();
    let identity = identity::status(&store).map_err(bad_request)?;
    let lock = state.lock.status();

    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "identity": identity,
        "lock": lock,
        "retention_days": settings::retention_days(&store),
        "log_reads": settings::log_reads(&store),
        "appearance": settings::appearance(&store),
        "generator": settings::generator(&store),
        "layout": settings::layout(&store),
        "attemptsLeft": MAX_FAILURES.saturating_sub(*state.failures.lock().unwrap()),
    })))
}

#[derive(Deserialize)]
struct SetupBody {
    /// `None` or absent means: no master password, protect the vault with a device key instead.
    password: Option<String>,
}

/// Create the vault's identity and leave it open.
///
/// The recovery key is in this response and in no other, ever. It is not stored, not derivable and
/// not recoverable — which is the property that makes the vault's promise true, and the reason the
/// interface makes somebody type it back before moving on.
async fn setup(State(state): State<Arc<AppState>>, Json(body): Json<SetupBody>) -> Answer {
    let mut store = state.store.lock().unwrap();

    let outcome = identity::create(&mut store, SetupOptions { password: body.password })
        .map_err(conflict)?;

    // A vault with no vault in it is a dead end: every screen past this one needs somewhere to put
    // the first record. So setup finishes with one, named plainly and renameable like any other.
    let first = crate::vaults::create_vault(
        &store,
        &outcome.vault_key,
        crate::vaults::VaultMeta {
            name: "Pessoal".into(),
            description: String::new(),
            colour: "default".into(),
            ..Default::default()
        },
    )
    .map_err(bad_request)?;

    // The vault setup makes goes in the trail like any other: the first line of the log should be
    // the moment this file started existing, not the second thing that happened to it.
    audit::note(&store, &state.lock, audit::action::VAULT_CREATED, Some(&first.id), Detail::new());

    let recovery_key = outcome.recovery_key.clone();
    state.lock.unlock_with(outcome.vault_key);

    Ok(Json(json!({
        "recoveryKey": recovery_key,
        "lock": state.lock.status(),
    })))
}

#[derive(Deserialize)]
struct RecoveryConfirmation {
    recovery_key: String,
}

/// Check that what was written down actually opens the vault.
///
/// Compared by deriving from it, not by string equality with something stored — there is nothing
/// stored to compare against, which is the whole point.
async fn confirm_recovery(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RecoveryConfirmation>,
) -> Answer {
    let store = state.store.lock().unwrap();

    identity::unlock(
        &store,
        UnlockAttempt {
            password: None,
            code: None,
            recovery_key: Some(body.recovery_key),
        },
    )
    .map_err(|_| bad_request("that is not the recovery key this store showed you"))?;

    identity::confirm_recovery(&store).map_err(bad_request)?;

    Ok(Json(json!({ "confirmed": true })))
}

#[derive(Deserialize)]
struct UnlockBody {
    password: Option<String>,
    code: Option<String>,
    recovery_key: Option<String>,
}

async fn unlock(State(state): State<Arc<AppState>>, Json(body): Json<UnlockBody>) -> Answer {
    {
        let failures = *state.failures.lock().unwrap();

        if failures >= MAX_FAILURES {
            return Err(Failure(
                StatusCode::TOO_MANY_REQUESTS,
                "muitas tentativas sem sucesso. Feche o Keyhold e abra novamente".into(),
                None,
            ));
        }
    }

    let store = state.store.lock().unwrap();

    let key = identity::unlock(
        &store,
        UnlockAttempt {
            password: body.password,
            code: body.code,
            recovery_key: body.recovery_key,
        },
    );

    match key {
        Ok(key) => {
            *state.failures.lock().unwrap() = 0;
            state.lock.unlock_with(key);

            // Emptying the trash of what has waited out its window happens here rather than on a
            // timer: keyhold only runs while somebody is using it, so a schedule would fire
            // whenever the machine happened to be awake. Unlocking is a moment the person is
            // present for, and the count comes back so the interface can say what happened.
            let purged =
                crate::records::purge_expired(&store, crate::settings::retention_days(&store))
                    .unwrap_or(0);

            // The first moment there is a key is the first moment the trail written before this
            // build existed can stop naming things in the clear. It runs once and then never
            // again, and records that it ran.
            let _ = state.lock.with_key(|key| audit::reseal(&store, key));

            audit::note(
                &store,
                &state.lock,
                audit::action::UNLOCK,
                None,
                Detail::new().with("purged", purged as i64),
            );

            Ok(Json(json!({ "lock": state.lock.status(), "purged": purged })))
        }
        Err(error) => {
            let mut failures = state.failures.lock().unwrap();
            *failures += 1;

            // A failed unlock is the more interesting half of the trail: it is the shape of
            // somebody trying. The reason is recorded, never what was typed.
            let _ = audit::record(
                &store,
                &state.lock,
                audit::Actor::You,
                audit::action::UNLOCK_FAILED,
                None,
                audit::Outcome::Denied,
                Detail::new().with("attempt", *failures as i64),
            );

            Err(Failure(
                StatusCode::UNAUTHORIZED,
                format!("{error}"),
                None,
            ))
        }
    }
}

async fn lock_now(State(state): State<Arc<AppState>>) -> Answer {
    state.lock.lock();
    state.aws.forget();

    {
        let store = state.store.lock().unwrap();
        audit::note(&store, &state.lock, audit::action::LOCK, None, Detail::new());
    }

    Ok(Json(json!({ "lock": state.lock.status() })))
}

#[derive(Deserialize)]
struct LogReads {
    on: bool,
}

async fn set_log_reads(State(state): State<Arc<AppState>>, Json(body): Json<LogReads>) -> Answer {
    // A decision about how the vault is protected belongs to somebody who can open it.
    if state.lock.with_key(|_| ()).is_none() {
        return Err(locked());
    }

    let store = state.store.lock().unwrap();

    let saved = settings::set_log_reads(&store, body.on).map_err(bad_request)?;

    Ok(Json(json!({ "log_reads": saved })))
}

#[derive(Deserialize)]
struct AuditQuery {
    #[serde(default)]
    limit: Option<i64>,
}

/// The trail, newest first.
///
/// Behind the lock like everything else — not because the rows are secret (they hold no secret),
/// but because the shape of somebody's day is not public either.
async fn read_audit(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(query): axum::extract::Query<AuditQuery>,
) -> Answer {
    let store = state.store.lock().unwrap();

    let events = state
        .lock
        .with_key(|key| audit::recent(&store, key, query.limit.unwrap_or(200)))
        .ok_or_else(locked)?
        .map_err(bad_request)?;

    Ok(Json(json!({ "events": events, "log_reads": settings::log_reads(&store) })))
}

async fn audit_integrity(State(state): State<Arc<AppState>>) -> Answer {
    if state.lock.with_key(|_| ()).is_none() {
        return Err(locked());
    }

    let store = state.store.lock().unwrap();
    let integrity = audit::verify(&store).map_err(bad_request)?;

    Ok(Json(json!({ "integrity": integrity })))
}

/// The interface saying "somebody is still here".
async fn heartbeat(State(state): State<Arc<AppState>>) -> Answer {
    state.lock.touch();
    state.aws.sweep(&state.lock);

    Ok(Json(json!({ "lock": state.lock.status() })))
}

#[derive(Deserialize)]
struct Visibility {
    visible: bool,
}

/// The window went away, or came back.
///
/// The browser reports it; the daemon decides what it means. A tab that stops reporting simply
/// keeps its idle deadline, which is the safe direction to fail in.
async fn visibility(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Visibility>,
) -> Answer {
    state.lock.set_visible(body.visible);

    Ok(Json(json!({ "lock": state.lock.status() })))
}

/// Every keyhold running as this person, and which one is answering right now.
///
/// Nothing here is about a vault's contents — it is a list of processes and their ages — so it
/// answers whether or not the vault is open.
async fn instances() -> Answer {
    Ok(Json(json!({
        "processes": crate::instance::processes(),
        "version": env!("CARGO_PKG_VERSION"),
    })))
}

/// Close one of the others.
///
/// Behind the lock on purpose. Listing is harmless, but ending a process is the sort of thing a tab
/// left open from a previous session should not still be able to do, and the panel that offers it
/// only exists inside an open vault anyway.
async fn close_instance(
    State(state): State<Arc<AppState>>,
    Path(pid): Path<i32>,
) -> Answer {
    if state.lock.with_key(|_| ()).is_none() {
        return Err(locked());
    }

    let store = state.store.lock().unwrap();
    let outcome = crate::instance::close(pid);

    let _ = audit::record(
        &store,
        &state.lock,
        audit::Actor::You,
        audit::action::INSTANCE_CLOSED,
        None,
        if outcome.is_ok() { audit::Outcome::Allowed } else { audit::Outcome::Denied },
        Detail::new().with("pid", pid as i64),
    );

    outcome.map_err(bad_request)?;

    Ok(Json(json!({ "processes": crate::instance::processes() })))
}

/// Whether the agent socket is open, and what is on the other side of the decision.
async fn agent_status(State(state): State<Arc<AppState>>) -> Answer {
    let store = state.store.lock().unwrap();

    let exposed = state
        .lock
        .with_key(|key| {
            crate::vaults::list_vaults(&store, key)
                .map(|found| {
                    found
                        .into_iter()
                        .filter(|vault| vault.agent_access && !vault.archived)
                        .count()
                })
                .unwrap_or(0)
        })
        .unwrap_or(0);

    Ok(Json(json!({
        "listening": crate::agent::listening(),
        "socket": crate::agent::socket_path().to_string_lossy(),
        "exposed_vaults": exposed,
        "may_reveal": settings::agent_may_reveal(&store),
        "can_copy": crate::clipboard::available(),
        "active_grants": state
            .lock
            .with_key(|key| crate::grants::count(&store, key))
            .unwrap_or(0),
        "command": format!(
            "claude mcp add keyhold --scope user -- {} mcp",
            std::env::current_exe()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "keyhold".into())
        ),
    })))
}

#[derive(Deserialize)]
struct AgentSurface {
    on: bool,
}

/// Open or close the socket.
///
/// Closing removes the file, so an agent finds nothing to connect to rather than something that
/// refuses — the difference between an integration that is off and one that is broken.
async fn set_agent_surface(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AgentSurface>,
) -> Answer {
    if state.lock.with_key(|_| ()).is_none() {
        return Err(locked());
    }

    if body.on {
        crate::agent::listen(state.clone()).map_err(bad_request)?;
    } else {
        crate::agent::stop();

        // Questions in flight have nobody left to answer them. Standing permissions are not
        // touched: closing the door is not the same as withdrawing what was already agreed, and
        // silently forgetting them would be a surprise the next time it opens.
        state.agent.forget_pending();
    }

    // Remembered, so an agent configured once keeps working after keyhold is closed and opened
    // again. The socket's path never changes; only whether it is there.
    {
        let store = state.store.lock().unwrap();
        settings::set_agent_surface(&store, body.on).map_err(bad_request)?;
    }

    Ok(Json(json!({ "listening": crate::agent::listening() })))
}

/// What an agent is waiting to be told.
///
/// Polled by the interface. A request that nobody answers times out on the daemon's side, so a
/// closed window cannot leave an agent hanging — or, worse, leave a question that gets answered by
/// whoever opens the window next.
#[derive(Deserialize)]
struct Polled {
    /// The interface says whether somebody has it in front of them. When nobody does, a question
    /// is also asked on the desktop — see `agent::DesktopKnock`.
    #[serde(default)]
    looking: bool,
}

async fn agent_requests(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(polled): axum::extract::Query<Polled>,
) -> Answer {
    if polled.looking {
        state.agent.looked();
    }

    if state.lock.with_key(|_| ()).is_none() {
        return Ok(Json(json!({ "requests": [] })));
    }

    let standing = {
        let store = state.store.lock().unwrap();

        state.lock.with_key(|key| crate::grants::count(&store, key)).unwrap_or(0)
    };

    Ok(Json(json!({
        "requests": state.agent.waiting(),
        "active_grants": standing,
    })))
}

#[derive(Deserialize)]
struct Answered {
    allowed: bool,
    /// Make the permission cover every field of the record, not only the one asked about.
    #[serde(default)]
    all_fields: bool,
    /// How long the yes lasts. Absent means "just this once".
    #[serde(default)]
    window: Option<Window>,
}

/// The windows the dialog offers, as they arrive on the wire.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum Window {
    Once,
    /// While the process that asked is still running.
    Process,
    Minutes { minutes: u64 },
    Always,
}

/// The interface answering one question, and saying how long the answer is good for.
///
/// A window other than `once` becomes a row in the vault. That is a decision about the vault rather
/// than about this request, which is why it is written here — with the key, under the lock, in the
/// trail — instead of being kept in the agent's memory the way a five-minute grant used to be.
async fn answer_agent_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Answered>,
) -> Answer {
    let asked = state.agent.answer(&id, body.allowed);

    let (Some(asked), true) = (asked, body.allowed) else {
        return Ok(Json(json!({ "answered": true })));
    };

    let scope = match body.window {
        None | Some(Window::Once) => return Ok(Json(json!({ "answered": true }))),
        Some(Window::Always) => crate::grants::Scope::Always,
        Some(Window::Minutes { minutes }) => crate::grants::Scope::Until {
            at: crate::identity::now_seconds() + minutes.clamp(1, 60 * 24 * 30) * 60,
        },
        Some(Window::Process) => {
            // The assistant rather than the bridge it speaks through: restarting the bridge is not
            // the assistant going away. Its start time goes in too, because pids are reused.
            let pid = asked.peer.parent_pid.unwrap_or(asked.peer.pid);

            match crate::grants::started_at(pid) {
                Some(since) => crate::grants::Scope::WhileRunning {
                    pid,
                    since,
                    process: asked.peer.parent.clone().unwrap_or_else(|| asked.peer.process.clone()),
                },
                None => return Err(bad_request("o processo não está mais em execução")),
            }
        }
    };

    let store = state.store.lock().unwrap();

    // The permission is for what the question showed: this record, this use, and for a command,
    // this program. A different use or a different program asks again.
    let cover = crate::grants::Cover {
        field: asked.field.as_str(),
        all_fields: body.all_fields,
        intent: asked.intent.as_str(),
        program: asked.command.as_ref().and_then(|argv| argv.first()).map(String::as_str),
    };
    let written = state.lock.with_key(|key| {
        crate::grants::grant_for(&store, key, &asked.record_id, &cover, scope.clone())
    });

    match written {
        None => Err(locked()),
        Some(Err(error)) => Err(bad_request(error)),
        Some(Ok(_)) => {
            let _ = audit::record(
                &store,
                &state.lock,
                audit::Actor::You,
                audit::action::AGENT_GRANTED,
                Some(asked.record_id.as_str()),
                audit::Outcome::Allowed,
                Detail::new()
                    .with("field", if body.all_fields { "todos os campos" } else { asked.field.as_str() })
                    .with("intent", asked.intent.as_str())
                    .with("program", cover.program.unwrap_or(""))
                    .with("asked_by", asked.peer.describe())
                    .with("window", describe_window(&scope).as_str()),
            );

            Ok(Json(json!({ "answered": true })))
        }
    }
}

/// What a window says in the trail. Never a value, never a record title — the trail names the
/// record by id, and the field label is the same one the dialog showed.
fn describe_window(scope: &crate::grants::Scope) -> String {
    match scope {
        crate::grants::Scope::Always => "sempre".into(),
        crate::grants::Scope::Until { at } => format!("até {at}"),
        crate::grants::Scope::WhileRunning { pid, process, .. } => {
            format!("enquanto {process} ({pid}) estiver aberto")
        }
    }
}

#[derive(Deserialize)]
struct MayReveal {
    on: bool,
}

/// Whether an agent may receive a value at all.
///
/// The one switch in this app that decides something no amount of instruction can: a value handed
/// to a model is in that model's session the moment it arrives.
async fn set_agent_may_reveal(
    State(state): State<Arc<AppState>>,
    Json(body): Json<MayReveal>,
) -> Answer {
    // A decision about how the vault is protected belongs to somebody who can open it.
    if state.lock.with_key(|_| ()).is_none() {
        return Err(locked());
    }

    let store = state.store.lock().unwrap();

    let saved = settings::set_agent_may_reveal(&store, body.on).map_err(bad_request)?;

    audit::note(
        &store,
        &state.lock,
        audit::action::PROTECTION_CHANGED,
        None,
        Detail::new().with("agent_may_reveal", saved),
    );

    Ok(Json(json!({ "may_reveal": saved })))
}

/// Everything standing right now, with what each covers and when it ends.
async fn agent_grants(State(state): State<Arc<AppState>>) -> Answer {
    let store = state.store.lock().unwrap();

    let standing = state
        .lock
        .with_key(|key| crate::grants::list(&store, key))
        .ok_or_else(locked)?
        .map_err(bad_request)?;

    Ok(Json(json!({ "grants": standing })))
}

/// Withdraw one.
async fn revoke_agent_grant(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Answer {
    let store = state.store.lock().unwrap();

    if state.lock.with_key(|_| ()).is_none() {
        return Err(locked());
    }

    crate::grants::revoke(&store, &id).map_err(bad_request)?;

    let _ = audit::record(
        &store,
        &state.lock,
        audit::Actor::You,
        audit::action::AGENT_GRANT_REVOKED,
        None,
        audit::Outcome::Allowed,
        Detail::new(),
    );

    Ok(Json(json!({ "revoked": true })))
}

/// Withdraw all of them, and drop any question still waiting.
async fn revoke_agent_grants(State(state): State<Arc<AppState>>) -> Answer {
    state.agent.forget_pending();

    let store = state.store.lock().unwrap();

    if state.lock.with_key(|_| ()).is_none() {
        return Err(locked());
    }

    let gone = crate::grants::revoke_all(&store).map_err(bad_request)?;

    if gone > 0 {
        let _ = audit::record(
            &store,
            &state.lock,
            audit::Actor::You,
            audit::action::AGENT_GRANT_REVOKED,
            None,
            audit::Outcome::Allowed,
            Detail::new().with("revoked", gone as i64),
        );
    }

    Ok(Json(json!({ "active_grants": 0 })))
}

#[derive(Deserialize)]
struct Retention {
    days: i64,
}

/// How long the trash keeps what was deleted. Zero means it keeps it until somebody says otherwise.
async fn set_retention(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Retention>,
) -> Answer {
    // A decision about how the vault is protected belongs to somebody who can open it.
    if state.lock.with_key(|_| ()).is_none() {
        return Err(locked());
    }

    let store = state.store.lock().unwrap();

    let saved = settings::set_retention_days(&store, body.days).map_err(bad_request)?;

    Ok(Json(json!({ "retention_days": saved })))
}

/// Theme and accent. Allowed while locked, like everything else on the unlock screen's side of the
/// line: it carries nothing about the vault.
async fn set_appearance(
    State(state): State<Arc<AppState>>,
    Json(body): Json<settings::Appearance>,
) -> Answer {
    let store = state.store.lock().unwrap();

    let saved = settings::set_appearance(&store, body).map_err(bad_request)?;

    Ok(Json(json!({ "appearance": saved })))
}

async fn set_lock_policy(
    State(state): State<Arc<AppState>>,
    Json(policy): Json<LockPolicy>,
) -> Answer {
    // A decision about how the vault is protected belongs to somebody who can open it.
    if state.lock.with_key(|_| ()).is_none() {
        return Err(locked());
    }

    let store = state.store.lock().unwrap();

    let saved = settings::set_lock_policy(&store, policy).map_err(bad_request)?;
    state.lock.set_policy(saved);

    Ok(Json(json!({ "lock": state.lock.status() })))
}

#[derive(Deserialize)]
struct ProtectionBody {
    /// `None` removes the master password and falls back to the device key.
    password: Option<String>,
}

/// Add, change or remove the master password.
///
/// Requires an unlocked vault, which means somebody already proved they can open it. Re-wraps one
/// key; every record stays exactly as it is.
async fn change_protection(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ProtectionBody>,
) -> Answer {
    let store = state.store.lock().unwrap();

    let result = state
        .lock
        .with_key(|key| identity::change_protection(&store, key, body.password.as_deref()))
        .ok_or_else(locked)?;

    result.map_err(bad_request)?;

    audit::note(
        &store,
        &state.lock,
        audit::action::PROTECTION_CHANGED,
        None,
        Detail::new().with("mode", if body.password.is_some() { "password" } else { "device" }),
    );

    Ok(Json(json!({ "changed": true })))
}

/// Start turning on the second factor: a fresh secret, and a QR code to scan.
///
/// Nothing is saved yet. The secret only becomes this vault's second factor once a code produced
/// from it comes back — otherwise a mis-scan would lock somebody out of their own vault.
async fn begin_second_factor(State(state): State<Arc<AppState>>) -> Answer {
    if state.lock.with_key(|_| ()).is_none() {
        return Err(locked());
    }

    let secret = totp::generate_secret();
    let uri = totp::provisioning_uri(&secret, "store");

    Ok(Json(json!({
        "secret": crate::encoding::base32(&secret),
        "uri": uri,
        "qr": qr_data_url(&uri).map_err(bad_request)?,
    })))
}

#[derive(Deserialize)]
struct SecondFactorBody {
    secret: String,
    code: String,
}

async fn enable_second_factor(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SecondFactorBody>,
) -> Answer {
    let secret = crate::encoding::from_base32(&body.secret)
        .ok_or_else(|| bad_request("that secret is not readable"))?;

    if !totp::verify(&secret, &body.code, identity::now_seconds()) {
        return Err(bad_request(
            "o código não confere. Confira se o aplicativo mostra o código deste cofre",
        ));
    }

    let store = state.store.lock().unwrap();

    // Replacing a factor that exists would let whoever is at an unlocked machine swap in their own
    // and lock the owner out. It is turned off first, and that asks for a code from the current one.
    let existing: Option<Vec<u8>> = store
        .conn()
        .query_row("SELECT totp_secret FROM identity WHERE id = 1", [], |row| row.get(0))
        .map_err(bad_request)?;
    if existing.is_some() {
        return Err(bad_request(
            "este cofre já tem um segundo fator. Desative o atual, com um código dele, antes de configurar outro",
        ));
    }

    state
        .lock
        .with_key(|key| identity::enable_second_factor(&store, key, &secret))
        .ok_or_else(locked)?
        .map_err(bad_request)?;

    audit::note(
        &store,
        &state.lock,
        audit::action::SECOND_FACTOR_CHANGED,
        None,
        Detail::new().with("enabled", true),
    );

    Ok(Json(json!({ "enabled": true })))
}

/// Turn the second factor off, with a current code.
///
/// Asking for a code here is the point: somebody who found an unlocked laptop should not be able to
/// quietly remove the factor that would have stopped them next time.
async fn disable_second_factor(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Answer {
    let store = state.store.lock().unwrap();
    let code = body.get("code").and_then(|c| c.as_str()).unwrap_or_default();

    let stored: Option<Vec<u8>> = store
        .conn()
        .query_row("SELECT totp_secret FROM identity WHERE id = 1", [], |row| row.get(0))
        .map_err(bad_request)?;

    let Some(sealed) = stored.as_deref() else {
        return Ok(Json(json!({ "enabled": false })));
    };

    let secret = state
        .lock
        .with_key(|key| crate::crypto::open(key, sealed, b"keyhold:totp-secret:v1"))
        .ok_or_else(locked)?
        .map_err(bad_request)?;

    // The same count as a wrong password: a six-digit code is guessable by anything that may try
    // without limit.
    {
        let mut failures = state.failures.lock().unwrap();
        if *failures >= MAX_FAILURES {
            return Err(Failure(
                StatusCode::TOO_MANY_REQUESTS,
                "muitas tentativas sem sucesso. Feche o Keyhold e abra novamente".into(),
                None,
            ));
        }
        if !totp::verify(&secret, code, identity::now_seconds()) {
            *failures += 1;
            return Err(bad_request("o código não confere"));
        }
    }

    identity::disable_second_factor(&store).map_err(bad_request)?;

    audit::note(
        &store,
        &state.lock,
        audit::action::SECOND_FACTOR_CHANGED,
        None,
        Detail::new().with("enabled", false),
    );

    Ok(Json(json!({ "enabled": false })))
}

/// Make up a password. Available while locked: the setup screen needs one before there is a vault.
async fn set_generator(State(state): State<Arc<AppState>>, Json(recipe): Json<Recipe>) -> Answer {
    let store = state.store.lock().unwrap();
    let saved = settings::set_generator(&store, recipe).map_err(bad_request)?;

    Ok(Json(json!({ "generator": saved })))
}

async fn set_layout(State(state): State<Arc<AppState>>, Json(layout): Json<settings::Layout>) -> Answer {
    let store = state.store.lock().unwrap();
    let saved = settings::set_layout(&store, layout).map_err(bad_request)?;

    Ok(Json(json!({ "layout": saved })))
}

/// The standard alerts. Sealed, so only while unlocked.
async fn alert_presets(State(state): State<Arc<AppState>>) -> Answer {
    let store = state.store.lock().unwrap();
    let presets = state
        .lock
        .with_key(|key| crate::alerts::load(&store, key))
        .ok_or_else(locked)?
        .map_err(bad_request)?;

    Ok(Json(json!({ "presets": presets })))
}

#[derive(Deserialize)]
struct AlertPresets {
    presets: Vec<crate::alerts::Preset>,
}

async fn set_alert_presets(State(state): State<Arc<AppState>>, Json(body): Json<AlertPresets>) -> Answer {
    let store = state.store.lock().unwrap();
    let presets = state
        .lock
        .with_key(|key| crate::alerts::save(&store, key, body.presets))
        .ok_or_else(locked)?
        .map_err(bad_request)?;

    Ok(Json(json!({ "presets": presets })))
}

async fn make_password(Json(recipe): Json<Recipe>) -> Answer {
    let generated = generator::generate(recipe);

    Ok(Json(json!({
        "password": generated.password,
        "entropyBits": generated.entropy_bits,
    })))
}

/// A QR code as a `data:` URL, so the interface can render it in an `<img>`.
///
/// Deliberately not an SVG string for the page to inject: injecting markup is the one thing this
/// bundle is not allowed to do, and a `data:` URL in `src` is covered by `img-src 'self' data:`.
fn qr_data_url(uri: &str) -> anyhow::Result<String> {
    use qrcode::render::svg;
    use qrcode::QrCode;

    let svg = QrCode::new(uri.as_bytes())?
        .render::<svg::Color>()
        .min_dimensions(200, 200)
        .quiet_zone(true)
        .build();

    Ok(format!(
        "data:image/svg+xml;base64,{}",
        crate::encoding::base64_standard(svg.as_bytes())
    ))
}

async fn require_token(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    if state.token.matches(presented) {
        // Activity is a request that changes something. Reads are what screens poll with, and a
        // poll must not keep an idle vault open (`Lock::with_key` no longer counts either). The
        // interface sends a heartbeat on key presses and clicks for the rest.
        let changes = !matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS);
        let response = next.run(request).await;

        if changes && response.status().is_success() {
            state.lock.touch();
        }

        return response;
    }

    // No detail, no hint about which part was wrong, and the same answer whether the header was
    // missing or incorrect.
    //
    // `session` is the one thing worth distinguishing, and it is about this tab rather than about
    // the vault: a wrong master password is also a 401, and an interface that treated the two the
    // same would throw somebody out of the lock screen for a typo. This says "the credential you
    // are holding is not one of ours" — which, since a takeover became a normal thing to do, is
    // what an old tab is holding after another instance took over.
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "unauthorised", "session": "invalid" })),
    )
        .into_response()
}

/// The one policy that matters, applied to every response.
///
/// **`script-src` is the strict one, and it stays strict.** No `'unsafe-inline'`, no
/// `'unsafe-eval'`, no remote origin: every line of JavaScript the browser runs came out of this
/// binary. That is the directive that stands between a stored value and code execution on the page
/// holding the keys.
///
/// **`style-src` allows inline, and that is a deliberate trade.** The interface's components
/// position their popovers and dialogs by setting `style` on an element at runtime — that is how
/// every serious component library does it, because a menu's position is not knowable at build
/// time. Under `style-src 'self'` the browser blocks those writes, and the result is not a
/// styling glitch: it is a select that opens in the wrong place and a scroll area with no
/// scrollbar. This was measured rather than assumed.
///
/// What that concession actually costs, stated plainly: an attacker who could already inject
/// markup could also inject CSS. The classic use for that is exfiltration — a selector that matches
/// a secret and fetches a URL to report it. Which is why the rest of this policy matters more than
/// it looks: `default-src 'none'` with `img-src 'self' data:`, `font-src 'self'` and
/// `connect-src 'self'` means CSS has nowhere to send anything. Injected styles can deface this
/// page; they cannot phone home.
const CSP: &str = "default-src 'none'; \
                   script-src 'self'; \
                   style-src 'self' 'unsafe-inline'; \
                   img-src 'self' data:; \
                   font-src 'self'; \
                   connect-src 'self'; \
                   base-uri 'none'; \
                   form-action 'none'; \
                   frame-ancestors 'none'; \
                   object-src 'none'";

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    let set = |headers: &mut HeaderMap, name: header::HeaderName, value: &'static str| {
        headers.insert(name, HeaderValue::from_static(value));
    };

    set(headers, header::CONTENT_SECURITY_POLICY, CSP);
    set(headers, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(headers, header::REFERRER_POLICY, "no-referrer");
    set(headers, header::CACHE_CONTROL, "no-store");
    headers.insert("cross-origin-opener-policy", HeaderValue::from_static("same-origin"));
    headers.insert("cross-origin-resource-policy", HeaderValue::from_static("same-origin"));
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("geolocation=(), camera=(), microphone=(), payment=(), usb=()"),
    );

    response
}

/// Serve the embedded bundle, falling back to the shell for any path the client routes itself.
async fn serve_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let candidate = if path.is_empty() { "index.html" } else { path };

    match Assets::get(candidate).or_else(|| Assets::get("index.html")) {
        Some(file) => {
            let mime = mime_guess::from_path(candidate).first_or_octet_stream();

            (
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                file.data.into_owned(),
            )
                .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            "the interface bundle is missing from this build — run `npm run build` in web/ and \
             rebuild",
        )
            .into_response(),
    }
}

#[cfg(test)]
mod host_tests {
    use super::*;

    #[test]
    fn the_name_comes_from_host_or_from_the_http2_authority() {
        let mut headers = HeaderMap::new();
        let uri: Uri = "https://keyhold.local:7717/health".parse().unwrap();

        // HTTP/2: no Host header, the name is in the URI.
        assert_eq!(host_of(&headers, &uri), "keyhold.local:7717");

        // HTTP/1.1: Host wins.
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:7717"));
        assert_eq!(host_of(&headers, &"/health".parse().unwrap()), "127.0.0.1:7717");

        // Neither: nothing, which the check refuses.
        assert_eq!(host_of(&HeaderMap::new(), &"/health".parse().unwrap()), "");
    }
}
