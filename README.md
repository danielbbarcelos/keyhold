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
downloaded, no update check, no analytics. (A planned vault type kept in AWS Parameter Store will
be the one exception, opt-in per vault and stated where you choose it. See
[Planned: vaults in AWS Parameter Store](#planned-vaults-in-aws-parameter-store).)

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

| Option | Effect |
|---|---|
| `--with=set-host[=NAME]` | Also give Keyhold a name in `/etc/hosts` (`keyhold.local` by default). Asks for sudo for that one step. See [A name instead of 127.0.0.1](#a-name-instead-of-127001) |

Or build it by hand:

```bash
(cd web && npm install && npm run build)
cargo build --release
./target/release/keyhold
```

Keyhold listens on `127.0.0.1`, port 7717 (or a free one if 7717 is taken, with a warning),
generates a session token, and opens your browser at `http://127.0.0.1:7717/#token=…`.

The token is in the URL **fragment** — the one part of an address a browser never sends over the
network. It is generated per run and never written to disk, so restarting Keyhold invalidates every
tab that was open.

```
keyhold              # start the interface
keyhold --port 9000  # bind another port (that one or an error)
keyhold --no-browser # do not open a browser
keyhold --present    # bring a running Keyhold forward (or start one)
keyhold --replace    # close a running Keyhold without asking first
keyhold where        # print where the vault and its directories live
sudo ~/.local/bin/keyhold set-host            # answer to keyhold.local as well
sudo ~/.local/bin/keyhold set-host --remove   # stop
```

`sudo` searches its own `secure_path`, which does not include `~/.local/bin`, so it needs the full
path: plain `sudo keyhold` answers "command not found".

### A name instead of 127.0.0.1

`sudo ~/.local/bin/keyhold set-host [name]` writes one line to `/etc/hosts`, `127.0.0.1  keyhold.local  # keyhold`,
and Keyhold answers to that name from its next start: `https://keyhold.local:7717`. The line is the
only place the name lives; remove it (or run `set-host --remove`) and the name stops working.

**It comes with HTTPS, through [mkcert](https://github.com/FiloSottile/mkcert).** A name other than
`127.0.0.1` is not a secure context over plain HTTP, and browsers then refuse the clipboard and
notifications Keyhold depends on. So `set-host` runs mkcert **as the user who called sudo** (the CA
their browsers trust lives in their home; one made as root would not be trusted), installing that
CA first if it was never installed, and leaves the certificate in `~/.config/keyhold/tls`, the key
readable only by them. Keyhold serves HTTPS whenever that certificate is there. Without mkcert, the
name is still set over HTTP, and `set-host` says how to get HTTPS
(`sudo apt install mkcert libnss3-tools`). `--remove` takes the certificate away too.

Only names that can never exist on the internet are accepted: `.local`, `.localhost`, `.internal`,
`.test` and `.home.arpa`. Keyhold refuses requests addressed to any other name, which is what stops
a web page from reaching it through DNS rebinding. A public name in `/etc/hosts` would work until a
browser resolved it over DNS-over-HTTPS and skipped the file, and then its owner could serve a page
Keyhold would take for its own.

The session token still changes on every run, so a bookmark arrives without it. That tab offers
**Abrir o Keyhold**, which asks the running Keyhold to open its own address in the browser, token
included. The token never comes back to the tab that asked, and a request from another site is
refused.

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

**The agent never uses the HTTP port.** That one belongs to your browser and can change when 7717 is
taken; the socket's path never changes, so a `claude mcp add` done once keeps working across
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

Every request of any kind still stops and asks you, naming the process that asked — and the same
dialog says how long that yes should last. **Just this once**, **while that assistant is open**, a
fixed window from 15 minutes to 30 days, or **for good**.

The middle one is usually what somebody means. It ends when the assistant's process does, and a new
process that inherits its pid inherits nothing else — the process's start time is recorded with it
and checked on every use.

A permission covers **one field of one record**, never a record, a vault, or "whatever that process
asks for next": the question named a field, and the answer does not quietly mean more than the
question did. Anything longer than "just this once" is written into the vault, sealed, so it
survives a restart — and is listed in **Ajustes → Acesso por agente** with what it covers, when it
ends, and a button to take it back. Every *use* still goes into the trail; the permission removes
the question, never the record of what was taken.

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

## Planned: vaults in AWS Parameter Store

> **Being built (F9).** The server side exists; the interface for it is on its way. Not yet: the
> agent surface does not read AWS vaults, and records do not move between local and AWS vaults.

Many teams already keep credentials in AWS Systems Manager Parameter Store, and most people on
those teams find the console hard to use. Keyhold will be able to show and edit those parameters
with the same interface as a local vault, while access stays where it already is: in IAM.

- **Two kinds of vault.** *Local*, as today. *AWS Parameter Store*, where every record is a
  parameter in one account and nothing is kept on this machine. Every vault shows where it lives.
- **AWS accounts.** Keyhold signs in through IAM Identity Center (SSO) itself, without the AWS CLI,
  and keeps the session only in memory. One sign-in can connect several accounts, each with a role
  and an optional title; the same account never appears twice. Renew and revoke apply to the
  sign-in, because that is what the token belongs to.
- **IAM decides who reads.** Values are `SecureString`, encrypted with KMS, with no second layer of
  Keyhold's own: anyone the policy allows can still use the console or the CLI. What the policy
  denies does not appear in Keyhold at all.
- **Folders are the path.** Every `/` is a folder. A record with one value is stored as that plain
  value, so applications keep reading it as they do today; a record with several fields becomes
  JSON. The first line of the parameter's description is the record's title and the rest is its
  notes. A new parameter inherits the KMS key of the folder it is created in.
- **Shared state, personal state.** Kind, "compromised" and "rotate by" are parameter tags, because
  everyone using the secret needs them. Favourites stay on your machine.
- **Staying current.** Keyhold rereads on open, when its window regains focus and every 60 seconds
  while visible, and checks the parameter's version before saving instead of overwriting someone
  else's change. With the session expired or no network, the vault is unavailable rather than
  served from a stale copy.
- **Deleting is final.** Parameter Store has no trash, so neither does an AWS vault; deleting asks
  for the name to be typed.
- **Attachments go to S3**, encrypted, under the same path as their parameter, so the same IAM
  rules apply. The bucket is set per vault (suggested `keyhold-<vault name>`). Keyhold never creates
  it: if it is missing or out of reach, the upload stops with a message saying what to ask the
  account's administrator for.
- **The agent surface works the same way:** off by default, enabled per vault, every value asked
  for in the interface first.

This is the one place Keyhold will send anything off the machine. It happens only for a vault you
create as an AWS vault, only to the account that vault names, and the threat model will say so in
those words. Local vaults stay exactly as they are.

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
