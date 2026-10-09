//! Command implementations. Each returns the JSON document printed on stdout.

use gio::SettingsSchemaKey;
use gio::prelude::*;
use serde_json::{Value, json};

use crate::convert::{self, describe, to_json};
use crate::error::{Error, Result};
use crate::guard;
use crate::index::{Entry, Index};
use crate::journal;
use crate::search::{self, Doc};

/// `range` of a key: enum choices, flags, or min/max; `null` when any value of
/// the type is allowed.
fn range_json(key: &SettingsSchemaKey) -> Value {
    let range = key.range();
    let kind = range.child_value(0).str().unwrap_or_default().to_string();
    let detail = range.child_value(1).as_variant().unwrap();
    match kind.as_str() {
        "enum" => json!({ "kind": "enum", "choices": to_json(&detail) }),
        "flags" => json!({ "kind": "flags", "flags": to_json(&detail) }),
        "range" => {
            json!({ "kind": "range", "min": to_json(&detail.child_value(0)), "max": to_json(&detail.child_value(1)) })
        }
        _ => Value::Null,
    }
}

/// Full description of one key. `settings` is `None` for a relocatable schema
/// without a path, which has no current value.
fn key_json(entry: &Entry, key: &SettingsSchemaKey, settings: Option<&gio::Settings>) -> Value {
    let name = key.name();
    let ty = key.value_type();
    let mut out = json!({
        "schema": entry.id(),
        "key": name.as_str(),
        "type": ty.as_str(),
        "type_description": describe(&ty),
        "summary": key.summary().map(|s| s.to_string()),
        "description": key.description().map(|s| s.to_string()),
        "range": range_json(key),
        "default": to_json(&key.default_value()),
        "guarded": guard::check(&entry.id(), &name),
    });
    if let Some(s) = settings {
        let o = out.as_object_mut().unwrap();
        o.insert("value".into(), to_json(&s.value(&name)));
        o.insert("is_default".into(), s.user_value(&name).is_none().into());
        // The effective default includes vendor overrides (e.g. Ubuntu's).
        if let Some(d) = s.default_value(&name) {
            o.insert("default".into(), to_json(&d));
        }
        o.insert("writable".into(), s.is_writable(&name).into());
    }
    out
}

pub fn schema(index: &Index, schema: Option<&str>, key: Option<&str>, path: Option<&str>) -> Result<Value> {
    let Some(id) = schema else {
        let schemas: Vec<Value> = index.entries().iter().map(Entry::summary_json).collect();
        return Ok(json!({ "count": schemas.len(), "schemas": schemas }));
    };
    let entry = index.get(id)?;
    let settings = match (entry.relocatable(), path) {
        (true, None) => None,
        _ => Some(entry.settings(path)?),
    };
    if let Some(k) = key {
        let key = entry.key(k)?;
        return Ok(key_json(entry, &key, settings.as_ref()));
    }
    let keys: Vec<Value> =
        entry.keys().iter().map(|k| key_json(entry, &entry.schema.key(k), settings.as_ref())).collect();
    let mut out = entry.summary_json();
    let o = out.as_object_mut().unwrap();
    o.insert("instance_path".into(), path.into());
    o.insert("keys".into(), keys.into());
    Ok(out)
}

pub fn search(index: &Index, query: &str, limit: usize, prefixes: &[String]) -> Result<Value> {
    let mut hits = Vec::new();
    for entry in index.entries() {
        let id = entry.id();
        if !matches_prefix(&id, prefixes) {
            continue;
        }
        for name in entry.keys() {
            let key = entry.schema.key(&name);
            let summary = key.summary().map(|s| s.to_string()).unwrap_or_default();
            let description = key.description().map(|s| s.to_string()).unwrap_or_default();
            let doc = Doc { schema: &id, key: &name, summary: &summary, description: &description };
            if let Some(m) = search::score(query, &doc) {
                hits.push((m, entry, key, summary));
            }
        }
    }
    // Keys matching every word, if there are any. Otherwise fall back to keys
    // matching some words (the agent's words may not be GNOME's: "dark mode"
    // vs color-scheme), ranked by how many matched, and say so.
    let query_words = search::word_count(query);
    let partial = !hits.iter().any(|h| h.0.words == query_words);
    if !partial {
        hits.retain(|h| h.0.words == query_words);
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id().cmp(&b.1.id())).then_with(|| a.2.name().cmp(&b.2.name())));
    let total = hits.len();
    let results: Vec<Value> = hits
        .into_iter()
        .take(limit)
        .map(|(m, entry, key, summary)| {
            let name = key.name();
            let value = if entry.relocatable() {
                Value::Null
            } else {
                entry.settings(None).map(|s| to_json(&s.value(&name))).unwrap_or(Value::Null)
            };
            json!({
                "schema": entry.id(),
                "key": name.as_str(),
                "type": key.value_type().as_str(),
                "summary": summary,
                "value": value,
                "relocatable": entry.relocatable(),
                "origin": entry.origin.to_json(),
                "matched_words": m.words,
                "score": m.score,
            })
        })
        .collect();
    Ok(json!({ "query": query, "partial": partial, "total": total, "results": results }))
}

pub fn get(index: &Index, schema: &str, key: &str, path: Option<&str>) -> Result<Value> {
    let entry = index.get(schema)?;
    let k = entry.key(key)?;
    let settings = entry.settings(path)?;
    Ok(json!({
        "schema": schema,
        "path": path,
        "key": key,
        "type": k.value_type().as_str(),
        "value": to_json(&settings.value(key)),
        "is_default": settings.user_value(key).is_none(),
    }))
}

/// Why `value` is not allowed by the key's range, if it isn't.
fn range_error(key: &SettingsSchemaKey, value: &glib::Variant) -> Option<String> {
    if key.range_check(value) {
        return None;
    }
    let range = range_json(key);
    let shown = to_json(value);
    Some(match range["kind"].as_str() {
        Some("enum") => format!("{shown} is not one of the allowed values: {}", list(&range["choices"])),
        Some("flags") => format!("{shown} contains values outside the allowed flags: {}", list(&range["flags"])),
        Some("range") => format!("{shown} is outside the allowed range {}..={}", range["min"], range["max"]),
        _ => format!("{shown} is not allowed"),
    })
}

fn list(v: &Value) -> String {
    v.as_array()
        .map(|a| {
            a.iter().map(|x| x.as_str().map_or_else(|| x.to_string(), str::to_string)).collect::<Vec<_>>().join(", ")
        })
        .unwrap_or_default()
}

pub struct WriteOpts<'a> {
    pub path: Option<&'a str>,
    pub dry_run: bool,
    pub yes: bool,
}

pub fn set(index: &Index, schema: &str, key: &str, raw: &str, opts: &WriteOpts) -> Result<Value> {
    let entry = index.get(schema)?;
    let k = entry.key(key)?;
    let ty = k.value_type();
    let invalid = |message: String| Error::InvalidValue {
        message,
        details: json!({
            "schema": schema,
            "key": key,
            "input": raw,
            "type": ty.as_str(),
            "type_description": describe(&ty),
            "range": range_json(&k),
            "hint": format!("see `gsap schema {schema} {key}`; values are JSON (true, 42, \"text\", [\"a\",\"b\"]) or GVariant text"),
        }),
    };
    let new = convert::parse_input(raw, &ty).map_err(invalid)?;
    if let Some(msg) = range_error(&k, &new) {
        return Err(invalid(msg));
    }
    write(entry, key, opts, "set", new)
}

pub fn reset(index: &Index, schema: &str, key: &str, opts: &WriteOpts) -> Result<Value> {
    let entry = index.get(schema)?;
    entry.key(key)?;
    let settings = entry.settings(opts.path)?;
    let default = settings.default_value(key).unwrap_or_else(|| entry.schema.key(key).default_value());
    write(entry, key, opts, "reset", default)
}

fn write(entry: &Entry, key: &str, opts: &WriteOpts, op: &str, new: glib::Variant) -> Result<Value> {
    let schema = entry.id();
    let settings = entry.settings(opts.path)?;
    let old = settings.value(key);
    let old_user = settings.user_value(key);
    if !settings.is_writable(key) {
        return Err(Error::NotWritable { schema, key: key.to_string() });
    }
    let guarded = guard::check(&schema, key);
    let mut out = json!({
        "schema": schema,
        "path": opts.path,
        "key": key,
        "op": op,
        "old": to_json(&old),
        "new": to_json(&new),
        "changed": old != new || (op == "reset" && old_user.is_some()),
        "dry_run": opts.dry_run,
    });
    if opts.dry_run {
        out["requires_yes"] = guarded.is_some().into();
        return Ok(out);
    }
    if let Some(reason) = guarded
        && !opts.yes
    {
        return Err(Error::Guarded { schema, key: key.to_string(), reason: reason.to_string() });
    }

    match op {
        "reset" => settings.reset(key),
        _ => settings.set_value(key, &new).map_err(|e| Error::Internal(format!("GSettings refused the write: {e}")))?,
    }
    gio::Settings::sync();

    // GSettings writes are fire-and-forget: if dconf-service can't be reached,
    // the change is silently dropped and the old value comes back.
    let now = settings.value(key);
    let took = match op {
        "reset" => settings.user_value(key).is_none(),
        _ => now == new,
    };
    if !took {
        return Err(Error::Internal(format!(
            "the {op} did not take effect: {schema} {key} still reads {}. Is dconf-service reachable on the \
             session bus (DBUS_SESSION_BUS_ADDRESS)?",
            to_json(&now)
        )));
    }
    out["value"] = to_json(&now);
    let change = journal::Change {
        op,
        schema: &schema,
        path: opts.path,
        key,
        old: &old,
        old_user_value: old_user.as_ref(),
        new: &now,
    };
    match journal::record(&change) {
        Ok(id) => out["journal_id"] = id.into(),
        Err(e) => out["journal_error"] = format!("{}: {e}", journal::path().display()).into(),
    }
    Ok(out)
}

pub fn changed(index: &Index, prefixes: &[String]) -> Result<Value> {
    let mut items = Vec::new();
    for entry in index.entries() {
        let id = entry.id();
        if entry.relocatable() || !matches_prefix(&id, prefixes) {
            continue;
        }
        let settings = entry.settings(None)?;
        for name in entry.keys() {
            let Some(user) = settings.user_value(&name) else { continue };
            let default = settings.default_value(&name).unwrap_or_else(|| entry.schema.key(&name).default_value());
            if user == default {
                continue;
            }
            items.push(json!({
                "schema": id,
                "key": name,
                "type": user.type_().as_str(),
                "value": to_json(&user),
                "default": to_json(&default),
            }));
        }
    }
    Ok(json!({ "count": items.len(), "changed": items }))
}

/// `prefixes` empty, or `id` equals one of them or sits below it (`org.gnome.desktop`
/// matches `org.gnome.desktop.interface`).
fn matches_prefix(id: &str, prefixes: &[String]) -> bool {
    prefixes.is_empty()
        || prefixes.iter().any(|p| id == p || id.strip_prefix(p.as_str()).is_some_and(|rest| rest.starts_with('.')))
}
