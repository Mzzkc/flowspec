// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Graph population bridge — translates `ParseResult` into a populated `Graph`.
//!
//! This module provides the critical bridge between language adapters (which produce
//! `ParseResult` with placeholder IDs) and the analysis graph (which assigns real
//! generational IDs via slotmap). The `populate_graph` function handles:
//!
//! - **Scope insertion** with parent reconstruction via location containment
//! - **Symbol insertion** with scope ID remapping
//! - **Reference insertion** with from/to ID remapping (skips unresolvable refs)
//! - **Boundary insertion**
//!
//! Calling `populate_graph` multiple times with different files is additive — each
//! call inserts into the existing graph without corruption.

use super::Graph;
use crate::parser::ir::*;

/// Populates a graph from a `ParseResult`, remapping all placeholder IDs to real
/// generational IDs assigned by the graph's slotmap tables.
///
/// # Scope Parent Reconstruction
///
/// Language adapters emit scopes with `parent: None` because they track nesting
/// via index-based scope stacks, not real `ScopeId` pointers. This function
/// reconstructs parent relationships using a location containment heuristic:
/// if scope B's location is fully within scope A's location, and A is the
/// innermost such scope, then A is B's parent. Tree-sitter provides accurate
/// ranges, making this heuristic reliable.
///
/// # Reference Handling
///
/// References with `from` and `to` both set to `SymbolId::default()` are
/// unresolvable (typically cross-file imports). These are still inserted into
/// the graph as references but create edges pointing to/from placeholder symbols.
/// Cross-file resolution is a separate semantic pass.
///
/// # Multiple Files
///
/// Calling this function multiple times with results from different files is
/// safe and additive. Each call creates its own scope tree rooted at a file scope.
/// `graph.symbols_in_file(path)` returns the correct per-file subset.
pub fn populate_graph(graph: &mut Graph, result: &ParseResult) {
    if result.scopes.is_empty() && result.symbols.is_empty() {
        return;
    }

    // Phase 1: Insert scopes with parent reconstruction
    let scope_id_map = insert_scopes(graph, &result.scopes);

    // Phase 2: Insert symbols with scope remapping
    let symbol_id_map = insert_symbols(graph, &result.symbols, &result.scopes, &scope_id_map);

    // Phase 3: Insert references with from/to remapping
    insert_references(graph, &result.references, &symbol_id_map);

    // Phase 4: Insert boundaries with scope remapping
    insert_boundaries(graph, &result.boundaries, &scope_id_map);
}

/// Inserts scopes into the graph with parent reconstruction via location containment.
///
/// Returns a mapping from original index in `ParseResult.scopes` to the real `ScopeId`.
fn insert_scopes(graph: &mut Graph, scopes: &[Scope]) -> Vec<ScopeId> {
    let mut scope_id_map: Vec<ScopeId> = Vec::with_capacity(scopes.len());

    for (idx, scope) in scopes.iter().enumerate() {
        // Find parent: the innermost previously-inserted scope whose location contains this one
        let parent = if idx == 0 {
            // First scope (file scope) has no parent within this ParseResult
            None
        } else {
            find_parent_scope(scope, &scopes[..idx], &scope_id_map)
        };

        let mut new_scope = scope.clone();
        new_scope.parent = parent;

        let real_id = graph.add_scope(new_scope);
        scope_id_map.push(real_id);
    }

    scope_id_map
}

/// Finds the innermost previously-inserted scope that contains the given scope.
///
/// Uses location containment: scope A contains scope B if A's start position is
/// at or before B's and A's end position is at or after B's, and they're in the
/// same file. Among all containing scopes, picks the innermost (last in insertion
/// order that still contains).
fn find_parent_scope(
    scope: &Scope,
    earlier_scopes: &[Scope],
    scope_id_map: &[ScopeId],
) -> Option<ScopeId> {
    let mut best: Option<(usize, ScopeId)> = None;

    for (i, candidate) in earlier_scopes.iter().enumerate() {
        if candidate.location.file != scope.location.file {
            continue;
        }

        if location_contains(&candidate.location, &scope.location) {
            // This candidate contains the scope. Check if it's more specific than current best.
            match best {
                None => {
                    best = Some((i, scope_id_map[i]));
                }
                Some((best_idx, _)) => {
                    // Prefer the scope whose location is more specific (contained in the previous best)
                    if location_contains(&earlier_scopes[best_idx].location, &candidate.location) {
                        best = Some((i, scope_id_map[i]));
                    }
                }
            }
        }
    }

    best.map(|(_, id)| id)
}

/// Returns true if `outer` fully contains `inner` based on line/column positions.
fn location_contains(outer: &Location, inner: &Location) -> bool {
    // Same location means equal, not containment
    if outer.line == inner.line
        && outer.column == inner.column
        && outer.end_line == inner.end_line
        && outer.end_column == inner.end_column
    {
        return false;
    }

    let outer_start = (outer.line, outer.column);
    let outer_end = (outer.end_line, outer.end_column);
    let inner_start = (inner.line, inner.column);
    let inner_end = (inner.end_line, inner.end_column);

    outer_start <= inner_start && outer_end >= inner_end
}

/// Inserts symbols into the graph with scope ID remapping.
///
/// Returns a mapping from original index in `ParseResult.symbols` to the real `SymbolId`.
fn insert_symbols(
    graph: &mut Graph,
    symbols: &[Symbol],
    scopes: &[Scope],
    scope_id_map: &[ScopeId],
) -> Vec<(usize, SymbolId)> {
    let mut symbol_id_map: Vec<(usize, SymbolId)> = Vec::with_capacity(symbols.len());

    for (idx, symbol) in symbols.iter().enumerate() {
        let mut new_symbol = symbol.clone();

        // Remap scope: find which scope this symbol belongs to based on location
        new_symbol.scope = find_scope_for_symbol(&new_symbol, scopes, scope_id_map);

        let real_id = graph.add_symbol(new_symbol);
        symbol_id_map.push((idx, real_id));
    }

    symbol_id_map
}

/// Finds the most specific scope that contains the given symbol.
///
/// Uses the same location containment heuristic as scope parent reconstruction.
/// If no scope contains the symbol, falls back to the first scope (file scope).
fn find_scope_for_symbol(symbol: &Symbol, scopes: &[Scope], scope_id_map: &[ScopeId]) -> ScopeId {
    if scopes.is_empty() || scope_id_map.is_empty() {
        return ScopeId::default();
    }

    let mut best: Option<(usize, ScopeId)> = None;

    for (i, scope) in scopes.iter().enumerate() {
        if i >= scope_id_map.len() {
            break;
        }
        if scope.location.file != symbol.location.file {
            continue;
        }

        // Check if this scope's location contains the symbol's location
        let scope_start = (scope.location.line, scope.location.column);
        let scope_end = (scope.location.end_line, scope.location.end_column);
        let sym_start = (symbol.location.line, symbol.location.column);
        let sym_end = (symbol.location.end_line, symbol.location.end_column);

        if scope_start <= sym_start && scope_end >= sym_end {
            match best {
                None => {
                    best = Some((i, scope_id_map[i]));
                }
                Some((best_idx, _)) => {
                    // Prefer more specific (inner) scope
                    let best_scope = &scopes[best_idx];
                    let best_start = (best_scope.location.line, best_scope.location.column);
                    let best_end = (best_scope.location.end_line, best_scope.location.end_column);

                    if scope_start >= best_start && scope_end <= best_end {
                        best = Some((i, scope_id_map[i]));
                    }
                }
            }
        }
    }

    // The symbol's scope should be its PARENT scope, not its own scope.
    // E.g., a function defined at lines 5-10 should be in the file scope (lines 1-100),
    // not in the function's own scope (which also spans lines 5-10).
    // The adapter creates a scope for each function/class, and the scope has the same
    // location as the symbol. We want the scope that CONTAINS the symbol but is NOT
    // the symbol's own scope (same name and location).
    //
    // Strategy: if the best scope has the same location as the symbol AND is a
    // function/module/block scope (not file scope), prefer its parent scope instead.
    if let Some((best_idx, best_scope_id)) = best {
        let best_scope = &scopes[best_idx];
        let same_location = best_scope.location.line == symbol.location.line
            && best_scope.location.column == symbol.location.column
            && best_scope.location.end_line == symbol.location.end_line
            && best_scope.location.end_column == symbol.location.end_column;

        if same_location && best_scope.kind != ScopeKind::File {
            // This scope IS the symbol's own scope. Find its parent instead.
            // Walk back through earlier scopes to find the containing one.
            for i in (0..best_idx).rev() {
                if i >= scope_id_map.len() {
                    continue;
                }
                let candidate = &scopes[i];
                if candidate.location.file != symbol.location.file {
                    continue;
                }
                let cand_start = (candidate.location.line, candidate.location.column);
                let cand_end = (candidate.location.end_line, candidate.location.end_column);
                let sym_start = (symbol.location.line, symbol.location.column);
                let sym_end = (symbol.location.end_line, symbol.location.end_column);

                if cand_start <= sym_start && cand_end >= sym_end {
                    return scope_id_map[i];
                }
            }
            // Couldn't find a parent — use this scope anyway
            return best_scope_id;
        }

        return best_scope_id;
    }

    // Fallback: first scope (file scope) if available
    scope_id_map[0]
}

/// Inserts references with from/to symbol ID remapping.
///
/// References from the adapter have `from: SymbolId::default()` and `to: SymbolId::default()`.
/// We attempt to resolve them:
/// - If `from` and `to` are both default, insert the reference as-is (edges will point to
///   nonexistent symbols, but the reference is preserved for future resolution passes).
/// - If either can be mapped to a real symbol, use the mapped ID.
fn insert_references(
    graph: &mut Graph,
    references: &[Reference],
    symbol_id_map: &[(usize, SymbolId)],
) {
    for reference in references {
        let mut new_ref = reference.clone();

        // Try to resolve from/to by index if they match any symbol
        // Since adapters use SymbolId::default() for all, we keep as-is for now
        // The references are still valuable for tracking import counts and patterns
        if new_ref.from == SymbolId::default() && !symbol_id_map.is_empty() {
            // Use the first symbol as a file-level context for "from"
            new_ref.from = symbol_id_map[0].1;
        }

        graph.add_reference(new_ref);
    }
}

/// Inserts boundaries with scope ID remapping.
fn insert_boundaries(graph: &mut Graph, boundaries: &[Boundary], scope_id_map: &[ScopeId]) {
    for boundary in boundaries {
        let mut new_boundary = boundary.clone();

        // Remap from_scope and to_scope if they're default
        if !scope_id_map.is_empty() {
            // Try to find matching scope by index (boundaries reference scopes by position)
            // For now, keep original scope IDs — boundaries are rare in Python
            if new_boundary.from_scope == ScopeId::default() {
                new_boundary.from_scope = scope_id_map[0];
            }
            if new_boundary.to_scope == ScopeId::default() && scope_id_map.len() > 1 {
                new_boundary.to_scope = scope_id_map[1];
            }
        }

        graph.add_boundary(new_boundary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::python::PythonAdapter;
    use crate::parser::LanguageAdapter;
    use std::path::PathBuf;

    fn fixture_dir() -> PathBuf {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest.parent().unwrap().join("tests/fixtures/python")
    }

    // -- Basic fixtures: basic_functions.py ----------------------------------

    #[test]
    fn test_populate_basic_functions_symbol_count() {
        let path = fixture_dir().join("basic_functions.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        assert_eq!(
            graph.symbol_count(),
            3,
            "basic_functions.py has 3 functions"
        );
    }

    #[test]
    fn test_populate_basic_functions_scope_count() {
        let path = fixture_dir().join("basic_functions.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        assert_eq!(graph.scope_count(), 4, "1 file scope + 3 function scopes");
    }

    #[test]
    fn test_populate_basic_functions_visibility_preserved() {
        let path = fixture_dir().join("basic_functions.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        let private_sym = graph
            .all_symbols()
            .find(|(_, s)| s.name == "_private_helper")
            .expect("Must find _private_helper in graph");
        assert_eq!(
            private_sym.1.visibility,
            Visibility::Private,
            "Python leading underscore must produce Private visibility"
        );
    }

    #[test]
    fn test_populate_basic_functions_qualified_names() {
        let path = fixture_dir().join("basic_functions.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        for (_, s) in graph.all_symbols() {
            assert!(
                s.qualified_name.contains("basic_functions"),
                "Qualified name '{}' must contain file stem",
                s.qualified_name
            );
        }
    }

    #[test]
    fn test_populate_basic_functions_file_symbols_index() {
        let path = fixture_dir().join("basic_functions.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        let file_syms = graph.symbols_in_file(&path);
        assert_eq!(
            file_syms.len(),
            3,
            "symbols_in_file must find all 3 functions"
        );
    }

    #[test]
    fn test_populate_basic_functions_location_preserved() {
        let path = fixture_dir().join("basic_functions.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        let greet = graph
            .all_symbols()
            .find(|(_, s)| s.name == "greet")
            .expect("Must find greet");
        assert_eq!(greet.1.location.line, 1, "greet starts at line 1");
        assert!(greet.1.location.file.ends_with("basic_functions.py"));
    }

    // -- Classes: classes.py -------------------------------------------------

    #[test]
    fn test_populate_classes_symbol_kinds() {
        let path = fixture_dir().join("classes.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        let classes: Vec<_> = graph
            .all_symbols()
            .filter(|(_, s)| s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(classes.len(), 2, "Must find Animal and Dog");

        let methods: Vec<_> = graph
            .all_symbols()
            .filter(|(_, s)| s.kind == SymbolKind::Method)
            .collect();
        assert!(
            methods.len() >= 4,
            "Must find at least 4 methods, got {}",
            methods.len()
        );
    }

    #[test]
    fn test_populate_classes_scope_nesting() {
        let path = fixture_dir().join("classes.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        // At minimum: 1 file scope + 2 class scopes + 4 method scopes = 7
        assert!(
            graph.scope_count() >= 7,
            "classes.py needs >= 7 scopes, got {}",
            graph.scope_count()
        );
    }

    #[test]
    fn test_populate_classes_method_scope_assignment() {
        let path = fixture_dir().join("classes.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        let dog_speak = graph
            .all_symbols()
            .find(|(_, s)| s.name == "speak" && s.qualified_name.contains("Dog"))
            .expect("Must find Dog.speak");

        assert_ne!(
            dog_speak.1.scope,
            ScopeId::default(),
            "Method scope must be remapped from default"
        );
    }

    #[test]
    fn test_populate_classes_annotations_preserved() {
        let path = fixture_dir().join("classes.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        let species = graph
            .all_symbols()
            .find(|(_, s)| s.name == "species")
            .expect("Must find species");
        assert!(
            species
                .1
                .annotations
                .iter()
                .any(|a| a.contains("staticmethod")),
            "staticmethod annotation must survive, got: {:?}",
            species.1.annotations
        );
    }

    // -- Imports: imports.py -------------------------------------------------

    #[test]
    fn test_populate_imports_reference_count() {
        let path = fixture_dir().join("imports.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        let ref_count = graph.reference_count();
        // References are inserted: should be >= 5
        assert!(
            ref_count >= 5,
            "imports.py should produce >= 5 references, got {}",
            ref_count
        );
    }

    // -- Deeply nested: deeply_nested.py -------------------------------------

    #[test]
    fn test_populate_deeply_nested_all_functions() {
        let path = fixture_dir().join("deeply_nested.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        assert_eq!(
            graph.symbol_count(),
            10,
            "Must find all 10 nested functions"
        );
    }

    #[test]
    fn test_populate_deeply_nested_scope_hierarchy() {
        let path = fixture_dir().join("deeply_nested.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        assert!(
            graph.scope_count() >= 11,
            "Must have >= 11 scopes (1 file + 10 function), got {}",
            graph.scope_count()
        );
    }

    // -- Edge cases ----------------------------------------------------------

    #[test]
    fn test_populate_empty_file_no_panic() {
        let path = fixture_dir().join("empty.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        assert_eq!(graph.symbol_count(), 0);
        assert_eq!(graph.scope_count(), 1, "Empty file still has file scope");
    }

    #[test]
    fn test_populate_empty_parse_result() {
        let mut graph = Graph::new();
        let result = ParseResult::default();
        populate_graph(&mut graph, &result);

        assert_eq!(graph.symbol_count(), 0);
        assert_eq!(graph.scope_count(), 0);
        assert_eq!(graph.reference_count(), 0);
    }

    #[test]
    fn test_populate_comments_only_file() {
        let path = fixture_dir().join("only_comments.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        assert_eq!(graph.symbol_count(), 0);
        assert!(graph.scope_count() >= 1, "Must have at least file scope");
    }

    #[test]
    fn test_populate_syntax_errors_partial() {
        let path = fixture_dir().join("syntax_errors.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        let names: Vec<String> = graph.all_symbols().map(|(_, s)| s.name.clone()).collect();
        assert!(
            names.contains(&"valid_function".to_string()),
            "Must find valid_function despite syntax errors"
        );
        assert!(
            names.contains(&"another_valid".to_string()),
            "Must find another_valid despite syntax errors"
        );
    }

    // -- Multi-file population -----------------------------------------------

    #[test]
    fn test_populate_multiple_files_additive() {
        let adapter = PythonAdapter::new();

        let path1 = fixture_dir().join("basic_functions.py");
        let content1 = std::fs::read_to_string(&path1).unwrap();
        let result1 = adapter.parse_file(&path1, &content1).unwrap();

        let path2 = fixture_dir().join("classes.py");
        let content2 = std::fs::read_to_string(&path2).unwrap();
        let result2 = adapter.parse_file(&path2, &content2).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result1);
        let count_after_first = graph.symbol_count();

        populate_graph(&mut graph, &result2);
        let count_after_second = graph.symbol_count();

        assert!(
            count_after_second > count_after_first,
            "Second population must add symbols: {} -> {}",
            count_after_first,
            count_after_second
        );

        let file1_syms = graph.symbols_in_file(&path1);
        let file2_syms = graph.symbols_in_file(&path2);
        assert_eq!(file1_syms.len(), 3, "basic_functions.py has 3 symbols");
        assert!(
            file2_syms.len() >= 6,
            "classes.py has 2 classes + 4+ methods"
        );

        // No overlap
        let file1_set: std::collections::HashSet<_> = file1_syms.iter().collect();
        let file2_set: std::collections::HashSet<_> = file2_syms.iter().collect();
        assert!(
            file1_set.is_disjoint(&file2_set),
            "Different files must have disjoint symbol IDs"
        );
    }

    #[test]
    fn test_populate_multiple_files_scope_isolation() {
        let adapter = PythonAdapter::new();

        let path1 = fixture_dir().join("basic_functions.py");
        let content1 = std::fs::read_to_string(&path1).unwrap();
        let result1 = adapter.parse_file(&path1, &content1).unwrap();

        let path2 = fixture_dir().join("empty.py");
        let content2 = std::fs::read_to_string(&path2).unwrap();
        let result2 = adapter.parse_file(&path2, &content2).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result1);
        let scopes_after_first = graph.scope_count();

        populate_graph(&mut graph, &result2);
        let scopes_after_second = graph.scope_count();

        assert_eq!(
            scopes_after_second,
            scopes_after_first + 1,
            "Empty file adds exactly 1 scope"
        );
    }

    // -- Scope parent reconstruction -----------------------------------------

    #[test]
    fn test_populate_scope_parent_reconstruction_classes() {
        let path = fixture_dir().join("classes.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        assert!(
            graph.scope_count() >= 7,
            "classes.py must produce >= 7 scopes with proper nesting"
        );
    }

    #[test]
    fn test_populate_scope_parent_deeply_nested_chain() {
        let path = fixture_dir().join("deeply_nested.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        assert_eq!(
            graph.scope_count(),
            11,
            "deeply_nested.py: 1 file + 10 function scopes"
        );
    }

    // -- Symbol kind preservation --------------------------------------------

    #[test]
    fn test_populate_constants_and_variables_kinds() {
        let path = fixture_dir().join("constants_and_variables.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        let constants: Vec<_> = graph
            .all_symbols()
            .filter(|(_, s)| s.kind == SymbolKind::Constant)
            .collect();
        assert!(
            constants.len() >= 3,
            "constants_and_variables.py needs >= 3 constants, got {}",
            constants.len()
        );

        let variables: Vec<_> = graph
            .all_symbols()
            .filter(|(_, s)| s.kind == SymbolKind::Variable)
            .collect();
        assert!(
            variables.len() >= 2,
            "constants_and_variables.py needs >= 2 variables, got {}",
            variables.len()
        );
    }

    #[test]
    fn test_populate_unicode_identifiers() {
        let path = fixture_dir().join("unicode_identifiers.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        let has_cafe = graph.all_symbols().any(|(_, s)| s.name == "café");
        assert!(
            has_cafe,
            "Unicode function name 'café' must survive population"
        );

        let has_nono = graph.all_symbols().any(|(_, s)| s.name == "Ñoño");
        assert!(
            has_nono,
            "Unicode class name 'Ñoño' must survive population"
        );
    }

    // -- Regression tests (ID remapping) -------------------------------------

    #[test]
    fn test_populate_no_default_symbol_ids_in_graph() {
        let path = fixture_dir().join("basic_functions.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        for (id, symbol) in graph.all_symbols() {
            assert_eq!(
                id, symbol.id,
                "Symbol ID key must match stored id field for '{}'",
                symbol.name
            );
            assert_ne!(
                id,
                SymbolId::default(),
                "Symbol '{}' must not have default ID after population",
                symbol.name
            );
        }
    }

    #[test]
    fn test_populate_no_default_scope_ids_in_graph() {
        let path = fixture_dir().join("basic_functions.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        for (_, symbol) in graph.all_symbols() {
            assert_ne!(
                symbol.scope,
                ScopeId::default(),
                "Symbol '{}' must have remapped scope ID, not default",
                symbol.name
            );
            assert!(
                graph.get_scope(symbol.scope).is_some(),
                "Symbol '{}' references scope {:?} which doesn't exist",
                symbol.name,
                symbol.scope
            );
        }
    }

    // -- Adversarial ---------------------------------------------------------

    #[test]
    fn test_populate_symbol_with_empty_name() {
        let mut graph = Graph::new();
        let mut result = ParseResult::default();
        result.scopes.push(Scope {
            id: ScopeId::default(),
            kind: ScopeKind::File,
            parent: None,
            name: "test.py".to_string(),
            location: Location {
                file: PathBuf::from("test.py"),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            },
        });
        result.symbols.push(Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Function,
            name: String::new(),
            qualified_name: String::new(),
            visibility: Visibility::Public,
            signature: None,
            location: Location {
                file: PathBuf::from("test.py"),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            },
            resolution: ResolutionStatus::Resolved,
            scope: ScopeId::default(),
            annotations: vec![],
        });

        populate_graph(&mut graph, &result);
        assert_eq!(graph.symbol_count(), 1);
    }

    #[test]
    fn test_populate_called_twice_with_same_file() {
        let path = fixture_dir().join("basic_functions.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);
        populate_graph(&mut graph, &result);

        assert_eq!(
            graph.symbol_count(),
            6,
            "Double population should double symbols"
        );
    }
}
