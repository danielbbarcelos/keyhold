//! Standing permissions: what an agent may take without asking again, and for how long.
//!
//! # Why these are on disk
//!
//! They used to live in memory, five minutes at a time, and die with the process. That was the
//! honest default while it was the only window on offer — and it was the wrong default for the way
//! this is used. An assistant that has to be re-approved every few minutes is one somebody stops
//! using, and a permission nobody is willing to grant protects nothing at all.
//!
//! So a permission can now outlive the process, and the cost of that is stated where it is granted
//! rather than hidden here: a standing permission is a decision to stop being asked. Three things
//! keep it from being a decision made once and forgotten:
//!
//!  1. **Every use is still in the trail.** The permission removes the question, never the record
//!     of what was taken.
//!  2. **They are listed and revocable**, one by one, with what each covers and when it ends.
//!  3. **A locked vault ignores all of them.** Nothing here is reachable without the key, which is
//!     also why the rows are sealed.
//!
//! # What a permission covers
//!
//! One field of one record, for one kind of use (reading it, copying it, or running one program
//! with it). If the person chose "todos os campos deste registro", every field of that one record,
//! still for that one use. Never a vault, never "everything this process asks for": the answer
//! means what the question showed.
//!
//! # The three windows
//!
//! [`Scope::WhileRunning`] is the one worth understanding. It binds to a process — in practice the
//! assistant itself rather than the bridge it talks through — and ends when that process does. A
//! pid alone would not do: pids are reused, so the process's start time is recorded with it and
//! checked on every use. A different process that inherits the number inherits nothing else.

use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::crypto::{self, SecretKey};
use crate::db::Store;
use crate::identity::now_seconds;
use crate::vaults::new_id;

/// How long a permission lasts.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Scope {
    /// Until a moment on the clock.
    Until { at: u64 },
    /// While one process is still running. `since` is its start time, which is what tells a
    /// recycled pid apart from the process that was granted anything.
    WhileRunning { pid: i64, since: u64, process: String },
    /// No end. The only window that has to be revoked to stop.
    Always,
}

impl Scope {
    /// Whether this window is still open.
    pub fn open(&self) -> bool {
        match self {
            Scope::Always => true,
            Scope::Until { at } => *at > now_seconds(),
            Scope::WhileRunning { pid, since, .. } => started_at(*pid) == Some(*since),
        }
    }
}

/// A permission as the interface lists it.
#[derive(Clone, Debug, Serialize)]
pub struct Grant {
    pub id: String,
    pub record_id: String,
    /// The record's title, opened for the list. Absent if the record is gone.
    pub record_title: String,
    /// The field's label, or [`ALL_FIELDS`] when the permission covers the whole record.
    pub field: String,
    /// `get`, `copy` or `run`: what the permission lets happen to the value.
    pub intent: String,
    /// For `run`, the program the permission is for.
    pub program: Option<String>,
    pub scope: Scope,
    pub created_at: String,
}

/// What a permission is for: which field, to do what with it.
///
/// A yes to reading a host name is not a yes to running a command with the password, and a yes to
/// `mysql` is not a yes to `sh`. The permission means what the question showed, so the question's
/// intent and program are part of it.
#[derive(Clone, Debug)]
pub struct Cover<'a> {
    pub field: &'a str,
    pub all_fields: bool,
    pub intent: &'a str,
    pub program: Option<&'a str>,
}

/// The sealed half: everything that would say too much in a plain column.
#[derive(Serialize, Deserialize)]
struct Sealed {
    field: String,
    #[serde(default)]
    all_fields: bool,
    intent: String,
    #[serde(default)]
    program: Option<String>,
    scope: Scope,
}

impl Sealed {
    fn same_cover(&self, cover: &Cover) -> bool {
        self.all_fields == cover.all_fields
            && (self.all_fields || self.field == cover.field)
            && self.intent == cover.intent
            && self.program.as_deref() == cover.program
    }
}

/// The record is part of what is authenticated: a row moved to another record does not open, so a
/// permission cannot be re-pointed by editing the file.
fn aad(id: &str, record_id: &str) -> Vec<u8> {
    format!("keyhold:agent-grant:v2|{id}|{record_id}").into_bytes()
}

/// How a permission for every field shows in a list. Only a label: what decides is a flag sealed
/// with the permission, so a field that happens to be called this gets nothing from it.
pub const ALL_FIELDS: &str = "*";

/// Record a permission to read one field. See [`grant_for`] for the general form.
pub fn grant(store: &Store, key: &SecretKey, record_id: &str, field: &str, scope: Scope) -> Result<String> {
    grant_for(store, key, record_id, &Cover { field, all_fields: false, intent: "get", program: None }, scope)
}

/// Record a permission, replacing whatever stood for the same thing.
///
/// Replacing rather than adding: two windows over one field is a question about which one wins, and
/// the answer somebody just gave is the one they meant.
pub fn grant_for(
    store: &Store,
    key: &SecretKey,
    record_id: &str,
    cover: &Cover,
    scope: Scope,
) -> Result<String> {
    for (id, sealed) in sealed_for(store, record_id)? {
        if open_one(key, &id, record_id, &sealed).map(|one| one.same_cover(cover)).unwrap_or(false) {
            store.conn().execute("DELETE FROM agent_grants WHERE id = ?1", params![&id])?;
        }
    }

    let id = new_id();
    let body = serde_json::to_vec(&Sealed {
        field: if cover.all_fields { String::new() } else { cover.field.to_string() },
        all_fields: cover.all_fields,
        intent: cover.intent.to_string(),
        program: cover.program.map(str::to_string),
        scope,
    })?;
    let sealed = crypto::seal(key, &body, &aad(&id, record_id))?;

    store.conn().execute(
        "INSERT INTO agent_grants (id, record_id, sealed, created_at)
         VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        params![&id, record_id, &sealed],
    )?;

    Ok(id)
}

/// Whether a standing permission lets this field be read right now.
pub fn allows(store: &Store, key: &SecretKey, record_id: &str, field: &str) -> bool {
    allows_use(store, key, record_id, field, "get", None)
}

/// Whether a standing permission covers this use of this field right now.
///
/// A window that has closed is deleted on the way past. Cleaning up here rather than on a timer
/// means it happens exactly when it matters and never while the vault is locked.
pub fn allows_use(
    store: &Store,
    key: &SecretKey,
    record_id: &str,
    field: &str,
    intent: &str,
    program: Option<&str>,
) -> bool {
    let found = match sealed_for(store, record_id) {
        Ok(found) => found,
        Err(_) => return false,
    };

    let rows = found
        .into_iter()
        .filter_map(|(id, sealed)| open_one(key, &id, record_id, &sealed).map(|one| (id, one)))
        .collect::<Vec<_>>();

    let mut allowed = false;

    for (id, one) in rows {
        if one.intent != intent || one.program.as_deref() != program {
            continue;
        }

        if !one.all_fields && one.field != field {
            continue;
        }

        if one.scope.open() {
            allowed = true;
        } else {
            let _ = store.conn().execute("DELETE FROM agent_grants WHERE id = ?1", params![&id]);
        }
    }

    allowed
}

/// Everything standing right now, newest first, with the closed ones swept away.
pub fn list(store: &Store, key: &SecretKey) -> Result<Vec<Grant>> {
    let mut statement = store.conn().prepare(
        "SELECT id, record_id, sealed, created_at FROM agent_grants ORDER BY created_at DESC",
    )?;

    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut standing = Vec::new();

    for (id, record_id, sealed, created_at) in rows {
        let Some(one) = open_one(key, &id, &record_id, &sealed) else {
            continue;
        };

        if !one.scope.open() {
            let _ = store.conn().execute("DELETE FROM agent_grants WHERE id = ?1", params![&id]);

            continue;
        }

        // An AWS record is not in the file: its parameter name stands in for the title.
        let record_title = match crate::aws::mapping::decode_id(&record_id) {
            Some((_, _, name)) => name,
            None => match crate::records::open(store, key, &record_id) {
                Ok(record) => record.payload.title,
                // The record is gone, and so is anything said about it.
                Err(_) => {
                    let _ = store.conn().execute("DELETE FROM agent_grants WHERE id = ?1", params![&id]);

                    continue;
                }
            },
        };

        standing.push(Grant {
            id,
            record_id,
            record_title,
            field: if one.all_fields { ALL_FIELDS.to_string() } else { one.field },
            intent: one.intent,
            program: one.program,
            scope: one.scope,
            created_at,
        });
    }

    Ok(standing)
}

/// How many are standing. The count the settings screen shows.
pub fn count(store: &Store, key: &SecretKey) -> usize {
    list(store, key).map(|standing| standing.len()).unwrap_or(0)
}

pub fn revoke(store: &Store, id: &str) -> Result<bool> {
    let gone = store.conn().execute("DELETE FROM agent_grants WHERE id = ?1", params![id])?;

    Ok(gone > 0)
}

pub fn revoke_all(store: &Store) -> Result<usize> {
    Ok(store.conn().execute("DELETE FROM agent_grants", [])?)
}

fn sealed_for(store: &Store, record_id: &str) -> Result<Vec<(String, Vec<u8>)>> {
    let mut statement =
        store.conn().prepare("SELECT id, sealed FROM agent_grants WHERE record_id = ?1")?;

    let rows = statement
        .query_map(params![record_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(rows)
}

fn open_one(key: &SecretKey, id: &str, record_id: &str, sealed: &[u8]) -> Option<Sealed> {
    let body = crypto::open(key, sealed, &aad(id, record_id)).ok()?;

    serde_json::from_slice(&body).ok()
}

/// When a process started, as the kernel marks it — its identity, alongside its pid.
pub fn started_at(pid: i64) -> Option<u64> {
    crate::procinfo::start_mark(i32::try_from(pid).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::identity::{self, SetupOptions};
    use crate::records::{self, Field, Payload};
    use crate::vaults;

    struct Fixture {
        store: Store,
        key: SecretKey,
        record: String,
        _dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("vault.db")).unwrap();

        let outcome =
            identity::create(&mut store, SetupOptions { password: Some("the password".into()) })
                .unwrap();

        let vault = vaults::create_vault(
            &store,
            &outcome.vault_key,
            vaults::VaultMeta {
                name: "Servidores".into(),
                description: String::new(),
                colour: "default".into(),
                ..Default::default()
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
                fields: vec![Field {
                    label: "Senha".into(),
                    kind: "password".into(),
                    value: "hunter2".into(),
                    ..Default::default()
                }],
                ..Payload::default()
            },
        )
        .unwrap();

        Fixture { store, key: outcome.vault_key, record: record.id, _dir: dir }
    }

    #[test]
    fn a_permission_covers_one_field_and_no_other() {
        let app = fixture();

        grant(&app.store, &app.key, &app.record, "Senha", Scope::Always).unwrap();

        assert!(allows(&app.store, &app.key, &app.record, "Senha"));
        assert!(!allows(&app.store, &app.key, &app.record, "Usuário"));
        assert!(!allows(&app.store, &app.key, "outro-registro", "Senha"));
    }

    #[test]
    fn a_window_that_has_closed_allows_nothing_and_is_swept_away() {
        let app = fixture();

        grant(
            &app.store,
            &app.key,
            &app.record,
            "Senha",
            Scope::Until { at: now_seconds() - 1 },
        )
        .unwrap();

        assert!(!allows(&app.store, &app.key, &app.record, "Senha"));
        assert_eq!(count(&app.store, &app.key), 0, "an expired row should not survive being read");
    }

    #[test]
    fn a_permission_bound_to_a_process_ends_when_it_does() {
        let app = fixture();

        // This process, by its real start time: the window is open.
        let mine = std::process::id() as i64;
        let since = started_at(mine).expect("this process has a start time");

        grant(
            &app.store,
            &app.key,
            &app.record,
            "Senha",
            Scope::WhileRunning { pid: mine, since, process: "teste".into() },
        )
        .unwrap();

        assert!(allows(&app.store, &app.key, &app.record, "Senha"));

        // The same pid with a start time that is not this process's — which is what a recycled pid
        // looks like. Nothing is inherited.
        grant(
            &app.store,
            &app.key,
            &app.record,
            "Senha",
            Scope::WhileRunning { pid: mine, since: since + 1, process: "teste".into() },
        )
        .unwrap();

        assert!(!allows(&app.store, &app.key, &app.record, "Senha"));
    }

    #[test]
    fn a_permission_outlives_the_process_that_granted_it() {
        // The whole reason these moved out of memory: "30 days" and "forever" mean nothing if
        // closing the window forgets them.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");

        let key = {
            let mut store = Store::open(&path).unwrap();
            let outcome =
                identity::create(&mut store, SetupOptions { password: Some("the password".into()) })
                    .unwrap();

            let vault = vaults::create_vault(
                &store,
                &outcome.vault_key,
                vaults::VaultMeta {
                    name: "Servidores".into(),
                    description: String::new(),
                    colour: "default".into(),
                    ..Default::default()
                },
            )
            .unwrap();

            let record = records::create(
                &store,
                &outcome.vault_key,
                &vault.id,
                None,
                "login",
                &Payload { title: "Produção".into(), ..Payload::default() },
            )
            .unwrap();

            grant(&store, &outcome.vault_key, &record.id, "Senha", Scope::Always).unwrap();

            outcome.vault_key
        };

        // A new process opening the same file.
        let store = Store::open(&path).unwrap();
        let standing = list(&store, &key).unwrap();

        assert_eq!(standing.len(), 1);
        assert_eq!(standing[0].field, "Senha");
        assert_eq!(standing[0].record_title, "Produção");
        assert_eq!(standing[0].scope, Scope::Always);
    }

    #[test]
    fn granting_the_same_field_twice_replaces_rather_than_stacks() {
        let app = fixture();

        grant(&app.store, &app.key, &app.record, "Senha", Scope::Always).unwrap();
        grant(
            &app.store,
            &app.key,
            &app.record,
            "Senha",
            Scope::Until { at: now_seconds() + 60 },
        )
        .unwrap();

        let standing = list(&app.store, &app.key).unwrap();

        assert_eq!(standing.len(), 1, "two windows over one field is a question nobody asked");
        assert!(matches!(standing[0].scope, Scope::Until { .. }), "the newer answer should win");
    }

    #[test]
    fn revoking_one_leaves_the_others_alone() {
        let app = fixture();

        let senha = grant(&app.store, &app.key, &app.record, "Senha", Scope::Always).unwrap();
        grant(&app.store, &app.key, &app.record, "Usuário", Scope::Always).unwrap();

        assert!(revoke(&app.store, &senha).unwrap());
        assert!(!allows(&app.store, &app.key, &app.record, "Senha"));
        assert!(allows(&app.store, &app.key, &app.record, "Usuário"));

        assert_eq!(revoke_all(&app.store).unwrap(), 1);
        assert_eq!(count(&app.store, &app.key), 0);
    }

    #[test]
    fn a_permission_is_not_readable_without_the_key() {
        let app = fixture();

        grant(&app.store, &app.key, &app.record, "Senha", Scope::Always).unwrap();

        // The field's label lives inside a record's sealed payload; a table naming it beside a
        // record id would put back exactly what that seal took away.
        let raw: Vec<u8> = app
            .store
            .conn()
            .query_row("SELECT sealed FROM agent_grants LIMIT 1", [], |row| row.get(0))
            .unwrap();

        assert!(
            !String::from_utf8_lossy(&raw).contains("Senha"),
            "the field's label is in the clear on disk"
        );
    }

    #[test]
    fn a_permission_for_every_field_covers_every_field_of_that_record_only() {
        let f = fixture();
        let other = records::create(
            &f.store,
            &f.key,
            &vaults::list_vaults(&f.store, &f.key).unwrap()[0].id,
            None,
            "login",
            &Payload { title: "Outro".into(), fields: vec![Field::new("Senha", "password", "x")], ..Payload::default() },
        )
        .unwrap()
        .id;

        grant_for(&f.store, &f.key, &f.record, &Cover { field: "", all_fields: true, intent: "get", program: None }, Scope::Always).unwrap();

        assert!(allows(&f.store, &f.key, &f.record, "Senha"));
        assert!(allows(&f.store, &f.key, &f.record, "Usuário"));
        assert!(!allows(&f.store, &f.key, &other, "Senha"), "another record is another question");
    }

    #[test]
    fn a_yes_to_reading_is_not_a_yes_to_running_and_one_program_is_not_another() {
        let f = fixture();

        grant(&f.store, &f.key, &f.record, "Senha", Scope::Always).unwrap();
        assert!(!allows_use(&f.store, &f.key, &f.record, "Senha", "run", Some("mysql")));

        let mysql = Cover { field: "Senha", all_fields: false, intent: "run", program: Some("mysql") };
        grant_for(&f.store, &f.key, &f.record, &mysql, Scope::Always).unwrap();

        assert!(allows_use(&f.store, &f.key, &f.record, "Senha", "run", Some("mysql")));
        assert!(!allows_use(&f.store, &f.key, &f.record, "Senha", "run", Some("sh")));
        assert!(!allows_use(&f.store, &f.key, &f.record, "Senha", "copy", None));
    }

    #[test]
    fn a_permission_moved_to_another_record_in_the_file_opens_nothing() {
        let f = fixture();

        grant(&f.store, &f.key, &f.record, "Senha", Scope::Always).unwrap();
        f.store.conn().execute("UPDATE agent_grants SET record_id = 'outro'", []).unwrap();

        assert!(!allows(&f.store, &f.key, "outro", "Senha"));
    }

    #[test]
    fn a_field_called_star_is_only_a_field() {
        let f = fixture();

        grant(&f.store, &f.key, &f.record, ALL_FIELDS, Scope::Always).unwrap();

        assert!(!allows(&f.store, &f.key, &f.record, "Senha"));
    }

    #[test]
    fn a_record_that_is_not_a_row_of_this_file_can_hold_a_permission() {
        let f = fixture();

        grant(&f.store, &f.key, "um-registro-da-aws", "Senha", Scope::Always).unwrap();

        assert!(allows(&f.store, &f.key, "um-registro-da-aws", "Senha"));
    }
}
