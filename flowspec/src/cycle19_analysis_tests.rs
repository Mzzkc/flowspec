// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Cycle 19 QA-2 (QA-Analysis) tests — format-aware `validate_manifest_size`,
//! dogfood baseline validation, structural gate verification.

use std::path::Path;

use crate::manifest::validate_manifest_size;

const SOURCE: u64 = 10_000;

// ===========================================================================
// Section 1: Format-Aware Size Validation (T1–T10)
// ===========================================================================

/// T1: YAML at 9.5x — just under 10x limit — must PASS.
#[test]
fn test_c19_t01_yaml_just_under_limit_passes() {
    let manifest = "x".repeat(95_000); // 9.5x
    let result = validate_manifest_size(&manifest, SOURCE, "yaml");
    assert!(
        result.is_ok(),
        "T1: YAML at 9.5x should pass (limit 10x). Got: {:?}",
        result.err()
    );
}

/// T2: YAML at 10.5x — just over 10x limit — must FAIL.
#[test]
fn test_c19_t02_yaml_just_over_limit_fails() {
    let manifest = "x".repeat(105_000); // 10.5x
    let result = validate_manifest_size(&manifest, SOURCE, "yaml");
    assert!(result.is_err(), "T2: YAML at 10.5x must fail (limit 10x)");
}

/// T3: JSON at 14.5x — just under 15x limit — must PASS.
#[test]
fn test_c19_t03_json_just_under_limit_passes() {
    let manifest = "x".repeat(145_000); // 14.5x
    let result = validate_manifest_size(&manifest, SOURCE, "json");
    assert!(
        result.is_ok(),
        "T3: JSON at 14.5x should pass (limit 15x). Got: {:?}",
        result.err()
    );
}

/// T4: JSON at 15.5x — just over 15x limit — must FAIL.
#[test]
fn test_c19_t04_json_just_over_limit_fails() {
    let manifest = "x".repeat(155_000); // 15.5x
    let result = validate_manifest_size(&manifest, SOURCE, "json");
    assert!(result.is_err(), "T4: JSON at 15.5x must fail (limit 15x)");
}

/// T5: SARIF at 19.5x — just under 20x limit — must PASS.
#[test]
fn test_c19_t05_sarif_just_under_limit_passes() {
    let manifest = "x".repeat(195_000); // 19.5x
    let result = validate_manifest_size(&manifest, SOURCE, "sarif");
    assert!(
        result.is_ok(),
        "T5: SARIF at 19.5x should pass (limit 20x). Got: {:?}",
        result.err()
    );
}

/// T6: SARIF at 20.5x — just over 20x limit — must FAIL.
#[test]
fn test_c19_t06_sarif_just_over_limit_fails() {
    let manifest = "x".repeat(205_000); // 20.5x
    let result = validate_manifest_size(&manifest, SOURCE, "sarif");
    assert!(result.is_err(), "T6: SARIF at 20.5x must fail (limit 20x)");
}

/// T7: Summary at 50x — extreme ratio — must PASS (summary is exempt).
#[test]
fn test_c19_t07_summary_exempt_always_passes() {
    let manifest = "x".repeat(500_000); // 50x
    let result = validate_manifest_size(&manifest, SOURCE, "summary");
    assert!(
        result.is_ok(),
        "T7: Summary format is exempt from size limits. Got: {:?}",
        result.err()
    );
}

/// T8: ADVERSARIAL — Unknown format string falls back to strictest limit (10x).
/// This is a fail-safe: unrecognized format strings must NOT get permissive limits.
#[test]
fn test_c19_t08_unknown_format_uses_strictest_limit() {
    let manifest = "x".repeat(105_000); // 10.5x
    let result = validate_manifest_size(&manifest, SOURCE, "markdown");
    assert!(
        result.is_err(),
        "T8: Unknown format 'markdown' at 10.5x must fail — uses strictest 10x default"
    );
    // Verify it's not silently passing
    let manifest_ok = "x".repeat(95_000); // 9.5x
    let result_ok = validate_manifest_size(&manifest_ok, SOURCE, "markdown");
    assert!(
        result_ok.is_ok(),
        "T8b: Unknown format at 9.5x should still pass under 10x. Got: {:?}",
        result_ok.err()
    );
}

/// T9: Source below SIZE_CHECK_MIN_SOURCE_BYTES — skip check for ALL formats.
/// The source-too-small bypass must work identically regardless of format.
#[test]
fn test_c19_t09_tiny_source_bypasses_all_formats() {
    let manifest = "x".repeat(50_000);
    for format in &["yaml", "json", "sarif", "summary", "unknown_fmt"] {
        let result = validate_manifest_size(&manifest, 500, format);
        assert!(
            result.is_ok(),
            "T9: Source < 1024 must bypass check for format '{}'. Got: {:?}",
            format,
            result.err()
        );
    }
}

/// T10: Manifest below MIN_MANIFEST_ALLOW_BYTES — always pass for ALL formats.
/// The 20KB floor is format-independent.
#[test]
fn test_c19_t10_byte_floor_bypasses_all_formats() {
    let manifest = "x".repeat(19_000); // Under 20KB floor
    for format in &["yaml", "json", "sarif", "summary"] {
        let result = validate_manifest_size(&manifest, 1500, format);
        assert!(
            result.is_ok(),
            "T10: Manifest < 20KB must pass for format '{}' even at high ratio. Got: {:?}",
            format,
            result.err()
        );
    }
}

// ===========================================================================
// Section 2: Dogfood Baseline Validation (T11–T15)
// ===========================================================================

/// Helper: run dogfood analysis on own src/ directory.
fn run_dogfood_c19() -> Vec<crate::manifest::types::DiagnosticEntry> {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(
        src_path.exists(),
        "Source directory not found at {:?}",
        src_path
    );
    let config = crate::config::Config::load(&src_path, None).unwrap();
    let result = crate::analyze(&src_path, &config, &["rust".to_string()])
        .expect("Dogfood analysis must not fail");
    result.manifest.diagnostics
}

fn count_pattern_c19(results: &[crate::manifest::types::DiagnosticEntry], pattern: &str) -> usize {
    results.iter().filter(|d| d.pattern == pattern).count()
}

/// T11: Total dogfood findings within ±30 of C19 baseline (588).
/// Tighter range than C18's 400-650 because C19 is a quality cycle — no new
/// diagnostic patterns, no analyzer changes, no parser changes that affect Rust.
#[test]
fn test_c19_t11_total_findings_within_baseline_tolerance() {
    let results = run_dogfood_c19();
    let total = results.len();
    eprintln!("T11: total findings = {}", total);
    assert!(
        total >= 558 && total <= 618,
        "T11: total={}, expected 558-618 (C19 baseline 588 ±30)",
        total
    );
}

/// T12: data_dead_end remains dominant pattern (>200).
/// This is the highest-volume pattern and a key indicator of analyzer health.
#[test]
fn test_c19_t12_data_dead_end_dominant() {
    let results = run_dogfood_c19();
    let dead_end = count_pattern_c19(&results, "data_dead_end");
    eprintln!("T12: data_dead_end = {}", dead_end);
    assert!(
        dead_end > 200,
        "T12: data_dead_end={}, must be >200 (C18 was 252)",
        dead_end
    );
}

/// T13: Total below 600 — regression guard.
/// If total jumps above 600, either a new FP source emerged or a suppression broke.
#[test]
fn test_c19_t13_total_below_regression_ceiling() {
    let results = run_dogfood_c19();
    let total = results.len();
    eprintln!("T13: total findings = {}", total);
    assert!(
        total < 600,
        "T13: total={} exceeds 600 regression ceiling — investigate new FP source",
        total
    );
}

/// T14: data_dead_end is specifically largest pattern — no other pattern exceeds it.
/// Validates that the pattern distribution hasn't shifted unexpectedly.
#[test]
fn test_c19_t14_data_dead_end_is_largest_pattern() {
    let results = run_dogfood_c19();
    let dead_end = count_pattern_c19(&results, "data_dead_end");
    let phantom = count_pattern_c19(&results, "phantom_dependency");
    let missing = count_pattern_c19(&results, "missing_reexport");
    let orphaned = count_pattern_c19(&results, "orphaned_impl");
    eprintln!(
        "T14: data_dead_end={}, phantom_dependency={}, missing_reexport={}, orphaned_impl={}",
        dead_end, phantom, missing, orphaned
    );
    assert!(
        dead_end > phantom && dead_end > missing && dead_end > orphaned,
        "T14: data_dead_end ({}) must be the dominant pattern",
        dead_end
    );
}

/// T15: ADVERSARIAL — No unknown pattern types appear in dogfood.
/// Guards against typos in pattern name strings or unexpected new patterns.
#[test]
fn test_c19_t15_no_unknown_patterns_in_dogfood() {
    let results = run_dogfood_c19();
    let known_patterns: std::collections::HashSet<&str> = [
        "data_dead_end",
        "phantom_dependency",
        "missing_reexport",
        "orphaned_impl",
        "stale_reference",
        "circular_dependency",
        "contract_mismatch",
        "layer_violation",
        "partial_wiring",
        "isolated_cluster",
        "duplication",
        "asymmetric_handling",
        "incomplete_migration",
    ]
    .into_iter()
    .collect();

    for diag in &results {
        assert!(
            known_patterns.contains(diag.pattern.as_str()),
            "T15: Unknown pattern '{}' in dogfood — regression or typo",
            diag.pattern
        );
    }
}

// ===========================================================================
// Section 3: Structural Gate Verification (T16–T17)
// ===========================================================================

/// T16: cycle-19/issues-filed.md exists.
/// Structural gate per manager's assignment: file must exist before code work.
#[test]
fn test_c19_t16_issues_filed_gate_file_exists() {
    let workspace =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../workspaces/build/cycle-19/issues-filed.md");
    assert!(
        workspace.exists(),
        "T16: cycle-19/issues-filed.md must exist — structural gate for issue filing"
    );
}

/// T17: cycle-19/issues-filed.md contains at least 3 GitHub issue URLs.
/// The manager requires 3+ issues filed. URLs must be https://github.com/ links.
#[test]
fn test_c19_t17_issues_filed_contains_minimum_urls() {
    let workspace =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../workspaces/build/cycle-19/issues-filed.md");
    let content =
        std::fs::read_to_string(&workspace).expect("T17: issues-filed.md must be readable");
    let url_count = content
        .lines()
        .filter(|line| line.contains("https://github.com/"))
        .count();
    assert!(
        url_count >= 3,
        "T17: Found {} GitHub URLs, need at least 3. Content:\n{}",
        url_count,
        content
    );
}
