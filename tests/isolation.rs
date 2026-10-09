//! Proves the test harness cannot touch the real session bus or dconf database.

mod common;

use common::TestEnv;

fn env_of_child(env: &TestEnv) -> Vec<(String, String)> {
    let out = env.command("/usr/bin/env").output().expect("run env");
    assert!(out.status.success());
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
        .collect()
}

#[test]
fn child_environment_is_private() {
    let env = TestEnv::new();
    let vars = env_of_child(&env);
    let get = |k: &str| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());

    // A private bus, never the real one.
    let bus = get("DBUS_SESSION_BUS_ADDRESS").expect("dbus-run-session sets a bus address");
    if let Ok(real) = std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        assert_ne!(bus, real);
    }
    assert!(!bus.contains("/run/user/"), "bus address points at the user's runtime dir: {bus}");

    // Everything that locates settings points into the temp dir.
    let tmp = env.path("");
    let tmp = tmp.to_str().unwrap();
    for key in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_DATA_HOME",
        "XDG_DATA_DIRS",
        "XDG_RUNTIME_DIR",
        "DCONF_PROFILE",
        "GSETTINGS_SCHEMA_DIR",
    ] {
        let v = get(key).unwrap_or_else(|| panic!("{key} not set"));
        assert!(v.starts_with(tmp), "{key}={v} is outside {tmp}");
    }
    assert_eq!(get("GSETTINGS_BACKEND").as_deref(), Some("dconf"));

    // Nothing else leaks through from the developer's session.
    let real_home = std::env::var("HOME").unwrap();
    for (k, v) in &vars {
        assert!(!v.contains(&real_home) || v.starts_with(tmp), "{k}={v} mentions the real HOME");
    }
}

#[test]
fn writes_land_in_the_temp_dconf_database() {
    let env = TestEnv::new();
    assert!(!env.user_db().exists());

    env.gsctl(&["set", "org.gsctl.test", "name-s", "Isolated"]).ok();

    // dconf-service on the private bus created the temp database...
    let db = std::fs::read(env.user_db()).expect("temp dconf database exists");
    assert!(db.windows(8).any(|w| w == b"Isolated"), "value not found in temp database");

    // ...and a fresh D-Bus session reads it back, so this is the real dconf
    // backend persisting to disk, not GSettings' in-memory fallback.
    let got = env.gsctl(&["get", "org.gsctl.test", "name-s"]).ok();
    assert_eq!(got["value"], "Isolated");
}

#[test]
fn only_test_schemas_are_visible() {
    let env = TestEnv::new();
    let list = env.gsctl(&["schema"]).ok();
    let ids: Vec<&str> = list["schemas"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"org.gsctl.test"));
    assert!(ids.contains(&"org.gnome.shell.extensions.gsctl-demo"));
    assert!(!ids.contains(&"org.gnome.desktop.interface"), "real system schemas leaked in: {ids:?}");
}
