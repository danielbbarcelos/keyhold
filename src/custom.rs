//! What a person makes their own: a library of icons to reuse, and the kinds of record a new one
//! can start as.
//!
//! Both are sealed under the master key, like the standard alerts: a picture of a service's logo
//! says which services somebody has accounts with, and so does a record type called after one.
//!
//! An icon in the library is only a picture with a name. A record that uses it keeps its own copy,
//! sealed with the record, so a record never depends on the library: deleting an icon here takes
//! it out of the picker and changes no record. The same holds for a type: it is where a new record
//! starts, and the record owes it nothing afterwards.

use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::crypto::{self, SecretKey};
use crate::db::Store;
use crate::encoding::{base64url, from_base64url};
use crate::records::{Icon, ICON_IMAGE_LIMIT};

const ICONS_SETTING: &str = "custom_icons";
const ICONS_AAD: &[u8] = b"keyhold:custom-icons:v1";
const TYPES_SETTING: &str = "record_types";
const TYPES_AAD: &[u8] = b"keyhold:record-types:v1";

const MAX_ICONS: usize = 300;
const MAX_TYPES: usize = 60;
const MAX_FIELDS: usize = 40;
const MAX_NAME: usize = 60;

/// The kinds of field a record can hold. The same list the interface draws.
const FIELD_KINDS: &[&str] = &[
    "text",
    "multiline",
    "multiline_hidden",
    "hidden",
    "password",
    "pin",
    "totp",
    "email",
    "name",
    "phone",
    "url",
    "address",
    "date",
    "security_question",
    "card",
];

/// A picture kept to be used again.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CustomIcon {
    pub id: String,
    pub name: String,
    /// A `data:image/...` address, already shrunk by the interface.
    pub image: String,
}

/// One field a new record of a type starts with.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TypeField {
    pub kind: String,
    #[serde(default)]
    pub label: String,
}

/// A kind of record: a name, an icon and the fields a new one starts with.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecordType {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub icon: Icon,
    #[serde(default)]
    pub fields: Vec<TypeField>,
}

fn clipped(text: &str, max: usize) -> String {
    text.trim().chars().take(max).collect()
}

fn field(kind: &str, label: &str) -> TypeField {
    TypeField { kind: kind.into(), label: label.into() }
}

fn lucide(name: &str, colour: &str) -> Icon {
    Icon { name: name.into(), colour: colour.into(), image: None }
}

/// What somebody who never changed anything finds: the four kinds Keyhold always offered.
pub fn starter_types() -> Vec<RecordType> {
    vec![
        RecordType {
            id: "login".into(),
            label: "Login".into(),
            icon: lucide("key-round", "sky"),
            fields: vec![field("email", ""), field("password", ""), field("url", ""), field("totp", "")],
        },
        RecordType {
            id: "cartao".into(),
            label: "Cartão".into(),
            icon: lucide("credit-card", "mint"),
            fields: vec![field("card", ""), field("phone", "Central de atendimento")],
        },
        RecordType {
            id: "ssh".into(),
            label: "Chave SSH".into(),
            icon: lucide("terminal", "stone"),
            fields: vec![
                field("text", "Servidor"),
                field("text", "Usuário"),
                field("multiline_hidden", "Chave privada"),
                field("password", "Frase secreta"),
            ],
        },
        RecordType {
            id: "nota".into(),
            label: "Nota".into(),
            icon: lucide("file-text", "sand"),
            fields: vec![field("multiline_hidden", "Nota")],
        },
    ]
}

fn read<T: DeserializeOwned>(store: &Store, master: &SecretKey, setting: &str, aad: &[u8]) -> Result<Option<T>> {
    let raw: Option<String> = store
        .conn()
        .query_row("SELECT value FROM settings WHERE key = ?1", params![setting], |row| row.get(0))
        .optional()?;

    let Some(raw) = raw else {
        return Ok(None);
    };

    let sealed = from_base64url(&raw).context("a personalização está danificada")?;
    let bytes = crypto::open(master, &sealed, aad).context("não foi possível abrir a personalização")?;

    Ok(Some(serde_json::from_slice(&bytes).context("a personalização não é legível")?))
}

fn write<T: Serialize>(store: &Store, master: &SecretKey, setting: &str, aad: &[u8], value: &T) -> Result<()> {
    let sealed = crypto::seal(master, &serde_json::to_vec(value)?, aad)?;

    store.conn().execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![setting, base64url(&sealed)],
    )?;

    Ok(())
}

fn check_image(image: &str) -> Result<()> {
    if !image.starts_with("data:image/") {
        bail!("o ícone precisa ser uma imagem");
    }
    if image.len() > ICON_IMAGE_LIMIT {
        bail!("a imagem do ícone é grande demais");
    }

    Ok(())
}

pub fn load_icons(store: &Store, master: &SecretKey) -> Result<Vec<CustomIcon>> {
    Ok(read(store, master, ICONS_SETTING, ICONS_AAD)?.unwrap_or_default())
}

pub fn save_icons(store: &Store, master: &SecretKey, icons: Vec<CustomIcon>) -> Result<Vec<CustomIcon>> {
    if icons.len() > MAX_ICONS {
        bail!("a biblioteca guarda até {MAX_ICONS} ícones");
    }

    let mut kept = Vec::with_capacity(icons.len());
    for icon in icons {
        if icon.id.trim().is_empty() {
            continue;
        }
        check_image(&icon.image)?;

        let name = clipped(&icon.name, MAX_NAME);
        kept.push(CustomIcon {
            id: clipped(&icon.id, 64),
            name: if name.is_empty() { "Sem nome".into() } else { name },
            image: icon.image,
        });
    }

    write(store, master, ICONS_SETTING, ICONS_AAD, &kept)?;

    Ok(kept)
}

/// The kinds of record, or the starters when none were ever saved.
pub fn load_types(store: &Store, master: &SecretKey) -> Result<Vec<RecordType>> {
    Ok(read(store, master, TYPES_SETTING, TYPES_AAD)?.unwrap_or_else(starter_types))
}

pub fn save_types(store: &Store, master: &SecretKey, types: Vec<RecordType>) -> Result<Vec<RecordType>> {
    if types.len() > MAX_TYPES {
        bail!("o Keyhold guarda até {MAX_TYPES} tipos de registro");
    }

    let mut kept = Vec::with_capacity(types.len());
    for one in types {
        let label = clipped(&one.label, MAX_NAME);
        if one.id.trim().is_empty() || label.is_empty() {
            continue;
        }
        if one.fields.len() > MAX_FIELDS {
            bail!("um tipo de registro tem até {MAX_FIELDS} campos");
        }
        if let Some(image) = one.icon.image.as_deref() {
            check_image(image)?;
        }

        let mut fields = Vec::with_capacity(one.fields.len());
        for f in one.fields {
            if !FIELD_KINDS.contains(&f.kind.as_str()) {
                bail!("o tipo de campo \"{}\" não existe", clipped(&f.kind, 32));
            }
            fields.push(TypeField { kind: f.kind, label: clipped(&f.label, MAX_NAME) });
        }

        kept.push(RecordType {
            id: clipped(&one.id, 64),
            label,
            icon: Icon {
                name: clipped(&one.icon.name, 40),
                colour: clipped(&one.icon.colour, 16),
                image: one.icon.image,
            },
            fields,
        });
    }

    write(store, master, TYPES_SETTING, TYPES_AAD, &kept)?;

    Ok(kept)
}

/// Back to the four kinds Keyhold starts with.
pub fn reset_types(store: &Store) -> Result<()> {
    store.conn().execute("DELETE FROM settings WHERE key = ?1", params![TYPES_SETTING])?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIXEL: &str = "data:image/png;base64,iVBORw0KGgo=";

    fn fresh() -> (tempfile::TempDir, Store, SecretKey) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("vault.db")).unwrap();

        (dir, store, SecretKey::random())
    }

    #[test]
    fn icons_are_sealed_and_come_back() {
        let (_dir, store, key) = fresh();

        assert!(load_icons(&store, &key).unwrap().is_empty());

        let mine = vec![CustomIcon { id: "a".into(), name: "Banco do cliente".into(), image: PIXEL.into() }];
        save_icons(&store, &key, mine.clone()).unwrap();

        let raw: String = store
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", params![ICONS_SETTING], |r| r.get(0))
            .unwrap();
        assert!(!raw.contains("cliente"), "the name is sealed");
        assert_eq!(load_icons(&store, &key).unwrap(), mine);
        assert!(load_icons(&store, &SecretKey::random()).is_err(), "another key opens nothing");
    }

    #[test]
    fn an_icon_that_is_not_a_picture_is_refused() {
        let (_dir, store, key) = fresh();

        let wrong = vec![CustomIcon { id: "a".into(), name: "x".into(), image: "https://exemplo.com/logo.png".into() }];
        assert!(save_icons(&store, &key, wrong).is_err());

        let huge = vec![CustomIcon { id: "a".into(), name: "x".into(), image: format!("data:image/png;base64,{}", "A".repeat(ICON_IMAGE_LIMIT)) }];
        assert!(save_icons(&store, &key, huge).is_err());
    }

    #[test]
    fn types_start_as_the_four_and_can_be_changed_and_put_back() {
        let (_dir, store, key) = fresh();

        assert_eq!(load_types(&store, &key).unwrap(), starter_types());

        let mine = vec![RecordType {
            id: "banco".into(),
            label: "Banco de dados".into(),
            icon: lucide("database", "sky"),
            fields: vec![field("text", "Host"), field("password", "")],
        }];
        assert_eq!(save_types(&store, &key, mine.clone()).unwrap(), mine);
        assert_eq!(load_types(&store, &key).unwrap(), mine);

        save_types(&store, &key, Vec::new()).unwrap();
        assert!(load_types(&store, &key).unwrap().is_empty(), "no types at all is a choice, and it is kept");

        reset_types(&store).unwrap();
        assert_eq!(load_types(&store, &key).unwrap(), starter_types());
    }

    #[test]
    fn a_type_with_a_field_kind_that_does_not_exist_is_refused() {
        let (_dir, store, key) = fresh();

        let wrong = vec![RecordType { id: "x".into(), label: "X".into(), icon: Icon::default(), fields: vec![field("senha", "")] }];
        assert!(save_types(&store, &key, wrong).is_err());
    }
}
