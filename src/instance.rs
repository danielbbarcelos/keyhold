//! One vault, one process.
//!
//! Two keyholds over one file is not a cosmetic problem. They both open the same SQLite database,
//! both hold their own copy of the schema's assumptions, and both race for the agent socket — whose
//! path is fixed, so the last one to bind wins and every earlier one becomes a process an assistant
//! can no longer reach but that is still holding an unlocked key in memory. The failure is quiet,
//! which is the worst kind.
//!
//! So a run claims the machine before it opens anything, by creating a marker file no other process
//! can create at the same time. What the second run does about it is a question for the person:
//! taking over means the vault that is currently open locks and forgets its key, and nothing should
//! do that to somebody's session because they double-clicked an icon.
//!
//! The marker is not a lock in the kernel's sense — it is a note with a PID in it, and a note can
//! outlive its writer. Every read checks that the process named is alive *and* is a keyhold, so a
//! crash leaves at worst a file that the next run deletes.

use std::fs;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

/// How long a replaced instance gets to close the vault properly before it is taken down.
const GRACE: Duration = Duration::from_secs(5);

/// What one running keyhold leaves behind so the next one can find it.
///
/// Nothing here is secret, and nothing here may become secret: the file is a note about a process,
/// not about a vault. The session token in particular stays in memory where it belongs — writing it
/// here would trade the whole "the token never touches the disk" property for the convenience of
/// reopening a tab.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Marker {
    pub pid: i32,
    /// The loopback port being served. Zero between claiming the machine and binding one.
    #[serde(default)]
    pub port: u16,
}

pub fn marker_path() -> PathBuf {
    paths::state_dir().join("instance.json")
}

/// The outcome of asking for the machine.
pub enum Claim {
    /// Nobody else was here. Hold on to this for as long as the process serves.
    Held(Held),
    /// Somebody else is, and this is who.
    Busy(Marker),
}

/// The claim itself. Dropping it releases the machine.
pub struct Held {
    pid: i32,
}

impl Held {
    /// Record the port, once the operating system has handed one over.
    pub fn note_port(&self, port: u16) {
        let _ = write_marker(&Marker { pid: self.pid, port });
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        // Only ever remove a marker that is still ours. Between this process being replaced and
        // this line running, the file on disk may already belong to whoever replaced it — and
        // deleting *that* would hand the machine to a third instance.
        if matches!(read_marker(), Some(marker) if marker.pid == self.pid) {
            let _ = fs::remove_file(marker_path());
        }
    }
}

/// Ask for the machine.
pub fn claim() -> Result<Claim> {
    paths::ensure_private_dir(&paths::state_dir())?;

    let pid = std::process::id() as i32;
    let marker = Marker { pid, port: 0 };

    // Two attempts, and no more: the first may lose to a marker left by a process that is already
    // gone, and clearing that is the only reason to go round again. A loop here would spin against
    // an instance starting at the same moment instead of saying so.
    for attempt in 0..2 {
        match create_marker(&marker) {
            Ok(()) => return Ok(Claim::Held(Held { pid })),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                match read_marker() {
                    Some(other) if alive(&other) => return Ok(Claim::Busy(other)),
                    // Either nobody is behind it or the file is unreadable. Both are rubbish left
                    // by a run that did not get to clean up.
                    _ if attempt == 0 => {
                        let _ = fs::remove_file(marker_path());
                    }
                    _ => bail!("could not claim {}", marker_path().display()),
                }
            }
            Err(error) => {
                return Err(error).with_context(|| format!("could not write {}", marker_path().display()))
            }
        }
    }

    bail!("could not claim {}", marker_path().display())
}

/// Whether the process a marker names is really there, and really a keyhold.
///
/// PIDs are reused. Signalling one that a stale marker happens to name would take down whatever
/// unrelated program inherited the number, so liveness alone is not enough — on Linux the name is
/// checked too. Where `/proc` says nothing, the signal test stands on its own; the marker is in a
/// directory only this user can write, which is the reason that is tolerable.
pub fn alive(marker: &Marker) -> bool {
    if marker.pid <= 1 || marker.pid == std::process::id() as i32 {
        return false;
    }

    if let Ok(name) = fs::read_to_string(format!("/proc/{}/comm", marker.pid)) {
        return name.trim() == env!("CARGO_PKG_NAME");
    }

    signal(marker.pid, 0)
}

/// What to do about a Keyhold that is already open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Leave it running and bring its window to the front. The safe answer, and the default.
    Present,
    /// Close it and start over. The vault it holds open locks and its key is forgotten.
    Replace,
    /// Neither. This run ends here.
    Nothing,
}

/// Put the question to whoever is at the keyboard.
///
/// Continuing is offered first and is what a bare Enter picks, because it is the answer that loses
/// nothing: double-clicking an icon is not a decision to close an open vault, and until this
/// existed that was the only thing the dialog could offer.
///
/// Two surfaces, each in its own language, because they are read by different people in different
/// places: the terminal answers to the same audience as `--help` and the startup banner, and the
/// desktop dialog belongs to the same app as the window it is about.
pub fn ask(other: &Marker) -> Choice {
    // An older build cannot be brought forward — see `present`. Offering it and then refusing is a
    // dead end, so the question is asked without it.
    let can_present = same_build(other.pid);

    if std::io::stdin().is_terminal() {
        return ask_in_terminal(other, can_present);
    }

    ask_on_desktop(other, can_present)
}

fn ask_in_terminal(other: &Marker, can_present: bool) -> Choice {
    println!();
    println!("  Keyhold is already running (pid {}).", other.pid);
    println!();

    if can_present {
        println!("    c  continue in it — opens the vault that is already unlocked");
    } else {
        println!("  It is running a different build of this program, which cannot be brought");
        println!("  forward. Starting over is the only way in.");
        println!();
    }

    println!("    r  replace it — that vault locks and the key it holds is forgotten");
    println!("    n  do nothing");
    println!();
    print!("  Which? [{}] ", if can_present { "C/r/n" } else { "r/N" });
    let _ = std::io::stdout().flush();

    let mut answer = String::new();

    if std::io::stdin().read_line(&mut answer).is_err() {
        return Choice::Nothing;
    }

    match answer.trim().to_lowercase().as_str() {
        "" if can_present => Choice::Present,
        "c" | "continue" if can_present => Choice::Present,
        "r" | "replace" => Choice::Replace,
        _ => Choice::Nothing,
    }
}

fn ask_on_desktop(other: &Marker, can_present: bool) -> Choice {
    const START_OVER: &str = "Encerrar e começar de novo";

    let text = if can_present {
        format!(
            "O Keyhold já está aberto (PID {}).\n\nAbrir e continuar traz para a frente o cofre \
             que já está destrancado — nada se perde.\n\nComeçar de novo encerra a instância \
             atual: o cofre tranca e a chave que ele guarda é esquecida.",
            other.pid
        )
    } else {
        format!(
            "O Keyhold já está aberto (PID {}), rodando uma versão diferente do programa — ela \
             não sabe se trazer para a frente.\n\nComeçar de novo encerra a instância atual: o \
             cofre tranca e a chave que ele guarda é esquecida.",
            other.pid
        )
    };

    // zenity has two buttons plus extras. An extra button exits non-zero and prints its own label,
    // which is how a three-way question fits into a yes/no dialog.
    let mut zenity = Command::new("zenity");
    zenity
        .args(["--question", "--title", "Keyhold", "--no-markup"])
        .arg("--text")
        .arg(&text);

    if can_present {
        zenity
            .args(["--ok-label", "Abrir e continuar", "--cancel-label", "Cancelar"])
            .arg("--extra-button")
            .arg(START_OVER);
    } else {
        zenity.args(["--ok-label", START_OVER, "--cancel-label", "Cancelar"]);
    }

    if let Ok(output) = zenity.output() {
        if output.status.success() {
            return if can_present { Choice::Present } else { Choice::Replace };
        }

        return if String::from_utf8_lossy(&output.stdout).trim() == START_OVER {
            Choice::Replace
        } else {
            Choice::Nothing
        };
    }

    // kdialog counts its buttons for us: yes, no, cancel.
    let kdialog = if can_present {
        Command::new("kdialog")
            .args(["--title", "Keyhold", "--yesnocancel"])
            .arg(&text)
            .args(["--yes-label", "Abrir e continuar", "--no-label", START_OVER])
            .status()
    } else {
        Command::new("kdialog")
            .args(["--title", "Keyhold", "--yesno"])
            .arg(&text)
            .args(["--yes-label", START_OVER, "--no-label", "Cancelar"])
            .status()
    };

    if let Ok(status) = kdialog {
        return match (status.code(), can_present) {
            (Some(0), true) => Choice::Present,
            (Some(0), false) => Choice::Replace,
            (Some(1), true) => Choice::Replace,
            _ => Choice::Nothing,
        };
    }

    // No way to ask means no way to consent to the destructive half. Bringing the open vault
    // forward is not destructive, so that is what happens — and it is said out loud.
    if can_present {
        announce(&format!(
            "O Keyhold já estava aberto (PID {}). Trouxe a janela para o seu navegador.",
            other.pid
        ));

        return Choice::Present;
    }

    announce(&format!(
        "O Keyhold já está aberto (PID {}), numa versão diferente, e não há como perguntar o que \
         fazer. Nada foi alterado.",
        other.pid
    ));

    Choice::Nothing
}

/// Ask the running instance to show itself: it opens its own URL in the browser.
///
/// `USR1` rather than a second socket. The signal carries nothing, the kernel already restricts it
/// to this user, and the answer — a session token in a URL — is put where it always goes, into the
/// browser by the process that owns it. Nothing about the running vault comes back here.
///
/// The version check is not caution for its own sake. `USR1` **kills** a process that has not
/// installed a handler for it, so sending one to a build that predates this feature would close the
/// vault somebody just asked to keep open — the exact opposite of what they answered. A build that
/// cannot be asked is one to say so about.
pub fn present(other: &Marker) -> Result<()> {
    if !same_build(other.pid) {
        bail!(
            "o Keyhold aberto no PID {} é de outra versão do programa e não sabe se trazer para a \
             frente. Encerre e comece de novo.",
            other.pid
        );
    }

    if !signal(other.pid, libc::SIGUSR1) {
        bail!("não foi possível falar com o Keyhold no PID {}", other.pid);
    }

    Ok(())
}

/// Whether another process is running the very same file this one is.
///
/// By device and inode rather than by path: reinstalling writes a new file at the same name, and
/// the process that was already running keeps the old one open. Two different builds, one path.
fn same_build(pid: i32) -> bool {
    use std::os::unix::fs::MetadataExt;

    let ours = match fs::metadata("/proc/self/exe") {
        Ok(meta) => meta,
        Err(_) => return false,
    };

    match fs::metadata(format!("/proc/{pid}/exe")) {
        Ok(theirs) => theirs.dev() == ours.dev() && theirs.ino() == ours.ino(),
        Err(_) => false,
    }
}

/// Say something where the desktop can show it.
///
/// The browser tab is the thing people miss: a window that opens behind the one they are looking at
/// is a window that did not open, as far as they can tell. A notification lands on top of whatever
/// has focus, which is the one place this can be sure of.
pub fn announce(text: &str) {
    let _ = Command::new("notify-send")
        .args(["--app-name", "Keyhold", "--icon", "keyhold", "Keyhold"])
        .arg(text)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Close the other instance and wait for it to actually be gone.
///
/// `TERM` first, because the running keyhold handles it: it stops serving, wipes the key on its way
/// out and removes its own marker. `KILL` only after the grace period, for a process too wedged to
/// do any of that — and then the marker is cleared here, since nothing ran to clear it.
pub fn replace(other: &Marker) -> Result<()> {
    if !signal(other.pid, libc::SIGTERM) && alive(other) {
        bail!("could not signal the keyhold at pid {}", other.pid);
    }

    let deadline = Instant::now() + GRACE;

    while Instant::now() < deadline {
        if !alive(other) {
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    if alive(other) {
        signal(other.pid, libc::SIGKILL);
        std::thread::sleep(Duration::from_millis(300));
    }

    if alive(other) {
        bail!("the keyhold at pid {} would not close", other.pid);
    }

    // It may have exited before it could tidy up, and the marker it left would otherwise look like
    // a live instance to the claim that comes next.
    if matches!(read_marker(), Some(stale) if stale.pid == other.pid) {
        let _ = fs::remove_file(marker_path());
    }

    Ok(())
}

fn signal(pid: i32, number: i32) -> bool {
    // SAFETY: `kill` takes two integers and touches nothing this process owns. A pid that no longer
    // exists is an error return, not undefined behaviour.
    unsafe { libc::kill(pid, number) == 0 }
}

fn create_marker(marker: &Marker) -> std::io::Result<()> {
    let body = serde_json::to_vec(marker).expect("a marker is two numbers");

    paths::write_private_file(&marker_path(), &body)
}

fn write_marker(marker: &Marker) -> std::io::Result<()> {
    let _ = fs::remove_file(marker_path());

    create_marker(marker)
}

pub fn read_marker() -> Option<Marker> {
    let body = fs::read(marker_path()).ok()?;

    serde_json::from_slice(&body).ok()
}

/// Who is running right now, if anybody. `None` covers "nobody" and "a marker nobody is behind".
pub fn running() -> Option<Marker> {
    read_marker().filter(alive)
}

/// Every keyhold process on this machine that belongs to this person, and what each is doing.
///
/// The interface shows this so that "why is my assistant talking to a vault I closed?" has an answer
/// on screen rather than in `ps`. It is also the place a leftover from an older build — one that
/// started before any of this existed and so claimed nothing — becomes visible.
///
/// Read from `/proc` and filtered by owner: another user's processes are none of this window's
/// business, and would not be signallable anyway.
pub fn processes() -> Vec<Process> {
    let me = std::process::id() as i32;
    let uid = our_uid();
    let marker = read_marker();
    let uptime = uptime_seconds().unwrap_or(0.0);
    let ticks = clock_ticks() as f64;

    let mut found: Vec<Process> = fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();

            name.to_str()?.parse::<i32>().ok()
        })
        .filter(|pid| owner_of(*pid) == Some(uid))
        .filter(|pid| is_keyhold(*pid))
        .map(|pid| Process {
            pid,
            role: role_of(pid),
            running_for: started_ticks(pid)
                .map(|started| (uptime - started as f64 / ticks).max(0.0) as u64)
                .unwrap_or(0),
            is_this_one: pid == me,
            holds_the_vault: matches!(&marker, Some(one) if one.pid == pid),
        })
        .collect();

    // This one first, then by age: the oldest leftover is the one worth looking at.
    found.sort_by_key(|one| (!one.is_this_one, std::cmp::Reverse(one.running_for)));
    found
}

/// What a keyhold process is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// Serving the vault and this window.
    Interface,
    /// `keyhold mcp` — the translator an assistant talks through. Holds no key.
    Agent,
    /// `keyhold run` — a command borrowing one secret for its own lifetime.
    Command,
    /// Something this build does not recognise, most likely from an older one.
    Other,
}

#[derive(Clone, Debug, Serialize)]
pub struct Process {
    pub pid: i32,
    pub role: Role,
    /// Seconds since it started.
    pub running_for: u64,
    /// The process answering this very request.
    pub is_this_one: bool,
    /// The one the marker names — the process that claimed the vault file.
    pub holds_the_vault: bool,
}

/// Close one of the others.
///
/// The pid arrives from a client, so it is never signalled on the client's word: a fresh scan has to
/// agree that it is a keyhold owned by this person, and that it is not this process. Between that
/// scan and the signal a pid could in principle be recycled — the window is microseconds wide and
/// the alternative is not signalling anything at all.
///
/// `TERM` only. A keyhold handles it: it stops serving, wipes the key and releases its claim. If it
/// will not go, that is worth saying rather than escalating behind somebody's back.
pub fn close(pid: i32) -> Result<()> {
    let target = processes()
        .into_iter()
        .find(|one| one.pid == pid)
        .ok_or_else(|| anyhow::anyhow!("não há um Keyhold com o PID {pid} nesta máquina"))?;

    if target.is_this_one {
        bail!("esta é a instância que está desenhando esta janela — use Trancar agora, ou feche-a");
    }

    if !signal(pid, libc::SIGTERM) {
        bail!("não foi possível pedir ao PID {pid} que encerrasse");
    }

    let deadline = Instant::now() + Duration::from_secs(3);

    while Instant::now() < deadline {
        if !signal(pid, 0) {
            return Ok(());
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    bail!("o PID {pid} não respondeu ao pedido de encerrar")
}

fn is_keyhold(pid: i32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/comm"))
        .map(|name| name.trim() == env!("CARGO_PKG_NAME"))
        .unwrap_or(false)
}

fn our_uid() -> u32 {
    // SAFETY: `getuid` reads a field of this process and cannot fail.
    unsafe { libc::getuid() }
}

fn owner_of(pid: i32) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;

    fs::metadata(format!("/proc/{pid}")).ok().map(|meta| meta.uid())
}

fn clock_ticks() -> u64 {
    // SAFETY: `sysconf` reads a constant of the running kernel.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };

    if ticks > 0 {
        ticks as u64
    } else {
        100
    }
}

fn uptime_seconds() -> Option<f64> {
    fs::read_to_string("/proc/uptime").ok()?.split_whitespace().next()?.parse().ok()
}

/// Field 22 of `/proc/<pid>/stat`, in clock ticks since boot.
///
/// The parse starts after the last `)` for the same reason the agent's does: field 2 is the command
/// name, and a command name may contain spaces and parentheses.
fn started_ticks(pid: i32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = stat.rfind(')')?;

    stat[after + 1..].split_whitespace().nth(19)?.parse().ok()
}

fn role_of(pid: i32) -> Role {
    let raw = match fs::read(format!("/proc/{pid}/cmdline")) {
        Ok(raw) => raw,
        Err(_) => return Role::Other,
    };

    let subcommand = raw
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .skip(1)
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .find(|part| !part.starts_with('-'));

    match subcommand.as_deref() {
        None => Role::Interface,
        Some("mcp") => Role::Agent,
        Some("run") => Role::Command,
        Some(_) => Role::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These tests point `XDG_STATE_HOME` at a temporary directory, which is process-wide. Holding
    /// this while one runs keeps two of them from claiming each other's marker.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A pid that certainly belongs to nobody: a process that has already been reaped.
    fn dead_pid() -> i32 {
        let mut child = Command::new("true").spawn().expect("spawn /bin/true");
        let pid = child.id() as i32;

        child.wait().expect("reap");

        pid
    }

    fn in_a_temp_state_dir() -> (std::sync::MutexGuard<'static, ()>, tempfile::TempDir) {
        let guard = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();

        std::env::set_var("XDG_STATE_HOME", dir.path());

        (guard, dir)
    }

    #[test]
    fn a_reaped_process_is_not_a_running_instance() {
        assert!(!alive(&Marker { pid: dead_pid(), port: 0 }));
    }

    #[test]
    fn this_process_never_counts_as_the_other_instance() {
        // Without this, a marker whose pid happens to match ours — a recycled number, a file copied
        // between machines — would make keyhold refuse to start and offer to kill itself.
        let mine = std::process::id() as i32;

        assert!(!alive(&Marker { pid: mine, port: 0 }));
        assert!(!alive(&Marker { pid: 1, port: 0 }));
    }

    #[test]
    fn a_claim_writes_a_marker_and_releases_it_when_dropped() {
        let (_serial, _dir) = in_a_temp_state_dir();

        let held = match claim().unwrap() {
            Claim::Held(held) => held,
            Claim::Busy(other) => panic!("nothing should hold an empty state dir, found {other:?}"),
        };

        let marker = read_marker().expect("a marker on disk");
        assert_eq!(marker.pid, std::process::id() as i32);
        assert_eq!(marker.port, 0);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode = fs::metadata(marker_path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "the marker is nobody else's business");
        }

        held.note_port(7717);
        assert_eq!(read_marker().unwrap().port, 7717);

        drop(held);
        assert!(!marker_path().exists(), "releasing the claim removes the marker");
    }

    #[test]
    fn a_marker_left_by_a_crash_does_not_hold_the_machine() {
        let (_serial, _dir) = in_a_temp_state_dir();

        paths::ensure_private_dir(&paths::state_dir()).unwrap();
        create_marker(&Marker { pid: dead_pid(), port: 5274 }).unwrap();

        assert!(running().is_none(), "nobody is behind that marker");

        match claim().unwrap() {
            Claim::Held(_) => {
                assert_eq!(read_marker().unwrap().pid, std::process::id() as i32);
            }
            Claim::Busy(other) => panic!("a dead pid should not hold the machine, found {other:?}"),
        }
    }

    #[test]
    fn only_this_very_binary_can_be_asked_to_show_itself() {
        // `USR1` kills a process that installed no handler for it, so "same program" is not a
        // nicety here: a build that predates the handler must never be signalled.
        assert!(same_build(std::process::id() as i32), "this process is certainly the same build");
        assert!(!same_build(1), "init is not a Keyhold");
        assert!(!same_build(dead_pid()), "a reaped process has no /proc/<pid>/exe");
    }

    #[test]
    fn a_claim_does_not_delete_a_marker_that_is_no_longer_ours() {
        let (_serial, _dir) = in_a_temp_state_dir();

        let held = match claim().unwrap() {
            Claim::Held(held) => held,
            Claim::Busy(other) => panic!("unexpected {other:?}"),
        };

        // What a takeover looks like from this side: somebody else's marker is now on disk. Dropping
        // ours must leave it alone, or the machine ends up unclaimed while an instance is serving.
        write_marker(&Marker { pid: dead_pid(), port: 1 }).unwrap();

        drop(held);

        assert!(marker_path().exists(), "somebody else's claim survives our drop");
    }
}
