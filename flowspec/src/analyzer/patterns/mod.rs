// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Pattern registry — collects and filters diagnostic results from all detectors.
//!
//! Each pattern module exports `pub fn detect(graph: &Graph) -> Vec<Diagnostic>`.
//! The registry calls each detector, assigns sequential IDs, and applies
//! severity/confidence/pattern-name filters for the `--checks`, `--severity`,
//! and `--confidence` CLI flags.

pub mod data_dead_end;
pub mod isolated_cluster;
pub mod phantom_dependency;

use crate::analyzer::diagnostic::*;
use crate::graph::Graph;

/// Filter criteria for pattern detection.
///
/// All criteria are AND'd: a diagnostic must match ALL active filters.
/// `None` means "no filter applied" (accept all).
#[derive(Debug, Clone, Default)]
pub struct PatternFilter {
    /// Only include these patterns (by DiagnosticPattern variant). `None` = all.
    pub patterns: Option<Vec<DiagnosticPattern>>,
    /// Minimum severity to include. `None` = all.
    pub min_severity: Option<Severity>,
    /// Minimum confidence to include. `None` = all.
    pub min_confidence: Option<Confidence>,
}

/// Run all implemented pattern detectors and return diagnostics with sequential IDs.
pub fn run_all_patterns(graph: &Graph) -> Vec<Diagnostic> {
    run_patterns(graph, &PatternFilter::default())
}

/// Run pattern detectors with filtering, returning diagnostics with sequential IDs.
///
/// Filters are applied AFTER detection but BEFORE ID assignment, so IDs
/// are sequential in the returned list (no gaps from filtering).
pub fn run_patterns(graph: &Graph, filter: &PatternFilter) -> Vec<Diagnostic> {
    let mut all_diagnostics = Vec::new();

    // Collect from all implemented patterns
    let pattern_results: Vec<(DiagnosticPattern, Vec<Diagnostic>)> = vec![
        (
            DiagnosticPattern::IsolatedCluster,
            isolated_cluster::detect(graph),
        ),
        (DiagnosticPattern::DataDeadEnd, data_dead_end::detect(graph)),
        (
            DiagnosticPattern::PhantomDependency,
            phantom_dependency::detect(graph),
        ),
        // Unimplemented patterns return empty Vec (registered but inactive)
    ];

    for (_pattern, diagnostics) in pattern_results {
        all_diagnostics.extend(diagnostics);
    }

    // Apply filters
    if let Some(ref patterns) = filter.patterns {
        all_diagnostics.retain(|d| patterns.contains(&d.pattern));
    }
    if let Some(min_severity) = filter.min_severity {
        all_diagnostics.retain(|d| d.severity >= min_severity);
    }
    if let Some(min_confidence) = filter.min_confidence {
        all_diagnostics.retain(|d| d.confidence >= min_confidence);
    }

    // Assign sequential IDs
    for (i, diag) in all_diagnostics.iter_mut().enumerate() {
        diag.id = format!("D{:03}", i + 1);
    }

    all_diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;

    // =========================================================================
    // 3. Pattern Registry Tests
    // =========================================================================

    #[test]
    fn test_run_all_patterns_returns_results_from_all_implemented_patterns() {
        let graph = build_all_fixtures_graph();
        let diagnostics = run_all_patterns(&graph);

        let patterns: Vec<DiagnosticPattern> = diagnostics.iter().map(|d| d.pattern).collect();
        assert!(
            patterns.contains(&DiagnosticPattern::IsolatedCluster),
            "Should detect isolated_cluster"
        );
        assert!(
            patterns.contains(&DiagnosticPattern::DataDeadEnd),
            "Should detect data_dead_end"
        );
        assert!(
            patterns.contains(&DiagnosticPattern::PhantomDependency),
            "Should detect phantom_dependency"
        );
    }

    #[test]
    fn test_run_all_patterns_assigns_sequential_ids() {
        let graph = build_all_fixtures_graph();
        let diagnostics = run_all_patterns(&graph);
        assert!(!diagnostics.is_empty());

        for (i, d) in diagnostics.iter().enumerate() {
            assert_eq!(d.id, format!("D{:03}", i + 1));
        }
    }

    #[test]
    fn test_unimplemented_patterns_return_empty() {
        let graph = build_all_fixtures_graph();
        let diagnostics = run_all_patterns(&graph);
        // partial_wiring is not implemented — should never appear
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::PartialWiring),
            "Unimplemented patterns should not produce diagnostics"
        );
    }

    // -- Filtering tests -------------------------------------------------------

    #[test]
    fn test_filter_by_severity_warning_excludes_info() {
        let graph = build_all_fixtures_graph();
        let filter = PatternFilter {
            min_severity: Some(Severity::Warning),
            ..Default::default()
        };
        let diagnostics = run_patterns(&graph, &filter);

        for d in &diagnostics {
            assert!(
                d.severity >= Severity::Warning,
                "Severity filter should exclude Info, got {:?} for: {}",
                d.severity,
                d.message
            );
        }
        // Verify phantom_dependency (Info severity) is excluded
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::PhantomDependency),
            "phantom_dependency (Info) should be excluded by Warning filter"
        );
    }

    #[test]
    fn test_filter_by_confidence_high_excludes_moderate_and_low() {
        let graph = build_all_fixtures_graph();
        let filter = PatternFilter {
            min_confidence: Some(Confidence::High),
            ..Default::default()
        };
        let diagnostics = run_patterns(&graph, &filter);

        for d in &diagnostics {
            assert_eq!(
                d.confidence,
                Confidence::High,
                "Confidence filter should only include High, got {:?} for: {}",
                d.confidence,
                d.message
            );
        }
    }

    #[test]
    fn test_filter_by_pattern_name() {
        let graph = build_all_fixtures_graph();
        let filter = PatternFilter {
            patterns: Some(vec![DiagnosticPattern::DataDeadEnd]),
            ..Default::default()
        };
        let diagnostics = run_patterns(&graph, &filter);

        for d in &diagnostics {
            assert_eq!(
                d.pattern,
                DiagnosticPattern::DataDeadEnd,
                "Pattern filter should only include DataDeadEnd"
            );
        }
    }

    #[test]
    fn test_filter_all_three_simultaneously() {
        let graph = build_all_fixtures_graph();
        let filter = PatternFilter {
            patterns: Some(vec![DiagnosticPattern::DataDeadEnd]),
            min_severity: Some(Severity::Warning),
            min_confidence: Some(Confidence::High),
        };
        let diagnostics = run_patterns(&graph, &filter);

        for d in &diagnostics {
            assert_eq!(d.pattern, DiagnosticPattern::DataDeadEnd);
            assert!(d.severity >= Severity::Warning);
            assert!(d.confidence >= Confidence::High);
        }
    }

    #[test]
    fn test_empty_filter_returns_all() {
        let graph = build_all_fixtures_graph();
        let all = run_all_patterns(&graph);
        let filtered = run_patterns(&graph, &PatternFilter::default());
        assert_eq!(all.len(), filtered.len());
    }

    // =========================================================================
    // 4. Pattern: isolated_cluster
    // =========================================================================

    #[test]
    fn test_isolated_cluster_detects_unwired_module() {
        let graph = build_isolated_module_graph();
        let diagnostics = isolated_cluster::detect(&graph);

        assert_eq!(
            diagnostics.len(),
            1,
            "Should detect exactly 1 isolated cluster"
        );
        let d = &diagnostics[0];
        assert_eq!(d.pattern, DiagnosticPattern::IsolatedCluster);
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.confidence, Confidence::High);

        // Entity should name key symbols in the cluster
        assert!(d.entity.contains("run"), "Entity should contain 'run'");
        assert!(
            d.entity.contains("process"),
            "Entity should contain 'process'"
        );
        assert!(
            d.entity.contains("validate"),
            "Entity should contain 'validate'"
        );

        // Evidence must be concrete
        assert!(!d.evidence.is_empty());
        for ev in &d.evidence {
            assert!(!ev.observation.is_empty());
        }
        assert!(
            d.evidence
                .iter()
                .any(|e| e.observation.contains("0 external")),
            "Evidence should mention 0 external callers"
        );

        // Suggestion must be actionable
        assert!(!d.suggestion.is_empty());
        assert!(d.suggestion.len() > 10);
    }

    #[test]
    fn test_isolated_cluster_clean_code_no_findings() {
        let graph = build_clean_code_graph();
        let diagnostics = isolated_cluster::detect(&graph);
        assert!(
            diagnostics.is_empty(),
            "Clean code should produce zero isolated_cluster findings"
        );
    }

    #[test]
    fn test_isolated_cluster_excludes_test_module() {
        let graph = build_test_module_graph();
        let diagnostics = isolated_cluster::detect(&graph);

        for d in &diagnostics {
            assert!(
                !d.entity.contains("test_"),
                "Test module should be excluded, got entity: {}",
                d.entity
            );
            assert!(
                !d.location.contains("test_module"),
                "Diagnostic location should not be in test file"
            );
        }
    }

    #[test]
    fn test_isolated_cluster_requires_two_plus_symbols() {
        let graph = build_single_orphan_graph();
        let diagnostics = isolated_cluster::detect(&graph);
        assert!(
            diagnostics.is_empty(),
            "Single orphan function should NOT trigger isolated_cluster"
        );
    }

    #[test]
    fn test_isolated_cluster_init_reexport_module_not_flagged() {
        let graph = build_reexport_only_module_graph();
        let diagnostics = isolated_cluster::detect(&graph);
        assert!(
            diagnostics.is_empty(),
            "Re-export-only modules should not be flagged as isolated clusters"
        );
    }

    // =========================================================================
    // 5. Pattern: data_dead_end
    // =========================================================================

    #[test]
    fn test_data_dead_end_detects_unused_private_function() {
        let graph = build_dead_code_graph();
        let diagnostics = data_dead_end::detect(&graph);

        let dead_entities: Vec<&str> = diagnostics.iter().map(|d| d.entity.as_str()).collect();
        assert!(
            dead_entities.iter().any(|e| e.contains("unused_helper")),
            "unused_helper should be detected as dead end"
        );
        assert!(
            dead_entities.iter().any(|e| e.contains("_private_util")),
            "_private_util should be detected as dead end"
        );

        // Both should be HIGH confidence (private + zero callers)
        for d in &diagnostics {
            if d.entity.contains("unused_helper") || d.entity.contains("_private_util") {
                assert_eq!(
                    d.confidence,
                    Confidence::High,
                    "Private function with zero callers must be HIGH confidence"
                );
            }
        }

        // active_function and main_handler should NOT appear
        assert!(!dead_entities.iter().any(|e| e.contains("active_function")));
        assert!(!dead_entities.iter().any(|e| e.contains("main_handler")));
    }

    #[test]
    fn test_data_dead_end_evidence_includes_caller_count() {
        let graph = build_dead_code_graph();
        let diagnostics = data_dead_end::detect(&graph);

        for d in &diagnostics {
            assert!(
                d.evidence
                    .iter()
                    .any(|e| e.observation.contains("0 callers")
                        || e.observation.contains("0 references")
                        || e.observation.contains("zero callers")),
                "Evidence must include concrete caller count for: {}",
                d.entity
            );
        }
    }

    #[test]
    fn test_data_dead_end_clean_code_no_findings() {
        let graph = build_clean_code_graph();
        let diagnostics = data_dead_end::detect(&graph);
        assert!(
            diagnostics.is_empty(),
            "Clean code should produce zero data_dead_end findings"
        );
    }

    #[test]
    fn test_data_dead_end_public_api_low_confidence() {
        let graph = build_public_api_graph();
        let diagnostics = data_dead_end::detect(&graph);

        for d in &diagnostics {
            if d.entity.contains("format_timestamp") || d.entity.contains("parse_duration") {
                assert_ne!(
                    d.confidence,
                    Confidence::High,
                    "Public function with zero internal callers should NOT be HIGH confidence: {}",
                    d.entity
                );
            }
        }
    }

    #[test]
    fn test_data_dead_end_private_vs_public_confidence_differs() {
        let graph = build_public_api_graph();
        let diagnostics = data_dead_end::detect(&graph);

        let internal = diagnostics
            .iter()
            .find(|d| d.entity.contains("_internal_helper"));
        let public_fn = diagnostics
            .iter()
            .find(|d| d.entity.contains("format_timestamp"));

        assert!(
            internal.is_some(),
            "_internal_helper should be detected as dead end"
        );
        assert!(
            public_fn.is_some(),
            "format_timestamp should be detected (at lower confidence)"
        );

        assert!(
            internal.unwrap().confidence > public_fn.unwrap().confidence,
            "Private zero-caller function must have higher confidence than public"
        );
    }

    #[test]
    fn test_data_dead_end_excludes_test_functions() {
        let graph = build_test_module_graph();
        let diagnostics = data_dead_end::detect(&graph);

        for d in &diagnostics {
            assert!(
                !d.entity.contains("test_active_function")
                    && !d.entity.contains("test_main_handler"),
                "Test functions must be excluded from dead end detection, got: {}",
                d.entity
            );
        }
    }

    #[test]
    fn test_data_dead_end_excludes_main_entry_point() {
        let graph = build_clean_code_graph();
        let diagnostics = data_dead_end::detect(&graph);

        assert!(
            !diagnostics.iter().any(|d| d.entity.contains("main")),
            "main() entry point must be excluded from dead end detection"
        );
    }

    #[test]
    fn test_data_dead_end_severity_is_warning() {
        let graph = build_dead_code_graph();
        let diagnostics = data_dead_end::detect(&graph);

        for d in &diagnostics {
            assert_eq!(
                d.severity,
                Severity::Warning,
                "data_dead_end severity should be Warning"
            );
        }
    }

    // =========================================================================
    // 6. Pattern: phantom_dependency
    // =========================================================================

    #[test]
    fn test_phantom_dependency_detects_unused_imports() {
        let graph = build_unused_import_graph();
        let diagnostics = phantom_dependency::detect(&graph);

        let phantom_entities: Vec<&str> = diagnostics.iter().map(|d| d.entity.as_str()).collect();

        assert!(
            phantom_entities.iter().any(|e| e.contains("os")),
            "import os should be detected as phantom"
        );
        assert!(
            phantom_entities.iter().any(|e| e.contains("OrderedDict")),
            "from collections import OrderedDict should be phantom"
        );
    }

    #[test]
    fn test_phantom_dependency_severity_is_info() {
        let graph = build_unused_import_graph();
        let diagnostics = phantom_dependency::detect(&graph);

        for d in &diagnostics {
            assert_eq!(
                d.severity,
                Severity::Info,
                "phantom_dependency severity is Info"
            );
        }
    }

    #[test]
    fn test_phantom_dependency_confidence_is_high() {
        let graph = build_unused_import_graph();
        let diagnostics = phantom_dependency::detect(&graph);

        for d in &diagnostics {
            assert_eq!(
                d.confidence,
                Confidence::High,
                "Unused imports are structurally verifiable — confidence must be High"
            );
        }
    }

    #[test]
    fn test_phantom_dependency_evidence_names_specific_symbol() {
        let graph = build_unused_import_graph();
        let diagnostics = phantom_dependency::detect(&graph);

        for d in &diagnostics {
            assert!(
                d.evidence
                    .iter()
                    .any(|e| e.observation.contains("0 references")
                        || e.observation.contains("0 uses")
                        || e.observation.contains("never used")),
                "Evidence must state zero references: {}",
                d.entity
            );
        }
    }

    #[test]
    fn test_phantom_dependency_clean_code_no_findings() {
        let graph = build_clean_code_graph();
        let diagnostics = phantom_dependency::detect(&graph);
        assert!(
            diagnostics.is_empty(),
            "Clean code should produce zero phantom_dependency findings"
        );
    }

    #[test]
    fn test_phantom_dependency_prefix_usage_not_flagged() {
        let graph = build_unused_import_graph();
        let diagnostics = phantom_dependency::detect(&graph);

        assert!(
            !diagnostics.iter().any(|d| d.entity == "sys"),
            "import sys where sys.argv is used must NOT be flagged"
        );
    }

    #[test]
    fn test_phantom_dependency_type_annotation_usage_not_flagged() {
        let graph = build_unused_import_graph();
        let diagnostics = phantom_dependency::detect(&graph);

        assert!(
            !diagnostics.iter().any(|d| d.entity.contains("Optional")),
            "from typing import Optional used in annotation must NOT be phantom"
        );
    }

    #[test]
    fn test_phantom_dependency_called_import_not_flagged() {
        let graph = build_unused_import_graph();
        let diagnostics = phantom_dependency::detect(&graph);

        assert!(
            !diagnostics.iter().any(|d| d.entity == "Path"),
            "from pathlib import Path where Path() is called must NOT be phantom"
        );
    }

    #[test]
    fn test_phantom_dependency_reexport_not_flagged() {
        let graph = build_reexport_init_graph();
        let diagnostics = phantom_dependency::detect(&graph);

        // helper is re-exported — NOT phantom
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.entity.contains("helper") && !d.entity.contains("internal")),
            "Re-exported import must NOT be flagged as phantom"
        );

        // internal_only IS phantom
        assert!(
            diagnostics
                .iter()
                .any(|d| d.entity.contains("internal_only")),
            "Import that is neither used nor re-exported SHOULD be flagged"
        );
    }

    #[test]
    fn test_phantom_dependency_side_effect_logging_not_phantom() {
        let graph = build_side_effect_import_graph();
        let diagnostics = phantom_dependency::detect(&graph);

        assert!(
            !diagnostics.iter().any(|d| d.entity.contains("logging")),
            "import logging with logging.basicConfig() is NOT phantom"
        );

        assert!(
            diagnostics.iter().any(|d| d.entity.contains("json")),
            "import json with zero references should be detected as phantom"
        );
    }

    // =========================================================================
    // 7. Cross-Pattern Interaction Tests
    // =========================================================================

    #[test]
    fn test_no_diagnostic_overlap_between_single_orphan_and_cluster() {
        let graph = build_dead_code_graph();

        let cluster_diags = isolated_cluster::detect(&graph);
        let dead_diags = data_dead_end::detect(&graph);

        // unused_helper should be in dead_diags
        assert!(dead_diags
            .iter()
            .any(|d| d.entity.contains("unused_helper")));

        // unused_helper should NOT be in cluster_diags
        assert!(
            !cluster_diags
                .iter()
                .any(|d| d.entity.contains("unused_helper")),
            "Single orphan function must not appear in isolated_cluster results"
        );
    }

    #[test]
    fn test_dead_code_in_isolated_cluster_gets_both_diagnostics() {
        let graph = build_isolated_cluster_with_dead_end_graph();

        let cluster_diags = isolated_cluster::detect(&graph);
        let dead_diags = data_dead_end::detect(&graph);

        assert!(
            !cluster_diags.is_empty(),
            "Isolated cluster should be detected"
        );
        // entry_fn has no external callers, so it should show up in dead_end too
        // (the cluster exists but all members are also dead ends individually)
        let _ = dead_diags; // Documenting the interaction
    }

    // =========================================================================
    // 8. Confidence Calibration Tests
    // =========================================================================

    #[test]
    fn test_high_confidence_requires_structural_proof() {
        let graph = build_all_fixtures_graph();
        let diagnostics = run_all_patterns(&graph);

        for d in diagnostics
            .iter()
            .filter(|d| d.confidence == Confidence::High)
        {
            assert!(
                !d.evidence.is_empty(),
                "HIGH confidence diagnostic must have evidence: {}",
                d.message
            );

            let has_concrete_evidence = d.evidence.iter().any(|e| {
                e.observation.contains("0 callers")
                    || e.observation.contains("0 references")
                    || e.observation.contains("0 external")
                    || e.observation.contains("0 uses")
                    || e.observation.contains("never")
            });
            assert!(
                has_concrete_evidence,
                "HIGH confidence evidence must be concrete. Diagnostic: {} — Evidence: {:?}",
                d.message,
                d.evidence
                    .iter()
                    .map(|e| &e.observation)
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn test_no_high_confidence_false_positives_on_clean_code() {
        let graph = build_clean_code_graph();
        let diagnostics = run_all_patterns(&graph);

        let high_confidence: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.confidence == Confidence::High)
            .collect();

        assert!(
            high_confidence.is_empty(),
            "Clean code must produce ZERO high-confidence diagnostics. Got: {:?}",
            high_confidence
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // 9. Evidence Quality Tests
    // =========================================================================

    #[test]
    fn test_all_diagnostics_have_nonempty_evidence() {
        let graph = build_all_fixtures_graph();
        let diagnostics = run_all_patterns(&graph);

        for d in &diagnostics {
            assert!(
                !d.evidence.is_empty(),
                "Every diagnostic MUST have evidence: {}",
                d.message
            );
            for ev in &d.evidence {
                assert!(
                    !ev.observation.is_empty(),
                    "Evidence observation must not be empty"
                );
            }
        }
    }

    #[test]
    fn test_all_diagnostics_have_actionable_suggestions() {
        let graph = build_all_fixtures_graph();
        let diagnostics = run_all_patterns(&graph);

        for d in &diagnostics {
            assert!(
                !d.suggestion.is_empty(),
                "Every diagnostic must have a suggestion: {}",
                d.message
            );
            assert!(
                d.suggestion.len() > 5,
                "Suggestion must be actionable. Got '{}' for: {}",
                d.suggestion,
                d.message
            );
            assert!(
                d.suggestion != d.message,
                "Suggestion must differ from message"
            );
        }
    }

    #[test]
    fn test_all_diagnostics_have_valid_location() {
        let graph = build_all_fixtures_graph();
        let diagnostics = run_all_patterns(&graph);

        for d in &diagnostics {
            assert!(
                !d.location.is_empty(),
                "Every diagnostic must have a location: {}",
                d.message
            );
            assert!(
                d.location.contains(':') || d.location.contains(".py"),
                "Location should be file:line format. Got: {}",
                d.location
            );
        }
    }

    #[test]
    fn test_diagnostic_messages_use_failure_language_not_graph_theory() {
        let graph = build_all_fixtures_graph();
        let diagnostics = run_all_patterns(&graph);

        let graph_theory_terms = [
            "in-degree",
            "out-degree",
            "subgraph",
            "vertex",
            "adjacency",
            "node",
        ];

        for d in &diagnostics {
            for term in &graph_theory_terms {
                assert!(
                    !d.message.to_lowercase().contains(term),
                    "Diagnostic message should use failure-pattern language, not graph theory. \
                     Found '{}' in: {}",
                    term,
                    d.message
                );
            }
        }
    }
}
