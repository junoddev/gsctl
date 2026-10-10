# Security

## Stance

gsctl runs with your rights and is not a sandbox. Anything running as you
can call `dconf write` directly, so gsctl's guards (`--yes` for keys that
can lock you out or run code) catch mistakes and give an agent a point to
stop and ask. They are not a security boundary.

Schema descriptions, extension metadata, state files and journal entries are
data, not instructions. An extension can ship any summary text it likes, and
a state file can set any key. Dry-run any file you didn't write
(`gsctl apply FILE --dry-run`) before applying it.

Secrets (string keys named `*password*`) are left out of exports and the
journal, and shown as `{"redacted": true}` (`<redacted>` in text output)
unless you pass `--show-secrets`.

## Agents

The real gate for an agent is the agent's own permission system. For Claude
Code, add this to `~/.claude/settings.json` (or a project's
`.claude/settings.json`):

```json
{
  "permissions": {
    "ask": [
      "Bash(gsctl * --yes)",
      "Bash(gsctl * --yes *)",
      "Bash(gsctl * --show-secrets)",
      "Bash(gsctl * --show-secrets *)"
    ],
    "deny": [
      "Bash(dconf write *)",
      "Bash(dconf load *)",
      "Bash(dconf reset *)",
      "Bash(gsettings set *)",
      "Bash(gsettings reset *)",
      "Bash(gsettings reset-recursively *)"
    ]
  }
}
```

`--yes` is the only way past a guard: there is no short form and no
environment variable. The agent can still make unguarded changes and
dry-runs without asking; anything with `--yes` (or `--show-secrets`) asks you
first, and direct dconf/gsettings writes, which skip gsctl's checks and
journal, are refused.

These are pattern rules over the command text, and best-effort. Claude Code
checks each part of a compound command and matches deny and ask rules past
leading variable assignments, but its docs are explicit that a rule doesn't
match the same program run another way (`/usr/bin/dconf write ...`,
`sh -c '...'`), so it "isn't a security boundary around the program". See
[Configure permissions](https://code.claude.com/docs/en/permissions). If you
need a hard boundary, run the agent in a sandbox without access to your
session bus.

## Scope

In scope:

- Guard bypasses: writing a guarded key without `--yes` through `--path`,
  `revert`, `apply`, `keys`, `ext`, or an extension schema shadowing a system
  one.
- Symlink or permission flaws in the journal (`~/.local/state/gsctl/`) or in
  `export --out` files.
- Terminal escape injection through `--format text` output.
- Secret leakage: a secret value in output, the journal or an export without
  `--show-secrets`.

Out of scope: anything `gsettings` could change that gsctl doesn't claim to
guard.

## Reporting

Use GitHub private vulnerability reporting on
[junoddev/gsctl](https://github.com/junoddev/gsctl) (Security tab, "Report a
vulnerability"). Please don't open a public issue. We aim to fix and
disclose within 90 days of the report.
