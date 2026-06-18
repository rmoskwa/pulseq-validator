# AF-3 — Add an agent-facing usage doc

| | |
|---|---|
| **ID** | AF-3 |
| **Priority** | P2 |
| **Effort** | S |
| **Status** | ☑ Done |
| **Area** | Documentation |

> Part of the [agent-facing backlog](README.md). See the index for shared context.

## Why it matters

The repo's agent-facing intent currently lives in `CLAUDE.md` and project memory,
while the README frames consumption around "CI gating" and "Python / web
consumers." There is no single document that tells an *agent* how to drive the
loop. An agent (or the harness author wiring it in) benefits from one explicit
page.

## Proposed approach

Add `AGENTS.md` at the repo root (or a "For AI agents" section in the README)
that states the loop concretely:

- Always invoke with `--json`; `measured`/`expected` are always present there
  (in the human report they need `--verbose`).
- Branch on the exit code first: `0` valid / `1` a check failed / `2` couldn't
  process the file (`crates/seq-validate-core/src/report.rs:155`).
- Route on `id` + `status`; treat `skip` as "not applicable," never as failure
  (only `status: fail` drives a nonzero exit).
- The full spec field list with units and the lenient policy (only provided
  fields are checked) — cross-link [AF-2](AF-2.md)'s schema once it exists.
- A short worked example: a failing `hardware.rf_b1` result and how to read
  `measured`/`expected`/`message` to fix it.

## Acceptance criteria

- `AGENTS.md` exists and covers: JSON mode, exit codes, `id`/`status` routing,
  `skip` semantics, spec fields + units, one worked fix example.
- README links to it.
- Documentation only; keep any links valid.

## Scope / non-goals

- Documentation only; do not change behavior here.
</content>
