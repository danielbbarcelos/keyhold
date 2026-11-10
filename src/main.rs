//! keyhold — a strictly local password vault.
//!
//! The whole product in three sentences: the vault file on disk is ciphertext and nothing else;
//! the key exists only in this process's memory, only while unlocked; and nothing keyhold does
//! reaches the network. Every design decision in this codebase is downstream of those three.
//!
//! What this binary does when run with no arguments: opens the vault file, picks a free loopback
//! port, generates a session token, serves the interface, and points a browser at it.

use keyhold::{db, hosts, http, instance, paths, session};

use std::net::{Ipv4Addr, SocketAddr};
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
    /// Port to bind. Defaults to 7717, or whatever is free if 7717 is taken.
    #[arg(long)]
    port: Option<u16>,

    /// Do not open a browser.
    #[arg(long)]
    no_browser: bool,

    /// Close a Keyhold that is already running instead of asking first.
    #[arg(long, conflicts_with = "present")]
    replace: bool,

    /// Bring a Keyhold that is already running to the front instead of asking first.
    ///
    /// With nothing running, this starts one — it is "open my vault", however that has to happen.
    #[arg(long)]
    present: bool,

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

    /// Give this machine's Keyhold a name, in /etc/hosts. Needs sudo.
    ///
    ///     sudo ~/.local/bin/keyhold set-host                  # keyhold.local
    ///     sudo ~/.local/bin/keyhold set-host cofre.local
    ///     sudo ~/.local/bin/keyhold set-host --remove
    ///
    /// The full path because sudo does not search ~/.local/bin.
    ///
    /// Only names that never exist on the internet: .local, .localhost, .internal, .test,
    /// .home.arpa. Restart Keyhold afterwards; it reads the name when it starts.
    #[command(verbatim_doc_comment)]
    SetHost {
        /// The name. Defaults to keyhold.local.
        name: Option<String>,
        /// Take the name out of /etc/hosts instead.
        #[arg(long, conflicts_with = "name")]
        remove: bool,
        /// Install mkcert's local authority without asking, when it is missing. It is trusted by
        /// this machine's browsers for every site, so by default Keyhold asks first.
        #[arg(long)]
        install_ca: bool,
    },
}

/// The port a Keyhold started without `--port` tries first, so a bookmark keeps working.
const DEFAULT_PORT: u16 = 7717;

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
        Some(Cmd::SetHost { name, remove, install_ca }) => set_host(name, remove, install_ca),
        None => serve(cli).await,
    }
}

async fn serve(cli: Cli) -> Result<()> {
    // Registered before anything can know this process exists: the default action for `USR1` is to
    // kill it, and the marker `claim` writes is what makes another instance able to send one.
    let show_yourself = show_yourself_signal();

    // Before the vault file is even opened: two processes over one database, both racing for an
    // agent socket whose path is fixed, is a mess nobody sees until an assistant is talking to a
    // vault the window is no longer showing.
    let mut took_over = false;

    let held = match instance::claim()? {
        instance::Claim::Held(held) => held,
        instance::Claim::Busy(other) => {
            let choice = match (cli.replace, cli.present) {
                (true, _) => instance::Choice::Replace,
                (_, true) => instance::Choice::Present,
                _ => instance::ask(&other),
            };

            match choice {
                // Nothing to start: the vault is already open, and the process holding it is the
                // one that can put it back in front of somebody.
                instance::Choice::Present => {
                    // Announced as well as printed: this branch is reached from a desktop dialog
                    // more often than from a terminal, and a desktop launcher has no terminal for a
                    // refusal to be read in.
                    if let Err(error) = instance::present(&other) {
                        instance::announce(&error.to_string());

                        return Err(error);
                    }

                    println!();
                    println!("  Keyhold is already running (pid {}). Brought it forward.", other.pid);
                    println!();

                    return Ok(());
                }
                instance::Choice::Nothing => {
                    println!();
                    println!("  Keyhold is already running (pid {}). Nothing was changed.", other.pid);
                    println!();

                    return Ok(());
                }
                instance::Choice::Replace => {}
            }

            instance::replace(&other)?;
            took_over = true;

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

    // A fixed port by default, so a bookmark or a browser's saved login keeps working run after run.
    // Asked for explicitly, a port is that port or an error; by default, a taken 7717 falls back to
    // whatever is free rather than refusing to open the vault. Binding to `127.0.0.1` rather than
    // `0.0.0.0` is not a default worth trusting to configuration: there is no setting that makes
    // keyhold listen on a network interface, because there is no version of this product that should.
    let requested = SocketAddr::from((Ipv4Addr::LOCALHOST, cli.port.unwrap_or(DEFAULT_PORT)));
    let listener = match tokio::net::TcpListener::bind(requested).await {
        Ok(listener) => listener,
        Err(_) if cli.port.is_none() => {
            println!();
            println!("  port {DEFAULT_PORT} is taken; using a free one. Bookmarks to :{DEFAULT_PORT} will not reach this run.");

            tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                .await
                .context("could not bind any port on 127.0.0.1")?
        }
        Err(error) => return Err(error).with_context(|| format!("could not bind {requested}")),
    };
    let bound = listener.local_addr()?;

    held.note_port(bound.port());

    // The name `sudo keyhold set-host` wrote, if any. Read once: a name added later counts from the
    // next start, which is what `set-host` tells the person.
    let names = hosts::names_on_this_machine();
    let host = names.first().cloned().unwrap_or_else(|| "127.0.0.1".to_string());
    http::allow_names(names);

    // HTTPS when `set-host` left a certificate: a name other than 127.0.0.1 is not a secure context
    // over plain HTTP, and the browser then refuses the clipboard and notifications this page needs.
    let tls = load_tls().await;
    let scheme = if tls.is_some() { "https" } else { "http" };

    let url = format!("{scheme}://{host}:{}/#token={}", bound.port(), token.expose());

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
        instance::open_browser(&url);

        // Only after a takeover, and only because of how one ends: somebody dismissed a dialog, and
        // the tab that replaces the one they had lands in a browser window that may well be behind
        // the dialog they were looking at. On a plain start they clicked the icon and are watching
        // for exactly this, so saying it would be noise.
        if took_over {
            instance::announce("Comecei de novo. O cofre está trancado, na aba nova do navegador.");
        }
    }

    // Another instance asking to be seen. It cannot open a browser on this process's behalf — it
    // does not know the URL, and the URL is where the session token lives — so it asks, and this is
    // the answer.
    if let Some(mut signals) = show_yourself {
        let url = url.clone();

        tokio::spawn(async move {
            while signals.recv().await.is_some() {
                instance::open_browser(&url);
                instance::announce("O cofre já estava aberto. A janela está no seu navegador.");
            }
        });
    }

    let state = Arc::new(AppState {
        token,
        agent: Default::default(),
        store: std::sync::Arc::new(std::sync::Mutex::new(store)),
        lock: Lock::new(policy),
        failures: std::sync::Mutex::new(0),
        aws: Default::default(),
    });

    // So a question from an agent can bring this window back when nobody has it in front of them.
    state.agent.set_door(url.clone());

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

    match tls {
        Some(config) => {
            let handle = axum_server::Handle::new();
            let stopping = handle.clone();

            tokio::spawn(async move {
                shutdown().await;
                stopping.graceful_shutdown(Some(std::time::Duration::from_secs(2)));
            });

            axum_server::from_tcp_rustls(listener.into_std()?, config)
                .handle(handle)
                .serve(http::router(state).into_make_service())
                .await?;
        }
        None => {
            axum::serve(listener, http::router(state))
                .with_graceful_shutdown(shutdown())
                .await?;
        }
    }

    // Explicit, and the last thing that happens: the machine is only released once nothing is being
    // served on it. Reading as `drop(held)` also keeps a future edit from moving the claim into a
    // scope that ends early.
    drop(held);

    Ok(())
}

/// Listen for "show yourself" without letting it be fatal.
///
/// `USR1` terminates a process that has not asked for it, so this is registered before the marker
/// that makes this process findable — and if registration fails the signal is simply never sent,
/// because the sender only ever signals a pid the marker named.
#[cfg(unix)]
fn show_yourself_signal() -> Option<tokio::signal::unix::Signal> {
    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1()).ok()
}

#[cfg(not(unix))]
fn show_yourself_signal() -> Option<std::convert::Infallible> {
    None
}

/// Ctrl+C, or a `TERM` from another Keyhold taking over.
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


/// The certificate `set-host` made, if there is one and it loads. A broken one is said out loud and
/// skipped: an unreadable certificate must not stand between somebody and their vault.
async fn load_tls() -> Option<axum_server::tls_rustls::RustlsConfig> {
    let (cert, key) = paths::tls_files();

    if !cert.exists() || !key.exists() {
        return None;
    }

    let _ = rustls::crypto::ring::default_provider().install_default();

    match axum_server::tls_rustls::RustlsConfig::from_pem_file(&cert, &key).await {
        Ok(config) => Some(config),
        Err(error) => {
            println!("  could not load the certificate in {}: {error}. Serving plain HTTP.", cert.display());
            None
        }
    }
}

/// `sudo keyhold set-host`: write or remove keyhold's line in /etc/hosts.
/// Ask on the terminal whether to install mkcert's authority. No terminal means no.
fn ask_to_install_authority() -> bool {
    use std::io::{BufRead, IsTerminal, Write};

    if !std::io::stdin().is_terminal() {
        return false;
    }

    println!();
    println!("  Para HTTPS, o mkcert precisa instalar uma autoridade local. Ela passa a ser aceita");
    println!("  pelos navegadores desta máquina para qualquer site, e a chave dela fica na sua home");
    println!("  (~/.local/share/mkcert). Quem copiar esse arquivo consegue se passar por qualquer site");
    println!("  aqui. Se você já usa mkcert, nada muda.");
    print!("  Instalar? [s/N] ");
    let _ = std::io::stdout().flush();

    let mut answer = String::new();
    let _ = std::io::stdin().lock().read_line(&mut answer);

    matches!(answer.trim().to_lowercase().as_str(), "s" | "sim" | "y" | "yes")
}

fn set_host(name: Option<String>, remove: bool, install_ca: bool) -> Result<()> {
    let name = name.unwrap_or_else(|| hosts::DEFAULT_NAME.to_string());

    if !remove {
        hosts::check_name(&name)?;
    }

    #[cfg(unix)]
    if unsafe { libc::geteuid() } != 0 {
        let again = if remove { "--remove".to_string() } else { name.clone() };

        // The full path, not `keyhold`: sudo searches its own secure_path, which does not include
        // ~/.local/bin, so "sudo keyhold" says the command does not exist.
        let me = std::env::current_exe()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "keyhold".to_string());

        anyhow::bail!(
            "mexer no /etc/hosts precisa de sudo. Rode de novo assim:\n\n    sudo {me} set-host {again}\n"
        );
    }

    let path = std::path::Path::new(hosts::HOSTS_FILE);
    let current = std::fs::read_to_string(path).context("não consegui ler o /etc/hosts")?;

    if remove {
        hosts::replace_file(path, &hosts::without(&current))?;
        let removed = hosts::remove_certificate()?;
        println!();
        if removed {
            println!("  Pronto: o nome e o certificado do Keyhold saíram. Reinicie o Keyhold para valer.");
        } else {
            println!("  O nome saiu do /etc/hosts. O certificado fica: rode com sudo (não como root) para apagá-lo.");
        }
        println!();

        return Ok(());
    }

    hosts::replace_file(path, &hosts::with_name(&current, &name))?;

    println!();
    println!("  {name} aponta para 127.0.0.1 no /etc/hosts.");

    let mut made = hosts::make_certificate(&name, install_ca)?;

    if matches!(made, hosts::Certificate::NeedsAuthority) && ask_to_install_authority() {
        made = hosts::make_certificate(&name, true)?;
    }

    // A certificate left from an earlier name would be served for this one and fail in the
    // browser; when no new one was made, the old one goes.
    if !matches!(made, hosts::Certificate::Made { .. }) {
        let _ = hosts::remove_certificate();
    }

    let scheme = match made {
        hosts::Certificate::NeedsAuthority => {
            println!();
            println!("  Sem a autoridade local do mkcert, o Keyhold fica em HTTP por esse nome. Para ter");
            println!("  HTTPS, rode de novo e responda que sim, ou passe --install-ca.");
            "http"
        }
        hosts::Certificate::Made { cert } => {
            println!("  Certificado do mkcert em {}.", cert.display());
            "https"
        }
        hosts::Certificate::NoMkcert => {
            println!();
            println!("  Sem mkcert, o Keyhold fica em HTTP por esse nome, e o navegador não deixa");
            println!("  copiar senhas nem mostrar notificações fora de 127.0.0.1. Para ter HTTPS:");
            println!();
            println!("      sudo apt install mkcert libnss3-tools");
            println!("      sudo {} set-host {name}", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "keyhold".into()));
            "http"
        }
        hosts::Certificate::NoInvoker => {
            println!("  Sem saber quem chamou o sudo, não gerei certificado: rode com sudo, não como root.");
            "http"
        }
    };

    println!();
    println!("  Reinicie o Keyhold para ele passar a atender por esse nome:");
    println!();
    println!("      {scheme}://{name}:{DEFAULT_PORT}");
    println!();

    Ok(())
}
