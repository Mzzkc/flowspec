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
