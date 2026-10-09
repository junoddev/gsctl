//! Read-only smoke test against the real installed schemas and the real
//! session. Only runs `search`, `schema` and `changed`, which never write.
//! Opt in with: `cargo test --test smoke_real -- --ignored`

use std::process::Command;

use serde_json::Value;

fn gsap(args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_gsap")).args(args).output().expect("run gsap");
    assert!(out.status.success(), "gsap {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("JSON output")
}

#[test]
#[ignore = "reads the real GNOME session; run with --ignored"]
fn read_only_commands_work_on_real_schemas() {
    let found = gsap(&["search", "workspace"]);
    let keys: Vec<&str> = found["results"].as_array().unwrap().iter().map(|r| r["key"].as_str().unwrap()).collect();
    assert!(keys.contains(&"dynamic-workspaces"), "{keys:?}");

    let key = gsap(&["schema", "org.gnome.mutter", "dynamic-workspaces"]);
    assert_eq!(key["type"], "b");
    assert!(key["value"].is_boolean());

    let changed = gsap(&["changed"]);
    assert!(changed["count"].is_u64());
}
