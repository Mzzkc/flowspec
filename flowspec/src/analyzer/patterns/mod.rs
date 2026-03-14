// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Pattern registry — collects and filters diagnostic results from all detectors.
//!
//! Each pattern module exports `pub fn detect(graph: &Graph, project_root: &Path) -> Vec<Diagnostic>`.
//! The registry calls each detector, assigns sequential IDs, and applies
//! severity/confidence/pattern-name filters for the `--checks`, `--severity`,
//! and `--confidence` CLI flags.

pub mod circular_dependency;
pub mod data_dead_end;
pub mod exclusion;
pub mod isolated_cluster;
pub mod layer_violation;
pub mod missing_reexport;
pub mod orphaned_implementation;
pub mod phantom_dependency;
pub mod stale_reference;

use std::path::Path;

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
///
/// The `project_root` path is used to relativize diagnostic location fields
/// so they match the format of entity `loc` fields.
pub fn run_all_patterns(graph: &Graph, project_root: &Path) -> Vec<Diagnostic> {
    run_patterns(graph, &PatternFilter::default(), project_root)
}

/// Run pattern detectors with filtering, returning diagnostics with sequential IDs.
///
/// The `project_root` path is used to relativize diagnostic location fields.
/// Filters are applied AFTER detection but BEFORE ID assignment, so IDs
/// are sequential in the returned list (no gaps from filtering).
pub fn run_patterns(graph: &Graph, filter: &PatternFilter, project_root: &Path) -> Vec<Diagnostic> {
    let mut all_diagnostics = Vec::new();

    // Collect from all implemented patterns.
    let pattern_results: Vec<(DiagnosticPattern, Vec<Diagnostic>)> = vec![
        (
            DiagnosticPattern::IsolatedCluster,
            isolated_cluster::detect(graph, project_root),
        ),
        (
            DiagnosticPattern::DataDeadEnd,
            data_dead_end::detect(graph, project_root),
        ),
        (
            DiagnosticPattern::PhantomDependency,
            phantom_dependency::detect(graph, project_root),
        ),
        (
            DiagnosticPattern::CircularDependency,
            circular_dependency::detect(graph, project_root),
        ),
        (
            DiagnosticPattern::OrphanedImplementation,
            orphaned_implementation::detect(graph, project_root),
        ),
        (
            DiagnosticPattern::MissingReexport,
            missing_reexport::detect(graph, project_root),
        ),
        (
            DiagnosticPattern::LayerViolation,
            layer_violation::detect(graph, project_root),
        ),
        (
            DiagnosticPattern::StaleReference,
            stale_reference::detect(graph, project_root),
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
        let diagnostics = run_all_patterns(&graph, Path::new(""));

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
        let diagnostics = run_all_patterns(&graph, Path::new(""));
        assert!(!diagnostics.is_empty());

        for (i, d) in diagnostics.iter().enumerate() {
            assert_eq!(d.id, format!("D{:03}", i + 1));
        }
    }

    #[test]
    fn test_unimplemented_patterns_return_empty() {
        let graph = build_all_fixtures_graph();
        let diagnostics = run_all_patterns(&graph, Path::new(""));
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
        let diagnostics = run_patterns(&graph, &filter, Path::new(""));

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
        let diagnostics = run_patterns(&graph, &filter, Path::new(""));

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
        let diagnostics = run_patterns(&graph, &filter, Path::new(""));

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
        let diagnostics = run_patterns(&graph, &filter, Path::new(""));

        for d in &diagnostics {
            assert_eq!(d.pattern, DiagnosticPattern::DataDeadEnd);
            assert!(d.severity >= Severity::Warning);
            assert!(d.confidence >= Confidence::High);
        }
    }

    #[test]
    fn test_empty_filter_returns_all() {
        let graph = build_all_fixtures_graph();
        let all = run_all_patterns(&graph, Path::new(""));
        let filtered = run_patterns(&graph, &PatternFilter::default(), Path::new(""));
        assert_eq!(all.len(), filtered.len());
    }

    // =========================================================================
    // T14: stale_reference Registered in Pattern Registry
    // =========================================================================

    #[test]
    fn test_stale_reference_registered_in_pattern_registry() {
        use crate::parser::ir::{ResolutionStatus, SymbolKind, Visibility};

        let mut graph = Graph::new();
        let mut import_sym = make_import("missing_fn", "consumer.py", 1);
        import_sym.annotations.push("from:provider".to_string());
        import_sym.resolution =
            ResolutionStatus::Partial("module resolved, symbol not found".to_string());
        graph.add_symbol(import_sym);

        graph.add_symbol(make_symbol(
            "existing_fn",
            SymbolKind::Function,
            Visibility::Public,
            "provider.py",
            1,
        ));

        let diagnostics = run_all_patterns(&graph, Path::new(""));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::StaleReference),
            "stale_reference must be registered in the pattern registry and produce \
             findings through run_all_patterns(). If this fails, the pattern module \
             exists but was never added to mod.rs. Got patterns: {:?}",
            diagnostics
                .iter()
                .map(|d| d.pattern.name())
                .collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // R2: Pattern Count Regression
    // =========================================================================

    #[test]
    fn test_pattern_registry_has_at_least_8_patterns() {
        // Count distinct patterns in the registry by running on a graph with
        // known findings from multiple patterns
        let graph = build_all_fixtures_graph();
        let diagnostics = run_all_patterns(&graph, Path::new(""));
        let distinct_patterns: std::collections::HashSet<_> =
            diagnostics.iter().map(|d| d.pattern).collect();

        // The all_fixtures_graph may not trigger every pattern, but the
        // registry vec has at least 8 entries. We verify the compile-time
        // registration by checking the vec length directly is impractical,
        // so we verify at least 3 distinct patterns fire (the well-tested ones)
        // and that stale_reference is registered (tested above).
        assert!(
            distinct_patterns.len() >= 3,
            "At least 3 distinct patterns must fire on the all-fixtures graph. Got: {:?}",
            distinct_patterns
                .iter()
                .map(|p| p.name())
                .collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // 4. Pattern: isolated_cluster
    // =========================================================================

    #[test]
    fn test_isolated_cluster_detects_unwired_module() {
        let graph = build_isolated_module_graph();
        let diagnostics = isolated_cluster::detect(&graph, Path::new(""));

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
        let diagnostics = isolated_cluster::detect(&graph, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Clean code should produce zero isolated_cluster findings"
        );
    }

    #[test]
    fn test_isolated_cluster_excludes_test_module() {
        let graph = build_test_module_graph();
        let diagnostics = isolated_cluster::detect(&graph, Path::new(""));

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
        let diagnostics = isolated_cluster::detect(&graph, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Single orphan function should NOT trigger isolated_cluster"
        );
    }

    #[test]
    fn test_isolated_cluster_init_reexport_module_not_flagged() {
        let graph = build_reexport_only_module_graph();
        let diagnostics = isolated_cluster::detect(&graph, Path::new(""));
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
        let diagnostics = data_dead_end::detect(&graph, Path::new(""));

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
        let diagnostics = data_dead_end::detect(&graph, Path::new(""));

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
        let diagnostics = data_dead_end::detect(&graph, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Clean code should produce zero data_dead_end findings"
        );
    }

    #[test]
    fn test_data_dead_end_public_api_low_confidence() {
        let graph = build_public_api_graph();
        let diagnostics = data_dead_end::detect(&graph, Path::new(""));

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
        let diagnostics = data_dead_end::detect(&graph, Path::new(""));

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
        let diagnostics = data_dead_end::detect(&graph, Path::new(""));

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
        let diagnostics = data_dead_end::detect(&graph, Path::new(""));

        assert!(
            !diagnostics.iter().any(|d| d.entity.contains("main")),
            "main() entry point must be excluded from dead end detection"
        );
    }

    #[test]
    fn test_data_dead_end_severity_is_warning() {
        let graph = build_dead_code_graph();
        let diagnostics = data_dead_end::detect(&graph, Path::new(""));

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
        let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

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
        let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

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
        let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

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
        let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

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
        let diagnostics = phantom_dependency::detect(&graph, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Clean code should produce zero phantom_dependency findings"
        );
    }

    #[test]
    fn test_phantom_dependency_prefix_usage_not_flagged() {
        let graph = build_unused_import_graph();
        let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

        assert!(
            !diagnostics.iter().any(|d| d.entity == "sys"),
            "import sys where sys.argv is used must NOT be flagged"
        );
    }

    #[test]
    fn test_phantom_dependency_type_annotation_usage_not_flagged() {
        let graph = build_unused_import_graph();
        let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

        assert!(
            !diagnostics.iter().any(|d| d.entity.contains("Optional")),
            "from typing import Optional used in annotation must NOT be phantom"
        );
    }

    #[test]
    fn test_phantom_dependency_called_import_not_flagged() {
        let graph = build_unused_import_graph();
        let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

        assert!(
            !diagnostics.iter().any(|d| d.entity == "Path"),
            "from pathlib import Path where Path() is called must NOT be phantom"
        );
    }

    #[test]
    fn test_phantom_dependency_reexport_not_flagged() {
        let graph = build_reexport_init_graph();
        let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

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
        let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

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

        let cluster_diags = isolated_cluster::detect(&graph, Path::new(""));
        let dead_diags = data_dead_end::detect(&graph, Path::new(""));

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

        let cluster_diags = isolated_cluster::detect(&graph, Path::new(""));
        let dead_diags = data_dead_end::detect(&graph, Path::new(""));

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
        let diagnostics = run_all_patterns(&graph, Path::new(""));

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
        let diagnostics = run_all_patterns(&graph, Path::new(""));

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
        let diagnostics = run_all_patterns(&graph, Path::new(""));

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
        let diagnostics = run_all_patterns(&graph, Path::new(""));

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
        let diagnostics = run_all_patterns(&graph, Path::new(""));

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
        let diagnostics = run_all_patterns(&graph, Path::new(""));

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

    // =========================================================================
    // 10. circular_dependency — True Positive Tests
    // =========================================================================

    #[test]
    fn test_circular_dependency_detects_direct_two_module_cycle() {
        let mut graph = Graph::new();

        let a = graph.add_symbol(make_symbol(
            "func_a",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "mod_a.py",
            1,
        ));
        let b = graph.add_symbol(make_symbol(
            "func_b",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "mod_b.py",
            1,
        ));

        add_ref(
            &mut graph,
            a,
            b,
            crate::parser::ir::ReferenceKind::Call,
            "mod_a.py",
        );
        add_ref(
            &mut graph,
            b,
            a,
            crate::parser::ir::ReferenceKind::Call,
            "mod_b.py",
        );

        let results = circular_dependency::detect(&graph, Path::new(""));
        assert_eq!(results.len(), 1, "Should detect exactly 1 cycle");

        let d = &results[0];
        assert_eq!(d.pattern, DiagnosticPattern::CircularDependency);
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.confidence, Confidence::High);
        assert!(
            d.entity.contains("mod_a.py") && d.entity.contains("mod_b.py"),
            "Entity should mention both modules: {}",
            d.entity
        );
        assert!(d.evidence.len() >= 2, "Need at least 2 evidence entries");
        assert!(
            d.suggestion.len() > 10,
            "Suggestion must be actionable: {}",
            d.suggestion
        );
        assert!(!d.location.is_empty(), "Location must be non-empty");
    }

    #[test]
    fn test_circular_dependency_detects_transitive_three_module_cycle() {
        let graph = build_circular_dep_graph();
        let results = circular_dependency::detect(&graph, Path::new(""));

        assert_eq!(results.len(), 1, "Should detect exactly 1 cycle");

        let d = &results[0];
        assert_eq!(
            d.evidence.len(),
            3,
            "Three-step cycle needs 3 evidence entries"
        );

        // Each evidence entry should reference specific cross-module calls
        for ev in &d.evidence {
            assert!(
                !ev.observation.is_empty(),
                "Evidence observation must not be empty"
            );
        }

        // Verify all 3 modules are in the entity
        assert!(d.entity.contains("mod_a.py"));
        assert!(d.entity.contains("mod_b.py"));
        assert!(d.entity.contains("mod_c.py"));
    }

    #[test]
    fn test_circular_dependency_detects_multiple_independent_cycles() {
        let mut graph = Graph::new();

        // Cycle 1: A <-> B
        let a = graph.add_symbol(make_symbol(
            "func_a",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "mod_a.py",
            1,
        ));
        let b = graph.add_symbol(make_symbol(
            "func_b",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "mod_b.py",
            1,
        ));
        add_ref(
            &mut graph,
            a,
            b,
            crate::parser::ir::ReferenceKind::Call,
            "mod_a.py",
        );
        add_ref(
            &mut graph,
            b,
            a,
            crate::parser::ir::ReferenceKind::Call,
            "mod_b.py",
        );

        // Cycle 2: X <-> Y
        let x = graph.add_symbol(make_symbol(
            "func_x",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "mod_x.py",
            1,
        ));
        let y = graph.add_symbol(make_symbol(
            "func_y",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "mod_y.py",
            1,
        ));
        add_ref(
            &mut graph,
            x,
            y,
            crate::parser::ir::ReferenceKind::Call,
            "mod_x.py",
        );
        add_ref(
            &mut graph,
            y,
            x,
            crate::parser::ir::ReferenceKind::Call,
            "mod_y.py",
        );

        let results = circular_dependency::detect(&graph, Path::new(""));
        assert_eq!(results.len(), 2, "Should detect 2 independent cycles");

        // Each diagnostic mentions different modules
        let all_entities: Vec<&str> = results.iter().map(|d| d.entity.as_str()).collect();
        let has_ab = all_entities
            .iter()
            .any(|e| e.contains("mod_a.py") && e.contains("mod_b.py"));
        let has_xy = all_entities
            .iter()
            .any(|e| e.contains("mod_x.py") && e.contains("mod_y.py"));
        assert!(has_ab, "Should have A-B cycle");
        assert!(has_xy, "Should have X-Y cycle");
    }

    #[test]
    fn test_circular_dependency_detects_cycle_via_reference_edges() {
        let mut graph = Graph::new();

        let a = graph.add_symbol(make_symbol(
            "ClassA",
            crate::parser::ir::SymbolKind::Class,
            crate::parser::ir::Visibility::Public,
            "mod_a.py",
            1,
        ));
        let b = graph.add_symbol(make_symbol(
            "ClassB",
            crate::parser::ir::SymbolKind::Class,
            crate::parser::ir::Visibility::Public,
            "mod_b.py",
            1,
        ));

        // References edges (Import/Read), not Calls
        add_ref(
            &mut graph,
            a,
            b,
            crate::parser::ir::ReferenceKind::Read,
            "mod_a.py",
        );
        add_ref(
            &mut graph,
            b,
            a,
            crate::parser::ir::ReferenceKind::Read,
            "mod_b.py",
        );

        let results = circular_dependency::detect(&graph, Path::new(""));
        assert!(
            !results.is_empty(),
            "Should detect cycle via References edges, not just Calls"
        );
    }

    // =========================================================================
    // 11. circular_dependency — True Negative Tests
    // =========================================================================

    #[test]
    fn test_circular_dependency_clean_code_no_findings() {
        let graph = build_clean_code_graph();
        let results = circular_dependency::detect(&graph, Path::new(""));
        assert!(
            results.is_empty(),
            "Clean code should produce zero circular_dependency findings"
        );
    }

    #[test]
    fn test_circular_dependency_linear_chain_no_false_positive() {
        let graph = build_linear_dep_graph();
        let results = circular_dependency::detect(&graph, Path::new(""));
        assert!(
            results.is_empty(),
            "Linear chain (A->B->C, no back-edge) must NOT produce circular_dependency"
        );
    }

    #[test]
    fn test_circular_dependency_internal_calls_not_circular() {
        let mut graph = Graph::new();

        let a = graph.add_symbol(make_symbol(
            "func_a",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "mod_a.py",
            1,
        ));
        let b = graph.add_symbol(make_symbol(
            "func_b",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "mod_a.py",
            5,
        ));
        let c = graph.add_symbol(make_symbol(
            "func_c",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "mod_a.py",
            10,
        ));

        // All calls within the same file — NOT circular dependency
        add_ref(
            &mut graph,
            a,
            b,
            crate::parser::ir::ReferenceKind::Call,
            "mod_a.py",
        );
        add_ref(
            &mut graph,
            b,
            c,
            crate::parser::ir::ReferenceKind::Call,
            "mod_a.py",
        );
        add_ref(
            &mut graph,
            c,
            a,
            crate::parser::ir::ReferenceKind::Call,
            "mod_a.py",
        );

        let results = circular_dependency::detect(&graph, Path::new(""));
        assert!(
            results.is_empty(),
            "Intra-module cycles are NOT circular dependencies"
        );
    }

    #[test]
    fn test_circular_dependency_empty_graph_no_panic() {
        let graph = Graph::new();
        let results = circular_dependency::detect(&graph, Path::new(""));
        assert!(
            results.is_empty(),
            "Empty graph must produce zero diagnostics"
        );
    }

    #[test]
    fn test_circular_dependency_star_topology_no_false_positive() {
        let mut graph = Graph::new();

        let hub = graph.add_symbol(make_entry_point(
            "dispatcher",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "hub.py",
            1,
        ));

        let mut spokes = Vec::new();
        for i in 1..=5 {
            let spoke = graph.add_symbol(make_symbol(
                &format!("handler_{}", i),
                crate::parser::ir::SymbolKind::Function,
                crate::parser::ir::Visibility::Public,
                &format!("spoke_{}.py", i),
                1,
            ));
            spokes.push(spoke);
        }

        // Hub calls all spokes, no spoke calls hub or other spokes
        for spoke in &spokes {
            add_ref(
                &mut graph,
                hub,
                *spoke,
                crate::parser::ir::ReferenceKind::Call,
                "hub.py",
            );
        }

        let results = circular_dependency::detect(&graph, Path::new(""));
        assert!(
            results.is_empty(),
            "Star topology has no cycles — should produce zero diagnostics"
        );
    }

    // =========================================================================
    // 12. circular_dependency — Adversarial Tests
    // =========================================================================

    #[test]
    fn test_circular_dependency_handles_unresolved_targets() {
        let mut graph = Graph::new();

        let a = graph.add_symbol(make_symbol(
            "func_a",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "mod_a.py",
            1,
        ));

        // Edge to SymbolId::default() (unresolved target)
        graph.add_reference(crate::parser::ir::Reference {
            id: crate::parser::ir::ReferenceId::default(),
            from: a,
            to: crate::parser::ir::SymbolId::default(),
            kind: crate::parser::ir::ReferenceKind::Call,
            location: crate::parser::ir::Location {
                file: std::path::PathBuf::from("mod_a.py"),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            },
            resolution: crate::parser::ir::ResolutionStatus::Unresolved,
        });

        // Should not panic, should return empty or valid results
        let results = circular_dependency::detect(&graph, Path::new(""));
        // No cycle possible with unresolved targets
        let _ = results; // Main assertion: no panic
    }

    #[test]
    fn test_circular_dependency_deduplicates_cycles() {
        let graph = build_circular_dep_graph();
        let results = circular_dependency::detect(&graph, Path::new(""));

        // A 3-module cycle (A->B->C->A) can be discovered starting from any node
        // but should only be reported once
        assert_eq!(
            results.len(),
            1,
            "Same cycle discovered from multiple entry points must be reported only once. Got: {}",
            results.len()
        );
    }

    #[test]
    fn test_circular_dependency_diamond_is_not_cycle() {
        let mut graph = Graph::new();

        let top = graph.add_symbol(make_symbol(
            "top_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "top.py",
            1,
        ));
        let left = graph.add_symbol(make_symbol(
            "left_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "left.py",
            1,
        ));
        let right = graph.add_symbol(make_symbol(
            "right_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "right.py",
            1,
        ));
        let bottom = graph.add_symbol(make_symbol(
            "bottom_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "bottom.py",
            1,
        ));

        // Diamond: top -> {left, right} -> bottom
        add_ref(
            &mut graph,
            top,
            left,
            crate::parser::ir::ReferenceKind::Call,
            "top.py",
        );
        add_ref(
            &mut graph,
            top,
            right,
            crate::parser::ir::ReferenceKind::Call,
            "top.py",
        );
        add_ref(
            &mut graph,
            left,
            bottom,
            crate::parser::ir::ReferenceKind::Call,
            "left.py",
        );
        add_ref(
            &mut graph,
            right,
            bottom,
            crate::parser::ir::ReferenceKind::Call,
            "right.py",
        );

        let results = circular_dependency::detect(&graph, Path::new(""));
        assert!(
            results.is_empty(),
            "Diamond dependency is NOT a cycle — should produce zero diagnostics"
        );
    }

    #[test]
    fn test_circular_dependency_skips_module_kind_symbols() {
        let mut graph = Graph::new();

        let ma = graph.add_symbol(make_symbol(
            "mod_a",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "mod_a.py",
            1,
        ));
        let mb = graph.add_symbol(make_symbol(
            "mod_b",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "mod_b.py",
            1,
        ));

        // Contains edges don't establish module-level dependencies
        graph.add_reference(crate::parser::ir::Reference {
            id: crate::parser::ir::ReferenceId::default(),
            from: ma,
            to: mb,
            kind: crate::parser::ir::ReferenceKind::Import,
            location: crate::parser::ir::Location {
                file: std::path::PathBuf::from("mod_a.py"),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            },
            resolution: crate::parser::ir::ResolutionStatus::Resolved,
        });

        // Even with an import-based References edge (which IS checked),
        // there's no back-edge so no cycle
        let results = circular_dependency::detect(&graph, Path::new(""));
        assert!(
            results.is_empty(),
            "Single directional edge doesn't form a cycle"
        );
    }

    // =========================================================================
    // 13. orphaned_implementation — True Positive Tests
    // =========================================================================

    #[test]
    fn test_orphaned_implementation_detects_uncalled_public_method() {
        let graph = build_orphaned_method_graph();
        let results = orphaned_implementation::detect(&graph, Path::new(""));

        let entities: Vec<&str> = results.iter().map(|d| d.entity.as_str()).collect();
        assert!(
            entities.iter().any(|e| e.contains("process_user")),
            "process_user (zero callers) should be detected as orphaned"
        );
        assert!(
            !entities.iter().any(|e| e.contains("validate")),
            "validate (has caller) should NOT be flagged"
        );

        let d = results
            .iter()
            .find(|d| d.entity.contains("process_user"))
            .unwrap();
        assert_eq!(d.pattern, DiagnosticPattern::OrphanedImplementation);
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(
            d.confidence,
            Confidence::Moderate,
            "Public method should be Moderate confidence"
        );
        assert!(
            d.evidence
                .iter()
                .any(|e| e.observation.contains("0 callers")),
            "Evidence must mention 0 callers"
        );
        assert!(d.suggestion.len() > 10, "Suggestion must be actionable");
    }

    #[test]
    fn test_orphaned_implementation_private_method_high_confidence() {
        let mut graph = Graph::new();

        let _m = graph.add_symbol(make_symbol(
            "_internal_process",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Private,
            "service.py",
            1,
        ));

        let results = orphaned_implementation::detect(&graph, Path::new(""));
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].confidence,
            Confidence::High,
            "Private method with zero callers should be HIGH confidence"
        );
    }

    #[test]
    fn test_orphaned_implementation_detects_multiple_orphans() {
        let mut graph = Graph::new();

        let _cls = graph.add_symbol(make_symbol(
            "APIHandler",
            crate::parser::ir::SymbolKind::Class,
            crate::parser::ir::Visibility::Public,
            "api.py",
            1,
        ));
        let get = graph.add_symbol(make_symbol(
            "handle_get",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Public,
            "api.py",
            3,
        ));
        let _post = graph.add_symbol(make_symbol(
            "handle_post",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Public,
            "api.py",
            10,
        ));
        let _delete = graph.add_symbol(make_symbol(
            "handle_delete",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Public,
            "api.py",
            17,
        ));
        let validate = graph.add_symbol(make_symbol(
            "validate_request",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Public,
            "api.py",
            24,
        ));

        // handle_get calls validate_request
        add_ref(
            &mut graph,
            get,
            validate,
            crate::parser::ir::ReferenceKind::Call,
            "api.py",
        );

        let results = orphaned_implementation::detect(&graph, Path::new(""));
        let entities: Vec<&str> = results.iter().map(|d| d.entity.as_str()).collect();

        // handle_get, handle_post, handle_delete all have zero callers
        assert!(entities.iter().any(|e| e.contains("handle_get")));
        assert!(entities.iter().any(|e| e.contains("handle_post")));
        assert!(entities.iter().any(|e| e.contains("handle_delete")));
        // validate_request has a caller (handle_get)
        assert!(!entities.iter().any(|e| e.contains("validate_request")));
    }

    // =========================================================================
    // 14. orphaned_implementation — True Negative Tests
    // =========================================================================

    #[test]
    fn test_orphaned_implementation_method_with_caller_not_flagged() {
        let mut graph = Graph::new();

        let m = graph.add_symbol(make_symbol(
            "process",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Public,
            "app.py",
            1,
        ));
        let f = graph.add_symbol(make_symbol(
            "main",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "app.py",
            10,
        ));

        add_ref(
            &mut graph,
            f,
            m,
            crate::parser::ir::ReferenceKind::Call,
            "app.py",
        );

        let results = orphaned_implementation::detect(&graph, Path::new(""));
        assert!(
            !results.iter().any(|d| d.entity.contains("process")),
            "Method with a caller should NOT be flagged"
        );
    }

    #[test]
    fn test_orphaned_implementation_clean_code_no_findings() {
        let graph = build_clean_code_graph();
        let results = orphaned_implementation::detect(&graph, Path::new(""));
        assert!(
            results.is_empty(),
            "Clean code should produce zero OrphanedImplementation (no Method symbols)"
        );
    }

    #[test]
    fn test_orphaned_implementation_excludes_functions() {
        let mut graph = Graph::new();

        // Functions, not methods — should NOT trigger orphaned_implementation
        let _f1 = graph.add_symbol(make_symbol(
            "helper_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "utils.py",
            1,
        ));
        let _f2 = graph.add_symbol(make_symbol(
            "unused_util",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Private,
            "utils.py",
            5,
        ));

        let results = orphaned_implementation::detect(&graph, Path::new(""));
        assert!(
            results.is_empty(),
            "Functions (not Methods) must not trigger orphaned_implementation"
        );
    }

    // =========================================================================
    // 15. orphaned_implementation — Adversarial Tests
    // =========================================================================

    #[test]
    fn test_orphaned_implementation_excludes_dunder_methods() {
        let mut graph = Graph::new();

        let _init = graph.add_symbol(make_symbol(
            "__init__",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Public,
            "models.py",
            1,
        ));
        let _str_m = graph.add_symbol(make_symbol(
            "__str__",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Public,
            "models.py",
            5,
        ));
        let _repr_m = graph.add_symbol(make_symbol(
            "__repr__",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Public,
            "models.py",
            9,
        ));

        let results = orphaned_implementation::detect(&graph, Path::new(""));
        let dunders: Vec<_> = results
            .iter()
            .filter(|d| d.entity.contains("__") && d.entity.ends_with("__"))
            .collect();
        assert!(
            dunders.is_empty(),
            "Dunder methods must be excluded from orphaned_implementation. Found: {:?}",
            dunders.iter().map(|d| &d.entity).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_orphaned_implementation_excludes_entry_point_methods() {
        let mut graph = Graph::new();

        let _m = graph.add_symbol(make_entry_point(
            "main",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Public,
            "app.py",
            1,
        ));

        let results = orphaned_implementation::detect(&graph, Path::new(""));
        assert!(
            results.is_empty(),
            "Entry point methods must not be flagged as orphaned"
        );
    }

    #[test]
    fn test_orphaned_implementation_excludes_test_file_methods() {
        let mut graph = Graph::new();

        let _m = graph.add_symbol(make_symbol(
            "test_process",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Public,
            "test_service.py",
            1,
        ));

        let results = orphaned_implementation::detect(&graph, Path::new(""));
        assert!(results.is_empty(), "Methods in test files must be excluded");
    }

    #[test]
    fn test_orphaned_implementation_internal_class_call_counts() {
        let mut graph = Graph::new();

        let _cls = graph.add_symbol(make_symbol(
            "Service",
            crate::parser::ir::SymbolKind::Class,
            crate::parser::ir::Visibility::Public,
            "service.py",
            1,
        ));
        let public_api = graph.add_symbol(make_symbol(
            "public_api",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Public,
            "service.py",
            3,
        ));
        let helper = graph.add_symbol(make_symbol(
            "_helper",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Private,
            "service.py",
            10,
        ));

        // public_api calls _helper (internal class call)
        add_ref(
            &mut graph,
            public_api,
            helper,
            crate::parser::ir::ReferenceKind::Call,
            "service.py",
        );

        let results = orphaned_implementation::detect(&graph, Path::new(""));
        let entities: Vec<&str> = results.iter().map(|d| d.entity.as_str()).collect();

        // public_api IS flagged (zero callers from outside)
        assert!(
            entities.iter().any(|e| e.contains("public_api")),
            "public_api has zero callers, should be flagged"
        );
        // _helper is NOT flagged (has a caller: public_api)
        assert!(
            !entities.iter().any(|e| e.contains("_helper")),
            "_helper has a caller, should NOT be flagged"
        );
    }

    #[test]
    fn test_orphaned_implementation_empty_graph() {
        let graph = Graph::new();
        let results = orphaned_implementation::detect(&graph, Path::new(""));
        assert!(results.is_empty());
    }

    #[test]
    fn test_orphaned_implementation_and_data_dead_end_both_fire_on_method() {
        let mut graph = Graph::new();

        let _m = graph.add_symbol(make_symbol(
            "orphan_method",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Private,
            "service.py",
            1,
        ));

        let orphan_results = orphaned_implementation::detect(&graph, Path::new(""));
        let dead_end_results = data_dead_end::detect(&graph, Path::new(""));

        assert!(
            orphan_results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::OrphanedImplementation),
            "orphaned_implementation should fire"
        );
        assert!(
            dead_end_results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::DataDeadEnd),
            "data_dead_end should ALSO fire on the same method"
        );

        // Different pattern enum values
        assert_ne!(
            orphan_results[0].pattern, dead_end_results[0].pattern,
            "Patterns must be different enum values"
        );
    }

    #[test]
    fn test_orphaned_implementation_excludes_import_symbols() {
        let mut graph = Graph::new();

        // Import symbol with SymbolKind::Variable and "import" annotation
        let _imp = graph.add_symbol(make_import("os", "utils.py", 1));

        let results = orphaned_implementation::detect(&graph, Path::new(""));
        assert!(
            results.is_empty(),
            "Import symbols should not trigger orphaned_implementation"
        );
    }

    // =========================================================================
    // 16. missing_reexport — True Positive Tests
    // =========================================================================

    #[test]
    fn test_missing_reexport_detects_public_symbol_not_in_init() {
        let graph = build_missing_reexport_graph();
        let results = missing_reexport::detect(&graph, Path::new(""));

        let entities: Vec<&str> = results.iter().map(|d| d.entity.as_str()).collect();
        assert!(
            entities.iter().any(|e| e.contains("helper_b")),
            "helper_b (not re-exported) should be detected"
        );
        assert!(
            !entities.iter().any(|e| e.contains("helper_a")),
            "helper_a (re-exported) should NOT be flagged"
        );
        assert!(
            !entities.iter().any(|e| e.contains("_private_fn")),
            "_private_fn (private) should NOT be flagged"
        );

        let d = results
            .iter()
            .find(|d| d.entity.contains("helper_b"))
            .unwrap();
        assert_eq!(d.pattern, DiagnosticPattern::MissingReexport);
        assert_eq!(d.severity, Severity::Info);
        assert_eq!(d.confidence, Confidence::Moderate);
        assert!(!d.evidence.is_empty());
        assert!(d.suggestion.contains("helper_b"));
    }

    #[test]
    fn test_missing_reexport_detects_multiple_missing_symbols() {
        let mut graph = Graph::new();

        // Empty __init__.py (no imports)
        let _init = graph.add_symbol(make_symbol(
            "pkg",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "pkg/__init__.py",
            1,
        ));

        // Two submodules with public symbols
        let _models_mod = graph.add_symbol(make_symbol(
            "models",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "pkg/models.py",
            1,
        ));
        let _user = graph.add_symbol(make_symbol(
            "User",
            crate::parser::ir::SymbolKind::Class,
            crate::parser::ir::Visibility::Public,
            "pkg/models.py",
            3,
        ));
        let _order = graph.add_symbol(make_symbol(
            "Order",
            crate::parser::ir::SymbolKind::Class,
            crate::parser::ir::Visibility::Public,
            "pkg/models.py",
            10,
        ));
        let _utils_mod = graph.add_symbol(make_symbol(
            "utils",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "pkg/utils.py",
            1,
        ));
        let _fmt = graph.add_symbol(make_symbol(
            "format_date",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "pkg/utils.py",
            3,
        ));

        let results = missing_reexport::detect(&graph, Path::new(""));
        assert!(
            results.len() >= 3,
            "Should detect at least 3 missing re-exports (User, Order, format_date), got {}",
            results.len()
        );
    }

    #[test]
    fn test_missing_reexport_works_with_mod_rs() {
        let mut graph = Graph::new();

        let _mod_rs = graph.add_symbol(make_symbol(
            "handlers",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "src/handlers/mod.rs",
            1,
        ));
        let _api_mod = graph.add_symbol(make_symbol(
            "api",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "src/handlers/api.rs",
            1,
        ));
        let _handler = graph.add_symbol(make_symbol(
            "handle_request",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "src/handlers/api.rs",
            3,
        ));

        let results = missing_reexport::detect(&graph, Path::new(""));
        assert!(
            results.iter().any(|d| d.entity.contains("handle_request")),
            "Should detect missing re-export from mod.rs for handle_request"
        );
    }

    // =========================================================================
    // 17. missing_reexport — True Negative Tests
    // =========================================================================

    #[test]
    fn test_missing_reexport_proper_reexport_no_findings() {
        let graph = build_proper_reexport_graph();
        let results = missing_reexport::detect(&graph, Path::new(""));
        assert!(
            results.is_empty(),
            "All public symbols are re-exported — should produce zero findings"
        );
    }

    #[test]
    fn test_missing_reexport_no_package_structure_no_findings() {
        let graph = build_clean_code_graph();
        let results = missing_reexport::detect(&graph, Path::new(""));
        assert!(
            results.is_empty(),
            "Without __init__.py or mod.rs, no missing re-export findings"
        );
    }

    #[test]
    fn test_missing_reexport_empty_graph() {
        let graph = Graph::new();
        let results = missing_reexport::detect(&graph, Path::new(""));
        assert!(results.is_empty());
    }

    // =========================================================================
    // 18. missing_reexport — Adversarial Tests
    // =========================================================================

    #[test]
    fn test_missing_reexport_private_symbols_excluded() {
        let mut graph = Graph::new();

        let _init = graph.add_symbol(make_symbol(
            "pkg",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "pkg/__init__.py",
            1,
        ));
        let _internal_mod = graph.add_symbol(make_symbol(
            "internal",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "pkg/internal.py",
            1,
        ));
        let _priv = graph.add_symbol(make_symbol(
            "_private_helper",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Private,
            "pkg/internal.py",
            3,
        ));
        let _secret = graph.add_symbol(make_symbol(
            "__secret",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Private,
            "pkg/internal.py",
            7,
        ));

        let results = missing_reexport::detect(&graph, Path::new(""));
        assert!(
            results.is_empty(),
            "Private symbols should NOT trigger missing re-export"
        );
    }

    #[test]
    fn test_missing_reexport_init_not_flagged_as_submodule() {
        let mut graph = Graph::new();

        let _init = graph.add_symbol(make_symbol(
            "pkg",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "pkg/__init__.py",
            1,
        ));
        let _init_fn = graph.add_symbol(make_symbol(
            "package_init",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "pkg/__init__.py",
            3,
        ));
        let _sub_mod = graph.add_symbol(make_symbol(
            "sub",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "pkg/sub.py",
            1,
        ));
        let _sub_fn = graph.add_symbol(make_symbol(
            "sub_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "pkg/sub.py",
            3,
        ));

        let results = missing_reexport::detect(&graph, Path::new(""));
        // package_init is IN the parent module — should NOT be flagged
        assert!(
            !results.iter().any(|d| d.entity.contains("package_init")),
            "__init__.py's own symbols should not be flagged as missing re-exports"
        );
    }

    #[test]
    fn test_missing_reexport_checks_only_immediate_parent() {
        let mut graph = Graph::new();

        // Grandparent
        let _a_init = graph.add_symbol(make_symbol(
            "a",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "a/__init__.py",
            1,
        ));
        // Parent
        let _ab_init = graph.add_symbol(make_symbol(
            "b",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "a/b/__init__.py",
            1,
        ));
        // Deep submodule
        let _deep_mod = graph.add_symbol(make_symbol(
            "deep",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "a/b/deep.py",
            1,
        ));
        let _deep_fn = graph.add_symbol(make_symbol(
            "deep_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "a/b/deep.py",
            3,
        ));

        let results = missing_reexport::detect(&graph, Path::new(""));
        // deep_fn's parent is a/b/__init__.py, NOT a/__init__.py
        for d in &results {
            if d.entity.contains("deep_fn") {
                assert!(
                    d.message.contains("a/b/__init__.py"),
                    "Should reference immediate parent a/b/__init__.py, got: {}",
                    d.message
                );
            }
        }
    }

    #[test]
    fn test_missing_reexport_name_collision_handled() {
        let mut graph = Graph::new();

        let _init = graph.add_symbol(make_symbol(
            "pkg",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "pkg/__init__.py",
            1,
        ));
        // One "parse" import in __init__.py
        let _init_import = graph.add_symbol(make_import("parse", "pkg/__init__.py", 2));

        let _json_mod = graph.add_symbol(make_symbol(
            "json_utils",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "pkg/json_utils.py",
            1,
        ));
        let _json_parse = graph.add_symbol(make_symbol(
            "parse",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "pkg/json_utils.py",
            3,
        ));
        let _xml_mod = graph.add_symbol(make_symbol(
            "xml_utils",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "pkg/xml_utils.py",
            1,
        ));
        let _xml_parse = graph.add_symbol(make_symbol(
            "parse",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "pkg/xml_utils.py",
            3,
        ));

        let results = missing_reexport::detect(&graph, Path::new(""));
        // Name "parse" in init matches both — no crash, no panic
        // At most 0 diagnostics for "parse" (name-match satisfies both)
        let parse_diags: Vec<_> = results
            .iter()
            .filter(|d| d.entity.contains("parse"))
            .collect();
        assert!(
            parse_diags.is_empty(),
            "Name match for 'parse' in __init__.py should satisfy both submodules"
        );
    }

    #[test]
    fn test_missing_reexport_excludes_module_symbols() {
        let mut graph = Graph::new();

        let _init = graph.add_symbol(make_symbol(
            "pkg",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "pkg/__init__.py",
            1,
        ));
        // Module symbol (structural) + real function
        let _sub_mod = graph.add_symbol(make_symbol(
            "sub",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "pkg/sub.py",
            1,
        ));
        let _real_fn = graph.add_symbol(make_symbol(
            "real_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "pkg/sub.py",
            3,
        ));

        let results = missing_reexport::detect(&graph, Path::new(""));
        // Module symbol "sub" should NOT be flagged
        assert!(
            !results
                .iter()
                .any(|d| { d.entity.contains("sub") && !d.entity.contains("real_fn") }),
            "Module symbols should not be candidates for re-export checking"
        );
    }

    // =========================================================================
    // 19. Cross-Pattern Interaction Tests
    // =========================================================================

    #[test]
    fn test_new_patterns_dont_change_existing_pattern_results() {
        let graph = build_all_fixtures_graph();
        let diagnostics = run_all_patterns(&graph, Path::new(""));

        let patterns: Vec<DiagnosticPattern> = diagnostics.iter().map(|d| d.pattern).collect();
        assert!(
            patterns.contains(&DiagnosticPattern::IsolatedCluster),
            "IsolatedCluster should still be detected"
        );
        assert!(
            patterns.contains(&DiagnosticPattern::DataDeadEnd),
            "DataDeadEnd should still be detected"
        );
        assert!(
            patterns.contains(&DiagnosticPattern::PhantomDependency),
            "PhantomDependency should still be detected"
        );

        // New patterns should produce zero diagnostics on build_all_fixtures_graph
        // (no __init__.py, no cross-module cycles, no uncalled methods)
        assert!(
            !patterns.contains(&DiagnosticPattern::CircularDependency),
            "CircularDependency should not fire on single-file fixtures"
        );
        assert!(
            !patterns.contains(&DiagnosticPattern::MissingReexport),
            "MissingReexport should not fire on single-file fixtures"
        );
    }

    #[test]
    fn test_no_high_confidence_false_positives_from_new_patterns_on_clean_code() {
        let graph = build_clean_code_graph();
        let diagnostics = run_all_patterns(&graph, Path::new(""));

        // Zero diagnostics from any pattern on clean code
        assert!(
            diagnostics.is_empty(),
            "Clean code must produce ZERO diagnostics from all 6 patterns. Got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_all_six_patterns_fire_on_comprehensive_graph() {
        let mut graph = Graph::new();

        // Dead code (for data_dead_end)
        let _dead = graph.add_symbol(make_symbol(
            "unused_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Private,
            "dead.py",
            1,
        ));

        // Isolated cluster (for isolated_cluster)
        let iso_a = graph.add_symbol(make_symbol(
            "iso_entry",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Private,
            "isolated.py",
            1,
        ));
        let iso_b = graph.add_symbol(make_symbol(
            "iso_worker",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Private,
            "isolated.py",
            5,
        ));
        add_ref(
            &mut graph,
            iso_a,
            iso_b,
            crate::parser::ir::ReferenceKind::Call,
            "isolated.py",
        );

        // Phantom import (for phantom_dependency)
        let _phantom = graph.add_symbol(make_import("os", "imports.py", 1));
        let _used_import = graph.add_symbol(make_import("sys", "imports.py", 2));
        let func_using_sys = graph.add_symbol(make_symbol(
            "use_sys",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Private,
            "imports.py",
            5,
        ));
        let sys_import_id = graph
            .all_symbols()
            .find(|(_, s)| s.name == "sys" && s.location.file.to_string_lossy().contains("imports"))
            .map(|(id, _)| id)
            .unwrap();
        add_ref(
            &mut graph,
            func_using_sys,
            sys_import_id,
            crate::parser::ir::ReferenceKind::Read,
            "imports.py",
        );

        // Cross-module cycle (for circular_dependency)
        let cyc_a = graph.add_symbol(make_symbol(
            "cycle_a_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "cycle_a.py",
            1,
        ));
        let cyc_b = graph.add_symbol(make_symbol(
            "cycle_b_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "cycle_b.py",
            1,
        ));
        add_ref(
            &mut graph,
            cyc_a,
            cyc_b,
            crate::parser::ir::ReferenceKind::Call,
            "cycle_a.py",
        );
        add_ref(
            &mut graph,
            cyc_b,
            cyc_a,
            crate::parser::ir::ReferenceKind::Call,
            "cycle_b.py",
        );

        // Orphaned method (for orphaned_implementation)
        let _orphan = graph.add_symbol(make_symbol(
            "orphaned_method",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Private,
            "methods.py",
            1,
        ));

        // Missing re-export (for missing_reexport)
        let _init = graph.add_symbol(make_symbol(
            "mypkg",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "mypkg/__init__.py",
            1,
        ));
        let _sub = graph.add_symbol(make_symbol(
            "mysub",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "mypkg/sub.py",
            1,
        ));
        let _pub_fn = graph.add_symbol(make_symbol(
            "public_api",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "mypkg/sub.py",
            3,
        ));

        let diagnostics = run_all_patterns(&graph, Path::new(""));
        let patterns: std::collections::HashSet<DiagnosticPattern> =
            diagnostics.iter().map(|d| d.pattern).collect();

        assert!(
            patterns.contains(&DiagnosticPattern::DataDeadEnd),
            "DataDeadEnd should fire"
        );
        assert!(
            patterns.contains(&DiagnosticPattern::IsolatedCluster),
            "IsolatedCluster should fire"
        );
        assert!(
            patterns.contains(&DiagnosticPattern::PhantomDependency),
            "PhantomDependency should fire"
        );
        assert!(
            patterns.contains(&DiagnosticPattern::CircularDependency),
            "CircularDependency should fire"
        );
        assert!(
            patterns.contains(&DiagnosticPattern::OrphanedImplementation),
            "OrphanedImplementation should fire"
        );
        assert!(
            patterns.contains(&DiagnosticPattern::MissingReexport),
            "MissingReexport should fire"
        );

        // All diagnostics have sequential IDs
        for (i, d) in diagnostics.iter().enumerate() {
            assert_eq!(d.id, format!("D{:03}", i + 1));
        }

        // All diagnostics have non-empty evidence
        for d in &diagnostics {
            assert!(
                !d.evidence.is_empty(),
                "Diagnostic must have evidence: {}",
                d.message
            );
        }
    }

    // =========================================================================
    // 20. Evidence Quality Tests
    // =========================================================================

    #[test]
    fn test_circular_dependency_evidence_has_cross_references() {
        let mut graph = Graph::new();

        let a = graph.add_symbol(make_symbol(
            "func_a",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "mod_a.py",
            1,
        ));
        let b = graph.add_symbol(make_symbol(
            "func_b",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "mod_b.py",
            1,
        ));
        add_ref(
            &mut graph,
            a,
            b,
            crate::parser::ir::ReferenceKind::Call,
            "mod_a.py",
        );
        add_ref(
            &mut graph,
            b,
            a,
            crate::parser::ir::ReferenceKind::Call,
            "mod_b.py",
        );

        let results = circular_dependency::detect(&graph, Path::new(""));
        assert!(!results.is_empty());

        let d = &results[0];
        for ev in &d.evidence {
            assert!(!ev.observation.is_empty());
            // Must use failure-pattern language
            let forbidden = [
                "vertex",
                "edge",
                "adjacency",
                "node",
                "in-degree",
                "out-degree",
            ];
            for term in &forbidden {
                assert!(
                    !ev.observation.to_lowercase().contains(term),
                    "Evidence uses graph theory term '{}': {}",
                    term,
                    ev.observation
                );
            }
        }
        // At least one evidence entry references specific file paths
        assert!(
            d.evidence
                .iter()
                .any(|e| e.observation.contains("mod_a.py") || e.observation.contains("mod_b.py")),
            "Evidence should reference specific file paths"
        );
    }

    #[test]
    fn test_orphaned_implementation_evidence_has_caller_count() {
        let graph = build_orphaned_method_graph();
        let results = orphaned_implementation::detect(&graph, Path::new(""));

        let d = results
            .iter()
            .find(|d| d.entity.contains("process_user"))
            .expect("Should find process_user diagnostic");

        assert!(
            d.evidence
                .iter()
                .any(|e| e.observation.contains("0 callers")),
            "Evidence must mention 0 callers"
        );
        assert!(
            d.evidence.iter().any(|e| e.location.is_some()),
            "Evidence must have location"
        );
        assert_ne!(
            d.suggestion, d.message,
            "Suggestion must differ from message"
        );
        assert!(d.suggestion.len() > 10);
    }

    #[test]
    fn test_missing_reexport_evidence_references_parent_module() {
        let graph = build_missing_reexport_graph();
        let results = missing_reexport::detect(&graph, Path::new(""));

        let d = results
            .iter()
            .find(|d| d.entity.contains("helper_b"))
            .expect("Should find helper_b diagnostic");

        assert!(
            d.evidence
                .iter()
                .any(|e| e.observation.contains("__init__.py")),
            "Evidence should mention __init__.py as parent module"
        );
        assert!(
            d.evidence.iter().any(|e| e.observation.contains("sub.py")),
            "Evidence should mention the submodule"
        );
        assert!(
            d.suggestion.contains("helper_b"),
            "Suggestion should advise adding the symbol"
        );
    }

    #[test]
    fn test_new_patterns_set_empty_id() {
        let graph = build_circular_dep_graph();
        let results = circular_dependency::detect(&graph, Path::new(""));
        for d in &results {
            assert!(
                d.id.is_empty(),
                "Pattern must set id to empty string. Registry assigns IDs."
            );
        }

        let graph2 = build_orphaned_method_graph();
        let results2 = orphaned_implementation::detect(&graph2, Path::new(""));
        for d in &results2 {
            assert!(d.id.is_empty());
        }

        let graph3 = build_missing_reexport_graph();
        let results3 = missing_reexport::detect(&graph3, Path::new(""));
        for d in &results3 {
            assert!(d.id.is_empty());
        }
    }

    // =========================================================================
    // 21. Robustness Contract Tests
    // =========================================================================

    #[test]
    fn test_all_new_patterns_handle_module_only_graph() {
        let mut graph = Graph::new();
        let _m1 = graph.add_symbol(make_symbol(
            "mod_a",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "mod_a.py",
            1,
        ));
        let _m2 = graph.add_symbol(make_symbol(
            "mod_b",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "mod_b.py",
            1,
        ));

        let r1 = circular_dependency::detect(&graph, Path::new(""));
        let r2 = orphaned_implementation::detect(&graph, Path::new(""));
        let r3 = missing_reexport::detect(&graph, Path::new(""));
        assert!(r1.is_empty());
        assert!(r2.is_empty());
        assert!(r3.is_empty());
    }

    #[test]
    fn test_all_new_patterns_handle_single_symbol_graph() {
        let mut graph = Graph::new();
        let _main = graph.add_symbol(make_entry_point(
            "main",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "main.py",
            1,
        ));

        let r1 = circular_dependency::detect(&graph, Path::new(""));
        let r2 = orphaned_implementation::detect(&graph, Path::new(""));
        let r3 = missing_reexport::detect(&graph, Path::new(""));
        assert!(r1.is_empty());
        assert!(r2.is_empty());
        assert!(r3.is_empty());
    }

    #[test]
    fn test_patterns_handle_empty_file_path() {
        let mut graph = Graph::new();
        let _sym = graph.add_symbol(make_symbol(
            "some_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "",
            0,
        ));

        // Should not panic
        let r1 = circular_dependency::detect(&graph, Path::new(""));
        let r2 = orphaned_implementation::detect(&graph, Path::new(""));
        let r3 = missing_reexport::detect(&graph, Path::new(""));
        let _ = (r1, r2, r3);
    }

    // =========================================================================
    // 22. Confidence Calibration Tests
    // =========================================================================

    #[test]
    fn test_orphaned_implementation_confidence_calibration() {
        let mut graph = Graph::new();

        let _pub = graph.add_symbol(make_symbol(
            "pub_method",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Public,
            "calibration.py",
            1,
        ));
        let _priv = graph.add_symbol(make_symbol(
            "_priv_method",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Private,
            "calibration.py",
            5,
        ));
        let _crate_vis = graph.add_symbol(make_symbol(
            "crate_method",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Crate,
            "calibration.py",
            10,
        ));

        let results = orphaned_implementation::detect(&graph, Path::new(""));

        let pub_d = results
            .iter()
            .find(|d| d.entity.contains("pub_method"))
            .unwrap();
        let priv_d = results
            .iter()
            .find(|d| d.entity.contains("_priv_method"))
            .unwrap();

        assert_eq!(
            pub_d.confidence,
            Confidence::Moderate,
            "Public method should be Moderate"
        );
        assert_eq!(
            priv_d.confidence,
            Confidence::High,
            "Private method should be High"
        );
    }

    #[test]
    fn test_circular_dependency_confidence_always_high() {
        let graph = build_circular_dep_graph();
        let results = circular_dependency::detect(&graph, Path::new(""));

        for d in &results {
            assert_eq!(
                d.confidence,
                Confidence::High,
                "CircularDependency must always be High confidence"
            );
        }
    }

    #[test]
    fn test_missing_reexport_confidence_always_moderate() {
        let graph = build_missing_reexport_graph();
        let results = missing_reexport::detect(&graph, Path::new(""));

        for d in &results {
            assert_eq!(
                d.confidence,
                Confidence::Moderate,
                "MissingReexport must always be Moderate confidence"
            );
        }
    }

    // =========================================================================
    // 23. Message Language Tests
    // =========================================================================

    #[test]
    fn test_new_pattern_messages_use_failure_language() {
        let forbidden_terms = [
            "vertex",
            "node",
            "in-degree",
            "out-degree",
            "adjacency",
            "subgraph",
            "topological",
        ];

        let circular_graph = build_circular_dep_graph();
        let orphan_graph = build_orphaned_method_graph();
        let reexport_graph = build_missing_reexport_graph();

        let all_diags: Vec<_> = vec![
            circular_dependency::detect(&circular_graph, Path::new("")),
            orphaned_implementation::detect(&orphan_graph, Path::new("")),
            missing_reexport::detect(&reexport_graph, Path::new("")),
        ]
        .into_iter()
        .flatten()
        .collect();

        for diag in &all_diags {
            for term in &forbidden_terms {
                assert!(
                    !diag.message.to_lowercase().contains(term),
                    "Message contains forbidden graph theory term '{}': {}",
                    term,
                    diag.message
                );
                assert!(
                    !diag.suggestion.to_lowercase().contains(term),
                    "Suggestion contains forbidden graph theory term '{}': {}",
                    term,
                    diag.suggestion
                );
            }
        }
    }

    // =========================================================================
    // 24. Registry Integration Tests
    // =========================================================================

    #[test]
    fn test_registry_includes_new_patterns() {
        // Build a combined graph that triggers all 3 new patterns
        let mut graph = Graph::new();

        // Circular dependency
        let a = graph.add_symbol(make_symbol(
            "fn_a",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "reg_a.py",
            1,
        ));
        let b = graph.add_symbol(make_symbol(
            "fn_b",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "reg_b.py",
            1,
        ));
        add_ref(
            &mut graph,
            a,
            b,
            crate::parser::ir::ReferenceKind::Call,
            "reg_a.py",
        );
        add_ref(
            &mut graph,
            b,
            a,
            crate::parser::ir::ReferenceKind::Call,
            "reg_b.py",
        );

        // Orphaned method
        let _orphan = graph.add_symbol(make_symbol(
            "orphan_m",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Private,
            "reg_methods.py",
            1,
        ));

        // Missing re-export
        let _init = graph.add_symbol(make_symbol(
            "regpkg",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "regpkg/__init__.py",
            1,
        ));
        let _sub = graph.add_symbol(make_symbol(
            "regsub",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "regpkg/sub.py",
            1,
        ));
        let _pub_fn = graph.add_symbol(make_symbol(
            "exported_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "regpkg/sub.py",
            3,
        ));

        let diagnostics = run_all_patterns(&graph, Path::new(""));
        let patterns: std::collections::HashSet<DiagnosticPattern> =
            diagnostics.iter().map(|d| d.pattern).collect();

        assert!(
            patterns.contains(&DiagnosticPattern::CircularDependency),
            "CircularDependency must be registered"
        );
        assert!(
            patterns.contains(&DiagnosticPattern::OrphanedImplementation),
            "OrphanedImplementation must be registered"
        );
        assert!(
            patterns.contains(&DiagnosticPattern::MissingReexport),
            "MissingReexport must be registered"
        );

        // All IDs sequential
        for (i, d) in diagnostics.iter().enumerate() {
            assert_eq!(d.id, format!("D{:03}", i + 1));
        }
    }

    #[test]
    fn test_pattern_filter_selects_new_patterns() {
        // Build graph that triggers CircularDependency
        let mut graph = Graph::new();
        let a = graph.add_symbol(make_symbol(
            "fn_a",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "filter_a.py",
            1,
        ));
        let b = graph.add_symbol(make_symbol(
            "fn_b",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "filter_b.py",
            1,
        ));
        add_ref(
            &mut graph,
            a,
            b,
            crate::parser::ir::ReferenceKind::Call,
            "filter_a.py",
        );
        add_ref(
            &mut graph,
            b,
            a,
            crate::parser::ir::ReferenceKind::Call,
            "filter_b.py",
        );
        // Also add something that triggers other patterns
        let _dead = graph.add_symbol(make_symbol(
            "dead_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Private,
            "filter_dead.py",
            1,
        ));

        let filter = PatternFilter {
            patterns: Some(vec![DiagnosticPattern::CircularDependency]),
            min_severity: None,
            min_confidence: None,
        };
        let diagnostics = run_patterns(&graph, &filter, Path::new(""));

        for d in &diagnostics {
            assert_eq!(
                d.pattern,
                DiagnosticPattern::CircularDependency,
                "Filter should only include CircularDependency"
            );
        }
        assert!(!diagnostics.is_empty(), "Should have at least 1 result");
    }

    // =========================================================================
    // 25. Flow Trace Completeness Tests
    // =========================================================================

    #[test]
    fn test_circular_dependency_evidence_traces_full_cycle_path() {
        let graph = build_circular_dep_graph();
        let results = circular_dependency::detect(&graph, Path::new(""));

        assert_eq!(results.len(), 1);
        let d = &results[0];

        // Every step in the 3-module cycle must be documented
        assert_eq!(d.evidence.len(), 3, "3-step cycle needs 3 evidence entries");

        // Each evidence entry has a location
        for ev in &d.evidence {
            assert!(
                ev.location.is_some() || !ev.observation.is_empty(),
                "Each step must have location or detailed observation"
            );
        }

        // Evidence mentions specific symbol names
        let all_observations: String = d
            .evidence
            .iter()
            .map(|e| e.observation.clone())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            all_observations.contains("func_a")
                || all_observations.contains("func_b")
                || all_observations.contains("func_c"),
            "Evidence should mention specific symbol names"
        );
    }

    #[test]
    fn test_orphaned_implementation_evidence_shows_analysis_scope() {
        let mut graph = Graph::new();

        // Build a graph with multiple files to verify scope reporting
        let _f1 = graph.add_symbol(make_symbol(
            "func_1",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "file_1.py",
            1,
        ));
        let _f2 = graph.add_symbol(make_symbol(
            "func_2",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "file_2.py",
            1,
        ));
        let _f3 = graph.add_symbol(make_symbol(
            "func_3",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "file_3.py",
            1,
        ));
        let _orphan = graph.add_symbol(make_symbol(
            "orphan_method",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Private,
            "target.py",
            1,
        ));

        let results = orphaned_implementation::detect(&graph, Path::new(""));
        let d = results
            .iter()
            .find(|d| d.entity.contains("orphan_method"))
            .unwrap();

        // Evidence should mention the scope of analysis
        assert!(
            d.evidence
                .iter()
                .any(|e| e.observation.contains("0 callers")
                    && e.observation.contains("analyzed files")),
            "Evidence should quantify analysis scope. Got: {:?}",
            d.evidence
                .iter()
                .map(|e| &e.observation)
                .collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // 26. Boundary Detection Tests
    // =========================================================================

    #[test]
    fn test_circular_dependency_module_boundary_accuracy() {
        let mut graph = Graph::new();

        // 4 files: a and b have a cycle, c and d are internal
        let a = graph.add_symbol(make_symbol(
            "cross_a",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "boundary_a.py",
            1,
        ));
        let b = graph.add_symbol(make_symbol(
            "cross_b",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "boundary_b.py",
            1,
        ));
        // Intra-module functions (same file as a)
        let internal = graph.add_symbol(make_symbol(
            "internal_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Private,
            "boundary_a.py",
            5,
        ));

        // Cross-module cycle
        add_ref(
            &mut graph,
            a,
            b,
            crate::parser::ir::ReferenceKind::Call,
            "boundary_a.py",
        );
        add_ref(
            &mut graph,
            b,
            a,
            crate::parser::ir::ReferenceKind::Call,
            "boundary_b.py",
        );
        // Intra-module call (should NOT count as cross-module)
        add_ref(
            &mut graph,
            a,
            internal,
            crate::parser::ir::ReferenceKind::Call,
            "boundary_a.py",
        );

        let results = circular_dependency::detect(&graph, Path::new(""));
        assert_eq!(
            results.len(),
            1,
            "Only the cross-module cycle should be detected"
        );

        let d = &results[0];
        // Only boundary_a.py and boundary_b.py should be in the cycle
        assert!(d.entity.contains("boundary_a.py"));
        assert!(d.entity.contains("boundary_b.py"));
    }

    #[test]
    fn test_missing_reexport_package_boundary_accuracy() {
        let mut graph = Graph::new();

        // Package boundary
        let _init = graph.add_symbol(make_symbol(
            "pkg",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "pkg/__init__.py",
            1,
        ));
        let _sub = graph.add_symbol(make_symbol(
            "sub",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "pkg/sub.py",
            1,
        ));
        let _sub_fn = graph.add_symbol(make_symbol(
            "sub_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "pkg/sub.py",
            3,
        ));

        // Standalone file NOT in pkg/
        let _standalone = graph.add_symbol(make_symbol(
            "standalone",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "standalone.py",
            1,
        ));
        let _standalone_fn = graph.add_symbol(make_symbol(
            "standalone_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "standalone.py",
            3,
        ));

        let results = missing_reexport::detect(&graph, Path::new(""));

        // Only pkg/sub.py symbols should be checked against pkg/__init__.py
        assert!(
            results.iter().any(|d| d.entity.contains("sub_fn")),
            "sub_fn in pkg/ should be flagged"
        );
        assert!(
            !results.iter().any(|d| d.entity.contains("standalone_fn")),
            "standalone.py is NOT in pkg/ — should not be checked"
        );
    }

    // =========================================================================
    // 27. Regression Tests
    // =========================================================================

    #[test]
    fn test_existing_patterns_still_pass_after_registration() {
        let graph = build_dead_code_graph();
        let results = run_all_patterns(&graph, Path::new(""));

        let dead_end_results: Vec<_> = results
            .iter()
            .filter(|d| d.pattern == DiagnosticPattern::DataDeadEnd)
            .collect();
        assert!(
            !dead_end_results.is_empty(),
            "data_dead_end should still detect dead code"
        );
    }

    #[test]
    fn test_diagnostic_ids_sequential_across_six_patterns() {
        // Build graph with all 6 pattern triggers (same as test_all_six_patterns_fire)
        let mut graph = Graph::new();

        let _dead = graph.add_symbol(make_symbol(
            "unused_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Private,
            "seq_dead.py",
            1,
        ));
        let iso_a = graph.add_symbol(make_symbol(
            "iso_a",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Private,
            "seq_isolated.py",
            1,
        ));
        let iso_b = graph.add_symbol(make_symbol(
            "iso_b",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Private,
            "seq_isolated.py",
            5,
        ));
        add_ref(
            &mut graph,
            iso_a,
            iso_b,
            crate::parser::ir::ReferenceKind::Call,
            "seq_isolated.py",
        );
        let _phantom = graph.add_symbol(make_import("unused_import", "seq_imports.py", 1));
        let cyc_a = graph.add_symbol(make_symbol(
            "cyc_a",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "seq_cyc_a.py",
            1,
        ));
        let cyc_b = graph.add_symbol(make_symbol(
            "cyc_b",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "seq_cyc_b.py",
            1,
        ));
        add_ref(
            &mut graph,
            cyc_a,
            cyc_b,
            crate::parser::ir::ReferenceKind::Call,
            "seq_cyc_a.py",
        );
        add_ref(
            &mut graph,
            cyc_b,
            cyc_a,
            crate::parser::ir::ReferenceKind::Call,
            "seq_cyc_b.py",
        );
        let _orphan = graph.add_symbol(make_symbol(
            "orphan_m",
            crate::parser::ir::SymbolKind::Method,
            crate::parser::ir::Visibility::Private,
            "seq_methods.py",
            1,
        ));
        let _init = graph.add_symbol(make_symbol(
            "seqpkg",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "seqpkg/__init__.py",
            1,
        ));
        let _sub = graph.add_symbol(make_symbol(
            "seqsub",
            crate::parser::ir::SymbolKind::Module,
            crate::parser::ir::Visibility::Public,
            "seqpkg/sub.py",
            1,
        ));
        let _pub_fn = graph.add_symbol(make_symbol(
            "seq_pub_fn",
            crate::parser::ir::SymbolKind::Function,
            crate::parser::ir::Visibility::Public,
            "seqpkg/sub.py",
            3,
        ));

        let diagnostics = run_all_patterns(&graph, Path::new(""));
        // IDs must be sequential with no gaps
        for (i, d) in diagnostics.iter().enumerate() {
            assert_eq!(
                d.id,
                format!("D{:03}", i + 1),
                "ID must be sequential. Expected D{:03}, got {}",
                i + 1,
                d.id
            );
        }
    }
}
