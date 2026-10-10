//! Search quality and agent ergonomics: ranking, inactive schemas, aliases,
//! extension hits, self-sufficient hits, output size and suggestions.

mod common;

use common::{EXTENSION_UUID, TestEnv};
use serde_json::{Value, json};

const S: &str = "org.gsctl.test";

fn results(out: &Value) -> &Vec<Value> {
    out["results"].as_array().unwrap()
}

fn keys(out: &Value) -> Vec<String> {
    results(out)
        .iter()
        .map(|r| match r["kind"].as_str() {
            Some("extension") => format!("ext:{}", r["uuid"].as_str().unwrap()),
            _ => format!("{} {}", r["schema"].as_str().unwrap(), r["key"].as_str().unwrap()),
        })
        .collect()
}

#[test]
fn hits_say_which_words_matched_and_keep_partial_ones() {
    let env = TestEnv::new();
    let out = env.gsctl(&["search", "frobnicator", "refresh"]).ok();
    assert_eq!(out["partial"], false);
    let hit = &results(&out)[0];
    assert_eq!(hit["matched"], json!(["frobnicator", "refresh"]));
    assert_eq!(hit["matched_words"], 2);
    assert!(hit.get("partial").is_none());

    // Full matches exist ("idle delay" → idle-delay, size-u), and the best
    // partial ones still follow, flagged.
    let out = env.gsctl(&["search", "idle", "delay"]).ok();
    let r = results(&out);
    assert_eq!(r[0]["key"], "idle-delay");
    assert_eq!(r[1]["key"], "size-u");
    let partial: Vec<&Value> = r.iter().filter(|h| h["partial"] == true).collect();
    assert!(!partial.is_empty(), "{out}");
    assert!(partial.iter().all(|h| h["matched"].as_array().unwrap().len() == 1));
    let first_partial = r.iter().position(|h| h["partial"] == true).unwrap();
    assert!(r[first_partial..].iter().all(|h| h["partial"] == true), "partial hits come last");
}

#[test]
fn partial_hits_keep_a_few_slots_when_full_matches_fill_the_limit() {
    let env = TestEnv::new();
    // Two keys say "Unsigned ...-bit", two more only "...-bit".
    let out = env.gsctl(&["search", "unsigned", "bit", "--limit", "2"]).ok();
    let r = results(&out);
    assert_eq!(r.len(), 2);
    assert!(r[0].get("partial").is_none());
    assert_eq!(r[1]["partial"], true);
    assert_eq!(r[1]["matched"], json!(["bit"]));
}

#[test]
fn enum_choices_are_searched() {
    let env = TestEnv::new();
    let out = env.gsctl(&["search", "prefer", "dark"]).ok();
    assert_eq!(results(&out)[0]["key"], "color-scheme");
    let out = env.gsctl(&["search", "italic"]).ok();
    assert_eq!(results(&out)[0]["key"], "opts");
}

#[test]
fn hits_carry_what_set_needs() {
    let env = TestEnv::new();
    let out = env.gsctl(&["search", "widgets", "zanzibar"]).ok();
    let hit = &results(&out)[0];
    assert_eq!(hit["kind"], "key");
    assert_eq!(hit["key"], "count-i");
    assert_eq!(hit["range"], json!({"kind": "range", "min": 0, "max": 100}));
    assert_eq!(hit["default"], 10);
    assert_eq!(hit["value"], 10);
    assert_eq!(hit["guarded"], Value::Null);
    assert_eq!(hit["description"], "How many widgets to show in the zanzibar panel.");

    let out = env.gsctl(&["search", "lock", "enabled", "--schema", "org.gnome.desktop.screensaver"]).ok();
    assert!(results(&out)[0]["guarded"].is_string());

    // Long descriptions are trimmed, long values truncated with a marker.
    let out = env.gsctl(&["search", "idleness"]).ok();
    let d = results(&out)[0]["description"].as_str().unwrap();
    assert!(d.chars().count() <= 161 && d.ends_with('…'), "{d}");
    let out = env.gsctl(&["search", "gargantuan"]).ok();
    let v = &results(&out)[0]["value"];
    assert_eq!(v["truncated"], true, "{v}");
    assert!(v["chars"].as_u64().unwrap() > 300);
    assert!(v["preview"].as_str().unwrap().starts_with("[\"aaaa"));
    // `get` still returns the whole value.
    let full = env.gsctl(&["get", "org.gsctl.search", "long-list"]).ok();
    assert_eq!(full["value"].as_array().unwrap().len(), 6);
}

#[test]
fn inactive_schemas_rank_last_and_are_marked() {
    let env = TestEnv::new();
    // A full match in an Ubuntu Touch schema no longer hides the rest.
    let out = env.gsctl(&["search", "screen", "timeout"]).ok();
    let k = keys(&out);
    assert_eq!(k[0], "org.gnome.desktop.session idle-delay", "{k:?}");
    let touch = results(&out).iter().find(|h| h["schema"] == "com.ubuntu.touch.system").unwrap();
    assert_eq!(touch["inactive"], true);
    assert_eq!(results(&out).last().unwrap()["schema"], "com.ubuntu.touch.system");

    // --all ranks it like any other key.
    let out = env.gsctl(&["search", "timeout", "--all"]).ok();
    assert_eq!(results(&out)[0]["schema"], "com.ubuntu.touch.system");
    assert_eq!(results(&out)[0]["inactive"], true);
}

#[test]
fn aliases_are_added_on_top_and_marked() {
    let env = TestEnv::new();
    // idle-delay never mentions the screen or blanking; the alias adds it on top.
    let out = env.gsctl(&["search", "screen", "blanking"]).ok();
    let hit = &results(&out)[0];
    assert_eq!(hit["key"], "idle-delay");
    assert_eq!(hit["via"], "alias");
    assert!(hit["note"].as_str().unwrap().contains("0 = never"));
    assert_eq!(hit["matched"], json!([]));
    assert!(hit.get("partial").is_none(), "alias hits aren't partial matches");
    // Other matches are still there.
    assert!(keys(&out).contains(&format!("{S} size-u")), "{out}");
    // People see the note too.
    let text = env.gsctl(&["--format", "text", "search", "screen", "blanking"]);
    let first = text.stdout.lines().next().unwrap_or_default().to_string();
    assert!(first.contains("idle-delay") && first.contains("0 = never"), "{}", text.stdout);

    // Filler words are dropped and reported.
    let out = env.gsctl(&["search", "turn", "off", "the", "frobnicator"]).ok();
    assert_eq!(out["ignored_words"], json!(["turn", "off", "the"]));
    assert_eq!(results(&out)[0]["key"], "flag-b");
    assert_eq!(out["partial"], false);
}

#[test]
fn extensions_are_searchable() {
    let env = TestEnv::new();
    let out = env.gsctl(&["search", "system", "demo"]).ok();
    let hit = results(&out).iter().find(|h| h["kind"] == "extension").expect("an extension hit");
    assert_eq!(hit["uuid"], "system@gsctl.test");
    assert_eq!(hit["name"], "System Demo");
    assert_eq!(hit["enabled"], true);
    assert_eq!(hit["enabled_by"], "session-mode");
    assert!(hit["hint"].as_str().unwrap().contains("gsctl ext"));

    let out = env.gsctl(&["search", EXTENSION_UUID]).ok();
    assert_eq!(keys(&out)[0], format!("ext:{EXTENSION_UUID}"));
    // A --schema filter is about schemas, so extensions are left out.
    let out = env.gsctl(&["search", "demo", "--schema", "org.gnome.shell"]).ok();
    assert!(results(&out).iter().all(|h| h["kind"] == "key"));
}

#[test]
fn json_is_compact_when_piped_unless_asked() {
    let env = TestEnv::new();
    let r = env.gsctl(&["get", S, "count-i"]);
    assert_eq!(r.stdout.lines().count(), 1, "{}", r.stdout);
    let r = env.gsctl(&["get", S, "count-i", "--pretty"]);
    assert!(r.stdout.lines().count() > 1);
    assert_eq!(r.json()["value"], 10);
    let out = env
        .command(env!("CARGO_BIN_EXE_gsctl"))
        .args(["get", S, "count-i"])
        .env("GSCTL_JSON", "pretty")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).lines().count() > 1);
    let r = env.gsctl(&["get", S, "count-i", "--pretty", "--compact"]);
    assert_eq!(r.status, 2);
}

#[test]
fn schema_lists_keys_briefly_unless_full() {
    let env = TestEnv::new();
    let brief = env.gsctl(&["schema", S]).ok();
    let k = brief["keys"].as_array().unwrap().iter().find(|k| k["key"] == "count-i").unwrap();
    assert_eq!(
        k,
        &json!({"key": "count-i", "type": "i", "value": 10, "is_default": true, "summary": "Number of widgets"})
    );
    assert!(brief["detail"].is_string());

    let full = env.gsctl(&["schema", S, "--full"]).ok();
    let k = full["keys"].as_array().unwrap().iter().find(|k| k["key"] == "count-i").unwrap();
    assert_eq!(k["range"]["max"], 100);
    assert!(full.get("detail").is_none());
    assert!(env.gsctl(&["schema", S]).stdout.len() * 2 < env.gsctl(&["schema", S, "--full"]).stdout.len());

    let guarded = env.gsctl(&["schema", "org.gnome.desktop.screensaver"]).ok();
    let k = guarded["keys"].as_array().unwrap().iter().find(|k| k["key"] == "lock-enabled").unwrap();
    assert!(k["guarded"].is_string());

    let list = env.gsctl(&["schema"]).ok();
    let touch = list["schemas"].as_array().unwrap().iter().find(|s| s["id"] == "com.ubuntu.touch.system").unwrap();
    assert_eq!(touch["inactive"], true);
}

#[test]
fn one_argument_schema_key_and_dconf_path_forms() {
    let env = TestEnv::new();
    assert_eq!(env.gsctl(&["get", "org.gsctl.test.count-i"]).ok()["value"], 10);
    assert_eq!(env.gsctl(&["get", "/org/gsctl/test/count-i"]).ok()["key"], "count-i");
    let k = env.gsctl(&["schema", "org.gsctl.test.count-i"]).ok();
    assert_eq!(k["range"]["max"], 100);
    assert_eq!(env.gsctl(&["schema", "/org/gsctl/test/mode"]).ok()["key"], "mode");

    let out = env.gsctl(&["set", "org.gsctl.test.count-i", "12"]).ok();
    assert_eq!(out["new"], 12);
    env.gsctl(&["set", "/org/gsctl/test/short-n", "-3"]).ok();
    assert_eq!(env.gsctl(&["get", S, "short-n"]).ok()["value"], -3);
    env.gsctl(&["reset", "org.gsctl.test.count-i"]).ok();
    assert_eq!(env.gsctl(&["get", S, "count-i"]).ok()["is_default"], true);
    // The two-argument form is unchanged.
    assert_eq!(env.gsctl(&["get", S, "count-i"]).ok()["value"], 10);

    // A listed relocatable instance, by its dconf path.
    env.gsctl(&["keys", "add", "Term", "<Super>t", "ptyxis", "--yes"]).ok();
    let p = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/";
    let got = env.gsctl(&["get", &format!("{p}binding")]).ok();
    assert_eq!(got["value"], "<Super>t");
    assert_eq!(got["path"], p);

    // A schema without a key, or a path that isn't a key, is a usage error.
    assert_eq!(env.gsctl(&["get", S]).status, 2);
    assert_eq!(env.gsctl(&["get", "/org/nowhere/x"]).status, 2);
    assert_eq!(env.gsctl(&["set", S, "count-i"]).status, 2);
}

#[test]
fn not_found_errors_suggest_names() {
    let env = TestEnv::new();
    let e = env.gsctl(&["get", S, "count"]).error()["error"].clone();
    assert_eq!(e["code"], "key_not_found");
    assert!(e["did_you_mean"].as_array().unwrap().contains(&json!("count-i")), "{e}");
    assert_eq!(e["hint"], format!("try `gsctl search count --schema {S}`"));

    let e = env.gsctl(&["get", "org.gsctl.tset", "count-i"]).error()["error"].clone();
    assert_eq!(e["code"], "schema_not_found");
    assert_eq!(e["exit_code"], 3);
    assert_eq!(e["did_you_mean"][0], "org.gsctl.test", "{e}");

    // A key that lives in another schema.
    let e = env.gsctl(&["get", S, "idle-delay"]).error()["error"].clone();
    assert_eq!(e["elsewhere"], json!(["org.gnome.desktop.session"]), "{e}");

    let e = env.gsctl(&["ext", "info", "demo"]).error()["error"].clone();
    assert_eq!(e["code"], "extension_not_found");
    assert!(e["did_you_mean"].as_array().unwrap().contains(&json!(EXTENSION_UUID)), "{e}");

    // The one-argument form reports the same errors.
    let e = env.gsctl(&["get", "org.gsctl.test.count"]).error()["error"].clone();
    assert_eq!(e["code"], "key_not_found");
}

#[test]
fn changed_hides_app_state_unless_asked() {
    let env = TestEnv::new();
    env.gsctl(&["set", S, "count-i", "11"]).ok();
    env.gsctl(&["set", S, "launch-count", "7"]).ok();
    let out = env.gsctl(&["changed"]).ok();
    assert_eq!(out["count"], 1, "{out}");
    assert_eq!(out["hidden_state"], 1);
    let out = env.gsctl(&["changed", "--include-state"]).ok();
    assert_eq!(out["count"], 2);
    let state = out["changed"].as_array().unwrap().iter().find(|c| c["key"] == "launch-count").unwrap();
    assert_eq!(state["state"], "timestamp or counter");
    assert!(out.get("hidden_state").is_none());
}

#[test]
fn commands_json_describes_the_cli() {
    let env = TestEnv::new();
    let out = env.gsctl(&["commands"]).ok();
    assert_eq!(out, env.gsctl(&["help", "--json"]).ok());
    let subs = out["subcommands"].as_array().unwrap();
    let set = subs.iter().find(|c| c["name"] == "set").unwrap();
    let args = set["args"].as_array().unwrap();
    assert!(args.iter().any(|a| a["name"] == "dry_run" && a["kind"] == "flag" && a["long"] == "--dry-run"));
    assert!(args.iter().any(|a| a["name"] == "schema" && a["kind"] == "positional" && a["required"] == true));
    let search = subs.iter().find(|c| c["name"] == "search").unwrap();
    let limit = search["args"].as_array().unwrap().iter().find(|a| a["name"] == "limit").unwrap();
    assert_eq!(limit["default"], "20");
    let ext = subs.iter().find(|c| c["name"] == "ext").unwrap();
    assert!(ext["subcommands"].as_array().unwrap().iter().any(|c| c["name"] == "enable"));
    let globals = out["global_args"].as_array().unwrap();
    let format = globals.iter().find(|a| a["name"] == "format").unwrap();
    assert_eq!(format["values"], json!(["json", "text"]));
    assert_eq!(out["error_codes"]["guarded"]["exit"], 6);
    assert_eq!(out["exit_codes"]["3"].as_array().unwrap().len(), 5);
}
