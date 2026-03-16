// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Diagnostic detection, flow tracing, and boundary analysis.
//!
//! Each analyzer is a standalone function that queries the graph and
//! produces diagnostics. No inheritance, no trait objects. The three
//! cycle-1 patterns are: `isolated_cluster`, `data_dead_end`, and
//! `phantom_dependency`.

/// Converts raw `Diagnostic` values into `DiagnosticEntry` manifest records.
pub mod conversion;
/// Diagnostic types — pattern, severity, confidence, evidence, suggestion.
pub mod diagnostic;
/// Graph-to-manifest field extraction — calls, called_by, visibility, module role, dependency graph.
pub mod extraction;
/// Data flow tracing engine — DFS over the call graph with depth limits and cycle detection.
pub mod flow;
/// Pattern detectors — one module per diagnostic pattern, plus the registry that collects them.
pub mod patterns;
