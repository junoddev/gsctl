//! Guards and security hardening: guards follow the dconf location written,
//! not just the schema id.

mod common;

use common::TestEnv;
use serde_json::{Value, json};

const RELOC: &str = "org.gsctl.test.reloc-enable";
const RDP: &str = "org.gnome.desktop.remote-desktop.rdp";
const RDP_PATH: &str = "/org/gnome/desktop/remote-desktop/rdp/";
/// No a11y schema is installed in the test env: guarded by its conventional path.
const A11Y_PATH: &str = "/org/gnome/desktop/a11y/keyboard/";
const CUSTOM: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";

fn write_json(env: &TestEnv, name: &str, doc: &Value) -> String {
    let path = env.path(name);
    std::fs::write(&path, serde_json::to_string_pretty(doc).unwrap()).unwrap();
    path.to_str().unwrap().to_string()
}

#[test]
fn path_of_a_fixed_schema_is_refused_for_relocatable_schemas() {
    let env = TestEnv::new();
    for args in [
        vec!["set", RELOC, "enable", "true", "--path", RDP_PATH, "--dry-run"],
        vec!["set", RELOC, "enable", "true", "--path", RDP_PATH, "--yes"],
        vec!["reset", RELOC, "enable", "--path", RDP_PATH, "--yes"],
    ] {
        let r = env.gsctl(&args);
        assert_eq!(r.status, 2, "{args:?}: {}", r.stderr);
        assert!(r.error()["error"]["message"].as_str().unwrap().contains(RDP), "{}", r.stderr);
    }
    assert_eq!(env.gsctl(&["get", RDP, "enable"]).ok()["is_default"], true);
}

#[test]
fn guards_follow_the_path_written() {
    let env = TestEnv::new();
    // The fixed schema itself is guarded.
    assert_eq!(env.gsctl(&["set", RDP, "enable", "true"]).status, 6);

    // A guarded location without an installed schema: guarded by its path.
    let dry = env.gsctl(&["set", RELOC, "enable", "true", "--path", A11Y_PATH, "--dry-run"]).ok();
    assert_eq!(dry["requires_yes"], true, "{dry}");
    let r = env.gsctl(&["set", RELOC, "enable", "true", "--path", A11Y_PATH]);
    assert_eq!(r.status, 6, "{}", r.stderr);
    assert_eq!(env.gsctl(&["get", RELOC, "enable", "--path", A11Y_PATH]).ok()["value"], false);
    let deeper = env.gsctl(&["set", RELOC, "enable", "true", "--path", "/org/gnome/desktop/remote-desktop/vnc/x/"]);
    assert_eq!(deeper.status, 6, "{}", deeper.stderr);

    // Revert is guarded on the path too.
    env.gsctl(&["set", RELOC, "enable", "true", "--path", A11Y_PATH, "--yes"]).ok();
    assert_eq!(env.gsctl(&["revert", "last"]).status, 6);
    env.gsctl(&["revert", "last", "--yes"]).ok();

    // Anywhere else is fine.
    let free = env.gsctl(&["set", RELOC, "enable", "true", "--path", "/org/gsctl/free/x/", "--dry-run"]).ok();
    assert_eq!(free["requires_yes"], false);
}

#[test]
fn listed_relocatable_schemas_keep_to_their_paths() {
    let env = TestEnv::new();
    let r = env.gsctl(&["set", CUSTOM, "name", "x", "--path", "/org/gsctl/elsewhere/"]);
    assert_eq!(r.status, 2, "{}", r.stderr);
    let r = env.gsctl(&["set", "org.gnome.Ptyxis.Profile", "label", "x", "--path", "/org/gnome/Ptyxis/"]);
    assert_eq!(r.status, 2, "{}", r.stderr);
    env.gsctl(&["set", "org.gnome.Ptyxis.Profile", "label", "x", "--path", "/org/gnome/Ptyxis/Profiles/a1/"]).ok();
}

#[test]
fn apply_checks_instance_paths_and_their_guards() {
    let env = TestEnv::new();
    let doc = json!({"gsctl": 1, "area": "other", "settings": {
        format!("{RELOC}:{RDP_PATH}"): {"enable": true},
    }});
    let r = env.gsctl(&["apply", &write_json(&env, "rdp.json", &doc), "--dry-run"]);
    assert_eq!(r.status, 2, "{}", r.stderr);
    assert_eq!(r.error()["error"]["code"], "batch_invalid");

    let doc = json!({"gsctl": 1, "area": "other", "settings": {
        format!("{RELOC}:{A11Y_PATH}"): {"enable": true},
    }});
    let file = write_json(&env, "a11y.json", &doc);
    let dry = env.gsctl(&["apply", &file, "--dry-run"]).ok();
    assert_eq!(dry["requires_yes"][0]["path"], A11Y_PATH, "{dry}");
    assert_eq!(env.gsctl(&["apply", &file]).status, 6);
    env.gsctl(&["apply", &file, "--yes"]).ok();
}

const CUSTOM0: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/";
const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const SHELL: &str = "org.gnome.shell";

#[test]
fn code_executing_keys_need_yes_and_say_so() {
    let env = TestEnv::new();
    let k = env.gsctl(&["schema", CUSTOM, "command"]).ok();
    assert!(k["guarded"].is_string(), "{k}");
    assert_eq!(k["executes"], true);
    let plain = env.gsctl(&["schema", "org.gnome.desktop.screensaver", "lock-enabled"]).ok();
    assert!(plain["guarded"].is_string());
    assert_eq!(plain["executes"], false);
    assert_eq!(env.gsctl(&["schema", "org.gsctl.test", "count-i"]).ok()["executes"], false);

    let dry = env.gsctl(&["set", CUSTOM, "command", "touch x", "--path", CUSTOM0, "--dry-run"]).ok();
    assert_eq!(dry["requires_yes"], true);
    assert_eq!(dry["executes"], true);
    assert!(dry["guarded"].is_string());
    let r = env.gsctl(&["set", CUSTOM, "command", "touch x", "--path", CUSTOM0]);
    assert_eq!(r.status, 6, "{}", r.stderr);
    let r = env.gsctl(&["set", MEDIA_KEYS, "custom-keybindings", &format!(r#"["{CUSTOM0}"]"#)]);
    assert_eq!(r.status, 6, "{}", r.stderr);
    env.gsctl(&["set", CUSTOM, "command", "true", "--path", CUSTOM0, "--yes"]).ok();
}

#[test]
fn extension_lists_are_guarded_wholesale() {
    let env = TestEnv::new();
    assert_eq!(env.gsctl(&["set", SHELL, "disabled-extensions", r#"["a@b"]"#]).status, 6);
    let dry = env.gsctl(&["set", SHELL, "enabled-extensions", r#"["a@b"]"#, "--dry-run"]).ok();
    assert_eq!(dry["executes"], true);
}

#[test]
fn ext_enable_needs_yes_but_disable_does_not() {
    let env = TestEnv::new();
    let uuid = common::EXTENSION_UUID;
    let dry = env.gsctl(&["ext", "enable", uuid, "--dry-run"]).ok();
    assert_eq!(dry["requires_yes"], true, "{dry}");
    assert_eq!(dry["executes"], true);
    let r = env.gsctl(&["ext", "enable", uuid]);
    assert_eq!(r.status, 6, "{}", r.stderr);
    assert_eq!(r.error()["error"]["code"], "guarded");
    assert_eq!(env.gsctl(&["get", SHELL, "enabled-extensions"]).ok()["is_default"], true);
    env.gsctl(&["ext", "enable", uuid, "--yes"]).ok();
    assert_eq!(env.gsctl(&["get", SHELL, "enabled-extensions"]).ok()["value"], json!([uuid]));

    let dry = env.gsctl(&["ext", "disable", uuid, "--dry-run"]).ok();
    assert_eq!(dry["requires_yes"], false);
    env.gsctl(&["ext", "disable", uuid]).ok();
    // Already enabled (by the session mode): nothing to confirm.
    let sys = common::SYSTEM_EXTENSION_UUID;
    assert_eq!(env.gsctl(&["ext", "enable", sys, "--dry-run"]).ok()["requires_yes"], false);
}

#[test]
fn keys_add_needs_yes() {
    let env = TestEnv::new();
    let r = env.gsctl(&["keys", "add", "Term", "<Super>F9", "ptyxis"]);
    assert_eq!(r.status, 6, "{}", r.stderr);
    assert_eq!(env.gsctl(&["get", MEDIA_KEYS, "custom-keybindings"]).ok()["is_default"], true);
    let dry = env.gsctl(&["keys", "add", "Term", "<Super>F9", "ptyxis", "--dry-run"]).ok();
    assert!(dry["writes"].as_array().unwrap().iter().any(|w| w["executes"] == true), "{dry}");
    env.gsctl(&["keys", "add", "Term", "<Super>F9", "ptyxis", "--yes"]).ok();
    // Removing a binding runs nothing, so it needs no --yes.
    env.gsctl(&["keys", "remove", "Term"]).ok();
}
