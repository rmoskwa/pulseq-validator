# AF-4 — Make parse-error output actionable

| | |
|---|---|
| **ID** | AF-4 |
| **Priority** | P2 |
| **Effort** | M |
| **Status** | ☐ Not started |
| **Area** | Output contract · error reporting |

> Part of the [agent-facing backlog](README.md). See the index for shared context.

## Why it matters

The actionability gradient is backwards from what the loop needs. A subtle
hardware violation gets a beautiful message ("GZ peak gradient 52.3 mT/m (block
1234) exceeds maxGrad 50.0 mT/m"), but the *most basic* mistake — emitting an
unparseable `.seq` — surfaces raw parser debug output, e.g.
`Syntax error in pulseq file: Parsing Error: ContextError { context:
[Label("[VERSION] section")], cause: None }`. The agent that needs the clearest
feedback gets the least.

## Proposed approach

- The parse error is produced at `Sequence::from_file` and wrapped verbatim into
  `Report::harness_error` at `crates/seq-validate/src/main.rs:85`.
- Translate the parser error into a human one-line summary before it reaches
  `harness_error` — at minimum, strip the Rust-debug framing and name the
  section/line the parser was in (the `Label(...)` context often already
  identifies it, e.g. "expected a `[VERSION]` section"). Ideally include the line
  number if `pulseq-parse` exposes it.
- Keep the raw error available too if useful (e.g. a structured `error` plus a
  human summary), but do not regress the single-shape JSON contract: `error`
  stays a string, `sequence` stays null, exit code stays `2`
  (`crates/seq-validate-core/src/report.rs:139`).
- Investigate whether `crates/pulseq-parse` can return a typed error with
  section + position rather than a stringly-typed debug blob; if cheap, that is
  the cleaner fix and benefits all consumers.

## Acceptance criteria

- Feeding a `.seq` with a malformed/missing `[VERSION]` section yields a
  human-readable one-line `error` (no `ContextError { … }` debug formatting),
  still exit `2`, still the uniform JSON shape.
- A test covering at least one malformed-file case asserting the friendlier
  message.
- `cargo fmt` / `clippy` / `test` all green.

## Scope / non-goals

- Do not attempt to *recover* from parse errors or partially validate a broken
  file — only improve the message.
</content>
