# AF-7 — Document & revisit the spec expressiveness ceiling

| | |
|---|---|
| **ID** | AF-7 |
| **Priority** | P3 |
| **Effort** | S (doc) |
| **Status** | ☐ Not started |
| **Area** | Documentation · scope |

> Part of the [agent-facing backlog](README.md). See the index for shared context.

## Why it matters

The spec encodes ~11 scalar/geometry fields (TE, TR, flip, slices, echo spacing,
matrix, FOV, oversampling). It cannot express block-level assertions,
sequence-type constraints ("must be spin-echo"), multi-echo TE₁/TE₂, b-values,
etc. This is a deliberate, defensible v1 scope decision (consistent with the
static/analytic charter — see README "Scope"). But the "planned contract" framing
implies an agent can encode its full design intent, when it can encode only a
slice. The mismatch should at least be *named* so agents (and their authors)
calibrate expectations.

## Proposed approach

- Short term (this is the actual task): add a "What a spec can and cannot
  express" subsection to the README expected-spec section (and/or the AGENTS.md
  from [AF-3](AF-3.md)), listing the supported fields and explicitly the common
  things that are out of scope.
- Longer term (capture only, do not implement here): note candidate extensions
  worth weighing against the static/analytic charter if demand appears — e.g.
  multi-echo TE list, a sequence-family/type assertion, per-axis acceleration
  factor. Each would be its own future task with its own measurement support.

## Acceptance criteria

- README (and AGENTS.md if present) clearly enumerates supported spec fields and
  states the out-of-scope categories.
- No behavior change.

## Scope / non-goals

- This item is documentation only. Adding new spec fields is explicitly **not**
  in scope here — each would be a separate, measurement-backed task.
</content>
