//! What Keyhold remembers about AWS between runs: the logins, the connections, the favourites.
//!
//! **Sealed, like everything else that names something.** A start URL says which organisation
//! somebody works for, an account id and a role say what they can reach, and a favourite names a
//! parameter. None of that is a secret in the cryptographic sense, but all of it is the kind of
//! thing the rest of this file refuses to leave readable, so it is one JSON document sealed under
//! the master key and kept in the settings table.
//!
//! **What is never here:** tokens, credentials, device codes, client secrets. A2 says those live
//! in memory only, and `aws::state` is where they do. A restart or a lock forgets them, and the
//! person logs in again.

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::crypto::{self, SecretKey};
use crate::db::Store;
use crate::encoding::{base64url, from_base64url};

const SETTING: &str = "aws_config";
const AAD: &[u8] = b"keyhold:aws-config:v1";

/// One IAM Identity Center sign-in: an organisation's start URL and the region of its directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Login {
    pub id: String,
    pub start_url: String,
    pub region: String,
    #[serde(default)]
    pub label: String,
}

/// One account reached through a login, with the role used there (A3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub login_id: String,
    pub account_id: String,
    #[serde(default)]
    pub account_name: String,
    pub role: String,
    /// What the interface calls it. Defaults to `<account id> / <role>`.
    #[serde(default)]
    pub title: String,
    /// A note for the person, never sent anywhere.
    #[serde(default)]
    pub note: String,
}

impl Connection {
    pub fn default_title(account_id: &str, role: &str) -> String {
        format!("{account_id} / {role}")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwsConfig {
    #[serde(default)]
    pub logins: Vec<Login>,
    #[serde(default)]
    pub connections: Vec<Connection>,
    /// `<account id>:<parameter name>`, for each parameter somebody starred (A14). Personal, so
    /// kept here rather than in a tag everybody with access to the account would see.
    #[serde(default)]
    pub favourites: Vec<String>,
}

impl AwsConfig {
    pub fn login(&self, id: &str) -> Option<&Login> {
        self.logins.iter().find(|login| login.id == id)
    }

    pub fn connection(&self, id: &str) -> Option<&Connection> {
        self.connections.iter().find(|connection| connection.id == id)
    }

    pub fn favourite_key(account_id: &str, name: &str) -> String {
        format!("{account_id}:{name}")
    }

    pub fn is_favourite(&self, account_id: &str, name: &str) -> bool {
        let key = Self::favourite_key(account_id, name);
        self.favourites.contains(&key)
    }

    pub fn set_favourite(&mut self, account_id: &str, name: &str, on: bool) {
        let key = Self::favourite_key(account_id, name);
        self.favourites.retain(|f| *f != key);
        if on {
            self.favourites.push(key);
        }
    }
}

/// The configuration, or an empty one if nothing was ever saved.
pub fn load(store: &Store, master: &SecretKey) -> Result<AwsConfig> {
    let raw: Option<String> = store
        .conn()
        .query_row("SELECT value FROM settings WHERE key = ?1", params![SETTING], |row| row.get(0))
        .optional()?;

    let Some(raw) = raw else {
        return Ok(AwsConfig::default());
    };

    let sealed = from_base64url(&raw).context("a configuração da AWS está danificada")?;
    let bytes = crypto::open(master, &sealed, AAD).context("não foi possível abrir a configuração da AWS")?;

    serde_json::from_slice(&bytes).context("a configuração da AWS não é legível")
}

pub fn save(store: &Store, master: &SecretKey, config: &AwsConfig) -> Result<()> {
    let sealed = crypto::seal(master, &serde_json::to_vec(config)?, AAD)?;

    store.conn().execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SETTING, base64url(&sealed)],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configuration_comes_back_and_is_not_readable_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("vault.db")).unwrap();
        let key = SecretKey::random();

        assert_eq!(load(&store, &key).unwrap(), AwsConfig::default());

        let mut config = AwsConfig::default();
        config.logins.push(Login {
            id: "L".into(),
            start_url: "https://empresa-secreta.awsapps.com/start".into(),
            region: "sa-east-1".into(),
            label: "Empresa".into(),
        });
        config.set_favourite("123456789012", "/app/db", true);
        save(&store, &key, &config).unwrap();

        assert_eq!(load(&store, &key).unwrap(), config);
        assert!(load(&store, &SecretKey::random()).is_err());

        let raw: String = store
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", params![SETTING], |r| r.get(0))
            .unwrap();
        assert!(!raw.contains("empresa-secreta"));
    }

    #[test]
    fn a_favourite_is_keyed_by_account_and_name() {
        let mut config = AwsConfig::default();
        config.set_favourite("111", "/a", true);
        config.set_favourite("111", "/a", true);
        assert_eq!(config.favourites, vec!["111:/a".to_string()]);
        assert!(config.is_favourite("111", "/a"));
        assert!(!config.is_favourite("222", "/a"));
        config.set_favourite("111", "/a", false);
        assert!(config.favourites.is_empty());
    }
}
