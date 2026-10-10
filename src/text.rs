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
        "revert" if out.get("results").is_some() => {
            for r in out["results"].as_array().into_iter().flatten() {
                line(write_text(r));
            }
            let what = if out["dry_run"] == true { "would revert" } else { "reverted" };
            line(format!("({what} batch {})", s(&out["reverts_batch"])));
        }
        "set" | "reset" | "revert" => line(write_text(out)),
        "changed" => {
            for c in out["changed"].as_array().into_iter().flatten() {
                let instance = match c["path"].as_str() {
                    Some(p) => format!("{}:{p}", s(&c["schema"])),
                    None => s(&c["schema"]).to_string(),
                };
                line(format!(
                    "{instance} {} = {}  (default {})",
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
            let reset = if out["dry_run"] == true { "would reset" } else { "reset" };
            for c in out["pruned"].as_array().into_iter().flatten() {
                line(format!("{} {} {reset} (pruned): {}", s(&c["schema"]), s(&c["key"]), compact(&c["old"])));
            }
            let n = out["changes"].as_array().map_or(0, Vec::len);
            let pruned = match out["pruned"].as_array() {
                Some(p) => format!(", {} {reset}", p.len()),
                None => String::new(),
            };
            line(format!("({n} {what}{pruned}, {} unchanged)", out["unchanged"]));
        }
        "export" if out.get("files").is_some() => {
            for f in out["files"].as_array().into_iter().flatten() {
                line(format!("{:<12} {}  ({} keys)", s(&f["area"]), s(&f["path"]), f["keys"]));
            }
            let skipped = out["skipped_state"].as_array().map_or(0, Vec::len);
            line(format!("({skipped} app-state keys skipped)"));
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
            if out["count"] == 0 {
                line("no conflicts".into());
            }
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
        "keys-remove" => {
            let what = if out["dry_run"] == true { "would remove" } else { "removed" };
            line(format!(
                "{what} {} at {}: {} runs {}",
                s(&out["name"]),
                s(&out["path"]),
                s(&out["accel"]),
                s(&out["command"])
            ));
        }
        "ext-list" => {
            for e in out["extensions"].as_array().into_iter().flatten() {
                let on = if e["enabled"] == true { "on " } else { "off" };
                line(format!("{on} {:<40} {}  ({})", s(&e["uuid"]), s(&e["name"]), s(&e["location"])));
            }
            if out["user_extensions_disabled"] == true {
                line("(all user extensions are disabled: org.gnome.shell disable-user-extensions)".into());
            }
        }
        "ext-info" => {
            let on = if out["enabled"] == true { "enabled" } else { "disabled" };
            line(format!("{} ({on}): {}", s(&out["uuid"]), s(&out["name"])));
            for sch in out["settings"].as_array().into_iter().flatten() {
                for k in sch["keys"].as_array().into_iter().flatten() {
                    line(format!("  {}", key_text(k)));
                }
            }
        }
        "ext-enable" => {
            let state = if out["enabled"] == true { "enabled" } else { "disabled" };
            let what = match (out["changed"] == true, out["dry_run"] == true) {
                (false, _) => "already",
                (true, true) => "would be",
                (true, false) => "now",
            };
            line(format!("{} {what} {state}", s(&out["uuid"])));
        }
        "doctor" => {
            for c in out["checks"].as_array().into_iter().flatten() {
                line(format!("{:<5} {:<17} {}", s(&c["status"]), s(&c["name"]), compact(&c["detail"])));
                if let Some(h) = c["hint"].as_str() {
                    line(format!("      {:<17} {h}", ""));
                }
            }
            line(if out["ok"] == true { "ok".into() } else { "PROBLEMS FOUND".into() });
        }
        _ => line(serde_json::to_string_pretty(out).unwrap()),
    }
    text
}

/// One write: `set`, `reset`, `revert`, or one result of a batch revert.
fn write_text(w: &Value) -> String {
    let what = if w["dry_run"] == true { "would change" } else { "changed" };
    let what = if w["changed"] == false { "unchanged" } else { what };
    let instance = match w["path"].as_str() {
        Some(p) => format!("{}:{p}", s(&w["schema"])),
        None => s(&w["schema"]).to_string(),
    };
    // A reset can change nothing visible but drop a user value.
    let note =
        if w["op"] == "reset" && w["old"] == w["new"] && w["changed"] == true { " (now the default)" } else { "" };
    format!("{instance} {} {what}: {} -> {}{note}", s(&w["key"]), compact(&w["old"]), compact(&w["new"]))
}

fn binding_text(b: &Value) -> String {
    let accels: Vec<&str> = b["accels"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let mut what = match b.get("custom") {
        Some(c) => format!("custom {:?}: {}", s(&c["name"]), s(&c["command"])),
        None => s(&b["label"]).to_string(),
    };
    if b["active"] == false {
        what.push_str("  (extension off)");
    }
    if b.get("custom").is_none() && !s(&b["summary"]).is_empty() {
        what.push_str(&format!("  # {}", s(&b["summary"])));
    }
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
