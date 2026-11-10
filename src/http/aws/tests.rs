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
}

#[derive(Default)]
struct Ssm {
    parameters: BTreeMap<String, FakeParameter>,
    /// Names under this prefix exist, but this role may not read them.
    denied: Vec<String>,
    clock: f64,
    /// Every `PutParameter` body, to check what was asked for.
    puts: Vec<Value>,
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

    match target.as_str() {
        "GetParametersByPath" => {
            let path = text("Path");
            let items: Vec<Value> = ssm
                .under(&path, body["Recursive"].as_bool().unwrap_or(false))
                .filter(|(name, _)| !ssm.denied(name))
                .map(|(name, p)| {
                    json!({
                        "Name": name, "Type": "SecureString", "Value": p.value,
                        "Version": p.version, "LastModifiedDate": p.modified,
                    })
                })
                .collect();
            let (items, next) = page(items, 10, body["NextToken"].as_str());
            ok(json!({ "Parameters": items, "NextToken": next }))
        }
        "DescribeParameters" => {
            let filter = &body["ParameterFilters"][0];
            let path = filter["Values"][0].as_str().unwrap_or("/").to_string();
            let recursive = filter["Option"] == "Recursive";
            // Describe sees everything, denied or not. Keyhold must not show what it only saw here.
            let items: Vec<Value> = ssm
                .under(&path, recursive)
                .map(|(name, p)| {
                    json!({
                        "Name": name, "Type": "SecureString", "KeyId": p.key_id,
                        "Description": p.description, "Version": p.version,
                        "LastModifiedDate": p.modified, "Tier": "Standard",
                    })
                })
                .collect();
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
                    "Name": name, "Type": "SecureString", "Value": p.value,
                    "Version": p.version, "LastModifiedDate": p.modified,
                }})),
            }
        }
        "PutParameter" => {
            let name = text("Name");
            ssm.puts.push(body.clone());
            if ssm.denied(&name) {
                return error("AccessDeniedException", "not allowed");
            }
            let overwrite = body["Overwrite"].as_bool().unwrap_or(false);
            ssm.clock += 1.0;
            let clock = ssm.clock;
            let key = body["KeyId"].as_str().unwrap_or("alias/aws/ssm").to_string();
            let description = body["Description"].as_str().map(str::to_string);
            let value = text("Value");

            let version = match ssm.parameters.get_mut(&name) {
                Some(_) if !overwrite => return error("ParameterAlreadyExists", "exists"),
                Some(existing) => {
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
                        },
                    );
                    1
                }
            };
            ok(json!({ "Version": version, "Tier": "Standard" }))
        }
        "DeleteParameter" => match ssm.parameters.remove(&text("Name")) {
            Some(_) => ok(json!({})),
            None => error("ParameterNotFound", "nope"),
        },
        "GetParameterHistory" => {
            let name = text("Name");
            let Some(p) = ssm.parameters.get(&name) else {
                return error("ParameterNotFound", "nope");
            };
            let items: Vec<Value> = p
                .history
                .iter()
                .map(|(version, value, description, at)| {
                    json!({
                        "Name": name, "Type": "SecureString", "KeyId": p.key_id,
                        "Description": description, "Value": value, "Version": version,
                        "LastModifiedDate": at,
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
        crate::aws::mapping::encode_id(&self.vault, name)
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
            aws: Some(AwsVault { connection_id: "CONN".into(), region: "sa-east-1".into(), bucket }),
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
        .map(|f| crate::aws::mapping::decode_id(f["id"].as_str().unwrap()).unwrap().1)
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
    assert_eq!(db["folder_id"], h.id("/app/prod"));

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
                "folder_id": h.id("/app/prod"),
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
    assert_eq!(stored.description.as_deref(), Some("Senha do Redis (produção)\nrotacionar todo mês"));
    assert!(stored.value.starts_with(r#"{"keyhold":1,"#));
    assert!(stored.tags.contains(&("keyhold:formato".into(), "json".into())));

    // In an empty folder, the marker's key is the folder's key.
    h.post(
        &format!("/vaults/{}/records", h.vault),
        json!({
            "folder_id": h.id("/app/dev"),
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
                "folder_id": h.id("/app/dev"),
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

    let restored = h.post(&format!("/records/{id}/versions/1/restore"), json!({})).await;
    assert_eq!(restored.status, 200, "{}", restored.body);
    assert_eq!(restored.body["record"]["version"], 4);
    assert_eq!(h.parameter("/app/prod/db").unwrap().value, "hunter2");
}

#[tokio::test]
async fn folders_are_markers_and_a_full_one_is_not_deleted() {
    let h = start(account, None, None).await;

    let made = h
        .post(&format!("/vaults/{}/folders", h.vault), json!({ "name": "novo", "parent_id": h.id("/app") }))
        .await;
    assert_eq!(made.status, 200, "{}", made.body);
    assert_eq!(made.body["folder"]["id"], h.id("/app/novo"));
    assert_eq!(made.body["folder"]["depth"], 1);
    assert!(h.parameter("/app/novo/.keep").is_some());

    let full = h.call("DELETE", &format!("/folders/{}", h.id("/app/prod")), None).await;
    assert_eq!(full.status, 409);
    assert!(h.parameter("/app/prod/db").is_some());

    let empty = h.call("DELETE", &format!("/folders/{}", h.id("/app/novo")), None).await;
    assert_eq!(empty.status, 200, "{}", empty.body);
    assert!(h.parameter("/app/novo/.keep").is_none());

    let rename = h.put(&format!("/folders/{}", h.id("/app/dev")), json!({ "name": "x" })).await;
    assert_eq!(rename.status, 400);
}

#[tokio::test]
async fn moving_renames_with_tags_and_deleting_is_for_good() {
    let h = start(account, None, None).await;
    let id = h.id("/app/prod/db");

    let starred = h.post(&format!("/records/{id}/favourite"), json!({ "on": true })).await;
    assert_eq!(starred.status, 200);

    let moved = h.post(&format!("/records/{id}/move"), json!({ "folder_id": h.id("/app/dev") })).await;
    assert_eq!(moved.status, 200, "{}", moved.body);
    assert_eq!(moved.body["id"], h.id("/app/dev/db"));
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
    assert_eq!(added.body["attachment"]["id"], h.id("app/prod/db/chave.pem"));

    let listed = h.get(&format!("/records/{id}/attachments")).await;
    let names: Vec<&str> = listed.body["attachments"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["chave.pem"]);

    let denied = h.get(&format!("/attachments/{}", h.id("app/prod/db/chave.pem"))).await;
    assert_eq!(denied.status, 403);
    assert!(denied.body["error"].as_str().unwrap().contains("keyhold-producao"));

    let requests = s3.requests();
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(requests[0].uri, "/keyhold-producao/app/prod/db/chave.pem");
    assert_eq!(requests[0].header("x-amz-server-side-encryption"), Some("aws:kms"));
    assert_eq!(requests[0].header("x-amz-server-side-encryption-aws-kms-key-id"), Some("alias/prod"));
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
