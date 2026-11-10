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
    /// mkcert is there but its local authority is not installed, and nobody said yes to it.
    NeedsAuthority,
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
pub fn make_certificate(name: &str, install_ca: bool) -> Result<Certificate> {
    use std::process::Command;

    let Some(who) = invoker() else {
        return Ok(Certificate::NoInvoker);
    };

    let Some(mkcert) = find_mkcert() else {
        return Ok(Certificate::NoMkcert);
    };

    // Everything in their home is done *as them*, by child processes running with their uid.
    // Root touching paths in a directory the user controls follows whatever symlink the user put
    // there: a `~/.config/keyhold -> /etc` turned a chmod into breaking the machine, and a race
    // between mkdir and chown turned it into handing them any file. As them, the worst a symlink
    // can do is point at their own files.
    let dir = who.home.join(".config/keyhold/tls");

    run_as(&who, Command::new("/bin/mkdir").arg("-p").arg(&dir))
        .context("não consegui criar a pasta do certificado")?;
    run_as(&who, Command::new("/bin/chmod").arg("700").arg(dir.parent().unwrap_or(&dir)).arg(&dir))
        .context("não consegui proteger a pasta do certificado")?;

    // The CA is the one thing never done silently. `mkcert -install` puts an authority every
    // browser here trusts *for every site*, with its private key in their home: worth having, but
    // a decision for them, asked by the caller.
    let caroot = who.home.join(".local/share/mkcert/rootCA.pem");

    if !caroot.exists() {
        if !install_ca {
            return Ok(Certificate::NeedsAuthority);
        }

        let mut install = Command::new(&mkcert);
        run_as(&who, install.arg("-install")).context("o mkcert não conseguiu instalar a autoridade local")?;
    }

    let cert = dir.join("cert.pem");
    let key = dir.join("key.pem");

    let mut make = Command::new(&mkcert);
    make.arg("-cert-file")
        .arg(&cert)
        .arg("-key-file")
        .arg(&key)
        .args([name, "localhost", "127.0.0.1", "::1"]);
    run_as(&who, &mut make).context("o mkcert falhou")?;

    run_as(&who, Command::new("/bin/chmod").arg("600").arg(&key))
        .context("não consegui proteger a chave do certificado")?;

    Ok(Certificate::Made { cert })
}

#[cfg(unix)]
fn find_mkcert() -> Option<std::path::PathBuf> {
    ["/usr/bin/mkcert", "/usr/local/bin/mkcert", "/opt/homebrew/bin/mkcert"]
        .into_iter()
        .map(std::path::PathBuf::from)
        .find(|path| path.exists())
}

/// Run a command with the invoking user's identity and a clean environment, and fail on a
/// non-zero exit with what it printed.
#[cfg(unix)]
fn run_as(who: &Invoker, command: &mut std::process::Command) -> Result<()> {
    use std::os::unix::process::CommandExt;

    let output = command
        .uid(who.uid)
        .gid(who.gid)
        .env_clear()
        .env("HOME", &who.home)
        .env("USER", &who.name)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .output()?;

    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }

    Ok(())
}

/// Take the certificate away. Done as the invoking user, for the same reason as making it; with
/// no invoker (plain root) there is nothing of theirs to touch, and the caller is told.
#[cfg(unix)]
pub fn remove_certificate() -> Result<bool> {
    let Some(who) = invoker() else {
        return Ok(false);
    };

    let dir = who.home.join(".config/keyhold/tls");
    run_as(&who, std::process::Command::new("/bin/rm").arg("-f").arg(dir.join("cert.pem")).arg(dir.join("key.pem")))
        .context("não consegui apagar o certificado")?;

    Ok(true)
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
