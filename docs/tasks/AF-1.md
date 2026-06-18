# AF-1 — Warn on unrecognized spec keys

| | |
|---|---|
| **ID** | AF-1 |
| **Priority** | P1 (highest leverage) |
| **Effort** | S–M |
| **Status** | ☑ Done |
| **Area** | Spec input contract |

> Part of the [agent-facing backlog](README.md). See the index for shared context.

## Why it matters

The spec parser silently ignores any key it does not recognize. This is
deliberate (it lets an agent embed free-form `name:` / `acquisition:` / `notes:`
authoring guidance), but it has a sharp failure mode: if an agent writes a *known
assertion under the wrong name* — `tr: 400` instead of `tr_ms`, `flipAngle`
instead of `flip_angle_deg`, `fov:` instead of `fov_mm:` — that assertion
silently becomes a **no-op and the run passes green**. A typo'd assertion is
indistinguishable from a satisfied one. For a verification loop this is the worst
failure mode: the tool actively tells the agent it succeeded. Agents typo field
names frequently, and the lenient policy converts those typos into false
confirmations.

## Proposed approach

Emit a `spec.unrecognized_fields` result with `status: warn` (so it is visible in
the report and JSON but does **not** change the exit code or the lenient
semantics) that lists the unknown top-level keys, ideally with a nearest-match
suggestion against the known field set.

- The known asserted keys are read in `Spec::from_yaml_str`
  (`crates/seq-validate-core/src/spec.rs:145`): `te_ms`, `tr_ms`,
  `flip_angle_deg`, `n_slices`, `echo_spacing_ms`, `fov_mm`, `matrix`,
  `oversampling`, `scanner`, `tolerances`.
- Maintain an **allowlist** that also contains the convention free-form blocks
  the example fixture uses (`name`, `acquisition`, `notes`) so they do not warn.
  Decide explicitly whether unknown keys warn or are silent; the recommendation
  is: warn on keys that are *near* a known key (likely typos), stay silent on
  clearly-free-form blocks — or simpler, warn on everything not in the allowlist
  and document that free-form notes belong under a `notes:` block.
- **Design note (not a local change):** `from_yaml_str` currently returns only
  `Result<Spec, String>` (`spec.rs:136`), with no channel for non-fatal warnings.
  To surface this as a `warn` *result* rather than a hard error, the unknown keys
  must flow to where results are assembled. Two viable shapes:
  1. Change the parse signature to return the unknown keys alongside the spec
     (e.g. `Result<(Spec, Vec<String>), String>`), and have `build_report` in
     `crates/seq-validate/src/main.rs:82` turn them into a
     `CheckResult::warn("spec.unrecognized_fields", …)` appended to `results`.
  2. Have `Spec` retain the unknown keys as a field and let `Spec::assert`
     (`spec.rs`, consumed at `main.rs:110`) emit the warning result.
  Option 1 keeps `Spec` a pure parsed-value type and is preferred.
- Use the existing nearest-match helper style if one exists; otherwise a simple
  Levenshtein/edit-distance over the known-key set is enough for "did you mean".

## Acceptance criteria

- A spec containing `tr: 400` (typo) produces a `spec.unrecognized_fields` `warn`
  result naming `tr` and suggesting `tr_ms`; exit code is unchanged by the warn.
- A spec containing only recognized keys plus an allowlisted free-form block
  (`name:`, `notes:`) produces **no** unrecognized-fields warning.
- The lenient assertion semantics are otherwise unchanged (absent known fields
  still skip; provided known fields still assert).
- Unit test in `spec.rs` covering: a typo'd key warns, an allowlisted key is
  silent, and a clean spec emits no such result.
- `cargo fmt` / `clippy` / `test` all green.

## Scope / non-goals

- Do **not** make unknown keys a hard error — that would break the deliberate
  free-form-notes affordance.
- Do not attempt to validate *values* of free-form blocks.
</content>
