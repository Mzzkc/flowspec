// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Entity data extraction from the analysis graph.
//!
//! Standalone query functions that extract manifest-ready data from a
//! populated Graph. These replace the hardcoded placeholders from cycle 1's
//! text scanner (`vis: "pub"`, `called_by: ["(detected)"]`).

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::graph::Graph;
use crate::parser::ir::{Symbol, SymbolId, SymbolKind, Visibility};

/// Direction of a module-level dependency edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyDirection {
    /// Only one module references the other.
    Unidirectional,
    /// Both modules reference each other.
    Bidirectional,
}

/// A module-level dependency edge for the manifest's `dependency_graph` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleDependency {
    /// Source module file path.
    pub from: String,
    /// Target module file path.
    pub to: String,
    /// Number of cross-file references.
    pub weight: usize,
    /// Whether the dependency is unidirectional or bidirectional.
    pub direction: DependencyDirection,
    /// Related diagnostic IDs (e.g., circular dependency findings).
    pub issues: Vec<String>,
}

/// Extracts module-level dependency edges from cross-file references in the graph.
///
/// Walks all symbols, checks outgoing edges for cross-file targets, and aggregates
/// by file pair. Bidirectional edges (A→B and B→A both exist) are merged into a
/// single entry with combined weight. The `from` field gets the lexicographically
/// smaller path for deterministic ordering.
pub fn extract_dependency_graph(graph: &Graph) -> Vec<ModuleDependency> {
    // Count cross-file edges per (source_file, target_file) pair
    let mut edge_counts: HashMap<(String, String), usize> = HashMap::new();

    for (sym_id, symbol) in graph.all_symbols() {
        let source_file = symbol.location.file.to_string_lossy().to_string();

        for edge in graph.edges_from(sym_id) {
            // Skip unresolved edges (SymbolId::default())
            if edge.target == SymbolId::default() {
                continue;
            }

            if let Some(target_sym) = graph.get_symbol(edge.target) {
                let target_file = target_sym.location.file.to_string_lossy().to_string();

                // Only count cross-file edges
                if source_file != target_file {
                    *edge_counts
                        .entry((source_file.clone(), target_file))
                        .or_insert(0) += 1;
                }
            }
        }
    }

    // Merge bidirectional edges: if both (A→B) and (B→A) exist, combine them
    let mut merged: HashMap<(String, String), ModuleDependency> = HashMap::new();

    for ((from, to), count) in edge_counts {
        // Canonical key: lexicographically smaller path first
        let (canonical_from, canonical_to) = if from <= to {
            (from.clone(), to.clone())
        } else {
            (to.clone(), from.clone())
        };

        let key = (canonical_from.clone(), canonical_to.clone());

        let entry = merged.entry(key).or_insert_with(|| ModuleDependency {
            from: canonical_from,
            to: canonical_to,
            weight: 0,
            direction: DependencyDirection::Unidirectional,
            issues: vec![],
        });

        entry.weight += count;

        // If the original (from, to) doesn't match canonical order,
        // it means we have a reverse edge → bidirectional
        if from > to {
            entry.direction = DependencyDirection::Bidirectional;
        }
    }

    let mut result: Vec<ModuleDependency> = merged.into_values().collect();
    result.sort_by(|a, b| a.from.cmp(&b.from).then(a.to.cmp(&b.to)));
    result
}

/// Extracts the qualified names of all symbols called by the given symbol.
///
/// Uses `graph.callees()` which returns outgoing edges with `EdgeKind::Calls`.
/// Filters `SymbolId::default()` phantom entries (unresolved cross-file
/// references) and symbols whose IDs cannot be resolved (defensive).
pub fn extract_calls(graph: &Graph, symbol_id: SymbolId) -> Vec<String> {
    graph
        .callees(symbol_id)
        .iter()
        .filter(|&&callee_id| callee_id != SymbolId::default())
        .filter_map(|&callee_id| {
            graph
                .get_symbol(callee_id)
                .map(|s| s.qualified_name.clone())
        })
        .collect()
}

/// Extracts the qualified names of all symbols that call the given symbol.
///
/// Uses `graph.callers()` which returns incoming edges with `EdgeKind::Calls`.
/// Filters `SymbolId::default()` phantom entries (unresolved cross-file
/// references) and symbols whose IDs cannot be resolved (defensive).
pub fn extract_called_by(graph: &Graph, symbol_id: SymbolId) -> Vec<String> {
    graph
        .callers(symbol_id)
        .iter()
        .filter(|&&caller_id| caller_id != SymbolId::default())
        .filter_map(|&caller_id| {
            graph
                .get_symbol(caller_id)
                .map(|s| s.qualified_name.clone())
        })
        .collect()
}

/// Extracts a manifest-ready visibility string from a Symbol.
///
/// Mapping:
/// - `Public` → `"pub"`
/// - `Private` → `"priv"`
/// - `Crate` → `"crate"`
/// - `Protected` → `"protected"`
pub fn extract_visibility(symbol: &Symbol) -> String {
    match symbol.visibility {
        Visibility::Public => "pub".to_string(),
        Visibility::Private => "priv".to_string(),
        Visibility::Crate => "crate".to_string(),
        Visibility::Protected => "protected".to_string(),
    }
}

/// Infers a meaningful role description for a module based on its symbols.
///
/// Heuristics (checked in priority order):
/// 1. Has `main` or `__main__` → "Entry point module"
/// 2. Test file or test functions → "Test module"
/// 3. Mostly classes/structs → "Data model module"
/// 4. Has import re-exports → "API boundary module"
/// 5. Mostly functions → "Utility module"
/// 6. Mixed → "Service module"
/// 7. Empty → "Empty module"
pub fn infer_module_role(graph: &Graph, file_path: &Path) -> String {
    let symbol_ids = graph.symbols_in_file(file_path);

    if symbol_ids.is_empty() {
        return "Empty module".to_string();
    }

    let symbols: Vec<&Symbol> = symbol_ids
        .iter()
        .filter_map(|&id| graph.get_symbol(id))
        .collect();

    if symbols.is_empty() {
        return "Empty module".to_string();
    }

    // Check for entry point
    let has_main = symbols.iter().any(|s| {
        s.name == "main"
            || s.name == "__main__"
            || s.annotations.contains(&"entry_point".to_string())
    });
    if has_main {
        return "Entry point module".to_string();
    }

    // Check for test module (by file path or function names)
    let file_str = file_path.to_string_lossy();
    let is_test_file = crate::analyzer::patterns::exclusion::is_test_path(&file_str);
    let test_fn_count = symbols
        .iter()
        .filter(|s| {
            s.name.starts_with("test_")
                && (s.kind == SymbolKind::Function || s.kind == SymbolKind::Method)
        })
        .count();
    if is_test_file || test_fn_count > 0 {
        return "Test module".to_string();
    }

    // Count symbol kinds (exclude Module and Method symbols from counts).
    // Methods belong to classes and should not dilute the class ratio.
    let non_module_symbols: Vec<&&Symbol> = symbols
        .iter()
        .filter(|s| s.kind != SymbolKind::Module && s.kind != SymbolKind::Method)
        .collect();

    if non_module_symbols.is_empty() {
        return "Empty module".to_string();
    }

    let class_count = non_module_symbols
        .iter()
        .filter(|s| {
            matches!(
                s.kind,
                SymbolKind::Class | SymbolKind::Struct | SymbolKind::Enum
            )
        })
        .count();

    let fn_count = non_module_symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Function)
        .count();

    let import_count = non_module_symbols
        .iter()
        .filter(|s| s.annotations.contains(&"import".to_string()))
        .count();

    let total = non_module_symbols.len();

    // Mostly classes → data model
    if class_count > 0 && class_count * 2 >= total {
        return "Data model module".to_string();
    }

    // Mostly imports → API boundary
    if import_count > 0 && import_count * 2 >= total {
        return "API boundary module".to_string();
    }

    // Mostly functions → utility
    if fn_count > 0 && fn_count * 2 >= total {
        return "Utility module".to_string();
    }

    // Mix of things
    "Service module".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;

    // =========================================================================
    // 2. Entity Data Extraction
    // =========================================================================

    // -- 2.1 extract_calls with known edges --

    #[test]
    fn test_extract_calls_returns_callee_names() {
        let mut g = Graph::new();
        let caller = g.add_symbol(make_symbol(
            "caller_fn",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        ));
        let callee1 = g.add_symbol(make_symbol(
            "callee_one",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            5,
        ));
        let callee2 = g.add_symbol(make_symbol(
            "callee_two",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            9,
        ));
        add_ref(
            &mut g,
            caller,
            callee1,
            crate::parser::ir::ReferenceKind::Call,
            "a.py",
        );
        add_ref(
            &mut g,
            caller,
            callee2,
            crate::parser::ir::ReferenceKind::Call,
            "a.py",
        );

        let calls = extract_calls(&g, caller);
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().any(|c| c.contains("callee_one")));
        assert!(calls.iter().any(|c| c.contains("callee_two")));
    }

    // -- 2.2 extract_calls zero callees --

    #[test]
    fn test_extract_calls_returns_empty_for_leaf_function() {
        let mut g = Graph::new();
        let leaf = g.add_symbol(make_symbol(
            "leaf_fn",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        ));
        let calls = extract_calls(&g, leaf);
        assert!(calls.is_empty(), "Leaf function should have zero calls");
    }

    // -- 2.3 recursive self-call --

    #[test]
    fn test_extract_calls_handles_recursive_function() {
        let mut g = Graph::new();
        let recursive = g.add_symbol(make_symbol(
            "factorial",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        ));
        add_ref(
            &mut g,
            recursive,
            recursive,
            crate::parser::ir::ReferenceKind::Call,
            "a.py",
        );

        let calls = extract_calls(&g, recursive);
        assert!(
            calls.iter().any(|c| c.contains("factorial")),
            "Recursive call should appear in calls list"
        );
    }

    // -- 2.4 extract_called_by with known edges --

    #[test]
    fn test_extract_called_by_returns_caller_names() {
        let mut g = Graph::new();
        let target = g.add_symbol(make_symbol(
            "target_fn",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        ));
        let caller1 = g.add_symbol(make_symbol(
            "caller_one",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            5,
        ));
        let caller2 = g.add_symbol(make_symbol(
            "caller_two",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            9,
        ));
        add_ref(
            &mut g,
            caller1,
            target,
            crate::parser::ir::ReferenceKind::Call,
            "a.py",
        );
        add_ref(
            &mut g,
            caller2,
            target,
            crate::parser::ir::ReferenceKind::Call,
            "a.py",
        );

        let called_by = extract_called_by(&g, target);
        assert_eq!(called_by.len(), 2);
        assert!(called_by.iter().any(|c| c.contains("caller_one")));
        assert!(called_by.iter().any(|c| c.contains("caller_two")));
    }

    // -- 2.5 extract_called_by zero callers --

    #[test]
    fn test_extract_called_by_returns_empty_for_uncalled_function() {
        let mut g = Graph::new();
        let _uncalled = g.add_symbol(make_symbol(
            "orphan_fn",
            SymbolKind::Function,
            Visibility::Private,
            "a.py",
            1,
        ));
        let called_by = extract_called_by(&g, _uncalled);
        assert!(called_by.is_empty());
    }

    // -- 2.6 Direction correctness --

    #[test]
    fn test_calls_and_called_by_are_inverses() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol(
            "func_a",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        ));
        let b = g.add_symbol(make_symbol(
            "func_b",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            5,
        ));
        // A calls B (A -> B)
        add_ref(&mut g, a, b, crate::parser::ir::ReferenceKind::Call, "a.py");

        let a_calls = extract_calls(&g, a);
        let a_called_by = extract_called_by(&g, a);
        let b_calls = extract_calls(&g, b);
        let b_called_by = extract_called_by(&g, b);

        // A calls B, so A.calls contains B
        assert!(
            a_calls.iter().any(|c| c.contains("func_b")),
            "A should call B"
        );
        // B is called by A, so B.called_by contains A
        assert!(
            b_called_by.iter().any(|c| c.contains("func_a")),
            "B should be called by A"
        );
        // A is NOT called by B
        assert!(
            a_called_by.is_empty() || !a_called_by.iter().any(|c| c.contains("func_b")),
            "A should NOT be called by B"
        );
        // B does NOT call A
        assert!(
            b_calls.is_empty() || !b_calls.iter().any(|c| c.contains("func_a")),
            "B should NOT call A"
        );
    }

    // -- 2.7 extract_visibility all variants --

    #[test]
    fn test_extract_visibility_all_variants() {
        let variants = vec![
            (Visibility::Public, "pub"),
            (Visibility::Private, "priv"),
            (Visibility::Crate, "crate"),
            (Visibility::Protected, "protected"),
        ];
        for (vis, expected) in variants {
            let sym = make_symbol("test_fn", SymbolKind::Function, vis, "a.py", 1);
            let result = extract_visibility(&sym);
            assert_eq!(
                result, expected,
                "Visibility::{:?} should produce '{}', got '{}'",
                vis, expected, result
            );
        }
    }

    // -- 2.8 Python private convention --

    #[test]
    fn test_python_underscore_private_produces_priv() {
        let mut sym = make_symbol(
            "_helper",
            SymbolKind::Function,
            Visibility::Private,
            "a.py",
            1,
        );
        sym.visibility = Visibility::Private; // PythonAdapter would set this
        let result = extract_visibility(&sym);
        assert_eq!(result, "priv", "Python _name should map to 'priv'");
    }

    // -- 2.9 Nonexistent symbol ID --

    #[test]
    fn test_extract_calls_with_nonexistent_symbol_id() {
        let g = Graph::new();
        let calls = extract_calls(&g, SymbolId::default());
        assert!(
            calls.is_empty(),
            "Nonexistent symbol should return empty Vec"
        );
    }

    #[test]
    fn test_extract_called_by_with_nonexistent_symbol_id() {
        let g = Graph::new();
        let called_by = extract_called_by(&g, SymbolId::default());
        assert!(
            called_by.is_empty(),
            "Nonexistent symbol should return empty Vec"
        );
    }

    // =========================================================================
    // 3. Module Role Inference
    // =========================================================================

    // -- 3.1 Entry point module --

    #[test]
    fn test_module_with_main_is_entry_point() {
        let mut g = Graph::new();
        let _main = g.add_symbol(make_entry_point(
            "main",
            SymbolKind::Function,
            Visibility::Public,
            "app.py",
            1,
        ));
        let _helper = g.add_symbol(make_symbol(
            "helper",
            SymbolKind::Function,
            Visibility::Private,
            "app.py",
            5,
        ));

        let role = infer_module_role(&g, Path::new("app.py"));
        assert!(
            role.to_lowercase().contains("entry point"),
            "Module with main should be entry point, got: '{}'",
            role
        );
    }

    // -- 3.2 Data model module --

    #[test]
    fn test_module_with_only_classes_is_data_model() {
        let mut g = Graph::new();
        for (name, line) in [("User", 1), ("Order", 10), ("Product", 20)] {
            g.add_symbol(make_symbol(
                name,
                SymbolKind::Class,
                Visibility::Public,
                "models.py",
                line,
            ));
        }

        let role = infer_module_role(&g, Path::new("models.py"));
        let role_lower = role.to_lowercase();
        assert!(
            role_lower.contains("model")
                || role_lower.contains("data")
                || role_lower.contains("class"),
            "Module with only classes should reflect data/model nature, got: '{}'",
            role
        );
    }

    // -- 3.3 Utility module --

    #[test]
    fn test_module_with_only_functions_is_utility() {
        let mut g = Graph::new();
        for (name, line) in [("parse_date", 1), ("format_name", 5), ("clean_text", 9)] {
            g.add_symbol(make_symbol(
                name,
                SymbolKind::Function,
                Visibility::Public,
                "utils.py",
                line,
            ));
        }

        let role = infer_module_role(&g, Path::new("utils.py"));
        let role_lower = role.to_lowercase();
        assert!(
            role_lower.contains("utility")
                || role_lower.contains("helper")
                || role_lower.contains("function"),
            "Module with only functions should reflect utility nature, got: '{}'",
            role
        );
    }

    // -- 3.4 Test module --

    #[test]
    fn test_module_with_test_functions_is_test() {
        let mut g = Graph::new();
        for (name, line) in [("test_create", 1), ("test_delete", 5)] {
            g.add_symbol(make_symbol(
                name,
                SymbolKind::Function,
                Visibility::Private,
                "test_api.py",
                line,
            ));
        }

        let role = infer_module_role(&g, Path::new("test_api.py"));
        assert!(
            role.to_lowercase().contains("test"),
            "Module with test functions should be test module, got: '{}'",
            role
        );
    }

    // -- 3.5 Empty file --

    #[test]
    fn test_empty_file_produces_meaningful_role() {
        let g = Graph::new();
        let role = infer_module_role(&g, Path::new("empty.py"));
        assert!(
            !role.is_empty(),
            "Empty file should produce non-empty role string"
        );
    }

    // -- 3.6 Mixed file --

    #[test]
    fn test_mixed_module_captures_dominant_characteristic() {
        let mut g = Graph::new();
        g.add_symbol(make_symbol(
            "Config",
            SymbolKind::Class,
            Visibility::Public,
            "service.py",
            1,
        ));
        g.add_symbol(make_symbol(
            "run_server",
            SymbolKind::Function,
            Visibility::Public,
            "service.py",
            10,
        ));
        g.add_symbol(make_symbol(
            "handle_request",
            SymbolKind::Function,
            Visibility::Public,
            "service.py",
            20,
        ));
        g.add_symbol(make_symbol(
            "validate",
            SymbolKind::Function,
            Visibility::Private,
            "service.py",
            30,
        ));

        let role = infer_module_role(&g, Path::new("service.py"));
        assert!(
            !role.is_empty(),
            "Mixed module should produce non-empty role"
        );
        // Must NOT be vacuous
        assert!(
            !role.contains("Module with"),
            "Role must NOT be vacuous 'Module with N entities', got: '{}'",
            role
        );
    }

    // -- 3.7 Vacuous regression guard --

    #[test]
    fn test_module_role_is_not_vacuous_count_string() {
        let mut g = Graph::new();
        let _ = g.add_symbol(make_symbol(
            "func_a",
            SymbolKind::Function,
            Visibility::Public,
            "mod.py",
            1,
        ));
        let _ = g.add_symbol(make_symbol(
            "func_b",
            SymbolKind::Function,
            Visibility::Public,
            "mod.py",
            5,
        ));

        let role = infer_module_role(&g, Path::new("mod.py"));
        // Regex check for "Module with N entities" pattern
        assert!(
            !role.contains("Module with"),
            "Module role must NOT be vacuous. Got: '{}'",
            role
        );
        assert!(!role.is_empty(), "Module role must not be empty");
    }

    // -- Edge endpoint validation: SymbolId::default() filtering --

    #[test]
    fn test_extract_calls_filters_default_symbol_id() {
        let mut g = Graph::new();
        let caller = g.add_symbol(make_symbol(
            "do_work",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        ));
        let real_callee = g.add_symbol(make_symbol(
            "helper",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            5,
        ));

        // Real edge: caller → helper
        add_ref(
            &mut g,
            caller,
            real_callee,
            crate::parser::ir::ReferenceKind::Call,
            "a.py",
        );
        // Phantom edge: caller → SymbolId::default() (unresolved cross-file)
        add_ref(
            &mut g,
            caller,
            SymbolId::default(),
            crate::parser::ir::ReferenceKind::Call,
            "a.py",
        );

        let calls = extract_calls(&g, caller);

        assert!(
            calls.iter().any(|c| c.contains("helper")),
            "Real callee 'helper' must appear in calls"
        );
        assert_eq!(
            calls.len(),
            1,
            "Phantom SymbolId::default() must be filtered — expected 1 call, got {}",
            calls.len()
        );
    }

    #[test]
    fn test_extract_called_by_filters_default_symbol_id() {
        let mut g = Graph::new();
        let target = g.add_symbol(make_symbol(
            "target_fn",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        ));
        let real_caller = g.add_symbol(make_symbol(
            "caller_fn",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            5,
        ));

        // Real edge: caller → target
        add_ref(
            &mut g,
            real_caller,
            target,
            crate::parser::ir::ReferenceKind::Call,
            "a.py",
        );
        // Phantom edge: SymbolId::default() → target
        add_ref(
            &mut g,
            SymbolId::default(),
            target,
            crate::parser::ir::ReferenceKind::Call,
            "a.py",
        );

        let called_by = extract_called_by(&g, target);

        assert!(
            called_by.iter().any(|c| c.contains("caller_fn")),
            "Real caller must appear"
        );
        assert_eq!(
            called_by.len(),
            1,
            "Phantom SymbolId::default() caller must be filtered — expected 1, got {}",
            called_by.len()
        );
    }

    #[test]
    fn test_default_symbol_id_does_not_remove_real_callees() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol(
            "fn_a",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        ));
        let b = g.add_symbol(make_symbol(
            "fn_b",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            5,
        ));
        let c = g.add_symbol(make_symbol(
            "fn_c",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            9,
        ));

        add_ref(&mut g, a, b, crate::parser::ir::ReferenceKind::Call, "a.py");
        add_ref(&mut g, a, c, crate::parser::ir::ReferenceKind::Call, "a.py");

        let calls = extract_calls(&g, a);
        assert_eq!(calls.len(), 2, "Both real callees must be preserved");
    }

    // -- Regression: visibility not hardcoded pub --

    #[test]
    fn test_entity_visibility_not_hardcoded_pub() {
        let private_sym = make_symbol(
            "_private_util",
            SymbolKind::Function,
            Visibility::Private,
            "dead_code.py",
            15,
        );
        let result = extract_visibility(&private_sym);
        assert_eq!(
            result, "priv",
            "_private_util must have vis 'priv', not '{}'",
            result
        );
    }

    // -- Regression: called_by not placeholder --

    #[test]
    fn test_called_by_not_placeholder_detected() {
        let g = build_dead_code_graph();
        // Find active_function
        let active_id = g
            .all_symbols()
            .find(|(_, s)| s.name == "active_function")
            .map(|(id, _)| id)
            .expect("active_function should exist");

        let called_by = extract_called_by(&g, active_id);
        assert!(
            !called_by.is_empty(),
            "active_function should have real callers"
        );
        for caller in &called_by {
            assert_ne!(
                caller, "(detected)",
                "called_by must not be placeholder '(detected)'"
            );
        }
        assert!(
            called_by.iter().any(|c| c.contains("main_handler")),
            "active_function should be called by main_handler"
        );
    }

    // -- No duplicate diagnostics regression --

    #[test]
    fn test_no_duplicate_diagnostics_from_run_all_patterns() {
        use crate::analyzer::patterns::run_all_patterns;
        use std::path::Path;

        let graph = build_all_fixtures_graph();
        let diagnostics = run_all_patterns(&graph, Path::new(""));

        for (i, d1) in diagnostics.iter().enumerate() {
            for (j, d2) in diagnostics.iter().enumerate() {
                if i != j {
                    let is_duplicate = d1.entity == d2.entity
                        && d1.pattern == d2.pattern
                        && d1.location == d2.location;
                    assert!(
                        !is_duplicate,
                        "Duplicate diagnostic found: pattern={:?}, entity='{}', location='{}'",
                        d1.pattern, d1.entity, d1.location
                    );
                }
            }
        }
    }

    // =========================================================================
    // QA-2: infer_module_role regression tests (Cycle 5)
    // =========================================================================

    // -- Substring false positive regression --

    #[test]
    fn test_infer_module_role_contest_results_is_not_test() {
        let mut g = Graph::new();
        g.add_symbol(make_symbol(
            "calculate_score",
            SymbolKind::Function,
            Visibility::Public,
            "contest_results.py",
            1,
        ));
        g.add_symbol(make_symbol(
            "rank_contestants",
            SymbolKind::Function,
            Visibility::Public,
            "contest_results.py",
            10,
        ));

        let role = infer_module_role(&g, Path::new("contest_results.py"));
        assert!(
            !role.to_lowercase().contains("test"),
            "contest_results.py must NOT be classified as test module, got: '{}'",
            role
        );
    }

    #[test]
    fn test_infer_module_role_latest_test_data_is_not_test() {
        let mut g = Graph::new();
        g.add_symbol(make_symbol(
            "load_data",
            SymbolKind::Function,
            Visibility::Public,
            "src/latest_test_data.py",
            1,
        ));

        let role = infer_module_role(&g, Path::new("src/latest_test_data.py"));
        assert!(
            !role.to_lowercase().contains("test"),
            "latest_test_data.py must NOT be classified as test module — 'test_' is a \
             substring, not a prefix. Got: '{}'",
            role
        );
    }

    #[test]
    fn test_infer_module_role_testing_utils_is_not_test() {
        let mut g = Graph::new();
        g.add_symbol(make_symbol(
            "setup_mock",
            SymbolKind::Function,
            Visibility::Public,
            "testing_utils.py",
            1,
        ));

        let role = infer_module_role(&g, Path::new("testing_utils.py"));
        assert!(
            !role.to_lowercase().contains("test"),
            "testing_utils.py must NOT be test module — 'testing_' != 'test_'. Got: '{}'",
            role
        );
    }

    #[test]
    fn test_infer_module_role_protest_module_is_not_test() {
        let mut g = Graph::new();
        g.add_symbol(make_symbol(
            "organize_protest",
            SymbolKind::Function,
            Visibility::Public,
            "protest.py",
            1,
        ));

        let role = infer_module_role(&g, Path::new("protest.py"));
        assert!(
            !role.to_lowercase().contains("test"),
            "protest.py must NOT be test module. Got: '{}'",
            role
        );
    }

    // -- Fixture path regression --

    #[test]
    fn test_infer_module_role_fixture_dead_code_is_not_test() {
        let mut g = Graph::new();
        g.add_symbol(make_symbol(
            "unused_helper",
            SymbolKind::Function,
            Visibility::Private,
            "tests/fixtures/python/dead_code.py",
            11,
        ));
        g.add_symbol(make_symbol(
            "active_function",
            SymbolKind::Function,
            Visibility::Public,
            "tests/fixtures/python/dead_code.py",
            1,
        ));

        let role = infer_module_role(&g, Path::new("tests/fixtures/python/dead_code.py"));
        assert!(
            !role.to_lowercase().contains("test"),
            "Fixture dead_code.py must NOT be 'Test module' — it's a fixture. Got: '{}'",
            role
        );
    }

    // -- True positive guards --

    #[test]
    fn test_infer_module_role_test_prefix_file_is_test() {
        let mut g = Graph::new();
        g.add_symbol(make_symbol(
            "test_create",
            SymbolKind::Function,
            Visibility::Private,
            "test_api.py",
            1,
        ));

        let role = infer_module_role(&g, Path::new("test_api.py"));
        assert!(
            role.to_lowercase().contains("test"),
            "test_api.py MUST be classified as test module. Got: '{}'",
            role
        );
    }

    #[test]
    fn test_infer_module_role_suffix_test_py_is_test() {
        let mut g = Graph::new();
        g.add_symbol(make_symbol(
            "validate",
            SymbolKind::Function,
            Visibility::Public,
            "handler_test.py",
            1,
        ));

        let role = infer_module_role(&g, Path::new("handler_test.py"));
        assert!(
            role.to_lowercase().contains("test"),
            "handler_test.py MUST be test module (suffix match). Got: '{}'",
            role
        );
    }

    #[test]
    fn test_infer_module_role_conftest_is_test() {
        let mut g = Graph::new();
        g.add_symbol(make_symbol(
            "shared_fixture",
            SymbolKind::Function,
            Visibility::Public,
            "conftest.py",
            1,
        ));

        let role = infer_module_role(&g, Path::new("conftest.py"));
        assert!(
            role.to_lowercase().contains("test"),
            "conftest.py MUST be test module. Got: '{}'",
            role
        );
    }

    #[test]
    fn test_infer_module_role_test_functions_without_test_path() {
        let mut g = Graph::new();
        g.add_symbol(make_symbol(
            "test_create_user",
            SymbolKind::Function,
            Visibility::Private,
            "api_checks.py",
            1,
        ));
        g.add_symbol(make_symbol(
            "test_delete_user",
            SymbolKind::Function,
            Visibility::Private,
            "api_checks.py",
            10,
        ));

        let role = infer_module_role(&g, Path::new("api_checks.py"));
        assert!(
            role.to_lowercase().contains("test"),
            "Module with only test_* functions MUST be test module regardless of path. Got: '{}'",
            role
        );
    }

    // -- Empty module edge case --

    #[test]
    fn test_infer_module_role_empty_contest_file_is_not_test() {
        let g = Graph::new();
        let role = infer_module_role(&g, Path::new("contest_results.py"));
        assert_eq!(
            role, "Empty module",
            "Empty file should be 'Empty module' regardless of path"
        );
    }

    // -- JS test convention integration with infer_module_role --

    #[test]
    fn test_infer_module_role_jest_test_file_is_test() {
        let mut g = Graph::new();
        g.add_symbol(make_symbol(
            "describe_block",
            SymbolKind::Function,
            Visibility::Public,
            "Button.test.tsx",
            1,
        ));

        let role = infer_module_role(&g, Path::new("Button.test.tsx"));
        assert!(
            role.to_lowercase().contains("test"),
            "Button.test.tsx MUST be classified as test module after JS conventions added. Got: '{}'",
            role
        );
    }

    #[test]
    fn test_infer_module_role_spec_file_is_test() {
        let mut g = Graph::new();
        g.add_symbol(make_symbol(
            "it_block",
            SymbolKind::Function,
            Visibility::Public,
            "app.spec.ts",
            1,
        ));

        let role = infer_module_role(&g, Path::new("app.spec.ts"));
        assert!(
            role.to_lowercase().contains("test"),
            "app.spec.ts MUST be test module. Got: '{}'",
            role
        );
    }

    // =========================================================================
    // QA-2: dependency_graph Extraction (Cycle 7)
    // =========================================================================

    #[test]
    fn test_dependency_graph_known_import_structure() {
        let mut g = Graph::new();
        let a_fn = g.add_symbol(make_symbol(
            "func_a",
            SymbolKind::Function,
            Visibility::Public,
            "mod_a.py",
            1,
        ));
        let b_fn = g.add_symbol(make_symbol(
            "func_b",
            SymbolKind::Function,
            Visibility::Public,
            "mod_b.py",
            1,
        ));
        add_ref(
            &mut g,
            a_fn,
            b_fn,
            crate::parser::ir::ReferenceKind::Import,
            "mod_a.py",
        );

        let deps = extract_dependency_graph(&g);
        assert_eq!(deps.len(), 1, "Should have exactly one dependency edge");
        assert_eq!(deps[0].from, "mod_a.py");
        assert_eq!(deps[0].to, "mod_b.py");
        assert_eq!(deps[0].weight, 1);
        assert_eq!(deps[0].direction, DependencyDirection::Unidirectional);
    }

    #[test]
    fn test_dependency_graph_empty_project() {
        let g = Graph::new();
        let deps = extract_dependency_graph(&g);
        assert!(deps.is_empty(), "Empty graph must produce no dependencies");
    }

    #[test]
    fn test_dependency_graph_single_file_no_edges() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol(
            "func_a",
            SymbolKind::Function,
            Visibility::Public,
            "main.py",
            1,
        ));
        let b = g.add_symbol(make_symbol(
            "func_b",
            SymbolKind::Function,
            Visibility::Public,
            "main.py",
            5,
        ));
        add_ref(
            &mut g,
            a,
            b,
            crate::parser::ir::ReferenceKind::Call,
            "main.py",
        );

        let deps = extract_dependency_graph(&g);
        assert!(
            deps.is_empty(),
            "Intra-file edges must not produce dependency edges"
        );
    }

    #[test]
    fn test_dependency_graph_circular_bidirectional() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol(
            "func_a",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        ));
        let b = g.add_symbol(make_symbol(
            "func_b",
            SymbolKind::Function,
            Visibility::Public,
            "b.py",
            1,
        ));
        add_ref(
            &mut g,
            a,
            b,
            crate::parser::ir::ReferenceKind::Call,
            "a.py",
        );
        add_ref(
            &mut g,
            b,
            a,
            crate::parser::ir::ReferenceKind::Call,
            "b.py",
        );

        let deps = extract_dependency_graph(&g);
        assert_eq!(
            deps.len(),
            1,
            "Bidirectional edges should be merged into one entry"
        );
        assert_eq!(deps[0].direction, DependencyDirection::Bidirectional);
        assert_eq!(deps[0].weight, 2, "Weight should be sum of both directions");
    }

    #[test]
    fn test_dependency_graph_edge_count_accuracy() {
        let mut g = Graph::new();
        let a1 = g.add_symbol(make_symbol(
            "func_a1",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        ));
        let a2 = g.add_symbol(make_symbol(
            "func_a2",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            5,
        ));
        let b1 = g.add_symbol(make_symbol(
            "func_b1",
            SymbolKind::Function,
            Visibility::Public,
            "b.py",
            1,
        ));
        add_ref(
            &mut g,
            a1,
            b1,
            crate::parser::ir::ReferenceKind::Call,
            "a.py",
        );
        add_ref(
            &mut g,
            a2,
            b1,
            crate::parser::ir::ReferenceKind::Import,
            "a.py",
        );

        let deps = extract_dependency_graph(&g);
        assert_eq!(deps.len(), 1);
        assert_eq!(
            deps[0].weight, 2,
            "Weight must reflect count of cross-file edges, not unique symbol pairs"
        );
    }

    #[test]
    fn test_dependency_graph_module_name_accuracy() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol(
            "handler",
            SymbolKind::Function,
            Visibility::Public,
            "src/api/handler.py",
            1,
        ));
        let b = g.add_symbol(make_symbol(
            "db_query",
            SymbolKind::Function,
            Visibility::Public,
            "src/db/query.py",
            1,
        ));
        add_ref(
            &mut g,
            a,
            b,
            crate::parser::ir::ReferenceKind::Call,
            "src/api/handler.py",
        );

        let deps = extract_dependency_graph(&g);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].from, "src/api/handler.py");
        assert_eq!(deps[0].to, "src/db/query.py");
    }

    #[test]
    fn test_dependency_graph_excludes_unresolved_edges() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol(
            "caller",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        ));
        // Add edge to SymbolId::default() (unresolved) via a reference
        add_ref(
            &mut g,
            a,
            SymbolId::default(),
            crate::parser::ir::ReferenceKind::Call,
            "a.py",
        );
        // Also add a valid cross-file edge
        let b = g.add_symbol(make_symbol(
            "callee",
            SymbolKind::Function,
            Visibility::Public,
            "b.py",
            1,
        ));
        add_ref(
            &mut g,
            a,
            b,
            crate::parser::ir::ReferenceKind::Call,
            "a.py",
        );

        let deps = extract_dependency_graph(&g);
        assert_eq!(deps.len(), 1, "Unresolved edges must be excluded");
        assert_eq!(deps[0].to, "b.py");
    }

    #[test]
    fn test_dependency_graph_star_import_counts() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol(
            "mod_a",
            SymbolKind::Module,
            Visibility::Public,
            "a.py",
            1,
        ));
        let b = g.add_symbol(make_symbol(
            "mod_b",
            SymbolKind::Module,
            Visibility::Public,
            "b.py",
            1,
        ));
        add_ref(
            &mut g,
            a,
            b,
            crate::parser::ir::ReferenceKind::Import,
            "a.py",
        );

        let deps = extract_dependency_graph(&g);
        assert_eq!(deps.len(), 1, "Star import must produce a dependency edge");
        assert_eq!(deps[0].weight, 1);
    }

    // =========================================================================
    // QA-2: Module Role Misclassification Fix (Cycle 7)
    // =========================================================================

    #[test]
    fn test_module_role_classes_with_methods_not_service() {
        let mut g = Graph::new();
        let f = "models.py";
        g.add_symbol(make_symbol(
            "User",
            SymbolKind::Class,
            Visibility::Public,
            f,
            1,
        ));
        g.add_symbol(make_symbol(
            "Product",
            SymbolKind::Class,
            Visibility::Public,
            f,
            10,
        ));
        g.add_symbol(make_symbol(
            "save",
            SymbolKind::Method,
            Visibility::Public,
            f,
            3,
        ));
        g.add_symbol(make_symbol(
            "validate",
            SymbolKind::Method,
            Visibility::Public,
            f,
            5,
        ));
        g.add_symbol(make_symbol(
            "delete",
            SymbolKind::Method,
            Visibility::Public,
            f,
            7,
        ));
        g.add_symbol(make_symbol(
            "get_name",
            SymbolKind::Method,
            Visibility::Public,
            f,
            12,
        ));
        g.add_symbol(make_symbol(
            "set_price",
            SymbolKind::Method,
            Visibility::Public,
            f,
            14,
        ));

        let role = infer_module_role(&g, Path::new(f));
        assert_eq!(
            role, "Data model module",
            "Module with 2 classes and 5 methods should be Data model, not Service"
        );
    }

    #[test]
    fn test_module_role_utility_still_correct_after_fix() {
        let mut g = Graph::new();
        let f = "utils.py";
        g.add_symbol(make_symbol(
            "parse_args",
            SymbolKind::Function,
            Visibility::Public,
            f,
            1,
        ));
        g.add_symbol(make_symbol(
            "format_output",
            SymbolKind::Function,
            Visibility::Public,
            f,
            5,
        ));
        g.add_symbol(make_symbol(
            "validate_input",
            SymbolKind::Function,
            Visibility::Public,
            f,
            9,
        ));

        let role = infer_module_role(&g, Path::new(f));
        assert_eq!(
            role, "Utility module",
            "Module with only functions should remain Utility module"
        );
    }

    #[test]
    fn test_module_role_mixed_classes_methods_functions() {
        let mut g = Graph::new();
        let f = "mixed.py";
        g.add_symbol(make_symbol(
            "UserModel",
            SymbolKind::Class,
            Visibility::Public,
            f,
            1,
        ));
        g.add_symbol(make_symbol(
            "ProductModel",
            SymbolKind::Class,
            Visibility::Public,
            f,
            20,
        ));
        for (i, name) in ["save", "delete_m", "validate_m", "update", "get_id"]
            .iter()
            .enumerate()
        {
            g.add_symbol(make_symbol(
                name,
                SymbolKind::Method,
                Visibility::Public,
                f,
                (3 + i * 2) as u32,
            ));
        }
        g.add_symbol(make_symbol(
            "helper_a",
            SymbolKind::Function,
            Visibility::Public,
            f,
            30,
        ));
        g.add_symbol(make_symbol(
            "helper_b",
            SymbolKind::Function,
            Visibility::Public,
            f,
            34,
        ));
        g.add_symbol(make_symbol(
            "helper_c",
            SymbolKind::Function,
            Visibility::Public,
            f,
            38,
        ));

        let role = infer_module_role(&g, Path::new(f));
        assert_eq!(
            role, "Utility module",
            "Mixed module with more functions than classes should be Utility"
        );
    }

    #[test]
    fn test_module_role_methods_only_no_classes() {
        let mut g = Graph::new();
        let f = "weird.py";
        g.add_symbol(make_symbol(
            "method_a",
            SymbolKind::Method,
            Visibility::Public,
            f,
            1,
        ));
        g.add_symbol(make_symbol(
            "method_b",
            SymbolKind::Method,
            Visibility::Public,
            f,
            5,
        ));

        let role = infer_module_role(&g, Path::new(f));
        assert_eq!(
            role, "Empty module",
            "Module with only methods (no classes) should be Empty after filtering"
        );
    }

    // =========================================================================
    // QA-2: Module Role Regression Guards (Cycle 7)
    // =========================================================================

    #[test]
    fn test_module_role_entry_point_unchanged() {
        let mut g = Graph::new();
        let f = "app.py";
        g.add_symbol(make_entry_point(
            "main",
            SymbolKind::Function,
            Visibility::Public,
            f,
            1,
        ));
        g.add_symbol(make_symbol(
            "helper",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        g.add_symbol(make_symbol(
            "process",
            SymbolKind::Method,
            Visibility::Public,
            f,
            8,
        ));

        let role = infer_module_role(&g, Path::new(f));
        assert_eq!(role, "Entry point module");
    }

    #[test]
    fn test_module_role_test_module_unchanged() {
        let mut g = Graph::new();
        let f = "test_models.py";
        g.add_symbol(make_symbol(
            "test_create_user",
            SymbolKind::Function,
            Visibility::Private,
            f,
            1,
        ));
        g.add_symbol(make_symbol(
            "TestUser",
            SymbolKind::Class,
            Visibility::Public,
            f,
            5,
        ));
        g.add_symbol(make_symbol(
            "setUp",
            SymbolKind::Method,
            Visibility::Public,
            f,
            7,
        ));

        let role = infer_module_role(&g, Path::new(f));
        assert_eq!(role, "Test module");
    }
}
