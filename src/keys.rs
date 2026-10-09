//! Keyboard shortcuts across every keybinding schema plus custom bindings.

use gio::prelude::*;
use glib::Variant;
use serde_json::{Value, json};

use crate::commands::{Action, Tags, WriteOpts, write};
use crate::error::{Error, Result};
use crate::index::{Entry, Index};
use crate::journal;
use crate::state::CUSTOM_KEYBINDING_SCHEMA;

/// Schemas whose keys are keybindings (`as`, or `s` in some older media keys).
pub const KEYBINDING_SCHEMAS: &[&str] = &[
    "org.gnome.desktop.wm.keybindings",
    "org.gnome.shell.keybindings",
    "org.gnome.mutter.keybindings",
    "org.gnome.mutter.wayland.keybindings",
    MEDIA_KEYS,
];
pub const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM_LIST_KEY: &str = "custom-keybindings";
const CUSTOM_BASE: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";

const MODIFIERS: &[(&str, &[&str])] = &[
    ("Control", &["control", "ctrl", "ctl", "primary"]),
    ("Shift", &["shift", "shft"]),
    ("Alt", &["alt", "mod1"]),
    ("Super", &["super", "mod4"]),
    ("Meta", &["meta"]),
    ("Hyper", &["hyper"]),
];

/// Canonical form of a GTK accelerator: modifiers in a fixed order, aliases
/// unified (`<Primary>`/`<Ctrl>` → `<Control>`, `<Mod4>` → `<Super>`), single
/// letters lowercased. `None` for an empty or disabled binding.
pub fn normalize(accel: &str) -> std::result::Result<Option<String>, String> {
    let accel = accel.trim();
    if accel.is_empty() || accel.eq_ignore_ascii_case("disabled") {
        return Ok(None);
    }
    let mut mods = [false; MODIFIERS.len()];
    let mut rest = accel;
    while let Some(after) = rest.strip_prefix('<') {
        let (name, tail) = after.split_once('>').ok_or_else(|| format!("unclosed '<' in {accel:?}"))?;
        let lower = name.to_ascii_lowercase();
        let i = MODIFIERS
            .iter()
            .position(|(_, aliases)| aliases.contains(&lower.as_str()))
            .ok_or_else(|| format!("unknown modifier <{name}> in {accel:?}"))?;
        mods[i] = true;
        rest = tail;
    }
    if rest.is_empty() {
        return Err(format!("no key after the modifiers in {accel:?}"));
    }
    if rest.contains(['<', '>', ' ']) {
        return Err(format!("unexpected characters in key {rest:?} of {accel:?}"));
    }
    let key = if rest.chars().count() == 1 { rest.to_lowercase() } else { rest.to_string() };
    let mut out: String =
        MODIFIERS.iter().zip(mods).filter(|(_, on)| *on).map(|((m, _), _)| format!("<{m}>")).collect();
    out.push_str(&key);
    Ok(Some(out))
}

/// Keys compare case-insensitively (`Return` = `return`), modifiers exactly.
fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

struct Binding {
    schema: String,
    key: String,
    path: Option<String>,
    accels: Vec<String>,
    summary: Option<String>,
    custom: Option<(String, String)>, // (name, command)
}

impl Binding {
    fn to_json(&self) -> Value {
        let mut v = json!({
            "schema": self.schema,
            "key": self.key,
            "path": self.path,
            "accels": self.accels,
            "summary": self.summary,
        });
        if let Some((name, command)) = &self.custom {
            v["custom"] = json!({ "name": name, "command": command });
        }
        v
    }

    fn normalized(&self) -> Vec<String> {
        self.accels.iter().filter_map(|a| normalize(a).ok().flatten()).collect()
    }
}

fn accels_of(v: &Variant) -> Vec<String> {
    match v.str() {
        Some(s) => vec![s.to_string()],
        None => v.get::<Vec<String>>().unwrap_or_default(),
    }
    .into_iter()
    .filter(|a| matches!(normalize(a), Ok(Some(_))))
    .collect()
}

fn custom_paths(index: &Index) -> Vec<String> {
    let Ok(media) = index.get(MEDIA_KEYS) else { return vec![] };
    if !media.schema.has_key(CUSTOM_LIST_KEY) {
        return vec![];
    }
    media.settings(None).map(|s| s.strv(CUSTOM_LIST_KEY).iter().map(|p| p.to_string()).collect()).unwrap_or_default()
}

/// Every binding with at least one accelerator.
fn all_bindings(index: &Index) -> Result<Vec<Binding>> {
    let mut out = Vec::new();
    for id in KEYBINDING_SCHEMAS {
        let Ok(entry) = index.get(id) else { continue };
        let settings = entry.settings(None)?;
        for key in entry.keys() {
            let ty = entry.schema.key(&key).value_type();
            if !matches!(ty.as_str(), "as" | "s") || key == CUSTOM_LIST_KEY {
                continue;
            }
            let accels = accels_of(&settings.value(&key));
            if accels.is_empty() {
                continue;
            }
            let summary = entry.schema.key(&key).summary().map(|s| s.to_string());
            out.push(Binding { schema: id.to_string(), key, path: None, accels, summary, custom: None });
        }
    }
    if let Ok(custom) = index.get(CUSTOM_KEYBINDING_SCHEMA) {
        for path in custom_paths(index) {
            let Ok(settings) = custom.settings(Some(&path)) else { continue };
            let accels = accels_of(&settings.value("binding"));
            let name = settings.string("name").to_string();
            let command = settings.string("command").to_string();
            out.push(Binding {
                schema: CUSTOM_KEYBINDING_SCHEMA.to_string(),
                key: "binding".into(),
                path: Some(path),
                accels,
                summary: Some(name.clone()),
                custom: Some((name, command)),
            });
        }
    }
    Ok(out)
}

pub fn list(index: &Index, custom_only: bool) -> Result<Value> {
    let bindings: Vec<Value> =
        all_bindings(index)?.iter().filter(|b| !custom_only || b.custom.is_some()).map(Binding::to_json).collect();
    Ok(json!({ "count": bindings.len(), "bindings": bindings }))
}

pub fn find(index: &Index, accel: &str) -> Result<Value> {
    let wanted = normalize(accel)
        .map_err(|m| Error::InvalidValue { message: m, details: json!({ "accel": accel }) })?
        .ok_or_else(|| Error::Usage("give an accelerator, e.g. '<Super>t'".into()))?;
    let matches: Vec<Value> = all_bindings(index)?
        .iter()
        .filter(|b| b.normalized().iter().any(|n| same(n, &wanted)))
        .map(Binding::to_json)
        .collect();
    Ok(json!({ "accel": accel, "normalized": wanted, "free": matches.is_empty(), "bindings": matches }))
}

pub fn conflicts(index: &Index) -> Result<Value> {
    let bindings = all_bindings(index)?;
    let mut groups: Vec<(String, Vec<&Binding>)> = Vec::new();
    for b in &bindings {
        for n in b.normalized() {
            match groups.iter_mut().find(|(k, _)| same(k, &n)) {
                Some((_, list)) => {
                    if !list.iter().any(|x| std::ptr::eq(*x, b)) {
                        list.push(b);
                    }
                }
                None => groups.push((n, vec![b])),
            }
        }
    }
    let conflicts: Vec<Value> = groups
        .into_iter()
        .filter(|(_, list)| list.len() > 1)
        .map(
            |(accel, list)| json!({ "accel": accel, "bindings": list.iter().map(|b| b.to_json()).collect::<Vec<_>>() }),
        )
        .collect();
    Ok(json!({ "count": conflicts.len(), "conflicts": conflicts }))
}

pub struct AddOpts<'a> {
    pub name: &'a str,
    pub accel: &'a str,
    pub command: &'a str,
    pub force: bool,
    pub dry_run: bool,
}

/// Create a custom keybinding at the first free `customN` path.
pub fn add(index: &Index, opts: &AddOpts) -> Result<Value> {
    let invalid = |m: String| Error::InvalidValue { message: m, details: json!({ "accel": opts.accel }) };
    let normalized = normalize(opts.accel).map_err(invalid)?.ok_or_else(|| invalid("empty accelerator".into()))?;
    if opts.name.trim().is_empty() || opts.command.trim().is_empty() {
        return Err(Error::Usage("name and command must not be empty".into()));
    }
    let media = index.get(MEDIA_KEYS)?;
    let custom = index.get(CUSTOM_KEYBINDING_SCHEMA)?;

    let taken = find(index, opts.accel)?;
    if taken["free"] == false && !opts.force {
        return Err(Error::Conflict {
            id: opts.accel.to_string(),
            message: format!(
                "{} is already bound ({}). Pick another accelerator, or re-run with --force to add it anyway",
                opts.accel,
                taken["bindings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|b| format!(
                        "{} {}",
                        b["schema"].as_str().unwrap_or_default(),
                        b["key"].as_str().unwrap_or_default()
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }

    let existing = custom_paths(index);
    let path = (0..)
        .map(|n| format!("{CUSTOM_BASE}custom{n}/"))
        .find(|p| !existing.contains(p) && !has_user_values(custom, p))
        .unwrap();
    let mut list = existing.clone();
    list.push(path.clone());

    let batch = journal::new_id(Some("keys-add"));
    let wopts = WriteOpts { path: Some(&path), dry_run: opts.dry_run, yes: false };
    let tags = Tags { reverts: None, batch: Some(&batch) };
    let mut writes = Vec::new();
    for (key, value) in [("name", opts.name), ("command", opts.command), ("binding", opts.accel)] {
        writes.push(write(custom, key, &wopts, "keys-add", Action::Set(value.into()), tags)?);
    }
    let lopts = WriteOpts { path: None, dry_run: opts.dry_run, yes: false };
    writes.push(write(media, CUSTOM_LIST_KEY, &lopts, "keys-add", Action::Set(list.to_variant()), tags)?);

    Ok(json!({
        "dry_run": opts.dry_run,
        "path": path,
        "name": opts.name,
        "command": opts.command,
        "accel": opts.accel,
        "normalized": normalized,
        "batch": if opts.dry_run { Value::Null } else { batch.into() },
        "writes": writes,
    }))
}

/// Whether a relocatable instance already holds values (a stale binding not
/// in the list), so a new binding doesn't inherit its leftovers.
fn has_user_values(entry: &Entry, path: &str) -> bool {
    entry.settings(Some(path)).is_ok_and(|s| entry.keys().iter().any(|k| s.user_value(k).is_some()))
}

#[cfg(test)]
mod tests {
    use super::normalize;

    fn n(a: &str) -> Option<String> {
        normalize(a).unwrap()
    }

    #[test]
    fn normalizes_aliases_order_and_case() {
        assert_eq!(n("<Super>t").as_deref(), Some("<Super>t"));
        assert_eq!(n("<Super>T").as_deref(), Some("<Super>t"));
        assert_eq!(n("<Primary><Super>t"), n("<Super><Ctrl>t"));
        assert_eq!(n("<Mod4><Shift>Return").as_deref(), Some("<Shift><Super>Return"));
        assert_eq!(n("<Alt>F4").as_deref(), Some("<Alt>F4"));
        assert_eq!(n("XF86AudioMute").as_deref(), Some("XF86AudioMute"));
    }

    #[test]
    fn empty_and_disabled() {
        assert_eq!(n(""), None);
        assert_eq!(n("disabled"), None);
    }

    #[test]
    fn rejects_malformed() {
        assert!(normalize("<Super>").is_err());
        assert!(normalize("<Super").is_err());
        assert!(normalize("<Banana>x").is_err());
        assert!(normalize("<Super>a b").is_err());
    }
}
