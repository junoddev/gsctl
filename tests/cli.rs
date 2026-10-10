//! End-to-end tests of the gsctl commands against the hermetic test schemas.

mod common;

use common::{EXTENSION_UUID, TestEnv};
use serde_json::{Value, json};

const S: &str = "org.gsctl.test";

fn journal(env: &TestEnv) -> Vec<Value> {
    match std::fs::read_to_string(env.journal()) {
        Ok(text) => text.lines().map(|l| serde_json::from_str(l).unwrap()).collect(),
        Err(_) => vec![],
    }
}

// --- search -----------------------------------------------------------------

#[test]
fn search_ranks_and_reports_values() {
    let env = TestEnv::new();
    let out = env.gsctl(&["search", "frobnicator"]).ok();
    assert_eq!(out["total"], 1);
    let hit = &out["results"][0];
    assert_eq!(hit["schema"], S);
    assert_eq!(hit["key"], "flag-b");
    assert_eq!(hit["type"], "b");
    assert_eq!(hit["value"], false);
    assert_eq!(hit["origin"]["type"], "system");

    // Key-name matches outrank summary/description matches.
    let out = env.gsctl(&["search", "idle", "delay"]).ok();
    assert_eq!(out["results"][0]["key"], "idle-delay");
    assert_eq!(out["results"][1]["key"], "size-u");
    let out = env.gsctl(&["search", "lock", "screen"]).ok();
    assert_eq!(out["results"][0]["key"], "lock-enabled");
}

#[test]
fn search_covers_extension_schemas() {
    let env = TestEnv::new();
    let out = env.gsctl(&["search", "pelican"]).ok();
    let hit = &out["results"][0];
    assert_eq!(hit["schema"], "org.gnome.shell.extensions.gsctl-demo");
    assert_eq!(hit["key"], "show-indicator");
    assert_eq!(hit["origin"]["type"], "extension");
    assert_eq!(hit["origin"]["uuid"], EXTENSION_UUID);
}

#[test]
fn search_limit_and_schema_filter() {
    let env = TestEnv::new();
    let all = env.gsctl(&["search", "s"]).ok();
    assert!(all["total"].as_u64().unwrap() > 3);
    let limited = env.gsctl(&["search", "s", "--limit", "2"]).ok();
    assert_eq!(limited["results"].as_array().unwrap().len(), 2);
    assert_eq!(limited["total"], all["total"]);
    let filtered = env.gsctl(&["search", "lock", "--schema", "org.gnome.desktop"]).ok();
    for r in filtered["results"].as_array().unwrap() {
        assert_eq!(r["schema"], "org.gnome.desktop.screensaver");
    }
    let none = env.gsctl(&["search", "zzzzzz"]).ok();
    assert_eq!(none["total"], 0);
}

#[test]
fn search_falls_back_to_partial_matches() {
    let env = TestEnv::new();
    let full = env.gsctl(&["search", "frobnicator"]).ok();
    assert_eq!(full["partial"], false);

    // No key mentions "turbo", so the best partial matches come back, flagged.
    let out = env.gsctl(&["search", "frobnicator", "turbo"]).ok();
    assert_eq!(out["partial"], true);
    assert_eq!(out["results"][0]["key"], "flag-b");
    assert_eq!(out["results"][0]["matched_words"], 1);
}

// --- schema -----------------------------------------------------------------

#[test]
fn schema_describes_a_key() {
    let env = TestEnv::new();
    let k = env.gsctl(&["schema", S, "count-i"]).ok();
    assert_eq!(k["type"], "i");
    assert_eq!(k["type_description"], "32-bit integer");
    assert_eq!(k["default"], 10);
    assert_eq!(k["value"], 10);
    assert_eq!(k["is_default"], true);
    assert_eq!(k["writable"], true);
    assert_eq!(k["range"], json!({"kind": "range", "min": 0, "max": 100}));
    assert_eq!(k["summary"], "Number of widgets");
    assert_eq!(k["guarded"], Value::Null);

    let e = env.gsctl(&["schema", S, "mode"]).ok();
    assert_eq!(e["type"], "s");
    assert_eq!(e["range"], json!({"kind": "enum", "choices": ["off", "on", "auto"]}));
    let c = env.gsctl(&["schema", S, "choice-s"]).ok();
    assert_eq!(c["range"]["choices"], json!(["left", "right"]));
    let f = env.gsctl(&["schema", S, "opts"]).ok();
    assert_eq!(f["type"], "as");
    assert_eq!(f["range"], json!({"kind": "flags", "flags": ["bold", "italic", "underline"]}));
    let g = env.gsctl(&["schema", "org.gnome.desktop.screensaver", "lock-enabled"]).ok();
    assert!(g["guarded"].is_string());
}

#[test]
fn schema_lists_keys_and_schemas() {
    let env = TestEnv::new();
    let sch = env.gsctl(&["schema", S]).ok();
    assert_eq!(sch["path"], "/org/gsctl/test/");
    assert_eq!(sch["relocatable"], false);
    let names: Vec<&str> = sch["keys"].as_array().unwrap().iter().map(|k| k["key"].as_str().unwrap()).collect();
    assert!(names.contains(&"flag-b") && names.contains(&"dict-asv"));

    let list = env.gsctl(&["schema"]).ok();
    let reloc = list["schemas"].as_array().unwrap().iter().find(|s| s["id"] == "org.gsctl.test.reloc").unwrap();
    assert_eq!(reloc["relocatable"], true);
}

#[test]
fn relocatable_schemas_need_a_path() {
    let env = TestEnv::new();
    let k = env.gsctl(&["schema", "org.gsctl.test.reloc", "binding"]).ok();
    assert!(k.get("value").is_none(), "no instance, so no value");

    let r = env.gsctl(&["get", "org.gsctl.test.reloc", "binding"]);
    assert_eq!(r.status, 2);
    assert_eq!(r.error()["error"]["code"], "usage");

    let p = "/org/gsctl/test/custom0/";
    env.gsctl(&["set", "org.gsctl.test.reloc", "binding", "<Super>t", "--path", p]).ok();
    let got = env.gsctl(&["get", "org.gsctl.test.reloc", "binding", "--path", p]).ok();
    assert_eq!(got["value"], "<Super>t");
    let k = env.gsctl(&["schema", "org.gsctl.test.reloc", "binding", "--path", p]).ok();
    assert_eq!(k["value"], "<Super>t");

    let bad = env.gsctl(&["get", S, "flag-b", "--path", p]);
    assert_eq!(bad.status, 2);
}

// --- get / set --------------------------------------------------------------

#[test]
fn not_found_errors() {
    let env = TestEnv::new();
    let r = env.gsctl(&["get", "org.nope", "x"]);
    assert_eq!(r.status, 3);
    assert_eq!(r.error()["error"]["code"], "schema_not_found");
    assert!(r.stdout.is_empty());
    let r = env.gsctl(&["get", S, "nope"]);
    assert_eq!(r.status, 3);
    assert_eq!(r.error()["error"]["code"], "key_not_found");
}

#[test]
fn set_and_get_every_type() {
    let env = TestEnv::new();
    let cases: &[(&str, &str, Value)] = &[
        ("flag-b", "true", json!(true)),
        ("count-i", "42", json!(42)),
        ("size-u", "600", json!(600)),
        ("ratio-d", "1.25", json!(1.25)),
        ("name-s", "Yaru-dark", json!("Yaru-dark")),
        ("list-as", r#"["a","b"]"#, json!(["a", "b"])),
        ("list-as", "@as []", json!([])),
        ("dict-asv", r#"{"x": 1, "y": "z", "on": true}"#, json!({"x": 1, "y": "z", "on": true})),
        ("dict-ass", r#"{"k": "v"}"#, json!({"k": "v"})),
        ("pair-ii", "[3, -4]", json!([3, -4])),
        ("triple-sdb", "('q', 2.5, false)", json!(["q", 2.5, false])),
        ("sources-ass", r#"[["xkb","de"],["ibus","mozc"]]"#, json!([["xkb", "de"], ["ibus", "mozc"]])),
        ("mode", "on", json!("on")),
        ("opts", r#"["bold","italic"]"#, json!(["bold", "italic"])),
        ("choice-s", "right", json!("right")),
        ("maybe-s", "hello", json!("hello")),
        ("maybe-s", "null", json!(null)),
        ("bytes-ay", "[104, 105, 0]", json!([104, 105, 0])),
        ("big-x", "-5000000000", json!(-5000000000i64)),
        ("big-t", "18446744073709551615", json!(u64::MAX)),
        ("byte-y", "7", json!(7)),
        ("short-n", "-7", json!(-7)),
        ("short-q", "7", json!(7)),
    ];
    for (key, input, expected) in cases {
        let out = env.gsctl(&["set", S, key, input]).ok();
        assert_eq!(&out["value"], expected, "set {key} {input}");
        let got = env.gsctl(&["get", S, key]).ok();
        assert_eq!(&got["value"], expected, "get {key} after set {input}");
    }
}

#[test]
fn set_rejects_bad_values_with_actionable_errors() {
    let env = TestEnv::new();
    let bad = |key: &str, input: &str| {
        let r = env.gsctl(&["set", S, key, input]);
        assert_eq!(r.status, 4, "set {key} {input}: {}", r.stderr);
        r.error()["error"].clone()
    };

    let e = bad("flag-b", "maybe");
    assert_eq!(e["code"], "invalid_value");
    assert_eq!(e["type"], "b");
    assert_eq!(e["input"], "maybe");

    let e = bad("count-i", "101");
    assert!(e["message"].as_str().unwrap().contains("outside the allowed range 0..=100"), "{e}");
    assert_eq!(e["range"]["max"], 100);

    let e = bad("mode", "sideways");
    assert!(e["message"].as_str().unwrap().contains("off, on, auto"), "{e}");

    let e = bad("opts", r#"["bold","blink"]"#);
    assert_eq!(e["range"]["kind"], "flags");

    let e = bad("list-as", r#"["a", 1]"#);
    assert_eq!(e["message"], "item 1: expected string, got integer");

    bad("size-u", "-1");
    bad("pair-ii", "[1]");

    // Nothing was written.
    assert_eq!(env.gsctl(&["get", S, "count-i"]).ok()["value"], 10);
    assert!(journal(&env).is_empty());
}

#[test]
fn dry_run_validates_without_writing() {
    let env = TestEnv::new();
    let out = env.gsctl(&["set", S, "count-i", "50", "--dry-run"]).ok();
    assert_eq!(out["dry_run"], true);
    assert_eq!(out["old"], 10);
    assert_eq!(out["new"], 50);
    assert_eq!(out["changed"], true);
    assert_eq!(out["requires_yes"], false);
    assert_eq!(env.gsctl(&["get", S, "count-i"]).ok()["value"], 10);
    assert!(journal(&env).is_empty());

    assert_eq!(env.gsctl(&["set", S, "count-i", "500", "--dry-run"]).status, 4);
}

#[test]
fn set_and_reset_are_journaled() {
    let env = TestEnv::new();
    let out = env.gsctl(&["set", S, "list-as", r#"["x"]"#]).ok();
    assert_eq!(out["old"], json!(["one", "two"]));
    assert!(out["journal_id"].is_string());

    let out = env.gsctl(&["reset", S, "list-as"]).ok();
    assert_eq!(out["old"], json!(["x"]));
    assert_eq!(out["value"], json!(["one", "two"]));
    let got = env.gsctl(&["get", S, "list-as"]).ok();
    assert_eq!(got["is_default"], true);

    let entries = journal(&env);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["op"], "set");
    assert_eq!(entries[0]["schema"], S);
    assert_eq!(entries[0]["key"], "list-as");
    assert_eq!(entries[0]["old"], json!(["one", "two"]));
    assert_eq!(entries[0]["old_user_value"], Value::Null);
    assert_eq!(entries[0]["new"], json!(["x"]));
    assert_eq!(entries[0]["new_gvariant"], "['x']");
    assert_eq!(entries[1]["op"], "reset");
    assert_eq!(entries[1]["old_user_value"], "['x']");
    assert_eq!(entries[1]["id"], out["journal_id"]);
    assert!(entries[1]["time"].as_str().unwrap().ends_with('Z'));
}

#[test]
fn reset_dry_run() {
    let env = TestEnv::new();
    env.gsctl(&["set", S, "name-s", "X"]).ok();
    let out = env.gsctl(&["reset", S, "name-s", "--dry-run"]).ok();
    assert_eq!(out["old"], "X");
    assert_eq!(out["new"], "Adwaita");
    assert_eq!(env.gsctl(&["get", S, "name-s"]).ok()["value"], "X");
}

#[test]
fn guarded_keys_need_yes() {
    let env = TestEnv::new();
    let screensaver = "org.gnome.desktop.screensaver";

    let r = env.gsctl(&["set", screensaver, "lock-enabled", "false"]);
    assert_eq!(r.status, 6);
    assert_eq!(r.error()["error"]["code"], "guarded");
    assert_eq!(env.gsctl(&["get", screensaver, "lock-enabled"]).ok()["value"], true);

    let dry = env.gsctl(&["set", screensaver, "lock-enabled", "false", "--dry-run"]).ok();
    assert_eq!(dry["requires_yes"], true);

    env.gsctl(&["set", screensaver, "lock-enabled", "false", "--yes"]).ok();
    assert_eq!(env.gsctl(&["get", screensaver, "lock-enabled"]).ok()["value"], false);

    assert_eq!(env.gsctl(&["reset", screensaver, "lock-enabled"]).status, 6);
    env.gsctl(&["reset", screensaver, "lock-enabled", "--yes"]).ok();

    // Other keys in the same schema are not guarded.
    env.gsctl(&["set", screensaver, "lock-delay", "30"]).ok();
}

#[test]
fn locked_keys_are_not_writable() {
    let env = TestEnv::new();
    env.lock_key();
    let k = env.gsctl(&["schema", S, "locked-s"]).ok();
    assert_eq!(k["writable"], false);
    assert_eq!(k["value"], "admin");
    let r = env.gsctl(&["set", S, "locked-s", "mine"]);
    assert_eq!(r.status, 5, "{}", r.stderr);
    assert_eq!(r.error()["error"]["code"], "not_writable");
}

#[test]
fn extension_keys_are_writable() {
    let env = TestEnv::new();
    let ext = "org.gnome.shell.extensions.gsctl-demo";
    env.gsctl(&["set", ext, "show-indicator", "false"]).ok();
    assert_eq!(env.gsctl(&["get", ext, "show-indicator"]).ok()["value"], false);
}

/// The JSON error line of a run without a bus (GLib may log to stderr too).
fn no_bus_error(r: &common::Run) -> Value {
    assert_eq!(r.status, 5, "stdout:\n{}\nstderr:\n{}", r.stdout, r.stderr);
    let err = r.stderr.lines().find(|l| l.starts_with('{')).expect("JSON error line");
    let err: Value = serde_json::from_str(err).unwrap();
    assert_eq!(err["error"]["code"], "no_session", "{err}");
    err
}

#[test]
fn writes_without_a_session_bus_are_refused() {
    let env = TestEnv::new();
    let err = no_bus_error(&env.gsctl_without_bus(&["set", S, "name-s", "Lost"]));
    assert!(err["error"]["message"].as_str().unwrap().contains("session bus"), "{err}");
    // Setting the value the sandbox already reads (the default) used to
    // "succeed" with changed: false, since read-back can't catch it.
    no_bus_error(&env.gsctl_without_bus(&["set", S, "name-s", "Adwaita"]));
    no_bus_error(&env.gsctl_without_bus(&["reset", S, "name-s"]));
    no_bus_error(&env.gsctl_without_bus(&["keys", "add", "T", "<Super><Alt>F12", "true"]));
    no_bus_error(&env.gsctl_without_bus(&["ext", "disable", common::EXTENSION_UUID]));

    // An apply with nothing to change is refused too: "unchanged" was
    // judged against defaults.
    let file = env.path("state.json");
    std::fs::write(&file, format!(r#"{{"gsctl": 1, "settings": {{"{S}": {{"name-s": "Adwaita"}}}}}}"#)).unwrap();
    no_bus_error(&env.gsctl_without_bus(&["apply", file.to_str().unwrap()]));
    assert!(journal(&env).is_empty());
    assert!(!env.user_db().exists());

    // Dry runs still work.
    let r = env.gsctl_without_bus(&["set", S, "name-s", "Lost", "--dry-run"]);
    assert_eq!(r.status, 0, "{}", r.stderr);
    assert_eq!(r.json()["dry_run"], true);
}

// --- changed ----------------------------------------------------------------

#[test]
fn changed_lists_only_customised_keys() {
    let env = TestEnv::new();
    assert_eq!(env.gsctl(&["changed"]).ok()["count"], 0);

    env.gsctl(&["set", S, "count-i", "11"]).ok();
    env.gsctl(&["set", S, "name-s", "Adwaita"]).ok(); // same as default
    env.gsctl(&["set", "org.gnome.desktop.screensaver", "lock-delay", "5"]).ok();

    let out = env.gsctl(&["changed"]).ok();
    let items = out["changed"].as_array().unwrap();
    assert_eq!(out["count"], 2, "{out}");
    assert!(items.iter().any(|c| c["key"] == "count-i" && c["value"] == 11 && c["default"] == 10));
    assert!(items.iter().any(|c| c["key"] == "lock-delay"));

    let only = env.gsctl(&["changed", "--schema", "org.gnome.desktop"]).ok();
    assert_eq!(only["count"], 1);
    assert_eq!(only["changed"][0]["schema"], "org.gnome.desktop.screensaver");
}

#[test]
fn changed_includes_listed_relocatable_instances() {
    let env = TestEnv::new();
    let custom = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
    let path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/";
    env.gsctl(&["keys", "add", "Terminal", "<Super>Return", "ptyxis"]).ok();
    // Unlisted instances can't be found, so they're left out.
    env.gsctl(&["set", "org.gnome.Ptyxis.Profile", "label", "Stale", "--path", "/org/gnome/Ptyxis/Profiles/x/"]).ok();

    let out = env.gsctl(&["changed", "--schema", custom]).ok();
    assert_eq!(out["count"], 3, "{out}");
    assert!(out["changed"].as_array().unwrap().iter().all(|c| c["path"] == path));
    assert_eq!(env.gsctl(&["changed", "--schema", "org.gnome.Ptyxis"]).ok()["count"], 0);
}

// --- conventions ------------------------------------------------------------

#[test]
fn usage_errors_are_json_with_exit_2() {
    let env = TestEnv::new();
    let r = env.gsctl(&["set", S]);
    assert_eq!(r.status, 2);
    assert_eq!(r.error()["error"]["code"], "usage");
    let r = env.gsctl(&["frobnicate"]);
    assert_eq!(r.status, 2);
    let help = env.gsctl(&["set", "--help"]);
    assert_eq!(help.status, 0);
    assert!(help.stdout.contains("--dry-run"));
}

#[test]
fn negative_numbers_are_values_not_flags() {
    let env = TestEnv::new();
    env.gsctl(&["set", S, "short-n", "-3"]).ok();
    assert_eq!(env.gsctl(&["get", S, "short-n"]).ok()["value"], -3);
}

#[test]
fn text_format() {
    let env = TestEnv::new();
    let r = env.gsctl(&["--format", "text", "get", S, "list-as"]);
    assert_eq!(r.stdout, "[\"one\",\"two\"]\n");
    let r = env.gsctl(&["set", S, "count-i", "12", "--format", "text"]);
    assert_eq!(r.stdout, "org.gsctl.test count-i changed: 10 -> 12\n");
}

// --- history / revert -------------------------------------------------------

fn id_of(out: &Value) -> String {
    out["journal_id"].as_str().expect("journal_id").to_string()
}

#[test]
fn history_lists_newest_first_with_filters() {
    let env = TestEnv::new();
    assert_eq!(env.gsctl(&["history"]).ok()["total"], 0);

    env.gsctl(&["set", S, "count-i", "20"]).ok();
    env.gsctl(&["set", S, "name-s", "A"]).ok();
    env.gsctl(&["set", "org.gnome.desktop.screensaver", "lock-delay", "9"]).ok();
    env.gsctl(&["reset", S, "count-i"]).ok();

    let h = env.gsctl(&["history"]).ok();
    assert_eq!(h["total"], 4);
    let ops: Vec<(&str, &str)> = h["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["op"].as_str().unwrap(), e["key"].as_str().unwrap()))
        .collect();
    assert_eq!(ops, [("reset", "count-i"), ("set", "lock-delay"), ("set", "name-s"), ("set", "count-i")]);

    assert_eq!(env.gsctl(&["history", "--limit", "1"]).ok()["entries"].as_array().unwrap().len(), 1);
    assert_eq!(env.gsctl(&["history", "--key", "count-i"]).ok()["total"], 2);
    assert_eq!(env.gsctl(&["history", "--schema", "org.gnome.desktop"]).ok()["total"], 1);
}

#[test]
fn revert_a_set_from_default_resets_the_key() {
    let env = TestEnv::new();
    let set = env.gsctl(&["set", S, "size-u", "600"]).ok();
    let out = env.gsctl(&["revert", "last"]).ok();
    assert_eq!(out["op"], "revert");
    assert_eq!(out["reverts"], id_of(&set));
    assert_eq!(out["value"], 300);
    let got = env.gsctl(&["get", S, "size-u"]).ok();
    assert_eq!(got["is_default"], true, "reverting a change from the default must reset, not pin the default");

    let h = env.gsctl(&["history"]).ok();
    assert_eq!(h["entries"][0]["reverts"], id_of(&set));
    assert_eq!(h["entries"][1]["reverted_by"], out["journal_id"]);
}

#[test]
fn revert_restores_previous_user_values() {
    let env = TestEnv::new();
    env.gsctl(&["set", S, "dict-asv", r#"{"a": 1}"#]).ok();
    let second = env.gsctl(&["set", S, "dict-asv", r#"{"b": true}"#]).ok();
    env.gsctl(&["revert", &id_of(&second)]).ok();
    let got = env.gsctl(&["get", S, "dict-asv"]).ok();
    assert_eq!(got["value"], json!({"a": 1}));
    assert_eq!(got["is_default"], false);

    // Reverting a reset brings back the value it cleared.
    env.gsctl(&["reset", S, "dict-asv"]).ok();
    env.gsctl(&["revert", "last"]).ok();
    assert_eq!(env.gsctl(&["get", S, "dict-asv"]).ok()["value"], json!({"a": 1}));
}

#[test]
fn revert_refuses_keys_changed_since_unless_forced() {
    let env = TestEnv::new();
    let first = env.gsctl(&["set", S, "name-s", "First"]).ok();
    env.gsctl(&["set", S, "name-s", "Second"]).ok();

    let r = env.gsctl(&["revert", &id_of(&first)]);
    assert_eq!(r.status, 7, "{}", r.stderr);
    assert_eq!(r.error()["error"]["code"], "conflict");
    assert_eq!(env.gsctl(&["get", S, "name-s"]).ok()["value"], "Second");

    env.gsctl(&["revert", &id_of(&first), "--force"]).ok();
    let got = env.gsctl(&["get", S, "name-s"]).ok();
    assert_eq!(got["value"], "Adwaita");
    assert_eq!(got["is_default"], true);
}

#[test]
fn revert_dry_run_unknown_ids_and_guards() {
    let env = TestEnv::new();
    let r = env.gsctl(&["revert", "last"]);
    assert_eq!(r.status, 3);
    assert_eq!(r.error()["error"]["code"], "entry_not_found");
    assert_eq!(env.gsctl(&["revert", "nope"]).status, 3);

    env.gsctl(&["set", S, "count-i", "33"]).ok();
    let dry = env.gsctl(&["revert", "last", "--dry-run"]).ok();
    assert_eq!(dry["old"], 33);
    assert_eq!(dry["new"], 10);
    assert_eq!(env.gsctl(&["get", S, "count-i"]).ok()["value"], 33);

    let screensaver = "org.gnome.desktop.screensaver";
    env.gsctl(&["set", screensaver, "lock-enabled", "false", "--yes"]).ok();
    assert_eq!(env.gsctl(&["revert", "last"]).status, 6);
    env.gsctl(&["revert", "last", "--yes"]).ok();
    assert_eq!(env.gsctl(&["get", screensaver, "lock-enabled"]).ok()["value"], true);
}

#[test]
fn revert_last_walks_back_like_an_undo_stack() {
    let env = TestEnv::new();
    env.gsctl(&["set", S, "count-i", "1"]).ok();
    env.gsctl(&["set", S, "count-i", "2"]).ok();
    env.gsctl(&["revert", "last"]).ok();
    assert_eq!(env.gsctl(&["get", S, "count-i"]).ok()["value"], 1);
    // The second undo reverts the first set, not the revert just made.
    let undo = env.gsctl(&["revert", "last"]).ok();
    let got = env.gsctl(&["get", S, "count-i"]).ok();
    assert_eq!(got["is_default"], true, "{got}");
    assert_eq!(env.gsctl(&["revert", "last"]).status, 3, "nothing left to undo");

    // Redo is reverting the revert by id.
    env.gsctl(&["revert", &id_of(&undo)]).ok();
    assert_eq!(env.gsctl(&["get", S, "count-i"]).ok()["value"], 1);
}

#[test]
fn revert_relocatable_keys() {
    let env = TestEnv::new();
    let p = "/org/gsctl/test/custom1/";
    env.gsctl(&["set", "org.gsctl.test.reloc", "name", "Terminal", "--path", p]).ok();
    env.gsctl(&["revert", "last"]).ok();
    assert_eq!(env.gsctl(&["get", "org.gsctl.test.reloc", "name", "--path", p]).ok()["value"], "");
}

// --- doctor -----------------------------------------------------------------

fn check<'a>(out: &'a Value, name: &str) -> &'a Value {
    out["checks"].as_array().unwrap().iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("no {name}"))
}

#[test]
fn doctor_in_a_working_session() {
    let env = TestEnv::new();
    let out = env.gsctl(&["doctor"]).ok();
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(check(&out, "settings_backend")["status"], "ok");
    assert_eq!(check(&out, "session_bus")["status"], "ok");
    assert_eq!(check(&out, "dconf_service")["status"], "ok");
    assert_eq!(check(&out, "gnome_shell")["status"], "warn");
    assert_eq!(check(&out, "dconf_database")["status"], "warn", "nothing written yet");
    assert_eq!(check(&out, "session_mode")["detail"]["name"], "testmode");

    env.gsctl(&["set", S, "count-i", "1"]).ok();
    assert_eq!(check(&env.gsctl(&["doctor"]).ok(), "dconf_database")["status"], "ok");
}

#[test]
fn doctor_without_a_bus() {
    let env = TestEnv::new();
    let r = env.gsctl_without_bus(&["doctor"]);
    assert_eq!(r.status, 0);
    let out = r.json();
    assert_eq!(out["ok"], false);
    assert_eq!(check(&out, "session_bus")["status"], "error");
}
