//! The agent surface, driven the way an agent drives it: over the socket, with nobody watching.
//!
//! These use the daemon's own state rather than a mock, because the questions worth asking are
//! about what it *refuses*: a vault nobody marked, a record in it, a request nobody answers.

use std::sync::Arc;

use keyhold::agent::protocol::{Request, Response};
use keyhold::agent::AgentState;
use keyhold::db::Store;
use keyhold::http::AppState;
use keyhold::identity::{self, SetupOptions};
use keyhold::lock::{Lock, LockPolicy};
use keyhold::records::{Field, Payload};
use keyhold::session::SessionToken;
use keyhold::{records, vaults};

struct Fixture {
    state: Arc<AppState>,
    record: String,
    vault: String,
    _dir: tempfile::TempDir,
}

/// A vault with one login in it, unlocked, and *not* marked for agents.
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("vault.db")).unwrap();

    let outcome =
        identity::create(&mut store, SetupOptions { password: Some("the password".into()) }).unwrap();

    let vault = vaults::create_vault(
        &store,
        &outcome.vault_key,
        vaults::VaultMeta {
            name: "Servidores".into(),
            description: String::new(),
            colour: "default".into(),
        },
    )
    .unwrap();

    let record = records::create(
        &store,
        &outcome.vault_key,
        &vault.id,
        None,
        "login",
        &Payload {
            title: "Produção".into(),
            username: "deploy".into(),
            fields: vec![Field {
                label: "Senha".into(),
                kind: "password".into(),
                value: "hunter2".into(),
            }],
            ..Payload::default()
        },
    )
    .unwrap();

    let lock = Lock::new(LockPolicy { idle_seconds: None, hidden_seconds: None });
    lock.unlock_with(outcome.vault_key);

    let state = Arc::new(AppState {
        token: SessionToken::generate(),
        agent: AgentState::default(),
        store: Arc::new(std::sync::Mutex::new(store)),
        lock,
        failures: std::sync::Mutex::new(0),
    });

    Fixture { state, record: record.id, vault: vault.id, _dir: dir }
}

impl Fixture {
    fn allow_agents(&self) {
        let store = self.state.store.lock().unwrap();
        vaults::set_agent_access(&store, &self.vault, true).unwrap();
    }

    /// Let values be handed over at all.
    ///
    /// Separate from `allow_agents` because they answer different questions: one is "may an agent
    /// see this vault", the other is "may an agent ever receive a value". The second is off by
    /// default, so a test about the approval flow has to ask for it — and a test about scope must
    /// turn it on too, or it would pass for the wrong reason.
    fn allow_reveal(&self) {
        let store = self.state.store.lock().unwrap();
        keyhold::settings::set_agent_may_reveal(&store, true).unwrap();
    }

    async fn ask(&self, request: Request) -> Response {
        keyhold::agent::handle_for_tests(request, &self.state).await
    }
}

#[tokio::test]
async fn an_unmarked_vault_is_invisible_rather_than_refused() {
    let app = fixture();

    // Nothing is marked, so there is nothing to find — not a denial, an absence.
    match app.ask(Request::Search { query: String::new() }).await {
        Response::Records { records } => assert!(records.is_empty(), "{records:?}"),
        other => panic!("unexpected: {other:?}"),
    }

    match app.ask(Request::Status).await {
        Response::Status { vaults, records, .. } => {
            assert!(vaults.is_empty());
            assert_eq!(records, 0);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn a_marked_vault_can_be_searched_without_handing_over_anything() {
    let app = fixture();
    app.allow_agents();

    let response = app.ask(Request::Search { query: "produ".into() }).await;

    match response {
        Response::Records { records } => {
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].title, "Produção");
            assert_eq!(records[0].username, "deploy");

            // The summary is metadata only.
            let rendered = serde_json::to_string(&records).unwrap();
            assert!(!rendered.contains("hunter2"), "{rendered}");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn describing_a_record_names_its_fields_without_their_values() {
    let app = fixture();
    app.allow_agents();

    match app.ask(Request::Describe { record: app.record.clone() }).await {
        Response::Fields { title, fields } => {
            assert_eq!(title, "Produção");
            assert!(fields.iter().any(|f| f.label == "Senha" && f.secret));

            let rendered = serde_json::to_string(&fields).unwrap();
            assert!(!rendered.contains("hunter2"), "{rendered}");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn a_record_outside_the_marked_vaults_is_refused_even_by_id() {
    let app = fixture();

    // Reading is allowed; what is not allowed is *this vault*. Without this line the test would
    // pass because of the policy rather than because of the scope, and would keep passing if the
    // scope check were deleted.
    app.allow_reveal();

    // The agent knows the id — it is the one thing it could have kept from before the vault was
    // unmarked. Knowing it must not be enough.
    match app.ask(Request::Describe { record: app.record.clone() }).await {
        Response::Refused { .. } => {}
        other => panic!("an unmarked record was described: {other:?}"),
    }

    match app.ask(Request::Get { record: app.record.clone(), field: "Senha".into() }).await {
        Response::Refused { .. } => {}
        other => panic!("an unmarked record handed over a value: {other:?}"),
    }
}

#[tokio::test]
async fn a_value_is_only_handed_over_after_somebody_says_yes() {
    let app = fixture();
    app.allow_agents();
    app.allow_reveal();

    let asking = {
        let state = app.state.clone();
        let record = app.record.clone();

        tokio::spawn(async move {
            keyhold::agent::handle_for_tests(
                Request::Get { record, field: "Senha".into() },
                &state,
            )
            .await
        })
    };

    // The request shows up as a question, naming what it wants.
    let waiting = loop {
        let waiting = app.state.agent.waiting();

        if !waiting.is_empty() {
            break waiting;
        }

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };

    assert_eq!(waiting[0].field, "Senha");
    assert_eq!(waiting[0].record_title, "Produção");
    assert_eq!(waiting[0].intent, "get");

    app.state.agent.answer(&waiting[0].id, true, 0);

    match asking.await.unwrap() {
        Response::Value { value } => assert_eq!(value, "hunter2"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn a_refusal_is_a_refusal() {
    let app = fixture();
    app.allow_agents();
    app.allow_reveal();

    let asking = {
        let state = app.state.clone();
        let record = app.record.clone();

        tokio::spawn(async move {
            keyhold::agent::handle_for_tests(
                Request::Get { record, field: "Senha".into() },
                &state,
            )
            .await
        })
    };

    let waiting = loop {
        let waiting = app.state.agent.waiting();

        if !waiting.is_empty() {
            break waiting;
        }

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };

    app.state.agent.answer(&waiting[0].id, false, 0);

    match asking.await.unwrap() {
        Response::Refused { .. } => {}
        other => panic!("a refused request handed over a value: {other:?}"),
    }

    // And the refusal is in the trail, as the agent.
    let store = app.state.store.lock().unwrap();
    let events = keyhold::audit::recent(&store, 5).unwrap();
    let denied = events
        .iter()
        .find(|event| event.result == keyhold::audit::Outcome::Denied)
        .expect("the refusal should be recorded");

    assert!(matches!(denied.actor, keyhold::audit::Actor::Agent { .. }));
}

/// Ask for a value and answer the question that appears, returning what the agent got.
async fn ask_and_answer(app: &Fixture, field: &str, allowed: bool, minutes: u64) -> Response {
    let state = app.state.clone();
    let record = app.record.clone();
    let wanted = field.to_string();

    let asking = tokio::spawn(async move {
        keyhold::agent::handle_for_tests(Request::Get { record, field: wanted }, &state).await
    });

    let waiting = loop {
        let waiting = app.state.agent.waiting();

        if !waiting.is_empty() {
            break waiting;
        }

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };

    app.state.agent.answer(&waiting[0].id, allowed, minutes);

    asking.await.unwrap()
}

#[tokio::test]
async fn a_grant_covers_the_next_request_for_the_same_field_and_nothing_else() {
    let app = fixture();
    app.allow_agents();
    app.allow_reveal();

    // A yes that stands for five minutes.
    assert!(matches!(ask_and_answer(&app, "Senha", true, 5).await, Response::Value { .. }));

    // The same field now answers without asking again — which is the whole point of a grant, and
    // is checked by the absence of a new question rather than by waiting on one.
    let again = app.ask(Request::Get { record: app.record.clone(), field: "Senha".into() }).await;

    assert!(matches!(again, Response::Value { .. }));
    assert!(app.state.agent.waiting().is_empty(), "a granted field asked again");

    // A different field is not covered: it puts a question up and waits for it.
    let state = app.state.clone();
    let record = app.record.clone();

    let other = tokio::spawn(async move {
        keyhold::agent::handle_for_tests(
            Request::Get { record, field: "username".into() },
            &state,
        )
        .await
    });

    let waiting = loop {
        let waiting = app.state.agent.waiting();

        if !waiting.is_empty() {
            break waiting;
        }

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };

    assert_eq!(waiting[0].field, "username", "a grant for one field covered another");

    app.state.agent.answer(&waiting[0].id, false, 0);
    other.await.unwrap();
}

#[tokio::test]
async fn revoking_takes_every_standing_yes_away() {
    let app = fixture();
    app.allow_agents();
    app.allow_reveal();

    ask_and_answer(&app, "Senha", true, 5).await;
    assert_eq!(app.state.agent.active_grants(), 1);

    app.state.agent.revoke_all();
    assert_eq!(app.state.agent.active_grants(), 0);

    // And the next request asks again.
    let state = app.state.clone();
    let record = app.record.clone();

    let asking = tokio::spawn(async move {
        keyhold::agent::handle_for_tests(
            Request::Get { record, field: "Senha".into() },
            &state,
        )
        .await
    });

    loop {
        let waiting = app.state.agent.waiting();

        if !waiting.is_empty() {
            app.state.agent.answer(&waiting[0].id, false, 0);
            break;
        }

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    asking.await.unwrap();
}

#[tokio::test]
async fn run_returns_the_output_and_never_the_secret() {
    let app = fixture();
    app.allow_agents();
    app.allow_reveal();

    let asking = {
        let state = app.state.clone();
        let record = app.record.clone();

        tokio::spawn(async move {
            keyhold::agent::handle_for_tests(
                Request::Run {
                    record,
                    field: "Senha".into(),
                    // Prints the length of the variable, so the test can prove the value arrived
                    // without the test itself echoing it.
                    command: vec![
                        "sh".into(),
                        "-c".into(),
                        "printf '%s' \"${#KEYHOLD_SECRET}\"".into(),
                    ],
                    variable: "KEYHOLD_SECRET".into(),
                },
                &state,
            )
            .await
        })
    };

    let waiting = loop {
        let waiting = app.state.agent.waiting();

        if !waiting.is_empty() {
            break waiting;
        }

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };

    assert_eq!(waiting[0].intent, "run");
    app.state.agent.answer(&waiting[0].id, true, 0);

    match asking.await.unwrap() {
        Response::Ran { code, stdout, .. } => {
            assert_eq!(code, 0);
            assert_eq!(stdout, "7", "the secret did not reach the child process");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn a_locked_vault_answers_nothing_at_all() {
    let app = fixture();
    app.allow_agents();
    app.allow_reveal();
    app.state.lock.lock();

    for request in [
        Request::Status,
        Request::Search { query: String::new() },
        Request::Describe { record: app.record.clone() },
        Request::Get { record: app.record.clone(), field: "Senha".into() },
    ] {
        match app.ask(request).await {
            Response::Refused { reason } => assert!(reason.contains("trancado"), "{reason}"),
            other => panic!("a locked vault answered: {other:?}"),
        }
    }
}

// ── what an agent may and may not receive ───────────────────────────────────────────────────────

#[tokio::test]
async fn a_value_is_refused_outright_until_reading_is_switched_on() {
    let app = fixture();
    app.allow_agents();

    // The default. No approval question is even raised: there is nothing to approve.
    let refused = app.ask(Request::Get { record: app.record.clone(), field: "Senha".into() }).await;

    match refused {
        Response::Refused { reason } => {
            assert!(reason.contains("não entrega valores"), "{reason}");
            assert!(reason.contains("keyhold_run"), "the refusal should say what to do instead");
        }
        other => panic!("a value was handed over with reading switched off: {other:?}"),
    }

    assert!(app.state.agent.waiting().is_empty(), "a refused-by-policy request still asked");

    // And the refusal is in the trail.
    let store = app.state.store.lock().unwrap();
    let events = keyhold::audit::recent(&store, 3).unwrap();

    assert!(events
        .iter()
        .any(|event| event.result == keyhold::audit::Outcome::Denied));
}

#[tokio::test]
async fn switching_reading_on_lets_a_value_through_the_usual_approval() {
    let app = fixture();
    app.allow_agents();
    app.allow_reveal();

    let state = app.state.clone();
    let record = app.record.clone();

    let asking = tokio::spawn(async move {
        keyhold::agent::handle_for_tests(Request::Get { record, field: "Senha".into() }, &state)
            .await
    });

    let waiting = loop {
        let waiting = app.state.agent.waiting();

        if !waiting.is_empty() {
            break waiting;
        }

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };

    app.state.agent.answer(&waiting[0].id, true, 0);

    match asking.await.unwrap() {
        Response::Value { value } => assert_eq!(value, "hunter2"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn status_tells_the_agent_what_it_is_allowed_to_do() {
    let app = fixture();
    app.allow_agents();

    match app.ask(Request::Status).await {
        Response::Status { may_reveal, .. } => {
            assert!(!may_reveal, "status should report that reading is off by default")
        }
        other => panic!("unexpected: {other:?}"),
    }

    app.allow_reveal();

    match app.ask(Request::Status).await {
        Response::Status { may_reveal, .. } => assert!(may_reveal),
        other => panic!("unexpected: {other:?}"),
    }
}
