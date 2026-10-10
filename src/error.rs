//! Errors with stable codes and exit statuses, reported as JSON on stderr.

use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Usage(String),
    #[error("schema not found: {0}")]
    SchemaNotFound(String),
    #[error("key not found: {schema} {key}")]
    KeyNotFound { schema: String, key: String },
    #[error("{message}")]
    InvalidValue { message: String, details: Value },
    #[error("key is not writable (locked by the system administrator): {schema} {key}")]
    NotWritable { schema: String, key: String },
    #[error("cannot write settings: {0}. Run gsctl inside the desktop session, not a sandbox (see `gsctl doctor`)")]
    NoSession(String),
    #[error("{schema} {key} is a guarded key: {reason}. Re-run with --yes to change it")]
    Guarded { schema: String, key: String, reason: String },
    #[error("extension not installed: {0}")]
    ExtensionNotFound(String),
    #[error("no custom keybinding named or at {0}")]
    BindingNotFound(String),
    #[error("journal entry not found: {0}")]
    EntryNotFound(String),
    #[error("{message}")]
    Conflict { id: String, message: String },
    /// A conflict with structured details merged into the error object
    /// (`keys add`: the bindings already on an accelerator or name).
    #[error("{message}")]
    ConflictWith { id: String, message: String, details: Value },
    /// Several problems at once (from `apply`); `exit` is the first problem's.
    #[error("{message}")]
    Batch { message: String, exit: i32, problems: Value },
    #[error("{0}")]
    Internal(String),
    /// A write in a batch failed after earlier writes were made. `details`
    /// (`batch`, `rolled_back`, `not_rolled_back`) is merged into the JSON;
    /// code and exit status are the failed write's.
    #[error("{message}")]
    BatchWrite { error: Box<Error>, message: String, details: Value },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Internal(_) => 1,
            Error::Usage(_) => 2,
            Error::SchemaNotFound(_)
            | Error::KeyNotFound { .. }
            | Error::EntryNotFound(_)
            | Error::ExtensionNotFound(_)
            | Error::BindingNotFound(_) => 3,
            Error::InvalidValue { .. } => 4,
            Error::NotWritable { .. } | Error::NoSession(_) => 5,
            Error::Guarded { .. } => 6,
            Error::Conflict { .. } | Error::ConflictWith { .. } => 7,
            Error::Batch { exit, .. } => *exit,
            Error::BatchWrite { error, .. } => error.exit_code(),
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Error::Internal(_) => "internal",
            Error::Usage(_) => "usage",
            Error::SchemaNotFound(_) => "schema_not_found",
            Error::KeyNotFound { .. } => "key_not_found",
            Error::InvalidValue { .. } => "invalid_value",
            Error::NotWritable { .. } => "not_writable",
            Error::NoSession(_) => "no_session",
            Error::Guarded { .. } => "guarded",
            Error::EntryNotFound(_) => "entry_not_found",
            Error::ExtensionNotFound(_) => "extension_not_found",
            Error::BindingNotFound(_) => "binding_not_found",
            Error::Conflict { .. } | Error::ConflictWith { .. } => "conflict",
            Error::Batch { .. } => "batch_invalid",
            Error::BatchWrite { error, .. } => error.code(),
        }
    }

    pub fn to_json(&self) -> Value {
        if let Error::BatchWrite { error, message, details } = self {
            let mut out = error.to_json();
            if let (Value::Object(err), Value::Object(details)) = (&mut out["error"], details.clone()) {
                err.insert("message".into(), message.clone().into());
                err.extend(details);
            }
            return out;
        }
        let mut err = json!({
            "code": self.code(),
            "exit_code": self.exit_code(),
            "message": self.to_string(),
        });
        let extra = match self {
            Error::SchemaNotFound(schema) => {
                json!({ "schema": schema, "hint": "try `gsctl search <words>` or `gsctl schema` to list schemas" })
            }
            Error::KeyNotFound { schema, key } => {
                json!({ "schema": schema, "key": key, "hint": format!("try `gsctl schema {schema}` to list its keys") })
            }
            Error::InvalidValue { details, .. } => details.clone(),
            Error::NotWritable { schema, key } => json!({ "schema": schema, "key": key }),
            Error::Guarded { schema, key, reason } => json!({ "schema": schema, "key": key, "reason": reason }),
            Error::EntryNotFound(id) => json!({ "id": id, "hint": "see `gsctl history`" }),
            Error::ExtensionNotFound(uuid) => json!({ "uuid": uuid, "hint": "see `gsctl ext list`" }),
            Error::BindingNotFound(what) => json!({ "binding": what, "hint": "see `gsctl keys list --custom`" }),
            Error::Conflict { id, .. } => json!({ "id": id }),
            Error::ConflictWith { id, details, .. } => {
                let mut v = json!({ "id": id });
                if let (Value::Object(v), Value::Object(d)) = (&mut v, details) {
                    v.extend(d.clone());
                }
                v
            }
            Error::Batch { problems, .. } => json!({ "problems": problems }),
            Error::NoSession(reason) => json!({ "reason": reason, "hint": "run `gsctl doctor`" }),
            Error::Usage(_) | Error::Internal(_) | Error::BatchWrite { .. } => Value::Null,
        };
        if let (Value::Object(err), Value::Object(extra)) = (&mut err, extra) {
            err.extend(extra);
        }
        json!({ "error": err })
    }
}
