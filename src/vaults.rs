//! Vaults and folders: the shape of what is kept, and the keys that open each part of it.
//!
//! **Why more than one vault on a machine with one owner.** Not for permissions — there is nobody
//! to keep out. For two things that are real even alone: a vault can stay locked while another is
//! open, and a vault is the unit the agent surface is allowed to see (F7). "The agent may read
//! *Servidores* and nothing else" is a sentence worth being able to say, and it needs a noun.
//!
//! **The key hierarchy** continues the one from `identity`:
//!
//! ```text
//! master key ──unwrap──> vault key ──unwrap──> folder key ──> (records, from F3)
//! ```
//!
//! Each level is a random key wrapped by the level above, never derived from it. That is what makes
//! moving a folder cheap, rotation cheap, and a future per-vault password possible without
//! re-encrypting anything.
//!
//! **Names are sealed too.** See the migration in `db.rs` for why: on a machine with no server, a
//! readable list of vault names is worth reading even when the passwords are not.

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::crypto::{self, unwrap_key, wrap_key, SecretKey};
use crate::db::Store;
use crate::encoding::base32;

/// How deep the tree may go.
///
/// Not a storage limit — a usability one. Past this, a path stops fitting on a screen and stops
/// being something anybody navigates; and a bounded depth means the recursive walks below can
/// never run away on a cycle written by a bug.
pub const MAX_DEPTH: usize = 8;

fn vault_meta_aad(id: &str) -> Vec<u8> {
    format!("keyhold:vault-meta:v1|{id}").into_bytes()
}

fn vault_key_aad(id: &str) -> Vec<u8> {
    format!("keyhold:vault-key:v1|{id}").into_bytes()
}

fn folder_meta_aad(id: &str) -> Vec<u8> {
    format!("keyhold:folder-meta:v1|{id}").into_bytes()
}

fn folder_key_aad(id: &str) -> Vec<u8> {
    format!("keyhold:folder-key:v1|{id}").into_bytes()
}

/// An identifier: 16 random bytes in base32.
///
/// Random rather than sequential because an id ends up in an AAD, in an agent request and in the
/// audit trail, and a counter would say how many things exist and in what order they were made.
pub fn new_id() -> String {
    base32(&crypto::random_bytes(16))
}

/// The readable part of a vault, which lives sealed on disk.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultMeta {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_colour")]
    pub colour: String,
}

fn default_colour() -> String {
    "brand".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FolderMeta {
    pub name: String,
    #[serde(default)]
    pub colour: Option<String>,
}

/// A vault as the interface sees it, once the file has been opened.
#[derive(Debug, Serialize)]
pub struct VaultSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub colour: String,
    pub agent_access: bool,
    pub archived: bool,
    pub created_at: String,
    /// Which generation of this vault's key its contents are wrapped with (F5).
    pub key_version: i64,
    pub key_rotated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FolderNode {
    pub id: String,
    pub vault_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub colour: Option<String>,
    pub depth: usize,
}

// ── vaults ──────────────────────────────────────────────────────────────────────────────────────

pub fn create_vault(store: &Store, master: &SecretKey, meta: VaultMeta) -> Result<VaultSummary> {
    if meta.name.trim().is_empty() {
        bail!("a vault needs a name");
    }

    let id = new_id();
    let key = SecretKey::random();

    let wrapped = wrap_key(master, &key, &vault_key_aad(&id))?;
    let sealed = crypto::seal(&key, &serde_json::to_vec(&meta)?, &vault_meta_aad(&id))?;

    let next: i64 = store
        .conn()
        .query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM vaults", [], |row| row.get(0))?;

    store.conn().execute(
        "INSERT INTO vaults (id, wrapped_key, sealed_meta, position, created_at)
         VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        params![id, wrapped, sealed, next],
    )?;

    read_vault(store, master, &id)
}

/// Every vault in the file, in the order they were made.
///
/// A vault whose metadata will not open is **skipped, not guessed at**: it means the file was
/// edited or a key is wrong, and inventing a name for it would hide that.
pub fn list_vaults(store: &Store, master: &SecretKey) -> Result<Vec<VaultSummary>> {
    let mut statement = store.conn().prepare(
        "SELECT id, wrapped_key, sealed_meta, agent_access, archived_at, created_at,
                key_version, key_rotated_at
         FROM vaults ORDER BY position",
    )?;

    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, Vec<u8>>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, Option<String>>(7)?,
        ))
    })?;

    let mut found = Vec::new();

    for row in rows {
        let (id, wrapped, sealed, agent, archived, created, key_version, rotated) = row?;

        let Ok(key) = unwrap_key(master, &wrapped, &vault_key_aad(&id)) else {
            continue;
        };

        let Ok(meta) = crypto::open(&key, &sealed, &vault_meta_aad(&id))
            .ok()
            .map(|bytes| serde_json::from_slice::<VaultMeta>(&bytes))
            .transpose()
        else {
            continue;
        };

        let Some(meta) = meta else { continue };

        found.push(VaultSummary {
            id,
            name: meta.name,
            description: meta.description,
            colour: meta.colour,
            agent_access: agent != 0,
            archived: archived.is_some(),
            created_at: created,
            key_version,
            key_rotated_at: rotated,
        });
    }

    Ok(found)
}

pub fn read_vault(store: &Store, master: &SecretKey, id: &str) -> Result<VaultSummary> {
    list_vaults(store, master)?
        .into_iter()
        .find(|vault| vault.id == id)
        .ok_or_else(|| anyhow!("there is no vault with that name here"))
}

/// The key that opens one vault's contents.
pub fn vault_key(store: &Store, master: &SecretKey, id: &str) -> Result<SecretKey> {
    let wrapped: Vec<u8> = store
        .conn()
        .query_row("SELECT wrapped_key FROM vaults WHERE id = ?1", params![id], |row| row.get(0))
        .optional()?
        .ok_or_else(|| anyhow!("there is no vault with that name here"))?;

    unwrap_key(master, &wrapped, &vault_key_aad(id)).context("this vault's key did not open")
}

pub fn update_vault(store: &Store, master: &SecretKey, id: &str, meta: VaultMeta) -> Result<VaultSummary> {
    if meta.name.trim().is_empty() {
        bail!("a vault needs a name");
    }

    let key = vault_key(store, master, id)?;
    let sealed = crypto::seal(&key, &serde_json::to_vec(&meta)?, &vault_meta_aad(id))?;

    store
        .conn()
        .execute("UPDATE vaults SET sealed_meta = ?1 WHERE id = ?2", params![sealed, id])?;

    read_vault(store, master, id)
}

/// Put a vault away, or bring it back.
///
/// Archiving hides; it never deletes. Deleting a vault would take everything in it with no way
/// back, so the only route to that is emptying it first — the trash, in F5.
pub fn set_archived(store: &Store, id: &str, archived: bool) -> Result<()> {
    let changed = store.conn().execute(
        "UPDATE vaults SET archived_at = CASE WHEN ?2 THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now') END
         WHERE id = ?1",
        params![id, archived],
    )?;

    if changed == 0 {
        bail!("there is no vault with that name here");
    }

    Ok(())
}

/// What deleting this vault would take with it.
///
/// Counted before anything is destroyed, so the confirmation can say a number instead of a warning.
/// "This will delete 47 records" is a sentence somebody can weigh; "this cannot be undone" is one
/// they have learned to click past.
#[derive(Debug, Serialize)]
pub struct VaultContents {
    pub records: i64,
    pub archived_records: i64,
    pub folders: i64,
}

pub fn contents_of(store: &Store, id: &str) -> Result<VaultContents> {
    let records: i64 = store.conn().query_row(
        "SELECT COUNT(*) FROM records WHERE vault_id = ?1 AND archived_at IS NULL",
        params![id],
        |row| row.get(0),
    )?;

    let archived: i64 = store.conn().query_row(
        "SELECT COUNT(*) FROM records WHERE vault_id = ?1 AND archived_at IS NOT NULL",
        params![id],
        |row| row.get(0),
    )?;

    let folders: i64 = store.conn().query_row(
        "SELECT COUNT(*) FROM folders WHERE vault_id = ?1",
        params![id],
        |row| row.get(0),
    )?;

    Ok(VaultContents { records, archived_records: archived, folders })
}

/// Destroy a vault and everything inside it.
///
/// **This is the largest irreversible action in keyhold**, and the only one that can take hundreds
/// of secrets at once. Two things guard it, and neither is a dialog:
///
///  - **The caller has to name the vault.** Not as a formality — as the difference between an
///    intention and a misclick. A confirmation box is answered by muscle memory; a name has to be
///    read off the screen and typed, which is the only check that survives somebody moving fast.
///    It also means a future agent surface cannot delete a vault it merely knows the id of.
///  - **The database does the cascade**, in one statement. Folders, records and every past version
///    hang off the vault by foreign key, so there is no order of deletions to get wrong and no
///    window where the vault is gone but its records are not.
///
/// Archiving exists for everything short of this. `set_archived` is the reversible answer, and the
/// interface offers it first.
pub fn delete_vault(store: &Store, master: &SecretKey, id: &str, confirm_name: &str) -> Result<VaultContents> {
    let vault = read_vault(store, master, id)?;

    if confirm_name.trim() != vault.name.trim() {
        bail!("o nome digitado não é o deste cofre");
    }

    let contents = contents_of(store, id)?;

    let removed = store
        .conn()
        .execute("DELETE FROM vaults WHERE id = ?1", params![id])?;

    if removed == 0 {
        bail!("there is no vault with that name here");
    }

    Ok(contents)
}

/// Mark whether a local agent may see this vault at all (F7).
///
/// Off by default, and it is a *visibility* switch rather than a permission: what is not marked is
/// never listed to an agent, so there is nothing for it to ask about.
pub fn set_agent_access(store: &Store, id: &str, allowed: bool) -> Result<()> {
    let changed = store.conn().execute(
        "UPDATE vaults SET agent_access = ?2 WHERE id = ?1",
        params![id, allowed as i64],
    )?;

    if changed == 0 {
        bail!("there is no vault with that name here");
    }

    Ok(())
}

/// Give a vault a new key, and re-wrap everything that hung off the old one.
///
/// **What this is for.** The vault key is what every record's key is wrapped with, so it is the one
/// value whose exposure would matter beyond a single record — a backup taken while unlocked, a
/// process that read this one's memory, a suspicion you cannot rule out. Rotating replaces it and
/// leaves every payload untouched: nothing is re-encrypted, only re-wrapped, which is the entire
/// reason keys hang off each other rather than being derived.
///
/// **It is one transaction, and it has to be.** A rotation that stops halfway is a vault whose
/// records are wrapped with two different keys and whose header names one of them — half the
/// contents unopenable, with no way to tell which half. Either every record moves to the new key or
/// none does.
pub fn rotate_vault(store: &mut Store, master: &SecretKey, id: &str) -> Result<VaultSummary> {
    let old_key = vault_key(store, master, id)?;
    let new_key = SecretKey::random();

    // Re-wrap in memory first, so a failure to open any one of them stops the whole thing before a
    // single row is written.
    let mut records: Vec<(String, Vec<u8>)> = Vec::new();
    {
        let mut statement = store
            .conn()
            .prepare("SELECT id, wrapped_key FROM records WHERE vault_id = ?1")?;

        let rows = statement.query_map(params![id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;

        for row in rows {
            let (record_id, wrapped) = row?;
            let aad = format!("keyhold:record-key:v1|{record_id}").into_bytes();

            let record_key = unwrap_key(&old_key, &wrapped, &aad)
                .with_context(|| format!("a chave de um registro não abriu ({record_id})"))?;

            records.push((record_id, wrap_key(&new_key, &record_key, &aad)?));
        }
    }

    let mut folders: Vec<(String, Vec<u8>)> = Vec::new();
    {
        let mut statement = store
            .conn()
            .prepare("SELECT id, wrapped_key FROM folders WHERE vault_id = ?1")?;

        let rows = statement.query_map(params![id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;

        for row in rows {
            let (folder_id, wrapped) = row?;
            let aad = folder_key_aad(&folder_id);

            let folder_key = unwrap_key(&old_key, &wrapped, &aad)
                .with_context(|| format!("a chave de uma pasta não abriu ({folder_id})"))?;

            folders.push((folder_id, wrap_key(&new_key, &folder_key, &aad)?));
        }
    }

    // The vault's own metadata is sealed with the vault key too, so it moves with everything else.
    let meta = read_vault(store, master, id)?;
    let sealed_meta = crypto::seal(
        &new_key,
        &serde_json::to_vec(&VaultMeta {
            name: meta.name,
            description: meta.description,
            colour: meta.colour,
        })?,
        &vault_meta_aad(id),
    )?;

    let wrapped_vault_key = wrap_key(master, &new_key, &vault_key_aad(id))?;

    let transaction = store.conn_mut().transaction()?;

    for (record_id, wrapped) in records {
        transaction.execute(
            "UPDATE records SET wrapped_key = ?2 WHERE id = ?1",
            params![record_id, wrapped],
        )?;
    }

    for (folder_id, wrapped) in folders {
        transaction.execute(
            "UPDATE folders SET wrapped_key = ?2 WHERE id = ?1",
            params![folder_id, wrapped],
        )?;
    }

    transaction.execute(
        "UPDATE vaults SET wrapped_key = ?2, sealed_meta = ?3, key_version = key_version + 1,
                           key_rotated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![id, wrapped_vault_key, sealed_meta],
    )?;

    transaction.commit()?;

    read_vault(store, master, id)
}

// ── folders ─────────────────────────────────────────────────────────────────────────────────────

pub fn create_folder(
    store: &Store,
    master: &SecretKey,
    vault_id: &str,
    parent_id: Option<&str>,
    meta: FolderMeta,
) -> Result<FolderNode> {
    if meta.name.trim().is_empty() {
        bail!("a folder needs a name");
    }

    let vault = vault_key(store, master, vault_id)?;

    if let Some(parent) = parent_id {
        let parent_vault = folder_vault(store, parent)?;

        if parent_vault != vault_id {
            bail!("that folder belongs to another vault");
        }

        if depth_of(store, parent)? + 1 >= MAX_DEPTH {
            bail!("folders here only go {MAX_DEPTH} levels deep");
        }
    }

    let id = new_id();
    let key = SecretKey::random();

    let wrapped = wrap_key(&vault, &key, &folder_key_aad(&id))?;
    let sealed = crypto::seal(&key, &serde_json::to_vec(&meta)?, &folder_meta_aad(&id))?;

    let next: i64 = store.conn().query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM folders WHERE vault_id = ?1",
        params![vault_id],
        |row| row.get(0),
    )?;

    store.conn().execute(
        "INSERT INTO folders (id, vault_id, parent_id, wrapped_key, sealed_meta, position, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        params![id, vault_id, parent_id, wrapped, sealed, next],
    )?;

    read_folder(store, master, vault_id, &id)
}

/// One vault's folders, flat, each carrying its depth so the interface can draw the tree.
pub fn list_folders(store: &Store, master: &SecretKey, vault_id: &str) -> Result<Vec<FolderNode>> {
    let vault = vault_key(store, master, vault_id)?;

    let mut statement = store.conn().prepare(
        "SELECT id, parent_id, wrapped_key, sealed_meta FROM folders
         WHERE vault_id = ?1 ORDER BY position",
    )?;

    let rows = statement.query_map(params![vault_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Vec<u8>>(2)?,
            row.get::<_, Vec<u8>>(3)?,
        ))
    })?;

    let mut found = Vec::new();

    for row in rows {
        let (id, parent_id, wrapped, sealed) = row?;

        let Ok(key) = unwrap_key(&vault, &wrapped, &folder_key_aad(&id)) else {
            continue;
        };

        let Ok(bytes) = crypto::open(&key, &sealed, &folder_meta_aad(&id)) else {
            continue;
        };

        let Ok(meta) = serde_json::from_slice::<FolderMeta>(&bytes) else {
            continue;
        };

        let depth = depth_of(store, &id)?;

        found.push(FolderNode {
            id,
            vault_id: vault_id.to_string(),
            parent_id,
            name: meta.name,
            colour: meta.colour,
            depth,
        });
    }

    Ok(found)
}

pub fn read_folder(
    store: &Store,
    master: &SecretKey,
    vault_id: &str,
    id: &str,
) -> Result<FolderNode> {
    list_folders(store, master, vault_id)?
        .into_iter()
        .find(|folder| folder.id == id)
        .ok_or_else(|| anyhow!("there is no folder with that name here"))
}

/// The key that opens one folder.
pub fn folder_key(store: &Store, master: &SecretKey, id: &str) -> Result<SecretKey> {
    let (vault_id, wrapped): (String, Vec<u8>) = store
        .conn()
        .query_row(
            "SELECT vault_id, wrapped_key FROM folders WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| anyhow!("there is no folder with that name here"))?;

    let vault = vault_key(store, master, &vault_id)?;

    unwrap_key(&vault, &wrapped, &folder_key_aad(id)).context("this folder's key did not open")
}

pub fn update_folder(
    store: &Store,
    master: &SecretKey,
    id: &str,
    meta: FolderMeta,
) -> Result<FolderNode> {
    if meta.name.trim().is_empty() {
        bail!("a folder needs a name");
    }

    let vault_id = folder_vault(store, id)?;
    let key = folder_key(store, master, id)?;
    let sealed = crypto::seal(&key, &serde_json::to_vec(&meta)?, &folder_meta_aad(id))?;

    store
        .conn()
        .execute("UPDATE folders SET sealed_meta = ?1 WHERE id = ?2", params![sealed, id])?;

    read_folder(store, master, &vault_id, id)
}

/// Move a folder under a new parent, or to the root.
///
/// Re-parenting is a single column: the folder's own key does not change, so nothing inside it is
/// touched. What this has to defend against is the two ways a tree stops being a tree — a folder
/// under itself, and a branch pushed past the depth limit.
pub fn move_folder(store: &Store, id: &str, new_parent: Option<&str>) -> Result<()> {
    let vault_id = folder_vault(store, id)?;

    if let Some(parent) = new_parent {
        if parent == id {
            bail!("a folder cannot be inside itself");
        }

        if folder_vault(store, parent)? != vault_id {
            bail!("that folder belongs to another vault");
        }

        if is_descendant(store, parent, id)? {
            bail!("a folder cannot be moved inside one of its own folders");
        }

        if depth_of(store, parent)? + 1 + height_of(store, id)? > MAX_DEPTH {
            bail!("that would push folders past {MAX_DEPTH} levels deep");
        }
    }

    store.conn().execute(
        "UPDATE folders SET parent_id = ?2 WHERE id = ?1",
        params![id, new_parent],
    )?;

    Ok(())
}

/// Delete an empty folder. Anything inside has to be moved or deleted first (F5's trash).
pub fn delete_folder(store: &Store, id: &str) -> Result<()> {
    let children: i64 = store.conn().query_row(
        "SELECT COUNT(*) FROM folders WHERE parent_id = ?1",
        params![id],
        |row| row.get(0),
    )?;

    if children > 0 {
        bail!("this folder still has folders inside it");
    }

    let removed = store
        .conn()
        .execute("DELETE FROM folders WHERE id = ?1", params![id])?;

    if removed == 0 {
        bail!("there is no folder with that name here");
    }

    Ok(())
}

// ── walking the tree ────────────────────────────────────────────────────────────────────────────

fn folder_vault(store: &Store, id: &str) -> Result<String> {
    store
        .conn()
        .query_row("SELECT vault_id FROM folders WHERE id = ?1", params![id], |row| row.get(0))
        .optional()?
        .ok_or_else(|| anyhow!("there is no folder with that name here"))
}

fn parent_of(store: &Store, id: &str) -> Result<Option<String>> {
    Ok(store
        .conn()
        .query_row("SELECT parent_id FROM folders WHERE id = ?1", params![id], |row| row.get(0))
        .optional()?
        .flatten())
}

/// How far below the root a folder sits. Bounded by `MAX_DEPTH`, so a cycle cannot spin forever.
fn depth_of(store: &Store, id: &str) -> Result<usize> {
    let mut depth = 0;
    let mut current = id.to_string();

    while let Some(parent) = parent_of(store, &current)? {
        depth += 1;
        current = parent;

        if depth > MAX_DEPTH {
            bail!("this folder tree loops back on itself");
        }
    }

    Ok(depth)
}

/// How many levels of folders sit below this one.
fn height_of(store: &Store, id: &str) -> Result<usize> {
    let mut statement = store
        .conn()
        .prepare("SELECT id FROM folders WHERE parent_id = ?1")?;

    let children: Vec<String> = statement
        .query_map(params![id], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;

    let mut tallest = 0;

    for child in children {
        tallest = tallest.max(1 + height_of(store, &child)?);
    }

    Ok(tallest)
}

fn is_descendant(store: &Store, candidate: &str, ancestor: &str) -> Result<bool> {
    let mut current = candidate.to_string();
    let mut steps = 0;

    while let Some(parent) = parent_of(store, &current)? {
        if parent == ancestor {
            return Ok(true);
        }

        current = parent;
        steps += 1;

        if steps > MAX_DEPTH {
            bail!("this folder tree loops back on itself");
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{self, SetupOptions};

    fn scratch() -> (tempfile::TempDir, Store, SecretKey) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("vault.db")).unwrap();

        let outcome =
            identity::create(&mut store, SetupOptions { password: Some("the password".into()) })
                .unwrap();

        (dir, store, outcome.vault_key)
    }

    fn meta(name: &str) -> VaultMeta {
        VaultMeta { name: name.into(), description: String::new(), colour: "brand".into() }
    }

    fn folder(name: &str) -> FolderMeta {
        FolderMeta { name: name.into(), colour: None }
    }

    #[test]
    fn a_vault_comes_back_with_the_name_it_was_given() {
        let (_dir, store, master) = scratch();

        let made = create_vault(&store, &master, meta("Servidores")).unwrap();
        assert_eq!(made.name, "Servidores");

        let listed = list_vaults(&store, &master).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Servidores");
    }

    #[test]
    fn the_name_is_not_readable_in_the_file() {
        let (dir, store, master) = scratch();
        create_vault(&store, &master, meta("Banco Itaú")).unwrap();
        drop(store);

        let raw = std::fs::read(dir.path().join("vault.db")).unwrap();
        let text = String::from_utf8_lossy(&raw);

        assert!(!text.contains("Banco Itaú"), "the vault's name is sitting in the file in the clear");
    }

    #[test]
    fn another_master_key_reads_nothing() {
        let (_dir, store, master) = scratch();
        create_vault(&store, &master, meta("Servidores")).unwrap();

        // A wrong key does not produce a wrong name — it produces no vault at all.
        let stranger = SecretKey::random();
        assert!(list_vaults(&store, &stranger).unwrap().is_empty());
    }

    #[test]
    fn a_vault_can_be_renamed_without_touching_its_key() {
        let (_dir, store, master) = scratch();
        let made = create_vault(&store, &master, meta("Trabalho")).unwrap();

        let before = vault_key(&store, &master, &made.id).unwrap();
        update_vault(&store, &master, &made.id, meta("Trabalho antigo")).unwrap();
        let after = vault_key(&store, &master, &made.id).unwrap();

        assert_eq!(before.as_bytes(), after.as_bytes());
        assert_eq!(read_vault(&store, &master, &made.id).unwrap().name, "Trabalho antigo");
    }

    #[test]
    fn archiving_hides_without_destroying() {
        let (_dir, store, master) = scratch();
        let made = create_vault(&store, &master, meta("Antigo")).unwrap();

        set_archived(&store, &made.id, true).unwrap();
        assert!(read_vault(&store, &master, &made.id).unwrap().archived);

        set_archived(&store, &made.id, false).unwrap();
        assert!(!read_vault(&store, &master, &made.id).unwrap().archived);
    }

    #[test]
    fn deleting_a_vault_needs_its_name_typed_back() {
        let (_dir, store, master) = scratch();
        let vault = create_vault(&store, &master, meta("Trabalho")).unwrap();

        assert!(delete_vault(&store, &master, &vault.id, "").is_err());
        assert!(delete_vault(&store, &master, &vault.id, "trabalho").is_err());
        assert!(delete_vault(&store, &master, &vault.id, "Trabalho ").is_ok());

        assert!(read_vault(&store, &master, &vault.id).is_err());
    }

    #[test]
    fn deleting_a_vault_takes_its_folders_records_and_history_with_it() {
        let (_dir, mut store, master) = scratch();
        let vault = create_vault(&store, &master, meta("Trabalho")).unwrap();

        let folder = create_folder(&store, &master, &vault.id, None, folder("Clientes")).unwrap();

        let record = crate::records::create(
            &store,
            &master,
            &vault.id,
            Some(&folder.id),
            "login",
            &crate::records::Payload { title: "Servidor".into(), ..Default::default() },
        )
        .unwrap();

        crate::records::update(
            &mut store,
            &master,
            &record.id,
            &crate::records::Payload { title: "Servidor renomeado".into(), ..Default::default() },
        )
        .unwrap();

        let counted = delete_vault(&store, &master, &vault.id, "Trabalho").unwrap();
        assert_eq!(counted.records, 1);
        assert_eq!(counted.folders, 1);

        for (table, column) in [
            ("folders", "vault_id"),
            ("records", "vault_id"),
        ] {
            let left: i64 = store
                .conn()
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {column} = ?1"),
                    params![vault.id],
                    |row| row.get(0),
                )
                .unwrap();

            assert_eq!(left, 0, "{table} survived the vault it belonged to");
        }

        let versions: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM record_versions WHERE record_id = ?1",
                params![record.id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(versions, 0, "an old ciphertext outlived its vault");
    }

    #[test]
    fn deleting_one_vault_leaves_the_others_untouched() {
        let (_dir, store, master) = scratch();
        let doomed = create_vault(&store, &master, meta("Some")).unwrap();
        let kept = create_vault(&store, &master, meta("Fica")).unwrap();

        crate::records::create(
            &store,
            &master,
            &kept.id,
            None,
            "login",
            &crate::records::Payload { title: "Servidor".into(), ..Default::default() },
        )
        .unwrap();

        delete_vault(&store, &master, &doomed.id, "Some").unwrap();

        assert_eq!(list_vaults(&store, &master).unwrap().len(), 1);
        assert_eq!(crate::records::list(&store, &kept.id, false).unwrap().len(), 1);
    }

    #[test]
    fn the_count_says_what_would_be_lost_before_anything_is() {
        let (_dir, store, master) = scratch();
        let vault = create_vault(&store, &master, meta("Trabalho")).unwrap();

        for title in ["Um", "Dois", "Tres"] {
            crate::records::create(
                &store,
                &master,
                &vault.id,
                None,
                "login",
                &crate::records::Payload { title: title.into(), ..Default::default() },
            )
            .unwrap();
        }

        let ids = crate::records::list(&store, &vault.id, false).unwrap();
        crate::records::archive(&store, &ids[0].id, true).unwrap();

        let counted = contents_of(&store, &vault.id).unwrap();

        assert_eq!(counted.records, 2);
        assert_eq!(counted.archived_records, 1);

        // And counting changed nothing.
        assert_eq!(crate::records::list(&store, &vault.id, true).unwrap().len(), 3);
    }

    #[test]
    fn agent_access_is_off_until_it_is_switched_on() {
        let (_dir, store, master) = scratch();
        let made = create_vault(&store, &master, meta("Servidores")).unwrap();

        assert!(!made.agent_access, "a new vault was visible to an agent");

        set_agent_access(&store, &made.id, true).unwrap();
        assert!(read_vault(&store, &master, &made.id).unwrap().agent_access);
    }

    #[test]
    fn a_vault_needs_a_name() {
        let (_dir, store, master) = scratch();

        assert!(create_vault(&store, &master, meta("   ")).is_err());
    }

    #[test]
    fn folders_nest_and_report_their_depth() {
        let (_dir, store, master) = scratch();
        let vault = create_vault(&store, &master, meta("Trabalho")).unwrap();

        let top = create_folder(&store, &master, &vault.id, None, folder("Clientes")).unwrap();
        let inner =
            create_folder(&store, &master, &vault.id, Some(&top.id), folder("Andrade")).unwrap();

        assert_eq!(top.depth, 0);
        assert_eq!(inner.depth, 1);
        assert_eq!(list_folders(&store, &master, &vault.id).unwrap().len(), 2);
    }

    #[test]
    fn a_folder_name_is_sealed_like_everything_else() {
        let (dir, store, master) = scratch();
        let vault = create_vault(&store, &master, meta("Trabalho")).unwrap();
        create_folder(&store, &master, &vault.id, None, folder("Cliente Andrade")).unwrap();
        drop(store);

        let raw = std::fs::read(dir.path().join("vault.db")).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("Cliente Andrade"));
    }

    #[test]
    fn a_folder_cannot_be_moved_inside_itself() {
        let (_dir, store, master) = scratch();
        let vault = create_vault(&store, &master, meta("Trabalho")).unwrap();

        let top = create_folder(&store, &master, &vault.id, None, folder("Clientes")).unwrap();
        let inner =
            create_folder(&store, &master, &vault.id, Some(&top.id), folder("Andrade")).unwrap();

        assert!(move_folder(&store, &top.id, Some(&top.id)).is_err());
        assert!(
            move_folder(&store, &top.id, Some(&inner.id)).is_err(),
            "a folder was moved into its own child"
        );
    }

    #[test]
    fn a_folder_cannot_cross_into_another_vault() {
        let (_dir, store, master) = scratch();
        let one = create_vault(&store, &master, meta("Um")).unwrap();
        let two = create_vault(&store, &master, meta("Dois")).unwrap();

        let here = create_folder(&store, &master, &one.id, None, folder("Aqui")).unwrap();
        let there = create_folder(&store, &master, &two.id, None, folder("Lá")).unwrap();

        assert!(move_folder(&store, &here.id, Some(&there.id)).is_err());
        assert!(create_folder(&store, &master, &one.id, Some(&there.id), folder("X")).is_err());
    }

    #[test]
    fn the_tree_stops_at_the_depth_limit() {
        let (_dir, store, master) = scratch();
        let vault = create_vault(&store, &master, meta("Fundo")).unwrap();

        let mut parent: Option<String> = None;

        for level in 0..MAX_DEPTH {
            let made = create_folder(
                &store,
                &master,
                &vault.id,
                parent.as_deref(),
                folder(&format!("nível {level}")),
            );

            match made {
                Ok(node) => parent = Some(node.id),
                Err(_) => {
                    assert_eq!(level, MAX_DEPTH - 1, "gave up at the wrong depth");
                    return;
                }
            }
        }

        assert!(
            create_folder(&store, &master, &vault.id, parent.as_deref(), folder("demais")).is_err(),
            "the tree grew past its limit"
        );
    }

    #[test]
    fn a_folder_with_folders_in_it_is_not_deleted_by_accident() {
        let (_dir, store, master) = scratch();
        let vault = create_vault(&store, &master, meta("Trabalho")).unwrap();

        let top = create_folder(&store, &master, &vault.id, None, folder("Clientes")).unwrap();
        let inner =
            create_folder(&store, &master, &vault.id, Some(&top.id), folder("Andrade")).unwrap();

        assert!(delete_folder(&store, &top.id).is_err());

        delete_folder(&store, &inner.id).unwrap();
        assert!(delete_folder(&store, &top.id).is_ok());
    }

    #[test]
    fn rotating_keeps_everything_readable_and_changes_every_wrapped_key() {
        let (_dir, mut store, master) = scratch();
        let vault = create_vault(&store, &master, meta("Trabalho")).unwrap();

        let folder =
            create_folder(&store, &master, &vault.id, None, folder("Clientes")).unwrap();

        let record = crate::records::create(
            &store,
            &master,
            &vault.id,
            Some(&folder.id),
            "login",
            &crate::records::Payload {
                title: "Servidor".into(),
                fields: vec![crate::records::Field {
                    label: "Senha".into(),
                    kind: "password".into(),
                    value: "hunter2".into(),
                }],
                ..Default::default()
            },
        )
        .unwrap();

        let before_key = vault_key(&store, &master, &vault.id).unwrap();
        let before_wrapped: Vec<u8> = store
            .conn()
            .query_row(
                "SELECT wrapped_key FROM records WHERE id = ?1",
                params![record.id],
                |row| row.get(0),
            )
            .unwrap();

        let rotated = rotate_vault(&mut store, &master, &vault.id).unwrap();

        // The vault key really changed...
        let after_key = vault_key(&store, &master, &vault.id).unwrap();
        assert_ne!(before_key.as_bytes(), after_key.as_bytes());
        assert_eq!(rotated.key_version, 2);
        assert!(rotated.key_rotated_at.is_some());

        // ...and so did every envelope hanging off it...
        let after_wrapped: Vec<u8> = store
            .conn()
            .query_row(
                "SELECT wrapped_key FROM records WHERE id = ?1",
                params![record.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_ne!(before_wrapped, after_wrapped);

        // ...while everything is still exactly as readable as before.
        assert_eq!(rotated.name, "Trabalho");
        assert_eq!(
            list_folders(&store, &master, &vault.id).unwrap()[0].name,
            "Clientes"
        );

        let opened = crate::records::open(&store, &master, &record.id).unwrap();
        assert_eq!(opened.payload.title, "Servidor");
        assert_eq!(opened.payload.fields[0].value, "hunter2");
    }

    #[test]
    fn rotating_one_vault_leaves_the_others_alone() {
        let (_dir, mut store, master) = scratch();
        let one = create_vault(&store, &master, meta("Um")).unwrap();
        let two = create_vault(&store, &master, meta("Dois")).unwrap();

        let untouched = vault_key(&store, &master, &two.id).unwrap();
        rotate_vault(&mut store, &master, &one.id).unwrap();

        assert_eq!(
            vault_key(&store, &master, &two.id).unwrap().as_bytes(),
            untouched.as_bytes()
        );
        assert_eq!(read_vault(&store, &master, &two.id).unwrap().key_version, 1);
    }

    #[test]
    fn a_folder_key_opens_only_through_its_own_vault() {
        let (_dir, store, master) = scratch();
        let vault = create_vault(&store, &master, meta("Trabalho")).unwrap();
        let made = create_folder(&store, &master, &vault.id, None, folder("Clientes")).unwrap();

        assert!(folder_key(&store, &master, &made.id).is_ok());
        assert!(
            folder_key(&store, &SecretKey::random(), &made.id).is_err(),
            "a folder key opened without the master key"
        );
    }
}
