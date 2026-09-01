<div align="center">

<img src="assets/icon.png" alt="" width="112" height="112">

# Keyhold

**A password vault that lives entirely on your machine.**

No server. No account. No sync. Nothing to sign up for.

</div>

---

## What it is

Keyhold keeps your passwords, keys and private notes in a single encrypted file on your own
computer, and shows them to you in your browser — without any of it ever going anywhere.

You open it, type one master password, and everything is there. You close it, and the key that
opens it stops existing until you type that password again.

It is a single program. There is no service behind it, no company holding a copy, and no account to
lose access to. If your internet is down, Keyhold works exactly the same.

**It can also answer a coding assistant running on your machine** — so you can ask Claude Code to
connect to a server without pasting a password into a chat. Every request stops and asks you first,
naming what is asking, and the preferred way hands the secret to a command instead of to the
assistant.

> **Status: early.** The vault works and is tested, but it has not been through anyone else's
> review. If you put something in it, keep a copy somewhere else too. See
> [docs/ROADMAP.md](docs/ROADMAP.md).

## What Keyhold promises

**The file on disk is unreadable without your password.** If someone copies your backup, syncs your
home folder to a cloud, or walks off with the drive, what they get is noise. The key is not stored
next to it — it is not stored anywhere.

**The key only exists while the vault is open.** It is built from your master password the moment
you unlock, kept in memory, and wiped when you lock or quit. Nobody has a copy: not us, not a
support desk, not someone with a court order — because there is no us and no desk.

**Nothing leaves your machine.** Keyhold only listens to your own computer, never fetches anything
from the internet, and carries everything it displays inside the program itself. No CDN, no fonts
downloaded, no update check, no analytics.

## What Keyhold does not promise

Saying this plainly is part of the design. A vault that implies otherwise is lying to you.

- **If your computer is already compromised, Keyhold loses.** Software that records your keystrokes
  will catch your master password as you type it. A program running as you can read this one's
  memory while the vault is open. Keyhold protects the file when it is closed and narrows the window
  when it is open; it cannot outrank the operating system it runs on.
- **A revealed password is revealed.** Screenshots, photos of the screen, and remembering it are all
  outside anything a program can do.
- **Lose your master password *and* your recovery key, and the data is gone.** There is no reset
  link, no backup copy, no email that fixes it. That is not an oversight — it is exactly what makes
  the first promise above true.

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

Keyhold picks a free loopback port, generates a session token, and opens your browser at
`http://127.0.0.1:<port>/#token=…`.

The token is in the URL **fragment** — the one part of an address a browser never sends over the
network. It is generated per run and never written to disk, so restarting Keyhold invalidates every
tab that was open.

```
keyhold              # start the interface
keyhold --port 7717  # bind a fixed port
keyhold --no-browser # do not open a browser
keyhold --present    # bring a running Keyhold forward (or start one)
keyhold --replace    # close a running Keyhold without asking first
keyhold where        # print where the vault and its directories live
```

### Only one at a time

Two Keyholds over one vault file is a mess that hides itself: both open the same database, and the
agent socket has a fixed path, so the last one to start owns the integration while the earlier ones
keep an unlocked key in memory behind a window you may not be looking at.

So a second start does not happen quietly. It finds the one already running and asks — in the
terminal, or as a desktop dialog when it was launched from the shortcut — and offers three answers:

- **Open and continue** *(the default)* — the running Keyhold puts its own window back in your
  browser. Nothing is closed, nothing is locked, and the vault you already unlocked is the one you
  get. Clicking an icon twice is not a decision to close an open vault.
- **Close it and start over** — the other one is asked to exit properly: that vault locks and its
  key is forgotten, which is exactly what closing a vault should mean.
- **Cancel** — nothing changes.

Continuing works by signalling the running process, which then opens its own URL. The session token
never leaves that process except into your browser, which is where it was always going. Just after
an upgrade, the Keyhold still running is an older build that does not answer that signal — so it is
not offered, and the dialog says why.

Because a browser tab can open behind whatever window you are looking at — and on Wayland no
program gets to force another one to the front — both of those answers also post a desktop
notification saying where the vault went.

**Instâncias** in the rail lists every Keyhold process running as you — the interface, any `keyhold
mcp` bridge an assistant opened, any `keyhold run` still holding a secret — with what each one is
for and how long it has been up. The others can be closed from there, behind a confirmation that
says what closing that particular one costs. A pid typed by a client is never signalled on the
client's word: the request is re-checked against a fresh scan of `/proc`, and the process serving
the window refuses to close itself.

## Letting a local agent use the vault

Keyhold can answer a coding agent running on the same machine — searching for a record, describing
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
restarts. The setting is remembered too — reopening Keyhold reopens the socket. An agent that
connects before you have unlocked is told the vault is locked, which is a more useful thing to hear
than that Keyhold is not running.

Closing the surface removes the socket, and a locked vault answers nothing.

### The assistant can use a credential without ever reading it

This is the part worth understanding, because it is not a matter of asking politely.

**A value handed to a model is in that model's session the moment it arrives.** Not when the model
repeats it — when the tool returns it. Telling an assistant "do not print this" does not undo that,
and no instruction in a tool description is a control.

So Keyhold's answer is to hand the secret to the *process*, never to the model:

```bash
keyhold run "Banco de dados" Senha -- psql "postgres://deploy@localhost/app"
```

The password goes into the command's environment; the assistant gets the command's output and never
the value. It can run that itself, from its terminal, for anything that needs a credential — a CLI
login, a curl, a deploy script. The same thing is available as the `keyhold_run` tool.

When it is **you** who needs the password — to paste into a login form — the assistant uses
`keyhold_copy`: Keyhold puts it on your clipboard, clears it after 45 seconds, and tells the
assistant only that it worked.

**Reading a value outright is off by default.** With it off, `keyhold_get` is not even offered, so a
password cannot end up in a chat transcript at all. You can switch it on in the settings for the
cases where nothing else fits, and the interface says plainly what that costs.

Every request of any kind still stops and asks you, naming the process that asked, answerable once
or for a few minutes.

A request expires by itself after 90 seconds, and the person driving an assistant is usually looking
at a terminal rather than at the vault. So the question also knocks on your desktop as a system
notification — carrying the record's title and the field's label, never a value. Allow it once in
**Ajustes → Acesso por agente**; browsers only offer notifications to a page you have said yes to.

## The desktop shortcut

`install.sh` adds **Keyhold** to your application launcher. Opening it starts Keyhold detached from
any terminal and opens the vault in your browser — the same thing running `keyhold` does, without a
terminal window left behind.

Because there is no terminal to print to, its output goes to a log instead:

```
~/.local/state/keyhold/app.log
```

That log is where the URL and any startup error end up, which is the first place to look if the
shortcut appears to do nothing.

Right-clicking the icon skips the question and answers it directly: **Open and continue**, or
**Close it and start over**.

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

All three are created `0700`, and the vault file `0600`. Keyhold re-checks and repairs those modes
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
cd web && npm run dev          # interface with hot reload, proxying to a Keyhold on :5274
```

`cargo test` is the gate. Two suites exist specifically to keep decisions from eroding:

- `tests/hardening.rs` — scans the interface source for the patterns that break the cryptography
  regardless of how careful the rest of the code is.
- `tests/http.rs` — drives the real router over a real socket to check that the API refuses
  requests without the session token and that every response carries the policy headers.

## Licence

MIT. See [LICENSE](LICENSE).
