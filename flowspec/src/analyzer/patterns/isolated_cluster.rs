// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Isolated cluster detection — connected components with zero external inbound edges.
//!
//! An isolated cluster is a group of 2+ symbols that reference each other
//! internally but have zero inbound edges from outside the group. This
//! indicates an "unwired feature" — code that talks to itself but nothing
//! calls into it.

use std::collections::HashSet;

use crate::analyzer::diagnostic::*;
use crate::graph::Graph;
use crate::parser::ir::{EdgeKind, SymbolKind};

/// Detect isolated clusters in the analysis graph.
///
/// A cluster must have 2+ symbols with internal edges and zero external
/// inbound edges. Single orphaned symbols are `data_dead_end`, not clusters.
/// Test modules and re-export-only modules are excluded.
pub fn detect(graph: &Graph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let components = graph.connected_components();

    for component in &components {
        // Require 2+ symbols for a cluster
        if component.len() < 2 {
            continue;
        }

        // Filter: skip if all symbols are excluded (test modules, re-export-only)
        let non_excluded: Vec<_> = component
            .iter()
            .filter(|&&id| {
                if let Some(sym) = graph.get_symbol(id) {
                    !is_test_or_example(sym) && sym.kind != SymbolKind::Module
                } else {
                    false
                }
            })
            .collect();

        if non_excluded.len() < 2 {
            continue;
        }

        // Check if the entire component is in a test file
        let all_test_file = component.iter().all(|&id| {
            graph
                .get_symbol(id)
                .map(|s| is_test_file(&s.location.file.to_string_lossy()))
                .unwrap_or(false)
        });
        if all_test_file {
            continue;
        }

        // Check for external inbound edges
        let component_set: HashSet<_> = component.iter().copied().collect();
        let has_external_inbound = component.iter().any(|&id| {
            graph.edges_to(id).iter().any(|edge| {
                // In incoming edges, edge.target is the SOURCE symbol
                !component_set.contains(&edge.target)
                    && matches!(edge.kind, EdgeKind::Calls | EdgeKind::References)
            })
        });

        if has_external_inbound {
            continue;
        }

        // Check for internal edges (must have at least one to be a "cluster")
        let has_internal_edges = component.iter().any(|&id| {
            graph.edges_from(id).iter().any(|edge| {
                component_set.contains(&edge.target)
                    && matches!(edge.kind, EdgeKind::Calls | EdgeKind::References)
            })
        });

        if !has_internal_edges {
            continue;
        }

        // Check if this is a re-export-only module (no logic, just imports)
        let all_imports = non_excluded.iter().all(|&&id| {
            graph
                .get_symbol(id)
                .map(|s| s.annotations.contains(&"import".to_string()))
                .unwrap_or(false)
        });
        if all_imports {
            continue;
        }

        // Build the diagnostic
        let symbol_names: Vec<String> = non_excluded
            .iter()
            .filter_map(|&&id| graph.get_symbol(id).map(|s| s.name.clone()))
            .collect();

        let internal_edge_count: usize = component
            .iter()
            .map(|&id| {
                graph
                    .edges_from(id)
                    .iter()
                    .filter(|e| component_set.contains(&e.target))
                    .count()
            })
            .sum();

        let entity = symbol_names.join(", ");
        let first_location = component
            .iter()
            .filter_map(|&id| graph.get_symbol(id))
            .map(|s| format!("{}:{}", s.location.file.display(), s.location.line))
            .next()
            .unwrap_or_default();

        let file_count = graph
            .all_symbols()
            .map(|(_, s)| s.location.file.clone())
            .collect::<HashSet<_>>()
            .len();

        diagnostics.push(Diagnostic {
            id: String::new(), // Assigned by registry
            pattern: DiagnosticPattern::IsolatedCluster,
            severity: Severity::Warning,
            confidence: Confidence::High,
            entity,
            message: format!(
                "Isolated cluster of {} symbols with {} internal references but 0 external callers",
                non_excluded.len(),
                internal_edge_count
            ),
            evidence: vec![
                Evidence {
                    observation: format!("0 external callers across {} analyzed files", file_count),
                    location: Some(first_location.clone()),
                    context: None,
                },
                Evidence {
                    observation: format!(
                        "{} internal references between {} symbols",
                        internal_edge_count,
                        non_excluded.len()
                    ),
                    location: None,
                    context: Some(format!("symbols: {}", symbol_names.join(", "))),
                },
            ],
            suggestion: "Wire this cluster into the rest of the codebase by calling one of its \
                entry points, or remove it if no longer needed."
                .to_string(),
            location: first_location,
        });
    }

    diagnostics
}

/// Check if a symbol is in a test or example file.
fn is_test_or_example(sym: &crate::parser::ir::Symbol) -> bool {
    let path = sym.location.file.to_string_lossy();
    is_test_file(&path) || sym.name.starts_with("test_")
}

/// Check if a file path indicates a test file.
fn is_test_file(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized.contains("test_")
        || normalized.contains("/tests/")
        || normalized.contains("/test/")
        || normalized.ends_with("_test.py")
        || normalized.ends_with("_test.rs")
}
