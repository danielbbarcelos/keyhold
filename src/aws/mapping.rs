//! How a Keyhold record becomes a Parameter Store parameter, and back.
//!
//! Everything in this file is pure: no request, no clock, no store. It is the part of an AWS vault
//! that has to be exactly right in both directions, because the same parameter is read by Keyhold,
//! by the AWS console, and by whatever application the secret is for. The rules are the product
//! owner's (A7 to A14 in `docs/ROADMAP.md`), and they are written down here once:
//!
//! - **A folder is a path.** Every `/` in a parameter name is a folder. A folder with nothing in it
//!   exists only as a marker parameter, `<path>/.keep`, which no list ever shows.
//! - **The value is the secret, and nothing else is.** A record with a single plain password is
//!   stored as that password, so `aws ssm get-parameter` hands an application the value it expects.
//!   Anything richer (several fields, a composite field, an icon, a field marked for the list or
//!   the copy button) is stored as Keyhold's JSON, `{"keyhold":1,"fields":[...],"icon":...}`, and
//!   the parameter carries the tag `keyhold:formato=json` so an application's own JSON config is
//!   never mistaken for one of ours.
//! - **The title lives in the description; the notes live in the value.** The description is not
//!   encrypted: anyone allowed `ssm:DescribeParameters` reads it, and CloudTrail records it with
//!   every `PutParameter`. A title is a label and can live there; notes are where people write
//!   "the backup PIN is 4411", so a record with notes is always stored as Keyhold's JSON with the
//!   notes inside the encrypted value. A parameter nobody described is titled after the last
//!   segment of its name. Descriptions written before this rule (title, newline, notes) are still
//!   read, and the next save moves those notes into the value.
//! - **Marks live in tags.** `keyhold:comprometido` (an ISO date) and `keyhold:trocar-ate`
//!   (`YYYY-MM-DD`). Favourites are personal and stay on this machine.
//! - **Ids carry where they point, and what they point at.**
//!   `aws~<vault id>~<r|f|a>~<base64url(name, path or object key)>`, so a route that receives one
//!   knows it is AWS, which vault, whether it is a record, a folder or an attachment, and which
//!   one, without a lookup table that would be a local copy of somebody's account. The kind is
//!   checked: a folder id handed to a record route is "not found", never a parameter of the same
//!   name.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::encoding::{base64url, from_base64url};
use crate::records::{Field, Icon, Payload};
use crate::vaults::FolderNode;

/// What every AWS id starts with. A local id is base32, which never contains `~`.
pub const ID_PREFIX: &str = "aws~";

/// The marker that makes an empty folder exist.
pub const KEEP: &str = ".keep";

/// What the marker holds. Parameter Store refuses an empty value.
pub const KEEP_VALUE: &str = "keyhold";

pub const TAG_FORMAT: &str = "keyhold:formato";
pub const TAG_FORMAT_JSON: &str = "json";
pub const TAG_COMPROMISED: &str = "keyhold:comprometido";
pub const TAG_ROTATION: &str = "keyhold:trocar-ate";

/// Parameter Store's limit on a description.
pub const DESCRIPTION_LIMIT: usize = 1024;

/// Parameter Store's limit on a whole name, prefix included.
pub const NAME_LIMIT: usize = 1011;

/// Parameter Store's limit on how deep a name may go.
pub const MAX_LEVELS: usize = 15;

/// A Standard parameter holds 4 KB; past that it has to be Advanced, which holds 8 KB.
pub const STANDARD_LIMIT: usize = 4096;
pub const ADVANCED_LIMIT: usize = 8192;

/// How a plain value reads back: one field, a password, labelled "Valor".
pub const PLAIN_LABEL: &str = "Valor";
pub const PLAIN_KIND: &str = "password";

/// The AWS-managed key Parameter Store uses when none is named. Inheriting it is the same as not
/// naming one, and naming it for S3 would fail (an AWS-managed key serves one service only).
pub const DEFAULT_SSM_KEY: &str = "alias/aws/ssm";

// ── ids ─────────────────────────────────────────────────────────────────────────────────────────

pub fn is_aws_id(id: &str) -> bool {
    id.starts_with(ID_PREFIX)
}

/// What an AWS id points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdKind {
    /// A parameter; the target is its name.
    Record,
    /// A path; the target is the folder's path.
    Folder,
    /// An S3 object; the target is its key, `<record prefix><file name>`.
    Attachment,
}

impl IdKind {
    fn tag(self) -> &'static str {
        match self {
            IdKind::Record => "r",
            IdKind::Folder => "f",
            IdKind::Attachment => "a",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "r" => Some(IdKind::Record),
            "f" => Some(IdKind::Folder),
            "a" => Some(IdKind::Attachment),
            _ => None,
        }
    }
}

/// `aws~<vault id>~<kind>~<base64url(target)>`.
pub fn encode_id(kind: IdKind, vault_id: &str, target: &str) -> String {
    format!("{ID_PREFIX}{vault_id}~{}~{}", kind.tag(), base64url(target.as_bytes()))
}

pub fn record_id(vault_id: &str, name: &str) -> String {
    encode_id(IdKind::Record, vault_id, name)
}

pub fn folder_id(vault_id: &str, path: &str) -> String {
    encode_id(IdKind::Folder, vault_id, path)
}

pub fn attachment_id(vault_id: &str, key: &str) -> String {
    encode_id(IdKind::Attachment, vault_id, key)
}

/// The kind, the vault id and the target an AWS id points at, or `None` for anything that is not
/// one. An attachment id whose key is not one file directly under a record's prefix is not one
/// either (see [`attachment_record`]).
pub fn decode_id(id: &str) -> Option<(IdKind, String, String)> {
    let rest = id.strip_prefix(ID_PREFIX)?;
    let mut parts = rest.splitn(3, '~');
    let (vault, tag, encoded) = (parts.next()?, parts.next()?, parts.next()?);

    if vault.is_empty() || encoded.is_empty() {
        return None;
    }

    let kind = IdKind::from_tag(tag)?;
    let target = String::from_utf8(from_base64url(encoded)?).ok()?;

    if kind == IdKind::Attachment {
        attachment_record(&target)?;
    }

    Some((kind, vault.to_string(), target))
}

/// Where a parameter's files go in the bucket: `<name without the leading slash>/`.
///
/// Only for names that start with `/`. Parameter Store keeps `db` and `/db` apart, but both would
/// map to the prefix `db/`, and the files of one would show up (and be deleted) as the other's.
/// Keyhold always writes names with the slash; a parameter without one simply has no attachments.
pub fn attachment_prefix(name: &str) -> Option<String> {
    let rest = name.strip_prefix('/')?;
    if rest.is_empty() || rest.starts_with('/') {
        return None;
    }
    Some(format!("{rest}/"))
}

/// The parameter an attachment key belongs to, or `None` when the key is not exactly one file
/// directly under a valid record's prefix: no leading `/`, no `.` or `..`, nothing nested.
pub fn attachment_record(key: &str) -> Option<String> {
    let (record, file) = key.rsplit_once('/')?;

    if record.is_empty() || record.starts_with('/') || file.is_empty() {
        return None;
    }
    // Parameter Store allows a segment called `..`; a URL would resolve it into another key.
    if key.split('/').any(|segment| segment == "." || segment == "..") {
        return None;
    }

    let name = format!("/{record}");
    validate_name(&name).ok()?;

    Some(name)
}

/// Whether an object key is one of this record's attachments (and not a file of a record nested
/// under it, which shares the prefix).
pub fn is_attachment_of(prefix: &str, key: &str) -> bool {
    key.strip_prefix(prefix).is_some_and(|file| !file.is_empty() && !file.contains('/'))
}

// ── names and paths ─────────────────────────────────────────────────────────────────────────────

/// A name as a path from the root. Parameter Store treats `db-password` and `/db-password` as two
/// different parameters, but for drawing a tree both sit at the root.
pub fn absolute(name: &str) -> String {
    format!("/{}", name.trim_start_matches('/'))
}

/// The folder a name sits in, or `None` for the root.
pub fn parent_path(name: &str) -> Option<String> {
    let absolute = absolute(name);
    let (parent, _) = absolute.rsplit_once('/')?;

    if parent.is_empty() {
        None
    } else {
        Some(parent.to_string())
    }
}

/// The last segment of a name.
pub fn leaf(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// A name under a folder (`None` = the root).
pub fn join(folder: Option<&str>, leaf: &str) -> String {
    match folder {
        Some(folder) => format!("{}/{leaf}", folder.trim_end_matches('/')),
        None => format!("/{leaf}"),
    }
}

pub fn is_keep(name: &str) -> bool {
    leaf(name) == KEEP
}

/// A segment as Parameter Store allows it: letters, digits, `.`, `-` and `_`.
pub fn validate_segment(segment: &str) -> Result<(), String> {
    if segment.is_empty() {
        return Err("o nome não pode ficar vazio".into());
    }

    if let Some(bad) = segment
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
    {
        return Err(format!(
            "o nome não pode ter '{bad}': na AWS só valem letras sem acento, números, ponto, hífen e sublinhado"
        ));
    }

    if segment == KEEP {
        return Err("esse nome é reservado para as pastas".into());
    }

    Ok(())
}

/// A whole name, checked against what AWS would refuse anyway, so the person hears it in words.
pub fn validate_name(name: &str) -> Result<(), String> {
    let absolute = absolute(name);

    if absolute.len() > NAME_LIMIT {
        return Err(format!("o nome passa de {NAME_LIMIT} caracteres"));
    }

    let segments: Vec<&str> = absolute.split('/').skip(1).collect();

    if segments.len() > MAX_LEVELS {
        return Err(format!("a AWS só aceita {MAX_LEVELS} níveis de pasta"));
    }

    if let Some(first) = segments.first() {
        let lower = first.to_ascii_lowercase();
        if lower.starts_with("aws") || lower.starts_with("ssm") {
            return Err("a AWS reserva nomes que começam com aws ou ssm".into());
        }
    }

    for segment in segments {
        validate_segment(segment)?;
    }

    Ok(())
}

/// A title or folder name may not contain `/`: it would read as one more folder.
pub fn validate_title(title: &str) -> Result<(), String> {
    if title.trim().is_empty() {
        return Err("o registro precisa de um título".into());
    }

    if title.contains('/') {
        return Err("o título não pode ter '/', porque na AWS isso vira uma pasta".into());
    }

    Ok(())
}

/// A parameter name made from a title: lowercase, ASCII, hyphens.
///
/// "Senha do Banco (produção)" becomes `senha-do-banco-producao`. Only a suggestion: the person
/// may type their own, which is then checked by [`validate_segment`] and nothing stricter.
pub fn suggest_name(title: &str) -> String {
    let mut out = String::new();
    let mut gap = false;

    for c in title.chars().flat_map(fold_accent) {
        if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.push(c.to_ascii_lowercase());
        } else {
            gap = true;
        }
    }

    // Dots trimmed from the ends, which also means a suggestion is never the `.keep` marker.
    let trimmed = out.trim_matches('.').to_string();

    if trimmed.is_empty() {
        "registro".into()
    } else {
        trimmed
    }
}

/// The letters Portuguese and its neighbours use, without their accents.
fn fold_accent(c: char) -> Vec<char> {
    let plain = match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'Á' | 'À' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'A',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'É' | 'È' | 'Ê' | 'Ë' => 'E',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'O',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
        'ç' => 'c',
        'Ç' => 'C',
        'ñ' => 'n',
        'Ñ' => 'N',
        'ß' => return vec!['s', 's'],
        other => other,
    };

    vec![plain]
}

// ── description: the title ──────────────────────────────────────────────────────────────────────

/// The title and the notes a description holds. No description: the title is the last segment.
///
/// Keyhold now writes only the title there; the notes are read for the descriptions written
/// before that, which carried them after the first line.
pub fn split_description(description: Option<&str>, name: &str) -> (String, String) {
    let description = description.unwrap_or_default();

    match description.split_once('\n') {
        Some((title, notes)) if !title.trim().is_empty() => {
            (title.trim().to_string(), notes.to_string())
        }
        None if !description.trim().is_empty() => (description.trim().to_string(), String::new()),
        _ => (leaf(name).to_string(), String::new()),
    }
}

/// The description Keyhold writes: the title alone, on one line, or the reason it cannot be one.
///
/// Only the title, because the description is readable by anyone allowed `DescribeParameters` and
/// lands in CloudTrail with every write. The notes go into the encrypted value instead
/// ([`encode_value`]).
pub fn description_for(title: &str) -> Result<String, String> {
    validate_title(title)?;

    let title = title.trim().replace(['\r', '\n'], " ");
    let length = title.chars().count();

    if length > DESCRIPTION_LIMIT {
        return Err(format!(
            "na AWS o título cabe em {DESCRIPTION_LIMIT} caracteres, e este tem {length}. \
             Encurte o título"
        ));
    }

    Ok(title)
}

// ── the value: plain or Keyhold's JSON ──────────────────────────────────────────────────────────

/// Keyhold's JSON, as it is written into a parameter's value.
#[derive(Serialize, Deserialize)]
struct Stored {
    keyhold: u32,
    fields: Vec<Field>,
    #[serde(default)]
    icon: Option<Icon>,
    /// The record's notes, inside the encrypted value rather than in the plaintext description.
    /// Absent in values written before notes moved here, and when there are none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    notes: String,
}

/// What a value holds: the fields, the icon, and the notes when it is Keyhold's JSON with notes.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub fields: Vec<Field>,
    pub icon: Option<Icon>,
    pub notes: String,
}

/// What a record's value becomes, and whether that is Keyhold's JSON.
///
/// Plain only when there is nothing a plain value could not carry back: exactly one field, of the
/// kind a plain value reads back as (a password), with no composite parts, no marks, no icon and
/// no notes. The single field's label is not kept in that case; it reads back as "Valor". Anything
/// else is JSON, so nothing the person set up is lost on the round trip, and notes always have
/// somewhere encrypted to go instead of the description.
pub fn encode_value(payload: &Payload) -> Result<(String, bool), String> {
    let notes = payload.notes.trim_end();

    let plain = match payload.fields.as_slice() {
        [only] => {
            only.kind == PLAIN_KIND
                && only.extra.is_empty()
                && !only.list
                && !only.primary
                && !only.value.is_empty()
                && payload.icon.is_none()
                && notes.is_empty()
        }
        _ => false,
    };

    let value = if plain {
        payload.fields[0].value.clone()
    } else {
        serde_json::to_string(&Stored {
            keyhold: 1,
            fields: payload.fields.clone(),
            icon: payload.icon.clone(),
            notes: notes.to_string(),
        })
        .map_err(|e| e.to_string())?
    };

    if value.len() > ADVANCED_LIMIT {
        return Err(format!(
            "na AWS um parâmetro guarda até {} KB, e este registro (campos e notas) tem {:.1} KB. \
             Guarde o que é grande num anexo",
            ADVANCED_LIMIT / 1024,
            value.len() as f64 / 1024.0
        ));
    }

    Ok((value, !plain))
}

/// The fields, icon and notes a value holds.
///
/// `tagged_json` is what the parameter's tags say: `Some(true)` for Keyhold's tag, `Some(false)`
/// for tags read and no tag there, `None` when the tags could not be read. Only the first and the
/// last let a value be read as Keyhold's JSON, and only when it really is Keyhold's shape; anything
/// else is one plain field, which is how an application's own JSON config stays readable.
pub fn decode_value(value: &str, tagged_json: Option<bool>) -> Decoded {
    if tagged_json != Some(false) {
        if let Ok(stored) = serde_json::from_str::<Stored>(value) {
            if stored.keyhold == 1 {
                return Decoded { fields: stored.fields, icon: stored.icon, notes: stored.notes };
            }
        }
    }

    Decoded {
        fields: vec![Field::new(PLAIN_LABEL, PLAIN_KIND, value)],
        icon: None,
        notes: String::new(),
    }
}

/// A whole payload from what Parameter Store keeps.
///
/// Notes come from the value when it has them; otherwise from a description written the old way,
/// with notes after the title, so nothing written before the move is lost on the way in (and the
/// next save carries them into the value).
pub fn payload_from(
    name: &str,
    description: Option<&str>,
    value: &str,
    tagged_json: Option<bool>,
) -> Payload {
    let (title, legacy_notes) = split_description(description, name);
    let decoded = decode_value(value, tagged_json);
    let notes = if decoded.notes.is_empty() { legacy_notes } else { decoded.notes };

    Payload { title, notes, fields: decoded.fields, icon: decoded.icon, ..Payload::default() }
}

// ── folders ─────────────────────────────────────────────────────────────────────────────────────

/// Every folder implied by a set of names: each prefix ending before a `/`.
pub fn folder_paths<'a>(names: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();

    for name in names {
        let mut current = parent_path(name);

        while let Some(path) = current {
            current = parent_path(&path);
            paths.insert(path);
        }
    }

    paths
}

/// The folder tree a vault shows, in the same shape local folders have.
pub fn folder_nodes<'a>(vault_id: &str, names: impl IntoIterator<Item = &'a str>) -> Vec<FolderNode> {
    folder_paths(names)
        .into_iter()
        .map(|path| FolderNode {
            id: folder_id(vault_id, &path),
            vault_id: vault_id.to_string(),
            parent_id: parent_path(&path).map(|parent| folder_id(vault_id, &parent)),
            name: leaf(&path).to_string(),
            colour: None,
            depth: path.matches('/').count().saturating_sub(1),
        })
        .collect()
}

/// Whether anything other than the folder's own marker lives at or below `path`.
pub fn folder_has_contents<'a>(path: &str, names: impl IntoIterator<Item = &'a str>) -> bool {
    let own_marker = join(Some(path), KEEP);
    let prefix = format!("{}/", path.trim_end_matches('/'));

    names
        .into_iter()
        .map(absolute)
        .any(|name| name.starts_with(&prefix) && name != own_marker)
}

// ── keys ────────────────────────────────────────────────────────────────────────────────────────

/// The KMS key a new parameter in `folder` should be written with.
///
/// The key of a parameter in the same folder if there is one (its `.keep` included), otherwise of
/// the nearest folder above it that has one. `None` means the account's default, which is also
/// what an inherited `alias/aws/ssm` comes down to. `known` is every name already read, with the
/// key its metadata reported.
pub fn inherit_key(folder: Option<&str>, known: &[(String, Option<String>)]) -> Option<String> {
    let mut by_folder: BTreeMap<Option<String>, Vec<&str>> = BTreeMap::new();

    for (name, key) in known {
        if let Some(key) = key.as_deref().filter(|k| !k.is_empty()) {
            by_folder.entry(parent_path(name)).or_default().push(key);
        }
    }

    let mut current = folder.map(|f| absolute(f.trim_end_matches('/')));

    loop {
        if let Some(keys) = by_folder.get(&current) {
            // The most common key in the folder, ties broken by name, so the answer does not
            // depend on the order AWS listed things in.
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for key in keys {
                *counts.entry(key).or_default() += 1;
            }
            let chosen = counts
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
                .map(|(key, _)| key.to_string());

            return chosen.filter(|key| key != DEFAULT_SSM_KEY);
        }

        current = parent_path(&current?);
    }
}

// ── dates ───────────────────────────────────────────────────────────────────────────────────────

/// Unix seconds as `YYYY-MM-DDTHH:MM:SSZ`, the shape every date in the interface already has.
pub fn iso_from_unix(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);

    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Howard Hinnant's days-to-civil.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;

    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A rotation date as the tag keeps it: `YYYY-MM-DD`, from whatever date-ish string came in.
pub fn rotation_date(due: &str) -> Result<String, String> {
    let day = due.get(..10).unwrap_or(due);
    let ok = day.len() == 10
        && day.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        });

    if ok {
        Ok(day.to_string())
    } else {
        Err("a data de troca precisa estar no formato AAAA-MM-DD".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(label: &str, kind: &str, value: &str) -> Field {
        Field::new(label, kind, value)
    }

    fn payload(fields: Vec<Field>) -> Payload {
        Payload { title: "Banco".into(), fields, ..Payload::default() }
    }

    #[test]
    fn an_id_says_which_vault_and_which_parameter_and_comes_back_whole() {
        let id = record_id("VAULT", "/app/prod/senha do banco+ç");
        assert!(is_aws_id(&id));
        assert_eq!(
            decode_id(&id),
            Some((IdKind::Record, "VAULT".to_string(), "/app/prod/senha do banco+ç".to_string()))
        );
        assert_eq!(decode_id(&folder_id("V", "/app")).unwrap().0, IdKind::Folder);
        assert_ne!(record_id("V", "/app"), folder_id("V", "/app"), "the kind is part of the id");
    }

    #[test]
    fn an_attachment_id_is_one_file_directly_under_a_record() {
        let id = attachment_id("V", "app/prod/db/chave.pem");
        assert_eq!(decode_id(&id).unwrap().2, "app/prod/db/chave.pem");
        assert_eq!(attachment_record("app/prod/db/chave.pem").as_deref(), Some("/app/prod/db"));

        for bad in ["/app/db/x", "x", "app/db/", "app/db/..", "app/.keep/x", "aws/db/x", "a b/x"] {
            assert!(decode_id(&attachment_id("V", bad)).is_none(), "{bad}");
        }

        // `db` and `/db` are two parameters; only the one with the slash has a prefix, so their
        // files can never be mistaken for each other's.
        assert_eq!(attachment_prefix("/db").as_deref(), Some("db/"));
        assert_eq!(attachment_prefix("db"), None);
        assert!(is_attachment_of("app/db/", "app/db/a.pdf"));
        assert!(!is_attachment_of("app/db/", "app/db/nested/a.pdf"));
        assert!(!is_attachment_of("app/db/", "app/db/"));
    }

    #[test]
    fn a_local_id_is_not_mistaken_for_an_aws_one() {
        assert!(!is_aws_id("ABCDEFGHIJKLMNOP"));
        assert_eq!(decode_id("ABCDEFGHIJKLMNOP"), None);
        assert_eq!(decode_id("aws~~"), None);
        assert_eq!(decode_id("aws~VAULT"), None);
    }

    #[test]
    fn a_name_is_suggested_from_the_title_in_lowercase_ascii_with_hyphens() {
        assert_eq!(suggest_name("Senha do Banco (produção)"), "senha-do-banco-producao");
        assert_eq!(suggest_name("  API key: Stripe!! "), "api-key-stripe");
        assert_eq!(suggest_name("ção"), "cao");
        assert_eq!(suggest_name("///"), "registro");
        assert_eq!(suggest_name("db_password.v2"), "db_password.v2");
        assert!(validate_segment(&suggest_name("Qualquer título, com vírgula")).is_ok());
    }

    #[test]
    fn only_what_aws_refuses_is_refused_in_a_name() {
        assert!(validate_name("/app/prod/db-password").is_ok());
        assert!(validate_name("db_password.v2").is_ok());
        assert!(validate_name("/app/senha com espaço").is_err());
        assert!(validate_name("/aws/qualquer").is_err());
        assert!(validate_name("/SSMx/qualquer").is_err());
        assert!(validate_name("/app/.keep").is_err());
        let deep = "/a".repeat(16);
        assert!(validate_name(&deep).is_err());
    }

    #[test]
    fn the_description_is_the_title_on_the_first_line_and_the_notes_after() {
        assert_eq!(
            split_description(Some("Banco\nconta da empresa\nsegunda linha"), "/x/banco"),
            ("Banco".to_string(), "conta da empresa\nsegunda linha".to_string())
        );
        assert_eq!(
            split_description(Some("Só o título"), "/x/y"),
            ("Só o título".to_string(), String::new())
        );
        assert_eq!(
            split_description(None, "/app/prod/db-password"),
            ("db-password".to_string(), String::new())
        );
        assert_eq!(
            split_description(Some("   "), "/app/db"),
            ("db".to_string(), String::new())
        );

        assert_eq!(description_for(" Banco\nda empresa ").unwrap(), "Banco da empresa");
    }

    #[test]
    fn notes_go_into_the_encrypted_value_never_into_the_description() {
        let original = Payload {
            notes: "PIN do backup: 4411".into(),
            ..payload(vec![field("Senha", "password", "hunter2")])
        };

        let description = description_for(&original.title).unwrap();
        assert!(!description.contains("4411"));

        let (value, json) = encode_value(&original).unwrap();
        assert!(json, "a record with notes is always Keyhold's JSON");
        assert!(value.contains("4411"));

        let back = payload_from("/app/db", Some(&description), &value, Some(true));
        assert_eq!(back.notes, "PIN do backup: 4411");
        assert_eq!(back.fields, original.fields);

        // Written the old way: notes after the title in the description. Still read.
        let old = payload_from("/app/db", Some("Banco\nnota antiga"), "hunter2", Some(false));
        assert_eq!(old.notes, "nota antiga");
        assert_eq!(old.title, "Banco");
    }

    #[test]
    fn a_title_past_the_limit_is_refused_in_words() {
        let error = description_for(&"x".repeat(DESCRIPTION_LIMIT + 1)).unwrap_err();
        assert!(error.contains("1024"), "{error}");
        assert!(!error.contains('—'));
    }

    #[test]
    fn a_title_with_a_slash_is_refused() {
        assert!(description_for("a/b").is_err());
        assert!(validate_title("").is_err());
    }

    #[test]
    fn one_password_is_stored_as_the_plain_value() {
        let (value, json) = encode_value(&payload(vec![field("Senha", "password", "hunter2")])).unwrap();
        assert_eq!(value, "hunter2");
        assert!(!json);

        let back = payload_from("/app/db", Some("Banco"), &value, Some(false));
        assert_eq!(back.title, "Banco");
        assert_eq!(back.fields, vec![field(PLAIN_LABEL, PLAIN_KIND, "hunter2")]);
    }

    #[test]
    fn several_fields_an_icon_or_a_mark_switch_to_json_and_come_back_intact() {
        let mut card = field("Cartão", "card", "4111111111111111");
        card.extra.insert("cvv".into(), "123".into());

        let cases = vec![
            payload(vec![field("Usuário", "text", "ana"), field("Senha", "password", "x")]),
            payload(vec![card]),
            Payload {
                icon: Some(Icon { name: "key".into(), colour: "ambar".into(), image: None }),
                ..payload(vec![field("Senha", "password", "x")])
            },
            payload(vec![Field { primary: true, ..field("Senha", "password", "x") }]),
            payload(vec![field("Código", "totp", "JBSWY3DPEHPK3PXP")]),
        ];

        for original in cases {
            let (value, json) = encode_value(&original).unwrap();
            assert!(json, "{value}");
            assert!(value.starts_with("{\"keyhold\":1,"), "{value}");

            let decoded = decode_value(&value, Some(true));
            assert_eq!(decoded.fields, original.fields);
            assert_eq!(decoded.icon, original.icon);
            assert!(!value.contains("\"notes\""), "no notes, no key: {value}");
        }
    }

    #[test]
    fn an_applications_own_json_is_shown_as_a_plain_value() {
        let config = r#"{"host":"db","password":"x"}"#;
        let fields = decode_value(config, Some(false)).fields;
        assert_eq!(fields, vec![field(PLAIN_LABEL, PLAIN_KIND, config)]);

        // Even Keyhold's shape, without Keyhold's tag, is somebody else's value.
        let ours = r#"{"keyhold":1,"fields":[]}"#;
        assert_eq!(decode_value(ours, Some(false)).fields.len(), 1);
        assert_eq!(decode_value(ours, Some(false)).fields[0].value, ours);

        // Tags unreadable: only Keyhold's exact shape is read as Keyhold's.
        assert!(decode_value(ours, None).fields.is_empty());
        assert_eq!(decode_value(config, None).fields[0].value, config);
    }

    #[test]
    fn a_value_too_large_for_aws_is_refused_before_it_is_sent() {
        let big = payload(vec![field("Senha", "password", &"x".repeat(ADVANCED_LIMIT + 1))]);
        assert!(encode_value(&big).is_err());
    }

    #[test]
    fn every_slash_is_a_folder_and_markers_make_empty_ones_exist() {
        let names = ["/app/prod/db", "/app/prod/api", "/app/dev/.keep", "/solto", "sem-barra"];
        let nodes = folder_nodes("V", names);

        let shown: Vec<(String, usize, Option<String>)> = nodes
            .iter()
            .map(|n| {
                (
                    decode_id(&n.id).unwrap().2,
                    n.depth,
                    n.parent_id.as_deref().map(|p| decode_id(p).unwrap().2),
                )
            })
            .collect();

        assert_eq!(
            shown,
            vec![
                ("/app".to_string(), 0, None),
                ("/app/dev".to_string(), 1, Some("/app".to_string())),
                ("/app/prod".to_string(), 1, Some("/app".to_string())),
            ]
        );
        assert_eq!(nodes[1].name, "dev");
        assert!(nodes.iter().all(|n| n.vault_id == "V" && n.colour.is_none()));
    }

    #[test]
    fn a_folder_with_only_its_marker_is_empty() {
        let names = ["/app/dev/.keep", "/app/prod/db"];
        assert!(!folder_has_contents("/app/dev", names));
        assert!(folder_has_contents("/app/prod", names));
        assert!(folder_has_contents("/app", names));
        assert!(!folder_has_contents("/ap", names));
    }

    #[test]
    fn the_key_comes_from_the_folder_or_the_nearest_one_above() {
        let known = vec![
            ("/app/.keep".to_string(), Some("alias/app".to_string())),
            ("/app/prod/db".to_string(), Some("alias/prod".to_string())),
            ("/app/prod/api".to_string(), Some("alias/prod".to_string())),
            ("/app/prod/old".to_string(), Some("alias/legacy".to_string())),
            ("/outra/x".to_string(), Some(DEFAULT_SSM_KEY.to_string())),
            ("/solto".to_string(), Some("alias/root".to_string())),
        ];

        assert_eq!(inherit_key(Some("/app/prod"), &known), Some("alias/prod".into()));
        assert_eq!(inherit_key(Some("/app/dev"), &known), Some("alias/app".into()));
        assert_eq!(inherit_key(Some("/app/dev/fundo"), &known), Some("alias/app".into()));
        assert_eq!(inherit_key(Some("/outra"), &known), None);
        assert_eq!(inherit_key(Some("/nova"), &known), Some("alias/root".into()));
        assert_eq!(inherit_key(None, &[]), None);
    }

    #[test]
    fn paths_split_and_join_the_way_aws_reads_them() {
        assert_eq!(parent_path("/app/prod/db"), Some("/app/prod".into()));
        assert_eq!(parent_path("/db"), None);
        assert_eq!(parent_path("db"), None);
        assert_eq!(leaf("/app/prod/db"), "db");
        assert_eq!(join(Some("/app/"), "db"), "/app/db");
        assert_eq!(join(None, "db"), "/db");
        assert!(is_keep("/app/.keep"));
    }

    #[test]
    fn dates_come_out_in_the_shape_the_interface_uses() {
        assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_unix(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso_from_unix(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rotation_date("2026-12-01").unwrap(), "2026-12-01");
        assert_eq!(rotation_date("2026-12-01T00:00:00Z").unwrap(), "2026-12-01");
        assert!(rotation_date("amanhã").is_err());
    }
}
