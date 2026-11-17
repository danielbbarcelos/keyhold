//! Sharing one record with somebody outside: a link to a page in the vault's own bucket (AWS
//! vaults) or a password-protected ZIP (local vaults). See `docs/AWS.md` and the threat model.
//!
//! What may go is decided here, once, for both: the fields the person ticked and, if ticked, the
//! notes. A 2FA seed never goes (it would hand over the second factor for good, past any expiry),
//! and neither do attachments.

pub mod crypto;
pub mod zip;

use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::records::Payload;
use crate::tr;

/// The recipient's page, the same bytes for every share: nothing about a record is in it. Its
/// inline script is allowed by hash, computed here so the page and its policy cannot drift apart.
pub fn recipient_page() -> String {
    const TEMPLATE: &str = include_str!("recipient.html");
    const ICON_DARK: &[u8] = include_bytes!("../../web/public/brand/keyhold-icon-dark.png");
    const ICON_LIGHT: &[u8] = include_bytes!("../../web/public/brand/keyhold-icon-light.png");

    let start = TEMPLATE.find("<script>").expect("the page has a script") + "<script>".len();
    let end = TEMPLATE.rfind("</script>").expect("the page has a script");
    let hash = crate::encoding::base64_standard(&Sha256::digest(&TEMPLATE.as_bytes()[start..end]));

    TEMPLATE
        .replace("{{SCRIPT_HASH}}", &hash)
        .replace("{{ICON_DARK}}", &crate::encoding::base64_standard(ICON_DARK))
        .replace("{{ICON_LIGHT}}", &crate::encoding::base64_standard(ICON_LIGHT))
}

/// What the person chose to send.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct Choice {
    /// Indices of the record's fields.
    #[serde(default)]
    pub fields: Vec<usize>,
    #[serde(default)]
    pub notes: bool,
}

/// One field as it travels: never a 2FA seed.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SharedField {
    pub label: String,
    pub kind: String,
    pub value: String,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub extra: std::collections::BTreeMap<String, String>,
}

/// The content of a share: what the person ticked, minus what never goes.
pub fn content(payload: &Payload, choice: &Choice) -> Result<(Vec<SharedField>, Option<String>, Vec<String>)> {
    let mut fields = Vec::new();
    let mut left_out = Vec::new();

    for (index, field) in payload.fields.iter().enumerate() {
        let ticked = choice.fields.contains(&index);
        if field.kind == "totp" {
            if ticked {
                left_out.push(field.label.clone());
            }
            continue;
        }
        if ticked {
            fields.push(SharedField {
                label: field.label.clone(),
                kind: field.kind.clone(),
                value: field.value.clone(),
                extra: field.extra.clone(),
            });
        }
    }

    let notes = (choice.notes && !payload.notes.trim().is_empty()).then(|| payload.notes.clone());

    if fields.is_empty() && notes.is_none() {
        bail!(tr!("escolha ao menos um campo para compartilhar"));
    }

    Ok((fields, notes, left_out))
}

/// The content as the page decrypts it.
pub fn content_json(fields: &[SharedField], notes: Option<&str>) -> Vec<u8> {
    json!({ "fields": fields, "notes": notes }).to_string().into_bytes()
}

/// A marker the page may POST: where, and the signed form fields.
#[derive(Clone, Debug, Serialize)]
pub struct Marker {
    pub url: String,
    pub fields: serde_json::Map<String, Value>,
}

impl Marker {
    pub fn new(url: String, fields: Vec<(String, String)>) -> Self {
        Self { url, fields: fields.into_iter().map(|(k, v)| (k, Value::String(v))).collect() }
    }
}

/// Everything the page needs before "Revelar", sealed under the link key.
#[derive(Clone, Debug, Serialize)]
pub struct Envelope {
    pub v: u8,
    pub title: String,
    pub sender: String,
    pub expires_at: u64,
    pub first: bool,
    /// `{ salt, iterations }` when the content also needs the opening password.
    pub password: Option<Value>,
    pub content: String,
    pub delete: Option<String>,
    pub markers: serde_json::Map<String, Value>,
}

/// The link's fragment: the key, the envelope's URL, the share id and its expiry. Everything after
/// `#` stays in the browser.
pub fn fragment(link_key: &[u8; 32], envelope_url: &str, share_id: &str, expires_at: u64) -> String {
    let mut fragment = url_pairs(&[
        ("k", &crate::encoding::base64url(link_key)),
        ("e", envelope_url),
        ("i", share_id),
        ("x", &expires_at.to_string()),
    ]);
    fragment.insert(0, '#');
    fragment
}

fn url_pairs(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={}", crate::aws::sigv4::uri_encode(v, false)))
        .collect::<Vec<_>>()
        .join("&")
}

/// A record as a text file for a local share: `.md`, `.txt` or `.html`. The `.html` carries no
/// script at all and escapes everything, under a policy that allows nothing.
pub fn as_file(title: &str, fields: &[SharedField], notes: Option<&str>, format: &str) -> Result<(String, Vec<u8>)> {
    // Written in the sharer's language: the file is theirs until it reaches the recipient.
    let part_label = |key: &str| match key {
        "question" => tr!("Pergunta"),
        "holder" => tr!("Nome no cartão"),
        "expiry" => tr!("Validade"),
        "code" => tr!("Código de segurança"),
        "pin" => tr!("PIN"),
        other => other.to_string(),
    };
    let notes_label = tr!("Observação");
    let base = tr!("registro");

    let mut lines: Vec<(String, String)> = Vec::new();
    for field in fields {
        if let Some(question) = field.extra.get("question") {
            lines.push((format!("{}, {}", field.label, part_label("question")), question.clone()));
        }
        lines.push((field.label.clone(), field.value.clone()));
        for (key, value) in &field.extra {
            if key != "question" {
                lines.push((format!("{}, {}", field.label, part_label(key)), value.clone()));
            }
        }
    }

    match format {
        "md" => {
            let mut out = format!("# {title}\n\n");
            for (label, value) in &lines {
                out.push_str(&format!("- **{label}:** `{}`\n", value.replace('`', "'")));
            }
            if let Some(notes) = notes {
                out.push_str(&format!("\n## {notes_label}\n\n{notes}\n"));
            }
            Ok((format!("{base}.md"), out.into_bytes()))
        }
        "txt" => {
            let mut out = format!("{title}\n\n");
            for (label, value) in &lines {
                out.push_str(&format!("{label}: {value}\n"));
            }
            if let Some(notes) = notes {
                out.push_str(&format!("\n{notes_label}:\n{notes}\n"));
            }
            Ok((format!("{base}.txt"), out.into_bytes()))
        }
        "html" => {
            let escape = |text: &str| {
                text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
            };
            let mut out = format!(
                "<!doctype html><html lang=\"{1}\"><head><meta charset=\"utf-8\">\
                 <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">\
                 <title>{0}</title><style>body{{font:15px/1.5 system-ui,sans-serif;max-width:40rem;margin:2rem auto;padding:0 1rem}}\
                 dt{{color:#666;font-size:13px;margin-top:.75rem}}dd{{margin:0;font-family:ui-monospace,monospace;word-break:break-all}}</style>\
                 </head><body><h1>{0}</h1><dl>",
                escape(title),
                crate::i18n::current().tag()
            );
            for (label, value) in &lines {
                out.push_str(&format!("<dt>{}</dt><dd>{}</dd>", escape(label), escape(value)));
            }
            out.push_str("</dl>");
            if let Some(notes) = notes {
                out.push_str(&format!("<h2>{}</h2><p style=\"white-space:pre-wrap\">{}</p>", escape(&notes_label), escape(notes)));
            }
            out.push_str("</body></html>\n");
            Ok((format!("{base}.html"), out.into_bytes()))
        }
        _ => bail!(tr!("formato desconhecido: use md, txt ou html")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::Field;

    fn payload() -> Payload {
        let mut card = Field::new("Cartão", "card", "4111 1111 1111 4242");
        card.extra.insert("pin".into(), "4321".into());
        Payload {
            title: "Banco <teste>".into(),
            notes: "só pela VPN".into(),
            fields: vec![
                Field::new("Usuário", "text", "deploy"),
                Field::new("Senha", "password", "hunter2"),
                Field::new("2FA", "totp", "JBSWY3DPEHPK3PXP"),
                card,
            ],
            ..Payload::default()
        }
    }

    #[test]
    fn a_2fa_seed_never_goes_even_when_ticked() {
        let (fields, notes, left_out) = content(&payload(), &Choice { fields: vec![0, 1, 2], notes: false }).unwrap();

        assert_eq!(fields.iter().map(|f| f.label.as_str()).collect::<Vec<_>>(), vec!["Usuário", "Senha"]);
        assert_eq!(left_out, vec!["2FA".to_string()]);
        assert!(notes.is_none());
        assert!(content(&payload(), &Choice { fields: vec![2], notes: false }).is_err(), "nothing left to share");
    }

    #[test]
    fn the_page_is_the_same_for_every_share_and_its_script_is_allowed_by_hash() {
        let page = recipient_page();
        assert_eq!(page, recipient_page());
        assert!(!page.contains("{{"), "a placeholder was left");

        let start = page.find("<script>").unwrap() + 8;
        let end = page.rfind("</script>").unwrap();
        let hash = crate::encoding::base64_standard(&Sha256::digest(&page.as_bytes()[start..end]));
        assert!(page.contains(&format!("'sha256-{hash}'")));
        assert!(!page.contains("innerHTML"), "the page writes text, never markup");
    }

    #[test]
    fn the_html_file_escapes_everything_and_runs_nothing() {
        let (fields, notes, _) = content(&payload(), &Choice { fields: vec![0, 1, 3], notes: true }).unwrap();
        let (name, bytes) = as_file("Banco <teste>", &fields, notes.as_deref(), "html").unwrap();
        let html = String::from_utf8(bytes).unwrap();

        assert_eq!(name, "registro.html");
        assert!(html.contains("Banco &lt;teste&gt;") && !html.contains("<teste>"));
        assert!(!html.contains("<script"));
        assert!(html.contains("Cartão, PIN"));
        assert!(html.contains("default-src 'none'"));
    }

    #[test]
    fn the_fragment_carries_the_key_and_an_encoded_url() {
        let key = [7u8; 32];
        let fragment = fragment(&key, "https://b.s3.us-east-2.amazonaws.com/s/e?X-Amz-Signature=ab&x=1", "abc", 99);

        assert!(fragment.starts_with("#k="));
        assert!(fragment.contains("&e=https%3A%2F%2Fb.s3"));
        assert!(fragment.ends_with("&i=abc&x=99"));
    }

    /// Not a check: writes a share a real browser can open, to try the page's WebCrypto against
    /// what this module seals. `KEYHOLD_DEMO_DIR=<dir> cargo test --lib write_a_demo_share -- --ignored`,
    /// then serve `<dir>` on 127.0.0.1:8799 and open the printed address.
    #[test]
    #[ignore]
    fn write_a_demo_share() {
        use crypto::*;

        let dir = std::path::PathBuf::from(std::env::var("KEYHOLD_DEMO_DIR").expect("KEYHOLD_DEMO_DIR"));
        std::fs::create_dir_all(&dir).unwrap();
        let base = "http://127.0.0.1:8799";

        for (name, password, first) in [("comsenha", Some("SenhaDeAbertura2026"), true), ("semsenha", None, false)] {
            let id = format!("demo{name}");
            let expires = crate::identity::now_seconds() + 3600;
            let aad = aad(&id, expires);
            let key = random32();
            let salt = [9u8; 16];

            let (fields, notes, _) = content(&payload(), &Choice { fields: vec![0, 1, 3], notes: true }).unwrap();
            let content_key = content_key(&key, password.map(|p| (p, &salt[..], PASSWORD_ITERATIONS)));
            std::fs::write(dir.join(format!("{name}-content.bin")), seal(&content_key, &content_json(&fields, notes.as_deref()), &aad).unwrap()).unwrap();

            let envelope = Envelope {
                v: 1,
                title: "Postgres produção".into(),
                sender: "Ana Souza".into(),
                expires_at: expires,
                first,
                password: password.map(|_| json!({ "salt": crate::encoding::base64url(&salt), "iterations": PASSWORD_ITERATIONS })),
                content: format!("{base}/{name}-content.bin"),
                delete: None,
                markers: serde_json::Map::new(),
            };
            std::fs::write(dir.join(format!("{name}-envelope.bin")), seal(&envelope_key(&key), &serde_json::to_vec(&envelope).unwrap(), &aad).unwrap()).unwrap();
            std::fs::write(dir.join(format!("{name}.html")), recipient_page()).unwrap();

            println!("{base}/{name}.html{}", fragment(&key, &format!("{base}/{name}-envelope.bin"), &id, expires));
        }
    }
}
