//! `gsctl ext` against a fake user extension (with its own schemas) and a fake
//! system extension (schema installed system-wide, like Ubuntu's).

mod common;

use common::{EXTENSION_UUID, SYSTEM_EXTENSION_UUID, TestEnv};
use serde_json::{Value, json};

const SHELL: &str = "org.gnome.shell";

fn ext<'a>(list: &'a Value, uuid: &str) -> &'a Value {
    list["extensions"].as_array().unwrap().iter().find(|e| e["uuid"] == uuid).expect(uuid)
}

#[test]
fn list_installed_extensions() {
    let env = TestEnv::new();
    let list = env.gsctl(&["ext", "list"]).ok();
    assert_eq!(list["count"], 2);
    let user = ext(&list, EXTENSION_UUID);
    assert_eq!(user["location"], "user");
    assert_eq!(user["enabled"], false);
    assert_eq!(user["schemas"], json!(["org.gnome.shell.extensions.gsctl-demo"]));
    let sys = ext(&list, SYSTEM_EXTENSION_UUID);
    assert_eq!(sys["location"], "system");
    // Found from getSettings() in its JS; the non-extension schema it reads is not its own.
    assert_eq!(sys["schemas"], json!(["org.gnome.shell.extensions.gsctl-system"]));
    assert_eq!(sys["enabled"], true);
    assert_eq!(sys["enabled_by"], "session-mode");
    assert_eq!(list["session_mode"]["name"], "testmode");
    assert_eq!(list["session_mode"]["source"], "env");
}

#[test]
fn session_mode_extensions() {
    let env = TestEnv::new();
    // Disabling a mode extension goes through disabled-extensions...
    env.gsctl(&["ext", "disable", SYSTEM_EXTENSION_UUID]).ok();
    assert_eq!(env.gsctl(&["get", SHELL, "disabled-extensions"]).ok()["value"], json!([SYSTEM_EXTENSION_UUID]));
    assert_eq!(ext(&env.gsctl(&["ext", "list"]).ok(), SYSTEM_EXTENSION_UUID)["enabled"], false);
    // ...and enabling it again only clears that, without adding it to enabled-extensions.
    env.gsctl(&["ext", "enable", SYSTEM_EXTENSION_UUID, "--yes"]).ok();
    assert_eq!(env.gsctl(&["get", SHELL, "enabled-extensions"]).ok()["is_default"], true);
    assert_eq!(ext(&env.gsctl(&["ext", "list"]).ok(), SYSTEM_EXTENSION_UUID)["enabled_by"], "session-mode");

    // disable-user-extensions turns off settings-enabled ones, not mode ones.
    env.gsctl(&["ext", "enable", EXTENSION_UUID, "--yes"]).ok();
    env.gsctl(&["set", SHELL, "disable-user-extensions", "true", "--yes"]).ok();
    let list = env.gsctl(&["ext", "list"]).ok();
    assert_eq!(ext(&list, EXTENSION_UUID)["enabled"], false);
    assert_eq!(ext(&list, SYSTEM_EXTENSION_UUID)["enabled"], true);
}

#[test]
fn enable_and_disable_edit_only_one_uuid() {
    let env = TestEnv::new();
    env.gsctl(&["set", SHELL, "enabled-extensions", r#"["other@x"]"#, "--yes"]).ok();

    let dry = env.gsctl(&["ext", "enable", EXTENSION_UUID, "--dry-run"]).ok();
    assert_eq!(dry["changed"], true);
    assert_eq!(ext(&env.gsctl(&["ext", "list"]).ok(), EXTENSION_UUID)["enabled"], false);

    // No --yes needed for a single-uuid edit of a guarded key.
    let out = env.gsctl(&["ext", "enable", EXTENSION_UUID, "--yes"]).ok();
    assert!(out["batch"].as_str().unwrap().starts_with("ext-enable-"));
    assert_eq!(env.gsctl(&["get", SHELL, "enabled-extensions"]).ok()["value"], json!(["other@x", EXTENSION_UUID]));
    assert_eq!(ext(&env.gsctl(&["ext", "list"]).ok(), EXTENSION_UUID)["enabled"], true);

    let again = env.gsctl(&["ext", "enable", EXTENSION_UUID, "--yes"]).ok();
    assert_eq!(again["changed"], false);

    env.gsctl(&["ext", "disable", EXTENSION_UUID]).ok();
    assert_eq!(env.gsctl(&["get", SHELL, "enabled-extensions"]).ok()["value"], json!(["other@x"]));
    assert_eq!(env.gsctl(&["get", SHELL, "disabled-extensions"]).ok()["value"], json!([EXTENSION_UUID]));

    // Re-enabling clears it from disabled-extensions too.
    env.gsctl(&["ext", "enable", EXTENSION_UUID, "--yes"]).ok();
    assert_eq!(env.gsctl(&["get", SHELL, "disabled-extensions"]).ok()["value"], json!([]));

    let list = env.gsctl(&["ext", "list"]).ok();
    assert_eq!(list["enabled_but_not_installed"], json!(["other@x"]));
}

#[test]
fn revert_last_undoes_ext_enable() {
    let env = TestEnv::new();
    env.gsctl(&["ext", "enable", EXTENSION_UUID, "--yes"]).ok();
    env.gsctl(&["revert", "last"]).ok();
    assert_eq!(env.gsctl(&["get", SHELL, "enabled-extensions"]).ok()["is_default"], true);
}

#[test]
fn info_includes_settings() {
    let env = TestEnv::new();
    let info = env.gsctl(&["ext", "info", EXTENSION_UUID]).ok();
    assert_eq!(info["metadata"]["name"], "Demo");
    let keys = &info["settings"][0]["keys"];
    assert_eq!(keys[0]["key"], "show-indicator");
    assert_eq!(keys[0]["value"], true);
}

#[test]
fn unknown_extensions() {
    let env = TestEnv::new();
    for args in [["ext", "info", "nope@x"], ["ext", "enable", "nope@x"]] {
        let r = env.gsctl(&args);
        assert_eq!(r.status, 3);
        assert_eq!(r.error()["error"]["code"], "extension_not_found");
    }
}

#[test]
fn reports_disable_user_extensions() {
    let env = TestEnv::new();
    env.gsctl(&["set", SHELL, "disable-user-extensions", "true", "--yes"]).ok();
    assert_eq!(env.gsctl(&["ext", "list"]).ok()["user_extensions_disabled"], true);
}

#[test]
fn enable_reports_the_real_state_when_user_extensions_are_off() {
    let env = TestEnv::new();
    env.gsctl(&["set", SHELL, "disable-user-extensions", "true", "--yes"]).ok();
    for args in [vec!["ext", "enable", EXTENSION_UUID, "--dry-run"], vec!["ext", "enable", EXTENSION_UUID, "--yes"]] {
        let out = env.gsctl(&args).ok();
        assert_eq!(out["changed"], true, "{args:?}");
        assert_eq!(out["enabled"], false, "{args:?}: disable-user-extensions keeps it off");
        assert_eq!(out["user_extensions_disabled"], true);
    }
    assert_eq!(ext(&env.gsctl(&["ext", "list"]).ok(), EXTENSION_UUID)["enabled"], false);

    // A session-mode extension is on regardless.
    let out = env.gsctl(&["ext", "enable", SYSTEM_EXTENSION_UUID, "--yes"]).ok();
    assert_eq!(out["enabled"], true);
    assert_eq!(out["enabled_by"], "session-mode");
}

#[test]
fn reverting_a_disable_needs_yes_since_it_enables() {
    let env = TestEnv::new();
    env.gsctl(&["ext", "enable", EXTENSION_UUID, "--yes"]).ok();
    env.gsctl(&["ext", "disable", EXTENSION_UUID]).ok();
    // Undoing the disable turns the extension back on: code runs again.
    let r = env.gsctl(&["revert", "last"]);
    assert_eq!(r.status, 6, "{}", r.stderr);
    assert_eq!(r.error()["error"]["code"], "guarded");
    env.gsctl(&["revert", "last", "--yes"]).ok();
    assert_eq!(ext(&env.gsctl(&["ext", "list"]).ok(), EXTENSION_UUID)["enabled"], true);
    // Undoing an enable only turns it off, so that needs no --yes.
    env.gsctl(&["revert", "last"]).ok();
    assert_eq!(ext(&env.gsctl(&["ext", "list"]).ok(), EXTENSION_UUID)["enabled"], false);
}

#[test]
fn plain_gnome_desktop_is_user_mode() {
    // Without GNOME_SHELL_SESSION_MODE, XDG_CURRENT_DESKTOP=GNOME (Fedora, Arch)
    // means the user mode, not the only installed mode file.
    let env = TestEnv::new();
    let r = env.session_script("env -u GNOME_SHELL_SESSION_MODE XDG_CURRENT_DESKTOP=GNOME \"$GSCTL\" ext list");
    let list = r.ok();
    assert_eq!(list["session_mode"]["name"], "user");
    assert_eq!(list["session_mode"]["source"], "env");
    assert_eq!(ext(&list, SYSTEM_EXTENSION_UUID)["enabled"], false);
}
