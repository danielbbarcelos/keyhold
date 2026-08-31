# Architecture

## One process, one authority

keyhold is a single binary. It owns the vault file, it owns the key while unlocked, and it decides
what any client may see. There are two clients — the browser interface and, from F7, a local agent —
and neither of them holds a key.

That is a deliberate reversal of the usual browser-vault design, where the page does the
cryptography and the server stores blobs it cannot read. That design is correct when the server is
remote and untrusted. Here there is no remote server: the binary is your own process, on your own
machine, under your own user. Doing the cryptography in the browser instead would cost three things
and buy nothing:

1. **An agent would need an open tab.** If the key only exists in a page's memory, nothing can look
   anything up unless a browser window happens to be open and unlocked.
2. **A JavaScript heap is the worst place for a key.** No zeroization, no `mlock`, a garbage
   collector that copies buffers without telling you, and an extension with access to the tab.
3. **Two authorities that drift.** The interface and the agent would each enforce the same rules in
   different code, and the second implementation is always the one that gets it wrong.

So: **one vault, one authority, two clients.** Same rules, same audit trail, same lock state.

## Layout

```
src/
  main.rs      the CLI, and what starting keyhold actually does
  lib.rs       the same code the tests drive
  paths.rs     where files live, and the permissions they are held to
  session.rs   the per-run token: generated, compared in constant time, never persisted
  db.rs        the vault file: open, restrict, migrate
  http.rs      the loopback surface: token gate, policy headers, embedded bundle
web/
  src/         the interface. No storage, no remote origin, no markup rendering
tests/
  hardening.rs the rules the interface must keep, checked against its source
  http.rs      what the API must refuse, driven through a real socket
```

## The lock, and what "locked" means

Locked is not a screen. Locked is the absence of the key: the derived material is zeroized and the
process cannot read the vault until somebody types the master password again. Quitting keyhold is
therefore the strongest possible lock, and a crash is a lock too.

This is why the session token is per-run and never stored. A tab from a previous run is a tab whose
provenance this process cannot vouch for, so it does not get to keep talking.
