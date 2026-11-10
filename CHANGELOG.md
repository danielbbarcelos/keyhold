# Changelog

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
