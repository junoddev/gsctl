//! Every schema GLib can see: the default schema source plus each GNOME Shell
//! extension's own `schemas/` directory.

use std::path::{Path, PathBuf};
use std::rc::Rc;

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
    /// `(path, schema id)` of every fixed schema in the index, shared, so an
    /// instance path can be checked without the index.
    fixed_paths: Rc<[(String, String)]>,
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
                self.check_instance_path(p)?;
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

    /// Whether `p` may hold an instance of this relocatable schema. It must
    /// be a valid path; not the path of any installed fixed schema (writing
    /// another schema's keys there would bypass that schema's guards and
    /// types); and, for the listed relocatable schemas (custom keybindings,
    /// Ptyxis profiles), below their usual prefix.
    pub fn check_instance_path(&self, p: &str) -> Result<()> {
        if !valid_path(p) {
            return Err(Error::Usage(format!("invalid path {p:?}: must start and end with '/'")));
        }
        if let Some((_, owner)) = self.fixed_paths.iter().find(|(fixed, _)| fixed == p) {
            return Err(Error::Usage(format!(
                "{p} is the path of the fixed schema {owner}; use {owner} itself rather than {} --path",
                self.id()
            )));
        }
        let id = self.id();
        if let Some(l) = LISTED.iter().find(|l| l.schema == id)
            && !(p.starts_with(l.prefix) && p.len() > l.prefix.len())
        {
            return Err(Error::Usage(format!("{id} instances live below {}, not at {p}", l.prefix)));
        }
        Ok(())
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
    /// Where instances live; `--path` for `schema` must be below it.
    prefix: &'static str,
}

const LISTED: &[Listed] = &[
    Listed {
        list_schema: "org.gnome.settings-daemon.plugins.media-keys",
        list_key: "custom-keybindings",
        schema: "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding",
        path: "{}",
        prefix: "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/",
    },
    Listed {
        list_schema: "org.gnome.Ptyxis",
        list_key: "profile-uuids",
        schema: "org.gnome.Ptyxis.Profile",
        path: "/org/gnome/Ptyxis/Profiles/{}/",
        prefix: "/org/gnome/Ptyxis/Profiles/",
    },
];

/// The schema ids, and the dconf paths, an extension's own `schemas/` dir may
/// add. GSConnect's id is `org.gnome.Shell.Extensions.GSConnect`, and some
/// extensions use their own reverse-DNS id at a path under this one.
const EXTENSION_NAMESPACE: &str = "org.gnome.shell.extensions.";
const EXTENSION_PATH: &str = "/org/gnome/shell/extensions/";

/// Whether an extension may add schema `id` with fixed `path`.
fn in_extension_namespace(id: &str, path: Option<&str>) -> bool {
    id.to_ascii_lowercase().starts_with(EXTENSION_NAMESPACE) || path.is_some_and(|p| p.starts_with(EXTENSION_PATH))
}

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
        let no_paths: Rc<[(String, String)]> = Rc::from(Vec::new());
        let push = |entries: &mut Vec<Entry>, schema: SettingsSchema, origin: Origin| {
            if !entries.iter().any(|e| e.schema.id() == schema.id()) {
                entries.push(Entry { schema, origin, fixed_paths: no_paths.clone() });
            }
        };

        // The system source first: it wins for every id it has, so an
        // extension (even a disabled one) can't redefine a system schema's
        // types, ranges, path or descriptions.
        if let Some(source) = &default {
            let (fixed, relocatable) = source.list_schemas(true);
            for id in fixed.iter().chain(relocatable.iter()) {
                if let Some(schema) = source.lookup(id, true) {
                    push(&mut entries, schema, Origin::System);
                }
            }
        }

        // Extensions may only add schemas in their own namespace, at a path
        // no system schema uses.
        for (uuid, dir) in extension_schema_dirs() {
            let source = match SettingsSchemaSource::from_directory(&dir, default.as_ref(), false) {
                Ok(s) => s,
                Err(_) => continue, // missing or stale gschemas.compiled
            };
            let (fixed, relocatable) = source.list_schemas(false);
            for id in fixed.iter().chain(relocatable.iter()) {
                let Some(schema) = source.lookup(id, false) else { continue };
                let path = schema.path();
                if !in_extension_namespace(id, path.as_deref()) {
                    continue;
                }
                if path.is_some() && entries.iter().any(|e| e.schema.path() == path) {
                    continue;
                }
                push(&mut entries, schema, Origin::Extension { uuid: uuid.clone(), dir: dir.clone() });
            }
        }

        entries.sort_by_key(|e| e.id());
        let fixed: Rc<[(String, String)]> =
            entries.iter().filter_map(|e| Some((e.schema.path()?.to_string(), e.id()))).collect();
        for e in &mut entries {
            e.fixed_paths = fixed.clone();
        }
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
                if entry.check_instance_path(&path).is_ok() {
                    out.push((entry, path));
                }
            }
        }
        out
    }
}

/// `(uuid, schemas dir)` for every installed extension that ships schemas,
/// user extensions (`$XDG_DATA_HOME`) before system ones (`$XDG_DATA_DIRS`).
/// Directories whose `metadata.json` uuid isn't the directory name are
/// skipped, as GNOME Shell skips them.
fn extension_schema_dirs() -> Vec<(String, PathBuf)> {
    let mut bases = vec![glib::user_data_dir()];
    bases.extend(glib::system_data_dirs());
    let mut out = Vec::new();
    for base in bases {
        let Ok(dir) = std::fs::read_dir(base.join("gnome-shell/extensions")) else { continue };
        let mut found: Vec<(String, PathBuf)> = dir
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| has_compiled_schemas(p) && crate::ext::metadata_matching_dir(p).is_some())
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
