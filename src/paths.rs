//! Where keyhold keeps things, and who is allowed to look.
//!
//! Three directories, following the XDG spec, each with a different lifetime:
//!
//!  - **data** (`~/.local/share/keyhold`) — the vault file. Survives everything; this is the one
//!    that matters, and the one a backup tool will happily copy to somebody else's disk. Which is
//!    why every byte of secret inside it is ciphertext.
//!  - **state** (`~/.local/state/keyhold`) — the agent socket and anything else tied to a running
//!    process. Meaningless once keyhold exits.
//!  - **config** (`~/.config/keyhold`) — preferences. Never a secret, never a key.
//!
//! Every one of them is created `0700` and re-checked on open. On a multi-user machine the default
//! umask is enough to leave a directory group-readable, and a vault file readable by `staff` is a
//! vault with a second owner nobody chose.

use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const APP: &str = "keyhold";

fn home() -> PathBuf {
    #[allow(deprecated)]
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Windows: `%LOCALAPPDATA%\\Keyhold\\<kind>` (the profile's own, not roamed to other machines,
/// and readable only by this user by the profile's default permissions).
/// The XDG variable still wins when set, which is how tests (and anyone who wants the vault
/// elsewhere) move it.
#[cfg(windows)]
fn windows_dir(kind: &str, var: &str) -> PathBuf {
    if let Some(value) = std::env::var_os(var).filter(|v| !v.is_empty()) {
        return PathBuf::from(value).join(APP);
    }

    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("AppData").join("Local"));

    base.join("Keyhold").join(kind)
}

#[cfg_attr(windows, allow(dead_code))]
fn xdg(var: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(var) {
        Some(value) if !value.is_empty() => PathBuf::from(value).join(APP),
        _ => home().join(fallback).join(APP),
    }
}

/// The vault lives here.
pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    return windows_dir("data", "XDG_DATA_HOME");

    #[cfg(not(windows))]
    return xdg("XDG_DATA_HOME", ".local/share");
}

/// Sockets and other things that die with the process.
pub fn state_dir() -> PathBuf {
    #[cfg(windows)]
    return windows_dir("state", "XDG_STATE_HOME");

    #[cfg(not(windows))]
    return xdg("XDG_STATE_HOME", ".local/state");
}

/// Preferences. Nothing here is ever secret.
pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    return windows_dir("config", "XDG_CONFIG_HOME");

    #[cfg(not(windows))]
    return xdg("XDG_CONFIG_HOME", ".config");
}

/// The certificate `sudo keyhold set-host` made with mkcert, and its key. Present means HTTPS.
pub fn tls_files() -> (PathBuf, PathBuf) {
    let dir = config_dir().join("tls");

    (dir.join("cert.pem"), dir.join("key.pem"))
}

pub fn vault_file() -> PathBuf {
    data_dir().join("vault.db")
}

/// Write a file only its owner can read, creating it with that mode rather than fixing it after.
///
/// The order matters: creating a file and then tightening its permissions leaves a window in which
/// the contents are world-readable, and key material is exactly what would be read in it.
pub fn write_private_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(&parent.to_path_buf())?;
    }

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    options.mode(0o600);

    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()?;

    Ok(())
}

/// Create a directory if missing and make sure it is ours alone.
///
/// The permission is re-applied on every call rather than only at creation: a directory that
/// already existed — restored from a backup, copied by hand, created by an older version — is
/// exactly the one whose mode nobody checked.
pub fn ensure_private_dir(path: &PathBuf) -> io::Result<()> {
    fs::create_dir_all(path)?;

    #[cfg(unix)]
    {
        let mut perms = fs::metadata(path)?.permissions();
        if perms.mode() & 0o077 != 0 {
            perms.set_mode(0o700);
            fs::set_permissions(path, perms)?;
        }
    }

    Ok(())
}
