//! The audit trail of an AWS vault: what goes where, and what a failure means.
//!
//! Pure rules, no network: `http::aws` does the writing. See `docs/AWS.md` for the account side.
//!
//! - **One log group per permission boundary**, the first segment of the record's parameter name:
//!   `/team/db/main` writes to `/keyhold/audit/team`, `/db` to `/keyhold/audit/#root` (`#` never
//!   appears in a parameter name). CloudWatch grants reading per log group, never per stream, so
//!   the group is the unit of who-may-read.
//! - **One stream per person**, named after their Identity Center user. The policy pins the name,
//!   so the stream is who did it, and nothing in an event's body is trusted for that.
//! - **What a failure means** depends on what failed. No permission or no log group means the
//!   account has not set a trail up: Keyhold says so and goes on, unless the bucket says the trail
//!   is required. Anything else (network, throttling, an answer that makes no sense) is "unknown",
//!   and a sensitive action does not happen without its event.

use serde::Serialize;
use serde_json::{json, Value};

use super::AwsError;
use crate::tr;

pub const DEFAULT_PREFIX: &str = "/keyhold/audit";

/// The log group of parameters directly at the root of the account.
pub const ROOT_SEGMENT: &str = "#root";

/// Events kept for a later attempt, at most. Past this the oldest go: a queue that grows without
/// end while CloudWatch is unreachable is memory, not a trail.
pub const QUEUE_LIMIT: usize = 1000;

/// The log group for a parameter.
pub fn log_group(prefix: &str, parameter: &str) -> String {
    let segment = parameter.trim_start_matches('/').split('/').collect::<Vec<_>>();
    let boundary = if segment.len() > 1 && !segment[0].is_empty() { segment[0] } else { ROOT_SEGMENT };

    format!("{}/{boundary}", prefix.trim_end_matches('/'))
}

/// What writing to one log group came to, in this unlock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Audited,
    /// The account has no trail for this boundary, and why, in a sentence for the screen.
    Off(String),
}

/// How a failed write is to be read.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// Not set up: no permission or no log group. A fact about the account, cached.
    Off(String),
    /// Could not tell: the action waits.
    Unavailable(String),
}

pub fn classify(error: &AwsError, group: &str) -> Failure {
    match error {
        AwsError::AccessDenied(_) => Failure::Off(
            tr!("sem permissão para gravar a trilha: falta a política audit-write ou o atributo keyhold-user no Identity Center"),
        ),
        AwsError::NotFound(_) => Failure::Off(tr!("o log group {group} não existe nesta conta", group = group)),
        other => Failure::Unavailable(tr!("o CloudWatch Logs não respondeu ({other})", other = other)),
    }
}

/// What the administrator left on a vault's bucket for every vault on it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BucketSettings {
    /// `keyhold:audit=required`: no trail means no sensitive action.
    pub required: bool,
    /// `keyhold:audit-log-group`: another prefix for the log groups.
    pub prefix: Option<String>,
    /// `keyhold:share=off`.
    pub share_off: bool,
    /// `keyhold:share-prefix`: where shares of root-level parameters go.
    pub share_prefix: Option<String>,
    /// The tags could not be read (no `s3:GetBucketTagging`, or no bucket): the defaults apply.
    pub unreadable: bool,
}

impl BucketSettings {
    pub fn from_tags(tags: &[(String, String)]) -> Self {
        let get = |key: &str| tags.iter().find(|(k, _)| k == key).map(|(_, v)| v.trim().to_string());

        Self {
            required: get("keyhold:audit").is_some_and(|v| v.eq_ignore_ascii_case("required")),
            prefix: get("keyhold:audit-log-group").filter(|v| v.starts_with('/') && v.len() > 1),
            share_off: get("keyhold:share").is_some_and(|v| v.eq_ignore_ascii_case("off")),
            share_prefix: get("keyhold:share-prefix").filter(|v| !v.is_empty()),
            unreadable: false,
        }
    }

    pub fn unreadable() -> Self {
        Self { unreadable: true, ..Self::default() }
    }

    pub fn prefix(&self) -> &str {
        self.prefix.as_deref().unwrap_or(DEFAULT_PREFIX)
    }
}

/// An event waiting to be sent.
#[derive(Clone, Debug)]
pub struct Queued {
    pub connection_id: String,
    pub region: String,
    pub group: String,
    pub stream: String,
    pub at_ms: i64,
    pub message: String,
}

/// The body of one event. Names a record and a field, never a value; the person and the time come
/// from the stream and from CloudWatch, so neither is written here as a claim.
pub fn message(action: &str, record: &str, parameter: &str, detail: Value) -> String {
    let mut body = json!({
        "v": 1,
        "action": action,
        "record": record,
        "parameter": parameter,
        "client": format!("keyhold {}", env!("CARGO_PKG_VERSION")),
    });

    if let (Some(body), Value::Object(detail)) = (body.as_object_mut(), detail) {
        for (key, value) in detail {
            // The fixed keys above are not the detail's to replace.
            body.entry(key).or_insert(value);
        }
    }

    body.to_string()
}

/// The labels of the fields that changed between two versions of a record, values never compared
/// in the output, only used to decide.
pub fn changed_fields(before: &crate::records::Payload, after: &crate::records::Payload) -> Vec<String> {
    let mut changed: Vec<String> = Vec::new();

    for field in &after.fields {
        let same = before.fields.iter().any(|old| old == field);
        if !same && !changed.contains(&field.label) {
            changed.push(field.label.clone());
        }
    }

    for old in &before.fields {
        if !after.fields.iter().any(|field| field.label == old.label) && !changed.contains(&old.label) {
            changed.push(old.label.clone());
        }
    }

    if before.title != after.title {
        changed.push("título".into());
    }
    if before.notes != after.notes {
        changed.push("observação".into());
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_segment_is_the_boundary() {
        assert_eq!(log_group(DEFAULT_PREFIX, "/team/db/main"), "/keyhold/audit/team");
        assert_eq!(log_group(DEFAULT_PREFIX, "/admins/root"), "/keyhold/audit/admins");
        assert_eq!(log_group(DEFAULT_PREFIX, "/db"), "/keyhold/audit/#root");
        assert_eq!(log_group("/acme/trail/", "/team/x"), "/acme/trail/team");
    }

    #[test]
    fn settings_come_from_the_bucket_tags() {
        let tags = vec![
            ("keyhold:audit".to_string(), "Required".to_string()),
            ("keyhold:audit-log-group".to_string(), "/acme/trail".to_string()),
            ("keyhold:share".to_string(), "off".to_string()),
        ];
        let settings = BucketSettings::from_tags(&tags);

        assert!(settings.required && settings.share_off);
        assert_eq!(settings.prefix(), "/acme/trail");
        assert_eq!(BucketSettings::from_tags(&[]).prefix(), DEFAULT_PREFIX);
        // A prefix that is not a path is ignored rather than written to somewhere odd.
        let odd = BucketSettings::from_tags(&[("keyhold:audit-log-group".into(), "trail".into())]);
        assert_eq!(odd.prefix(), DEFAULT_PREFIX);
    }

    #[test]
    fn not_set_up_is_off_and_anything_else_is_unknown() {
        assert!(matches!(classify(&AwsError::AccessDenied("x".into()), "/g"), Failure::Off(_)));
        assert!(matches!(classify(&AwsError::NotFound("x".into()), "/g"), Failure::Off(_)));
        assert!(matches!(classify(&AwsError::Network("x".into()), "/g"), Failure::Unavailable(_)));
        assert!(matches!(classify(&AwsError::Throttled("x".into()), "/g"), Failure::Unavailable(_)));
    }

    #[test]
    fn an_event_names_things_and_keeps_its_own_keys() {
        let body: Value =
            serde_json::from_str(&message("field.revealed", "r1", "/team/db", json!({ "field": "Senha", "action": "forged" })))
                .unwrap();

        assert_eq!(body["action"], "field.revealed");
        assert_eq!(body["field"], "Senha");
        assert!(body.get("user").is_none() && body.get("ip").is_none());
    }

    #[test]
    fn changed_fields_names_what_moved_and_never_carries_a_value() {
        use crate::records::{Field, Payload};

        let before = Payload {
            title: "Banco".into(),
            fields: vec![Field::new("Senha", "password", "a"), Field::new("Usuário", "text", "ana")],
            ..Payload::default()
        };
        let after = Payload {
            title: "Banco".into(),
            fields: vec![Field::new("Senha", "password", "b"), Field::new("Usuário", "text", "ana")],
            ..Payload::default()
        };

        assert_eq!(changed_fields(&before, &after), vec!["Senha".to_string()]);
    }
}
