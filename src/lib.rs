//! keyhold — a strictly local password vault.
//!
//! Three promises hold this codebase together, and everything else is downstream of them:
//!
//!  1. **The vault file is ciphertext.** A backup, a synced folder or another program reading your
//!     files finds noise.
//!  2. **The key exists only while unlocked**, in this process's memory, with no copy anywhere —
//!     including for you. Locking is forgetting.
//!  3. **Nothing leaves the machine.** No server, no account, no sync, no telemetry, and no asset
//!     fetched at runtime.
//!
//! The library exists so the test suite can exercise the same code the binary runs, rather than a
//! second implementation that agrees with it on a good day.

pub mod crypto;
pub mod db;
pub mod encoding;
pub mod generator;
pub mod health;
pub mod http;
pub mod identity;
pub mod lock;
pub mod paths;
pub mod records;
pub mod session;
pub mod settings;
pub mod totp;
pub mod vaults;
