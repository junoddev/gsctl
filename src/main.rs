use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use gsctl::commands::{self, WriteOpts};
use gsctl::doctor;
use gsctl::error::{Error, Result};
use gsctl::ext;
use gsctl::index::Index;
use gsctl::keys::{self, AddOpts};
use gsctl::state::{self, ApplyOpts, ExportOpts};
use gsctl::watch::{self, WatchOpts};
use serde_json::Value;

/// A discoverable, validated JSON interface to GNOME settings (GSettings/dconf).
///
/// Typical loop: `gsctl search <words>` → `gsctl schema <schema> <key>` →
/// `gsctl set <schema> <key> <value> --dry-run` → `gsctl set ...` → `gsctl get ...`.
///
/// Output is JSON on stdout; errors are JSON on stderr. Exit codes: 0 ok,
/// 1 internal error, 2 usage, 3 schema or key not found, 4 invalid value,
/// 5 not writable, 6 guarded key (needs --yes), 7 conflict (revert of a key
/// changed since; needs --force).
#[derive(Parser)]
#[command(name = "gsctl", version, max_term_width = 100)]
struct Cli {
    /// Output format.
    #[arg(long, global = true, value_enum, default_value_t = Format::Json)]
    format: Format,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Json,
    Text,
}

#[derive(Subcommand)]
enum Command {
    /// Ranked search over schema ids, key names, summaries and descriptions.
    Search {
        /// Words to search for; every word must match.
        #[arg(required = true)]
        query: Vec<String>,
        /// Maximum number of results.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Only search this schema and the schemas below it (repeatable).
        #[arg(long)]
        schema: Vec<String>,
    },
    /// Describe a key (type, default, current value, allowed values), a whole
    /// schema, or list every schema when called without arguments.
    Schema {
        schema: Option<String>,
        key: Option<String>,
        /// Instance path for a relocatable schema, e.g. /org/gnome/.../custom0/.
        #[arg(long)]
        path: Option<String>,
    },
    /// Print a key's current value.
    Get {
        schema: String,
        key: String,
        /// Instance path for a relocatable schema.
        #[arg(long)]
        path: Option<String>,
    },
    /// Validate and write a value. VALUE is JSON (true, 42, "text", ["a","b"]),
    /// GVariant text (@as [], ('xkb', 'us')), or bare text for string keys.
    Set {
        schema: String,
        key: String,
        #[arg(allow_hyphen_values = true)]
        value: String,
        /// Instance path for a relocatable schema.
        #[arg(long)]
        path: Option<String>,
        /// Validate and show the change without writing it.
        #[arg(long)]
        dry_run: bool,
        /// Confirm a change to a guarded key.
        #[arg(long)]
        yes: bool,
    },
    /// Reset a key to its default.
    Reset {
        schema: String,
        key: String,
        /// Instance path for a relocatable schema.
        #[arg(long)]
        path: Option<String>,
        /// Show the change without writing it.
        #[arg(long)]
        dry_run: bool,
        /// Confirm a change to a guarded key.
        #[arg(long)]
        yes: bool,
    },
    /// Export customised settings as JSON state files, one per area. Prints a
    /// single document, or writes <area>.json files with --out. Machine state
    /// (counters, timestamps, migration markers) is skipped unless
    /// --include-state.
    Export {
        /// Only these areas (repeatable); see `gsctl areas`.
        #[arg(long)]
        area: Vec<String>,
        /// Write one <area>.json per area into this directory.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Also export keys that look like machine or app state.
        #[arg(long)]
        include_state: bool,
    },
    /// Apply state files from `export`. Validates every value first and writes
    /// nothing if any is invalid; only keys that differ are written, as one
    /// journal batch (`gsctl revert <batch>` undoes it).
    Apply {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Only keys in these areas (repeatable).
        #[arg(long)]
        area: Vec<String>,
        /// Show what would change without writing.
        #[arg(long)]
        dry_run: bool,
        /// Confirm changes to guarded keys.
        #[arg(long)]
        yes: bool,
        /// Also reset customised keys in the files' areas that the files don't
        /// list, making them the whole state of those areas. App state is
        /// never reset. Check with --dry-run first.
        #[arg(long)]
        prune: bool,
    },
    /// Stream changes as NDJSON: a "ready" event, then one "changed" event per
    /// key change (schema, key, value). Runs until interrupted, --count or
    /// --timeout.
    Watch {
        /// Only this schema and the schemas below it (repeatable).
        #[arg(long)]
        schema: Vec<String>,
        /// Exit after this many changes.
        #[arg(long)]
        count: Option<usize>,
        /// Exit after this many seconds.
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Check that gsctl can see and change this session's real settings
    /// (session bus, dconf, sandboxing). Run it first in a new environment.
    Doctor,
    /// List the export areas and the schemas each covers.
    Areas,
    /// GNOME Shell extensions: installed, enabled, and their settings.
    #[command(subcommand)]
    Ext(ExtCommand),
    /// Keyboard shortcuts across all keybinding schemas and custom bindings.
    #[command(subcommand)]
    Keys(KeysCommand),
    /// List recorded changes (set, reset, revert, apply), newest first.
    History {
        /// Maximum number of entries.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Only this schema and the schemas below it (repeatable).
        #[arg(long)]
        schema: Vec<String>,
        /// Only this key.
        #[arg(long)]
        key: Option<String>,
    },
    /// Undo a recorded change: restore the previous value, or reset the key if
    /// it was at its default. ID is an entry or batch id from `gsctl history`,
    /// or `last` (the newest entry, or its whole batch).
    Revert {
        id: String,
        /// Revert even if the key has changed since the entry.
        #[arg(long)]
        force: bool,
        /// Show the change without writing it.
        #[arg(long)]
        dry_run: bool,
        /// Confirm a change to a guarded key.
        #[arg(long)]
        yes: bool,
    },
    /// List keys whose value differs from the default: what was customised.
    Changed {
        /// Only this schema and the schemas below it (repeatable).
        #[arg(long)]
        schema: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ExtCommand {
    /// Installed extensions and whether each is enabled in settings.
    List,
    /// One extension's metadata and settings (keys with current values).
    Info { uuid: String },
    /// Enable an installed extension (adds just its uuid). Its code runs
    /// inside GNOME Shell, so this needs --yes.
    Enable {
        uuid: String,
        #[arg(long)]
        dry_run: bool,
        /// Confirm running the extension's code.
        #[arg(long)]
        yes: bool,
    },
    /// Disable an extension (removes just its uuid; no --yes needed).
    Disable {
        uuid: String,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum KeysCommand {
    /// What is bound to ACCEL? e.g. '<Super>t'. Aliases and modifier order
    /// don't matter (<Ctrl> = <Primary> = <Control>).
    Find { accel: String },
    /// Every binding that has an accelerator.
    List {
        /// Only custom keybindings.
        #[arg(long)]
        custom: bool,
    },
    /// Accelerators bound to more than one action.
    Conflicts,
    /// Add a custom keybinding that runs COMMAND. Refuses an accelerator
    /// that's already bound unless --force. Needs --yes, since it sets up a
    /// command to run.
    Add {
        name: String,
        accel: String,
        command: String,
        /// Add even if ACCEL is already bound.
        #[arg(long)]
        force: bool,
        /// Show the writes without making them.
        #[arg(long)]
        dry_run: bool,
        /// Confirm adding a binding that runs COMMAND.
        #[arg(long)]
        yes: bool,
    },
    /// Remove a custom keybinding, by name, path or `customN`. `gsctl revert
    /// last` brings it back.
    Remove {
        binding: String,
        /// Show the writes without making them.
        #[arg(long)]
        dry_run: bool,
    },
}

/// The JSON document to print, or `None` for streaming commands that print
/// their own output.
fn run(cli: &Cli) -> Result<Option<Value>> {
    let index = Index::load()?;
    if let Command::Watch { schema, count, timeout } = &cli.command {
        return watch::watch(&index, &WatchOpts { prefixes: schema, count: *count, timeout: *timeout });
    }
    match &cli.command {
        Command::Search { query, limit, schema } => commands::search(&index, &query.join(" "), *limit, schema),
        Command::Schema { schema, key, path } => {
            commands::schema(&index, schema.as_deref(), key.as_deref(), path.as_deref())
        }
        Command::Get { schema, key, path } => commands::get(&index, schema, key, path.as_deref()),
        Command::Set { schema, key, value, path, dry_run, yes } => {
            let opts = WriteOpts { path: path.as_deref(), dry_run: *dry_run, yes: *yes };
            commands::set(&index, schema, key, value, &opts)
        }
        Command::Reset { schema, key, path, dry_run, yes } => {
            let opts = WriteOpts { path: path.as_deref(), dry_run: *dry_run, yes: *yes };
            commands::reset(&index, schema, key, &opts)
        }
        Command::Changed { schema } => commands::changed(&index, schema),
        Command::Export { area, out, include_state } => {
            let opts = ExportOpts { areas: area, out_dir: out.as_deref(), include_state: *include_state };
            state::export(&index, &opts)
        }
        Command::Apply { files, area, dry_run, yes, prune } => {
            let opts = ApplyOpts { files, areas: area, prune: *prune, dry_run: *dry_run, yes: *yes };
            state::apply(&index, &opts)
        }
        Command::Areas => Ok(state::areas()),
        Command::Doctor => doctor::doctor(&index),
        Command::Ext(e) => match e {
            ExtCommand::List => ext::list(&index),
            ExtCommand::Info { uuid } => ext::info(&index, uuid),
            ExtCommand::Enable { uuid, dry_run, yes } => ext::set_enabled(&index, uuid, true, *dry_run, *yes),
            ExtCommand::Disable { uuid, dry_run } => ext::set_enabled(&index, uuid, false, *dry_run, false),
        },
        Command::Keys(k) => match k {
            KeysCommand::Find { accel } => keys::find(&index, accel),
            KeysCommand::List { custom } => keys::list(&index, *custom),
            KeysCommand::Conflicts => keys::conflicts(&index),
            KeysCommand::Add { name, accel, command, force, dry_run, yes } => {
                let opts = AddOpts { name, accel, command, force: *force, dry_run: *dry_run, yes: *yes };
                keys::add(&index, &opts)
            }
            KeysCommand::Remove { binding, dry_run } => keys::remove(&index, binding, *dry_run),
        },
        Command::History { limit, schema, key } => commands::history(*limit, schema, key.as_deref()),
        Command::Revert { id, force, dry_run, yes } => commands::revert(&index, id, *force, *dry_run, *yes),
        Command::Watch { .. } => unreachable!("handled above"),
    }
    .map(Some)
}

fn command_name(c: &Command) -> &'static str {
    match c {
        Command::Search { .. } => "search",
        Command::Schema { .. } => "schema",
        Command::Get { .. } => "get",
        Command::Set { .. } => "set",
        Command::Reset { .. } => "reset",
        Command::Changed { .. } => "changed",
        Command::Export { .. } => "export",
        Command::Apply { .. } => "apply",
        Command::Areas => "areas",
        Command::Doctor => "doctor",
        Command::Watch { .. } => "watch",
        Command::Ext(ExtCommand::List) => "ext-list",
        Command::Ext(ExtCommand::Info { .. }) => "ext-info",
        Command::Ext(_) => "ext-enable",
        Command::Keys(KeysCommand::Find { .. }) | Command::Keys(KeysCommand::List { .. }) => "keys-list",
        Command::Keys(KeysCommand::Conflicts) => "keys-conflicts",
        Command::Keys(KeysCommand::Add { .. }) => "keys-add",
        Command::Keys(KeysCommand::Remove { .. }) => "keys-remove",
        Command::History { .. } => "history",
        Command::Revert { .. } => "revert",
    }
}

fn fail(err: &Error) -> ExitCode {
    eprintln!("{}", err.to_json());
    ExitCode::from(err.exit_code() as u8)
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if !e.use_stderr() => {
            // --help and --version
            let _ = e.print();
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            let message = e.render().to_string();
            let message = message.trim().trim_start_matches("error: ");
            return fail(&Error::Usage(message.to_string()));
        }
    };
    match run(&cli) {
        Ok(None) => ExitCode::SUCCESS,
        Ok(Some(out)) => {
            match cli.format {
                Format::Json => println!("{}", serde_json::to_string_pretty(&out).unwrap()),
                Format::Text => print!("{}", gsctl::text::render(command_name(&cli.command), &out)),
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}
