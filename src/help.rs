//! `gsctl commands` (`gsctl help --json`): the command line as JSON, built
//! from clap's command tree so it can't drift from the real parser.

use clap::{Arg, ArgAction, Command};
use serde_json::{Value, json};

use crate::error::CODES;

/// `cmd` and its subcommands, recursively.
pub fn tree(cmd: &Command) -> Value {
    node(cmd, true)
}

fn node(cmd: &Command, root: bool) -> Value {
    let args: Vec<Value> = cmd
        .get_arguments()
        .filter(|a| !matches!(a.get_id().as_str(), "help" | "version") && !a.is_global_set())
        .map(arg)
        .collect();
    let mut out = json!({
        "name": cmd.get_name(),
        "about": cmd.get_about().map(|s| s.to_string()),
        "args": args,
    });
    if root {
        // The root: global options apply to every command.
        let globals: Vec<Value> = cmd.get_arguments().filter(|a| a.is_global_set()).map(arg).collect();
        out["global_args"] = globals.into();
        out["version"] = cmd.get_version().into();
    }
    let subs: Vec<Value> = cmd.get_subcommands().filter(|s| s.get_name() != "help").map(|s| node(s, false)).collect();
    if !subs.is_empty() {
        out["subcommands"] = subs.into();
    }
    out
}

fn arg(a: &Arg) -> Value {
    let positional = a.is_positional();
    let takes_value = !matches!(a.get_action(), ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count);
    let mut out = json!({
        "name": a.get_id().as_str(),
        "kind": if positional { "positional" } else if takes_value { "option" } else { "flag" },
        "help": a.get_help().map(|s| s.to_string()),
        "required": a.is_required_set(),
    });
    if !positional {
        out["long"] = a.get_long().map(|l| format!("--{l}")).into();
    }
    if matches!(a.get_action(), ArgAction::Append) || a.get_num_args().is_some_and(|n| n.max_values() > 1) {
        out["repeatable"] = true.into();
    }
    let defaults: Vec<String> = a.get_default_values().iter().map(|v| v.to_string_lossy().into_owned()).collect();
    if takes_value && !defaults.is_empty() {
        out["default"] = defaults.join(",").into();
    }
    let values: Vec<String> = a.get_possible_values().iter().map(|v| v.get_name().to_string()).collect();
    if !values.is_empty() {
        out["values"] = values.into();
    }
    out
}

/// Exit status → meaning.
pub fn exit_codes() -> Value {
    let mut out = serde_json::Map::new();
    out.insert("0".into(), "ok".into());
    for (code, exit, _) in CODES {
        let entry = out.entry(exit.to_string()).or_insert_with(|| json!([]));
        entry.as_array_mut().unwrap().push((*code).into());
    }
    out.into()
}

/// Error code → exit status and meaning.
pub fn error_codes() -> Value {
    CODES
        .iter()
        .map(|(code, exit, meaning)| ((*code).to_string(), json!({ "exit": exit, "meaning": meaning })))
        .collect::<serde_json::Map<_, _>>()
        .into()
}
