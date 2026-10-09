---
name: gnome-settings
description: Read and change GNOME desktop settings (GSettings/dconf) safely with the `gsap` CLI. Use when asked to change a GNOME preference (theme, dark mode, fonts, workspaces, dock, power, keyboard shortcuts, extension options), to find which key controls something, or to see what has been customised. Not related to the GSAP animation library.
---

# GNOME settings with `gsap`

`gsap` reads every installed GSettings schema (including extensions' own
schemas) and gives you search, types, allowed values, validated writes and a
journal. Prefer it over `gsettings` and `dconf`. Never apply a raw
`dconf dump`/`dconf load`: it mixes preferences with machine state and skips
all validation.

All output is JSON on stdout. Errors are JSON on stderr with a `code`, a
`message`, and often a `hint`, the key's `type` and its allowed `range`.

## The loop

1. **Search** for the setting. Every word must match; key names rank highest.

       gsap search dark mode
       gsap search workspace --schema org.gnome.mutter --limit 5

   Each result has `schema`, `key`, `type`, `summary` and the current `value`.

2. **Inspect** the key before changing it.

       gsap schema org.gnome.desktop.interface color-scheme

   Read `type` (GVariant signature), `type_description`, `range` (enum
   `choices`, `flags`, or `min`/`max`), `default`, `value`, `writable` and
   `guarded`. `gsap schema <schema>` lists all keys; `gsap schema` lists all
   schemas.

3. **Dry-run** the change. It validates the value and shows `old` → `new`
   without writing.

       gsap set org.gnome.desktop.interface color-scheme prefer-dark --dry-run

4. **Set** it.

       gsap set org.gnome.desktop.interface color-scheme prefer-dark

   The response includes the value read back (`value`) and a `journal_id`.

5. **Verify** with `get` (and, if it matters, by looking at the desktop).

       gsap get org.gnome.desktop.interface color-scheme

6. **Record** it. Once `gsap export` exists, export the area into the setup
   repo and commit. Until then, note the `schema key value` triple in the
   setup repo's notes.

To undo, `gsap reset <schema> <key>` restores the default. The previous value
of every change is in `~/.local/state/gsap/journal.ndjson`.

## Writing values

VALUE is parsed against the key's type. Use JSON:

| type | example |
|---|---|
| `b` | `true` |
| `i` `u` `d` | `42`, `1.25` (negative numbers work: `-3`) |
| `s`, enums | `prefer-dark` (bare text is fine for strings) or `"prefer-dark"` |
| `as`, flags | `'["firefox.desktop","org.gnome.Nautilus.desktop"]'` |
| `a{sv}` | `'{"show": true, "size": 24}'` |
| tuples `(ss)`, `a(ss)` | `'[["xkb","us"],["xkb","de"]]'` |
| maybe `ms` | `null` or the inner value |

GVariant text also works (`"@as []"`, `"[('xkb', 'us')]"`). Quote JSON for the
shell.

## Exit codes

| code | meaning | what to do |
|---|---|---|
| 0 | ok | |
| 2 | usage | fix the command line (relocatable schemas need `--path`) |
| 3 | schema or key not found | `gsap search` for the right name |
| 4 | invalid value | read `type`, `range` and `message` in the error and retry |
| 5 | not writable | locked by the administrator; tell the user |
| 6 | guarded key | ask the user, then re-run with `--yes` |

## Guarded keys

Some keys can lock the user out or break input: remote desktop
(`org.gnome.desktop.remote-desktop.*`), `org.gnome.desktop.input-sources`,
`org.gnome.desktop.screensaver lock-enabled`,
`org.gnome.desktop.lockdown disable-lock-screen`, accessibility
(`org.gnome.desktop.a11y.*`), and `org.gnome.shell enabled-extensions` /
`disable-user-extensions`. `set`/`reset` refuse them without `--yes`, and
`--dry-run` reports `"requires_yes": true`. Get the user's explicit OK first,
especially over a remote session.

## Other commands

- `gsap changed [--schema org.gnome.desktop]`: keys that differ from their
  defaults. Useful to see what the user has customised. Some entries are app
  state (counters, timestamps, `migrated` flags) rather than preferences.
- Relocatable schemas (for example custom keybindings) need an instance path:
  `gsap get <schema> <key> --path /org/gnome/.../custom0/`.
- `--format text` prints a compact human-readable form.
