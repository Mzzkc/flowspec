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

    // Phase 3: Insert references with from/to remapping (including call resolution)
    insert_references(graph, &result.references, &symbol_id_map, &result.symbols);

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
/// For `ReferenceKind::Call` references with `ResolutionStatus::Partial("call:<name>")`,
/// performs intra-file resolution:
/// - `from` is resolved via location containment (which function/method contains the call)
/// - `to` is resolved via name matching against same-file symbols
///
/// For other references (imports), uses the existing first-symbol fallback for `from`.
fn insert_references(
    graph: &mut Graph,
    references: &[Reference],
    symbol_id_map: &[(usize, SymbolId)],
    symbols: &[Symbol],
) {
    for reference in references {
        let mut new_ref = reference.clone();

        match &reference.resolution {
            ResolutionStatus::Partial(info) if info.starts_with("call:") => {
                let callee_name = &info[5..]; // after "call:"

                // Resolve from: find the symbol whose location contains this call
                new_ref.from = find_containing_symbol(&reference.location, symbols, symbol_id_map)
                    .unwrap_or_else(|| {
                        // Fallback: first symbol (module-level call)
                        if !symbol_id_map.is_empty() {
                            symbol_id_map[0].1
                        } else {
                            SymbolId::default()
                        }
                    });

                // Resolve to: match callee name against same-file symbols
                new_ref.to =
                    resolve_callee(callee_name, &new_ref.from, graph, symbol_id_map, symbols);
            }
            _ => {
                // Non-call references: keep existing behavior
                if new_ref.from == SymbolId::default() && !symbol_id_map.is_empty() {
                    new_ref.from = symbol_id_map[0].1;
                }
            }
        }

        graph.add_reference(new_ref);
    }
}

/// Finds the innermost function/method symbol whose location contains the reference.
///
/// Used to determine which symbol a call expression belongs to. Returns `None` for
/// module-level calls that are not inside any function or method body.
fn find_containing_symbol(
    ref_location: &Location,
    symbols: &[Symbol],
    symbol_id_map: &[(usize, SymbolId)],
) -> Option<SymbolId> {
    let mut best: Option<(usize, SymbolId)> = None;

    for &(idx, real_id) in symbol_id_map {
        let sym = &symbols[idx];

        if sym.location.file != ref_location.file {
            continue;
        }

        // Only functions and methods can "contain" calls
        if !matches!(sym.kind, SymbolKind::Function | SymbolKind::Method) {
            continue;
        }

        let sym_start = (sym.location.line, sym.location.column);
        let sym_end = (sym.location.end_line, sym.location.end_column);
        let ref_start = (ref_location.line, ref_location.column);
        let ref_end = (ref_location.end_line, ref_location.end_column);

        if sym_start <= ref_start && sym_end >= ref_end {
            match best {
                None => best = Some((idx, real_id)),
                Some((best_idx, _)) => {
                    // Prefer more specific (innermost) containing symbol
                    let best_sym = &symbols[best_idx];
                    let best_start = (best_sym.location.line, best_sym.location.column);
                    let best_end = (best_sym.location.end_line, best_sym.location.end_column);

                    if sym_start >= best_start && sym_end <= best_end {
                        best = Some((idx, real_id));
                    }
                }
            }
        }
    }

    best.map(|(_, id)| id)
}

/// Resolves a callee name to a `SymbolId` by matching against same-file symbols.
///
/// Handles three patterns:
/// - `self.method` — matches `Method` symbols in the same class scope as `from`
/// - `simple_name` — matches any symbol with the same name
/// - `obj.attr` — stays unresolved (cross-file or requires type inference)
fn resolve_callee(
    callee_name: &str,
    from_id: &SymbolId,
    graph: &Graph,
    symbol_id_map: &[(usize, SymbolId)],
    symbols: &[Symbol],
) -> SymbolId {
    // Handle self.method pattern
    if let Some(method_name) = callee_name.strip_prefix("self.") {
        if let Some(from_sym) = graph.get_symbol(*from_id) {
            let from_scope = from_sym.scope;
            for &(idx, real_id) in symbol_id_map {
                let sym = &symbols[idx];
                if sym.name == method_name && sym.kind == SymbolKind::Method {
                    if let Some(graph_sym) = graph.get_symbol(real_id) {
                        if graph_sym.scope == from_scope {
                            return real_id;
                        }
                    }
                }
            }
        }
        return SymbolId::default();
    }

    // Dotted names (module.func) — cross-file, cannot resolve intra-file
    if callee_name.contains('.') {
        return SymbolId::default();
    }

    // Simple name: match against all symbols in the same file
    let mut candidates: Vec<(usize, SymbolId)> = Vec::new();
    for &(idx, real_id) in symbol_id_map {
        let sym = &symbols[idx];
        if sym.name == callee_name {
            candidates.push((idx, real_id));
        }
    }

    match candidates.len() {
        0 => SymbolId::default(),
        1 => candidates[0].1,
        _ => {
            // Multiple matches: prefer Function/Method/Class over Variable/Constant
            for &(idx, real_id) in &candidates {
                let sym = &symbols[idx];
                if matches!(
                    sym.kind,
                    SymbolKind::Function | SymbolKind::Method | SymbolKind::Class
                ) {
                    return real_id;
                }
            }
            candidates[0].1
        }
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

    // =========================================================================
    // QA-Foundation: Call-site detection + intra-file resolution tests
    // =========================================================================

    /// Parse inline Python source into a temp file, populate a graph, return (graph, path).
    fn parse_and_populate(source: &str) -> (Graph, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test_module.py");
        std::fs::write(&path, source).expect("write fixture");
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, source).expect("parse");
        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);
        std::mem::forget(dir);
        (graph, path)
    }

    /// Find a symbol by name in the graph. Panics if not found.
    fn find_symbol<'a>(graph: &'a Graph, name: &str) -> (SymbolId, &'a Symbol) {
        graph
            .all_symbols()
            .find(|(_, s)| s.name == name)
            .unwrap_or_else(|| panic!("Symbol '{}' not found in graph", name))
    }

    /// Count Call edges outgoing from a symbol.
    fn call_edge_count(graph: &Graph, from_id: SymbolId) -> usize {
        graph
            .edges_from(from_id)
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .count()
    }

    /// Count Call edges incoming to a symbol.
    fn inbound_call_count(graph: &Graph, to_id: SymbolId) -> usize {
        graph
            .edges_to(to_id)
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .count()
    }

    // -- P0: Call-Site Extraction — Basic Detection ----------------------------

    #[test]
    fn test_simple_function_call_produces_call_reference() {
        let source = r#"
def callee():
    pass

def caller():
    callee()
"#;
        let (graph, _path) = parse_and_populate(source);

        let has_call_edge = graph.all_symbols().any(|(id, _)| {
            graph
                .edges_from(id)
                .iter()
                .any(|e| e.kind == EdgeKind::Calls)
        });

        assert!(
            has_call_edge,
            "PythonAdapter must produce at least one Call edge for `callee()`."
        );
    }

    #[test]
    fn test_multiple_calls_in_single_function() {
        let source = r#"
def a():
    pass

def b():
    pass

def c():
    pass

def main():
    a()
    b()
    c()
"#;
        let (graph, _) = parse_and_populate(source);
        let (main_id, _) = find_symbol(&graph, "main");

        let count = call_edge_count(&graph, main_id);
        assert_eq!(
            count, 3,
            "main() calls a(), b(), c() — must have exactly 3 outgoing Calls edges, got {}.",
            count
        );
    }

    #[test]
    fn test_nested_calls_both_detected() {
        let source = r#"
def f(x):
    return x

def g():
    return 42

def outer():
    f(g())
"#;
        let (graph, _) = parse_and_populate(source);
        let (outer_id, _) = find_symbol(&graph, "outer");

        let count = call_edge_count(&graph, outer_id);
        assert_eq!(
            count, 2,
            "outer() calls f(g()) — must detect BOTH f and g. Got {} call edges.",
            count
        );
    }

    #[test]
    fn test_call_in_list_comprehension_detected() {
        let source = r#"
def transform(x):
    return x * 2

def process(items):
    return [transform(x) for x in items]
"#;
        let (graph, _) = parse_and_populate(source);
        let (process_id, _) = find_symbol(&graph, "process");

        let callees = graph.callees(process_id);
        let callee_names: Vec<String> = callees
            .iter()
            .filter_map(|id| graph.get_symbol(*id))
            .map(|s| s.name.clone())
            .collect();

        assert!(
            callee_names.contains(&"transform".to_string()),
            "process() must detect transform() inside list comprehension. Callees: {:?}.",
            callee_names
        );
    }

    #[test]
    fn test_call_in_decorator_argument_detected() {
        let source = r#"
def config():
    return {}

def decorator(cfg):
    def wrapper(fn):
        return fn
    return wrapper

@decorator(config())
def my_func():
    pass
"#;
        let (graph, _) = parse_and_populate(source);

        let (config_id, _) = find_symbol(&graph, "config");
        let inbound = inbound_call_count(&graph, config_id);

        assert!(
            inbound >= 1,
            "config() used as decorator argument must be detected. Inbound Calls: {}.",
            inbound
        );
    }

    #[test]
    fn test_chained_calls_detect_inner_callee() {
        let source = r#"
def builder():
    return None

def run():
    builder().execute()
"#;
        let (graph, _) = parse_and_populate(source);
        let (builder_id, _) = find_symbol(&graph, "builder");

        let inbound = inbound_call_count(&graph, builder_id);
        assert!(
            inbound >= 1,
            "builder() in chained call must be detected. Inbound Calls: {}.",
            inbound
        );
    }

    #[test]
    fn test_calls_with_various_argument_forms() {
        let source = r#"
def f():
    pass

def g(a, b):
    pass

def h(x=0):
    pass

def run():
    f()
    g(1, 2)
    h(x=3)
"#;
        let (graph, _) = parse_and_populate(source);
        let (run_id, _) = find_symbol(&graph, "run");

        let count = call_edge_count(&graph, run_id);
        assert_eq!(
            count, 3,
            "run() makes 3 calls with different argument forms. Got {} Calls edges.",
            count
        );
    }

    // -- P0: Intra-File Resolution — Basic Name Matching ----------------------

    #[test]
    fn test_intrafile_call_resolves_to_defined_function() {
        let source = r#"
def helper():
    return 42

def caller():
    helper()
"#;
        let (graph, _) = parse_and_populate(source);
        let (caller_id, _) = find_symbol(&graph, "caller");
        let (helper_id, _) = find_symbol(&graph, "helper");

        let callees = graph.callees(caller_id);
        assert!(
            callees.contains(&helper_id),
            "caller()'s callees must include helper's real SymbolId."
        );

        let callers = graph.callers(helper_id);
        assert!(
            callers.contains(&caller_id),
            "helper's callers must include caller. Bidirectional edge insertion broken."
        );
    }

    #[test]
    fn test_call_to_external_name_stays_unresolved() {
        let source = r#"
import json

def run():
    json.dumps({"key": "value"})
"#;
        let (graph, _) = parse_and_populate(source);
        let (run_id, _) = find_symbol(&graph, "run");

        let callees = graph.callees(run_id);
        let callee_names: Vec<String> = callees
            .iter()
            .filter_map(|id| graph.get_symbol(*id))
            .map(|s| s.name.clone())
            .collect();

        assert!(
            !callee_names.contains(&"dumps".to_string()),
            "json.dumps() is cross-file — must NOT resolve to a local symbol. Resolved: {:?}.",
            callee_names
        );
    }

    #[test]
    fn test_calls_to_builtins_stay_unresolved() {
        let source = r#"
def run():
    print("hello")
    x = len([1, 2, 3])
    isinstance(x, int)
"#;
        let (graph, _) = parse_and_populate(source);
        let (run_id, _) = find_symbol(&graph, "run");

        let resolved_callees = graph.callees(run_id);
        let resolved_names: Vec<String> = resolved_callees
            .iter()
            .filter_map(|id| graph.get_symbol(*id))
            .map(|s| s.name.clone())
            .collect();

        assert!(
            resolved_names.is_empty(),
            "Built-in calls must NOT resolve to any graph symbol. Resolved: {:?}.",
            resolved_names
        );
    }

    #[test]
    fn test_call_to_undefined_name_stays_unresolved() {
        let source = r#"
def run():
    nonexistent_function()
"#;
        let (graph, _) = parse_and_populate(source);
        let (run_id, _) = find_symbol(&graph, "run");

        let resolved_callees = graph.callees(run_id);
        assert!(
            resolved_callees.is_empty()
                || resolved_callees
                    .iter()
                    .all(|id| graph.get_symbol(*id).is_none()),
            "Call to undefined name must not resolve to any real symbol. Callees: {:?}.",
            resolved_callees
        );
    }

    #[test]
    fn test_class_instantiation_resolves_to_class() {
        let source = r#"
class Foo:
    def __init__(self):
        pass

def create():
    return Foo()
"#;
        let (graph, _) = parse_and_populate(source);
        let (create_id, _) = find_symbol(&graph, "create");
        let (foo_id, _) = find_symbol(&graph, "Foo");

        let callees = graph.callees(create_id);
        assert!(
            callees.contains(&foo_id),
            "Foo() instantiation must resolve to the Foo class symbol. Callees: {:?}.",
            callees
        );
    }

    // -- P1: Method Calls and self Resolution ---------------------------------

    #[test]
    fn test_self_method_call_resolves_to_class_method() {
        let source = r#"
class Service:
    def validate(self, data):
        return data is not None

    def run(self, data):
        if self.validate(data):
            return data
"#;
        let (graph, _) = parse_and_populate(source);

        let (run_id, _) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "run" && s.kind == SymbolKind::Method)
            .expect("Must find run method");
        let (validate_id, _) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "validate" && s.kind == SymbolKind::Method)
            .expect("Must find validate method");

        let callees = graph.callees(run_id);
        assert!(
            callees.contains(&validate_id),
            "self.validate() in run() must resolve to the validate method. Callees: {:?}.",
            callees
        );
    }

    #[test]
    fn test_non_self_method_call_stays_unresolved() {
        let source = r#"
def run(obj):
    obj.process()
"#;
        let (graph, _) = parse_and_populate(source);
        let (run_id, _) = find_symbol(&graph, "run");

        let resolved_callees = graph.callees(run_id);
        let resolved_names: Vec<String> = resolved_callees
            .iter()
            .filter_map(|id| graph.get_symbol(*id))
            .map(|s| s.name.clone())
            .collect();

        assert!(
            !resolved_names.contains(&"process".to_string()),
            "obj.process() must NOT resolve to a local symbol. Resolved: {:?}.",
            resolved_names
        );
    }

    #[test]
    fn test_self_method_to_undefined_method_stays_unresolved() {
        let source = r#"
class Child:
    def run(self):
        self.parent_method()
"#;
        let (graph, _) = parse_and_populate(source);
        let (run_id, _) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "run")
            .expect("Must find run");

        let resolved_callees = graph.callees(run_id);
        assert!(
            resolved_callees.is_empty()
                || resolved_callees
                    .iter()
                    .all(|id| graph.get_symbol(*id).is_none()),
            "self.parent_method() with no local definition must stay unresolved. Resolved: {:?}.",
            resolved_callees
        );
    }

    #[test]
    fn test_multiple_self_calls_resolve_to_correct_methods() {
        let source = r#"
class Pipeline:
    def step_a(self):
        self.step_b()
        self.step_c()

    def step_b(self):
        pass

    def step_c(self):
        pass
"#;
        let (graph, _) = parse_and_populate(source);
        let (a_id, _) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "step_a")
            .expect("step_a");
        let (b_id, _) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "step_b")
            .expect("step_b");
        let (c_id, _) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "step_c")
            .expect("step_c");

        let callees = graph.callees(a_id);
        assert!(callees.contains(&b_id), "step_a must call step_b");
        assert!(callees.contains(&c_id), "step_a must call step_c");
        assert_eq!(callees.len(), 2, "step_a calls exactly 2 methods");
    }

    // -- P1: Edge Type and Graph Invariant Verification -----------------------

    #[test]
    fn test_call_edges_have_correct_edge_kind() {
        let source = r#"
def target():
    pass

def caller():
    target()
"#;
        let (graph, _) = parse_and_populate(source);
        let (caller_id, _) = find_symbol(&graph, "caller");
        let (target_id, _) = find_symbol(&graph, "target");

        let outgoing = graph.edges_from(caller_id);
        let call_edges: Vec<_> = outgoing
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        assert_eq!(call_edges.len(), 1, "Exactly one Calls edge from caller");
        assert_eq!(
            call_edges[0].target, target_id,
            "Call edge target must be target's real SymbolId, not default"
        );
    }

    #[test]
    fn test_edge_count_matches_source_call_count() {
        let source = r#"
def f1(): pass
def f2(): pass
def f3(): pass
def f4(): pass
def f5(): pass

def orchestrator():
    f1()
    f2()
    f3()
    f4()
    f5()
"#;
        let (graph, _) = parse_and_populate(source);
        let (orch_id, _) = find_symbol(&graph, "orchestrator");

        let count = call_edge_count(&graph, orch_id);
        assert_eq!(
            count, 5,
            "orchestrator() has exactly 5 calls — edge count must match. Got {}.",
            count
        );
    }

    #[test]
    fn test_from_symbol_is_correct_containing_function() {
        let source = r#"
def target_a():
    pass

def target_b():
    pass

def caller_one():
    target_a()

def caller_two():
    target_b()
"#;
        let (graph, _) = parse_and_populate(source);
        let (one_id, _) = find_symbol(&graph, "caller_one");
        let (two_id, _) = find_symbol(&graph, "caller_two");
        let (a_id, _) = find_symbol(&graph, "target_a");
        let (b_id, _) = find_symbol(&graph, "target_b");

        // caller_one calls target_a, NOT target_b
        let one_callees = graph.callees(one_id);
        assert!(one_callees.contains(&a_id), "caller_one must call target_a");
        assert!(
            !one_callees.contains(&b_id),
            "caller_one must NOT call target_b"
        );

        // caller_two calls target_b, NOT target_a
        let two_callees = graph.callees(two_id);
        assert!(two_callees.contains(&b_id), "caller_two must call target_b");
        assert!(
            !two_callees.contains(&a_id),
            "caller_two must NOT call target_a"
        );

        // Verify via callers() — bidirectional check
        let a_callers = graph.callers(a_id);
        assert!(
            a_callers.contains(&one_id),
            "target_a's caller must be caller_one"
        );
        assert!(
            !a_callers.contains(&two_id),
            "target_a must NOT show caller_two as caller"
        );
    }

    // -- P2: Adversarial Edge Cases -------------------------------------------

    #[test]
    fn test_recursive_call_creates_self_edge() {
        let source = r#"
def factorial(n):
    if n <= 1:
        return 1
    return n * factorial(n - 1)
"#;
        let (graph, _) = parse_and_populate(source);
        let (fact_id, _) = find_symbol(&graph, "factorial");

        let callees = graph.callees(fact_id);
        assert!(
            callees.contains(&fact_id),
            "Recursive call must create a self-loop Calls edge. Callees: {:?}.",
            callees
        );
    }

    #[test]
    fn test_call_inside_lambda_detected() {
        let source = r#"
def process(x):
    return x * 2

def run():
    fn = lambda x: process(x)
    return fn(1)
"#;
        let (graph, _) = parse_and_populate(source);
        let (process_id, _) = find_symbol(&graph, "process");

        let inbound = inbound_call_count(&graph, process_id);
        assert!(
            inbound >= 1,
            "process() called inside lambda body must be detected. Inbound Calls: {}.",
            inbound
        );
    }

    #[test]
    fn test_name_shadowing_does_not_crash() {
        let source = r#"
def helper():
    return 42

def run():
    helper = 1
    helper()
"#;
        let (graph, _) = parse_and_populate(source);
        // Main assertion: no panic during parse + populate
        let (run_id, _) = find_symbol(&graph, "run");
        let _callees = graph.callees(run_id);
    }

    #[test]
    fn test_dynamic_dispatch_stays_unresolved() {
        let source = r#"
def run(obj):
    func = getattr(obj, 'method')
    func()
"#;
        let (graph, _) = parse_and_populate(source);
        let (run_id, _) = find_symbol(&graph, "run");

        let resolved = graph.callees(run_id);
        let resolved_names: Vec<String> = resolved
            .iter()
            .filter_map(|id| graph.get_symbol(*id))
            .map(|s| s.name.clone())
            .collect();

        assert!(
            resolved_names.is_empty(),
            "Dynamic dispatch must not resolve to any local symbol. Resolved: {:?}.",
            resolved_names
        );
    }

    #[test]
    fn test_module_level_call_detected() {
        let source = r#"
def setup():
    return 42

result = setup()
"#;
        let (graph, _) = parse_and_populate(source);
        let (setup_id, _) = find_symbol(&graph, "setup");

        let inbound = inbound_call_count(&graph, setup_id);
        assert!(
            inbound >= 1,
            "Module-level call setup() must be detected. Inbound Calls: {}.",
            inbound
        );
    }

    // -- P0: Integration with data_dead_end Pattern ---------------------------

    /// Parse fixture content via temp path to avoid /tests/ path exclusion.
    fn parse_fixture_for_pattern(fixture_name: &str) -> Graph {
        let fixture_path = fixture_dir().join(fixture_name);
        let content = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("Failed to read fixture {}: {}", fixture_name, e));

        // Use temp path to avoid is_excluded() /tests/ path check
        let dir = tempfile::tempdir().expect("tempdir");
        let temp_path = dir.path().join(fixture_name);
        std::fs::write(&temp_path, &content).expect("write");

        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&temp_path, &content).expect("parse");

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);
        std::mem::forget(dir);
        graph
    }

    #[test]
    fn test_dead_code_fixture_correct_with_call_detection() {
        use crate::analyzer::patterns::data_dead_end;

        let graph = parse_fixture_for_pattern("dead_code.py");

        let diagnostics = data_dead_end::detect(&graph);
        let flagged_names: Vec<String> = diagnostics.iter().map(|d| d.entity.clone()).collect();

        // True positives: unused_helper, _private_util
        assert!(
            flagged_names.iter().any(|n| n.contains("unused_helper")),
            "unused_helper must be flagged as dead end. Flagged: {:?}",
            flagged_names
        );
        assert!(
            flagged_names.iter().any(|n| n.contains("_private_util")),
            "_private_util must be flagged as dead end. Flagged: {:?}",
            flagged_names
        );

        // True negative: active_function is called by main_handler — NOT dead
        assert!(
            !flagged_names.iter().any(|n| n.contains("active_function")),
            "active_function is called by main_handler — must NOT be flagged. Flagged: {:?}",
            flagged_names
        );

        // True negative: main_handler is excluded as entry point (main_* pattern)
        assert!(
            !flagged_names.iter().any(|n| n.contains("main_handler")),
            "main_handler is excluded by name pattern — must NOT be flagged. Flagged: {:?}",
            flagged_names
        );
    }

    #[test]
    fn test_clean_code_fixture_zero_dead_ends() {
        use crate::analyzer::patterns::data_dead_end;

        let graph = parse_fixture_for_pattern("clean_code.py");

        let diagnostics = data_dead_end::detect(&graph);
        let flagged: Vec<String> = diagnostics.iter().map(|d| d.entity.clone()).collect();

        assert!(
            diagnostics.is_empty(),
            "clean_code.py must produce ZERO dead end diagnostics. Flagged: {:?}.",
            flagged
        );
    }

    // -- P1: Regression Guard — Existing Behavior Preserved -------------------

    #[test]
    fn test_existing_import_references_unchanged_after_call_detection() {
        let path = fixture_dir().join("imports.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        // Count import references from adapter output
        let import_refs = result
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Import)
            .count();

        assert!(
            import_refs >= 5,
            "imports.py must still produce >= 5 Import references. Got {}.",
            import_refs
        );

        // Populate and verify total refs include both imports and any call refs
        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        let total_refs = graph.reference_count();
        assert!(
            total_refs >= import_refs,
            "Graph reference count ({}) must be >= adapter import count ({}).",
            total_refs,
            import_refs
        );
    }
}
