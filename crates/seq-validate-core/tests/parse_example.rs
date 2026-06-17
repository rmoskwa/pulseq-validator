//! Acceptance tests for Step 1 (docs/01-vendor-parser.md) against the bundled
//! v1.5.1 example, `fixtures/t1_spgr_axial_brain.seq`:
//!
//!   * the example parses with no errors (and no interpreter warnings),
//!   * a snapshot of block count, definitions, and timing holds,
//!   * the raw layer round-trips (stays addressable),
//!   * parse is sub-second and empirically O(n) in block count.
#![allow(clippy::expect_used)] // test helper `load` intentionally panics on failure

use std::time::Instant;

use seq_validate_core::{Sequence, pulseq_parse, raw_sections};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/t1_spgr_axial_brain.seq"
);

// Known-good snapshot of the example sequence.
const EXPECT_BLOCKS: usize = 50_688;
const EXPECT_TOTAL_DURATION_S: f64 = 76.809_216; // == the `TotalDuration` definition

fn load() -> Sequence {
    Sequence::from_file(FIXTURE).expect("example .seq must parse")
}

#[test]
fn parses_without_error_or_warning() {
    let seq = load();
    assert!(!seq.blocks.is_empty(), "expected a non-empty sequence");
    assert!(
        seq.warnings.is_empty(),
        "expected no interpreter warnings, got: {:?}",
        seq.warnings
    );
}

#[test]
fn snapshot_version_and_definitions() {
    let seq = load();

    // [VERSION] — confirms the v1.5.1 path is exercised end to end.
    assert_eq!(
        (seq.version.major, seq.version.minor, seq.version.revision),
        (1, 5, 1)
    );
    assert_eq!(seq.version.suppl, None);
    assert_eq!(seq.version.to_string(), "1.5.1");

    // Typed definitions.
    assert_eq!(seq.name.as_deref(), Some("t1_spgr"));
    assert_eq!(seq.fov, [0.24, 0.24, 0.003]);
    assert_eq!(seq.time_raster.grad, 4e-6);
    assert_eq!(seq.time_raster.rf, 2e-6);
    assert_eq!(seq.time_raster.adc, 2e-6);
    assert_eq!(seq.time_raster.block, 4e-6);

    // Full [DEFINITIONS] table, verbatim.
    let expected: &[(&str, &str)] = &[
        ("AdcRasterTime", "2e-06"),
        ("BlockDurationRaster", "4e-06"),
        ("FOV", "0.24 0.24 0.003"),
        ("GradientRasterTime", "4e-06"),
        ("Name", "t1_spgr"),
        ("Nslices", "44"),
        ("RadiofrequencyRasterTime", "2e-06"),
        ("SliceThickness", "0.003"),
        ("TotalDuration", "76.809216"),
    ];
    assert_eq!(seq.definitions.len(), expected.len(), "definition count");
    for (k, v) in expected {
        assert_eq!(
            seq.definitions.get(*k).map(String::as_str),
            Some(*v),
            "definition `{k}`"
        );
    }
}

#[test]
fn snapshot_block_count_and_timing() {
    let seq = load();

    assert_eq!(seq.blocks.len(), EXPECT_BLOCKS, "block count");
    assert_eq!(
        seq.starts.len(),
        seq.blocks.len(),
        "starts align with blocks"
    );

    // Absolute start times: begin at zero, monotonic non-decreasing, and each
    // equals the previous start plus the previous block's duration.
    assert_eq!(seq.start(0), Some(0.0));
    let mut t = 0.0;
    for (i, block) in seq.blocks.iter().enumerate() {
        assert!(
            (seq.starts[i] - t).abs() <= 1e-9 * t.max(1.0),
            "start[{i}] = {} disagrees with cumulative {t}",
            seq.starts[i]
        );
        assert!(seq.starts[i] >= 0.0, "negative start at block {i}");
        t += block.duration;
    }

    // Total duration matches the running sum and the file's own TotalDuration.
    assert!((seq.total_duration - t).abs() <= 1e-9 * t);
    assert!(
        (seq.total_duration - EXPECT_TOTAL_DURATION_S).abs() <= 1e-6,
        "total_duration {} != expected {EXPECT_TOTAL_DURATION_S}",
        seq.total_duration
    );

    eprintln!(
        "[snapshot] blocks={} total_duration={:.6}s last_start={:.6}s",
        seq.blocks.len(),
        seq.total_duration,
        seq.starts.last().copied().unwrap_or_default()
    );
}

#[test]
fn raw_layer_round_trips() {
    let source = std::fs::read_to_string(FIXTURE).unwrap();
    let sections = raw_sections(&source).expect("raw layer must parse");

    // The raw layer is addressable and carries the same blocks as the IR.
    let raw_blocks = sections
        .iter()
        .find_map(|s| match s {
            pulseq_parse::raw::Section::Blocks(b) => Some(b.len()),
            _ => None,
        })
        .expect("a [BLOCKS] section");
    assert_eq!(raw_blocks, EXPECT_BLOCKS, "raw block count");

    let version = sections
        .iter()
        .find_map(|s| match s {
            pulseq_parse::raw::Section::Version(v) => Some((v.major, v.minor, v.revision)),
            _ => None,
        })
        .expect("a [VERSION] section");
    assert_eq!(version, (1, 5, 1));
}

/// Parse must be sub-second (acceptance) and O(n) in block count. A quadratic
/// parser over ~50k blocks would spend tens of milliseconds *per block*; we
/// assert a generous per-block budget that only a linear parser can meet, plus
/// a wall-clock ceiling well under one second even in a debug build.
#[test]
fn parse_is_sub_second_and_linear() {
    let t = Instant::now();
    let seq = load();
    let elapsed = t.elapsed();

    let per_block_us = elapsed.as_secs_f64() * 1e6 / seq.blocks.len() as f64;
    eprintln!(
        "[perf] parsed {} blocks in {:?} ({per_block_us:.3} us/block)",
        seq.blocks.len(),
        elapsed
    );

    // The "well under a second" claim is a release-build statement (~30 ms
    // here); winnow is ~10x slower unoptimized, so the debug ceiling is looser
    // to stay CI-stable. Either way it stays comfortably sub-second.
    let ceiling = if cfg!(debug_assertions) { 2.0 } else { 0.25 };
    assert!(
        elapsed.as_secs_f64() < ceiling,
        "parse took {elapsed:?}, expected < {ceiling} s"
    );
    // O(n) tripwire: comfortably above the observed µs/block (release ~0.6,
    // debug ~8), far below the ~ms/block a quadratic parser would show here.
    assert!(
        per_block_us < 50.0,
        "{per_block_us:.3} us/block suggests worse-than-linear parsing"
    );
}
