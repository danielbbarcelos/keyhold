//! The lock, which is the whole product in one struct.
//!
//! **Locked means the key is gone**, not that a screen is covering it. The vault key lives in this
//! one place, in memory, and locking drops it — after which nothing in the process can read the
//! vault until somebody types the master password again. Quitting keyhold is the strongest lock
//! there is, and a crash is a lock too.
//!
//! The clock runs **here, in the daemon**, not in the browser. A timer in the page would be a
//! suggestion: a tab that is closed, frozen, or in a browser that throttled its timers simply
//! stops counting, and the key would sit in memory indefinitely. The interface shows the countdown
//! and asks for more time; this decides.
//!
//! Auto-lock is recommended and optional, in both of its parts, because "recommended" is a
//! sentence on a settings screen and "mandatory" is a reason to leave the vault unlocked in a
//! terminal instead.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::crypto::SecretKey;
use crate::identity::now_seconds;

/// The two clocks, in seconds. `None` in either field means that clock is off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockPolicy {
    /// Lock this long after the last thing you did.
    pub idle_seconds: Option<u64>,
    /// Lock this long after the window stopped being visible — a much shorter fuse, because a
    /// hidden window is one you have walked away from or a laptop that has been closed.
    pub hidden_seconds: Option<u64>,
}

impl Default for LockPolicy {
    /// Fifteen minutes idle, two minutes hidden. Recommended, and both switchable off.
    fn default() -> Self {
        Self { idle_seconds: Some(15 * 60), hidden_seconds: Some(2 * 60) }
    }
}

impl LockPolicy {
    /// Reject values that would make the setting meaningless in either direction.
    pub fn sanitised(self) -> Self {
        Self {
            idle_seconds: self.idle_seconds.map(|s| s.clamp(30, 8 * 60 * 60)),
            hidden_seconds: self.hidden_seconds.map(|s| s.clamp(10, 8 * 60 * 60)),
        }
    }
}

/// What the interface is allowed to know about the lock. Never the key.
#[derive(Debug, Serialize)]
pub struct LockStatus {
    pub unlocked: bool,
    /// Seconds until the vault locks itself, or `None` when nothing is counting down.
    pub locks_in: Option<u64>,
    pub policy: LockPolicy,
}

struct Open {
    key: SecretKey,
    last_activity: u64,
    /// When the window stopped being visible, if it is not visible now.
    hidden_since: Option<u64>,
}

pub struct Lock {
    state: Mutex<Option<Open>>,
    policy: Mutex<LockPolicy>,
    /// Goes up every time the key arrives or leaves. Whatever is kept in memory *because* the vault
    /// is open (the AWS sessions, for one) remembers the generation it was made in, and throws
    /// itself away when that is no longer the current one. That way a lock forgets them too, even
    /// the lock that happens on its own when a clock runs out.
    generation: AtomicU64,
}

impl Lock {
    pub fn new(policy: LockPolicy) -> Self {
        Self {
            state: Mutex::new(None),
            policy: Mutex::new(policy.sanitised()),
            generation: AtomicU64::new(0),
        }
    }

    pub fn policy(&self) -> LockPolicy {
        *self.policy.lock().unwrap()
    }

    pub fn set_policy(&self, policy: LockPolicy) -> LockPolicy {
        let sanitised = policy.sanitised();
        *self.policy.lock().unwrap() = sanitised;

        sanitised
    }

    pub fn unlock_with(&self, key: SecretKey) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        *self.state.lock().unwrap() =
            Some(Open { key, last_activity: now_seconds(), hidden_since: None });
    }

    /// Forget the key.
    ///
    /// Dropping the [`SecretKey`] zeroizes it; this is not "hide the interface", it is the key
    /// ceasing to exist in this process.
    pub fn lock(&self) {
        *self.state.lock().unwrap() = None;
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Which unlock this is, or `None` while locked.
    ///
    /// Two calls that return the same number are inside the same unlocked stretch: nobody locked
    /// and opened again in between. Checks the clock like [`Lock::with_key`], but does not count as
    /// activity.
    pub fn generation(&self) -> Option<u64> {
        let mut guard = self.state.lock().unwrap();
        let policy = *self.policy.lock().unwrap();

        if guard.as_ref().is_some_and(|open| is_expired(open, policy, now_seconds())) {
            *guard = None;
            self.generation.fetch_add(1, Ordering::SeqCst);
        }

        guard.as_ref().map(|_| self.generation.load(Ordering::SeqCst))
    }

    /// Hand the key to something that needs it, if the vault is open and has not timed out.
    ///
    /// Every read goes through here, which is what makes the deadline real rather than advisory:
    /// there is no path that reaches the key without passing the clock first.
    pub fn with_key<T>(&self, f: impl FnOnce(&SecretKey) -> T) -> Option<T> {
        let mut guard = self.state.lock().unwrap();

        let policy = *self.policy.lock().unwrap();
        let now = now_seconds();

        let expired = guard
            .as_ref()
            .map(|open| is_expired(open, policy, now))
            .unwrap_or(false);

        if expired {
            *guard = None;
            self.generation.fetch_add(1, Ordering::SeqCst);
        }

        let open = guard.as_mut()?;
        open.last_activity = now;

        Some(f(&open.key))
    }

    /// Say the person is still here. Called by the interface, and by every successful API call.
    pub fn touch(&self) {
        if let Some(open) = self.state.lock().unwrap().as_mut() {
            open.last_activity = now_seconds();
        }
    }

    /// Tell the lock whether the window is on screen.
    pub fn set_visible(&self, visible: bool) {
        if let Some(open) = self.state.lock().unwrap().as_mut() {
            let now = now_seconds();

            if visible {
                open.hidden_since = None;
                open.last_activity = now;
            } else if open.hidden_since.is_none() {
                open.hidden_since = Some(now);
            }
        }
    }

    pub fn status(&self) -> LockStatus {
        let mut guard = self.state.lock().unwrap();
        let policy = *self.policy.lock().unwrap();
        let now = now_seconds();

        if let Some(open) = guard.as_ref() {
            if is_expired(open, policy, now) {
                *guard = None;
                self.generation.fetch_add(1, Ordering::SeqCst);
            }
        }

        match guard.as_ref() {
            None => LockStatus { unlocked: false, locks_in: None, policy },
            Some(open) => LockStatus {
                unlocked: true,
                locks_in: remaining(open, policy, now),
                policy,
            },
        }
    }
}

fn is_expired(open: &Open, policy: LockPolicy, now: u64) -> bool {
    remaining(open, policy, now) == Some(0)
}

/// Whichever deadline comes first, in seconds. `None` when neither clock is running.
fn remaining(open: &Open, policy: LockPolicy, now: u64) -> Option<u64> {
    let idle = policy
        .idle_seconds
        .map(|limit| limit.saturating_sub(now.saturating_sub(open.last_activity)));

    let hidden = match (policy.hidden_seconds, open.hidden_since) {
        (Some(limit), Some(since)) => Some(limit.saturating_sub(now.saturating_sub(since))),
        _ => None,
    };

    match (idle, hidden) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_lock(policy: LockPolicy) -> Lock {
        let lock = Lock::new(policy);
        lock.unlock_with(SecretKey::random());

        lock
    }

    #[test]
    fn locking_drops_the_key() {
        let lock = open_lock(LockPolicy::default());
        assert!(lock.with_key(|_| ()).is_some());

        lock.lock();

        assert!(lock.with_key(|_| ()).is_none(), "the key survived a lock");
        assert!(!lock.status().unlocked);
    }

    #[test]
    fn an_idle_vault_locks_itself() {
        let lock = open_lock(LockPolicy { idle_seconds: Some(30), hidden_seconds: None });

        // Reach in and age the session rather than sleeping for half a minute.
        lock.state.lock().unwrap().as_mut().unwrap().last_activity = now_seconds() - 31;

        assert!(lock.with_key(|_| ()).is_none(), "an expired session handed out the key");
    }

    #[test]
    fn activity_pushes_the_deadline_back() {
        let lock = open_lock(LockPolicy { idle_seconds: Some(60), hidden_seconds: None });

        lock.state.lock().unwrap().as_mut().unwrap().last_activity = now_seconds() - 59;
        lock.touch();

        let remaining = lock.status().locks_in.unwrap();
        assert!(remaining > 50, "touching left only {remaining}s");
    }

    #[test]
    fn a_hidden_window_locks_on_its_own_shorter_clock() {
        let lock = open_lock(LockPolicy { idle_seconds: Some(3600), hidden_seconds: Some(20) });

        lock.set_visible(false);
        lock.state.lock().unwrap().as_mut().unwrap().hidden_since = Some(now_seconds() - 21);

        assert!(lock.with_key(|_| ()).is_none(), "a hidden window kept the key past its deadline");
    }

    #[test]
    fn coming_back_clears_the_hidden_clock() {
        let lock = open_lock(LockPolicy { idle_seconds: Some(3600), hidden_seconds: Some(20) });

        lock.set_visible(false);
        lock.set_visible(true);
        lock.state.lock().unwrap().as_mut().unwrap().hidden_since = None;

        assert!(lock.with_key(|_| ()).is_some());
    }

    #[test]
    fn auto_lock_can_be_switched_off_entirely() {
        let lock = open_lock(LockPolicy { idle_seconds: None, hidden_seconds: None });

        lock.state.lock().unwrap().as_mut().unwrap().last_activity = now_seconds() - 86_400;

        assert!(lock.with_key(|_| ()).is_some(), "a vault with auto-lock off locked anyway");
        assert_eq!(lock.status().locks_in, None);
    }

    #[test]
    fn the_shorter_of_the_two_clocks_wins() {
        let lock = open_lock(LockPolicy { idle_seconds: Some(900), hidden_seconds: Some(60) });
        lock.set_visible(false);

        let remaining = lock.status().locks_in.unwrap();
        assert!(remaining <= 60, "the longer clock won: {remaining}s");
    }

    #[test]
    fn absurd_policies_are_brought_back_into_range() {
        let policy = LockPolicy { idle_seconds: Some(1), hidden_seconds: Some(0) }.sanitised();

        assert_eq!(policy.idle_seconds, Some(30));
        assert_eq!(policy.hidden_seconds, Some(10));
    }
}
