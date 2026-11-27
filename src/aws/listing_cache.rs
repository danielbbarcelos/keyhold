//! The last listing of an AWS vault, kept in the file so that opening the vault draws at once.
//!
//! An account with a few hundred parameters takes seconds to list, and the tree waited for it every
//! time the vault was opened. What is kept is exactly what the page was last given for the vault's
//! folders and records: titles, folders, icons, the field shown in lists, marks. Never a secret
//! value, because the page is never given one (see [`crate::records::sealed_json`]), and never the
//! raw parameters, which carry decrypted values.
//!
//! It is sealed with the master key, with the vault and the part in the authenticated data, so the
//! file says only that a listing exists and when it was taken. It is a picture, not a source:
//! nothing that writes to AWS or decides a key reads it, and the page reads AWS again a few seconds
//! after drawing it.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde_json::Value;

use crate::crypto::{self, SecretKey};
use crate::db::Store;
use crate::identity::now_seconds;

/// The parts of a listing that are kept: the answers of the two listing routes.
pub const PARTS: [&str; 2] = ["folders", "records"];

fn aad(vault_id: &str, part: &str) -> Vec<u8> {
    format!("keyhold:aws-listing:v1|{vault_id}|{part}").into_bytes()
}

/// Keep what a listing route answered.
pub fn put(store: &Store, master: &SecretKey, vault_id: &str, part: &str, answer: &Value) -> Result<()> {
    let sealed = crypto::seal(master, &serde_json::to_vec(answer)?, &aad(vault_id, part))?;

    store.conn().execute(
        "INSERT INTO aws_listing_cache (vault_id, part, sealed, synced_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (vault_id, part) DO UPDATE SET sealed = excluded.sealed, synced_at = excluded.synced_at",
        params![vault_id, part, sealed, now_seconds() as i64],
    )?;

    Ok(())
}

/// What was kept for one part, and when (unix seconds).
pub fn get(store: &Store, master: &SecretKey, vault_id: &str, part: &str) -> Option<(Value, u64)> {
    let (sealed, at): (Vec<u8>, i64) = store
        .conn()
        .query_row(
            "SELECT sealed, synced_at FROM aws_listing_cache WHERE vault_id = ?1 AND part = ?2",
            params![vault_id, part],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .ok()
        .flatten()?;

    let body = crypto::open(master, &sealed, &aad(vault_id, part)).ok()?;

    Some((serde_json::from_slice(&body).ok()?, at.max(0) as u64))
}

/// Forget a vault's listing: its connection changed, or its place in the account did.
pub fn forget(store: &Store, vault_id: &str) -> Result<()> {
    store.conn().execute("DELETE FROM aws_listing_cache WHERE vault_id = ?1", params![vault_id])?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store() -> (tempfile::TempDir, Store, SecretKey, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("vault.db")).unwrap();
        let master = SecretKey::random();
        let vault = crate::vaults::create_vault(
            &store,
            &master,
            crate::vaults::VaultMeta { name: "Infra".into(), ..Default::default() },
        )
        .unwrap();

        (dir, store, master, vault.id)
    }

    #[test]
    fn a_listing_comes_back_as_it_was_kept() {
        let (_dir, store, master, id) = store();
        let answer = json!({ "folders": [{ "name": "Bancos" }] });

        assert!(get(&store, &master, &id, "folders").is_none());
        put(&store, &master, &id, "folders", &answer).unwrap();

        let (found, at) = get(&store, &master, &id, "folders").unwrap();
        assert_eq!(found, answer);
        assert!(at > 0);
    }

    #[test]
    fn the_file_does_not_show_what_was_listed() {
        let (_dir, store, master, id) = store();
        put(&store, &master, &id, "records", &json!({ "records": [{ "title": "Postgres produção" }] })).unwrap();

        let sealed: Vec<u8> =
            store.conn().query_row("SELECT sealed FROM aws_listing_cache", [], |row| row.get(0)).unwrap();
        assert!(!String::from_utf8_lossy(&sealed).contains("Postgres"));
    }

    #[test]
    fn a_part_swapped_for_another_does_not_open() {
        let (_dir, store, master, id) = store();
        put(&store, &master, &id, "folders", &json!({ "folders": [] })).unwrap();
        store.conn().execute("UPDATE aws_listing_cache SET part = 'records'", []).unwrap();

        assert!(get(&store, &master, &id, "records").is_none());
    }

    #[test]
    fn forgetting_and_deleting_the_vault_take_it_away() {
        let (_dir, store, master, id) = store();
        put(&store, &master, &id, "folders", &json!({})).unwrap();
        forget(&store, &id).unwrap();
        assert!(get(&store, &master, &id, "folders").is_none());

        put(&store, &master, &id, "folders", &json!({})).unwrap();
        store.conn().execute("DELETE FROM vaults WHERE id = ?1", params![id]).unwrap();
        let left: i64 = store.conn().query_row("SELECT COUNT(*) FROM aws_listing_cache", [], |row| row.get(0)).unwrap();
        assert_eq!(left, 0);
    }
}
