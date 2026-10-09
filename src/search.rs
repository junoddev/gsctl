//! Ranked full-text search over schema ids, key names, summaries and
//! descriptions.

/// The text of one key, as searched.
pub struct Doc<'a> {
    pub schema: &'a str,
    pub key: &'a str,
    pub summary: &'a str,
    pub description: &'a str,
}

/// How well a key matched a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Match {
    /// Number of query words found. Compared first, so keys matching more
    /// words always rank higher.
    pub words: usize,
    pub score: u32,
}

/// Number of words in `query`, as `score` counts them.
pub fn word_count(query: &str) -> usize {
    words(query).len()
}

/// Score `doc` against `query`, or `None` if no query word appears.
///
/// Matches in the key name count most, then the schema id, the summary, and
/// the description. Words are compared
/// case-insensitively, a trailing plural "s" is ignored, and `-`, `_` and `.`
/// count as spaces, so "dynamic workspace" finds `dynamic-workspaces`.
pub fn score(query: &str, doc: &Doc) -> Option<Match> {
    let words = words(query);
    if words.is_empty() {
        return None;
    }
    let key = normalize(doc.key);
    let schema = normalize(doc.schema);
    let summary = normalize(doc.summary);
    let description = normalize(doc.description);
    let key_words = words_of(&key);
    let schema_words = words_of(&schema);

    let mut total = 0;
    let mut matched = 0;
    for w in &words {
        let mut s = 0;
        if key_words.iter().any(|t| same_word(t, w)) {
            s += 30;
        } else if key.contains(w.as_str()) {
            s += 20;
        }
        if schema_words.iter().any(|t| same_word(t, w)) {
            s += 12;
        } else if schema.contains(w.as_str()) {
            s += 8;
        }
        if contains_word(&summary, w) {
            s += 10;
        } else if summary.contains(w.as_str()) {
            s += 6;
        }
        if contains_word(&description, w) {
            s += 3;
        } else if description.contains(w.as_str()) {
            s += 2;
        }
        if s > 0 {
            matched += 1;
            total += s;
        }
    }
    // The whole query is the key name ("dynamic workspaces" → dynamic-workspaces).
    if words.join(" ") == key {
        total += 50;
    }
    (matched > 0).then_some(Match { words: matched, score: total })
}

fn normalize(s: &str) -> String {
    s.to_lowercase().replace(['-', '_', '.', '/'], " ")
}

fn words(s: &str) -> Vec<String> {
    normalize(s).split_whitespace().map(str::to_string).collect()
}

fn words_of(s: &str) -> Vec<&str> {
    s.split_whitespace().collect()
}

fn contains_word(text: &str, w: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric()).any(|t| same_word(t, w))
}

/// Equal, or equal apart from a plural "s" ("workspace" ~ "workspaces").
fn same_word(a: &str, b: &str) -> bool {
    a == b || a.strip_suffix('s') == Some(b) || b.strip_suffix('s') == Some(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc<'a>(schema: &'a str, key: &'a str, summary: &'a str, description: &'a str) -> Doc<'a> {
        Doc { schema, key, summary, description }
    }

    fn sc(query: &str, d: &Doc) -> u32 {
        score(query, d).unwrap().score
    }

    #[test]
    fn counts_matched_words() {
        let d = doc("org.gnome.mutter", "dynamic-workspaces", "Workspaces are managed dynamically", "");
        assert_eq!(score("dynamic workspaces", &d).unwrap().words, 2);
        assert_eq!(score("dynamic bananas", &d).unwrap().words, 1);
        assert!(score("bananas", &d).is_none());
        assert!(score("", &d).is_none());
    }

    #[test]
    fn more_matched_words_always_rank_higher() {
        let both = doc("org.a", "x", "", "dark theme");
        let one = doc("org.a", "dark", "Dark dark dark", "");
        assert!(score("dark theme", &both).unwrap() > score("dark theme", &one).unwrap());
    }

    #[test]
    fn key_name_beats_summary_beats_description() {
        let by_key = doc("org.a", "idle-delay", "", "");
        let by_summary = doc("org.a", "x", "Idle delay", "");
        let by_desc = doc("org.a", "x", "", "The idle delay in seconds");
        let k = sc("idle delay", &by_key);
        let s = sc("idle delay", &by_summary);
        let d = sc("idle delay", &by_desc);
        assert!(k > s && s > d, "{k} {s} {d}");
    }

    #[test]
    fn exact_key_name_gets_a_bonus() {
        let exact = doc("org.a", "dynamic-workspaces", "", "");
        let partial = doc("org.a", "dynamic-workspaces-extra", "", "");
        assert!(sc("dynamic-workspaces", &exact) > sc("dynamic-workspaces", &partial));
    }

    #[test]
    fn substring_matches_count_less_than_whole_words() {
        let whole = doc("org.a", "workspace", "", "");
        let sub = doc("org.a", "subworkspace", "", "");
        assert!(sc("workspace", &whole) > sc("workspace", &sub));
    }

    #[test]
    fn plurals_match_whole_words() {
        let d = doc("org.gnome.mutter", "dynamic-workspaces", "Workspaces are managed dynamically", "");
        let plural = sc("workspace", &d);
        let sub = sc("workspace", &doc("org.a", "subworkspace", "", ""));
        assert!(plural > sub, "{plural} {sub}");
        assert!(score("dynamic workspace", &d).is_some());
    }

    #[test]
    fn case_insensitive_and_separator_agnostic() {
        let d = doc("org.gnome.desktop.interface", "color-scheme", "Color scheme", "");
        assert!(score("COLOR_SCHEME", &d).is_some());
        assert!(score("desktop interface", &d).is_some());
    }
}
