# Changelog

## Unreleased

### New

- **Ajustes, Campos de senha**: whether the browser may fill in and save a record's password and
  hidden fields. Off by default: those fields hide what is typed without being password fields,
  so the browser and its password manager leave them alone.

### Changed

- Vaults are listed by name on Início, in the vault switcher and in Ajustes, and the icon library
  by name in the picker and in Personalização.

### Fixed

- **A picture as the icon of a record in an AWS vault** was refused: a parameter holds 8 KB, and
  the picture alone took that. The parameter now gets a small copy, for whoever else opens the
  vault, with the picture's place in your icon library beside it, so on your machine the full
  picture is what shows. A record copied into AWS whose picture does not fit goes without it
  instead of failing.

## 1.0.3

Keyhold made your own: record types, an icon library, more icons and colours, in a new
Personalização screen.

### New

- **Personalização**, a new place on the rail beside Ajustes: appearance, record types, the icon
  library and the standard alerts. Ajustes keeps what decides how the vault is protected.
- **Record types are yours**: Login, Cartão, Chave SSH and Nota can be changed, reordered or
  removed, and new ones made, each with its icon and the fields a new record starts with.
- **An icon library**: a picture used as an icon is kept, sealed, to be used again on any record
  or vault. The icon picker has two tabs, the system's icons and your own.
- More than 270 icons to choose from, and fourteen colours instead of seven.
- "Editar cofre", beside the vault's name, changes its name, icon and colour without leaving the page.

### Changed

- Folders are picked by typing, in a searchable list, wherever something is moved or filed.
- In an AWS vault the tree and the record stand as skeletons while something is saved, created,
  moved or cloned, instead of showing the old state until AWS answers.
- Everything that can be clicked shows the pointer, menu items and options included.
- The countdown ring of a 2FA code runs clockwise.
- The AWS confirmation dialog can copy its link, to confirm in another browser.
- In the vault switcher the AWS mark is always at the end of the row, the check sits beside the
  name, and a long name is cut instead of pushing them out.

## 1.0.2

A security review of the whole project, and Windows starting on a machine that has nothing else
installed.

### Security

- **The approval dialog shows the exact command** an assistant wants to run with a secret, and the
  trail records it. A command can do anything with the value it receives, and the dialog now says
  so instead of promising the opposite.
- **A lasting yes is for one use and one program**: reading a field is not running a command with
  it, and a yes to `mysql` is not a yes to `sh`. Permissions are sealed to their record, so editing
  the file cannot point one at another record. "Todos os campos deste registro" starts unchecked.
  Existing permissions are dropped by this upgrade and asked again.
- A command run with a secret is stopped after five minutes or one megabyte of output, and no
  longer blocks Keyhold while it runs.
- Lock clocks, read logging, trash retention and what an assistant may read can only be changed
  with the vault open. A second factor cannot be replaced without a code from the current one,
  and wrong codes when turning it off count like wrong passwords.
- A question is only asked about a field that exists. Switching the integration off also ends
  connections that were already open. "Enquanto estiver aberto" is not offered for a process
  started by the system.
- A vault file asking for an absurd key derivation is refused instead of taking Keyhold down, a
  crash no longer writes a core file, and a trail line written at the instant of locking names
  nothing.
- `install.ps1` registered `keyhold://` behind PowerShell, where a crafted link could become a
  command. It now uses the same launcher as the installer, which ignores the link.
- Releases: build jobs only read the repository, actions are pinned to commits, npm install
  scripts are off, and every download comes with `SHA256SUMS.txt` and a build provenance
  attestation (`gh attestation verify`).

### Fixed

- **Windows**: Keyhold installed and then failed to start with "VCRUNTIME140.dll não foi
  encontrado" on a machine without the Visual C++ runtime. The runtime is now inside the
  executables, which also carry version information and the icon.
- **A lasting permission for a record in an AWS vault could not be saved**, so every request asked
  again.
- Attachments above about 1.5 MB were refused by the server before reaching the 25 MB limit.

## 1.0.1

Secrets used by coding assistants never show up in their conversation or in the commands they
write, and one approval answers a whole burst of requests.

### Security

- **A secret never reaches an assistant**, from a local vault or an AWS one, whatever the switch in
  Ajustes says. `keyhold_get` refuses passwords, PINs, cards and hidden fields without even asking;
  they are used through `keyhold_run`, in an environment variable (`MYSQL_PWD`, `PGPASSWORD`) or in
  a `{{secret}}` argument filled in at execution. A command that already carries the value is
  refused, and the value is erased from the output (plain, base64, URL and hex) before it goes
  back. Every MCP client receives these rules when it connects, and again with each search.

### New

- **A Windows installer**, `KeyholdSetup.exe`: per user, no administrator, Start menu entry,
  `keyhold://`, uninstallable. Every tag `v*` (or release published on GitHub) now builds the three
  downloads by itself and publishes the release with its changelog section.
- **Coding assistants reach AWS vaults** switched on for them in Ajustes → Agentes, with the same
  approval per field and the same trail as a local vault, through the AWS session open in Keyhold.
- **Renewing an expired AWS session from the vault**: the code and the AWS page come up right
  there, and the vault reloads once AWS says yes.
- Every button with only an icon names its action in a tooltip.
- **One question per burst**: requests from the same assistant for the same record come up in one
  dialog, and a lasting yes can cover every field of that record.

### Changed

- **The interface's words**, rewritten to one writing guide: no dashes or semicolons, dialog titles
  that state the action, destructive confirmations that name what is lost, errors that say what
  did not happen and what to do, and one vocabulary (excluir, remover, arquivar, descartar). A
  vault is now "arquivado", not "guardado". `npm run lint:copy` checks the mechanical rules.
- How long a yes lasts is picked in a searchable list, and the approval dialog no longer spills
  out of its frame.
- Lists in Ajustes (vaults, agents, the trail switch) are bordered rows with dividers and hover,
  easier to aim at on a wide screen.

## 1.0.0

Keyhold on Linux, macOS and Windows with downloads ready to install, names in AWS said before they
are sent, sessions that renew themselves, alerts on records, a password generator, and a tree
that moves and deletes several things at once.

### New

- **macOS and Windows.** Keyhold now runs on both, tested on every change alongside Linux. On
  macOS the installer makes `Keyhold.app`; on Windows `install.ps1` adds it to the Start menu.
  The agent talks through a per-user named pipe on Windows, and on both ends only a process of the
  same user is answered. `set-host` is still Linux and macOS only.
- **A new record is written on the page**, like an edit, with the templates (Login, Cartão…) on
  top and, in an AWS vault, the parameter name as you type.

- **"Pede atenção" explains itself**: the red dot opens what was found (compromised, change date
  passed, weak, repeated or old password) and what to do, with Editar and "Não está mais
  comprometido" right there.
- **Collapse or expand every folder** of a vault's tree with one button.
- **Alerts on a record.** A coloured band with an icon, a title and a line of text at the top of
  the record ("Não compartilhar com terceiros"), as many as needed, each with an × to take it off.
  Standard alerts live in Ajustes → Alertas, sealed under the master key; changing one changes
  every record using it, and a record keeps a copy for anywhere the standard one is unknown.
- **A password generator dialog.** Type (password or PIN), length, the four kinds of character,
  which symbols exactly, and avoiding look-alikes; the value can be edited by hand before it is
  used. Its default lives in Ajustes → Gerador de senhas, or "Salvar como padrão" in the dialog.
- **Widths you drag**: the vault's tree, and the column of field names in a record, kept by
  Keyhold across restarts. Double-click a handle to go back to the default.
- Text cut short in the tree and in field names shows whole in a tooltip.
- **Several items at once in the tree**: Ctrl (⌘) click and Shift click pick records and folders,
  and dragging them drops all of them on a folder or on the root. Folders can be dragged now too,
  local and AWS. In AWS, moving shows first every parameter name that changes, from and to, since
  each one is rewritten under its new name and loses its history; a folder holding a `String`
  parameter or attachments is refused whole.
  In the tree, Ctrl click picks instead of opening a new tab; the middle button still does that.
  The selection has an "Ações" menu: move to the root or a folder, copy, favourite, and delete
  (a picked folder goes with everything inside it, shown a shade lighter while picked).
- **Nova pasta** in the top bar (at the root, or inside the folder being shown), and a right-click on
  the tree's empty space to make a folder or a record at the root. A folder at the root could not
  be made at all before.

### Changed

- In an AWS vault, saving something that turns a lone password's plain value into Keyhold's JSON
  (an alert, an icon, notes, another field) asks first: applications reading the value stop
  understanding it.

### Fixed

- An AWS vault without a session shows as "Indisponível" on Início instead of "0 registros".
- The title in the record editor lines up with Pasta and Trocar até again; a hint under one field
  no longer pushes its neighbours down.

## 0.3.0

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
- **Names in AWS, said before they are sent.** An AWS folder takes any name ("Bancos (PJ)"): its
  path gets a clean segment (`/bancos-pj`) and the name lives on the folder's marker, the way a
  record's title lives in its description. Under a title or a folder name, "Na AWS: …" shows the
  parameter name being made, and "Editar nome" lets you write your own, with the field turning red
  on anything AWS refuses (space, accents, `: ; , + = [ ] ( )`).
- **Renaming in AWS.** Changing a record's title or a folder's name offers "Mudar o nome na AWS
  também", and each AWS vault decides whether it starts ticked (Ajustes → Cofres). Without it only
  the name in Keyhold changes; with it the parameter (or everything in the folder) moves to the new
  name and loses its history. A folder holding attachments or a `String` parameter keeps its path.
- **Skeletons while things load**: a vault opening, a new vault, a record being opened. An AWS
  vault no longer shows "empty" before its parameters arrive.
- **Abrir em nova guia** on everything that leads somewhere (the rail, vaults, folders, records,
  the path at the top, Ajustes), on right-click, Ctrl click or the middle button. A reload comes
  back to the same place.

- **AWS sessions renew themselves.** Keyhold now asks Identity Center for a refresh token and
  trades it for a new access token before the hour runs out, for as long as Identity Center's own
  session duration allows and only while Keyhold is unlocked. The rail shows "ativa" instead of a
  countdown. Logins made before this need one last "Renovar".
- When the program stops answering, the tab says so plainly and offers **Abrir o Keyhold**
  (through `keyhold://`, which the installer now registers) and **Tentar de novo**.

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
