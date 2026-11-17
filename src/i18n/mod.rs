//! Portuguese and English for everything Keyhold says.
//!
//! Portuguese is the source: every message is written in Portuguese in the code, inside `tr!`, and
//! English comes from a catalog keyed by that Portuguese text. A message with values names them in
//! braces (`"o log group {group} não existe"`) and `tr!` fills them in either language. A test reads
//! the source and fails on any `tr!` without an English line, so a new message cannot ship in one
//! language only.
//!
//! The language is the machine's, not the vault's: it is a plain setting read before unlocking (the
//! lock screen speaks it too), held here in a static once known.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

mod en;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Pt,
    En,
}

impl Lang {
    /// `en`, `en-US` and the like are English; anything else is Portuguese.
    pub fn from_tag(tag: &str) -> Self {
        if tag.trim().to_ascii_lowercase().starts_with("en") {
            Lang::En
        } else {
            Lang::Pt
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Lang::Pt => "pt-BR",
            Lang::En => "en",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    CURRENT.store(if lang == Lang::En { 1 } else { 0 }, Ordering::Relaxed);
}

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == 1 {
        Lang::En
    } else {
        Lang::Pt
    }
}

fn catalog() -> &'static HashMap<&'static str, &'static str> {
    static CATALOG: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    CATALOG.get_or_init(|| en::ENTRIES.iter().flat_map(|part| part.iter()).copied().collect())
}

/// The English line for a Portuguese template, if the catalog has one.
pub fn english(template: &str) -> Option<&'static str> {
    catalog().get(template).copied()
}

/// `template` in the current language, with `{name}` replaced by each value.
pub fn tr(template: &'static str, values: &[(&str, &dyn Display)]) -> String {
    let chosen = match current() {
        Lang::Pt => template,
        Lang::En => english(template).unwrap_or(template),
    };

    let mut out = chosen.to_string();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

/// `tr!("texto")` or `tr!("o grupo {group} não existe", group = name)`.
#[macro_export]
macro_rules! tr {
    ($template:literal) => {
        $crate::i18n::tr($template, &[])
    };
    ($template:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::tr($template, &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `tr!("…")` in the source has an English line, and every placeholder survives
    /// translation.
    #[test]
    fn every_message_has_an_english_line() {
        fn walk(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, found);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    found.push(path);
                }
            }
        }

        let mut files = Vec::new();
        walk(&std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);

        let mut missing = Vec::new();
        for file in files {
            if file.ends_with("i18n/mod.rs") {
                continue;
            }
            // Comments name messages too; only code counts. `tr!(` may be followed by a line break.
            let text: String = std::fs::read_to_string(&file)
                .unwrap()
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            let text = text.replace("tr!(\n", "tr!(");
            let compact: String = {
                let mut out = String::with_capacity(text.len());
                let mut chars = text.chars().peekable();
                while let Some(c) = chars.next() {
                    out.push(c);
                    if out.ends_with("tr!(") {
                        while chars.peek().is_some_and(|c| c.is_whitespace()) {
                            chars.next();
                        }
                    }
                }
                out
            };
            let mut rest = compact.as_str();
            while let Some(at) = rest.find("tr!(\"") {
                let after = &rest[at + 5..];
                // `include_str!("…")` ends in `tr!(` too: only a `tr!` that starts a word counts.
                let glued = rest[..at].chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_');
                let Some(end) = closing_quote(after) else { break };
                if glued {
                    rest = &after[end..];
                    continue;
                }
                let template = after[..end].replace("\\\"", "\"").replace("\\\\", "\\");
                match english(&template) {
                    None => missing.push(format!("{}: {template}", file.display())),
                    Some(line) => {
                        for name in placeholders(&template) {
                            assert!(line.contains(&format!("{{{name}}}")), "{{{name}}} is lost in: {line}");
                        }
                    }
                }
                rest = &after[end..];
            }
        }

        assert!(missing.is_empty(), "messages without English:\n{}", missing.join("\n"));
    }

    fn closing_quote(text: &str) -> Option<usize> {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => i += 2,
                b'"' => return Some(i),
                _ => i += 1,
            }
        }
        None
    }

    fn placeholders(template: &str) -> Vec<&str> {
        template
            .split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(name, _)| name))
            .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            .collect()
    }

    #[test]
    fn values_fill_either_language() {
        set(Lang::Pt);
        assert_eq!(tr!("o cofre não existe mais"), "o cofre não existe mais");
        set(Lang::En);
        assert_eq!(tr!("o cofre não existe mais"), "the vault no longer exists");
        set(Lang::Pt);
    }
}
