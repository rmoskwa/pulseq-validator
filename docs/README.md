# docs/

Design notes and a captured backlog of improvement ideas for `pulseq-validator`.
Nothing here is scheduled work; these are documented action items that a future
engineer can pick up and implement as discrete tasks.

## Contents

- [`tasks/`](tasks/) — backlog of improvement tasks, **one file per task**.
  Start at [`tasks/README.md`](tasks/README.md) for the shared context and the
  index. Captured from a design review on 2026-06-17 of how the validator is
  consumed by AI coding agents.

## Conventions for task files

Each task file is self-contained so it can be implemented (or imported into an
issue tracker) without re-deriving the context:

- A metadata header — ID, priority, effort, status, area.
- **Why it matters** — the gap, framed from the consumer's point of view.
- **Proposed approach** — concrete, with `file:line` pointers into the current
  tree. A starting point, not a contract; confirm before editing.
- **Acceptance criteria** — what "done" looks like, including the repo's standard
  gate: `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` all green (see [`CLAUDE.md`](../CLAUDE.md)).
- **Scope / non-goals** — what to deliberately leave out, so a task doesn't grow.

Priorities: **P1** (highest leverage), **P2**, **P3**. Effort: **S / M / L**.
</content>
