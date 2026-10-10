//! `gsctl keys` against stand-in keybinding schemas.

mod common;

use common::TestEnv;
use serde_json::{Value, json};

const WM: &str = "org.gnome.desktop.wm.keybindings";
const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const CUSTOM0: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/";
const CUSTOM1: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom1/";

fn keys_of(v: &Value) -> Vec<String> {
    v["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| format!("{} {}", b["schema"].as_str().unwrap(), b["key"].as_str().unwrap()))
        .collect()
}

#[test]
fn list_skips_empty_bindings() {
    let env = TestEnv::new();
    let out = env.gsctl(&["keys", "list"]).ok();
    let keys = keys_of(&out);
    assert!(keys.contains(&format!("{WM} close")));
    assert!(keys.contains(&format!("{MEDIA_KEYS} screensaver")));
    assert!(!keys.contains(&format!("{WM} show-desktop")), "empty binding listed");
    assert!(!keys.iter().any(|k| k.ends_with("custom-keybindings")));
}

#[test]
fn find_normalizes_accelerators() {
    let env = TestEnv::new();
    let out = env.gsctl(&["keys", "find", "<Mod4>H"]).ok();
    assert_eq!(out["normalized"], "<Super>h");
    assert_eq!(out["free"], false);
    assert_eq!(keys_of(&out), [format!("{WM} minimize")]);

    let out = env.gsctl(&["keys", "find", "<Super>Tab"]).ok();
    assert_eq!(keys_of(&out), [format!("{WM} switch-applications")]);

    let free = env.gsctl(&["keys", "find", "<Ctrl><Super>t"]).ok();
    assert_eq!(free["free"], true);
    assert_eq!(free["normalized"], "<Control><Super>t");

    assert_eq!(env.gsctl(&["keys", "find", "<Banana>x"]).status, 4);
}

#[test]
fn conflicts_across_schemas() {
    let env = TestEnv::new();
    assert_eq!(env.gsctl(&["keys", "conflicts"]).ok()["count"], 0);
    env.gsctl(&["set", "org.gnome.shell.keybindings", "toggle-overview", r#"["<Alt>F4"]"#]).ok();
    let out = env.gsctl(&["keys", "conflicts"]).ok();
    assert_eq!(out["count"], 1);
    assert_eq!(out["conflicts"][0]["accel"], "<Alt>F4");
    assert_eq!(out["conflicts"][0]["bindings"].as_array().unwrap().len(), 2);
}

#[test]
fn add_creates_a_custom_binding() {
    let env = TestEnv::new();
    let dry = env.gsctl(&["keys", "add", "Terminal", "<Super>Return", "ptyxis", "--dry-run"]).ok();
    assert_eq!(dry["path"], CUSTOM0);
    assert_eq!(env.gsctl(&["keys", "list", "--custom"]).ok()["count"], 0);

    let out = env.gsctl(&["keys", "add", "Terminal", "<Super>Return", "ptyxis", "--yes"]).ok();
    assert_eq!(out["path"], CUSTOM0);
    assert!(out["batch"].as_str().unwrap().starts_with("keys-add-"));
    let list = env.gsctl(&["keys", "list", "--custom"]).ok();
    assert_eq!(list["count"], 1);
    assert_eq!(list["bindings"][0]["custom"], json!({"name": "Terminal", "command": "ptyxis"}));
    assert_eq!(list["bindings"][0]["accels"], json!(["<Super>Return"]));
    assert_eq!(env.gsctl(&["get", MEDIA_KEYS, "custom-keybindings"]).ok()["value"], json!([CUSTOM0]));

    // The next one goes to custom1, and find sees the custom binding.
    let second = env.gsctl(&["keys", "add", "Files", "<Super>e", "nautilus", "--yes"]).ok();
    assert_eq!(second["path"], CUSTOM1);
    let found = env.gsctl(&["keys", "find", "<super>return"]).ok();
    assert_eq!(found["bindings"][0]["custom"]["name"], "Terminal");
}

#[test]
fn add_refuses_taken_accelerators() {
    let env = TestEnv::new();
    let r = env.gsctl(&["keys", "add", "Lock", "<Super>l", "true", "--yes"]);
    assert_eq!(r.status, 7, "{}", r.stderr);
    assert!(r.error()["error"]["message"].as_str().unwrap().contains("screensaver"));
    assert_eq!(env.gsctl(&["keys", "list", "--custom"]).ok()["count"], 0);

    env.gsctl(&["keys", "add", "Lock", "<Super>l", "true", "--force", "--yes"]).ok();
    assert_eq!(env.gsctl(&["keys", "conflicts"]).ok()["count"], 1);

    assert_eq!(env.gsctl(&["keys", "add", "Bad", "<Super>", "x", "--yes"]).status, 4);
}

#[test]
fn remove_deletes_a_custom_binding_and_revert_brings_it_back() {
    let env = TestEnv::new();
    env.gsctl(&["keys", "add", "Terminal", "<Super>Return", "ptyxis", "--yes"]).ok();
    env.gsctl(&["keys", "add", "Files", "<Super>e", "nautilus", "--yes"]).ok();

    let dry = env.gsctl(&["keys", "remove", "Terminal", "--dry-run"]).ok();
    assert_eq!(dry["path"], CUSTOM0);
    assert_eq!(env.gsctl(&["keys", "list", "--custom"]).ok()["count"], 2);

    let out = env.gsctl(&["keys", "remove", "Terminal"]).ok();
    assert_eq!(out["path"], CUSTOM0);
    assert_eq!(out["accel"], "<Super>Return");
    assert!(out["batch"].as_str().unwrap().starts_with("keys-remove-"));
    assert_eq!(out["writes"].as_array().unwrap().len(), 4, "{out}");
    assert_eq!(env.gsctl(&["get", MEDIA_KEYS, "custom-keybindings"]).ok()["value"], json!([CUSTOM1]));
    assert_eq!(env.gsctl(&["get", CUSTOM, "name", "--path", CUSTOM0]).ok()["is_default"], true);
    assert_eq!(env.gsctl(&["keys", "find", "<Super>Return"]).ok()["free"], true);

    env.gsctl(&["revert", "last", "--yes"]).ok();
    let list = env.gsctl(&["keys", "list", "--custom"]).ok();
    assert_eq!(list["count"], 2, "{list}");
    assert_eq!(env.gsctl(&["get", MEDIA_KEYS, "custom-keybindings"]).ok()["value"], json!([CUSTOM0, CUSTOM1]));
    assert_eq!(env.gsctl(&["get", CUSTOM, "command", "--path", CUSTOM0]).ok()["value"], "ptyxis");

    // By short path and full path too.
    env.gsctl(&["keys", "remove", "custom1"]).ok();
    env.gsctl(&["keys", "remove", CUSTOM0]).ok();
    assert_eq!(env.gsctl(&["keys", "list", "--custom"]).ok()["count"], 0);
    // The last removal resets the list instead of pinning [].
    assert_eq!(env.gsctl(&["get", MEDIA_KEYS, "custom-keybindings"]).ok()["is_default"], true);
}

#[test]
fn remove_refuses_unknown_and_ambiguous_names() {
    let env = TestEnv::new();
    let r = env.gsctl(&["keys", "remove", "Nope"]);
    assert_eq!(r.status, 3, "{}", r.stderr);
    assert_eq!(r.error()["error"]["code"], "binding_not_found");

    env.gsctl(&["keys", "add", "Same", "<Super>F1", "true", "--yes"]).ok();
    // A second binding with the same name takes --force.
    env.gsctl(&["keys", "add", "Same", "<Super>F2", "true", "--force", "--yes"]).ok();
    let r = env.gsctl(&["keys", "remove", "Same"]);
    assert_eq!(r.status, 2, "{}", r.stderr);
    assert!(r.error()["error"]["message"].as_str().unwrap().contains(CUSTOM1));
    assert_eq!(env.gsctl(&["keys", "list", "--custom"]).ok()["count"], 2);
}

#[test]
fn revert_last_undoes_keys_add() {
    let env = TestEnv::new();
    env.gsctl(&["keys", "add", "Terminal", "<Super>Return", "ptyxis", "--yes"]).ok();
    env.gsctl(&["revert", "last", "--yes"]).ok();
    assert_eq!(env.gsctl(&["keys", "list", "--custom"]).ok()["count"], 0);
    assert_eq!(env.gsctl(&["get", MEDIA_KEYS, "custom-keybindings"]).ok()["is_default"], true);
    let name = env.gsctl(&["get", CUSTOM, "name", "--path", CUSTOM0]).ok();
    assert_eq!(name["is_default"], true);
}

const DEMO: &str = "org.gnome.shell.extensions.gsctl-demo";
const SYSTEM_EXT: &str = "org.gnome.shell.extensions.gsctl-system";
const ORPHAN: &str = "org.gnome.shell.extensions.gsctl-orphan.keybindings";

fn binding<'a>(v: &'a Value, schema: &str, key: &str) -> Option<&'a Value> {
    v["bindings"].as_array().unwrap().iter().find(|b| b["schema"] == schema && b["key"] == key)
}

#[test]
fn list_includes_extension_bindings_with_their_source() {
    let env = TestEnv::new();
    let out = env.gsctl(&["keys", "list"]).ok();

    let wm = binding(&out, WM, "close").unwrap();
    assert_eq!(wm["source"], "gnome");
    assert_eq!(wm["active"], true);

    // Bundled with a user extension, found by its value; the extension is off.
    let demo = binding(&out, DEMO, "take-window").expect("extension binding listed");
    assert_eq!(demo["source"], "extension");
    assert_eq!(demo["extension"], "demo@gsctl.test");
    assert_eq!(demo["accels"], json!(["<Super>t"]));
    assert_eq!(demo["active"], false);

    // Installed system-wide, owned through the extension's JS, enabled by the session mode.
    let sys = binding(&out, SYSTEM_EXT, "app-hotkey-1").expect("system extension binding listed");
    assert_eq!(sys["extension"], "system@gsctl.test");
    assert_eq!(sys["active"], true);

    // A keybinding schema: string keys count too.
    let orphan = binding(&out, ORPHAN, "toggle").expect("string key in a keybinding schema");
    assert_eq!(orphan["accels"], json!(["<Super>F12"]));
    assert_eq!(orphan["source"], "extension");

    // Not bindings: a string that only displays one, words, an empty list.
    assert!(binding(&out, SYSTEM_EXT, "shortcut-text").is_none());
    assert!(binding(&out, SYSTEM_EXT, "labels").is_none());
    assert!(binding(&out, SYSTEM_EXT, "dock-side").is_none());
    assert!(binding(&out, ORPHAN, "spare").is_none());
    assert!(binding(&out, "org.gsctl.test", "list-as").is_none());

    // A value set later is picked up.
    env.gsctl(&["set", ORPHAN, "spare", r#"["<Super>k"]"#]).ok();
    let out = env.gsctl(&["keys", "find", "<Super>k"]).ok();
    assert_eq!(keys_of(&out), [format!("{ORPHAN} spare")]);
}

#[test]
fn find_and_conflicts_see_active_extension_bindings() {
    let env = TestEnv::new();
    // The demo extension is off: its binding is reported but doesn't take the accelerator.
    let out = env.gsctl(&["keys", "find", "<Super>t"]).ok();
    assert_eq!(out["free"], true);
    assert_eq!(keys_of(&out), [format!("{DEMO} take-window")]);

    env.gsctl(&["set", "org.gnome.shell", "enabled-extensions", r#"["demo@gsctl.test"]"#, "--yes"]).ok();
    let out = env.gsctl(&["keys", "find", "<super>T"]).ok();
    assert_eq!(out["free"], false);
    assert_eq!(out["bindings"][0]["extension"], "demo@gsctl.test");

    let r = env.gsctl(&["keys", "add", "Terminal", "<Super>t", "ptyxis", "--yes"]);
    assert_eq!(r.status, 7, "{}", r.stderr);
    assert!(r.error()["error"]["message"].as_str().unwrap().contains("demo@gsctl.test take-window"));

    env.gsctl(&["set", WM, "minimize", r#"["<Super>1"]"#]).ok();
    let out = env.gsctl(&["keys", "conflicts"]).ok();
    assert_eq!(out["count"], 1, "{out}");
    assert_eq!(out["conflicts"][0]["accel"], "<Super>1");
    assert_eq!(keys_of(&out["conflicts"][0]), [format!("{WM} minimize"), format!("{SYSTEM_EXT} app-hotkey-1")]);
}

#[test]
fn odd_but_valid_bindings_stay_listed() {
    let env = TestEnv::new();
    let odd = r#"["<Release><Super>z", "<Mod5>space", "<Lock>F1", "<Super><shift>f", "<Super>KP_2", "<Super>Above_Tab", "XF86AudioMute"]"#;
    env.gsctl(&["set", WM, "show-desktop", odd]).ok();
    let out = env.gsctl(&["keys", "list"]).ok();
    let b = binding(&out, WM, "show-desktop").unwrap();
    assert_eq!(b["accels"].as_array().unwrap().len(), 7, "{b}");
    for accel in ["<Release><Super>z", "<Mod5>space", "<Lock>F1", "<Shift><Super>f", "<Super>kp_2", "XF86AudioMute"] {
        let found = env.gsctl(&["keys", "find", accel]).ok();
        assert_eq!(keys_of(&found), [format!("{WM} show-desktop")], "{accel}");
    }
    // Even a form gsctl wouldn't accept as input is listed as it is.
    env.gsctl(&["set", WM, "show-desktop", r#"["<Super>banana"]"#]).ok();
    let out = env.gsctl(&["keys", "list"]).ok();
    assert_eq!(binding(&out, WM, "show-desktop").unwrap()["accels"], json!(["<Super>banana"]));
}

#[test]
fn invalid_accelerators_are_refused_with_a_suggestion() {
    let env = TestEnv::new();
    for (input, suggestion) in [
        ("Super+T", "<Super>t"),
        ("Ctrl+Alt+Del", "<Control><Alt>Delete"),
        ("super+return", "<Super>Return"),
        ("<Super>Del", "<Super>Delete"),
        ("Ctrl + Shift + Esc", "<Control><Shift>Escape"),
        ("<Control>+", "<Control>plus"),
    ] {
        let r = env.gsctl(&["keys", "find", input]);
        assert_eq!(r.status, 4, "{input}: {}", r.stderr);
        let err = &r.error()["error"];
        assert_eq!(err["code"], "invalid_value");
        assert_eq!(err["suggestion"], suggestion, "{input}");
        assert!(err["hint"].as_str().unwrap().contains("<Super>"));

        let r = env.gsctl(&["keys", "add", "X", input, "true", "--dry-run"]);
        assert_eq!(r.status, 4, "{input}: {}", r.stderr);
        assert_eq!(r.error()["error"]["suggestion"], suggestion);
    }
    let r = env.gsctl(&["keys", "find", "banana"]);
    assert_eq!(r.status, 4);
    assert_eq!(r.error()["error"]["suggestion"], Value::Null);
    assert_eq!(env.gsctl(&["keys", "find", "<Super>banana"]).status, 4);
    assert_eq!(env.gsctl(&["keys", "list", "--custom"]).ok()["count"], 0);

    // Valid forms with any modifier spelling or key case.
    for ok in
        ["<Super>Return", "<super>return", "<Primary><Shift>Page_Up", "<Release>F2", "XF86Calculator", "<Ctrl>KP_Add"]
    {
        assert_eq!(env.gsctl(&["keys", "find", ok]).status, 0, "{ok}");
    }
}

#[test]
fn add_refuses_a_duplicate_name() {
    let env = TestEnv::new();
    env.gsctl(&["keys", "add", "Terminal", "<Super>Return", "ptyxis", "--yes"]).ok();
    let r = env.gsctl(&["keys", "add", "Terminal", "<Super>F5", "kitty", "--dry-run"]);
    assert_eq!(r.status, 7, "{}", r.stderr);
    let err = &r.error()["error"];
    assert_eq!(err["code"], "conflict");
    assert_eq!(err["existing"][0]["path"], CUSTOM0);
    assert_eq!(err["existing"][0]["accel"], "<Super>Return");
    assert_eq!(err["existing"][0]["command"], "ptyxis");
    let msg = err["message"].as_str().unwrap();
    assert!(msg.contains(CUSTOM0) && msg.contains("ptyxis"), "{msg}");

    env.gsctl(&["keys", "add", "Terminal", "<Super>F5", "kitty", "--force", "--yes"]).ok();
    assert_eq!(env.gsctl(&["keys", "list", "--custom"]).ok()["count"], 2);
}

#[test]
fn add_stores_the_trimmed_accelerator_as_typed() {
    let env = TestEnv::new();
    let out = env.gsctl(&["keys", "add", "Files", "  <Primary><super>e ", "nautilus", "--yes"]).ok();
    assert_eq!(out["accel"], "<Primary><super>e");
    assert_eq!(out["normalized"], "<Control><Super>e");
    assert_eq!(env.gsctl(&["get", CUSTOM, "binding", "--path", CUSTOM0]).ok()["value"], "<Primary><super>e");
}

#[test]
fn add_and_remove_pass_yes_through_and_report_requires_yes() {
    let env = TestEnv::new();
    let dry = env.gsctl(&["keys", "add", "Terminal", "<Super>Return", "ptyxis", "--dry-run"]).ok();
    assert!(dry["requires_yes"].is_boolean(), "{dry}");
    env.gsctl(&["keys", "add", "Terminal", "<Super>Return", "ptyxis", "--yes"]).ok();
    let dry = env.gsctl(&["keys", "remove", "Terminal", "--dry-run"]).ok();
    assert!(dry["requires_yes"].is_boolean(), "{dry}");
    env.gsctl(&["keys", "remove", "Terminal"]).ok();
    assert_eq!(env.gsctl(&["keys", "list", "--custom"]).ok()["count"], 0);
}

#[test]
fn text_output_names_bindings_for_people() {
    let env = TestEnv::new();
    let r = env.gsctl(&["--format", "text", "keys", "conflicts"]);
    assert_eq!(r.status, 0, "{}", r.stderr);
    assert_eq!(r.stdout.trim(), "no conflicts");

    env.gsctl(&["keys", "add", "Terminal", "<Super>Return", "ptyxis", "--yes"]).ok();
    let r = env.gsctl(&["keys", "add", "Other", "<Super>Return", "kitty", "--yes"]);
    assert_eq!(r.status, 7);
    let msg = r.error()["error"]["message"].as_str().unwrap().to_string();
    assert!(msg.contains(r#"custom "Terminal""#), "{msg}");
    assert!(!msg.contains(CUSTOM), "{msg}");

    let r = env.gsctl(&["--format", "text", "keys", "list"]);
    assert!(r.stdout.contains("custom 'Terminal': ptyxis"), "{}", r.stdout);
    assert!(r.stdout.contains("demo@gsctl.test take-window"), "{}", r.stdout);
    assert!(r.stdout.contains("(extension off)"), "{}", r.stdout);
}

#[test]
fn add_rolls_back_when_a_write_fails() {
    if !cfg!(debug_assertions) {
        return; // the fault hook only exists in debug builds
    }
    let env = TestEnv::new();
    let script = format!(
        r#"GSCTL_TEST_FAIL_WRITE='{MEDIA_KEYS} custom-keybindings' "$GSCTL" keys add Terminal '<Super>Return' ptyxis --yes > "$TMP/fail.out" 2> "$TMP/fail.err"; echo $? > "$TMP/fail.status""#
    );
    env.session_script(&script);
    let status: i32 = std::fs::read_to_string(env.path("fail.status")).unwrap().trim().parse().unwrap();
    let err: Value = serde_json::from_str(&std::fs::read_to_string(env.path("fail.err")).unwrap()).unwrap();
    assert_eq!(status, 1, "{err}");
    assert!(err["error"]["batch"].as_str().unwrap().starts_with("keys-add-"), "{err}");
    assert_eq!(err["error"]["rolled_back"].as_array().unwrap().len(), 3, "{err}");
    for key in ["name", "command", "binding"] {
        assert_eq!(env.gsctl(&["get", CUSTOM, key, "--path", CUSTOM0]).ok()["is_default"], true, "{key}");
    }
    assert_eq!(env.gsctl(&["get", MEDIA_KEYS, "custom-keybindings"]).ok()["is_default"], true);
}
