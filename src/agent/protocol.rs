//! What the MCP subprocess and the daemon say to each other.
//!
//! One JSON object per line, over a unix socket. Deliberately not HTTP: the socket's file
//! permission *is* the authentication, so there is no token to put in a config file, no port for
//! another local process to find, and nothing in `/proc/*/cmdline` to read. When the vault closes,
//! the socket goes with it and the integration stops existing rather than starts refusing.
//!
//! The subprocess holds no key and no state. Everything below is a question it asks and an answer
//! the daemon decides — including whether the person said yes.

use serde::{Deserialize, Serialize};

/// What an agent may ask for.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Is the vault open, and what may be seen at all?
    Status,
    /// Find records by title or username. Never returns a secret.
    Search { query: String },
    /// What fields a record has, by label and kind. Never their values.
    Describe { record: String },
    /// One field's value. Needs the person to say yes.
    Get { record: String, field: String },
    /// Run a command with a field's value in an environment variable. The value never comes back.
    Run { record: String, field: String, command: Vec<String>, variable: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecordSummary {
    pub id: String,
    pub title: String,
    pub username: String,
    pub url: String,
    pub vault: String,
    pub kind: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FieldSummary {
    pub label: String,
    /// `password`, `text`, `hidden`, `url` or `date` — enough to know what it is, never what it says.
    pub kind: String,
    /// Whether asking for this one will need approval.
    pub secret: bool,
}

/// What the daemon answers.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Status { unlocked: bool, vaults: Vec<String>, records: usize },
    Records { records: Vec<RecordSummary> },
    Fields { title: String, fields: Vec<FieldSummary> },
    Value { value: String },
    Ran { code: i32, stdout: String, stderr: String },
    /// Refused, with a reason meant to be read by a person over the agent's shoulder.
    Refused { reason: String },
}

impl Response {
    pub fn refused(reason: impl Into<String>) -> Self {
        Response::Refused { reason: reason.into() }
    }
}

/// Who is asking, as the daemon learns it from the socket itself.
///
/// Read from `SO_PEERCRED` rather than from anything the client says: a name a client sends is a
/// name a client chooses, and the approval dialog exists to tell somebody which process is really
/// on the other end.
#[derive(Clone, Debug, Serialize)]
pub struct Peer {
    pub pid: i64,
    /// The process on the socket, which is always `keyhold mcp` — it is the translator.
    pub process: String,
    /// What launched it: `claude`, or whatever else is driving. This is the name worth showing.
    pub parent: Option<String>,
}

impl Peer {
    /// How to name this peer to a person: the thing that is actually asking.
    pub fn describe(&self) -> String {
        match &self.parent {
            Some(parent) => format!("{parent} (via {})", self.process),
            None => self.process.clone(),
        }
    }
}
