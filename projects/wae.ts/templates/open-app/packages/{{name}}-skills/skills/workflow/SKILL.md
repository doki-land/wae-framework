---
name: workflow
description: Inspect a directory and plan an install or upgrade of {{nameTitle}} without touching host state.
---

# {{nameTitle}} agent workflow

This skill drives the `{{name}}` CLI. The CLI is non-interactive and every
command returns a stable exit code:

- `0` success
- `1` runtime failure
- `2` usage or validation failure
- `3` structured capability failure (`unsupported_platform`, `not_implemented`,
  `dependency_blocked`)

Prefer `--json` and read the `{ "ok": true, "data": ... }` or
`{ "ok": false, "error": ... }` envelope. Do not parse human prose.

## Read-only discovery

1. Read the capability table. A capability whose `state` is not `null` is
   unavailable and must not be attempted.

   `{{name}} capabilities --json`

2. Inspect a bounded directory tree. `--limit` keeps the report small.

   `{{name}} inspect --root <path> --limit <n> --json`

## Install or upgrade

`install` is idempotent: it plans a config write, an install marker, and a
shortcut under the platform standard locations. It never enables auto-start and
never requests silent elevation.

3. Preview the plan. `--no-shortcut` skips shortcut creation and
   `--reset-config` rewrites configuration instead of reusing it.

   `{{name}} install --dry-run --no-shortcut --reset-config --json`

4. Show the plan to the user. Do not apply it without explicit confirmation.

   `{{name}} install --actor <id> --json`

## Uninstall

`uninstall` removes the shortcut, install marker, and configuration unless
`--keep-config` is set.

5. Preview removal.

   `{{name}} uninstall --dry-run --keep-config --json`

6. Apply after explicit confirmation.

   `{{name}} uninstall --actor <id> --json`

## Audit

Every install, upgrade, and uninstall call is recorded by the shared audit
facility. Use `--actor-kind agent` and a stable `--actor` id so the records are
attributable.

   `{{name}} audit --kind <user|agent> --limit <n> --json`

## Safety

- Mutating commands are `install` and `uninstall`. Always run the `--dry-run`
  form first and show the complete action list to the user.
- Do not execute a mutating command without explicit confirmation（确认）from the
  user after the dry-run preview.
- Do not retry a structured capability failure with elevated privileges. Report
  the `code`, `capability`, and `platform` fields instead.
- Do not edit host configuration files directly. The CLI and audit records are
  the only supported path.
- Treat `--json` output as the contract. Exit code `3` means the capability is
  unavailable on this platform or is intentionally not implemented yet.
