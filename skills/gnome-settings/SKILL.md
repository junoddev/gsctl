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

1. **Search** for the setting. Key names rank highest, then schema ids,
   summaries and descriptions; plurals match ("workspace" finds
   `dynamic-workspaces`).

       gsap search color scheme
       gsap search workspace --schema org.gnome.mutter --limit 5

   Each result has `schema`, `key`, `type`, `summary` and the current `value`.
   If no key matches every word, you get the keys matching the most words and
   `"partial": true`. Treat those as hints and try GNOME's vocabulary instead
   (dark mode is `color-scheme`, the dock is `dash-to-dock`, screen blanking
   is `idle-delay`).

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

6. **Record** it in the setup repo: export the area and commit the file.

       gsap areas                                   # which area owns what
       gsap export --area interface --out <repo>/gnome/
       git -C <repo> diff                           # review, then commit

## State files: export and apply

`gsap export` writes only customised keys, one JSON file per area
(`extensions`, `keybindings`, `interface`, `shell`, `input`, `power`,
`remote`, `a11y`, `other`), sorted for clean diffs. Relocatable instances that
GNOME lists in a key (custom keybindings, Ptyxis profiles) are included as
`schema:/path/` entries. App state (counters, timestamps,
`migrated` flags, window geometry, TLS paths) is skipped and listed under
`skipped_state`; don't add it back by hand.

    gsap apply <repo>/gnome/*.json --dry-run     # review "changes" and "requires_yes"
    gsap apply <repo>/gnome/*.json               # add --yes only after the user agrees to guarded keys
    gsap revert last                             # undoes the whole apply

`apply` validates every value first and writes nothing if anything is wrong
(error `batch_invalid` with a `problems` list). It only writes keys that
differ, so running it twice is safe. It does not reset keys missing from the
file. Edit state files with real values from `gsap schema`; a value shown as
`{"$gvariant": "..."}` is typed GVariant text and should be kept as is.

## Undo

Every `set`, `reset` and `revert` is journaled with the previous value.

    gsap history                      # newest first; --key, --schema, --limit
    gsap revert last --dry-run        # or an id from history
    gsap revert last

`revert` restores the previous value, or resets the key if it was at its
default. `revert last` is an undo stack: each run undoes the newest change
not yet undone (a whole batch for `apply`, `keys`, `ext`). To redo, revert
the revert by its id. If the key has changed since that entry it refuses (exit 7); check
`gsap get` and `gsap history --key <key>`, and only use `--force` if
overwriting the newer value is really intended. `gsap reset <schema> <key>`
goes straight back to the default.

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
| 5 | not writable | `not_writable`: locked by the administrator; `no_session`: no session bus or dconf-service (sandboxed), so nothing can be saved. Tell the user either way |
| 6 | guarded key | ask the user, then re-run with `--yes` |
| 7 | conflict | `revert` of a key changed since; inspect, then `--force` if intended |

## Guarded keys

Some keys can lock the user out or break input: remote desktop
(`org.gnome.desktop.remote-desktop.*`), `org.gnome.desktop.input-sources`,
`org.gnome.desktop.screensaver lock-enabled`,
`org.gnome.desktop.lockdown disable-lock-screen`, accessibility
(`org.gnome.desktop.a11y.*`), and `org.gnome.shell enabled-extensions` /
`disable-user-extensions`. `set`/`reset` refuse them without `--yes`, and
`--dry-run` reports `"requires_yes": true`. Get the user's explicit OK first,
especially over a remote session.

## Keyboard shortcuts

Shortcuts live in several schemas (wm, shell, mutter, media-keys) plus
custom bindings in a relocatable schema. Use `gsap keys` rather than editing
those keys by hand:

    gsap keys find '<Super>t'          # what's on it? "free": true if nothing
    gsap keys conflicts                 # accelerators bound twice
    gsap keys list [--custom]
    gsap keys add Terminal '<Super>Return' ptyxis --dry-run
    gsap keys add Terminal '<Super>Return' ptyxis
    gsap keys remove Terminal           # by name, path or customN

`keys add` picks the next free `customN` slot and refuses an accelerator
that's taken (exit 7). Free it first, or use `--force` only if the user
wants both. `gsap revert last` removes the binding again. `keys remove`
takes the binding off the list and resets its keys; a name shared by two
bindings is refused (exit 2), so remove by path. `revert last` brings it back. To change a
built-in shortcut, `gsap set org.gnome.desktop.wm.keybindings <key>
'["<Super>x"]'` (these keys are string arrays; `[]` disables).

## Extensions

    gsap ext list                 # installed, enabled, enabled_by, schemas
    gsap ext info <uuid>          # metadata plus its settings with values
    gsap ext enable <uuid> [--dry-run]
    gsap ext disable <uuid> [--dry-run]

Change an extension's options through its schema (`gsap ext info` lists it,
e.g. Ubuntu Dock is `org.gnome.shell.extensions.dash-to-dock`). `enable` and
`disable` edit just that uuid, so they don't need `--yes`, and `gsap revert
last` undoes them. Never `set` `enabled-extensions` wholesale. On Ubuntu the
session mode enables the bundled extensions (`enabled_by: "session-mode"`):
disabling one adds it to `disabled-extensions`.

## Other commands

- `gsap doctor`: run first in a new environment. If `session_bus` is an
  error or `dconf_database` warns, you are probably sandboxed: values read
  as defaults and writes fail. Tell the user instead of retrying.
- `gsap watch [--schema ...] [--count N] [--timeout S]`: NDJSON stream; wait
  for the `ready` event, then each change arrives as
  `{"event":"changed","schema","key","value"}`. Handy for "change it in
  Settings and I'll capture the key".
- `gsap changed [--schema org.gnome.desktop]`: keys that differ from their
  defaults. Useful to see what the user has customised. Some entries are app
  state (counters, timestamps, `migrated` flags) rather than preferences.
- Relocatable schemas (for example custom keybindings) need an instance path:
  `gsap get <schema> <key> --path /org/gnome/.../custom0/`.
- `--format text` prints a compact human-readable form.
