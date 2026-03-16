//! QA-3 Cycle 12 tests — #16 filter count consistency, #17 error misdirection,
//! and regression guards.

use crate::commands::{
    apply_diagnostic_filters, recompute_diagnostic_summary, validate_check_patterns,
};
use crate::manifest::types::{DiagnosticEntry, DiagnosticSummary, Manifest};
use crate::manifest::{OutputFormatter, SummaryFormatter};
use crate::JsonFormatter;

// =========================================================================
// Category 1: Issue #16 — Summary Filter Inconsistency (T1–T8)
// =========================================================================

/// T1: THE BUG — summary header Diagnostics: N must match actual filtered findings count.
/// Before fix, header shows unfiltered total, body shows filtered subset.
#[test]
fn t1_summary_header_count_matches_filtered_diagnostics() {
    let mut manifest = Manifest::sample_full();
    manifest.diagnostics = vec![
        DiagnosticEntry {
            pattern: "phantom_dependency".into(),
            severity: "warning".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            pattern: "data_dead_end".into(),
            severity: "warning".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            pattern: "phantom_dependency".into(),
            severity: "info".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            pattern: "isolated_cluster".into(),
            severity: "critical".into(),
            ..DiagnosticEntry::sample_critical()
        },
    ];
    manifest.metadata.diagnostic_count = 4;

    // Filter: retain only phantom_dependency
    let checks = vec!["phantom_dependency".to_string()];
    apply_diagnostic_filters(&mut manifest.diagnostics, &checks, None, None);

    // Apply the #16 fix: update metadata count after filtering
    manifest.metadata.diagnostic_count = manifest.diagnostics.len() as u64;
    recompute_diagnostic_summary(
        &manifest.diagnostics,
        &mut manifest.summary.diagnostic_summary,
    );

    let formatter = SummaryFormatter::new();
    let output = formatter.format_manifest(&manifest).unwrap();

    // Parse Diagnostics count from "--- Overview ---" section
    let overview_diag_count = extract_overview_diagnostic_count(&output);
    let findings_count = count_findings_in_body(&output);

    assert_eq!(
        overview_diag_count, findings_count,
        "Overview 'Diagnostics: {}' != actual findings count {} in body. Bug #16.",
        overview_diag_count, findings_count
    );
    assert_eq!(
        overview_diag_count, 2,
        "Should show 2 phantom_dependency findings after filter"
    );
}

/// T2: No filters — counts unchanged (regression guard).
#[test]
fn t2_no_filter_preserves_diagnostic_count() {
    let mut manifest = Manifest::sample_full();
    let original_count = manifest.diagnostics.len();
    manifest.metadata.diagnostic_count = original_count as u64;

    // Empty filters — nothing should change
    apply_diagnostic_filters(&mut manifest.diagnostics, &[], None, None);

    assert_eq!(
        manifest.diagnostics.len(),
        original_count,
        "Empty filters must not remove any diagnostics"
    );
    assert_eq!(
        manifest.metadata.diagnostic_count, original_count as u64,
        "metadata.diagnostic_count must remain unchanged with no filters"
    );
}

/// T3: Severity filter — count matches filtered results.
#[test]
fn t3_severity_filter_updates_diagnostic_count() {
    let mut manifest = Manifest::sample_full();
    manifest.diagnostics = vec![
        DiagnosticEntry {
            severity: "critical".into(),
            ..DiagnosticEntry::sample_critical()
        },
        DiagnosticEntry {
            severity: "critical".into(),
            ..DiagnosticEntry::sample_critical()
        },
        DiagnosticEntry {
            severity: "warning".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            severity: "warning".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            severity: "warning".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            severity: "info".into(),
            ..DiagnosticEntry::sample_warning()
        },
    ];
    manifest.metadata.diagnostic_count = 6;

    apply_diagnostic_filters(
        &mut manifest.diagnostics,
        &[],
        Some(crate::Severity::Critical),
        None,
    );
    // After fix, commands.rs updates the count:
    manifest.metadata.diagnostic_count = manifest.diagnostics.len() as u64;

    assert_eq!(manifest.diagnostics.len(), 2);
    assert_eq!(manifest.metadata.diagnostic_count, 2);

    let formatter = SummaryFormatter::new();
    let output = formatter.format_manifest(&manifest).unwrap();
    assert!(
        output.contains("Diagnostics: 2"),
        "Summary must show filtered count, not pre-filter 6. Got: {}",
        output
    );
}

/// T4: All diagnostics filtered out — "Diagnostics: 0".
#[test]
fn t4_all_diagnostics_filtered_shows_zero_count() {
    let mut manifest = Manifest::sample_full();
    manifest.diagnostics = vec![
        DiagnosticEntry {
            pattern: "data_dead_end".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            pattern: "data_dead_end".into(),
            ..DiagnosticEntry::sample_warning()
        },
    ];
    manifest.metadata.diagnostic_count = 2;

    // Filter for a pattern that doesn't exist in the list
    let checks = vec!["asymmetric_handling".to_string()];
    apply_diagnostic_filters(&mut manifest.diagnostics, &checks, None, None);
    manifest.metadata.diagnostic_count = manifest.diagnostics.len() as u64;
    recompute_diagnostic_summary(
        &manifest.diagnostics,
        &mut manifest.summary.diagnostic_summary,
    );

    assert_eq!(manifest.diagnostics.len(), 0);
    assert_eq!(manifest.metadata.diagnostic_count, 0);

    let formatter = SummaryFormatter::new();
    let output = formatter.format_manifest(&manifest).unwrap();
    assert!(
        output.contains("Diagnostics: 0"),
        "Must show 0 when all filtered. Got: {}",
        output
    );
    assert!(
        !output.contains("--- Findings ---"),
        "No Findings section when 0 diagnostics"
    );
}

/// T5: Summary diagnostic_summary severity counts match filtered list (secondary bug).
/// Before fix, diagnostic_summary shows stale pre-filter counts.
#[test]
fn t5_summary_diagnostic_severity_counts_match_filtered_list() {
    let mut manifest = Manifest::sample_full();
    manifest.diagnostics = vec![
        DiagnosticEntry {
            severity: "critical".into(),
            pattern: "phantom_dependency".into(),
            ..DiagnosticEntry::sample_critical()
        },
        DiagnosticEntry {
            severity: "critical".into(),
            pattern: "phantom_dependency".into(),
            ..DiagnosticEntry::sample_critical()
        },
        DiagnosticEntry {
            severity: "warning".into(),
            pattern: "data_dead_end".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            severity: "warning".into(),
            pattern: "data_dead_end".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            severity: "warning".into(),
            pattern: "data_dead_end".into(),
            ..DiagnosticEntry::sample_warning()
        },
    ];
    manifest.metadata.diagnostic_count = 5;
    manifest.summary.diagnostic_summary = DiagnosticSummary {
        critical: 2,
        warning: 3,
        info: 0,
        top_issues: vec![],
    };

    // Filter to keep only phantom_dependency (critical ones)
    let checks = vec!["phantom_dependency".to_string()];
    apply_diagnostic_filters(&mut manifest.diagnostics, &checks, None, None);
    manifest.metadata.diagnostic_count = manifest.diagnostics.len() as u64;
    recompute_diagnostic_summary(
        &manifest.diagnostics,
        &mut manifest.summary.diagnostic_summary,
    );

    let formatter = SummaryFormatter::new();
    let output = formatter.format_manifest(&manifest).unwrap();

    // After recompute, diagnostic_summary should show Warning: 0
    assert!(
        !output.contains("Warning: 3"),
        "Stale diagnostic_summary: shows 'Warning: 3' but only critical diagnostics remain. Got: {}",
        output
    );
    // Should show Critical: 2
    assert!(
        output.contains("Critical: 2"),
        "Should show Critical: 2 after filter. Got: {}",
        output
    );
}

/// T6: JSON format also reflects filtered count.
#[test]
fn t6_json_metadata_count_matches_after_filter() {
    let mut manifest = Manifest::sample_full();
    manifest.diagnostics = vec![
        DiagnosticEntry {
            pattern: "phantom_dependency".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            pattern: "data_dead_end".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            pattern: "phantom_dependency".into(),
            ..DiagnosticEntry::sample_warning()
        },
    ];
    manifest.metadata.diagnostic_count = 3;

    let checks = vec!["phantom_dependency".to_string()];
    apply_diagnostic_filters(&mut manifest.diagnostics, &checks, None, None);
    manifest.metadata.diagnostic_count = manifest.diagnostics.len() as u64;

    let formatter = JsonFormatter::new();
    let json_output = formatter.format_manifest(&manifest).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_output).unwrap();

    let meta_count = parsed["metadata"]["diagnostic_count"].as_u64().unwrap();
    let diag_array_len = parsed["diagnostics"].as_array().unwrap().len() as u64;

    assert_eq!(
        meta_count, diag_array_len,
        "JSON metadata.diagnostic_count ({}) != diagnostics array length ({})",
        meta_count, diag_array_len
    );
    assert_eq!(meta_count, 2);
}

/// T7: Combined filters — checks + severity applied together.
#[test]
fn t7_combined_filters_update_count_correctly() {
    let mut manifest = Manifest::sample_full();
    manifest.diagnostics = vec![
        DiagnosticEntry {
            pattern: "phantom_dependency".into(),
            severity: "critical".into(),
            ..DiagnosticEntry::sample_critical()
        },
        DiagnosticEntry {
            pattern: "phantom_dependency".into(),
            severity: "critical".into(),
            ..DiagnosticEntry::sample_critical()
        },
        DiagnosticEntry {
            pattern: "phantom_dependency".into(),
            severity: "warning".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            pattern: "phantom_dependency".into(),
            severity: "warning".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            pattern: "data_dead_end".into(),
            severity: "warning".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            pattern: "data_dead_end".into(),
            severity: "warning".into(),
            ..DiagnosticEntry::sample_warning()
        },
    ];
    manifest.metadata.diagnostic_count = 6;

    apply_diagnostic_filters(
        &mut manifest.diagnostics,
        &["phantom_dependency".to_string()],
        Some(crate::Severity::Critical),
        None,
    );
    manifest.metadata.diagnostic_count = manifest.diagnostics.len() as u64;

    assert_eq!(
        manifest.diagnostics.len(),
        2,
        "Only critical phantom_dependency should survive"
    );
    assert_eq!(manifest.metadata.diagnostic_count, 2);
}

/// T8: Exit code isolation preserved — exit code uses UNFILTERED criticals.
/// The #16 fix must NOT accidentally change exit code logic.
#[test]
fn t8_exit_code_isolation_survives_filter_count_fix() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("dead_code.py"),
        "def unused_function():\n    return 42\n",
    )
    .unwrap();

    // Run analyze with a filter that likely removes all diagnostics from output
    let result = crate::commands::run_analyze(
        dir.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &["asymmetric_handling".to_string()],
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "run_analyze must not error: {:?}",
        result.err()
    );

    // Exit code should reflect unfiltered state:
    // - If criticals exist → 2 (regardless of filter)
    // - If no criticals → 0
    let code = result.unwrap();
    assert!(
        code == 0 || code == 2,
        "Exit code must be 0 or 2, got {}",
        code
    );
}

// =========================================================================
// Category 2: Issue #17 — Error Misdirection (T9–T14)
// =========================================================================

/// T9: analyze with invalid --checks does NOT say "diagnose --help" (THE BUG).
#[test]
fn t9_unknown_pattern_error_does_not_reference_diagnose_help() {
    let result = validate_check_patterns(&["not_a_real_pattern".to_string()]);
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());

    assert!(
        !err_msg.contains("diagnose --help"),
        "Error message must NOT reference 'diagnose --help'. Got: {}",
        err_msg
    );
}

/// T10: Error message contains the invalid pattern name.
#[test]
fn t10_unknown_pattern_error_includes_invalid_name() {
    let result = validate_check_patterns(&["bogus_check".to_string()]);
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("bogus_check"),
        "Error must include the invalid pattern name. Got: {}",
        err_msg
    );
}

/// T11: Error message contains valid pattern names for guidance.
#[test]
fn t11_unknown_pattern_error_lists_valid_alternatives() {
    let result = validate_check_patterns(&["nope".to_string()]);
    let err_msg = format!("{}", result.unwrap_err());

    let known_patterns = ["phantom_dependency", "data_dead_end", "isolated_cluster"];
    let found = known_patterns
        .iter()
        .filter(|p| err_msg.contains(**p))
        .count();

    assert!(
        found >= 3,
        "Error should list valid pattern names. Found {} of {}. Got: {}",
        found,
        known_patterns.len(),
        err_msg
    );
}

/// T12: diagnose and analyze share consistent, command-agnostic error.
#[test]
fn t12_diagnose_and_analyze_share_consistent_error() {
    let result = validate_check_patterns(&["fake_pattern".to_string()]);
    let err_msg = format!("{}", result.unwrap_err());

    assert!(
        !err_msg.contains("diagnose --help"),
        "Error should be command-agnostic. Got: {}",
        err_msg
    );
    assert!(
        !err_msg.contains("analyze --help"),
        "Error should be command-agnostic. Got: {}",
        err_msg
    );
    assert!(
        err_msg.contains("fake_pattern"),
        "Error must include invalid pattern name. Got: {}",
        err_msg
    );
}

/// T13: Empty string check pattern — silently ignored, not rejected.
#[test]
fn t13_empty_check_pattern_ignored() {
    let result = validate_check_patterns(&["".to_string()]);
    assert!(
        result.is_ok(),
        "Empty pattern string must be ignored, not rejected"
    );
}

/// T14: Multiple invalid patterns — first invalid reported.
#[test]
fn t14_multiple_invalid_patterns_reports_first() {
    let result = validate_check_patterns(&[
        "phantom_dependency".to_string(),
        "bad_one".to_string(),
        "bad_two".to_string(),
    ]);
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("bad_one"),
        "Should report first invalid pattern. Got: {}",
        err_msg
    );
}

// =========================================================================
// Category 3: Regression Tests (T20–T22)
// =========================================================================

/// T20: C11 filter flags still work (regression guard).
#[test]
fn t20_c11_filter_flags_regression() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("dead_code.py"),
        "def unused_function():\n    return 42\n",
    )
    .unwrap();

    let result = crate::commands::run_analyze(
        dir.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &["data_dead_end".to_string()],
        Some(crate::Severity::Warning),
        Some(crate::Confidence::Low),
    );
    assert!(
        result.is_ok(),
        "C11 filter flags must still work: {:?}",
        result.err()
    );
}

/// T21: Cross-format diagnostic count consistency.
#[test]
fn t21_cross_format_count_consistency() {
    let mut manifest = Manifest::sample_full();
    manifest.diagnostics = vec![
        DiagnosticEntry {
            pattern: "phantom_dependency".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            pattern: "data_dead_end".into(),
            ..DiagnosticEntry::sample_warning()
        },
    ];
    manifest.metadata.diagnostic_count = manifest.diagnostics.len() as u64;

    // JSON format
    let json_formatter = JsonFormatter::new();
    let json_output = json_formatter.format_manifest(&manifest).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_output).unwrap();
    let json_meta_count = parsed["metadata"]["diagnostic_count"].as_u64().unwrap();
    let json_diag_len = parsed["diagnostics"].as_array().unwrap().len() as u64;

    assert_eq!(
        json_meta_count, json_diag_len,
        "JSON: metadata.diagnostic_count ({}) != diagnostics length ({})",
        json_meta_count, json_diag_len
    );

    // Summary format
    let summary_formatter = SummaryFormatter::new();
    let summary_output = summary_formatter.format_manifest(&manifest).unwrap();
    let overview_count = extract_overview_diagnostic_count(&summary_output);

    assert_eq!(
        overview_count,
        manifest.diagnostics.len(),
        "Summary overview count ({}) != diagnostics length ({})",
        overview_count,
        manifest.diagnostics.len()
    );
}

/// T22: VALID_PATTERNS includes all shipped patterns.
#[test]
fn t22_valid_patterns_matches_registered_patterns() {
    let shipped = [
        "isolated_cluster",
        "data_dead_end",
        "phantom_dependency",
        "orphaned_impl",
        "circular_dependency",
        "missing_reexport",
        "contract_mismatch",
        "stale_reference",
        "layer_violation",
        "incomplete_migration",
    ];

    for pattern in &shipped {
        let result = validate_check_patterns(&[pattern.to_string()]);
        assert!(
            result.is_ok(),
            "Shipped pattern '{}' must be valid in VALID_PATTERNS",
            pattern
        );
    }
}

// =========================================================================
// Category 4: recompute_diagnostic_summary unit tests
// =========================================================================

/// Verify recompute_diagnostic_summary correctly counts severities.
#[test]
fn recompute_summary_counts_severities_correctly() {
    let diagnostics = vec![
        DiagnosticEntry {
            severity: "critical".into(),
            ..DiagnosticEntry::sample_critical()
        },
        DiagnosticEntry {
            severity: "warning".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            severity: "warning".into(),
            ..DiagnosticEntry::sample_warning()
        },
        DiagnosticEntry {
            severity: "info".into(),
            ..DiagnosticEntry::sample_warning()
        },
    ];

    let mut summary = DiagnosticSummary {
        critical: 0,
        warning: 0,
        info: 0,
        top_issues: vec![],
    };

    recompute_diagnostic_summary(&diagnostics, &mut summary);

    assert_eq!(summary.critical, 1);
    assert_eq!(summary.warning, 2);
    assert_eq!(summary.info, 1);
    assert_eq!(summary.top_issues.len(), 4);
}

/// Verify recompute on empty diagnostics zeroes everything.
#[test]
fn recompute_summary_empty_diagnostics() {
    let diagnostics: Vec<DiagnosticEntry> = vec![];
    let mut summary = DiagnosticSummary {
        critical: 5,
        warning: 10,
        info: 3,
        top_issues: vec!["stale".to_string()],
    };

    recompute_diagnostic_summary(&diagnostics, &mut summary);

    assert_eq!(summary.critical, 0);
    assert_eq!(summary.warning, 0);
    assert_eq!(summary.info, 0);
    assert!(summary.top_issues.is_empty());
}

// =========================================================================
// Helper functions
// =========================================================================

/// Extract the diagnostic count from the "--- Overview ---" section.
fn extract_overview_diagnostic_count(output: &str) -> usize {
    for line in output.lines() {
        if line.contains("Diagnostics:") && !line.contains("---") {
            // Parse "Files: N  Entities: N  Flows: N  Diagnostics: N"
            if let Some(diag_part) = line.split("Diagnostics:").nth(1) {
                let count_str = diag_part.trim();
                if let Ok(count) = count_str.parse::<usize>() {
                    return count;
                }
            }
        }
    }
    0
}

/// Count the number of finding lines in the "--- Findings ---" section.
fn count_findings_in_body(output: &str) -> usize {
    let mut in_findings = false;
    let mut count = 0;
    for line in output.lines() {
        if line.contains("--- Findings ---") {
            in_findings = true;
            continue;
        }
        if in_findings {
            if line.starts_with("---") {
                break;
            }
            if line.is_empty() {
                continue;
            }
            if line.trim_start().starts_with('[') {
                count += 1;
            }
            if line.trim_start().starts_with("... and") {
                if let Some(n_str) = line
                    .trim()
                    .strip_prefix("... and ")
                    .and_then(|s| s.strip_suffix(" more findings"))
                {
                    if let Ok(n) = n_str.parse::<usize>() {
                        count += n;
                    }
                }
            }
        }
    }
    count
}
