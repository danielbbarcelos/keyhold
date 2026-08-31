//! Files kept inside a record.
//!
//! A credential is often not only a password: it is a private key, a certificate, a signed
//! agreement, a recovery-code sheet somebody photographed. Those belong with the record they are
//! about, and nowhere else — least of all in a Downloads folder.
//!
//! **The name and the type are sealed with the file, not beside it.** "id_rsa" and
//! "contrato-assinado.pdf" say plenty on their own, so nothing outside the ciphertext describes
//! what is being kept. What does stay outside is the byte count, which any observer could infer
//! from the row's size anyway and which the interface needs to draw a list without opening every
//! file in it.
//!
//! **The bytes live in the vault file.** One file to back up, one file to set permissions on, and
//! no directory where a decrypted temporary could be left behind.

use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::crypto::{self, unwrap_key, wrap_key, SecretKey};
use crate::db::Store;
use crate::records;
use crate::vaults::{self, new_id};

/// The largest file keyhold will take.
///
/// A limit exists because the whole vault is one SQLite file that gets read and written as a unit;
/// a 2 GB video in it would make every open slower forever. Twenty-five megabytes covers keys,
/// certificates and scanned documents, which is what a vault is for.
pub const MAX_BYTES: usize = 25 * 1024 * 1024;

fn meta_aad(id: &str) -> Vec<u8> {
    format!("keyhold:attachment-meta:v1|{id}").into_bytes()
}

fn blob_aad(id: &str, record_id: &str) -> Vec<u8> {
    format!("keyhold:attachment:v1|{id}|{record_id}").into_bytes()
}

fn key_aad(id: &str) -> Vec<u8> {
    format!("keyhold:attachment-key:v1|{id}").into_bytes()
}

#[derive(Debug, Serialize, Deserialize)]
struct FileMeta {
    name: String,
    mime: String,
}

/// An attachment as the interface lists it: named, sized, and still sealed.
#[derive(Debug, Serialize)]
pub struct Attachment {
    pub id: String,
    pub record_id: String,
    pub name: String,
    pub mime: String,
    pub size: i64,
    pub created_at: String,
}

/// The file itself, opened.
pub struct OpenFile {
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

pub fn add(
    store: &Store,
    master: &SecretKey,
    record_id: &str,
    name: &str,
    mime: &str,
    bytes: &[u8],
) -> Result<Attachment> {
    if name.trim().is_empty() {
        bail!("o arquivo precisa de um nome");
    }

    if bytes.is_empty() {
        bail!("o arquivo está vazio");
    }

    if bytes.len() > MAX_BYTES {
        bail!(
            "o arquivo tem {:.1} MB; o limite é {} MB",
            bytes.len() as f64 / (1024.0 * 1024.0),
            MAX_BYTES / (1024 * 1024)
        );
    }

    let header = records::header(store, record_id)?;
    let vault_key = vaults::vault_key(store, master, &header.vault_id)?;

    let id = new_id();
    let file_key = SecretKey::random();

    let wrapped = wrap_key(&vault_key, &file_key, &key_aad(&id))?;
    let sealed_meta = crypto::seal(
        &file_key,
        &serde_json::to_vec(&FileMeta { name: name.into(), mime: mime.into() })?,
        &meta_aad(&id),
    )?;
    let sealed_blob = crypto::seal(&file_key, bytes, &blob_aad(&id, record_id))?;

    store.conn().execute(
        "INSERT INTO attachments (id, record_id, wrapped_key, sealed_meta, sealed_blob, size, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        params![id, record_id, wrapped, sealed_meta, sealed_blob, bytes.len() as i64],
    )?;

    read(store, master, &id)
}

/// Everything attached to one record, without reading a single file's contents.
pub fn list(store: &Store, master: &SecretKey, record_id: &str) -> Result<Vec<Attachment>> {
    let mut statement = store.conn().prepare(
        "SELECT id, wrapped_key, sealed_meta, size, created_at
         FROM attachments WHERE record_id = ?1 ORDER BY created_at",
    )?;

    let header = records::header(store, record_id)?;
    let vault_key = vaults::vault_key(store, master, &header.vault_id)?;

    let rows = statement.query_map(params![record_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, Vec<u8>>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;

    let mut found = Vec::new();

    for row in rows {
        let (id, wrapped, sealed_meta, size, created_at) = row?;

        let Ok(file_key) = unwrap_key(&vault_key, &wrapped, &key_aad(&id)) else {
            continue;
        };

        let Ok(bytes) = crypto::open(&file_key, &sealed_meta, &meta_aad(&id)) else {
            continue;
        };

        let Ok(meta) = serde_json::from_slice::<FileMeta>(&bytes) else {
            continue;
        };

        found.push(Attachment {
            id,
            record_id: record_id.to_string(),
            name: meta.name,
            mime: meta.mime,
            size,
            created_at,
        });
    }

    Ok(found)
}

pub fn read(store: &Store, master: &SecretKey, id: &str) -> Result<Attachment> {
    let record_id: String = store
        .conn()
        .query_row("SELECT record_id FROM attachments WHERE id = ?1", params![id], |row| row.get(0))
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("não há um arquivo com esse nome aqui"))?;

    list(store, master, &record_id)?
        .into_iter()
        .find(|one| one.id == id)
        .ok_or_else(|| anyhow::anyhow!("não há um arquivo com esse nome aqui"))
}

/// Open one file. The only path to its bytes.
pub fn open(store: &Store, master: &SecretKey, id: &str) -> Result<OpenFile> {
    let (record_id, wrapped, sealed_meta, sealed_blob): (String, Vec<u8>, Vec<u8>, Vec<u8>) = store
        .conn()
        .query_row(
            "SELECT record_id, wrapped_key, sealed_meta, sealed_blob FROM attachments WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("não há um arquivo com esse nome aqui"))?;

    let header = records::header(store, &record_id)?;
    let vault_key = vaults::vault_key(store, master, &header.vault_id)?;

    let file_key =
        unwrap_key(&vault_key, &wrapped, &key_aad(id)).context("a chave deste arquivo não abriu")?;

    let meta: FileMeta = serde_json::from_slice(
        &crypto::open(&file_key, &sealed_meta, &meta_aad(id)).context("este arquivo não abriu")?,
    )?;

    let bytes = crypto::open(&file_key, &sealed_blob, &blob_aad(id, &record_id))
        .context("este arquivo não abriu")?;

    Ok(OpenFile { name: meta.name, mime: meta.mime, bytes })
}

pub fn remove(store: &Store, id: &str) -> Result<()> {
    let removed = store
        .conn()
        .execute("DELETE FROM attachments WHERE id = ?1", params![id])?;

    if removed == 0 {
        bail!("não há um arquivo com esse nome aqui");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{self, SetupOptions};
    use crate::records::Payload;
    use crate::vaults::VaultMeta;

    fn scratch() -> (tempfile::TempDir, Store, SecretKey, String) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("vault.db")).unwrap();

        let outcome =
            identity::create(&mut store, SetupOptions { password: Some("the password".into()) })
                .unwrap();

        let vault = vaults::create_vault(
            &store,
            &outcome.vault_key,
            VaultMeta { name: "Trabalho".into(), description: String::new(), colour: "default".into() },
        )
        .unwrap();

        let record = records::create(
            &store,
            &outcome.vault_key,
            &vault.id,
            None,
            "ssh_key",
            &Payload { title: "Chave".into(), ..Payload::default() },
        )
        .unwrap();

        (dir, store, outcome.vault_key, record.id)
    }

    #[test]
    fn a_file_comes_back_byte_for_byte() {
        let (_dir, store, master, record) = scratch();
        let contents = b"-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n";

        let added = add(&store, &master, &record, "id_ed25519", "text/plain", contents).unwrap();

        assert_eq!(added.name, "id_ed25519");
        assert_eq!(added.size, contents.len() as i64);

        let opened = open(&store, &master, &added.id).unwrap();

        assert_eq!(opened.bytes, contents);
        assert_eq!(opened.name, "id_ed25519");
        assert_eq!(opened.mime, "text/plain");
    }

    #[test]
    fn neither_the_contents_nor_the_name_are_readable_in_the_file() {
        let (dir, store, master, record) = scratch();

        add(
            &store,
            &master,
            &record,
            "contrato-assinado.pdf",
            "application/pdf",
            b"conteudo confidencial do contrato",
        )
        .unwrap();

        drop(store);

        let raw = std::fs::read(dir.path().join("vault.db")).unwrap();
        let text = String::from_utf8_lossy(&raw);

        assert!(!text.contains("contrato-assinado.pdf"), "the file's name is in the clear");
        assert!(!text.contains("conteudo confidencial"), "the file's contents are in the clear");
        assert!(!text.contains("application/pdf"), "the file's type is in the clear");
    }

    #[test]
    fn the_list_names_the_files_without_reading_them() {
        let (_dir, store, master, record) = scratch();

        add(&store, &master, &record, "um.txt", "text/plain", b"aaa").unwrap();
        add(&store, &master, &record, "dois.pdf", "application/pdf", b"bbbb").unwrap();

        let listed = list(&store, &master, &record).unwrap();

        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "um.txt");
        assert_eq!(listed[1].size, 4);
    }

    #[test]
    fn a_file_moved_to_another_record_does_not_open() {
        let (_dir, store, master, record) = scratch();
        let added = add(&store, &master, &record, "id_rsa", "text/plain", b"segredo").unwrap();

        // Somebody with write access to the file re-points the row at another record.
        let header = records::header(&store, &record).unwrap();
        let other = records::create(
            &store,
            &master,
            &header.vault_id,
            None,
            "note",
            &Payload { title: "Outro".into(), ..Payload::default() },
        )
        .unwrap();

        store
            .conn()
            .execute(
                "UPDATE attachments SET record_id = ?2 WHERE id = ?1",
                params![added.id, other.id],
            )
            .unwrap();

        assert!(open(&store, &master, &added.id).is_err(), "a file opened under another record");
    }

    #[test]
    fn a_tampered_file_fails_rather_than_returning_rubbish() {
        let (_dir, store, master, record) = scratch();
        let added = add(&store, &master, &record, "id_rsa", "text/plain", b"segredo").unwrap();

        let mut sealed: Vec<u8> = store
            .conn()
            .query_row(
                "SELECT sealed_blob FROM attachments WHERE id = ?1",
                params![added.id],
                |row| row.get(0),
            )
            .unwrap();

        let last = sealed.len() - 1;
        sealed[last] ^= 0x01;

        store
            .conn()
            .execute(
                "UPDATE attachments SET sealed_blob = ?2 WHERE id = ?1",
                params![added.id, sealed],
            )
            .unwrap();

        assert!(open(&store, &master, &added.id).is_err());
    }

    #[test]
    fn deleting_a_record_takes_its_files_with_it() {
        let (_dir, store, master, record) = scratch();
        add(&store, &master, &record, "id_rsa", "text/plain", b"segredo").unwrap();

        records::archive(&store, &record, true).unwrap();
        records::destroy(&store, &record).unwrap();

        let left: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM attachments WHERE record_id = ?1",
                params![record],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(left, 0, "a file outlived the record it belonged to");
    }

    #[test]
    fn an_empty_or_oversized_file_is_refused() {
        let (_dir, store, master, record) = scratch();

        assert!(add(&store, &master, &record, "vazio.txt", "text/plain", b"").is_err());
        assert!(add(&store, &master, &record, "  ", "text/plain", b"abc").is_err());

        let huge = vec![0u8; MAX_BYTES + 1];
        assert!(add(&store, &master, &record, "grande.bin", "application/octet-stream", &huge).is_err());
    }
}
