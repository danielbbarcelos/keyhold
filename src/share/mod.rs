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
        bail!("escolha ao menos um campo para compartilhar");
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
    const PARTS: [(&str, &str); 5] =
        [("question", "Pergunta"), ("holder", "Nome no cartão"), ("expiry", "Validade"), ("code", "Código de segurança"), ("pin", "PIN")];
    let part_label = |key: &str| PARTS.iter().find(|(k, _)| *k == key).map(|(_, l)| *l).unwrap_or(key).to_string();

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
                out.push_str(&format!("\n## Observação\n\n{notes}\n"));
            }
            Ok(("registro.md".into(), out.into_bytes()))
        }
        "txt" => {
            let mut out = format!("{title}\n\n");
            for (label, value) in &lines {
                out.push_str(&format!("{label}: {value}\n"));
            }
            if let Some(notes) = notes {
                out.push_str(&format!("\nObservação:\n{notes}\n"));
            }
            Ok(("registro.txt".into(), out.into_bytes()))
        }
        "html" => {
            let escape = |text: &str| {
                text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
            };
            let mut out = format!(
                "<!doctype html><html lang=\"pt-BR\"><head><meta charset=\"utf-8\">\
                 <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">\
                 <title>{0}</title><style>body{{font:15px/1.5 system-ui,sans-serif;max-width:40rem;margin:2rem auto;padding:0 1rem}}\
                 dt{{color:#666;font-size:13px;margin-top:.75rem}}dd{{margin:0;font-family:ui-monospace,monospace;word-break:break-all}}</style>\
                 </head><body><h1>{0}</h1><dl>",
                escape(title)
            );
            for (label, value) in &lines {
                out.push_str(&format!("<dt>{}</dt><dd>{}</dd>", escape(label), escape(value)));
            }
            out.push_str("</dl>");
            if let Some(notes) = notes {
                out.push_str(&format!("<h2>Observação</h2><p style=\"white-space:pre-wrap\">{}</p>", escape(notes)));
            }
            out.push_str("</body></html>\n");
            Ok(("registro.html".into(), out.into_bytes()))
        }
        _ => bail!("formato desconhecido: use md, txt ou html"),
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
}
