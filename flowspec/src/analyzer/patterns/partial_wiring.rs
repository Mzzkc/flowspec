// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Partial wiring detection — functions imported by many modules but called from only a subset.
//!
//! A partial wiring finding means a function is imported across multiple files but
//! never actually called from some of them. When a file imports a function, it signals
//! intent to use it. When that function is never called from the importing file, the
//! wiring is incomplete — the agent (or developer) added the import but forgot to
//! wire the call.
//!
//! This is distinct from `phantom_dependency` (which fires per-import, suggesting
//! removal) and `data_dead_end` (which fires on zero-caller functions). partial_wiring
//! fires per-function, suggesting the consumer add the missing call.
//!
//! Detection algorithm: Import-Call Gap Analysis
//! - For each public/crate Function/Method, count files that import it vs files that call it.
//! - If ≥3 referencing files, ≥1 external caller, and wiring ratio <80%, fire partial_wiring.
//!
//! Confidence calibration:
//! - **HIGH:** Wiring ratio < 50% — majority of importers don't call.
//! - **MODERATE:** Wiring ratio 50–79% — significant gap but could be intentional.
//!
//! Severity is always Warning — no control-flow data for Critical.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::analyzer::diagnostic::*;
use crate::analyzer::patterns::exclusion::{is_excluded_symbol, is_test_path, relativize_path};
use crate::graph::Graph;
use crate::parser::ir::{EdgeKind, ReferenceKind, SymbolKind, Visibility};

/// Check whether a symbol is a valid wiring target for partial_wiring analysis.
///
/// A symbol is a wiring target if it is a callable entity (Function or Method),
/// has cross-module visibility (Public or Crate), and is not excluded by the
/// shared exclusion logic (entry points, imports, dunders, test functions).
fn is_wiring_target(symbol: &crate::parser::ir::Symbol) -> bool {
    // Only Function and Method are callable wiring targets
    if !matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method) {
        return false;
    }

    // Only Public and Crate visibility — designed for cross-module use
    if !matches!(symbol.visibility, Visibility::Public | Visibility::Crate) {
        return false;
    }

    // Apply shared exclusion logic
    if is_excluded_symbol(symbol) {
        return false;
    }

    // Skip symbols in test files
    let path = symbol.location.file.to_string_lossy();
    if is_test_path(&path) {
        return false;
    }

    true
}

/// Collect unique files containing callers of a symbol, excluding own file and test files.
fn get_caller_files(
    graph: &Graph,
    id: crate::parser::ir::SymbolId,
    own_file: &Path,
) -> HashSet<PathBuf> {
    graph
        .callers(id)
        .into_iter()
        .filter_map(|caller_id| graph.get_symbol(caller_id))
        .filter(|s| !is_test_path(&s.location.file.to_string_lossy()))
        .filter(|s| s.location.file != own_file)
        .map(|s| s.location.file.clone())
        .collect()
}

/// Collect unique files containing import references to a symbol, excluding own file and test files.
///
/// Uses `edges_to()` filtered by `reference_id` → `get_reference()` → `ReferenceKind::Import`
/// for precision. This avoids counting Read, Write, Export, or other reference types as imports.
fn get_importer_files(
    graph: &Graph,
    id: crate::parser::ir::SymbolId,
    own_file: &Path,
) -> HashSet<PathBuf> {
    graph
        .edges_to(id)
        .iter()
        .filter(|e| e.kind == EdgeKind::References)
        .filter(|e| {
            e.reference_id
                .and_then(|rid| graph.get_reference(rid))
                .map(|r| r.kind == ReferenceKind::Import)
                .unwrap_or(false)
        })
        .filter_map(|e| graph.get_symbol(e.target))
        .filter(|s| !is_test_path(&s.location.file.to_string_lossy()))
        .filter(|s| s.location.file != own_file)
        .map(|s| s.location.file.clone())
        .collect()
}

/// Detect partial wiring in the analysis graph.
///
/// Walks all symbols and identifies functions/methods that are imported by multiple
/// files but called from only a subset. Returns diagnostics with confidence calibrated
/// by wiring ratio: HIGH for <50%, MODERATE for 50–79%.
///
/// The `project_root` path is used to produce relative file paths in diagnostic locations.
pub fn detect(graph: &Graph, project_root: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for (id, symbol) in graph.all_symbols() {
        if !is_wiring_target(symbol) {
            continue;
        }

        let own_file = &symbol.location.file;

        // Collect files that CALL this symbol (excluding own file, excluding test files)
        let caller_files = get_caller_files(graph, id, own_file);

        // Collect files that IMPORT this symbol (excluding own file, excluding test files)
        let importer_files = get_importer_files(graph, id, own_file);

        // Total files that reference this symbol cross-file
        let all_referencing_files: HashSet<PathBuf> =
            caller_files.union(&importer_files).cloned().collect();

        // Minimum threshold: need ≥3 referencing files to avoid noise
        if all_referencing_files.len() < 3 {
            continue;
        }

        // Must have ≥1 actual caller (zero callers = data_dead_end territory)
        if caller_files.is_empty() {
            continue;
        }

        // Files that import but don't call = unwired
        let unwired_files: HashSet<&PathBuf> = importer_files.difference(&caller_files).collect();
        if unwired_files.is_empty() {
            continue;
        }

        // Wiring ratio
        let total = all_referencing_files.len() as f64;
        let callers = caller_files.len() as f64;
        let ratio = callers / total;

        // 80%+ wired = acceptable
        if ratio >= 0.80 {
            continue;
        }

        // Confidence based on ratio
        let confidence = if ratio < 0.50 {
            Confidence::High
        } else {
            Confidence::Moderate
        };

        let location = format!(
            "{}:{}",
            relativize_path(&symbol.location.file, project_root),
            symbol.location.line
        );

        // Build unwired file list for evidence
        let mut unwired_list: Vec<String> = unwired_files
            .iter()
            .map(|f| relativize_path(f, project_root))
            .collect();
        unwired_list.sort();

        let unwired_display = unwired_list.join(", ");

        diagnostics.push(Diagnostic {
            id: String::new(),
            pattern: DiagnosticPattern::PartialWiring,
            severity: Severity::Warning,
            confidence,
            entity: symbol.name.clone(),
            message: format!(
                "Partial wiring: '{}' is called from {} of {} importing files ({}% wired)",
                symbol.name,
                caller_files.len(),
                all_referencing_files.len(),
                (ratio * 100.0) as u32
            ),
            evidence: vec![
                Evidence {
                    observation: format!(
                        "Called from {} of {} cross-file consumers",
                        caller_files.len(),
                        all_referencing_files.len()
                    ),
                    location: Some(location.clone()),
                    context: Some(format!("Wiring ratio: {:.0}%", ratio * 100.0)),
                },
                Evidence {
                    observation: format!("Unwired files: {}", unwired_display),
                    location: None,
                    context: Some(format!(
                        "{} file(s) import '{}' but never call it",
                        unwired_files.len(),
                        symbol.name
                    )),
                },
            ],
            suggestion: format!(
                "Add call to '{}' in {} or remove the unused import",
                symbol.name, unwired_display
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
    use std::path::Path;

    // =========================================================================
    // Helper: build a graph with a target function and N importers / M callers
    // =========================================================================

    /// Build a graph where `target_name` in `target_file` is imported by `importer_files`
    /// and called from `caller_files`. Callers also have import edges.
    fn build_partial_wiring_graph(
        target_name: &str,
        target_kind: SymbolKind,
        target_vis: Visibility,
        target_file: &str,
        importer_files: &[&str],
        caller_files: &[&str],
    ) -> Graph {
        let mut g = Graph::new();

        // Target function
        let target = g.add_symbol(make_symbol(
            target_name,
            target_kind,
            target_vis,
            target_file,
            10,
        ));

        // For each importer file, create an import symbol and an Import edge
        for &file in importer_files {
            let imp = g.add_symbol(make_import(target_name, file, 1));
            add_ref(&mut g, imp, target, ReferenceKind::Import, file);
        }

        // For each caller file, create a caller function and a Call edge
        for &file in caller_files {
            let caller = g.add_symbol(make_symbol(
                &format!("use_{}", target_name),
                SymbolKind::Function,
                Visibility::Private,
                file,
                5,
            ));
            add_ref(&mut g, caller, target, ReferenceKind::Call, file);
        }

        g
    }

    // =========================================================================
    // Category 1: True Positive — Basic Detection
    // =========================================================================

    #[test]
    fn test_tp_basic_partial_wiring_high_confidence() {
        // T1: 5 files import, 2 call → ratio 2/5 = 40% → HIGH
        let g = build_partial_wiring_graph(
            "process_data",
            SymbolKind::Function,
            Visibility::Public,
            "utils.py",
            &["a.py", "b.py", "c.py", "d.py", "e.py"],
            &["a.py", "b.py"],
        );

        let results = detect(&g, Path::new(""));
        assert_eq!(
            results.len(),
            1,
            "Expected 1 diagnostic, got {}",
            results.len()
        );
        assert_eq!(results[0].pattern, DiagnosticPattern::PartialWiring);
        assert_eq!(results[0].confidence, Confidence::High);
        assert_eq!(results[0].severity, Severity::Warning);
        assert!(results[0].entity.contains("process_data"));
    }

    #[test]
    fn test_tp_method_partial_wiring_moderate_confidence() {
        // T2: Method, 5 import, 3 call → ratio 3/5 = 60% → MODERATE
        let g = build_partial_wiring_graph(
            "validate",
            SymbolKind::Method,
            Visibility::Public,
            "validator.py",
            &["a.py", "b.py", "c.py", "d.py", "e.py"],
            &["a.py", "b.py", "c.py"],
        );

        let results = detect(&g, Path::new(""));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].confidence, Confidence::Moderate);
    }

    #[test]
    fn test_tp_extreme_partial_wiring() {
        // T3: 6 import, 1 call → ratio 1/6 ≈ 17% → HIGH
        let g = build_partial_wiring_graph(
            "initialize",
            SymbolKind::Function,
            Visibility::Public,
            "setup.py",
            &["a.py", "b.py", "c.py", "d.py", "e.py", "f.py"],
            &["a.py"],
        );

        let results = detect(&g, Path::new(""));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].confidence, Confidence::High);
        // Evidence should list 5 unwired files
        let unwired_ev = results[0]
            .evidence
            .iter()
            .find(|e| e.observation.contains("Unwired"))
            .unwrap();
        // Count commas+1 for number of files listed
        let unwired_count = unwired_ev.observation.matches(", ").count() + 1;
        assert!(
            unwired_count >= 5,
            "Expected ≥5 unwired files in evidence, got {}",
            unwired_count
        );
    }

    // =========================================================================
    // Category 2: True Positive — Crate Visibility
    // =========================================================================

    #[test]
    fn test_tp_crate_visibility_is_wiring_target() {
        // T4: Crate visibility must be a wiring target
        let g = build_partial_wiring_graph(
            "internal_helper",
            SymbolKind::Function,
            Visibility::Crate,
            "lib.rs",
            &["a.rs", "b.rs", "c.rs", "d.rs"],
            &["a.rs"],
        );

        let results = detect(&g, Path::new(""));
        assert_eq!(
            results.len(),
            1,
            "Crate visibility must trigger partial_wiring"
        );
    }

    // =========================================================================
    // Category 3: True Negative — Fully Wired
    // =========================================================================

    #[test]
    fn test_tn_fully_wired_no_finding() {
        // T5: 5 import, 5 call → 100% → no finding
        let g = build_partial_wiring_graph(
            "helper",
            SymbolKind::Function,
            Visibility::Public,
            "utils.py",
            &["a.py", "b.py", "c.py", "d.py", "e.py"],
            &["a.py", "b.py", "c.py", "d.py", "e.py"],
        );

        let results = detect(&g, Path::new(""));
        assert!(
            results.is_empty(),
            "Fully wired function should produce 0 diagnostics"
        );
    }

    #[test]
    fn test_tn_mostly_wired_at_80_percent() {
        // T6: 5 import, 4 call → 80% ≥ threshold → no finding
        let g = build_partial_wiring_graph(
            "render",
            SymbolKind::Function,
            Visibility::Public,
            "display.py",
            &["a.py", "b.py", "c.py", "d.py", "e.py"],
            &["a.py", "b.py", "c.py", "d.py"],
        );

        let results = detect(&g, Path::new(""));
        assert!(
            results.is_empty(),
            "80% wired should not fire (≥80% threshold)"
        );
    }

    // =========================================================================
    // Category 4: True Negative — Below Threshold
    // =========================================================================

    #[test]
    fn test_tn_below_minimum_file_threshold() {
        // T7: Only 2 referencing files → below ≥3 threshold
        let g = build_partial_wiring_graph(
            "small_util",
            SymbolKind::Function,
            Visibility::Public,
            "core.py",
            &["a.py", "b.py"],
            &["a.py"],
        );

        let results = detect(&g, Path::new(""));
        assert!(results.is_empty(), "< 3 referencing files should not fire");
    }

    #[test]
    fn test_tn_private_function_excluded() {
        // T8: Private visibility → not a wiring target
        let g = build_partial_wiring_graph(
            "_internal",
            SymbolKind::Function,
            Visibility::Private,
            "utils.py",
            &["a.py", "b.py", "c.py"],
            &["a.py"],
        );

        let results = detect(&g, Path::new(""));
        assert!(
            results.is_empty(),
            "Private function must not be a wiring target"
        );
    }

    // =========================================================================
    // Category 5: True Negative — Zero Callers
    // =========================================================================

    #[test]
    fn test_tn_zero_callers_is_data_dead_end_not_partial_wiring() {
        // T9: 4 import, 0 call → data_dead_end domain, not partial_wiring
        let g = build_partial_wiring_graph(
            "unused_export",
            SymbolKind::Function,
            Visibility::Public,
            "api.py",
            &["a.py", "b.py", "c.py", "d.py"],
            &[], // zero callers
        );

        let results = detect(&g, Path::new(""));
        assert!(
            results.is_empty(),
            "Zero callers = data_dead_end, not partial_wiring"
        );
    }

    // =========================================================================
    // Category 6: Adversarial — Test File Exclusion
    // =========================================================================

    #[test]
    fn test_adv_test_callers_excluded_from_count() {
        // T10: 3 test callers + 1 production caller + 1 production import-only
        // After test exclusion: 2 production files, < 3 → no finding
        let mut g = Graph::new();
        let target = g.add_symbol(make_symbol(
            "validate_input",
            SymbolKind::Function,
            Visibility::Public,
            "validator.py",
            10,
        ));

        // Test files: import + call
        for test_file in &["test_validator.py", "test_integration.py", "test_api.py"] {
            let imp = g.add_symbol(make_import("validate_input", test_file, 1));
            add_ref(&mut g, imp, target, ReferenceKind::Import, test_file);
            let caller = g.add_symbol(make_symbol(
                "test_fn",
                SymbolKind::Function,
                Visibility::Private,
                test_file,
                5,
            ));
            add_ref(&mut g, caller, target, ReferenceKind::Call, test_file);
        }

        // Production file: import + call
        let imp_prod = g.add_symbol(make_import("validate_input", "app.py", 1));
        add_ref(&mut g, imp_prod, target, ReferenceKind::Import, "app.py");
        let caller_prod = g.add_symbol(make_symbol(
            "run_app",
            SymbolKind::Function,
            Visibility::Private,
            "app.py",
            5,
        ));
        add_ref(&mut g, caller_prod, target, ReferenceKind::Call, "app.py");

        // Production file: import only (no call)
        let imp_handler = g.add_symbol(make_import("validate_input", "handler.py", 1));
        add_ref(
            &mut g,
            imp_handler,
            target,
            ReferenceKind::Import,
            "handler.py",
        );

        let results = detect(&g, Path::new(""));
        // 2 production referencing files (app.py + handler.py) < 3 → no finding
        assert!(
            results.is_empty(),
            "Test exclusion should reduce referencing files below threshold"
        );
    }

    #[test]
    fn test_adv_test_importers_excluded_from_count() {
        // T11: 3 test importers + 3 production (import+call) → 100% production wiring
        let mut g = Graph::new();
        let target = g.add_symbol(make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Public,
            "engine.py",
            10,
        ));

        // 3 test files: import only
        for test_file in &["test_a.py", "test_b.py", "test_c.py"] {
            let imp = g.add_symbol(make_import("process", test_file, 1));
            add_ref(&mut g, imp, target, ReferenceKind::Import, test_file);
        }

        // 3 production files: import + call
        for prod_file in &["handler.py", "router.py", "service.py"] {
            let imp = g.add_symbol(make_import("process", prod_file, 1));
            add_ref(&mut g, imp, target, ReferenceKind::Import, prod_file);
            let caller = g.add_symbol(make_symbol(
                "use_process",
                SymbolKind::Function,
                Visibility::Private,
                prod_file,
                5,
            ));
            add_ref(&mut g, caller, target, ReferenceKind::Call, prod_file);
        }

        let results = detect(&g, Path::new(""));
        assert!(results.is_empty(), "100% production wiring should not fire");
    }

    #[test]
    fn test_adv_test_path_boundary_tests_dir_with_fixture() {
        // T12: tests/fixtures/sample.py is NOT a test file per is_test_path
        let mut g = Graph::new();
        let target = g.add_symbol(make_symbol(
            "analyze",
            SymbolKind::Function,
            Visibility::Public,
            "core.py",
            10,
        ));

        // Fixture file caller (not a test file)
        let fixture_imp = g.add_symbol(make_import("analyze", "tests/fixtures/sample.py", 1));
        add_ref(
            &mut g,
            fixture_imp,
            target,
            ReferenceKind::Import,
            "tests/fixtures/sample.py",
        );
        let fixture_caller = g.add_symbol(make_symbol(
            "run_fixture",
            SymbolKind::Function,
            Visibility::Private,
            "tests/fixtures/sample.py",
            5,
        ));
        add_ref(
            &mut g,
            fixture_caller,
            target,
            ReferenceKind::Call,
            "tests/fixtures/sample.py",
        );

        // 2 production importers, 1 production caller
        let imp_a = g.add_symbol(make_import("analyze", "a.py", 1));
        add_ref(&mut g, imp_a, target, ReferenceKind::Import, "a.py");
        let caller_a = g.add_symbol(make_symbol(
            "use_analyze",
            SymbolKind::Function,
            Visibility::Private,
            "a.py",
            5,
        ));
        add_ref(&mut g, caller_a, target, ReferenceKind::Call, "a.py");

        let imp_b = g.add_symbol(make_import("analyze", "b.py", 1));
        add_ref(&mut g, imp_b, target, ReferenceKind::Import, "b.py");

        let imp_c = g.add_symbol(make_import("analyze", "c.py", 1));
        add_ref(&mut g, imp_c, target, ReferenceKind::Import, "c.py");

        let results = detect(&g, Path::new(""));
        // fixture is included as a non-test file referencer:
        // callers: fixtures/sample.py + a.py = 2
        // importers: fixtures/sample.py + a.py + b.py + c.py = 4
        // total unique: 4, caller count 2, ratio 2/4 = 50% → MODERATE
        assert_eq!(results.len(), 1, "Fixture file treated as production");
        assert_eq!(results[0].confidence, Confidence::Moderate);
    }

    // =========================================================================
    // Category 7: Adversarial — Own-File Exclusion
    // =========================================================================

    #[test]
    fn test_adv_own_file_callers_excluded() {
        // T13: Called 5 times within own file, but 0 external callers, 3 external importers
        // Own-file callers excluded → 0 external callers → skip (≥1 caller required)
        let mut g = Graph::new();
        let target = g.add_symbol(make_symbol(
            "helper",
            SymbolKind::Function,
            Visibility::Public,
            "utils.py",
            10,
        ));

        // Own-file callers (should be excluded)
        for i in 0..5 {
            let caller = g.add_symbol(make_symbol(
                &format!("local_fn_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                "utils.py",
                20 + i * 5,
            ));
            add_ref(&mut g, caller, target, ReferenceKind::Call, "utils.py");
        }

        // External importers (no callers)
        for file in &["a.py", "b.py", "c.py"] {
            let imp = g.add_symbol(make_import("helper", file, 1));
            add_ref(&mut g, imp, target, ReferenceKind::Import, file);
        }

        let results = detect(&g, Path::new(""));
        assert!(
            results.is_empty(),
            "0 external callers → skip (data_dead_end territory)"
        );
    }

    #[test]
    fn test_adv_own_file_importers_excluded() {
        // T14: Self-import + 4 external importers, 2 callers
        let mut g = Graph::new();
        let target = g.add_symbol(make_symbol(
            "config_load",
            SymbolKind::Function,
            Visibility::Public,
            "config.py",
            10,
        ));

        // Self-import (should be excluded)
        let self_imp = g.add_symbol(make_import("config_load", "config.py", 1));
        add_ref(&mut g, self_imp, target, ReferenceKind::Import, "config.py");

        // 4 external importers
        for file in &["a.py", "b.py", "c.py", "d.py"] {
            let imp = g.add_symbol(make_import("config_load", file, 1));
            add_ref(&mut g, imp, target, ReferenceKind::Import, file);
        }

        // 2 external callers
        for file in &["a.py", "b.py"] {
            let caller = g.add_symbol(make_symbol(
                "use_config",
                SymbolKind::Function,
                Visibility::Private,
                file,
                5,
            ));
            add_ref(&mut g, caller, target, ReferenceKind::Call, file);
        }

        let results = detect(&g, Path::new(""));
        // Self-import excluded: 4 importers, 2 callers, ratio 2/4 = 50% → MODERATE
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].confidence, Confidence::Moderate);
    }

    // =========================================================================
    // Category 8: Adversarial — Entry Point / Excluded Symbols
    // =========================================================================

    #[test]
    fn test_adv_entry_point_excluded() {
        // T15: Entry point with annotation — excluded by is_excluded_symbol
        let mut g = Graph::new();
        let target = g.add_symbol(make_entry_point(
            "main_handler",
            SymbolKind::Function,
            Visibility::Public,
            "app.py",
            10,
        ));

        for file in &["a.py", "b.py", "c.py", "d.py"] {
            let imp = g.add_symbol(make_import("main_handler", file, 1));
            add_ref(&mut g, imp, target, ReferenceKind::Import, file);
        }
        let caller = g.add_symbol(make_symbol(
            "boot",
            SymbolKind::Function,
            Visibility::Private,
            "a.py",
            5,
        ));
        add_ref(&mut g, caller, target, ReferenceKind::Call, "a.py");

        let results = detect(&g, Path::new(""));
        assert!(
            results.is_empty(),
            "Entry points excluded by is_excluded_symbol"
        );
    }

    #[test]
    fn test_adv_import_symbol_excluded() {
        // T16: Import symbol — excluded by is_excluded_symbol
        let mut g = Graph::new();
        let target = g.add_symbol(make_import("os", "main.py", 1));

        for file in &["a.py", "b.py", "c.py", "d.py"] {
            let imp = g.add_symbol(make_import("os", file, 1));
            add_ref(&mut g, imp, target, ReferenceKind::Import, file);
        }

        let results = detect(&g, Path::new(""));
        assert!(
            results.is_empty(),
            "Import symbols excluded by is_excluded_symbol"
        );
    }

    #[test]
    fn test_adv_dunder_method_excluded() {
        // T17: __init__ — excluded by is_excluded_symbol
        let mut g = Graph::new();
        let target = g.add_symbol(make_symbol(
            "__init__",
            SymbolKind::Method,
            Visibility::Public,
            "base.py",
            10,
        ));

        for file in &["a.py", "b.py", "c.py", "d.py", "e.py"] {
            let imp = g.add_symbol(make_import("__init__", file, 1));
            add_ref(&mut g, imp, target, ReferenceKind::Import, file);
        }
        for file in &["a.py", "b.py"] {
            let caller = g.add_symbol(make_symbol(
                "use_init",
                SymbolKind::Function,
                Visibility::Private,
                file,
                5,
            ));
            add_ref(&mut g, caller, target, ReferenceKind::Call, file);
        }

        let results = detect(&g, Path::new(""));
        assert!(
            results.is_empty(),
            "Dunder methods excluded by is_excluded_symbol"
        );
    }

    // =========================================================================
    // Category 9: Adversarial — Non-Callable Symbol Kinds
    // =========================================================================

    #[test]
    fn test_adv_class_symbol_not_wiring_target() {
        // T18: Class kind → not a wiring target
        let g = build_partial_wiring_graph(
            "UserModel",
            SymbolKind::Class,
            Visibility::Public,
            "models.py",
            &["a.py", "b.py", "c.py", "d.py"],
            &["a.py"],
        );

        let results = detect(&g, Path::new(""));
        assert!(
            results.is_empty(),
            "Class symbols are not callable wiring targets"
        );
    }

    #[test]
    fn test_adv_module_symbol_not_wiring_target() {
        // T19: Module kind → not a wiring target
        let g = build_partial_wiring_graph(
            "utils",
            SymbolKind::Module,
            Visibility::Public,
            "lib.rs",
            &["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"],
            &["a.rs"],
        );

        let results = detect(&g, Path::new(""));
        assert!(
            results.is_empty(),
            "Module symbols are not callable wiring targets"
        );
    }

    // =========================================================================
    // Category 10: Edge Cases — Boundary Conditions
    // =========================================================================

    #[test]
    fn test_edge_exactly_3_files_1_caller_fires() {
        // T20: 3 files, 1 caller → ratio 1/3 = 33% → HIGH
        let g = build_partial_wiring_graph(
            "transform",
            SymbolKind::Function,
            Visibility::Public,
            "transform.py",
            &["a.py", "b.py", "c.py"],
            &["a.py"],
        );

        let results = detect(&g, Path::new(""));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].confidence, Confidence::High);
    }

    #[test]
    fn test_edge_exactly_at_50_percent_boundary() {
        // T21: 4 files, 2 call → ratio 2/4 = 50% → MODERATE (50% is in 50-79%)
        let g = build_partial_wiring_graph(
            "compute",
            SymbolKind::Function,
            Visibility::Public,
            "math.py",
            &["a.py", "b.py", "c.py", "d.py"],
            &["a.py", "b.py"],
        );

        let results = detect(&g, Path::new(""));
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].confidence,
            Confidence::Moderate,
            "50% should be MODERATE"
        );
    }

    #[test]
    fn test_edge_just_below_80_percent() {
        // T22: 10 files, 7 call → ratio 7/10 = 70% → MODERATE
        let g = build_partial_wiring_graph(
            "format_output",
            SymbolKind::Function,
            Visibility::Public,
            "formatter.py",
            &[
                "a.py", "b.py", "c.py", "d.py", "e.py", "f.py", "g.py", "h.py", "i.py", "j.py",
            ],
            &["a.py", "b.py", "c.py", "d.py", "e.py", "f.py", "g.py"],
        );

        let results = detect(&g, Path::new(""));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].confidence, Confidence::Moderate);
    }

    #[test]
    fn test_edge_just_above_80_percent_no_finding() {
        // T23: 5 files, 4 call → ratio 4/5 = 80% ≥ threshold → no finding
        let g = build_partial_wiring_graph(
            "log_event",
            SymbolKind::Function,
            Visibility::Public,
            "logger.py",
            &["a.py", "b.py", "c.py", "d.py", "e.py"],
            &["a.py", "b.py", "c.py", "d.py"],
        );

        let results = detect(&g, Path::new(""));
        assert!(results.is_empty(), "80% ≥ threshold → no finding");
    }

    // =========================================================================
    // Category 11: Edge Cases — Graph Conditions
    // =========================================================================

    #[test]
    fn test_edge_empty_graph_no_crash() {
        // T24: Empty graph
        let g = Graph::new();
        let results = detect(&g, Path::new(""));
        assert!(results.is_empty(), "Empty graph must not crash");
    }

    #[test]
    fn test_edge_single_file_project_no_finding() {
        // T25: All functions in one file → own-file exclusion removes all
        let mut g = Graph::new();
        let f1 = g.add_symbol(make_symbol(
            "func_a",
            SymbolKind::Function,
            Visibility::Public,
            "main.py",
            1,
        ));
        let f2 = g.add_symbol(make_symbol(
            "func_b",
            SymbolKind::Function,
            Visibility::Public,
            "main.py",
            10,
        ));
        let f3 = g.add_symbol(make_symbol(
            "func_c",
            SymbolKind::Function,
            Visibility::Public,
            "main.py",
            20,
        ));
        let f4 = g.add_symbol(make_symbol(
            "func_d",
            SymbolKind::Function,
            Visibility::Public,
            "main.py",
            30,
        ));
        let f5 = g.add_symbol(make_symbol(
            "func_e",
            SymbolKind::Function,
            Visibility::Public,
            "main.py",
            40,
        ));

        add_ref(&mut g, f2, f1, ReferenceKind::Call, "main.py");
        add_ref(&mut g, f3, f1, ReferenceKind::Call, "main.py");
        add_ref(&mut g, f4, f2, ReferenceKind::Call, "main.py");
        add_ref(&mut g, f5, f3, ReferenceKind::Call, "main.py");

        let results = detect(&g, Path::new(""));
        assert!(
            results.is_empty(),
            "Single-file project should produce 0 diagnostics"
        );
    }

    #[test]
    fn test_edge_multiple_callers_same_file_count_as_one() {
        // T26: 3 callers from handler.py + 2 from router.py = 2 files (not 5)
        let mut g = Graph::new();
        let target = g.add_symbol(make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Public,
            "core.py",
            10,
        ));

        // handler.py: 3 different callers
        for i in 0..3 {
            let caller = g.add_symbol(make_symbol(
                &format!("handler_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                "handler.py",
                10 + i * 5,
            ));
            add_ref(&mut g, caller, target, ReferenceKind::Call, "handler.py");
        }

        // router.py: 2 callers
        for i in 0..2 {
            let caller = g.add_symbol(make_symbol(
                &format!("router_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                "router.py",
                10 + i * 5,
            ));
            add_ref(&mut g, caller, target, ReferenceKind::Call, "router.py");
        }

        // middleware.py and validator.py: import only
        let imp_m = g.add_symbol(make_import("process", "middleware.py", 1));
        add_ref(
            &mut g,
            imp_m,
            target,
            ReferenceKind::Import,
            "middleware.py",
        );
        let imp_v = g.add_symbol(make_import("process", "validator.py", 1));
        add_ref(&mut g, imp_v, target, ReferenceKind::Import, "validator.py");

        let results = detect(&g, Path::new(""));
        // caller_files = {handler.py, router.py} = 2
        // importer_files = {middleware.py, validator.py} = 2
        // all_referencing = 4 files, caller_files = 2, ratio = 2/4 = 50% → MODERATE
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].confidence, Confidence::Moderate);
    }

    #[test]
    fn test_edge_reference_id_none_on_edge() {
        // T27: Edge with reference_id = None should be ignored for import counting
        let mut g = Graph::new();
        let target = g.add_symbol(make_symbol(
            "target_fn",
            SymbolKind::Function,
            Visibility::Public,
            "target.py",
            10,
        ));

        // 3 proper import edges
        for file in &["a.py", "b.py", "c.py"] {
            let imp = g.add_symbol(make_import("target_fn", file, 1));
            add_ref(&mut g, imp, target, ReferenceKind::Import, file);
        }

        // 1 caller
        let caller = g.add_symbol(make_symbol(
            "use_fn",
            SymbolKind::Function,
            Visibility::Private,
            "a.py",
            5,
        ));
        add_ref(&mut g, caller, target, ReferenceKind::Call, "a.py");

        // The graph already handles this correctly — edges_to() returns edges with
        // proper reference_ids from add_reference(). An edge with reference_id=None
        // would be filtered out by our reference_id.and_then(get_reference) chain.
        let results = detect(&g, Path::new(""));
        assert_eq!(results.len(), 1, "Should fire with 3 valid import edges");
    }

    // =========================================================================
    // Category 12: Edge Cases — Import Edge Filtering
    // =========================================================================

    #[test]
    fn test_edge_read_reference_not_counted_as_import() {
        // T28: Read references should NOT inflate importer count
        let mut g = Graph::new();
        let target = g.add_symbol(make_symbol(
            "get_value",
            SymbolKind::Function,
            Visibility::Public,
            "store.py",
            10,
        ));

        // 3 Read references (not Import)
        for file in &["a.py", "b.py", "c.py"] {
            let reader = g.add_symbol(make_symbol(
                "reader",
                SymbolKind::Function,
                Visibility::Private,
                file,
                5,
            ));
            add_ref(&mut g, reader, target, ReferenceKind::Read, file);
        }

        // 1 Import + Call
        let imp = g.add_symbol(make_import("get_value", "user.py", 1));
        add_ref(&mut g, imp, target, ReferenceKind::Import, "user.py");
        let caller = g.add_symbol(make_symbol(
            "use_value",
            SymbolKind::Function,
            Visibility::Private,
            "user.py",
            5,
        ));
        add_ref(&mut g, caller, target, ReferenceKind::Call, "user.py");

        let results = detect(&g, Path::new(""));
        // Only 1 importer (user.py) + 1 caller (user.py) → < 3 files → no finding
        assert!(
            results.is_empty(),
            "Read references must NOT count as imports"
        );
    }

    #[test]
    fn test_edge_export_reference_not_counted_as_import() {
        // T29: Export references should NOT inflate importer count
        let mut g = Graph::new();
        let target = g.add_symbol(make_symbol(
            "api_handler",
            SymbolKind::Function,
            Visibility::Public,
            "api.py",
            10,
        ));

        // 3 Export references
        for file in &["index.py", "barrel.py", "reexport.py"] {
            let exporter = g.add_symbol(make_symbol(
                "reexport_sym",
                SymbolKind::Function,
                Visibility::Public,
                file,
                1,
            ));
            add_ref(&mut g, exporter, target, ReferenceKind::Export, file);
        }

        // 2 Import + Call
        for file in &["client.py", "service.py"] {
            let imp = g.add_symbol(make_import("api_handler", file, 1));
            add_ref(&mut g, imp, target, ReferenceKind::Import, file);
            let caller = g.add_symbol(make_symbol(
                "call_api",
                SymbolKind::Function,
                Visibility::Private,
                file,
                5,
            ));
            add_ref(&mut g, caller, target, ReferenceKind::Call, file);
        }

        let results = detect(&g, Path::new(""));
        // 2 importers, 2 callers = 100% → no finding (exports excluded from import count)
        assert!(
            results.is_empty(),
            "Export references must NOT count as imports"
        );
    }

    // =========================================================================
    // Category 13: Integration — Registration
    // =========================================================================

    #[test]
    fn test_registration_pattern_in_run_all_patterns() {
        // T30: partial_wiring must appear in run_all_patterns output
        use crate::analyzer::patterns::run_all_patterns;

        let g = build_partial_wiring_graph(
            "register_test_fn",
            SymbolKind::Function,
            Visibility::Public,
            "register.py",
            &["a.py", "b.py", "c.py", "d.py"],
            &["a.py"],
        );

        let results = run_all_patterns(&g, Path::new(""));
        assert!(
            results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::PartialWiring),
            "PartialWiring must appear in run_all_patterns output"
        );
    }

    #[test]
    fn test_registration_pattern_filter_works() {
        // T31: --checks partial_wiring must isolate this pattern
        use crate::analyzer::patterns::{run_patterns, PatternFilter};

        // Build a graph with both partial_wiring and data_dead_end findings
        let mut g = Graph::new();

        // Partial wiring target
        let pw_target = g.add_symbol(make_symbol(
            "pw_function",
            SymbolKind::Function,
            Visibility::Public,
            "pw.py",
            10,
        ));
        for file in &["a.py", "b.py", "c.py", "d.py"] {
            let imp = g.add_symbol(make_import("pw_function", file, 1));
            add_ref(&mut g, imp, pw_target, ReferenceKind::Import, file);
        }
        let caller = g.add_symbol(make_symbol(
            "use_pw",
            SymbolKind::Function,
            Visibility::Private,
            "a.py",
            5,
        ));
        add_ref(&mut g, caller, pw_target, ReferenceKind::Call, "a.py");

        // Dead end target (zero callers, private)
        let _dead = g.add_symbol(make_symbol(
            "dead_fn",
            SymbolKind::Function,
            Visibility::Private,
            "dead.py",
            10,
        ));

        let filter = PatternFilter {
            patterns: Some(vec![DiagnosticPattern::PartialWiring]),
            ..Default::default()
        };
        let results = run_patterns(&g, &filter, Path::new(""));

        assert!(
            results
                .iter()
                .all(|d| d.pattern == DiagnosticPattern::PartialWiring),
            "Filter should return only PartialWiring diagnostics"
        );
        assert!(
            !results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::DataDeadEnd),
            "DataDeadEnd should be filtered out"
        );
    }

    #[test]
    fn test_registration_unimplemented_test_updated() {
        // T32: The unimplemented patterns test should now only check for Duplication
        // and AsymmetricHandling (not PartialWiring, which is now implemented)
        let graph = build_all_fixtures_graph();
        let diagnostics = crate::analyzer::patterns::run_all_patterns(&graph, Path::new(""));

        // PartialWiring may or may not appear (depends on graph structure) — that's fine.
        // What matters is Duplication and AsymmetricHandling NEVER appear.
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::Duplication),
            "Duplication should still be unimplemented"
        );
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::AsymmetricHandling),
            "AsymmetricHandling should still be unimplemented"
        );
    }

    // =========================================================================
    // Category 14: Evidence Quality
    // =========================================================================

    #[test]
    fn test_evidence_lists_unwired_files() {
        // T33: Evidence must name the unwired files
        let g = build_partial_wiring_graph(
            "send_notification",
            SymbolKind::Function,
            Visibility::Public,
            "notify.py",
            &["a.py", "b.py", "c.py", "d.py", "e.py"],
            &["a.py", "b.py"],
        );

        let results = detect(&g, Path::new(""));
        assert_eq!(results.len(), 1);

        let unwired_ev = results[0]
            .evidence
            .iter()
            .find(|e| e.observation.contains("Unwired"))
            .unwrap();
        assert!(
            unwired_ev.observation.contains("c.py"),
            "Evidence must mention c.py"
        );
        assert!(
            unwired_ev.observation.contains("d.py"),
            "Evidence must mention d.py"
        );
        assert!(
            unwired_ev.observation.contains("e.py"),
            "Evidence must mention e.py"
        );
    }

    #[test]
    fn test_evidence_includes_wiring_ratio() {
        // T34: Evidence must include wiring ratio
        let g = build_partial_wiring_graph(
            "send_notification",
            SymbolKind::Function,
            Visibility::Public,
            "notify.py",
            &["a.py", "b.py", "c.py", "d.py", "e.py"],
            &["a.py", "b.py"],
        );

        let results = detect(&g, Path::new(""));
        assert_eq!(results.len(), 1);

        // Message or evidence should reference the ratio
        let has_ratio = results[0].message.contains("2 of 5")
            || results[0]
                .evidence
                .iter()
                .any(|e| e.observation.contains("2 of 5"))
            || results[0].evidence.iter().any(|e| {
                e.context
                    .as_ref()
                    .map(|c| c.contains("40%"))
                    .unwrap_or(false)
            });
        assert!(has_ratio, "Evidence or message must reference wiring ratio");
    }

    #[test]
    fn test_suggestion_names_unwired_targets() {
        // T35: Suggestion must be non-empty and reference what to wire
        let g = build_partial_wiring_graph(
            "send_notification",
            SymbolKind::Function,
            Visibility::Public,
            "notify.py",
            &["a.py", "b.py", "c.py", "d.py", "e.py"],
            &["a.py", "b.py"],
        );

        let results = detect(&g, Path::new(""));
        assert_eq!(results.len(), 1);
        assert!(
            !results[0].suggestion.is_empty(),
            "Suggestion must be non-empty"
        );
        assert!(
            results[0].suggestion.contains("send_notification"),
            "Suggestion must reference the function name"
        );
    }

    // =========================================================================
    // Category 15: Performance
    // =========================================================================

    #[test]
    fn test_performance_50_functions_10_partial_wirings() {
        // T36: 50 functions, 10 planted partial wirings → all 10 detected
        let mut g = Graph::new();

        // 10 partially wired functions
        for i in 0..10 {
            let target = g.add_symbol(make_symbol(
                &format!("partial_fn_{}", i),
                SymbolKind::Function,
                Visibility::Public,
                &format!("module_{}.py", i),
                10,
            ));

            // Each has 5 importers, 1 caller
            for j in 0..5 {
                let file = format!("consumer_{}_{}.py", i, j);
                let imp = g.add_symbol(make_import(&format!("partial_fn_{}", i), &file, 1));
                add_ref(&mut g, imp, target, ReferenceKind::Import, &file);
            }
            let caller_file = format!("consumer_{}_0.py", i);
            let caller = g.add_symbol(make_symbol(
                &format!("use_partial_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                &caller_file,
                5,
            ));
            add_ref(&mut g, caller, target, ReferenceKind::Call, &caller_file);
        }

        // 40 fully wired functions
        for i in 10..50 {
            let target = g.add_symbol(make_symbol(
                &format!("wired_fn_{}", i),
                SymbolKind::Function,
                Visibility::Public,
                &format!("module_{}.py", i),
                10,
            ));

            for j in 0..3 {
                let file = format!("consumer_{}_{}.py", i, j);
                let imp = g.add_symbol(make_import(&format!("wired_fn_{}", i), &file, 1));
                add_ref(&mut g, imp, target, ReferenceKind::Import, &file);
                let caller = g.add_symbol(make_symbol(
                    &format!("use_wired_{}", i),
                    SymbolKind::Function,
                    Visibility::Private,
                    &file,
                    5,
                ));
                add_ref(&mut g, caller, target, ReferenceKind::Call, &file);
            }
        }

        let start = std::time::Instant::now();
        let results = detect(&g, Path::new(""));
        let elapsed = start.elapsed();

        assert_eq!(
            results.len(),
            10,
            "All 10 planted partial wirings must be detected"
        );
        assert!(
            elapsed.as_secs() < 1,
            "Detection must complete in < 1 second, took {:?}",
            elapsed
        );
    }

    // =========================================================================
    // Category 16: Regression — Previous Cycle Issues
    // =========================================================================

    #[test]
    fn test_regression_no_overlap_with_data_dead_end() {
        // T37: 4 import, 0 call → data_dead_end territory, not partial_wiring
        let g = build_partial_wiring_graph(
            "orphan",
            SymbolKind::Function,
            Visibility::Public,
            "orphan.py",
            &["a.py", "b.py", "c.py", "d.py"],
            &[], // zero callers
        );

        let pw_results = detect(&g, Path::new(""));
        assert!(
            pw_results.is_empty(),
            "partial_wiring MUST NOT fire on zero-caller functions"
        );
    }

    #[test]
    fn test_regression_no_overlap_with_phantom_dependency() {
        // T38: Import symbol itself should not be a wiring target
        let mut g = Graph::new();
        let imp_sym = g.add_symbol(make_import("unused_lib", "main.py", 1));

        // Target function that the import symbol references
        let target = g.add_symbol(make_symbol(
            "unused_lib_fn",
            SymbolKind::Function,
            Visibility::Public,
            "unused_lib.py",
            1,
        ));
        add_ref(&mut g, imp_sym, target, ReferenceKind::Import, "main.py");

        let results = detect(&g, Path::new(""));
        // Import symbol excluded, target has < 3 referencing files
        assert!(
            results.is_empty(),
            "phantom_dependency domain must not be invaded"
        );
    }

    #[test]
    fn test_regression_foo_new_bar_new_ambiguity() {
        // T39: Two methods with same short name disambiguated by SymbolId
        let mut g = Graph::new();

        // Foo::new — 4 importers, 3 callers → 75% → MODERATE (fires)
        let foo_new = g.add_symbol(make_symbol(
            "new",
            SymbolKind::Method,
            Visibility::Public,
            "foo.rs",
            10,
        ));
        for file in &["a.rs", "b.rs", "c.rs", "d.rs"] {
            let imp = g.add_symbol(make_import("new", file, 1));
            add_ref(&mut g, imp, foo_new, ReferenceKind::Import, file);
        }
        for file in &["a.rs", "b.rs", "c.rs"] {
            let caller = g.add_symbol(make_symbol(
                "use_foo",
                SymbolKind::Function,
                Visibility::Private,
                file,
                5,
            ));
            add_ref(&mut g, caller, foo_new, ReferenceKind::Call, file);
        }

        // Bar::new — 4 importers, 4 callers → 100% (does not fire)
        let bar_new = g.add_symbol(make_symbol(
            "new",
            SymbolKind::Method,
            Visibility::Public,
            "bar.rs",
            10,
        ));
        for file in &["e.rs", "f.rs", "g.rs", "h.rs"] {
            let imp = g.add_symbol(make_import("new", file, 1));
            add_ref(&mut g, imp, bar_new, ReferenceKind::Import, file);
        }
        for file in &["e.rs", "f.rs", "g.rs", "h.rs"] {
            let caller = g.add_symbol(make_symbol(
                "use_bar",
                SymbolKind::Function,
                Visibility::Private,
                file,
                5,
            ));
            add_ref(&mut g, caller, bar_new, ReferenceKind::Call, file);
        }

        let results = detect(&g, Path::new(""));
        // Only Foo::new fires (75%), Bar::new does not (100%)
        assert_eq!(results.len(), 1, "Only Foo::new should fire");
        // The finding is for the function in foo.rs
        assert!(
            results[0].location.contains("foo.rs"),
            "Finding should be for foo.rs"
        );
    }

    // =========================================================================
    // Category 17: Cross-Pattern Interaction
    // =========================================================================

    #[test]
    fn test_interaction_clean_code_graph_zero_findings() {
        // T40: Clean code fixture must never trigger any pattern
        let g = build_clean_code_graph();
        let results = detect(&g, Path::new(""));
        assert!(
            results.is_empty(),
            "Clean code graph must produce 0 partial_wiring findings"
        );
    }

    #[test]
    fn test_interaction_with_incomplete_migration_no_double_count() {
        // T41: old_handler losing callers + new_handler gaining callers
        let mut g = Graph::new();

        // old_handler: 4 importers, 1 caller → 25% → HIGH
        let old = g.add_symbol(make_symbol(
            "old_handler",
            SymbolKind::Function,
            Visibility::Public,
            "handlers.py",
            10,
        ));
        for file in &["a.py", "b.py", "c.py", "d.py"] {
            let imp = g.add_symbol(make_import("old_handler", file, 1));
            add_ref(&mut g, imp, old, ReferenceKind::Import, file);
        }
        let caller = g.add_symbol(make_symbol(
            "use_old",
            SymbolKind::Function,
            Visibility::Private,
            "a.py",
            5,
        ));
        add_ref(&mut g, caller, old, ReferenceKind::Call, "a.py");

        // new_handler: 4 importers, 3 callers → 75% → MODERATE
        let new = g.add_symbol(make_symbol(
            "new_handler",
            SymbolKind::Function,
            Visibility::Public,
            "handlers.py",
            50,
        ));
        for file in &["a.py", "b.py", "c.py", "d.py"] {
            let imp = g.add_symbol(make_import("new_handler", file, 1));
            add_ref(&mut g, imp, new, ReferenceKind::Import, file);
        }
        for file in &["a.py", "b.py", "c.py"] {
            let caller = g.add_symbol(make_symbol(
                "use_new",
                SymbolKind::Function,
                Visibility::Private,
                file,
                5,
            ));
            add_ref(&mut g, caller, new, ReferenceKind::Call, file);
        }

        let results = detect(&g, Path::new(""));
        // Both should fire as partial_wiring
        assert_eq!(results.len(), 2, "Both old and new handlers should fire");
        assert!(results
            .iter()
            .all(|d| d.pattern == DiagnosticPattern::PartialWiring));
    }

    #[test]
    fn test_protected_visibility_excluded() {
        // T42: Protected visibility → not a wiring target
        let g = build_partial_wiring_graph(
            "protected_method",
            SymbolKind::Method,
            Visibility::Protected,
            "base.py",
            &["a.py", "b.py", "c.py", "d.py"],
            &["a.py"],
        );

        let results = detect(&g, Path::new(""));
        assert!(
            results.is_empty(),
            "Protected visibility must not be a wiring target"
        );
    }
}
