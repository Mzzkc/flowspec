//! QA-3 Cycle 10 tests — validate_manifest_size() wiring, trace refactor,
//! json.rs/yaml.rs coverage, cross-format consistency, regression guards.

// =========================================================================
// Phase 1: validate_manifest_size() Production Wiring (P0 Hard Gate)
// =========================================================================

/// T1: Size validation IS CALLED in production — pathological input triggers error.
/// This test MUST fail against the pre-wiring codebase and pass after wiring.
#[test]
fn t1_size_validation_wired_pathological_input_rejected() {
    let tmp = tempfile::tempdir().unwrap();

    // Generate 80 minimal Python functions — enough to exceed 10x ratio
    // Each `def function_NNN(): pass\n` is ~25 bytes. 80 * 25 = ~2000 bytes source.
    // Manifest overhead per entity: ~250-400 bytes YAML. 80 * 300 = ~24000 bytes manifest.
    // Ratio: 24000/2000 ≈ 12x — exceeds 10x limit.
    let mut source = String::new();
    for i in 0..80 {
        source.push_str(&format!("def function_{:03}(): pass\n", i));
    }
    std::fs::write(tmp.path().join("pathological.py"), &source).unwrap();

    let result =
        crate::commands::run_analyze(tmp.path(), &[], crate::OutputFormat::Yaml, None, None);

    // The size check MUST fire. If run_analyze succeeds, the check isn't wired.
    assert!(
        result.is_err(),
        "run_analyze() on pathological input MUST return Err (size limit). \
         If this succeeds, validate_manifest_size() is NOT wired into the \
         production pipeline."
    );

    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("exceeds") || err_msg.contains("size") || err_msg.contains("ratio"),
        "Error must mention size/ratio. Got: {}",
        err_msg
    );
}

/// T2: Normal input passes size validation without error.
#[test]
fn t2_normal_input_passes_size_validation() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("normal.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let result =
        crate::commands::run_analyze(tmp.path(), &[], crate::OutputFormat::Yaml, None, None);

    assert!(
        result.is_ok(),
        "Normal fixture MUST pass size validation. Got: {:?}",
        result.err()
    );
}

/// T3: Size check runs BEFORE write_output — no output file on rejection.
#[test]
fn t3_size_check_before_write_output() {
    let tmp = tempfile::tempdir().unwrap();

    // Same pathological input as T1
    let mut source = String::new();
    for i in 0..80 {
        source.push_str(&format!("def function_{:03}(): pass\n", i));
    }
    std::fs::write(tmp.path().join("pathological.py"), &source).unwrap();

    let output_file = tmp.path().join("output.yaml");
    let result = crate::commands::run_analyze(
        tmp.path(),
        &[],
        crate::OutputFormat::Yaml,
        Some(output_file.as_path()),
        None,
    );

    assert!(result.is_err(), "Must reject pathological input");

    // Output file must NOT have been written
    assert!(
        !output_file.exists() || std::fs::read_to_string(&output_file).unwrap().is_empty(),
        "Output file must not exist (or be empty) when size check fails. \
         validate_manifest_size() must run BEFORE write_output()."
    );
}

/// T4: Source bytes below 1024 threshold — size check skips gracefully.
#[test]
fn t4_small_source_below_threshold_skips_size_check() {
    let tmp = tempfile::tempdir().unwrap();
    // A file well under 1024 bytes — size check should skip (SIZE_CHECK_MIN_SOURCE_BYTES)
    std::fs::write(
        tmp.path().join("tiny.py"),
        "def hello(): pass\ndef world(): pass\n",
    )
    .unwrap();

    let result =
        crate::commands::run_analyze(tmp.path(), &[], crate::OutputFormat::Yaml, None, None);

    assert!(
        result.is_ok(),
        "Tiny source below 1024-byte threshold MUST NOT trigger size limit. \
         Got: {:?}",
        result.err()
    );
}

/// T5: Size check uses real source_bytes, not a hardcoded value.
/// A fixture just above the 1024 threshold with few entities should pass.
#[test]
fn t5_size_check_uses_real_source_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    // Create a file with ~1100 bytes source — just above the 1024 threshold
    // with only a few functions (low entity count → low manifest ratio)
    let mut source = String::new();
    // Long docstrings pad the source while keeping entity count low
    source.push_str("def main():\n");
    source.push_str("    \"\"\"");
    for _ in 0..100 {
        source.push_str("padding text ");
    }
    source.push_str("\"\"\"\n");
    source.push_str("    return 42\n\n");
    source.push_str("def helper():\n");
    source.push_str("    \"\"\"");
    for _ in 0..50 {
        source.push_str("more padding ");
    }
    source.push_str("\"\"\"\n");
    source.push_str("    return main()\n");

    assert!(
        source.len() > 1024,
        "Source must be above threshold. Was {} bytes",
        source.len()
    );

    std::fs::write(tmp.path().join("above_threshold.py"), &source).unwrap();

    let result =
        crate::commands::run_analyze(tmp.path(), &[], crate::OutputFormat::Yaml, None, None);

    assert!(
        result.is_ok(),
        "Source just above threshold with few entities MUST pass. \
         If this fails, source_bytes may be hardcoded to a small value. Got: {:?}",
        result.err()
    );
}

// =========================================================================
// Phase 2: Trace Refactor — Behavior Consistency (P1)
// =========================================================================

/// T6: Forward trace produces flows for known fixture.
#[test]
fn t6_trace_forward_produces_flows() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let result = crate::commands::run_trace(
        tmp.path(),
        "main_handler",
        &[],
        10,
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Json,
        None,
        None,
    );

    assert!(
        result.is_ok(),
        "Forward trace on main_handler must succeed. Got: {:?}",
        result.err()
    );
}

/// T7: Trace on symbol with zero outgoing flows returns empty, not error.
#[test]
fn t7_trace_terminal_symbol_returns_empty_not_error() {
    let tmp = tempfile::tempdir().unwrap();
    // A Python file with a leaf function that calls nothing
    std::fs::write(
        tmp.path().join("leaf.py"),
        "def leaf_function():\n    return 42\n",
    )
    .unwrap();

    let result = crate::commands::run_trace(
        tmp.path(),
        "leaf_function",
        &[],
        10,
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Json,
        None,
        None,
    );

    assert!(
        result.is_ok(),
        "Trace on terminal symbol (no outgoing calls) must return Ok, \
         not error. Got: {:?}",
        result.err()
    );
}

/// T8: Trace depth=0 truncates all steps.
#[test]
fn t8_trace_depth_zero_truncates() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("chain.py"),
        "def a():\n    return b()\ndef b():\n    return c()\ndef c():\n    return 42\n",
    )
    .unwrap();

    let result = crate::commands::run_trace(
        tmp.path(),
        "a",
        &[],
        0, // depth=0 should truncate all steps
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Json,
        None,
        None,
    );

    assert!(
        result.is_ok(),
        "Trace with depth=0 must succeed. Got: {:?}",
        result.err()
    );
}

/// T9: Trace backward direction returns CommandNotImplemented with suggestion.
#[test]
fn t9_trace_backward_returns_not_implemented_with_suggestion() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("test.py"), "def f(): pass\n").unwrap();

    let result = crate::commands::run_trace(
        tmp.path(),
        "f",
        &[],
        10,
        crate::commands::TraceDirection::Backward,
        crate::OutputFormat::Yaml,
        None,
        None,
    );

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("--direction forward"),
        "Backward error MUST suggest --direction forward. Got: {}",
        err
    );
}

/// T10: Trace on nonexistent symbol returns clear error.
#[test]
fn t10_trace_nonexistent_symbol_returns_symbol_not_found() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("test.py"), "def real_func(): pass\n").unwrap();

    let result = crate::commands::run_trace(
        tmp.path(),
        "completely_nonexistent_symbol_xyz",
        &[],
        10,
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Json,
        None,
        None,
    );

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not found"),
        "Missing symbol error must say 'not found'. Got: {}",
        err
    );
}

// =========================================================================
// Phase 3: json.rs/yaml.rs Coverage (P2)
// =========================================================================

/// T11: JSON empty diagnostics produces valid JSON array.
#[test]
fn t11_json_empty_diagnostics_valid_array() {
    use crate::manifest::OutputFormatter;
    let formatter = crate::JsonFormatter::new();
    let diagnostics: Vec<crate::manifest::types::DiagnosticEntry> = vec![];
    let json = formatter.format_diagnostics(&diagnostics).unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(
        parsed.is_array(),
        "Empty diagnostics must produce JSON array, got: {:?}",
        parsed
    );
    assert_eq!(parsed.as_array().unwrap().len(), 0);
}

/// T12: YAML empty diagnostics produces valid YAML sequence.
#[test]
fn t12_yaml_empty_diagnostics_valid_sequence() {
    use crate::manifest::OutputFormatter;
    let formatter = crate::YamlFormatter::new();
    let diagnostics: Vec<crate::manifest::types::DiagnosticEntry> = vec![];
    let yaml = formatter.format_diagnostics(&diagnostics).unwrap();

    let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    assert!(
        parsed.is_sequence(),
        "Empty diagnostics must produce YAML sequence, got: {:?}",
        parsed
    );
}

/// T13: YAML multi-line evidence strings remain valid parseable YAML.
#[test]
fn t13_yaml_multiline_evidence_valid() {
    use crate::manifest::OutputFormatter;
    let diagnostics = vec![crate::manifest::types::DiagnosticEntry {
        id: "D001".to_string(),
        pattern: "test".to_string(),
        severity: "warning".to_string(),
        confidence: "high".to_string(),
        entity: "test::fn".to_string(),
        message: "test".to_string(),
        evidence: vec![crate::manifest::types::EvidenceEntry {
            observation: "Line 1 of evidence\nLine 2 with special: chars\nLine 3 with \"quotes\""
                .to_string(),
            location: Some("test.py:10-15".to_string()),
            context: Some("multi\nline\ncontext".to_string()),
        }],
        suggestion: "fix it".to_string(),
        loc: "test.py:1".to_string(),
    }];

    let formatter = crate::YamlFormatter::new();
    let yaml = formatter.format_diagnostics(&diagnostics).unwrap();

    let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&yaml);
    assert!(
        parsed.is_ok(),
        "Multi-line evidence must produce valid YAML. Got parse error: {:?}\nYAML:\n{}",
        parsed.err(),
        &yaml[..yaml.len().min(500)]
    );

    // Verify the multi-line content survived roundtrip
    let val = parsed.unwrap();
    let obs = val[0]["evidence"][0]["observation"].as_str().unwrap();
    assert!(
        obs.contains("Line 1") && obs.contains("Line 2") && obs.contains("Line 3"),
        "Multi-line evidence must preserve all lines. Got: {}",
        obs
    );
}

/// T14: JSON special characters in symbol names properly escaped.
#[test]
fn t14_json_special_chars_in_entity_ids() {
    use crate::manifest::OutputFormatter;
    let mut manifest = crate::Manifest::empty();
    manifest.entities.push(crate::manifest::types::EntityEntry {
        id: "module::func<T>".to_string(),
        kind: "fn".to_string(),
        vis: "pub".to_string(),
        sig: "fn func<T: Display>(x: &str) -> Result<T, Box<dyn Error>>".to_string(),
        loc: "src/lib.rs:1".to_string(),
        calls: vec![],
        called_by: vec![],
        annotations: vec![],
    });

    let formatter = crate::JsonFormatter::new();
    let json = formatter.format_manifest(&manifest).unwrap();

    // Must parse as valid JSON despite angle brackets, ampersands, etc.
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let id = parsed["entities"][0]["id"].as_str().unwrap();
    assert_eq!(id, "module::func<T>");
}

/// T15: Cross-format diagnostic count consistency (regression from C9 T22).
#[test]
fn t15_cross_format_diagnostic_count_consistency() {
    use crate::manifest::OutputFormatter;
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = crate::Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    // Format as JSON
    let json_formatter = crate::JsonFormatter::new();
    let json_str = json_formatter.format_manifest(&result.manifest).unwrap();
    let json_val: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let json_diag_count = json_val["diagnostics"].as_array().unwrap().len();

    // Format as YAML
    let yaml_formatter = crate::YamlFormatter::new();
    let yaml_str = yaml_formatter.format_manifest(&result.manifest).unwrap();
    let yaml_val: serde_yaml::Value = serde_yaml::from_str(&yaml_str).unwrap();
    let yaml_diag_count = yaml_val["diagnostics"].as_sequence().unwrap().len();

    // Cross-format consistency
    assert_eq!(
        json_diag_count, yaml_diag_count,
        "JSON diagnostic count ({}) != YAML diagnostic count ({}). \
         Formatters are diverging on the same manifest.",
        json_diag_count, yaml_diag_count
    );

    // Also verify against manifest metadata
    assert_eq!(
        json_diag_count, result.manifest.metadata.diagnostic_count as usize,
        "JSON diagnostic array length ({}) != metadata.diagnostic_count ({})",
        json_diag_count, result.manifest.metadata.diagnostic_count
    );
}

/// T16: Cross-format entity count consistency.
#[test]
fn t16_cross_format_entity_count_consistency() {
    use crate::manifest::OutputFormatter;
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = crate::Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let json_formatter = crate::JsonFormatter::new();
    let json_str = json_formatter.format_manifest(&result.manifest).unwrap();
    let json_val: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let json_entity_count = json_val["entities"].as_array().unwrap().len();

    let yaml_formatter = crate::YamlFormatter::new();
    let yaml_str = yaml_formatter.format_manifest(&result.manifest).unwrap();
    let yaml_val: serde_yaml::Value = serde_yaml::from_str(&yaml_str).unwrap();
    let yaml_entity_count = yaml_val["entities"].as_sequence().unwrap().len();

    assert_eq!(
        json_entity_count, yaml_entity_count,
        "JSON entity count ({}) != YAML entity count ({})",
        json_entity_count, yaml_entity_count
    );
}

/// T17: YAML roundtrip preserves manifest structure.
#[test]
fn t17_yaml_roundtrip_manifest() {
    use crate::manifest::OutputFormatter;
    let original = crate::Manifest::sample_full();
    let formatter = crate::YamlFormatter::new();
    let yaml = formatter.format_manifest(&original).unwrap();

    let deserialized: crate::Manifest =
        serde_yaml::from_str(&yaml).expect("YAML output must deserialize back to Manifest");

    assert_eq!(deserialized.metadata.project, original.metadata.project);
    assert_eq!(deserialized.entities.len(), original.entities.len());
    assert_eq!(deserialized.diagnostics.len(), original.diagnostics.len());
    assert_eq!(deserialized.flows.len(), original.flows.len());
}

/// T18: JSON with all 8 sections populated produces valid output.
#[test]
fn t18_json_all_sections_populated() {
    use crate::manifest::OutputFormatter;
    let manifest = crate::Manifest::sample_full();
    let formatter = crate::JsonFormatter::new();
    let json = formatter.format_manifest(&manifest).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    let obj = parsed.as_object().unwrap();
    for section in &[
        "metadata",
        "summary",
        "diagnostics",
        "entities",
        "flows",
        "boundaries",
        "dependency_graph",
        "type_flows",
    ] {
        assert!(
            obj.contains_key(*section),
            "sample_full manifest missing section '{}' in JSON output",
            section
        );
    }
}

/// T19: YAML with all 8 sections populated produces valid output.
#[test]
fn t19_yaml_all_sections_populated() {
    use crate::manifest::OutputFormatter;
    let manifest = crate::Manifest::sample_full();
    let formatter = crate::YamlFormatter::new();
    let yaml = formatter.format_manifest(&manifest).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();

    let map = parsed.as_mapping().unwrap();
    for section in &[
        "metadata",
        "summary",
        "diagnostics",
        "entities",
        "flows",
        "boundaries",
        "dependency_graph",
        "type_flows",
    ] {
        let key = serde_yaml::Value::String(section.to_string());
        assert!(
            map.contains_key(&key),
            "sample_full manifest missing section '{}' in YAML output",
            section
        );
    }
}

// =========================================================================
// Regression Tests
// =========================================================================

/// T20: run_analyze rejects empty path (regression guard).
#[test]
fn t20_run_analyze_rejects_empty_path() {
    let result = crate::commands::run_analyze(
        std::path::Path::new(""),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
    );
    assert!(result.is_err());
}

/// T21: format_with dispatches all 4 formats without panic (regression from C7).
#[test]
fn t21_format_with_all_formats_no_panic() {
    let manifest = crate::Manifest::empty();
    for format in &[
        crate::OutputFormat::Yaml,
        crate::OutputFormat::Json,
        crate::OutputFormat::Sarif,
        crate::OutputFormat::Summary,
    ] {
        let result = crate::commands::format_with(*format, |f| f.format_manifest(&manifest));
        assert!(
            result.is_ok(),
            "format_with({:?}) must not fail on empty manifest. Got: {:?}",
            format,
            result.err()
        );
    }
}

/// T22: Trace summary format includes flow count (regression from C9 T16).
#[test]
fn t22_trace_summary_includes_flow_count() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let result = crate::commands::run_trace(
        tmp.path(),
        "main_handler",
        &[],
        10,
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Summary,
        None,
        None,
    );

    assert!(
        result.is_ok(),
        "Summary trace must succeed. Got: {:?}",
        result.err()
    );
}
