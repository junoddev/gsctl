//! `--format text`: compact human-readable renderings of the JSON output, and
//! of errors.
//!
//! Every string that reaches the terminal goes through [`escape`]: schemas,
//! values, extension metadata and the journal are not trusted, and an ESC
//! byte in a value could otherwise drive the terminal.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::Value;

/// `s` with control characters (C0 including ESC, DEL, C1) and bidi
/// overrides shown as `\u{..}`, so they can't drive the terminal.
pub fn escape(s: &str) -> String {
    escape_with(s, false)
}

/// Like [`escape`], but keeps newlines and tabs (for descriptions).
fn escape_text(s: &str) -> String {
    escape_with(s, true)
}

fn escape_with(s: &str, keep_newlines: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let layout = keep_newlines && (c == '\n' || c == '\t');
        let bidi = matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
        if (c.is_control() && !layout) || bidi {
            let _ = write!(out, "\\u{{{:x}}}", c as u32);
        } else {
            out.push(c);
        }
    }
    out
}

/// A string field, escaped; empty if it isn't a string.
fn s(v: &Value) -> String {
    escape(v.as_str().unwrap_or(""))
}

/// A value as people read it: strings quoted (`'prefer-dark'`, `''`), lists
/// and maps in GVariant-like notation, everything escaped.
pub fn value(v: &Value) -> String {
    if *v == crate::guard::redacted() {
        return "<redacted>".into();
    }
    match v {
        Value::String(text) => quote(text),
        Value::Array(items) => format!("[{}]", items.iter().map(value).collect::<Vec<_>>().join(", ")),
        Value::Object(map) => {
            let items: Vec<String> = map.iter().map(|(k, v)| format!("{}: {}", quote(k), value(v))).collect();
            format!("{{{}}}", items.join(", "))
        }
        other => other.to_string(),
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", escape(&text.replace('\\', "\\\\").replace('\'', "\\'")))
}

/// `1 key`, `2 keys`.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn arr(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

/// Rows of cells, every cell but a row's last padded to its column's width
/// (capped, so one long value doesn't push everything right). A row's last
/// cell doesn't widen its column.
fn table(rows: &[Vec<String>]) -> Vec<String> {
    table_capped(rows, &[])
}

/// [`table`] with per-column width caps (40 for columns not listed).
fn table_capped(rows: &[Vec<String>], caps: &[usize]) -> Vec<String> {
    let width = |c: &str| c.chars().count();
    let mut widths: Vec<usize> = Vec::new();
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(row.len().saturating_sub(1)) {
            if widths.len() <= i {
                widths.push(0);
            }
            widths[i] = widths[i].max(width(cell).min(caps.get(i).copied().unwrap_or(40)));
        }
    }
    rows.iter()
        .map(|row| {
            let mut l = String::new();
            for (i, cell) in row.iter().enumerate() {
                if i + 1 < row.len() {
                    l.push_str(cell);
                    l.push_str(&" ".repeat(widths[i].saturating_sub(width(cell)) + 2));
                } else {
                    l.push_str(cell);
                }
            }
            l.trim_end().to_string()
        })
        .collect()
}

/// `schema` or `schema:path`.
fn instance(v: &Value) -> String {
    match v["path"].as_str() {
        Some(p) => format!("{}:{}", s(&v["schema"]), escape(p)),
        None => s(&v["schema"]),
    }
}

/// A guard reason from the JSON, wherever this output keeps it.
fn guard_reason(v: &Value) -> Option<String> {
    for field in ["guard_reason", "guarded", "reason"] {
        match &v[field] {
            Value::String(r) => return Some(escape(r)),
            Value::Object(o) => {
                if let Some(r) = o.get("reason").and_then(Value::as_str) {
                    return Some(escape(r));
                }
            }
            _ => {}
        }
    }
    None
}

/// `(guarded: needs --yes — <reason>)`.
fn guarded_note(reason: Option<String>) -> String {
    match reason {
        Some(r) => format!("(guarded: needs --yes — {r})"),
        None => "(guarded: needs --yes)".to_string(),
    }
}

/// The guard note for a write result whose dry run says `requires_yes`, or
/// any of whose `writes` does.
fn write_guard(w: &Value) -> Option<String> {
    if w["requires_yes"] == true {
        return Some(guarded_note(guard_reason(w)));
    }
    arr(&w["writes"]).iter().find_map(write_guard)
}

/// A range as people read it: `one of 'a', 'b'`, `flags 'x', 'y'`, `0–100`.
fn range_text(r: &Value) -> Option<String> {
    let list = |v: &Value| arr(v).iter().map(value).collect::<Vec<_>>().join(", ");
    match r["kind"].as_str()? {
        "enum" => Some(format!("one of {}", list(&r["choices"]))),
        "flags" => Some(format!("flags {}", list(&r["flags"]))),
        "range" => Some(format!("{}–{}", value(&r["min"]), value(&r["max"]))),
        _ => None,
    }
}

/// An error document (`{"error": {...}}`) as `error:` / `hint:` lines.
pub fn render_error(doc: &Value) -> String {
    let e = &doc["error"];
    let mut t = format!("error: {}\n", escape_text(e["message"].as_str().unwrap_or("unknown error")));
    for p in arr(&e["problems"]) {
        let file = p["file"].as_str().map(|f| format!("{}: ", escape(f))).unwrap_or_default();
        let what: Vec<String> = [s(&p["schema"]), s(&p["key"])].into_iter().filter(|x| !x.is_empty()).collect();
        let what = if what.is_empty() { String::new() } else { format!("{}: ", what.join(" ")) };
        let _ = writeln!(t, "  {file}{what}{}", s(&p["message"]));
    }
    for field in ["did_you_mean", "suggestions"] {
        let items: Vec<String> = match &e[field] {
            Value::String(x) => vec![escape(x)],
            Value::Array(list) => list
                .iter()
                .map(|x| match x {
                    Value::String(x) => escape(x),
                    Value::Object(_) => {
                        let parts: Vec<String> = ["schema", "key", "id", "uuid", "name"]
                            .iter()
                            .map(|f| s(&x[*f]))
                            .filter(|p| !p.is_empty())
                            .collect();
                        if parts.is_empty() { value(x) } else { parts.join(" ") }
                    }
                    other => value(other),
                })
                .collect(),
            _ => vec![],
        };
        if !items.is_empty() {
            let _ = writeln!(t, "did you mean: {}?", items.join(", "));
        }
    }
    if let Some(h) = e["hint"].as_str() {
        let _ = writeln!(t, "hint: {}", escape(h));
    }
    t
}

/// One `watch` event as a line, and whether it belongs on stderr (status
/// rather than a change). `None` for events with nothing to show.
pub fn watch_event(ev: &Value) -> Option<(String, bool)> {
    match ev["event"].as_str()? {
        "ready" => {
            Some((format!("watching {}; press Ctrl-C to stop", count_of(&ev["schemas"], "schema", "schemas")), true))
        }
        "timeout" => Some(("timed out".to_string(), true)),
        "changed" => {
            let secs = ev["time_ms"].as_i64().unwrap_or(0) / 1000;
            let time = glib::DateTime::from_unix_local(secs)
                .ok()
                .and_then(|d| d.format("%H:%M:%S").ok())
                .map_or_else(|| "--:--:--".to_string(), |t| t.to_string());
            let default = if ev["is_default"] == true { "  (default)" } else { "" };
            Some((format!("{time} {} {} = {}{default}", instance(ev), s(&ev["key"]), value(&ev["value"])), false))
        }
        _ => None,
    }
}

fn count_of(n: &Value, one: &str, many: &str) -> String {
    count(n.as_u64().unwrap_or(0) as usize, one, many)
}

/// A journal time (`2026-10-09T12:00:00Z`) in local time.
fn local_time(t: &str) -> String {
    glib::DateTime::from_iso8601(t, None)
        .ok()
        .and_then(|d| d.to_local().ok())
        .and_then(|d| d.format("%Y-%m-%d %H:%M:%S").ok())
        .map_or_else(|| escape(t), |d| d.to_string())
}

pub fn render(command: &str, out: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    match command {
        "search" => search(out, &mut lines),
        "schema" if out.get("schemas").is_some() => {
            let rows: Vec<Vec<String>> = arr(&out["schemas"])
                .iter()
                .map(|sch| {
                    let reloc = if sch["relocatable"] == true { "(relocatable)" } else { "" };
                    vec![s(&sch["id"]), reloc.to_string()]
                })
                .collect();
            lines.extend(table(&rows));
            lines.push(count(rows.len(), "schema", "schemas"));
        }
        "schema" if out.get("keys").is_some() => {
            let place = match (out["instance_path"].as_str(), out["path"].as_str()) {
                (Some(p), _) | (None, Some(p)) => escape(p),
                _ if out["relocatable"] == true => "(relocatable: pass --path for values)".to_string(),
                _ => String::new(),
            };
            lines.push(format!("{}  {place}", s(&out["id"])).trim_end().to_string());
            key_table(arr(&out["keys"]), "  ", &mut lines);
        }
        "schema" => key_detail(out, &mut lines),
        "get" => lines.push(value(&out["value"])),
        "revert" if out.get("results").is_some() => {
            for r in arr(&out["results"]) {
                lines.push(write_text(r));
            }
            let n = arr(&out["results"]).len();
            let what = if out["dry_run"] == true { "would revert" } else { "reverted" };
            lines.push(format!("{what} batch {} ({})", s(&out["reverts_batch"]), count(n, "key", "keys")));
        }
        "set" | "reset" | "revert" => lines.push(write_text(out)),
        "changed" => {
            let rows: Vec<Vec<String>> = arr(&out["changed"])
                .iter()
                .map(|c| {
                    vec![
                        instance(c),
                        s(&c["key"]),
                        format!("= {}", value(&c["value"])),
                        format!("(default {})", value(&c["default"])),
                    ]
                })
                .collect();
            lines.extend(table_capped(&rows, &[48, 32, 24]));
            lines.push(match rows.len() {
                0 => "no keys differ from their defaults".to_string(),
                n => format!("{} changed from the default", count(n, "key", "keys")),
            });
        }
        "history" => history(out, &mut lines),
        "apply" => apply(out, &mut lines),
        "export" => export(out, &mut lines),
        "areas" => {
            let rows: Vec<Vec<String>> =
                arr(&out["areas"]).iter().map(|a| vec![s(&a["name"]), s(&a["description"])]).collect();
            lines.extend(table(&rows));
        }
        "keys-list" => {
            let rows: Vec<Vec<String>> = arr(&out["bindings"]).iter().map(binding_row).collect();
            lines.extend(table(&rows));
            if out["free"] == true {
                lines.push(format!("{} is free", s(&out["accel"])));
            }
        }
        "keys-conflicts" => {
            if out["count"] == 0 {
                lines.push("no conflicts".into());
            }
            for c in arr(&out["conflicts"]) {
                lines.push(format!("{}:", s(&c["accel"])));
                let rows: Vec<Vec<String>> = arr(&c["bindings"]).iter().map(binding_row).collect();
                lines.extend(table(&rows).into_iter().map(|l| format!("  {l}")));
            }
        }
        "keys-add" | "keys-remove" => {
            let what = match (command, out["dry_run"] == true) {
                ("keys-add", true) => "would add",
                ("keys-add", false) => "added",
                (_, true) => "would remove",
                (_, false) => "removed",
            };
            let mut l = format!(
                "{what} {} at {}: {} runs {}",
                quote(out["name"].as_str().unwrap_or("")),
                s(&out["path"]),
                s(&out["accel"]),
                quote(out["command"].as_str().unwrap_or(""))
            );
            if let Some(g) = write_guard(out) {
                l.push_str(&format!("  {g}"));
            }
            lines.push(l);
        }
        "ext-list" => {
            let rows: Vec<Vec<String>> = arr(&out["extensions"])
                .iter()
                .map(|e| {
                    let on = if e["enabled"] == true { "on" } else { "off" };
                    vec![on.to_string(), s(&e["uuid"]), s(&e["name"]), format!("({})", s(&e["location"]))]
                })
                .collect();
            lines.extend(table(&rows));
            if out["user_extensions_disabled"] == true {
                lines.push("(all user extensions are disabled: org.gnome.shell disable-user-extensions)".into());
            }
        }
        "ext-info" => {
            let on = if out["enabled"] == true { "enabled" } else { "disabled" };
            lines.push(format!("{} ({on}): {}", s(&out["uuid"]), s(&out["name"])));
            if let Some(d) = out["description"].as_str() {
                lines.push(format!("  {}", escape(d.lines().next().unwrap_or(""))));
            }
            for sch in arr(&out["settings"]) {
                lines.push(format!("  {}", s(&sch["id"])));
                key_table(arr(&sch["keys"]), "    ", &mut lines);
            }
        }
        "ext-enable" => {
            let state = if out["enabled"] == true { "enabled" } else { "disabled" };
            let what = match (out["changed"] == true, out["dry_run"] == true) {
                (false, _) => "already",
                (true, true) => "would be",
                (true, false) => "now",
            };
            let mut l = format!("{} {what} {state}", s(&out["uuid"]));
            if let Some(g) = write_guard(out) {
                l.push_str(&format!("  {g}"));
            }
            lines.push(l);
        }
        "doctor" => doctor(out, &mut lines),
        _ => lines.push(escape_text(&serde_json::to_string_pretty(out).unwrap())),
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn search(out: &Value, lines: &mut Vec<String>) {
    let results = arr(&out["results"]);
    if results.is_empty() {
        lines.push("no matches; try fewer or different words".into());
        return;
    }
    let rows: Vec<Vec<String>> = results
        .iter()
        .map(|r| {
            if r["kind"] == "extension" {
                let state = if r["enabled"] == true { "on" } else { "off" };
                return vec![
                    format!("  extension {}", s(&r["uuid"])),
                    String::new(),
                    state.into(),
                    format!("# {}", s(&r["name"])),
                ];
            }
            let mark = if r["is_default"] == false { "*" } else { " " };
            let shown = if r["relocatable"] == true && r["value"].is_null() {
                "(relocatable)".to_string()
            } else {
                value(&r["value"])
            };
            let summary = s(&r["summary"]);
            let mut summary = if summary.is_empty() { summary } else { format!("# {summary}") };
            if r["inactive"] == true {
                summary.push_str("  (not used on this desktop)");
            }
            if let Some(note) = r["note"].as_str() {
                summary.push_str(&format!("  ({})", escape(note)));
            }
            vec![format!("{mark} {}", s(&r["schema"])), s(&r["key"]), shown, summary]
        })
        .collect();
    lines.extend(table_capped(&rows, &[48, 32, 16]));
    let total = out["total"].as_u64().unwrap_or(0) as usize;
    let shown = results.len();
    let more = if total > shown { format!("showing {shown} of {total}; use --limit for more") } else { String::new() };
    lines.push(match (out["partial"] == true, more.is_empty()) {
        (false, true) => count(total, "match", "matches"),
        (false, false) => format!("{} ({more})", count(total, "match", "matches")),
        (true, true) => format!("no key matches every word; {} for some words", count(total, "match", "matches")),
        (true, false) => {
            format!("no key matches every word; {} for some words ({more})", count(total, "match", "matches"))
        }
    });
}

/// Rows for a list of keys (a schema, an extension's settings): changed
/// keys are marked `*`.
fn key_table(keys: &[Value], indent: &str, lines: &mut Vec<String>) {
    let mut any_changed = false;
    let rows: Vec<Vec<String>> = keys
        .iter()
        .map(|k| {
            let changed = k["is_default"] == false;
            any_changed |= changed;
            let mut notes = Vec::new();
            let shown = match k.get("value") {
                Some(v) => {
                    if changed {
                        notes.push(format!("(default {})", value(&k["default"])));
                    }
                    format!("= {}", value(v))
                }
                None => format!("default {}", value(&k["default"])),
            };
            if let Some(r) = range_text(&k["range"]) {
                notes.push(r);
            }
            if k["guarded"].is_string() {
                notes.push(guarded_note(guard_reason(k)));
            }
            if k["writable"] == false {
                notes.push("(locked)".into());
            }
            if let Some(sum) = k["summary"].as_str() {
                notes.push(format!("# {}", escape(sum)));
            }
            let mark = if changed { "*" } else { " " };
            vec![format!("{indent}{mark} {}", s(&k["key"])), s(&k["type"]), shown, notes.join("  ")]
        })
        .collect();
    lines.extend(table_capped(&rows, &[40, 8, 20]));
    if any_changed {
        lines.push(format!("{indent}(* changed from the default)"));
    }
}

/// One key, described in full.
fn key_detail(k: &Value, lines: &mut Vec<String>) {
    lines.push(format!("{} {}", s(&k["schema"]), s(&k["key"])));
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row = |label: &str, text: String| rows.push(vec![format!("  {label}:"), text]);
    match k.get("value") {
        Some(v) if k["is_default"] == true => row("value", format!("{} (the default)", value(v))),
        Some(v) => {
            row("value", value(v));
            row("default", value(&k["default"]));
        }
        None => {
            row("value", "(relocatable: pass --path for an instance's value)".into());
            row("default", value(&k["default"]));
        }
    }
    let ty = match k["type_description"].as_str() {
        Some(d) => format!("{} ({})", s(&k["type"]), escape(d)),
        None => s(&k["type"]),
    };
    row("type", ty);
    if let Some(r) = range_text(&k["range"]) {
        row("allowed", r);
    }
    if k["guarded"].is_string() {
        row("guarded", format!("needs --yes — {}", guard_reason(k).unwrap_or_default()));
    }
    if k["writable"] == false {
        row("writable", "no (locked by the system administrator)".into());
    }
    if let Some(sum) = k["summary"].as_str() {
        row("summary", escape(sum));
    }
    lines.extend(table(&rows));
    if let Some(d) = k["description"].as_str() {
        lines.push(String::new());
        lines.push(escape_text(d));
    }
}

/// One write: `set`, `reset`, `revert`, or one result of a batch revert.
fn write_text(w: &Value) -> String {
    let target = format!("{} {}", instance(w), s(&w["key"]));
    let (old, new) = (value(&w["old"]), value(&w["new"]));
    let dry = w["dry_run"] == true;
    let op = w["op"].as_str().unwrap_or("set");
    let mut l = if w["changed"] == false {
        match op {
            "reset" => format!("{target} is already the default, {new}"),
            _ => format!("{target} is already {new}"),
        }
    } else {
        let verb = match (op, dry) {
            ("reset", true) => "would reset",
            ("reset", false) => "reset",
            ("revert", true) => "would revert",
            ("revert", false) => "reverted",
            (_, true) => "would set",
            (_, false) => "set",
        };
        // A reset can change nothing visible but drop a user value.
        match op {
            "reset" if old == new => format!("{verb} {target}: {new} (now the default)"),
            "reset" => format!("{verb} {target}: {old} -> {new} (the default)"),
            _ => format!("{verb} {target}: {old} -> {new}"),
        }
    };
    if let Some(g) = write_guard(w) {
        l.push_str(&format!("  {g}"));
    }
    if let Some(e) = w["journal_error"].as_str() {
        l.push_str(&format!("\nwarning: not recorded in the journal: {}", escape(e)));
    }
    l
}

fn binding_row(b: &Value) -> Vec<String> {
    let accels: Vec<String> = arr(&b["accels"]).iter().filter_map(Value::as_str).map(escape).collect();
    let mut what = match b.get("custom") {
        Some(c) => format!("custom {}: {}", quote(c["name"].as_str().unwrap_or("")), s(&c["command"])),
        None => s(&b["label"]).to_string(),
    };
    if b["active"] == false {
        what.push_str("  (extension off)");
    }
    if b.get("custom").is_none() && !s(&b["summary"]).is_empty() {
        what.push_str(&format!("  # {}", s(&b["summary"])));
    }
    vec![accels.join(" "), what]
}

/// History, newest first, with the entries of one batch under one header.
fn history(out: &Value, lines: &mut Vec<String>) {
    let entries = arr(&out["entries"]);
    if entries.is_empty() {
        lines.push("no recorded changes".into());
        return;
    }
    let change = |e: &Value| {
        let mut l = format!("{} {}: {} -> {}", instance(e), s(&e["key"]), value(&e["old"]), value(&e["new"]));
        if let Some(r) = e["reverts"].as_str() {
            l.push_str(&format!("  (undoes {})", escape(r)));
        }
        if let Some(by) = e["reverted_by"].as_str() {
            l.push_str(&format!("  [reverted by {}]", escape(by)));
        }
        l
    };
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        let e = &entries[i];
        match e["batch"].as_str() {
            Some(b) => {
                let n = entries[i..].iter().take_while(|x| x["batch"] == b).count();
                let group = &entries[i..i + n];
                let reverted = if group.iter().all(|x| x["reverted_by"].is_string()) { "  [reverted]" } else { "" };
                rows.push(vec![
                    local_time(e["time"].as_str().unwrap_or("")),
                    format!("batch {}", escape(b)),
                    format!("({}){reverted}", count(n, "change", "changes")),
                ]);
                for x in group {
                    rows.push(vec![String::new(), format!("  {}", s(&x["id"])), s(&x["op"]), change(x)]);
                }
                i += n;
            }
            None => {
                rows.push(vec![local_time(e["time"].as_str().unwrap_or("")), s(&e["id"]), s(&e["op"]), change(e)]);
                i += 1;
            }
        }
    }
    lines.extend(table(&rows));
    let total = out["total"].as_u64().unwrap_or(0) as usize;
    if total > entries.len() {
        lines.push(format!("(showing {} of {total}; use --limit for more)", entries.len()));
    }
}

fn apply(out: &Value, lines: &mut Vec<String>) {
    let dry = out["dry_run"] == true;
    let guards = arr(&out["requires_yes"]);
    let note = |c: &Value| {
        guards
            .iter()
            .find(|g| g["schema"] == c["schema"] && g["key"] == c["key"])
            .map(|g| guarded_note(guard_reason(g)))
            .unwrap_or_default()
    };
    let mut rows: Vec<Vec<String>> = Vec::new();
    let (set, reset) = if dry { ("would set", "would reset") } else { ("set", "reset") };
    for c in arr(&out["changes"]) {
        let change = format!("{} -> {}", value(&c["old"]), value(&c["new"]));
        rows.push(vec![set.into(), format!("{} {}", instance(c), s(&c["key"])), change, note(c)]);
    }
    for c in arr(&out["pruned"]) {
        let change = format!("{} -> {} (the default; pruned)", value(&c["old"]), value(&c["new"]));
        rows.push(vec![reset.into(), format!("{} {}", instance(c), s(&c["key"])), change, note(c)]);
    }
    lines.extend(table(&rows));

    let n = arr(&out["changes"]).len();
    let unchanged = out["unchanged"].as_u64().unwrap_or(0) as usize;
    let mut parts = vec![format!("{} {}", count(n, "key", "keys"), if dry { "would change" } else { "changed" })];
    if out["pruned"].is_array() {
        let p = arr(&out["pruned"]).len();
        parts.push(format!("{p} {}", if dry { "would be reset" } else { "reset" }));
    }
    parts.push(format!("{unchanged} already {}", if unchanged == 1 { "matches" } else { "match" }));
    let mut summary = parts.join(", ");
    if dry {
        summary = format!("dry run: {summary}");
        if !guards.is_empty() {
            let need = if guards.len() == 1 { "needs" } else { "need" };
            summary.push_str(&format!("; {} {need} --yes", count(guards.len(), "guarded key", "guarded keys")));
        }
    } else if let Some(b) = out["batch"].as_str() {
        summary.push_str(&format!(" (batch {}; undo with `gsctl revert last`)", escape(b)));
    }
    lines.push(summary);
}

fn export(out: &Value, lines: &mut Vec<String>) {
    let skipped = arr(&out["skipped_state"]).len();
    let mut notes = Vec::new();
    if skipped > 0 {
        notes.push(format!(
            "skipped {}; --include-state exports them",
            count(skipped, "app-state key", "app-state keys")
        ));
    }
    if let Some(n) = out["skipped_areas"]["remote"].as_u64() {
        notes.push(format!(
            "left out the remote area, {}; --include-remote exports it",
            count(n as usize, "key", "keys")
        ));
    }
    if let Some(files) = out["files"].as_array() {
        let rows: Vec<Vec<String>> = files
            .iter()
            .map(|f| {
                let n = f["keys"].as_u64().unwrap_or(0) as usize;
                vec![s(&f["area"]), s(&f["path"]), format!("({})", count(n, "key", "keys"))]
            })
            .collect();
        lines.extend(table(&rows));
        lines.push(match files.len() {
            0 => "nothing customised to export; no files written".to_string(),
            n => format!("wrote {}", count(n, "file", "files")),
        });
    } else {
        let names = crate::state::area_names();
        let mut by_area: BTreeMap<usize, (&str, usize)> = BTreeMap::new();
        let settings = out["settings"].as_object();
        for (inst, keys) in settings.into_iter().flatten() {
            let schema = inst.split_once(':').map_or(inst.as_str(), |(sch, _)| sch);
            for key in keys.as_object().into_iter().flatten().map(|(k, _)| k) {
                let area = crate::state::area_of(schema, key);
                let pos = names.iter().position(|a| *a == area).unwrap_or(names.len());
                by_area.entry(pos).or_insert((area, 0)).1 += 1;
            }
        }
        if by_area.is_empty() {
            lines.push("nothing customised to export".into());
        } else {
            let rows: Vec<Vec<String>> =
                by_area.values().map(|(a, n)| vec![format!("{a}:"), count(*n, "key", "keys")]).collect();
            lines.extend(table(&rows));
            notes.push("--format json prints the settings; --out DIR writes one file per area".into());
        }
    }
    lines.extend(notes.into_iter().map(|n| format!("({n})")));
}

fn doctor(out: &Value, lines: &mut Vec<String>) {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for c in arr(&out["checks"]) {
        let d = &c["detail"];
        let detail = match c["name"].as_str() {
            _ if d.is_string() => s(d),
            Some("session_mode") => format!("{} (from {})", s(&d["name"]), s(&d["source"])),
            Some("schemas") => format!(
                "{} ({} from extension dirs)",
                d["total"].as_u64().unwrap_or(0),
                d["from_extension_dirs"].as_u64().unwrap_or(0)
            ),
            _ => match d.as_object() {
                Some(o) => o.iter().map(|(k, v)| format!("{}: {}", escape(k), value(v))).collect::<Vec<_>>().join(", "),
                None => value(d),
            },
        };
        rows.push(vec![s(&c["status"]), s(&c["name"]), detail]);
        if let Some(h) = c["hint"].as_str() {
            rows.push(vec![String::new(), String::new(), format!("hint: {}", escape(h))]);
        }
    }
    lines.extend(table(&rows));
    let warnings = out["warnings"].as_u64().unwrap_or(0) as usize;
    lines.push(match (out["ok"] == true, warnings) {
        (true, 0) => "ok: gsctl can read and change this session's settings".to_string(),
        (true, w) => format!("ok, with {}", count(w, "warning", "warnings")),
        (false, _) => "problems found: changes will fail (see the errors above)".to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn escapes_control_characters() {
        assert_eq!(escape("a\x1b[31mb"), "a\\u{1b}[31mb");
        assert_eq!(escape("x\u{9b}y\u{7f}\u{202e}"), "x\\u{9b}y\\u{7f}\\u{202e}");
        assert_eq!(escape("line\nbreak"), "line\\u{a}break");
        assert_eq!(escape_text("line\nbreak\x07"), "line\nbreak\\u{7}");
        assert_eq!(escape("plain – text ✓"), "plain – text ✓");
    }

    #[test]
    fn values_are_quoted() {
        assert_eq!(value(&json!("prefer-dark")), "'prefer-dark'");
        assert_eq!(value(&json!("")), "''");
        assert_eq!(value(&json!("it's")), "'it\\'s'");
        assert_eq!(value(&json!(["a", "b"])), "['a', 'b']");
        assert_eq!(value(&json!({"k": 1})), "{'k': 1}");
        assert_eq!(value(&json!(1.0)), "1.0");
        assert_eq!(value(&json!(null)), "null");
        assert_eq!(value(&json!("\x1b]0;x")), "'\\u{1b}]0;x'");
        assert_eq!(value(&json!({"redacted": true})), "<redacted>");
    }

    #[test]
    fn tables_align_all_but_the_last_column() {
        let rows = vec![
            vec!["a".to_string(), "bb".to_string(), "x".to_string()],
            vec!["ccc".to_string(), "d".to_string(), "y".to_string()],
            vec!["header only".to_string()],
        ];
        assert_eq!(table(&rows), ["a    bb  x", "ccc  d   y", "header only"]);
    }

    #[test]
    fn ranges() {
        assert_eq!(range_text(&json!({"kind": "range", "min": 100, "max": 2500})).unwrap(), "100–2500");
        assert_eq!(range_text(&json!({"kind": "enum", "choices": ["a", "b"]})).unwrap(), "one of 'a', 'b'");
        assert!(range_text(&Value::Null).is_none());
    }

    #[test]
    fn error_lines() {
        let doc =
            json!({"error": {"code": "x", "message": "bad\x1b[2J", "hint": "try this", "did_you_mean": ["a", "b"]}});
        assert_eq!(render_error(&doc), "error: bad\\u{1b}[2J\ndid you mean: a, b?\nhint: try this\n");
    }
}
