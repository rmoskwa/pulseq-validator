# Agent-facing improvements — backlog index

Captured 2026-06-17 from a review of how this repo is designed for AI-agent
consumption (the loop where an agent writes a `.seq`, validates it, reads the
report, and iterates). One file per task; pick any up independently.

## Context for the reader

The primary purpose of this repo is to be a validator that an AI coding agent (or
an agent harness / Claude Code skill) calls as a feedback loop while authoring a
Pulseq sequence. The agent emits a `.seq` file and an optional expected-value
`.spec.yaml`, runs `seq-validate`, and reads the report to fix its work. The
output contract — uniform `CheckResult` model, versioned JSON, three-valued exit
codes, `skip` as a first-class non-failing status — is already well-suited to
that loop. The tasks below are the gaps found in that review, ranked by how much
they affect the agent loop.

## Index

| ID | Title | Priority | Effort |
|----|-------|----------|--------|
| [AF-1](AF-1.md) | Warn on unrecognized spec keys | **P1** | S–M |
| [AF-2](AF-2.md) | Publish a spec JSON Schema + emit it from the CLI | P2 | M |
| [AF-3](AF-3.md) | Add an agent-facing usage doc (AGENTS.md) | P2 | S |
| [AF-4](AF-4.md) | Make parse-error output actionable | P2 | M |
| [AF-5](AF-5.md) | Structured remediation field, or drop the "hint" framing | P3 | S–M |
| [AF-6](AF-6.md) | Make the check catalog discoverable (`--list-checks`/`--explain`) | P3 | M |
| [AF-7](AF-7.md) | Document & revisit the spec expressiveness ceiling | P3 | S |

Status legend used in each file: ☐ not started.
</content>
