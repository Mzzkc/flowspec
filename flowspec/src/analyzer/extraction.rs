// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Entity data extraction from the analysis graph.
//!
//! Standalone query functions that extract manifest-ready data from a
//! populated Graph. These replace the hardcoded placeholders from cycle 1's
//! text scanner (`vis: "pub"`, `called_by: ["(detected)"]`).

use std::path::Path;

use crate::graph::Graph;
use crate::parser::ir::{Symbol, SymbolId, SymbolKind, Visibility};

/// Extracts the qualified names of all symbols called by the given symbol.
///
/// Uses `graph.callees()` which returns outgoing edges with `EdgeKind::Calls`.
/// Symbols whose IDs cannot be resolved are silently skipped (defensive).
pub fn extract_calls(graph: &Graph, symbol_id: SymbolId) -> Vec<String> {
    graph
        .callees(symbol_id)
        .iter()
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
/// Symbols whose IDs cannot be resolved are silently skipped (defensive).
pub fn extract_called_by(graph: &Graph, symbol_id: SymbolId) -> Vec<String> {
    graph
        .callers(symbol_id)
        .iter()
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
    let is_test_file = file_str.contains("test_") || file_str.contains("_test.");
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

    // Count symbol kinds (exclude Module symbols from counts)
    let non_module_symbols: Vec<&&Symbol> = symbols
        .iter()
        .filter(|s| s.kind != SymbolKind::Module)
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

        let graph = build_all_fixtures_graph();
        let diagnostics = run_all_patterns(&graph);

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
}
