# gsctl

A command-line interface to GNOME's settings (GSettings/dconf) built for AI
agents, and usable by people. You can search for the key that controls
something, see its type and allowed values, change it with validation and
`--dry-run`, and undo anything from a journal. Everything is JSON.

    $ gsctl search dark --format text
    ...
    org.gnome.desktop.interface color-scheme = prefer-dark  # Color scheme

    $ gsctl set org.gnome.desktop.interface color-scheme sideways --dry-run
    {"error":{"code":"invalid_value","exit_code":4,
     "message":"\"sideways\" is not one of the allowed values: default, prefer-dark, prefer-light", ...}}

    $ gsctl set org.gnome.desktop.interface color-scheme prefer-light
    $ gsctl revert last

## Why

Some Linux desktops are configured through a few plain-text files. On
[Omarchy](https://github.com/basecamp/omarchy) (Arch + Hyprland), every piece
(the compositor, the terminals, the theme) reads its own config from
`~/.config`. Omarchy's defaults load first and your files override them, so an
agent can read the setup, edit a line and reload.

GNOME can't work that way. Shell, Mutter, settings-daemon and the apps keep
their settings in one binary dconf database, described by hundreds of schemas
and thousands of typed keys, with no file to read or edit. Even Omarchy calls
`gsettings` to set GTK's colour scheme and theme, because GTK apps read them
from dconf. The tools that exist don't help an agent much:

- `dconf dump /` is thousands of lines of preferences mixed with machine
  state, with no types, defaults, allowed values or descriptions.
- `gsettings` has the metadata but no search, no JSON, no error an agent can
  act on, and no record of what changed.
- Keyboard shortcuts are spread over five schemas, plus custom bindings in a
  relocatable schema at paths listed in another key.
- Extension settings live in each extension's own `schemas/` directory, which
  plain `gsettings` often can't see.

gsctl gives GNOME what Omarchy gets from plain files: a settings surface that an
agent (or a person) can read, search and change safely. It's a typed interface
over dconf rather than text files, and `export`/`apply` gives you file-shaped
snapshots of the settings you've actually changed, to keep in a dotfiles repo.

## What it does

gsctl builds its view from the installed schemas at runtime, through GLib's own
API, with no hard-coded key list. Installing an extension makes its keys
searchable.

| Command | Purpose |
|---|---|
| `search <words>` | Ranked search over schema ids, key names, summaries and descriptions |
| `schema <schema> [key]` | Type, default, current value, allowed values, description, writability |
| `get` / `set` / `reset` | Read and write a key. `set` validates the type and range first; `--dry-run` writes nothing |
| `changed` | Only the keys that differ from their defaults |
| `history` / `revert <id\|last>` | Every write is journaled with its previous value; `revert last` walks back like an undo stack |
| `export` / `apply` | Per-area JSON state files (keybindings, interface, extensions, ...). `apply` validates everything before writing anything; `--prune` resets keys the files no longer list |
| `keys find\|list\|conflicts\|add\|remove` | Shortcuts across every keybinding schema and custom bindings: "what's on `<Super>a`?" |
| `ext list\|info\|enable\|disable` | Extensions, whether they're enabled (by GNOME Shell's rules, including the session mode's own), and their settings |
| `watch` | Stream changes as NDJSON, or wait for the next one (`--count`, `--timeout`) |
| `doctor` | Check that gsctl is talking to the real session (bus, dconf, sandboxing) |

Conventions:

- JSON on stdout (`--format text` for people), errors as JSON on stderr.
- Exit codes: 0 ok, 1 internal error, 2 usage, 3 not found, 4 invalid value,
  5 not writable or no session, 6 guarded key, 7 conflict.
- Values are JSON (`true`, `42`, `["a","b"]`), GVariant text (`@as []`), or
  bare text for string keys.

### Guardrails

- A few keys could cut off remote access or input: remote desktop, input
  sources, the lock screen, accessibility, and wholesale replacement of
  `enabled-extensions`. They need `--yes`; `--dry-run` reports
  `requires_yes`.
- Writes are read back, and gsctl fails if GSettings silently dropped them.
- Without a session bus (in a sandbox, for example), every write fails with
  `no_session` instead of appearing to succeed.

## Agent skill

`skills/gnome-settings/SKILL.md` is an agent skill that teaches the loop:
search → schema → `set --dry-run` → `set` → `get` to verify → `export` and
commit. Link or copy it into your agent's skills directory, e.g.
`~/.claude/skills/gnome-settings` for Claude Code.

## Install

Needs Rust (via `rustup`), `pkg-config` and GLib's headers
(`libglib2.0-dev` on Debian and Ubuntu). Tested on Ubuntu with GNOME 50.

    cargo install --path . --locked

Then run `gsctl doctor` to check it can see your session.

## Development

Tests run under `dbus-run-session` with throwaway schemas and a temporary dconf
database; they never touch your real settings:

    cargo test

An opt-in, read-only smoke test runs against the installed schemas:

    cargo test --test smoke_real -- --ignored

[NOTES.md](NOTES.md) has the design, the decisions made along the way, and the
results of testing on a real desktop.
