//! The records: the things the vault actually exists to hold.
//!
//! **A record is one sealed blob plus the few columns needed to find it.** Title, username,
//! address, notes and every custom field live inside the blob, in the order somebody arranged
//! them. Outside it: which vault and folder, what kind, which version, and the dates. That split is
//! the whole storage design, and it follows from what the file is allowed to reveal — enough to
//! sort a list, never enough to read one.
//!
//! **Kinds are templates, not schemas.** A login and a bank card are the same shape underneath: a
//! title, three optional built-ins, and an ordered list of fields. The kind only decides what the
//! interface offers to fill in. That is deliberate — a schema per kind would put a product decision
//! inside the cryptography, where changing it later means migrating data nobody can read.
//!
//! **The additional data binds a payload to its place and its version**: `record | id | version |
//! vault`. Swapping two records' blobs fails to open. Pasting yesterday's blob over today's fails
//! to open. Neither returns something plausible-looking, which is the property that matters.
//!
//! **The key hierarchy stops at two levels, on purpose.** A record's key is wrapped by its vault's
//! key, not by its folder's. In a shared vault the third level pays for itself — you hand somebody
//! a folder by handing over one key. Here there is nobody to hand it to, so a third level would
//! only mean re-wrapping a key every time a record is dragged between folders, in exchange for
//! nothing. Folder keys still seal folder names.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::crypto::{self, unwrap_key, wrap_key, SecretKey};
use crate::db::Store;
use crate::vaults::{self, new_id};

/// What older versions of keyhold called a record's kind.
///
/// Records no longer have a type: a record is a title, an icon, notes and whatever fields it needs.
/// The `kind` column stays because records written before that change carry it, and the interface
/// uses it to pick a default icon for a record that has none of its own. New records are written as
/// `item`. Unknown kinds are kept rather than rejected.
pub const KINDS: [&str; 6] = ["item", "login", "note", "ssh_key", "certificate", "card"];

/// The kinds of field a record can hold.
///
/// A kind decides how a field is drawn and whether it is a secret: it never changes how the field
/// is stored. Unknown kinds are kept, shown as plain text, so a vault from a newer keyhold opens.
pub const FIELD_KINDS: [&str; 15] = [
    "text",
    "multiline",
    "multiline_hidden",
    "hidden",
    "password",
    "pin",
    "totp",
    "email",
    "name",
    "phone",
    "url",
    "address",
    "date",
    "security_question",
    "card",
];

/// Field kinds whose value is a secret: kept under dots, never in a list preview, and the ones the
/// main button of a record copies.
pub fn is_secret_kind(kind: &str) -> bool {
    matches!(
        kind,
        "password" | "pin" | "hidden" | "multiline_hidden" | "totp" | "security_question" | "card"
    )
}

fn payload_aad(id: &str, version: i64, vault_id: &str) -> Vec<u8> {
    format!("keyhold:record:v1|{id}|{version}|{vault_id}").into_bytes()
}

fn key_aad(id: &str) -> Vec<u8> {
    format!("keyhold:record-key:v1|{id}").into_bytes()
}

/// One field of a record, as the person arranged it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub label: String,
    /// One of [`FIELD_KINDS`]. Decides how the field is drawn and whether it is a secret.
    pub kind: String,
    /// The value somebody copies: the password, the answer to a security question, a card number.
    pub value: String,
    /// The other parts of a composite field: the question of a security question; holder, expiry,
    /// code and PIN of a card.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
    /// Shown under the title in lists. Without one marked, the first e-mail, name or text is.
    #[serde(default, skip_serializing_if = "is_false")]
    pub list: bool,
    /// Copied by the record's main button. Without one marked, the first secret is.
    #[serde(default, skip_serializing_if = "is_false")]
    pub primary: bool,
}

impl Field {
    pub fn new(label: &str, kind: &str, value: &str) -> Self {
        Field { label: label.into(), kind: kind.into(), value: value.into(), ..Field::default() }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// How a record looks in a list: a lucide icon in one of the interface's colours, or a picture of
/// the person's own, sealed with the rest of the payload. Nothing is fetched from the web.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Icon {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub colour: String,
    /// A `data:image/...` address, already shrunk by the interface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// The largest picture a record's icon may carry, as a data address.
pub const ICON_IMAGE_LIMIT: usize = 256 * 1024;

/// Everything readable about a record. Never leaves the process except to the interface that asked.
///
/// `username` and `url` are what records looked like before they lost their type. They are read so
/// old records still open, folded into ordinary fields by [`Payload::normalise`], and never written
/// again.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Payload {
    pub title: String,
    #[serde(default, skip_serializing)]
    pub username: String,
    #[serde(default, skip_serializing)]
    pub url: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub fields: Vec<Field>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
}

impl Payload {
    /// Fold the old built-in username and address into fields, where every record now keeps them.
    ///
    /// The username goes first, the address right after the first password (where people expect to
    /// find it), and both keep the place a list used to give them: the username is what shows under
    /// the title.
    pub fn normalise(mut self) -> Self {
        let username = std::mem::take(&mut self.username);
        let url = std::mem::take(&mut self.url);

        if !url.trim().is_empty() {
            let at = self
                .fields
                .iter()
                .position(|f| f.kind == "password")
                .map(|i| i + 1)
                .unwrap_or(self.fields.len());
            self.fields.insert(at, Field::new("Endereço web", "url", &url));
        }

        if !username.trim().is_empty() {
            let kind = if username.contains('@') { "email" } else { "text" };
            let label = if kind == "email" { "E-mail" } else { "Usuário" };
            self.fields.insert(0, Field::new(label, kind, &username));
        }

        self
    }

    /// What a list shows under the title: the field marked for it, or the first e-mail, name or
    /// text. Never a secret, whatever the marks say.
    pub fn subtitle(&self) -> String {
        let shown = |f: &&Field| !is_secret_kind(&f.kind) && !f.value.trim().is_empty();

        self.fields
            .iter()
            .filter(shown)
            .find(|f| f.list)
            .or_else(|| {
                self.fields
                    .iter()
                    .filter(shown)
                    .find(|f| matches!(f.kind.as_str(), "email" | "name" | "text"))
            })
            .map(|f| f.value.lines().next().unwrap_or_default().to_string())
            .unwrap_or_default()
    }

    /// The first web address, for search and for the agent.
    pub fn first_url(&self) -> String {
        self.fields
            .iter()
            .find(|f| f.kind == "url" && !f.value.trim().is_empty())
            .map(|f| f.value.clone())
            .unwrap_or_default()
    }

    /// The secret the record's main button copies.
    pub fn primary_secret(&self) -> Option<&Field> {
        let secret = |f: &&Field| is_secret_kind(&f.kind) && !f.value.is_empty();

        self.fields
            .iter()
            .filter(secret)
            .find(|f| f.primary)
            .or_else(|| self.fields.iter().filter(secret).find(|f| f.kind != "totp"))
    }

    /// Refuse what the interface should never have sent.
    pub fn check(&self) -> Result<()> {
        if self.title.trim().is_empty() {
            bail!("a record needs a title");
        }

        if let Some(image) = self.icon.as_ref().and_then(|i| i.image.as_deref()) {
            if !image.starts_with("data:image/") {
                bail!("a record's picture has to be an image");
            }
            if image.len() > ICON_IMAGE_LIMIT {
                bail!("this picture is too large for a record's icon");
            }
        }

        Ok(())
    }
}

/// A record with its payload left sealed — enough to list, sort and flag, and nothing more.
#[derive(Debug, Serialize)]
pub struct RecordHeader {
    pub id: String,
    pub vault_id: String,
    pub folder_id: Option<String>,
    pub kind: String,
    pub version: i64,
    pub favourite: bool,
    pub rotation_due_at: Option<String>,
    pub compromised_at: Option<String>,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// A header plus the two fields a list has to show to be worth reading.
///
/// **Why the list opens payloads after all.** The first cut of this returned headers only, on the
/// principle that drawing a list should not mean decrypting the vault. On a server that principle
/// is right; here it produced a list of rows called "Login · 2026-08-31", which is not a list.
///
/// The reasoning that makes this safe is specific, and worth writing down: the key already lives in
/// this process, the person has already unlocked, and a title is something the interface is about to
/// show them anyway. Nothing leaves the machine and nothing is written down. What stays out of the
/// preview is everything else — the fields, the notes, every value that is actually a secret — so a
/// list is a list, and reading a password is still a separate act that F6 will write into the trail.
#[derive(Debug, Serialize)]
pub struct RecordPreview {
    #[serde(flatten)]
    pub header: RecordHeader,
    pub title: String,
    /// The line under the title. See [`Payload::subtitle`].
    pub subtitle: String,
    pub url: String,
    pub icon: Option<Icon>,
}

/// A record the caller has the key for.
#[derive(Debug, Serialize)]
pub struct OpenRecord {
    #[serde(flatten)]
    pub header: RecordHeader,
    pub payload: Payload,
}

#[derive(Debug, Serialize)]
pub struct VersionEntry {
    pub version: i64,
    pub created_at: String,
}

pub fn create(
    store: &Store,
    master: &SecretKey,
    vault_id: &str,
    folder_id: Option<&str>,
    kind: &str,
    payload: &Payload,
) -> Result<RecordHeader> {
    payload.check()?;
    let payload = &payload.clone().normalise();

    let vault_key = vaults::vault_key(store, master, vault_id)?;
    check_folder(store, vault_id, folder_id)?;

    let id = new_id();
    let record_key = SecretKey::random();

    let wrapped = wrap_key(&vault_key, &record_key, &key_aad(&id))?;
    let sealed = crypto::seal(
        &record_key,
        &serde_json::to_vec(payload)?,
        &payload_aad(&id, 1, vault_id),
    )?;

    store.conn().execute(
        "INSERT INTO records (
             id, vault_id, folder_id, kind, wrapped_key, sealed_payload, version,
             created_at, updated_at
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, 1,
             strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         )",
        params![id, vault_id, folder_id, kind, wrapped, sealed],
    )?;

    header(store, &id)
}

/// The headers in one vault, without opening a single payload.
///
/// This is what a list renders from. Opening every record to draw a list would mean decrypting the
/// whole vault to show it, which is exactly what the interface should not do.
pub fn list(store: &Store, vault_id: &str, include_archived: bool) -> Result<Vec<RecordHeader>> {
    let mut statement = store.conn().prepare(
        "SELECT id, vault_id, folder_id, kind, version, favourite, rotation_due_at,
                compromised_at, archived_at, created_at, updated_at
         FROM records
         WHERE vault_id = ?1 AND (?2 OR archived_at IS NULL)
         ORDER BY updated_at DESC",
    )?;

    let rows = statement.query_map(params![vault_id, include_archived], row_to_header)?;

    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// The list, with just enough of each record opened to name it.
///
/// A record whose payload will not open keeps its place in the list under a plain marker rather than
/// vanishing: a row that silently disappears is how somebody loses track of a secret they still
/// have.
pub fn list_with_previews(
    store: &Store,
    master: &SecretKey,
    vault_id: &str,
    include_archived: bool,
) -> Result<Vec<RecordPreview>> {
    let headers = list(store, vault_id, include_archived)?;
    let mut previews = Vec::with_capacity(headers.len());

    for header in headers {
        let opened = open(store, master, &header.id).ok();

        previews.push(match opened {
            Some(record) => RecordPreview {
                subtitle: record.payload.subtitle(),
                url: record.payload.first_url(),
                header: record.header,
                title: record.payload.title,
                icon: record.payload.icon,
            },
            None => RecordPreview {
                header,
                title: "(não foi possível abrir)".into(),
                subtitle: String::new(),
                url: String::new(),
                icon: None,
            },
        });
    }

    Ok(previews)
}

pub fn header(store: &Store, id: &str) -> Result<RecordHeader> {
    store
        .conn()
        .query_row(
            "SELECT id, vault_id, folder_id, kind, version, favourite, rotation_due_at,
                    compromised_at, archived_at, created_at, updated_at
             FROM records WHERE id = ?1",
            params![id],
            row_to_header,
        )
        .optional()?
        .ok_or_else(|| anyhow!("there is no record with that name here"))
}

/// Open one record.
///
/// The only path to a plaintext payload, so that "who read what, and when" has exactly one place to
/// be answered from when the audit trail arrives in F6.
pub fn open(store: &Store, master: &SecretKey, id: &str) -> Result<OpenRecord> {
    let header = header(store, id)?;

    let (wrapped, sealed): (Vec<u8>, Vec<u8>) = store.conn().query_row(
        "SELECT wrapped_key, sealed_payload FROM records WHERE id = ?1",
        params![id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    let vault_key = vaults::vault_key(store, master, &header.vault_id)?;
    let record_key = unwrap_key(&vault_key, &wrapped, &key_aad(id))
        .context("this record's key did not open")?;

    let bytes = crypto::open(
        &record_key,
        &sealed,
        &payload_aad(id, header.version, &header.vault_id),
    )
    .context("this record did not open")?;

    Ok(OpenRecord {
        header,
        payload: serde_json::from_slice::<Payload>(&bytes)
            .context("this record is not readable")?
            .normalise(),
    })
}

/// Write a new version of a record.
///
/// The previous ciphertext is kept in `record_versions` before the new one lands, so history is a
/// by-product of saving rather than a feature that has to remember to run. The version number goes
/// up by one and into the new blob's additional data, which is what makes the old blob unusable in
/// the new slot.
pub fn update(
    store: &mut Store,
    master: &SecretKey,
    id: &str,
    payload: &Payload,
) -> Result<RecordHeader> {
    payload.check()?;
    let payload = &payload.clone().normalise();

    let current = header(store, id)?;
    let next = current.version + 1;

    let (wrapped, sealed_before): (Vec<u8>, Vec<u8>) = store.conn().query_row(
        "SELECT wrapped_key, sealed_payload FROM records WHERE id = ?1",
        params![id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    let vault_key = vaults::vault_key(store, master, &current.vault_id)?;
    let record_key = unwrap_key(&vault_key, &wrapped, &key_aad(id))
        .context("this record's key did not open")?;

    let sealed = crypto::seal(
        &record_key,
        &serde_json::to_vec(payload)?,
        &payload_aad(id, next, &current.vault_id),
    )?;

    let transaction = store.conn_mut().transaction()?;

    transaction.execute(
        "INSERT OR REPLACE INTO record_versions (record_id, version, sealed_payload, created_at)
         VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        params![id, current.version, sealed_before],
    )?;

    transaction.execute(
        "UPDATE records SET sealed_payload = ?2, version = ?3,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![id, sealed, next],
    )?;

    transaction.commit()?;

    header(store, id)
}

/// Move a record to another folder, or to the vault's root.
///
/// One column. Nothing is re-encrypted, because a record's key belongs to its vault rather than to
/// the folder it happens to sit in.
pub fn move_to_folder(store: &Store, id: &str, folder_id: Option<&str>) -> Result<()> {
    let current = header(store, id)?;
    check_folder(store, &current.vault_id, folder_id)?;

    store.conn().execute(
        "UPDATE records SET folder_id = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![id, folder_id],
    )?;

    Ok(())
}

/// Move a record to another vault.
///
/// The one move that re-encrypts. A record's key is wrapped under its vault's key, and its payload
/// is sealed with the vault id in the additional data — so a row that changed `vault_id` and
/// nothing else would be a row nobody can open, its own history included. Everything that names the
/// old vault is rebuilt here in one transaction: the wrapped record key, the current payload, every
/// past version, and the wrapped key of every attachment.
///
/// **The record key itself stays.** It never left this file, and the vault it is moving to is under
/// the same master password; minting a new one would rewrite every version's ciphertext and buy
/// nothing. What changes is who wraps it, which is exactly what "this is in that vault now" means —
/// including for `agent_access`, which is decided per vault.
///
/// A version that will not open stops the move instead of being dropped. A history with a hole in
/// it is worse than a record that stayed where it was.
pub fn move_to_vault(
    store: &mut Store,
    master: &SecretKey,
    id: &str,
    vault_id: &str,
    folder_id: Option<&str>,
) -> Result<()> {
    let current = header(store, id)?;

    if current.vault_id == vault_id {
        return move_to_folder(store, id, folder_id);
    }

    let from = vaults::vault_key(store, master, &current.vault_id)?;
    let to = vaults::vault_key(store, master, vault_id)?;

    check_folder(store, vault_id, folder_id)?;

    let (wrapped, sealed): (Vec<u8>, Vec<u8>) = store.conn().query_row(
        "SELECT wrapped_key, sealed_payload FROM records WHERE id = ?1",
        params![id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    let record_key =
        unwrap_key(&from, &wrapped, &key_aad(id)).context("this record's key did not open")?;

    let rewrapped = wrap_key(&to, &record_key, &key_aad(id))?;

    let bytes = crypto::open(
        &record_key,
        &sealed,
        &payload_aad(id, current.version, &current.vault_id),
    )
    .context("this record did not open")?;

    let resealed = crypto::seal(&record_key, &bytes, &payload_aad(id, current.version, vault_id))?;

    let older = {
        let mut statement = store
            .conn()
            .prepare("SELECT version, sealed_payload FROM record_versions WHERE record_id = ?1")?;

        let rows = statement.query_map(params![id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;

        let collected = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        collected
    };

    let mut moved = Vec::with_capacity(older.len());

    for (version, blob) in older {
        let bytes = crypto::open(&record_key, &blob, &payload_aad(id, version, &current.vault_id))
            .with_context(|| format!("version {version} of this record did not open"))?;

        moved.push((
            version,
            crypto::seal(&record_key, &bytes, &payload_aad(id, version, vault_id))?,
        ));
    }

    let transaction = store.conn_mut().transaction()?;

    transaction.execute(
        "UPDATE records SET vault_id = ?2, folder_id = ?3, wrapped_key = ?4, sealed_payload = ?5,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![id, vault_id, folder_id, rewrapped, resealed],
    )?;

    for (version, blob) in moved {
        transaction.execute(
            "UPDATE record_versions SET sealed_payload = ?3 WHERE record_id = ?1 AND version = ?2",
            params![id, version, blob],
        )?;
    }

    crate::attachments::rewrap_for_vault(&transaction, id, &from, &to)?;

    transaction.commit()?;

    Ok(())
}

pub fn set_favourite(store: &Store, id: &str, favourite: bool) -> Result<()> {
    changed(store.conn().execute(
        "UPDATE records SET favourite = ?2 WHERE id = ?1",
        params![id, favourite as i64],
    )?)
}

/// Say that this secret is known to somebody it should not be.
pub fn set_compromised(store: &Store, id: &str, compromised: bool) -> Result<()> {
    changed(store.conn().execute(
        "UPDATE records SET compromised_at =
             CASE WHEN ?2 THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now') END
         WHERE id = ?1",
        params![id, compromised],
    )?)
}

pub fn set_rotation_due(store: &Store, id: &str, due: Option<&str>) -> Result<()> {
    changed(store.conn().execute(
        "UPDATE records SET rotation_due_at = ?2 WHERE id = ?1",
        params![id, due],
    )?)
}

/// Delete a record — which means archiving it.
///
/// There is no undo anywhere else in this app: nothing can bring back a secret whose ciphertext is
/// gone. So deleting puts a record in the trash and F5 decides when the trash empties.
pub fn archive(store: &Store, id: &str, archived: bool) -> Result<()> {
    changed(store.conn().execute(
        "UPDATE records SET archived_at =
             CASE WHEN ?2 THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now') END,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![id, archived],
    )?)
}

pub fn history(store: &Store, id: &str) -> Result<Vec<VersionEntry>> {
    let mut statement = store.conn().prepare(
        "SELECT version, created_at FROM record_versions WHERE record_id = ?1
         ORDER BY version DESC",
    )?;

    let rows = statement.query_map(params![id], |row| {
        Ok(VersionEntry { version: row.get(0)?, created_at: row.get(1)? })
    })?;

    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Read one old version, without making it current.
pub fn read_version(
    store: &Store,
    master: &SecretKey,
    id: &str,
    version: i64,
) -> Result<Payload> {
    let current = header(store, id)?;

    if version == current.version {
        return Ok(open(store, master, id)?.payload);
    }

    let sealed: Vec<u8> = store
        .conn()
        .query_row(
            "SELECT sealed_payload FROM record_versions WHERE record_id = ?1 AND version = ?2",
            params![id, version],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| anyhow!("this record has no version {version}"))?;

    let wrapped: Vec<u8> = store.conn().query_row(
        "SELECT wrapped_key FROM records WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )?;

    let vault_key = vaults::vault_key(store, master, &current.vault_id)?;
    let record_key = unwrap_key(&vault_key, &wrapped, &key_aad(id))?;

    let bytes = crypto::open(
        &record_key,
        &sealed,
        &payload_aad(id, version, &current.vault_id),
    )
    .context("that version did not open")?;

    Ok(serde_json::from_slice::<Payload>(&bytes)
        .context("that version is not readable")?
        .normalise())
}

/// Bring an old version back as a new one.
///
/// Restoring writes forward rather than rewinding: the version number keeps climbing, and what was
/// current before the restore is itself kept. Nothing in this vault's history is ever overwritten.
pub fn restore_version(
    store: &mut Store,
    master: &SecretKey,
    id: &str,
    version: i64,
) -> Result<RecordHeader> {
    let payload = read_version(store, master, id, version)?;

    update(store, master, id, &payload)
}

/// Destroy an archived record, for good.
///
/// This is the only operation in keyhold with nothing behind it: the ciphertext and every past
/// version go, and no key, backup or master password brings them back. Which is exactly why it is
/// separate from `archive`, and why the interface asks before calling it.
pub fn destroy(store: &Store, id: &str) -> Result<()> {
    let header = header(store, id)?;

    if !header.archived {
        bail!("this record is not in the trash");
    }

    // The versions go with it: `record_versions` cascades on delete, and leaving old ciphertext
    // behind for a record nobody can name is how a "deleted" secret survives a deletion.
    changed(store.conn().execute("DELETE FROM records WHERE id = ?1", params![id])?)
}

/// Empty the trash of everything that has sat there past the retention window.
///
/// Returns how many records were destroyed. Called on unlock rather than from a timer: keyhold only
/// runs while somebody is using it, so "every day at 04:15" would mean a purge that happens when
/// the machine happens to be awake — and a schedule that fires unpredictably is worse than one that
/// fires at a moment the person can see.
pub fn purge_expired(store: &Store, retention_days: i64) -> Result<usize> {
    if retention_days <= 0 {
        return Ok(0);
    }

    let removed = store.conn().execute(
        "DELETE FROM records
         WHERE archived_at IS NOT NULL
           AND archived_at <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?1)",
        params![format!("-{retention_days} days")],
    )?;

    Ok(removed)
}

/// What is in the trash, with how long each item has left.
pub fn trash(store: &Store, master: &SecretKey, vault_id: &str) -> Result<Vec<RecordPreview>> {
    let all = list_with_previews(store, master, vault_id, true)?;

    Ok(all.into_iter().filter(|preview| preview.header.archived).collect())
}

fn check_folder(store: &Store, vault_id: &str, folder_id: Option<&str>) -> Result<()> {
    let Some(folder_id) = folder_id else {
        return Ok(());
    };

    let owner: Option<String> = store
        .conn()
        .query_row(
            "SELECT vault_id FROM folders WHERE id = ?1",
            params![folder_id],
            |row| row.get(0),
        )
        .optional()?;

    match owner.as_deref() {
        Some(owner) if owner == vault_id => Ok(()),
        Some(_) => bail!("that folder belongs to another vault"),
        None => bail!("there is no folder with that name here"),
    }
}

fn changed(rows: usize) -> Result<()> {
    if rows == 0 {
        bail!("there is no record with that name here");
    }

    Ok(())
}

fn row_to_header(row: &rusqlite::Row<'_>) -> rusqlite::Result<RecordHeader> {
    Ok(RecordHeader {
        id: row.get(0)?,
        vault_id: row.get(1)?,
        folder_id: row.get(2)?,
        kind: row.get(3)?,
        version: row.get(4)?,
        favourite: row.get::<_, i64>(5)? != 0,
        rotation_due_at: row.get(6)?,
        compromised_at: row.get(7)?,
        archived: row.get::<_, Option<String>>(8)?.is_some(),
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{self, SetupOptions};
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
            VaultMeta {
                name: "Trabalho".into(),
                description: String::new(),
                colour: "brand".into(),
            },
        )
        .unwrap();

        (dir, store, outcome.vault_key, vault.id)
    }

    fn login(title: &str, password: &str) -> Payload {
        Payload {
            title: title.into(),
            username: "daniel".into(),
            url: "https://example.com".into(),
            fields: vec![Field::new("Senha", "password", password)],
            ..Payload::default()
        }
    }

    #[test]
    fn a_record_comes_back_exactly_as_it_went_in() {
        let (_dir, store, master, vault) = scratch();

        let made =
            create(&store, &master, &vault, None, "login", &login("Servidor", "hunter2")).unwrap();

        let opened = open(&store, &master, &made.id).unwrap();

        assert_eq!(opened.payload.title, "Servidor");
        assert_eq!(opened.payload.primary_secret().unwrap().value, "hunter2");
        assert_eq!(opened.header.version, 1);
    }

    #[test]
    fn nothing_readable_reaches_the_file() {
        let (dir, store, master, vault) = scratch();

        create(
            &store,
            &master,
            &vault,
            None,
            "login",
            &login("Servidor de produção", "hunter2"),
        )
        .unwrap();

        drop(store);

        let raw = std::fs::read(dir.path().join("vault.db")).unwrap();
        let text = String::from_utf8_lossy(&raw);

        for secret in ["hunter2", "Servidor de produção", "daniel", "example.com"] {
            assert!(!text.contains(secret), "`{secret}` is sitting in the file in the clear");
        }
    }

    #[test]
    fn the_list_never_opens_a_payload() {
        let (_dir, store, master, vault) = scratch();
        create(&store, &master, &vault, None, "login", &login("Um", "senha")).unwrap();

        // No master key is even offered to `list`, which is the point: drawing a list must not mean
        // decrypting the vault.
        let headers = list(&store, &vault, false).unwrap();

        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].kind, "login");
    }

    #[test]
    fn saving_keeps_the_version_that_came_before() {
        let (_dir, mut store, master, vault) = scratch();
        let made = create(&store, &master, &vault, None, "login", &login("Servidor", "old")).unwrap();

        let updated = update(&mut store, &master, &made.id, &login("Servidor", "new")).unwrap();

        assert_eq!(updated.version, 2);
        assert_eq!(open(&store, &master, &made.id).unwrap().payload.primary_secret().unwrap().value, "new");

        let past = history(&store, &made.id).unwrap();
        assert_eq!(past.len(), 1);
        assert_eq!(read_version(&store, &master, &made.id, 1).unwrap().primary_secret().unwrap().value, "old");
    }

    #[test]
    fn restoring_writes_forward_rather_than_rewinding() {
        let (_dir, mut store, master, vault) = scratch();
        let made = create(&store, &master, &vault, None, "login", &login("Servidor", "first")).unwrap();

        update(&mut store, &master, &made.id, &login("Servidor", "second")).unwrap();
        let restored = restore_version(&mut store, &master, &made.id, 1).unwrap();

        assert_eq!(restored.version, 3, "restoring rewound the version instead of advancing it");
        assert_eq!(open(&store, &master, &made.id).unwrap().payload.primary_secret().unwrap().value, "first");

        // And what was current before the restore is still there.
        assert_eq!(read_version(&store, &master, &made.id, 2).unwrap().primary_secret().unwrap().value, "second");
    }

    #[test]
    fn an_old_ciphertext_cannot_be_pasted_over_a_new_one() {
        let (_dir, mut store, master, vault) = scratch();
        let made = create(&store, &master, &vault, None, "login", &login("Servidor", "old")).unwrap();

        let stale: Vec<u8> = store
            .conn()
            .query_row(
                "SELECT sealed_payload FROM records WHERE id = ?1",
                params![made.id],
                |row| row.get(0),
            )
            .unwrap();

        update(&mut store, &master, &made.id, &login("Servidor", "rotated")).unwrap();

        // Somebody with write access to the file puts the pre-rotation blob back.
        store
            .conn()
            .execute(
                "UPDATE records SET sealed_payload = ?2 WHERE id = ?1",
                params![made.id, stale],
            )
            .unwrap();

        assert!(
            open(&store, &master, &made.id).is_err(),
            "a rolled-back payload opened as if it were current"
        );
    }

    #[test]
    fn a_payload_cannot_be_moved_between_records() {
        let (_dir, store, master, vault) = scratch();
        let one = create(&store, &master, &vault, None, "login", &login("Um", "a")).unwrap();
        let two = create(&store, &master, &vault, None, "login", &login("Dois", "b")).unwrap();

        let borrowed: Vec<u8> = store
            .conn()
            .query_row(
                "SELECT sealed_payload FROM records WHERE id = ?1",
                params![one.id],
                |row| row.get(0),
            )
            .unwrap();

        store
            .conn()
            .execute(
                "UPDATE records SET sealed_payload = ?2 WHERE id = ?1",
                params![two.id, borrowed],
            )
            .unwrap();

        assert!(open(&store, &master, &two.id).is_err(), "a record opened another record's payload");
    }

    #[test]
    fn one_flipped_byte_makes_a_record_fail_rather_than_lie() {
        let (_dir, store, master, vault) = scratch();
        let made = create(&store, &master, &vault, None, "login", &login("Servidor", "senha")).unwrap();

        let mut sealed: Vec<u8> = store
            .conn()
            .query_row(
                "SELECT sealed_payload FROM records WHERE id = ?1",
                params![made.id],
                |row| row.get(0),
            )
            .unwrap();

        let last = sealed.len() - 1;
        sealed[last] ^= 0x01;

        store
            .conn()
            .execute(
                "UPDATE records SET sealed_payload = ?2 WHERE id = ?1",
                params![made.id, sealed],
            )
            .unwrap();

        assert!(open(&store, &master, &made.id).is_err());
    }

    #[test]
    fn deleting_means_archiving() {
        let (_dir, store, master, vault) = scratch();
        let made = create(&store, &master, &vault, None, "login", &login("Servidor", "senha")).unwrap();

        archive(&store, &made.id, true).unwrap();

        assert!(list(&store, &vault, false).unwrap().is_empty());
        assert_eq!(list(&store, &vault, true).unwrap().len(), 1);

        // And it is still readable, which is what makes restoring possible at all.
        assert_eq!(open(&store, &master, &made.id).unwrap().payload.title, "Servidor");

        archive(&store, &made.id, false).unwrap();
        assert_eq!(list(&store, &vault, false).unwrap().len(), 1);
    }

    #[test]
    fn a_record_cannot_be_filed_in_another_vaults_folder() {
        let (_dir, store, master, vault) = scratch();

        let other = vaults::create_vault(
            &store,
            &master,
            VaultMeta { name: "Pessoal".into(), description: String::new(), colour: "brand".into() },
        )
        .unwrap();

        let stray = vaults::create_folder(
            &store,
            &master,
            &other.id,
            None,
            crate::vaults::FolderMeta { name: "Lá".into(), colour: None },
        )
        .unwrap();

        assert!(create(&store, &master, &vault, Some(&stray.id), "login", &login("X", "y")).is_err());

        let made = create(&store, &master, &vault, None, "login", &login("X", "y")).unwrap();
        assert!(move_to_folder(&store, &made.id, Some(&stray.id)).is_err());
    }

    /// Somebody with a spare vault and a text editor cannot move a record by hand.
    ///
    /// This is the reason `move_to_vault` exists at all: the vault id is in the payload's
    /// additional data, so an `UPDATE records SET vault_id` is a record that stops opening.
    #[test]
    fn changing_the_vault_column_by_hand_breaks_the_record() {
        let (_dir, store, master, vault) = scratch();

        let other = vaults::create_vault(
            &store,
            &master,
            VaultMeta { name: "Pessoal".into(), description: String::new(), colour: "brand".into() },
        )
        .unwrap();

        let made = create(&store, &master, &vault, None, "login", &login("Servidor", "senha")).unwrap();

        store
            .conn()
            .execute(
                "UPDATE records SET vault_id = ?2 WHERE id = ?1",
                params![made.id, other.id],
            )
            .unwrap();

        assert!(open(&store, &master, &made.id).is_err(), "a hand-edited move opened anyway");
    }

    #[test]
    fn moving_between_vaults_carries_the_record_and_its_history() {
        let (_dir, mut store, master, vault) = scratch();

        let other = vaults::create_vault(
            &store,
            &master,
            VaultMeta { name: "Pessoal".into(), description: String::new(), colour: "brand".into() },
        )
        .unwrap();

        let made = create(&store, &master, &vault, None, "login", &login("Servidor", "primeira")).unwrap();
        update(&mut store, &master, &made.id, &login("Servidor", "segunda")).unwrap();

        let before: Vec<u8> = store
            .conn()
            .query_row(
                "SELECT sealed_payload FROM records WHERE id = ?1",
                params![made.id],
                |row| row.get(0),
            )
            .unwrap();

        move_to_vault(&mut store, &master, &made.id, &other.id, None).unwrap();

        let moved = open(&store, &master, &made.id).unwrap();

        assert_eq!(moved.header.vault_id, other.id);
        assert_eq!(moved.payload.primary_secret().unwrap().value, "segunda");

        // The version written before the move still opens, which is the part a naive move loses.
        let old = read_version(&store, &master, &made.id, 1).unwrap();
        assert_eq!(old.primary_secret().unwrap().value, "primeira");

        let after: Vec<u8> = store
            .conn()
            .query_row(
                "SELECT sealed_payload FROM records WHERE id = ?1",
                params![made.id],
                |row| row.get(0),
            )
            .unwrap();

        assert_ne!(before, after, "the payload was not re-sealed under the new vault");

        assert!(list(&store, &vault, true).unwrap().is_empty(), "it is still listed where it was");
        assert_eq!(list(&store, &other.id, true).unwrap().len(), 1);
    }

    #[test]
    fn a_moved_record_takes_its_attachments_with_it() {
        let (_dir, mut store, master, vault) = scratch();

        let other = vaults::create_vault(
            &store,
            &master,
            VaultMeta { name: "Pessoal".into(), description: String::new(), colour: "brand".into() },
        )
        .unwrap();

        let made = create(&store, &master, &vault, None, "login", &login("Servidor", "senha")).unwrap();

        let file =
            crate::attachments::add(&store, &master, &made.id, "chave.pem", "text/plain", b"---")
                .unwrap();

        move_to_vault(&mut store, &master, &made.id, &other.id, None).unwrap();

        let reopened = crate::attachments::open(&store, &master, &file.id).unwrap();

        assert_eq!(reopened.bytes, b"---");
    }

    #[test]
    fn a_move_can_file_the_record_in_the_new_vaults_folder_and_nowhere_else() {
        let (_dir, mut store, master, vault) = scratch();

        let other = vaults::create_vault(
            &store,
            &master,
            VaultMeta { name: "Pessoal".into(), description: String::new(), colour: "brand".into() },
        )
        .unwrap();

        let here = vaults::create_folder(
            &store,
            &master,
            &vault,
            None,
            crate::vaults::FolderMeta { name: "Aqui".into(), colour: None },
        )
        .unwrap();

        let there = vaults::create_folder(
            &store,
            &master,
            &other.id,
            None,
            crate::vaults::FolderMeta { name: "Lá".into(), colour: None },
        )
        .unwrap();

        let made = create(&store, &master, &vault, Some(&here.id), "login", &login("X", "y")).unwrap();

        assert!(
            move_to_vault(&mut store, &master, &made.id, &other.id, Some(&here.id)).is_err(),
            "a record was filed in a folder of the vault it just left"
        );

        // The refused move left everything where it was, rather than half-way.
        assert_eq!(header(&store, &made.id).unwrap().vault_id, vault);

        move_to_vault(&mut store, &master, &made.id, &other.id, Some(&there.id)).unwrap();

        let moved = header(&store, &made.id).unwrap();

        assert_eq!(moved.vault_id, other.id);
        assert_eq!(moved.folder_id.as_deref(), Some(there.id.as_str()));
    }

    #[test]
    fn moving_between_folders_re_encrypts_nothing() {
        let (_dir, store, master, vault) = scratch();

        let folder = vaults::create_folder(
            &store,
            &master,
            &vault,
            None,
            crate::vaults::FolderMeta { name: "Clientes".into(), colour: None },
        )
        .unwrap();

        let made = create(&store, &master, &vault, None, "login", &login("Servidor", "senha")).unwrap();

        let before: Vec<u8> = store
            .conn()
            .query_row(
                "SELECT sealed_payload FROM records WHERE id = ?1",
                params![made.id],
                |row| row.get(0),
            )
            .unwrap();

        move_to_folder(&store, &made.id, Some(&folder.id)).unwrap();

        let after: Vec<u8> = store
            .conn()
            .query_row(
                "SELECT sealed_payload FROM records WHERE id = ?1",
                params![made.id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(before, after, "moving a record rewrote its ciphertext");
        assert_eq!(header(&store, &made.id).unwrap().folder_id.as_deref(), Some(folder.id.as_str()));
    }

    #[test]
    fn custom_fields_keep_the_order_they_were_given() {
        let (_dir, store, master, vault) = scratch();

        let payload = Payload {
            title: "Chave".into(),
            fields: vec![
                Field::new("Porta", "text", "2222"),
                Field::new("Chave privada", "hidden", "—"),
                Field::new("Frase", "password", "s"),
            ],
            ..Payload::default()
        };

        let made = create(&store, &master, &vault, None, "ssh_key", &payload).unwrap();
        let opened = open(&store, &master, &made.id).unwrap();

        let labels: Vec<&str> = opened.payload.fields.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(labels, ["Porta", "Chave privada", "Frase"]);
    }

    #[test]
    fn a_preview_names_the_record_without_handing_over_its_fields() {
        let (_dir, store, master, vault) = scratch();
        create(&store, &master, &vault, None, "login", &login("Servidor", "hunter2")).unwrap();

        let previews = list_with_previews(&store, &master, &vault, false).unwrap();
        let rendered = serde_json::to_string(&previews).unwrap();

        assert_eq!(previews[0].title, "Servidor");
        assert_eq!(previews[0].subtitle, "daniel");
        assert!(!rendered.contains("hunter2"), "a list preview carried a secret: {rendered}");
        assert!(!rendered.contains("fields"), "a list preview carried the fields: {rendered}");
    }

    #[test]
    fn a_record_that_will_not_open_still_keeps_its_place_in_the_list() {
        let (_dir, store, master, vault) = scratch();
        let made = create(&store, &master, &vault, None, "login", &login("Servidor", "x")).unwrap();

        let mut sealed: Vec<u8> = store
            .conn()
            .query_row(
                "SELECT sealed_payload FROM records WHERE id = ?1",
                params![made.id],
                |row| row.get(0),
            )
            .unwrap();

        sealed[0] ^= 0xff;

        store
            .conn()
            .execute(
                "UPDATE records SET sealed_payload = ?2 WHERE id = ?1",
                params![made.id, sealed],
            )
            .unwrap();

        let previews = list_with_previews(&store, &master, &vault, false).unwrap();

        assert_eq!(previews.len(), 1, "a damaged record vanished from the list");
        assert!(previews[0].title.contains("não foi possível"));
    }

    #[test]
    fn destroying_is_only_possible_from_the_trash() {
        let (_dir, store, master, vault) = scratch();
        let made = create(&store, &master, &vault, None, "login", &login("Servidor", "x")).unwrap();

        assert!(destroy(&store, &made.id).is_err(), "a live record was destroyed outright");

        archive(&store, &made.id, true).unwrap();
        destroy(&store, &made.id).unwrap();

        assert!(header(&store, &made.id).is_err());
    }

    #[test]
    fn destroying_takes_the_history_with_it() {
        let (_dir, mut store, master, vault) = scratch();
        let made = create(&store, &master, &vault, None, "login", &login("Servidor", "old")).unwrap();

        update(&mut store, &master, &made.id, &login("Servidor", "new")).unwrap();
        assert_eq!(history(&store, &made.id).unwrap().len(), 1);

        archive(&store, &made.id, true).unwrap();
        destroy(&store, &made.id).unwrap();

        let left: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM record_versions WHERE record_id = ?1",
                params![made.id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(left, 0, "an old ciphertext outlived the record it belonged to");
    }

    #[test]
    fn the_purge_only_takes_what_has_waited_long_enough() {
        let (_dir, store, master, vault) = scratch();

        let recent = create(&store, &master, &vault, None, "login", &login("Recente", "x")).unwrap();
        let old = create(&store, &master, &vault, None, "login", &login("Antigo", "x")).unwrap();
        let alive = create(&store, &master, &vault, None, "login", &login("Vivo", "x")).unwrap();

        archive(&store, &recent.id, true).unwrap();
        archive(&store, &old.id, true).unwrap();

        // Age one of them past the window by hand rather than waiting a month.
        store
            .conn()
            .execute(
                "UPDATE records SET archived_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-40 days')
                 WHERE id = ?1",
                params![old.id],
            )
            .unwrap();

        assert_eq!(purge_expired(&store, 30).unwrap(), 1);

        assert!(header(&store, &old.id).is_err(), "the expired record survived");
        assert!(header(&store, &recent.id).is_ok(), "a fresh deletion was purged early");
        assert!(header(&store, &alive.id).is_ok(), "a live record was purged");
    }

    #[test]
    fn retention_switched_off_purges_nothing() {
        let (_dir, store, master, vault) = scratch();
        let made = create(&store, &master, &vault, None, "login", &login("Antigo", "x")).unwrap();

        archive(&store, &made.id, true).unwrap();
        store
            .conn()
            .execute(
                "UPDATE records SET archived_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-400 days')
                 WHERE id = ?1",
                params![made.id],
            )
            .unwrap();

        assert_eq!(purge_expired(&store, 0).unwrap(), 0);
        assert!(header(&store, &made.id).is_ok());
    }

    #[test]
    fn the_trash_lists_only_what_was_deleted() {
        let (_dir, store, master, vault) = scratch();
        let kept = create(&store, &master, &vault, None, "login", &login("Vivo", "x")).unwrap();
        let gone = create(&store, &master, &vault, None, "login", &login("Excluido", "x")).unwrap();

        archive(&store, &gone.id, true).unwrap();

        let listed = trash(&store, &master, &vault).unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Excluido");
        assert!(listed.iter().all(|one| one.header.id != kept.id));
    }

    #[test]
    fn a_record_needs_a_title() {
        let (_dir, store, master, vault) = scratch();

        let blank = Payload { title: "  ".into(), ..Payload::default() };
        assert!(create(&store, &master, &vault, None, "login", &blank).is_err());
    }
}
