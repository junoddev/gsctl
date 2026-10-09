//! Append-only log of every change gsap makes, with the previous value, at
//! `$XDG_STATE_HOME/gsap/journal.ndjson` (default `~/.local/state/...`).
//! `history` and `revert` will read it.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

pub fn path() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state"));
    state.join("gsap/journal.ndjson")
}

/// One recorded change. Values are stored both as JSON (for reading) and as
/// typed GVariant text (for exact replay). `old_user_value` is `None` when the
/// key was at its default, so a revert should `reset` rather than `set`.
pub struct Change<'a> {
    pub op: &'a str,
    pub schema: &'a str,
    pub path: Option<&'a str>,
    pub key: &'a str,
    pub old: &'a glib::Variant,
    pub old_user_value: Option<&'a glib::Variant>,
    pub new: &'a glib::Variant,
}

/// Append `change` and return its entry id.
pub fn record(change: &Change) -> std::io::Result<String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let id = format!("{}-{}", now.as_millis(), std::process::id());
    let entry = json!({
        "id": id,
        "time": rfc3339(now.as_secs()),
        "op": change.op,
        "schema": change.schema,
        "path": change.path,
        "key": change.key,
        "old": crate::convert::to_json(change.old),
        "new": crate::convert::to_json(change.new),
        "old_gvariant": change.old.print(true).as_str(),
        "old_user_value": change.old_user_value.map(|v| v.print(true).to_string()),
        "new_gvariant": change.new.print(true).as_str(),
    });
    append(&entry)?;
    Ok(id)
}

fn append(entry: &Value) -> std::io::Result<()> {
    let path = path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
    // One write per line keeps concurrent appends from interleaving.
    f.write_all(format!("{entry}\n").as_bytes())
}

/// Format Unix seconds as `YYYY-MM-DDTHH:MM:SSZ`.
fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn rfc3339() {
        assert_eq!(super::rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(super::rfc3339(1_791_547_200), "2026-10-09T12:00:00Z");
    }
}
