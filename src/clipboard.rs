//! Putting a secret on the desktop clipboard, and taking it back off.
//!
//! **Why this exists.** When somebody asks an assistant for a password, what they almost always
//! want is the password *in their hands* — to paste into a login form — not the password *in the
//! conversation*. Handing it back through the agent puts it in the model's context and out of this
//! machine, permanently, for a use that never needed it there. Copying puts it exactly where it was
//! wanted and tells the agent only that it worked.
//!
//! **What it is not.** The clipboard is a shared surface: another program running as this user can
//! read it, and some desktops keep a history that nothing here can reach. So the value is cleared
//! after a short window, and the interface says plainly that the operating system may have kept a
//! copy. It is better than the alternative, not perfect.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{bail, Result};

/// How long a copied secret stays on the clipboard.
pub const CLEAR_AFTER_SECONDS: u64 = 45;

/// The tools that own the clipboard on a Linux desktop, in the order worth trying.
///
/// Wayland first because a session that has both is a Wayland session running an X11 bridge, and
/// writing through the bridge lands in the wrong place often enough to be worth avoiding.
fn writer() -> Option<(&'static str, Vec<&'static str>)> {
    let has = |program: &str| {
        Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {program}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    };

    if std::env::var_os("WAYLAND_DISPLAY").is_some() && has("wl-copy") {
        return Some(("wl-copy", vec![]));
    }

    if has("xclip") {
        return Some(("xclip", vec!["-selection", "clipboard"]));
    }

    if has("xsel") {
        return Some(("xsel", vec!["--clipboard", "--input"]));
    }

    if has("wl-copy") {
        return Some(("wl-copy", vec![]));
    }

    None
}

fn write(value: &str) -> Result<()> {
    let Some((program, args)) = writer() else {
        bail!("nenhuma ferramenta de área de transferência encontrada (instale wl-clipboard ou xclip)");
    };

    let mut child = Command::new(program)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    // Through stdin, never as an argument: `/proc/*/cmdline` is readable by every process of this
    // user, and a password on a command line is a password anybody can watch go by.
    child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("não foi possível escrever na área de transferência"))?
        .write_all(value.as_bytes())?;

    // xclip holds the selection by staying alive; waiting for it would block until the clipboard is
    // replaced. Waiting on the others is equally unnecessary — the write already happened.
    drop(child.stdin.take());

    Ok(())
}

/// Copy `value`, and clear it after [`CLEAR_AFTER_SECONDS`].
///
/// The clear overwrites unconditionally rather than checking what is there first: reading the
/// clipboard back to compare would mean holding the secret in a second place to compare it against.
/// The cost is wiping something the person copied in the meantime, which is why the window is
/// short and the interface says how long it is.
pub fn copy_for_a_while(value: &str) -> Result<u64> {
    write(value)?;

    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(CLEAR_AFTER_SECONDS)).await;

        // A single space rather than an empty string: some clipboard tools treat empty as "leave it
        // alone", and a space is not the secret, which is the whole requirement.
        let _ = write(" ");
    });

    Ok(CLEAR_AFTER_SECONDS)
}

/// Whether copying is possible at all here.
pub fn available() -> bool {
    writer().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_knows_whether_it_can_copy() {
        // Not an assertion about this machine — just that asking never panics and never blocks.
        let _ = available();
    }

    #[test]
    fn the_window_is_short_enough_to_matter() {
        assert!(CLEAR_AFTER_SECONDS <= 60);
    }
}
