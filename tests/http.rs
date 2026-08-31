//! What the HTTP surface must refuse, what it must always say, and the shape of a first unlock.
//!
//! These drive the same router the binary serves, through a real loopback socket, because the
//! interesting failures — a header dropped by a middleware ordering change, a route that quietly
//! escapes the auth layer, a secret that turns up in a response nobody looked at — do not
//! reproduce against a hand-built request.

use std::sync::Arc;

use keyhold::db::Store;
use keyhold::http::{router, AppState, TOKEN_HEADER};
use keyhold::lock::{Lock, LockPolicy};
use keyhold::session::SessionToken;

struct Running {
    base: String,
    token: String,
    /// The very store the server is using, so a test can age a row the way a month of calendar
    /// would — without a second connection that would see its own snapshot.
    store: std::sync::Arc<std::sync::Mutex<Store>>,
    _dir: tempfile::TempDir,
}

async fn start() -> Running {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("vault.db")).unwrap();

    let token = SessionToken::generate();
    let exposed = token.expose().to_string();

    let shared = std::sync::Arc::new(std::sync::Mutex::new(store));

    let state = Arc::new(AppState {
        token,
        store: shared.clone(),
        lock: Lock::new(LockPolicy::default()),
        failures: std::sync::Mutex::new(0),
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());

    tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });

    Running { base, token: exposed, store: shared, _dir: dir }
}

impl Running {
    async fn get(&self, path: &str) -> Reply {
        http::send("GET", &format!("{}{path}", self.base), Some(&self.token), None).await
    }

    async fn get_without_token(&self, path: &str) -> Reply {
        http::send("GET", &format!("{}{path}", self.base), None, None).await
    }

    async fn post(&self, path: &str, body: serde_json::Value) -> Reply {
        http::send(
            "POST",
            &format!("{}{path}", self.base),
            Some(&self.token),
            Some(&body.to_string()),
        )
        .await
    }
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null)
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// A deliberately tiny HTTP client.
///
/// Pulling in a full client for a dozen requests would put a dependency tree into the build of a
/// program whose whole argument is that it depends on very little.
mod http {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub async fn send(
        method: &str,
        url: &str,
        token: Option<&str>,
        body: Option<&str>,
    ) -> super::Reply {
        let rest = url.strip_prefix("http://").expect("http url");
        let (authority, path) = match rest.find('/') {
            Some(at) => (&rest[..at], &rest[at..]),
            None => (rest, "/"),
        };

        let mut stream = tokio::net::TcpStream::connect(authority).await.unwrap();

        let mut request =
            format!("{method} {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n");

        if let Some(token) = token {
            request.push_str(&format!("{}: {}\r\n", super::TOKEN_HEADER, token));
        }

        if let Some(body) = body {
            request.push_str("Content-Type: application/json\r\n");
            request.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }

        request.push_str("\r\n");

        if let Some(body) = body {
            request.push_str(body);
        }

        stream.write_all(request.as_bytes()).await.unwrap();

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw).into_owned();

        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
        let mut lines = head.lines();

        let status = lines
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);

        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_string()))
            .collect();

        super::Reply { status, headers, body: body.to_string() }
    }
}

// ── the gate ────────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_api_refuses_a_request_with_no_token() {
    let app = start().await;

    assert_eq!(app.get_without_token("/api/status").await.status, 401);
}

#[tokio::test]
async fn the_api_refuses_a_wrong_token() {
    let app = start().await;
    let wrong = format!("{}x", &app.token[..app.token.len() - 1]);

    let reply =
        http::send("GET", &format!("{}/api/status", app.base), Some(&wrong), None).await;

    assert_eq!(reply.status, 401);
}

#[tokio::test]
async fn every_response_carries_the_policy() {
    let app = start().await;

    for path in ["/health", "/", "/api/status"] {
        let reply = app.get(path).await;

        let csp = reply
            .header("content-security-policy")
            .unwrap_or_else(|| panic!("{path} answered without a CSP"));

        assert!(csp.contains("default-src 'none'"), "{path}: {csp}");
        assert!(csp.contains("frame-ancestors 'none'"), "{path}: {csp}");

        // Scripts are the directive that matters, and it is checked precisely rather than by
        // searching the whole policy: `style-src` is allowed to carry `'unsafe-inline'` (see the
        // reasoning where CSP is defined) and a substring search over the whole header would
        // happily accept `script-src 'unsafe-inline'` alongside it.
        let scripts = csp
            .split(';')
            .map(str::trim)
            .find(|directive| directive.starts_with("script-src"))
            .unwrap_or_else(|| panic!("{path} has no script-src: {csp}"));

        assert_eq!(
            scripts, "script-src 'self'",
            "{path} loosened script-src, which is the one that must never loosen"
        );

        // And nothing anywhere may name an origin off this machine.
        for directive in csp.split(';') {
            assert!(
                !directive.contains("http:") && !directive.contains("//"),
                "{path} allows a remote origin: {directive}"
            );
        }

        assert_eq!(reply.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(reply.header("referrer-policy"), Some("no-referrer"));
        assert_eq!(reply.header("cross-origin-resource-policy"), Some("same-origin"));
    }
}

#[tokio::test]
async fn health_says_nothing_about_the_vault() {
    let app = start().await;
    let reply = app.get_without_token("/health").await;

    assert_eq!(reply.status, 200);
    assert!(!reply.body.contains("identity"), "health leaked vault state: {}", reply.body);
    assert!(!reply.body.contains("unlocked"), "health leaked vault state: {}", reply.body);
}

// ── the first unlock ────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_new_vault_reports_itself_as_empty_and_locked() {
    let app = start().await;
    let status = app.get("/api/status").await.json();

    assert_eq!(status["identity"]["initialised"], false);
    assert_eq!(status["lock"]["unlocked"], false);
}

#[tokio::test]
async fn setting_up_hands_back_the_recovery_key_once_and_leaves_the_vault_open() {
    let app = start().await;

    let created = app
        .post("/api/setup", serde_json::json!({ "password": "a long master password" }))
        .await;

    assert_eq!(created.status, 200);

    let recovery = created.json()["recoveryKey"].as_str().unwrap().to_string();
    assert!(recovery.len() > 40, "recovery key looks too short: {recovery}");
    assert_eq!(created.json()["lock"]["unlocked"], true);

    // It is in that response and in no other, ever — and neither is the master password itself.
    // (`"protection":"password"` names the *mode*, which the interface has to know about.)
    let status = app.get("/api/status").await;
    assert!(!status.body.contains(&recovery), "the recovery key came back a second time");
    assert!(
        !status.body.contains("a long master password"),
        "the master password came back from the server: {}",
        status.body
    );
}

#[tokio::test]
async fn locking_then_unlocking_needs_the_password() {
    let app = start().await;

    app.post("/api/setup", serde_json::json!({ "password": "the master password" })).await;
    app.post("/api/lock", serde_json::json!({})).await;

    assert_eq!(app.get("/api/status").await.json()["lock"]["unlocked"], false);

    let wrong = app
        .post("/api/unlock", serde_json::json!({ "password": "not the one" }))
        .await;
    assert_eq!(wrong.status, 401);
    assert_eq!(app.get("/api/status").await.json()["lock"]["unlocked"], false);

    let right = app
        .post("/api/unlock", serde_json::json!({ "password": "the master password" }))
        .await;
    assert_eq!(right.status, 200);
    assert_eq!(right.json()["lock"]["unlocked"], true);
}

#[tokio::test]
async fn repeated_wrong_answers_stop_being_answered() {
    let app = start().await;
    app.post("/api/setup", serde_json::json!({ "password": "the master password" })).await;
    app.post("/api/lock", serde_json::json!({})).await;

    for _ in 0..5 {
        app.post("/api/unlock", serde_json::json!({ "password": "wrong" })).await;
    }

    let blocked = app
        .post("/api/unlock", serde_json::json!({ "password": "the master password" }))
        .await;

    assert_eq!(blocked.status, 429, "guessing was never slowed down");
}

#[tokio::test]
async fn the_recovery_key_opens_a_vault_whose_password_is_gone() {
    let app = start().await;

    let recovery = app
        .post("/api/setup", serde_json::json!({ "password": "forgotten" }))
        .await
        .json()["recoveryKey"]
        .as_str()
        .unwrap()
        .to_string();

    app.post("/api/lock", serde_json::json!({})).await;

    let reply = app
        .post("/api/unlock", serde_json::json!({ "recovery_key": recovery }))
        .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.json()["lock"]["unlocked"], true);
}

#[tokio::test]
async fn a_vault_can_be_set_up_without_a_master_password() {
    let app = start().await;

    let created = app.post("/api/setup", serde_json::json!({ "password": null })).await;
    assert_eq!(created.status, 200);

    app.post("/api/lock", serde_json::json!({})).await;

    // No password to give: the device key is what opens it, and the interface says what that costs.
    let reopened = app.post("/api/unlock", serde_json::json!({})).await;

    assert_eq!(reopened.status, 200);
    assert_eq!(app.get("/api/status").await.json()["identity"]["protection"], "device");
}

#[tokio::test]
async fn setting_up_twice_is_refused() {
    let app = start().await;

    app.post("/api/setup", serde_json::json!({ "password": "first" })).await;
    let second = app.post("/api/setup", serde_json::json!({ "password": "second" })).await;

    assert_eq!(second.status, 409, "a second setup would have orphaned the contents");
}

// ── auto-lock ───────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_lock_policy_can_be_changed_and_switched_off() {
    let app = start().await;
    app.post("/api/setup", serde_json::json!({ "password": "the master password" })).await;

    let reply = http::send(
        "PUT",
        &format!("{}/api/settings/lock", app.base),
        Some(&app.token),
        Some(&serde_json::json!({ "idle_seconds": 300, "hidden_seconds": null }).to_string()),
    )
    .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.json()["lock"]["policy"]["idle_seconds"], 300);
    assert_eq!(reply.json()["lock"]["policy"]["hidden_seconds"], serde_json::Value::Null);

    let off = http::send(
        "PUT",
        &format!("{}/api/settings/lock", app.base),
        Some(&app.token),
        Some(&serde_json::json!({ "idle_seconds": null, "hidden_seconds": null }).to_string()),
    )
    .await;

    assert_eq!(off.json()["lock"]["locks_in"], serde_json::Value::Null);
}

#[tokio::test]
async fn hiding_the_window_shortens_the_countdown() {
    let app = start().await;
    app.post("/api/setup", serde_json::json!({ "password": "the master password" })).await;

    let before = app.get("/api/status").await.json()["lock"]["locks_in"]
        .as_u64()
        .unwrap();

    let after = app
        .post("/api/visibility", serde_json::json!({ "visible": false }))
        .await
        .json()["lock"]["locks_in"]
        .as_u64()
        .unwrap();

    assert!(after < before, "hiding the window did not shorten anything: {before} → {after}");
}

// ── the second factor ───────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_second_factor_is_off_until_a_code_proves_the_app_has_it() {
    let app = start().await;
    app.post("/api/setup", serde_json::json!({ "password": "the master password" })).await;

    let begun = app.post("/api/second-factor/begin", serde_json::json!({})).await;
    assert_eq!(begun.status, 200);

    let secret = begun.json()["secret"].as_str().unwrap().to_string();
    assert!(begun.json()["qr"].as_str().unwrap().starts_with("data:image/svg+xml;base64,"));

    // Beginning changes nothing: a mis-scan must not lock somebody out of their own vault.
    assert_eq!(app.get("/api/status").await.json()["identity"]["second_factor"], false);

    let refused = app
        .post("/api/second-factor", serde_json::json!({ "secret": secret, "code": "000000" }))
        .await;
    assert_eq!(refused.status, 400);
    assert_eq!(app.get("/api/status").await.json()["identity"]["second_factor"], false);
}

#[tokio::test]
async fn a_vault_with_a_second_factor_will_not_open_on_the_password_alone() {
    let app = start().await;
    app.post("/api/setup", serde_json::json!({ "password": "the master password" })).await;

    let secret = app
        .post("/api/second-factor/begin", serde_json::json!({}))
        .await
        .json()["secret"]
        .as_str()
        .unwrap()
        .to_string();

    let raw = keyhold::encoding::from_base32(&secret).unwrap();
    let code = keyhold::totp::code_now(&raw, keyhold::identity::now_seconds());

    let enabled = app
        .post("/api/second-factor", serde_json::json!({ "secret": secret, "code": code }))
        .await;
    assert_eq!(enabled.status, 200);
    assert_eq!(app.get("/api/status").await.json()["identity"]["second_factor"], true);

    app.post("/api/lock", serde_json::json!({})).await;

    let without = app
        .post("/api/unlock", serde_json::json!({ "password": "the master password" }))
        .await;
    assert_eq!(without.status, 401);

    let code = keyhold::totp::code_now(&raw, keyhold::identity::now_seconds());
    let with = app
        .post(
            "/api/unlock",
            serde_json::json!({ "password": "the master password", "code": code }),
        )
        .await;
    assert_eq!(with.status, 200);
}

#[tokio::test]
async fn turning_the_second_factor_on_needs_an_open_vault() {
    let app = start().await;
    app.post("/api/setup", serde_json::json!({ "password": "the master password" })).await;
    app.post("/api/lock", serde_json::json!({})).await;

    assert_eq!(app.post("/api/second-factor/begin", serde_json::json!({})).await.status, 423);
}

// ── the generator ───────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_generator_answers_before_a_vault_exists() {
    let app = start().await;

    // The setup screen needs a password to suggest, and at that point there is no vault at all.
    let reply = app
        .post(
            "/api/password",
            serde_json::json!({
                "length": 20,
                "lowercase": true,
                "uppercase": true,
                "digits": true,
                "symbols": true,
                "avoid_ambiguous": false
            }),
        )
        .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.json()["password"].as_str().unwrap().chars().count(), 20);
    assert!(reply.json()["entropyBits"].as_u64().unwrap() > 100);
}

// ── vaults, folders and records ─────────────────────────────────────────────────────────────────

/// Set up the file and return the vault that setup itself created.
///
/// Setting up makes one vault, because every screen past the last setup step needs somewhere to put
/// the first record. These tests use that one rather than making a second, which is also what
/// somebody opening keyhold for the first time will do.
async fn with_one_vault(app: &Running) -> String {
    app.post("/api/setup", serde_json::json!({ "password": "the master password" })).await;

    let listed = app.get("/api/vaults").await.json();

    listed["vaults"][0]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_record_round_trips_through_the_api() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    let made = app
        .post(
            &format!("/api/vaults/{vault}/records"),
            serde_json::json!({
                "kind": "login",
                "payload": {
                    "title": "Produção",
                    "username": "deploy",
                    "url": "https://example.com",
                    "notes": "",
                    "fields": [{ "label": "Senha", "kind": "password", "value": "hunter2" }]
                }
            }),
        )
        .await;

    assert_eq!(made.status, 200);
    let id = made.json()["record"]["id"].as_str().unwrap().to_string();

    // The list names each record and carries none of their secrets.
    let listed = app.get(&format!("/api/vaults/{vault}/records")).await;
    assert_eq!(listed.json()["records"].as_array().unwrap().len(), 1);
    assert_eq!(listed.json()["records"][0]["title"], "Produção");
    assert!(!listed.body.contains("hunter2"), "the list handed out a secret: {}", listed.body);

    // Opening one does.
    let opened = app.get(&format!("/api/records/{id}")).await;
    assert_eq!(opened.json()["record"]["payload"]["fields"][0]["value"], "hunter2");
    assert_eq!(opened.json()["record"]["payload"]["title"], "Produção");
}

#[tokio::test]
async fn nothing_about_the_contents_answers_while_the_vault_is_locked() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    let record = app
        .post(
            &format!("/api/vaults/{vault}/records"),
            serde_json::json!({
                "kind": "login",
                "payload": { "title": "Produção", "fields": [] }
            }),
        )
        .await
        .json()["record"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    app.post("/api/lock", serde_json::json!({})).await;

    // Not just the secret: the shape of a locked vault is not public either.
    for path in [
        "/api/vaults".to_string(),
        format!("/api/vaults/{vault}/folders"),
        format!("/api/vaults/{vault}/records"),
        format!("/api/records/{record}"),
        format!("/api/records/{record}/versions"),
    ] {
        assert_eq!(app.get(&path).await.status, 423, "{path} answered a locked vault");
    }
}

#[tokio::test]
async fn setting_up_leaves_somewhere_to_put_the_first_record() {
    let app = start().await;
    app.post("/api/setup", serde_json::json!({ "password": "the master password" })).await;

    let listed = app.get("/api/vaults").await.json();

    assert_eq!(listed["vaults"].as_array().unwrap().len(), 1);
    assert_eq!(listed["vaults"][0]["name"], "Pessoal");
}

#[tokio::test]
async fn a_vault_is_invisible_to_an_agent_until_it_is_marked() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    let agent_access = |listed: &serde_json::Value| {
        listed["vaults"]
            .as_array()
            .unwrap()
            .iter()
            .find(|one| one["id"] == vault.as_str())
            .unwrap()["agent_access"]
            .clone()
    };

    assert_eq!(agent_access(&app.get("/api/vaults").await.json()), false);

    app.post(&format!("/api/vaults/{vault}/agent-access"), serde_json::json!({ "on": true }))
        .await;

    assert_eq!(agent_access(&app.get("/api/vaults").await.json()), true);
}

#[tokio::test]
async fn folders_nest_and_refuse_to_nest_wrongly() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    let top = app
        .post(&format!("/api/vaults/{vault}/folders"), serde_json::json!({ "name": "Clientes" }))
        .await
        .json()["folder"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let inner = app
        .post(
            &format!("/api/vaults/{vault}/folders"),
            serde_json::json!({ "name": "Andrade", "parent_id": top }),
        )
        .await;

    assert_eq!(inner.status, 200);
    assert_eq!(inner.json()["folder"]["depth"], 1);

    let into_itself = app
        .post(&format!("/api/folders/{top}/move"), serde_json::json!({ "parent_id": top }))
        .await;

    assert_eq!(into_itself.status, 400);
}

#[tokio::test]
async fn saving_a_record_keeps_what_it_replaced() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    let id = app
        .post(
            &format!("/api/vaults/{vault}/records"),
            serde_json::json!({
                "kind": "login",
                "payload": {
                    "title": "Produção",
                    "fields": [{ "label": "Senha", "kind": "password", "value": "antiga" }]
                }
            }),
        )
        .await
        .json()["record"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let updated = http::send(
        "PUT",
        &format!("{}/api/records/{id}", app.base),
        Some(&app.token),
        Some(
            &serde_json::json!({
                "title": "Produção",
                "fields": [{ "label": "Senha", "kind": "password", "value": "nova" }]
            })
            .to_string(),
        ),
    )
    .await;

    assert_eq!(updated.json()["record"]["version"], 2);

    let history = app.get(&format!("/api/records/{id}/versions")).await;
    assert_eq!(history.json()["versions"].as_array().unwrap().len(), 1);

    let old = app.get(&format!("/api/records/{id}/versions/1")).await;
    assert_eq!(old.json()["payload"]["fields"][0]["value"], "antiga");

    let restored = app
        .post(&format!("/api/records/{id}/versions/1/restore"), serde_json::json!({}))
        .await;
    assert_eq!(restored.json()["record"]["version"], 3);

    let now = app.get(&format!("/api/records/{id}")).await;
    assert_eq!(now.json()["record"]["payload"]["fields"][0]["value"], "antiga");
}

#[tokio::test]
async fn deleting_a_record_puts_it_in_the_trash_rather_than_destroying_it() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    let id = app
        .post(
            &format!("/api/vaults/{vault}/records"),
            serde_json::json!({ "kind": "note", "payload": { "title": "Rascunho", "fields": [] } }),
        )
        .await
        .json()["record"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    app.post(&format!("/api/records/{id}/archive"), serde_json::json!({ "on": true })).await;

    let visible = app.get(&format!("/api/vaults/{vault}/records")).await;
    assert!(visible.json()["records"].as_array().unwrap().is_empty());

    let including = app.get(&format!("/api/vaults/{vault}/records?archived=true")).await;
    assert_eq!(including.json()["records"].as_array().unwrap().len(), 1);

    app.post(&format!("/api/records/{id}/archive"), serde_json::json!({ "on": false })).await;
    assert_eq!(
        app.get(&format!("/api/vaults/{vault}/records")).await.json()["records"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

// ── health, rotation and the trash (F5) ─────────────────────────────────────────────────────────

#[tokio::test]
async fn health_names_what_is_wrong_and_stays_quiet_otherwise() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    let weak = |title: &str, password: &str| {
        serde_json::json!({
            "kind": "login",
            "payload": {
                "title": title,
                "fields": [{ "label": "Senha", "kind": "password", "value": password }]
            }
        })
    };

    app.post(&format!("/api/vaults/{vault}/records"), weak("Forte", "9k2Lm4pQz7XwTb1r!Vd"))
        .await;

    let clean = app.get(&format!("/api/vaults/{vault}/health")).await;
    assert_eq!(clean.json()["health"]["findings"].as_array().unwrap().len(), 0);

    app.post(&format!("/api/vaults/{vault}/records"), weak("Fraca", "Verao2024!")).await;

    let found = app.get(&format!("/api/vaults/{vault}/health")).await;
    let findings = found.json()["health"]["findings"].clone();

    assert_eq!(findings.as_array().unwrap().len(), 1);
    assert_eq!(findings[0]["title"], "Fraca");
    assert!(
        findings[0]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason.as_str().unwrap().contains("palavra com números")),
        "{findings}"
    );

    // The findings say what is wrong; they never carry the password itself.
    assert!(!found.body.contains("Verao2024"), "health handed back a secret: {}", found.body);
}

#[tokio::test]
async fn rotating_a_vault_keeps_its_contents_readable() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    let id = app
        .post(
            &format!("/api/vaults/{vault}/records"),
            serde_json::json!({
                "kind": "login",
                "payload": {
                    "title": "Servidor",
                    "fields": [{ "label": "Senha", "kind": "password", "value": "hunter2" }]
                }
            }),
        )
        .await
        .json()["record"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let rotated = app.post(&format!("/api/vaults/{vault}/rotate"), serde_json::json!({})).await;

    assert_eq!(rotated.status, 200);
    assert_eq!(rotated.json()["vault"]["key_version"], 2);

    let opened = app.get(&format!("/api/records/{id}")).await;
    assert_eq!(opened.json()["record"]["payload"]["fields"][0]["value"], "hunter2");
}

#[tokio::test]
async fn the_trash_holds_what_was_deleted_until_it_is_destroyed() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    let id = app
        .post(
            &format!("/api/vaults/{vault}/records"),
            serde_json::json!({ "kind": "note", "payload": { "title": "Rascunho", "fields": [] } }),
        )
        .await
        .json()["record"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Destroying something that is not in the trash is refused.
    let early = app.post(&format!("/api/records/{id}/destroy"), serde_json::json!({})).await;
    assert_eq!(early.status, 400);

    app.post(&format!("/api/records/{id}/archive"), serde_json::json!({ "on": true })).await;

    let waiting = app.get(&format!("/api/vaults/{vault}/trash")).await;
    assert_eq!(waiting.json()["records"].as_array().unwrap().len(), 1);
    assert_eq!(waiting.json()["retention_days"], 30);

    let destroyed = app.post(&format!("/api/records/{id}/destroy"), serde_json::json!({})).await;
    assert_eq!(destroyed.status, 200);

    assert_eq!(app.get(&format!("/api/records/{id}")).await.status, 400);
    assert!(app
        .get(&format!("/api/vaults/{vault}/trash"))
        .await
        .json()["records"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn the_retention_window_can_be_changed_and_switched_off() {
    let app = start().await;
    with_one_vault(&app).await;

    let saved = http::send(
        "PUT",
        &format!("{}/api/settings/retention", app.base),
        Some(&app.token),
        Some(&serde_json::json!({ "days": 7 }).to_string()),
    )
    .await;

    assert_eq!(saved.json()["retention_days"], 7);
    assert_eq!(app.get("/api/status").await.json()["retention_days"], 7);

    let off = http::send(
        "PUT",
        &format!("{}/api/settings/retention", app.base),
        Some(&app.token),
        Some(&serde_json::json!({ "days": 0 }).to_string()),
    )
    .await;

    assert_eq!(off.json()["retention_days"], 0);
}

#[tokio::test]
async fn unlocking_empties_what_the_trash_has_held_too_long() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    let id = app
        .post(
            &format!("/api/vaults/{vault}/records"),
            serde_json::json!({ "kind": "note", "payload": { "title": "Antigo", "fields": [] } }),
        )
        .await
        .json()["record"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    app.post(&format!("/api/records/{id}/archive"), serde_json::json!({ "on": true })).await;

    // Age the deletion past the window, the way a month of calendar would.
    {
        let store = app.store.lock().unwrap();
        store
            .conn()
            .execute(
                "UPDATE records SET archived_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-40 days')
                 WHERE id = ?1",
                [&id],
            )
            .unwrap();
    }

    app.post("/api/lock", serde_json::json!({})).await;

    let reopened = app
        .post("/api/unlock", serde_json::json!({ "password": "the master password" }))
        .await;

    assert_eq!(reopened.json()["purged"], 1, "the purge did not run on unlock");
    assert!(app
        .get(&format!("/api/vaults/{vault}/trash"))
        .await
        .json()["records"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn deleting_a_vault_needs_its_name_and_takes_everything_with_it() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    app.post(
        &format!("/api/vaults/{vault}/records"),
        serde_json::json!({ "kind": "note", "payload": { "title": "Segredo", "fields": [] } }),
    )
    .await;

    let counted = app.get(&format!("/api/vaults/{vault}/contents")).await;
    assert_eq!(counted.json()["contents"]["records"], 1);

    let wrong = http::send(
        "DELETE",
        &format!("{}/api/vaults/{vault}", app.base),
        Some(&app.token),
        Some(&serde_json::json!({ "confirm_name": "outro nome" }).to_string()),
    )
    .await;

    assert_eq!(wrong.status, 400, "a vault was deleted without its name");
    assert_eq!(app.get("/api/vaults").await.json()["vaults"].as_array().unwrap().len(), 1);

    let right = http::send(
        "DELETE",
        &format!("{}/api/vaults/{vault}", app.base),
        Some(&app.token),
        Some(&serde_json::json!({ "confirm_name": "Pessoal" }).to_string()),
    )
    .await;

    assert_eq!(right.status, 200);
    assert_eq!(right.json()["deleted"]["records"], 1);
    assert!(app.get("/api/vaults").await.json()["vaults"].as_array().unwrap().is_empty());
}

// ── the trail (F6) ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_trail_records_what_happened_and_verifies() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    let id = app
        .post(
            &format!("/api/vaults/{vault}/records"),
            serde_json::json!({
                "kind": "login",
                "payload": {
                    "title": "Servidor",
                    "fields": [{ "label": "Senha", "kind": "password", "value": "hunter2" }]
                }
            }),
        )
        .await
        .json()["record"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    app.post(
        &format!("/api/records/{id}/accesses"),
        serde_json::json!({ "kind": "reveal", "field": "Senha" }),
    )
    .await;

    let trail = app.get("/api/audit").await;
    let events = trail.json()["events"].clone();
    let actions: Vec<&str> =
        events.as_array().unwrap().iter().map(|e| e["action"].as_str().unwrap()).collect();

    assert!(actions.contains(&"secret.revealed"), "{actions:?}");
    assert!(actions.contains(&"record.created"), "{actions:?}");
    assert!(actions.contains(&"vault.created"), "{actions:?}");

    // The trail says a field called "Senha" was revealed, never what it said.
    assert!(!trail.body.contains("hunter2"), "the trail carried a secret: {}", trail.body);
    assert_eq!(events[0]["detail"]["field"], "Senha");

    let integrity = app.get("/api/audit/integrity").await;
    assert_eq!(integrity.json()["integrity"]["intact"], true);
}

#[tokio::test]
async fn a_failed_unlock_is_written_down() {
    let app = start().await;
    with_one_vault(&app).await;

    app.post("/api/lock", serde_json::json!({})).await;
    app.post("/api/unlock", serde_json::json!({ "password": "errada" })).await;
    app.post("/api/unlock", serde_json::json!({ "password": "the master password" })).await;

    let events = app.get("/api/audit").await.json()["events"].clone();
    let failed = events
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["action"] == "vault.unlock.failed")
        .expect("a failed unlock should be recorded");

    assert_eq!(failed["result"], "denied");
    assert_eq!(failed["detail"]["attempt"], 1);
}

#[tokio::test]
async fn editing_the_trail_in_the_file_is_detected() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    app.post(
        &format!("/api/vaults/{vault}/records"),
        serde_json::json!({ "kind": "note", "payload": { "title": "Uma nota", "fields": [] } }),
    )
    .await;

    assert_eq!(app.get("/api/audit/integrity").await.json()["integrity"]["intact"], true);

    {
        let store = app.store.lock().unwrap();
        store
            .conn()
            .execute("UPDATE audit_events SET action = 'vault.lock' WHERE seq = 1", [])
            .unwrap();
    }

    let integrity = app.get("/api/audit/integrity").await.json();

    assert_eq!(integrity["integrity"]["intact"], false);
    assert_eq!(integrity["integrity"]["broken_at"], 1);
}

#[tokio::test]
async fn reads_stop_being_recorded_when_that_is_switched_off() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    let id = app
        .post(
            &format!("/api/vaults/{vault}/records"),
            serde_json::json!({ "kind": "note", "payload": { "title": "Uma nota", "fields": [] } }),
        )
        .await
        .json()["record"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    http::send(
        "PUT",
        &format!("{}/api/settings/log-reads", app.base),
        Some(&app.token),
        Some(&serde_json::json!({ "on": false }).to_string()),
    )
    .await;

    app.post(
        &format!("/api/records/{id}/accesses"),
        serde_json::json!({ "kind": "reveal", "field": "Senha" }),
    )
    .await;

    let actions: Vec<String> = app.get("/api/audit").await.json()["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["action"].as_str().unwrap().to_string())
        .collect();

    assert!(!actions.iter().any(|action| action == "secret.revealed"), "{actions:?}");
    // Writes are still recorded: switching off reads is not switching off the trail.
    assert!(actions.iter().any(|action| action == "record.created"), "{actions:?}");
}

#[tokio::test]
async fn the_trail_says_nothing_while_the_vault_is_locked() {
    let app = start().await;
    with_one_vault(&app).await;

    app.post("/api/lock", serde_json::json!({})).await;

    assert_eq!(app.get("/api/audit").await.status, 423);
    assert_eq!(app.get("/api/audit/integrity").await.status, 423);
}

#[tokio::test]
async fn a_vault_can_be_exported_and_brought_back() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    app.post(
        &format!("/api/vaults/{vault}/records"),
        serde_json::json!({
            "kind": "login",
            "payload": {
                "title": "Servidor",
                "username": "deploy",
                "fields": [{ "label": "Senha", "kind": "password", "value": "hunter2" }]
            }
        }),
    )
    .await;

    let short = app
        .post(&format!("/api/vaults/{vault}/export"), serde_json::json!({ "passphrase": "curta" }))
        .await;
    assert_eq!(short.status, 400, "a short passphrase should be refused");

    let exported = app
        .post(
            &format!("/api/vaults/{vault}/export"),
            serde_json::json!({ "passphrase": "uma senha de arquivo" }),
        )
        .await;

    assert_eq!(exported.status, 200);
    assert!(!exported.body.contains("hunter2"), "the archive leaked a secret");
    assert!(!exported.body.contains("Servidor"), "the archive leaked a title");

    let envelope = exported.json()["archive"].clone();

    // Bringing it back into a second vault reproduces the record.
    let other = app
        .post("/api/vaults", serde_json::json!({ "name": "Cópia", "description": "", "colour": "default" }))
        .await
        .json()["vault"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let wrong = app
        .post(
            &format!("/api/vaults/{other}/import"),
            serde_json::json!({
                "from": "archive",
                "envelope": envelope,
                "passphrase": "outra senha inteira"
            }),
        )
        .await;
    assert_eq!(wrong.status, 400, "the wrong passphrase opened the archive");

    let brought = app
        .post(
            &format!("/api/vaults/{other}/import"),
            serde_json::json!({
                "from": "archive",
                "envelope": envelope,
                "passphrase": "uma senha de arquivo"
            }),
        )
        .await;

    assert_eq!(brought.status, 200);
    assert_eq!(brought.json()["imported"]["records"], 1);

    let listed = app.get(&format!("/api/vaults/{other}/records")).await;
    assert_eq!(listed.json()["records"][0]["title"], "Servidor");
}

#[tokio::test]
async fn a_csv_from_another_manager_can_be_imported() {
    let app = start().await;
    let vault = with_one_vault(&app).await;

    let done = app
        .post(
            &format!("/api/vaults/{vault}/import"),
            serde_json::json!({
                "from": "csv",
                "text": "name,username,password,folder\nServidor,deploy,hunter2,Infra\nBanco,daniel,outra,Infra\n"
            }),
        )
        .await;

    assert_eq!(done.status, 200);
    assert_eq!(done.json()["imported"]["records"], 2);
    assert_eq!(done.json()["imported"]["folders"], 1);

    let listed = app.get(&format!("/api/vaults/{vault}/records")).await;
    assert_eq!(listed.json()["records"].as_array().unwrap().len(), 2);

    // And the import is in the trail, saying where it came from.
    let events = app.get("/api/audit").await.json()["events"].clone();
    let imported = events
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["action"] == "vault.imported")
        .expect("the import should be recorded");

    assert_eq!(imported["detail"]["from"], "csv");
    assert_eq!(imported["detail"]["records"], 2);
}
