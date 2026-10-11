# Changelog

All notable changes to gsctl are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and gsctl uses
[Semantic Versioning](https://semver.org/).

The JSON output, the error codes and the exit codes are the contract agents
rely on. Before 1.0 they may change in a minor release (0.x); any breaking
change to them is listed here under that release.

## [Unreleased]

### Fixed

- `ext list` and `doctor` treat a plain GNOME session (`XDG_CURRENT_DESKTOP=GNOME`,
  as on Fedora and Arch) as the `user` session mode. gsctl used to pick the
  only installed mode file instead, which is `classic` wherever GNOME Classic
  is installed, and reported Classic's extensions as enabled.
- Extension schemas whose id isn't spelled `org.gnome.shell.extensions.*`,
  such as GSConnect's `org.gnome.Shell.Extensions.GSConnect`, or which use
  their own id at a path under `/org/gnome/shell/extensions/`, are now found
  by `search`, `schema` and `get`.
- `apply` follows a symlinked state file when it points to a regular file you
  own, so dotfiles linked into place by GNU Stow work again. Symlinks to
  other users' files, FIFOs and oversized files are still refused.

- `revert` of a `keys add` (and any write that only removes commands, such
  as clearing a shortcut's `command`) no longer needs `--yes`, as the README
  always showed. Reverts that add or restore a command, re-enable an
  extension, or touch the other guarded keys still need it.

### Added

- `doctor` warns when GNOME's desktop schemas (`gsettings-desktop-schemas`)
  aren't installed, instead of reporting that everything is fine.

## [0.1.0] - 2026-10-10

First public release.

### Added

- `search`: ranked search over schema ids, key names, summaries and
  descriptions, tuned for agent phrasing (plurals, partial matches), with hits
  that carry type, value and summary.
- `schema`: a key's type, default, current value, allowed values, description
  and writability in one call; a schema's keys; or every schema.
- `get`, `set`, `reset`: `set` checks the type and range before writing, takes
  JSON, GVariant text or a bare word, has `--dry-run`, and reads every write
  back so a silently dropped write fails. Keys can be named as `SCHEMA KEY`,
  `schema.key` or a dconf path.
- `changed`: the keys that differ from their defaults, without app state
  unless `--include-state`.
- A journal of every write with its previous value; `history` lists it and
  `revert <id|last>` undoes an entry or a whole batch, like an undo stack.
- `export` / `apply` / `areas`: per-area JSON state files for dotfiles.
  `apply` validates everything before writing anything, writes one revertible
  batch, and `--prune` resets keys the files no longer list. The `remote`
  area is left out of exports by default.
- `keys find|list|conflicts|add|remove`: keyboard shortcuts across every
  keybinding schema, extension bindings and custom bindings; `add` refuses an
  accelerator that's taken unless `--force`, and writes as one batch.
- `ext list|info|enable|disable`: extensions, their enabled state by GNOME
  Shell's rules (including the session mode's), and their settings, with
  extension schemas visible without `--schemadir`.
- `watch`: changes as NDJSON or text, with `--count` and `--timeout`.
- `doctor`: checks the session bus, dconf and sandboxing before anything else.
- JSON on stdout and JSON errors on stderr with stable codes, exit statuses,
  hints and did-you-mean suggestions; `--format text` (or `GSCTL_FORMAT`) for
  people, with control characters escaped. `gsctl commands` (or
  `gsctl help --json`) lists every command, flag, error code and exit code.
- `completions bash|zsh|fish` and `man`.
- An agent skill, `skills/gnome-settings/SKILL.md`, for the search, schema,
  dry run, set and verify loop.

### Security

- Guarded keys need `--yes`: remote desktop, input sources, the lock screen
  and idle settings, accessibility, keyboard shortcuts, extension lists,
  default-application and extension command keys that run code, proxy
  settings, and keyboard repeat below a usable floor. Guards apply to the
  dconf location, not just the schema id, and `--dry-run` reports
  `requires_yes`.
- Without a session bus or dconf-service, writes fail with `no_session`
  instead of appearing to succeed.
- Secret values (proxy passwords) are redacted in output as
  `{"redacted": true}` unless `--show-secrets`, and never exported or
  journaled.
- The journal and exported files are private (0600 files, 0700 directories,
  tightened if found loose) and never written through symlinks; `apply` reads
  only regular files, not through symlinks, with a size cap.
- Extension schemas can't shadow system schemas.
- SECURITY.md: the trust model, a recommended Claude Code permission setup,
  scope, and how to report a vulnerability privately.

[Unreleased]: https://github.com/junoddev/gsctl/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/junoddev/gsctl/releases/tag/v0.1.0
