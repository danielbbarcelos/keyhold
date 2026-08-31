# keyhold

A password vault that lives entirely on your machine.

No server, no account, no sync, no telemetry. keyhold is a single binary that opens a vault file,
serves an interface to your own browser on loopback, and forgets the key when you close it.

> **Status: early.** The foundation is in place; the vault does not hold secrets yet. See
> [docs/ROADMAP.md](docs/ROADMAP.md) for what is built and what is not. Do not put a real password
> in it until the roadmap says F3 is done.

## What keyhold promises

**The file on disk is ciphertext.** A backup, a synced folder, or another program reading your home
directory finds noise. There is no key stored beside it.

**The key exists only while the vault is unlocked.** It is derived from your master password when
you unlock, held in this process's memory, and wiped when you lock or quit. There is no copy —
including for you, and including for anybody who compels us, because there is no us.

**Nothing leaves the machine.** keyhold binds to `127.0.0.1`, fetches nothing at runtime, and ships
every byte the interface renders inside the binary. There is no CDN, no font host, no update ping.

## What keyhold does not promise

Being honest about this is part of the design:

- **Malware on your machine wins.** A keylogger reads your master password as you type it. A
  program running as you can read this process's memory while the vault is open. keyhold protects
  the vault at rest and narrows the window when it is open; it cannot outrank the operating system.
- **A screenshot is a screenshot.** So is a photograph of the screen, and so is a person who
  memorises the password.
- **Lose your master password and your recovery key and the data is gone.** There is no escrow, no
  reset, no support address that can help. That is what makes the first promise true.

## Install

Requires a Rust toolchain and Node — Node only to build the interface bundle, which is then
compiled into the binary and never needed again.

```bash
git clone https://github.com/danielbbarcelos/keyhold
cd keyhold
./install.sh
```

The installer builds the bundle, compiles the binary, puts it in `~/.local/bin`, and adds a desktop
shortcut. Re-run it any time to upgrade in place.

| Variable | Effect |
|---|---|
| `SKIP_BUILD=1` | Skip the frontend build and use whatever is in `web/dist` |
| `SKIP_DESKTOP=1` | Do not create the desktop shortcut |
| `BIN_DIR=…` | Install the binary somewhere other than `~/.local/bin` |

Or build it by hand:

```bash
(cd web && npm install && npm run build)
cargo build --release
./target/release/keyhold
```

keyhold picks a free loopback port, generates a session token, and opens your browser at
`http://127.0.0.1:<port>/#token=…`.

The token is in the URL **fragment** — the one part of an address a browser never sends over the
network. It is generated per run and never written to disk, so restarting keyhold invalidates every
tab that was open.

```
keyhold              # start the interface
keyhold --port 7717  # bind a fixed port
keyhold --no-browser # do not open a browser
keyhold where        # print where the vault and its directories live
```

## Letting a local agent use the vault

keyhold can answer a coding agent running on the same machine — searching for a record, describing
its fields, and handing over a value once you say yes.

```bash
claude mcp add keyhold --scope user -- ~/.local/bin/keyhold mcp
```

Then switch the surface on in **Ajustes → Acesso por agente**, and mark the vaults it may see. What
is not marked is not listed to the agent at all.

The connection is a unix socket at `~/.local/state/keyhold/agent.sock`, mode `0600`: the file's
permission is the authentication, so there is no token in a config file and no port for another
process to find.

**The agent never uses the HTTP port.** That one is picked fresh on every run and belongs to your
browser; the socket's path never changes, so a `claude mcp add` done once keeps working across
restarts. The setting is remembered too — reopening keyhold reopens the socket. An agent that
connects before you have unlocked is told the vault is locked, which is a more useful thing to hear
than that keyhold is not running.

Closing the surface removes the socket, and a locked vault answers nothing.

**Every value handed to an assistant leaves this machine** — it becomes part of the model's
context. That is the opposite of everything else here, so each request is a question in the
interface, naming what is asking, answerable once or for a few minutes. Prefer `keyhold_run`, which
executes a command with the secret in an environment variable and returns only the output: the
value never enters the conversation.

## The desktop shortcut

`install.sh` adds **keyhold** to your application launcher. Opening it starts keyhold detached from
any terminal and opens the vault in your browser — the same thing running `keyhold` does, without a
terminal window left behind.

Because there is no terminal to print to, its output goes to a log instead:

```
~/.local/state/keyhold/app.log
```

That log is where the URL and any startup error end up, which is the first place to look if the
shortcut appears to do nothing.

| File | What |
|---|---|
| `~/.local/share/applications/keyhold.desktop` | the launcher entry |
| `~/.local/bin/keyhold-launch` | the wrapper it runs |
| `~/.local/share/icons/hicolor/*/apps/keyhold.png` | the icon, at each size |

**To change the icon**, replace `assets/icon.png` (a square PNG, 1024×1024 is ideal) and run
`./install.sh` again. If the launcher keeps showing the old one, its cache is stale:
`gtk-update-icon-cache -f -t ~/.local/share/icons/hicolor`, or log out and back in.

**To stop an instance the shortcut started**, close it from the interface, or find it with
`ss -ltnp | grep keyhold` — locking is what matters, and quitting is the strongest lock there is.

**To remove the shortcut:**

```bash
rm ~/.local/share/applications/keyhold.desktop ~/.local/bin/keyhold-launch
rm ~/.local/share/icons/hicolor/*/apps/keyhold.png
```

## Where things are kept

| Path | What | Lifetime |
|---|---|---|
| `~/.local/share/keyhold/vault.db` | the vault — ciphertext and nothing else | forever; this is the file to back up |
| `~/.local/state/keyhold/` | sockets and other per-run state | dies with the process |
| `~/.config/keyhold/` | preferences; never a key, never a secret | forever |

All three are created `0700`, and the vault file `0600`. keyhold re-checks and repairs those modes
every time it opens, because the directory that already existed is the one nobody checked.

## Security design

The short version:

- **AES-256-GCM** for payloads, with additional authenticated data binding each record to its own
  identity and version — a byte flipped in the file makes decryption fail rather than return
  garbage, and an old ciphertext cannot be swapped back in.
- **Argon2id** (64 MiB, 3 iterations) to turn the master password into a key, with parameters
  versioned in the file so they can be raised later without migrating data.
- **Wrong password is a decryption failure**, not a hash comparison. Nothing in the file confirms a
  guess is right except the data opening.
- **A strict Content-Security-Policy** on the interface: `default-src 'none'`, `script-src 'self'`
  with no `unsafe-inline` and no `unsafe-eval`, and no remote origin in any directive — enforced by
  a test, because the page that renders secrets is the page an injected script would want.
  (`style-src` allows inline styles, which the component library needs to position menus; the
  reasoning and what it costs are in the threat model.)
- **No web storage anywhere in the interface.** No `localStorage`, no `sessionStorage`, no
  IndexedDB, no cookies. A test fails the build if one appears.

The full reasoning, including what is deliberately out of scope, is in
[docs/THREAT_MODEL.md](docs/THREAT_MODEL.md).

## Development

```bash
cargo test                     # includes the hardening and HTTP-surface checks
cd web && npm run build        # rebuild the embedded interface
cd web && npm run dev          # interface with hot reload, proxying to a keyhold on :5274
```

`cargo test` is the gate. Two suites exist specifically to keep decisions from eroding:

- `tests/hardening.rs` — scans the interface source for the patterns that break the cryptography
  regardless of how careful the rest of the code is.
- `tests/http.rs` — drives the real router over a real socket to check that the API refuses
  requests without the session token and that every response carries the policy headers.

## Licence

MIT. See [LICENSE](LICENSE).
