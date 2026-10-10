//! `watch`: stream setting changes as NDJSON.
//!
//! Subscribes to GSettings' own `changed` signal on every (matching) schema,
//! so events arrive as schema + key with no dconf path translation. Only keys
//! that belong to an installed schema are reported.

use std::cell::Cell;
use std::io::Write;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use gio::prelude::*;
use serde_json::{Value, json};

use crate::commands::matches_prefix;
use crate::error::{Error, Result};
use crate::index::Index;

pub struct WatchOpts<'a> {
    pub prefixes: &'a [String],
    /// Exit after this many change events.
    pub count: Option<usize>,
    /// Exit after this many seconds.
    pub timeout: Option<u64>,
    /// Lines for people (`HH:MM:SS schema key = value`) instead of NDJSON.
    pub text: bool,
}

/// Set from `WatchOpts::text` for the rest of the process.
static TEXT: AtomicBool = AtomicBool::new(false);

fn emit(v: &Value) {
    if TEXT.load(Ordering::Relaxed) {
        match crate::text::watch_event(v) {
            Some((line, true)) => eprintln!("{line}"),
            Some((line, false)) => write_line(&line),
            None => {}
        }
    } else {
        write_line(&v.to_string());
    }
}

fn write_line(line: &str) {
    let mut out = std::io::stdout().lock();
    // A closed pipe (e.g. `| head`) just ends the stream.
    if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
        std::process::exit(0);
    }
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

/// Runs until `count` events or `timeout`, or forever. Prints its own output;
/// returns `None` so main prints nothing more.
pub fn watch(index: &Index, opts: &WatchOpts) -> Result<Option<Value>> {
    // GLib timeouts are u32: in milliseconds they wrap after ~49 days, so
    // count in seconds and refuse what doesn't fit (~136 years).
    let timeout = match opts.timeout {
        Some(secs) => Some(u32::try_from(secs).map_err(|_| {
            Error::Usage(format!("--timeout {secs} is too large; the maximum is {} seconds", u32::MAX))
        })?),
        None => None,
    };
    TEXT.store(opts.text, Ordering::Relaxed);
    let main = glib::MainLoop::new(None, false);
    let seen = Rc::new(Cell::new(0usize));
    let mut watched: Vec<gio::Settings> = Vec::new();

    let mut subscribe = |entry: &crate::index::Entry, path: Option<String>| -> Result<()> {
        let settings = entry.settings(path.as_deref())?;
        let schema = entry.id();
        let (main, seen, count) = (main.clone(), seen.clone(), opts.count);
        settings.connect_changed(None, move |s, key| {
            // One dconf change can signal several keys before quit() takes
            // effect: never print more than `count`.
            if count.is_some_and(|c| seen.get() >= c) {
                return;
            }
            emit(&json!({
                "event": "changed",
                "time_ms": now_ms() as u64,
                "schema": schema,
                "path": path,
                "key": key,
                "value": crate::guard::shown(key, &s.value(key)),
                "is_default": s.user_value(key).is_none(),
            }));
            seen.set(seen.get() + 1);
            if count.is_some_and(|c| seen.get() >= c) {
                main.quit();
            }
        });
        // GSettings only emits `changed` for keys read after connecting.
        for key in entry.keys() {
            let _ = settings.value(&key);
        }
        watched.push(settings);
        Ok(())
    };

    for entry in index.entries().iter().filter(|e| !e.relocatable() && matches_prefix(&e.id(), opts.prefixes)) {
        subscribe(entry, None)?;
    }
    // Listed relocatable instances (custom keybindings, ...) that exist now.
    for (entry, path) in index.listed_instances() {
        if matches_prefix(&entry.id(), opts.prefixes) {
            subscribe(entry, Some(path))?;
        }
    }

    emit(&json!({ "event": "ready", "time_ms": now_ms() as u64, "schemas": watched.len() }));
    if opts.count == Some(0) {
        return Ok(None);
    }
    if let Some(secs) = timeout {
        let main = main.clone();
        glib::timeout_add_seconds_local_once(secs, move || {
            emit(&json!({ "event": "timeout", "time_ms": now_ms() as u64 }));
            main.quit();
        });
    }
    main.run();
    drop(watched);
    Ok(None)
}
