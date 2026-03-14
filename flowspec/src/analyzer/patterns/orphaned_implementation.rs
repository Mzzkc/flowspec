// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Orphaned implementation detection — methods with zero dispatch points.
//!
//! Finds `SymbolKind::Method` symbols that have zero inbound Call edges.
//! These are implementations that exist but are never dispatched to.
//! Distinguishes from `data_dead_end` by targeting only methods (not
//! functions, variables, or constants). Both patterns may fire on the
//! same method — the messages and suggestions differ.

use std::collections::HashSet;
use std::path::Path;

use crate::analyzer::diagnostic::*;
use crate::analyzer::patterns::exclusion::{is_excluded_symbol, relativize_path};
use crate::graph::Graph;
use crate::parser::ir::{EdgeKind, SymbolKind, Visibility};

/// Detect orphaned implementations in the analysis graph.
///
/// An orphaned implementation is a `SymbolKind::Method` with zero callers.
/// Exclusions: dunder methods (runtime-dispatched), entry points, test
/// file methods, import symbols. Confidence is calibrated by visibility:
/// private methods with zero callers are high confidence; public methods
/// might be dispatched dynamically (moderate confidence).
///
/// The `project_root` path is used to produce relative file paths in
/// diagnostic locations, matching the format of entity `loc` fields.
///
/// Severity: Warning. Confidence: High (private) or Moderate (public).
pub fn detect(graph: &Graph, project_root: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    let file_count = graph
        .all_symbols()
        .map(|(_, s)| s.location.file.clone())
        .collect::<HashSet<_>>()
        .len();

    for (id, symbol) in graph.all_symbols() {
        // Only target Method symbols — Function is data_dead_end's domain
        if symbol.kind != SymbolKind::Method {
            continue;
        }

        // Skip excluded symbols (shared exclusion logic)
        if is_excluded_symbol(symbol) {
            continue;
        }

        // Count inbound Call edges only (method dispatch)
        let caller_count = graph
            .edges_to(id)
            .iter()
            .filter(|edge| matches!(edge.kind, EdgeKind::Calls | EdgeKind::References))
            .count();

        if caller_count > 0 {
            continue;
        }

        // Determine confidence based on visibility
        let confidence = match symbol.visibility {
            Visibility::Private | Visibility::Protected => Confidence::High,
            Visibility::Crate => Confidence::Moderate,
            Visibility::Public => {
                // Underscore-prefixed public methods are likely internal
                if symbol.name.starts_with('_') && !symbol.name.starts_with("__") {
                    Confidence::High
                } else {
                    Confidence::Moderate
                }
            }
        };

        let location = format!(
            "{}:{}",
            relativize_path(&symbol.location.file, project_root),
            symbol.location.line
        );

        diagnostics.push(Diagnostic {
            id: String::new(),
            pattern: DiagnosticPattern::OrphanedImplementation,
            severity: Severity::Warning,
            confidence,
            entity: symbol.qualified_name.clone(),
            message: format!(
                "Orphaned implementation: method '{}' has no dispatch points",
                symbol.name
            ),
            evidence: vec![Evidence {
                observation: format!("0 callers in {} analyzed files", file_count),
                location: Some(location.clone()),
                context: Some(format!(
                    "visibility: {:?}, kind: Method",
                    symbol.visibility
                )),
            }],
            suggestion: format!(
                "Wire a dispatch path to '{}', or remove the implementation if it is no longer needed.",
                symbol.name
            ),
            location,
        });
    }

    diagnostics
}
