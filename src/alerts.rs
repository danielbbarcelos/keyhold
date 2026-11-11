//! Alerts: a coloured note with an icon, a title and a line of text that a record wears at the top,
//! where "IMPORTANTE: não compartilhar com terceiros!" used to be buried in the notes.
//!
//! A record keeps its alerts in its payload, sealed with it. The presets kept here are the
//! person's standard alerts; a record that uses one keeps both the preset's id and a copy of its
//! content. On this machine the preset wins, so changing it in Ajustes changes every record that
//! uses it; anywhere the preset is unknown (another machine, someone else opening an AWS vault) the
//! copy is shown, so an alert never arrives empty.
//!
//! The presets are sealed under the master key, like the AWS configuration: an alert's words can
//! say something about what the vault holds.

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::crypto::{self, SecretKey};
use crate::db::Store;
use crate::encoding::{base64url, from_base64url};

const SETTING: &str = "alert_presets";
const AAD: &[u8] = b"keyhold:alert-presets:v1";

/// The most presets kept, and the longest a title or text may be.
const MAX_PRESETS: usize = 50;
const MAX_TITLE: usize = 80;
const MAX_TEXT: usize = 400;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    /// One of the interface's soft colours (`rose`, `amber`, `sky`...).
    pub colour: String,
    /// A lucide icon name.
    pub icon: String,
    pub title: String,
    #[serde(default)]
    pub text: String,
}

/// What a person who never set anything up finds: the one they kept writing by hand.
fn starters() -> Vec<Preset> {
    vec![
        Preset {
            id: "nao-compartilhar".into(),
            colour: "rose".into(),
            icon: "shield-alert".into(),
            title: "Não compartilhar com terceiros".into(),
            text: "Este acesso é pessoal. Não envie a senha por mensagem nem a passe para outra pessoa.".into(),
        },
        Preset {
            id: "uso-interno".into(),
            colour: "amber".into(),
            icon: "building-2".into(),
            title: "Uso interno".into(),
            text: "Só para a equipe. Não use fora dos sistemas da empresa.".into(),
        },
    ]
}

fn clipped(text: &str, max: usize) -> String {
    text.trim().chars().take(max).collect()
}

fn sanitised(presets: Vec<Preset>) -> Vec<Preset> {
    presets
        .into_iter()
        .filter(|p| !p.id.trim().is_empty() && !p.title.trim().is_empty())
        .take(MAX_PRESETS)
        .map(|p| Preset {
            id: clipped(&p.id, 64),
            colour: clipped(&p.colour, 16),
            icon: clipped(&p.icon, 32),
            title: clipped(&p.title, MAX_TITLE),
            text: clipped(&p.text, MAX_TEXT),
        })
        .collect()
}

/// The presets, or the starters when none were ever saved.
pub fn load(store: &Store, master: &SecretKey) -> Result<Vec<Preset>> {
    let raw: Option<String> = store
        .conn()
        .query_row("SELECT value FROM settings WHERE key = ?1", params![SETTING], |row| row.get(0))
        .optional()?;

    let Some(raw) = raw else {
        return Ok(starters());
    };

    let sealed = from_base64url(&raw).context("os alertas padrão estão danificados")?;
    let bytes = crypto::open(master, &sealed, AAD).context("os alertas padrão não abriram")?;

    serde_json::from_slice(&bytes).context("os alertas padrão não são legíveis")
}

pub fn save(store: &Store, master: &SecretKey, presets: Vec<Preset>) -> Result<Vec<Preset>> {
    let presets = sanitised(presets);
    let sealed = crypto::seal(master, &serde_json::to_vec(&presets)?, AAD)?;

    store.conn().execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SETTING, base64url(&sealed)],
    )?;

    Ok(presets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_sealed_and_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("vault.db")).unwrap();
        let key = SecretKey::random();

        assert_eq!(load(&store, &key).unwrap(), starters(), "nothing saved yet: the starters");

        let mine = vec![Preset { id: "a".into(), colour: "sky".into(), icon: "info".into(), title: "Segredo do cliente".into(), text: String::new() }];
        save(&store, &key, mine.clone()).unwrap();

        let raw: String = store.conn().query_row("SELECT value FROM settings WHERE key = ?1", params![SETTING], |r| r.get(0)).unwrap();
        assert!(!raw.contains("cliente"), "the words are sealed");
        assert_eq!(load(&store, &key).unwrap(), mine);
        assert!(load(&store, &SecretKey::random()).is_err(), "another key opens nothing");
    }

    #[test]
    fn a_preset_without_a_title_is_dropped_and_long_text_is_cut() {
        let kept = sanitised(vec![
            Preset { id: "x".into(), title: " ".into(), ..Preset::default() },
            Preset { id: "y".into(), title: "T".repeat(500), text: "t".repeat(5000), ..Preset::default() },
        ]);

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].title.chars().count(), MAX_TITLE);
        assert_eq!(kept[0].text.chars().count(), MAX_TEXT);
    }
}
