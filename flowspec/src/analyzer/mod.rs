// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Diagnostic detection, flow tracing, and boundary analysis.
//!
//! Each analyzer is a standalone function that queries the [`Graph`](crate::graph::Graph)
//! and produces diagnostics. No inheritance, no trait objects — just functions
//! that take a graph reference and return `Vec<Diagnostic>`.
//!
//! Flowspec ships eleven diagnostic patterns across five severity levels
//! (Critical, Warning, Info, Style, Note). Pattern implementations live in
//! [`patterns`], with one module per pattern plus a registry that collects
//! all detectors. The [`flow`] module provides the DFS-based flow tracing
//! engine used by patterns that need to follow data through the call graph.

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
