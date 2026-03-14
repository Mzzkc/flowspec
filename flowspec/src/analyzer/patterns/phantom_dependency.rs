// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Phantom dependency detection — imports where zero imported symbols are used.
//!
//! An import is phantom when nothing in the importing file references the
//! imported symbol. Prefix usage (`sys.argv` for `import sys`) counts as
//! a reference. Re-exports (`__all__`) count as usage. Type annotations
//! count as usage.

use std::collections::HashSet;
use std::path::Path;

use crate::analyzer::diagnostic::*;
use crate::analyzer::patterns::exclusion::relativize_path;
use crate::graph::Graph;
use crate::parser::ir::EdgeKind;

/// Detect phantom dependencies in the analysis graph.
///
/// Import symbols are identified by the "import" annotation. A phantom
/// import has zero incoming edges (Calls or References) from other
/// symbols in the same file.
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
        // Only check import symbols
        if !symbol.annotations.contains(&"import".to_string()) {
            continue;
        }

        // Get symbols in the same file
        let file_path = &symbol.location.file;
        let file_symbols: HashSet<_> = graph.symbols_in_file(file_path).into_iter().collect();

        // Check if any symbol in the same file references this import
        let same_file_references = graph
            .edges_to(id)
            .iter()
            .filter(|edge| {
                // edge.target is the SOURCE in incoming edges
                file_symbols.contains(&edge.target)
                    && edge.target != id
                    && matches!(edge.kind, EdgeKind::Calls | EdgeKind::References)
            })
            .count();

        if same_file_references > 0 {
            continue;
        }

        let location = format!(
            "{}:{}",
            relativize_path(&symbol.location.file, project_root),
            symbol.location.line
        );

        diagnostics.push(Diagnostic {
            id: String::new(), // Assigned by registry
            pattern: DiagnosticPattern::PhantomDependency,
            severity: Severity::Info,
            confidence: Confidence::High,
            entity: symbol.name.clone(),
            message: format!(
                "Phantom dependency: import '{}' is never used in this file",
                symbol.name
            ),
            evidence: vec![Evidence {
                observation: format!(
                    "0 references to '{}' in {} analyzed files",
                    symbol.name, file_count
                ),
                location: Some(location.clone()),
                context: Some("imported symbol has no usage in the importing file".to_string()),
            }],
            suggestion: format!(
                "Remove the unused import '{}' to reduce phantom dependencies and improve clarity.",
                symbol.name
            ),
            location,
        });
    }

    diagnostics
}
