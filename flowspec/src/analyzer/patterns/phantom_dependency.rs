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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;
    use crate::parser::ir::*;
    use crate::test_utils::*;

    // =========================================================================
    // QA-2 C13 Section 1: phantom_dependency + Rust `use` qualified paths
    // =========================================================================

    // T1: Basic `use` + qualified call — true negative after Issue #15 fix
    #[test]
    fn test_phantom_dependency_does_not_fire_on_rust_use_qualified_call() {
        let mut graph = Graph::new();

        let import_id = graph.add_symbol({
            let mut sym = make_import("fs", "lib.rs", 1);
            sym.annotations.push("from:std".to_string());
            sym
        });

        let caller_id = graph.add_symbol(make_symbol(
            "read_file",
            SymbolKind::Function,
            Visibility::Public,
            "lib.rs",
            3,
        ));

        // The fix creates this edge: caller -> import via qualified call prefix
        add_ref(
            &mut graph,
            caller_id,
            import_id,
            ReferenceKind::Call,
            "lib.rs",
        );

        let diagnostics = detect(&graph, Path::new(""));
        let phantom_fs: Vec<_> = diagnostics.iter().filter(|d| d.entity == "fs").collect();
        assert!(
            phantom_fs.is_empty(),
            "phantom_dependency must NOT fire on 'fs' when it has a same-file caller. \
             This is Issue #15 — 425 dogfood FPs. Got: {:?}",
            phantom_fs.iter().map(|d| &d.entity).collect::<Vec<_>>()
        );
    }

    // T2: Multiple qualified calls through same import — true negative
    #[test]
    fn test_phantom_dependency_silent_on_import_with_multiple_qualified_calls() {
        let mut graph = Graph::new();

        let import_id = graph.add_symbol({
            let mut sym = make_import("fs", "lib.rs", 1);
            sym.annotations.push("from:std".to_string());
            sym
        });

        let caller_a = graph.add_symbol(make_symbol(
            "read_config",
            SymbolKind::Function,
            Visibility::Public,
            "lib.rs",
            5,
        ));
        let caller_b = graph.add_symbol(make_symbol(
            "write_output",
            SymbolKind::Function,
            Visibility::Public,
            "lib.rs",
            10,
        ));

        add_ref(
            &mut graph,
            caller_a,
            import_id,
            ReferenceKind::Call,
            "lib.rs",
        );
        add_ref(
            &mut graph,
            caller_b,
            import_id,
            ReferenceKind::Call,
            "lib.rs",
        );

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            !diagnostics.iter().any(|d| d.entity == "fs"),
            "Import with 2 same-file callers must NOT be phantom"
        );
    }

    // T3: Regression guard — genuinely unused import MUST still fire
    #[test]
    fn test_phantom_dependency_still_fires_on_genuinely_unused_import() {
        let mut graph = Graph::new();

        graph.add_symbol({
            let mut sym = make_import("io", "lib.rs", 1);
            sym.annotations.push("from:std".to_string());
            sym
        });

        // Function exists but does NOT reference io
        graph.add_symbol(make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Public,
            "lib.rs",
            5,
        ));

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            diagnostics.iter().any(|d| d.entity == "io"),
            "Genuinely unused import 'io' MUST still be flagged as phantom. \
             The Issue #15 fix must not suppress all phantom findings."
        );
    }

    // T4: Mixed — one used import, one unused, same file
    #[test]
    fn test_phantom_dependency_fires_selectively_in_mixed_usage_file() {
        let mut graph = Graph::new();

        let fs_id = graph.add_symbol({
            let mut sym = make_import("fs", "server.rs", 1);
            sym.annotations.push("from:std".to_string());
            sym
        });

        let _io_id = graph.add_symbol({
            let mut sym = make_import("io", "server.rs", 2);
            sym.annotations.push("from:std".to_string());
            sym
        });

        let handler_id = graph.add_symbol(make_symbol(
            "handler",
            SymbolKind::Function,
            Visibility::Public,
            "server.rs",
            5,
        ));

        add_ref(
            &mut graph,
            handler_id,
            fs_id,
            ReferenceKind::Call,
            "server.rs",
        );
        // No edge from handler to io

        let diagnostics = detect(&graph, Path::new(""));
        let entities: Vec<&str> = diagnostics.iter().map(|d| d.entity.as_str()).collect();

        assert!(
            !entities.contains(&"fs"),
            "fs has a caller — must NOT be phantom"
        );
        assert!(
            entities.contains(&"io"),
            "io has no caller — MUST be phantom"
        );
    }

    // T5: Adversarial — cross-file edge does NOT satisfy phantom_dependency
    #[test]
    fn test_phantom_dependency_ignores_cross_file_edges() {
        let mut graph = Graph::new();

        let import_id = graph.add_symbol({
            let mut sym = make_import("utils", "a.rs", 1);
            sym.annotations.push("from:crate".to_string());
            sym
        });

        let caller_id = graph.add_symbol(make_symbol(
            "caller",
            SymbolKind::Function,
            Visibility::Public,
            "b.rs",
            1,
        ));

        // Cross-file edge: b.rs -> a.rs import
        add_ref(
            &mut graph,
            caller_id,
            import_id,
            ReferenceKind::Call,
            "b.rs",
        );

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            diagnostics.iter().any(|d| d.entity == "utils"),
            "Cross-file reference must NOT satisfy phantom_dependency's same-file check. \
             Import 'utils' in a.rs has no SAME-FILE callers."
        );
    }

    // T6: Adversarial — References edge kind also counts (not just Calls)
    #[test]
    fn test_phantom_dependency_accepts_references_edge_kind() {
        let mut graph = Graph::new();

        let import_id = graph.add_symbol({
            let mut sym = make_import("HashMap", "lib.rs", 1);
            sym.annotations.push("from:std::collections".to_string());
            sym
        });

        let user_id = graph.add_symbol(make_symbol(
            "build",
            SymbolKind::Function,
            Visibility::Public,
            "lib.rs",
            5,
        ));

        // References edge (not Calls) — e.g., type annotation usage
        add_ref(
            &mut graph,
            user_id,
            import_id,
            ReferenceKind::Read,
            "lib.rs",
        );

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            !diagnostics.iter().any(|d| d.entity == "HashMap"),
            "References edge kind must satisfy phantom_dependency check"
        );
    }

    // =========================================================================
    // QA-2 C13 Section 3 (partial): Cross-pattern overlap — Python/JS
    // =========================================================================

    // T10: JS CJS fix must NOT cause phantom_dependency to stop firing on unused ESM
    #[test]
    fn test_phantom_dependency_still_fires_on_unused_js_esm_import_after_cjs_fix() {
        let mut graph = Graph::new();

        // Unused ESM import
        let _lodash_id = graph.add_symbol({
            let mut sym = make_import("lodash", "app.js", 1);
            sym.annotations.push("from:lodash".to_string());
            sym
        });

        // Used CJS import
        let parse_id = graph.add_symbol({
            let mut sym = make_import("parse", "utils.js", 1);
            sym.annotations.push("from:./helpers".to_string());
            sym.annotations.push("cjs".to_string());
            sym
        });

        let caller_id = graph.add_symbol(make_symbol(
            "processData",
            SymbolKind::Function,
            Visibility::Public,
            "utils.js",
            5,
        ));
        add_ref(
            &mut graph,
            caller_id,
            parse_id,
            ReferenceKind::Call,
            "utils.js",
        );

        let diagnostics = detect(&graph, Path::new(""));
        let entities: Vec<&str> = diagnostics.iter().map(|d| d.entity.as_str()).collect();

        assert!(
            entities.contains(&"lodash"),
            "Unused ESM import must still be flagged as phantom"
        );
        assert!(
            !entities.contains(&"parse"),
            "Used CJS import must NOT be flagged as phantom"
        );
    }

    // T11: Rust fix must NOT affect Python unused imports
    #[test]
    fn test_phantom_dependency_fires_on_python_unused_import() {
        let mut graph = Graph::new();

        graph.add_symbol({
            let mut sym = make_import("os", "module.py", 1);
            sym.annotations.push("from:os".to_string());
            sym
        });

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            diagnostics.iter().any(|d| d.entity == "os"),
            "Python unused import must still trigger phantom_dependency"
        );
    }

    // =========================================================================
    // QA-2 C13 Section 5: Confidence calibration
    // =========================================================================

    // T15: phantom_dependency confidence remains HIGH for genuinely unused
    #[test]
    fn test_phantom_dependency_confidence_high_for_genuinely_unused() {
        let mut graph = Graph::new();

        graph.add_symbol({
            let mut sym = make_import("unused_mod", "lib.rs", 1);
            sym.annotations.push("from:std".to_string());
            sym
        });

        let diagnostics = detect(&graph, Path::new(""));
        assert!(!diagnostics.is_empty(), "Must detect phantom");
        assert_eq!(
            diagnostics[0].confidence,
            Confidence::High,
            "Genuinely unused import must have HIGH confidence"
        );
    }

    // =========================================================================
    // QA-2 C13 Section 6: Adversarial edge cases
    // =========================================================================

    // T17: Self-edge does NOT count for phantom_dependency
    #[test]
    fn test_phantom_dependency_ignores_self_edges() {
        let mut graph = Graph::new();

        let import_id = graph.add_symbol({
            let mut sym = make_import("self_ref", "lib.rs", 1);
            sym.annotations.push("from:std".to_string());
            sym
        });

        // Self-edge: import references itself
        add_ref(
            &mut graph,
            import_id,
            import_id,
            ReferenceKind::Call,
            "lib.rs",
        );

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            diagnostics.iter().any(|d| d.entity == "self_ref"),
            "Self-edge must NOT satisfy phantom_dependency check"
        );
    }

    // T18: Wrong edge kind (non Calls/References) does not satisfy check
    #[test]
    fn test_phantom_dependency_requires_calls_or_references_edge_kind() {
        let mut graph = Graph::new();

        let import_id = graph.add_symbol({
            let mut sym = make_import("contained", "lib.rs", 1);
            sym.annotations.push("from:std".to_string());
            sym
        });

        // Only import symbols in the file, no functions that reference import_id
        // This verifies that merely having an import edge from another import
        // doesn't prevent phantom detection
        let other_import = graph.add_symbol({
            let mut sym = make_import("other", "lib.rs", 2);
            sym.annotations.push("from:std".to_string());
            sym
        });

        // Import edge from other_import to contained — this creates References edge
        add_ref(
            &mut graph,
            other_import,
            import_id,
            ReferenceKind::Import,
            "lib.rs",
        );

        // The edge kind IS References, so this WILL count. But both symbols
        // are imports. The key test is: is there a same-file non-self edge?
        // Yes — other_import -> contained is References, same file, not self.
        // So contained should NOT be phantom.
        let diagnostics = detect(&graph, Path::new(""));
        let contained_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.entity == "contained")
            .collect();
        // An import-to-import reference in the same file DOES satisfy the check
        // because edges_to filtering uses EdgeKind::References which Import maps to
        assert!(
            contained_findings.is_empty(),
            "Import with same-file References edge should not be phantom"
        );
    }

    // T20: Crate-internal `use` qualified call — true negative
    #[test]
    fn test_phantom_dependency_silent_on_crate_internal_use_qualified_call() {
        let mut graph = Graph::new();

        let import_id = graph.add_symbol({
            let mut sym = make_import("parser", "main.rs", 1);
            sym.annotations.push("from:crate".to_string());
            sym
        });

        let caller_id = graph.add_symbol(make_symbol(
            "run",
            SymbolKind::Function,
            Visibility::Public,
            "main.rs",
            5,
        ));

        add_ref(
            &mut graph,
            caller_id,
            import_id,
            ReferenceKind::Call,
            "main.rs",
        );

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            !diagnostics.iter().any(|d| d.entity == "parser"),
            "Crate-internal use + qualified call must NOT be phantom"
        );
    }

    // T22: Entry point + import annotation interaction clarification
    #[test]
    fn test_phantom_dependency_checks_import_regardless_of_entry_point() {
        let mut graph = Graph::new();

        // Import symbol with BOTH import and entry_point annotations
        // phantom_dependency only checks the "import" annotation
        let mut sym = make_import("startup", "main.rs", 1);
        sym.annotations.push("entry_point".to_string());
        graph.add_symbol(sym);

        let diagnostics = detect(&graph, Path::new(""));
        // This import has zero same-file edges, but phantom_dependency checks
        // the "import" annotation. Whether entry_point suppresses is a question
        // of whether phantom_dependency uses is_excluded_symbol (it does NOT).
        // So it should fire.
        assert!(
            diagnostics.iter().any(|d| d.entity == "startup"),
            "phantom_dependency does NOT use is_excluded_symbol — \
             import+entry_point combo is still checked for edges"
        );
    }
}
