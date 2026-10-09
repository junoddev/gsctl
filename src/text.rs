//! `--format text`: compact human-readable renderings of the JSON output.

use serde_json::Value;

fn compact(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

pub fn render(command: &str, out: &Value) -> String {
    let mut text = String::new();
    let mut line = |l: String| {
        text.push_str(&l);
        text.push('\n');
    };
    match command {
        "search" => {
            for r in out["results"].as_array().into_iter().flatten() {
                line(format!(
                    "{} {} = {}  # {}",
                    s(&r["schema"]),
                    s(&r["key"]),
                    compact(&r["value"]),
                    s(&r["summary"])
                ));
            }
            let shown = out["results"].as_array().map_or(0, Vec::len);
            let kind = if out["partial"] == true { " partial" } else { "" };
            line(format!("({shown} of {}{kind} matches)", out["total"]));
        }
        "schema" if out.get("schemas").is_some() => {
            for sch in out["schemas"].as_array().into_iter().flatten() {
                let extra = if sch["relocatable"] == true { " (relocatable)" } else { "" };
                line(format!("{}{extra}", s(&sch["id"])));
            }
        }
        "schema" if out.get("keys").is_some() => {
            for k in out["keys"].as_array().into_iter().flatten() {
                line(key_text(k));
            }
        }
        "schema" => {
            line(key_text(out));
            if let Some(d) = out["description"].as_str() {
                line(String::new());
                line(d.to_string());
            }
        }
        "get" => line(compact(&out["value"])),
        "set" | "reset" => {
            let what = if out["dry_run"] == true { "would change" } else { "changed" };
            let what = if out["changed"] == false { "unchanged" } else { what };
            line(format!(
                "{} {} {what}: {} -> {}",
                s(&out["schema"]),
                s(&out["key"]),
                compact(&out["old"]),
                compact(&out["new"])
            ));
        }
        "changed" => {
            for c in out["changed"].as_array().into_iter().flatten() {
                line(format!(
                    "{} {} = {}  (default {})",
                    s(&c["schema"]),
                    s(&c["key"]),
                    compact(&c["value"]),
                    compact(&c["default"])
                ));
            }
        }
        _ => line(serde_json::to_string_pretty(out).unwrap()),
    }
    text
}

fn key_text(k: &Value) -> String {
    let mut l = format!("{} ({})", s(&k["key"]), s(&k["type"]));
    if let Some(v) = k.get("value") {
        l.push_str(&format!(" = {}", compact(v)));
    }
    l.push_str(&format!("  default {}", compact(&k["default"])));
    match k["range"]["kind"].as_str() {
        Some("enum") => l.push_str(&format!("  one of {}", k["range"]["choices"])),
        Some("flags") => l.push_str(&format!("  flags {}", k["range"]["flags"])),
        Some("range") => l.push_str(&format!("  range {}..={}", k["range"]["min"], k["range"]["max"])),
        _ => {}
    }
    if let Some(sum) = k["summary"].as_str() {
        l.push_str(&format!("  # {sum}"));
    }
    l
}
