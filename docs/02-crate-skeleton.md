# Step 2 — Library crate, result model, JSON schema, CLI shell

**Goal:** The public surface and the contracts every check plugs into — before
any real check exists.

**Depends on:** Step 1 (IR).

## Why

Locking the result model and JSON schema first means every check from Step 3 on
emits a uniform, stable, CI-consumable result. The CLI and library are the two
delivery surfaces; JSON is the integration contract (no Python bindings in v1).

## Tasks

- [x] Cargo workspace: a **library crate** (the engine) + a thin **binary
      crate** (`seq-validate` CLI). → `crates/seq-validate` added to the
      workspace as a thin shell over `seq-validate-core`.
- [x] Define the result model:
      `CheckResult { id, status, measured, expected, severity, message }` where
      `status ∈ {pass, fail, warn, skip}` and `severity ∈ {error, warn, info}`.
      → `core::result` (`Status`/`Severity` enums, `pass`/`fail`/`warn`/`skip`
      builders, `with_measured`/`with_expected`/`with_severity`). `measured`/
      `expected` are `serde_json::Value` for type-flexibility.
- [x] Define a `Report` aggregating per-check results + sequence-level metadata
      (file, version, name, parse stats). → `core::report` (`Report` +
      `SequenceMeta` + `Summary`). One `Report` type covers success and the
      harness-error case (`error`/`sequence` nullable) so `--json` is uniform.
- [x] Define the **check unit** abstraction (a discrete, registrable unit —
      keep it simple, e.g. a function/trait returning `CheckResult`s). No
      plugin/dynamic-loading machinery yet (deferred), but discrete enough to
      extract later. → `core::checks` (`Check` trait + `CheckCtx` + `registry()`
      + `run_all()`). The registry is intentionally **empty**; Steps 3–6 fill it.
- [x] Serialize `Report` to **stable JSON** (versioned schema; document field
      meanings). This is the integration contract — treat changes as breaking.
      → serde-derived JSON, `SCHEMA_VERSION = 1`, contract pinned by
      `crates/seq-validate-core/schema/report-v1.schema.json` (every field
      documented) and validated in tests.
- [x] Human-readable report renderer (grouped by category, colorized
      pass/fail/warn/skip). → `core::render` (zero-dep ANSI; `Category` derived
      from the `id` prefix so results group without bloating the result model).
- [x] CLI shell: `seq-validate <file.seq> [--json] [--profile <name>]
      [--spec <spec.yaml>]`. Wire arg parsing; checks come later. → `clap`
      derive; `--profile`/`--spec` are parsed and print an inactivity note
      until Steps 6/7 consume them.
- [x] **Exit-code policy:** `0` if no `fail`; nonzero on any `fail`; distinct
      code for harness/parse error. `warn`/`skip` do not fail the run. →
      `Report::exit_code()` (`2` harness/parse error, `1` any `fail`, else `0`).

## Acceptance criteria — met

- `seq-validate example.seq` runs end-to-end and prints a report with zero
  checks (empty but well-formed). → `crates/seq-validate/tests/cli.rs`.
- `--json` emits schema-valid JSON validated by a test. →
  `crates/seq-validate-core/tests/report_schema.rs` (validates success, empty,
  and harness-error payloads against the schema; round-trips through serde).
- Exit codes behave per policy against synthetic results. →
  `crates/seq-validate-core/tests/result_model.rs`.

## References

- pulsepal harness result/exit conventions (`param_check.py`: exit 0 pass /
  1 fail / 2 harness error; `skip` for inapplicable fields) — mirror this.

## Risks / notes

- Resist building the plugin boundary here. Discrete check units are enough;
  extraction is a later, cheap refactor (per the deferred-modularity decision).
- Version the JSON schema from day one so downstream consumers can pin it.
