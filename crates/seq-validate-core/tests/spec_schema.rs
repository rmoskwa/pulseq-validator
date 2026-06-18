//! The `--spec` input is a contract too, so it gets a published JSON Schema
//! (`schema/spec-v1.schema.json`). These tests pin it: the schema compiles, the
//! embedded `SPEC_SCHEMA` const matches the file on disk, and the bundled example
//! spec validates against it.
#![allow(clippy::expect_used, clippy::panic)] // test helpers intentionally panic on failure

use seq_validate_core::{SPEC_SCHEMA, serde_json};
use serde_json::Value;

const SCHEMA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/spec-v1.schema.json");
const EXAMPLE_SPEC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/t1_spgr_axial_brain.spec.yaml"
);

fn schema() -> Value {
    let text = std::fs::read_to_string(SCHEMA_PATH).expect("schema file is readable");
    serde_json::from_str(&text).expect("schema file is valid JSON")
}

/// A YAML spec, parsed and projected into JSON (the domain the schema validates).
/// YAML maps cleanly: mappings to objects, sequences to arrays, null to null.
fn spec_as_json(yaml: &str) -> Value {
    serde_yaml::from_str(yaml).expect("spec YAML deserializes into a JSON value")
}

fn assert_spec_valid(yaml: &str) {
    let validator = jsonschema::validator_for(&schema()).expect("schema compiles");
    let instance = spec_as_json(yaml);
    if !validator.is_valid(&instance) {
        let errors: Vec<String> = validator
            .iter_errors(&instance)
            .map(|e| format!("  - {e}"))
            .collect();
        panic!(
            "spec violates spec-v1.schema.json:\n{}\n--- spec ---\n{yaml}",
            errors.join("\n")
        );
    }
}

#[test]
fn schema_file_compiles() {
    jsonschema::validator_for(&schema()).expect("spec-v1.schema.json must be a valid schema");
}

#[test]
fn embedded_schema_matches_file_on_disk() {
    // The binary emits `SPEC_SCHEMA` via `--emit-spec-schema`; keep it byte-identical
    // to the committed file so the emitted contract is the published one.
    let on_disk = std::fs::read_to_string(SCHEMA_PATH).expect("schema file is readable");
    assert_eq!(
        SPEC_SCHEMA, on_disk,
        "embedded SPEC_SCHEMA is out of sync with schema/spec-v1.schema.json"
    );
}

#[test]
fn bundled_example_spec_validates() {
    let yaml = std::fs::read_to_string(EXAMPLE_SPEC).expect("example spec is readable");
    assert_spec_valid(&yaml);
}

#[test]
fn opt_out_and_tolerance_shapes_validate() {
    // The `none`/null opt-out, the per-axis geometry vectors, free-form blocks, and
    // every tolerance shape (`exact`, `{abs}`, `{rel}`) are all accepted.
    assert_spec_valid(
        "\
name: coverage
notes: free text
te_ms: none
tr_ms: ~
flip_angle_deg: 80
matrix: [192, 192, 1]
fov_mm: [240, 240]
oversampling: [2, 1, 1]
tolerances:\n  te_ms: {abs: 1.0}\n  fov_mm_x: {rel: 0.05}\n  matrix_x: exact\n",
    );
    // An empty document asserts nothing and is valid.
    assert_spec_valid("");
}

#[test]
fn typod_top_level_key_is_flagged() {
    // `tr` (a typo of `tr_ms`) is outside the recognized contract; the schema flags
    // it via additionalProperties, the static mirror of the tool's runtime warning.
    let validator = jsonschema::validator_for(&schema()).expect("schema compiles");
    assert!(
        !validator.is_valid(&spec_as_json("tr: 400\n")),
        "an unrecognized top-level key must fail the schema"
    );
}
