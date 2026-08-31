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

mod contents;

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode, Uri};
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
    pub store: std::sync::Arc<std::sync::Mutex<Store>>,
    pub lock: Lock,
    /// Failed unlocks since the last success, for the one attack a local app can slow down: a
    /// person at an open laptop guessing. It does nothing about an attack on the file itself —
    /// see `docs/THREAT_MODEL.md`, which says so rather than implying this is more than it is.
    pub failures: std::sync::Mutex<u32>,
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
        .route("/settings/retention", put(set_retention))
        .route("/settings/log-reads", put(set_log_reads))
        .route("/audit", get(read_audit))
        .route("/audit/integrity", get(audit_integrity))
        .route("/protection", put(change_protection))
        .route("/second-factor/begin", post(begin_second_factor))
        .route("/second-factor", post(enable_second_factor))
        .route("/second-factor", delete(disable_second_factor))
        .route("/password", post(make_password))
        .merge(contents::routes())
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));

    Router::new()
        .route("/health", get(health))
        .nest("/api", api)
        .fallback(serve_asset)
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

/// A failure the interface can show without the server having explained too much.
pub(crate) struct Failure(pub StatusCode, pub String);

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

pub(crate) type Answer = Result<Json<Value>, Failure>;

pub(crate) fn bad_request(error: impl std::fmt::Display) -> Failure {
    Failure(StatusCode::BAD_REQUEST, error.to_string())
}

pub(crate) fn conflict(error: impl std::fmt::Display) -> Failure {
    Failure(StatusCode::CONFLICT, error.to_string())
}

pub(crate) fn locked() -> Failure {
    Failure(StatusCode::LOCKED, "the vault is locked".into())
}

/// Liveness only. Says that something is listening and what version it is — never whether a vault
/// exists, and never anything about its contents.
async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

/// Everything the interface needs to choose a screen. No secrets, in any state.
async fn status(State(state): State<Arc<AppState>>) -> Answer {
    let store = state.store.lock().unwrap();
    let identity = identity::status(&store).map_err(bad_request)?;
    let lock = state.lock.status();

    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "identity": identity,
        "lock": lock,
        "retention_days": settings::retention_days(&store),
        "log_reads": settings::log_reads(&store),
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
        },
    )
    .map_err(bad_request)?;

    // The vault setup makes goes in the trail like any other: the first line of the log should be
    // the moment this file started existing, not the second thing that happened to it.
    audit::note(&store, audit::action::VAULT_CREATED, Some(&first.id), Detail::new());

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
                "too many failed attempts — close keyhold and open it again".into(),
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

            audit::note(
                &store,
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
                audit::Actor::You,
                audit::action::UNLOCK_FAILED,
                None,
                audit::Outcome::Denied,
                Detail::new().with("attempt", *failures as i64),
            );

            Err(Failure(
                StatusCode::UNAUTHORIZED,
                format!("{error}"),
            ))
        }
    }
}

async fn lock_now(State(state): State<Arc<AppState>>) -> Answer {
    state.lock.lock();

    {
        let store = state.store.lock().unwrap();
        audit::note(&store, audit::action::LOCK, None, Detail::new());
    }

    Ok(Json(json!({ "lock": state.lock.status() })))
}

#[derive(Deserialize)]
struct LogReads {
    on: bool,
}

async fn set_log_reads(State(state): State<Arc<AppState>>, Json(body): Json<LogReads>) -> Answer {
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
    if state.lock.with_key(|_| ()).is_none() {
        return Err(locked());
    }

    let store = state.store.lock().unwrap();
    let events = audit::recent(&store, query.limit.unwrap_or(200)).map_err(bad_request)?;

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

#[derive(Deserialize)]
struct Retention {
    days: i64,
}

/// How long the trash keeps what was deleted. Zero means it keeps it until somebody says otherwise.
async fn set_retention(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Retention>,
) -> Answer {
    let store = state.store.lock().unwrap();

    let saved = settings::set_retention_days(&store, body.days).map_err(bad_request)?;

    Ok(Json(json!({ "retention_days": saved })))
}

async fn set_lock_policy(
    State(state): State<Arc<AppState>>,
    Json(policy): Json<LockPolicy>,
) -> Answer {
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
            "that code is not right — check the app is showing this store's entry",
        ));
    }

    let store = state.store.lock().unwrap();

    state
        .lock
        .with_key(|key| identity::enable_second_factor(&store, key, &secret))
        .ok_or_else(locked)?
        .map_err(bad_request)?;

    audit::note(
        &store,
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

    if !totp::verify(&secret, code, identity::now_seconds()) {
        return Err(bad_request("that code is not right"));
    }

    identity::disable_second_factor(&store).map_err(bad_request)?;

    audit::note(
        &store,
        audit::action::SECOND_FACTOR_CHANGED,
        None,
        Detail::new().with("enabled", false),
    );

    Ok(Json(json!({ "enabled": false })))
}

/// Make up a password. Available while locked: the setup screen needs one before there is a vault.
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
        return next.run(request).await;
    }

    // No detail, no hint about which part was wrong, and the same answer whether the header was
    // missing or incorrect.
    (StatusCode::UNAUTHORIZED, Json(json!({ "error": "unauthorised" }))).into_response()
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
