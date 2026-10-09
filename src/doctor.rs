//! `doctor`: can gsctl see and change this session's real settings?
//!
//! Agents often run sandboxed (no session bus, `~/.config/dconf` hidden).
//! There gsctl reads schema defaults and writes are dropped, so check first.

use gio::prelude::*;
use serde_json::{Value, json};

use std::sync::OnceLock;

use crate::error::{Error, Result};
use crate::index::{Index, Origin};
use crate::{ext, journal};

fn check(name: &str, status: &str, detail: impl Into<Value>, hint: Option<&str>) -> Value {
    json!({ "name": name, "status": status, "detail": detail.into(), "hint": hint })
}

fn name_has_owner(bus: &gio::DBusConnection, name: &str) -> Option<bool> {
    bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameHasOwner",
        Some(&(name,).to_variant()),
        Some(glib::VariantTy::new("(b)").unwrap()),
        gio::DBusCallFlags::NONE,
        2000,
        None::<&gio::Cancellable>,
    )
    .ok()
    .and_then(|v| v.child_value(0).get::<bool>())
}

fn activatable(bus: &gio::DBusConnection, name: &str) -> Option<bool> {
    let reply = bus
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ListActivatableNames",
            None,
            Some(glib::VariantTy::new("(as)").unwrap()),
            gio::DBusCallFlags::NONE,
            2000,
            None::<&gio::Cancellable>,
        )
        .ok()?;
    Some(reply.child_value(0).get::<Vec<String>>()?.iter().any(|n| n == name))
}

/// Why writes can't reach the real dconf database, if they can't: the
/// fatal subset of `doctor`, checked before every write. GSettings drops
/// writes silently in these cases, and read-back can't tell when the new
/// value equals what the sandbox reads (usually the default).
fn write_blocker() -> Option<String> {
    static BLOCKER: OnceLock<Option<String>> = OnceLock::new();
    BLOCKER
        .get_or_init(|| {
            let backend = gio::SettingsBackend::default().type_().name().to_string();
            if !backend.contains("DConf") {
                return Some(format!("GSettings is using {backend}, not dconf, so changes won't persist"));
            }
            let bus = match gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) {
                Ok(bus) => bus,
                Err(e) => return Some(format!("no session bus ({}), so dconf can't commit changes", e.message())),
            };
            let reachable = name_has_owner(&bus, "ca.desrt.dconf").unwrap_or(false)
                || activatable(&bus, "ca.desrt.dconf").unwrap_or(false);
            (!reachable).then(|| "ca.desrt.dconf is neither running nor activatable on the session bus".to_string())
        })
        .clone()
}

pub fn require_writable() -> Result<()> {
    write_blocker().map_or(Ok(()), |reason| Err(Error::NoSession(reason)))
}

pub fn doctor(index: &Index) -> Result<Value> {
    let mut checks = Vec::new();

    let backend = gio::SettingsBackend::default();
    let backend_type = backend.type_().name().to_string();
    checks.push(if backend_type.contains("DConf") {
        check("settings_backend", "ok", backend_type, None)
    } else {
        check(
            "settings_backend",
            "error",
            backend_type,
            Some("GSettings is not using dconf, so changes won't persist (check GSETTINGS_BACKEND and the dconf GIO module)"),
        )
    });

    match gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) {
        Err(e) => {
            checks.push(check(
                "session_bus",
                "error",
                e.message(),
                Some("no session bus: writes are silently dropped by GSettings, so gsctl refuses them (exit 5). Run inside the desktop session, not a sandbox"),
            ));
        }
        Ok(bus) => {
            checks.push(check("session_bus", "ok", "connected", None));
            let running = name_has_owner(&bus, "ca.desrt.dconf").unwrap_or(false);
            let can_start = activatable(&bus, "ca.desrt.dconf").unwrap_or(false);
            checks.push(match (running, can_start) {
                (true, _) => check("dconf_service", "ok", "running", None),
                (false, true) => check("dconf_service", "ok", "activatable", None),
                (false, false) => check(
                    "dconf_service",
                    "error",
                    "ca.desrt.dconf is neither running nor activatable on this bus",
                    Some("writes will fail; is dconf-service installed and this the desktop's session bus?"),
                ),
            });
            let shell = name_has_owner(&bus, "org.gnome.Shell").unwrap_or(false);
            checks.push(if shell {
                check("gnome_shell", "ok", "running on this bus", None)
            } else {
                check(
                    "gnome_shell",
                    "warn",
                    "org.gnome.Shell is not on this bus",
                    Some("settings still work, but this may not be the live desktop session"),
                )
            });
        }
    }

    let db = glib::user_config_dir().join("dconf/user");
    checks.push(if db.is_file() {
        check("dconf_database", "ok", db.display().to_string(), None)
    } else {
        check(
            "dconf_database",
            "warn",
            format!("{} not found", db.display()),
            Some("current values will all read as defaults: a sandbox may be hiding ~/.config/dconf (or nothing was ever changed)"),
        )
    });

    let mode = ext::session_mode();
    checks.push(check(
        "session_mode",
        if mode.source == "guess" { "warn" } else { "ok" },
        json!({ "name": mode.name, "source": mode.source }),
        (mode.source == "guess").then_some("session environment not visible; guessed from the installed mode files"),
    ));

    let total = index.entries().len();
    let from_ext = index.entries().iter().filter(|e| matches!(e.origin, Origin::Extension { .. })).count();
    checks.push(check("schemas", "ok", json!({ "total": total, "from_extension_dirs": from_ext }), None));
    checks.push(check("journal", "ok", journal::path().display().to_string(), None));

    let ok = !checks.iter().any(|c| c["status"] == "error");
    let warnings = checks.iter().filter(|c| c["status"] == "warn").count();
    Ok(json!({ "ok": ok, "warnings": warnings, "checks": checks }))
}
