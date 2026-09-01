//! keyhold — a strictly local password vault.
//!
//! The whole product in three sentences: the vault file on disk is ciphertext and nothing else;
//! the key exists only in this process's memory, only while unlocked; and nothing keyhold does
//! reaches the network. Every design decision in this codebase is downstream of those three.
//!
//! What this binary does when run with no arguments: opens the vault file, picks a free loopback
//! port, generates a session token, serves the interface, and points a browser at it.

use keyhold::{db, http, instance, paths, session};

use std::net::{Ipv4Addr, SocketAddr};
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use db::Store;
use http::AppState;
use keyhold::lock::Lock;
use session::SessionToken;

#[derive(Parser)]
#[command(name = "keyhold", version, about = "A strictly local password vault.")]
struct Cli {
    /// Port to bind. Defaults to whatever the operating system has free.
    #[arg(long)]
    port: Option<u16>,

    /// Do not open a browser.
    #[arg(long)]
    no_browser: bool,

    /// Close a keyhold that is already running instead of asking first.
    #[arg(long)]
    replace: bool,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print where keyhold keeps its files.
    Where,
    /// Speak the Model Context Protocol on stdin/stdout, for a local agent.
    ///
    /// Meant to be launched by the agent, not by hand:
    ///     claude mcp add keyhold --scope user -- keyhold mcp
    Mcp,

    /// Run a command with a secret from the vault in its environment.
    ///
    ///     keyhold run "Banco de dados" Senha -- psql "$KEYHOLD_SECRET"
    ///
    /// The value goes to the command and nowhere else — not to a terminal, not to whatever is
    /// reading this terminal. keyhold asks you to approve it first.
    Run {
        /// The record's title, as it appears in the vault.
        record: String,
        /// The field's label, for example `Senha`.
        field: String,
        /// Environment variable to put it in. Defaults to KEYHOLD_SECRET.
        #[arg(long, short)]
        var: Option<String>,
        /// The command, after `--`.
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Cmd::Where) => {
            println!("vault   {}", paths::vault_file().display());
            println!("state   {}", paths::state_dir().display());
            println!("config  {}", paths::config_dir().display());
            Ok(())
        }
        Some(Cmd::Mcp) => keyhold::mcp::serve(),
        Some(Cmd::Run { record, field, var, command }) => {
            let code = keyhold::mcp::run_command(&record, &field, var.as_deref(), &command)?;

            // The command's own exit code, so this composes in a shell like anything else.
            std::process::exit(code);
        }
        None => serve(cli).await,
    }
}

async fn serve(cli: Cli) -> Result<()> {
    // Before the vault file is even opened: two processes over one database, both racing for an
    // agent socket whose path is fixed, is a mess nobody sees until an assistant is talking to a
    // vault the window is no longer showing.
    let held = match instance::claim()? {
        instance::Claim::Held(held) => held,
        instance::Claim::Busy(other) => {
            if !cli.replace && !instance::ask_to_replace(&other) {
                println!();
                println!("  Keyhold is already running (pid {}). Nothing was changed.", other.pid);
                println!();

                return Ok(());
            }

            instance::replace(&other)?;

            match instance::claim()? {
                instance::Claim::Held(held) => held,
                instance::Claim::Busy(other) => {
                    anyhow::bail!("another Keyhold (pid {}) started while this one was taking over", other.pid)
                }
            }
        }
    };

    let store = Store::open(&paths::vault_file())?;

    // Read the lock policy before anything can change it: the vault starts locked, and the clock
    // it will run on is the one the person chose last time, not the default.
    let policy = keyhold::settings::lock_policy(&store);
    let agent_wanted = keyhold::settings::agent_surface(&store);
    let token = SessionToken::generate();

    // Port 0 asks the OS for a free one. Binding to `127.0.0.1` rather than `0.0.0.0` is not a
    // default worth trusting to configuration: there is no setting that makes keyhold listen on a
    // network interface, because there is no version of this product that should.
    let requested = SocketAddr::from((Ipv4Addr::LOCALHOST, cli.port.unwrap_or(0)));
    let listener = tokio::net::TcpListener::bind(requested)
        .await
        .with_context(|| format!("could not bind {requested}"))?;
    let bound = listener.local_addr()?;

    held.note_port(bound.port());

    let url = format!("http://127.0.0.1:{}/#token={}", bound.port(), token.expose());

    println!();
    println!("  Keyhold {}", env!("CARGO_PKG_VERSION"));
    println!("  {url}");
    println!("  vault   {}", paths::vault_file().display());
    println!();
    println!("  Ctrl+C to stop. Closing Keyhold forgets the key.");

    if agent_wanted {
        println!();
        println!("  agent   {}", keyhold::agent::socket_path().display());
    }
    println!();

    if !cli.no_browser {
        open_browser(&url);
    }

    let state = Arc::new(AppState {
        token,
        agent: Default::default(),
        store: std::sync::Arc::new(std::sync::Mutex::new(store)),
        lock: Lock::new(policy),
        failures: std::sync::Mutex::new(0),
    });

    // Reopen the agent socket if it was left on.
    //
    // Deliberately before anything is unlocked: the socket answers "o cofre está trancado" to an
    // agent that connects too early, which is a far better thing for one to hear than "keyhold is
    // not running". Every operation behind it checks the lock, so an open socket over a locked
    // vault gives nothing away.
    if agent_wanted {
        if let Err(error) = keyhold::agent::listen(state.clone()) {
            eprintln!("  could not open the agent socket: {error}");
        }
    }

    axum::serve(listener, http::router(state))
        .with_graceful_shutdown(shutdown())
        .await?;

    // Explicit, and the last thing that happens: the machine is only released once nothing is being
    // served on it. Reading as `drop(held)` also keeps a future edit from moving the claim into a
    // scope that ends early.
    drop(held);

    Ok(())
}

/// Ctrl+C, or a `TERM` from another keyhold taking over.
///
/// `TERM` is handled rather than left to the kernel for one reason: the default action kills the
/// process where it stands, which skips wiping the key and skips releasing the claim. A takeover is
/// a routine thing here, so it should end this instance the same way Ctrl+C does.
async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(_) => return drop(tokio::signal::ctrl_c().await),
        };

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Hand the URL to the desktop.
///
/// The token is in the fragment, which is why this is safe to pass as an argument: the fragment
/// never reaches a server, and `xdg-open` is not a server. It is still visible in this process's
/// own `/proc/self/cmdline` for as long as the helper runs, which is the reason the token is
/// worthless the moment keyhold exits.
fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };

    let _ = Command::new(opener)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
