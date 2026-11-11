//! AWS vaults end to end, against a Parameter Store that lives in this test.
//!
//! The fake below keeps parameters in a map and answers the operations Keyhold uses the way AWS
//! does: versions that climb, `ParameterAlreadyExists`, a default key when none is named, tags
//! only on create, history, and a path somebody's policy denies. That last one matters most: it
//! is in `DescribeParameters` but refused by everything that reads a value, which is how A6 is
//! checked (what cannot be read does not appear).
//!
//! Identity Center is not involved: the test hands the connection credentials directly, which is
//! what a finished login would have done. No request leaves the machine.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

use crate::aws::config::{self, AwsConfig, Connection, Login};
use crate::aws::fake::{Canned, FakeAws};
use crate::aws::state::Endpoints;
use crate::aws::Credentials;
use crate::crypto::SecretKey;
use crate::db::Store;
use crate::http::{router, AppState, TOKEN_HEADER};
use crate::lock::{Lock, LockPolicy};
use crate::session::SessionToken;
use crate::vaults::{self, AwsVault, VaultMeta};

// ── the fake ────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct FakeParameter {
    value: String,
    version: i64,
    description: Option<String>,
    key_id: String,
    tags: Vec<(String, String)>,
    /// (version, value, description, modified)
    history: Vec<(i64, String, Option<String>, f64)>,
    modified: f64,
    /// `SecureString` when empty; `String` or `StringList` for the plain-text kinds.
    kind: String,
}

impl FakeParameter {
    fn kind(&self) -> &str {
        if self.kind.is_empty() { "SecureString" } else { &self.kind }
    }
}

#[derive(Default)]
struct Ssm {
    parameters: BTreeMap<String, FakeParameter>,
    /// Names under this prefix exist, but this role may not read them.
    denied: Vec<String>,
    /// The role may not call DescribeParameters at all.
    describe_denied: bool,
    /// The role may not call GetParameterHistory.
    history_denied: bool,
    /// Names whose PutParameter is refused (everything else about them is allowed).
    put_denied: Vec<String>,
    /// Names whose DeleteParameter is refused.
    delete_denied: Vec<String>,
    /// RemoveTagsFromResource is refused.
    untag_denied: bool,
    /// Names where somebody else writes a version just before the next PutParameter lands.
    race: Vec<String>,
    clock: f64,
    /// Every `PutParameter` body, to check what was asked for.
    puts: Vec<Value>,
    /// Every operation, in order, with the name it was about.
    calls: Vec<(String, String)>,
}

type Shared = Arc<Mutex<Ssm>>;

impl Ssm {
    fn seed(&mut self, name: &str, value: &str, description: Option<&str>, key: &str, tags: &[(&str, &str)]) {
        self.clock += 1.0;
        self.parameters.insert(
            name.to_string(),
            FakeParameter {
                value: value.into(),
                version: 1,
                description: description.map(str::to_string),
                key_id: key.into(),
                tags: tags.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                history: vec![(1, value.into(), description.map(str::to_string), self.clock)],
                modified: self.clock,
                kind: String::new(),
            },
        );
    }

    fn denied(&self, name: &str) -> bool {
        self.denied.iter().any(|prefix| name.starts_with(prefix))
    }

    fn under<'a>(&'a self, path: &str, recursive: bool) -> impl Iterator<Item = (&'a String, &'a FakeParameter)> {
        let prefix = if path == "/" { "/".to_string() } else { format!("{path}/") };
        self.parameters.iter().filter(move |(name, _)| {
            name.starts_with(&prefix) && (recursive || !name[prefix.len()..].contains('/'))
        })
    }
}

fn error(code: &str, message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [("content-type", "application/x-amz-json-1.1")],
        json!({ "__type": code, "message": message }).to_string(),
    )
        .into_response()
}

fn ok(body: Value) -> Response {
    (StatusCode::OK, [("content-type", "application/x-amz-json-1.1")], body.to_string()).into_response()
}

fn page(items: Vec<Value>, size: usize, token: Option<&str>) -> (Vec<Value>, Option<String>) {
    let start: usize = token.and_then(|t| t.parse().ok()).unwrap_or(0);
    let end = (start + size).min(items.len());
    let next = (end < items.len()).then(|| end.to_string());
    (items[start..end].to_vec(), next)
}

fn tags_json(tags: &[(String, String)]) -> Value {
    json!(tags.iter().map(|(k, v)| json!({ "Key": k, "Value": v })).collect::<Vec<_>>())
}

async fn ssm(State(shared): State<Shared>, request: Request) -> Response {
    let target = request
        .headers()
        .get("x-amz-target")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .trim_start_matches("AmazonSSM.")
        .to_string();
    assert!(request.headers().get("authorization").is_some(), "every request is signed");

    let body = axum::body::to_bytes(request.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let text = |key: &str| body[key].as_str().unwrap_or_default().to_string();
    let mut ssm = shared.lock().unwrap();
    let about = if body["Name"].is_string() { text("Name") } else { text("ResourceId") };
    ssm.calls.push((target.clone(), about));

    match target.as_str() {
        "GetParametersByPath" => {
            let path = text("Path");
            let items: Vec<Value> = ssm
                .under(&path, body["Recursive"].as_bool().unwrap_or(false))
                .filter(|(name, _)| !ssm.denied(name))
                .map(|(name, p)| {
                    json!({
                        "Name": name, "Type": p.kind(), "Value": p.value,
                        "Version": p.version, "LastModifiedDate": p.modified,
                    })
                })
                .collect();
            let (items, next) = page(items, 10, body["NextToken"].as_str());
            ok(json!({ "Parameters": items, "NextToken": next }))
        }
        "DescribeParameters" => {
            if ssm.describe_denied {
                return error("AccessDeniedException", "not allowed to describe");
            }
            let filter = &body["ParameterFilters"][0];
            let value = filter["Values"][0].as_str().unwrap_or("/").to_string();
            let describe = |name: &String, p: &FakeParameter| {
                json!({
                    "Name": name, "Type": p.kind(),
                    "KeyId": (p.kind() == "SecureString").then(|| p.key_id.clone()),
                    "Description": p.description, "Version": p.version,
                    "LastModifiedDate": p.modified, "Tier": "Standard",
                })
            };
            // Describe sees everything, denied or not. Keyhold must not show what it only saw here.
            let items: Vec<Value> = if filter["Key"] == "Name" {
                ssm.parameters.iter().filter(|(name, _)| **name == value).map(|(n, p)| describe(n, p)).collect()
            } else {
                ssm.under(&value, filter["Option"] == "Recursive").map(|(n, p)| describe(n, p)).collect()
            };
            let (items, next) = page(items, 50, body["NextToken"].as_str());
            ok(json!({ "Parameters": items, "NextToken": next }))
        }
        "GetParameter" => {
            let name = text("Name");
            if ssm.denied(&name) {
                return error("AccessDeniedException", "not allowed");
            }
            match ssm.parameters.get(&name) {
                None => error("ParameterNotFound", "nope"),
                Some(p) => ok(json!({ "Parameter": {
                    "Name": name, "Type": p.kind(), "Value": p.value,
                    "Version": p.version, "LastModifiedDate": p.modified,
                }})),
            }
        }
        "PutParameter" => {
            let name = text("Name");
            ssm.puts.push(body.clone());
            if ssm.denied(&name) || ssm.put_denied.contains(&name) {
                return error("AccessDeniedException", "not allowed");
            }
            assert_eq!(body["Type"], "SecureString");
            let overwrite = body["Overwrite"].as_bool().unwrap_or(false);
            ssm.clock += 1.0;
            let clock = ssm.clock;
            let key = body["KeyId"].as_str().unwrap_or("alias/aws/ssm").to_string();
            let description = body["Description"].as_str().map(str::to_string);
            let value = text("Value");
            let raced = ssm.race.contains(&name);
            ssm.race.retain(|n| *n != name);

            let version = match ssm.parameters.get_mut(&name) {
                Some(_) if !overwrite => return error("ParameterAlreadyExists", "exists"),
                Some(existing) => {
                    if raced {
                        existing.version += 1;
                        let v = existing.version;
                        existing.history.push((v, "de outra pessoa".into(), existing.description.clone(), clock));
                    }
                    existing.version += 1;
                    existing.value = value.clone();
                    existing.key_id = key;
                    existing.description = description.clone();
                    existing.modified = clock;
                    existing.history.push((existing.version, value, description, clock));
                    existing.version
                }
                None => {
                    let tags = body["Tags"]
                        .as_array()
                        .map(|tags| {
                            tags.iter()
                                .map(|t| (t["Key"].as_str().unwrap().into(), t["Value"].as_str().unwrap().into()))
                                .collect()
                        })
                        .unwrap_or_default();
                    ssm.parameters.insert(
                        name,
                        FakeParameter {
                            value: value.clone(),
                            version: 1,
                            description: description.clone(),
                            key_id: key,
                            tags,
                            history: vec![(1, value, description, clock)],
                            modified: clock,
                            kind: String::new(),
                        },
                    );
                    1
                }
            };
            ok(json!({ "Version": version, "Tier": "Standard" }))
        }
        "DeleteParameter" => {
            let name = text("Name");
            if ssm.delete_denied.contains(&name) {
                return error("AccessDeniedException", "not allowed to delete");
            }
            match ssm.parameters.remove(&name) {
                Some(_) => ok(json!({})),
                None => error("ParameterNotFound", "nope"),
            }
        }
        "GetParameterHistory" => {
            if ssm.history_denied {
                return error("AccessDeniedException", "not allowed to read history");
            }
            let name = text("Name");
            let decrypt = body["WithDecryption"].as_bool().unwrap_or(false);
            let Some(p) = ssm.parameters.get(&name) else {
                return error("ParameterNotFound", "nope");
            };
            let items: Vec<Value> = p
                .history
                .iter()
                .map(|(version, value, description, at)| {
                    json!({
                        "Name": name, "Type": p.kind(), "KeyId": p.key_id,
                        "Description": description,
                        "Value": if decrypt { value.clone() } else { "(cifrado)".to_string() },
                        "Version": version, "LastModifiedDate": at, "Tier": "Standard",
                    })
                })
                .collect();
            ok(json!({ "Parameters": items }))
        }
        "ListTagsForResource" => match ssm.parameters.get(&text("ResourceId")) {
            Some(p) => ok(json!({ "TagList": tags_json(&p.tags) })),
            None => error("InvalidResourceId", "nope"),
        },
        "AddTagsToResource" => {
            let Some(p) = ssm.parameters.get_mut(&text("ResourceId")) else {
                return error("InvalidResourceId", "nope");
            };
            for tag in body["Tags"].as_array().unwrap() {
                let (k, v) = (tag["Key"].as_str().unwrap(), tag["Value"].as_str().unwrap());
                p.tags.retain(|(key, _)| key != k);
                p.tags.push((k.into(), v.into()));
            }
            ok(json!({}))
        }
        "RemoveTagsFromResource" => {
            if ssm.untag_denied {
                return error("AccessDeniedException", "not allowed to untag");
            }
            let Some(p) = ssm.parameters.get_mut(&text("ResourceId")) else {
                return error("InvalidResourceId", "nope");
            };
            let keys: Vec<&str> = body["TagKeys"].as_array().unwrap().iter().filter_map(|k| k.as_str()).collect();
            p.tags.retain(|(key, _)| !keys.contains(&key.as_str()));
            ok(json!({}))
        }
        other => error("InvalidAction", other),
    }
}

async fn start_ssm(shared: Shared) -> String {
    let app = axum::Router::new().fallback(ssm).with_state(shared);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}")
}

// ── Keyhold, pointed at it ──────────────────────────────────────────────────────────────────────

struct Harness {
    base: String,
    token: String,
    vault: String,
    ssm: Shared,
    state: Arc<AppState>,
    client: reqwest::Client,
    _dir: tempfile::TempDir,
}

struct Reply {
    status: u16,
    body: Value,
}

impl Harness {
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> Reply {
        let url = format!("{}/api{path}", self.base);
        let mut request = self
            .client
            .request(reqwest::Method::from_bytes(method.as_bytes()).unwrap(), url)
            .header(TOKEN_HEADER, &self.token);
        if let Some(body) = body {
            request = request.header("content-type", "application/json").body(body.to_string());
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let text = response.text().await.unwrap();
        Reply { status, body: serde_json::from_str(&text).unwrap_or(Value::String(text)) }
    }

    async fn get(&self, path: &str) -> Reply {
        self.call("GET", path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> Reply {
        self.call("POST", path, Some(body)).await
    }

    async fn put(&self, path: &str, body: Value) -> Reply {
        self.call("PUT", path, Some(body)).await
    }

    fn id(&self, name: &str) -> String {
        crate::aws::mapping::record_id(&self.vault, name)
    }

    fn folder(&self, path: &str) -> String {
        crate::aws::mapping::folder_id(&self.vault, path)
    }

    fn attachment(&self, key: &str) -> String {
        crate::aws::mapping::attachment_id(&self.vault, key)
    }

    fn with_ssm<T>(&self, f: impl FnOnce(&mut Ssm) -> T) -> T {
        f(&mut self.ssm.lock().unwrap())
    }

    fn parameter(&self, name: &str) -> Option<FakeParameter> {
        self.ssm.lock().unwrap().parameters.get(name).cloned()
    }

    async fn records(&self) -> Vec<Value> {
        let reply = self.get(&format!("/vaults/{}/records", self.vault)).await;
        assert_eq!(reply.status, 200, "{}", reply.body);
        reply.body["records"].as_array().unwrap().clone()
    }
}

async fn start(seed: impl FnOnce(&mut Ssm), bucket: Option<String>, s3: Option<String>) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("vault.db")).unwrap();
    let key = SecretKey::random();

    let mut config = AwsConfig::default();
    config.logins.push(Login {
        id: "LOGIN".into(),
        start_url: "https://exemplo.awsapps.com/start".into(),
        region: "sa-east-1".into(),
        label: "Exemplo".into(),
    });
    config.connections.push(Connection {
        id: "CONN".into(),
        login_id: "LOGIN".into(),
        account_id: "123456789012".into(),
        account_name: "Produção".into(),
        role: "Leitura".into(),
        title: "123456789012 / Leitura".into(),
        note: String::new(),
    });
    config::save(&store, &key, &config).unwrap();

    let vault = vaults::create_vault(
        &store,
        &key,
        VaultMeta {
            name: "Produção".into(),
            kind: "aws".into(),
            aws: Some(AwsVault { connection_id: "CONN".into(), region: "sa-east-1".into(), bucket, rename_with_title: false }),
            ..Default::default()
        },
    )
    .unwrap();

    let shared: Shared = Arc::default();
    seed(&mut shared.lock().unwrap());
    let ssm_base = start_ssm(shared.clone()).await;

    let token = SessionToken::generate();
    let exposed = token.expose().to_string();
    let lock = Lock::new(LockPolicy { idle_seconds: None, hidden_seconds: None });
    lock.unlock_with(key);

    let state = Arc::new(AppState {
        token,
        agent: Default::default(),
        store: Arc::new(Mutex::new(store)),
        lock,
        failures: Mutex::new(0),
        aws: Default::default(),
        copies: Default::default(),
    });
    state.aws.set_endpoints(Endpoints { ssm: Some(ssm_base), s3, ..Endpoints::default() });
    state.aws.insert_credentials(
        &state.lock,
        "CONN",
        Credentials::new("AKIDEXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", None),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    Harness {
        base,
        token: exposed,
        vault: vault.id,
        ssm: shared,
        state,
        client: reqwest::Client::new(),
        _dir: dir,
    }
}

fn account(ssm: &mut Ssm) {
    ssm.seed("/app/prod/db", "hunter2", Some("Banco de produção\nsó para o deploy"), "alias/prod", &[
        ("keyhold:comprometido", "2026-01-01T00:00:00Z"),
    ]);
    ssm.seed(
        "/app/prod/api",
        r#"{"keyhold":1,"fields":[{"label":"Usuário","kind":"text","value":"deploy","list":true},{"label":"Senha","kind":"password","value":"s3nha"}],"icon":null}"#,
        Some("API"),
        "alias/prod",
        &[("keyhold:formato", "json"), ("keyhold:trocar-ate", "2026-12-01")],
    );
    ssm.seed("/app/dev/.keep", "keyhold", Some("Pasta criada pelo Keyhold"), "alias/dev", &[]);
    ssm.seed("/config/app", r#"{"host":"db","password":"x"}"#, None, "alias/aws/ssm", &[]);
    ssm.seed("/secreto/chave", "nao-pode", Some("Segredo de outro time"), "alias/x", &[]);
    ssm.denied.push("/secreto".into());
}

// ── the tests ───────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn an_aws_vault_lists_what_the_role_can_read_in_the_shapes_a_local_vault_uses() {
    let h = start(account, None, None).await;

    let vaults = h.get("/vaults").await;
    let listed = &vaults.body["vaults"][0];
    assert_eq!(listed["kind"], "aws");
    assert_eq!(listed["aws"]["connection_id"], "CONN");
    assert_eq!(listed["aws"]["region"], "sa-east-1");

    let folders = h.get(&format!("/vaults/{}/folders", h.vault)).await;
    assert_eq!(folders.status, 200, "{}", folders.body);
    let paths: Vec<String> = folders.body["folders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| crate::aws::mapping::decode_id(f["id"].as_str().unwrap()).unwrap().2)
        .collect();
    // `/secreto` is only visible to DescribeParameters, so it is not a folder here (A6).
    assert_eq!(paths, vec!["/app", "/app/dev", "/app/prod", "/config"]);

    let records = h.records().await;
    let titles: Vec<&str> = records.iter().map(|r| r["title"].as_str().unwrap()).collect();
    assert_eq!(records.len(), 3, "no .keep, nothing denied: {titles:?}");
    assert!(!titles.contains(&"Segredo de outro time"));

    let db = records.iter().find(|r| r["id"] == h.id("/app/prod/db")).unwrap();
    assert_eq!(db["title"], "Banco de produção");
    assert_eq!(db["subtitle"], "");
    assert_eq!(db["kind"], "item");
    assert_eq!(db["version"], 1);
    assert_eq!(db["archived"], false);
    assert_eq!(db["compromised_at"], "2026-01-01T00:00:00Z");
    assert_eq!(db["folder_id"], h.folder("/app/prod"));

    let api = records.iter().find(|r| r["id"] == h.id("/app/prod/api")).unwrap();
    assert_eq!(api["subtitle"], "deploy");
    assert_eq!(api["rotation_due_at"], "2026-12-01");

    // An application's own JSON, untagged, is a plain value titled by its name.
    let config = h.get(&format!("/records/{}", h.id("/config/app"))).await;
    assert_eq!(config.body["record"]["payload"]["title"], "app");
    assert_eq!(config.body["record"]["payload"]["fields"][0]["label"], "Valor");
    assert_eq!(
        config.body["record"]["payload"]["fields"][0]["value"],
        r#"{"host":"db","password":"x"}"#
    );

    let opened = h.get(&format!("/records/{}", h.id("/app/prod/db"))).await;
    assert_eq!(opened.body["record"]["payload"]["notes"], "só para o deploy");
    assert_eq!(opened.body["record"]["payload"]["fields"][0]["value"], "hunter2");

    let denied = h.get(&format!("/records/{}", h.id("/secreto/chave"))).await;
    assert_eq!(denied.status, 403);
    assert_eq!(denied.body["aws"], "access_denied");

    let trash = h.get(&format!("/vaults/{}/trash", h.vault)).await;
    assert_eq!(trash.body["records"], json!([]));
}

#[tokio::test]
async fn a_record_is_written_with_a_suggested_name_and_the_key_of_its_folder() {
    let h = start(account, None, None).await;

    let made = h
        .post(
            &format!("/vaults/{}/records", h.vault),
            json!({
                "folder_id": h.folder("/app/prod"),
                "payload": {
                    "title": "Senha do Redis (produção)",
                    "notes": "rotacionar todo mês",
                    "fields": [
                        { "label": "Usuário", "kind": "text", "value": "default" },
                        { "label": "Senha", "kind": "password", "value": "r3d1s" }
                    ]
                }
            }),
        )
        .await;
    assert_eq!(made.status, 200, "{}", made.body);
    assert_eq!(made.body["record"]["id"], h.id("/app/prod/senha-do-redis-producao"));

    let stored = h.parameter("/app/prod/senha-do-redis-producao").unwrap();
    assert_eq!(stored.key_id, "alias/prod", "inherited from the folder (A11)");
    // M1: the title in the description, the notes in the encrypted value.
    assert_eq!(stored.description.as_deref(), Some("Senha do Redis (produção)"));
    assert!(stored.value.starts_with(r#"{"keyhold":1,"#));
    assert!(stored.value.contains(r#""notes":"rotacionar todo mês""#), "{}", stored.value);
    assert!(stored.tags.contains(&("keyhold:formato".into(), "json".into())));

    // In an empty folder, the marker's key is the folder's key.
    h.post(
        &format!("/vaults/{}/records", h.vault),
        json!({
            "folder_id": h.folder("/app/dev"),
            "name": "token",
            "payload": { "title": "Token", "fields": [{ "label": "Senha", "kind": "password", "value": "t" }] }
        }),
    )
    .await;
    let token = h.parameter("/app/dev/token").unwrap();
    assert_eq!(token.key_id, "alias/dev");
    assert_eq!(token.value, "t", "one password is the plain value (A8)");
    assert!(token.tags.is_empty());

    // Same name again: refused, not overwritten.
    let again = h
        .post(
            &format!("/vaults/{}/records", h.vault),
            json!({
                "folder_id": h.folder("/app/dev"),
                "name": "token",
                "payload": { "title": "Outro", "fields": [{ "label": "Senha", "kind": "password", "value": "x" }] }
            }),
        )
        .await;
    assert_eq!(again.status, 409);
    assert_eq!(again.body["aws"], "conflict");
    assert_eq!(h.parameter("/app/dev/token").unwrap().value, "t");

    // A title with a slash would be a folder.
    let slash = h
        .post(
            &format!("/vaults/{}/records", h.vault),
            json!({ "payload": { "title": "a/b", "fields": [] } }),
        )
        .await;
    assert_eq!(slash.status, 400);
}

#[tokio::test]
async fn saving_checks_the_version_and_switches_the_format_with_the_field_count() {
    let h = start(account, None, None).await;
    let id = h.id("/app/prod/db");

    let stale = h
        .put(
            &format!("/records/{id}"),
            json!({
                "expected_version": 7,
                "title": "Banco", "fields": [{ "label": "Senha", "kind": "password", "value": "novo" }]
            }),
        )
        .await;
    assert_eq!(stale.status, 409, "{}", stale.body);
    assert_eq!(stale.body["aws"], "conflict");
    assert_eq!(h.parameter("/app/prod/db").unwrap().value, "hunter2", "nothing overwritten (A12)");

    let saved = h
        .put(
            &format!("/records/{id}"),
            json!({
                "expected_version": 1,
                "title": "Banco", "notes": "",
                "fields": [
                    { "label": "Usuário", "kind": "text", "value": "admin" },
                    { "label": "Senha", "kind": "password", "value": "novo" }
                ]
            }),
        )
        .await;
    assert_eq!(saved.status, 200, "{}", saved.body);
    assert_eq!(saved.body["record"]["version"], 2);

    let stored = h.parameter("/app/prod/db").unwrap();
    assert!(stored.value.starts_with(r#"{"keyhold":1,"#));
    assert!(stored.tags.contains(&("keyhold:formato".into(), "json".into())));
    assert_eq!(stored.key_id, "alias/prod", "an overwrite keeps the key it had");
    assert!(stored.tags.iter().any(|(k, _)| k == "keyhold:comprometido"), "other tags stay");

    // Back to one password: back to the plain value, and the tag goes.
    let header = h
        .client
        .put(format!("{}/api/records/{id}", h.base))
        .header(TOKEN_HEADER, &h.token)
        .header("x-keyhold-expected-version", "2")
        .header("content-type", "application/json")
        .body(json!({ "title": "Banco", "fields": [{ "label": "Senha", "kind": "password", "value": "so-isso" }] }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(header.status(), 200);

    let stored = h.parameter("/app/prod/db").unwrap();
    assert_eq!(stored.value, "so-isso");
    assert!(!stored.tags.iter().any(|(k, _)| k == "keyhold:formato"));

    // History: the versions before the current one, newest first; restoring writes forward.
    let versions = h.get(&format!("/records/{id}/versions")).await;
    let listed: Vec<i64> = versions.body["versions"].as_array().unwrap().iter().map(|v| v["version"].as_i64().unwrap()).collect();
    assert_eq!(listed, vec![2, 1]);

    let old = h.get(&format!("/records/{id}/versions/1")).await;
    assert_eq!(old.body["payload"]["fields"][0]["value"], "hunter2");

    // A restore checked against a version that is no longer current is refused, like a save.
    let stale = h
        .post(&format!("/records/{id}/versions/1/restore"), json!({ "expected_version": 2 }))
        .await;
    assert_eq!(stale.status, 409, "{}", stale.body);

    let restored = h.post(&format!("/records/{id}/versions/1/restore"), json!({})).await;
    assert_eq!(restored.status, 200, "{}", restored.body);
    assert_eq!(restored.body["record"]["version"], 4);
    assert!(restored.body.get("warning").is_none(), "{}", restored.body);

    // Version 1 had its notes in the description (the old way). Restored, they move into the value.
    let back = h.parameter("/app/prod/db").unwrap();
    assert!(back.value.contains("hunter2") && back.value.contains("só para o deploy"), "{}", back.value);
    assert_eq!(back.description.as_deref(), Some("Banco de produção"));
    assert!(back.tags.contains(&("keyhold:formato".into(), "json".into())));
    let opened = h.get(&format!("/records/{id}")).await;
    assert_eq!(opened.body["record"]["payload"]["notes"], "só para o deploy");
    assert_eq!(opened.body["record"]["payload"]["fields"][0]["value"], "hunter2");
}

#[tokio::test]
async fn folders_are_markers_and_a_full_one_is_not_deleted() {
    let h = start(account, None, None).await;

    let made = h
        .post(&format!("/vaults/{}/folders", h.vault), json!({ "name": "novo", "parent_id": h.folder("/app") }))
        .await;
    assert_eq!(made.status, 200, "{}", made.body);
    assert_eq!(made.body["folder"]["id"], h.folder("/app/novo"));
    assert_eq!(made.body["folder"]["depth"], 1);
    assert!(h.parameter("/app/novo/.keep").is_some());

    let full = h.call("DELETE", &format!("/folders/{}", h.folder("/app/prod")), None).await;
    assert_eq!(full.status, 409);
    assert!(h.parameter("/app/prod/db").is_some());

    let empty = h.call("DELETE", &format!("/folders/{}", h.folder("/app/novo")), None).await;
    assert_eq!(empty.status, 200, "{}", empty.body);
    assert!(h.parameter("/app/novo/.keep").is_none());

    let moved = h.post(&format!("/folders/{}/move", h.folder("/app/dev")), json!({ "parent_id": null })).await;
    assert_eq!(moved.status, 400, "moving an AWS folder elsewhere is still refused");
}

#[tokio::test]
async fn a_folder_has_a_free_name_and_a_clean_path() {
    let h = start(account, None, None).await;

    let made = h
        .post(&format!("/vaults/{}/folders", h.vault), json!({ "name": "Bancos (PJ)", "parent_id": h.folder("/app") }))
        .await;
    assert_eq!(made.status, 200, "{}", made.body);
    assert_eq!(made.body["folder"]["id"], h.folder("/app/bancos-pj"));
    assert_eq!(made.body["folder"]["name"], "Bancos (PJ)");
    assert_eq!(h.parameter("/app/bancos-pj/.keep").unwrap().description.as_deref(), Some("Bancos (PJ)"));

    let typed = h
        .post(&format!("/vaults/{}/folders", h.vault), json!({ "name": "Cartões", "segment": "Cards", "parent_id": h.folder("/app") }))
        .await;
    assert_eq!(typed.status, 200, "{}", typed.body);
    assert!(h.parameter("/app/Cards/.keep").is_some(), "a segment typed by hand is kept as typed");

    let bad = h
        .post(&format!("/vaults/{}/folders", h.vault), json!({ "name": "X", "segment": "com espaço", "parent_id": null }))
        .await;
    assert_eq!(bad.status, 400);

    let tree = h.get(&format!("/vaults/{}/folders", h.vault)).await;
    let names: Vec<&str> = tree.body["folders"].as_array().unwrap().iter().filter_map(|f| f["name"].as_str()).collect();
    assert!(names.contains(&"Bancos (PJ)"));
    assert!(names.contains(&"dev"), "the old marker sentence is never a name");
}

#[tokio::test]
async fn renaming_a_folder_changes_its_name_and_moves_its_path_only_when_asked() {
    let h = start(account, None, None).await;

    let named = h.put(&format!("/folders/{}", h.folder("/app/prod")), json!({ "name": "Produção" })).await;
    assert_eq!(named.status, 200, "{}", named.body);
    assert_eq!(named.body["folder"]["id"], h.folder("/app/prod"));
    assert_eq!(h.parameter("/app/prod/.keep").unwrap().description.as_deref(), Some("Produção"));
    assert_eq!(h.parameter("/app/prod/.keep").unwrap().key_id, "alias/prod", "a new marker takes the folder's key");
    assert!(h.parameter("/app/prod/db").is_some(), "a name alone moves nothing");

    let moved = h
        .put(&format!("/folders/{}", h.folder("/app/prod")), json!({ "name": "Produção", "segment": "producao" }))
        .await;
    assert_eq!(moved.status, 200, "{}", moved.body);
    assert_eq!(moved.body["folder"]["id"], h.folder("/app/producao"));
    assert!(h.parameter("/app/prod/db").is_none());
    assert_eq!(h.parameter("/app/producao/db").unwrap().value, "hunter2");
    assert_eq!(h.parameter("/app/producao/api").unwrap().key_id, "alias/prod");
    assert_eq!(h.parameter("/app/producao/.keep").unwrap().description.as_deref(), Some("Produção"));
    assert!(h.parameter("/app/prod/.keep").is_none());

    let taken = h
        .put(&format!("/folders/{}", h.folder("/app/producao")), json!({ "name": "Dev", "segment": "dev" }))
        .await;
    assert_eq!(taken.status, 409, "a path already there is refused");

    let slash = h.put(&format!("/folders/{}", h.folder("/app/dev")), json!({ "name": "a/b" })).await;
    assert_eq!(slash.status, 400);
}

#[tokio::test]
async fn a_folder_with_a_string_parameter_keeps_its_path_and_says_why() {
    let h = start(
        |ssm| {
            account(ssm);
            ssm.seed("/app/dev/legado", "texto", Some("Legado"), "", &[]);
            ssm.parameters.get_mut("/app/dev/legado").unwrap().kind = "String".into();
        },
        None,
        None,
    )
    .await;

    let renamed = h
        .put(&format!("/folders/{}", h.folder("/app/dev")), json!({ "name": "Desenvolvimento", "segment": "desenvolvimento" }))
        .await;
    assert_eq!(renamed.status, 200, "{}", renamed.body);
    assert_eq!(renamed.body["folder"]["id"], h.folder("/app/dev"));
    assert!(renamed.body["warning"].as_str().unwrap().contains("legado"));
    assert!(h.parameter("/app/dev/legado").is_some());
    assert_eq!(h.parameter("/app/dev/.keep").unwrap().description.as_deref(), Some("Desenvolvimento"));
}

#[tokio::test]
async fn a_record_is_renamed_in_aws_only_with_a_new_name() {
    let h = start(account, None, None).await;
    let id = h.id("/app/prod/db");

    let opened = h.get(&format!("/records/{id}")).await;
    assert_eq!(opened.body["record"]["parameter"], "/app/prod/db");

    let renamed = h
        .post(&format!("/records/{id}/move"), json!({ "folder_id": h.folder("/app/prod"), "name": "banco-principal" }))
        .await;
    assert_eq!(renamed.status, 200, "{}", renamed.body);
    assert_eq!(renamed.body["id"], h.id("/app/prod/banco-principal"));
    assert!(h.parameter("/app/prod/db").is_none());

    let bad = h
        .post(&format!("/records/{}/move", h.id("/app/prod/banco-principal")), json!({ "folder_id": h.folder("/app/prod"), "name": "com espaço" }))
        .await;
    assert_eq!(bad.status, 400);
}

#[tokio::test]
async fn moving_renames_with_tags_and_deleting_is_for_good() {
    let h = start(account, None, None).await;
    let id = h.id("/app/prod/db");

    let starred = h.post(&format!("/records/{id}/favourite"), json!({ "on": true })).await;
    assert_eq!(starred.status, 200);

    let moved = h.post(&format!("/records/{id}/move"), json!({ "folder_id": h.folder("/app/dev") })).await;
    assert_eq!(moved.status, 200, "{}", moved.body);
    assert_eq!(moved.body["id"], h.id("/app/dev/db"));
    assert_eq!(moved.body["history_lost"], true, "a move starts the history over (M3)");
    assert!(moved.body.get("warning").is_none());
    assert!(h.parameter("/app/prod/db").is_none());

    let there = h.parameter("/app/dev/db").unwrap();
    assert_eq!(there.value, "hunter2");
    assert_eq!(there.key_id, "alias/prod", "the key goes with it");
    assert!(there.tags.iter().any(|(k, _)| k == "keyhold:comprometido"));

    let records = h.records().await;
    let row = records.iter().find(|r| r["id"] == h.id("/app/dev/db")).unwrap();
    assert_eq!(row["favourite"], true, "the favourite follows the rename");

    let elsewhere = h
        .post(&format!("/records/{}/move", h.id("/app/dev/db")), json!({ "vault_id": "OUTRO" }))
        .await;
    assert_eq!(elsewhere.status, 400);

    let rotation = h
        .post(&format!("/records/{}/rotation", h.id("/app/dev/db")), json!({ "due_at": "2027-01-31" }))
        .await;
    assert_eq!(rotation.status, 200);
    assert!(h.parameter("/app/dev/db").unwrap().tags.contains(&("keyhold:trocar-ate".into(), "2027-01-31".into())));

    let clean = h.post(&format!("/records/{}/compromised", h.id("/app/dev/db")), json!({ "on": false })).await;
    assert_eq!(clean.status, 200);
    assert!(!h.parameter("/app/dev/db").unwrap().tags.iter().any(|(k, _)| k == "keyhold:comprometido"));

    let gone = h.post(&format!("/records/{}/archive", h.id("/app/dev/db")), json!({ "on": true })).await;
    assert_eq!(gone.status, 200, "{}", gone.body);
    assert!(h.parameter("/app/dev/db").is_none(), "no trash in an AWS vault (A16)");

    let back = h.post(&format!("/records/{}/archive", h.id("/app/dev/db")), json!({ "on": false })).await;
    assert_eq!(back.status, 400);
}

#[tokio::test]
async fn without_a_session_the_vault_answers_424_and_says_so() {
    let h = start(account, None, None).await;

    // As after a restart: the credentials and the login's token are gone.
    h.state.aws.forget();

    let reply = h.get(&format!("/vaults/{}/records", h.vault)).await;
    assert_eq!(reply.status, 424, "{}", reply.body);
    assert_eq!(reply.body["aws"], "session_expired");
    assert!(!reply.body["error"].as_str().unwrap().contains('—'));

    let overview = h.get("/aws").await;
    assert_eq!(overview.body["logins"][0]["connected"], false);
    assert_eq!(overview.body["connections"][0]["used_by"], json!([h.vault]));

    // A connection a vault stands on is not removed from under it.
    let removal = h.call("DELETE", "/aws/connections/CONN", None).await;
    assert_eq!(removal.status, 409);
    let login_removal = h.call("DELETE", "/aws/logins/LOGIN", None).await;
    assert_eq!(login_removal.status, 409);
}

#[tokio::test]
async fn one_account_is_one_connection() {
    let h = start(account, None, None).await;

    let twice = h
        .post(
            "/aws/connections",
            json!({ "login_id": "LOGIN", "account_id": "123456789012", "role": "Admin" }),
        )
        .await;
    assert_eq!(twice.status, 409);

    let other = h
        .post(
            "/aws/connections",
            json!({ "login_id": "LOGIN", "account_id": "210987654321", "account_name": "Dev", "role": "Admin" }),
        )
        .await;
    assert_eq!(other.status, 200, "{}", other.body);
    assert_eq!(other.body["connection"]["title"], "210987654321 / Admin");

    let id = other.body["connection"]["id"].as_str().unwrap().to_string();
    let edited = h
        .put(&format!("/aws/connections/{id}"), json!({ "role": "Leitura", "title": "", "note": "nota" }))
        .await;
    assert_eq!(edited.body["connection"]["title"], "210987654321 / Leitura");
    assert_eq!(edited.body["connection"]["note"], "nota");

    let removed = h.call("DELETE", &format!("/aws/connections/{id}"), None).await;
    assert_eq!(removed.status, 200);

    let aws_vault_without_connection = h
        .post(
            "/vaults",
            json!({ "name": "X", "kind": "aws", "aws": { "connection_id": id, "region": "sa-east-1" } }),
        )
        .await;
    assert_eq!(aws_vault_without_connection.status, 400);
}

#[tokio::test]
async fn attachments_go_to_the_bucket_under_the_parameter_name_with_its_key() {
    let s3 = FakeAws::start(vec![
        Canned::empty(200).with_header("etag", "\"abc\""),
        Canned::xml(
            200,
            "<ListBucketResult><IsTruncated>false</IsTruncated>\
             <Contents><Key>app/prod/db/chave.pem</Key><Size>5</Size><LastModified>2026-01-01T00:00:00.000Z</LastModified></Contents>\
             <Contents><Key>app/prod/db/sub/outro</Key><Size>1</Size><LastModified>2026-01-01T00:00:00.000Z</LastModified></Contents>\
             </ListBucketResult>",
        ),
        Canned::xml(403, "<Error><Code>AccessDenied</Code><Message>no</Message></Error>"),
    ])
    .await;
    let h = start(account, Some("keyhold-producao".into()), Some(s3.base.clone())).await;
    let id = h.id("/app/prod/db");

    let added = h
        .post(
            &format!("/records/{id}/attachments"),
            json!({ "name": "chave.pem", "mime": "application/x-pem-file", "data": crate::encoding::base64url(b"chave") }),
        )
        .await;
    assert_eq!(added.status, 200, "{}", added.body);
    assert_eq!(added.body["attachment"]["id"], h.attachment("app/prod/db/chave.pem"));

    let listed = h.get(&format!("/records/{id}/attachments")).await;
    let names: Vec<&str> = listed.body["attachments"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["chave.pem"]);

    let denied = h.get(&format!("/attachments/{}", h.attachment("app/prod/db/chave.pem"))).await;
    assert_eq!(denied.status, 403);
    assert!(denied.body["error"].as_str().unwrap().contains("keyhold-producao"));

    let requests = s3.requests();
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(requests[0].uri, "/keyhold-producao/app/prod/db/chave.pem");
    assert_eq!(requests[0].header("x-amz-server-side-encryption"), Some("aws:kms"));
    assert_eq!(requests[0].header("x-amz-server-side-encryption-aws-kms-key-id"), Some("alias/prod"));
    for request in &requests {
        assert_eq!(request.header("x-amz-expected-bucket-owner"), Some("123456789012"), "M5");
    }
}

#[tokio::test]
async fn a_session_about_to_end_is_renewed_with_its_refresh_token() {
    let sso = FakeAws::start(vec![
        Canned::json(200, json!({
            "clientId": "client", "clientSecret": "client-secret", "clientSecretExpiresAt": 4_000_000_000u64
        })),
        Canned::json(200, json!({
            "deviceCode": "device", "userCode": "ABCD-EFGH",
            "verificationUri": "https://device.sso.sa-east-1.amazonaws.com/",
            "verificationUriComplete": "https://device.sso.sa-east-1.amazonaws.com/?user_code=ABCD-EFGH",
            "expiresIn": 600, "interval": 5
        })),
        Canned::json(200, json!({ "accessToken": "access", "expiresIn": 300, "refreshToken": "refresh" })),
        Canned::json(200, json!({ "accessToken": "access-2", "expiresIn": 3600 })),
    ])
    .await;
    let h = start(account, None, None).await;
    h.state.aws.set_endpoints(Endpoints {
        oidc: Some(sso.base.clone()),
        portal: Some(sso.base.clone()),
        ..Endpoints::default()
    });

    let started = h
        .post("/aws/logins", json!({ "start_url": "https://outra.awsapps.com/start", "region": "us-east-1", "label": "Outra" }))
        .await;
    let id = started.body["login"]["id"].as_str().unwrap().to_string();
    let done = h.post(&format!("/aws/logins/{id}/poll"), json!({})).await;
    assert_eq!(done.body["state"], "done", "{}", done.body);

    // Five minutes left: reading the overview trades the refresh token for a new hour.
    let overview = h.get("/aws").await;
    let login = overview.body["logins"].as_array().unwrap().iter().find(|l| l["id"] == id.as_str()).cloned().unwrap();
    assert_eq!(login["renews"], true);
    assert!(login["expires_at"].as_u64().unwrap() > crate::aws::now_unix() + 3000, "{login}");

    let requests = sso.requests();
    assert_eq!(requests[0].json()["grantTypes"][1], "refresh_token");
    let last = requests.last().unwrap().json();
    assert_eq!(last["grantType"], "refresh_token");
    assert_eq!(last["refreshToken"], "refresh");
}

#[tokio::test]
async fn a_device_login_is_started_polled_kept_in_memory_and_revoked() {
    let sso = FakeAws::start(vec![
        Canned::json(200, json!({
            "clientId": "client", "clientSecret": "client-secret", "clientSecretExpiresAt": 4_000_000_000u64
        })),
        Canned::json(200, json!({
            "deviceCode": "device", "userCode": "ABCD-EFGH",
            "verificationUri": "https://device.sso.sa-east-1.amazonaws.com/",
            "verificationUriComplete": "https://device.sso.sa-east-1.amazonaws.com/?user_code=ABCD-EFGH",
            "expiresIn": 600, "interval": 5
        })),
        Canned::json(400, json!({ "error": "authorization_pending" })),
        Canned::json(200, json!({ "accessToken": "access", "expiresIn": 3600, "refreshToken": "refresh" })),
        Canned::empty(200),
    ])
    .await;
    let h = start(account, None, None).await;
    h.state.aws.set_endpoints(Endpoints {
        oidc: Some(sso.base.clone()),
        portal: Some(sso.base.clone()),
        ..Endpoints::default()
    });

    let started = h
        .post(
            "/aws/logins",
            json!({ "start_url": "https://outra.awsapps.com/start", "region": "us-east-1", "label": "Outra" }),
        )
        .await;
    assert_eq!(started.status, 200, "{}", started.body);
    assert_eq!(started.body["device"]["user_code"], "ABCD-EFGH");
    assert_eq!(started.body["login"]["connected"], false);
    let id = started.body["login"]["id"].as_str().unwrap().to_string();

    // Not saved until somebody approves it.
    assert_eq!(h.get("/aws").await.body["logins"].as_array().unwrap().len(), 1);

    let pending = h.post(&format!("/aws/logins/{id}/poll"), json!({})).await;
    assert_eq!(pending.body["state"], "pending");

    let done = h.post(&format!("/aws/logins/{id}/poll"), json!({})).await;
    assert_eq!(done.body["state"], "done", "{}", done.body);

    let find = |body: &Value| {
        body["logins"].as_array().unwrap().iter().find(|l| l["id"] == id.as_str()).cloned().unwrap()
    };

    let login = find(&h.get("/aws").await.body);
    assert_eq!(login["connected"], true);
    assert_eq!(login["label"], "Outra");

    // The token is not in the file, and the login is sealed.
    {
        let store = h.state.store.lock().unwrap();
        let raw: String = store
            .conn()
            .query_row("SELECT value FROM settings WHERE key = 'aws_config'", [], |r| r.get(0))
            .unwrap();
        assert!(!raw.contains("access") && !raw.contains("outra.awsapps"));
    }

    let revoked = h.post(&format!("/aws/logins/{id}/revoke"), json!({})).await;
    assert_eq!(revoked.body["revoked"], true);
    let login = find(&h.get("/aws").await.body);
    assert_eq!(login["connected"], false, "revoked, still listed (A4)");

    let requests = sso.requests();
    assert_eq!(requests.last().unwrap().uri, "/logout");
    assert_eq!(requests.last().unwrap().header("x-amz-sso_bearer_token"), Some("access"));
}

// ── the security review's findings ──────────────────────────────────────────────────────────────

fn save_body(expected: i64, fields: Value) -> Value {
    json!({ "expected_version": expected, "title": "Banco", "fields": fields })
}

fn one_password(value: &str) -> Value {
    json!([{ "label": "Senha", "kind": "password", "value": value }])
}

fn two_fields() -> Value {
    json!([
        { "label": "Usuário", "kind": "text", "value": "admin" },
        { "label": "Senha", "kind": "password", "value": "novo" }
    ])
}

#[tokio::test]
async fn without_describe_the_key_comes_from_history_and_without_both_nothing_is_written() {
    let h = start(account, None, None).await;
    let id = h.id("/app/prod/db");
    h.with_ssm(|ssm| ssm.describe_denied = true);

    // Describe refused: the newest history entry still says which key the secret is under.
    let saved = h.put(&format!("/records/{id}"), save_body(1, one_password("novo"))).await;
    assert_eq!(saved.status, 200, "{}", saved.body);
    assert_eq!(h.parameter("/app/prod/db").unwrap().key_id, "alias/prod", "never downgraded (H1)");
    let put = h.with_ssm(|ssm| ssm.puts.last().cloned().unwrap());
    assert_eq!(put["KeyId"], "alias/prod");

    // History refused too: nothing is sent at all.
    h.with_ssm(|ssm| ssm.history_denied = true);
    let puts_before = h.with_ssm(|ssm| ssm.puts.len());

    let refused = h.put(&format!("/records/{id}"), save_body(2, one_password("outro"))).await;
    assert_eq!(refused.status, 403, "{}", refused.body);
    assert_eq!(refused.body["aws"], "access_denied");
    let text = refused.body["error"].as_str().unwrap();
    assert!(text.contains("ssm:DescribeParameters") && !text.contains('—'), "{text}");

    let moved = h.post(&format!("/records/{id}/move"), json!({ "folder_id": h.folder("/app/dev") })).await;
    assert_eq!(moved.status, 403, "a move without metadata is aborted: {}", moved.body);
    assert!(h.parameter("/app/prod/db").is_some() && h.parameter("/app/dev/db").is_none());

    // A new record cannot inherit a key from a listing whose keys could not be read.
    let created = h
        .post(
            &format!("/vaults/{}/records", h.vault),
            json!({ "folder_id": h.folder("/app/prod"), "payload": { "title": "Novo", "fields": one_password("x") } }),
        )
        .await;
    assert_eq!(created.status, 403, "{}", created.body);
    let folder = h
        .post(&format!("/vaults/{}/folders", h.vault), json!({ "name": "nova", "parent_id": h.folder("/app/prod") }))
        .await;
    assert_eq!(folder.status, 403, "{}", folder.body);

    assert_eq!(h.with_ssm(|ssm| ssm.puts.len()), puts_before, "not one PutParameter");
    assert_eq!(h.parameter("/app/prod/db").unwrap().value, "novo");

    // Reading still works, titled by name: only writing needs the key.
    let records = h.records().await;
    assert!(records.iter().any(|r| r["title"] == "db"));
}

#[tokio::test]
async fn a_parameter_without_a_leading_slash_is_described_by_its_exact_name() {
    let h = start(
        |ssm| ssm.seed("solto", "hunter2", Some("Solto"), "alias/solto", &[]),
        None,
        None,
    )
    .await;

    let saved = h.put(&format!("/records/{}", h.id("solto")), save_body(1, one_password("novo"))).await;
    assert_eq!(saved.status, 200, "{}", saved.body);
    assert_eq!(h.parameter("solto").unwrap().key_id, "alias/solto");

    let describe = h.with_ssm(|ssm| ssm.calls.iter().filter(|(op, _)| op == "DescribeParameters").count());
    assert!(describe >= 1);
}

#[tokio::test]
async fn notes_are_kept_in_the_value_and_old_ones_move_there_on_the_next_save() {
    let h = start(account, None, None).await;
    let id = h.id("/app/prod/db");

    // Written the old way: notes after the title in the description. Still read.
    let opened = h.get(&format!("/records/{id}")).await;
    assert_eq!(opened.body["record"]["payload"]["notes"], "só para o deploy");

    let saved = h
        .put(
            &format!("/records/{id}"),
            json!({
                "expected_version": 1, "title": "Banco de produção", "notes": "só para o deploy",
                "fields": one_password("hunter2")
            }),
        )
        .await;
    assert_eq!(saved.status, 200, "{}", saved.body);

    let stored = h.parameter("/app/prod/db").unwrap();
    assert_eq!(stored.description.as_deref(), Some("Banco de produção"), "the title alone");
    assert!(stored.value.contains("só para o deploy"));
    assert!(stored.tags.contains(&("keyhold:formato".into(), "json".into())));

    let again = h.get(&format!("/records/{id}")).await;
    assert_eq!(again.body["record"]["payload"]["notes"], "só para o deploy");
    assert_eq!(again.body["record"]["payload"]["fields"][0]["value"], "hunter2");
}

#[tokio::test]
async fn a_write_that_lands_after_somebody_elses_still_succeeds_and_says_so() {
    let h = start(account, None, None).await;
    let id = h.id("/app/prod/db");
    h.with_ssm(|ssm| ssm.race.push("/app/prod/db".into()));

    let saved = h.put(&format!("/records/{id}"), save_body(1, one_password("meu"))).await;
    assert_eq!(saved.status, 200, "{}", saved.body);
    assert_eq!(saved.body["record"]["version"], 3);
    let warning = saved.body["warning"].as_str().expect("a warning (M2)");
    assert!(warning.contains("versão 3") && warning.contains("histórico") && !warning.contains('—'), "{warning}");

    // Metadata and tags are read before the version check, so only GetParameter sits between
    // the check and the write.
    let calls = h.with_ssm(|ssm| ssm.calls.clone());
    let ops: Vec<&str> = calls.iter().map(|(op, _)| op.as_str()).collect();
    let put = ops.iter().rposition(|op| *op == "PutParameter").unwrap();
    assert_eq!(ops[put - 1], "GetParameter", "{ops:?}");

    // Without a race there is nothing to warn about.
    let calm = h.put(&format!("/records/{id}"), save_body(3, one_password("de novo"))).await;
    assert_eq!(calm.status, 200);
    assert!(calm.body.get("warning").is_none(), "{}", calm.body);
}

#[tokio::test]
async fn the_format_tag_goes_on_before_the_put_and_comes_off_after_it() {
    let h = start(account, None, None).await;
    let id = h.id("/app/prod/db");

    // To JSON with the put refused: the tag went first, the value did not change, and it still
    // reads as the plain value it is.
    h.with_ssm(|ssm| ssm.put_denied.push("/app/prod/db".into()));
    let refused = h.put(&format!("/records/{id}"), save_body(1, two_fields())).await;
    assert_eq!(refused.status, 403, "{}", refused.body);
    let stored = h.parameter("/app/prod/db").unwrap();
    assert_eq!(stored.value, "hunter2");
    assert!(stored.tags.contains(&("keyhold:formato".into(), "json".into())));
    let opened = h.get(&format!("/records/{id}")).await;
    assert_eq!(opened.body["record"]["payload"]["fields"][0]["value"], "hunter2", "fresh after the error");

    h.with_ssm(|ssm| ssm.put_denied.clear());
    let calls_before = h.with_ssm(|ssm| ssm.calls.len());
    let to_json = h.put(&format!("/records/{id}"), save_body(1, two_fields())).await;
    assert_eq!(to_json.status, 200, "{}", to_json.body);
    let calls = h.with_ssm(|ssm| ssm.calls[calls_before..].to_vec());
    let ops: Vec<&str> = calls.iter().map(|(op, _)| op.as_str()).collect();
    assert!(!ops.contains(&"AddTagsToResource"), "already tagged: {ops:?}");

    // Back to plain with the untag refused: saved, with a warning, and it reads as plain.
    h.with_ssm(|ssm| ssm.untag_denied = true);
    let to_plain = h.put(&format!("/records/{id}"), save_body(2, one_password("so-isso"))).await;
    assert_eq!(to_plain.status, 200, "{}", to_plain.body);
    assert!(to_plain.body["warning"].as_str().unwrap().contains("keyhold:formato"), "{}", to_plain.body);
    let opened = h.get(&format!("/records/{id}")).await;
    assert_eq!(opened.body["record"]["payload"]["fields"][0]["value"], "so-isso");
    assert_eq!(opened.body["record"]["payload"]["fields"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_move_that_cannot_delete_the_old_name_succeeds_with_both_copies_and_says_so() {
    let h = start(account, None, None).await;
    let id = h.id("/app/prod/db");
    h.post(&format!("/records/{id}/favourite"), json!({ "on": true })).await;
    h.with_ssm(|ssm| ssm.delete_denied.push("/app/prod/db".into()));

    let moved = h.post(&format!("/records/{id}/move"), json!({ "folder_id": h.folder("/app/dev") })).await;
    assert_eq!(moved.status, 200, "{}", moved.body);
    assert_eq!(moved.body["moved"], true);
    assert_eq!(moved.body["id"], h.id("/app/dev/db"));
    assert_eq!(moved.body["history_lost"], true);
    let warning = moved.body["warning"].as_str().unwrap();
    assert!(warning.contains("duas cópias") && !warning.contains('—'), "{warning}");
    assert!(h.parameter("/app/prod/db").is_some() && h.parameter("/app/dev/db").is_some());
    assert_eq!(h.parameter("/app/dev/db").unwrap().description.as_deref(), Some("Banco de produção\nsó para o deploy"));

    // Both copies show up right away (the caches were cleared), and the favourite went with the move.
    let records = h.records().await;
    let new = records.iter().find(|r| r["id"] == h.id("/app/dev/db")).unwrap();
    let old = records.iter().find(|r| r["id"] == h.id("/app/prod/db")).unwrap();
    assert_eq!(new["favourite"], true);
    assert_eq!(old["favourite"], false);
}

#[tokio::test]
async fn a_record_with_attachments_is_not_moved_and_its_files_go_when_it_is_deleted() {
    let listing = "<ListBucketResult><IsTruncated>false</IsTruncated>\
         <Contents><Key>app/prod/db/chave.pem</Key><Size>5</Size><LastModified>2026-01-01T00:00:00.000Z</LastModified></Contents>\
         <Contents><Key>app/prod/db/sub/outro</Key><Size>1</Size><LastModified>2026-01-01T00:00:00.000Z</LastModified></Contents>\
         </ListBucketResult>";
    let s3 = FakeAws::start(vec![
        Canned::xml(200, listing),
        Canned::xml(200, listing),
        Canned::empty(204),
    ])
    .await;
    let h = start(account, Some("keyhold-producao".into()), Some(s3.base.clone())).await;
    let id = h.id("/app/prod/db");

    let moved = h.post(&format!("/records/{id}/move"), json!({ "folder_id": h.folder("/app/dev") })).await;
    assert_eq!(moved.status, 409, "{}", moved.body);
    assert!(moved.body["error"].as_str().unwrap().contains("anexo"), "{}", moved.body);
    assert!(h.parameter("/app/prod/db").is_some() && h.parameter("/app/dev/db").is_none());

    let deleted = h.post(&format!("/records/{id}/archive"), json!({ "on": true })).await;
    assert_eq!(deleted.status, 200, "{}", deleted.body);
    assert!(h.parameter("/app/prod/db").is_none());

    let requests = s3.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].method, "DELETE");
    assert_eq!(requests[2].uri, "/keyhold-producao/app/prod/db/chave.pem", "only its own file, not a nested record's");
}

#[tokio::test]
async fn a_record_whose_bucket_cannot_be_listed_is_not_deleted() {
    let s3 = FakeAws::start(vec![Canned::xml(403, "<Error><Code>AccessDenied</Code><Message>no</Message></Error>")]).await;
    let h = start(account, Some("keyhold-producao".into()), Some(s3.base.clone())).await;

    let refused = h.post(&format!("/records/{}/archive", h.id("/app/prod/db")), json!({ "on": true })).await;
    assert_eq!(refused.status, 403, "{}", refused.body);
    let text = refused.body["error"].as_str().unwrap();
    assert!(text.contains("não apaguei") && text.contains("keyhold-producao") && !text.contains('—'), "{text}");
    assert!(h.parameter("/app/prod/db").is_some());

    // Without a bucket there is nothing to leave behind, and the record goes as before.
    let plain = start(account, None, None).await;
    let gone = plain.post(&format!("/records/{}/archive", plain.id("/app/prod/db")), json!({ "on": true })).await;
    assert_eq!(gone.status, 200);
}

#[tokio::test]
async fn a_plain_text_parameter_is_flagged_and_never_overwritten() {
    let h = start(
        |ssm| {
            account(ssm);
            ssm.seed("/config/url", "https://exemplo", Some("URL"), "", &[]);
            ssm.parameters.get_mut("/config/url").unwrap().kind = "String".into();
        },
        None,
        None,
    )
    .await;
    let id = h.id("/config/url");

    let records = h.records().await;
    let row = records.iter().find(|r| r["id"] == id).unwrap();
    assert_eq!(row["plaintext"], true);
    let db = records.iter().find(|r| r["id"] == h.id("/app/prod/db")).unwrap();
    assert_eq!(db["plaintext"], false);

    let opened = h.get(&format!("/records/{id}")).await;
    assert_eq!(opened.body["record"]["plaintext"], true);

    let refused = h.put(&format!("/records/{id}"), save_body(1, one_password("x"))).await;
    assert_eq!(refused.status, 400, "{}", refused.body);
    assert!(refused.body["error"].as_str().unwrap().contains("SecureString"));
    let moved = h.post(&format!("/records/{id}/move"), json!({ "folder_id": h.folder("/app/dev") })).await;
    assert_eq!(moved.status, 400);
    assert_eq!(h.parameter("/config/url").unwrap().value, "https://exemplo");
    assert!(h.with_ssm(|ssm| ssm.puts.is_empty()));
}

#[tokio::test]
async fn an_id_of_one_kind_is_not_accepted_where_another_is_expected() {
    let h = start(account, None, None).await;

    // A folder id on a record route, and a record id on a folder route.
    let folder_as_record = h.get(&format!("/records/{}", h.folder("/app/prod/db"))).await;
    assert_eq!(folder_as_record.status, 404);
    let record_as_folder = h.call("DELETE", &format!("/folders/{}", h.id("/app/dev")), None).await;
    assert_eq!(record_as_folder.status, 404);

    // An attachment id that is not one file directly under a record.
    for key in ["/app/prod/db/x", "app/prod/db/../x", "app"] {
        let bad = h.get(&format!("/attachments/{}", h.attachment(key))).await;
        assert_eq!(bad.status, 404, "{key}");
    }
}

#[tokio::test]
async fn editing_or_archiving_a_vault_forgets_what_was_cached_for_it() {
    let h = start(account, None, None).await;
    h.records().await;
    let cached = |h: &Harness| h.state.aws.with(&h.state.lock, |i| i.snapshots.contains_key(&h.vault)).unwrap();
    assert!(cached(&h));

    let edited = h
        .put(
            &format!("/vaults/{}", h.vault),
            json!({ "name": "Produção", "kind": "aws", "aws": { "connection_id": "CONN", "region": "sa-east-1" } }),
        )
        .await;
    assert_eq!(edited.status, 200, "{}", edited.body);
    assert!(!cached(&h));

    h.records().await;
    let archived = h.post(&format!("/vaults/{}/archive", h.vault), json!({ "on": true })).await;
    assert_eq!(archived.status, 200);
    assert!(!cached(&h));
}

#[tokio::test]
async fn an_auto_lock_with_nobody_asking_still_forgets_the_aws_session() {
    let h = start(account, None, None).await;
    h.records().await;
    assert!(h.state.aws.held() > 0);

    super::start_sweeper(&h.state, std::time::Duration::from_millis(20));
    h.state.lock.lock();

    for _ in 0..100 {
        if h.state.aws.held() == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the sweeper left {} things in memory after the lock", h.state.aws.held());
}

#[tokio::test]
async fn removing_a_login_logs_it_out_of_identity_center() {
    let sso = FakeAws::start(vec![Canned::empty(200)]).await;
    let h = start(account, None, None).await;
    h.state.aws.set_endpoints(Endpoints {
        oidc: Some(sso.base.clone()),
        portal: Some(sso.base.clone()),
        ssm: None,
        s3: None,
    });

    {
        let store = h.state.store.lock().unwrap();
        h.state
            .lock
            .with_key(|key| {
                let mut config = config::load(&store, key).unwrap();
                config.logins.push(Login {
                    id: "SOLTO".into(),
                    start_url: "https://solto.awsapps.com/start".into(),
                    region: "us-east-1".into(),
                    label: "Solto".into(),
                });
                config::save(&store, key, &config).unwrap();
            })
            .unwrap();
    }
    let registration = crate::aws::ClientRegistration::from_parts("c".into(), "s".into(), 4_000_000_000);
    let token = crate::aws::SsoToken::from_parts("vivo".into(), None, 4_000_000_000, registration);
    h.state.aws.with(&h.state.lock, |inner| inner.tokens.insert("SOLTO".into(), token));

    let removed = h.call("DELETE", "/aws/logins/SOLTO", None).await;
    assert_eq!(removed.status, 200, "{}", removed.body);

    let requests = sso.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].uri, "/logout");
    assert_eq!(requests[0].header("x-amz-sso_bearer_token"), Some("vivo"));
    assert_eq!(h.state.aws.with(&h.state.lock, |i| i.tokens.len()), Some(0));
}

// ── copying between vaults (§9.1) ───────────────────────────────────────────────────────────────

impl Harness {
    async fn local_vault(&self, name: &str) -> String {
        let made = self.post("/vaults", json!({ "name": name })).await;
        assert_eq!(made.status, 200, "{}", made.body);
        made.body["vault"]["id"].as_str().unwrap().to_string()
    }

    async fn local_folder(&self, vault: &str, name: &str, parent: Option<&str>) -> String {
        let made = self.post(&format!("/vaults/{vault}/folders"), json!({ "name": name, "parent_id": parent })).await;
        assert_eq!(made.status, 200, "{}", made.body);
        made.body["folder"]["id"].as_str().unwrap().to_string()
    }

    async fn local_record(&self, vault: &str, folder: Option<&str>, payload: Value) -> String {
        let made = self.post(&format!("/vaults/{vault}/records"), json!({ "folder_id": folder, "payload": payload })).await;
        assert_eq!(made.status, 200, "{}", made.body);
        made.body["record"]["id"].as_str().unwrap().to_string()
    }

    async fn copy_all(&self, scope: Value) -> Vec<Value> {
        let plan = self.post("/copy/plan", scope.clone()).await;
        assert_eq!(plan.status, 200, "{}", plan.body);
        let mut done = Vec::new();
        for item in plan.body["items"].as_array().unwrap() {
            let ran = self
                .post(
                    "/copy/run",
                    json!({
                        "source_vault": scope["source_vault"],
                        "dest_vault": scope["dest_vault"],
                        "dest_folder": scope["dest_folder"],
                        "item": { "source_id": item["source_id"], "rel": item["rel"], "name": item["name"] },
                    }),
                )
                .await;
            assert_eq!(ran.status, 200, "{}", ran.body);
            done.push(ran.body);
        }
        done
    }

    async fn trail(&self, action: &str) -> Vec<Value> {
        let read = self.get("/audit").await;
        assert_eq!(read.status, 200, "{}", read.body);
        let events = read.body["events"].as_array().cloned().unwrap_or_default();
        events.into_iter().filter(|e| e["action"] == action).collect()
    }
}

fn login(email: &str, password: &str) -> Value {
    json!({
        "title": "Banco Inter",
        "notes": "conta corrente",
        "icon": { "name": "landmark", "colour": "sky" },
        "fields": [
            { "label": "E-mail", "kind": "email", "value": email, "list": true },
            { "label": "Senha", "kind": "password", "value": password }
        ]
    })
}

#[tokio::test]
async fn a_local_vault_is_copied_whole_with_folders_files_and_favourites_and_the_original_stays() {
    let h = start(account, None, None).await;
    let from = h.local_vault("Pessoal").await;
    let to = h.local_vault("Trabalho").await;

    let banks = h.local_folder(&from, "Bancos", None).await;
    let card = h.local_folder(&from, "Cartões", Some(&banks)).await;
    let inter = h.local_record(&from, Some(&banks), login("eu@example.com", "s3nha-do-banco")).await;
    let visa = h
        .local_record(&from, Some(&card), json!({ "title": "Visa", "fields": [{ "label": "PIN", "kind": "pin", "value": "1234" }] }))
        .await;
    h.local_record(&from, None, json!({ "title": "Solto", "fields": [] })).await;
    h.post(&format!("/records/{inter}/favourite"), json!({ "on": true })).await;
    h.post(
        &format!("/records/{inter}/attachments"),
        json!({ "name": "contrato.pdf", "mime": "application/pdf", "data": crate::encoding::base64url(b"%PDF") }),
    )
    .await;
    // The trash is not copied.
    let gone = h.local_record(&from, None, json!({ "title": "Apagado", "fields": [] })).await;
    h.post(&format!("/records/{gone}/archive"), json!({ "on": true })).await;

    let scope = json!({ "source_vault": from, "dest_vault": to });
    let plan = h.post("/copy/plan", scope.clone()).await;
    assert_eq!(plan.status, 200, "{}", plan.body);
    assert_eq!(plan.body["checks"]["ok"], true);
    assert_eq!(plan.body["metadata_visible"], false);
    let items = plan.body["items"].as_array().unwrap();
    assert_eq!(items.len(), 3, "{}", plan.body);
    let row = items.iter().find(|i| i["source_id"] == inter).unwrap();
    assert_eq!(row["path"], "Bancos / Banco Inter");
    assert_eq!(row["attachments"], 1);
    assert!(row["conflict"].is_null());
    assert!(!plan.body.to_string().contains("s3nha-do-banco"), "a plan never carries a value");
    let visa_row = items.iter().find(|i| i["source_id"] == visa).unwrap();
    assert_eq!(visa_row["rel"], json!(["Bancos", "Cartões"]));

    let done = h.copy_all(scope.clone()).await;
    assert!(done.iter().all(|d| !d.to_string().contains("s3nha-do-banco")));

    let copied = done.iter().find(|d| d["source_id"] == inter).unwrap()["dest_id"].as_str().unwrap().to_string();
    let opened = h.get(&format!("/records/{copied}")).await;
    assert_eq!(opened.body["record"]["vault_id"], to);
    assert_eq!(opened.body["record"]["favourite"], true);
    assert_eq!(opened.body["record"]["version"], 1, "only the current version travels (C3)");
    assert_eq!(opened.body["record"]["payload"]["fields"][1]["value"], "s3nha-do-banco");
    assert_eq!(opened.body["record"]["payload"]["notes"], "conta corrente");
    assert_eq!(opened.body["record"]["payload"]["icon"]["name"], "landmark");
    let files = h.get(&format!("/records/{copied}/attachments")).await;
    assert_eq!(files.body["attachments"][0]["name"], "contrato.pdf");

    let folders = h.get(&format!("/vaults/{to}/folders")).await;
    let names: Vec<&str> = folders.body["folders"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Bancos", "Cartões"], "folders made once, nested");

    // C1: the original is untouched.
    let original = h.get(&format!("/records/{inter}")).await;
    assert_eq!(original.body["record"]["vault_id"], from);
    assert_eq!(original.body["record"]["archived"], false);

    // The trail says what was copied, from where to where, and never a value.
    let lines = h.trail("record.copied").await;
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().all(|l| l["detail"]["to_vault"] == to && l["detail"]["from_vault"] == from));
    assert!(!lines.iter().any(|l| l.to_string().contains("s3nha")));

    // A second plan finds every name taken (and that is what resuming relies on).
    let again = h.post("/copy/plan", scope.clone()).await;
    let conflicts: Vec<&Value> = again.body["items"].as_array().unwrap().iter().map(|i| &i["conflict"]).collect();
    assert!(conflicts.iter().all(|c| c["reason"] == "exists"), "{}", again.body);
}

#[tokio::test]
async fn conflicts_are_renamed_overwritten_or_left_and_originals_go_to_the_trash() {
    let h = start(account, None, None).await;
    let from = h.local_vault("Pessoal").await;
    let to = h.local_vault("Trabalho").await;
    let inter = h.local_record(&from, None, login("eu@example.com", "nova")).await;
    let existing = h.local_record(&to, None, login("velho@example.com", "antiga")).await;

    let plan = h.post("/copy/plan", json!({ "source_vault": from, "dest_vault": to, "record_ids": [inter] })).await;
    let item = &plan.body["items"][0];
    assert_eq!(item["conflict"]["reason"], "exists");
    assert_eq!(item["conflict"]["dest_id"], existing);

    let renamed = h
        .post(
            "/copy/run",
            json!({ "source_vault": from, "dest_vault": to, "item": { "source_id": inter, "action": "rename", "title": "Banco Inter (2)" } }),
        )
        .await;
    assert_eq!(renamed.status, 200, "{}", renamed.body);
    let made = renamed.body["dest_id"].as_str().unwrap();
    assert_eq!(h.get(&format!("/records/{made}")).await.body["record"]["payload"]["title"], "Banco Inter (2)");

    let over = h
        .post(
            "/copy/run",
            json!({ "source_vault": from, "dest_vault": to, "item": { "source_id": inter, "action": "overwrite", "dest_id": existing } }),
        )
        .await;
    assert_eq!(over.status, 200, "{}", over.body);
    let now = h.get(&format!("/records/{existing}")).await;
    assert_eq!(now.body["record"]["payload"]["fields"][1]["value"], "nova");
    assert_eq!(now.body["record"]["version"], 2, "an overwrite is a new version, the old one kept");

    // Overwriting a record of another vault is refused.
    let elsewhere = h
        .post(
            "/copy/run",
            json!({ "source_vault": from, "dest_vault": to, "item": { "source_id": inter, "action": "overwrite", "dest_id": inter } }),
        )
        .await;
    assert_eq!(elsewhere.status, 400);

    // A source id from another vault is refused.
    let wrong = h
        .post("/copy/run", json!({ "source_vault": to, "dest_vault": from, "item": { "source_id": inter } }))
        .await;
    assert_eq!(wrong.status, 400);

    // C7, local: to the trash, in one call, without typing anything.
    let removed = h.post("/copy/originals", json!({ "source_vault": from, "record_ids": [inter] })).await;
    assert_eq!(removed.status, 200, "{}", removed.body);
    assert_eq!(removed.body["removed"], 1);
    assert_eq!(removed.body["trash"], true);
    let trash = h.get(&format!("/vaults/{from}/trash")).await;
    assert_eq!(trash.body["records"].as_array().map(Vec::len), Some(1), "{}", trash.body);
}

#[tokio::test]
async fn a_local_folder_goes_to_aws_with_names_from_titles_and_the_folders_key() {
    let h = start(
        |ssm| {
            account(ssm);
            ssm.seed("/app/dev/bancos/banco-inter", "ja-existe", Some("Banco Inter"), "alias/dev", &[]);
            ssm.seed("/app/dev/bancos/legado", "texto", Some("Legado"), "", &[]);
            ssm.parameters.get_mut("/app/dev/bancos/legado").unwrap().kind = "String".into();
        },
        None,
        None,
    )
    .await;
    let from = h.local_vault("Pessoal").await;
    let banks = h.local_folder(&from, "Bancos", None).await;
    let inter = h.local_record(&from, Some(&banks), login("eu@example.com", "s3nha")).await;
    let wifi = h
        .local_record(&from, Some(&banks), json!({ "title": "Wi-Fi de casa", "fields": [{ "label": "Senha", "kind": "password", "value": "wifi123" }] }))
        .await;
    let legacy = h
        .local_record(&from, Some(&banks), json!({ "title": "Legado", "fields": [{ "label": "Senha", "kind": "password", "value": "x" }] }))
        .await;

    let scope = json!({ "source_vault": from, "folder_id": banks, "dest_vault": h.vault, "dest_folder": h.folder("/app/dev") });
    let plan = h.post("/copy/plan", scope.clone()).await;
    assert_eq!(plan.status, 200, "{}", plan.body);
    assert_eq!(plan.body["metadata_visible"], true, "C6");
    let items = plan.body["items"].as_array().unwrap();
    let by = |id: &str| items.iter().find(|i| i["source_id"] == id).unwrap().clone();
    assert_eq!(by(&wifi)["name"], "/app/dev/bancos/wi-fi-de-casa");
    assert!(by(&wifi)["conflict"].is_null());
    assert_eq!(by(&inter)["conflict"]["reason"], "exists");
    assert_eq!(by(&inter)["conflict"]["read_only"], false);
    assert_eq!(by(&legacy)["conflict"]["read_only"], true, "a String parameter is never offered for overwriting");
    assert!(!plan.body.to_string().contains("wifi123"));

    let ran = h
        .post(
            "/copy/run",
            json!({ "source_vault": from, "dest_vault": h.vault, "dest_folder": h.folder("/app/dev"),
                    "item": { "source_id": wifi, "rel": ["Bancos"], "name": by(&wifi)["name"] } }),
        )
        .await;
    assert_eq!(ran.status, 200, "{}", ran.body);
    let stored = h.parameter("/app/dev/bancos/wi-fi-de-casa").unwrap();
    assert_eq!(stored.value, "wifi123", "a single password stays a plain value");
    assert_eq!(stored.key_id, "alias/dev", "the key comes from the folder, never the default (H1)");

    let json_one = h
        .post(
            "/copy/run",
            json!({ "source_vault": from, "dest_vault": h.vault,
                    "item": { "source_id": inter, "action": "rename", "title": "Banco Inter (2)", "name": "/app/dev/bancos/banco-inter-2" } }),
        )
        .await;
    assert_eq!(json_one.status, 200, "{}", json_one.body);
    let stored = h.parameter("/app/dev/bancos/banco-inter-2").unwrap();
    assert!(stored.value.contains(r#""notes":"conta corrente""#));
    assert!(stored.tags.contains(&("keyhold:formato".into(), "json".into())));

    // Overwriting a String parameter is refused by the save path, and nothing is written.
    let puts_before = h.with_ssm(|ssm| ssm.puts.len());
    let refused = h
        .post(
            "/copy/run",
            json!({ "source_vault": from, "dest_vault": h.vault,
                    "item": { "source_id": legacy, "action": "overwrite", "dest_id": h.id("/app/dev/bancos/legado") } }),
        )
        .await;
    assert_eq!(refused.status, 400, "{}", refused.body);
    assert_eq!(h.with_ssm(|ssm| ssm.puts.len()), puts_before);
    assert_eq!(h.parameter("/app/dev/bancos/legado").unwrap().value, "texto");
}

#[tokio::test]
async fn an_aws_folder_comes_home_and_aws_originals_go_only_with_the_name_typed() {
    let h = start(
        |ssm| {
            account(ssm);
            ssm.seed("/app/prod/url", "https://exemplo", Some("URL"), "", &[]);
            ssm.parameters.get_mut("/app/prod/url").unwrap().kind = "String".into();
        },
        None,
        None,
    )
    .await;
    let to = h.local_vault("Pessoal").await;

    let scope = json!({ "source_vault": h.vault, "folder_id": h.folder("/app/prod"), "dest_vault": to });
    let plan = h.post("/copy/plan", scope.clone()).await;
    assert_eq!(plan.status, 200, "{}", plan.body);
    assert_eq!(plan.body["metadata_visible"], false);
    let items = plan.body["items"].as_array().unwrap();
    assert_eq!(items.len(), 3, "{}", plan.body);
    assert!(items.iter().all(|i| i["rel"] == json!(["prod"])));
    assert!(!plan.body.to_string().contains("hunter2"));

    let done = h.copy_all(scope).await;
    let db = done.iter().find(|d| d["source_id"] == h.id("/app/prod/db")).unwrap()["dest_id"].as_str().unwrap().to_string();
    let opened = h.get(&format!("/records/{db}")).await;
    assert_eq!(opened.body["record"]["payload"]["title"], "Banco de produção");
    assert_eq!(opened.body["record"]["payload"]["fields"][0]["value"], "hunter2");
    assert_eq!(opened.body["record"]["payload"]["notes"], "só para o deploy");

    let url = done.iter().find(|d| d["source_id"] == h.id("/app/prod/url")).unwrap()["dest_id"].as_str().unwrap().to_string();
    let opened = h.get(&format!("/records/{url}")).await;
    assert_eq!(opened.body["record"]["payload"]["fields"][0]["value"], "https://exemplo");
    assert!(crate::records::is_secret_kind(opened.body["record"]["payload"]["fields"][0]["kind"].as_str().unwrap()));

    // C7, AWS: never without the vault's name typed, and then for good.
    let ids = json!([h.id("/app/prod/db")]);
    let untyped = h.post("/copy/originals", json!({ "source_vault": h.vault, "record_ids": ids })).await;
    assert_eq!(untyped.status, 400);
    let wrong = h.post("/copy/originals", json!({ "source_vault": h.vault, "record_ids": ids, "confirm_name": "produção" })).await;
    assert_eq!(wrong.status, 400);
    assert!(h.parameter("/app/prod/db").is_some());

    let typed = h.post("/copy/originals", json!({ "source_vault": h.vault, "record_ids": ids, "confirm_name": "Produção" })).await;
    assert_eq!(typed.status, 200, "{}", typed.body);
    assert_eq!(typed.body["removed"], 1);
    assert_eq!(typed.body["trash"], false);
    assert!(h.parameter("/app/prod/db").is_none());
    assert!(h.parameter("/app/prod/api").is_some(), "only what was asked");

    // An id of another vault is not deleted through this one.
    let other = h.post("/copy/originals", json!({ "source_vault": h.vault, "record_ids": [db], "confirm_name": "Produção" })).await;
    assert_eq!(other.body["removed"], 0);
}

#[tokio::test]
async fn a_copy_into_aws_without_a_bucket_or_keys_is_stopped_before_anything_is_written() {
    let h = start(
        |ssm| {
            account(ssm);
            ssm.describe_denied = true;
        },
        None,
        None,
    )
    .await;
    let from = h.local_vault("Pessoal").await;
    let inter = h.local_record(&from, None, login("eu@example.com", "s3nha")).await;
    h.post(
        &format!("/records/{inter}/attachments"),
        json!({ "name": "a.txt", "mime": "text/plain", "data": crate::encoding::base64url(b"oi") }),
    )
    .await;

    let plan = h.post("/copy/plan", json!({ "source_vault": from, "dest_vault": h.vault, "dest_folder": h.folder("/app/dev") })).await;
    assert_eq!(plan.status, 200, "{}", plan.body);
    assert_eq!(plan.body["checks"]["ok"], false);
    let problems = plan.body["checks"]["problems"].to_string();
    assert!(problems.contains("bucket"), "{problems}");
    assert!(problems.contains("DescribeParameters"), "{problems}");
    assert!(!problems.contains('—'));
    assert!(h.with_ssm(|ssm| ssm.puts.is_empty()));
}

#[tokio::test]
async fn a_copy_stops_when_the_vault_locks() {
    let h = start(account, None, None).await;
    let from = h.local_vault("Pessoal").await;
    let to = h.local_vault("Trabalho").await;
    let inter = h.local_record(&from, None, login("eu@example.com", "s3nha")).await;

    h.state.lock.lock();

    let ran = h.post("/copy/run", json!({ "source_vault": from, "dest_vault": to, "item": { "source_id": inter } })).await;
    assert_eq!(ran.status, 423, "{}", ran.body);
    let plan = h.post("/copy/plan", json!({ "source_vault": from, "dest_vault": to })).await;
    assert_eq!(plan.status, 423);
}

// ── copy: what the review found (2026-11-11) ────────────────────────────────────────────────────

/// A second vault entry on the same connection and region: another name for the same parameters.
fn alias_vault(h: &Harness) -> String {
    let store = h.state.store.lock().unwrap();
    h.state
        .lock
        .with_key(|key| {
            vaults::create_vault(
                &store,
                key,
                VaultMeta {
                    name: "Produção de novo".into(),
                    kind: "aws".into(),
                    aws: Some(AwsVault { connection_id: "CONN".into(), region: "sa-east-1".into(), bucket: None, rename_with_title: false }),
                    ..Default::default()
                },
            )
        })
        .unwrap()
        .unwrap()
        .id
}

#[tokio::test]
async fn a_record_copied_onto_itself_is_never_overwritten_and_its_original_never_deleted() {
    let h = start(account, None, None).await;
    let from = h.local_vault("Pessoal").await;
    let inter = h.local_record(&from, None, login("eu@example.com", "s3nha")).await;

    // Local: the same vault, the same place.
    let plan = h.post("/copy/plan", json!({ "source_vault": from, "dest_vault": from, "record_ids": [inter] })).await;
    assert_eq!(plan.body["items"][0]["conflict"]["reason"], "self", "{}", plan.body);

    let over = h
        .post("/copy/run", json!({ "source_vault": from, "dest_vault": from, "item": { "source_id": inter, "action": "overwrite", "dest_id": inter } }))
        .await;
    assert_eq!(over.status, 400, "{}", over.body);

    let removed = h.post("/copy/originals", json!({ "source_vault": from, "record_ids": [inter] })).await;
    assert_eq!(removed.body["removed"], 0, "nothing was copied, so nothing may be deleted");
    assert_eq!(h.get(&format!("/records/{inter}")).await.body["record"]["archived"], false);

    // AWS: another vault entry over the same account and region is the same place.
    let alias = alias_vault(&h);
    let db = h.id("/app/prod/db");
    let plan = h
        .post("/copy/plan", json!({ "source_vault": h.vault, "dest_vault": alias, "record_ids": [db], "dest_folder": crate::aws::mapping::folder_id(&alias, "/app/prod") }))
        .await;
    assert_eq!(plan.status, 200, "{}", plan.body);
    assert_eq!(plan.body["items"][0]["conflict"]["reason"], "self", "{}", plan.body);
    let dest = plan.body["items"][0]["conflict"]["dest_id"].clone();

    let over = h
        .post("/copy/run", json!({ "source_vault": h.vault, "dest_vault": alias, "item": { "source_id": db, "action": "overwrite", "dest_id": dest } }))
        .await;
    assert_eq!(over.status, 400, "{}", over.body);

    let gone = h.post("/copy/originals", json!({ "source_vault": h.vault, "record_ids": [db], "confirm_name": "Produção" })).await;
    assert_eq!(gone.body["removed"], 0, "{}", gone.body);
    assert!(h.parameter("/app/prod/db").is_some(), "the only copy survived");
}

#[tokio::test]
async fn only_originals_copied_whole_and_still_there_can_be_deleted() {
    let h = start(account, None, None).await;
    let from = h.local_vault("Pessoal").await;
    let inter = h.local_record(&from, None, login("eu@example.com", "s3nha")).await;
    let other = h.local_record(&from, None, json!({ "title": "Outro", "fields": [] })).await;
    h.post(
        &format!("/records/{inter}/attachments"),
        json!({ "name": "a.txt", "mime": "text/plain", "data": crate::encoding::base64url(b"oi") }),
    )
    .await;

    // Into AWS with no bucket: the record arrives, its file does not.
    let ran = h
        .post("/copy/run", json!({ "source_vault": from, "dest_vault": h.vault, "dest_folder": h.folder("/app/dev"), "item": { "source_id": inter } }))
        .await;
    assert_eq!(ran.status, 200, "{}", ran.body);
    assert_eq!(ran.body["files_ok"], false, "{}", ran.body);

    let removed = h.post("/copy/originals", json!({ "source_vault": from, "record_ids": [inter, other] })).await;
    assert_eq!(removed.body["removed"], 0, "{}", removed.body);
    assert_eq!(removed.body["failed"].as_array().unwrap().len(), 2);

    // Sent again, the same record is not written twice.
    let again = h
        .post("/copy/run", json!({ "source_vault": from, "dest_vault": h.vault, "dest_folder": h.folder("/app/dev"), "item": { "source_id": inter } }))
        .await;
    assert_eq!(again.status, 200, "{}", again.body);
    assert_eq!(again.body["already"], true);
    assert_eq!(again.body["dest_id"], ran.body["dest_id"]);

    // A whole copy whose destination is then deleted no longer counts.
    let to = h.local_vault("Trabalho").await;
    let copied = h.post("/copy/run", json!({ "source_vault": from, "dest_vault": to, "item": { "source_id": other } })).await;
    let dest = copied.body["dest_id"].as_str().unwrap().to_string();
    h.post(&format!("/records/{dest}/archive"), json!({ "on": true })).await;
    let removed = h.post("/copy/originals", json!({ "source_vault": from, "record_ids": [other] })).await;
    assert_eq!(removed.body["removed"], 0, "{}", removed.body);

    // A copy that is there: the original goes to the trash once, and a second call leaves it be.
    let fresh = h.post("/copy/run", json!({ "source_vault": from, "dest_vault": to, "item": { "source_id": other, "action": "rename", "title": "Outro" } })).await;
    assert_eq!(fresh.status, 200, "{}", fresh.body);
    let removed = h.post("/copy/originals", json!({ "source_vault": from, "record_ids": [other] })).await;
    assert_eq!(removed.body["removed"], 1, "{}", removed.body);
    let twice = h.post("/copy/originals", json!({ "source_vault": from, "record_ids": [other] })).await;
    assert_eq!(twice.body["removed"], 0);
    assert_eq!(twice.body["skipped"], 1, "{}", twice.body);

    // Locked: nothing is answered.
    h.state.lock.lock();
    let locked = h.post("/copy/originals", json!({ "source_vault": from, "record_ids": [other] })).await;
    assert_eq!(locked.status, 423);
}

#[tokio::test]
async fn an_overwrite_into_aws_checks_the_version_the_plan_saw() {
    let h = start(account, None, None).await;
    let from = h.local_vault("Pessoal").await;
    let api = h.local_record(&from, None, json!({ "title": "API", "fields": [{ "label": "Senha", "kind": "password", "value": "nova" }] })).await;

    let plan = h.post("/copy/plan", json!({ "source_vault": from, "dest_vault": h.vault, "record_ids": [api], "dest_folder": h.folder("/app/prod") })).await;
    let conflict = plan.body["items"][0]["conflict"].clone();
    assert_eq!(conflict["reason"], "exists", "{}", plan.body);
    let seen = conflict["version"].as_i64().unwrap();

    let stale = h
        .post("/copy/run", json!({ "source_vault": from, "dest_vault": h.vault, "item": { "source_id": api, "action": "overwrite", "dest_id": conflict["dest_id"], "expected_version": seen - 1 } }))
        .await;
    assert_eq!(stale.status, 409, "{}", stale.body);

    let fine = h
        .post("/copy/run", json!({ "source_vault": from, "dest_vault": h.vault, "item": { "source_id": api, "action": "overwrite", "dest_id": conflict["dest_id"], "expected_version": seen } }))
        .await;
    assert_eq!(fine.status, 200, "{}", fine.body);
    assert_eq!(h.parameter("/app/prod/api").unwrap().value, "nova");
}

#[tokio::test]
async fn a_rename_moves_past_names_already_taken() {
    let h = start(
        |ssm| {
            account(ssm);
            ssm.seed("/app/prod/db-2", "ocupado", Some("Outro"), "alias/prod", &[]);
        },
        None,
        None,
    )
    .await;
    let from = h.local_vault("Pessoal").await;
    let db = h.local_record(&from, None, json!({ "title": "db", "fields": [{ "label": "Senha", "kind": "password", "value": "x" }] })).await;

    let ran = h
        .post("/copy/run", json!({ "source_vault": from, "dest_vault": h.vault, "item": { "source_id": db, "action": "rename", "title": "db", "name": "/app/prod/db-2" } }))
        .await;
    assert_eq!(ran.status, 200, "{}", ran.body);
    assert_eq!(ran.body["name"], "/app/prod/db-3");
    assert!(h.parameter("/app/prod/db-3").is_some());

    // Local: "X (2)" taken becomes "X (3)".
    let to = h.local_vault("Trabalho").await;
    h.local_record(&to, None, json!({ "title": "Banco Inter (2)", "fields": [] })).await;
    let inter = h.local_record(&from, None, login("eu@example.com", "s")).await;
    let ran = h
        .post("/copy/run", json!({ "source_vault": from, "dest_vault": to, "item": { "source_id": inter, "action": "rename", "title": "Banco Inter (2)" } }))
        .await;
    assert_eq!(ran.status, 200, "{}", ran.body);
    assert_eq!(ran.body["title"], "Banco Inter (3)");
}

#[tokio::test]
async fn a_string_parameter_holding_json_reads_as_one_password_per_key() {
    let h = start(
        |ssm| {
            account(ssm);
            ssm.seed("/app/legado/config", r#"{"host":"db.interno","port":5432,"password":"p4ss"}"#, Some("Config"), "", &[]);
            ssm.parameters.get_mut("/app/legado/config").unwrap().kind = "String".into();
            ssm.seed("/app/legado/url", "texto simples", Some("URL"), "", &[]);
            ssm.parameters.get_mut("/app/legado/url").unwrap().kind = "String".into();
        },
        None,
        None,
    )
    .await;

    let opened = h.get(&format!("/records/{}", h.id("/app/legado/config"))).await;
    let fields = opened.body["record"]["payload"]["fields"].as_array().unwrap().clone();
    let pairs: Vec<(String, String, String)> = fields
        .iter()
        .map(|f| (f["label"].as_str().unwrap().into(), f["kind"].as_str().unwrap().into(), f["value"].as_str().unwrap().into()))
        .collect();
    assert_eq!(pairs.len(), 3, "{pairs:?}");
    assert!(pairs.contains(&("port".into(), "password".into(), "5432".into())));
    assert!(pairs.contains(&("password".into(), "password".into(), "p4ss".into())));

    let plain = h.get(&format!("/records/{}", h.id("/app/legado/url"))).await;
    assert_eq!(plain.body["record"]["payload"]["fields"][0]["label"], "Valor");

    // And it comes home the same way.
    let to = h.local_vault("Pessoal").await;
    let done = h
        .copy_all(json!({ "source_vault": h.vault, "record_ids": [h.id("/app/legado/config")], "dest_vault": to }))
        .await;
    let copied = done[0]["dest_id"].as_str().unwrap();
    let home = h.get(&format!("/records/{copied}")).await;
    assert_eq!(home.body["record"]["payload"]["fields"].as_array().unwrap().len(), 3);

    // A SecureString with an application's JSON keeps it whole: saving must not rewrite it.
    let config = h.get(&format!("/records/{}", h.id("/config/app"))).await;
    assert_eq!(config.body["record"]["payload"]["fields"].as_array().unwrap().len(), 1);
}
