//! Parse every `*.seq` under `fixtures/` through the full IR. Guards that the
//! pulseq-parse parser handles each bundled example (trapezoid + free gradients,
//! rotation/label extensions) end to end — and, crucially, that it **never
//! panics**: an unsupported construct must surface as a recoverable `Error`.
//!
//! `sos-liver.seq` currently exercises a rotation case the interpreter doesn't
//! implement yet (rotating gradient axes with different shapes/delays). Until
//! Step 5 implements it, that file is expected to degrade gracefully to an
//! error rather than crash. When Step 5 lands, this test will flag it so the
//! expectation can be flipped.

use std::time::Instant;

use seq_validate_core::Sequence;

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures");

/// Fixtures we expect to fail *gracefully* (clean `Err`, no panic) for now.
const EXPECTED_UNSUPPORTED: &[&str] = &["sos-liver.seq"];

#[test]
fn all_fixtures_parse_or_degrade_gracefully() {
    let mut paths: Vec<_> = std::fs::read_dir(FIXTURE_DIR)
        .expect("fixtures/ dir")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().map(|x| x == "seq").unwrap_or(false))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no .seq fixtures found");

    let mut problems = Vec::new();
    for path in &paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let unsupported = EXPECTED_UNSUPPORTED.contains(&name.as_str());
        let t = Instant::now();

        match Sequence::from_file(path) {
            Ok(seq) => {
                eprintln!(
                    "OK   {name:32} v{} blocks={:6} dur={:9.3}s warnings={} ({:?})",
                    seq.version,
                    seq.blocks.len(),
                    seq.total_duration,
                    seq.warnings.len(),
                    t.elapsed(),
                );
                for w in &seq.warnings {
                    eprintln!("        warning: {w}");
                }
                if !seq.warnings.is_empty() {
                    problems.push(format!("{name}: unexpected interpreter warning(s)"));
                }
                if unsupported {
                    problems.push(format!(
                        "{name}: now parses! remove it from EXPECTED_UNSUPPORTED \
                         (Step 5 rotation likely landed)"
                    ));
                }
            }
            Err(e) => {
                let msg = e.to_string();
                eprintln!("ERR  {name:32} {msg}");
                if !unsupported {
                    problems.push(format!("{name}: unexpected error: {msg}"));
                } else if !(msg.contains("rotation") && msg.contains("not supported")) {
                    problems.push(format!("{name}: failed for an unexpected reason: {msg}"));
                }
            }
        }
    }

    assert!(problems.is_empty(), "fixture problems: {problems:#?}");
}
