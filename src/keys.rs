//! Keyboard shortcuts across every keybinding schema (GNOME's own and
//! extensions') plus custom bindings.

use std::collections::HashMap;

use gio::prelude::*;
use glib::Variant;
use serde_json::{Value, json};

use crate::commands::{Action, BatchItem, Tags, WriteOpts, write, write_batch};
use crate::error::{Error, Result};
use crate::ext;
use crate::index::{Entry, Index, Origin};
use crate::journal;
use crate::state::CUSTOM_KEYBINDING_SCHEMA;

/// GNOME's own schemas whose keys are keybindings (`as`, or `s` in some
/// older media keys), with the short name used in messages.
pub const KEYBINDING_SCHEMAS: &[&str] = &[
    "org.gnome.desktop.wm.keybindings",
    "org.gnome.shell.keybindings",
    "org.gnome.mutter.keybindings",
    "org.gnome.mutter.wayland.keybindings",
    MEDIA_KEYS,
];
const SHORT_NAMES: &[&str] = &["wm", "shell", "mutter", "mutter-wayland", "media-keys"];
pub const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const EXTENSION_PREFIX: &str = "org.gnome.shell.extensions.";
const CUSTOM_LIST_KEY: &str = "custom-keybindings";
const CUSTOM_BASE: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";

const SYNTAX_HINT: &str = "GTK accelerator syntax: modifiers in angle brackets, then one key name, e.g. '<Super>t', \
     '<Control><Alt>Delete', '<Super>Return', '<Shift><Super>Page_Up', 'XF86AudioMute'. Modifiers: Shift, \
     Control (Ctrl, Primary), Alt, Super, Meta, Hyper, Mod2-5, Lock, Release";

/// Modifiers in canonical order, with the spellings GTK and mutter accept.
const MODIFIERS: &[(&str, &[&str])] = &[
    ("Control", &["control", "ctrl", "ctl", "primary"]),
    ("Shift", &["shift", "shft"]),
    ("Alt", &["alt", "mod1"]),
    ("Super", &["super", "mod4"]),
    ("Meta", &["meta"]),
    ("Hyper", &["hyper"]),
    ("Mod2", &["mod2"]),
    ("Mod3", &["mod3"]),
    ("Mod5", &["mod5"]),
    ("Lock", &["lock"]),
    ("Release", &["release"]),
];

/// Keysym names beyond letters, digits and the `F1`, `KP_*`, `XF86*`,
/// `dead_*`, `0x…` and `U…` patterns: special keys, punctuation and Latin-1.
/// Matched case-insensitively, as mutter does.
const KEY_NAMES: &[&str] = &[
    // Special keys.
    "Return",
    "Tab",
    "ISO_Left_Tab",
    "space",
    "Escape",
    "Print",
    "Sys_Req",
    "Home",
    "End",
    "Page_Up",
    "Prior",
    "Page_Down",
    "Next",
    "Left",
    "Right",
    "Up",
    "Down",
    "Delete",
    "BackSpace",
    "Insert",
    "Pause",
    "Break",
    "Scroll_Lock",
    "Num_Lock",
    "Caps_Lock",
    "Menu",
    "Above_Tab",
    "Linefeed",
    "Clear",
    "Help",
    "Undo",
    "Redo",
    "Find",
    "Cancel",
    "Execute",
    "Select",
    "Begin",
    "Multi_key",
    "Mode_switch",
    "Super_L",
    "Super_R",
    "Alt_L",
    "Alt_R",
    "Control_L",
    "Control_R",
    "Shift_L",
    "Shift_R",
    "Meta_L",
    "Meta_R",
    "Hyper_L",
    "Hyper_R",
    "ISO_Level3_Shift",
    "ISO_Level5_Shift",
    "ISO_Next_Group",
    "ISO_Prev_Group",
    "ISO_First_Group",
    "ISO_Last_Group",
    // Punctuation.
    "exclam",
    "quotedbl",
    "numbersign",
    "dollar",
    "percent",
    "ampersand",
    "apostrophe",
    "quoteright",
    "parenleft",
    "parenright",
    "asterisk",
    "plus",
    "comma",
    "minus",
    "period",
    "slash",
    "colon",
    "semicolon",
    "less",
    "equal",
    "greater",
    "question",
    "at",
    "bracketleft",
    "backslash",
    "bracketright",
    "asciicircum",
    "underscore",
    "grave",
    "quoteleft",
    "braceleft",
    "bar",
    "braceright",
    "asciitilde",
    // Latin-1 symbols.
    "nobreakspace",
    "exclamdown",
    "cent",
    "sterling",
    "currency",
    "yen",
    "brokenbar",
    "section",
    "diaeresis",
    "copyright",
    "ordfeminine",
    "guillemotleft",
    "notsign",
    "hyphen",
    "registered",
    "macron",
    "degree",
    "plusminus",
    "twosuperior",
    "threesuperior",
    "acute",
    "mu",
    "paragraph",
    "periodcentered",
    "cedilla",
    "onesuperior",
    "masculine",
    "guillemotright",
    "onequarter",
    "onehalf",
    "threequarters",
    "questiondown",
    "multiply",
    "division",
    "EuroSign",
    // Latin-1 letters (lower case; matching ignores case).
    "agrave",
    "aacute",
    "acircumflex",
    "atilde",
    "adiaeresis",
    "aring",
    "ae",
    "ccedilla",
    "egrave",
    "eacute",
    "ecircumflex",
    "ediaeresis",
    "igrave",
    "iacute",
    "icircumflex",
    "idiaeresis",
    "eth",
    "ntilde",
    "ograve",
    "oacute",
    "ocircumflex",
    "otilde",
    "odiaeresis",
    "oslash",
    "ooblique",
    "ugrave",
    "uacute",
    "ucircumflex",
    "udiaeresis",
    "yacute",
    "thorn",
    "ssharp",
    "ydiaeresis",
];

/// Everyday names for keys, for suggestions only.
const KEY_ALIASES: &[(&str, &str)] = &[
    ("del", "Delete"),
    ("esc", "Escape"),
    ("enter", "Return"),
    ("ret", "Return"),
    ("bksp", "BackSpace"),
    ("pgup", "Page_Up"),
    ("pageup", "Page_Up"),
    ("pgdn", "Page_Down"),
    ("pgdown", "Page_Down"),
    ("pagedown", "Page_Down"),
    ("ins", "Insert"),
    ("spacebar", "space"),
    ("prtsc", "Print"),
    ("prtscr", "Print"),
    ("printscreen", "Print"),
    ("capslock", "Caps_Lock"),
    ("+", "plus"),
    (",", "comma"),
    (".", "period"),
    ("/", "slash"),
    ("\\", "backslash"),
    ("-", "minus"),
    ("=", "equal"),
    ("`", "grave"),
    ("'", "apostrophe"),
    (";", "semicolon"),
    ("[", "bracketleft"),
    ("]", "bracketright"),
    ("!", "exclam"),
    ("?", "question"),
];

/// Everyday names for modifiers, for suggestions only.
const MODIFIER_ALIASES: &[(&str, &str)] = &[
    ("win", "Super"),
    ("windows", "Super"),
    ("logo", "Super"),
    ("cmd", "Super"),
    ("command", "Super"),
    ("option", "Alt"),
    ("opt", "Alt"),
];

fn modifier_index(name: &str) -> Option<usize> {
    let lower = name.trim().to_ascii_lowercase();
    MODIFIERS.iter().position(|(_, aliases)| aliases.contains(&lower.as_str()))
}

fn all_alnum(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

/// The canonical spelling of a keysym name GTK/mutter would accept, or
/// `None` if it isn't one.
fn canonical_key(name: &str) -> Option<String> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return c.is_ascii_alphanumeric().then(|| c.to_ascii_lowercase().to_string());
    }
    if let Some(known) = KEY_NAMES.iter().find(|k| k.eq_ignore_ascii_case(name)) {
        return Some(known.to_string());
    }
    if let Some(n) = strip_prefix_ci(name, "F")
        && n.parse::<u8>().is_ok_and(|n| (1..=35).contains(&n))
        && !n.starts_with('0')
    {
        return Some(format!("F{n}"));
    }
    for prefix in ["KP_", "XF86", "dead_"] {
        if let Some(rest) = strip_prefix_ci(name, prefix)
            && all_alnum(rest)
        {
            return Some(format!("{prefix}{rest}"));
        }
    }
    let hex = |s: &str, len: std::ops::RangeInclusive<usize>| {
        len.contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
    };
    if strip_prefix_ci(name, "0x").is_some_and(|h| hex(h, 1..=8))
        || strip_prefix_ci(name, "U").is_some_and(|h| hex(h, 4..=6))
    {
        return Some(name.to_string());
    }
    None
}

/// A parsed accelerator: which modifiers, and the key.
struct Accel {
    mods: [bool; MODIFIERS.len()],
    key: String,
}

impl Accel {
    fn has_modifier(&self) -> bool {
        self.mods.iter().any(|m| *m)
    }

    fn canonical(&self) -> String {
        let mut out: String =
            MODIFIERS.iter().zip(self.mods).filter(|(_, on)| *on).map(|((m, _), _)| format!("<{m}>")).collect();
        out.push_str(&self.key);
        out
    }
}

/// Parse `<Mod>…key`. `None` for an empty or disabled binding. With
/// `strict`, the key must be a known keysym name; otherwise any word is
/// kept (so unusual bindings already on the system still compare).
fn parse(accel: &str, strict: bool) -> std::result::Result<Option<Accel>, String> {
    let accel = accel.trim();
    if accel.is_empty() || accel.eq_ignore_ascii_case("disabled") {
        return Ok(None);
    }
    let mut mods = [false; MODIFIERS.len()];
    let mut rest = accel;
    while let Some(after) = rest.strip_prefix('<') {
        let (name, tail) = after.split_once('>').ok_or_else(|| format!("unclosed '<' in {accel:?}"))?;
        let i = modifier_index(name).ok_or_else(|| format!("unknown modifier <{name}> in {accel:?}"))?;
        mods[i] = true;
        rest = tail;
    }
    if rest.is_empty() {
        return Err(format!("no key after the modifiers in {accel:?}"));
    }
    if rest.contains(['<', '>', ' ']) {
        return Err(format!("unexpected characters in key {rest:?} of {accel:?}"));
    }
    let key = match canonical_key(rest) {
        Some(k) => k,
        None if strict => return Err(format!("{rest:?} is not a key name GNOME knows, in {accel:?}")),
        None if rest.chars().count() == 1 => rest.to_lowercase(),
        None => rest.to_string(),
    };
    Ok(Some(Accel { mods, key }))
}

/// Canonical form of an accelerator, leniently: modifiers in a fixed order,
/// aliases unified (`<Primary>`/`<Ctrl>` → `<Control>`, `<Mod4>` →
/// `<Super>`), single letters lowercased, known key names spelled as GTK
/// does. `None` for an empty or disabled binding.
pub fn normalize(accel: &str) -> std::result::Result<Option<String>, String> {
    Ok(parse(accel, false)?.map(|a| a.canonical()))
}

/// Strictly validate an accelerator given as input: the canonical form, or
/// an `invalid_value` error with a `suggestion` for forms like `Super+T`.
pub fn validate(accel: &str) -> Result<Option<String>> {
    parse(accel, true).map(|a| a.map(|a| a.canonical())).map_err(|message| {
        let t = accel.trim();
        let message = if !t.contains('<') && t.len() > 1 && (t.contains('+') || t.contains(' ')) {
            format!("{t:?} isn't GTK accelerator syntax: modifiers go in angle brackets")
        } else {
            message
        };
        let suggestion = suggest(accel);
        let message = match &suggestion {
            Some(s) => format!("{message}; did you mean {s:?}?"),
            None => message,
        };
        Error::InvalidValue {
            message,
            details: json!({ "accel": accel, "suggestion": suggestion, "hint": SYNTAX_HINT }),
        }
    })
}

/// The GTK form of a human-written shortcut (`Super+T` → `<Super>t`,
/// `Ctrl+Alt+Del` → `<Control><Alt>Delete`, `<Super>Del` → `<Super>Delete`).
pub fn suggest(input: &str) -> Option<String> {
    let input = input.trim();
    let (mods, key): (Vec<&str>, &str) = if input.starts_with('<') {
        let mut mods = Vec::new();
        let mut rest = input;
        while let Some(after) = rest.strip_prefix('<') {
            let (name, tail) = after.split_once('>')?;
            mods.push(name);
            rest = tail;
        }
        (mods, rest.trim())
    } else {
        let (body, plus_key) = match input.strip_suffix("++") {
            Some(body) => (body, true),
            None => (input, false),
        };
        let mut parts: Vec<&str> = if body.contains('+') {
            body.split('+').map(str::trim).collect()
        } else {
            body.split_whitespace().collect()
        };
        let key = if plus_key { "+" } else { parts.pop()? };
        (parts, key)
    };
    let mut accel = Accel { mods: [false; MODIFIERS.len()], key: String::new() };
    for m in mods {
        let i = modifier_index(m).or_else(|| {
            let (_, canon) = MODIFIER_ALIASES.iter().find(|(a, _)| a.eq_ignore_ascii_case(m.trim()))?;
            modifier_index(canon)
        })?;
        accel.mods[i] = true;
    }
    accel.key = canonical_key(key)
        .or_else(|| KEY_ALIASES.iter().find(|(a, _)| a.eq_ignore_ascii_case(key)).map(|(_, k)| k.to_string()))?;
    let out = accel.canonical();
    (out != input).then_some(out)
}

/// Keys compare case-insensitively (`Return` = `return`), modifiers exactly.
fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

enum Source {
    Gnome(&'static str),
    /// An extension's schema; the uuid if gsctl knows which extension owns it.
    Extension(Option<String>),
    Custom {
        name: String,
        command: String,
    },
}

struct Binding {
    schema: String,
    key: String,
    path: Option<String>,
    accels: Vec<String>,
    summary: Option<String>,
    source: Source,
    /// False for an extension that isn't enabled: its bindings don't grab keys.
    active: bool,
}

impl Binding {
    /// How people name the binding: `wm close`, `custom "Terminal"`,
    /// `paperwm@paperwm.github.com take-window`.
    fn label(&self) -> String {
        match &self.source {
            Source::Gnome(short) => format!("{short} {}", self.key),
            Source::Extension(Some(uuid)) => format!("{uuid} {}", self.key),
            Source::Extension(None) => format!("{} {}", self.schema, self.key),
            Source::Custom { name, .. } => format!("custom {name:?}"),
        }
    }

    fn to_json(&self) -> Value {
        let source = match self.source {
            Source::Gnome(_) => "gnome",
            Source::Extension(_) => "extension",
            Source::Custom { .. } => "custom",
        };
        let mut v = json!({
            "schema": self.schema,
            "key": self.key,
            "path": self.path,
            "accels": self.accels,
            "summary": self.summary,
            "source": source,
            "label": self.label(),
            "active": self.active,
        });
        match &self.source {
            Source::Extension(uuid) => v["extension"] = json!(uuid),
            Source::Custom { name, command } => v["custom"] = json!({ "name": name, "command": command }),
            Source::Gnome(_) => {}
        }
        v
    }

    fn normalized(&self) -> Vec<String> {
        self.accels.iter().filter_map(|a| normalize(a).ok().flatten()).collect()
    }
}

fn strings_of(v: &Variant) -> Vec<String> {
    match v.str() {
        Some(s) => vec![s.to_string()],
        None => v.get::<Vec<String>>().unwrap_or_default(),
    }
}

/// The non-empty accelerators of a value, as stored (even unusual ones).
fn accels_of(v: &Variant) -> Vec<String> {
    strings_of(v).into_iter().filter(|a| !a.trim().is_empty() && !a.trim().eq_ignore_ascii_case("disabled")).collect()
}

/// Whether a value looks like a binding outside a keybinding schema: a
/// non-empty list of valid accelerators, at least one with a modifier (so a
/// list of words or letters doesn't count).
fn looks_bound(v: &Variant) -> bool {
    let accels = accels_of(v);
    let parsed: Vec<_> = accels.iter().map(|a| parse(a, true)).collect();
    !parsed.is_empty()
        && parsed.iter().all(|p| matches!(p, Ok(Some(_))))
        && parsed.iter().any(|p| matches!(p, Ok(Some(a)) if a.has_modifier()))
}

/// Whether `key` of an extension schema holds keybindings. In a schema
/// named for keybindings (like PaperWM's `….paperwm.keybindings`) every `as`
/// key counts, and `s` keys holding a valid accelerator; elsewhere only `as`
/// keys whose default or current value looks bound (dash-to-dock's
/// `app-hotkey-1`, not its `shortcut-text` string).
fn extension_key_is_binding(entry: &Entry, settings: &gio::Settings, key: &str) -> bool {
    let named = entry.id().to_ascii_lowercase().contains("keybinding");
    let ty = entry.schema.key(key).value_type();
    let default = entry.schema.key(key).default_value();
    match ty.as_str() {
        "as" => named || looks_bound(&default) || looks_bound(&settings.value(key)),
        "s" => {
            named
                && [default, settings.value(key)]
                    .iter()
                    .any(|v| matches!(parse(v.str().unwrap_or(""), true), Ok(Some(_))))
        }
        _ => false,
    }
}

fn custom_paths(index: &Index) -> Vec<String> {
    let Ok(media) = index.get(MEDIA_KEYS) else { return vec![] };
    if !media.schema.has_key(CUSTOM_LIST_KEY) {
        return vec![];
    }
    media.settings(None).map(|s| s.strv(CUSTOM_LIST_KEY).iter().map(|p| p.to_string()).collect()).unwrap_or_default()
}

/// Schema id → (owning extension, enabled), from `gsctl ext list`.
///
/// Several extensions can claim a schema (PaperWM's JS names Tiling
/// Assistant's), so the best claim wins: the extension whose metadata
/// `settings-schema` is the schema, then one whose uuid names it
/// (`tiling-assistant@ubuntu.com`), then any other.
fn extension_owners(index: &Index) -> HashMap<String, (String, bool)> {
    let mut best: HashMap<String, (u8, String, bool)> = HashMap::new();
    let Ok(list) = ext::list(index) else { return HashMap::new() };
    let installed = ext::installed();
    for e in list["extensions"].as_array().into_iter().flatten() {
        let (Some(uuid), enabled) = (e["uuid"].as_str(), e["enabled"] == true) else { continue };
        let declared = installed.iter().find(|x| x.uuid == uuid).and_then(|x| x.metadata["settings-schema"].as_str());
        for schema in e["schemas"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            let rank = claim_rank(schema, uuid, declared);
            if best.get(schema).is_none_or(|(r, ..)| rank > *r) {
                best.insert(schema.to_string(), (rank, uuid.to_string(), enabled));
            }
        }
    }
    best.into_iter().map(|(schema, (_, uuid, enabled))| (schema, (uuid, enabled))).collect()
}

/// How strongly extension `uuid` (declaring `declared` in its metadata)
/// owns `schema`.
fn claim_rank(schema: &str, uuid: &str, declared: Option<&str>) -> u8 {
    let name = uuid.split('@').next().unwrap_or(uuid);
    if declared == Some(schema) {
        3
    } else if schema.split('.').any(|part| part == name) {
        2
    } else {
        1
    }
}

/// Every binding with at least one accelerator: GNOME's, extensions', then
/// custom ones.
fn all_bindings(index: &Index) -> Result<Vec<Binding>> {
    let mut out = Vec::new();
    for (id, short) in KEYBINDING_SCHEMAS.iter().zip(SHORT_NAMES) {
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
            let source = Source::Gnome(short);
            out.push(Binding { schema: id.to_string(), key, path: None, accels, summary, source, active: true });
        }
    }

    let mut owners: Option<HashMap<String, (String, bool)>> = None;
    for entry in index.entries() {
        let id = entry.id();
        let bundled = matches!(entry.origin, Origin::Extension { .. });
        if entry.relocatable()
            || KEYBINDING_SCHEMAS.contains(&id.as_str())
            || !(bundled || id.starts_with(EXTENSION_PREFIX))
        {
            continue;
        }
        let Ok(settings) = entry.settings(None) else { continue };
        for key in entry.keys() {
            if !extension_key_is_binding(entry, &settings, &key) {
                continue;
            }
            let accels = accels_of(&settings.value(&key));
            if accels.is_empty() {
                continue;
            }
            let owners = owners.get_or_insert_with(|| extension_owners(index));
            let owner = owners.get(&id).cloned();
            let uuid = match &entry.origin {
                Origin::Extension { uuid, .. } => Some(uuid.clone()),
                Origin::System => owner.as_ref().map(|(u, _)| u.clone()),
            };
            // An owner gsctl can't find is assumed active, so `find` doesn't
            // call a taken accelerator free.
            let active = owner.is_none_or(|(_, enabled)| enabled);
            let summary = entry.schema.key(&key).summary().map(|s| s.to_string());
            let source = Source::Extension(uuid);
            out.push(Binding { schema: id.clone(), key, path: None, accels, summary, source, active });
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
                source: Source::Custom { name, command },
                active: true,
            });
        }
    }
    Ok(out)
}

pub fn list(index: &Index, custom_only: bool) -> Result<Value> {
    let bindings: Vec<Value> = all_bindings(index)?
        .iter()
        .filter(|b| !custom_only || matches!(b.source, Source::Custom { .. }))
        .map(Binding::to_json)
        .collect();
    Ok(json!({ "count": bindings.len(), "bindings": bindings }))
}

fn bound_to<'a>(bindings: &'a [Binding], wanted: &str) -> Vec<&'a Binding> {
    bindings.iter().filter(|b| b.normalized().iter().any(|n| same(n, wanted))).collect()
}

/// What's on `accel`. Bindings of disabled extensions are listed, but don't
/// make it taken (`free`).
pub fn find(index: &Index, accel: &str) -> Result<Value> {
    let wanted = validate(accel)?.ok_or_else(|| Error::Usage("give an accelerator, e.g. '<Super>t'".into()))?;
    let bindings = all_bindings(index)?;
    let matches = bound_to(&bindings, &wanted);
    Ok(json!({
        "accel": accel,
        "normalized": wanted,
        "free": !matches.iter().any(|b| b.active),
        "bindings": matches.iter().map(|b| b.to_json()).collect::<Vec<_>>(),
    }))
}

/// Accelerators bound to more than one active binding.
pub fn conflicts(index: &Index) -> Result<Value> {
    let bindings = all_bindings(index)?;
    let mut groups: Vec<(String, Vec<&Binding>)> = Vec::new();
    for b in bindings.iter().filter(|b| b.active) {
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

/// One planned write: `value` `None` resets the key.
struct Planned<'a> {
    entry: &'a Entry,
    key: String,
    path: Option<String>,
    value: Option<Variant>,
}

/// Make a batch of writes through the write layer, all or nothing as far as
/// checks go: every write is previewed first (writability, guards), and if
/// any needs `--yes` that wasn't given, the write layer refuses it before
/// anything is written. Returns the writes and whether any needs `--yes`.
fn run_plan(plan: &[Planned], op: &str, dry_run: bool, yes: bool, batch: &str) -> Result<(Vec<Value>, bool)> {
    let tags = Tags { reverts: None, batch: Some(batch) };
    let one = |p: &Planned, dry_run: bool, yes: bool| {
        let action = match &p.value {
            Some(v) => Action::Set(v.clone()),
            None => Action::Reset,
        };
        write(p.entry, &p.key, &WriteOpts { path: p.path.as_deref(), dry_run, yes }, op, action, tags)
    };
    let preview = plan.iter().map(|p| one(p, true, yes)).collect::<Result<Vec<_>>>()?;
    let requires_yes = preview.iter().any(|w| w["requires_yes"] == true);
    if dry_run {
        return Ok((preview, requires_yes));
    }
    if !yes && let Some(i) = preview.iter().position(|w| w["requires_yes"] == true) {
        // Refused (`guarded`) by the write layer, with its reason.
        one(&plan[i], false, false)?;
    }
    // One batch: a write failing partway rolls back the ones before it.
    let items = plan
        .iter()
        .map(|p| BatchItem {
            entry: p.entry,
            key: p.key.clone(),
            path: p.path.clone(),
            action: p.value.clone().map_or(Action::Reset, Action::Set),
            yes,
            reverts: None,
        })
        .collect();
    Ok((write_batch(items, op, Some(batch), false)?, requires_yes))
}

pub struct AddOpts<'a> {
    pub name: &'a str,
    pub accel: &'a str,
    pub command: &'a str,
    pub force: bool,
    pub dry_run: bool,
    pub yes: bool,
}

/// Create a custom keybinding at the first free `customN` path.
pub fn add(index: &Index, opts: &AddOpts) -> Result<Value> {
    let accel = opts.accel.trim();
    let normalized = validate(accel)?.ok_or_else(|| Error::InvalidValue {
        message: "empty accelerator".into(),
        details: json!({ "accel": opts.accel, "hint": SYNTAX_HINT }),
    })?;
    if opts.name.trim().is_empty() || opts.command.trim().is_empty() {
        return Err(Error::Usage("name and command must not be empty".into()));
    }
    let media = index.get(MEDIA_KEYS)?;
    let custom = index.get(CUSTOM_KEYBINDING_SCHEMA)?;
    let bindings = all_bindings(index)?;

    let same_name: Vec<&Binding> = bindings
        .iter()
        .filter(|b| matches!(&b.source, Source::Custom { name, .. } if name.trim() == opts.name.trim()))
        .collect();
    if !same_name.is_empty() && !opts.force {
        let existing: Vec<Value> = same_name
            .iter()
            .map(|b| {
                let Source::Custom { name, command } = &b.source else { unreachable!() };
                json!({ "path": b.path, "name": name, "accel": b.accels.join(" "), "command": command })
            })
            .collect();
        let first = &existing[0];
        return Err(Error::ConflictWith {
            id: opts.name.to_string(),
            message: format!(
                "a custom keybinding named {:?} already exists at {} ({} runs {}). Remove it first (`gsctl keys remove {}`), pick another name, or re-run with --force to add another",
                opts.name,
                first["path"].as_str().unwrap_or_default(),
                first["accel"].as_str().unwrap_or_default(),
                first["command"].as_str().unwrap_or_default(),
                first["path"].as_str().unwrap_or_default(),
            ),
            details: json!({ "existing": existing }),
        });
    }

    let taken: Vec<&Binding> = bound_to(&bindings, &normalized).into_iter().filter(|b| b.active).collect();
    if !taken.is_empty() && !opts.force {
        return Err(Error::ConflictWith {
            id: accel.to_string(),
            message: format!(
                "{accel} is already bound to {}. Pick another accelerator, or re-run with --force to add it anyway",
                taken.iter().map(|b| b.label()).collect::<Vec<_>>().join(", ")
            ),
            details: json!({ "bindings": taken.iter().map(|b| b.to_json()).collect::<Vec<_>>() }),
        });
    }

    let existing = custom_paths(index);
    let path = (0..)
        .map(|n| format!("{CUSTOM_BASE}custom{n}/"))
        .find(|p| !existing.contains(p) && !has_user_values(custom, p))
        .unwrap();
    let mut list = existing.clone();
    list.push(path.clone());

    let mut plan: Vec<Planned> = [("name", opts.name), ("command", opts.command), ("binding", accel)]
        .into_iter()
        .map(|(key, value)| Planned {
            entry: custom,
            key: key.into(),
            path: Some(path.clone()),
            value: Some(value.into()),
        })
        .collect();
    plan.push(Planned { entry: media, key: CUSTOM_LIST_KEY.into(), path: None, value: Some(list.to_variant()) });

    let batch = journal::new_id(Some("keys-add"));
    let (writes, requires_yes) = run_plan(&plan, "keys-add", opts.dry_run, opts.yes, &batch)?;

    Ok(json!({
        "dry_run": opts.dry_run,
        "path": path,
        "name": opts.name,
        "command": opts.command,
        "accel": accel,
        "normalized": normalized,
        "requires_yes": requires_yes,
        "batch": if opts.dry_run { Value::Null } else { batch.into() },
        "writes": writes,
    }))
}

/// Remove a custom keybinding, found by exact name, path, or `customN`:
/// drop it from the list, then reset its keys, as one batch (so `revert
/// last` brings it back).
pub fn remove(index: &Index, which: &str, dry_run: bool, yes: bool) -> Result<Value> {
    let media = index.get(MEDIA_KEYS)?;
    let custom = index.get(CUSTOM_KEYBINDING_SCHEMA)?;
    let existing = custom_paths(index);
    let by_path: Vec<&String> =
        existing.iter().filter(|p| *p == which || p.trim_end_matches('/').rsplit('/').next() == Some(which)).collect();
    let matches = if by_path.is_empty() {
        let mut named = Vec::new();
        for p in &existing {
            if custom.settings(Some(p)).is_ok_and(|s| s.string("name") == which) {
                named.push(p);
            }
        }
        named
    } else {
        by_path
    };
    let path = match matches.as_slice() {
        [] => return Err(Error::BindingNotFound(which.to_string())),
        [p] => (*p).clone(),
        several => {
            return Err(Error::Usage(format!(
                "{} custom keybindings are named {which:?}; remove one by path: {}",
                several.len(),
                several.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", ")
            )));
        }
    };
    let settings = custom.settings(Some(&path))?;
    let name = settings.string("name").to_string();
    let accel = settings.string("binding").to_string();
    let command = settings.string("command").to_string();

    let list: Vec<&String> = existing.iter().filter(|p| **p != path).collect();
    // Back to the default rather than pinning an empty list.
    let value = if list.is_empty() { None } else { Some(list.to_variant()) };
    let mut plan = vec![Planned { entry: media, key: CUSTOM_LIST_KEY.into(), path: None, value }];
    for key in custom.keys() {
        if settings.user_value(&key).is_some() {
            plan.push(Planned { entry: custom, key, path: Some(path.clone()), value: None });
        }
    }
    let batch = journal::new_id(Some("keys-remove"));
    let (writes, requires_yes) = run_plan(&plan, "keys-remove", dry_run, yes, &batch)?;

    Ok(json!({
        "dry_run": dry_run,
        "path": path,
        "name": name,
        "accel": accel,
        "command": command,
        "requires_yes": requires_yes,
        "batch": if dry_run { Value::Null } else { batch.into() },
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
    use super::{claim_rank, normalize, parse, suggest};

    #[test]
    fn the_extension_named_for_a_schema_owns_it() {
        let ta = "org.gnome.shell.extensions.tiling-assistant";
        // PaperWM only quotes it in its JS.
        assert!(
            claim_rank(ta, "tiling-assistant@ubuntu.com", Some(ta))
                > claim_rank(ta, "paperwm@paperwm.github.com", None)
        );
        assert!(
            claim_rank(ta, "tiling-assistant@ubuntu.com", None) > claim_rank(ta, "paperwm@paperwm.github.com", None)
        );
        let dock = "org.gnome.shell.extensions.dash-to-dock";
        assert_eq!(claim_rank(dock, "ubuntu-dock@ubuntu.com", None), 1);
    }

    fn n(a: &str) -> Option<String> {
        normalize(a).unwrap()
    }

    fn valid(a: &str) -> bool {
        matches!(parse(a, true), Ok(Some(_)))
    }

    #[test]
    fn normalizes_aliases_order_and_case() {
        assert_eq!(n("<Super>t").as_deref(), Some("<Super>t"));
        assert_eq!(n("<Super>T").as_deref(), Some("<Super>t"));
        assert_eq!(n("<Primary><Super>t"), n("<Super><Ctrl>t"));
        assert_eq!(n("<Mod4><Shift>Return").as_deref(), Some("<Shift><Super>Return"));
        assert_eq!(n("<Alt>F4").as_deref(), Some("<Alt>F4"));
        assert_eq!(n("XF86AudioMute").as_deref(), Some("XF86AudioMute"));
        assert_eq!(n("<super>return").as_deref(), Some("<Super>Return"));
        assert_eq!(n("<Release><Mod5><Lock>kp_add").as_deref(), Some("<Mod5><Lock><Release>KP_add"));
    }

    #[test]
    fn empty_and_disabled() {
        assert_eq!(n(""), None);
        assert_eq!(n("  "), None);
        assert_eq!(n("disabled"), None);
    }

    #[test]
    fn rejects_malformed() {
        assert!(normalize("<Super>").is_err());
        assert!(normalize("<Super").is_err());
        assert!(normalize("<Banana>x").is_err());
        assert!(normalize("<Super>a b").is_err());
    }

    #[test]
    fn strict_parsing_knows_key_names() {
        for ok in [
            "<Super>t",
            "<Super>1",
            "<Super>F12",
            "F35",
            "<Control><Alt>Delete",
            "<Super>Page_Down",
            "<Super>Prior",
            "<Super>bracketleft",
            "<Super>comma",
            "<Super>Above_Tab",
            "<Super>KP_2",
            "XF86MonBrightnessUp",
            "<Release><Super>z",
            "<Mod5>space",
            "<Lock>F1",
            "<Hyper><Meta>grave",
            "<Shift>ISO_Left_Tab",
            "<Super>eacute",
            "<Super>dead_acute",
            "<Super>0x1008ff11",
        ] {
            assert!(valid(ok), "{ok}");
        }
        for bad in
            ["<Super>banana", "banana", "Super+T", "<Super>+", "<Super>F0", "<Super>F36", "<Super>Del", "<Super>é"]
        {
            assert!(!valid(bad), "{bad}");
        }
    }

    #[test]
    fn suggests_gtk_forms() {
        assert_eq!(suggest("Super+T").as_deref(), Some("<Super>t"));
        assert_eq!(suggest("Ctrl+Alt+Del").as_deref(), Some("<Control><Alt>Delete"));
        assert_eq!(suggest("super+return").as_deref(), Some("<Super>Return"));
        assert_eq!(suggest("Win+E").as_deref(), Some("<Super>e"));
        assert_eq!(suggest("Ctrl++").as_deref(), Some("<Control>plus"));
        assert_eq!(suggest("ctrl shift esc").as_deref(), Some("<Control><Shift>Escape"));
        assert_eq!(suggest("<Super>Enter").as_deref(), Some("<Super>Return"));
        assert_eq!(suggest("banana"), None);
        assert_eq!(suggest("Super+banana"), None);
    }
}
