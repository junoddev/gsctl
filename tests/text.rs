//! Output for people at a terminal: `--format text`, `GSCTL_FORMAT`, text
//! errors, help, completions and the man page.

mod common;

use common::TestEnv;

const S: &str = "org.gsctl.test";

fn text(env: &TestEnv, args: &[&str]) -> common::Run {
    let mut all = vec!["--format", "text"];
    all.extend_from_slice(args);
    env.gsctl(&all)
}

// --- errors and usage -------------------------------------------------------

#[test]
fn errors_are_plain_lines_in_text_mode() {
    let env = TestEnv::new();
    let r = text(&env, &["get", "org.nope", "x"]);
    assert_eq!(r.status, 3);
    assert!(r.stdout.is_empty());
    assert!(r.stderr.starts_with("error: schema not found: org.nope\n"), "{}", r.stderr);
    assert!(r.stderr.contains("\nhint: try `gsctl search"), "{}", r.stderr);
    assert!(!r.stderr.contains('{'), "{}", r.stderr);

    // JSON stays the default.
    let r = env.gsctl(&["get", "org.nope", "x"]);
    assert_eq!(r.error()["error"]["code"], "schema_not_found");
}

#[test]
fn batch_problems_are_listed_in_text_mode() {
    let env = TestEnv::new();
    let file = env.path("bad.json");
    std::fs::write(
        &file,
        r#"{"gsctl": 1, "area": "other", "settings": {"org.gsctl.test": {"count-i": 500, "nope": 1}}}"#,
    )
    .unwrap();
    let r = text(&env, &["apply", file.to_str().unwrap()]);
    assert_eq!(r.status, 4);
    assert!(r.stderr.starts_with("error: 2 problem"), "{}", r.stderr);
    assert!(r.stderr.contains("count-i: 500 is outside the allowed range 0–100"), "{}", r.stderr);
    assert!(r.stderr.contains("key not found"), "{}", r.stderr);
}

#[test]
fn usage_errors_use_clap_rendering_in_text_mode() {
    let env = TestEnv::new();
    let r = text(&env, &["set", S]);
    assert_eq!(r.status, 2);
    assert!(r.stderr.starts_with("error:"), "{}", r.stderr);
    assert!(r.stderr.contains("Usage:"), "{}", r.stderr);
    assert!(!r.stderr.starts_with('{'), "{}", r.stderr);
    // `--format=text` after the subcommand counts too.
    let r = env.gsctl(&["set", "--format=text"]);
    assert!(r.stderr.contains("Usage:"), "{}", r.stderr);
}

#[test]
fn bare_gsctl_prints_help() {
    let env = TestEnv::new();
    let r = env.gsctl(&[]);
    assert_eq!(r.status, 2);
    let out = format!("{}{}", r.stdout, r.stderr);
    assert!(out.contains("Usage: gsctl"), "{out}");
    assert!(out.contains("search"), "{out}");
    assert!(!out.contains("\\n"), "help must not be a JSON string: {out}");
    let r = env.gsctl(&["keys"]);
    let out = format!("{}{}", r.stdout, r.stderr);
    assert!(out.contains("Usage: gsctl keys"), "{out}");
}

#[test]
fn gsctl_format_env_sets_the_default() {
    let env = TestEnv::new();
    let r = env.session_script(r#"GSCTL_FORMAT=text "$GSCTL" get org.gsctl.test count-i"#);
    assert_eq!(r.stdout, "10\n");
    // The flag wins.
    let r = env.session_script(r#"GSCTL_FORMAT=text "$GSCTL" --format json get org.gsctl.test count-i"#);
    assert_eq!(r.json()["value"], 10);
    let r = env.session_script(r#"GSCTL_FORMAT=yaml "$GSCTL" get org.gsctl.test count-i"#);
    assert_eq!(r.status, 2);
    assert!(r.stderr.contains("GSCTL_FORMAT"), "{}", r.stderr);
}

// --- text renderers ---------------------------------------------------------

#[test]
fn control_characters_are_escaped() {
    let env = TestEnv::new();
    env.gsctl(&["set", S, "name-s", "evil\x1b]0;pwned\x07\x1b[2J"]).ok();
    for args in [&["get", S, "name-s"][..], &["schema", S, "name-s"], &["schema", S], &["changed"], &["history"]] {
        let r = text(&env, args);
        assert_eq!(r.status, 0, "{args:?}: {}", r.stderr);
        assert!(!r.stdout.contains('\x1b') && !r.stdout.contains('\x07'), "{args:?}: {:?}", r.stdout);
        assert!(r.stdout.contains("evil\\u{1b}]0;pwned\\u{7}\\u{1b}[2J"), "{args:?}: {}", r.stdout);
    }
    let r = text(&env, &["get", "org.nope\x1b[31m", "x"]);
    assert!(!r.stderr.contains('\x1b'), "{:?}", r.stderr);
}

#[test]
fn values_are_quoted_and_defaults_described() {
    let env = TestEnv::new();
    let r = text(&env, &["schema", S, "name-s"]);
    assert!(r.stdout.starts_with("org.gsctl.test name-s\n"), "{}", r.stdout);
    assert!(r.stdout.contains("value:    'Adwaita' (the default)"), "{}", r.stdout);
    let r = text(&env, &["schema", S, "count-i"]);
    assert!(r.stdout.contains("allowed:  0–100"), "{}", r.stdout);
    assert!(!r.stdout.contains("..="), "{}", r.stdout);
    let r = text(&env, &["schema", S, "mode"]);
    assert!(r.stdout.contains("one of 'off', 'on', 'auto'"), "{}", r.stdout);

    env.gsctl(&["set", S, "name-s", ""]).ok();
    let r = text(&env, &["get", S, "name-s"]);
    assert_eq!(r.stdout, "''\n");
    let r = text(&env, &["schema", S, "name-s"]);
    assert!(r.stdout.contains("value:    ''\n  default:  'Adwaita'"), "{}", r.stdout);

    // Lists mark changed keys and align columns.
    let r = text(&env, &["schema", S]);
    let line = r.stdout.lines().find(|l| l.contains(" name-s ")).unwrap();
    assert!(line.starts_with("  * name-s"), "{line}");
    assert!(line.contains("= ''") && line.contains("(default 'Adwaita')"), "{line}");
    let unchanged = r.stdout.lines().find(|l| l.contains(" count-i ")).unwrap();
    assert!(unchanged.starts_with("    count-i"), "{unchanged}");
    assert!(!unchanged.contains("default"), "no default shown for unchanged keys: {unchanged}");
    let col = |l: &str| l.find('=').unwrap();
    assert_eq!(col(line), col(unchanged), "values align:\n{}", r.stdout);
    assert!(r.stdout.contains("(* changed from the default)"));
}

#[test]
fn writes_say_what_they_do() {
    let env = TestEnv::new();
    let r = text(&env, &["set", S, "count-i", "12", "--dry-run"]);
    assert_eq!(r.stdout, "would set org.gsctl.test count-i: 10 -> 12\n");
    let r = text(&env, &["set", S, "ratio-d", "1.0"]);
    assert_eq!(r.stdout, "org.gsctl.test ratio-d is already 1.0\n");
    let r = text(&env, &["set", S, "mode", "on"]);
    assert_eq!(r.stdout, "set org.gsctl.test mode: 'auto' -> 'on'\n");
    let r = text(&env, &["reset", S, "mode"]);
    assert_eq!(r.stdout, "reset org.gsctl.test mode: 'on' -> 'auto' (the default)\n");
    let r = text(&env, &["revert", "last"]);
    assert_eq!(r.stdout, "reverted org.gsctl.test mode: 'auto' -> 'on'\n");
}

#[test]
fn guarded_keys_show_in_text_dry_runs() {
    let env = TestEnv::new();
    let ss = "org.gnome.desktop.screensaver";
    let r = text(&env, &["set", ss, "lock-enabled", "false", "--dry-run"]);
    assert!(r.stdout.contains("(guarded: needs --yes"), "{}", r.stdout);
    let r = text(&env, &["reset", ss, "lock-enabled", "--dry-run"]);
    assert!(r.stdout.contains("(guarded: needs --yes"), "{}", r.stdout);
    let r = text(&env, &["schema", ss, "lock-enabled"]);
    assert!(r.stdout.contains("guarded:  needs --yes — changes whether the session locks"), "{}", r.stdout);
    let r = text(&env, &["schema", ss]);
    assert!(r.stdout.contains("(guarded: needs --yes — changes whether the session locks)"), "{}", r.stdout);

    let file = env.path("ss.json");
    let doc = format!(r#"{{"gsctl": 1, "settings": {{"{ss}": {{"lock-enabled": false, "lock-delay": 5}}}}}}"#);
    std::fs::write(&file, doc).unwrap();
    let r = text(&env, &["apply", file.to_str().unwrap(), "--dry-run"]);
    let line = r.stdout.lines().find(|l| l.contains("lock-enabled")).unwrap();
    assert!(line.contains("(guarded: needs --yes — changes whether the session locks)"), "{}", r.stdout);
    assert!(
        r.stdout.contains("dry run: 2 keys would change, 0 already match; 1 guarded key needs --yes"),
        "{}",
        r.stdout
    );
}

#[test]
fn search_text() {
    let env = TestEnv::new();
    let r = text(&env, &["search", "frobnicator"]);
    assert_eq!(r.stdout, "  org.gsctl.test  flag-b  false  # Enable the frobnicator\n1 match\n");
    let r = text(&env, &["search", "zzzzzz"]);
    assert_eq!(r.stdout, "no matches; try fewer or different words\n");
    let r = text(&env, &["search", "accelerator"]);
    assert!(r.stdout.contains("(relocatable)"), "{}", r.stdout);
    assert!(!r.stdout.contains("null"), "{}", r.stdout);
}

#[test]
fn changed_text() {
    let env = TestEnv::new();
    let r = text(&env, &["changed"]);
    assert_eq!(r.stdout, "no keys differ from their defaults\n");
    env.gsctl(&["set", S, "mode", "on"]).ok();
    let r = text(&env, &["changed"]);
    assert_eq!(r.stdout, "org.gsctl.test  mode  = 'on'  (default 'auto')\n1 key changed from the default\n");
}

#[test]
fn history_groups_batches_in_local_time() {
    let env = TestEnv::new();
    env.gsctl(&["set", S, "count-i", "12"]).ok();
    let file = env.path("a.json");
    std::fs::write(&file, r#"{"gsctl": 1, "settings": {"org.gsctl.test": {"mode": "on", "name-s": "X"}}}"#).unwrap();
    env.gsctl(&["apply", file.to_str().unwrap()]).ok();
    env.gsctl(&["revert", "last"]).ok();
    let r = text(&env, &["history"]);
    let lines: Vec<&str> = r.stdout.lines().collect();
    // Newest first: the batch revert, the apply (reverted), then the set.
    assert!(lines[0].contains("batch revert-apply-") && lines[0].ends_with("(2 changes)"), "{}", r.stdout);
    let apply = lines.iter().position(|l| l.contains("batch apply-")).unwrap();
    assert!(lines[apply].ends_with("(2 changes)  [reverted]"), "{}", r.stdout);
    assert!(lines[apply + 1].contains("apply") && lines[apply + 1].contains("[reverted by "), "{}", r.stdout);
    let last = lines.last().unwrap();
    assert!(last.contains("set") && last.contains("org.gsctl.test count-i: 10 -> 12"), "{}", r.stdout);
    // Local time, not the journal's UTC stamp.
    let time = &last[..19];
    assert!(time.as_bytes()[4] == b'-' && time.as_bytes()[10] == b' ' && !time.contains('T'), "{last}");
}

#[test]
fn export_text_summarises_areas() {
    let env = TestEnv::new();
    let r = text(&env, &["export"]);
    assert_eq!(r.stdout, "nothing customised to export\n");
    env.gsctl(&["set", S, "count-i", "42"]).ok();
    env.gsctl(&["set", S, "mode", "on"]).ok();
    env.gsctl(&["set", "org.gnome.desktop.screensaver", "lock-delay", "30"]).ok();
    env.gsctl(&["set", S, "launch-count", "3"]).ok();
    let r = text(&env, &["export"]);
    assert!(r.stdout.starts_with("power:  1 key\nother:  2 keys\n"), "{}", r.stdout);
    assert!(r.stdout.contains("skipped 1 app-state key"), "{}", r.stdout);
    let dir = env.path("out");
    let r = text(&env, &["export", "--out", dir.to_str().unwrap()]);
    assert!(r.stdout.contains("other.json  (2 keys)"), "{}", r.stdout);
    assert!(r.stdout.contains("wrote 2 files"), "{}", r.stdout);
}

#[test]
fn watch_text_lines() {
    let env = TestEnv::new();
    let script = r#"set -e
"$GSCTL" --format text watch --count 1 > "$TMP/watch.out" 2> "$TMP/watch.err" &
pid=$!
for i in $(seq 200); do grep -q 'watching' "$TMP/watch.err" 2>/dev/null && break; sleep 0.05; done
"$GSCTL" set org.gsctl.test mode on > /dev/null
wait $pid
"#;
    let run = env.session_script(script);
    assert_eq!(run.status, 0, "{}", run.stderr);
    let out = std::fs::read_to_string(env.path("watch.out")).unwrap();
    let line = out.lines().next().unwrap_or_default();
    assert!(line.ends_with(" org.gsctl.test mode = 'on'"), "{out:?}");
    assert_eq!(line.as_bytes()[2], b':', "{line}");
}

#[test]
fn doctor_text_has_no_json() {
    let env = TestEnv::new();
    let r = text(&env, &["doctor"]);
    assert!(!r.stdout.contains('{'), "{}", r.stdout);
    assert!(r.stdout.contains("session_mode") && r.stdout.contains("testmode (from env)"), "{}", r.stdout);
    let last = r.stdout.lines().last().unwrap();
    assert!(last.starts_with("ok"), "{}", r.stdout);
}
