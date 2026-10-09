//! Every schema GLib can see: the default schema source plus each GNOME Shell
//! extension's own `schemas/` directory.

use std::path::{Path, PathBuf};

use gio::prelude::*;
use gio::{SettingsSchema, SettingsSchemaKey, SettingsSchemaSource};
use serde_json::{Value, json};

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Origin {
    System,
    Extension { uuid: String, dir: PathBuf },
}

impl Origin {
    pub fn to_json(&self) -> Value {
        match self {
            Origin::System => json!({ "type": "system" }),
            Origin::Extension { uuid, dir } => json!({ "type": "extension", "uuid": uuid, "dir": dir }),
        }
    }
}

pub struct Entry {
    pub schema: SettingsSchema,
    pub origin: Origin,
}

impl Entry {
    pub fn id(&self) -> String {
        self.schema.id().to_string()
    }

    /// Relocatable schemas have no fixed path; a path must be given to use them.
    pub fn relocatable(&self) -> bool {
        self.schema.path().is_none()
    }

    pub fn key(&self, name: &str) -> Result<SettingsSchemaKey> {
        if self.schema.has_key(name) {
            Ok(self.schema.key(name))
        } else {
            Err(Error::KeyNotFound { schema: self.id(), key: name.to_string() })
        }
    }

    /// Key names, sorted.
    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.schema.list_keys().iter().map(|k| k.to_string()).collect();
        keys.sort();
        keys
    }

    /// Open a `GSettings` for this schema, at `path` for relocatable schemas.
    pub fn settings(&self, path: Option<&str>) -> Result<gio::Settings> {
        let path = match (self.relocatable(), path) {
            (true, None) => {
                return Err(Error::Usage(format!(
                    "{} is relocatable: pass --path /some/dconf/path/ to choose an instance",
                    self.id()
                )));
            }
            (true, Some(p)) => {
                if !valid_path(p) {
                    return Err(Error::Usage(format!("invalid path {p:?}: must start and end with '/'")));
                }
                Some(p)
            }
            (false, Some(_)) => {
                return Err(Error::Usage(format!(
                    "{} has a fixed path; --path is only for relocatable schemas",
                    self.id()
                )));
            }
            (false, None) => None,
        };
        Ok(gio::Settings::new_full(&self.schema, None::<&gio::SettingsBackend>, path))
    }

    pub fn summary_json(&self) -> Value {
        json!({
            "id": self.id(),
            "path": self.schema.path().map(|p| p.to_string()),
            "relocatable": self.relocatable(),
            "origin": self.origin.to_json(),
        })
    }
}

/// A relocatable schema whose instances are listed in a key of another
/// schema. `path` turns a list item into the instance path (`{}` is the item).
struct Listed {
    list_schema: &'static str,
    list_key: &'static str,
    schema: &'static str,
    path: &'static str,
}

const LISTED: &[Listed] = &[
    Listed {
        list_schema: "org.gnome.settings-daemon.plugins.media-keys",
        list_key: "custom-keybindings",
        schema: "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding",
        path: "{}",
    },
    Listed {
        list_schema: "org.gnome.Ptyxis",
        list_key: "profile-uuids",
        schema: "org.gnome.Ptyxis.Profile",
        path: "/org/gnome/Ptyxis/Profiles/{}/",
    },
];

fn valid_path(p: &str) -> bool {
    p.starts_with('/') && p.ends_with('/') && !p.contains("//")
}

pub struct Index {
    entries: Vec<Entry>,
}

impl Index {
    /// Load the default source and all extension schema directories.
    pub fn load() -> Result<Index> {
        let default = SettingsSchemaSource::default();
        let mut entries: Vec<Entry> = Vec::new();
        let push = |entries: &mut Vec<Entry>, schema: SettingsSchema, origin: Origin| {
            if !entries.iter().any(|e| e.schema.id() == schema.id()) {
                entries.push(Entry { schema, origin });
            }
        };

        // Extensions first: like GNOME Shell, an extension's bundled schema wins
        // over a copy installed system-wide, and it tells us which extension
        // owns the keys.
        for (uuid, dir) in extension_schema_dirs() {
            let source = match SettingsSchemaSource::from_directory(&dir, default.as_ref(), false) {
                Ok(s) => s,
                Err(_) => continue, // missing or stale gschemas.compiled
            };
            let (fixed, relocatable) = source.list_schemas(false);
            for id in fixed.iter().chain(relocatable.iter()) {
                if let Some(schema) = source.lookup(id, false) {
                    push(&mut entries, schema, Origin::Extension { uuid: uuid.clone(), dir: dir.clone() });
                }
            }
        }

        if let Some(source) = &default {
            let (fixed, relocatable) = source.list_schemas(true);
            for id in fixed.iter().chain(relocatable.iter()) {
                if let Some(schema) = source.lookup(id, true) {
                    push(&mut entries, schema, Origin::System);
                }
            }
        }

        entries.sort_by_key(|e| e.id());
        Ok(Index { entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, id: &str) -> Result<&Entry> {
        self.entries.iter().find(|e| e.schema.id() == id).ok_or_else(|| Error::SchemaNotFound(id.to_string()))
    }

    /// The current instances of relocatable schemas that GNOME tracks in a
    /// list key (custom keybindings, Ptyxis profiles). Items that don't make
    /// a valid path are skipped.
    pub fn listed_instances(&self) -> Vec<(&Entry, String)> {
        let mut out = Vec::new();
        for l in LISTED {
            let (Ok(list), Ok(entry)) = (self.get(l.list_schema), self.get(l.schema)) else { continue };
            if !list.schema.has_key(l.list_key) || !entry.relocatable() {
                continue;
            }
            let Ok(settings) = list.settings(None) else { continue };
            for item in settings.strv(l.list_key) {
                let path = l.path.replace("{}", &item);
                if valid_path(&path) {
                    out.push((entry, path));
                }
            }
        }
        out
    }
}

/// `(uuid, schemas dir)` for every installed extension that ships schemas,
/// user extensions (`$XDG_DATA_HOME`) before system ones (`$XDG_DATA_DIRS`).
fn extension_schema_dirs() -> Vec<(String, PathBuf)> {
    let mut bases = vec![glib::user_data_dir()];
    bases.extend(glib::system_data_dirs());
    let mut out = Vec::new();
    for base in bases {
        let Ok(dir) = std::fs::read_dir(base.join("gnome-shell/extensions")) else { continue };
        let mut found: Vec<(String, PathBuf)> = dir
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| has_compiled_schemas(p))
            .filter_map(|p| Some((p.file_name()?.to_str()?.to_string(), p.join("schemas"))))
            .collect();
        found.sort();
        out.extend(found);
    }
    out
}

fn has_compiled_schemas(ext_dir: &Path) -> bool {
    ext_dir.join("schemas/gschemas.compiled").is_file()
}
