use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::error::ErrorKind;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
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
/// Output is JSON on stdout and errors are JSON on stderr, for agents. For
/// people: `--format text`, or `export GSCTL_FORMAT=text`; errors are plain
/// text when stderr is a terminal. Exit codes: 0 ok, 1 internal error,
/// 2 usage, 3 schema or key not found, 4 invalid value, 5 not writable,
/// 6 guarded key (needs --yes), 7 conflict (revert of a key changed since;
/// needs --force).
#[derive(Parser)]
#[command(
    name = "gsctl",
    version,
    max_term_width = 100,
    arg_required_else_help = true,
    after_help = "Examples:
  gsctl doctor                                        # can gsctl reach this session?
  gsctl search animations --format text
  gsctl set org.gnome.desktop.interface color-scheme prefer-dark --dry-run
  gsctl revert last                                   # undo the last change

Shell completions: gsctl completions bash|zsh|fish. Man page: gsctl man."
)]
struct Cli {
    /// Output format [default: json, or $GSCTL_FORMAT].
    #[arg(long, global = true, value_enum)]
    format: Option<Format>,

    /// Print JSON on one line. The default when stdout isn't a terminal
    /// (or GSCTL_JSON=compact).
    #[arg(long, global = true, conflicts_with = "pretty")]
    compact: bool,

    /// Pretty-print JSON. The default on a terminal (or GSCTL_JSON=pretty).
    #[arg(long, global = true)]
    pretty: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Format {
    Json,
    Text,
}

#[derive(Subcommand)]
enum Command {
    /// Ranked search over schema ids, key names, summaries and descriptions.
    #[command(after_help = "Examples:
  gsctl search animations
  gsctl search clock seconds --format text
  gsctl search lock --schema org.gnome.desktop --limit 5

GNOME's words may not be yours: try a single word (\"dark\", not \"dark mode\").")]
    Search {
        /// Words to search for; keys matching every word come first.
        #[arg(required = true)]
        query: Vec<String>,
        /// Maximum number of results.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Only search this schema and the schemas below it (repeatable).
        #[arg(long)]
        schema: Vec<String>,
        /// Rank schemas that don't apply to this desktop (Ubuntu Touch, Unity,
        /// deprecated) like the others instead of last.
        #[arg(long)]
        all: bool,
    },
    /// Describe a key (type, default, current value, allowed values), list a
    /// schema's keys briefly, or list every schema when called without
    /// arguments. SCHEMA KEY can also be one schema.key or /dconf/path/key.
    Schema {
        /// Schema id, e.g. org.gnome.desktop.interface; omit to list every schema.
        schema: Option<String>,
        /// Key name, e.g. color-scheme; omit to list the schema's keys.
        key: Option<String>,
        /// Instance path for a relocatable schema, e.g. /org/gnome/.../custom0/.
        #[arg(long)]
        path: Option<String>,
        /// Describe every key of the schema in full.
        #[arg(long)]
        full: bool,
        /// Show secret values (passwords) instead of {"redacted": true}.
        #[arg(long)]
        show_secrets: bool,
    },
    /// Print a key's current value. SCHEMA KEY can also be one schema.key or
    /// /dconf/path/key argument.
    Get {
        /// Schema id, e.g. org.gnome.desktop.interface.
        schema: String,
        /// Key name, e.g. color-scheme (omit it with a schema.key or /dconf/path/key argument).
        key: Option<String>,
        /// Instance path for a relocatable schema.
        #[arg(long)]
        path: Option<String>,
        /// Show secret values (passwords) instead of {"redacted": true}.
        #[arg(long)]
        show_secrets: bool,
    },
    /// Validate and write a value. VALUE is JSON (true, 42, "text", ["a","b"]),
    /// GVariant text (@as [], ('xkb', 'us')), or bare text for string keys.
    /// SCHEMA KEY can also be one schema.key or /dconf/path/key argument.
    #[command(after_help = r#"Examples (one per value type; quote anything with [ ] < > ( ) spaces or quotes):
  gsctl set org.gnome.desktop.interface color-scheme prefer-dark       # string: bare text
  gsctl set org.gnome.desktop.wm.preferences button-layout ''          # empty string
  gsctl set org.gnome.desktop.interface clock-show-seconds true        # boolean (or yes/no, on/off)
  gsctl set org.gnome.desktop.session idle-delay 600                   # integer
  gsctl set org.gnome.desktop.interface text-scaling-factor 1.25       # number
  gsctl set org.gnome.shell favorite-apps '["firefox.desktop", "org.gnome.Nautilus.desktop"]'
  gsctl set org.gnome.desktop.wm.keybindings switch-to-workspace-1 '["<Super>1"]'
  gsctl set org.gnome.desktop.wm.keybindings show-desktop '[]'         # empty list
  gsctl set org.gnome.desktop.input-sources sources "[('xkb', 'us'), ('xkb', 'de')]" --yes

Check with --dry-run first. `gsctl schema SCHEMA KEY` shows the type and allowed values;
`gsctl revert last` undoes the change."#)]
    Set {
        /// Schema id, e.g. org.gnome.desktop.interface.
        schema: String,
        /// Key name, e.g. color-scheme (or the value, with a schema.key argument).
        #[arg(allow_negative_numbers = true)]
        key: String,
        /// The new value (see the examples below for quoting).
        #[arg(allow_hyphen_values = true)]
        value: Option<String>,
        /// Instance path for a relocatable schema.
        #[arg(long)]
        path: Option<String>,
        /// Validate and show the change without writing it.
        #[arg(long)]
        dry_run: bool,
        /// Confirm a change to a guarded key.
        #[arg(long)]
        yes: bool,
        /// Show secret values (passwords) instead of {"redacted": true}.
        #[arg(long)]
        show_secrets: bool,
    },
    /// Reset a key to its default.
    Reset {
        /// Schema id, e.g. org.gnome.desktop.interface.
        schema: String,
        /// Key name, e.g. color-scheme (omit it with a schema.key or /dconf/path/key argument).
        key: Option<String>,
        /// Instance path for a relocatable schema.
        #[arg(long)]
        path: Option<String>,
        /// Show the change without writing it.
        #[arg(long)]
        dry_run: bool,
        /// Confirm a change to a guarded key.
        #[arg(long)]
        yes: bool,
        /// Show secret values (passwords) instead of {"redacted": true}.
        #[arg(long)]
        show_secrets: bool,
    },
    /// Export customised settings as JSON state files, one per area. Prints a
    /// single document, or writes <area>.json files with --out. Machine state
    /// (counters, timestamps, migration markers) is skipped unless
    /// --include-state. The remote area (remote desktop, including whether
    /// RDP is enabled) is left out unless --include-remote or --area remote.
    #[command(after_help = "Examples:
  gsctl export --format text                          # what would be exported, per area
  gsctl export --out ~/dotfiles/gnome                 # one <area>.json per area
  gsctl export --area keybindings --area interface > gnome.json

Then commit the directory, and on another machine (or later):
  gsctl apply ~/dotfiles/gnome --dry-run && gsctl apply ~/dotfiles/gnome")]
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
        /// Also export the remote area (remote desktop settings).
        #[arg(long)]
        include_remote: bool,
    },
    /// Apply state files from `export`. Validates every value first and writes
    /// nothing if any is invalid; only keys that differ are written, as one
    /// journal batch (`gsctl revert <batch>` undoes it).
    #[command(after_help = "Examples:
  gsctl apply ~/dotfiles/gnome --dry-run --format text   # every *.json in the directory
  gsctl apply ~/dotfiles/gnome
  gsctl apply interface.json keybindings.json --area keybindings
  gsctl apply ~/dotfiles/gnome --prune --dry-run         # also reset keys the files don't list
  gsctl revert last                                      # undo the whole apply")]
    Apply {
        /// State files from `gsctl export`, or a directory of them (every
        /// *.json in it, in name order), such as the one `export --out` wrote.
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
    /// --timeout. With --format text, prints `HH:MM:SS schema key = value`.
    #[command(after_help = "Examples:
  gsctl watch --format text                           # then change something in Settings
  gsctl watch --schema org.gnome.desktop.interface --count 1 --timeout 30")]
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
        /// Show secret values (passwords) instead of {"redacted": true}.
        #[arg(long)]
        show_secrets: bool,
    },
    /// Check that gsctl can see and change this session's real settings
    /// (session bus, dconf, sandboxing). Run it first in a new environment.
    Doctor,
    /// List the export areas and the schemas each covers.
    Areas,
    /// GNOME Shell extensions: installed, enabled, and their settings.
    #[command(subcommand, arg_required_else_help = true)]
    Ext(ExtCommand),
    /// Keyboard shortcuts across all keybinding schemas and custom bindings.
    #[command(subcommand, arg_required_else_help = true)]
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
    #[command(after_help = "Examples:
  gsctl revert last --dry-run                         # what the undo would do
  gsctl revert last                                   # undo the newest change (or batch)
  gsctl revert last                                   # again: the one before, like an undo stack
  gsctl revert apply-1791575264841-248795-1           # a batch id from `gsctl history`")]
    Revert {
        /// Entry or batch id from `gsctl history`, or `last`.
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
    /// App state (counters, timestamps, window geometry) is left out unless
    /// --include-state.
    Changed {
        /// Only this schema and the schemas below it (repeatable).
        #[arg(long)]
        schema: Vec<String>,
        /// Also list keys that look like app state, marked with "state".
        #[arg(long)]
        include_state: bool,
        /// Show secret values (passwords) instead of {"redacted": true}.
        #[arg(long)]
        show_secrets: bool,
    },
    /// Print a shell completion script.
    #[command(after_help = "Install:
  gsctl completions bash > ~/.local/share/bash-completion/completions/gsctl
  gsctl completions zsh > ~/.zfunc/_gsctl            # with fpath+=(~/.zfunc) in ~/.zshrc
  gsctl completions fish > ~/.config/fish/completions/gsctl.fish")]
    Completions {
        /// The shell to complete for.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Print the man page (roff).
    #[command(after_help = "Install:
  mkdir -p ~/.local/share/man/man1 && gsctl man > ~/.local/share/man/man1/gsctl.1
  man gsctl")]
    Man,
    /// Machine-readable list of every command, argument and flag, plus exit
    /// and error codes (also `gsctl help --json`).
    Commands,
}

#[derive(Subcommand)]
enum ExtCommand {
    /// Installed extensions and whether each is enabled in settings.
    List,
    /// One extension's metadata and settings (keys with current values).
    Info {
        /// Extension uuid, e.g. dash-to-dock@micxgx.gmail.com (see `gsctl ext list`).
        uuid: String,
    },
    /// Enable an installed extension (adds just its uuid). Its code runs
    /// inside GNOME Shell, so this needs --yes.
    Enable {
        /// Extension uuid (see `gsctl ext list`).
        uuid: String,
        /// Show the change without writing it.
        #[arg(long)]
        dry_run: bool,
        /// Confirm running the extension's code.
        #[arg(long)]
        yes: bool,
    },
    /// Disable an extension (removes just its uuid).
    Disable {
        /// Extension uuid (see `gsctl ext list`).
        uuid: String,
        /// Show the change without writing it.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum KeysCommand {
    /// What is bound to ACCEL? e.g. '<Super>t'. Aliases and modifier order
    /// don't matter (<Ctrl> = <Primary> = <Control>).
    Find {
        /// Accelerator in GTK syntax, e.g. '<Super>t' (quote it: < and > are shell redirects).
        accel: String,
    },
    /// Every binding that has an accelerator.
    List {
        /// Only custom keybindings.
        #[arg(long)]
        custom: bool,
    },
    /// Accelerators bound to more than one action.
    Conflicts,
    /// Add a custom keybinding that runs COMMAND. ACCEL is GTK syntax
    /// ('<Super>t', '<Control><Alt>Delete'). Refuses an accelerator that's
    /// already bound, or a NAME already used, unless --force. Needs --yes,
    /// since it sets up a command to run.
    #[command(after_help = "Examples:
  gsctl keys add Terminal '<Super>t' ptyxis --dry-run
  gsctl keys add Terminal '<Super>t' ptyxis --yes
  gsctl keys add 'Screenshot area' '<Super><Shift>s' 'gnome-screenshot -a' --yes
  gsctl keys find '<Super>t'                          # check it's there
  gsctl revert <journal_id>                           # undo the add

Quote the accelerator: < and > are shell redirects.")]
    Add {
        /// Name shown in Settings, e.g. Terminal.
        name: String,
        /// Accelerator in GTK syntax, e.g. '<Super>Return'.
        accel: String,
        /// Command line to run, e.g. ptyxis (quote it if it has spaces).
        command: String,
        /// Add even if ACCEL is already bound or NAME is taken.
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
        /// The binding's name, its path, or `customN` (see `gsctl keys list --custom`).
        binding: String,
        /// Show the writes without making them.
        #[arg(long)]
        dry_run: bool,
    },
}

/// The JSON document to print, or `None` for streaming commands that print
/// their own output.
fn run(cli: &Cli, text: bool) -> Result<Option<Value>> {
    // These describe the CLI itself and need no session.
    let mut buf = Vec::new();
    match &cli.command {
        Command::Completions { shell } => clap_complete::generate(*shell, &mut Cli::command(), "gsctl", &mut buf),
        Command::Man => clap_mangen::Man::new(Cli::command())
            .render(&mut buf)
            .map_err(|e| Error::Internal(format!("cannot render the man page: {e}")))?,
        _ => {}
    }
    if !buf.is_empty() {
        // A closed pipe (`| head`) is fine.
        let _ = std::io::stdout().write_all(&buf);
        return Ok(None);
    }
    if let Command::Commands = cli.command {
        return Ok(Some(commands_json()));
    }
    gsctl::guard::show_secrets(shows_secrets(&cli.command));
    let index = Index::load()?;
    if let Command::Watch { schema, count, timeout, .. } = &cli.command {
        return watch::watch(&index, &WatchOpts { prefixes: schema, count: *count, timeout: *timeout, text });
    }
    match &cli.command {
        Command::Search { query, limit, schema, all } => {
            let opts = commands::SearchOpts { limit: *limit, prefixes: schema, all: *all };
            commands::search(&index, &query.join(" "), &opts)
        }
        Command::Schema { schema, key, path, full, .. } => {
            // One schema.key or /dconf/path/key argument.
            let split = match (schema, key) {
                (Some(s), None) => commands::split_target(&index, s),
                _ => None,
            };
            // Text output shows ranges and defaults, so it needs the full form.
            let full = *full || text;
            match split {
                Some((s, k, p)) => {
                    commands::schema_with(&index, Some(&s), Some(&k), p.as_deref().or(path.as_deref()), full)
                }
                None => commands::schema_with(&index, schema.as_deref(), key.as_deref(), path.as_deref(), full),
            }
        }
        Command::Get { schema, key, path, .. } => {
            let (s, k, p) = commands::target(&index, schema, key.as_deref(), path.as_deref())?;
            commands::get(&index, &s, &k, p.as_deref())
        }
        Command::Set { schema, key, value, path, dry_run, yes, .. } => {
            // `set schema.key VALUE`: the value arrives in KEY's place.
            let (key, value) = match value {
                Some(v) => (Some(key.as_str()), v),
                None => (None, key),
            };
            let (s, k, p) = commands::target(&index, schema, key, path.as_deref())?;
            let opts = WriteOpts { path: p.as_deref(), dry_run: *dry_run, yes: *yes };
            commands::set(&index, &s, &k, value, &opts)
        }
        Command::Reset { schema, key, path, dry_run, yes, .. } => {
            let (s, k, p) = commands::target(&index, schema, key.as_deref(), path.as_deref())?;
            let opts = WriteOpts { path: p.as_deref(), dry_run: *dry_run, yes: *yes };
            commands::reset(&index, &s, &k, &opts)
        }
        Command::Changed { schema, include_state, .. } => commands::changed(&index, schema, *include_state),
        Command::Commands => unreachable!("handled above"),
        Command::Export { area, out, include_state, include_remote } => {
            let opts = ExportOpts {
                areas: area,
                out_dir: out.as_deref(),
                include_state: *include_state,
                include_remote: *include_remote,
            };
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
        Command::Watch { .. } | Command::Completions { .. } | Command::Man => unreachable!("handled above"),
    }
    .map(Some)
}

/// `--show-secrets`, on the commands that have it.
fn shows_secrets(c: &Command) -> bool {
    match c {
        Command::Schema { show_secrets, .. }
        | Command::Get { show_secrets, .. }
        | Command::Set { show_secrets, .. }
        | Command::Reset { show_secrets, .. }
        | Command::Watch { show_secrets, .. }
        | Command::Changed { show_secrets, .. } => *show_secrets,
        _ => false,
    }
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
        Command::Completions { .. } => "completions",
        Command::Man => "man",
        Command::Commands => "commands",
    }
}

/// `gsctl commands` / `gsctl help --json`: the clap command tree as JSON.
fn commands_json() -> Value {
    let mut out = gsctl::help::tree(&Cli::command());
    out["exit_codes"] = gsctl::help::exit_codes();
    out["error_codes"] = gsctl::help::error_codes();
    out
}

/// Compact JSON unless asked for pretty, or stdout is a terminal.
fn pretty(cli: &Cli) -> bool {
    if cli.pretty || cli.compact {
        return cli.pretty;
    }
    match std::env::var("GSCTL_JSON").as_deref() {
        Ok("pretty") => true,
        Ok("compact") => false,
        _ => std::io::stdout().is_terminal(),
    }
}

/// The output format and whether it was chosen (by `--format` or
/// `GSCTL_FORMAT`) rather than defaulted. The flag wins over the variable.
fn resolve_format(flag: Option<Format>) -> Result<(Format, bool)> {
    if let Some(f) = flag {
        return Ok((f, true));
    }
    match std::env::var("GSCTL_FORMAT").ok().as_deref().map(str::trim) {
        None | Some("") => Ok((Format::Json, false)),
        Some(v) => match Format::from_str(v, true) {
            Ok(f) => Ok((f, true)),
            Err(_) => Err(Error::Usage(format!("GSCTL_FORMAT must be json or text, not {v:?}"))),
        },
    }
}

/// `--format` from the raw arguments, for errors raised before clap has
/// parsed them.
fn format_flag_in_args() -> Option<Format> {
    let args: Vec<String> = std::env::args().skip(1).take_while(|a| a != "--").collect();
    let mut found = None;
    for (i, a) in args.iter().enumerate() {
        let value = match a.strip_prefix("--format") {
            Some("") => args.get(i + 1).map(String::as_str),
            Some(rest) => rest.strip_prefix('='),
            None => None,
        };
        if let Some(f) = value.and_then(|v| Format::from_str(v, true).ok()) {
            found = Some(f);
        }
    }
    found
}

/// Errors are plain text with `--format text` (or `GSCTL_FORMAT=text`), and
/// when no format was chosen and stderr is a terminal; JSON otherwise.
fn text_errors(format: Format, explicit: bool) -> bool {
    format == Format::Text || (!explicit && std::io::stderr().is_terminal())
}

fn fail(err: &Error, text: bool) -> ExitCode {
    if text {
        eprint!("{}", gsctl::text::render_error(&err.to_json()));
    } else {
        eprintln!("{}", err.to_json());
    }
    ExitCode::from(err.exit_code() as u8)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["help", "--json"] {
        println!("{}", commands_json());
        return ExitCode::SUCCESS;
    }
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if !e.use_stderr() => {
            // --help and --version
            let _ = e.print();
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            let (format, explicit) = resolve_format(format_flag_in_args()).unwrap_or((Format::Json, false));
            // Bare `gsctl` (or `gsctl keys`) gets clap's help, as do people.
            if e.kind() == ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand || text_errors(format, explicit) {
                let _ = e.print();
                return ExitCode::from(2);
            }
            let message = e.render().to_string();
            let message = message.trim().trim_start_matches("error: ");
            return fail(&Error::Usage(message.to_string()), false);
        }
    };
    let (format, explicit) = match resolve_format(cli.format) {
        Ok(f) => f,
        Err(e) => return fail(&e, std::io::stderr().is_terminal()),
    };
    let text = format == Format::Text;
    match run(&cli, text) {
        Ok(None) => ExitCode::SUCCESS,
        Ok(Some(out)) => {
            match format {
                Format::Json if pretty(&cli) => println!("{}", serde_json::to_string_pretty(&out).unwrap()),
                Format::Json => println!("{out}"),
                Format::Text => print!("{}", gsctl::text::render(command_name(&cli.command), &out)),
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(&commands::with_suggestions(e), text_errors(format, explicit)),
    }
}
