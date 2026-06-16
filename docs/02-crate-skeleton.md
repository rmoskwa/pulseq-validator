# Step 2 — Library crate, result model, JSON schema, CLI shell

**Goal:** The public surface and the contracts every check plugs into — before
any real check exists.

**Depends on:** Step 1 (IR).

## Why

Locking the result model and JSON schema first means every check from Step 3 on
emits a uniform, stable, CI-consumable result. The CLI and library are the two
delivery surfaces; JSON is the integration contract (no Python bindings in v1).

## Tasks

- [ ] Cargo workspace: a **library crate** (the engine) + a thin **binary
      crate** (`seq-validate` CLI).
- [ ] Define the result model:
      `CheckResult { id, status, measured, expected, severity, message }` where
      `status ∈ {pass, fail, warn, skip}` and `severity ∈ {error, warn, info}`.
- [ ] Define a `Report` aggregating per-check results + sequence-level metadata
      (file, version, name, parse stats).
- [ ] Define the **check unit** abstraction (a discrete, registrable unit —
      keep it simple, e.g. a function/trait returning `CheckResult`s). No
      plugin/dynamic-loading machinery yet (deferred), but discrete enough to
      extract later.
- [ ] Serialize `Report` to **stable JSON** (versioned schema; document field
      meanings). This is the integration contract — treat changes as breaking.
- [ ] Human-readable report renderer (grouped by category, colorized
      pass/fail/warn/skip).
- [ ] CLI shell: `seq-validate <file.seq> [--json] [--profile <name>]
      [--spec <spec.yaml>]`. Wire arg parsing; checks come later.
- [ ] **Exit-code policy:** `0` if no `fail`; nonzero on any `fail`; distinct
      code for harness/parse error. `warn`/`skip` do not fail the run.

## Acceptance criteria

- `seq-validate example.seq` runs end-to-end and prints a report with zero
  checks (empty but well-formed).
- `--json` emits schema-valid JSON validated by a test.
- Exit codes behave per policy against synthetic results.

## References

- pulsepal harness result/exit conventions (`param_check.py`: exit 0 pass /
  1 fail / 2 harness error; `skip` for inapplicable fields) — mirror this.

## Risks / notes

- Resist building the plugin boundary here. Discrete check units are enough;
  extraction is a later, cheap refactor (per the deferred-modularity decision).
- Version the JSON schema from day one so downstream consumers can pin it.
