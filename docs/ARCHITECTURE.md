# Architecture

## One process, one authority

Keyhold is a single binary. It owns the vault file, it owns the key while unlocked, and it decides
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
  instance.rs  one vault, one process: the claim, and taking over a running one
  session.rs   the per-run token: generated, compared in constant time, never persisted
  db.rs        the vault file: open, restrict, migrate
  http.rs      the loopback surface: token gate, policy headers, embedded bundle
web/
  src/         the interface. No storage, no remote origin, no markup rendering
tests/
  hardening.rs the rules the interface must keep, checked against its source
  http.rs      what the API must refuse, driven through a real socket
```

## Vaults in AWS Parameter Store (F9)

A vault has a kind. A local vault is the file, as always. An AWS vault keeps only its name and its
place (connection, region, bucket) in the file, sealed like any vault's metadata; its records are
SecureString parameters in one AWS account, read from `/` with `GetParametersByPath`, and its
attachments are S3 objects under the parameter's name.

```
src/aws/
  sigv4.rs  ssm.rs  s3.rs  sso.rs  error.rs   the client layer: signing, Parameter Store, S3, SSO
  mapping.rs   record <-> parameter: ids, names, folders, value format, description, tags, keys
  config.rs    logins, connections, favourites: sealed under the master key, in `settings`
  state.rs     tokens, credentials, short caches: memory only, tied to the lock's generation
src/http/
  aws.rs       the /aws routes, and the AWS side of every vault, folder, record and attachment route
```

**Same routes, same shapes.** `http/contents.rs` checks, before it touches the store, whether the
vault is an AWS one or the id starts with `aws~`; if so the request goes to `http/aws.rs`, which
answers with the same `FolderNode`, `RecordPreview`, `OpenRecord` and `Attachment` the local side
uses. Ids carry their place and their kind: `aws~<vault id>~<r|f|a>~<base64url(parameter name,
path or object key)>`, and a route refuses an id of the wrong kind as "not found". An attachment
id must be one file directly under a record's prefix (`<name without its leading slash>/`), and
only names that start with `/` have a prefix, so `db` and `/db` can never share files.

**Where a record's parts live.** The value (encrypted) holds the fields and the notes, either as
the plain secret (one password, nothing else) or as Keyhold's JSON. The description holds the title
alone, the path is the folder, and tags hold the marks: all three are readable by anyone allowed to
describe the account, which is why notes are not among them.

**What the AWS routes add to the shared shapes.** Records (list and open) carry `plaintext: true`
for a `String`/`StringList` parameter, which Keyhold will not overwrite. A save or restore may
answer `warning` (another version landed in between, or the format tag could not be removed). A
restore takes `expected_version` in its body (or the `x-keyhold-expected-version` header), like a
save. A move answers `history_lost: true`, and `warning` when the old name could not be deleted.

**Fail closed on the key.** Every write that must name a KMS key (overwrite, move, attachment, and
create, which inherits the folder's) reads it first (DescribeParameters by exact name, then the
newest history entry, or the listing's metadata for inheritance) and refuses when it cannot.

**The store's lock is never held across a request to AWS.** It is a `std::sync::Mutex`; every AWS
function takes it, reads what it needs, lets go, and only then awaits the network.

**Tied to the lock.** `Lock` keeps a generation that changes on every unlock and every lock
(including the automatic one). `AwsState` remembers the generation its tokens were made in and
drops everything when it no longer matches, so a lock forgets every AWS session and the person logs
in again after unlocking. The generation is read under `AwsState`'s own mutex; requests that fetch
tokens or credentials capture it when they start and store through `with_generation`, which refuses
if a lock happened meanwhile. A background task (`http::start_aws_sweeper`, every five seconds)
sweeps the state, so an auto-lock forgets everything without waiting for the next request, and
stale cache entries are removed rather than skipped.

**Clients.** The HTTP client ignores system proxies and, outside the tests, refuses non-HTTPS URLs;
endpoint overrides are crate-private and only the tests set them. S3 requests carry a signed
`x-amz-expected-bucket-owner` with the connection's account, and downloads are capped at the
attachment limit before the body is read.

**Errors that say what to do.** `AwsError` becomes `{ "error": "<frase>", "aws": "<kind>" }` with
424 for an expired AWS session (401 stays reserved for Keyhold's own session token), 403, 404, 409,
429, 502 and 400.

## The lock, and what "locked" means

Locked is not a screen. Locked is the absence of the key: the derived material is zeroized and the
process cannot read the vault until somebody types the master password again. Quitting Keyhold is
therefore the strongest possible lock, and a crash is a lock too.

This is why the session token is per-run and never stored. A tab from a previous run is a tab whose
provenance this process cannot vouch for, so it does not get to keep talking.
