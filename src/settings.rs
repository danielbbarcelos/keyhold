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
