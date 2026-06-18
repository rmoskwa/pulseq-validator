# AF-5 — Structured remediation, or stop calling the message a "hint"

| | |
|---|---|
| **ID** | AF-5 |
| **Priority** | P3 |
| **Effort** | S–M |
| **Status** | ☐ Not started |
| **Area** | Output contract |

> Part of the [agent-facing backlog](README.md). See the index for shared context.

## Why it matters

The result model carries fix guidance only inside the prose `message`
(`crates/seq-validate-core/src/result.rs:51`). There is no dedicated
`hint` / `remediation` / `docs_url` field. It works because the messages are
good, but an agent extracting "how to fix this" is parsing prose, which is less
reliable than reading a field. Separately, design/README language describes the
output as carrying a "remediation hint" — which implies structure that does not
exist. Pick one of two directions and make the docs match reality.

## Proposed approach

Either:

- **(a) Add structure** — an optional `hint: Option<String>` (and/or `docs_url`)
  on `CheckResult`. This is a JSON-contract change: bump `SCHEMA_VERSION`
  (`crates/seq-validate-core/src/report.rs:27`), update
  `schema/report-v1.schema.json` → `report-v2.schema.json`, and populate hints on
  the highest-value checks first (hardware limits, raster alignment, spec
  failures). Update the builder API (`result.rs:71`–`125`) with a
  `.with_hint(...)`.
- **(b) Align the docs** — if a separate field is not wanted, scrub "remediation
  hint" phrasing from README/design notes so consumers don't expect a field that
  isn't there, and document that the fix guidance lives in `message`.

Recommendation: start with (b) (cheap, removes a false promise), and only do (a)
if agent testing shows prose-parsing is unreliable in practice.

## Acceptance criteria

- If (a): `schema_version` bumped, new schema file added, schema test updated,
  hints present on the chosen checks, JSON still validates.
- If (b): no occurrence of "remediation hint" implying a field; docs state the
  guidance is in `message`.
- `cargo fmt` / `clippy` / `test` all green.

## Scope / non-goals

- Do not add hints to every check at once; if (a), seed the high-value ones and
  leave the rest `null`.
</content>
