// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Data dead end detection — symbols defined but never consumed.
//!
//! Functions with zero callers, variables assigned but never read.
//! Confidence is calibrated by visibility: private functions with zero
//! callers are high confidence; public functions might be library API
//! consumed by external code we can't see (low confidence).

use std::collections::HashSet;

use crate::analyzer::diagnostic::*;
use crate::graph::Graph;
use crate::parser::ir::{EdgeKind, SymbolKind, Visibility};

/// Detect data dead ends in the analysis graph.
///
/// A dead end is a symbol with zero inbound consumption edges (calls,
/// references). Exclusions: entry points, test functions, test modules,
/// import symbols (handled by phantom_dependency), module symbols.
pub fn detect(graph: &Graph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    let file_count = graph
        .all_symbols()
        .map(|(_, s)| s.location.file.clone())
        .collect::<HashSet<_>>()
        .len();

    for (id, symbol) in graph.all_symbols() {
        // Skip excluded symbols
        if is_excluded(symbol) {
            continue;
        }

        // Count inbound consumption edges (Calls + References, not Contains)
        let inbound_count = graph
            .edges_to(id)
            .iter()
            .filter(|edge| matches!(edge.kind, EdgeKind::Calls | EdgeKind::References))
            .count();

        if inbound_count > 0 {
            continue;
        }

        // Determine confidence based on visibility
        let confidence = match symbol.visibility {
            Visibility::Private | Visibility::Protected => Confidence::High,
            Visibility::Crate => Confidence::Moderate,
            Visibility::Public => {
                // Check if the name starts with underscore (Python private convention)
                if symbol.name.starts_with('_') && !symbol.name.starts_with("__") {
                    Confidence::High
                } else {
                    Confidence::Low
                }
            }
        };

        let location = format!(
            "{}:{}",
            symbol.location.file.display(),
            symbol.location.line
        );

        diagnostics.push(Diagnostic {
            id: String::new(), // Assigned by registry
            pattern: DiagnosticPattern::DataDeadEnd,
            severity: Severity::Warning,
            confidence,
            entity: symbol.qualified_name.clone(),
            message: format!(
                "Dead end: {} '{}' is defined but never called or referenced",
                kind_label(symbol.kind),
                symbol.name
            ),
            evidence: vec![Evidence {
                observation: format!("0 callers in {} analyzed files", file_count),
                location: Some(location.clone()),
                context: Some(format!(
                    "visibility: {:?}, kind: {:?}",
                    symbol.visibility, symbol.kind
                )),
            }],
            suggestion: format!(
                "Remove '{}' if it is no longer needed, or add a caller. \
                 If this is intentional API surface, consider marking it as an entry point.",
                symbol.name
            ),
            location,
        });
    }

    diagnostics
}

/// Check if a symbol should be excluded from dead-end detection.
fn is_excluded(symbol: &crate::parser::ir::Symbol) -> bool {
    // Skip entry points (explicitly marked as called from outside analysis scope)
    if symbol.annotations.contains(&"entry_point".to_string()) {
        return true;
    }

    // Skip import symbols (handled by phantom_dependency)
    if symbol.annotations.contains(&"import".to_string()) {
        return true;
    }

    // Skip module symbols (structural, not dead code)
    if symbol.kind == SymbolKind::Module {
        return true;
    }

    // Skip class symbols (classes are structural containers)
    if symbol.kind == SymbolKind::Class || symbol.kind == SymbolKind::Struct {
        return true;
    }

    // Skip entry points (main, main_handler, __main__, etc.)
    if symbol.name == "main"
        || symbol.name == "__main__"
        || symbol.name == "if_name_main"
        || symbol.name.starts_with("main_")
        || symbol.name.ends_with("_main")
    {
        return true;
    }

    // Skip test functions
    if symbol.name.starts_with("test_") {
        return true;
    }

    // Skip test modules (file path contains test indicators)
    let path = symbol.location.file.to_string_lossy();
    if path.contains("test_") || path.contains("/tests/") || path.contains("/test/") {
        return true;
    }

    // Skip dunder methods (Python special methods)
    if symbol.name.starts_with("__") && symbol.name.ends_with("__") {
        return true;
    }

    false
}

/// Human-readable label for a symbol kind.
fn kind_label(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Class => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Variable => "variable",
        SymbolKind::Constant => "constant",
        SymbolKind::Module => "module",
        SymbolKind::Trait => "trait",
        SymbolKind::Interface => "interface",
        SymbolKind::Macro => "macro",
        SymbolKind::Enum => "enum",
    }
}
