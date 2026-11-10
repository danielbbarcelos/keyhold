//! What Keyhold holds in memory for AWS while the vault is open, and only while it is.
//!
//! Identity Center tokens, logins waiting for somebody to approve them in a browser, the role
//! credentials each connection is using, and two short caches (a vault's listing, a parameter's
//! tags) that keep one screen from asking AWS the same question twice in a second.
//!
//! **Tied to the lock.** Every access goes through [`AwsState::with`], which asks the lock which
//! unlock this is ([`Lock::generation`]) and drops everything held when the answer changed. Locking,
//! the auto-lock running out, and unlocking again all change it, so a token obtained before a lock
//! is never used after one: the person logs in again (A2). [`AwsState::forget`] does the same at
//! once, for the places that know the lock just happened, and [`AwsState::sweep`] is run every few
//! seconds by a background task (`http::start_aws_sweeper`) so an auto-lock with nobody clicking
//! still drops the tokens within seconds.
//!
//! **A request that outlives a lock does not store what it fetched.** A login poll, a token
//! refresh or a role-credentials call takes a second or two on the network, and the vault can lock
//! in that gap. Those paths capture the generation when they start ([`AwsState::generation`]) and
//! store through [`AwsState::with_generation`], which refuses when the generation moved: the
//! credentials obtained for the old unlock are dropped instead of being handed to the new one.
//!
//! **Stale is gone, not skipped.** Cache entries past their few seconds are removed on every
//! access and on every sweep, so a listing of somebody's account does not sit in memory for the
//! rest of the session just because nobody opened that vault again.
//!
//! **Nothing here is written down.** No file, no settings row. A13 says an AWS vault has no local
//! copy, and the caches are seconds long and in memory for that reason.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::config::Login;
use super::{
    now_unix, AwsError, AwsResult, Credentials, DeviceLogin, Parameter, ParameterMetadata,
    S3Client, SsmClient, SsoClient, SsoToken, Tag,
};
use crate::lock::Lock;

/// How long a vault's listing is reused: long enough for the folders and the records of one screen
/// to come from one read, short enough that somebody else's change shows up on the next click.
pub const SNAPSHOT_SECONDS: u64 = 10;

/// How long a parameter's tags are reused.
pub const TAGS_SECONDS: u64 = 60;

/// A login somebody started and has not approved yet.
pub struct Pending {
    pub login: Login,
    pub device: DeviceLogin,
    /// Whether the login was already saved before this attempt (a renewal), or is new.
    pub known: bool,
}

/// One read of a whole vault: every parameter GetParametersByPath returned, and the metadata
/// DescribeParameters had for those (and only those) names.
pub struct Snapshot {
    pub at: u64,
    pub parameters: Vec<Parameter>,
    pub metadata: HashMap<String, ParameterMetadata>,
    /// Why `metadata` is empty, when DescribeParameters failed. A listing still draws (titled by
    /// name), but nothing that needs a key may be decided from it: see H1 in the threat model.
    pub metadata_error: Option<AwsError>,
}

impl Snapshot {
    fn is_fresh(&self, now: u64) -> bool {
        now.saturating_sub(self.at) < SNAPSHOT_SECONDS
    }
}

/// Where requests go. Empty means AWS itself; tests point these at a local fake.
#[derive(Clone, Debug, Default)]
pub struct Endpoints {
    pub oidc: Option<String>,
    pub portal: Option<String>,
    pub ssm: Option<String>,
    pub s3: Option<String>,
}

#[derive(Default)]
pub struct Inner {
    generation: Option<u64>,
    /// Identity Center tokens, by login id.
    pub tokens: HashMap<String, SsoToken>,
    /// Device logins in progress, by login id.
    pub pending: HashMap<String, Pending>,
    /// Role credentials, by connection id.
    pub credentials: HashMap<String, Credentials>,
    /// Listings, by vault id.
    pub snapshots: HashMap<String, Arc<Snapshot>>,
    /// Tags, by `<vault id>|<parameter name>`, with when they were read.
    pub tags: HashMap<String, (u64, Vec<Tag>)>,
}

impl Inner {
    /// Drop cache entries past their lifetime and device logins past their deadline.
    fn prune(&mut self, now: u64) {
        self.snapshots.retain(|_, snapshot| snapshot.is_fresh(now));
        self.tags.retain(|_, (at, _)| now.saturating_sub(*at) < TAGS_SECONDS);
        self.pending.retain(|_, pending| pending.device.expires_at > now);
    }

    /// Start over for another unlock (or none).
    fn reset(&mut self, generation: Option<u64>) {
        *self = Inner { generation, ..Inner::default() };
    }
}

#[derive(Default)]
pub struct AwsState {
    inner: Mutex<Inner>,
    endpoints: Mutex<Endpoints>,
}

impl AwsState {
    /// Work with what is held, or `None` while the vault is locked.
    ///
    /// Whatever belonged to an earlier unlock is dropped before `f` sees it. The generation is read
    /// while this state's own mutex is held, so a lock that lands between the two cannot leave
    /// `f` looking at the previous unlock's tokens under the new generation's name.
    pub fn with<T>(&self, lock: &Lock, f: impl FnOnce(&mut Inner) -> T) -> Option<T> {
        let mut inner = self.inner.lock().unwrap();
        let Some(generation) = lock.generation() else {
            inner.reset(None);
            return None;
        };

        if inner.generation != Some(generation) {
            inner.reset(Some(generation));
        }
        inner.prune(now_unix());

        Some(f(&mut inner))
    }

    /// Which unlock this is, for a request to remember when it starts and hand back to
    /// [`AwsState::with_generation`] when it wants to store what it fetched. `None` while locked.
    pub fn generation(&self, lock: &Lock) -> Option<u64> {
        self.with(lock, |inner| inner.generation).flatten()
    }

    /// [`AwsState::with`], but only if the vault is still in the unlock `generation` names.
    ///
    /// For storing tokens and credentials a request fetched over the network: if the vault locked
    /// (and perhaps unlocked again) while it waited, they belong to a session that is over, and
    /// `None` comes back without `f` having run.
    pub fn with_generation<T>(
        &self,
        lock: &Lock,
        generation: u64,
        f: impl FnOnce(&mut Inner) -> T,
    ) -> Option<T> {
        let mut inner = self.inner.lock().unwrap();
        let current = lock.generation();

        if current != Some(generation) {
            if inner.generation != current {
                inner.reset(current);
            }
            return None;
        }

        if inner.generation != current {
            inner.reset(current);
        }
        inner.prune(now_unix());

        Some(f(&mut inner))
    }

    /// Drop every token, credential and cache now.
    pub fn forget(&self) {
        *self.inner.lock().unwrap() = Inner::default();
    }

    /// Drop everything if the vault locked since it was last looked at, and whatever cache entry
    /// has gone stale either way. Run every few seconds by `http::start_aws_sweeper`, and from the
    /// routes the interface polls.
    pub fn sweep(&self, lock: &Lock) {
        let mut inner = self.inner.lock().unwrap();
        let current = lock.generation();

        if current.is_none() || inner.generation != current {
            inner.reset(current);
        }
        inner.prune(now_unix());
    }

    /// Forget a vault's listing and tags after something in it changed.
    pub fn invalidate(&self, vault_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.snapshots.remove(vault_id);
        let prefix = format!("{vault_id}|");
        inner.tags.retain(|key, _| !key.starts_with(&prefix));
    }

    pub fn sso(&self, region: &str) -> AwsResult<SsoClient> {
        let endpoints = self.endpoints.lock().unwrap().clone();
        let client = SsoClient::new(region)?;

        Ok(match (endpoints.oidc, endpoints.portal) {
            (Some(oidc), Some(portal)) => client.with_endpoints(&oidc, &portal),
            _ => client,
        })
    }

    pub fn ssm(&self, region: &str, credentials: Credentials) -> AwsResult<SsmClient> {
        let endpoint = self.endpoints.lock().unwrap().ssm.clone();
        let client = SsmClient::new(region, credentials)?;

        Ok(match endpoint {
            Some(endpoint) => client.with_endpoint(&endpoint),
            None => client,
        })
    }

    /// An S3 client that refuses any bucket not owned by `account_id` (the connection's account).
    pub fn s3(&self, region: &str, credentials: Credentials, account_id: &str) -> AwsResult<S3Client> {
        let endpoint = self.endpoints.lock().unwrap().s3.clone();
        let client = S3Client::new(region, credentials)?.with_expected_owner(account_id);

        Ok(match endpoint {
            Some(endpoint) => client.with_endpoint(&endpoint),
            None => client,
        })
    }

    /// Send every request somewhere other than AWS. Only the tests can, which run against a fake.
    #[cfg(test)]
    pub(crate) fn set_endpoints(&self, endpoints: Endpoints) {
        *self.endpoints.lock().unwrap() = endpoints;
    }

    /// Hand a connection credentials without an Identity Center login. For tests.
    #[cfg(test)]
    pub(crate) fn insert_credentials(&self, lock: &Lock, connection_id: &str, credentials: Credentials) {
        self.with(lock, |inner| {
            inner.credentials.insert(connection_id.to_string(), credentials);
        });
    }

    /// How many things are held, of any kind, without going through the lock. For tests.
    #[cfg(test)]
    pub(crate) fn held(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.tokens.len()
            + inner.pending.len()
            + inner.credentials.len()
            + inner.snapshots.len()
            + inner.tags.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::SecretKey;
    use crate::lock::LockPolicy;

    fn open() -> Lock {
        let lock = Lock::new(LockPolicy { idle_seconds: None, hidden_seconds: None });
        lock.unlock_with(SecretKey::random());
        lock
    }

    #[test]
    fn what_was_held_is_gone_after_a_lock_and_a_new_unlock() {
        let lock = Lock::new(LockPolicy { idle_seconds: None, hidden_seconds: None });
        let state = AwsState::default();

        assert!(state.with(&lock, |_| ()).is_none(), "nothing while locked");

        lock.unlock_with(SecretKey::random());
        state.insert_credentials(&lock, "C", Credentials::new("AKIA", "secret", None));
        assert_eq!(state.with(&lock, |i| i.credentials.len()), Some(1));

        lock.lock();
        assert!(state.with(&lock, |i| i.credentials.len()).is_none());
        assert_eq!(state.held(), 0, "a look while locked already drops it");

        lock.unlock_with(SecretKey::random());
        assert_eq!(state.with(&lock, |i| i.credentials.len()), Some(0));
    }

    #[test]
    fn a_request_that_started_before_a_lock_cannot_store_what_it_fetched() {
        let lock = open();
        let state = AwsState::default();
        let started = state.generation(&lock).unwrap();

        // The vault locks and opens again while the request waits on the network.
        lock.lock();
        lock.unlock_with(SecretKey::random());

        let stored = state.with_generation(&lock, started, |inner| {
            inner.credentials.insert("C".into(), Credentials::new("AKIA", "secret", None));
        });
        assert!(stored.is_none());
        assert_eq!(state.with(&lock, |i| i.credentials.len()), Some(0));

        let now = state.generation(&lock).unwrap();
        assert!(state.with_generation(&lock, now, |_| ()).is_some());
    }

    #[test]
    fn stale_cache_entries_are_removed_not_just_skipped() {
        let lock = open();
        let state = AwsState::default();
        let old = now_unix() - SNAPSHOT_SECONDS - TAGS_SECONDS;

        state.with(&lock, |inner| {
            inner.snapshots.insert(
                "V".into(),
                Arc::new(Snapshot {
                    at: old,
                    parameters: vec![],
                    metadata: HashMap::new(),
                    metadata_error: None,
                }),
            );
            inner.tags.insert("V|/a".into(), (old, vec![]));
            inner.tags.insert("V|/b".into(), (now_unix(), vec![]));
        });

        state.sweep(&lock);
        assert_eq!(state.with(&lock, |i| (i.snapshots.len(), i.tags.len())), Some((0, 1)));
    }

    #[test]
    fn a_sweep_after_a_lock_drops_everything_without_any_request() {
        let lock = open();
        let state = AwsState::default();
        state.insert_credentials(&lock, "C", Credentials::new("AKIA", "secret", None));
        assert_eq!(state.held(), 1);

        lock.lock();
        state.sweep(&lock);
        assert_eq!(state.held(), 0);
    }
}
