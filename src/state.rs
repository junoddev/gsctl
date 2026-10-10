//! `export` and `apply`: desktop settings as per-area JSON state files.
//!
//! A state file holds only keys that differ from their defaults:
//!
//! ```json
//! {
//!   "gsctl": 1,
//!   "area": "interface",
//!   "settings": {
//!     "org.gnome.desktop.interface": { "color-scheme": "prefer-dark" },
//!     "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/.../custom0/": { "name": "Terminal" }
//!   }
//! }
//! ```
//!
//! Relocatable instances are written `schema:path`, like `gsettings`. Values
//! are JSON; a value that JSON can't round-trip exactly (a `v` holding a
//! `uint32`, say) is written as `{"$gvariant": "<typed GVariant text>"}`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use gio::prelude::*;
use glib::Variant;
use serde_json::{Map, Value, json};

use crate::commands::{Action, BatchItem, range_error, write_batch};
use crate::convert::{from_json, to_json};
use crate::error::{Error, Result};
use crate::index::{Entry, Index};
use crate::journal;
use crate::{doctor, guard};

pub const FORMAT_VERSION: u64 = 1;
const GVARIANT: &str = "$gvariant";

/// Schema patterns: an exact id, or `prefix.*` for the schemas below `prefix`.
/// `keys` limits a rule to some keys of the schema.
struct Rule {
    schema: &'static str,
    keys: Option<&'static [&'static str]>,
}

const fn all(schema: &'static str) -> Rule {
    Rule { schema, keys: None }
}

pub struct Area {
    pub name: &'static str,
    pub description: &'static str,
    rules: &'static [Rule],
}

const SHELL_EXTENSION_KEYS: &[&str] =
    &["enabled-extensions", "disabled-extensions", "disable-user-extensions", "disable-extension-version-validation"];

pub const CUSTOM_KEYBINDING_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const MEDIA_KEYS_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";

/// Areas in match order: the first area with a matching rule owns the key;
/// anything unmatched is `other`.
pub const AREAS: &[Area] = &[
    Area {
        name: "extensions",
        description: "Enabled extensions and every extension's own settings",
        rules: &[
            Rule { schema: "org.gnome.shell", keys: Some(SHELL_EXTENSION_KEYS) },
            all("org.gnome.shell.extensions.*"),
        ],
    },
    Area {
        name: "keybindings",
        description: "Keyboard shortcuts, including custom ones",
        rules: &[
            all("org.gnome.desktop.wm.keybindings"),
            all("org.gnome.shell.keybindings"),
            all("org.gnome.mutter.keybindings"),
            all("org.gnome.mutter.wayland.keybindings"),
            all(MEDIA_KEYS_SCHEMA),
            all(CUSTOM_KEYBINDING_SCHEMA),
        ],
    },
    Area {
        name: "interface",
        description: "Appearance, fonts, background, window manager and workspaces",
        rules: &[
            all("org.gnome.desktop.interface"),
            all("org.gnome.desktop.background"),
            all("org.gnome.desktop.wm.preferences"),
            all("org.gnome.mutter"),
            all("org.gnome.shell.ubuntu"),
            all("org.gnome.desktop.sound"),
        ],
    },
    Area {
        name: "shell",
        description: "GNOME Shell: dock favourites, app folders, switchers",
        rules: &[
            all("org.gnome.shell"),
            all("org.gnome.shell.app-switcher"),
            all("org.gnome.shell.window-switcher"),
            all("org.gnome.desktop.app-folders"),
            all("org.gnome.desktop.app-folders.*"),
        ],
    },
    Area {
        name: "input",
        description: "Keyboard layouts, mouse, touchpad and keyboard settings",
        rules: &[all("org.gnome.desktop.input-sources"), all("org.gnome.desktop.peripherals.*")],
    },
    Area {
        name: "power",
        description: "Power, idle, screen blanking and locking",
        rules: &[
            all("org.gnome.settings-daemon.plugins.power"),
            all("org.gnome.desktop.session"),
            all("org.gnome.desktop.screensaver"),
            all("org.gnome.desktop.lockdown"),
        ],
    },
    Area {
        name: "remote",
        description: "Remote desktop (RDP/VNC)",
        rules: &[all("org.gnome.desktop.remote-desktop"), all("org.gnome.desktop.remote-desktop.*")],
    },
    Area {
        name: "a11y",
        description: "Accessibility",
        rules: &[all("org.gnome.desktop.a11y"), all("org.gnome.desktop.a11y.*")],
    },
];

pub const OTHER: &str = "other";

fn rule_matches(rule: &Rule, schema: &str, key: &str) -> bool {
    let schema_ok = match rule.schema.strip_suffix(".*") {
        Some(base) => schema.strip_prefix(base).is_some_and(|rest| rest.starts_with('.')),
        None => schema == rule.schema,
    };
    schema_ok && rule.keys.is_none_or(|keys| keys.contains(&key))
}

/// The area that owns `schema key`.
pub fn area_of(schema: &str, key: &str) -> &'static str {
    AREAS.iter().find(|a| a.rules.iter().any(|r| rule_matches(r, schema, key))).map_or(OTHER, |a| a.name)
}

pub fn area_names() -> Vec<&'static str> {
    AREAS.iter().map(|a| a.name).chain([OTHER]).collect()
}

/// A usage error unless every name in `areas` is a known area.
fn check_areas<'a>(areas: impl IntoIterator<Item = &'a str>, context: &str) -> Result<()> {
    let names = area_names();
    for a in areas {
        if !names.contains(&a) {
            return Err(Error::Usage(format!("{context}unknown area {a:?}; areas are: {}", names.join(", "))));
        }
    }
    Ok(())
}

/// A state file's `area`: a name, a list of names, or absent.
fn file_areas(file: &Path, area: &Value) -> Result<Vec<String>> {
    let bad = || Error::Usage(format!("{}: \"area\" must be an area name or a list of them", file.display()));
    let areas: Vec<String> = match area {
        Value::Null => vec![],
        Value::String(a) => vec![a.clone()],
        Value::Array(list) => {
            list.iter().map(|a| a.as_str().map(str::to_string).ok_or_else(bad)).collect::<Result<_>>()?
        }
        _ => return Err(bad()),
    };
    check_areas(areas.iter().map(String::as_str), &format!("{}: ", file.display()))?;
    Ok(areas)
}

/// Why `schema key` looks like machine or app state rather than a preference,
/// if it does. State is left out of exports unless asked for.
pub fn state_reason(schema: &str, key: &str) -> Option<&'static str> {
    const STATE_SCHEMAS: &[&str] = &["com.ubuntu.update-manager", "com.ubuntu.update-notifier"];
    const WINDOW_GEOMETRY: &[&str] =
        &["window-size", "window-position", "window-state", "window-width", "window-height", "window-maximized"];
    if STATE_SCHEMAS.contains(&schema) {
        return Some("update bookkeeping");
    }
    if schema == "org.gnome.desktop.notifications" && key == "application-children" {
        return Some("list of apps that have sent notifications");
    }
    if key.contains("migrated") {
        return Some("migration marker");
    }
    if key.ends_with("-time") || key.ends_with("-timestamp") || key.contains("launch-count") {
        return Some("timestamp or counter");
    }
    if key.ends_with("last-shown-version") || key.starts_with("last-version") {
        return Some("version bookkeeping");
    }
    if WINDOW_GEOMETRY.iter().any(|w| key.contains(w)) {
        return Some("window geometry");
    }
    if key == "tls-cert" || key == "tls-key" {
        return Some("machine-specific certificate path");
    }
    if schema == "org.gnome.desktop.peripherals.keyboard" && key == "numlock-state" {
        return Some("remembered NumLock state");
    }
    if key == "overridden-settings" {
        return Some("an extension's saved copy of the settings it overrides");
    }
    if key.ends_with("-selected") {
        return Some("UI selection");
    }
    None
}

/// JSON for a state file: plain JSON when it converts back exactly, else
/// typed GVariant text.
fn encode(v: &Variant) -> Value {
    let j = to_json(v);
    match from_json(&j, v.type_()) {
        Ok(back) if &back == v => j,
        _ => json!({ GVARIANT: v.print(true).as_str() }),
    }
}

fn decode(j: &Value, ty: &glib::VariantTy) -> std::result::Result<Variant, String> {
    if let Value::Object(map) = j
        && map.len() == 1
        && let Some(text) = map.get(GVARIANT)
    {
        let text = text.as_str().ok_or("$gvariant must be a string")?;
        return crate::convert::parse_finite(ty, text).map_err(|e| format!("invalid GVariant text {text:?}: {e}"));
    }
    from_json(j, ty)
}

struct Item {
    area: &'static str,
    instance: String,
    key: String,
    value: Value,
}

/// Non-default keys of one schema instance.
fn customised(entry: &Entry, path: Option<&str>) -> Result<Vec<(String, Variant)>> {
    let settings = entry.settings(path)?;
    let mut out = Vec::new();
    for name in entry.keys() {
        let Some(user) = settings.user_value(&name) else { continue };
        let default = settings.default_value(&name).unwrap_or_else(|| entry.schema.key(&name).default_value());
        if user != default {
            out.push((name, user));
        }
    }
    Ok(out)
}

pub struct ExportOpts<'a> {
    pub areas: &'a [String],
    pub out_dir: Option<&'a Path>,
    pub include_state: bool,
}

pub fn export(index: &Index, opts: &ExportOpts) -> Result<Value> {
    check_areas(opts.areas.iter().map(String::as_str), "")?;
    let wanted = |area: &str| opts.areas.is_empty() || opts.areas.iter().any(|a| a == area);

    let mut items = Vec::new();
    let mut skipped = Vec::new();
    let mut add = |entry: &Entry, path: Option<&str>, items: &mut Vec<Item>| -> Result<()> {
        let id = entry.id();
        for (key, value) in customised(entry, path)? {
            let area = area_of(&id, &key);
            if !wanted(area) {
                continue;
            }
            if !opts.include_state
                && let Some(reason) = state_reason(&id, &key)
            {
                skipped.push(json!({ "schema": id, "key": key, "reason": reason }));
                continue;
            }
            let instance = match path {
                Some(p) => format!("{id}:{p}"),
                None => id.clone(),
            };
            items.push(Item { area, instance, key, value: encode(&value) });
        }
        Ok(())
    };
    for entry in index.entries().iter().filter(|e| !e.relocatable()) {
        add(entry, None, &mut items)?;
    }
    for (entry, path) in index.listed_instances() {
        add(entry, Some(&path), &mut items)?;
    }

    let mut by_area: BTreeMap<&str, Vec<&Item>> = BTreeMap::new();
    for item in &items {
        by_area.entry(item.area).or_default().push(item);
    }

    match opts.out_dir {
        None => {
            let areas: Vec<&str> = by_area.keys().copied().collect();
            let mut doc = document(Value::from(areas), items.iter());
            if !skipped.is_empty() {
                doc["skipped_state"] = skipped.into();
            }
            Ok(doc)
        }
        Some(dir) => {
            fs::create_dir_all(dir).map_err(|e| Error::Internal(format!("cannot create {}: {e}", dir.display())))?;
            let mut files = Vec::new();
            for (area, items) in &by_area {
                let path = dir.join(format!("{area}.json"));
                let doc = document(Value::from(*area), items.iter().copied());
                let text = serde_json::to_string_pretty(&doc).unwrap() + "\n";
                fs::write(&path, text).map_err(|e| Error::Internal(format!("cannot write {}: {e}", path.display())))?;
                files.push(json!({ "area": area, "path": path, "keys": items.len() }));
            }
            Ok(json!({ "files": files, "skipped_state": skipped }))
        }
    }
}

/// A state file document with settings sorted by instance, then key.
fn document<'a>(area: Value, items: impl Iterator<Item = &'a Item>) -> Value {
    let mut sorted: BTreeMap<&str, BTreeMap<&str, &Value>> = BTreeMap::new();
    for i in items {
        sorted.entry(&i.instance).or_default().insert(&i.key, &i.value);
    }
    let settings: Map<String, Value> = sorted
        .into_iter()
        .map(|(inst, keys)| {
            let keys: Map<String, Value> = keys.into_iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
            (inst.to_string(), Value::Object(keys))
        })
        .collect();
    json!({ "gsctl": FORMAT_VERSION, "area": area, "settings": settings })
}

pub struct ApplyOpts<'a> {
    pub files: &'a [PathBuf],
    pub areas: &'a [String],
    /// Also reset customised keys in the files' areas that the files don't list.
    pub prune: bool,
    pub dry_run: bool,
    pub yes: bool,
}

struct Planned<'a> {
    entry: &'a Entry,
    path: Option<String>,
    key: String,
    value: Variant,
    old: Variant,
    /// A `--prune` reset: `value` is the default.
    reset: bool,
}

/// Apply state files: validate everything, then write only the keys that
/// differ. Nothing is written if any value is invalid or a guarded key would
/// change without `--yes`. With `prune`, customised keys in the files' areas
/// that no file lists are reset too (app state is never pruned).
pub fn apply(index: &Index, opts: &ApplyOpts) -> Result<Value> {
    let mut problems = Vec::new();
    let mut planned: Vec<Planned> = Vec::new();
    let mut unchanged = 0;
    let mut listed: Vec<(String, Option<String>, String)> = Vec::new();
    let mut all_file_areas: Vec<String> = Vec::new();
    let mut seen: BTreeMap<(String, Option<String>, String), &Path> = BTreeMap::new();
    check_areas(opts.areas.iter().map(String::as_str), "--area: ")?;

    for file in opts.files {
        let text =
            fs::read_to_string(file).map_err(|e| Error::Usage(format!("cannot read {}: {e}", file.display())))?;
        let doc: Value = serde_json::from_str(&text)
            .map_err(|e| Error::Usage(format!("{} is not valid JSON: {e}", file.display())))?;
        if doc["gsctl"].as_u64() != Some(FORMAT_VERSION) {
            return Err(Error::Usage(format!("{} is not a gsctl v{FORMAT_VERSION} state file", file.display())));
        }
        let Some(settings) = doc["settings"].as_object() else {
            return Err(Error::Usage(format!("{} has no \"settings\" object", file.display())));
        };
        let areas = file_areas(file, &doc["area"])?;
        if opts.prune {
            if areas.is_empty() {
                return Err(Error::Usage(format!(
                    "{} names no \"area\", so --prune can't tell which keys it owns",
                    file.display()
                )));
            }
            all_file_areas.extend(areas);
        }
        for (instance, keys) in settings {
            let (schema, path) = match instance.split_once(':') {
                Some((s, p)) => (s, Some(p)),
                None => (instance.as_str(), None),
            };
            let mut problem = |key: &str, err: &Error| {
                problems.push(json!({
                    "file": file, "schema": schema, "path": path, "key": key,
                    "code": err.code(), "exit_code": err.exit_code(), "message": err.to_string(),
                }));
            };
            let entry = match index.get(schema) {
                Ok(e) => e,
                Err(err) => {
                    problem("", &err);
                    continue;
                }
            };
            let settings = match entry.settings(path) {
                Ok(s) => s,
                Err(err) => {
                    problem("", &err);
                    continue;
                }
            };
            let Some(keys) = keys.as_object() else {
                problem("", &Error::Usage(format!("{instance} must map key names to values")));
                continue;
            };
            for (key, j) in keys {
                // One write per key: two files setting it is a mistake, not a merge.
                let id = (schema.to_string(), path.map(str::to_string), key.clone());
                if let Some(first) = seen.get(&id) {
                    let msg = format!("{instance} {key} is set in both {} and {}", first.display(), file.display());
                    problem(key, &Error::Usage(msg));
                    continue;
                }
                seen.insert(id, file);
                listed.push((schema.to_string(), path.map(str::to_string), key.clone()));
                if !opts.areas.is_empty() && !opts.areas.iter().any(|a| a == area_of(schema, key)) {
                    continue;
                }
                let k = match entry.key(key) {
                    Ok(k) => k,
                    Err(err) => {
                        problem(key, &err);
                        continue;
                    }
                };
                let ty = k.value_type();
                let value = match decode(j, &ty) {
                    Ok(v) => v,
                    Err(message) => {
                        problem(key, &Error::InvalidValue { message, details: Value::Null });
                        continue;
                    }
                };
                if let Some(message) = range_error(&k, &value) {
                    problem(key, &Error::InvalidValue { message, details: Value::Null });
                    continue;
                }
                let old = settings.value(key);
                if old == value {
                    unchanged += 1;
                    continue;
                }
                if !settings.is_writable(key) {
                    problem(key, &Error::NotWritable { schema: schema.to_string(), key: key.clone() });
                    continue;
                }
                planned.push(Planned {
                    entry,
                    path: path.map(str::to_string),
                    key: key.clone(),
                    value,
                    old,
                    reset: false,
                });
            }
        }
    }

    let pruned = if opts.prune { plan_prune(index, opts, &all_file_areas, &listed, &mut problems)? } else { vec![] };

    if !problems.is_empty() {
        let exit = problems[0]["exit_code"].as_i64().unwrap_or(4) as i32;
        return Err(Error::Batch {
            message: format!("{} problem(s) in the state files; nothing was written", problems.len()),
            exit,
            problems: problems.into(),
        });
    }

    let guarded: Vec<Value> = planned
        .iter()
        .chain(&pruned)
        .filter_map(|p| {
            guard::check(&p.entry.id(), &p.key).map(|r| json!({ "schema": p.entry.id(), "key": p.key, "reason": r }))
        })
        .collect();
    let change_json = |p: &Planned| json!({ "schema": p.entry.id(), "path": p.path, "key": p.key, "old": to_json(&p.old), "new": to_json(&p.value) });
    let prune_json = |list: Vec<Value>| if opts.prune { Value::from(list) } else { Value::Null };

    if opts.dry_run {
        let changes: Vec<Value> = planned.iter().map(change_json).collect();
        let pruned = prune_json(pruned.iter().map(change_json).collect());
        return Ok(json!({
            "dry_run": true, "changes": changes, "pruned": pruned, "unchanged": unchanged, "requires_yes": guarded,
        }));
    }
    // Even with nothing to change: in a sandbox the "unchanged" comparison
    // is against defaults, not the real settings.
    doctor::require_writable()?;
    if !guarded.is_empty() && !opts.yes {
        let first = &guarded[0];
        return Err(Error::Guarded {
            schema: first["schema"].as_str().unwrap_or_default().to_string(),
            key: first["key"].as_str().unwrap_or_default().to_string(),
            reason: format!(
                "{} (and {} guarded key(s) in total); nothing was written",
                first["reason"].as_str().unwrap_or_default(),
                guarded.len()
            ),
        });
    }

    let batch = journal::new_id(Some("apply"));
    let items = planned
        .iter()
        .chain(&pruned)
        .map(|p| BatchItem {
            entry: p.entry,
            key: p.key.clone(),
            path: p.path.clone(),
            action: if p.reset { Action::Reset } else { Action::Set(p.value.clone()) },
            yes: true,
            reverts: None,
        })
        .collect();
    let outs = write_batch(items, "apply", Some(&batch), false)?;
    let (mut changes, mut resets) = (Vec::new(), Vec::new());
    for (p, out) in planned.iter().chain(&pruned).zip(outs) {
        let mut c = change_json(p);
        c["journal_id"] = out["journal_id"].clone();
        if p.reset { resets.push(c) } else { changes.push(c) }
    }
    let batch = if changes.is_empty() && resets.is_empty() { Value::Null } else { batch.into() };
    Ok(json!({
        "dry_run": false, "batch": batch, "changes": changes, "pruned": prune_json(resets), "unchanged": unchanged,
    }))
}

/// `--prune`: customised keys (fixed schemas and listed instances) in the
/// files' areas, narrowed by `--area`, that no file lists. App state is left
/// alone, since `export` never writes it.
fn plan_prune<'a>(
    index: &'a Index,
    opts: &ApplyOpts,
    file_areas: &[String],
    listed: &[(String, Option<String>, String)],
    problems: &mut Vec<Value>,
) -> Result<Vec<Planned<'a>>> {
    let owned = |area: &str| {
        file_areas.iter().any(|a| a == area) && (opts.areas.is_empty() || opts.areas.iter().any(|a| a == area))
    };
    let fixed = index.entries().iter().filter(|e| !e.relocatable()).map(|e| (e, None));
    let instances = index.listed_instances().into_iter().map(|(e, p)| (e, Some(p)));
    let mut out = Vec::new();
    for (entry, path) in fixed.chain(instances) {
        let id = entry.id();
        let settings = entry.settings(path.as_deref())?;
        for (key, old) in customised(entry, path.as_deref())? {
            if !owned(area_of(&id, &key))
                || state_reason(&id, &key).is_some()
                || listed.iter().any(|(s, p, k)| *s == id && *p == path && *k == key)
            {
                continue;
            }
            if !settings.is_writable(&key) {
                let err = Error::NotWritable { schema: id.clone(), key: key.clone() };
                problems.push(json!({
                    "file": null, "schema": id, "path": path, "key": key,
                    "code": err.code(), "exit_code": err.exit_code(), "message": format!("--prune: {err}"),
                }));
                continue;
            }
            let value = settings.default_value(&key).unwrap_or_else(|| entry.schema.key(&key).default_value());
            out.push(Planned { entry, path: path.clone(), key, value, old, reset: true });
        }
    }
    Ok(out)
}

pub fn areas() -> Value {
    let mut list: Vec<Value> = AREAS
        .iter()
        .map(|a| {
            let schemas: Vec<String> = a
                .rules
                .iter()
                .map(|r| match r.keys {
                    Some(keys) => format!("{} ({})", r.schema, keys.join(", ")),
                    None => r.schema.to_string(),
                })
                .collect();
            json!({ "name": a.name, "description": a.description, "schemas": schemas })
        })
        .collect();
    list.push(json!({ "name": OTHER, "description": "Every other customised key", "schemas": [] }));
    json!({ "areas": list })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::ty;

    #[test]
    fn areas_route_keys() {
        assert_eq!(area_of("org.gnome.shell", "enabled-extensions"), "extensions");
        assert_eq!(area_of("org.gnome.shell", "favorite-apps"), "shell");
        assert_eq!(area_of("org.gnome.shell.extensions.dash-to-dock", "dock-position"), "extensions");
        assert_eq!(area_of("org.gnome.shell.keybindings", "toggle-overview"), "keybindings");
        assert_eq!(area_of(CUSTOM_KEYBINDING_SCHEMA, "binding"), "keybindings");
        assert_eq!(area_of("org.gnome.mutter", "dynamic-workspaces"), "interface");
        assert_eq!(area_of("org.gnome.mutter.keybindings", "toggle-tiled-left"), "keybindings");
        assert_eq!(area_of("org.gnome.desktop.peripherals.touchpad", "tap-to-click"), "input");
        assert_eq!(area_of("org.gnome.desktop.remote-desktop.rdp", "enable"), "remote");
        assert_eq!(area_of("org.gnome.desktop.a11y.keyboard", "enable"), "a11y");
        assert_eq!(area_of("org.gnome.Ptyxis", "default-profile-uuid"), "other");
    }

    #[test]
    fn state_keys() {
        assert!(state_reason("com.ubuntu.update-manager", "launch-count").is_some());
        assert!(state_reason("org.gnome.nautilus.preferences", "migrated-gtk-settings").is_some());
        assert!(state_reason("org.gnome.shell", "welcome-dialog-last-shown-version").is_some());
        assert!(state_reason("org.gnome.shell.extensions.tiling-assistant", "last-version-installed").is_some());
        assert!(state_reason("org.gnome.desktop.remote-desktop.rdp", "tls-key").is_some());
        assert!(state_reason("org.gnome.TextEditor", "window-maximized").is_some());
        assert!(state_reason("org.gnome.desktop.peripherals.keyboard", "numlock-state").is_some());
        assert!(state_reason("org.gnome.seahorse.manager", "keyrings-selected").is_some());
        assert!(state_reason("org.gnome.shell.extensions.tiling-assistant", "overridden-settings").is_some());
        assert!(state_reason("org.gnome.desktop.peripherals.keyboard", "remember-numlock-state").is_none());
        assert!(state_reason("org.gnome.desktop.interface", "color-scheme").is_none());
        assert!(state_reason("org.gnome.settings-daemon.plugins.power", "sleep-inactive-ac-type").is_none());
    }

    #[test]
    fn encode_falls_back_to_gvariant_when_json_is_lossy() {
        let exact = Variant::parse(Some(&ty("a{sv}")), "{'a': <1>, 'b': <'x'>}").unwrap();
        assert_eq!(encode(&exact), json!({"a": 1, "b": "x"}));
        let lossy = Variant::parse(Some(&ty("a{sv}")), "{'a': <uint32 1>}").unwrap();
        let enc = encode(&lossy);
        assert_eq!(enc, json!({"$gvariant": "{'a': <uint32 1>}"}));
        assert_eq!(decode(&enc, &ty("a{sv}")).unwrap(), lossy);
        let plain = Variant::parse(Some(&ty("a{sv}")), "{'a': <1>}").unwrap();
        assert_eq!(decode(&json!({"a": 1}), &ty("a{sv}")).unwrap(), plain);
    }
}
