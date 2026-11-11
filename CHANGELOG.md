# Changelog

## 0.3.0 (pre-release)

Copying between vaults, local and AWS alike, a vault's own icon, and Keyhold's own logo.

### New

- **Copiar para…** copies a whole vault, a folder or a few records into any vault and folder,
  including from a local vault to AWS Parameter Store and back. It never moves: the original stays
  until you decide, at the end, to delete it (a local original goes to the trash; an AWS original
  only after you type the vault's name, and only what arrived whole). Before writing, Keyhold
  checks the AWS session, the destination's keys and the attachments bucket; going to AWS, it lists
  the parameter names it will create and reminds you they are visible as metadata. A name already
  taken asks you, one by one, whether to rename, overwrite or skip. If something fails half way,
  "Continuar de onde parou" picks up without writing anything twice. Only the current version
  travels; the history stays with the original.
- **A vault's icon**: a lucide icon in a soft colour or a picture of your own, sealed with the
  vault's name, picked when creating the vault or in Ajustes → Cofres.
- **The AWS session in the rail**: when a sign-in is active, the rail shows how long until it
  expires, and renewing (with its code) happens right there.
- **Content width** in Ajustes → Aparência: the whole window, or centred at up to 1280 px.
- **Keyhold's own logo**, in the rail (following the theme), on the unlock screen, and as the app
  and browser icon.
- A `String` parameter holding a JSON object reads as one password field per key.

### Changed

- Ajustes shows one section at a time.
- A new AWS vault picks its account from cards, and says how to connect one when there is none.
- The rail has more room.

### Fixed

- **Auto-lock while the window is open.** Background checks the interface makes (for agent
  requests, and now the AWS session) counted as activity, so a visible window never locked from
  idleness. Only your own clicks and keys count now. As a consequence, an agent's request on its own
  no longer keeps the vault open.

## 0.2.0

A new interface, records without a type, and vaults that can live in AWS Parameter Store.

### New

- **A new interface, one shell for every screen.** A narrow rail (Início, Trilha, Instâncias,
  Ajustes, and the lock with its countdown), a top bar with the path and the search, and cards
  everywhere. Inside a vault the tree of folders and records stays on the left, as a tree or a
  flat list, with Favoritos and Atenção as filters and the trash at its foot. Grafite and Claro
  themes with four accent colours.
- **Início**: the vaults, recently opened records, favourites and what needs attention across every
  vault, and what agents have been doing.
- **Records have no type any more.** A record is a title, an icon, notes and the fields it needs, of
  fifteen kinds: text, multi-line text (visible or hidden), hidden field, password (with generator),
  PIN, 2FA code (shown live with its countdown), e-mail, name, phone, web address, address, date,
  security question and answer, and payment card. New records start empty, with templates (Login,
  Cartão, Chave SSH, Nota) that only add fields.
- **Icons**: a lucide icon in one of the interface's soft colours, or a picture of your own, sealed
  with the record. Nothing is fetched from the web.
- **Editing on the page**, beside the tree, with each field's own menu: show it under the title,
  make it the main action, change its kind, duplicate, remove. Dates are picked from a calendar in
  the interface's own look.
- **AWS Parameter Store vaults.** Sign in through IAM Identity Center from Keyhold itself, connect
  accounts with a role, and keep a vault in an account's Parameter Store: folders are the path,
  values are `SecureString` with KMS, attachments go to an S3 bucket you name, saves refuse to
  overwrite somebody else's change, and the session lives only in memory.
- **`set-host` with HTTPS.** `sudo keyhold set-host` gives Keyhold a local name in `/etc/hosts` and,
  with mkcert, a certificate for it. Port 7717 by default, and a bookmark opens Keyhold through the
  running instance.
- **Desktop knock**: an agent's request shows as a desktop notification when the Keyhold tab is not
  in front of you.

### Changed

- Records written by 0.1 open unchanged: their username and address become ordinary fields.
- The line under a title, and an agent's search, only use a field marked for the list or an e-mail
  or name, never just any text.
- For a 2FA field an agent receives the current code, never the secret behind it.
- CSV import recognises 2FA columns and Bitwarden's login columns, and imports unknown columns as
  hidden fields.
- Locking clears the clipboard immediately.
- Leaving an edit with unsaved changes, or closing a filled-in new record, asks first.

### Security

- `set-host` does everything in your home directory as you, never as root, so a symlink there
  cannot turn its file operations against the system; and it asks before installing mkcert's
  local authority, which browsers trust for every site.
- Spellcheck is off in secret fields, so a browser's cloud spellchecker never sees them.
- A secret revealed in one record can no longer appear in the next one opened.

### Not yet

- Coding assistants cannot reach AWS vaults.
- Records do not move between a local vault and an AWS vault.
- Selecting several records at once (bulk move and delete) from 0.1 is not back yet.

## 0.1.0

The first version: a local encrypted vault with folders, history, trash, attachments, health
checks, an audit trail, export and import, and a local agent surface over MCP.
