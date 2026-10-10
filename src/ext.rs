//! GNOME Shell extensions: installed metadata, enabled state in settings,
//! and each extension's own schema.

use std::path::{Path, PathBuf};

use gio::prelude::*;
use serde_json::{Value, json};

use crate::commands::{self, Action, Tags, WriteOpts, write};
use crate::error::{Error, Result};
use crate::index::Index;
use crate::{fsutil, journal};

const SHELL: &str = "org.gnome.shell";
const ENABLED: &str = "enabled-extensions";
const DISABLED: &str = "disabled-extensions";
const DISABLE_USER: &str = "disable-user-extensions";
const ENABLE_REASON: &str = "enabling an extension runs its code inside GNOME Shell";

pub struct Extension {
    pub uuid: String,
    pub dir: PathBuf,
    pub user: bool,
    pub metadata: Value,
}

impl Extension {
    /// Schema ids this extension uses: `settings-schema` from its metadata,
    /// any schemas it ships in its own `schemas/` dir, and installed
    /// `org.gnome.shell.extensions.*` ids quoted in its JavaScript (Ubuntu's
    /// dock and DING declare no `settings-schema` but call
    /// `getSettings('org.gnome.shell.extensions.…')`).
    pub fn schemas(&self, index: &Index) -> Vec<String> {
        let mut ids: Vec<String> = self.metadata["settings-schema"].as_str().map(str::to_string).into_iter().collect();
        let mut add = |id: String| {
            if !ids.contains(&id) {
                ids.push(id);
            }
        };
        for e in index.entries() {
            if let crate::index::Origin::Extension { uuid, .. } = &e.origin
                && *uuid == self.uuid
            {
                add(e.id());
            }
        }
        // Only extension-namespaced schemas: extensions also read GNOME's own
        // (DING reads nautilus and interface settings), which aren't theirs.
        for literal in js_string_literals(&self.dir) {
            if literal.starts_with("org.gnome.shell.extensions.") && index.get(&literal).is_ok() {
                add(literal);
            }
        }
        ids
    }
}

/// Installed extensions, user ones first; a user copy hides a system one with
/// the same uuid, as in GNOME Shell.
pub fn installed() -> Vec<Extension> {
    let mut out: Vec<Extension> = Vec::new();
    let mut bases = vec![(glib::user_data_dir(), true)];
    bases.extend(glib::system_data_dirs().into_iter().map(|d| (d, false)));
    for (base, user) in bases {
        let Ok(dir) = std::fs::read_dir(base.join("gnome-shell/extensions")) else { continue };
        let mut found: Vec<Extension> = dir.filter_map(|e| e.ok()).filter_map(|e| load(&e.path(), user)).collect();
        found.sort_by(|a, b| a.uuid.cmp(&b.uuid));
        for ext in found {
            if !out.iter().any(|e| e.uuid == ext.uuid) {
                out.push(ext);
            }
        }
    }
    out
}

/// Dotted string literals (`'a.b.c'`) in the extension's `.js` files, a few
/// directory levels deep. Only real directories and regular files count:
/// symlinks aren't followed, FIFOs and other special files are skipped, and
/// the number of entries and the size of each file are capped.
fn js_string_literals(dir: &Path) -> Vec<String> {
    const MAX_ENTRIES: usize = 5000;
    let mut out = Vec::new();
    let mut seen = 0;
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.filter_map(|e| e.ok()) {
            seen += 1;
            if seen > MAX_ENTRIES {
                return out;
            }
            // DirEntry::file_type doesn't follow symlinks.
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() && depth < 3 {
                stack.push((p, depth + 1));
            } else if ft.is_file()
                && p.extension().is_some_and(|x| x == "js")
                && let Some(text) = fsutil::read_small(&p)
            {
                for quote in ['\'', '"'] {
                    for (i, part) in text.split(quote).enumerate() {
                        let looks_like_id = i % 2 == 1
                            && part.contains('.')
                            && part.len() < 128
                            && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
                        if looks_like_id && !out.iter().any(|o| o == part) {
                            out.push(part.to_string());
                        }
                    }
                }
            }
        }
    }
    out
}

fn load(dir: &Path, user: bool) -> Option<Extension> {
    let metadata = metadata_matching_dir(dir)?;
    let uuid = metadata["uuid"].as_str()?.to_string();
    Some(Extension { uuid, dir: dir.to_path_buf(), user, metadata })
}

/// An extension directory's `metadata.json`, if it's a small regular file
/// whose `uuid` is the directory's name. GNOME Shell refuses an extension
/// whose uuid doesn't match its directory, so gsctl ignores it too.
pub(crate) fn metadata_matching_dir(dir: &Path) -> Option<Value> {
    let text = fsutil::read_small(&dir.join("metadata.json"))?;
    let metadata: Value = serde_json::from_str(&text).ok()?;
    let name = dir.file_name()?.to_str()?;
    (metadata["uuid"].as_str() == Some(name)).then_some(metadata)
}

struct ShellState {
    enabled: Vec<String>,
    disabled: Vec<String>,
    user_disabled: bool,
    mode: SessionMode,
}

/// The GNOME Shell session mode and the extensions it enables by itself
/// (Ubuntu's `ubuntu` mode enables the dock, DING, AppIndicators, ...).
pub struct SessionMode {
    pub name: String,
    /// `env` when read from the session environment, `guess` when inferred
    /// from the only installed mode file, `default` for plain `user`.
    pub source: &'static str,
    pub extensions: Vec<String>,
}

fn mode_file(name: &str) -> Option<Value> {
    glib::system_data_dirs().into_iter().find_map(|d| {
        let text = std::fs::read_to_string(d.join("gnome-shell/modes").join(format!("{name}.json"))).ok()?;
        serde_json::from_str(&text).ok()
    })
}

pub fn session_mode() -> SessionMode {
    let from_env = std::env::var("GNOME_SHELL_SESSION_MODE").ok().filter(|m| !m.is_empty()).or_else(|| {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok()?;
        let first = desktop.split(':').next()?.to_lowercase();
        mode_file(&first).map(|_| first)
    });
    let (name, source) = match from_env {
        Some(m) => (m, "env"),
        None => {
            let installed: Vec<String> = glib::system_data_dirs()
                .into_iter()
                .filter_map(|d| std::fs::read_dir(d.join("gnome-shell/modes")).ok())
                .flat_map(|rd| rd.filter_map(|e| e.ok()))
                .filter_map(|e| e.path().file_stem()?.to_str().map(str::to_string))
                .filter(|m| m != "initial-setup")
                .collect();
            match installed.as_slice() {
                [only] => (only.clone(), "guess"),
                _ => ("user".to_string(), "default"),
            }
        }
    };
    let mut extensions = Vec::new();
    let mut current = Some(name.clone());
    let mut seen = Vec::new();
    while let Some(m) = current.take() {
        if seen.contains(&m) {
            break;
        }
        seen.push(m.clone());
        if let Some(doc) = mode_file(&m) {
            for u in doc["enabledExtensions"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                if !extensions.iter().any(|e| e == u) {
                    extensions.push(u.to_string());
                }
            }
            current = doc["parentMode"].as_str().map(str::to_string);
        }
    }
    SessionMode { name, source, extensions }
}

fn shell_state(index: &Index) -> Result<ShellState> {
    let entry = index.get(SHELL)?;
    let settings = entry.settings(None)?;
    let strv = |k: &str| -> Vec<String> {
        if entry.schema.has_key(k) { settings.strv(k).iter().map(|s| s.to_string()).collect() } else { vec![] }
    };
    let (enabled, disabled) = (strv(ENABLED), strv(DISABLED));
    let user_disabled = entry.schema.has_key(DISABLE_USER) && settings.boolean(DISABLE_USER);
    Ok(ShellState { enabled, disabled, user_disabled, mode: session_mode() })
}

impl ShellState {
    fn by_mode(&self, uuid: &str) -> bool {
        self.mode.extensions.iter().any(|u| u == uuid)
    }

    /// What enables `uuid`, following GNOME Shell: `disabled-extensions` wins;
    /// otherwise the session mode, or `enabled-extensions` unless
    /// `disable-user-extensions` is set.
    fn enabled_by(&self, uuid: &str) -> Option<&'static str> {
        if self.disabled.iter().any(|u| u == uuid) {
            None
        } else if self.by_mode(uuid) {
            Some("session-mode")
        } else if self.enabled.iter().any(|u| u == uuid) && !self.user_disabled {
            Some("settings")
        } else {
            None
        }
    }

    fn mode_json(&self) -> Value {
        json!({ "name": self.mode.name, "source": self.mode.source, "extensions": self.mode.extensions })
    }
}

fn summary(ext: &Extension, state: &ShellState, index: &Index) -> Value {
    let m = &ext.metadata;
    json!({
        "uuid": ext.uuid,
        "name": m["name"],
        "description": m["description"],
        "version": m.get("version-name").or(m.get("version")).cloned().unwrap_or(Value::Null),
        "shell_version": m["shell-version"],
        "location": if ext.user { "user" } else { "system" },
        "dir": ext.dir,
        "enabled": state.enabled_by(&ext.uuid).is_some(),
        "enabled_by": state.enabled_by(&ext.uuid),
        "schemas": ext.schemas(index),
        "has_prefs": ext.dir.join("prefs.js").is_file(),
    })
}

pub fn list(index: &Index) -> Result<Value> {
    let state = shell_state(index)?;
    let exts = installed();
    let items: Vec<Value> = exts.iter().map(|e| summary(e, &state, index)).collect();
    let missing: Vec<&String> = state.enabled.iter().filter(|u| !exts.iter().any(|e| &e.uuid == *u)).collect();
    Ok(json!({
        "count": items.len(),
        "user_extensions_disabled": state.user_disabled,
        "session_mode": state.mode_json(),
        "enabled_but_not_installed": missing,
        "extensions": items,
    }))
}

fn find<'a>(exts: &'a [Extension], uuid: &str) -> Result<&'a Extension> {
    exts.iter().find(|e| e.uuid == uuid).ok_or_else(|| Error::ExtensionNotFound(uuid.to_string()))
}

pub fn info(index: &Index, uuid: &str) -> Result<Value> {
    let state = shell_state(index)?;
    let exts = installed();
    let ext = find(&exts, uuid)?;
    let mut out = summary(ext, &state, index);
    let mut settings = Vec::new();
    for id in ext.schemas(index) {
        if let Ok(v) = commands::schema(index, Some(&id), None, None) {
            settings.push(v);
        }
    }
    out["metadata"] = ext.metadata.clone();
    out["settings"] = settings.into();
    Ok(out)
}

/// Enable or disable one extension by editing only its uuid in
/// `enabled-extensions` / `disabled-extensions`, as the Extensions app does.
/// These keys are guarded against wholesale replacement; a single-uuid edit
/// skips that guard. Enabling runs the extension's code inside GNOME Shell,
/// so it needs `yes` (the dry run reports `requires_yes`); disabling doesn't.
pub fn set_enabled(index: &Index, uuid: &str, enable: bool, dry_run: bool, yes: bool) -> Result<Value> {
    let exts = installed();
    find(&exts, uuid)?;
    let state = shell_state(index)?;
    let entry = index.get(SHELL)?;

    let mut enabled = state.enabled.clone();
    let mut disabled = state.disabled.clone();
    if enable {
        // The session mode already enables its own extensions.
        if !state.by_mode(uuid) && !enabled.iter().any(|u| u == uuid) {
            enabled.push(uuid.to_string());
        }
        disabled.retain(|u| u != uuid);
    } else {
        enabled.retain(|u| u != uuid);
        if !disabled.iter().any(|u| u == uuid) {
            disabled.push(uuid.to_string());
        }
    }

    let changes = enabled != state.enabled || disabled != state.disabled;
    let requires_yes = enable && changes;
    if requires_yes && !dry_run && !yes {
        return Err(Error::Guarded { schema: SHELL.into(), key: ENABLED.into(), reason: ENABLE_REASON.into() });
    }

    let op = if enable { "ext-enable" } else { "ext-disable" };
    let batch = journal::new_id(Some(op));
    let opts = WriteOpts { path: None, dry_run, yes: true };
    let tags = Tags { reverts: None, batch: Some(&batch) };
    let mut writes = Vec::new();
    for (key, new, old) in [(ENABLED, &enabled, &state.enabled), (DISABLED, &disabled, &state.disabled)] {
        if new != old && entry.schema.has_key(key) {
            writes.push(write(entry, key, &opts, op, Action::Set(new.to_variant()), tags)?);
        }
    }
    Ok(json!({
        "uuid": uuid,
        "enabled": enable,
        "session_mode": state.mode_json(),
        "changed": !writes.is_empty(),
        "dry_run": dry_run,
        "requires_yes": requires_yes,
        "guarded": if requires_yes { Value::from(ENABLE_REASON) } else { Value::Null },
        "executes": enable,
        "batch": if dry_run || writes.is_empty() { Value::Null } else { batch.into() },
        "user_extensions_disabled": state.user_disabled,
        "writes": writes,
    }))
}
