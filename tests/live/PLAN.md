# Live regression test plan

The cargo tests are hermetic: each one runs on a private D-Bus session with
throwaway schemas and a temporary dconf database. They can't see what only a
real desktop has: the distribution's schemas and GLib, a running GNOME Shell
and its session mode, installed extensions, system dconf locks, and the
published install path. This suite covers that. Run it before every release,
on at least one distribution other than the one gsctl was developed on.

`regression.py` is the plan in executable form: one function per case, in the
order below. It's Python 3 with only the standard library, so it runs on any
distribution as is.

## Running it

It changes real settings. Every key it touches is snapshotted first (with
`dconf read`, not gsctl) and restored at the end, and its journal goes to a
temporary `XDG_STATE_HOME`. Still, run it in a test VM or a throwaway login.

In a terminal inside the GNOME session under test:

    python3 regression.py --gsctl ~/.local/bin/gsctl --i-know-this-changes-settings --report report.json

- `--only KEY` runs one group (any case-id prefix); `--list` prints the cases.
- Exit status is 0 only if every case passed or was skipped and every key was
  restored.
- For the locked-key case (LCK-01), lock a key in the system dconf database as
  root first, then pass it in `GSCTL_LOCKED_KEY`:

      # as root, on a system whose /etc/dconf/profile/user lists system-db:local
      mkdir -p /etc/dconf/db/local.d/locks
      printf "[org/gnome/desktop/interface]\nclock-format='12h'\n" > /etc/dconf/db/local.d/50-gsctl-regress
      echo /org/gnome/desktop/interface/clock-format > /etc/dconf/db/local.d/locks/gsctl-regress
      dconf update

      # then, as the user
      GSCTL_LOCKED_KEY="org.gnome.desktop.interface clock-format" python3 regression.py ...

  Remove both files and run `dconf update` again afterwards.
- The NET cases fetch the website and the latest release; they're skipped
  offline.

## Environments

| Environment | Why | Status |
|---|---|---|
| Fedora Workstation 44 (GNOME 50), live session in a VM | Stock GNOME; GNOME Classic's mode file installed; SELinux; Ptyxis | Run for every release |
| Ubuntu 26.04 (GNOME 50) | Where gsctl is developed; Ubuntu's session mode and bundled extensions | Run for every release, in a VM or a spare login |
| Arch Linux (latest GNOME) | Newest GLib and GNOME | Before a minor release |
| Fedora pre-release (next GNOME) | Catch schema changes early | When a beta is out |

CI already builds and runs the hermetic tests on Ubuntu 22.04 and 24.04,
Fedora and Arch.

## Pass criteria

- No FAIL or ERROR. A SKIP needs a reason that holds for that environment (no
  range on a key, no extensions installed, offline).
- Every touched key restored (`restored N keys`, no restore failures).
- A new bug found here gets a hermetic cargo test too, where one is possible.

## Cases

### INST: Binary, help, man page, completions

| Case | Checks |
|---|---|
| INST-01 | --version prints gsctl and a semver |
| INST-02 | --help lists the commands and exit codes |
| INST-03 | set --help has an example per value type |
| INST-04 | man prints a roff page |
| INST-05 | completions for bash, zsh and fish; others are a usage error |
| INST-06 | commands describes every subcommand, exit code and error code |
| INST-07 | an unknown subcommand is a usage error as JSON on stderr |

### DOC: doctor

| Case | Checks |
|---|---|
| DOC-01 | doctor in the live session: ok, no errors |
| DOC-02 | doctor reads the session mode from the environment, not a guess |
| DOC-03 | doctor without a session bus reports an error and still exits 0 |
| DOC-04 | doctor with the memory backend reports settings_backend as an error |
| DOC-05 | doctor warns when the desktop schemas can't be found |
| DOC-06 | doctor text output ends with a verdict line |

### SRCH: search

| Case | Checks |
|---|---|
| SRCH-01 | 'dark mode' finds color-scheme first |
| SRCH-02 | 'animations' finds enable-animations in the top 3 |
| SRCH-03 | --limit caps results; total counts all matches |
| SRCH-04 | --schema restricts hits to that schema |
| SRCH-05 | a query with no match returns zero results, exit 0 |
| SRCH-06 | installed extensions are search hits by name |
| SRCH-07 | hits carry type, value, default, range and guarded |
| SRCH-08 | text output has one line per hit and a count line |

### SCH: schema

| Case | Checks |
|---|---|
| SCH-01 | schema KEY gives type, range, default, value and writability |
| SCH-02 | schema SCHEMA lists its keys |
| SCH-03 | schema.key and /dconf/path/key work as one argument |
| SCH-04 | unknown schema and key are not_found (exit 3) |
| SCH-05 | guarded keys show their reason; the keyboard floor shows its minimum |
| SCH-06 | code-running keys are marked executes |

### GET: get and output format

| Case | Checks |
|---|---|
| GET-01 | get returns value and is_default, matching gsettings |
| GET-02 | --path on a fixed schema is a usage error |
| GET-03 | a malformed --path is a usage error |
| GET-04 | GSCTL_FORMAT=text gives text; --format json overrides it |

### SET: set: value parsing and validation

| Case | Checks |
|---|---|
| SET-01 | set writes, reads back, journals, and gsettings agrees |
| SET-02 | --dry-run writes nothing |
| SET-03 | booleans accept true/false, yes/no, on/off |
| SET-04 | an invalid enum value is invalid_value (4) with the allowed choices |
| SET-05 | doubles: in range ok, out of range, NaN, inf and 1e400 refused |
| SET-06 | integers: a value outside the schema's range is refused |
| SET-07 | a value of the wrong type is invalid_value |
| SET-08 | strings take bare text, including spaces and the empty string |
| SET-09 | string lists: JSON, one-item shorthand, and [] |
| SET-10 | GVariant text works for tuple arrays (guarded key, with --yes) |
| SET-11 | setting the current value reports changed: false |
| SET-12 | set reset-to-default via reset; reset --dry-run writes nothing |

### GRD: Guards

| Case | Checks |
|---|---|
| GRD-01 | a guarded key without --yes is refused (6); --dry-run reports requires_yes |
| GRD-02 | a guarded key with --yes is written |
| GRD-03 | keyboard delay: below the floor needs --yes, at or above doesn't |
| GRD-04 | other guarded keys from the review are guarded |
| GRD-05 | a guarded write says why, in the error |

### JRN: Journal, history and revert

| Case | Checks |
|---|---|
| JRN-01 | history lists gsctl's writes, newest first, and filters by key |
| JRN-02 | revert by id restores the old value |
| JRN-03 | revert last walks back like an undo stack |
| JRN-04 | revert last after setting the same value twice undoes the real change |
| JRN-05 | revert refuses (7) when the key changed since; --force overrides |
| JRN-06 | revert of an unknown id is entry_not_found (3) |
| JRN-07 | revert --dry-run writes nothing |
| JRN-08 | the journal is 0600 in a 0700 dir; a loosened one is tightened |
| JRN-09 | a symlinked journal isn't written through; the write reports journal_error |
| JRN-10 | secrets are redacted, not journaled, and can't be reverted |

### KEY: Keyboard shortcuts

| Case | Checks |
|---|---|
| KEY-01 | keys list and find agree on a stock binding |
| KEY-02 | a free accelerator is reported free |
| KEY-03 | keys add: dry-run marks executes; without --yes refused (6) |
| KEY-04 | keys add --yes creates a working binding gsettings can see |
| KEY-05 | keys add refuses a taken accelerator or name (7); --force adds anyway |
| KEY-06 | keys remove by name needs no --yes; reverting it does |
| KEY-07 | undoing keys add needs no --yes |
| KEY-08 | built-in shortcuts are set as string arrays and reset |
| KEY-09 | keys conflicts runs and returns a count |

### EXT: Extensions

| Case | Checks |
|---|---|
| EXT-01 | ext list reports the session mode from the environment |
| EXT-02 | ext info shows an extension's schemas; an unknown uuid is not found (3) |
| EXT-03 | ext enable needs --yes; disable doesn't; undoing a disable does |
| EXT-04 | a user extension's own schemas are indexed, including GSConnect-style ids |
| EXT-05 | an extension can't redefine a system schema |
| EXT-06 | extension keys named like commands need --yes |

### CHG: changed

| Case | Checks |
|---|---|
| CHG-01 | changed lists a key gsctl changed, and --schema filters |
| CHG-02 | changed redacts secrets |

### EXP: export and apply

| Case | Checks |
|---|---|
| EXP-01 | export writes one private JSON file per area, without remote settings |
| EXP-02 | apply restores exported values and only writes what differs |
| EXP-03 | apply validates everything first: one bad value writes nothing |
| EXP-04 | apply of a guarded key needs --yes |
| EXP-05 | apply follows a symlink to a file you own (GNU Stow) |
| EXP-06 | apply refuses a symlink to someone else's file, a FIFO, and a huge file |
| EXP-07 | apply refuses a file that isn't a gsctl state file |
| EXP-08 | apply --prune --dry-run lists customised keys the files don't |

### WAT: watch

| Case | Checks |
|---|---|
| WAT-01 | watch reports a change made by another program |
| WAT-02 | watch --timeout ends with a timeout event, exit 0 |
| WAT-03 | watch redacts secrets |

### SES: No session

| Case | Checks |
|---|---|
| SES-01 | without a session bus, writes fail with no_session (5) instead of vanishing |
| SES-02 | with the memory backend, writes fail with no_session (5) |

### LCK: Locked keys

| Case | Checks |
|---|---|
| LCK-01 | a key locked by the system dconf database is not_writable (5) |

### OUT: Output and robustness

| Case | Checks |
|---|---|
| OUT-01 | text output escapes control characters in values |
| OUT-02 | errors are JSON on stderr when it isn't a terminal; text with --format text |
| OUT-03 | a closed pipe doesn't panic |
| OUT-04 | areas describes every export area |

### NET: Published docs and release (needs network)

| Case | Checks |
|---|---|
| NET-01 | agents.md, llms.txt and the skill are published |
| NET-02 | agents.md's commands match this gsctl (doctor verdict, search flags) |
| NET-03 | the latest release is this version |

## Results

| Date | gsctl | Environment | Result |
|---|---|---|---|
| 2026-10-11 | 0.1.1 | Fedora 44 live VM, GNOME 50 | 91 passed, 4 failed, 1 skipped. Bugs: a no-op `set` took the next `revert last` (JRN-04); `gsctl … \| head` panicked on the closed pipe (OUT-03). The other failures and the skip were mistakes in the cases, since fixed |
| 2026-10-11 | f4c3385 + both fixes | Fedora 44 live VM, GNOME 50 | 96 passed, 20 keys restored |
