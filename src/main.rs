use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use gsap::commands::{self, WriteOpts};
use gsap::error::{Error, Result};
use gsap::index::Index;
use serde_json::Value;

/// A discoverable, validated JSON interface to GNOME settings (GSettings/dconf).
///
/// Typical loop: `gsap search <words>` → `gsap schema <schema> <key>` →
/// `gsap set <schema> <key> <value> --dry-run` → `gsap set ...` → `gsap get ...`.
///
/// Output is JSON on stdout; errors are JSON on stderr. Exit codes: 0 ok,
/// 1 internal error, 2 usage, 3 schema or key not found, 4 invalid value,
/// 5 not writable, 6 guarded key (needs --yes), 7 conflict (revert of a key
/// changed since; needs --force).
#[derive(Parser)]
#[command(name = "gsap", version, max_term_width = 100)]
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
    /// List recorded changes (set, reset, revert), newest first.
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
    /// it was at its default. ID is from `gsap history`, or `last`.
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

fn run(cli: &Cli) -> Result<Value> {
    let index = Index::load()?;
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
        Command::History { limit, schema, key } => commands::history(*limit, schema, key.as_deref()),
        Command::Revert { id, force, dry_run, yes } => commands::revert(&index, id, *force, *dry_run, *yes),
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
        Ok(out) => {
            match cli.format {
                Format::Json => println!("{}", serde_json::to_string_pretty(&out).unwrap()),
                Format::Text => print!("{}", gsap::text::render(command_name(&cli.command), &out)),
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}
