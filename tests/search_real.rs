//! Search-quality harness against the real installed schemas and extensions.
//! Agent-phrased queries, each with the key (or extension) it should find in
//! the top 5. Read-only: it only runs `gsctl search`.
//!
//! Opt in with: `cargo test --release --test search_real -- --ignored --nocapture`
//! (prints a table of ranks; fails if any expected hit is outside the top 5).

use std::process::Command;

use serde_json::Value;

enum Want {
    /// `(schema, key)`; `schema` may be `None` when the key name is unique enough.
    Key(Option<&'static str>, &'static str),
    Ext(&'static str),
}
use Want::*;

const CASES: &[(&str, Want)] = &[
    ("dark mode", Key(Some("org.gnome.desktop.interface"), "color-scheme")),
    ("screen timeout", Key(Some("org.gnome.desktop.session"), "idle-delay")),
    ("screen blanking", Key(Some("org.gnome.desktop.session"), "idle-delay")),
    ("caps lock", Key(Some("org.gnome.desktop.input-sources"), "xkb-options")),
    ("remap caps lock", Key(Some("org.gnome.desktop.input-sources"), "xkb-options")),
    ("window buttons", Key(Some("org.gnome.desktop.wm.preferences"), "button-layout")),
    ("screen lock", Key(Some("org.gnome.desktop.screensaver"), "lock-enabled")),
    ("wallpaper", Key(Some("org.gnome.desktop.background"), "picture-uri")),
    ("desktop icons", Ext("ding@rastersoft.com")),
    ("dock size", Key(None, "dash-max-icon-size")),
    ("night mode", Key(None, "night-light-enabled")),
    ("natural scrolling", Key(Some("org.gnome.desktop.peripherals.touchpad"), "natural-scroll")),
    ("clock seconds", Key(None, "clock-show-seconds")),
    ("hot corner", Key(None, "enable-hot-corners")),
    ("font size", Key(Some("org.gnome.desktop.interface"), "text-scaling-factor")),
    ("battery percentage", Key(None, "show-battery-percentage")),
    ("turn off animations", Key(None, "enable-animations")),
];

/// Extra keys that should at least be visible (in the default 20 results).
const VISIBLE: &[(&str, &str, &str)] = &[("font size", "org.gnome.desktop.interface", "font-name")];

fn search(query: &str) -> Vec<Value> {
    let mut args = vec!["search"];
    args.extend(query.split_whitespace());
    args.extend(["--limit", "50"]);
    let out = Command::new(env!("CARGO_BIN_EXE_gsctl")).args(&args).output().expect("run gsctl");
    assert!(out.status.success(), "gsctl {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).expect("JSON output");
    v["results"].as_array().cloned().unwrap_or_default()
}

fn is(hit: &Value, want: &Want) -> bool {
    match want {
        Key(schema, key) => hit["key"] == *key && schema.is_none_or(|s| hit["schema"] == s),
        Ext(uuid) => hit["kind"] == "extension" && hit["uuid"] == *uuid,
    }
}

fn label(hit: &Value) -> String {
    match hit["kind"].as_str() {
        Some("extension") => format!("ext {}", hit["uuid"].as_str().unwrap_or("?")),
        _ => format!("{} {}", hit["schema"].as_str().unwrap_or("?"), hit["key"].as_str().unwrap_or("?")),
    }
}

#[test]
#[ignore = "reads the real installed schemas; run with --ignored"]
fn agent_queries_find_the_right_key() {
    let mut failures = Vec::new();
    println!("\n| query | rank | first hit |\n|---|---|---|");
    for (query, want) in CASES {
        let hits = search(query);
        let rank = hits.iter().position(|h| is(h, want)).map(|i| i + 1);
        let first = hits.first().map(label).unwrap_or_default();
        let shown = rank.map_or("miss".to_string(), |r| r.to_string());
        println!("| {query} | {shown} | {first} |");
        if rank.is_none_or(|r| r > 5) {
            failures.push(format!("{query}: rank {shown}"));
        }
    }
    for (query, schema, key) in VISIBLE {
        let hits = search(query);
        let rank = hits.iter().position(|h| h["schema"] == *schema && h["key"] == *key).map(|i| i + 1);
        println!("| {query} (also {key}) | {} | |", rank.map_or("miss".to_string(), |r| r.to_string()));
        if rank.is_none_or(|r| r > 20) {
            failures.push(format!("{query}: {key} not in the default results"));
        }
    }
    assert!(failures.is_empty(), "outside the top 5: {failures:#?}");
}
