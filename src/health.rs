//! What is wrong with the passwords in a vault, said in words somebody can act on.
//!
//! **This deliberately does not score entropy.** `log2(alphabet) × length` is exactly right for a
//! password the generator produced and a lie about one a person invented: "Verao2024!" scores like
//! ten random characters and is among the first guesses any real attack makes. A number nobody can
//! act on is worse than no number, because it feels like an answer.
//!
//! So instead of a score, this reports **named, checkable reasons**: too short, one class of
//! character, a shape that reads as a word with a year stuck on it, a value already used somewhere
//! else in this vault, one that repeats the record's own title or username, or one that has simply
//! been sitting there long enough to be worth changing. Every finding is something a person can
//! look at and agree with — which is the bar for a screen whose only purpose is to get somebody to
//! change a password they are attached to.
//!
//! It runs where the keys are, because judging a password means reading it. Nothing here is written
//! down: the findings are computed on request and handed to the interface that asked.

use anyhow::Result;
use serde::Serialize;

use crate::crypto::SecretKey;
use crate::db::Store;
use crate::records::{self, Payload};

/// Below this, length has stopped carrying the password.
const SHORT_LENGTH: usize = 12;

/// After this long without a change, a password is old enough to be worth revisiting.
pub const STALE_DAYS: i64 = 365;

/// The shapes that make a password predictable regardless of its length.
///
/// Not a dictionary — a dictionary in the binary would be a large file that still misses the
/// password in front of it. These are the *patterns* that describe most human passwords.
fn predictable(value: &str) -> Option<&'static str> {
    let lower = value.to_lowercase();

    // Ordered from the most specific description to the least. A run of letters contains "abcd",
    // so checking the common-sequence list first would report every all-letter password as a
    // keyboard run — true, and less useful than saying it has no digits or symbols at all.
    if value.chars().all(|c| c.is_ascii_digit()) {
        return Some("só números");
    }

    if value.chars().all(|c| c.is_alphabetic()) {
        return Some("só letras");
    }

    for common in ["senha", "password", "qwerty", "asdf", "zxcv", "1234", "admin", "abcd"] {
        if lower.contains(common) {
            return Some("contém uma sequência comum");
        }
    }

    // A word, then a short run of digits, then maybe one punctuation mark: the shape of most
    // passwords people invent when a site demands a number.
    let trimmed = value.trim_end_matches(|c: char| "!@#$%*.".contains(c));
    let digits_at_end = trimmed.chars().rev().take_while(char::is_ascii_digit).count();

    if (1..=4).contains(&digits_at_end) {
        let head = &trimmed[..trimmed.len() - digits_at_end];

        if !head.is_empty() && head.chars().all(char::is_alphabetic) {
            return Some("uma palavra com números no fim");
        }
    }

    None
}

fn classes(value: &str) -> usize {
    let lower = value.chars().any(|c| c.is_lowercase());
    let upper = value.chars().any(|c| c.is_uppercase());
    let digit = value.chars().any(|c| c.is_ascii_digit());
    let other = value.chars().any(|c| !c.is_alphanumeric());

    [lower, upper, digit, other].iter().filter(|on| **on).count()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Issue {
    Compromised,
    Rotation,
    Reused,
    Weak,
    Stale,
}

impl Issue {
    /// Worst first. The order decides what a list sorted by severity shows at the top.
    fn severity(self) -> u8 {
        match self {
            Issue::Compromised => 0,
            Issue::Rotation => 1,
            Issue::Reused => 2,
            Issue::Weak => 3,
            Issue::Stale => 4,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Finding {
    pub record_id: String,
    pub title: String,
    pub folder_id: Option<String>,
    pub issues: Vec<Issue>,
    /// Said in the person's own words, ready to render. One line per issue.
    pub reasons: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct VaultHealth {
    pub findings: Vec<Finding>,
    /// How many records were looked at, so "nothing wrong" can be told apart from "nothing here".
    pub examined: usize,
}

/// Which fields of a record count as a password worth judging.
fn secrets_of(payload: &Payload) -> Vec<&str> {
    payload
        .fields
        .iter()
        .filter(|field| field.kind == "password")
        .map(|field| field.value.as_str())
        .filter(|value| !value.is_empty())
        .collect()
}

pub fn examine(store: &Store, master: &SecretKey, vault_id: &str) -> Result<VaultHealth> {
    let headers = records::list(store, vault_id, false)?;
    let mut opened = Vec::new();

    for header in headers {
        if let Ok(record) = records::open(store, master, &header.id) {
            opened.push(record);
        }
    }

    // Reuse is a property of the vault, not of a record, so it needs one pass over everything
    // before any single record can be judged.
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();

    for record in &opened {
        for secret in secrets_of(&record.payload) {
            *counts.entry(secret).or_default() += 1;
        }
    }

    let now = crate::identity::now_seconds() as i64;
    let mut findings = Vec::new();

    for record in &opened {
        let mut issues = Vec::new();
        let mut reasons = Vec::new();

        if record.header.compromised_at.is_some() {
            issues.push(Issue::Compromised);
            reasons.push("Você marcou este segredo como comprometido.".to_string());
        }

        if let Some(due) = &record.header.rotation_due_at {
            if parse_date(due).map(|when| when <= now).unwrap_or(false) {
                issues.push(Issue::Rotation);
                reasons.push("A data que você marcou para trocar já passou.".to_string());
            }
        }

        for secret in secrets_of(&record.payload) {
            if counts.get(secret).copied().unwrap_or(0) > 1 {
                push_once(&mut issues, &mut reasons, Issue::Reused,
                    "Esta senha está repetida em outro registro deste cofre.".to_string());
            }

            if secret.chars().count() < SHORT_LENGTH {
                push_once(&mut issues, &mut reasons, Issue::Weak,
                    format!("Tem menos de {SHORT_LENGTH} caracteres."));
            }

            if classes(secret) == 1 {
                push_once(&mut issues, &mut reasons, Issue::Weak,
                    "Usa um só tipo de caractere.".to_string());
            }

            if let Some(shape) = predictable(secret) {
                push_once(&mut issues, &mut reasons, Issue::Weak, format!("É {shape}."));
            }

            let title = record.payload.title.to_lowercase();
            let username = record.payload.subtitle().to_lowercase();
            let lower = secret.to_lowercase();

            if (!title.is_empty() && lower.contains(&title))
                || (!username.is_empty() && lower.contains(&username))
            {
                push_once(&mut issues, &mut reasons, Issue::Weak,
                    "Repete o título ou o usuário do próprio registro.".to_string());
            }
        }

        if !secrets_of(&record.payload).is_empty() {
            if let Some(changed) = parse_date(&record.header.updated_at) {
                let days = (now - changed) / 86_400;

                if days >= STALE_DAYS {
                    issues.push(Issue::Stale);
                    reasons.push(format!("Não muda há {days} dias."));
                }
            }
        }

        if issues.is_empty() {
            continue;
        }

        issues.sort_by_key(|issue| issue.severity());

        findings.push(Finding {
            record_id: record.header.id.clone(),
            title: record.payload.title.clone(),
            folder_id: record.header.folder_id.clone(),
            issues,
            reasons,
        });
    }

    // Worst first, so the screen opens on what matters.
    findings.sort_by_key(|finding| {
        finding.issues.first().map(|issue| issue.severity()).unwrap_or(u8::MAX)
    });

    Ok(VaultHealth { findings, examined: opened.len() })
}

fn push_once(issues: &mut Vec<Issue>, reasons: &mut Vec<String>, issue: Issue, reason: String) {
    if !issues.contains(&issue) {
        issues.push(issue);
    }

    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

/// Parse the `YYYY-MM-DDTHH:MM:SSZ` this codebase writes, into unix seconds.
///
/// Written out rather than pulled in: it is one format, produced by one place, and a date library
/// in this binary would be a dependency that never earns its keep.
fn parse_date(value: &str) -> Option<i64> {
    let date = value.get(..10)?;
    let mut parts = date.split('-');

    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;

    let time = value.get(11..19).unwrap_or("00:00:00");
    let mut clock = time.split(':');

    let hour: i64 = clock.next()?.parse().unwrap_or(0);
    let minute: i64 = clock.next()?.parse().unwrap_or(0);
    let second: i64 = clock.next()?.parse().unwrap_or(0);

    // Days since the epoch, by the civil-from-days algorithm (Howard Hinnant's), which is exact for
    // every date this will ever see and needs no table.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{self, SetupOptions};
    use crate::records::Field;
    use crate::vaults::{self, VaultMeta};

    fn scratch() -> (tempfile::TempDir, Store, SecretKey, String) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("vault.db")).unwrap();

        let outcome =
            identity::create(&mut store, SetupOptions { password: Some("the password".into()) })
                .unwrap();

        let vault = vaults::create_vault(
            &store,
            &outcome.vault_key,
            VaultMeta { name: "Trabalho".into(), description: String::new(), colour: "brand".into() },
        )
        .unwrap();

        (dir, store, outcome.vault_key, vault.id)
    }

    fn with_password(title: &str, password: &str) -> Payload {
        Payload {
            title: title.into(),
            fields: vec![Field {
                label: "Senha".into(),
                kind: "password".into(),
                value: password.into(),
                ..Default::default()
            }],
            ..Payload::default()
        }
    }

    fn add(store: &Store, master: &SecretKey, vault: &str, title: &str, password: &str) -> String {
        records::create(store, master, vault, None, "login", &with_password(title, password))
            .unwrap()
            .id
    }

    #[test]
    fn a_strong_unique_password_has_nothing_said_about_it() {
        let (_dir, store, master, vault) = scratch();
        add(&store, &master, &vault, "Servidor", "9k2Lm4pQz7XwTb1r!Vd");

        let health = examine(&store, &master, &vault).unwrap();

        assert_eq!(health.examined, 1);
        assert!(health.findings.is_empty(), "{:?}", health.findings);
    }

    #[test]
    fn a_short_password_is_reported() {
        let (_dir, store, master, vault) = scratch();
        add(&store, &master, &vault, "Servidor", "Ab1!x");

        let health = examine(&store, &master, &vault).unwrap();

        assert!(health.findings[0].issues.contains(&Issue::Weak));
        assert!(health.findings[0].reasons.iter().any(|r| r.contains("menos de 12")));
    }

    #[test]
    fn the_same_password_in_two_records_is_reported_in_both() {
        let (_dir, store, master, vault) = scratch();
        add(&store, &master, &vault, "Um", "9k2Lm4pQz7XwTb1r!Vd");
        add(&store, &master, &vault, "Dois", "9k2Lm4pQz7XwTb1r!Vd");

        let health = examine(&store, &master, &vault).unwrap();

        assert_eq!(health.findings.len(), 2);
        assert!(health.findings.iter().all(|f| f.issues.contains(&Issue::Reused)));
    }

    #[test]
    fn the_shapes_people_actually_use_are_caught() {
        let (_dir, store, master, vault) = scratch();

        for (title, password, expected) in [
            ("Ano", "Verao2024!", "palavra com números"),
            ("Digitos", "839201847362", "só números"),
            ("Letras", "abcdefghijklm", "só letras"),
            ("Comum", "MinhaSenha123456", "sequência comum"),
        ] {
            add(&store, &master, &vault, title, password);

            let health = examine(&store, &master, &vault).unwrap();
            let found = health.findings.iter().find(|f| f.title == title).unwrap();

            assert!(
                found.reasons.iter().any(|r| r.contains(expected)),
                "{title} ({password}) → {:?}",
                found.reasons
            );
        }
    }

    #[test]
    fn a_password_that_repeats_the_record_is_reported() {
        let (_dir, store, master, vault) = scratch();
        add(&store, &master, &vault, "Contabil", "Contabil#2291Xk");

        let health = examine(&store, &master, &vault).unwrap();

        assert!(health.findings[0].reasons.iter().any(|r| r.contains("Repete o título")));
    }

    #[test]
    fn a_record_marked_compromised_comes_first() {
        let (_dir, store, master, vault) = scratch();
        add(&store, &master, &vault, "Fraca", "abc");
        let bad = add(&store, &master, &vault, "Vazada", "9k2Lm4pQz7XwTb1r!Vd");

        records::set_compromised(&store, &bad, true).unwrap();

        let health = examine(&store, &master, &vault).unwrap();

        assert_eq!(health.findings[0].title, "Vazada");
        assert_eq!(health.findings[0].issues[0], Issue::Compromised);
    }

    #[test]
    fn an_overdue_rotation_is_reported() {
        let (_dir, store, master, vault) = scratch();
        let id = add(&store, &master, &vault, "Servidor", "9k2Lm4pQz7XwTb1r!Vd");

        records::set_rotation_due(&store, &id, Some("2020-01-01T00:00:00Z")).unwrap();

        let health = examine(&store, &master, &vault).unwrap();

        assert!(health.findings[0].issues.contains(&Issue::Rotation));
    }

    #[test]
    fn a_rotation_still_in_the_future_says_nothing() {
        let (_dir, store, master, vault) = scratch();
        let id = add(&store, &master, &vault, "Servidor", "9k2Lm4pQz7XwTb1r!Vd");

        records::set_rotation_due(&store, &id, Some("2999-01-01T00:00:00Z")).unwrap();

        assert!(examine(&store, &master, &vault).unwrap().findings.is_empty());
    }

    #[test]
    fn a_record_with_no_password_is_not_judged() {
        let (_dir, store, master, vault) = scratch();

        records::create(
            &store,
            &master,
            &vault,
            None,
            "note",
            &Payload { title: "Uma nota".into(), ..Payload::default() },
        )
        .unwrap();

        let health = examine(&store, &master, &vault).unwrap();

        assert_eq!(health.examined, 1);
        assert!(health.findings.is_empty());
    }

    #[test]
    fn dates_parse_the_way_this_codebase_writes_them() {
        // Known values: the epoch, and a date this project will actually see.
        assert_eq!(parse_date("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_date("2026-08-31T12:00:00Z"), Some(1_788_177_600));
        assert_eq!(parse_date("not a date"), None);
    }
}
