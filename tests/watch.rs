//! `gsctl watch`: the watcher and the writers share one private D-Bus session.

mod common;

use common::TestEnv;
use serde_json::Value;

/// Start `watch ARGS` in the background, wait for its ready event, run the
/// `then` commands, and return the watcher's NDJSON events.
fn watch_while(env: &TestEnv, args: &str, then: &[&str]) -> Vec<Value> {
    let mut script = format!(
        r#"set -e
"$GSCTL" watch {args} > "$TMP/watch.out" &
pid=$!
for i in $(seq 200); do grep -q '"ready"' "$TMP/watch.out" 2>/dev/null && break; sleep 0.05; done
"#
    );
    for cmd in then {
        script.push_str(&format!("\"$GSCTL\" {cmd} > /dev/null\n"));
    }
    script.push_str("wait $pid\n");
    let run = env.session_script(&script);
    assert_eq!(run.status, 0, "script failed:\n{}", run.stderr);
    std::fs::read_to_string(env.path("watch.out"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad NDJSON line {l:?}: {e}")))
        .collect()
}

#[test]
fn streams_changes_as_schema_and_key() {
    let env = TestEnv::new();
    let events = watch_while(
        &env,
        "--count 2 --timeout 20",
        &["set org.gsctl.test count-i 11", "set org.gnome.desktop.screensaver logout-delay 7"],
    );
    assert_eq!(events[0]["event"], "ready");
    assert!(events[0]["schemas"].as_u64().unwrap() > 3);
    let changes: Vec<(&str, &str, &Value)> =
        events[1..].iter().map(|e| (e["schema"].as_str().unwrap(), e["key"].as_str().unwrap(), &e["value"])).collect();
    assert_eq!(
        changes,
        [
            ("org.gsctl.test", "count-i", &Value::from(11)),
            ("org.gnome.desktop.screensaver", "logout-delay", &Value::from(7))
        ]
    );
    assert_eq!(events[1]["event"], "changed");
    assert_eq!(events[1]["is_default"], false);
}

#[test]
fn secrets_are_redacted_unless_asked() {
    let env = TestEnv::new();
    let set = "set org.gnome.system.proxy.http authentication-password hunter2 --yes";
    let events = watch_while(&env, "--schema org.gnome.system.proxy --count 1 --timeout 20", &[set]);
    assert_eq!(events[1]["value"], serde_json::json!({"redacted": true}), "{events:?}");
    let set = "set org.gnome.system.proxy.http authentication-password swordfish --yes";
    let events = watch_while(&env, "--schema org.gnome.system.proxy --count 1 --timeout 20 --show-secrets", &[set]);
    assert_eq!(events[1]["value"], "swordfish");
}

#[test]
fn schema_filter() {
    let env = TestEnv::new();
    let events = watch_while(
        &env,
        "--schema org.gnome.desktop --count 1 --timeout 20",
        &["set org.gsctl.test count-i 11", "set org.gnome.desktop.screensaver logout-delay 7"],
    );
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["key"], "logout-delay");
}

#[test]
fn reports_resets_and_custom_keybindings() {
    let env = TestEnv::new();
    env.gsctl(&["keys", "add", "Terminal", "<Super>Return", "ptyxis", "--yes"]).ok();
    let events = watch_while(
        &env,
        "--count 1 --timeout 20 --schema org.gnome.settings-daemon.plugins.media-keys.custom-keybinding",
        &[
            r#"set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding binding '<Super>t' --path /org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/"#,
        ],
    );
    assert_eq!(events[1]["path"], "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/");
    assert_eq!(events[1]["value"], "<Super>t");

    let env = TestEnv::new();
    env.gsctl(&["set", "org.gsctl.test", "name-s", "X"]).ok();
    let events = watch_while(&env, "--count 1 --timeout 20", &["reset org.gsctl.test name-s"]);
    assert_eq!(events[1]["value"], "Adwaita");
    assert_eq!(events[1]["is_default"], true);
}

#[test]
fn count_is_never_exceeded_by_multi_key_changes() {
    let env = TestEnv::new();
    let doc = serde_json::json!({"gsctl": 1, "settings": {
        "org.gsctl.test": { "count-i": 50, "name-s": "X", "size-u": 600, "flag-b": true }
    }});
    let file = env.path("multi.json");
    std::fs::write(&file, doc.to_string()).unwrap();
    for count in [1, 2] {
        let events =
            watch_while(&env, &format!("--count {count} --timeout 20"), &[&format!("apply {}", file.display())]);
        assert_eq!(events.len(), count + 1, "{events:?}");
        env.gsctl(&["revert", "last"]).ok();
    }
}

#[test]
fn huge_timeouts_do_not_wrap() {
    let env = TestEnv::new();
    // 4294968 s is 4294968000 ms, which wrapped to 704 ms as a u32.
    let script = r#"set -e
"$GSCTL" watch --count 1 --timeout 4294968 > "$TMP/watch.out" &
pid=$!
for i in $(seq 200); do grep -q '"ready"' "$TMP/watch.out" 2>/dev/null && break; sleep 0.05; done
sleep 1.5
"$GSCTL" set org.gsctl.test count-i 11 > /dev/null
wait $pid
"#;
    let run = env.session_script(script);
    assert_eq!(run.status, 0, "{}", run.stderr);
    let out = std::fs::read_to_string(env.path("watch.out")).unwrap();
    let kinds: Vec<String> =
        out.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["event"].as_str().unwrap().to_string()).collect();
    assert_eq!(kinds, ["ready", "changed"]);

    // Beyond u32 seconds is refused up front.
    let r = env.gsctl(&["watch", "--timeout", "4294967296"]);
    assert_eq!(r.status, 2, "{}{}", r.stdout, r.stderr);
    assert_eq!(r.error()["error"]["code"], "usage");
}

#[test]
fn timeout_ends_the_stream() {
    let env = TestEnv::new();
    let events = watch_while(&env, "--timeout 1", &[]);
    let kinds: Vec<&str> = events.iter().map(|e| e["event"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["ready", "timeout"]);
}
