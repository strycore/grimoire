//! Schema-validation integration tests.
//!
//! Loads `schema/spell.schema.json` and `schema/manifest.schema.json` and:
//!   1. Validates every YAML spell in `spells/` against the spell schema.
//!   2. Validates every TOML manifest fixture below against the manifest schema.
//!   3. Asserts that hand-rolled "should fail" inputs really do fail.
//!
//! These tests double as a contract test for outside contributors: any new
//! spell that breaks the schema fails CI before merging.

use jsonschema::Validator;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn load_validator(name: &str) -> Validator {
    let schema_path = project_root().join("schema").join(name);
    let raw = fs::read_to_string(&schema_path)
        .unwrap_or_else(|e| panic!("read {}: {}", schema_path.display(), e));
    let schema: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("parse {}: {}", schema_path.display(), e));
    jsonschema::validator_for(&schema)
        .unwrap_or_else(|e| panic!("compile {}: {}", schema_path.display(), e))
}

fn yaml_to_json(yaml: &str) -> Value {
    serde_yml::from_str(yaml).unwrap_or_else(|e| panic!("yaml→value: {e}"))
}

fn toml_to_json(s: &str) -> Value {
    let parsed: toml::Value = toml::from_str(s).unwrap_or_else(|e| panic!("toml: {e}"));
    serde_json::to_value(parsed).unwrap()
}

fn validation_errors(validator: &Validator, value: &Value) -> Vec<String> {
    validator
        .iter_errors(value)
        .map(|e| e.to_string())
        .collect()
}

#[test]
fn every_embedded_spell_validates() {
    let validator = load_validator("spell.schema.json");
    let spells_dir = project_root().join("spells");

    let mut failures: Vec<String> = Vec::new();
    let mut count = 0;
    for entry in fs::read_dir(&spells_dir).expect("read spells dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        count += 1;
        let yaml = fs::read_to_string(&path).expect("read yaml");
        let value = yaml_to_json(&yaml);
        let errs = validation_errors(&validator, &value);
        if !errs.is_empty() {
            failures.push(format!(
                "{}: {} schema violation(s)\n    - {}",
                path.file_name().unwrap().to_string_lossy(),
                errs.len(),
                errs.join("\n    - "),
            ));
        }
    }

    assert!(count > 0, "no spells found in {}", spells_dir.display());
    assert!(
        failures.is_empty(),
        "{} of {count} spells failed schema validation:\n{}",
        failures.len(),
        failures.join("\n"),
    );
}

#[test]
fn spell_schema_rejects_missing_required_fields() {
    let validator = load_validator("spell.schema.json");

    // Missing `cast`.
    let yaml = r#"
name: bad
summary: missing cast
verify: "true"
"#;
    let errs = validation_errors(&validator, &yaml_to_json(yaml));
    assert!(
        !errs.is_empty(),
        "expected validation error for missing `cast` field"
    );
}

#[test]
fn spell_schema_rejects_unknown_top_level_field() {
    let validator = load_validator("spell.schema.json");
    let yaml = r#"
name: bad
summary: typo
verify: "true"
mystery_field: 42
cast:
  default: shell
  channels:
    shell:
      type: shell
      summary: ""
      run: "true"
"#;
    let errs = validation_errors(&validator, &yaml_to_json(yaml));
    assert!(
        errs.iter().any(|e| e.contains("mystery_field")),
        "expected error mentioning `mystery_field`, got: {errs:?}"
    );
}

#[test]
fn spell_schema_rejects_bad_name_pattern() {
    let validator = load_validator("spell.schema.json");
    let yaml = r#"
name: Bad-Name
summary: bad
verify: "true"
cast:
  default: shell
  channels:
    shell:
      type: shell
      summary: ""
      run: "true"
"#;
    let errs = validation_errors(&validator, &yaml_to_json(yaml));
    assert!(
        !errs.is_empty(),
        "expected pattern violation for capitalized name"
    );
}

#[test]
fn spell_schema_rejects_unknown_channel_type() {
    let validator = load_validator("spell.schema.json");
    let yaml = r#"
name: bad
summary: bad channel
verify: "true"
cast:
  default: weird
  channels:
    weird:
      type: brainwash
      summary: ""
      run: "true"
"#;
    let errs = validation_errors(&validator, &yaml_to_json(yaml));
    assert!(
        errs.iter()
            .any(|e| e.contains("brainwash") || e.contains("enum")),
        "expected enum-violation error, got: {errs:?}"
    );
}

#[test]
fn spell_schema_rejects_empty_channels_object() {
    let validator = load_validator("spell.schema.json");
    let yaml = r#"
name: bad
summary: no channels
verify: "true"
cast:
  default: shell
  channels: {}
"#;
    let errs = validation_errors(&validator, &yaml_to_json(yaml));
    assert!(
        !errs.is_empty(),
        "expected error for empty `channels` object"
    );
}

#[test]
fn manifest_schema_validates_typical_project_manifest() {
    let validator = load_validator("manifest.schema.json");
    let toml_str = r#"
spells = [
  "rust-dev >= 1.95",
  "bun-dev >= 1.0",
  "java-sdk >= 18, < 20",
  "android-dev",
]

[overrides.rust-dev]
channel = "rustup"

[profiles.work]
spells = ["rust-dev", "slack"]
"#;
    let errs = validation_errors(&validator, &toml_to_json(toml_str));
    assert!(errs.is_empty(), "schema rejected valid manifest: {errs:?}");
}

#[test]
fn manifest_schema_rejects_unknown_field_in_override() {
    let validator = load_validator("manifest.schema.json");
    let toml_str = r#"
spells = []

[overrides.rust-dev]
mystery = "huh"
"#;
    let errs = validation_errors(&validator, &toml_to_json(toml_str));
    assert!(
        !errs.is_empty(),
        "expected error for unknown field in override"
    );
}

#[test]
fn manifest_schema_rejects_bad_override_name() {
    let validator = load_validator("manifest.schema.json");
    let toml_str = r#"
spells = []

[overrides."Bad-Name"]
channel = "x"
"#;
    let errs = validation_errors(&validator, &toml_to_json(toml_str));
    assert!(
        !errs.is_empty(),
        "expected pattern violation for capitalized override key"
    );
}

#[test]
fn schemas_are_well_formed_json() {
    // Compiling them in load_validator() above already tests this; this is a
    // belt-and-suspenders read-and-parse that makes the failure mode obvious
    // when a contributor breaks the JSON syntax.
    for name in ["spell.schema.json", "manifest.schema.json"] {
        let path = project_root().join("schema").join(name);
        let raw = fs::read_to_string(&path).expect("read schema");
        let _: Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("schema {name} is not valid JSON: {e}"));
        assert!(
            Path::new(&path).exists(),
            "schema {name} should exist at canonical location"
        );
    }
}
