//! Getting a vault out, and getting one back in.
//!
//! # Export
//!
//! **It is never plaintext.** An export that opened in a text editor would turn "somebody exported
//! the vault" into "somebody's Downloads folder is the vault". The archive is sealed with a
//! passphrase typed for the occasion.
//!
//! **The passphrase is not the master password**, and is asked for separately on purpose: a file
//! that outlives the session should not be opened by the credential that opens everything else, and
//! being asked a second time is being asked, once, to think about where the file is going.
//!
//! **The format documents itself.** The envelope carries the KDF, its parameters, the salt and the
//! nonce in the clear, because an archive nobody can open in six months protects nothing — it just
//! loses the data. None of that weakens it: the passphrase is the secret, and everything else is
//! what any correct implementation would have to ask for anyway.
//!
//! # Import
//!
//! The classic reason to refuse imports is a CSV of plaintext passwords being uploaded to a server.
//! There is no server here and the file never leaves the machine, so the argument does not hold —
//! and refusing would mean telling somebody to retype four hundred passwords by hand, which is how
//! a vault ends up unused.
//!
//! Two formats: keyhold's own sealed archive, and plain CSV, because a CSV is what every other
//! password manager will hand you. The CSV path says plainly what it is reading and never guesses
//! at a column it does not recognise.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::crypto::{self, KdfParams, SecretKey, SALT_LEN};
use crate::db::Store;
use crate::records::{self, Field, Payload};
use crate::vaults;

/// What the archive says about itself, and what the ciphertext is bound to.
pub const FORMAT: &str = "keyhold.export";
pub const VERSION: u32 = 1;

/// Below this, the archive is not really sealed.
pub const MIN_PASSPHRASE: usize = 12;

/// One record, as it travels.
#[derive(Debug, Serialize, Deserialize)]
pub struct ExportedRecord {
    pub kind: String,
    pub folder: Option<String>,
    pub payload: Payload,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Archive {
    pub vault: String,
    pub exported_at: String,
    pub records: Vec<ExportedRecord>,
}

/// The envelope: everything needed to open the archive except the passphrase.
#[derive(Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub format: String,
    pub version: u32,
    pub kdf: String,
    pub kdf_params: KdfParams,
    /// Base64url, like every other opaque value this program writes down.
    pub salt: String,
    pub sealed: String,
}

fn archive_aad() -> Vec<u8> {
    format!("{FORMAT}:v{VERSION}").into_bytes()
}

/// Seal a whole vault into one archive.
pub fn export(
    store: &Store,
    master: &SecretKey,
    vault_id: &str,
    passphrase: &str,
) -> Result<Envelope> {
    if passphrase.chars().count() < MIN_PASSPHRASE {
        bail!("a senha do arquivo precisa de pelo menos {MIN_PASSPHRASE} caracteres");
    }

    let vault = vaults::read_vault(store, master, vault_id)?;
    let folders = vaults::list_folders(store, master, vault_id)?;

    let mut exported = Vec::new();

    for header in records::list(store, vault_id, false)? {
        let record = records::open(store, master, &header.id)?;

        exported.push(ExportedRecord {
            kind: record.header.kind,
            folder: record
                .header
                .folder_id
                .and_then(|id| folders.iter().find(|f| f.id == id).map(|f| f.name.clone())),
            payload: record.payload,
        });
    }

    let archive = Archive {
        vault: vault.name,
        exported_at: store.conn().query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
            [],
            |row| row.get(0),
        )?,
        records: exported,
    };

    let params = KdfParams::default();
    let salt = crypto::random_bytes(SALT_LEN);
    let key = crypto::derive_key(passphrase, &salt, params)?;

    let sealed = crypto::seal(&key, &serde_json::to_vec(&archive)?, &archive_aad())?;

    Ok(Envelope {
        format: FORMAT.into(),
        version: VERSION,
        kdf: "argon2id".into(),
        kdf_params: params,
        salt: crate::encoding::base64url(&salt),
        sealed: crate::encoding::base64url(&sealed),
    })
}

/// Open an archive produced by [`export`].
pub fn open_archive(envelope: &Envelope, passphrase: &str) -> Result<Archive> {
    if envelope.format != FORMAT {
        bail!("o arquivo não foi exportado pelo Keyhold");
    }

    if envelope.version > VERSION {
        bail!(
            "o arquivo foi gerado por uma versão mais nova do Keyhold (formato {}). Atualize o Keyhold para importá-lo",
            envelope.version
        );
    }

    envelope.kdf_params.validate()?;

    let salt = crate::encoding::from_base64url(&envelope.salt)
        .ok_or_else(|| anyhow!("o cabeçalho do arquivo está corrompido"))?;
    let sealed = crate::encoding::from_base64url(&envelope.sealed)
        .ok_or_else(|| anyhow!("o conteúdo do arquivo está corrompido"))?;

    let key = crypto::derive_key(passphrase, &salt, envelope.kdf_params)?;

    let bytes = crypto::open(&key, &sealed, &archive_aad())
        .map_err(|_| anyhow!("a senha não corresponde a este arquivo"))?;

    serde_json::from_slice(&bytes).context("o arquivo foi aberto, mas o conteúdo não é legível")
}

#[derive(Debug, Serialize)]
pub struct Imported {
    pub records: usize,
    pub folders: usize,
}

/// Write records into a vault, creating any folder they name.
///
/// Everything arrives as a new record: nothing is matched against what is already there and nothing
/// is overwritten. An import that silently merged would be an import that can quietly replace a
/// password with an older one, and the vault has no way to know which of the two you wanted.
pub fn import(
    store: &Store,
    master: &SecretKey,
    vault_id: &str,
    records_in: &[ExportedRecord],
) -> Result<Imported> {
    let mut folders = vaults::list_folders(store, master, vault_id)?;
    let mut created_folders = 0;

    for incoming in records_in {
        let folder_id = match incoming.folder.as_deref().filter(|name| !name.trim().is_empty()) {
            None => None,
            Some(name) => {
                if let Some(found) = folders.iter().find(|folder| folder.name == name) {
                    Some(found.id.clone())
                } else {
                    let made = vaults::create_folder(
                        store,
                        master,
                        vault_id,
                        None,
                        vaults::FolderMeta { name: name.to_string(), colour: None },
                    )?;

                    let id = made.id.clone();
                    folders.push(made);
                    created_folders += 1;

                    Some(id)
                }
            }
        };

        records::create(
            store,
            master,
            vault_id,
            folder_id.as_deref(),
            &incoming.kind,
            &incoming.payload,
        )?;
    }

    Ok(Imported { records: records_in.len(), folders: created_folders })
}

/// Read the CSV every other password manager exports.
///
/// Deliberately conservative: it reads the columns it recognises by name and puts anything else it
/// finds into a custom field named after its own column, rather than dropping it. A password
/// manager that silently loses a column is one you find out about months later.
pub fn from_csv(text: &str) -> Result<Vec<ExportedRecord>> {
    let mut rows = parse_csv(text);

    if rows.is_empty() {
        bail!("o arquivo está vazio");
    }

    let header: Vec<String> = rows.remove(0).into_iter().map(|cell| cell.to_lowercase()).collect();

    let column = |names: &[&str]| -> Option<usize> {
        header.iter().position(|cell| names.contains(&cell.trim()))
    };

    let title = column(&["name", "title", "nome", "título", "titulo", "account"])
        .ok_or_else(|| anyhow!("o CSV não possui uma coluna de nome (name, title, nome)"))?;
    // Bitwarden prefixes its login columns with `login_`.
    let username = column(&["username", "user", "login", "login_username", "usuário", "usuario", "email"]);
    let password = column(&["password", "senha", "pass", "login_password"]);
    let url = column(&["url", "uri", "login_uri", "website", "site", "endereço", "endereco"]);
    let totp = column(&["totp", "otpauth", "otp", "login_totp", "2fa"]);
    let notes = column(&["notes", "note", "notas", "observações", "comentário"]);
    let folder = column(&["folder", "group", "grouping", "pasta", "categoria"]);

    let known: Vec<usize> = [Some(title), username, password, url, notes, folder, totp]
        .into_iter()
        .flatten()
        .collect();

    let mut found = Vec::new();

    for row in rows {
        let cell = |at: Option<usize>| -> String {
            at.and_then(|index| row.get(index)).cloned().unwrap_or_default()
        };

        let name = cell(Some(title));

        if name.trim().is_empty() {
            continue;
        }

        let mut fields = Vec::new();

        if let Some(secret) = password.map(|at| cell(Some(at))).filter(|v| !v.is_empty()) {
            fields.push(Field::new("Senha", "password", &secret));
        }

        if let Some(seed) = totp.map(|at| cell(Some(at))).filter(|v| !v.trim().is_empty()) {
            fields.push(Field::new("Código 2FA", "totp", &seed));
        }

        // Whatever the file had that this does not understand, kept under its own column name.
        for (index, value) in row.iter().enumerate() {
            if known.contains(&index) || value.trim().is_empty() {
                continue;
            }

            // Kept hidden: a column this does not understand may well be a secret (an API key, a
            // recovery code), and a visible text field would surface it in lists and searches.
            let label = header.get(index).cloned().unwrap_or_else(|| format!("Coluna {index}"));
            fields.push(Field::new(&label, "hidden", value));
        }

        found.push(ExportedRecord {
            kind: "login".into(),
            folder: folder.map(|at| cell(Some(at))).filter(|v| !v.trim().is_empty()),
            payload: Payload {
                title: name,
                username: cell(username),
                url: cell(url),
                notes: cell(notes),
                fields,
                icon: None,
                alerts: Vec::new(),
            }
            .normalise(),
        });
    }

    if found.is_empty() {
        bail!("nenhuma linha do CSV possui um nome");
    }

    Ok(found)
}

/// A CSV reader, written out rather than pulled in.
///
/// It handles the one thing that actually matters and that a naive `split(',')` gets wrong: quoted
/// fields containing commas, newlines and escaped quotes. A password manager's export is full of
/// all three, and a parser that mangles them corrupts the data on the way in — silently.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                cell.push('"');
            }
            (true, '"') => quoted = false,
            (true, _) => cell.push(c),
            (false, '"') => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut cell)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            (false, _) => cell.push(c),
        }
    }

    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }

    rows.retain(|row| row.iter().any(|cell| !cell.trim().is_empty()));
    rows
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
            VaultMeta { name: "Trabalho".into(), description: String::new(), colour: "default".into(), ..Default::default() },
        )
        .unwrap();

        (dir, store, outcome.vault_key, vault.id)
    }

    fn login(title: &str, password: &str) -> Payload {
        Payload {
            title: title.into(),
            username: "deploy".into(),
            fields: vec![Field {
                label: "Senha".into(),
                kind: "password".into(),
                value: password.into(),
                ..Default::default()
            }],
            ..Payload::default()
        }
    }

    #[test]
    fn an_archive_round_trips() {
        let (_dir, store, master, vault) = scratch();
        records::create(&store, &master, &vault, None, "login", &login("Servidor", "hunter2")).unwrap();

        let envelope = export(&store, &master, &vault, "uma senha de arquivo").unwrap();
        let archive = open_archive(&envelope, "uma senha de arquivo").unwrap();

        assert_eq!(archive.vault, "Trabalho");
        assert_eq!(archive.records.len(), 1);
        assert_eq!(archive.records[0].payload.primary_secret().unwrap().value, "hunter2");
    }

    #[test]
    fn the_archive_is_not_readable_without_the_passphrase() {
        let (_dir, store, master, vault) = scratch();
        records::create(&store, &master, &vault, None, "login", &login("Servidor", "hunter2")).unwrap();

        let envelope = export(&store, &master, &vault, "uma senha de arquivo").unwrap();
        let rendered = serde_json::to_string(&envelope).unwrap();

        assert!(!rendered.contains("hunter2"), "the archive carried a secret in the clear");
        assert!(!rendered.contains("Servidor"));
        assert!(open_archive(&envelope, "outra senha inteira").is_err());
    }

    #[test]
    fn the_envelope_says_how_to_open_itself() {
        let (_dir, store, master, vault) = scratch();

        let envelope = export(&store, &master, &vault, "uma senha de arquivo").unwrap();

        assert_eq!(envelope.format, FORMAT);
        assert_eq!(envelope.kdf, "argon2id");
        assert!(envelope.kdf_params.memory_kib >= 64 * 1024);
        assert!(!envelope.salt.is_empty());
    }

    #[test]
    fn a_short_passphrase_is_refused() {
        let (_dir, store, master, vault) = scratch();

        assert!(export(&store, &master, &vault, "curta").is_err());
    }

    #[test]
    fn a_tampered_archive_does_not_open() {
        let (_dir, store, master, vault) = scratch();
        records::create(&store, &master, &vault, None, "login", &login("Servidor", "x")).unwrap();

        let mut envelope = export(&store, &master, &vault, "uma senha de arquivo").unwrap();

        let mut bytes = crate::encoding::from_base64url(&envelope.sealed).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        envelope.sealed = crate::encoding::base64url(&bytes);

        assert!(open_archive(&envelope, "uma senha de arquivo").is_err());
    }

    #[test]
    fn an_archive_weakened_in_the_header_is_refused() {
        let (_dir, store, master, vault) = scratch();
        let mut envelope = export(&store, &master, &vault, "uma senha de arquivo").unwrap();

        // Somebody edits the file to ask for a cheap derivation.
        envelope.kdf_params = KdfParams { memory_kib: 8, iterations: 1, parallelism: 1 };

        assert!(open_archive(&envelope, "uma senha de arquivo").is_err());
    }

    #[test]
    fn importing_an_archive_recreates_the_records_and_their_folders() {
        let (_dir, store, master, vault) = scratch();

        let incoming = vec![
            ExportedRecord {
                kind: "login".into(),
                folder: Some("Clientes".into()),
                payload: login("Servidor", "hunter2"),
            },
            ExportedRecord {
                kind: "login".into(),
                folder: Some("Clientes".into()),
                payload: login("Banco", "outro"),
            },
        ];

        let done = import(&store, &master, &vault, &incoming).unwrap();

        assert_eq!(done.records, 2);
        assert_eq!(done.folders, 1, "the folder should have been made once, not twice");

        let listed = records::list_with_previews(&store, &master, &vault, false).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|one| one.header.folder_id.is_some()));
    }

    #[test]
    fn a_csv_from_another_manager_is_read() {
        let csv = "name,url,username,password,note\n\
                   Servidor,https://exemplo.test,deploy,hunter2,uma nota\n\
                   Banco,,daniel,outra senha,\n";

        let found = from_csv(csv).unwrap();

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].payload.title, "Servidor");
        assert_eq!(found[0].payload.subtitle(), "deploy");
        assert_eq!(found[0].payload.first_url(), "https://exemplo.test");
        assert_eq!(found[0].payload.primary_secret().unwrap().value, "hunter2");
        assert_eq!(found[1].payload.title, "Banco");
        assert!(found[1].payload.primary_secret().unwrap().value == "outra senha");
    }

    #[test]
    fn a_csv_with_commas_and_quotes_inside_fields_survives() {
        let csv = "name,password,note\n\
                   \"Servidor, o principal\",\"tem \"\"aspas\"\" dentro\",\"uma nota\nem duas linhas\"\n";

        let found = from_csv(csv).unwrap();

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].payload.title, "Servidor, o principal");
        assert_eq!(found[0].payload.primary_secret().unwrap().value, "tem \"aspas\" dentro");
        assert_eq!(found[0].payload.notes, "uma nota\nem duas linhas");
    }

    #[test]
    fn a_column_the_reader_does_not_know_is_kept_rather_than_dropped() {
        let csv = "name,password,otp_secret\nServidor,hunter2,ABCDEF\n";

        let found = from_csv(csv).unwrap();
        let extra = found[0].payload.fields.iter().find(|f| f.label == "otp_secret");

        assert!(extra.is_some(), "an unrecognised column was silently dropped");
        assert_eq!(extra.unwrap().value, "ABCDEF");
    }

    #[test]
    fn a_csv_with_a_folder_column_files_the_records() {
        let csv = "name,password,folder\nServidor,x,Infra\nBanco,y,Infra\n";

        let found = from_csv(csv).unwrap();

        assert_eq!(found[0].folder.as_deref(), Some("Infra"));
        assert_eq!(found[1].folder.as_deref(), Some("Infra"));
    }

    #[test]
    fn a_csv_without_a_name_column_is_refused_rather_than_guessed_at() {
        assert!(from_csv("foo,bar\n1,2\n").is_err());
    }
}
