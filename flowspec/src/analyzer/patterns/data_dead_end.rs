// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Data dead end detection — symbols defined but never consumed.
//!
//! Functions with zero callers, variables assigned but never read.
//! Confidence is calibrated by visibility: private functions with zero
//! callers are high confidence; public functions might be library API
//! consumed by external code we can't see (low confidence).

use std::collections::HashSet;
use std::path::Path;

use crate::analyzer::diagnostic::*;
use crate::analyzer::patterns::exclusion::{is_excluded_symbol, relativize_path};
use crate::graph::Graph;
use crate::parser::ir::{EdgeKind, SymbolKind, Visibility};

/// Detect data dead ends in the analysis graph.
///
/// A dead end is a symbol with zero inbound consumption edges (calls,
/// references). Exclusions: entry points, test functions, test modules,
/// import symbols (handled by phantom_dependency), module symbols,
/// class/struct symbols (structural containers).
///
/// The `project_root` path is used to produce relative file paths in
/// diagnostic locations, matching the format of entity `loc` fields.
pub fn detect(graph: &Graph, project_root: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    let file_count = graph
        .all_symbols()
        .map(|(_, s)| s.location.file.clone())
        .collect::<HashSet<_>>()
        .len();

    for (id, symbol) in graph.all_symbols() {
        // Skip excluded symbols (shared exclusion logic)
        if is_excluded_symbol(symbol) {
            continue;
        }

        // Pattern-specific: skip structural container kinds and Methods
        // (Methods have a dedicated orphaned_implementation pattern)
        if symbol.kind == SymbolKind::Module
            || symbol.kind == SymbolKind::Class
            || symbol.kind == SymbolKind::Struct
            || symbol.kind == SymbolKind::Method
        {
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
            relativize_path(&symbol.location.file, project_root),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ir::ReferenceKind;
    use crate::test_utils::*;

    // =========================================================================
    // QA-2 C13 Section 3: Cross-pattern domain overlap — CJS import exclusion
    // =========================================================================

    // =========================================================================
    // QA-2 C14 Section 3: Cross-Pattern Regression — data_dead_end
    // =========================================================================

    // T18: Import-annotated symbols still excluded after type reference fix
    #[test]
    fn test_c14_data_dead_end_excludes_import_symbols_after_type_reference_fix() {
        let mut graph = Graph::new();

        let import_id = graph.add_symbol({
            let mut sym = make_import("Config", "lib.rs", 1);
            sym.annotations.push("from:crate::config".to_string());
            sym
        });

        let func_id = graph.add_symbol(make_symbol(
            "load",
            SymbolKind::Function,
            Visibility::Public,
            "lib.rs",
            10,
        ));

        // Type reference edge from Worker 1's fix
        add_ref(
            &mut graph,
            func_id,
            import_id,
            ReferenceKind::Read,
            "lib.rs",
        );

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            !diagnostics.iter().any(|d| d.entity.contains("Config")),
            "Import symbol 'Config' must be excluded from data_dead_end via \
             is_excluded_symbol() annotation check, regardless of edge count"
        );
    }

    // =========================================================================
    // QA-2 C13 Section 3: Cross-pattern domain overlap — CJS import exclusion
    // =========================================================================

    // T12: CJS-imported symbols must be excluded from data_dead_end
    #[test]
    fn test_data_dead_end_does_not_fire_on_cjs_import_symbol() {
        let mut graph = Graph::new();

        graph.add_symbol({
            let mut sym = make_import("parse", "app.js", 1);
            sym.annotations.push("from:./utils".to_string());
            sym.annotations.push("cjs".to_string());
            sym
        });

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            !diagnostics.iter().any(|d| d.entity.contains("parse")),
            "CJS import symbol must be excluded from data_dead_end via is_excluded_symbol()"
        );
    }
}
