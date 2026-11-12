//! `keyhold-launch`: what the Windows Start menu, the desktop shortcut and `keyhold://` links open.
//!
//! `keyhold.exe` is a console program (it is also the command line and the agent's MCP server), so
//! opening it from a shortcut would leave a console window on screen whose closing ends Keyhold.
//! This one has no window at all: it starts `keyhold.exe --present` beside it, hidden, with its
//! output in a log only this user can read (the log carries the session address), and exits.
//! Keyhold then opens itself in the browser, or brings the running one forward.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::fs::{self, OpenOptions};
use std::process::{Command, Stdio};

fn main() {
    let program = if cfg!(windows) { "keyhold.exe" } else { "keyhold" };
    let Some(keyhold) = std::env::current_exe().ok().and_then(|me| me.parent().map(|dir| dir.join(program))) else {
        return;
    };

    let state = keyhold::paths::state_dir();
    let _ = keyhold::paths::ensure_private_dir(&state);
    let log = state.join("app.log");
    if log.exists() {
        let _ = fs::rename(&log, state.join("app.log.1"));
    }
    let output = OpenOptions::new().create(true).write(true).truncate(true).open(&log);

    let mut command = Command::new(keyhold);
    command.arg("--present").stdin(Stdio::null());
    match output.and_then(|file| Ok((file.try_clone()?, file))) {
        Ok((out, err)) => {
            command.stdout(out).stderr(err);
        }
        Err(_) => {
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console for the child either.
        command.creation_flags(0x0800_0000);
    }

    let _ = command.spawn();
}
