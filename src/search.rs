//! Ranked full-text search over schema ids, key names, summaries,
//! descriptions and allowed values (and extension names), plus the two small
//! tables that tune it: schemas that don't apply to this desktop, and a few
//! aliases for words GNOME never uses.

/// The text of one searchable thing (a key, or an extension), as searched.
#[derive(Default)]
pub struct Doc<'a> {
    pub schema: &'a str,
    /// Key name (an extension's name).
    pub key: &'a str,
    pub summary: &'a str,
    pub description: &'a str,
    /// Enum choices or flag names, space-separated (`default prefer-dark`).
    pub choices: &'a str,
}

/// A query split into the words that are scored and the filler words that
/// are dropped ("turn off animations" scores only "animations").
#[derive(Debug, Clone)]
pub struct Query {
    pub words: Vec<String>,
    pub ignored: Vec<String>,
}

/// Words agents add that never name a setting. Dropped unless the whole query
/// is made of them.
const FILLER: &[&str] = &[
    "a", "an", "the", "to", "of", "for", "on", "off", "in", "my", "me", "i", "it", "is", "turn", "set", "change",
    "make", "how", "do", "want", "please", "setting", "settings",
];

impl Query {
    pub fn parse(query: &str) -> Query {
        let all = words(query);
        let (ignored, words): (Vec<String>, Vec<String>) =
            all.iter().cloned().partition(|w| FILLER.contains(&w.as_str()));
        if words.is_empty() { Query { words: all, ignored: vec![] } } else { Query { words, ignored } }
    }
}

/// How well a doc matched a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Query words found, in query order.
    pub matched: Vec<String>,
    /// Rarity-weighted score; only comparable within one query.
    pub score: u32,
}

impl Match {
    pub fn words(&self) -> usize {
        self.matched.len()
    }
}

/// Unweighted score of one word in one doc: key name 30 (20 as a substring),
/// schema id 12 (8), summary and allowed values 10 (6), description 3 (2).
fn word_score(w: &str, d: &Normalized) -> u32 {
    let mut s = 0;
    if d.key.split_whitespace().any(|t| same_word(t, w)) {
        s += 30;
    } else if d.key.contains(w) {
        s += 20;
    }
    if d.schema.split_whitespace().any(|t| same_word(t, w)) {
        s += 12;
    } else if d.schema.contains(w) {
        s += 8;
    }
    let summary_like = |text: &str| {
        if contains_word(text, w) {
            10
        } else if text.contains(w) {
            6
        } else {
            0
        }
    };
    s += summary_like(&d.summary).max(summary_like(&d.choices));
    if contains_word(&d.description, w) {
        s += 3;
    } else if d.description.contains(w) {
        s += 2;
    }
    s
}

struct Normalized {
    schema: String,
    key: String,
    summary: String,
    description: String,
    choices: String,
}

impl Normalized {
    fn of(doc: &Doc) -> Normalized {
        Normalized {
            schema: normalize(doc.schema),
            key: normalize(doc.key),
            summary: normalize(doc.summary),
            description: normalize(doc.description),
            choices: normalize(doc.choices),
        }
    }
}

/// Score every doc and return the matching ones, best first: docs matching
/// every query word, then the rest. Each word is weighted by its rarity
/// across `docs` (inverse document frequency), so in "dark mode" a match on
/// "dark" (a few keys) counts for more than one on "mode" (dozens).
pub fn rank(query: &Query, docs: &[Doc]) -> Vec<(usize, Match)> {
    if query.words.is_empty() {
        return vec![];
    }
    let normalized: Vec<Normalized> = docs.iter().map(Normalized::of).collect();
    let raw: Vec<Vec<u32>> =
        normalized.iter().map(|d| query.words.iter().map(|w| word_score(w, d)).collect()).collect();
    let n = docs.len().max(2) as f64;
    let weights: Vec<u32> = (0..query.words.len())
        .map(|i| {
            let df = raw.iter().filter(|r| r[i] > 0).count().max(1) as f64;
            // 100 for a word found once, down to 10 for one found everywhere.
            ((100.0 * (n / df).ln() / n.ln()).round() as u32).clamp(10, 100)
        })
        .collect();
    let joined = query.words.join(" ");
    let mut hits: Vec<(usize, Match)> = raw
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let matched: Vec<String> =
                query.words.iter().zip(r).filter(|(_, s)| **s > 0).map(|(w, _)| w.clone()).collect();
            if matched.is_empty() {
                return None;
            }
            let mut score: u32 = r.iter().zip(&weights).map(|(s, w)| s * w).sum();
            // The whole query is the key name ("dynamic workspaces" → dynamic-workspaces),
            // or a phrase in it (an extension named "Desktop Icons NG") or in the summary.
            let d = &normalized[i];
            if joined == d.key {
                score += 50 * 100;
            } else if query.words.len() > 1 && has_phrase(&d.key, &joined) {
                score += 40 * 100;
            }
            if query.words.len() > 1 && has_phrase(&d.summary, &joined) {
                score += 15 * 100;
            }
            Some((i, Match { matched, score }))
        })
        .collect();
    hits.sort_by(|(ia, a), (ib, b)| {
        b.words()
            .cmp(&a.words())
            .then(b.score.cmp(&a.score))
            .then(docs[*ia].schema.cmp(docs[*ib].schema))
            .then(docs[*ia].key.cmp(docs[*ib].key))
    });
    hits
}

/// Schemas installed on Ubuntu desktops that nothing in the GNOME session
/// reads. Found on Ubuntu 26.04 with GNOME 50 by checking which
/// binaries and libraries mention them: none, apart from libunity for the
/// Unity lenses. A prefix ending in `.` also matches the schema itself.
const INACTIVE: &[(&str, &str)] = &[
    // gsettings-ubuntu-schemas: Ubuntu Touch / Unity 8 leftovers.
    ("com.ubuntu.touch.", "Ubuntu Touch schema; not used by the GNOME desktop"),
    ("com.ubuntu.phone", "Ubuntu Touch schema; not used by the GNOME desktop"),
    ("com.ubuntu.notifications.", "Ubuntu Touch schema; not used by the GNOME desktop"),
    ("com.ubuntu.sound", "Ubuntu Touch schema; GNOME uses org.gnome.desktop.sound"),
    ("com.ubuntu.user-interface.", "Unity 8 schema; GNOME uses org.gnome.desktop.interface"),
    ("com.canonical.unity.", "Unity desktop schema; not used by GNOME"),
    ("com.canonical.Unity.", "Unity desktop schema; not used by GNOME"),
];

/// Why schema `id` doesn't apply to this desktop, if it doesn't: listed in
/// [`INACTIVE`], or a `.deprecated` schema kept for settings migration.
pub fn inactive_reason(id: &str) -> Option<&'static str> {
    if id.split('.').any(|p| p == "deprecated") {
        return Some("deprecated schema, kept only to migrate old settings");
    }
    INACTIVE
        .iter()
        .find(|(p, _)| match p.strip_suffix('.') {
            Some(base) => id == base || id.starts_with(p),
            None => id == *p,
        })
        .map(|(_, reason)| *reason)
}

/// What an alias points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Key(&'static str, &'static str),
}

/// Everyday words that GNOME's schemas never use for the setting people
/// mean, so neither ranking nor rarity can find it. Each phrase matches when
/// every one of its words starts a query word ("blank" matches "blanking").
/// Alias hits are added at the top, marked `"via": "alias"`; they never hide
/// other results. Keep this tiny: prefer fixes that stay schema-driven.
const ALIASES: &[(&str, Target, &str)] = &[
    // color-scheme only says "prefer-dark" in its choices; dozens of `*-mode` keys and
    // `*-dark` keys outrank it even with rarity weighting (measured: 43rd without this).
    (
        "dark mode",
        Target::Key("org.gnome.desktop.interface", "color-scheme"),
        "prefer-dark or default; on Ubuntu also set gtk-theme Yaru-dark for older apps",
    ),
    (
        "dark theme",
        Target::Key("org.gnome.desktop.interface", "color-scheme"),
        "prefer-dark or default; on Ubuntu also set gtk-theme Yaru-dark for older apps",
    ),
    (
        "dark style",
        Target::Key("org.gnome.desktop.interface", "color-scheme"),
        "prefer-dark or default; on Ubuntu also set gtk-theme Yaru-dark for older apps",
    ),
    // idle-delay says "session is considered idle"; never "screen", "blank" or "timeout".
    (
        "screen timeout",
        Target::Key("org.gnome.desktop.session", "idle-delay"),
        "seconds of inactivity before the screen blanks; 0 = never",
    ),
    (
        "screen blank",
        Target::Key("org.gnome.desktop.session", "idle-delay"),
        "seconds of inactivity before the screen blanks; 0 = never",
    ),
    (
        "screen off",
        Target::Key("org.gnome.desktop.session", "idle-delay"),
        "seconds of inactivity before the screen blanks; 0 = never",
    ),
    // Caps Lock remaps are XKB options (e.g. caps:escape, ctrl:nocaps); the schema only says "XKB options".
    (
        "caps",
        Target::Key("org.gnome.desktop.input-sources", "xkb-options"),
        "remap Caps Lock with XKB options such as caps:escape or ctrl:nocaps",
    ),
    // The schema says "background picture", never "wallpaper".
    (
        "wallpaper",
        Target::Key("org.gnome.desktop.background", "picture-uri"),
        "the desktop background (picture-uri-dark for the dark style)",
    ),
    // Several keys are named lock-screen (notifications, break reminders, lockdown); whether the
    // session locks is screensaver lock-enabled.
    (
        "screen lock",
        Target::Key("org.gnome.desktop.screensaver", "lock-enabled"),
        "whether the screen locks when it blanks; see also lock-delay",
    ),
    (
        "lock screen",
        Target::Key("org.gnome.desktop.screensaver", "lock-enabled"),
        "whether the screen locks when it blanks; see also lock-delay",
    ),
];

/// Aliases whose phrase matches `query`, with their notes, deduplicated.
pub fn aliases(query: &Query) -> Vec<(Target, &'static str)> {
    let mut out: Vec<(Target, &'static str)> = Vec::new();
    let all: Vec<&String> = query.words.iter().chain(&query.ignored).collect();
    for (phrase, target, note) in ALIASES {
        let hit = phrase.split_whitespace().all(|p| all.iter().any(|w| w.starts_with(p)));
        if hit && !out.iter().any(|(t, _)| t == target) {
            out.push((*target, note));
        }
    }
    out
}

fn normalize(s: &str) -> String {
    s.to_lowercase().replace(['-', '_', '.', '/'], " ")
}

fn words(s: &str) -> Vec<String> {
    normalize(s).split_whitespace().map(str::to_string).collect()
}

/// `phrase` (space-separated words) appears in `text` on word boundaries.
fn has_phrase(text: &str, phrase: &str) -> bool {
    let words: Vec<&str> = text.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).collect();
    let want: Vec<&str> = phrase.split(' ').collect();
    words.windows(want.len()).any(|win| win.iter().zip(&want).all(|(a, b)| same_word(a, b)))
}

fn contains_word(text: &str, w: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric()).any(|t| same_word(t, w))
}

/// Equal, or equal apart from a plural "s" ("workspace" ~ "workspaces").
fn same_word(a: &str, b: &str) -> bool {
    a == b || a.strip_suffix('s') == Some(b) || b.strip_suffix('s') == Some(a)
}

/// Levenshtein distance, for "did you mean" suggestions.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Up to `n` of `candidates` close to `input`: within a small edit distance,
/// or containing it (or contained in it) as a whole. Closest first.
pub fn closest<'a>(input: &str, candidates: impl IntoIterator<Item = &'a str>, n: usize) -> Vec<String> {
    let lower = input.to_lowercase();
    let max = (lower.chars().count() / 6).clamp(1, 3);
    let mut scored: Vec<(usize, &str)> = candidates
        .into_iter()
        .filter_map(|c| {
            let cl = c.to_lowercase();
            let d = edit_distance(&lower, &cl);
            let contains = lower.len() >= 3 && (cl.contains(&lower) || lower.contains(&cl));
            (d <= max || contains).then_some((if contains { d.min(max) } else { d }, c))
        })
        .collect();
    scored.sort();
    scored.dedup_by(|a, b| a.1 == b.1);
    scored.into_iter().take(n).map(|(_, c)| c.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc<'a>(schema: &'a str, key: &'a str, summary: &'a str, description: &'a str) -> Doc<'a> {
        Doc { schema, key, summary, description, choices: "" }
    }

    fn q(s: &str) -> Query {
        Query::parse(s)
    }

    /// Rank of each doc (by index) for `query`, best first.
    fn order(query: &str, docs: &[Doc]) -> Vec<usize> {
        rank(&q(query), docs).into_iter().map(|(i, _)| i).collect()
    }

    fn one(query: &str, d: Doc) -> Option<Match> {
        rank(&q(query), &[d]).into_iter().next().map(|(_, m)| m)
    }

    #[test]
    fn reports_matched_words() {
        let d = || doc("org.gnome.mutter", "dynamic-workspaces", "Workspaces are managed dynamically", "");
        assert_eq!(one("dynamic workspaces", d()).unwrap().matched, ["dynamic", "workspaces"]);
        assert_eq!(one("dynamic bananas", d()).unwrap().matched, ["dynamic"]);
        assert!(one("bananas", d()).is_none());
        assert!(one("", d()).is_none());
    }

    #[test]
    fn more_matched_words_always_rank_higher() {
        let docs = [doc("org.a", "dark", "Dark dark dark", ""), doc("org.a", "x", "", "dark theme")];
        assert_eq!(order("dark theme", &docs), [1, 0]);
    }

    #[test]
    fn key_name_beats_summary_beats_description() {
        let docs = [
            doc("org.a", "x", "", "The idle delay in seconds"),
            doc("org.a", "x", "Idle delay", ""),
            doc("org.a", "idle-delay", "", ""),
        ];
        assert_eq!(order("idle delay", &docs), [2, 1, 0]);
    }

    #[test]
    fn exact_key_name_gets_a_bonus() {
        let docs = [doc("org.a", "dynamic-workspaces-extra", "", ""), doc("org.a", "dynamic-workspaces", "", "")];
        assert_eq!(order("dynamic-workspaces", &docs), [1, 0]);
    }

    #[test]
    fn substring_matches_count_less_than_whole_words() {
        let docs = [doc("org.a", "subworkspace", "", ""), doc("org.a", "workspace", "", "")];
        assert_eq!(order("workspace", &docs), [1, 0]);
    }

    #[test]
    fn plurals_match_whole_words() {
        let docs =
            [doc("org.a", "subworkspace", "", ""), doc("org.gnome.mutter", "dynamic-workspaces", "Workspaces", "")];
        assert_eq!(order("workspace", &docs), [1, 0]);
        assert_eq!(order("dynamic workspace", &docs)[0], 1);
    }

    #[test]
    fn case_insensitive_and_separator_agnostic() {
        assert!(one("COLOR_SCHEME", doc("org.gnome.desktop.interface", "color-scheme", "Color scheme", "")).is_some());
        assert!(one("desktop interface", doc("org.gnome.desktop.interface", "color-scheme", "", "")).is_some());
    }

    #[test]
    fn allowed_values_are_searched_like_the_summary() {
        let d = Doc {
            schema: "org.a",
            key: "color-scheme",
            choices: "default prefer-dark prefer-light",
            ..Default::default()
        };
        assert_eq!(one("dark", d).unwrap().matched, ["dark"]);
    }

    #[test]
    fn rare_words_outweigh_common_ones() {
        // "mode" is in many key names, "dark" in one; without weighting they would tie
        // and the alphabetically first schema would win.
        let mut docs: Vec<Doc> = ["flight-mode", "silent-mode", "dwell-mode", "edge-mode", "share-mode"]
            .iter()
            .map(|k| doc("org.a", k, "", ""))
            .collect();
        docs.push(doc("org.z", "dark-x", "", ""));
        for i in 0..20 {
            docs.push(doc("org.c", ["x", "y", "z", "w"][i % 4], "", ""));
        }
        assert_eq!(order("dark mode", &docs)[0], 5);
    }

    #[test]
    fn filler_words_are_dropped() {
        let query = q("turn off animations");
        assert_eq!(query.words, ["animations"]);
        assert_eq!(query.ignored, ["turn", "off"]);
        assert_eq!(q("off").words, ["off"]);
    }

    #[test]
    fn inactive_schemas() {
        assert!(inactive_reason("com.ubuntu.touch.system").is_some());
        assert!(inactive_reason("com.canonical.unity.desktop.interface").is_some());
        assert!(inactive_reason("com.ubuntu.user-interface").is_some());
        assert!(inactive_reason("com.ubuntu.user-interface.desktop").is_some());
        assert!(inactive_reason("org.gnome.settings-daemon.peripherals.touchpad.deprecated").is_some());
        assert!(inactive_reason("org.gnome.desktop.interface").is_none());
        assert!(inactive_reason("com.ubuntu.login-screen").is_none());
        assert!(inactive_reason("com.ubuntu.phonebook").is_none());
    }

    #[test]
    fn aliases_match_word_prefixes() {
        let idle = Target::Key("org.gnome.desktop.session", "idle-delay");
        assert_eq!(aliases(&q("screen blanking")).first().map(|a| a.0), Some(idle));
        assert_eq!(aliases(&q("screen timeout")).len(), 1);
        assert_eq!(aliases(&q("turn the screen off")).first().map(|a| a.0), Some(idle));
        assert!(aliases(&q("screen")).is_empty());
        assert!(aliases(&q("dark")).is_empty());
        assert!(aliases(&q("font size")).is_empty());
    }

    #[test]
    fn suggestions() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        let keys = ["color-scheme", "cursor-size", "font-name"];
        assert_eq!(closest("colour-scheme", keys, 3), ["color-scheme"]);
        assert_eq!(closest("scheme", keys, 3), ["color-scheme"]);
        assert!(closest("zzzzzz", keys, 3).is_empty());
    }
}
