# Roadmap

Each phase ends with its tests green. Nothing moves to the next phase while the previous one has a
promise it does not keep.

| Phase | Delivers | State |
|---|---|---|
| **F0** | Foundation: single binary, loopback bind on a free port, per-run session token in the URL fragment, embedded interface bundle, SQLite vault file with owner-only permissions, strict CSP, and the hardening suite that keeps all of it from eroding | **done** |
| **F1** | Identity and the lock: master password (optional, generated or your own), second factor, Argon2id, key wrapping, recovery key shown once, unlock and lock, auto-lock on idle and on hidden window, zeroization along every path | **done** |
| **F2** | Vaults and folders: several vaults with sealed names, folder tree with its own key, archive and restore, per-vault agent visibility | **done** |
| **F3** | Records: five kinds as templates rather than schemas, reorderable custom fields, per-record AAD, monotonic versions, restorable history | **done** (attachments deferred to F6) |
| **F4** | Daily use: masked fields that reveal on a deadline, clipboard hand-off with clearing, generator, favourites, command palette, tabs, lenses | **done** |
| **F5** | Health, rotation, trash with retention | **done** |
| **F6** | Encrypted attachments, audit trail with a verifiable hash chain, encrypted export, import | **done** |
| **F7** | Agent surface: a unix socket and an MCP server so a local coding agent can look things up without the secret passing through a chat transcript | **done** |
| **F8** | Release: packaging, CI, signed builds | **next** |

## What F1–F4 put in place

**Three layers, each one a choice with a stated cost.** A master password is recommended and
optional — without one, keyhold falls back to a device key stored beside the vault, and the setup
screen says plainly that this means anybody who can read your files can open your vault. A second
factor from an authenticator app is optional, and the interface says what it does and does not
protect against. Auto-lock is recommended and switchable off, in both of its clocks.

**The lock is real, and it runs in the daemon.** Not a screen: locking drops the key. Every route
that touches a payload goes through one function that consults the clock first, so a request that
arrives a second late gets a 423 rather than a secret. A timer in the browser would be a
suggestion — a throttled or suspended tab simply stops counting.

**Names are sealed, not just passwords.** Vault names, folder names, record titles, usernames and
addresses all live inside encrypted blobs. On a machine with no server, a readable list of vault
names is worth reading even when every password in it is ciphertext. What the file still shows is
shape: how many records exist, how they nest, and when they changed.

**The additional data binds each payload to its place and version.** Swapping two records' blobs
fails to open; pasting an old blob over a new one fails to open. Both are covered by tests that do
exactly that to a real vault file.

## What F7 put in place

```text
Claude Code ──stdio (MCP)──> `keyhold mcp` ──unix socket, 0600──> the running keyhold
```

The middle process holds no key and no state; it translates. The daemon is the only thing that can
open anything, and every decision is made there.

**Why a socket and not a port.** The socket's file permission *is* the authentication: no token in
a config file, no port for another local process to find, nothing in `/proc/*/cmdline` to read. It
is created when you switch the surface on and removed when you switch it off, so an agent finds
nothing to connect to rather than something that refuses.

**What actually protects you**, in order of how much it is worth:

1. **Scope by marking.** A vault an agent may see is marked as such, off by default. What is not
   marked is not listed — an absence, not a denial.
2. **Approval per request.** Every value is a question answered in the interface, naming what is
   asking. The socket reports the process *and its parent*, because the process is always
   `keyhold mcp` and the parent is what is actually driving. Both come from `/proc`, never from
   anything the client says about itself.
3. **A locked vault is a dead integration**, and the socket never accepts a master password.
4. **The trail records the agent as its own actor**, refusals included.
5. **`keyhold_run` beats `keyhold_get`.** A command run with the secret in a child process's
   environment keeps the value out of the model's context entirely — and out of `/proc/*/cmdline`,
   which is readable by every process of this user, unlike `environ`.

Telling a model not to leak a secret is not on that list. The tool descriptions do say it, and the
interface never relies on it.

## What F6 put in place

**A trail that can be checked.** Append-only by construction — no `UPDATE`, no `DELETE`, nowhere —
with each row carrying the hash of the one before it, so editing or removing a line breaks every
hash after it and the integrity check names the first break. It records unlocking (and failing to),
creating, changing and deleting; reads are recorded too, and that half can be switched off. It never
contains a value: the trail says a field called "Senha" was revealed, never what it said, and a
deny-list drops anything secret-looking on the way in rather than trusting every caller to remember.

**Attachments, sealed name and all.** A file's name and type are encrypted *with* it, because
"id_rsa" and "contrato-assinado.pdf" say plenty on their own. The bytes live in the vault file
rather than a directory: one file to back up, and no place where a decrypted temporary is forgotten.

**Export that is never plaintext, and import that never merges.** The archive is sealed with a
passphrase of its own — not the master password — and its envelope names the KDF, its parameters,
the salt and the nonce, because an archive nobody can open in six months protects nothing. Import
reads that archive or the CSV every other manager exports, and always creates: matching an incoming
record against an existing one would mean the vault deciding which of two passwords is current, and
it cannot know.

## What F5 put in place

**Health that names reasons instead of scoring.** `log2(alphabet) × length` is exact for a
generated password and a lie about an invented one, so the health lens reports checkable findings —
too short, one class of character, a word with a year on the end, reused elsewhere in the vault,
repeating the record's own title, or simply untouched for a year. Every line is something a person
can look at and agree with, which is the bar for a screen whose only job is to get somebody to
change a password they are attached to.

**Rotation as re-wrapping, not re-encryption.** A vault's key can be replaced and everything that
hung off it re-wrapped, in one transaction. No payload is touched, which makes rotating cheap
enough to do on a suspicion. A rotation that stopped halfway would leave records under two
different keys with the header naming one — so it is all-or-nothing by construction.

**Vaults can be renamed, put away and deleted.** Archiving hides and is undone with a click;
deleting takes the vault's folders, records and every past version with it, in one cascade. It asks
for the vault's name typed back rather than a yes/no — a confirmation box is answered from muscle
memory, a name has to be read and typed — and states a count first, because "this deletes 47
records" is something somebody can weigh and "cannot be undone" is a sentence everybody clicks past.

**A trash that waits.** Deleting archives; destroying is separate, confirmed, and takes the record's
whole version history with it. The retention window is configurable, defaults to 30 days, and can
be switched off entirely. The purge runs on unlock rather than on a timer: keyhold only exists
while somebody is using it, so a schedule would fire whenever the machine happened to be awake.

## What F0 put in place

- `keyhold` binds `127.0.0.1` on an OS-assigned free port. The bind address is not configurable —
  there is no setting that makes it listen anywhere else.
- A 32-byte session token per run, delivered in the URL fragment, compared in constant time,
  required by every `/api` route, and never written to disk.
- The interface bundle compiled into the binary. Nothing is read from disk at runtime and nothing
  is fetched from the network.
- A strict `Content-Security-Policy` on every response, plus `nosniff`, `no-referrer`,
  `same-origin` resource policy and a restrictive permissions policy.
- The vault file at `~/.local/share/keyhold/vault.db`, `0600`, with `WAL` and `secure_delete`, a
  numbered migration chain, and a refusal to open a file written by a newer build.
- `tests/hardening.rs` and `tests/http.rs`, which fail the build on a forbidden pattern in the
  interface or on an API route that answers without the token.

## Deliberately not done yet

`keyhold mcp` exits with an error rather than answering. An agent surface that half-works is one
somebody wires up and trusts.

Attachments, the audit trail, export and import are not built. Deleting a record archives it, but
nothing empties the archive yet — that retention window is F5.
