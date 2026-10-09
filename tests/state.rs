//! `export` / `apply` end to end, against the hermetic test schemas.

mod common;

use common::TestEnv;
use serde_json::{Value, json};

const S: &str = "org.gsap.test";
const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const CUSTOM0: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/";

/// Customise keys across several areas, including a custom keybinding, an
/// `a{sv}` that JSON can't represent exactly, and an app-state key.
fn customise(env: &TestEnv) {
    env.gsap(&["set", S, "count-i", "42"]).ok();
    env.gsap(&["set", S, "dict-asv", "{'n': <uint32 7>, 's': <'x'>}"]).ok();
    env.gsap(&["set", S, "launch-count", "12"]).ok();
    env.gsap(&["set", "org.gnome.desktop.screensaver", "lock-delay", "30"]).ok();
    env.gsap(&["set", "org.gnome.shell.extensions.gsap-demo", "show-indicator", "false"]).ok();
    env.gsap(&["set", MEDIA_KEYS, "custom-keybindings", &format!(r#"["{CUSTOM0}"]"#)]).ok();
    for (k, v) in [("name", "Terminal"), ("command", "ptyxis"), ("binding", "<Super>Return")] {
        env.gsap(&["set", CUSTOM, k, v, "--path", CUSTOM0]).ok();
    }
}

fn write_json(env: &TestEnv, name: &str, doc: &Value) -> String {
    let path = env.path(name);
    std::fs::write(&path, serde_json::to_string_pretty(doc).unwrap()).unwrap();
    path.to_str().unwrap().to_string()
}

fn journal_len(env: &TestEnv) -> usize {
    std::fs::read_to_string(env.journal()).map(|t| t.lines().count()).unwrap_or(0)
}

#[test]
fn export_nothing_customised() {
    let env = TestEnv::new();
    let doc = env.gsap(&["export"]).ok();
    assert_eq!(doc["gsap"], 1);
    assert_eq!(doc["settings"], json!({}));
}

#[test]
fn export_groups_by_area_and_skips_state() {
    let env = TestEnv::new();
    customise(&env);
    let doc = env.gsap(&["export"]).ok();
    assert_eq!(doc["area"], json!(["extensions", "keybindings", "other", "power"]));
    let s = &doc["settings"];
    assert_eq!(s[S]["count-i"], 42);
    // JSON would turn the uint32 into an int32, so it's kept as GVariant text.
    assert_eq!(s[S]["dict-asv"], json!({"$gvariant": "{'n': <uint32 7>, 's': <'x'>}"}));
    assert_eq!(s["org.gnome.desktop.screensaver"]["lock-delay"], 30);
    assert_eq!(s["org.gnome.shell.extensions.gsap-demo"]["show-indicator"], false);
    assert_eq!(s[MEDIA_KEYS]["custom-keybindings"], json!([CUSTOM0]));
    assert_eq!(
        s[format!("{CUSTOM}:{CUSTOM0}")],
        json!({"binding": "<Super>Return", "command": "ptyxis", "name": "Terminal"})
    );

    assert!(s[S].get("launch-count").is_none());
    assert_eq!(doc["skipped_state"][0]["key"], "launch-count");
    let all = env.gsap(&["export", "--include-state"]).ok();
    assert_eq!(all["settings"][S]["launch-count"], 12);

    let power = env.gsap(&["export", "--area", "power"]).ok();
    assert_eq!(power["settings"], json!({"org.gnome.desktop.screensaver": {"lock-delay": 30}}));
    assert_eq!(env.gsap(&["export", "--area", "nope"]).status, 2);
}

#[test]
fn export_to_directory_writes_one_file_per_area() {
    let env = TestEnv::new();
    customise(&env);
    let dir = env.path("state-files");
    let out = env.gsap(&["export", "--out", dir.to_str().unwrap()]).ok();
    let files: Vec<&str> = out["files"].as_array().unwrap().iter().map(|f| f["area"].as_str().unwrap()).collect();
    assert_eq!(files, ["extensions", "keybindings", "other", "power"]);
    let kb: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("keybindings.json")).unwrap()).unwrap();
    assert_eq!(kb["area"], "keybindings");
    assert!(kb.get("skipped_state").is_none(), "state files stay clean");
    assert_eq!(kb["settings"].as_object().unwrap().len(), 2);
}

#[test]
fn export_then_apply_reproduces_the_settings() {
    let source = TestEnv::new();
    customise(&source);
    let exported = source.gsap(&["export"]).ok();

    let target = TestEnv::new();
    let file = write_json(&target, "all.json", &exported);
    let dry = target.gsap(&["apply", &file, "--dry-run"]).ok();
    assert_eq!(dry["changes"].as_array().unwrap().len(), 8);
    assert_eq!(journal_len(&target), 0);

    let out = target.gsap(&["apply", &file]).ok();
    assert_eq!(out["changes"].as_array().unwrap().len(), 8);
    assert!(out["batch"].as_str().unwrap().starts_with("apply-"));

    let mut again = target.gsap(&["export"]).ok();
    let mut expected = exported.clone();
    again.as_object_mut().unwrap().remove("skipped_state");
    expected.as_object_mut().unwrap().remove("skipped_state");
    assert_eq!(again, expected);
    let got = target.gsap(&["get", CUSTOM, "binding", "--path", CUSTOM0]).ok();
    assert_eq!(got["value"], "<Super>Return");

    // Idempotent: a second apply changes nothing and journals nothing.
    let before = journal_len(&target);
    let second = target.gsap(&["apply", &file]).ok();
    assert_eq!(second["changes"], json!([]));
    assert_eq!(second["unchanged"], 8);
    assert_eq!(second["batch"], Value::Null);
    assert_eq!(journal_len(&target), before);
}

#[test]
fn export_includes_listed_ptyxis_profiles() {
    let source = TestEnv::new();
    let profile = |uuid: &str| format!("/org/gnome/Ptyxis/Profiles/{uuid}/");
    source.gsap(&["set", "org.gnome.Ptyxis", "profile-uuids", r#"["aa11", "bb22"]"#]).ok();
    source.gsap(&["set", "org.gnome.Ptyxis.Profile", "label", "Work", "--path", &profile("aa11")]).ok();
    source.gsap(&["set", "org.gnome.Ptyxis.Profile", "opacity", "0.9", "--path", &profile("aa11")]).ok();
    // A stale profile that is no longer listed stays out of the export.
    source.gsap(&["set", "org.gnome.Ptyxis.Profile", "label", "Old", "--path", &profile("cc33")]).ok();

    let doc = source.gsap(&["export"]).ok();
    let s = doc["settings"].as_object().unwrap();
    assert_eq!(s[&format!("org.gnome.Ptyxis.Profile:{}", profile("aa11"))], json!({"label": "Work", "opacity": 0.9}));
    assert!(s.keys().all(|k| !k.contains("cc33") && !k.contains("bb22")), "{doc}");
    assert_eq!(s["org.gnome.Ptyxis"]["profile-uuids"], json!(["aa11", "bb22"]));

    let target = TestEnv::new();
    let file = write_json(&target, "ptyxis.json", &doc);
    target.gsap(&["apply", &file]).ok();
    let got = target.gsap(&["get", "org.gnome.Ptyxis.Profile", "label", "--path", &profile("aa11")]).ok();
    assert_eq!(got["value"], "Work");
}

#[test]
fn apply_validates_everything_before_writing() {
    let env = TestEnv::new();
    let doc = json!({
        "gsap": 1,
        "settings": {
            S: { "count-i": 50, "flag-b": "yes", "mode": "sideways", "nope": 1 },
            "org.nope": { "x": 1 },
        }
    });
    let file = write_json(&env, "bad.json", &doc);
    let r = env.gsap(&["apply", &file]);
    assert_eq!(r.status, 4, "{}", r.stderr);
    let err = r.error()["error"].clone();
    assert_eq!(err["code"], "batch_invalid");
    let problems = err["problems"].as_array().unwrap();
    let codes: Vec<(&str, &str)> =
        problems.iter().map(|p| (p["key"].as_str().unwrap(), p["code"].as_str().unwrap())).collect();
    assert_eq!(
        codes,
        [("flag-b", "invalid_value"), ("mode", "invalid_value"), ("nope", "key_not_found"), ("", "schema_not_found")]
    );
    // The valid key was not written either.
    assert_eq!(env.gsap(&["get", S, "count-i"]).ok()["value"], 10);
    assert_eq!(journal_len(&env), 0);
}

#[test]
fn apply_rejects_files_that_are_not_state_files() {
    let env = TestEnv::new();
    let file = write_json(&env, "x.json", &json!({"hello": 1}));
    assert_eq!(env.gsap(&["apply", &file]).status, 2);
    assert_eq!(env.gsap(&["apply", env.path("missing.json").to_str().unwrap()]).status, 2);
}

#[test]
fn apply_needs_yes_for_guarded_keys() {
    let env = TestEnv::new();
    let doc = json!({"gsap": 1, "settings": {
        "org.gnome.desktop.screensaver": { "lock-enabled": false, "lock-delay": 5 }
    }});
    let file = write_json(&env, "guarded.json", &doc);
    let dry = env.gsap(&["apply", &file, "--dry-run"]).ok();
    assert_eq!(dry["requires_yes"][0]["key"], "lock-enabled");

    let r = env.gsap(&["apply", &file]);
    assert_eq!(r.status, 6);
    assert_eq!(env.gsap(&["get", "org.gnome.desktop.screensaver", "lock-delay"]).ok()["value"], 0);

    env.gsap(&["apply", &file, "--yes"]).ok();
    assert_eq!(env.gsap(&["get", "org.gnome.desktop.screensaver", "lock-enabled"]).ok()["value"], false);
}

#[test]
fn apply_area_filter() {
    let env = TestEnv::new();
    let doc = json!({"gsap": 1, "settings": {
        S: { "count-i": 50 },
        "org.gnome.desktop.screensaver": { "lock-delay": 5 }
    }});
    let file = write_json(&env, "mixed.json", &doc);
    let out = env.gsap(&["apply", &file, "--area", "power"]).ok();
    assert_eq!(out["changes"].as_array().unwrap().len(), 1);
    assert_eq!(env.gsap(&["get", S, "count-i"]).ok()["value"], 10);
}

#[test]
fn revert_undoes_a_whole_apply() {
    let env = TestEnv::new();
    env.gsap(&["set", S, "name-s", "Mine"]).ok();
    let doc = json!({"gsap": 1, "settings": {
        S: { "count-i": 50, "name-s": "Theirs" },
        "org.gnome.desktop.screensaver": { "lock-delay": 5 }
    }});
    let file = write_json(&env, "a.json", &doc);
    let applied = env.gsap(&["apply", &file]).ok();
    let batch = applied["batch"].as_str().unwrap().to_string();

    let h = env.gsap(&["history"]).ok();
    assert_eq!(h["entries"][0]["batch"], batch.as_str());
    assert_eq!(h["entries"][0]["op"], "apply");

    let dry = env.gsap(&["revert", "last", "--dry-run"]).ok();
    assert_eq!(dry["results"].as_array().unwrap().len(), 3);
    assert_eq!(env.gsap(&["get", S, "count-i"]).ok()["value"], 50);

    let out = env.gsap(&["revert", &batch]).ok();
    assert_eq!(out["reverts_batch"], batch.as_str());
    assert_eq!(out["results"].as_array().unwrap().len(), 3);
    assert_eq!(env.gsap(&["get", S, "name-s"]).ok()["value"], "Mine");
    assert_eq!(env.gsap(&["get", S, "count-i"]).ok()["is_default"], true);
    assert_eq!(env.gsap(&["get", "org.gnome.desktop.screensaver", "lock-delay"]).ok()["is_default"], true);
}

#[test]
fn batch_revert_checks_every_key_first() {
    let env = TestEnv::new();
    let doc = json!({"gsap": 1, "settings": { S: { "count-i": 50, "name-s": "Theirs" } }});
    let file = write_json(&env, "a.json", &doc);
    let batch = env.gsap(&["apply", &file]).ok()["batch"].as_str().unwrap().to_string();
    env.gsap(&["set", S, "name-s", "Changed later"]).ok();

    let r = env.gsap(&["revert", &batch]);
    assert_eq!(r.status, 7);
    // count-i was not reverted either.
    assert_eq!(env.gsap(&["get", S, "count-i"]).ok()["value"], 50);
}

#[test]
fn apply_prune_resets_unlisted_keys_in_the_files_areas() {
    let env = TestEnv::new();
    customise(&env); // count-i, dict-asv, launch-count (state), lock-delay (power), a custom keybinding
    env.gsap(&["set", S, "name-s", "Mine"]).ok();
    let doc = json!({"gsap": 1, "area": "other", "settings": { S: { "count-i": 42 } }});
    let file = write_json(&env, "other.json", &doc);

    let before = journal_len(&env);
    let dry = env.gsap(&["apply", &file, "--prune", "--dry-run"]).ok();
    let keys: Vec<&str> = dry["pruned"].as_array().unwrap().iter().map(|p| p["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["dict-asv", "name-s"], "{dry}");
    assert_eq!(dry["changes"], json!([]));
    assert_eq!(journal_len(&env), before);

    let out = env.gsap(&["apply", &file, "--prune"]).ok();
    assert_eq!(out["pruned"].as_array().unwrap().len(), 2);
    assert_eq!(env.gsap(&["get", S, "name-s"]).ok()["is_default"], true);
    assert_eq!(env.gsap(&["get", S, "count-i"]).ok()["value"], 42);
    assert_eq!(env.gsap(&["get", S, "launch-count"]).ok()["value"], 12, "app state is never pruned");
    assert_eq!(env.gsap(&["get", "org.gnome.desktop.screensaver", "lock-delay"]).ok()["value"], 30, "other area");

    env.gsap(&["revert", "last"]).ok();
    assert_eq!(env.gsap(&["get", S, "name-s"]).ok()["value"], "Mine");

    // Without --prune the same file leaves everything else alone.
    assert_eq!(env.gsap(&["apply", &file]).ok()["pruned"], Value::Null);
}

#[test]
fn apply_prune_covers_listed_instances_guards_and_needs_an_area() {
    let env = TestEnv::new();
    customise(&env);
    let doc = json!({"gsap": 1, "area": "keybindings", "settings": {}});
    let file = write_json(&env, "kb.json", &doc);
    let out = env.gsap(&["apply", &file, "--prune"]).ok();
    assert_eq!(out["pruned"].as_array().unwrap().len(), 4, "{out}");
    assert_eq!(env.gsap(&["get", MEDIA_KEYS, "custom-keybindings"]).ok()["is_default"], true);
    assert_eq!(env.gsap(&["get", CUSTOM, "name", "--path", CUSTOM0]).ok()["is_default"], true);

    env.gsap(&["set", "org.gnome.desktop.screensaver", "lock-enabled", "false", "--yes"]).ok();
    let power = write_json(&env, "power.json", &json!({"gsap": 1, "area": "power", "settings": {}}));
    let r = env.gsap(&["apply", &power, "--prune"]);
    assert_eq!(r.status, 6, "{}", r.stderr);
    assert_eq!(env.gsap(&["get", "org.gnome.desktop.screensaver", "lock-enabled"]).ok()["value"], false);

    let no_area = write_json(&env, "bare.json", &json!({"gsap": 1, "settings": {}}));
    assert_eq!(env.gsap(&["apply", &no_area, "--prune"]).status, 2);
}

#[test]
fn areas_lists_every_area() {
    let env = TestEnv::new();
    let out = env.gsap(&["areas"]).ok();
    let names: Vec<&str> = out["areas"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["extensions", "keybindings", "interface", "shell", "input", "power", "remote", "a11y", "other"]);
}
