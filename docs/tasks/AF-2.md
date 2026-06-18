# AF-2 — Publish a spec JSON Schema and emit it from the CLI

| | |
|---|---|
| **ID** | AF-2 |
| **Priority** | P2 |
| **Effort** | M |
| **Status** | ☑ Done |
| **Area** | Spec input contract · discoverability |

> Part of the [agent-facing backlog](README.md). See the index for shared context.

## Why it matters

The contract is asymmetric. The **report** (output) has a published, versioned
JSON Schema (`crates/seq-validate-core/schema/report-v1.schema.json`) referenced
from the README — a machine can learn how to *read* the validator. The **spec**
(input) has no schema; its format lives only in the README, the example fixture,
and Rust doc-comments. An agent operating in a harness without the README in its
context cannot introspect the spec format from the binary, so it cannot reliably
author a spec from the tool alone.

## Proposed approach

- Author `crates/seq-validate-core/schema/spec-v1.schema.json` describing the
  spec YAML (fields, types, units in `description`, the `[x, y, z]` geometry
  vectors, the `tolerances` sub-map shape, and the `none` / null opt-out). Mirror
  the structure of `Spec` (`crates/seq-validate-core/src/spec.rs:100`) and
  `default_tolerance` (`spec.rs:82`).
- Add a CLI flag to emit it, e.g. `seq-validate --emit-spec-schema` (and
  optionally `--emit-report-schema`) that prints the embedded schema JSON and
  exits 0. Wire into `Cli` in `crates/seq-validate/src/main.rs:34`. Embed the
  schema with `include_str!` so the binary is self-contained.
- Add a test that the emitted schema parses as JSON and that the example fixture
  `fixtures/t1_spgr_axial_brain.spec.yaml` validates against it (a JSON-Schema
  validator dev-dependency, or a hand-rolled structural check if a dep is
  unwanted — match the repo's existing dependency posture).

## Acceptance criteria

- `seq-validate --emit-spec-schema` prints a valid JSON Schema and exits 0.
- The bundled example spec validates against the emitted schema (test).
- README's expected-spec section links the new schema next to the report schema.
- `cargo fmt` / `clippy` / `test` all green.

## Scope / non-goals

- Keep it descriptive of the *current* fields only; new fields are
  [AF-7](AF-7.md).
- No code generation of the schema from the Rust types unless trivial — a
  hand-written, tested schema is acceptable and simpler.
</content>
