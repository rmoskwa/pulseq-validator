# AF-6 — Make the check catalog discoverable (`--list-checks` / `--explain`)

| | |
|---|---|
| **ID** | AF-6 |
| **Priority** | P3 |
| **Effort** | M |
| **Status** | ☐ Not started |
| **Area** | CLI · discoverability |

> Part of the [agent-facing backlog](README.md). See the index for shared context.

## Why it matters

An agent that receives `trajectory.geometry_agreement: warn` has only the
message; it cannot ask the tool "what is this check, when does it fire, how do I
fix it." The set of check `id`s is not enumerable from the CLI. Message quality
makes this minor, but a discoverable registry helps an agent build a model of the
check space and reduces guesswork.

## Proposed approach

- The check registry already lives in one place: `checks::registry()`
  (`crates/seq-validate-core/src/checks.rs:60`), concatenating the
  `integrity` / `metrics` / `trajectory` / `hardware` modules.
- Give each `Check` a stable `id` and a short `description()` (one line: what it
  verifies, when it skips). Add `seq-validate --list-checks` to print the
  catalog (id + one-liner + category), and optionally `--explain <id>` for a
  longer paragraph.
- Output the catalog in both human and `--json` form for symmetry with the
  report.

## Acceptance criteria

- `seq-validate --list-checks` enumerates every check `id` with a one-line
  description, grouped by category, exit 0.
- The list is generated from the registry (no hand-maintained second copy).
- `cargo fmt` / `clippy` / `test` all green.

## Scope / non-goals

- `--explain <id>` is optional; `--list-checks` is the core deliverable.
- Do not duplicate the spec-field docs here (those belong to
  [AF-2](AF-2.md)/[AF-3](AF-3.md)).
</content>
