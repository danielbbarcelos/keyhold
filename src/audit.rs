//! The trail: what was done, when, and by whom.
//!
//! **Why a vault needs one at all.** Cryptography answers what somebody *can* open. It cannot
//! answer what they *did* open, and that second question is the only one that matters after a
//! laptop is left unlocked, or once an agent is allowed to read things (F7).
//!
//! **The private half of an event is sealed.** Who did it, what it was about, and the detail go
//! into one blob under the vault key; only the time, the action, the outcome and the chain stay in
//! the clear. This was not the original design — the trail was readable without a key on the
//! argument that a log you cannot consult while locked is no use when the vault is the problem —
//! and the argument did not survive looking at the file: `strings vault.db` printed which field of
//! which record had been read, and by what. A field's label is content, not shape. What is left
//! outside still shows that something was revealed at 21:34 and that nothing has been edited since,
//! and the route that reads the trail already required an open vault, so the interface lost nothing.
//!
//! An event recorded while the vault is closed — a failed unlock — cannot be sealed. Those stay in
//! the clear, and by construction they name nothing: an attempt counter and no scope at all.
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

use crate::crypto::{self, SecretKey};
use crate::db::Store;
use crate::lock::Lock;

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
    pub const RECORD_MOVED: &str = "record.moved";
    pub const RECORD_DELETED: &str = "record.deleted";
    pub const RECORD_DESTROYED: &str = "record.destroyed";
    pub const RECORD_RESTORED: &str = "record.restored";
    pub const RECORD_COPIED: &str = "record.copied";
    pub const SECRET_REVEALED: &str = "secret.revealed";
    pub const SECRET_COPIED: &str = "secret.copied";
    pub const VAULT_CREATED: &str = "vault.created";
    pub const VAULT_DELETED: &str = "vault.deleted";
    pub const VAULT_ROTATED: &str = "vault.rotated";
    pub const EXPORTED: &str = "vault.exported";
    pub const IMPORTED: &str = "vault.imported";
    pub const INSTANCE_CLOSED: &str = "instance.closed";
    pub const AGENT_GRANTED: &str = "agent.granted";
    pub const AUDIT_RESEALED: &str = "audit.resealed";
    pub const AGENT_GRANT_REVOKED: &str = "agent.grant.revoked";
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
/// The half of an event that says something about the vault.
#[derive(Serialize, Deserialize)]
struct Private {
    actor: String,
    scope: Option<String>,
    detail: String,
}

fn aad(seq: i64) -> Vec<u8> {
    // The sequence number is in the additional data, so a sealed blob cannot be moved to another
    // line of the log — which is the one edit the chain alone would not notice, since both hashes
    // would be recomputed by whoever moved it.
    format!("keyhold:audit:v1|{seq}").into_bytes()
}

/// Write a line.
///
/// The lock rather than the key: this is called from both sides of it — from handlers that have
/// just done something with the vault open, and from the one that records a failed unlock, where
/// there is no key and never will be for that event.
///
/// **Never call this from inside `Lock::with_key`.** That mutex is not reentrant, and taking it
/// twice on one thread waits forever for itself.
pub fn record(
    store: &Store,
    lock: &Lock,
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

    let private = Private {
        actor: actor_text.clone(),
        scope: scope.map(str::to_string),
        detail: detail.clone(),
    };

    let sealed = lock
        .with_key(|key| seal_private(key, seq, &private))
        .transpose()?;

    let hash = match &sealed {
        Some(sealed) => hash_sealed(seq, &at, action, result.as_text(), sealed, &prev_hash),
        None => hash_of(seq, &at, &actor_text, action, scope, result.as_text(), &detail, &prev_hash),
    };

    match sealed {
        Some(sealed) => store.conn().execute(
            "INSERT INTO audit_events
                 (seq, at, actor, action, scope, result, detail, prev_hash, hash, sealed)
             VALUES (?1, ?2, '', ?3, NULL, ?4, NULL, ?5, ?6, ?7)",
            params![seq, at, action, result.as_text(), prev_hash, hash, sealed],
        )?,
        None => store.conn().execute(
            "INSERT INTO audit_events
                 (seq, at, actor, action, scope, result, detail, prev_hash, hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![seq, at, actor_text, action, scope, result.as_text(), detail, prev_hash, hash],
        )?,
    };

    Ok(seq)
}

fn seal_private(key: &SecretKey, seq: i64, private: &Private) -> Result<Vec<u8>> {
    crypto::seal(key, &serde_json::to_vec(private)?, &aad(seq))
}

fn open_private(key: &SecretKey, seq: i64, sealed: &[u8]) -> Option<Private> {
    serde_json::from_slice(&crypto::open(key, sealed, &aad(seq)).ok()?).ok()
}

/// The hash of a line whose private half is sealed.
///
/// It covers the ciphertext rather than what is inside it, so the chain can still be verified
/// without the key — the one thing about the trail that is genuinely better read while locked.
fn hash_sealed(
    seq: i64,
    at: &str,
    action: &str,
    result: &str,
    sealed: &[u8],
    prev_hash: &str,
) -> String {
    let mut hasher = Sha256::new();

    for field in [seq.to_string().as_bytes(), at.as_bytes(), action.as_bytes(), result.as_bytes(), sealed, prev_hash.as_bytes()]
    {
        hasher.update(field.len().to_string().as_bytes());
        hasher.update(b"\n");
        hasher.update(field);
    }

    format!("{:x}", hasher.finalize())
}

/// The most recent events, newest first.
pub fn recent(store: &Store, key: &SecretKey, limit: i64) -> Result<Vec<Event>> {
    let mut statement = store.conn().prepare(
        "SELECT seq, at, actor, action, scope, result, detail, hash, sealed
         FROM audit_events ORDER BY seq DESC LIMIT ?1",
    )?;

    let rows = statement.query_map(params![limit.clamp(1, 1000)], |row| {
        let seq: i64 = row.get(0)?;
        let sealed: Option<Vec<u8>> = row.get(8)?;

        // A line whose blob will not open is shown for what it is rather than dropped: a trail that
        // quietly omits what it cannot read is worse than one that says it cannot read it.
        let private = match &sealed {
            Some(sealed) => open_private(key, seq, sealed).unwrap_or(Private {
                actor: String::new(),
                scope: None,
                detail: "{\"unreadable\":true}".into(),
            }),
            None => Private {
                actor: row.get(2)?,
                scope: row.get(4)?,
                detail: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "{}".into()),
            },
        };

        Ok(Event {
            seq,
            at: row.get(1)?,
            actor: Actor::parse(&private.actor),
            action: row.get(3)?,
            scope: private.scope,
            result: if row.get::<_, String>(5)? == "denied" {
                Outcome::Denied
            } else {
                Outcome::Allowed
            },
            detail: serde_json::from_str(&private.detail).unwrap_or(serde_json::Value::Null),
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
        "SELECT seq, at, actor, action, scope, result, detail, prev_hash, hash, sealed
         FROM audit_events ORDER BY seq",
    )?;

    let mut rows = statement.query([])?;
    let mut expected_prev = GENESIS.to_string();
    let mut checked = 0;

    while let Some(row) = rows.next()? {
        let seq: i64 = row.get(0)?;
        let at: String = row.get(1)?;
        let actor: String = row.get::<_, Option<String>>(2)?.unwrap_or_default();
        let action: String = row.get(3)?;
        let scope: Option<String> = row.get(4)?;
        let result: String = row.get(5)?;
        let detail: Option<String> = row.get(6)?;
        let prev_hash: String = row.get(7)?;
        let hash: String = row.get(8)?;
        let sealed: Option<Vec<u8>> = row.get(9)?;

        let recomputed = match &sealed {
            Some(sealed) => hash_sealed(seq, &at, &action, &result, sealed, &prev_hash),
            None => hash_of(
                seq,
                &at,
                &actor,
                &action,
                scope.as_deref(),
                &result,
                &detail.unwrap_or_default(),
                &prev_hash,
            ),
        };

        if prev_hash != expected_prev || recomputed != hash {
            return Ok(Integrity { intact: false, checked, broken_at: Some(seq) });
        }

        expected_prev = hash;
        checked += 1;
    }

    Ok(Integrity { intact: true, checked, broken_at: None })
}

/// Convenience for the many callers that only care that the write happened.
pub fn note(store: &Store, lock: &Lock, action: &str, scope: Option<&str>, detail: Detail) {
    let _ = record(store, lock, Actor::You, action, scope, Outcome::Allowed, detail);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trail with a key behind it, which is the only way most of these lines get written.
    fn scratch() -> (tempfile::TempDir, Store, Lock) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("vault.db")).unwrap();
        let lock = Lock::new(crate::lock::LockPolicy { idle_seconds: None, hidden_seconds: None });

        lock.unlock_with(SecretKey::random());

        (dir, store, lock)
    }

    /// The key the fixture's lock is holding, for the calls that take one directly.
    fn key_of(lock: &Lock) -> SecretKey {
        lock.with_key(|key| SecretKey::from_bytes(*key.as_bytes())).unwrap()
    }

    #[test]
    fn nothing_private_is_left_in_the_clear() {
        let (_dir, store, lock) = scratch();

        note(
            &store,
            &lock,
            action::SECRET_REVEALED,
            Some("REGISTRO-SECRETO"),
            Detail::new().with("field", "Senha"),
        );

        // The columns a `sqlite3` session would print.
        let (actor, scope, detail): (String, Option<String>, Option<String>) = store
            .conn()
            .query_row("SELECT actor, scope, detail FROM audit_events", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();

        assert_eq!(actor, "");
        assert_eq!(scope, None, "the record's id is not shape, it is what was looked at");
        assert_eq!(detail, None);

        let sealed: Vec<u8> = store
            .conn()
            .query_row("SELECT sealed FROM audit_events", [], |row| row.get(0))
            .unwrap();

        let raw = String::from_utf8_lossy(&sealed);
        assert!(!raw.contains("Senha"), "the field's label is in the clear");
        assert!(!raw.contains("REGISTRO-SECRETO"), "the record's id is in the clear");

        // And it all comes back for somebody holding the key.
        let events = recent(&store, &key_of(&lock), 10).unwrap();
        assert_eq!(events[0].scope.as_deref(), Some("REGISTRO-SECRETO"));
        assert_eq!(events[0].detail["field"], "Senha");
    }

    #[test]
    fn a_sealed_line_cannot_be_moved_to_another_place_in_the_log() {
        let (_dir, store, lock) = scratch();

        note(&store, &lock, action::UNLOCK, None, Detail::new());
        note(&store, &lock, action::SECRET_REVEALED, Some("um"), Detail::new());

        // Swapping two blobs is the edit the chain alone would miss, because whoever swapped them
        // would recompute both hashes. The sequence number is in the additional data, so the blob
        // simply does not open in its new place.
        let first: Vec<u8> = store
            .conn()
            .query_row("SELECT sealed FROM audit_events WHERE seq = 1", [], |row| row.get(0))
            .unwrap();

        store
            .conn()
            .execute("UPDATE audit_events SET sealed = ?1 WHERE seq = 2", params![first])
            .unwrap();

        let events = recent(&store, &key_of(&lock), 10).unwrap();
        let moved = events.iter().find(|event| event.seq == 2).unwrap();

        assert_eq!(moved.detail["unreadable"], true);
    }

    #[test]
    fn an_event_written_with_the_vault_closed_stays_readable_and_says_nothing() {
        let (_dir, store, lock) = scratch();

        lock.lock();
        record(
            &store,
            &lock,
            Actor::You,
            action::UNLOCK_FAILED,
            None,
            Outcome::Denied,
            Detail::new().with("attempt", 2i64),
        )
        .unwrap();

        let sealed: Option<Vec<u8>> = store
            .conn()
            .query_row("SELECT sealed FROM audit_events", [], |row| row.get(0))
            .unwrap();

        assert!(sealed.is_none(), "there was no key to seal it with");
        assert!(verify(&store).unwrap().intact, "an unsealed line still chains");
    }

    #[test]
    fn resealing_closes_the_plaintext_era_and_leaves_the_chain_whole() {
        let (_dir, store, lock) = scratch();

        // What an older build wrote: the private half in plain columns, hashed the old way.
        lock.lock();
        note(&store, &lock, action::SECRET_REVEALED, Some("antigo"), Detail::new().with("field", "Senha"));
        assert!(verify(&store).unwrap().intact);

        store
            .conn()
            .execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('audit_plaintext_through', '1')",
                [],
            )
            .unwrap();

        let key = SecretKey::random();
        assert_eq!(reseal(&store, &key).unwrap(), 1);

        let left: Option<String> = store
            .conn()
            .query_row("SELECT detail FROM audit_events WHERE seq = 1", [], |row| row.get(0))
            .unwrap();

        assert_eq!(left, None, "the old line still names the field");
        assert!(verify(&store).unwrap().intact, "the rewritten chain must verify");

        // It says out loud that it happened.
        let events = recent(&store, &key, 10).unwrap();
        assert_eq!(events[0].action, action::AUDIT_RESEALED);
        assert_eq!(events[0].detail["lines"], 1);

        // And it never runs twice.
        assert_eq!(reseal(&store, &key).unwrap(), 0);
    }

    #[test]
    fn an_empty_trail_is_intact() {
        let (_dir, store, lock) = scratch();
        let integrity = verify(&store).unwrap();

        assert!(integrity.intact);
        assert_eq!(integrity.checked, 0);
    }

    #[test]
    fn events_come_back_newest_first() {
        let (_dir, store, lock) = scratch();

        note(&store, &lock, action::UNLOCK, None, Detail::new());
        note(&store, &lock, action::RECORD_CREATED, Some("abc"), Detail::new().with("kind", "login"));

        let events = recent(&store, &key_of(&lock), 10).unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].action, action::RECORD_CREATED);
        assert_eq!(events[0].scope.as_deref(), Some("abc"));
        assert_eq!(events[0].detail["kind"], "login");
        assert!(verify(&store).unwrap().intact);
    }

    #[test]
    fn a_denial_is_recorded_like_anything_else() {
        let (_dir, store, lock) = scratch();

        record(
            &store,
            &lock,
            Actor::Agent { process: "claude".into(), pid: 4242 },
            action::SECRET_REVEALED,
            Some("rec"),
            Outcome::Denied,
            Detail::new(),
        )
        .unwrap();

        let events = recent(&store, &key_of(&lock), 1).unwrap();

        assert_eq!(events[0].result, Outcome::Denied);
        assert_eq!(
            events[0].actor,
            Actor::Agent { process: "claude".into(), pid: 4242 }
        );
    }

    #[test]
    fn editing_a_row_breaks_the_chain_at_that_row() {
        let (_dir, store, lock) = scratch();

        for _ in 0..5 {
            note(&store, &lock, action::UNLOCK, None, Detail::new());
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
        let (_dir, store, lock) = scratch();

        for _ in 0..4 {
            note(&store, &lock, action::UNLOCK, None, Detail::new());
        }

        store.conn().execute("DELETE FROM audit_events WHERE seq = 2", []).unwrap();

        let integrity = verify(&store).unwrap();

        assert!(!integrity.intact, "a deleted line went unnoticed");
        assert_eq!(integrity.broken_at, Some(3));
    }

    #[test]
    fn a_secret_never_reaches_the_trail_even_if_a_caller_passes_one() {
        let (_dir, store, lock) = scratch();

        note(
            &store,
            &lock,
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

        let events = recent(&store, &key_of(&lock), 1).unwrap();
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
        let (_dir, store, lock) = scratch();

        note(&store, &lock, "ab", Some("c"), Detail::new());
        note(&store, &lock, "a", Some("bc"), Detail::new());

        let events = recent(&store, &key_of(&lock), 2).unwrap();

        assert_ne!(events[0].hash, events[1].hash);
        assert!(verify(&store).unwrap().intact);
    }
}

/// Where the plaintext era ends, if it has not been closed yet.
const PLAINTEXT_MARK: &str = "audit_plaintext_through";

/// Seal the lines written before the trail learned to seal them, and rewrite the chain over them.
///
/// Called on unlock, because it is the first moment there is a key. Until it runs, those lines say
/// in the clear which field of which record was read — which is the whole reason for this.
///
/// **It rewrites hashes**, which is not a thing this module does anywhere else. The alternative was
/// leaving the leak, and the trade is stated rather than hidden: the chain over the re-sealed
/// stretch is recomputed, and the very next line records that it happened and how many it covered.
/// Nothing is deleted, nothing is reordered, and every line keeps its sequence number — a reader
/// can see exactly where the seam is.
pub fn reseal(store: &Store, key: &SecretKey) -> Result<usize> {
    let through: i64 = match store
        .conn()
        .query_row("SELECT value FROM meta WHERE key = ?1", params![PLAINTEXT_MARK], |row| {
            row.get::<_, String>(0)
        })
        .optional()?
    {
        Some(raw) => raw.parse().unwrap_or(0),
        None => return Ok(0),
    };

    if through == 0 {
        store.conn().execute("DELETE FROM meta WHERE key = ?1", params![PLAINTEXT_MARK])?;

        return Ok(0);
    }

    // All of it or none of it. Half a re-seal is a chain that no longer verifies, and the next
    // thing somebody would do about that is stop trusting the trail.
    let transaction = store.conn().unchecked_transaction()?;

    // Seal the private half of every line up to the mark.
    let mut sealed_count = 0;

    let rows: Vec<(i64, String, Option<String>, Option<String>)> = {
        let mut statement = store.conn().prepare(
            "SELECT seq, actor, scope, detail FROM audit_events
             WHERE seq <= ?1 AND sealed IS NULL ORDER BY seq",
        )?;

        let collected = statement
            .query_map(params![through], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        collected
    };

    for (seq, actor, scope, detail) in rows {
        let private = Private { actor, scope, detail: detail.unwrap_or_else(|| "{}".into()) };
        let sealed = seal_private(key, seq, &private)?;

        store.conn().execute(
            "UPDATE audit_events SET actor = '', scope = NULL, detail = NULL, sealed = ?2
             WHERE seq = ?1",
            params![seq, sealed],
        )?;

        sealed_count += 1;
    }

    rechain(store)?;

    store.conn().execute("DELETE FROM meta WHERE key = ?1", params![PLAINTEXT_MARK])?;

    transaction.commit()?;

    if sealed_count > 0 {
        // Recorded through the sealed path like anything else, and it lands after the seam so its
        // own hash is over the rewritten chain.
        let detail = serde_json::to_string(&Detail::new().with("lines", sealed_count as i64).sanitised().0)?;
        append_sealed(store, key, action::AUDIT_RESEALED, Outcome::Allowed, &Private {
            actor: Actor::You.as_text(),
            scope: None,
            detail,
        })?;
    }

    Ok(sealed_count as usize)
}

/// Recompute every `prev_hash` and `hash` in sequence order.
fn rechain(store: &Store) -> Result<()> {
    let rows: Vec<(i64, String, String, String, String, Option<String>, Option<String>, Option<Vec<u8>>)> = {
        let mut statement = store.conn().prepare(
            "SELECT seq, at, action, result, actor, scope, detail, sealed
             FROM audit_events ORDER BY seq",
        )?;

        let collected = statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        collected
    };

    let mut prev_hash = GENESIS.to_string();

    for (seq, at, action, result, actor, scope, detail, sealed) in rows {
        let hash = match &sealed {
            Some(sealed) => hash_sealed(seq, &at, &action, &result, sealed, &prev_hash),
            None => hash_of(
                seq,
                &at,
                &actor,
                &action,
                scope.as_deref(),
                &result,
                &detail.unwrap_or_default(),
                &prev_hash,
            ),
        };

        store.conn().execute(
            "UPDATE audit_events SET prev_hash = ?2, hash = ?3 WHERE seq = ?1",
            params![seq, &prev_hash, &hash],
        )?;

        prev_hash = hash;
    }

    Ok(())
}

/// Append one line whose private half is already assembled. Only [`reseal`] needs this.
fn append_sealed(
    store: &Store,
    key: &SecretKey,
    action: &str,
    result: Outcome,
    private: &Private,
) -> Result<()> {
    let (prev_seq, prev_hash): (i64, String) = store
        .conn()
        .query_row("SELECT seq, hash FROM audit_events ORDER BY seq DESC LIMIT 1", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()?
        .unwrap_or((0, GENESIS.to_string()));

    let seq = prev_seq + 1;
    let at: String = store
        .conn()
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| row.get(0))?;

    let sealed = seal_private(key, seq, private)?;
    let hash = hash_sealed(seq, &at, action, result.as_text(), &sealed, &prev_hash);

    store.conn().execute(
        "INSERT INTO audit_events (seq, at, actor, action, scope, result, detail, prev_hash, hash, sealed)
         VALUES (?1, ?2, '', ?3, NULL, ?4, NULL, ?5, ?6, ?7)",
        params![seq, at, action, result.as_text(), prev_hash, hash, sealed],
    )?;

    Ok(())
}
