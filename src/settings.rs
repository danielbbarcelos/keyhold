//! Preferences that survive a restart.
//!
//! Only things that are not secret live here, in plain rows: how long before the vault locks
//! itself, and what the generator should produce by default. Anything that would help somebody
//! open the vault belongs in `identity`, sealed.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use crate::db::Store;
use crate::lock::LockPolicy;

const LOCK_POLICY: &str = "lock_policy";
const RETENTION_DAYS: &str = "trash_retention_days";

/// How long a deleted record waits in the trash before it stops existing.
///
/// Thirty days is the usual answer and the default here. Zero means "wait forever" — a real choice
/// on a machine with no storage quota, where nothing is ever lost to a deadline somebody forgot
/// about, at the cost of a trash that grows.
pub const DEFAULT_RETENTION_DAYS: i64 = 30;

pub fn retention_days(store: &Store) -> i64 {
    read(store, RETENTION_DAYS)
        .ok()
        .flatten()
        .and_then(|raw| raw.parse().ok())
        .map(|days: i64| days.clamp(0, 3650))
        .unwrap_or(DEFAULT_RETENTION_DAYS)
}

pub fn set_retention_days(store: &Store, days: i64) -> Result<i64> {
    let clamped = days.clamp(0, 3650);
    write(store, RETENTION_DAYS, &clamped.to_string())?;

    Ok(clamped)
}

pub fn lock_policy(vault: &Store) -> LockPolicy {
    read(vault, LOCK_POLICY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<LockPolicy>(&raw).ok())
        .map(LockPolicy::sanitised)
        .unwrap_or_default()
}

pub fn set_lock_policy(vault: &Store, policy: LockPolicy) -> Result<LockPolicy> {
    let sanitised = policy.sanitised();
    write(vault, LOCK_POLICY, &serde_json::to_string(&sanitised)?)?;

    Ok(sanitised)
}

const AGENT_MAY_REVEAL: &str = "agent_may_reveal";

/// Whether an agent may receive the *value* of a secret.
///
/// **Off by default, and that is the whole point of this setting.** Once a value reaches a model it
/// is in that model's context and out of this machine, and nothing here decides what it writes
/// afterwards — not a tool description, not a warning appended to the value. The only thing that
/// reliably stops a password appearing in a chat transcript is not handing it over.
///
/// With this off, an agent still has everything it needs for the common cases: it can find records,
/// read their structure, run a command with the secret in the child's environment, and put a value
/// on the clipboard for the person to paste. What it cannot do is read one out loud.
pub fn agent_may_reveal(vault: &Store) -> bool {
    read(vault, AGENT_MAY_REVEAL).ok().flatten().map(|raw| raw == "true").unwrap_or(false)
}

pub fn set_agent_may_reveal(vault: &Store, on: bool) -> Result<bool> {
    write(vault, AGENT_MAY_REVEAL, if on { "true" } else { "false" })?;

    Ok(on)
}

const AGENT_SURFACE: &str = "agent_surface";

/// Whether the agent socket should be opened when keyhold starts.
///
/// Remembered rather than asked again every time. The socket's path never changes, so an agent
/// configured once keeps working across restarts — and a switch that silently resets on every
/// launch would be a switch nobody trusts.
pub fn agent_surface(vault: &Store) -> bool {
    read(vault, AGENT_SURFACE).ok().flatten().map(|raw| raw == "true").unwrap_or(false)
}

pub fn set_agent_surface(vault: &Store, on: bool) -> Result<bool> {
    write(vault, AGENT_SURFACE, if on { "true" } else { "false" })?;

    Ok(on)
}

const LOG_READS: &str = "log_reads";

/// Whether revealing and copying a secret is written to the trail.
///
/// On by default, and switchable off. The trade is real in both directions: with it on, the vault
/// keeps a record of your own use of it — which is what makes "what did the agent read?" answerable
/// (F7), and also what makes it a log of you. With it off, writes and security events are still
/// recorded; only the reads stop.
pub fn log_reads(vault: &Store) -> bool {
    read(vault, LOG_READS)
        .ok()
        .flatten()
        .map(|raw| raw != "false")
        .unwrap_or(true)
}

pub fn set_log_reads(vault: &Store, on: bool) -> Result<bool> {
    write(vault, LOG_READS, if on { "true" } else { "false" })?;

    Ok(on)
}

const APPEARANCE: &str = "appearance";

/// How the interface looks: which theme and which accent colour.
///
/// Not secret and not about the vault's contents, which is why it sits here in a plain row. It is
/// read before unlocking too, so the unlock screen opens in the same theme as everything else.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Appearance {
    /// `grafite` (dark), `claro` (light) or `sistema` (follow the desktop).
    pub theme: String,
    /// `ambar`, `oceano`, `floresta` or `coral`.
    pub accent: String,
    /// `inteira` (the content fills the window) or `contida` (centred, at most 1280px wide).
    /// Rows written before this existed have none, and read as `inteira`.
    #[serde(default = "full_width")]
    pub width: String,
}

fn full_width() -> String {
    "inteira".into()
}

impl Default for Appearance {
    fn default() -> Self {
        Self { theme: "grafite".into(), accent: "ambar".into(), width: full_width() }
    }
}

impl Appearance {
    /// Anything unknown falls back to the default rather than failing: an old or hand-edited row
    /// must never leave the interface without a theme.
    pub fn sanitised(self) -> Self {
        let fallback = Self::default();
        let theme = match self.theme.as_str() {
            "grafite" | "claro" | "sistema" => self.theme,
            _ => fallback.theme,
        };
        let accent = match self.accent.as_str() {
            "ambar" | "oceano" | "floresta" | "coral" => self.accent,
            _ => fallback.accent,
        };
        let width = match self.width.as_str() {
            "inteira" | "contida" => self.width,
            _ => fallback.width,
        };

        Self { theme, accent, width }
    }
}

pub fn appearance(vault: &Store) -> Appearance {
    read(vault, APPEARANCE)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Appearance>(&raw).ok())
        .map(Appearance::sanitised)
        .unwrap_or_default()
}

pub fn set_appearance(vault: &Store, appearance: Appearance) -> Result<Appearance> {
    let sanitised = appearance.sanitised();
    write(vault, APPEARANCE, &serde_json::to_string(&sanitised)?)?;

    Ok(sanitised)
}

fn read(vault: &Store, key: &str) -> Result<Option<String>> {
    Ok(vault
        .conn()
        .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |row| row.get(0))
        .optional()?)
}

fn write(vault: &Store, key: &str, value: &str) -> Result<()> {
    vault.conn().execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let vault = Store::open(&dir.path().join("vault.db")).unwrap();

        (dir, vault)
    }

    #[test]
    fn a_fresh_vault_gets_the_recommended_policy() {
        let (_dir, vault) = scratch();
        assert_eq!(lock_policy(&vault), LockPolicy::default());
    }

    #[test]
    fn a_saved_policy_comes_back() {
        let (_dir, vault) = scratch();
        let chosen = LockPolicy { idle_seconds: Some(300), hidden_seconds: None };

        set_lock_policy(&vault, chosen).unwrap();

        assert_eq!(lock_policy(&vault), chosen);
    }

    #[test]
    fn auto_lock_switched_off_stays_off_across_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");

        {
            let vault = Store::open(&path).unwrap();
            set_lock_policy(&vault, LockPolicy { idle_seconds: None, hidden_seconds: None }).unwrap();
        }

        let vault = Store::open(&path).unwrap();
        assert_eq!(lock_policy(&vault), LockPolicy { idle_seconds: None, hidden_seconds: None });
    }

    #[test]
    fn retention_defaults_and_can_be_switched_off() {
        let (_dir, store) = scratch();

        assert_eq!(retention_days(&store), DEFAULT_RETENTION_DAYS);

        set_retention_days(&store, 7).unwrap();
        assert_eq!(retention_days(&store), 7);

        set_retention_days(&store, 0).unwrap();
        assert_eq!(retention_days(&store), 0);
    }

    #[test]
    fn an_absurd_retention_is_brought_into_range() {
        let (_dir, store) = scratch();

        assert_eq!(set_retention_days(&store, -5).unwrap(), 0);
        assert_eq!(set_retention_days(&store, 99_999).unwrap(), 3650);
    }

    #[test]
    fn an_agent_may_not_read_values_until_that_is_switched_on() {
        let (_dir, store) = scratch();

        assert!(!agent_may_reveal(&store), "revealing to an agent must be off by default");

        set_agent_may_reveal(&store, true).unwrap();
        assert!(agent_may_reveal(&store));
    }

    #[test]
    fn the_agent_surface_is_off_until_asked_for_and_then_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");

        {
            let store = Store::open(&path).unwrap();
            assert!(!agent_surface(&store), "the surface should start closed");

            set_agent_surface(&store, true).unwrap();
        }

        // A restart finds it the way it was left.
        let store = Store::open(&path).unwrap();
        assert!(agent_surface(&store));
    }

    #[test]
    fn appearance_defaults_and_ignores_what_it_does_not_know() {
        let (_dir, store) = scratch();

        assert_eq!(appearance(&store), Appearance::default());

        let saved = set_appearance(
            &store,
            Appearance { theme: "claro".into(), accent: "rosa-choque".into(), width: "contida".into() },
        )
        .unwrap();

        assert_eq!(
            saved,
            Appearance { theme: "claro".into(), accent: "ambar".into(), width: "contida".into() }
        );
        assert_eq!(appearance(&store), saved);
    }

    #[test]
    fn an_appearance_saved_before_the_width_existed_fills_the_window() {
        let old: Appearance = serde_json::from_str(r#"{"theme":"claro","accent":"oceano"}"#).unwrap();

        assert_eq!(old.width, "inteira");
    }

    #[test]
    fn reads_are_logged_until_somebody_says_otherwise() {
        let (_dir, store) = scratch();

        assert!(log_reads(&store));

        set_log_reads(&store, false).unwrap();
        assert!(!log_reads(&store));

        set_log_reads(&store, true).unwrap();
        assert!(log_reads(&store));
    }

    #[test]
    fn a_damaged_setting_falls_back_to_the_recommendation() {
        let (_dir, vault) = scratch();
        write(&vault, LOCK_POLICY, "not json at all").unwrap();

        assert_eq!(lock_policy(&vault), LockPolicy::default());
    }
}
