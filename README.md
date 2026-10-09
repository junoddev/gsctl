# gsctl — gnome-settings-agent-proxy

A Rust CLI that gives AI agents a discoverable, validated, JSON interface to
GNOME's settings (GSettings/dconf): search keys, inspect their types and allowed
values, set them with validation and `--dry-run`, and keep a journal of changes.

The command is `gsctl`; the bundled agent skill is `gnome-settings`
(`skills/gnome-settings/SKILL.md`). See [NOTES.md](NOTES.md) for the problem,
design, architecture and roadmap.

## Build

Needs `rustup`, `pkg-config` and `libglib2.0-dev`.

    cargo build --release

Tests run under `dbus-run-session` with throwaway schemas and a temporary
dconf database; they never touch your real settings:

    cargo test
