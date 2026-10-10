//! The dotfiles loop: `export --out DIR`, then `apply DIR`; and the remote
//! area, which `export` leaves out unless asked.

mod common;

use common::TestEnv;
use serde_json::json;

const S: &str = "org.gsctl.test";
const RDP: &str = "org.gnome.desktop.remote-desktop.rdp";

#[test]
fn apply_accepts_the_export_directory() {
    let source = TestEnv::new();
    source.gsctl(&["set", S, "count-i", "42"]).ok();
    source.gsctl(&["set", "org.gnome.desktop.screensaver", "logout-delay", "30"]).ok();
    let dir = source.path("dotfiles/gnome");
    let d = dir.to_str().unwrap();
    source.gsctl(&["export", "--out", d]).ok();
    // Other files in the directory are ignored.
    std::fs::write(dir.join("README.md"), "notes").unwrap();

    let target = TestEnv::new();
    let dry = target.gsctl(&["apply", d, "--dry-run"]).ok();
    let keys: Vec<&str> = dry["changes"].as_array().unwrap().iter().map(|c| c["key"].as_str().unwrap()).collect();
    // Files are read in name order: other.json, then power.json.
    assert_eq!(keys, ["count-i", "logout-delay"]);
    target.gsctl(&["apply", d]).ok();
    assert_eq!(target.gsctl(&["get", S, "count-i"]).ok()["value"], 42);
    assert_eq!(target.gsctl(&["get", "org.gnome.desktop.screensaver", "logout-delay"]).ok()["value"], 30);

    let empty = target.path("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let r = target.gsctl(&["apply", empty.to_str().unwrap()]);
    assert_eq!(r.status, 2);
    assert!(r.error()["error"]["message"].as_str().unwrap().contains("no .json files"), "{}", r.stderr);
}

#[test]
fn export_leaves_out_remote_unless_asked() {
    let env = TestEnv::new();
    env.gsctl(&["set", RDP, "enable", "true", "--yes"]).ok();
    env.gsctl(&["set", S, "count-i", "42"]).ok();

    let doc = env.gsctl(&["export"]).ok();
    assert!(doc["settings"].get(RDP).is_none(), "{doc}");
    assert_eq!(doc["skipped_areas"], json!({"remote": 1}));
    assert_eq!(doc["area"], json!(["other"]));
    let text = env.gsctl(&["export", "--format", "text"]);
    assert!(text.stdout.contains("(left out the remote area, 1 key; --include-remote exports it)"), "{}", text.stdout);

    let with = env.gsctl(&["export", "--include-remote"]).ok();
    assert_eq!(with["settings"][RDP]["enable"], true);
    assert!(with.get("skipped_areas").is_none());
    let only = env.gsctl(&["export", "--area", "remote"]).ok();
    assert_eq!(only["settings"], json!({ RDP: {"enable": true} }));

    let dir = env.path("out");
    let out = env.gsctl(&["export", "--out", dir.to_str().unwrap()]).ok();
    assert_eq!(out["skipped_areas"], json!({"remote": 1}));
    assert!(!dir.join("remote.json").exists());
}
