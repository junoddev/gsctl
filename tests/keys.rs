//! `gsap keys` against stand-in keybinding schemas.

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
    let out = env.gsap(&["keys", "list"]).ok();
    let keys = keys_of(&out);
    assert!(keys.contains(&format!("{WM} close")));
    assert!(keys.contains(&format!("{MEDIA_KEYS} screensaver")));
    assert!(!keys.contains(&format!("{WM} show-desktop")), "empty binding listed");
    assert!(!keys.iter().any(|k| k.ends_with("custom-keybindings")));
}

#[test]
fn find_normalizes_accelerators() {
    let env = TestEnv::new();
    let out = env.gsap(&["keys", "find", "<Mod4>H"]).ok();
    assert_eq!(out["normalized"], "<Super>h");
    assert_eq!(out["free"], false);
    assert_eq!(keys_of(&out), [format!("{WM} minimize")]);

    let out = env.gsap(&["keys", "find", "<Super>Tab"]).ok();
    assert_eq!(keys_of(&out), [format!("{WM} switch-applications")]);

    let free = env.gsap(&["keys", "find", "<Ctrl><Super>t"]).ok();
    assert_eq!(free["free"], true);
    assert_eq!(free["normalized"], "<Control><Super>t");

    assert_eq!(env.gsap(&["keys", "find", "<Banana>x"]).status, 4);
}

#[test]
fn conflicts_across_schemas() {
    let env = TestEnv::new();
    assert_eq!(env.gsap(&["keys", "conflicts"]).ok()["count"], 0);
    env.gsap(&["set", "org.gnome.shell.keybindings", "toggle-overview", r#"["<Alt>F4"]"#]).ok();
    let out = env.gsap(&["keys", "conflicts"]).ok();
    assert_eq!(out["count"], 1);
    assert_eq!(out["conflicts"][0]["accel"], "<Alt>F4");
    assert_eq!(out["conflicts"][0]["bindings"].as_array().unwrap().len(), 2);
}

#[test]
fn add_creates_a_custom_binding() {
    let env = TestEnv::new();
    let dry = env.gsap(&["keys", "add", "Terminal", "<Super>Return", "ptyxis", "--dry-run"]).ok();
    assert_eq!(dry["path"], CUSTOM0);
    assert_eq!(env.gsap(&["keys", "list", "--custom"]).ok()["count"], 0);

    let out = env.gsap(&["keys", "add", "Terminal", "<Super>Return", "ptyxis"]).ok();
    assert_eq!(out["path"], CUSTOM0);
    assert!(out["batch"].as_str().unwrap().starts_with("keys-add-"));
    let list = env.gsap(&["keys", "list", "--custom"]).ok();
    assert_eq!(list["count"], 1);
    assert_eq!(list["bindings"][0]["custom"], json!({"name": "Terminal", "command": "ptyxis"}));
    assert_eq!(list["bindings"][0]["accels"], json!(["<Super>Return"]));
    assert_eq!(env.gsap(&["get", MEDIA_KEYS, "custom-keybindings"]).ok()["value"], json!([CUSTOM0]));

    // The next one goes to custom1, and find sees the custom binding.
    let second = env.gsap(&["keys", "add", "Files", "<Super>e", "nautilus"]).ok();
    assert_eq!(second["path"], CUSTOM1);
    let found = env.gsap(&["keys", "find", "<super>return"]).ok();
    assert_eq!(found["bindings"][0]["custom"]["name"], "Terminal");
}

#[test]
fn add_refuses_taken_accelerators() {
    let env = TestEnv::new();
    let r = env.gsap(&["keys", "add", "Lock", "<Super>l", "true"]);
    assert_eq!(r.status, 7, "{}", r.stderr);
    assert!(r.error()["error"]["message"].as_str().unwrap().contains("screensaver"));
    assert_eq!(env.gsap(&["keys", "list", "--custom"]).ok()["count"], 0);

    env.gsap(&["keys", "add", "Lock", "<Super>l", "true", "--force"]).ok();
    assert_eq!(env.gsap(&["keys", "conflicts"]).ok()["count"], 1);

    assert_eq!(env.gsap(&["keys", "add", "Bad", "<Super>", "x"]).status, 4);
}

#[test]
fn remove_deletes_a_custom_binding_and_revert_brings_it_back() {
    let env = TestEnv::new();
    env.gsap(&["keys", "add", "Terminal", "<Super>Return", "ptyxis"]).ok();
    env.gsap(&["keys", "add", "Files", "<Super>e", "nautilus"]).ok();

    let dry = env.gsap(&["keys", "remove", "Terminal", "--dry-run"]).ok();
    assert_eq!(dry["path"], CUSTOM0);
    assert_eq!(env.gsap(&["keys", "list", "--custom"]).ok()["count"], 2);

    let out = env.gsap(&["keys", "remove", "Terminal"]).ok();
    assert_eq!(out["path"], CUSTOM0);
    assert_eq!(out["accel"], "<Super>Return");
    assert!(out["batch"].as_str().unwrap().starts_with("keys-remove-"));
    assert_eq!(out["writes"].as_array().unwrap().len(), 4, "{out}");
    assert_eq!(env.gsap(&["get", MEDIA_KEYS, "custom-keybindings"]).ok()["value"], json!([CUSTOM1]));
    assert_eq!(env.gsap(&["get", CUSTOM, "name", "--path", CUSTOM0]).ok()["is_default"], true);
    assert_eq!(env.gsap(&["keys", "find", "<Super>Return"]).ok()["free"], true);

    env.gsap(&["revert", "last"]).ok();
    let list = env.gsap(&["keys", "list", "--custom"]).ok();
    assert_eq!(list["count"], 2, "{list}");
    assert_eq!(env.gsap(&["get", MEDIA_KEYS, "custom-keybindings"]).ok()["value"], json!([CUSTOM0, CUSTOM1]));
    assert_eq!(env.gsap(&["get", CUSTOM, "command", "--path", CUSTOM0]).ok()["value"], "ptyxis");

    // By short path and full path too.
    env.gsap(&["keys", "remove", "custom1"]).ok();
    env.gsap(&["keys", "remove", CUSTOM0]).ok();
    assert_eq!(env.gsap(&["keys", "list", "--custom"]).ok()["count"], 0);
    // The last removal resets the list instead of pinning [].
    assert_eq!(env.gsap(&["get", MEDIA_KEYS, "custom-keybindings"]).ok()["is_default"], true);
}

#[test]
fn remove_refuses_unknown_and_ambiguous_names() {
    let env = TestEnv::new();
    let r = env.gsap(&["keys", "remove", "Nope"]);
    assert_eq!(r.status, 3, "{}", r.stderr);
    assert_eq!(r.error()["error"]["code"], "binding_not_found");

    env.gsap(&["keys", "add", "Same", "<Super>F1", "true"]).ok();
    env.gsap(&["keys", "add", "Same", "<Super>F2", "true"]).ok();
    let r = env.gsap(&["keys", "remove", "Same"]);
    assert_eq!(r.status, 2, "{}", r.stderr);
    assert!(r.error()["error"]["message"].as_str().unwrap().contains(CUSTOM1));
    assert_eq!(env.gsap(&["keys", "list", "--custom"]).ok()["count"], 2);
}

#[test]
fn revert_last_undoes_keys_add() {
    let env = TestEnv::new();
    env.gsap(&["keys", "add", "Terminal", "<Super>Return", "ptyxis"]).ok();
    env.gsap(&["revert", "last"]).ok();
    assert_eq!(env.gsap(&["keys", "list", "--custom"]).ok()["count"], 0);
    assert_eq!(env.gsap(&["get", MEDIA_KEYS, "custom-keybindings"]).ok()["is_default"], true);
    let name = env.gsap(&["get", CUSTOM, "name", "--path", CUSTOM0]).ok();
    assert_eq!(name["is_default"], true);
}
