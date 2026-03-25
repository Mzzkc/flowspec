// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Stale reference detection — imports referencing symbols that no longer exist.
//!
//! A stale reference is an import where the target module exists in the project
//! but the specific imported symbol cannot be found in that module. This happens
//! when functions are renamed, removed, or restructured without updating all
//! import sites.
//!
//! Two detection signals with distinct confidence levels:
//! - **Signal 1 (HIGH):** `ResolutionStatus::Partial("module resolved, symbol not found")`
//!   — the module file was found, the symbol name was searched, and it doesn't exist.
//! - **Signal 2 (MODERATE):** `ResolutionStatus::Unresolved` with a `from:` annotation
//!   pointing to a local-looking module — the module may have been deleted.
//!
//! Does NOT use `is_excluded_symbol()` as a pre-filter because that function
//! skips all import-annotated symbols, and this pattern specifically targets imports.

use std::path::Path;

use crate::analyzer::diagnostic::*;
use crate::analyzer::patterns::exclusion::relativize_path;
use crate::graph::Graph;
use crate::parser::ir::ResolutionStatus;

/// Detect stale references in the analysis graph.
///
/// Walks all import-annotated symbols and checks their resolution status.
/// Symbols with `Partial("module resolved, symbol not found")` are flagged
/// at HIGH confidence (evidence-based). Unresolved imports to local-looking
/// modules are flagged at MODERATE confidence (heuristic).
///
/// Star imports (`Partial("star import - module resolved")`) are skipped
/// because they are inherently ambiguous, not stale.
///
/// The `project_root` path is used to produce relative file paths in
/// diagnostic locations.
pub fn detect(graph: &Graph, project_root: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for (_id, symbol) in graph.all_symbols() {
        // Only check import symbols
        if !symbol.annotations.contains(&"import".to_string()) {
            continue;
        }

        // Extract the "from:" module annotation if present
        let from_module = symbol
            .annotations
            .iter()
            .find(|a| a.starts_with("from:"))
            .map(|a| &a[5..]);

        match &symbol.resolution {
            // Signal 1: Module exists but symbol not found — HIGH confidence
            ResolutionStatus::Partial(reason) if reason == "module resolved, symbol not found" => {
                let location = format!(
                    "{}:{}",
                    relativize_path(&symbol.location.file, project_root),
                    symbol.location.line
                );

                let module_info = from_module
                    .map(|m| format!(" from module '{}'", m))
                    .unwrap_or_default();

                diagnostics.push(Diagnostic {
                    id: String::new(),
                    pattern: DiagnosticPattern::StaleReference,
                    severity: Severity::Warning,
                    confidence: Confidence::High,
                    entity: symbol.name.clone(),
                    message: format!(
                        "Stale reference: import '{}'{} targets a symbol that no longer exists",
                        symbol.name, module_info
                    ),
                    evidence: vec![Evidence {
                        observation: format!(
                            "Symbol '{}' not found in target module{}. \
                             The symbol may have been renamed, removed, or moved.",
                            symbol.name, module_info
                        ),
                        location: Some(location.clone()),
                        context: Some("module resolved, symbol not found".to_string()),
                    }],
                    suggestion: format!(
                        "Update or remove the import of '{}'. Check if the symbol \
                         was renamed or moved to a different module.",
                        symbol.name
                    ),
                    location,
                });
            }

            // Skip star imports — ambiguous by nature, not stale
            ResolutionStatus::Partial(reason) if reason.starts_with("star import") => {
                continue;
            }

            // Signal 2: Unresolved import with local-looking "from:" module
            ResolutionStatus::Unresolved => {
                // Only flag if there's a "from:" annotation pointing to a
                // local-looking module. Without "from:", this is a bare import
                // (e.g., `import os`) which we skip.
                if let Some(module_name) = from_module {
                    // Check if this looks like a local module: relative import
                    // prefix (.) or the module name appears as a file in the graph
                    let is_local = module_name.starts_with('.')
                        || graph.all_symbols().any(|(_, s)| {
                            let file_str = s.location.file.to_string_lossy();
                            let stem = file_str
                                .rsplit('/')
                                .next()
                                .unwrap_or(&file_str)
                                .trim_end_matches(".py")
                                .trim_end_matches(".js")
                                .trim_end_matches(".ts")
                                .trim_end_matches(".rs");
                            stem == module_name
                        });

                    if is_local {
                        let location = format!(
                            "{}:{}",
                            relativize_path(&symbol.location.file, project_root),
                            symbol.location.line
                        );

                        diagnostics.push(Diagnostic {
                            id: String::new(),
                            pattern: DiagnosticPattern::StaleReference,
                            severity: Severity::Warning,
                            confidence: Confidence::Moderate,
                            entity: symbol.name.clone(),
                            message: format!(
                                "Stale reference: import '{}' from '{}' could not be resolved",
                                symbol.name, module_name
                            ),
                            evidence: vec![Evidence {
                                observation: format!(
                                    "Import '{}' from local module '{}' is unresolved. \
                                     The module exists but the symbol may have been removed.",
                                    symbol.name, module_name
                                ),
                                location: Some(location.clone()),
                                context: Some("unresolved import to local module".to_string()),
                            }],
                            suggestion: format!(
                                "Verify that '{}' still exists in module '{}'. \
                                 The symbol may have been renamed or removed.",
                                symbol.name, module_name
                            ),
                            location,
                        });
                    }
                }
            }

            // Resolved or other Partial — not stale
            _ => {}
        }
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
    // T1: True Positive — Renamed Function Import
    // =========================================================================

    #[test]
    fn test_stale_reference_fires_on_renamed_function_import() {
        let graph = build_stale_reference_graph();
        let diagnostics = detect(&graph, Path::new(""));

        assert!(
            !diagnostics.is_empty(),
            "stale_reference must fire when an import targets a renamed symbol"
        );

        let old_fn_diag = diagnostics
            .iter()
            .find(|d| d.entity.contains("old_function"));
        assert!(
            old_fn_diag.is_some(),
            "old_function must be flagged as stale reference. Got: {:?}",
            diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
        );
        assert_eq!(
            old_fn_diag.unwrap().pattern,
            DiagnosticPattern::StaleReference
        );
    }

    // =========================================================================
    // T2: True Negative — Valid Imports
    // =========================================================================

    #[test]
    fn test_stale_reference_does_not_fire_on_valid_imports() {
        let graph = build_simple_import_graph();
        let diagnostics = detect(&graph, Path::new(""));

        assert!(
            diagnostics.is_empty(),
            "stale_reference must NOT fire on valid resolved imports. Got {} findings: {:?}",
            diagnostics.len(),
            diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T3: Adversarial — Aliased Import That Resolves Successfully
    // =========================================================================

    #[test]
    fn test_stale_reference_does_not_fire_on_resolved_alias() {
        let graph = build_reexport_graph();
        let diagnostics = detect(&graph, Path::new(""));

        let alias_related = diagnostics
            .iter()
            .filter(|d| d.entity.contains("core_function") || d.entity.contains("public_function"))
            .collect::<Vec<_>>();

        assert!(
            alias_related.is_empty(),
            "Aliased imports that resolve successfully must NOT be flagged as stale. Got: {:?}",
            alias_related.iter().map(|d| &d.entity).collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T4: Cross-File Stale Reference (Symbol Removed From Target Module)
    // =========================================================================

    #[test]
    fn test_stale_reference_fires_on_cross_file_removed_symbol() {
        let mut graph = Graph::new();
        let file_a = "file_a.py";
        let file_b = "file_b.py";

        // file_b has `transform` but NOT `process`
        graph.add_symbol(make_symbol(
            "transform",
            SymbolKind::Function,
            Visibility::Public,
            file_b,
            1,
        ));

        // file_a imports `process` from file_b — but process doesn't exist
        let mut import_sym = make_import("process", file_a, 1);
        import_sym.annotations.push("from:file_b".to_string());
        import_sym.resolution =
            ResolutionStatus::Partial("module resolved, symbol not found".to_string());
        graph.add_symbol(import_sym);

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            diagnostics.iter().any(|d| d.entity.contains("process")),
            "Cross-file stale reference to removed symbol 'process' must be detected"
        );
    }

    // =========================================================================
    // T5: Confidence Level Calibration
    // =========================================================================

    #[test]
    fn test_stale_reference_confidence_high_for_module_resolved_symbol_missing() {
        let mut graph = Graph::new();
        let mut import_sym = make_import("old_fn", "consumer.py", 1);
        import_sym.annotations.push("from:provider".to_string());
        import_sym.resolution =
            ResolutionStatus::Partial("module resolved, symbol not found".to_string());
        graph.add_symbol(import_sym);

        graph.add_symbol(make_symbol(
            "new_fn",
            SymbolKind::Function,
            Visibility::Public,
            "provider.py",
            1,
        ));

        let diagnostics = detect(&graph, Path::new(""));
        assert!(!diagnostics.is_empty(), "Must detect stale reference");

        let diag = &diagnostics[0];
        assert_eq!(
            diag.confidence,
            Confidence::High,
            "Signal 1 (module resolved, symbol not found) must be HIGH confidence, \
             not {:?}. This is evidence-based, not heuristic.",
            diag.confidence
        );
    }

    #[test]
    fn test_stale_reference_confidence_moderate_for_heuristic_detection() {
        let mut graph = Graph::new();
        // Unresolved import to a local-looking module
        let mut import_sym = make_import("some_fn", "consumer.py", 1);
        import_sym.annotations.push("from:local_utils".to_string());
        import_sym.resolution = ResolutionStatus::Unresolved;
        graph.add_symbol(import_sym);

        // Add a symbol in local_utils.py so the module looks local
        graph.add_symbol(make_symbol(
            "other_fn",
            SymbolKind::Function,
            Visibility::Public,
            "local_utils.py",
            1,
        ));

        let diagnostics = detect(&graph, Path::new(""));

        for diag in &diagnostics {
            assert_ne!(
                diag.confidence,
                Confidence::High,
                "Heuristic-based detection (unresolved local import) must NOT use HIGH \
                 confidence. Got HIGH for entity: {}",
                diag.entity
            );
        }
    }

    // =========================================================================
    // T6: Evidence Quality — Location and Missing Target
    // =========================================================================

    #[test]
    fn test_stale_reference_evidence_includes_location_and_target() {
        let mut graph = Graph::new();
        let mut import_sym = make_import("defunct_function", "app.py", 7);
        import_sym.annotations.push("from:lib".to_string());
        import_sym.resolution =
            ResolutionStatus::Partial("module resolved, symbol not found".to_string());
        graph.add_symbol(import_sym);

        graph.add_symbol(make_symbol(
            "other_fn",
            SymbolKind::Function,
            Visibility::Public,
            "lib.py",
            1,
        ));

        let diagnostics = detect(&graph, Path::new(""));
        assert!(!diagnostics.is_empty(), "Must detect stale reference");

        let diag = &diagnostics[0];

        // Evidence is non-empty
        assert!(
            !diag.evidence.is_empty(),
            "stale_reference diagnostic must include evidence"
        );

        // Location references the source file
        assert!(
            diag.location.contains("app.py"),
            "Location must reference the importing file. Got: {}",
            diag.location
        );

        // Entity or evidence mentions the missing symbol
        let mentions_target = diag.entity.contains("defunct_function")
            || diag
                .evidence
                .iter()
                .any(|e| e.observation.contains("defunct_function"));
        assert!(
            mentions_target,
            "Evidence or entity must mention the specific missing symbol 'defunct_function'"
        );
    }

    // =========================================================================
    // T7: Multiple Stale References in One File
    // =========================================================================

    #[test]
    fn test_stale_reference_detects_all_stale_imports_in_one_file() {
        let mut graph = Graph::new();
        let consumer = "consumer.py";
        let provider = "provider.py";

        // Provider has active_helper only
        graph.add_symbol(make_symbol(
            "active_helper",
            SymbolKind::Function,
            Visibility::Public,
            provider,
            1,
        ));

        // Consumer imports 3 things, 2 are stale
        for (name, resolution) in [
            (
                "removed_helper",
                ResolutionStatus::Partial("module resolved, symbol not found".to_string()),
            ),
            (
                "another_removed",
                ResolutionStatus::Partial("module resolved, symbol not found".to_string()),
            ),
            ("active_helper", ResolutionStatus::Resolved),
        ] {
            let mut import_sym = make_import(name, consumer, 1);
            import_sym.annotations.push("from:provider".to_string());
            import_sym.resolution = resolution;
            graph.add_symbol(import_sym);
        }

        let diagnostics = detect(&graph, Path::new(""));

        let stale_entities: Vec<&str> = diagnostics.iter().map(|d| d.entity.as_str()).collect();
        assert!(
            stale_entities.iter().any(|e| e.contains("removed_helper")),
            "removed_helper must be detected. Got: {:?}",
            stale_entities
        );
        assert!(
            stale_entities.iter().any(|e| e.contains("another_removed")),
            "another_removed must be detected. Got: {:?}",
            stale_entities
        );
        assert!(
            diagnostics.len() >= 2,
            "At least 2 stale references must be reported, got {}",
            diagnostics.len()
        );
    }

    // =========================================================================
    // T8: Stale Reference in Test File Still Reported
    // =========================================================================

    #[test]
    fn test_stale_reference_reports_findings_in_test_files() {
        let mut graph = Graph::new();
        let test_file = "test_integration.py";
        let provider = "module.py";

        graph.add_symbol(make_symbol(
            "current_fn",
            SymbolKind::Function,
            Visibility::Public,
            provider,
            1,
        ));

        let mut import_sym = make_import("old_api", test_file, 3);
        import_sym.annotations.push("from:module".to_string());
        import_sym.resolution =
            ResolutionStatus::Partial("module resolved, symbol not found".to_string());
        graph.add_symbol(import_sym);

        let diagnostics = detect(&graph, Path::new(""));

        assert!(
            diagnostics.iter().any(|d| d.entity.contains("old_api")),
            "Stale reference in test file must still be reported. \
             The detector must NOT use is_excluded_symbol() as a pre-filter \
             because it skips all import-annotated symbols."
        );
    }

    // =========================================================================
    // T9: Star Import Should NOT Fire
    // =========================================================================

    #[test]
    fn test_stale_reference_does_not_fire_on_star_import() {
        let mut graph = Graph::new();
        let mut import_sym = make_import("*:utils", "main.py", 1);
        import_sym.annotations.push("from:utils".to_string());
        import_sym.resolution =
            ResolutionStatus::Partial("star import - module resolved".to_string());
        graph.add_symbol(import_sym);

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Star imports must NOT be flagged as stale references. Got: {:?}",
            diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T10: Missing Module (Third-Party) Should NOT Fire
    // =========================================================================

    #[test]
    fn test_stale_reference_does_not_fire_on_unresolved_third_party() {
        let mut graph = Graph::new();
        let mut import_sym = make_import("get", "app.py", 1);
        import_sym.annotations.push("from:requests".to_string());
        import_sym.resolution = ResolutionStatus::Unresolved;
        graph.add_symbol(import_sym);
        // Note: NO symbols in "requests.py" — it's a third-party module

        let diagnostics = detect(&graph, Path::new(""));

        // Either no diagnostic fires, or if it does, confidence must not be High
        for diag in &diagnostics {
            if diag.entity.contains("get") {
                assert_ne!(
                    diag.confidence,
                    Confidence::High,
                    "Third-party unresolved import must NOT be HIGH confidence"
                );
            }
        }
    }

    // =========================================================================
    // T11: Empty Graph Produces No Diagnostics (No Panic)
    // =========================================================================

    #[test]
    fn test_stale_reference_empty_graph_no_panic() {
        let graph = Graph::new();
        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Empty graph must produce zero diagnostics"
        );
    }

    // =========================================================================
    // T12: Import Symbol Without "from:" Annotation Handled Gracefully
    // =========================================================================

    #[test]
    fn test_stale_reference_handles_import_without_from_annotation() {
        let mut graph = Graph::new();
        // Import with no "from:" annotation — bare import
        let mut import_sym = make_import("os", "script.py", 1);
        // Note: only has "import" annotation, no "from:" annotation
        import_sym.resolution = ResolutionStatus::Unresolved;
        graph.add_symbol(import_sym);

        let diagnostics = detect(&graph, Path::new(""));
        // Should not panic and should not flag bare imports without "from:"
        let os_diags: Vec<_> = diagnostics.iter().filter(|d| d.entity == "os").collect();
        assert!(
            os_diags.is_empty(),
            "Bare imports without 'from:' annotation should be skipped, not flagged"
        );
    }

    // =========================================================================
    // T13: Severity Is Warning (Per Spec)
    // =========================================================================

    #[test]
    fn test_stale_reference_severity_is_warning() {
        let mut graph = Graph::new();
        let mut import_sym = make_import("old_fn", "main.py", 1);
        import_sym.annotations.push("from:utils".to_string());
        import_sym.resolution =
            ResolutionStatus::Partial("module resolved, symbol not found".to_string());
        graph.add_symbol(import_sym);

        graph.add_symbol(make_symbol(
            "new_fn",
            SymbolKind::Function,
            Visibility::Public,
            "utils.py",
            1,
        ));

        let diagnostics = detect(&graph, Path::new(""));
        assert!(!diagnostics.is_empty());
        assert_eq!(
            diagnostics[0].severity,
            Severity::Warning,
            "stale_reference severity must be Warning per spec. Got: {:?}",
            diagnostics[0].severity
        );
    }

    // =========================================================================
    // Graph builders for tests
    // =========================================================================

    /// Build a graph with a stale reference: main.py imports old_function from utils.py,
    /// but utils.py only has new_function and valid_function.
    fn build_stale_reference_graph() -> Graph {
        let mut graph = Graph::new();

        // utils.py: new_function (renamed from old_function), valid_function
        graph.add_symbol(make_symbol(
            "new_function",
            SymbolKind::Function,
            Visibility::Public,
            "utils.py",
            1,
        ));
        graph.add_symbol(make_symbol(
            "valid_function",
            SymbolKind::Function,
            Visibility::Public,
            "utils.py",
            5,
        ));

        // main.py: imports old_function (stale) and valid_function (ok)
        let mut stale_import = make_import("old_function", "main.py", 1);
        stale_import.annotations.push("from:utils".to_string());
        stale_import.resolution =
            ResolutionStatus::Partial("module resolved, symbol not found".to_string());
        graph.add_symbol(stale_import);

        let mut valid_import = make_import("valid_function", "main.py", 2);
        valid_import.annotations.push("from:utils".to_string());
        valid_import.resolution = ResolutionStatus::Resolved;
        graph.add_symbol(valid_import);

        graph
    }

    /// Build a graph with valid cross-file imports (simple_import fixture).
    fn build_simple_import_graph() -> Graph {
        let mut graph = Graph::new();

        // b.py defines helper
        graph.add_symbol(make_symbol(
            "helper",
            SymbolKind::Function,
            Visibility::Public,
            "b.py",
            1,
        ));

        // a.py imports helper — resolved successfully
        let mut import_sym = make_import("helper", "a.py", 1);
        import_sym.annotations.push("from:b".to_string());
        import_sym.resolution = ResolutionStatus::Resolved;
        graph.add_symbol(import_sym);

        graph
    }

    /// Build a graph with aliased import that resolves (reexport fixture).
    fn build_reexport_graph() -> Graph {
        let mut graph = Graph::new();

        // internal.py defines core_function
        graph.add_symbol(make_symbol(
            "core_function",
            SymbolKind::Function,
            Visibility::Public,
            "internal.py",
            1,
        ));

        // api.py: from internal import core_function as public_function
        let mut import_sym = make_import("public_function", "api.py", 1);
        import_sym.annotations.push("from:internal".to_string());
        import_sym
            .annotations
            .push("original_name:core_function".to_string());
        import_sym.resolution = ResolutionStatus::Resolved;
        graph.add_symbol(import_sym);

        graph
    }

    // =========================================================================
    // QA-2 C13 Section 2: stale_reference + CJS destructured imports
    // =========================================================================

    // T7: Valid CJS destructured import — true negative
    #[test]
    fn test_stale_reference_does_not_fire_on_resolved_cjs_destructured_import() {
        let mut graph = Graph::new();

        graph.add_symbol(make_symbol(
            "parse",
            SymbolKind::Function,
            Visibility::Public,
            "utils.js",
            1,
        ));

        let mut import_sym = make_import("parse", "app.js", 1);
        import_sym.annotations.push("from:./utils".to_string());
        import_sym.annotations.push("cjs".to_string());
        import_sym.resolution = ResolutionStatus::Resolved;
        graph.add_symbol(import_sym);

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            !diagnostics.iter().any(|d| d.entity == "parse"),
            "Resolved CJS destructured import must NOT be flagged as stale"
        );
    }

    // T8: CJS destructured import with alias — true negative
    #[test]
    fn test_stale_reference_silent_on_resolved_aliased_cjs_import() {
        let mut graph = Graph::new();

        graph.add_symbol(make_symbol(
            "formatDate",
            SymbolKind::Function,
            Visibility::Public,
            "utils.js",
            1,
        ));

        let mut import_sym = make_import("fmt", "app.js", 1);
        import_sym.annotations.push("from:./utils".to_string());
        import_sym.annotations.push("cjs".to_string());
        import_sym
            .annotations
            .push("original_name:formatDate".to_string());
        import_sym.resolution = ResolutionStatus::Resolved;
        graph.add_symbol(import_sym);

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            !diagnostics.iter().any(|d| d.entity == "fmt"),
            "Resolved aliased CJS import must NOT be flagged as stale"
        );
    }

    // T9: CJS import that fails resolution — true positive preserved
    #[test]
    fn test_stale_reference_fires_on_unresolved_cjs_destructured_import() {
        let mut graph = Graph::new();

        graph.add_symbol(make_symbol(
            "existingFn",
            SymbolKind::Function,
            Visibility::Public,
            "utils.js",
            1,
        ));

        let mut import_sym = make_import("removedFn", "app.js", 1);
        import_sym.annotations.push("from:./utils".to_string());
        import_sym.annotations.push("cjs".to_string());
        import_sym.resolution =
            ResolutionStatus::Partial("module resolved, symbol not found".to_string());
        graph.add_symbol(import_sym);

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            diagnostics.iter().any(|d| d.entity == "removedFn"),
            "CJS import to removed symbol MUST be flagged as stale reference"
        );
        let diag = diagnostics
            .iter()
            .find(|d| d.entity == "removedFn")
            .unwrap();
        assert_eq!(
            diag.confidence,
            Confidence::High,
            "Module-resolved-symbol-not-found must be HIGH confidence"
        );
    }

    // =========================================================================
    // QA-2 C13 Section 5: Confidence calibration for CJS
    // =========================================================================

    // T16: CJS annotation does not downgrade confidence
    #[test]
    fn test_stale_reference_confidence_high_for_cjs_module_resolved_symbol_missing() {
        let mut graph = Graph::new();

        graph.add_symbol(make_symbol(
            "existing",
            SymbolKind::Function,
            Visibility::Public,
            "utils.js",
            1,
        ));

        let mut import_sym = make_import("deleted", "app.js", 1);
        import_sym.annotations.push("from:utils".to_string());
        import_sym.annotations.push("cjs".to_string());
        import_sym.resolution =
            ResolutionStatus::Partial("module resolved, symbol not found".to_string());
        graph.add_symbol(import_sym);

        let diagnostics = detect(&graph, Path::new(""));
        let diag = diagnostics
            .iter()
            .find(|d| d.entity == "deleted")
            .expect("Must detect stale CJS import");
        assert_eq!(
            diag.confidence,
            Confidence::High,
            "CJS annotation must not downgrade confidence from HIGH for Signal 1"
        );
    }
}
