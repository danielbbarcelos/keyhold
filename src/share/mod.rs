//! Sharing one record with somebody outside: a link to a page in the vault's own bucket (AWS
//! vaults) or a password-protected ZIP (local vaults). See `docs/AWS.md` and the threat model.

pub mod crypto;
pub mod zip;
