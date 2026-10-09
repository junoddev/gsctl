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
        "set" | "reset" | "revert" => {
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
        "history" => {
            for e in out["entries"].as_array().into_iter().flatten() {
                let note = match (e["reverts"].as_str(), e["reverted_by"].as_str()) {
                    (Some(r), _) => format!("  (reverts {r})"),
                    (None, Some(by)) => format!("  (reverted by {by})"),
                    _ => String::new(),
                };
                line(format!(
                    "{} {} {:<6} {} {}: {} -> {}{note}",
                    s(&e["id"]),
                    s(&e["time"]),
                    s(&e["op"]),
                    s(&e["schema"]),
                    s(&e["key"]),
                    compact(&e["old"]),
                    compact(&e["new"])
                ));
            }
        }
        "apply" => {
            let what = if out["dry_run"] == true { "would change" } else { "changed" };
            for c in out["changes"].as_array().into_iter().flatten() {
                line(format!(
                    "{} {} {what}: {} -> {}",
                    s(&c["schema"]),
                    s(&c["key"]),
                    compact(&c["old"]),
                    compact(&c["new"])
                ));
            }
            let n = out["changes"].as_array().map_or(0, Vec::len);
            line(format!("({n} {what}, {} unchanged)", out["unchanged"]));
        }
        "areas" => {
            for a in out["areas"].as_array().into_iter().flatten() {
                line(format!("{:<12} {}", s(&a["name"]), s(&a["description"])));
            }
        }
        "keys-list" => {
            for b in out["bindings"].as_array().into_iter().flatten() {
                line(binding_text(b));
            }
            if out["free"] == true {
                line(format!("{} is free", s(&out["accel"])));
            }
        }
        "keys-conflicts" => {
            for c in out["conflicts"].as_array().into_iter().flatten() {
                line(format!("{}:", s(&c["accel"])));
                for b in c["bindings"].as_array().into_iter().flatten() {
                    line(format!("  {}", binding_text(b)));
                }
            }
        }
        "keys-add" => {
            let what = if out["dry_run"] == true { "would add" } else { "added" };
            line(format!(
                "{what} {} at {}: {} runs {}",
                s(&out["name"]),
                s(&out["path"]),
                s(&out["accel"]),
                s(&out["command"])
            ));
        }
        _ => line(serde_json::to_string_pretty(out).unwrap()),
    }
    text
}

fn binding_text(b: &Value) -> String {
    let accels: Vec<&str> = b["accels"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let what = match b.get("custom") {
        Some(c) => format!("custom {:?}: {}", s(&c["name"]), s(&c["command"])),
        None => format!("{} {}  # {}", s(&b["schema"]), s(&b["key"]), s(&b["summary"])),
    };
    format!("{:<28} {what}", accels.join(" "))
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
