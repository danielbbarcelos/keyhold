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
//! once, for the places that know the lock just happened.
//!
//! **Nothing here is written down.** No file, no settings row. A13 says an AWS vault has no local
//! copy, and the caches are seconds long and in memory for that reason.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::config::Login;
use super::{
    AwsResult, Credentials, DeviceLogin, Parameter, ParameterMetadata, S3Client, SsmClient,
    SsoClient, SsoToken, Tag,
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

#[derive(Default)]
pub struct AwsState {
    inner: Mutex<Inner>,
    endpoints: Mutex<Endpoints>,
}

impl AwsState {
    /// Work with what is held, or `None` while the vault is locked.
    ///
    /// Whatever belonged to an earlier unlock is dropped before `f` sees it.
    pub fn with<T>(&self, lock: &Lock, f: impl FnOnce(&mut Inner) -> T) -> Option<T> {
        let generation = lock.generation()?;
        let mut inner = self.inner.lock().unwrap();

        if inner.generation != Some(generation) {
            *inner = Inner { generation: Some(generation), ..Inner::default() };
        }

        Some(f(&mut inner))
    }

    /// Drop every token, credential and cache now.
    pub fn forget(&self) {
        *self.inner.lock().unwrap() = Inner::default();
    }

    /// Drop everything if the vault locked since it was last looked at. Called from the routes the
    /// interface polls, so an auto-lock forgets the tokens within seconds rather than at the next
    /// AWS request.
    pub fn sweep(&self, lock: &Lock) {
        let current = lock.generation();
        let mut inner = self.inner.lock().unwrap();

        if current.is_none() || inner.generation != current {
            *inner = Inner { generation: current, ..Inner::default() };
        }
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

    pub fn s3(&self, region: &str, credentials: Credentials) -> AwsResult<S3Client> {
        let endpoint = self.endpoints.lock().unwrap().s3.clone();
        let client = S3Client::new(region, credentials)?;

        Ok(match endpoint {
            Some(endpoint) => client.with_endpoint(&endpoint),
            None => client,
        })
    }

    /// Send every request somewhere other than AWS. For tests, which run against a local fake.
    #[doc(hidden)]
    pub fn set_endpoints(&self, endpoints: Endpoints) {
        *self.endpoints.lock().unwrap() = endpoints;
    }

    /// Hand a connection credentials without an Identity Center login. For tests.
    #[doc(hidden)]
    pub fn insert_credentials(&self, lock: &Lock, connection_id: &str, credentials: Credentials) {
        self.with(lock, |inner| {
            inner.credentials.insert(connection_id.to_string(), credentials);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::SecretKey;
    use crate::lock::LockPolicy;

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

        lock.unlock_with(SecretKey::random());
        assert_eq!(state.with(&lock, |i| i.credentials.len()), Some(0));
    }
}
