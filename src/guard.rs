//! Guarded keys: changing them needs `--yes`. Two tiers: keys that could cut
//! off remote access or input, and keys whose value runs code (`executes`).
//!
//! Guards apply to the dconf location written, not just the schema id: a
//! relocatable schema written with `--path` is checked as the schema that
//! conventionally lives at that path (`/org/gnome/desktop/a11y/keyboard/` →
//! `org.gnome.desktop.a11y.keyboard`), and `--path` can't point at an
//! installed fixed schema's path at all (see `Entry::settings`).
//!
//! Some guards only apply below a floor (a key-repeat `delay` under 100 ms):
//! see [`Guard::applies_to`]. Extension keys that look like commands are guarded by name and type,
//! since no list can cover every extension.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

struct Rule {
    /// Schema id, or a prefix ending in `.` (matches the schema and its children).
    schema: &'static str,
    /// Key name, a prefix ending in `*`, or `None` for every key in the schema.
    key: Option<&'static str>,
    reason: &'static str,
    /// The value is a command or code that gets run.
    executes: bool,
    /// Only values below this need `--yes` (for integer keys).
    below: Option<u32>,
    /// Writes that run less (see [`runs_less`]) don't need `--yes`.
    runs_less_ok: bool,
}

const fn rule(schema: &'static str, key: Option<&'static str>, reason: &'static str) -> Rule {
    Rule { schema, key, reason, executes: false, below: None, runs_less_ok: false }
}

const fn runs(schema: &'static str, key: Option<&'static str>, reason: &'static str) -> Rule {
    Rule { schema, key, reason, executes: true, below: None, runs_less_ok: true }
}

/// Guarded only when the new value is below `min`.
const fn floor(schema: &'static str, key: &'static str, min: u32, reason: &'static str) -> Rule {
    Rule { schema, key: Some(key), reason, executes: false, below: Some(min), runs_less_ok: false }
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
    runs(
        "org.gnome.desktop.default-applications.",
        Some("exec*"),
        "chooses the program run as this default application",
    ),
    runs(
        "org.gnome.evolution-data-server.calendar",
        Some("notify-programs"),
        "lists programs that calendar reminders may run",
    ),
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
    // Emptying the list wholesale still needs --yes; `ext disable` edits one uuid.
    Rule {
        schema: "org.gnome.shell",
        key: Some("enabled-extensions"),
        reason: "replaces the whole list of enabled extensions, which run code inside GNOME Shell",
        executes: true,
        below: None,
        runs_less_ok: false,
    },
    // Values that can cut off remote access or input, or weaken the lock.
    rule("org.gnome.desktop.remote-desktop.", None, "can cut off remote desktop access"),
    rule("org.gnome.desktop.input-sources", None, "can leave the keyboard typing the wrong layout"),
    rule("org.gnome.desktop.screensaver", Some("lock-enabled"), "changes whether the session locks"),
    rule("org.gnome.desktop.screensaver", Some("lock-delay"), "changes how soon the session locks"),
    rule(
        "org.gnome.desktop.screensaver",
        Some("ubuntu-lock-on-suspend"),
        "changes whether the session locks on suspend",
    ),
    rule(
        "org.gnome.desktop.screensaver",
        Some("logout-enabled"),
        "changes whether the unlock dialog offers logging out",
    ),
    rule(
        "org.gnome.desktop.screensaver",
        Some("user-switch-enabled"),
        "changes whether the unlock dialog offers switching user",
    ),
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
    rule(
        "org.gnome.desktop.interface",
        Some("toolkit-accessibility"),
        "turning it off cuts screen readers and other assistive tools off from apps",
    ),
    floor(
        "org.gnome.desktop.peripherals.keyboard",
        "delay",
        100,
        "a key-repeat delay under 100 ms repeats keys before they can be released",
    ),
    floor(
        "org.gnome.desktop.peripherals.keyboard",
        "repeat-interval",
        10,
        "a key-repeat interval under 10 ms floods input with repeated keys",
    ),
    rule("org.gnome.desktop.peripherals.touchpad", Some("send-events"), "can turn the touchpad off"),
    rule("org.gnome.desktop.notifications", Some("show-banners"), "hides alerts, including future gsctl notifications"),
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
    /// Only new values below this need `--yes`.
    pub below: Option<u32>,
    /// A write that runs less than the current value needs no `--yes`.
    pub runs_less_ok: bool,
}

impl Guard {
    pub fn to_json(&self) -> Value {
        let mut out = json!({ "reason": self.reason, "executes": self.executes });
        if let Some(min) = self.below {
            out["below"] = min.into();
        }
        out
    }

    /// Whether writing `new` needs `--yes`: always, unless the guard has a
    /// floor and `new` is an integer at or above it.
    pub fn applies_to(&self, new: &glib::Variant) -> bool {
        let Some(min) = self.below else { return true };
        match new.get::<u32>().map(i64::from).or_else(|| new.get::<i32>().map(i64::from)) {
            Some(v) => v < i64::from(min),
            None => true,
        }
    }
}

/// Reason for the extension-key heuristic.
const EXTENSION_COMMAND: &str = "an extension key named like a command; the extension may run its value";

/// Extension keys that look like they hold a command: a string or string
/// list named `*command*` or `exec*`, in an extension schema (or one at an
/// extension's conventional path). A heuristic: extensions don't declare
/// which keys they run.
fn extension_command(schema: &str, path: Option<&str>, key: &str, ty: &str, from_extension: bool) -> Option<Guard> {
    const NAMESPACE: &str = "org.gnome.shell.extensions.";
    let in_namespace = |id: &str| id.starts_with(NAMESPACE);
    let extension =
        from_extension || in_namespace(schema) || path.is_some_and(|p| in_namespace(&schema_id_for_path(p)));
    let named = key.contains("command") || key.starts_with("exec");
    let text = matches!(ty, "s" | "ms" | "as");
    (extension && named && text).then_some(Guard {
        reason: EXTENSION_COMMAND,
        executes: true,
        below: None,
        runs_less_ok: true,
    })
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
        .map(|r| Guard { reason: r.reason, executes: r.executes, below: r.below, runs_less_ok: r.runs_less_ok })
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

/// [`check_at`], then the extension-command heuristic for a key of type
/// `ty` (a GVariant type string) in a schema that may come from an extension.
pub fn check_typed(schema: &str, path: Option<&str>, key: &str, ty: &str, from_extension: bool) -> Option<Guard> {
    check_at(schema, path, key).or_else(|| extension_command(schema, path, key, ty, from_extension))
}

/// The guard on writing `key` of an indexed schema at `path` (`None` for a
/// fixed schema, whose own path is then checked too). What every write and
/// listing uses. A guard with a floor still needs [`Guard::applies_to`] for
/// the value written: see [`check_write`].
pub fn check_entry(entry: &crate::index::Entry, path: Option<&str>, key: &str) -> Option<Guard> {
    let fixed = entry.schema.path();
    let ty =
        if entry.schema.has_key(key) { entry.schema.key(key).value_type().as_str().to_string() } else { String::new() };
    let from_extension = matches!(entry.origin, crate::index::Origin::Extension { .. });
    check_typed(&entry.id(), path.or(fixed.as_deref()), key, &ty, from_extension)
}

/// The guard on writing `new` to `key`: [`check_entry`], unless `new`
/// clears the guard's floor, or the guard allows it and `new` runs nothing
/// the current value doesn't (see [`runs_less`]).
pub fn check_write(entry: &crate::index::Entry, path: Option<&str>, key: &str, new: &glib::Variant) -> Option<Guard> {
    let guard = check_entry(entry, path, key).filter(|g| g.applies_to(new))?;
    if guard.runs_less_ok
        && let Ok(settings) = entry.settings(path)
        && runs_less(&gio::prelude::SettingsExt::value(&settings, key), new)
    {
        return None;
    }
    Some(guard)
}

/// Whether `new` runs nothing that `old` doesn't: an empty string, or a list
/// whose entries are all in `old`. So removing a shortcut's command, or
/// undoing `keys add`, needs no `--yes`. Booleans never count: which value is
/// the safe one depends on the key.
fn runs_less(old: &glib::Variant, new: &glib::Variant) -> bool {
    if let Some(s) = new.str() {
        return s.is_empty() || old.str() == Some(s);
    }
    match (old.get::<Vec<String>>(), new.get::<Vec<String>>()) {
        (Some(old), Some(new)) => new.iter().all(|n| old.contains(n)),
        _ => false,
    }
}

/// Whether a key of type `ty` (a GVariant type string) holds a secret, like
/// `org.gnome.system.proxy.http authentication-password`: a string-like key
/// whose name mentions a password (`disable-show-password`, a boolean, isn't
/// one). Secrets aren't exported and their values aren't journaled.
pub fn is_secret(key: &str, ty: &str) -> bool {
    key.contains("password") && matches!(ty, "s" | "ms" | "ay" | "as")
}

/// Set by `--show-secrets` for the rest of the process.
static SHOW_SECRETS: AtomicBool = AtomicBool::new(false);

/// Show secret values in output instead of [`redacted`] (`--show-secrets`).
pub fn show_secrets(on: bool) {
    SHOW_SECRETS.store(on, Ordering::Relaxed);
}

/// What output shows in place of a secret value.
pub fn redacted() -> Value {
    json!({ "redacted": true })
}

/// `v`, the value of `key`, as output JSON: [`redacted`] if it's a secret,
/// unless `--show-secrets`.
pub fn shown(key: &str, v: &glib::Variant) -> Value {
    if is_secret(key, v.type_().as_str()) && !SHOW_SECRETS.load(Ordering::Relaxed) {
        redacted()
    } else {
        crate::convert::to_json(v)
    }
}

/// The schema id GNOME's convention gives the schema at a dconf path:
/// `/org/gnome/desktop/a11y/keyboard/` → `org.gnome.desktop.a11y.keyboard`.
fn schema_id_for_path(path: &str) -> String {
    path.trim_matches('/').replace('/', ".")
}

#[cfg(test)]
mod tests {
    use super::{check, check_at, check_typed, find};
    use glib::prelude::*;

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
            ("org.gnome.desktop.screensaver", "ubuntu-lock-on-suspend"),
            ("org.gnome.desktop.screensaver", "logout-enabled"),
            ("org.gnome.desktop.screensaver", "user-switch-enabled"),
            ("org.gnome.desktop.interface", "toolkit-accessibility"),
            ("org.gnome.desktop.peripherals.touchpad", "send-events"),
            ("org.gnome.desktop.notifications", "show-banners"),
            ("org.gnome.desktop.peripherals.keyboard", "delay"),
            ("org.gnome.desktop.peripherals.keyboard", "repeat-interval"),
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
            ("org.gnome.desktop.default-applications.office.calendar", "exec"),
            ("org.gnome.desktop.default-applications.office.tasks", "exec"),
            ("org.gnome.desktop.default-applications.anything-new", "exec-whatever"),
            ("org.gnome.evolution-data-server.calendar", "notify-programs"),
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
        assert!(check("org.gnome.desktop.peripherals.touchpad", "tap-to-click").is_none());
        assert!(check("org.gnome.desktop.peripherals.keyboard", "repeat").is_none());
        assert!(check("org.gnome.desktop.notifications", "show-in-lock-screen").is_none());
        assert!(check("org.gnome.shell", "favorite-apps").is_none());
        assert!(check("org.gnome.desktop.input-sources-extra", "x").is_none());
        assert!(check("org.gnome.desktop.a11yish", "x").is_none());
        assert!(check("org.gnome.desktop.default-applications.office.calendar", "needs-term").is_none());
        assert!(check("org.gnome.evolution-data-server.calendar", "notify-enable-audio").is_none());
    }

    #[test]
    fn floors_guard_only_low_values() {
        let delay = find("org.gnome.desktop.peripherals.keyboard", "delay").unwrap();
        assert_eq!(delay.below, Some(100));
        assert!(delay.applies_to(&50u32.to_variant()));
        assert!(delay.applies_to(&0u32.to_variant()));
        assert!(!delay.applies_to(&100u32.to_variant()));
        assert!(!delay.applies_to(&500u32.to_variant()));
        assert!(delay.applies_to(&(-1i32).to_variant()));
        assert!(!delay.applies_to(&200i32.to_variant()));
        // Not an integer: can't tell, so guarded.
        assert!(delay.applies_to(&"x".to_variant()));
        let interval = find("org.gnome.desktop.peripherals.keyboard", "repeat-interval").unwrap();
        assert!(interval.applies_to(&9u32.to_variant()));
        assert!(!interval.applies_to(&10u32.to_variant()));
        // No floor: every value.
        let lock = find("org.gnome.desktop.screensaver", "lock-enabled").unwrap();
        assert!(lock.applies_to(&true.to_variant()));
        assert_eq!(find("org.gnome.desktop.peripherals.keyboard", "repeat"), None);
        assert_eq!(lock.to_json().get("below"), None);
        assert_eq!(delay.to_json()["below"], 100);
    }

    #[test]
    fn extension_keys_named_like_commands() {
        let ext = "org.gnome.shell.extensions.demo";
        for (key, ty) in [("launch-command", "s"), ("commands", "as"), ("exec", "s"), ("exec-on-click", "ms")] {
            let g = check_typed(ext, None, key, ty, false).expect(key);
            assert!(g.executes, "{key}");
        }
        // Not text, or not named like a command.
        assert!(check_typed(ext, None, "use-custom-command", "b", false).is_none());
        assert!(check_typed(ext, None, "command-count", "i", false).is_none());
        assert!(check_typed(ext, None, "label", "s", false).is_none());
        assert!(check_typed(ext, None, "executable-bit", "b", false).is_none());
        // Outside the extension namespace only if it came from an extension
        // or is written at an extension's path.
        assert!(check_typed("org.example.app", None, "command", "s", false).is_none());
        assert!(check_typed("org.example.app", None, "command", "s", true).is_some());
        let at = Some("/org/gnome/shell/extensions/demo/");
        assert!(check_typed("org.example.reloc", at, "command", "s", false).is_some());
        // Listed rules win over the heuristic.
        let g = check_typed("org.gnome.desktop.default-applications.terminal", None, "exec", "s", false).unwrap();
        assert_eq!(g.reason, "chooses the program run as the terminal");
    }

    #[test]
    fn runs_less_only_when_nothing_new_runs() {
        use glib::prelude::*;
        let list = |v: &[&str]| v.to_variant();
        assert!(super::runs_less(&list(&["/a/", "/b/"]), &list(&["/a/"])));
        assert!(super::runs_less(&list(&["/a/"]), &list(&[])));
        assert!(!super::runs_less(&list(&["/a/"]), &list(&["/a/", "/b/"])));
        assert!(super::runs_less(&"ptyxis".to_variant(), &"".to_variant()));
        assert!(!super::runs_less(&"".to_variant(), &"ptyxis".to_variant()));
        assert!(!super::runs_less(&true.to_variant(), &false.to_variant()));
    }

    #[test]
    fn secrets() {
        assert!(super::is_secret("authentication-password", "s"));
        assert!(!super::is_secret("disable-show-password", "b"));
        assert!(!super::is_secret("remember-mount-password", "b"));
        assert!(!super::is_secret("authentication-user", "s"));
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
