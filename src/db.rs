//! The vault file, and the little that F0 puts in it.
//!
//! One SQLite file, opened `0600`, holding ciphertext and the metadata needed to find it. The
//! schema arrives in numbered steps recorded in `meta`, so an older vault opened by a newer binary
//! is migrated forward and a newer vault opened by an older binary is **refused** rather than
//! half-understood — a vault a binary does not fully understand is a vault it can corrupt.
//!
//! Two settings that are not defaults and are not optional here:
//!
//!  - `journal_mode = WAL` keeps a crash from truncating the file, and the `-wal` sidecar is
//!    covered by the same directory mode.
//!  - `secure_delete = ON` makes SQLite overwrite freed pages instead of leaving them for the next
//!    write. Deleting a record should not leave its ciphertext sitting in a free page — even
//!    ciphertext, because "this row existed and was this long" is itself information.

use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::Connection;

/// The schema this binary understands. A file claiming more than this is not opened.
pub const SCHEMA_VERSION: i64 = 10;

pub struct Store {
    conn: Connection,
    dir: std::path::PathBuf,
}

impl Store {
    /// Open (creating if needed) the vault at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            crate::paths::ensure_private_dir(&parent.to_path_buf())
                .with_context(|| format!("could not prepare {}", parent.display()))?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("could not open the vault at {}", path.display()))?;

        restrict_file(path)?;

        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "secure_delete", "ON")?;

        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

        let mut vault = Self { conn, dir };
        vault.migrate()?;

        Ok(vault)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// For the writes that need a transaction. Taking `&mut self` is the point: a transaction and a
    /// stray read on the same connection is how a half-written record gets read as a whole one.
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// A file that belongs to this vault, in the same directory and under the same permissions.
    ///
    /// Deriving it from the vault's own path rather than from the environment means a test, a
    /// second vault or a `--vault` flag all get a consistent set of files without a global to
    /// coordinate.
    pub fn sibling(&self, name: &str) -> std::path::PathBuf {
        self.dir.join(name)
    }

    /// Whether anybody has set this vault up yet.
    pub fn is_initialised(&self) -> bool {
        self.conn
            .query_row("SELECT 1 FROM identity WHERE id = 1", [], |_| Ok(()))
            .is_ok()
    }

    fn migrate(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             ) STRICT;",
        )?;

        let found: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE((SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'schema_version'), 0)",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if found > SCHEMA_VERSION {
            bail!(
                "this vault was written by a newer Keyhold (schema {found}, this build understands \
                 {SCHEMA_VERSION}). Upgrade Keyhold rather than letting an older build write to it."
            );
        }

        // Steps are applied in order and never edited once released: an edited migration is a
        // vault that differs depending on when it was created.
        if found < 1 {
            self.conn.execute_batch(
                "BEGIN;
                 INSERT OR REPLACE INTO meta (key, value) VALUES ('schema_version', '1');
                 INSERT OR REPLACE INTO meta (key, value)
                     VALUES ('created_at', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
                 COMMIT;",
            )?;
        }

        // F1: how this vault is protected, and the settings that govern the lock.
        //
        // One row, forever: `id = 1` with a CHECK constraint. A vault has exactly one identity, and
        // saying so in the schema is cheaper than defending the invariant in every query.
        if found < 2 {
            self.conn.execute_batch(
                "BEGIN;
                 CREATE TABLE identity (
                     id                         INTEGER PRIMARY KEY CHECK (id = 1),
                     protection                 TEXT    NOT NULL,
                     kdf_params                 TEXT    NOT NULL,
                     salt                       BLOB,
                     wrapped_vault_key          BLOB    NOT NULL,
                     recovery_salt              BLOB    NOT NULL,
                     recovery_wrapped_vault_key BLOB    NOT NULL,
                     recovery_confirmed         INTEGER NOT NULL DEFAULT 0,
                     totp_secret                BLOB,
                     created_at                 TEXT    NOT NULL
                 ) STRICT;

                 CREATE TABLE settings (
                     key   TEXT PRIMARY KEY,
                     value TEXT NOT NULL
                 ) STRICT;

                 UPDATE meta SET value = '2' WHERE key = 'schema_version';
                 COMMIT;",
            )?;
        }

        // F2: the vaults inside the file, and the folders inside those.
        //
        // Names are **sealed**, not stored in the clear. On a machine with no server, the threat is
        // somebody reading the file — and a list of vault names ("Banco", "Cliente Andrade") is
        // worth reading even when every password in it is ciphertext. Sorting and searching happen
        // in memory afterwards, which for a local vault costs nothing worth measuring.
        //
        // What the file still shows: how many vaults and folders exist, how they nest, and when
        // they were made. Hiding shape as well would mean giving up the tree.
        if found < 3 {
            self.conn.execute_batch(
                "BEGIN;
                 CREATE TABLE vaults (
                     id           TEXT    PRIMARY KEY,
                     wrapped_key  BLOB    NOT NULL,
                     sealed_meta  BLOB    NOT NULL,
                     agent_access INTEGER NOT NULL DEFAULT 0,
                     position     INTEGER NOT NULL DEFAULT 0,
                     archived_at  TEXT,
                     created_at   TEXT    NOT NULL
                 ) STRICT;

                 CREATE TABLE folders (
                     id          TEXT PRIMARY KEY,
                     vault_id    TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
                     parent_id   TEXT REFERENCES folders(id) ON DELETE CASCADE,
                     wrapped_key BLOB NOT NULL,
                     sealed_meta BLOB NOT NULL,
                     position    INTEGER NOT NULL DEFAULT 0,
                     created_at  TEXT NOT NULL
                 ) STRICT;

                 CREATE INDEX folders_by_vault ON folders (vault_id, parent_id);

                 UPDATE meta SET value = '3' WHERE key = 'schema_version';
                 COMMIT;",
            )?;
        }

        // F3: the records themselves.
        //
        // One sealed blob per record holds everything readable about it — title, username, address,
        // notes and the custom fields in the order somebody arranged them. Only the columns needed
        // to find, sort and flag a record without opening it stay outside: which vault and folder
        // it is in, its kind, its version, and the dates.
        //
        // `version` is in the blob's additional data as well as in this column, which is what stops
        // an old ciphertext from being pasted back over a new one.
        if found < 4 {
            self.conn.execute_batch(
                "BEGIN;
                 CREATE TABLE records (
                     id              TEXT    PRIMARY KEY,
                     vault_id        TEXT    NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
                     folder_id       TEXT    REFERENCES folders(id) ON DELETE SET NULL,
                     kind            TEXT    NOT NULL,
                     wrapped_key     BLOB    NOT NULL,
                     sealed_payload  BLOB    NOT NULL,
                     version         INTEGER NOT NULL DEFAULT 1,
                     favourite       INTEGER NOT NULL DEFAULT 0,
                     rotation_due_at TEXT,
                     compromised_at  TEXT,
                     archived_at     TEXT,
                     created_at      TEXT    NOT NULL,
                     updated_at      TEXT    NOT NULL
                 ) STRICT;

                 CREATE INDEX records_by_vault ON records (vault_id, archived_at);
                 CREATE INDEX records_by_folder ON records (folder_id);

                 CREATE TABLE record_versions (
                     record_id      TEXT    NOT NULL REFERENCES records(id) ON DELETE CASCADE,
                     version        INTEGER NOT NULL,
                     sealed_payload BLOB    NOT NULL,
                     created_at     TEXT    NOT NULL,
                     PRIMARY KEY (record_id, version)
                 ) STRICT;

                 UPDATE meta SET value = '4' WHERE key = 'schema_version';
                 COMMIT;",
            )?;
        }

        // F5: which generation of a vault's key everything inside it is wrapped with.
        //
        // Rotation changes the key and re-wraps what it protects. The number is not used to decrypt
        // anything — the wrapped keys carry that — it exists so the interface can say when a vault
        // was last rotated, and so a half-finished rotation is visible rather than silent.
        if found < 5 {
            self.conn.execute_batch(
                "BEGIN;
                 ALTER TABLE vaults ADD COLUMN key_version INTEGER NOT NULL DEFAULT 1;
                 ALTER TABLE vaults ADD COLUMN key_rotated_at TEXT;
                 UPDATE meta SET value = '5' WHERE key = 'schema_version';
                 COMMIT;",
            )?;
        }

        // F6: the trail.
        //
        // Append-only by construction, not by convention: no `updated_at`, no `deleted_at`, and
        // nothing in the codebase issues an UPDATE or DELETE against it. Each row carries the hash
        // of the one before it, so removing or editing a line breaks every hash after it — which is
        // the only thing that makes a log worth reading back.
        //
        // `seq` is the chain's order and an AUTOINCREMENT on purpose: plain rowids get reused after
        // a delete, and a chain whose positions can repeat is not a chain.
        if found < 6 {
            self.conn.execute_batch(
                "BEGIN;
                 CREATE TABLE audit_events (
                     seq       INTEGER PRIMARY KEY AUTOINCREMENT,
                     at        TEXT NOT NULL,
                     actor     TEXT NOT NULL,
                     action    TEXT NOT NULL,
                     scope     TEXT,
                     result    TEXT NOT NULL,
                     detail    TEXT,
                     prev_hash TEXT NOT NULL,
                     hash      TEXT NOT NULL
                 ) STRICT;

                 CREATE INDEX audit_by_time ON audit_events (at DESC);

                 UPDATE meta SET value = '6' WHERE key = 'schema_version';
                 COMMIT;",
            )?;
        }

        // F6: attachments.
        //
        // The file's name and type are sealed *with* it, not beside it: "id_rsa" and
        // "contrato-assinado.pdf" say plenty on their own. What stays outside the seal is the byte
        // count — a length that any observer could infer from the row's size anyway, and that the
        // interface needs to show a list without opening every file in it.
        //
        // The bytes live in this table rather than in a directory: one file to back up, one file to
        // set permissions on, and no second place where a decrypted temporary could be forgotten.
        if found < 7 {
            self.conn.execute_batch(
                "BEGIN;
                 CREATE TABLE attachments (
                     id          TEXT    PRIMARY KEY,
                     record_id   TEXT    NOT NULL REFERENCES records(id) ON DELETE CASCADE,
                     wrapped_key BLOB    NOT NULL,
                     sealed_meta BLOB    NOT NULL,
                     sealed_blob BLOB    NOT NULL,
                     size        INTEGER NOT NULL,
                     created_at  TEXT    NOT NULL
                 ) STRICT;

                 CREATE INDEX attachments_by_record ON attachments (record_id);

                 UPDATE meta SET value = '7' WHERE key = 'schema_version';
                 COMMIT;",
            )?;
        }

        // F7+: standing permissions for the agent.
        //
        // These used to live in memory and die with the process, which was the honest default while
        // the only window on offer was five minutes. It is the wrong default for the way this is
        // actually used: an assistant that has to be re-approved every few minutes is one somebody
        // stops using, and a permission nobody grants protects nothing.
        //
        // `record_id` is in the clear so that deleting a record takes its permissions with it — ids
        // are already outside the seal everywhere else. The field's label is not: labels live inside
        // a record's sealed payload, and a table listing "Senha" next to a record id would put back
        // what that seal took away. So the label and the window go in one sealed blob.
        if found < 8 {
            self.conn.execute_batch(
                "BEGIN;
                 CREATE TABLE agent_grants (
                     id         TEXT PRIMARY KEY,
                     record_id  TEXT NOT NULL REFERENCES records(id) ON DELETE CASCADE,
                     sealed     BLOB NOT NULL,
                     created_at TEXT NOT NULL
                 ) STRICT;

                 CREATE INDEX agent_grants_by_record ON agent_grants (record_id);

                 UPDATE meta SET value = '8' WHERE key = 'schema_version';
                 COMMIT;",
            )?;
        }

        // F8: the trail stops naming things in the clear.
        //
        // It used to be readable without a key on purpose — "a log you need the vault open to
        // inspect is a log you cannot consult when the vault is the problem". That reasoning did
        // not survive contact with the file: `strings vault.db` printed which fields were read,
        // from which record, by which process. A field's label is content, not shape, and the same
        // argument that seals it inside a record's payload applies to a line about that field.
        //
        // What stays outside: the time, the action, the outcome, and the chain. Enough to see that
        // something was revealed at 21:34 and that the log has not been edited; not enough to know
        // what. And the route that reads the trail already required an open vault, so nothing the
        // interface could do before is lost.
        //
        // The mark records where the plaintext era ends. Rows up to it are re-sealed on the next
        // unlock, when there is a key to do it with; rows after it with no `sealed` are the ones
        // written while the vault was closed, which name nothing.
        if found < 9 {
            self.conn.execute_batch(
                "BEGIN;
                 ALTER TABLE audit_events ADD COLUMN sealed BLOB;

                 INSERT OR REPLACE INTO meta (key, value)
                     VALUES ('audit_plaintext_through',
                             (SELECT COALESCE(MAX(seq), 0) FROM audit_events));

                 UPDATE meta SET value = '9' WHERE key = 'schema_version';
                 COMMIT;",
            )?;
        }

        // Standing permissions for records that are not rows of this file (AWS parameters) could
        // not be written: the table pointed at `records`. It no longer does. The permissions that
        // existed are dropped rather than carried over, because what a permission covers changed
        // with this version (one use, one program) and an old yes was a yes to more than that.
        if found < 10 {
            self.conn.execute_batch(
                "BEGIN;
                 DROP TABLE IF EXISTS agent_grants;

                 CREATE TABLE agent_grants (
                     id         TEXT PRIMARY KEY,
                     record_id  TEXT NOT NULL,
                     sealed     BLOB NOT NULL,
                     created_at TEXT NOT NULL
                 ) STRICT;

                 CREATE INDEX agent_grants_by_record ON agent_grants (record_id);

                 UPDATE meta SET value = '10' WHERE key = 'schema_version';
                 COMMIT;",
            )?;
        }

        Ok(())
    }
}

/// Make the vault file readable and writable by its owner only.
#[cfg(unix)]
fn restrict_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = std::fs::metadata(path)?.permissions();
    if perms.mode() & 0o077 != 0 {
        perms.set_mode(0o600);
        std::fs::set_permissions(path, perms)?;
    }

    Ok(())
}

#[cfg(not(unix))]
fn restrict_file(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_twice_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");

        Store::open(&path).unwrap();
        Store::open(&path).unwrap();

        let conn = Connection::open(&path).unwrap();
        let version: i64 = conn
            .query_row("SELECT CAST(value AS INTEGER) FROM meta WHERE key='schema_version'", [], |r| r.get(0))
            .unwrap();

        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    #[cfg(unix)]
    fn vault_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        Store::open(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "vault file is reachable by somebody else");
    }

    #[test]
    fn a_newer_vault_is_refused_rather_than_migrated_backwards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        Store::open(&path).unwrap();

        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE meta SET value = ?1 WHERE key='schema_version'", [SCHEMA_VERSION + 1])
            .unwrap();
        drop(conn);

        assert!(Store::open(&path).is_err());
    }
}
