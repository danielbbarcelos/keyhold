//! A friendlier name for this machine's keyhold, kept in `/etc/hosts`.
//!
//! `sudo keyhold set-host` writes one line, `127.0.0.1  keyhold.local  # keyhold`, and the running
//! keyhold reads that line back at start to know which name it may answer to. The file is the only
//! place the name lives: delete the line and the name stops working, with nothing else to clean up.
//!
//! Two limits, both about the same attack (DNS rebinding — see `http::only_this_machine`):
//!
//!  - **Only names that cannot exist on the internet.** `.local`, `.localhost`, `.internal`,
//!    `.test` and `.home.arpa` are reserved for exactly this. A public name in `/etc/hosts` works
//!    until a browser resolves it over DNS-over-HTTPS and skips the file, and then whoever owns that
//!    name can serve a page keyhold would accept as its own.
//!  - **Only lines carrying the mark, pointing at loopback.** keyhold never trusts a name just
//!    because something else put it in `/etc/hosts`.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};

pub const HOSTS_FILE: &str = "/etc/hosts";
pub const DEFAULT_NAME: &str = "keyhold.local";

/// The comment that marks a line as keyhold's.
const MARK: &str = "# keyhold";

/// Suffixes that are reserved and never resolve on the public internet.
const LOCAL_SUFFIXES: [&str; 5] = [".local", ".localhost", ".internal", ".test", ".home.arpa"];

/// Whether a name may be used: lowercase letters, digits and hyphens, in dot-separated labels,
/// ending in one of the reserved suffixes with at least one label before it.
pub fn check_name(name: &str) -> Result<()> {
    let suffix = LOCAL_SUFFIXES.iter().find(|suffix| name.ends_with(*suffix));

    let Some(suffix) = suffix else {
        bail!(
            "\"{name}\" não termina num domínio local. Use um nome terminado em {}: eles nunca \
             existem na internet, e um nome que existe pode ser servido por outra pessoa.",
            LOCAL_SUFFIXES.join(", ")
        );
    };

    let head = &name[..name.len() - suffix.len()];

    let label_ok = |label: &str| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };

    if head.is_empty() || name.len() > 253 || !head.split('.').all(label_ok) {
        bail!(
            "\"{name}\" não é um nome válido. Use letras minúsculas, números e hífen, por exemplo \
             {DEFAULT_NAME}."
        );
    }

    Ok(())
}

/// The names keyhold marked in a hosts file, pointing at loopback.
pub fn marked_names(contents: &str) -> Vec<String> {
    contents
        .lines()
        .filter_map(|line| {
            let (entry, comment) = line.split_once('#')?;

            if format!("#{comment}").trim() != MARK {
                return None;
            }

            let mut fields = entry.split_whitespace();
            let address = fields.next()?;

            if address != "127.0.0.1" && address != "::1" {
                return None;
            }

            Some(fields.filter(|name| check_name(name).is_ok()).map(str::to_string).collect::<Vec<_>>())
        })
        .flatten()
        .collect()
}

/// The hosts file with keyhold's line replaced by one for `name`. Every other line is kept as is.
pub fn with_name(contents: &str, name: &str) -> String {
    let mut out = without(contents);

    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }

    out.push_str(&format!("127.0.0.1\t{name}\t{MARK}\n"));
    out
}

/// The hosts file without keyhold's line.
pub fn without(contents: &str) -> String {
    let mut out = String::with_capacity(contents.len());

    for line in contents.split_inclusive('\n') {
        let is_ours = line
            .split_once('#')
            .map(|(_, comment)| format!("#{comment}").trim() == MARK)
            .unwrap_or(false);

        if !is_ours {
            out.push_str(line);
        }
    }

    out
}

/// The names this machine's `/etc/hosts` gives keyhold. Unreadable means none.
pub fn names_on_this_machine() -> Vec<String> {
    fs::read_to_string(HOSTS_FILE).map(|contents| marked_names(&contents)).unwrap_or_default()
}

/// Replace a file without ever leaving it half written: a temporary file beside it, then a rename.
///
/// `/etc/hosts` being empty or truncated for a moment would break every name lookup on the machine,
/// which is a lot to risk for a nicer URL.
pub fn replace_file(path: &Path, contents: &str) -> Result<()> {
    let dir = path.parent().context("o arquivo não tem pasta")?;
    let temp = dir.join(".hosts.keyhold.tmp");

    {
        let mut file = fs::File::create(&temp)
            .with_context(|| format!("não consegui escrever em {}", dir.display()))?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = fs::metadata(path).map(|meta| meta.permissions().mode()).unwrap_or(0o644);
        fs::set_permissions(&temp, fs::Permissions::from_mode(mode))?;
    }

    fs::rename(&temp, path).with_context(|| format!("não consegui substituir {}", path.display()))?;

    Ok(())
}

/// Where the person who ran `sudo` lives: their name, ids and home. `None` outside sudo.
#[cfg(unix)]
pub struct Invoker {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: std::path::PathBuf,
}

#[cfg(unix)]
pub fn invoker() -> Option<Invoker> {
    use std::ffi::{CStr, CString};

    let name = std::env::var("SUDO_USER").ok().filter(|name| !name.is_empty() && name != "root")?;
    let c_name = CString::new(name.clone()).ok()?;

    // SAFETY: getpwnam returns a pointer into static storage, read here before anything else can
    // call it again; this runs once, in a short-lived command, on one thread.
    let entry = unsafe { libc::getpwnam(c_name.as_ptr()) };

    if entry.is_null() {
        return None;
    }

    let (uid, gid, home) = unsafe {
        let entry = &*entry;
        (entry.pw_uid, entry.pw_gid, CStr::from_ptr(entry.pw_dir).to_string_lossy().into_owned())
    };

    Some(Invoker { name, uid, gid, home: home.into() })
}

/// What came of trying to make a certificate.
pub enum Certificate {
    Made { cert: std::path::PathBuf },
    NoMkcert,
    NoInvoker,
}

/// Make a certificate for `name` (and localhost, 127.0.0.1, ::1) with mkcert, **as the person who
/// ran sudo**.
///
/// It has to be them and not root: the CA their browsers trust is the one in their own home. Run as
/// root, mkcert would mint a second CA nobody's browser knows, and the page would still be "not
/// secure". The files land in their `~/.config/keyhold/tls`, owned by them, the key `0600`.
#[cfg(unix)]
pub fn make_certificate(name: &str) -> Result<Certificate> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let Some(who) = invoker() else {
        return Ok(Certificate::NoInvoker);
    };

    let mkcert = ["/usr/bin/mkcert", "/usr/local/bin/mkcert", "/opt/homebrew/bin/mkcert"]
        .into_iter()
        .map(std::path::PathBuf::from)
        .find(|path| path.exists());

    let Some(mkcert) = mkcert else {
        return Ok(Certificate::NoMkcert);
    };

    // ~/.config/keyhold/tls, created as needed and handed to them: a directory root made in their
    // home and left owned by root would be one they cannot write to later.
    let config = who.home.join(".config");
    let keyhold = config.join("keyhold");
    let dir = keyhold.join("tls");

    for (path, private) in [(&config, false), (&keyhold, true), (&dir, true)] {
        if !path.exists() {
            fs::create_dir(path).with_context(|| format!("não consegui criar {}", path.display()))?;
            chown(path, who.uid, who.gid)?;
        }

        if private {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }

    let as_them = |command: &mut Command| {
        command
            .uid(who.uid)
            .gid(who.gid)
            .env_clear()
            .env("HOME", &who.home)
            .env("USER", &who.name)
            .env("PATH", "/usr/local/bin:/usr/bin:/bin");
    };

    // The CA first, if they never made one. mkcert itself asks sudo for the system store; the
    // browsers' stores live in their home and need nothing.
    let caroot = who.home.join(".local/share/mkcert/rootCA.pem");

    if !caroot.exists() {
        let mut install = Command::new(&mkcert);
        as_them(&mut install);

        let status = install.arg("-install").status().context("não consegui rodar o mkcert")?;

        if !status.success() {
            bail!("o mkcert não conseguiu instalar a autoridade local (mkcert -install)");
        }
    }

    let cert = dir.join("cert.pem");
    let key = dir.join("key.pem");

    let mut make = Command::new(&mkcert);
    as_them(&mut make);

    let output = make
        .arg("-cert-file")
        .arg(&cert)
        .arg("-key-file")
        .arg(&key)
        .args([name, "localhost", "127.0.0.1", "::1"])
        .output()
        .context("não consegui rodar o mkcert")?;

    if !output.status.success() {
        bail!("o mkcert falhou: {}", String::from_utf8_lossy(&output.stderr).trim());
    }

    fs::set_permissions(&key, fs::Permissions::from_mode(0o600))?;

    Ok(Certificate::Made { cert })
}

/// Take the certificate away with the name. Missing files are not an error.
#[cfg(unix)]
pub fn remove_certificate() -> Result<()> {
    if let Some(who) = invoker() {
        let dir = who.home.join(".config/keyhold/tls");

        for file in ["cert.pem", "key.pem"] {
            match fs::remove_file(dir.join(file)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }

    Ok(())
}

#[cfg(unix)]
fn chown(path: &Path, uid: u32, gid: u32) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())?;

    // SAFETY: a valid NUL-terminated path and plain integers.
    if unsafe { libc::chown(c_path.as_ptr(), uid, gid) } != 0 {
        bail!("não consegui passar {} para o seu usuário", path.display());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_names_pass() {
        for good in ["keyhold.local", "cofre.localhost", "meu-cofre.internal", "a.b.test", "k.home.arpa"] {
            assert!(check_name(good).is_ok(), "{good} should pass");
        }

        for bad in [
            "keyhold.com",
            "local",
            ".local",
            "Keyhold.local",
            "-x.local",
            "x-.local",
            "a..b.local",
            "keyhold.local.evil.com",
            "key hold.local",
        ] {
            assert!(check_name(bad).is_err(), "{bad} should fail");
        }
    }

    #[test]
    fn reads_only_marked_loopback_lines() {
        let hosts = "127.0.0.1 localhost\n\
                     127.0.1.1 laptop\n\
                     127.0.0.1\tkeyhold.local\t# keyhold\n\
                     10.0.0.5 cofre.local # keyhold\n\
                     127.0.0.1 outro.local # comentário qualquer\n";

        assert_eq!(marked_names(hosts), vec!["keyhold.local".to_string()]);
    }

    #[test]
    fn writing_replaces_our_line_and_keeps_the_rest() {
        let hosts = "127.0.0.1 localhost\n127.0.0.1\tvelho.local\t# keyhold\n::1 ip6-localhost\n";

        let written = with_name(hosts, "keyhold.local");

        assert_eq!(
            written,
            "127.0.0.1 localhost\n::1 ip6-localhost\n127.0.0.1\tkeyhold.local\t# keyhold\n"
        );
        assert_eq!(marked_names(&written), vec!["keyhold.local".to_string()]);
        assert_eq!(without(&written), "127.0.0.1 localhost\n::1 ip6-localhost\n");
    }

    #[test]
    fn a_file_without_a_final_newline_still_gets_a_line_of_its_own() {
        assert_eq!(
            with_name("127.0.0.1 localhost", "keyhold.local"),
            "127.0.0.1 localhost\n127.0.0.1\tkeyhold.local\t# keyhold\n"
        );
    }

    #[test]
    fn replacing_keeps_the_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts");
        fs::write(&path, "127.0.0.1 localhost\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        replace_file(&path, &with_name("127.0.0.1 localhost\n", "keyhold.local")).unwrap();

        assert_eq!(marked_names(&fs::read_to_string(&path).unwrap()), vec!["keyhold.local"]);
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o644);
    }
}
