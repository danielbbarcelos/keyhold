//! The trail: what was done, when, and by whom.
//!
//! **Why a vault needs one at all.** Cryptography answers what somebody *can* open. It cannot
//! answer what they *did* open, and that second question is the only one that matters after a
//! laptop is left unlocked, or once an agent is allowed to read things (F7). The trail is the only
//! part of this file keyhold can read without a key, and that is deliberate: a log you need the
//! vault open to inspect is a log you cannot consult when the vault is the problem.
//!
//! **It is append-only by construction.** No `UPDATE`, no `DELETE`, nowhere in this codebase. Each
//! row carries the hash of the one before it, so editing or removing a line breaks every hash after
//! it — [`verify`] finds exactly where. That does not stop somebody with write access to the file
//! from truncating the whole chain; nothing local can. What it stops is a quiet edit.
//!
//! **What it never contains.** No password, no field value, no note — not even encrypted. The trail
//! records that a field called "Senha" on a given record was revealed, never what it said.
//! [`Detail::sanitised`] drops anything that looks like a secret rather than trusting every caller
//! to remember, because the one caller that forgets is the one that matters.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::Store;

/// Who did the thing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Actor {
    /// The person at the keyboard, through the interface.
    You,
    /// A local agent, named by the process that asked (F7).
    Agent { process: String, pid: i64 },
}

impl Actor {
    fn as_text(&self) -> String {
        match self {
            Actor::You => "you".into(),
            Actor::Agent { process, pid } => format!("agent:{process}:{pid}"),
        }
    }

    fn parse(raw: &str) -> Self {
        let mut parts = raw.splitn(3, ':');

        match (parts.next(), parts.next(), parts.next()) {
            (Some("agent"), Some(process), Some(pid)) => Actor::Agent {
                process: process.to_string(),
                pid: pid.parse().unwrap_or(0),
            },
            _ => Actor::You,
        }
    }
}

/// Whether it was allowed to happen.
///
/// Denials are recorded too, and they are the more interesting half: a refused read is the shape of
/// somebody — or something — trying.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Allowed,
    Denied,
}

impl Outcome {
    fn as_text(self) -> &'static str {
        match self {
            Outcome::Allowed => "allowed",
            Outcome::Denied => "denied",
        }
    }
}

/// The actions worth writing down, as stable strings.
///
/// Written out rather than derived from a type so that a rename in Rust cannot silently rewrite the
/// meaning of rows already on disk.
pub mod action {
    pub const UNLOCK: &str = "vault.unlock";
    pub const UNLOCK_FAILED: &str = "vault.unlock.failed";
    pub const LOCK: &str = "vault.lock";
    pub const RECORD_CREATED: &str = "record.created";
    pub const RECORD_UPDATED: &str = "record.updated";
    pub const RECORD_DELETED: &str = "record.deleted";
    pub const RECORD_DESTROYED: &str = "record.destroyed";
    pub const RECORD_RESTORED: &str = "record.restored";
    pub const SECRET_REVEALED: &str = "secret.revealed";
    pub const SECRET_COPIED: &str = "secret.copied";
    pub const VAULT_CREATED: &str = "vault.created";
    pub const VAULT_DELETED: &str = "vault.deleted";
    pub const VAULT_ROTATED: &str = "vault.rotated";
    pub const EXPORTED: &str = "vault.exported";
    pub const IMPORTED: &str = "vault.imported";
    pub const PROTECTION_CHANGED: &str = "identity.protection.changed";
    pub const SECOND_FACTOR_CHANGED: &str = "identity.second-factor.changed";
}

/// The keys whose *values* are never written down, whatever a caller passes.
///
/// A deny-list rather than an allow-list, because the cost of the two mistakes is not symmetric:
/// dropping a harmless field loses a line of context, keeping a harmful one writes a password into
/// the one table that is readable without a key.
const FORBIDDEN_KEYS: [&str; 10] = [
    "password", "senha", "secret", "value", "valor", "token", "passphrase", "key", "chave", "code",
];

/// Context for an event: small, flat, and free of anything secret.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Detail(pub serde_json::Map<String, serde_json::Value>);

impl Detail {
    pub fn new() -> Self {
        Self(serde_json::Map::new())
    }

    pub fn with(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        self.0.insert(key.to_string(), value.into());
        self
    }

    /// Drop anything that looks like it carries a secret.
    ///
    /// Runs on the way in, so there is no path from a careless caller to a value on disk.
    fn sanitised(mut self) -> Self {
        self.0.retain(|key, _| {
            let lower = key.to_lowercase();

            !FORBIDDEN_KEYS.iter().any(|forbidden| lower.contains(forbidden))
        });

        self
    }
}

#[derive(Debug, Serialize)]
pub struct Event {
    pub seq: i64,
    pub at: String,
    pub actor: Actor,
    pub action: String,
    pub scope: Option<String>,
    pub result: Outcome,
    pub detail: serde_json::Value,
    pub hash: String,
}

/// The hash of a row, over every field that matters plus the previous hash.
///
/// The fields are joined with a separator that cannot appear in them (a newline after each length
/// prefix), so that two different rows cannot serialise to the same bytes by moving a character
/// from one field into the next.
fn hash_of(
    seq: i64,
    at: &str,
    actor: &str,
    action: &str,
    scope: Option<&str>,
    result: &str,
    detail: &str,
    prev_hash: &str,
) -> String {
    let mut hasher = Sha256::new();

    for field in [
        seq.to_string().as_str(),
        at,
        actor,
        action,
        scope.unwrap_or(""),
        result,
        detail,
        prev_hash,
    ] {
        hasher.update(field.len().to_string().as_bytes());
        hasher.update(b"\n");
        hasher.update(field.as_bytes());
    }

    format!("{:x}", hasher.finalize())
}

/// The hash the first row of a chain builds on.
const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Write one event.
///
/// Failure to record is not failure of the action: a vault that refuses to unlock because its log
/// is full would be a vault whose log is a denial-of-service. Callers use `let _ =`, and the
/// integrity check is what notices a gap.
pub fn record(
    store: &Store,
    actor: Actor,
    action: &str,
    scope: Option<&str>,
    result: Outcome,
    detail: Detail,
) -> Result<i64> {
    let detail = serde_json::to_string(&detail.sanitised().0)?;
    let actor_text = actor.as_text();

    let (prev_seq, prev_hash): (i64, String) = store
        .conn()
        .query_row(
            "SELECT seq, hash FROM audit_events ORDER BY seq DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .unwrap_or((0, GENESIS.to_string()));

    let seq = prev_seq + 1;
    let at: String = store
        .conn()
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| row.get(0))?;

    let hash = hash_of(
        seq,
        &at,
        &actor_text,
        action,
        scope,
        result.as_text(),
        &detail,
        &prev_hash,
    );

    store.conn().execute(
        "INSERT INTO audit_events (seq, at, actor, action, scope, result, detail, prev_hash, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![seq, at, actor_text, action, scope, result.as_text(), detail, prev_hash, hash],
    )?;

    Ok(seq)
}

/// The most recent events, newest first.
pub fn recent(store: &Store, limit: i64) -> Result<Vec<Event>> {
    let mut statement = store.conn().prepare(
        "SELECT seq, at, actor, action, scope, result, detail, hash
         FROM audit_events ORDER BY seq DESC LIMIT ?1",
    )?;

    let rows = statement.query_map(params![limit.clamp(1, 1000)], |row| {
        Ok(Event {
            seq: row.get(0)?,
            at: row.get(1)?,
            actor: Actor::parse(&row.get::<_, String>(2)?),
            action: row.get(3)?,
            scope: row.get(4)?,
            result: if row.get::<_, String>(5)? == "denied" {
                Outcome::Denied
            } else {
                Outcome::Allowed
            },
            detail: serde_json::from_str(&row.get::<_, String>(6)?)
                .unwrap_or(serde_json::Value::Null),
            hash: row.get(7)?,
        })
    })?;

    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[derive(Debug, Serialize)]
pub struct Integrity {
    pub intact: bool,
    pub checked: i64,
    /// The first row whose hash does not match, if any. Everything after it is suspect too.
    pub broken_at: Option<i64>,
}

/// Recompute the whole chain.
///
/// Walks in order, rebuilding each hash from the row's own fields and the previous hash. The first
/// mismatch is reported and the walk stops: everything after a broken link is unverifiable anyway,
/// and a list of a thousand "also broken" lines hides the one that matters.
pub fn verify(store: &Store) -> Result<Integrity> {
    let mut statement = store.conn().prepare(
        "SELECT seq, at, actor, action, scope, result, detail, prev_hash, hash
         FROM audit_events ORDER BY seq",
    )?;

    let mut rows = statement.query([])?;
    let mut expected_prev = GENESIS.to_string();
    let mut checked = 0;

    while let Some(row) = rows.next()? {
        let seq: i64 = row.get(0)?;
        let at: String = row.get(1)?;
        let actor: String = row.get(2)?;
        let action: String = row.get(3)?;
        let scope: Option<String> = row.get(4)?;
        let result: String = row.get(5)?;
        let detail: String = row.get(6)?;
        let prev_hash: String = row.get(7)?;
        let hash: String = row.get(8)?;

        let recomputed = hash_of(
            seq,
            &at,
            &actor,
            &action,
            scope.as_deref(),
            &result,
            &detail,
            &prev_hash,
        );

        if prev_hash != expected_prev || recomputed != hash {
            return Ok(Integrity { intact: false, checked, broken_at: Some(seq) });
        }

        expected_prev = hash;
        checked += 1;
    }

    Ok(Integrity { intact: true, checked, broken_at: None })
}

/// Convenience for the many callers that only care that the write happened.
pub fn note(store: &Store, action: &str, scope: Option<&str>, detail: Detail) {
    let _ = record(store, Actor::You, action, scope, Outcome::Allowed, detail);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("vault.db")).unwrap();

        (dir, store)
    }

    #[test]
    fn an_empty_trail_is_intact() {
        let (_dir, store) = scratch();
        let integrity = verify(&store).unwrap();

        assert!(integrity.intact);
        assert_eq!(integrity.checked, 0);
    }

    #[test]
    fn events_come_back_newest_first() {
        let (_dir, store) = scratch();

        note(&store, action::UNLOCK, None, Detail::new());
        note(&store, action::RECORD_CREATED, Some("abc"), Detail::new().with("kind", "login"));

        let events = recent(&store, 10).unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].action, action::RECORD_CREATED);
        assert_eq!(events[0].scope.as_deref(), Some("abc"));
        assert_eq!(events[0].detail["kind"], "login");
        assert!(verify(&store).unwrap().intact);
    }

    #[test]
    fn a_denial_is_recorded_like_anything_else() {
        let (_dir, store) = scratch();

        record(
            &store,
            Actor::Agent { process: "claude".into(), pid: 4242 },
            action::SECRET_REVEALED,
            Some("rec"),
            Outcome::Denied,
            Detail::new(),
        )
        .unwrap();

        let events = recent(&store, 1).unwrap();

        assert_eq!(events[0].result, Outcome::Denied);
        assert_eq!(
            events[0].actor,
            Actor::Agent { process: "claude".into(), pid: 4242 }
        );
    }

    #[test]
    fn editing_a_row_breaks_the_chain_at_that_row() {
        let (_dir, store) = scratch();

        for _ in 0..5 {
            note(&store, action::UNLOCK, None, Detail::new());
        }

        assert!(verify(&store).unwrap().intact);

        store
            .conn()
            .execute("UPDATE audit_events SET action = 'vault.lock' WHERE seq = 3", [])
            .unwrap();

        let integrity = verify(&store).unwrap();

        assert!(!integrity.intact);
        assert_eq!(integrity.broken_at, Some(3));
        assert_eq!(integrity.checked, 2, "it should have verified the rows before the break");
    }

    #[test]
    fn removing_a_row_breaks_the_chain() {
        let (_dir, store) = scratch();

        for _ in 0..4 {
            note(&store, action::UNLOCK, None, Detail::new());
        }

        store.conn().execute("DELETE FROM audit_events WHERE seq = 2", []).unwrap();

        let integrity = verify(&store).unwrap();

        assert!(!integrity.intact, "a deleted line went unnoticed");
        assert_eq!(integrity.broken_at, Some(3));
    }

    #[test]
    fn a_secret_never_reaches_the_trail_even_if_a_caller_passes_one() {
        let (_dir, store) = scratch();

        note(
            &store,
            action::SECRET_REVEALED,
            Some("rec"),
            Detail::new()
                .with("field", "Senha")
                .with("password", "hunter2")
                .with("valor", "hunter2")
                .with("api_token", "abc123")
                .with("chave_privada", "-----BEGIN")
                .with("count", 1),
        );

        let events = recent(&store, 1).unwrap();
        let rendered = serde_json::to_string(&events[0].detail).unwrap();

        // The label of the field survives; every value-bearing key is gone.
        assert_eq!(events[0].detail["field"], "Senha");
        assert_eq!(events[0].detail["count"], 1);
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(!rendered.contains("abc123"), "{rendered}");
        assert!(!rendered.contains("BEGIN"), "{rendered}");
    }

    #[test]
    fn two_rows_cannot_collide_by_shifting_a_character_between_fields() {
        let (_dir, store) = scratch();

        note(&store, "ab", Some("c"), Detail::new());
        note(&store, "a", Some("bc"), Detail::new());

        let events = recent(&store, 2).unwrap();

        assert_ne!(events[0].hash, events[1].hash);
        assert!(verify(&store).unwrap().intact);
    }
}
