//! Command implementations. Each returns the JSON document printed on stdout.

use std::collections::HashMap;

use gio::SettingsSchemaKey;
use gio::prelude::*;
use serde_json::{Value, json};

use crate::convert::{self, describe, to_json};
use crate::error::{Error, Result};
use crate::index::{Entry, Index};
use crate::journal;
use crate::search;
use crate::{doctor, ext, guard};

/// `range` of a key: enum choices, flags, or min/max; `null` when any value of
/// the type is allowed.
pub(crate) fn range_json(key: &SettingsSchemaKey) -> Value {
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
fn key_json(entry: &Entry, key: &SettingsSchemaKey, settings: Option<&gio::Settings>, path: Option<&str>) -> Value {
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
    });
    let guard = guard::check_entry(entry, path, &name);
    out["guarded"] = guard.map(|g| g.reason).into();
    out["executes"] = guard.is_some_and(|g| g.executes).into();
    // Says whether the summary and description come from an extension.
    out["origin"] = entry.origin.to_json();
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

/// One key in a schema listing: enough to pick a key, not to set it.
fn key_brief(entry: &Entry, key: &SettingsSchemaKey, settings: Option<&gio::Settings>) -> Value {
    let name = key.name();
    let mut out = json!({ "key": name.as_str(), "type": key.value_type().as_str() });
    if let Some(s) = settings {
        out["value"] = truncated(to_json(&s.value(&name)));
        out["is_default"] = s.user_value(&name).is_none().into();
    }
    out["summary"] = key.summary().map(|s| s.to_string()).into();
    if let Some(g) = guard::check_entry(entry, None, &name) {
        out["guarded"] = g.reason.into();
    }
    out
}

/// `schema` with the default detail: one key in full, a schema's keys briefly.
pub fn schema(index: &Index, schema: Option<&str>, key: Option<&str>, path: Option<&str>) -> Result<Value> {
    schema_with(index, schema, key, path, false)
}

/// Every schema; one schema's keys (briefly, or in full with `full`); or one
/// key in full.
pub fn schema_with(
    index: &Index,
    schema: Option<&str>,
    key: Option<&str>,
    path: Option<&str>,
    full: bool,
) -> Result<Value> {
    let Some(id) = schema else {
        let schemas: Vec<Value> = index
            .entries()
            .iter()
            .map(|e| {
                let mut s = e.summary_json();
                if search::inactive_reason(&e.id()).is_some() {
                    s["inactive"] = true.into();
                }
                s
            })
            .collect();
        return Ok(json!({ "count": schemas.len(), "schemas": schemas }));
    };
    let entry = index.get(id)?;
    let settings = match (entry.relocatable(), path) {
        (true, None) => None,
        _ => Some(entry.settings(path)?),
    };
    if let Some(k) = key {
        let key = entry.key(k)?;
        return Ok(key_json(entry, &key, settings.as_ref(), path));
    }
    let keys: Vec<Value> = entry
        .keys()
        .iter()
        .map(|k| {
            let key = entry.schema.key(k);
            if full {
                key_json(entry, &key, settings.as_ref(), path)
            } else {
                key_brief(entry, &key, settings.as_ref())
            }
        })
        .collect();
    let mut out = entry.summary_json();
    let o = out.as_object_mut().unwrap();
    if let Some(reason) = search::inactive_reason(id) {
        o.insert("inactive".into(), reason.into());
    }
    o.insert("instance_path".into(), path.into());
    o.insert("keys".into(), keys.into());
    if !full {
        o.insert(
            "detail".into(),
            format!("brief; `gsctl schema {id} <key>` or --full for type, range and default").into(),
        );
    }
    Ok(out)
}

/// Longest description in a search hit, in characters.
const DESCRIPTION_CHARS: usize = 160;
/// Longest value (as JSON text) shown in search hits and schema listings.
const VALUE_CHARS: usize = 300;
/// Slots kept for the best partial matches when full matches fill the limit.
const PARTIAL_SLOTS: usize = 3;

/// `text` with whitespace collapsed, cut to `max` characters with "…".
fn trim_text(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

/// `v`, or a marked preview of it if its JSON is longer than [`VALUE_CHARS`].
fn truncated(v: Value) -> Value {
    let text = v.to_string();
    let len = text.chars().count();
    if len <= VALUE_CHARS {
        return v;
    }
    json!({ "truncated": true, "chars": len, "preview": trim_text(&text, VALUE_CHARS) })
}

pub struct SearchOpts<'a> {
    pub limit: usize,
    pub prefixes: &'a [String],
    /// Rank schemas that don't apply to this desktop like the others.
    pub all: bool,
}

enum Item<'a> {
    Key(&'a Entry, SettingsSchemaKey),
    Ext(&'a ext::Extension),
}

/// Ranked search. Results come in this order: alias hits (`"via": "alias"`),
/// keys matching every word, the best partial matches (`"partial": true`,
/// at least a few even when full matches fill the limit), then keys of
/// schemas that don't apply to this desktop (`"inactive": true`) unless
/// `all`. Each key hit carries what `set` needs: type, range, default,
/// guard, value. Extensions match by name, description and uuid.
pub fn search(index: &Index, query: &str, opts: &SearchOpts) -> Result<Value> {
    let q = search::Query::parse(query);
    let mut items: Vec<Item> = Vec::new();
    let mut texts: Vec<[String; 5]> = Vec::new();
    for entry in index.entries() {
        let id = entry.id();
        if !matches_prefix(&id, opts.prefixes) {
            continue;
        }
        for name in entry.keys() {
            let key = entry.schema.key(&name);
            let range = range_json(&key);
            let choices =
                range.get("choices").or(range.get("flags")).map(|c| list(c).replace(',', "")).unwrap_or_default();
            let text = |s: Option<glib::GString>| s.map(|s| s.to_string()).unwrap_or_default();
            texts.push([id.clone(), name, text(key.summary()), text(key.description()), choices]);
            items.push(Item::Key(entry, key));
        }
    }
    let exts = if opts.prefixes.is_empty() { ext::installed() } else { vec![] };
    for e in &exts {
        let m = &e.metadata;
        let field = |k: &str| m[k].as_str().unwrap_or_default().to_string();
        texts.push([e.uuid.clone(), field("name"), String::new(), field("description"), String::new()]);
        items.push(Item::Ext(e));
    }
    let docs: Vec<search::Doc> = texts
        .iter()
        .map(|[schema, key, summary, description, choices]| search::Doc { schema, key, summary, description, choices })
        .collect();
    let ranked = search::rank(&q, &docs);

    // Alias hits first, then the groups described above.
    let mut alias_hits: Vec<(usize, Option<search::Match>, &str)> = Vec::new();
    for (target, note) in search::aliases(&q) {
        let search::Target::Key(schema, key) = target;
        if !matches_prefix(schema, opts.prefixes) {
            continue;
        }
        let found = items.iter().position(|it| matches!(it, Item::Key(e, k) if e.id() == schema && k.name() == key));
        if let Some(i) = found {
            let m = ranked.iter().find(|(j, _)| *j == i).map(|(_, m)| m.clone());
            alias_hits.push((i, m, note));
        }
    }
    let inactive =
        |i: usize| !opts.all && matches!(&items[i], Item::Key(e, _) if search::inactive_reason(&e.id()).is_some());
    let n_words = q.words.len();
    let rest = ranked.iter().filter(|(i, _)| !alias_hits.iter().any(|a| a.0 == *i));
    let (full, partial): (Vec<_>, Vec<_>) =
        rest.clone().filter(|(i, _)| !inactive(*i)).partition(|(_, m)| m.words() == n_words);
    let demoted: Vec<_> = rest.filter(|(i, _)| inactive(*i)).collect();
    let total = alias_hits.len() + full.len() + partial.len() + demoted.len();

    let mut chosen: Vec<(usize, Option<search::Match>, Option<&str>)> =
        alias_hits.iter().map(|(i, m, note)| (*i, m.clone(), Some(*note))).collect();
    let reserve = if full.is_empty() { 0 } else { partial.len().min(PARTIAL_SLOTS).min(opts.limit / 2) };
    let room = opts.limit.saturating_sub(chosen.len());
    chosen.extend(full.iter().take(room.saturating_sub(reserve)).map(|(i, m)| (*i, Some(m.clone()), None)));
    for group in [&partial, &demoted] {
        let room = opts.limit.saturating_sub(chosen.len());
        chosen.extend(group.iter().take(room).map(|(i, m)| (*i, Some(m.clone()), None)));
    }
    chosen.truncate(opts.limit);

    let ext_state =
        if chosen.iter().any(|(i, _, _)| matches!(items[*i], Item::Ext(_))) { ext::list(index).ok() } else { None };
    let results: Vec<Value> = chosen
        .into_iter()
        .map(|(i, m, alias)| {
            let mut hit = match &items[i] {
                Item::Key(entry, key) => key_hit(entry, key),
                Item::Ext(e) => ext_hit(e, ext_state.as_ref()),
            };
            let matched: Vec<String> = m.as_ref().map(|m| m.matched.clone()).unwrap_or_default();
            hit["matched"] = json!(matched);
            hit["matched_words"] = matched.len().into();
            if matched.len() < n_words && alias.is_none() {
                hit["partial"] = true.into();
            }
            if let Some(note) = alias {
                hit["via"] = "alias".into();
                hit["note"] = note.into();
            }
            if let Item::Key(e, _) = &items[i]
                && search::inactive_reason(&e.id()).is_some()
            {
                hit["inactive"] = true.into();
            }
            hit["score"] = m.map_or(0, |m| m.score / 100).into();
            hit
        })
        .collect();
    let mut out = json!({ "query": query, "partial": full.is_empty() && alias_hits.is_empty(), "total": total });
    if !q.ignored.is_empty() {
        out["ignored_words"] = json!(q.ignored);
    }
    out["results"] = results.into();
    Ok(out)
}

/// A key search hit: everything needed to go straight to `set`.
fn key_hit(entry: &Entry, key: &SettingsSchemaKey) -> Value {
    let name = key.name();
    let id = entry.id();
    let settings = if entry.relocatable() { None } else { entry.settings(None).ok() };
    let default = settings.as_ref().and_then(|s| s.default_value(&name)).unwrap_or_else(|| key.default_value());
    json!({
        "kind": "key",
        "schema": id,
        "key": name.as_str(),
        "type": key.value_type().as_str(),
        "summary": key.summary().map(|s| s.to_string()).unwrap_or_default(),
        "description": key.description().map(|s| trim_text(&s, DESCRIPTION_CHARS)),
        "value": settings.as_ref().map_or(Value::Null, |s| truncated(to_json(&s.value(&name)))),
        "default": truncated(to_json(&default)),
        "range": range_json(key),
        "guarded": guard::check_entry(entry, None, &name).map(|g| g.reason),
        "relocatable": entry.relocatable(),
        "origin": entry.origin.to_json(),
    })
}

fn ext_hit(e: &ext::Extension, state: Option<&Value>) -> Value {
    let listed = state
        .and_then(|s| s["extensions"].as_array())
        .and_then(|a| a.iter().find(|x| x["uuid"] == e.uuid.as_str()))
        .cloned()
        .unwrap_or(Value::Null);
    json!({
        "kind": "extension",
        "uuid": e.uuid,
        "name": e.metadata["name"],
        "description": e.metadata["description"].as_str().map(|d| trim_text(d, DESCRIPTION_CHARS)),
        "enabled": listed["enabled"],
        "enabled_by": listed["enabled_by"],
        "schemas": listed["schemas"],
        "hint": format!("gsctl ext disable {} / gsctl ext enable {}; settings: gsctl ext info {}", e.uuid, e.uuid, e.uuid),
    })
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
pub(crate) fn range_error(key: &SettingsSchemaKey, value: &glib::Variant) -> Option<String> {
    if key.range_check(value) {
        return None;
    }
    let range = range_json(key);
    let shown = to_json(value);
    Some(match range["kind"].as_str() {
        Some("enum") => format!("{shown} is not one of the allowed values: {}", list(&range["choices"])),
        Some("flags") => format!("{shown} contains values outside the allowed flags: {}", list(&range["flags"])),
        Some("range") => format!("{shown} is outside the allowed range {}–{}", range["min"], range["max"]),
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
            "hint": format!("see `gsctl schema {schema} {key}`; values are JSON (true, 42, \"text\", [\"a\",\"b\"]) or GVariant text"),
        }),
    };
    let new = convert::parse_input(raw, &ty).map_err(invalid)?;
    if let Some(msg) = range_error(&k, &new) {
        return Err(invalid(msg));
    }
    write(entry, key, opts, "set", Action::Set(new), Tags::default())
}

pub fn reset(index: &Index, schema: &str, key: &str, opts: &WriteOpts) -> Result<Value> {
    let entry = index.get(schema)?;
    entry.key(key)?;
    write(entry, key, opts, "reset", Action::Reset, Tags::default())
}

#[derive(Clone)]
pub(crate) enum Action {
    Set(glib::Variant),
    Reset,
}

/// Extra journal fields.
#[derive(Default, Clone, Copy)]
pub(crate) struct Tags<'a> {
    /// The journal id a revert undoes.
    pub reverts: Option<&'a str>,
    /// Groups the entries of one `apply` or batch revert.
    pub batch: Option<&'a str>,
}

/// Validate-free core of every write: writability, guard, dry-run, the write
/// itself, read-back verification and the journal. `op` labels the journal
/// entry.
pub(crate) fn write(entry: &Entry, key: &str, opts: &WriteOpts, op: &str, action: Action, tags: Tags) -> Result<Value> {
    let schema = entry.id();
    let settings = entry.settings(opts.path)?;
    let is_reset = matches!(action, Action::Reset);
    let new = match action {
        Action::Set(v) => v,
        Action::Reset => settings.default_value(key).unwrap_or_else(|| entry.schema.key(key).default_value()),
    };
    let old = settings.value(key);
    let old_user = settings.user_value(key);
    if !settings.is_writable(key) {
        return Err(Error::NotWritable { schema, key: key.to_string() });
    }
    let guarded = guard::check_entry(entry, opts.path, key);
    let mut out = json!({
        "schema": schema,
        "path": opts.path,
        "key": key,
        "op": op,
        "old": to_json(&old),
        "new": to_json(&new),
        "changed": old != new || (is_reset && old_user.is_some()),
        "dry_run": opts.dry_run,
    });
    if opts.dry_run {
        out["requires_yes"] = guarded.is_some().into();
        if let Some(g) = guarded {
            out["guarded"] = g.reason.into();
            out["executes"] = g.executes.into();
        }
        return Ok(out);
    }
    doctor::require_writable()?;
    if let Some(g) = guarded
        && !opts.yes
    {
        return Err(Error::Guarded { schema, key: key.to_string(), reason: g.reason.to_string() });
    }
    // Test hook (debug builds only): make the write of "<schema> <key>" fail,
    // to exercise batch rollback.
    #[cfg(debug_assertions)]
    if std::env::var("GSCTL_TEST_FAIL_WRITE").is_ok_and(|v| v == format!("{schema} {key}")) {
        return Err(Error::Internal(format!("injected write failure for {schema} {key} (GSCTL_TEST_FAIL_WRITE)")));
    }

    if is_reset {
        settings.reset(key);
    } else {
        settings.set_value(key, &new).map_err(|e| Error::Internal(format!("GSettings refused the write: {e}")))?;
    }
    gio::Settings::sync();

    // GSettings writes are fire-and-forget: if dconf-service can't be reached,
    // the change is silently dropped and the old value comes back.
    let now = settings.value(key);
    let took = if is_reset { settings.user_value(key).is_none() } else { now == new };
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
        reverts: tags.reverts,
        batch: tags.batch,
    };
    match journal::record(&change) {
        Ok(id) => out["journal_id"] = id.into(),
        Err(e) => out["journal_error"] = format!("{}: {e}", journal::path().display()).into(),
    }
    Ok(out)
}

/// One write of a batch, for [`write_batch`].
pub(crate) struct BatchItem<'a> {
    pub entry: &'a Entry,
    pub key: String,
    pub path: Option<String>,
    pub action: Action,
    pub yes: bool,
    /// For a revert, the journal id it undoes.
    pub reverts: Option<String>,
}

/// Write `items` in order, journaled with `op` in `batch`. Callers check
/// everything first (values, writability, guards, conflicts); if a write
/// still fails, the writes already made are undone, newest first, and
/// journaled as `rollback` entries that revert them (batch
/// `rollback-<batch>`). The error then carries the batch id and what was
/// rolled back.
pub(crate) fn write_batch(items: Vec<BatchItem>, op: &str, batch: Option<&str>, dry_run: bool) -> Result<Vec<Value>> {
    let mut done: Vec<(Value, &BatchItem, Option<glib::Variant>)> = Vec::new();
    for item in &items {
        let before = if dry_run { None } else { item.entry.settings(item.path.as_deref())?.user_value(&item.key) };
        let opts = WriteOpts { path: item.path.as_deref(), dry_run, yes: item.yes };
        let tags = Tags { reverts: item.reverts.as_deref(), batch };
        match write(item.entry, &item.key, &opts, op, item.action.clone(), tags) {
            Ok(out) => done.push((out, item, before)),
            Err(e) if done.is_empty() => return Err(e),
            Err(e) => return Err(roll_back(e, done, batch.unwrap_or_default())),
        }
    }
    Ok(done.into_iter().map(|(out, _, _)| out).collect())
}

fn roll_back(error: Error, done: Vec<(Value, &BatchItem, Option<glib::Variant>)>, batch: &str) -> Error {
    let rollback_batch = format!("rollback-{batch}");
    let (mut rolled_back, mut not_rolled_back) = (Vec::new(), Vec::new());
    for (out, item, before) in done.into_iter().rev() {
        let opts = WriteOpts { path: item.path.as_deref(), dry_run: false, yes: true };
        let tags = Tags { reverts: out["journal_id"].as_str(), batch: Some(&rollback_batch) };
        let action = before.map_or(Action::Reset, Action::Set);
        let summary = json!({
            "schema": out["schema"], "path": out["path"], "key": out["key"], "journal_id": out["journal_id"],
        });
        match write(item.entry, &item.key, &opts, "rollback", action, tags) {
            Ok(_) => rolled_back.push(summary),
            Err(e) => {
                let mut s = summary;
                s["error"] = e.to_string().into();
                not_rolled_back.push(s);
            }
        }
    }
    let message = if not_rolled_back.is_empty() {
        format!("{error} (batch {batch}: the {} earlier write(s) were rolled back)", rolled_back.len())
    } else {
        format!(
            "{error} (batch {batch}: {} earlier write(s) rolled back, {} could not be; see not_rolled_back)",
            rolled_back.len(),
            not_rolled_back.len()
        )
    };
    let details = json!({ "batch": batch, "rolled_back": rolled_back, "not_rolled_back": not_rolled_back });
    Error::BatchWrite { error: Box::new(error), message, details }
}

pub fn history(limit: usize, prefixes: &[String], key: Option<&str>) -> Result<Value> {
    let (entries, skipped) = journal::read_counting_skipped()
        .map_err(|e| Error::Internal(format!("cannot read {}: {e}", journal::path().display())))?;
    let undone = undone_by(&entries);
    let matching: Vec<Value> = entries
        .iter()
        .rev()
        .filter(|e| matches_prefix(e["schema"].as_str().unwrap_or_default(), prefixes))
        .filter(|e| key.is_none_or(|k| e["key"] == k))
        .map(|e| {
            let mut e = e.clone();
            if let Some(by) = e["id"].as_str().and_then(|id| undone.get(id)) {
                e["reverted_by"] = by.clone().into();
            }
            e
        })
        .collect();
    let total = matching.len();
    let shown: Vec<Value> = matching.into_iter().take(limit).collect();
    let mut out = json!({ "journal": journal::path(), "total": total, "entries": shown });
    if skipped > 0 {
        // Corrupt lines (a torn write, or edited by hand) are left out.
        out["skipped_lines"] = skipped.into();
    }
    Ok(out)
}

/// Journal entries whose change is undone, mapped to the revert that undid
/// it. An entry is undone if a revert of it is not itself undone, so after
/// undo + redo the entry counts as in effect again. Reverts are always newer
/// than what they revert, so one pass from the newest entry settles it; a
/// tampered journal pointing at a newer entry just counts as "not undone".
fn undone_by(entries: &[Value]) -> HashMap<String, String> {
    let mut reverts_of: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in entries {
        if let (Some(target), Some(id)) = (e["reverts"].as_str(), e["id"].as_str()) {
            reverts_of.entry(target).or_default().push(id);
        }
    }
    let mut undone: HashMap<String, String> = HashMap::new();
    for e in entries.iter().rev() {
        let Some(id) = e["id"].as_str() else { continue };
        let by = reverts_of.get(id).and_then(|rs| rs.iter().rev().find(|r| !undone.contains_key(**r)));
        if let Some(by) = by {
            undone.insert(id.to_string(), by.to_string());
        }
    }
    undone
}

/// Undo journal entry `id`, every entry of batch `id` (an `apply`), or `last`:
/// the newest change that isn't a revert and is still in effect (with the
/// rest of its batch), so repeated `revert last` walks back like an undo
/// stack. To redo, revert the revert by id; the redone change is then in
/// effect again, so the next `revert last` undoes it once more. A batch counts
/// as undone only when all its entries are, and only its entries still in
/// effect are reverted. Each key goes back to its previous value, or is reset
/// if it was at its default.
///
/// Every key is checked before any is written: it must still hold the
/// entry's value (else a conflict, unless `force`), be writable, and guarded
/// keys need `yes`. If a write still fails, the writes already made are
/// rolled back.
pub fn revert(index: &Index, id: &str, force: bool, dry_run: bool, yes: bool) -> Result<Value> {
    let entries = read_journal()?;
    let undone = undone_by(&entries);
    let in_effect = |e: &&Value| !e["id"].as_str().is_some_and(|i| undone.contains_key(i));
    let batch_of = |e: &Value| e["batch"].as_str().map(str::to_string);
    let rest_of_batch =
        |b: &str| -> Vec<&Value> { entries.iter().filter(|x| x["batch"] == b).filter(in_effect).collect() };
    let target: Vec<&Value> = match id {
        "last" => match entries.iter().rev().filter(in_effect).find(|e| e["reverts"].is_null()) {
            Some(e) => match batch_of(e) {
                Some(b) => rest_of_batch(&b),
                None => vec![e],
            },
            None => vec![],
        },
        _ => match entries.iter().find(|e| e["id"] == id) {
            Some(e) => vec![e],
            None if entries.iter().any(|e| e["batch"] == id) => {
                let rest = rest_of_batch(id);
                if rest.is_empty() {
                    return Err(Error::Conflict {
                        id: id.to_string(),
                        message: format!("batch {id} has already been reverted; see `gsctl history`"),
                    });
                }
                rest
            }
            None => vec![],
        },
    };
    if target.is_empty() {
        return Err(Error::EntryNotFound(id.to_string()));
    }
    let batch = match target.as_slice() {
        [single] if single["batch"].is_null() || id == single["id"] => None,
        _ => batch_of(target[0]),
    };

    // Undo in reverse order, after checking every entry.
    let prepared = target.iter().rev().map(|e| prepare_revert(index, e, force)).collect::<Result<Vec<_>>>()?;
    if !dry_run {
        doctor::require_writable()?;
        if !yes {
            for p in prepared.iter().filter(|p| !p.targeted) {
                if let Some(reason) =
                    guard::check_entry(p.item.entry, p.item.path.as_deref(), &p.item.key).map(|g| g.reason)
                {
                    let (schema, key) = (p.item.entry.id(), p.item.key.clone());
                    return Err(Error::Guarded { schema, key, reason: reason.to_string() });
                }
            }
        }
    }

    let run_batch = batch.as_ref().map(|b| format!("revert-{b}"));
    let items: Vec<BatchItem> = prepared.into_iter().map(|p| BatchItem { yes: yes || p.targeted, ..p.item }).collect();
    let mut results = write_batch(items, "revert", run_batch.as_deref(), dry_run)?;
    for (out, e) in results.iter_mut().zip(target.iter().rev()) {
        out["reverts"] = e["id"].clone();
    }
    match batch {
        None => Ok(results.pop().unwrap()),
        Some(b) => Ok(json!({ "reverts_batch": b, "batch": run_batch, "dry_run": dry_run, "results": results })),
    }
}

struct PreparedRevert<'a> {
    item: BatchItem<'a>,
    /// Turns one extension off: one uuid removed from `enabled-extensions` or
    /// added to `disabled-extensions`, as `ext disable` does without `--yes`,
    /// so the revert doesn't need it either. Judged from the values, never
    /// from the journal's `op`, which could be forged.
    targeted: bool,
}

const SHELL_SCHEMA: &str = "org.gnome.shell";
const EXTENSION_LISTS: [&str; 2] = ["enabled-extensions", "disabled-extensions"];

/// `a` and `b` are string lists that differ by exactly one item added or
/// removed, order otherwise kept.
fn one_item_apart(a: &glib::Variant, b: &glib::Variant) -> bool {
    let (Some(a), Some(b)) = (a.get::<Vec<String>>(), b.get::<Vec<String>>()) else { return false };
    let (long, short) = if a.len() > b.len() { (a, b) } else { (b, a) };
    long.len() == short.len() + 1
        && (0..long.len()).any(|i| {
            let mut l = long.clone();
            l.remove(i);
            l == short
        })
}

fn prepare_revert<'a>(index: &'a Index, e: &Value, force: bool) -> Result<PreparedRevert<'a>> {
    let id = e["id"].as_str().unwrap_or_default();
    let field =
        |name: &str| e[name].as_str().ok_or_else(|| Error::Internal(format!("journal entry {id} has no {name}")));
    let (schema, key) = (field("schema")?, field("key")?);
    if e["redacted"] == true {
        return Err(Error::Usage(format!(
            "journal entry {id} changed a secret ({schema} {key}); its values aren't stored, so it can't be \
             reverted. Set the key again instead"
        )));
    }
    let path = e["path"].as_str();
    let entry = index.get(schema)?;
    let k = entry.key(key)?;
    let ty = k.value_type();
    let parse = |text: &str| {
        convert::parse_finite(&ty, text).map_err(|err| {
            Error::Internal(format!("journal entry {id}: cannot parse {text:?} as {}: {err}", ty.as_str()))
        })
    };

    let expected = parse(field("new_gvariant")?)?;
    let settings = entry.settings(path)?;
    let current = settings.value(key);
    if !settings.is_writable(key) {
        return Err(Error::NotWritable { schema: schema.to_string(), key: key.to_string() });
    }
    if current != expected && !force {
        return Err(Error::Conflict {
            id: id.to_string(),
            message: format!(
                "{schema} {key} has changed since entry {id}: it is {} now, not {}. Re-run with --force to revert anyway",
                to_json(&current),
                to_json(&expected)
            ),
        });
    }

    let action = match e["old_user_value"].as_str() {
        None => Action::Reset,
        Some(text) => {
            let v = parse(text)?;
            if let Some(msg) = range_error(&k, &v) {
                // The schema changed since the entry was written.
                return Err(Error::InvalidValue {
                    message: format!("cannot revert entry {id}: {msg}"),
                    details: json!({ "schema": schema, "key": key, "range": range_json(&k) }),
                });
            }
            Action::Set(v)
        }
    };
    let targeted = path.is_none() && schema == SHELL_SCHEMA && EXTENSION_LISTS.contains(&key) && {
        let after = match &action {
            Action::Set(v) => v.clone(),
            Action::Reset => settings.default_value(key).unwrap_or_else(|| k.default_value()),
        };
        // Only a revert that turns one extension off: re-enabling runs its
        // code, so that needs --yes like `ext enable`.
        let len = |v: &glib::Variant| v.get::<Vec<String>>().map_or(0, |l| l.len());
        let turns_off =
            if key == "enabled-extensions" { len(&after) < len(&current) } else { len(&after) > len(&current) };
        turns_off && one_item_apart(&current, &after)
    };
    let item = BatchItem {
        entry,
        key: key.to_string(),
        path: path.map(str::to_string),
        action,
        yes: false,
        reverts: Some(id.to_string()),
    };
    Ok(PreparedRevert { item, targeted })
}

fn read_journal() -> Result<Vec<Value>> {
    journal::read().map_err(|e| Error::Internal(format!("cannot read {}: {e}", journal::path().display())))
}

/// Customised keys, without app state (counters, timestamps, window
/// geometry: export's classifier) unless `include_state`, in which case
/// state entries are marked with `"state": reason`.
pub fn changed(index: &Index, prefixes: &[String], include_state: bool) -> Result<Value> {
    let fixed = index.entries().iter().filter(|e| !e.relocatable()).map(|e| (e, None));
    let listed = index.listed_instances().into_iter().map(|(e, p)| (e, Some(p)));
    let mut items = Vec::new();
    let mut hidden = 0;
    for (entry, path) in fixed.chain(listed) {
        let id = entry.id();
        if !matches_prefix(&id, prefixes) {
            continue;
        }
        let settings = entry.settings(path.as_deref())?;
        for name in entry.keys() {
            let Some(user) = settings.user_value(&name) else { continue };
            let default = settings.default_value(&name).unwrap_or_else(|| entry.schema.key(&name).default_value());
            if user == default {
                continue;
            }
            let state = crate::state::state_reason(&id, &name);
            if state.is_some() && !include_state {
                hidden += 1;
                continue;
            }
            let mut item = json!({
                "schema": id,
                "path": path,
                "key": name,
                "type": user.type_().as_str(),
                "value": to_json(&user),
                "default": to_json(&default),
            });
            if let Some(reason) = state {
                item["state"] = reason.into();
            }
            items.push(item);
        }
    }
    let mut out = json!({ "count": items.len() });
    if !include_state {
        out["hidden_state"] = hidden.into();
    }
    out["changed"] = items.into();
    Ok(out)
}

/// Split a single `schema.key` or `/dconf/path/key` argument into schema,
/// key and (for a listed relocatable instance) path. `None` if `arg` is a
/// schema id, or can't be split unambiguously.
pub fn split_target(index: &Index, arg: &str) -> Option<(String, String, Option<String>)> {
    if let Some(rest) = arg.strip_prefix('/') {
        let (dir, key) = rest.rsplit_once('/')?;
        let dir = format!("/{dir}/");
        if let Some(e) = index.entries().iter().find(|e| e.schema.path().as_deref() == Some(dir.as_str())) {
            return e.schema.has_key(key).then(|| (e.id(), key.to_string(), None));
        }
        return index
            .listed_instances()
            .into_iter()
            .find(|(e, p)| *p == dir && e.schema.has_key(key))
            .map(|(e, p)| (e.id(), key.to_string(), Some(p)));
    }
    if index.get(arg).is_ok() {
        return None;
    }
    // Key names have no dots, so the last one splits schema from key.
    let (schema, key) = arg.rsplit_once('.')?;
    index.get(schema).ok()?;
    Some((schema.to_string(), key.to_string(), None))
}

/// `(schema, key, path)` from `SCHEMA KEY`, or from one `schema.key` or
/// `/dconf/path/key` argument. A path from the argument and `--path` must agree.
pub fn target(
    index: &Index,
    first: &str,
    key: Option<&str>,
    path: Option<&str>,
) -> Result<(String, String, Option<String>)> {
    if let Some(k) = key {
        return Ok((first.to_string(), k.to_string(), path.map(str::to_string)));
    }
    match split_target(index, first) {
        Some((s, k, p)) => match (p, path) {
            (Some(a), Some(b)) if a != b => Err(Error::Usage(format!("{first} is at {a}, but --path says {b}"))),
            (p, given) => Ok((s, k, p.or(given.map(str::to_string)))),
        },
        None if index.get(first).is_ok() => Err(Error::Usage(format!(
            "{first} is a schema: give SCHEMA KEY, or one schema.key argument, then any VALUE"
        ))),
        None if first.starts_with('/') => {
            Err(Error::Usage(format!("no key at dconf path {first}; try `gsctl search <words>` to find it")))
        }
        None => Err(Error::SchemaNotFound(first.to_string())),
    }
}

/// `e` with `did_you_mean` suggestions when it is a not-found error: close
/// names by edit distance, plus the best search hits.
pub fn with_suggestions(e: Error) -> Error {
    if !matches!(e, Error::SchemaNotFound(_) | Error::KeyNotFound { .. } | Error::ExtensionNotFound(_)) {
        return e;
    }
    let Ok(index) = Index::load() else { return e };
    let (did_you_mean, elsewhere) = suggest(&index, &e);
    if did_you_mean.is_empty() && elsewhere.is_empty() {
        return e;
    }
    Error::Suggest { inner: Box::new(e), did_you_mean, elsewhere }
}

/// `(did_you_mean, elsewhere)`: close names, and for a key, the other
/// schemas that have a key of that exact name.
fn suggest(index: &Index, e: &Error) -> (Vec<String>, Vec<String>) {
    // Search hits fill in only when no name is close, and only those
    // matching every word.
    let top_hits = |query: &str, prefixes: &[String]| -> Vec<Value> {
        let opts = SearchOpts { limit: 5, prefixes, all: false };
        let hits = search(index, query, &opts).ok().and_then(|v| v["results"].as_array().cloned());
        hits.unwrap_or_default().into_iter().filter(|h| h.get("partial").is_none()).take(3).collect()
    };
    let add = |out: &mut Vec<String>, s: &str| {
        if !out.iter().any(|o| o == s) && out.len() < 5 {
            out.push(s.to_string());
        }
    };
    match e {
        Error::SchemaNotFound(id) => {
            let ids: Vec<String> = index.entries().iter().map(Entry::id).collect();
            let mut out = search::closest(id, ids.iter().map(String::as_str), 3);
            for hit in if out.is_empty() { top_hits(id, &[]) } else { vec![] } {
                if let Some(s) = hit["schema"].as_str() {
                    add(&mut out, s);
                }
            }
            (out, vec![])
        }
        Error::KeyNotFound { schema, key } => {
            let Ok(entry) = index.get(schema) else { return (vec![], vec![]) };
            let keys = entry.keys();
            let mut out = search::closest(key, keys.iter().map(String::as_str), 3);
            for hit in if out.is_empty() { top_hits(key, std::slice::from_ref(schema)) } else { vec![] } {
                if let Some(k) = hit["key"].as_str()
                    && hit["schema"] == schema.as_str()
                {
                    add(&mut out, k);
                }
            }
            let elsewhere = index
                .entries()
                .iter()
                .filter(|e| e.schema.has_key(key) && search::inactive_reason(&e.id()).is_none())
                .map(Entry::id)
                .take(5)
                .collect();
            (out, elsewhere)
        }
        Error::ExtensionNotFound(uuid) => {
            let exts = ext::installed();
            let mut out = search::closest(uuid, exts.iter().map(|x| x.uuid.as_str()), 3);
            let lower = uuid.to_lowercase();
            for x in &exts {
                let name = x.metadata["name"].as_str().unwrap_or_default().to_lowercase();
                let stem = x.uuid.split('@').next().unwrap_or_default();
                // Ubuntu Dock's settings are dash-to-dock's.
                let by_schema = lower.len() >= 3 && x.schemas(index).iter().any(|s| s.to_lowercase().contains(&lower));
                if name.contains(&lower) || (!stem.is_empty() && lower.contains(stem)) || by_schema {
                    add(&mut out, &x.uuid);
                }
            }
            (out, vec![])
        }
        _ => (vec![], vec![]),
    }
}

/// `prefixes` empty, or `id` equals one of them or sits below it (`org.gnome.desktop`
/// matches `org.gnome.desktop.interface`).
pub(crate) fn matches_prefix(id: &str, prefixes: &[String]) -> bool {
    prefixes.is_empty()
        || prefixes.iter().any(|p| id == p || id.strip_prefix(p.as_str()).is_some_and(|rest| rest.starts_with('.')))
}
