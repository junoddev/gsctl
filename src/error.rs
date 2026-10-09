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
    #[error("{schema} {key} is a guarded key: {reason}. Re-run with --yes to change it")]
    Guarded { schema: String, key: String, reason: String },
    #[error("{0}")]
    Internal(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Internal(_) => 1,
            Error::Usage(_) => 2,
            Error::SchemaNotFound(_) | Error::KeyNotFound { .. } => 3,
            Error::InvalidValue { .. } => 4,
            Error::NotWritable { .. } => 5,
            Error::Guarded { .. } => 6,
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
            Error::Guarded { .. } => "guarded",
        }
    }

    pub fn to_json(&self) -> Value {
        let mut err = json!({
            "code": self.code(),
            "exit_code": self.exit_code(),
            "message": self.to_string(),
        });
        let extra = match self {
            Error::SchemaNotFound(schema) => json!({ "schema": schema, "hint": "try `gsap search <words>` or `gsap schema` to list schemas" }),
            Error::KeyNotFound { schema, key } => json!({ "schema": schema, "key": key, "hint": format!("try `gsap schema {schema}` to list its keys") }),
            Error::InvalidValue { details, .. } => details.clone(),
            Error::NotWritable { schema, key } => json!({ "schema": schema, "key": key }),
            Error::Guarded { schema, key, reason } => json!({ "schema": schema, "key": key, "reason": reason }),
            Error::Usage(_) | Error::Internal(_) => Value::Null,
        };
        if let (Value::Object(err), Value::Object(extra)) = (&mut err, extra) {
            err.extend(extra);
        }
        json!({ "error": err })
    }
}
