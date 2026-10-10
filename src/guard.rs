//! Guarded keys: changing them needs `--yes`. Two tiers: keys that could cut
//! off remote access or input, and keys whose value runs code (`executes`).
//!
//! Guards apply to the dconf location written, not just the schema id: a
//! relocatable schema written with `--path` is checked as the schema that
//! conventionally lives at that path (`/org/gnome/desktop/a11y/keyboard/` →
//! `org.gnome.desktop.a11y.keyboard`), and `--path` can't point at an
//! installed fixed schema's path at all (see `Entry::settings`).

use serde_json::{Value, json};

struct Rule {
    /// Schema id, or a prefix ending in `.` (matches the schema and its children).
    schema: &'static str,
    /// Key name, a prefix ending in `*`, or `None` for every key in the schema.
    key: Option<&'static str>,
    reason: &'static str,
    /// The value is a command or code that gets run.
    executes: bool,
}

const fn rule(schema: &'static str, key: Option<&'static str>, reason: &'static str) -> Rule {
    Rule { schema, key, reason, executes: false }
}

const fn runs(schema: &'static str, key: Option<&'static str>, reason: &'static str) -> Rule {
    Rule { schema, key, reason, executes: true }
}

/// First match wins.
const RULES: &[Rule] = &[
    // Values that run code.
    runs(
        "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding",
        Some("command"),
        "runs this shell command when the shortcut is pressed",
    ),
    runs(
        "org.gnome.settings-daemon.plugins.media-keys",
        Some("custom-keybindings"),
        "activates custom shortcuts, which run shell commands",
    ),
    runs("org.gnome.Ptyxis.Profile", Some("custom-command"), "runs this command in the profile's terminals"),
    runs("org.gnome.Ptyxis.Profile", Some("use-custom-command"), "runs the profile's custom command in its terminals"),
    runs("org.gnome.desktop.default-applications.terminal", Some("exec*"), "chooses the program run as the terminal"),
    runs("org.gnome.desktop.media-handling", Some("autorun-*"), "chooses programs run when media is inserted"),
    runs("org.gnome.desktop.screensaver", Some("logout-command"), "runs this command to log out"),
    runs(
        "org.gnome.desktop.screensaver",
        Some("embedded-keyboard-command"),
        "runs this command as the on-screen keyboard",
    ),
    runs(
        "org.gnome.shell",
        Some("development-tools"),
        "enables Looking Glass, which runs JavaScript inside GNOME Shell",
    ),
    runs(
        "org.gnome.shell",
        Some("disable-extension-version-validation"),
        "lets extensions written for other GNOME versions run inside GNOME Shell",
    ),
    runs(
        "org.gnome.shell",
        Some("enabled-extensions"),
        "replaces the whole list of enabled extensions, which run code inside GNOME Shell",
    ),
    // Values that can cut off remote access or input, or weaken the lock.
    rule("org.gnome.desktop.remote-desktop.", None, "can cut off remote desktop access"),
    rule("org.gnome.desktop.input-sources", None, "can leave the keyboard typing the wrong layout"),
    rule("org.gnome.desktop.screensaver", Some("lock-enabled"), "changes whether the session locks"),
    rule("org.gnome.desktop.screensaver", Some("lock-delay"), "changes how soon the session locks"),
    rule(
        "org.gnome.desktop.screensaver",
        Some("idle-activation-enabled"),
        "changes whether the screen shield comes up when idle",
    ),
    rule(
        "org.gnome.desktop.session",
        Some("idle-delay"),
        "changes when the session goes idle (screen blank, shield, lock)",
    ),
    rule("org.gnome.desktop.lockdown", None, "lockdown settings can disable locking, logging out or other safeguards"),
    rule("org.gnome.desktop.a11y.", None, "accessibility settings can make input or the screen hard to use"),
    rule("org.gnome.system.proxy.", None, "proxy settings can send network traffic through another host"),
    rule(
        "org.gnome.settings-daemon.plugins.power",
        Some("sleep-inactive-*"),
        "can suspend the machine when idle, cutting off remote access",
    ),
    rule("org.gnome.shell", Some("disabled-extensions"), "replaces the whole list of disabled extensions"),
    rule("org.gnome.shell", Some("disable-user-extensions"), "can disable every extension at once"),
    rule("org.gnome.mutter.wayland", Some("xwayland-allow-grabs"), "lets X11 apps grab the keyboard and pointer"),
];

/// Why a key is guarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guard {
    pub reason: &'static str,
    /// The value is a command or code that gets run, so a change runs code
    /// chosen by whoever wrote the value.
    pub executes: bool,
}

impl Guard {
    pub fn to_json(&self) -> Value {
        json!({ "reason": self.reason, "executes": self.executes })
    }
}

fn find(schema: &str, key: &str) -> Option<Guard> {
    RULES
        .iter()
        .find(|r| {
            let schema_matches = match r.schema.strip_suffix('.') {
                Some(base) => schema == base || schema.starts_with(r.schema),
                None => schema == r.schema,
            };
            let key_matches = match r.key {
                None => true,
                Some(k) => match k.strip_suffix('*') {
                    Some(prefix) => key.starts_with(prefix),
                    None => k == key,
                },
            };
            schema_matches && key_matches
        })
        .map(|r| Guard { reason: r.reason, executes: r.executes })
}

/// Why `schema key` is guarded, or `None` if it isn't. Ignores the instance
/// path; writes should use [`check_at`].
pub fn check(schema: &str, key: &str) -> Option<&'static str> {
    find(schema, key).map(|g| g.reason)
}

/// The guard on writing `key` of `schema` at `path` (the `--path` of a
/// relocatable schema, `None` for a fixed one): the schema's own guard, or
/// the guard of the schema that conventionally lives at `path`.
pub fn check_at(schema: &str, path: Option<&str>, key: &str) -> Option<Guard> {
    find(schema, key).or_else(|| find(&schema_id_for_path(path?), key))
}

/// The schema id GNOME's convention gives the schema at a dconf path:
/// `/org/gnome/desktop/a11y/keyboard/` → `org.gnome.desktop.a11y.keyboard`.
fn schema_id_for_path(path: &str) -> String {
    path.trim_matches('/').replace('/', ".")
}

#[cfg(test)]
mod tests {
    use super::{check, check_at, find};

    #[test]
    fn guarded_keys() {
        assert!(check("org.gnome.desktop.remote-desktop.rdp", "enable").is_some());
        assert!(check("org.gnome.desktop.remote-desktop", "anything").is_some());
        assert!(check("org.gnome.desktop.input-sources", "sources").is_some());
        assert!(check("org.gnome.desktop.screensaver", "lock-enabled").is_some());
        assert!(check("org.gnome.desktop.lockdown", "disable-lock-screen").is_some());
        assert!(check("org.gnome.desktop.a11y.keyboard", "slowkeys-enable").is_some());
        assert!(check("org.gnome.desktop.a11y.applications", "screen-reader-enabled").is_some());
        assert!(check("org.gnome.shell", "enabled-extensions").is_some());
        for (schema, key) in [
            ("org.gnome.system.proxy", "mode"),
            ("org.gnome.system.proxy.http", "host"),
            ("org.gnome.system.proxy.socks", "port"),
            ("org.gnome.desktop.lockdown", "disable-log-out"),
            ("org.gnome.desktop.session", "idle-delay"),
            ("org.gnome.desktop.screensaver", "lock-delay"),
            ("org.gnome.desktop.screensaver", "idle-activation-enabled"),
            ("org.gnome.settings-daemon.plugins.power", "sleep-inactive-ac-type"),
            ("org.gnome.settings-daemon.plugins.power", "sleep-inactive-battery-timeout"),
            ("org.gnome.shell", "disabled-extensions"),
            ("org.gnome.mutter.wayland", "xwayland-allow-grabs"),
        ] {
            assert!(!find(schema, key).expect(key).executes, "{schema} {key}");
        }
    }

    #[test]
    fn code_executing_keys() {
        for (schema, key) in [
            ("org.gnome.settings-daemon.plugins.media-keys.custom-keybinding", "command"),
            ("org.gnome.settings-daemon.plugins.media-keys", "custom-keybindings"),
            ("org.gnome.Ptyxis.Profile", "custom-command"),
            ("org.gnome.Ptyxis.Profile", "use-custom-command"),
            ("org.gnome.desktop.default-applications.terminal", "exec"),
            ("org.gnome.desktop.default-applications.terminal", "exec-arg"),
            ("org.gnome.desktop.media-handling", "autorun-x-content-start-app"),
            ("org.gnome.desktop.media-handling", "autorun-never"),
            ("org.gnome.shell", "development-tools"),
            ("org.gnome.shell", "disable-extension-version-validation"),
            ("org.gnome.shell", "enabled-extensions"),
        ] {
            assert!(find(schema, key).expect(key).executes, "{schema} {key}");
        }
    }

    #[test]
    fn ordinary_keys() {
        assert!(check("org.gnome.settings-daemon.plugins.media-keys.custom-keybinding", "name").is_none());
        assert!(check("org.gnome.settings-daemon.plugins.media-keys", "screensaver").is_none());
        assert!(check("org.gnome.settings-daemon.plugins.power", "idle-dim").is_none());
        assert!(check("org.gnome.desktop.media-handling", "automount").is_none());
        assert!(check("org.gnome.desktop.session", "session-name").is_none());
        assert!(check("org.gnome.mutter.wayland", "xwayland-scaling-factor").is_none());
        assert!(check("org.gnome.system.proxyish", "x").is_none());
        assert!(check("org.gnome.desktop.interface", "color-scheme").is_none());
        assert!(check("org.gnome.shell", "favorite-apps").is_none());
        assert!(check("org.gnome.desktop.input-sources-extra", "x").is_none());
        assert!(check("org.gnome.desktop.a11yish", "x").is_none());
    }

    #[test]
    fn guards_follow_the_path() {
        let reloc = "org.example.reloc";
        assert!(check_at(reloc, Some("/org/gnome/desktop/a11y/keyboard/"), "enable").is_some());
        assert!(check_at(reloc, Some("/org/gnome/desktop/remote-desktop/rdp/"), "enable").is_some());
        assert!(check_at(reloc, Some("/org/gnome/desktop/screensaver/"), "lock-enabled").is_some());
        assert!(check_at(reloc, Some("/org/gnome/desktop/screensaver/"), "other").is_none());
        assert!(check_at(reloc, Some("/org/example/a11y/"), "enable").is_none());
        assert!(check_at(reloc, None, "enable").is_none());
        assert!(check_at("org.gnome.desktop.a11y.keyboard", None, "enable").is_some());
    }
}
