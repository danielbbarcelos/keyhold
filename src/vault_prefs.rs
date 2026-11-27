//! What the person decided about a vault as a whole, apart from what it holds.
//!
//! Three things, one sealed row per vault:
//!
//!  - **Favourite**, for the vaults that go first on Início.
//!  - **How often it is opened**, a count and the last time, for "mais acessados". Counted here
//!    rather than in the page so that every tab and every restart adds to the same number.
//!  - **The agent's policy**: whether an assistant asking for something in this vault is asked
//!    about every time (the default), or let through until a date or for good.
//!
//! # The agent's policy, and why it covers a whole vault
//!
//! A standing permission in [`crate::grants`] is one field of one record for one use. That is the
//! right answer to a question about one field, and the wrong one for a vault somebody has decided,
//! as a whole, an assistant may work in: they would be asked again for every record, which is how
//! people learn to click yes without reading. So a vault can be let through, and the cost of that
//! is stated where it is chosen. What does not change with it:
//!
//!  1. **A secret is still never handed to an agent.** `get` of a secret field is refused whatever
//!     the policy says; the policy only removes the question before `run` and `copy`.
//!  2. **Every use is still in the trail**, naming the policy as the reason nobody was asked.
//!  3. **The vault still has to be available to agents** (`agent_access`): the policy is what
//!     happens after that door, never instead of it.
//!  4. **A locked vault ignores it.** The row is sealed with the master key, with the vault's id in
//!     the authenticated data, so it cannot be read, forged or moved to another vault without the
//!     key.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::crypto::{self, SecretKey};
use crate::db::Store;
use crate::identity::now_seconds;

/// What happens when an assistant asks for something in this vault.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum AgentPolicy {
    /// Every request is a question, as before this existed.
    #[default]
    Ask,
    /// Let through, without asking, until a moment on the clock (unix seconds).
    Until { at: u64 },
    /// Let through, without asking, until somebody changes it.
    Always,
}

impl AgentPolicy {
    /// Whether this policy lets a request through right now.
    pub fn lets_through(&self) -> bool {
        match self {
            AgentPolicy::Ask => false,
            AgentPolicy::Until { at } => *at > now_seconds(),
            AgentPolicy::Always => true,
        }
    }

    /// The words the trail uses for it. Never more than the dialog showed.
    pub fn describe(&self) -> String {
        match self {
            AgentPolicy::Ask => "sempre perguntar".into(),
            AgentPolicy::Until { at } => format!("liberado até {at}"),
            AgentPolicy::Always => "liberado sem prazo".into(),
        }
    }
}

/// Everything kept about one vault outside its contents.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Prefs {
    #[serde(default)]
    pub favorite: bool,
    /// How many times the vault's page was opened.
    #[serde(default)]
    pub opened: u64,
    /// Unix seconds of the last opening.
    #[serde(default)]
    pub last_opened_at: Option<u64>,
    #[serde(default)]
    pub agent: AgentPolicy,
}

fn aad(vault_id: &str) -> Vec<u8> {
    format!("keyhold:vault-prefs:v1|{vault_id}").into_bytes()
}

/// The vault's preferences, or the defaults when nothing was ever kept (or the row does not open,
/// which only an edited file does: the defaults are the cautious answer).
pub fn read(store: &Store, master: &SecretKey, vault_id: &str) -> Prefs {
    let sealed: Option<Vec<u8>> = store
        .conn()
        .query_row("SELECT sealed FROM vault_prefs WHERE vault_id = ?1", params![vault_id], |row| row.get(0))
        .optional()
        .ok()
        .flatten();

    sealed
        .and_then(|sealed| crypto::open(master, &sealed, &aad(vault_id)).ok())
        .and_then(|body| serde_json::from_slice(&body).ok())
        .unwrap_or_default()
}

fn write(store: &Store, master: &SecretKey, vault_id: &str, prefs: &Prefs) -> Result<()> {
    let sealed = crypto::seal(master, &serde_json::to_vec(prefs)?, &aad(vault_id))?;

    store.conn().execute(
        "INSERT INTO vault_prefs (vault_id, sealed) VALUES (?1, ?2)
         ON CONFLICT (vault_id) DO UPDATE SET sealed = excluded.sealed",
        params![vault_id, sealed],
    )?;

    Ok(())
}

/// Change the preferences of a vault that exists, in one read and one write.
pub fn update(store: &Store, master: &SecretKey, vault_id: &str, change: impl FnOnce(&mut Prefs)) -> Result<Prefs> {
    let exists: bool = store
        .conn()
        .query_row("SELECT EXISTS (SELECT 1 FROM vaults WHERE id = ?1)", params![vault_id], |row| row.get(0))?;
    if !exists {
        anyhow::bail!(crate::tr!("o cofre não existe"));
    }

    let mut prefs = read(store, master, vault_id);
    change(&mut prefs);
    write(store, master, vault_id, &prefs)?;

    Ok(prefs)
}

/// One more opening of the vault's page.
pub fn opened(store: &Store, master: &SecretKey, vault_id: &str) -> Result<Prefs> {
    update(store, master, vault_id, |prefs| {
        prefs.opened = prefs.opened.saturating_add(1);
        prefs.last_opened_at = Some(now_seconds());
    })
}

/// Whether this vault lets an agent's request through without asking, right now.
pub fn agent_lets_through(store: &Store, master: &SecretKey, vault_id: &str) -> bool {
    read(store, master, vault_id).agent.lets_through()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store, SecretKey, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("vault.db")).unwrap();
        let master = SecretKey::random();
        let vault = crate::vaults::create_vault(
            &store,
            &master,
            crate::vaults::VaultMeta { name: "Pessoal".into(), ..Default::default() },
        )
        .unwrap();

        (dir, store, master, vault.id)
    }

    #[test]
    fn a_vault_nobody_touched_asks() {
        let (_dir, store, master, id) = store();

        assert_eq!(read(&store, &master, &id), Prefs::default());
        assert!(!agent_lets_through(&store, &master, &id));
    }

    #[test]
    fn openings_add_up() {
        let (_dir, store, master, id) = store();

        opened(&store, &master, &id).unwrap();
        let prefs = opened(&store, &master, &id).unwrap();

        assert_eq!(prefs.opened, 2);
        assert!(prefs.last_opened_at.is_some());
    }

    #[test]
    fn a_policy_past_its_date_asks_again() {
        let (_dir, store, master, id) = store();

        update(&store, &master, &id, |prefs| prefs.agent = AgentPolicy::Until { at: now_seconds() - 1 }).unwrap();
        assert!(!agent_lets_through(&store, &master, &id));

        update(&store, &master, &id, |prefs| prefs.agent = AgentPolicy::Until { at: now_seconds() + 60 }).unwrap();
        assert!(agent_lets_through(&store, &master, &id));

        update(&store, &master, &id, |prefs| prefs.agent = AgentPolicy::Always).unwrap();
        assert!(agent_lets_through(&store, &master, &id));
    }

    #[test]
    fn a_row_moved_to_another_vault_does_not_open() {
        let (_dir, store, master, id) = store();
        let other = crate::vaults::create_vault(
            &store,
            &master,
            crate::vaults::VaultMeta { name: "Outro".into(), ..Default::default() },
        )
        .unwrap();

        update(&store, &master, &id, |prefs| prefs.agent = AgentPolicy::Always).unwrap();
        store
            .conn()
            .execute(
                "INSERT INTO vault_prefs (vault_id, sealed) SELECT ?2, sealed FROM vault_prefs WHERE vault_id = ?1",
                params![id, other.id],
            )
            .unwrap();

        assert!(!agent_lets_through(&store, &master, &other.id));
    }

    #[test]
    fn a_vault_that_does_not_exist_keeps_nothing() {
        let (_dir, store, master, _id) = store();

        assert!(opened(&store, &master, "nope").is_err());
    }

    #[test]
    fn the_row_goes_with_the_vault() {
        let (_dir, store, master, id) = store();
        update(&store, &master, &id, |prefs| prefs.favorite = true).unwrap();

        store.conn().execute("DELETE FROM vaults WHERE id = ?1", params![id]).unwrap();

        let left: i64 = store.conn().query_row("SELECT COUNT(*) FROM vault_prefs", [], |row| row.get(0)).unwrap();
        assert_eq!(left, 0);
    }
}
