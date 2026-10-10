# gsctl

A command-line interface to GNOME's settings (GSettings/dconf) built for AI
agents, and usable by people. You can search for the key that controls
something, see its type and allowed values, change it with validation and
`--dry-run`, and undo anything from a journal. Output is JSON for agents, or
plain text for people. Website: https://junoddev.github.io/gsctl/

    $ gsctl search animations --limit 2 --format text
      org.gnome.desktop.interface              enable-animations  true  # Enable Animations
      org.gnome.shell.extensions.dash-to-dock  animation-time     0.2   # Animation time
    8 matches (showing 2 of 8; use --limit for more)

    $ gsctl set org.gnome.desktop.interface color-scheme sideways --dry-run
    error: "sideways" is not one of the allowed values: default, prefer-dark, prefer-light
    hint: see `gsctl schema org.gnome.desktop.interface color-scheme`; ...

    $ gsctl set org.gnome.desktop.interface color-scheme prefer-light
    $ gsctl revert last

Results depend on what's installed: that second hit is Ubuntu's dock. An
agent, whose stderr isn't a terminal, gets the same error as JSON:
`{"error":{"code":"invalid_value","exit_code":4,"message":...,"range":{...},"hint":...}}`.

## Install

Prebuilt binaries for x86_64 and aarch64 Linux (glibc 2.35 or newer, so
Ubuntu 22.04, Debian 12, Fedora 36 and later) are on
[GitHub Releases](https://github.com/junoddev/gsctl/releases). The shell
installer puts `gsctl` in `~/.local/bin`:

    curl --proto '=https' --tlsv1.2 -LsSf https://github.com/junoddev/gsctl/releases/latest/download/gsctl-installer.sh | sh

Each archive also has the man page and bash, zsh and fish completions.

With Rust, from crates.io:

    cargo install gsctl --locked

Or from source:

    git clone https://github.com/junoddev/gsctl
    cd gsctl
    cargo install --path . --locked

Building needs Rust 1.92 or newer, so install it with
[rustup](https://rustup.rs); a distribution's `rustc` is often too old. It
also needs `pkg-config` and GLib's headers:

| Distribution | Packages |
|---|---|
| Debian, Ubuntu | `pkg-config libglib2.0-dev` |
| Fedora | `pkg-config glib2-devel` |
| Arch | `pkg-config glib2` |

## Quick start

Check that gsctl can see your desktop session:

    $ gsctl doctor --format text
    ok  settings_backend  DConfSettingsBackend
    ok  session_bus       connected
    ...
    ok: gsctl can read and change this session's settings

Switch to dark mode, look, and undo it:

    $ export GSCTL_FORMAT=text        # plain text instead of JSON, for this shell
    $ gsctl schema org.gnome.desktop.interface color-scheme
    org.gnome.desktop.interface color-scheme
      value:    'default' (the default)
      type:     s (string)
      allowed:  one of 'default', 'prefer-dark', 'prefer-light'
      summary:  Color scheme
    ...
    $ gsctl set org.gnome.desktop.interface color-scheme prefer-dark --dry-run
    would set org.gnome.desktop.interface color-scheme: 'default' -> 'prefer-dark'
    $ gsctl set org.gnome.desktop.interface color-scheme prefer-dark
    set org.gnome.desktop.interface color-scheme: 'default' -> 'prefer-dark'
    $ gsctl revert last
    reverted org.gnome.desktop.interface color-scheme: 'prefer-dark' -> 'default'

`gsctl changed` lists what you've customised, `gsctl history` what gsctl
changed (grouped by batch, in local time), and every `gsctl revert last`
undoes one more step.

Without `--format text` (or `GSCTL_FORMAT=text`), output is JSON. Errors are
plain `error:`/`hint:` lines whenever stderr is a terminal, and JSON otherwise.

### Shell completion and man page

    gsctl completions bash > ~/.local/share/bash-completion/completions/gsctl
    gsctl completions zsh > ~/.zfunc/_gsctl        # with fpath+=(~/.zfunc) in ~/.zshrc
    gsctl completions fish > ~/.config/fish/completions/gsctl.fish
    mkdir -p ~/.local/share/man/man1 && gsctl man > ~/.local/share/man/man1/gsctl.1

Completion covers commands and options, not schema or key names.
`gsctl <command> --help` has examples for the common commands.

### Quoting values in the shell

`set` takes JSON, GVariant text, or a bare word for string keys. The shell
gets to the value first, so:

| Value | Type it as |
|---|---|
| A word | `prefer-dark` (no quotes needed) |
| An empty string | `''` |
| Text with spaces | `'Cantarell 11'` |
| A boolean | `true` / `false` (also `yes`/`no`, `on`/`off`) |
| A number | `600`, `1.25`, `-3` |
| A list (JSON) | `'["firefox.desktop", "org.gnome.Nautilus.desktop"]'`: single quotes outside, double inside |
| A one-item string list | `firefox.desktop` works too and becomes `['firefox.desktop']` |
| An empty list | `'[]'` |
| A shortcut | `'["<Super>1"]'`: always quote `<` and `>`, which the shell treats as redirection |
| GVariant text | `"[('xkb', 'us'), ('xkb', 'de')]"`: double quotes outside, single inside |

`gsctl schema SCHEMA KEY` shows a key's type and allowed values, and
`gsctl set --help` has one example per type. When a value is wrong, the error
gives an example of the right shape.

### Keyboard shortcuts

    $ gsctl keys find '<Super>Return'
    $ gsctl keys add Terminal '<Super>Return' ptyxis --dry-run
    would add 'Terminal' at /org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/: <Super>Return runs 'ptyxis'
    $ gsctl keys add Terminal '<Super>Return' ptyxis --yes
    $ gsctl revert last               # removes it again

`keys add` refuses an accelerator that's already taken (`--force` overrides).
Writing shortcuts, like enabling or disabling extensions, is guarded:
check with `--dry-run`, then confirm with `--yes`.

### Keeping your settings in dotfiles

`export` writes the settings you've changed from their defaults as one JSON
file per area (interface, keybindings, extensions, power, ...); `apply`
validates every value before writing anything and only writes what differs.

    $ gsctl export --out ~/dotfiles/gnome --format text
    extensions   /home/me/dotfiles/gnome/extensions.json   (11 keys)
    interface    /home/me/dotfiles/gnome/interface.json    (9 keys)
    keybindings  /home/me/dotfiles/gnome/keybindings.json  (34 keys)
    ...
    wrote 6 files
    $ cd ~/dotfiles && git add gnome && git commit -m "GNOME settings"

    # later, or on another machine:
    $ gsctl apply ~/dotfiles/gnome --dry-run --format text
    $ gsctl apply ~/dotfiles/gnome
    $ gsctl revert last               # undoes the whole apply

- `apply` takes the directory (every `*.json` in it) or individual files.
- Guarded keys in the files (input sources, lock screen, shortcuts, ...)
  need `--yes`; the dry run marks them.
- `apply --prune` also resets keys in the files' areas that the files no
  longer list, so the files become the whole state of those areas. Check
  with `--dry-run` first.
- Counters, timestamps and similar app state are left out
  (`--include-state` exports them).
- The `remote` area (remote desktop, including whether RDP is enabled) is
  left out by default, so applying your dotfiles on another machine doesn't
  switch on remote access there. Add `--include-remote` (or
  `--area remote`) to export it.

## gsettings and gsctl

| | `gsettings` | `gsctl` |
|---|---|---|
| Find the key for something | `list-recursively` and grep | `search`, ranked over names, summaries and descriptions |
| Type, default, allowed values | `range`, `describe`, one call each | `schema`, all in one |
| Change a value | `set`, GVariant text only | `set`, with type and range checks, `--dry-run`, guarded keys |
| Output | GVariant text | JSON, or text for people |
| Undo | none | `history`, `revert last` |
| Shortcuts | five schemas, plus a relocatable schema for custom ones | `keys find\|list\|conflicts\|add\|remove` |
| Extension settings | often not visible without `--schemadir` | included |
| Dotfiles | `dconf dump`/`load` of everything | `export`/`apply` per area, validated |

gsctl uses the same schemas and the same dconf database, so `gsettings`,
Settings and gsctl always agree.

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
| `watch` | Stream changes as NDJSON (or `HH:MM:SS schema key = value` lines), or wait for the next one (`--count`, `--timeout`) |
| `doctor` | Check that gsctl is talking to the real session (bus, dconf, sandboxing) |
| `completions <shell>` / `man` | Shell completion scripts and a man page |

Conventions:

- JSON on stdout and errors as JSON on stderr, for agents. `--format text`
  (or `GSCTL_FORMAT=text`; the flag wins) for people; errors are plain text
  when stderr is a terminal. Text output escapes control characters, so a
  value can't drive your terminal.
- Exit codes: 0 ok, 1 internal error, 2 usage, 3 not found, 4 invalid value,
  5 not writable or no session, 6 guarded key, 7 conflict.
- Values are JSON (`true`, `42`, `["a","b"]`), GVariant text (`@as []`), or
  bare text for string keys (see "Quoting values in the shell").

### Guardrails

- Some keys could cut off remote access or input: remote desktop, input
  sources, the lock screen, accessibility, keyboard shortcuts, and the
  extension lists. Changing them needs `--yes`; `--dry-run` reports
  `requires_yes` (and text output says `guarded: needs --yes`).
- Writes are read back, and gsctl fails if GSettings silently dropped them.
- Without a session bus (in a sandbox, for example), every write fails with
  `no_session` instead of appearing to succeed.
- Secret values (proxy passwords) are redacted in output as
  `{"redacted": true}` unless you pass `--show-secrets`, and never exported or
  journaled.

## Safety

gsctl trusts you: it makes changes visible (`--dry-run`, the journal) and
undoable (`revert`), and asks for `--yes` only on keys that can lock you out
or run code. It isn't a sandbox; anything running as you can call
`dconf write`. For an agent, the real gate is the agent's own permissions.
In Claude Code, this makes every `--yes` ask you first and refuses direct
dconf/gsettings writes:

```json
{
  "permissions": {
    "ask": ["Bash(gsctl * --yes)", "Bash(gsctl * --yes *)"],
    "deny": [
      "Bash(dconf write *)", "Bash(dconf load *)", "Bash(dconf reset *)",
      "Bash(gsettings set *)", "Bash(gsettings reset *)", "Bash(gsettings reset-recursively *)"
    ]
  }
}
```

Pattern rules are best-effort (`sh -c` or a full path gets around them). See
[SECURITY.md](SECURITY.md) for the full stance, the recommended setup and
how to report a vulnerability.

## Agent skill

To have an agent set gsctl up for you, tell it:

    Read https://junoddev.github.io/gsctl/agents.md and set up gsctl on this machine.

`skills/gnome-settings/SKILL.md` is an agent skill that teaches the loop:
search → schema → `set --dry-run` → `set` → `get` to verify → `export` and
commit. Link or copy it into your agent's skills directory, e.g.
`~/.claude/skills/gnome-settings` for Claude Code.

## Development

Tested on Ubuntu with GNOME 50. Tests run under `dbus-run-session` with
throwaway schemas and a temporary dconf database; they never touch your real
settings:

    cargo test

An opt-in, read-only smoke test runs against the installed schemas:

    cargo test --test smoke_real -- --ignored

## License

Copyright 2026 Aaron Junod. Licensed under either of the
[Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT license](LICENSE-MIT), at your option. Unless you state otherwise, any
contribution you submit for inclusion in gsctl is dual licensed the same
way, without any additional terms or conditions.
