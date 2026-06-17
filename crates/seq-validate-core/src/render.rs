//! Human-readable rendering of a [`Report`], grouped by [`Category`] and
//! optionally colorized.
//!
//! [`render`] returns the report as a `String` (the CLI prints it) so it is
//! straightforward to assert on in tests. Color is a caller decision: pass
//! `color = false` for pipes/CI and plain-text tests, `true` for a TTY. The
//! ANSI codes are hand-rolled (no dependency); when `color` is false every
//! painter is the identity, so the output is clean text.

use std::fmt::Write;

use crate::report::Report;
use crate::result::{Category, CheckResult, Status};

const RESET: &str = "\x1b[0m";

/// Render `report` to a string, with ANSI color iff `color`.
pub fn render(report: &Report, color: bool) -> String {
    let mut out = String::new();

    // Header: file, then sequence identity + parse stats (when parsed).
    let _ = writeln!(out, "{}", report.file);
    if let Some(seq) = &report.sequence {
        let name = seq.name.as_deref().unwrap_or("(unnamed)");
        let _ = writeln!(out, "Pulseq {} · {}", seq.pulseq_version, name);
        let _ = writeln!(out, "{} blocks · {:.3} s", seq.blocks, seq.duration_s);
        for w in &seq.parse_warnings {
            let _ = writeln!(
                out,
                "{}",
                paint(&format!("parse warning: {w}"), "33", color)
            );
        }
    }
    let _ = writeln!(out);

    // A harness/parse error short-circuits: there are no results to group.
    if let Some(err) = &report.error {
        let _ = writeln!(out, "{}", paint(&format!("error: {err}"), "1;31", color));
        let _ = writeln!(out);
        return out;
    }

    if report.results.is_empty() {
        let _ = writeln!(out, "No checks run.");
        let _ = writeln!(out);
    } else {
        // Pad ids to a common width so messages line up across categories.
        let id_width = report.results.iter().map(|r| r.id.len()).max().unwrap_or(0);
        for &cat in Category::DISPLAY_ORDER {
            let group: Vec<&CheckResult> = report
                .results
                .iter()
                .filter(|r| Category::from_id(&r.id) == cat)
                .collect();
            if group.is_empty() {
                continue;
            }
            let _ = writeln!(out, "{}", paint(cat.title(), "1", color));
            for r in group {
                render_result(&mut out, r, id_width, color);
            }
            let _ = writeln!(out);
        }
    }

    let s = &report.summary;
    let _ = writeln!(
        out,
        "Summary: {} passed, {} failed, {} warnings, {} skipped",
        count(s.pass, "32", color),
        count(s.fail, "31", color),
        count(s.warn, "33", color),
        count(s.skip, "90", color),
    );
    out
}

/// Render one result line: `  LABEL  id            message  [measured=… expected=…]`.
fn render_result(out: &mut String, r: &CheckResult, id_width: usize, color: bool) {
    let label = paint(status_label(r.status), status_color(r.status), color);
    let mut line = format!(
        "  {label}  {id:<id_width$}  {msg}",
        id = r.id,
        msg = r.message
    );

    let mut extra = Vec::new();
    if let Some(m) = &r.measured {
        extra.push(format!("measured={}", compact(m)));
    }
    if let Some(e) = &r.expected {
        extra.push(format!("expected={}", compact(e)));
    }
    if !extra.is_empty() {
        let _ = write!(line, "  [{}]", extra.join(" "));
    }
    let _ = writeln!(out, "{line}");
}

/// Fixed-width status tag.
fn status_label(status: Status) -> &'static str {
    match status {
        Status::Pass => "PASS",
        Status::Fail => "FAIL",
        Status::Warn => "WARN",
        Status::Skip => "SKIP",
    }
}

/// ANSI SGR color code per status: green / red / yellow / gray.
fn status_color(status: Status) -> &'static str {
    match status {
        Status::Pass => "32",
        Status::Fail => "31",
        Status::Warn => "33",
        Status::Skip => "90",
    }
}

/// Color a count only when it is nonzero (a plain `0` reads as muted on its own).
fn count(n: usize, code: &str, color: bool) -> String {
    if n > 0 {
        paint(&n.to_string(), code, color)
    } else {
        n.to_string()
    }
}

/// Compact one-line JSON for a measured/expected value.
fn compact(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

/// Wrap `s` in an ANSI SGR sequence when `color`, otherwise return it as-is.
fn paint(s: &str, code: &str, color: bool) -> String {
    if color {
        format!("\x1b[{code}m{s}{RESET}")
    } else {
        s.to_string()
    }
}
