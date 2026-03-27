// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Cycle 13 QA-3 (Surface) tests — trace dedup, symbol disambiguation, #17 regression.

use std::collections::HashSet;

use crate::commands::{find_matching_symbol, validate_check_patterns};
use crate::deduplicate_flows;
use crate::manifest::types::{EntityEntry, FlowEntry, FlowStep};
use crate::manifest::OutputFormatter;
use crate::test_utils::{add_ref, make_symbol};
use crate::Graph;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_flow(id: &str, entry: &str, exit: &str, step_entities: &[&str]) -> FlowEntry {
    FlowEntry {
        id: id.to_string(),
        description: format!("Flow from {} to {}", entry, exit),
        entry: entry.to_string(),
        exit: exit.to_string(),
        steps: step_entities
            .iter()
            .map(|e| FlowStep {
                entity: e.to_string(),
                action: "call".to_string(),
                in_type: "unknown".to_string(),
                out_type: "unknown".to_string(),
            })
            .collect(),
        issues: Vec::new(),
    }
}

fn entity(id: &str, loc: &str) -> EntityEntry {
    EntityEntry {
        id: id.to_string(),
        kind: "fn".to_string(),
        vis: "pub".to_string(),
        sig: String::new(),
        loc: loc.to_string(),
        calls: vec![],
        called_by: vec![],
        annotations: vec![],
    }
}

fn load_config(path: &std::path::Path) -> crate::Config {
    crate::Config::load(path, None).unwrap()
}

// ===========================================================================
// Category 1: Trace --direction both Deduplication (T1–T12)
// ===========================================================================

/// T1: Identical flows from forward and backward are deduplicated.
#[test]
fn trace_both_dedup_removes_exact_duplicates() {
    let flows = vec![
        make_flow("F001", "A", "B", &["B"]),
        make_flow("F002", "A", "B", &["B"]), // exact duplicate
        make_flow("F003", "C", "D", &["D"]),
    ];
    let result = deduplicate_flows(flows);
    assert_eq!(result.len(), 2, "Duplicate flow must be removed");
    assert_eq!(result[0].entry, "A");
    assert_eq!(result[1].entry, "C");
}

/// T2: Different-direction flows are NOT deduplicated.
#[test]
fn trace_both_preserves_different_direction_flows() {
    let flows = vec![
        make_flow("F001", "A", "B", &["B"]), // forward: A->B
        make_flow("F002", "B", "A", &["A"]), // backward: B->A (different!)
    ];
    let result = deduplicate_flows(flows);
    assert_eq!(
        result.len(),
        2,
        "Different entry/exit pairs must be preserved"
    );
}

/// T3: Flow IDs are sequential after dedup (no gaps).
#[test]
fn trace_both_dedup_renumbers_ids() {
    let flows = vec![
        make_flow("F001", "A", "B", &["B"]),
        make_flow("F002", "A", "B", &["B"]), // duplicate — removed
        make_flow("F003", "C", "D", &["D"]),
        make_flow("F004", "E", "F", &["F"]),
    ];
    let result = deduplicate_flows(flows);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].id, "F001");
    assert_eq!(result[1].id, "F002"); // NOT F003
    assert_eq!(result[2].id, "F003"); // NOT F004
}

/// T4: Empty trace produces empty output.
#[test]
fn trace_both_empty_result() {
    let flows: Vec<FlowEntry> = vec![];
    let result = deduplicate_flows(flows);
    assert!(result.is_empty());
}

/// T5: Cyclic flows deduplicated correctly.
#[test]
fn trace_both_cyclic_dedup() {
    let mut flow1 = make_flow("F001", "A", "A", &["B", "A"]);
    flow1.description = "Cyclic flow from A".to_string();
    let mut flow2 = make_flow("F002", "A", "A", &["B", "A"]);
    flow2.description = "Cyclic flow from A".to_string();
    let flows = vec![flow1, flow2];
    let result = deduplicate_flows(flows);
    assert_eq!(result.len(), 1, "Duplicate cyclic flow must be removed");
    assert!(result[0].description.contains("Cyclic"));
}

/// T6: Large flow set dedup performance (not O(n^2)).
#[test]
fn trace_both_dedup_not_quadratic() {
    let mut flows = Vec::new();
    for i in 0..100 {
        let entry = format!("sym_{}", i % 50); // 50 unique, 50 duplicates
        let exit = format!("exit_{}", i % 50);
        flows.push(make_flow(
            &format!("F{:03}", i + 1),
            &entry,
            &exit,
            &[&exit],
        ));
    }
    let result = deduplicate_flows(flows);
    assert_eq!(
        result.len(),
        50,
        "50 unique flows expected from 100 with 50 dupes"
    );
}

/// T7: Dedup key includes steps, not just entry/exit.
#[test]
fn trace_both_dedup_considers_steps() {
    let flows = vec![
        make_flow("F001", "A", "C", &["B", "C"]), // A->B->C
        make_flow("F002", "A", "C", &["D", "C"]), // A->D->C (different path!)
    ];
    let result = deduplicate_flows(flows);
    assert_eq!(
        result.len(),
        2,
        "Same endpoints but different steps must be preserved"
    );
}

/// T8: Dedup preserves flow ordering (first occurrence kept).
#[test]
fn trace_both_dedup_preserves_order() {
    let flows = vec![
        make_flow("F001", "A", "B", &["B"]), // forward result
        make_flow("F002", "C", "D", &["D"]), // forward result
        make_flow("F003", "A", "B", &["B"]), // backward duplicate of F001
        make_flow("F004", "E", "F", &["F"]), // backward unique
    ];
    let result = deduplicate_flows(flows);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].entry, "A"); // first occurrence kept
    assert_eq!(result[1].entry, "C");
    assert_eq!(result[2].entry, "E");
}

/// T9: JSON output is parseable after dedup.
#[test]
fn trace_both_json_parseable_after_dedup() {
    let flows = vec![
        make_flow("F001", "A", "B", &["B"]),
        make_flow("F002", "A", "B", &["B"]), // duplicate
        make_flow("F003", "C", "D", &["D"]),
    ];
    let deduped = deduplicate_flows(flows);
    let json = serde_json::to_string_pretty(&deduped).expect("JSON serialization must succeed");
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&json).expect("JSON must be parseable");
    assert_eq!(parsed.len(), 2);
    for entry in &parsed {
        assert!(entry.get("id").is_some(), "Each flow must have an id");
        assert!(entry.get("entry").is_some(), "Each flow must have entry");
        assert!(entry.get("exit").is_some(), "Each flow must have exit");
        assert!(entry.get("steps").is_some(), "Each flow must have steps");
    }
}

/// T10: YAML output is parseable after dedup.
#[test]
fn trace_both_yaml_parseable_after_dedup() {
    let flows = vec![
        make_flow("F001", "A", "B", &["B"]),
        make_flow("F002", "A", "B", &["B"]),
        make_flow("F003", "C", "D", &["D"]),
    ];
    let deduped = deduplicate_flows(flows);
    let yaml = serde_yaml::to_string(&deduped).expect("YAML serialization must succeed");
    let parsed: Vec<serde_yaml::Value> =
        serde_yaml::from_str(&yaml).expect("YAML must be parseable");
    assert_eq!(parsed.len(), 2);
}

/// T11: SARIF output valid after dedup.
#[test]
fn trace_both_sarif_valid_after_dedup() {
    let flows = vec![
        make_flow("F001", "A", "B", &["B"]),
        make_flow("F002", "A", "B", &["B"]),
        make_flow("F003", "C", "D", &["D"]),
    ];
    let deduped = deduplicate_flows(flows);
    let sarif =
        crate::commands::format_trace_sarif(&deduped).expect("SARIF formatting must succeed");
    let parsed: serde_json::Value = serde_json::from_str(&sarif).expect("SARIF must be valid JSON");
    assert_eq!(parsed["version"], "2.1.0");
    assert!(parsed["$schema"].as_str().is_some());
    let results = parsed["runs"][0]["results"].as_array().unwrap();
    assert_eq!(
        results.len(),
        2,
        "SARIF results must match deduped flow count"
    );
}

/// T12: Summary format shows correct count after dedup.
#[test]
fn trace_both_summary_correct_count_after_dedup() {
    let flows = vec![
        make_flow("F001", "A", "B", &["B"]),
        make_flow("F002", "A", "B", &["B"]),
        make_flow("F003", "C", "D", &["D"]),
        make_flow("F004", "E", "F", &["F"]),
    ];
    let deduped = deduplicate_flows(flows);
    let summary_line = format!("Trace: sym ({} flow(s) matched)", deduped.len());
    assert!(
        summary_line.contains("3 flow(s) matched"),
        "Summary must reflect deduped count, got: {}",
        summary_line
    );
}

// ===========================================================================
// Category 2: Symbol Disambiguation (T13–T22)
// ===========================================================================

/// T13: Same-name files in different directories get directory-prefixed IDs.
#[test]
fn disambiguate_same_filename_different_dirs() {
    let dir = tempfile::TempDir::new().unwrap();
    let parser_dir = dir.path().join("parser");
    let core_dir = dir.path().join("core");
    std::fs::create_dir_all(&parser_dir).unwrap();
    std::fs::create_dir_all(&core_dir).unwrap();

    std::fs::write(parser_dir.join("utils.py"), "def helper():\n    pass\n").unwrap();
    std::fs::write(core_dir.join("utils.py"), "def helper():\n    pass\n").unwrap();

    let config = load_config(dir.path());
    let result = crate::analyze(dir.path(), &config, &["python".to_string()]).unwrap();

    let helper_ids: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter(|e| e.id.contains("helper"))
        .map(|e| e.id.as_str())
        .collect();

    assert_eq!(
        helper_ids.len(),
        2,
        "Both helper symbols must appear. Got: {:?}",
        helper_ids
    );

    let unique: HashSet<&&str> = helper_ids.iter().collect();
    assert_eq!(
        unique.len(),
        2,
        "Entity IDs must be distinct after disambiguation. Got: {:?}",
        helper_ids
    );

    let has_dir_prefix = helper_ids.iter().any(|id| id.contains('/'));
    assert!(
        has_dir_prefix,
        "Disambiguated IDs must include directory prefix. Got: {:?}",
        helper_ids
    );
}

/// T14: Unique filenames do NOT get directory prefix.
#[test]
fn no_disambiguation_when_unambiguous() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.py"), "def run():\n    pass\n").unwrap();
    std::fs::write(dir.path().join("utils.py"), "def helper():\n    pass\n").unwrap();

    let config = load_config(dir.path());
    let result = crate::analyze(dir.path(), &config, &["python".to_string()]).unwrap();

    for ent in &result.manifest.entities {
        assert!(
            !ent.id.contains('/'),
            "Unique filenames must NOT get directory prefix. Got: {}",
            ent.id
        );
    }
}

/// T15: Disambiguation applies to symbol matching in trace.
#[test]
fn trace_uses_disambiguated_names() {
    let entities = vec![
        entity("parser/utils::process", "parser/utils.py:5"),
        entity("core/utils::process", "core/utils.py:12"),
    ];
    let result = find_matching_symbol("process", &entities);
    assert!(result.is_err(), "Ambiguous match must return error");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("parser/utils::process"),
        "Error must list disambiguated names. Got: {}",
        err
    );
    assert!(
        err.contains("core/utils::process"),
        "Error must list both candidates. Got: {}",
        err
    );
}

/// T17: Disambiguation with nested scopes.
#[test]
fn disambiguate_nested_class_methods() {
    let dir = tempfile::TempDir::new().unwrap();
    let a_dir = dir.path().join("a");
    let b_dir = dir.path().join("b");
    std::fs::create_dir_all(&a_dir).unwrap();
    std::fs::create_dir_all(&b_dir).unwrap();

    std::fs::write(
        a_dir.join("models.py"),
        "class User:\n    def save(self):\n        pass\n",
    )
    .unwrap();
    std::fs::write(
        b_dir.join("models.py"),
        "class User:\n    def save(self):\n        pass\n",
    )
    .unwrap();

    let config = load_config(dir.path());
    let result = crate::analyze(dir.path(), &config, &["python".to_string()]).unwrap();

    let save_ids: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter(|e| e.id.contains("save"))
        .map(|e| e.id.as_str())
        .collect();

    let unique: HashSet<&&str> = save_ids.iter().collect();
    assert_eq!(
        unique.len(),
        save_ids.len(),
        "Nested method IDs must be distinct. Got: {:?}",
        save_ids
    );
}

/// T19: Ambiguous error message includes location info.
#[test]
fn ambiguous_error_includes_location() {
    let entities = vec![
        entity("utils::helper", "src/parser/utils.py:5"),
        entity("core::helper", "src/core/utils.py:12"),
    ];
    // Both match name part "helper"
    let result = find_matching_symbol("helper", &entities);
    assert!(result.is_err(), "Two name matches must be ambiguous");
    let err = result.unwrap_err().to_string();
    // Error must include location info for disambiguation
    assert!(
        err.contains("utils.py"),
        "Ambiguous error must include location info. Got: {}",
        err
    );
}

/// T20: Disambiguation in YAML/JSON output consistency.
#[test]
fn disambiguated_names_consistent_across_formats() {
    let dir = tempfile::TempDir::new().unwrap();
    let parser_dir = dir.path().join("parser");
    let core_dir = dir.path().join("core");
    std::fs::create_dir_all(&parser_dir).unwrap();
    std::fs::create_dir_all(&core_dir).unwrap();

    std::fs::write(parser_dir.join("utils.py"), "def helper():\n    pass\n").unwrap();
    std::fs::write(core_dir.join("utils.py"), "def helper():\n    pass\n").unwrap();

    let config = load_config(dir.path());
    let result = crate::analyze(dir.path(), &config, &["python".to_string()]).unwrap();

    let yaml = crate::YamlFormatter::new()
        .format_manifest(&result.manifest)
        .unwrap();
    let json = crate::JsonFormatter::new()
        .format_manifest(&result.manifest)
        .unwrap();

    let yaml_parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    let json_parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    let yaml_ids: Vec<String> = yaml_parsed["entities"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();
    let json_ids: Vec<String> = json_parsed["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();

    assert_eq!(
        yaml_ids, json_ids,
        "Entity IDs must be identical across YAML and JSON formats"
    );
}

/// T22: Single-file project needs no disambiguation.
#[test]
fn single_file_no_disambiguation() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("main.py"),
        "def alpha():\n    pass\ndef beta():\n    pass\n",
    )
    .unwrap();

    let config = load_config(dir.path());
    let result = crate::analyze(dir.path(), &config, &["python".to_string()]).unwrap();

    for ent in &result.manifest.entities {
        assert!(
            !ent.id.contains('/'),
            "Single-file project must not disambiguate. Got: {}",
            ent.id
        );
    }
}

// ===========================================================================
// Category 3: Issue #17 Regression Tests (T23–T27)
// ===========================================================================

/// T23: UnknownPattern error lists exactly 13 patterns.
#[test]
fn unknown_pattern_error_lists_all_13_patterns() {
    let result = validate_check_patterns(&["bogus".to_string()]);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();

    let expected_patterns = [
        "isolated_cluster",
        "data_dead_end",
        "phantom_dependency",
        "orphaned_impl",
        "circular_dependency",
        "missing_reexport",
        "contract_mismatch",
        "stale_reference",
        "layer_violation",
        "duplication",
        "partial_wiring",
        "asymmetric_handling",
        "incomplete_migration",
    ];

    for pattern in &expected_patterns {
        assert!(
            err_msg.contains(pattern),
            "Error must list pattern '{}'. Got: {}",
            pattern,
            err_msg
        );
    }
}

/// T24: Error message contains no command-specific references.
#[test]
fn unknown_pattern_error_no_command_reference() {
    let result = validate_check_patterns(&["bogus".to_string()]);
    let err_msg = result.unwrap_err().to_string();

    for cmd in &["analyze", "diagnose", "trace", "--help"] {
        assert!(
            !err_msg.contains(cmd),
            "Error must not reference command '{}'. Got: {}",
            cmd,
            err_msg
        );
    }
}

/// T25: Error includes the user's invalid pattern name.
#[test]
fn unknown_pattern_error_includes_input() {
    let result = validate_check_patterns(&["my_typo_pattern".to_string()]);
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("my_typo_pattern"),
        "Error must echo the invalid input. Got: {}",
        err_msg
    );
}

/// T27: VALID_PATTERNS const matches error.rs hardcoded list.
#[test]
fn valid_patterns_sync_check() {
    let commands_patterns = [
        "isolated_cluster",
        "data_dead_end",
        "phantom_dependency",
        "orphaned_impl",
        "circular_dependency",
        "missing_reexport",
        "contract_mismatch",
        "stale_reference",
        "layer_violation",
        "duplication",
        "partial_wiring",
        "asymmetric_handling",
        "incomplete_migration",
    ];

    for pattern in &commands_patterns {
        assert!(
            validate_check_patterns(&[pattern.to_string()]).is_ok(),
            "Pattern '{}' must be valid",
            pattern
        );
    }

    let err_msg = validate_check_patterns(&["__invalid__".to_string()])
        .unwrap_err()
        .to_string();
    for pattern in &commands_patterns {
        assert!(
            err_msg.contains(pattern),
            "Error message must list '{}'. Got: {}",
            pattern,
            err_msg
        );
    }
}

// ===========================================================================
// Category 4: Pipe Safety & Output Format Compliance (T28–T33)
// ===========================================================================

/// T28: Trace JSON output is a valid JSON array.
#[test]
fn trace_json_is_array() {
    let flows = vec![make_flow("F001", "A", "B", &["B"])];
    let json = serde_json::to_string_pretty(&flows).unwrap();
    assert!(json.starts_with('['), "Trace JSON must start with [");
    assert!(json.ends_with(']'), "Trace JSON must end with ]");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed.is_array());
}

/// T31: Zero-result trace produces valid output in all formats.
#[test]
fn trace_zero_results_all_formats() {
    let empty: Vec<FlowEntry> = vec![];

    // JSON
    let json = serde_json::to_string_pretty(&empty).unwrap();
    assert_eq!(json.trim(), "[]");

    // YAML
    let yaml = serde_yaml::to_string(&empty).unwrap();
    let parsed: Vec<serde_yaml::Value> = serde_yaml::from_str(&yaml).unwrap();
    assert!(parsed.is_empty());

    // SARIF
    let sarif = crate::commands::format_trace_sarif(&empty).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&sarif).unwrap();
    assert_eq!(parsed["runs"][0]["results"].as_array().unwrap().len(), 0);
}

// ===========================================================================
// Category 5: Regression Guards (T34–T37)
// ===========================================================================

/// T35: C12 #16 fix still holds — summary filter consistency.
#[test]
fn summary_diagnostic_counts_after_filter() {
    use crate::commands::{apply_diagnostic_filters, recompute_diagnostic_summary};
    use crate::manifest::types::{DiagnosticEntry, DiagnosticSummary};

    let mut diagnostics = vec![
        DiagnosticEntry::sample_critical(),
        DiagnosticEntry::sample_warning(),
    ];
    let mut summary = DiagnosticSummary {
        critical: 1,
        warning: 1,
        info: 0,
        top_issues: vec![],
    };

    apply_diagnostic_filters(&mut diagnostics, &[], Some(crate::Severity::Critical), None);
    recompute_diagnostic_summary(&diagnostics, &mut summary);

    assert_eq!(summary.critical, 1);
    assert_eq!(
        summary.warning, 0,
        "Warning must be gone after severity filter"
    );
    assert_eq!(
        diagnostics.len() as u64,
        summary.critical + summary.warning + summary.info,
        "Summary counts must match filtered diagnostics count"
    );
}

/// T36: Cross-format entity ID consistency.
#[test]
fn entity_ids_match_across_yaml_json() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("main.py"),
        "def run():\n    pass\ndef helper():\n    pass\n",
    )
    .unwrap();

    let config = load_config(dir.path());
    let result = crate::analyze(dir.path(), &config, &["python".to_string()]).unwrap();

    let yaml = crate::YamlFormatter::new()
        .format_manifest(&result.manifest)
        .unwrap();
    let json = crate::JsonFormatter::new()
        .format_manifest(&result.manifest)
        .unwrap();

    let yaml_parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    let json_parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    let yaml_ids: Vec<String> = yaml_parsed["entities"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();
    let json_ids: Vec<String> = json_parsed["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();

    assert_eq!(yaml_ids, json_ids);
}

/// T37: Trace forward-only and backward-only still work after dedup changes.
#[test]
fn trace_single_direction_unaffected_by_dedup() {
    use crate::analyzer::flow::{trace_flows_from, trace_flows_to};
    use crate::parser::ir::{ReferenceKind, SymbolKind, Visibility};

    let mut g = Graph::new();
    let a = g.add_symbol(make_symbol(
        "alpha",
        SymbolKind::Function,
        Visibility::Public,
        "test.py",
        1,
    ));
    let b = g.add_symbol(make_symbol(
        "beta",
        SymbolKind::Function,
        Visibility::Public,
        "test.py",
        5,
    ));
    add_ref(&mut g, a, b, ReferenceKind::Call, "test.py");

    let forward = trace_flows_from(&g, a, 10);
    let backward = trace_flows_to(&g, b, 10);

    assert!(
        !forward.is_empty(),
        "Forward trace must find flows from alpha"
    );
    assert!(
        !backward.is_empty(),
        "Backward trace must find flows to beta"
    );
}
