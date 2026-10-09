//! Keys that could cut off remote access or input. Changing them needs `--yes`.

struct Rule {
    /// Schema id, or a prefix ending in `.` (matches the schema and its children).
    schema: &'static str,
    /// Key name, or `None` for every key in the schema.
    key: Option<&'static str>,
    reason: &'static str,
}

const RULES: &[Rule] = &[
    Rule { schema: "org.gnome.desktop.remote-desktop.", key: None, reason: "can cut off remote desktop access" },
    Rule {
        schema: "org.gnome.desktop.input-sources",
        key: None,
        reason: "can leave the keyboard typing the wrong layout",
    },
    Rule {
        schema: "org.gnome.desktop.screensaver",
        key: Some("lock-enabled"),
        reason: "changes whether the session locks",
    },
    Rule {
        schema: "org.gnome.desktop.lockdown",
        key: Some("disable-lock-screen"),
        reason: "changes whether the session can lock",
    },
    Rule {
        schema: "org.gnome.desktop.a11y.",
        key: None,
        reason: "accessibility settings can make input or the screen hard to use",
    },
    Rule {
        schema: "org.gnome.shell",
        key: Some("enabled-extensions"),
        reason: "replaces the whole list of enabled extensions",
    },
    Rule {
        schema: "org.gnome.shell",
        key: Some("disable-user-extensions"),
        reason: "can disable every extension at once",
    },
];

/// Why `schema key` is guarded, or `None` if it isn't.
pub fn check(schema: &str, key: &str) -> Option<&'static str> {
    RULES
        .iter()
        .find(|r| {
            let schema_matches = match r.schema.strip_suffix('.') {
                Some(base) => schema == base || schema.starts_with(r.schema),
                None => schema == r.schema,
            };
            schema_matches && r.key.is_none_or(|k| k == key)
        })
        .map(|r| r.reason)
}

#[cfg(test)]
mod tests {
    use super::check;

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
    }

    #[test]
    fn ordinary_keys() {
        assert!(check("org.gnome.desktop.screensaver", "lock-delay").is_none());
        assert!(check("org.gnome.desktop.interface", "color-scheme").is_none());
        assert!(check("org.gnome.shell", "favorite-apps").is_none());
        assert!(check("org.gnome.desktop.input-sources-extra", "x").is_none());
        assert!(check("org.gnome.desktop.a11yish", "x").is_none());
    }
}
