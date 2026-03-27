// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Graph population bridge — translates `ParseResult` into a populated `Graph`.
//!
//! This module provides the critical bridge between language adapters (which produce
//! `ParseResult` with placeholder IDs) and the analysis graph (which assigns real
//! generational IDs via slotmap). The `populate_graph` function handles:
//!
//! - **Scope insertion** with parent reconstruction via location containment
//! - **Symbol insertion** with scope ID remapping
//! - **Reference insertion** with from/to ID remapping and **intra-file resolution**
//! - **Boundary insertion**
//!
//! ## Intra-File Reference Resolution
//!
//! For `ReferenceKind::Call` references (produced by `PythonAdapter::extract_all_calls`):
//! - **`from`** is resolved via location containment — `find_containing_symbol()` finds
//!   the innermost Function/Method whose source range contains the call site
//! - **`to`** is resolved via name matching — `resolve_callee()` matches the callee name
//!   against symbols defined in the same file
//!
//! What stays **unresolved** (`SymbolId::default()`):
//! - Cross-file calls (e.g., `module.func()`) — requires M5 cross-file resolution
//! - Built-in calls (`print`, `len`) — no symbol definition in user code
//! - Dynamic dispatch (`getattr()`) — not statically resolvable
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
/// Module-level calls (where `find_containing_symbol` returns `None`) are attributed to
/// a lazily-created synthetic `<module>` symbol with `SymbolKind::Module`. This eliminates
/// self-referencing edges that occurred when the fallback assigned `from` to the first
/// symbol in the file (which could also be the callee).
///
/// For other references (imports), uses the existing first-symbol fallback for `from`.
fn insert_references(
    graph: &mut Graph,
    references: &[Reference],
    symbol_id_map: &[(usize, SymbolId)],
    symbols: &[Symbol],
) {
    // Lazily-created <module> symbol for module-level calls.
    // Created on first module-level call, reused for subsequent ones in the same file.
    let mut module_symbol_id: Option<SymbolId> = None;

    for reference in references {
        let mut new_ref = reference.clone();

        match &reference.resolution {
            ResolutionStatus::Partial(info) if info.starts_with("call:") => {
                let callee_name = &info[5..]; // after "call:"

                // Resolve from: find the symbol whose location contains this call
                new_ref.from = find_containing_symbol(&reference.location, symbols, symbol_id_map)
                    .unwrap_or_else(|| {
                        // Module-level call: create or reuse the <module> symbol
                        *module_symbol_id
                            .get_or_insert_with(|| create_module_symbol(graph, &reference.location))
                    });

                // Resolve to: match callee name against same-file symbols
                new_ref.to =
                    resolve_callee(callee_name, &new_ref.from, graph, symbol_id_map, symbols);

                // Instance-attribute type resolution fallback:
                // When self.attr.method() fails normal resolution (because resolve_callee
                // can't find a method named "attr.method" on the class), try resolving
                // through instance attribute type annotations from __init__.
                if new_ref.to == SymbolId::default() {
                    if let Some(method_part) = callee_name
                        .strip_prefix("self.")
                        .or_else(|| callee_name.strip_prefix("this."))
                    {
                        if method_part.contains('.') {
                            new_ref.to = resolve_through_instance_attr(
                                method_part,
                                references,
                                graph,
                                symbol_id_map,
                                symbols,
                            );
                        }
                    }
                }

                // Skip adding unresolved call edges — they create phantom edges to
                // SymbolId::default() that pollute callees()/callers() results.
                if new_ref.to == SymbolId::default() {
                    continue;
                }
            }
            ResolutionStatus::Partial(info) if info.starts_with("attribute_access:") => {
                // Attribute access references (e.g., `obj.attr`) are resolved differently
                // from direct calls. The info string encodes "attribute_access:<root_name>"
                // where root_name is the importable symbol (e.g., the module or object that
                // was imported). We extract the root name and resolve it via proximity-based
                // import resolution, connecting the attribute access to the imported symbol.
                let root_name = &info["attribute_access:".len()..];

                // Resolve from: find the containing function/method
                new_ref.from = find_containing_symbol(&reference.location, symbols, symbol_id_map)
                    .unwrap_or_else(|| {
                        *module_symbol_id
                            .get_or_insert_with(|| create_module_symbol(graph, &reference.location))
                    });

                // Resolve to: find the import symbol with matching name
                new_ref.to = resolve_import_by_name(
                    root_name,
                    symbol_id_map,
                    symbols,
                    reference.location.line,
                );
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

/// Creates a synthetic `<module>` symbol representing module-level code.
///
/// Module-level calls (e.g., `result = setup()` at top level) need a real symbol
/// to serve as the caller. The `<module>` symbol uses `SymbolKind::Module`, which
/// is already filtered from pattern entity lists, so it won't appear in diagnostics
/// like `data_dead_end`.
fn create_module_symbol(graph: &mut Graph, ref_location: &Location) -> SymbolId {
    let file_name = ref_location
        .file
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    graph.add_symbol(Symbol {
        id: SymbolId::default(),
        kind: SymbolKind::Module,
        name: "<module>".to_string(),
        qualified_name: format!("{}::<module>", file_name),
        visibility: Visibility::Public,
        signature: None,
        location: Location {
            file: ref_location.file.clone(),
            line: 1,
            column: 1,
            end_line: u32::MAX,
            end_column: 1,
        },
        resolution: ResolutionStatus::Resolved,
        scope: ScopeId::default(),
        annotations: vec![],
    })
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
/// Handles four patterns:
/// - `self.method` — matches `Method` symbols in the same class scope as `from`
/// - `this.method` — same as `self.method` (JavaScript `this` receiver)
/// - `simple_name` — matches any symbol with the same name
/// - `obj.attr` — stays unresolved (cross-file or requires type inference)
fn resolve_callee(
    callee_name: &str,
    from_id: &SymbolId,
    graph: &Graph,
    symbol_id_map: &[(usize, SymbolId)],
    symbols: &[Symbol],
) -> SymbolId {
    // Handle self.method (Python/Rust) and this.method (JavaScript) patterns
    let method_name = callee_name
        .strip_prefix("self.")
        .or_else(|| callee_name.strip_prefix("this."));
    if let Some(method_name) = method_name {
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

/// Resolves `self.attr.method()` through instance-attribute type annotations.
///
/// When `resolve_callee` cannot resolve `self.attr.method()` (because there is no
/// method named `attr.method` on the class), this function checks whether the
/// file's `instance_attr_type:` references map `attr` to a known type, then looks
/// up `method` on that type within the same file.
///
/// `dotted_method` is the portion after `self.` — e.g., `_backend.execute`.
/// Only handles one level of dispatch (`self.attr.method()`). Deeper chains
/// (`self.a.b.c()`) are not supported in v1.
///
/// Returns `SymbolId::default()` if resolution fails (type not annotated, type not
/// found in same file, or method not found on type). The caller drops the edge
/// in this case — existing behavior, no crash.
fn resolve_through_instance_attr(
    dotted_method: &str,
    references: &[Reference],
    graph: &Graph,
    symbol_id_map: &[(usize, SymbolId)],
    symbols: &[Symbol],
) -> SymbolId {
    let (attr_name, method_name) = match dotted_method.split_once('.') {
        Some((a, m)) => (a, m),
        None => return SymbolId::default(),
    };

    // Search references for instance_attr_type:*.attr_name=TypeName
    let type_name = references.iter().find_map(|r| {
        if let ResolutionStatus::Partial(info) = &r.resolution {
            if let Some(rest) = info.strip_prefix("instance_attr_type:") {
                // Format: ClassName.attr_name=TypeName
                if let Some(eq_pos) = rest.rfind('=') {
                    let attr_part = &rest[..eq_pos];
                    let type_part = &rest[eq_pos + 1..];
                    if attr_part.ends_with(&format!(".{}", attr_name)) {
                        return Some(type_part.to_string());
                    }
                }
            }
        }
        None
    });

    let type_name = match type_name {
        Some(t) => t,
        None => return SymbolId::default(),
    };

    // Find method_name on type_name class in same file
    for &(idx, real_id) in symbol_id_map {
        let sym = &symbols[idx];
        if sym.name == method_name && sym.kind == SymbolKind::Method {
            // Check if this method belongs to the type's class by looking at
            // the graph symbol's scope — it should share scope with a class
            // symbol named type_name
            if let Some(graph_sym) = graph.get_symbol(real_id) {
                // Find a class symbol with matching name and scope
                for &(class_idx, class_id) in symbol_id_map {
                    let class_sym = &symbols[class_idx];
                    if class_sym.name == type_name && class_sym.kind == SymbolKind::Class {
                        if let Some(class_graph_sym) = graph.get_symbol(class_id) {
                            // The method's scope should match the class's ID scope
                            // (methods are scoped under their class)
                            if graph_sym.scope == class_graph_sym.scope
                                || sym
                                    .qualified_name
                                    .contains(&format!("{}::{}", type_name, method_name))
                            {
                                return real_id;
                            }
                        }
                    }
                }
            }
        }
    }

    SymbolId::default()
}

/// Resolves an import symbol by name for attribute access references.
///
/// Searches the symbol list for an import symbol (annotated with `"import"`) whose
/// name matches the given root identifier. When multiple imports share the same name
/// (e.g., test functions each importing `use std::path::Path`), returns the one
/// closest to `ref_line` (nearest preceding import by line number). Falls back to
/// any matching import if none precedes the reference. Returns `SymbolId::default()`
/// if no match.
pub(crate) fn resolve_import_by_name(
    root_name: &str,
    symbol_id_map: &[(usize, SymbolId)],
    symbols: &[Symbol],
    ref_line: u32,
) -> SymbolId {
    let mut best_id = SymbolId::default();
    let mut best_line: Option<u32> = None;
    let mut any_match_id = SymbolId::default();

    for &(idx, real_id) in symbol_id_map {
        let sym = &symbols[idx];
        if sym.name == root_name && sym.annotations.contains(&"import".to_string()) {
            let sym_line = sym.location.line;

            // Track any match as fallback (for when no import precedes the reference)
            if any_match_id == SymbolId::default() {
                any_match_id = real_id;
            }

            // Only consider imports at or before the reference line
            if sym_line <= ref_line {
                match best_line {
                    Some(prev_line) if sym_line > prev_line => {
                        // Closer preceding import found
                        best_id = real_id;
                        best_line = Some(sym_line);
                    }
                    None => {
                        // First preceding import
                        best_id = real_id;
                        best_line = Some(sym_line);
                    }
                    _ => {} // Keep existing best (farther or equal)
                }
            }
        }
    }

    if best_id != SymbolId::default() {
        best_id
    } else {
        // No preceding import found — fall back to any matching import
        any_match_id
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

/// Resolves a JS relative import path to a file in the module map.
///
/// For `./provider` imported from `/project/src/consumer.js`, resolves to
/// `/project/src/provider.js` by:
/// 1. Getting the importing file's directory (`/project/src/`)
/// 2. Joining the relative path (`./provider` → `/project/src/provider`)
/// 3. Looking up the resolved path in the module map
///
/// Handles `./`, `../`, and index file resolution. Returns `None` if the
/// module is not found (external or missing file).
fn resolve_js_relative_import(
    import_path: &str,
    importing_file: &std::path::Path,
    module_map: &std::collections::HashMap<String, std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    let import_dir = importing_file.parent()?;

    // Join the relative path with the importing file's directory
    let resolved = import_dir.join(import_path);

    // Normalize the path (resolve ../ components)
    let normalized = normalize_path(&resolved);

    // Try to find in module map by checking each entry's absolute path
    // The module map values are absolute paths; we compare against our resolved path
    for file_path in module_map.values() {
        let file_without_ext = file_path.with_extension("");
        if file_without_ext == normalized {
            return Some(file_path.clone());
        }
        // Also check index file: ./utils → utils/index.js
        if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str()) {
            if file_stem == "index" {
                if let Some(parent) = file_path.parent() {
                    if parent == normalized {
                        return Some(file_path.clone());
                    }
                }
            }
        }
    }

    None
}

/// Normalizes a path by resolving `.` and `..` components without filesystem access.
fn normalize_path(path: &std::path::Path) -> std::path::PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                // Pop the last normal component if possible
                if !components.is_empty() {
                    components.pop();
                }
            }
            std::path::Component::CurDir => {
                // Skip . components
            }
            _ => {
                components.push(component);
            }
        }
    }
    components.iter().collect()
}

/// Resolves a Rust module path (`crate::`, `super::`, `self::`) to a file.
fn resolve_rust_module_path(
    module_name: &str,
    importing_file: &std::path::Path,
    module_map: &std::collections::HashMap<String, std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    if module_name.starts_with("crate::") {
        module_map.get(module_name).cloned()
    } else if module_name.starts_with("super::") || module_name.starts_with("self::") {
        let importing_module_key = find_module_key_for_file(importing_file, module_map)?;
        let resolved = resolve_relative_rust_path(module_name, &importing_module_key);
        module_map.get(&resolved).cloned()
    } else {
        None
    }
}

/// Finds the module map key for a given file path (reverse lookup).
fn find_module_key_for_file(
    file: &std::path::Path,
    module_map: &std::collections::HashMap<String, std::path::PathBuf>,
) -> Option<String> {
    module_map
        .iter()
        .find(|(_, v)| v.as_path() == file)
        .map(|(k, _)| k.clone())
}

/// Resolves `super::` and `self::` to absolute `crate::` paths.
/// Handles chained prefixes like `self::super::` by normalizing first.
fn resolve_relative_rust_path(module_name: &str, importing_module_key: &str) -> String {
    // Normalize self::super → super (self:: is the current module, super:: goes up)
    let normalized = if module_name.starts_with("self::super") {
        &module_name[6..] // strip "self::" prefix, leaving "super" or "super::..."
    } else {
        module_name
    };
    if let Some(rest) = normalized.strip_prefix("self::") {
        format!("{}::{}", importing_module_key, rest)
    } else if normalized == "super" {
        // Bare super (from self::super normalization) → parent module
        if let Some(parent_end) = importing_module_key.rfind("::") {
            importing_module_key[..parent_end].to_string()
        } else {
            "crate".to_string()
        }
    } else if let Some(rest) = normalized.strip_prefix("super::") {
        if let Some(parent_end) = importing_module_key.rfind("::") {
            let parent = &importing_module_key[..parent_end];
            format!("{}::{}", parent, rest)
        } else {
            format!("crate::{}", rest)
        }
    } else {
        module_name.to_string()
    }
}

/// Checks whether `lookup_name` refers to a child module of `parent_module_key`.
///
/// Only applies to Rust-style module paths (containing `::`). Returns `false`
/// for Python/JS imports to avoid cross-language false matches.
fn is_child_module(
    parent_module_key: &str,
    lookup_name: &str,
    module_map: &std::collections::HashMap<String, std::path::PathBuf>,
) -> bool {
    if !parent_module_key.contains("::") {
        return false;
    }
    let child_key = format!("{}::{}", parent_module_key, lookup_name);
    module_map.contains_key(&child_key)
}

/// Resolves Python relative imports (`.b`, `..parent`, `...top`) to module map keys.
///
/// Python relative imports use leading dots to indicate the package level:
/// - `.b` (1 dot) = sibling module in the same package
/// - `..parent` (2 dots) = module in the parent package
/// - `...top` (3 dots) = module two levels up
///
/// The algorithm:
/// 1. Count leading dots to determine depth
/// 2. Find the importing file's module key via reverse lookup
/// 3. Strip `depth` trailing components from the module key to get the base package
/// 4. Append the dotted name (if any) after the dots
/// 5. Look up the resolved key in the module map
fn resolve_python_relative_import(
    module_name: &str,
    importing_file: &std::path::Path,
    module_map: &std::collections::HashMap<String, std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    // Count leading dots
    let dot_count = module_name.chars().take_while(|&c| c == '.').count();
    if dot_count == 0 {
        return None;
    }

    // Extract the name after the dots (may be empty for `from . import x`)
    let after_dots = &module_name[dot_count..];

    // Find the importing file's module key
    let importing_key = find_module_key_for_file(importing_file, module_map)?;

    // Split the importing module key into components
    let components: Vec<&str> = importing_key.split('.').collect();

    // Remove `dot_count` trailing components:
    // 1 dot removes the file's own component (going to its package)
    // 2 dots removes the file + one package level, etc.
    if dot_count > components.len() {
        // Can't go above the root package
        return None;
    }
    let base_components = &components[..components.len() - dot_count];

    // Build the resolved key
    let resolved_key = if after_dots.is_empty() {
        // `from . import x` — resolve to the package itself
        base_components.join(".")
    } else if base_components.is_empty() {
        after_dots.to_string()
    } else {
        format!("{}.{}", base_components.join("."), after_dots)
    };

    if resolved_key.is_empty() {
        return None;
    }

    module_map.get(&resolved_key).cloned()
}

/// Resolves cross-file import references by matching import symbols to definitions
/// in other files via a module-to-file mapping.
///
/// This is the second resolution pass, called after all files are parsed and
/// `populate_graph()` has been called for each. It iterates import symbols (those
/// with `"import"` annotation), extracts their `"from:<module>"` annotation, routes
/// to the appropriate language-specific resolver, then searches the target file's
/// symbols for a matching definition.
///
/// # Language-specific resolution
///
/// Module names are routed based on their prefix format:
/// - **JS/TS relative** (`./`, `../`): resolved via file-system relative path.
/// - **Rust paths** (`crate::`, `super::`, `self::`): resolved via Rust module hierarchy.
/// - **Python relative** (`.b`, `..parent`): dot-prefix converted to package-qualified
///   module key via [`resolve_python_relative_import`].
/// - **Absolute**: direct lookup in the module map (with Rust `self::` fallback).
///
/// # Resolution outcomes
///
/// - **Resolved:** Module found in map AND target symbol found in that file.
///   A cross-file `Reference` edge is created with the real `SymbolId`.
/// - **Partial("module resolved, symbol not found"):** Module file exists but
///   the specific symbol wasn't found (e.g., dynamic attribute).
/// - **Partial("external"):** Module not in map — likely stdlib or third-party.
///   Left unchanged.
/// - **Partial("star import"):** Star imports get module-level resolution only.
///
/// # Idempotency
///
/// Safe to call multiple times. Already-resolved imports are skipped. New edges
/// are only created if the import is not yet resolved.
pub fn resolve_cross_file_imports(
    graph: &mut Graph,
    module_map: &std::collections::HashMap<String, std::path::PathBuf>,
) {
    use std::collections::HashMap;

    if module_map.is_empty() {
        return;
    }

    // Phase 1: Collect import symbols that need resolution.
    // We collect IDs first to avoid borrow checker issues with simultaneous read+write.
    // Tuple: (id, module_name, lookup_name, importing_file_path)
    let mut imports_to_resolve: Vec<(SymbolId, String, String, std::path::PathBuf)> = Vec::new();

    for (id, symbol) in graph.all_symbols() {
        if !symbol.annotations.contains(&"import".to_string()) {
            continue;
        }

        // Skip already resolved imports (idempotency)
        if symbol.resolution == ResolutionStatus::Resolved {
            continue;
        }

        // Extract the "from:<module>" annotation
        let module_name = symbol
            .annotations
            .iter()
            .find_map(|a| a.strip_prefix("from:"))
            .map(|s| s.to_string());

        let Some(module_name) = module_name else {
            continue;
        };

        // Determine the name to look up in the target file.
        // For aliased imports, use "original_name:<name>" if present.
        let lookup_name = symbol
            .annotations
            .iter()
            .find_map(|a| a.strip_prefix("original_name:"))
            .map(|s| s.to_string())
            .unwrap_or_else(|| symbol.name.clone());

        imports_to_resolve.push((id, module_name, lookup_name, symbol.location.file.clone()));
    }

    // Phase 2: Build a reverse index from file path to symbols defined in that file.
    // This avoids repeated graph.symbols_in_file() calls.
    let mut file_symbols_cache: HashMap<std::path::PathBuf, Vec<(SymbolId, String, SymbolKind)>> =
        HashMap::new();
    for (id, symbol) in graph.all_symbols() {
        // Only index non-import definition symbols (functions, classes, methods, etc.)
        if symbol.kind == SymbolKind::Module {
            continue;
        }
        file_symbols_cache
            .entry(symbol.location.file.clone())
            .or_default()
            .push((id, symbol.name.clone(), symbol.kind));
    }

    // Phase 3: Resolve each import.
    let mut new_references: Vec<(SymbolId, SymbolId, Location)> = Vec::new();
    let mut resolution_updates: Vec<(SymbolId, ResolutionStatus)> = Vec::new();

    for (import_id, module_name, lookup_name, importing_file) in &imports_to_resolve {
        // Route to language-specific resolution based on module path format
        let target_file = if module_name.starts_with("./") || module_name.starts_with("../") {
            resolve_js_relative_import(module_name, importing_file, module_map)
        } else if module_name.starts_with("crate::")
            || module_name.starts_with("super::")
            || module_name.starts_with("self::")
        {
            resolve_rust_module_path(module_name, importing_file, module_map)
        } else if module_name.starts_with('.')
            && !module_name.starts_with("./")
            && importing_file.extension().and_then(|e| e.to_str()) == Some("py")
        {
            // Python relative import: `.b`, `..parent`, `...top`
            resolve_python_relative_import(module_name, importing_file, module_map)
        } else {
            let direct = module_map.get(module_name.as_str()).cloned();
            if direct.is_some() {
                direct
            } else if importing_file.extension().and_then(|e| e.to_str()) == Some("rs") {
                // Rust bare paths — try self:: relative fallback
                resolve_rust_module_path(
                    &format!("self::{}", module_name),
                    importing_file,
                    module_map,
                )
            } else {
                None
            }
        };
        let target_file = target_file.as_ref();

        let Some(target_file) = target_file else {
            // Module not found in map — likely stdlib/third-party. Leave as-is.
            continue;
        };

        // Check if this is a star import (name starts with "*:")
        if let Some(symbol) = graph.get_symbol(*import_id) {
            if symbol.name.starts_with("*:") {
                resolution_updates.push((
                    *import_id,
                    ResolutionStatus::Partial("star import - module resolved".to_string()),
                ));
                continue;
            }
        }

        // Look for the target symbol in the target file
        let target_symbols = file_symbols_cache.get(target_file);

        if let Some(symbols) = target_symbols {
            // Find a symbol with matching name. Prefer Function/Method/Class over others.
            let mut best_match: Option<SymbolId> = None;
            let mut any_match: Option<SymbolId> = None;

            for (sym_id, sym_name, sym_kind) in symbols {
                if sym_name == lookup_name {
                    if any_match.is_none() {
                        any_match = Some(*sym_id);
                    }
                    if matches!(
                        sym_kind,
                        SymbolKind::Function | SymbolKind::Method | SymbolKind::Class
                    ) {
                        best_match = Some(*sym_id);
                        break;
                    }
                }
            }

            let target_id = best_match.or(any_match);

            if let Some(target_id) = target_id {
                // Get the import symbol's location for the new reference
                if let Some(import_sym) = graph.get_symbol(*import_id) {
                    new_references.push((*import_id, target_id, import_sym.location.clone()));
                }
                resolution_updates.push((*import_id, ResolutionStatus::Resolved));
            } else if lookup_name == module_name {
                // Module-level import (`import X`) — module file found is enough.
                // No specific symbol to match since we're importing the whole module.
                resolution_updates.push((*import_id, ResolutionStatus::Resolved));
            } else if is_child_module(module_name, lookup_name, module_map) {
                // Child module import: lookup_name is a submodule, not a symbol.
                resolution_updates.push((*import_id, ResolutionStatus::Resolved));
            } else {
                // Module found but symbol not in it
                resolution_updates.push((
                    *import_id,
                    ResolutionStatus::Partial("module resolved, symbol not found".to_string()),
                ));
            }
        } else {
            // Module file exists in map but has no non-import symbols
            // This handles empty modules
            if lookup_name == module_name || lookup_name.starts_with("*:") {
                // Module-level import (e.g., `import utils`) — module found is enough
                resolution_updates.push((*import_id, ResolutionStatus::Resolved));
            } else if is_child_module(module_name, lookup_name, module_map) {
                // Child module fallback for empty parent modules
                resolution_updates.push((*import_id, ResolutionStatus::Resolved));
            } else {
                resolution_updates.push((
                    *import_id,
                    ResolutionStatus::Partial("module resolved, symbol not found".to_string()),
                ));
            }
        }
    }

    // Phase 4: Apply resolution updates and create new edges.
    for (id, new_resolution) in resolution_updates {
        if let Some(symbol) = graph.get_symbol_mut(id) {
            symbol.resolution = new_resolution;
        }
    }

    for (from_id, to_id, location) in new_references {
        graph.add_reference(Reference {
            id: ReferenceId::default(),
            from: from_id,
            to: to_id,
            kind: ReferenceKind::Import,
            location,
            resolution: ResolutionStatus::Resolved,
        });
    }

    // Phase 5: Create transitive call edges through resolved import chains.
    // When a call edge points to an import symbol that was just resolved,
    // create a direct call edge from the caller to the resolved definition.
    // This makes `callers(def_id)` return cross-file callers.
    let mut transitive_calls: Vec<(SymbolId, SymbolId, Location)> = Vec::new();
    for (id, symbol) in graph.all_symbols() {
        if !symbol.annotations.contains(&"import".to_string()) {
            continue;
        }
        if symbol.resolution != ResolutionStatus::Resolved {
            continue;
        }
        // Find the resolved definition target from import edges
        let import_targets: Vec<SymbolId> = graph
            .edges_from(id)
            .iter()
            .filter(|e| e.kind == EdgeKind::References)
            .map(|e| e.target)
            .collect();
        if import_targets.is_empty() {
            continue;
        }
        // Find callers of this import symbol (intra-file call resolution)
        let callers_of_import = graph.callers(id);
        for caller_id in callers_of_import {
            // Skip if transitive edge already exists (idempotency)
            let existing_callees = graph.callees(caller_id);
            if let Some(caller_sym) = graph.get_symbol(caller_id) {
                let loc = caller_sym.location.clone();
                for &target_id in &import_targets {
                    if !existing_callees.contains(&target_id) {
                        transitive_calls.push((caller_id, target_id, loc.clone()));
                    }
                }
            }
        }
    }

    for (caller_id, def_id, location) in transitive_calls {
        graph.add_reference(Reference {
            id: ReferenceId::default(),
            from: caller_id,
            to: def_id,
            kind: ReferenceKind::Call,
            location,
            resolution: ResolutionStatus::Resolved,
        });
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
        use std::path::Path;

        let graph = parse_fixture_for_pattern("dead_code.py");

        let diagnostics = data_dead_end::detect(&graph, Path::new(""));
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
        use std::path::Path;

        let graph = parse_fixture_for_pattern("clean_code.py");

        let diagnostics = data_dead_end::detect(&graph, Path::new(""));
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

    // =========================================================================
    // QA-Foundation Cycle 3: Module-level call attribution + import symbols
    // =========================================================================

    /// Find all symbols matching a predicate.
    fn find_symbols<'a>(
        graph: &'a Graph,
        pred: impl Fn(&Symbol) -> bool,
    ) -> Vec<(SymbolId, &'a Symbol)> {
        graph.all_symbols().filter(|(_, s)| pred(s)).collect()
    }

    /// Find the <module> symbol if it exists.
    fn find_module_symbol(graph: &Graph) -> Option<(SymbolId, &Symbol)> {
        graph
            .all_symbols()
            .find(|(_, s)| s.kind == SymbolKind::Module && s.name == "<module>")
    }

    /// Assert no Module symbol has self-referencing Calls edges.
    fn assert_no_spurious_self_edges(graph: &Graph) {
        for (id, symbol) in graph.all_symbols() {
            let self_edges: Vec<_> = graph
                .edges_from(id)
                .iter()
                .filter(|e| e.target == id && e.kind == EdgeKind::Calls)
                .collect();
            if !self_edges.is_empty() && symbol.kind == SymbolKind::Module {
                panic!(
                    "Module symbol '{}' has self-referencing Calls edge — this is the bug",
                    symbol.name
                );
            }
        }
    }

    // -- Category 1: Module-Level Call Attribution — No Self-Referencing Edges --

    #[test]
    fn test_module_level_call_no_self_reference() {
        let source = r#"
result = setup()

def setup():
    return 42
"#;
        let (graph, _) = parse_and_populate(source);
        let (setup_id, _) = find_symbol(&graph, "setup");
        let self_edges: Vec<_> = graph
            .edges_from(setup_id)
            .iter()
            .filter(|e| e.target == setup_id && e.kind == EdgeKind::Calls)
            .collect();

        assert!(
            self_edges.is_empty(),
            "Module-level `result = setup()` must NOT create self-referencing edge on setup. \
             Found {} self-edges.",
            self_edges.len()
        );
    }

    #[test]
    fn test_module_level_call_attributed_to_module_symbol() {
        let source = r#"
result = setup()

def setup():
    return 42
"#;
        let (graph, _) = parse_and_populate(source);
        let (setup_id, _) = find_symbol(&graph, "setup");

        let callers = graph.callers(setup_id);
        assert!(
            !callers.is_empty(),
            "setup() must have at least one caller from module-level code"
        );

        for caller_id in &callers {
            let caller = graph.get_symbol(*caller_id).expect("caller must exist");
            assert_ne!(
                *caller_id, setup_id,
                "Caller of setup() must NOT be setup itself. Caller is '{}' ({:?})",
                caller.name, caller.kind
            );
            assert_eq!(
                caller.kind,
                SymbolKind::Module,
                "Module-level call must be attributed to a Module symbol, not {:?} '{}'",
                caller.kind,
                caller.name
            );
        }
    }

    #[test]
    fn test_multiple_module_level_calls_same_module_symbol() {
        let source = r#"
a = foo()
b = bar()
c = foo()

def foo():
    return 1

def bar():
    return 2
"#;
        let (graph, _) = parse_and_populate(source);
        let (foo_id, _) = find_symbol(&graph, "foo");
        let (bar_id, _) = find_symbol(&graph, "bar");

        let foo_callers = graph.callers(foo_id);
        let bar_callers = graph.callers(bar_id);

        assert!(!foo_callers.is_empty(), "foo must have module-level caller");
        assert!(!bar_callers.is_empty(), "bar must have module-level caller");

        let module_callers: Vec<_> = foo_callers
            .iter()
            .chain(bar_callers.iter())
            .filter(|id| {
                graph
                    .get_symbol(**id)
                    .map(|s| s.kind == SymbolKind::Module)
                    .unwrap_or(false)
            })
            .collect();
        assert!(
            !module_callers.is_empty(),
            "At least one caller must be Module kind"
        );

        let first = module_callers[0];
        for caller in &module_callers {
            assert_eq!(
                **caller, *first,
                "All module-level calls must share the same <module> symbol"
            );
        }
    }

    #[test]
    fn test_only_module_level_calls_no_functions() {
        let source = r#"
print("hello")
len([1, 2, 3])
"#;
        let (graph, _) = parse_and_populate(source);
        assert_no_spurious_self_edges(&graph);

        let module_sym = find_module_symbol(&graph);
        assert!(
            module_sym.is_some(),
            "File with module-level calls must create a <module> symbol"
        );
    }

    #[test]
    fn test_module_level_forward_reference() {
        let source = r#"
x = later_func()

def other():
    pass

def later_func():
    return 99
"#;
        let (graph, _) = parse_and_populate(source);
        let (later_id, _) = find_symbol(&graph, "later_func");

        let inbound = inbound_call_count(&graph, later_id);
        assert!(
            inbound >= 1,
            "Module-level call to forward-declared later_func must resolve. Inbound: {}",
            inbound
        );
        assert_no_spurious_self_edges(&graph);
    }

    #[test]
    fn test_no_self_referencing_edges_dead_code_fixture() {
        let graph = parse_fixture_for_pattern("dead_code.py");
        assert_no_spurious_self_edges(&graph);
    }

    #[test]
    fn test_no_self_referencing_edges_clean_code_fixture() {
        let graph = parse_fixture_for_pattern("clean_code.py");
        assert_no_spurious_self_edges(&graph);
    }

    #[test]
    fn test_no_self_referencing_edges_isolated_module_fixture() {
        let graph = parse_fixture_for_pattern("isolated_module.py");
        assert_no_spurious_self_edges(&graph);
    }

    // -- Category 2: <module> Symbol Properties --------------------------------

    #[test]
    fn test_module_symbol_properties() {
        let source = r#"
result = setup()

def setup():
    return 42
"#;
        let (graph, _) = parse_and_populate(source);
        let (_, module_sym) = find_module_symbol(&graph)
            .expect("<module> symbol must exist for file with module-level calls");

        assert_eq!(module_sym.kind, SymbolKind::Module);
        assert_eq!(module_sym.name, "<module>");
        assert!(
            module_sym.qualified_name.contains("<module>"),
            "Qualified name '{}' must contain '<module>'",
            module_sym.qualified_name
        );
    }

    #[test]
    fn test_module_symbol_not_flagged_as_dead_end() {
        use crate::analyzer::patterns::data_dead_end;
        use std::path::Path;

        let source = r#"
result = setup()

def setup():
    return 42
"#;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("module_call_test.py");
        std::fs::write(&path, source).expect("write");
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, source).expect("parse");
        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);
        std::mem::forget(dir);

        let diagnostics = data_dead_end::detect(&graph, Path::new(""));
        let flagged: Vec<_> = diagnostics.iter().map(|d| d.entity.clone()).collect();

        assert!(
            !flagged.iter().any(|n| n.contains("<module>")),
            "<module> symbol must NOT be flagged as dead end. Flagged: {:?}",
            flagged
        );
    }

    #[test]
    fn test_no_module_symbol_without_module_level_calls() {
        let source = r#"
def foo():
    return 1

def bar():
    foo()
"#;
        let (graph, _) = parse_and_populate(source);
        let module_sym = find_module_symbol(&graph);

        assert!(
            module_sym.is_none(),
            "<module> symbol must NOT be created when all calls are inside functions. \
             Found: {:?}",
            module_sym.map(|(_, s)| &s.name)
        );
    }

    // -- Category 3: Import Symbol Creation ------------------------------------

    #[test]
    fn test_import_creates_symbol() {
        let source = r#"
import os

def main():
    pass
"#;
        let (graph, _) = parse_and_populate(source);

        let import_syms: Vec<_> = graph
            .all_symbols()
            .filter(|(_, s)| s.annotations.contains(&"import".to_string()))
            .collect();

        assert!(
            !import_syms.is_empty(),
            "`import os` must create a Symbol with 'import' annotation. Found 0 import symbols."
        );

        let os_sym = import_syms.iter().find(|(_, s)| s.name == "os");
        assert!(
            os_sym.is_some(),
            "Import symbol must have name 'os'. Found: {:?}",
            import_syms.iter().map(|(_, s)| &s.name).collect::<Vec<_>>()
        );

        let (_, os) = os_sym.unwrap();
        assert_eq!(
            os.kind,
            SymbolKind::Module,
            "Import symbol must have kind Module"
        );
    }

    #[test]
    fn test_from_import_creates_symbol() {
        let source = r#"
from collections import OrderedDict

def main():
    pass
"#;
        let (graph, _) = parse_and_populate(source);

        let import_syms = find_symbols(&graph, |s| {
            s.annotations.contains(&"import".to_string()) && s.name == "OrderedDict"
        });

        assert_eq!(
            import_syms.len(),
            1,
            "`from collections import OrderedDict` must create exactly 1 import symbol. Found: {:?}",
            import_syms.iter().map(|(_, s)| &s.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_multi_import_creates_multiple_symbols() {
        let source = r#"
import os, sys

def main():
    pass
"#;
        let (graph, _) = parse_and_populate(source);

        let import_syms = find_symbols(&graph, |s| s.annotations.contains(&"import".to_string()));

        assert!(
            import_syms.len() >= 2,
            "`import os, sys` must create at least 2 import symbols. Found {}: {:?}",
            import_syms.len(),
            import_syms.iter().map(|(_, s)| &s.name).collect::<Vec<_>>()
        );

        assert!(
            import_syms.iter().any(|(_, s)| s.name == "os"),
            "Must have import symbol for 'os'"
        );
        assert!(
            import_syms.iter().any(|(_, s)| s.name == "sys"),
            "Must have import symbol for 'sys'"
        );
    }

    #[test]
    fn test_import_symbol_location() {
        let source = "import os\nfrom pathlib import Path\n\ndef main():\n    pass\n";
        let (graph, _) = parse_and_populate(source);

        let os_sym = graph
            .all_symbols()
            .find(|(_, s)| s.annotations.contains(&"import".to_string()) && s.name == "os");
        assert!(os_sym.is_some(), "os import symbol must exist");
        let (_, os) = os_sym.unwrap();
        assert_eq!(os.location.line, 1, "import os is on line 1");

        let path_sym = graph
            .all_symbols()
            .find(|(_, s)| s.annotations.contains(&"import".to_string()) && s.name == "Path");
        assert!(path_sym.is_some(), "Path import symbol must exist");
        let (_, path) = path_sym.unwrap();
        assert_eq!(
            path.location.line, 2,
            "from pathlib import Path is on line 2"
        );
    }

    #[test]
    fn test_aliased_import_uses_alias_name() {
        let source = r#"
import os as operating_system
from pathlib import Path as P

def main():
    pass
"#;
        let (graph, _) = parse_and_populate(source);

        let import_syms = find_symbols(&graph, |s| s.annotations.contains(&"import".to_string()));

        assert!(
            import_syms
                .iter()
                .any(|(_, s)| s.name == "operating_system"),
            "Aliased `import os as operating_system` must use alias. Found: {:?}",
            import_syms.iter().map(|(_, s)| &s.name).collect::<Vec<_>>()
        );
        assert!(
            import_syms.iter().any(|(_, s)| s.name == "P"),
            "Aliased `from pathlib import Path as P` must use alias 'P'. Found: {:?}",
            import_syms.iter().map(|(_, s)| &s.name).collect::<Vec<_>>()
        );
    }

    // -- Category 4: phantom_dependency Integration ----------------------------

    #[test]
    fn test_phantom_dependency_fires_on_unused_imports() {
        use crate::analyzer::patterns::phantom_dependency;
        use std::path::Path;

        let source = r#"
import os
from collections import OrderedDict

def process():
    return "done"
"#;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("phantom_test.py");
        std::fs::write(&path, source).expect("write");
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, source).expect("parse");
        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);
        std::mem::forget(dir);

        let diagnostics = phantom_dependency::detect(&graph, Path::new(""));
        let flagged: Vec<_> = diagnostics.iter().map(|d| d.entity.clone()).collect();

        assert!(
            !diagnostics.is_empty(),
            "phantom_dependency must fire when import symbols exist but are unused. Got 0 diagnostics."
        );
        assert!(
            flagged.iter().any(|n| n == "os"),
            "os must be flagged as phantom. Flagged: {:?}",
            flagged
        );
        assert!(
            flagged.iter().any(|n| n == "OrderedDict"),
            "OrderedDict must be flagged as phantom. Flagged: {:?}",
            flagged
        );
    }

    #[test]
    fn test_phantom_dependency_not_flagged_for_called_import() {
        use crate::analyzer::patterns::phantom_dependency;
        use std::path::Path;

        let source = r#"
from pathlib import Path

def resolve(name):
    p = Path(name)
    return p
"#;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("used_import.py");
        std::fs::write(&path, source).expect("write");
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, source).expect("parse");
        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);
        std::mem::forget(dir);

        let diagnostics = phantom_dependency::detect(&graph, Path::new(""));
        let flagged: Vec<_> = diagnostics.iter().map(|d| d.entity.clone()).collect();

        assert!(
            !flagged.iter().any(|n| n == "Path"),
            "Path is used via Path(name) constructor call — must NOT be flagged. Flagged: {:?}",
            flagged
        );
    }

    #[test]
    fn test_phantom_dependency_unused_import_fixture() {
        use crate::analyzer::patterns::phantom_dependency;
        use std::path::Path;

        let graph = parse_fixture_for_pattern("unused_import.py");
        let diagnostics = phantom_dependency::detect(&graph, Path::new(""));
        let flagged: Vec<_> = diagnostics.iter().map(|d| d.entity.clone()).collect();

        // TRUE POSITIVES (must be flagged)
        assert!(
            flagged.iter().any(|n| n == "os"),
            "os is never used — TRUE POSITIVE. Flagged: {:?}",
            flagged
        );
        assert!(
            flagged.iter().any(|n| n == "OrderedDict"),
            "OrderedDict is never used — TRUE POSITIVE. Flagged: {:?}",
            flagged
        );

        // TRUE NEGATIVE (used via constructor call Path(name))
        assert!(
            !flagged.iter().any(|n| n == "Path"),
            "Path is used via Path(name) — TRUE NEGATIVE. Flagged: {:?}",
            flagged
        );
    }

    // -- Category 5: Regression Guards -----------------------------------------

    #[test]
    fn test_regression_intra_file_call_still_resolves() {
        let source = r#"
def callee():
    return 1

def caller():
    callee()
"#;
        let (graph, _) = parse_and_populate(source);
        let (callee_id, _) = find_symbol(&graph, "callee");
        let (caller_id, _) = find_symbol(&graph, "caller");

        let callees = graph.callees(caller_id);
        assert!(
            callees.contains(&callee_id),
            "Intra-file call resolution must still work. caller's callees: {:?}",
            callees
        );
    }

    #[test]
    fn test_regression_self_method_still_resolves() {
        let source = r#"
class MyClass:
    def process(self):
        self.helper()

    def helper(self):
        pass
"#;
        let (graph, _) = parse_and_populate(source);
        let (process_id, _) = find_symbol(&graph, "process");
        let (helper_id, _) = find_symbol(&graph, "helper");

        let callees = graph.callees(process_id);
        assert!(
            callees.contains(&helper_id),
            "self.helper() must still resolve. process callees: {:?}",
            callees
        );
    }

    #[test]
    fn test_regression_recursive_call_still_works() {
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
            "Recursive call must still create self-loop. Callees: {:?}",
            callees
        );
    }

    #[test]
    fn test_regression_dead_code_fixture_unchanged() {
        use crate::analyzer::patterns::data_dead_end;
        use std::path::Path;

        let graph = parse_fixture_for_pattern("dead_code.py");
        let diagnostics = data_dead_end::detect(&graph, Path::new(""));
        let flagged: Vec<_> = diagnostics.iter().map(|d| d.entity.clone()).collect();

        assert!(
            flagged.iter().any(|n| n.contains("unused_helper")),
            "Regression: unused_helper must still be flagged"
        );
        assert!(
            flagged.iter().any(|n| n.contains("_private_util")),
            "Regression: _private_util must still be flagged"
        );
        assert!(
            !flagged.iter().any(|n| n.contains("active_function")),
            "Regression: active_function must NOT be flagged"
        );
        assert!(
            !flagged.iter().any(|n| n.contains("main_handler")),
            "Regression: main_handler must NOT be flagged"
        );
    }

    #[test]
    fn test_regression_import_references_still_exist() {
        let path = fixture_dir().join("imports.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let import_refs = result
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Import)
            .count();

        assert!(
            import_refs >= 5,
            "Import References must still be produced alongside new import Symbols. Got {}.",
            import_refs
        );
    }

    #[test]
    fn test_regression_basic_functions_symbol_count_unchanged() {
        let path = fixture_dir().join("basic_functions.py");
        let content = std::fs::read_to_string(&path).unwrap();
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, &content).unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        assert_eq!(
            graph.symbol_count(),
            3,
            "basic_functions.py has 3 functions and no imports — symbol count must not change"
        );
    }

    // -- Category 6: Adversarial Edge Cases ------------------------------------

    #[test]
    fn test_module_level_call_in_if_name_main() {
        let source = r#"
def main():
    return 42

if __name__ == "__main__":
    main()
"#;
        let (graph, _) = parse_and_populate(source);
        let (main_id, _) = find_symbol(&graph, "main");

        let callers = graph.callers(main_id);
        for caller_id in &callers {
            assert_ne!(
                *caller_id, main_id,
                "`main()` inside `if __name__` block must NOT self-reference"
            );
        }
        assert_no_spurious_self_edges(&graph);
    }

    #[test]
    fn test_import_inside_function_body() {
        let source = r#"
def lazy_load():
    import json
    return json.loads("{}")
"#;
        let (graph, _) = parse_and_populate(source);

        let import_syms = find_symbols(&graph, |s| {
            s.annotations.contains(&"import".to_string()) && s.name == "json"
        });

        assert!(
            !import_syms.is_empty(),
            "Import inside function body must still create import symbol."
        );
    }

    #[test]
    fn test_star_import_no_crash() {
        let source = r#"
from os import *

def main():
    pass
"#;
        let (graph, _) = parse_and_populate(source);
        let _ = graph.symbol_count();
    }

    #[test]
    fn test_conditional_import_both_branches() {
        let source = r#"
try:
    import ujson as json
except ImportError:
    import json

def parse(data):
    return json.loads(data)
"#;
        let (graph, _) = parse_and_populate(source);

        let import_syms = find_symbols(&graph, |s| s.annotations.contains(&"import".to_string()));

        assert!(
            import_syms.len() >= 2,
            "Both try/except import branches should produce symbols. Found {}: {:?}",
            import_syms.len(),
            import_syms.iter().map(|(_, s)| &s.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_empty_file_no_crash_module_level() {
        let source = "";
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("empty.py");
        std::fs::write(&path, source).expect("write");
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, source).expect("parse");
        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);
        std::mem::forget(dir);

        assert_eq!(graph.symbol_count(), 0);
        assert_no_spurious_self_edges(&graph);
    }

    // -- Category 7: Adversarial Import Edge Cases -----------------------------

    #[test]
    fn test_dotted_import_creates_symbol() {
        let source = r#"
import os.path

def check(f):
    return os.path.exists(f)
"#;
        let (graph, _) = parse_and_populate(source);

        let import_syms = find_symbols(&graph, |s| s.annotations.contains(&"import".to_string()));
        assert!(
            !import_syms.is_empty(),
            "`import os.path` must create at least one import symbol"
        );
    }

    #[test]
    fn test_reimport_same_module() {
        let source = r#"
import os
import os

def main():
    pass
"#;
        let (graph, _) = parse_and_populate(source);

        let os_imports = find_symbols(&graph, |s| {
            s.annotations.contains(&"import".to_string()) && s.name == "os"
        });

        assert!(
            os_imports.len() >= 2,
            "Two `import os` statements should create 2 import symbols. Found: {}",
            os_imports.len()
        );
    }

    #[test]
    fn test_future_import_no_crash() {
        // `from __future__ import annotations` is parsed by tree-sitter-python as
        // `future_import_statement`, not `import_from_statement`. We don't create
        // import symbols for __future__ imports (they're compiler directives).
        // The key assertion is no crash.
        let source = r#"
from __future__ import annotations

def main() -> int:
    return 42
"#;
        let (graph, _) = parse_and_populate(source);
        let _ = graph.symbol_count(); // Must not panic
    }

    #[test]
    fn test_module_calls_and_imports_coexist() {
        let source = r#"
import os
from pathlib import Path

result = setup()

def setup():
    p = Path(".")
    return str(p)
"#;
        let (graph, _) = parse_and_populate(source);

        // Import symbols exist
        let import_syms = find_symbols(&graph, |s| s.annotations.contains(&"import".to_string()));
        assert!(
            import_syms.len() >= 2,
            "Must have import symbols for os and Path"
        );

        // Module-level call works
        let (setup_id, _) = find_symbol(&graph, "setup");
        let inbound = inbound_call_count(&graph, setup_id);
        assert!(inbound >= 1, "Module-level setup() call must be detected");

        // No self-referencing edges
        assert_no_spurious_self_edges(&graph);

        // Path is used via constructor call — should have incoming edge
        let path_sym = graph
            .all_symbols()
            .find(|(_, s)| s.annotations.contains(&"import".to_string()) && s.name == "Path");
        if let Some((path_id, _)) = path_sym {
            let path_inbound = graph
                .edges_to(path_id)
                .iter()
                .filter(|e| matches!(e.kind, EdgeKind::Calls | EdgeKind::References))
                .count();
            assert!(
                path_inbound >= 1,
                "Path used via Path('.') constructor — should have incoming edge. Got {}",
                path_inbound
            );
        }
    }

    // =========================================================================
    // Cross-file resolution tests (M5)
    // =========================================================================

    /// Helper: parse multiple Python files and run cross-file resolution.
    fn cross_file_graph(files: &[(&str, &str)]) -> Graph {
        let adapter = PythonAdapter::new();
        let mut graph = Graph::new();
        let mut paths = Vec::new();

        for (filename, content) in files {
            let path = PathBuf::from(format!("project/{}", filename));
            let result = adapter.parse_file(&path, content).unwrap();
            populate_graph(&mut graph, &result);
            paths.push(path);
        }

        let module_map = crate::build_module_map(&paths);
        resolve_cross_file_imports(&mut graph, &module_map);
        graph
    }

    // -- Category 1: Module-to-File Mapping ----------------------------------

    #[test]
    fn test_module_map_simple_file() {
        let files = vec![PathBuf::from("project/utils.py")];
        let map = crate::build_module_map(&files);
        assert!(map.contains_key("utils"), "must map 'utils' -> utils.py");
        assert_eq!(map["utils"], PathBuf::from("project/utils.py"));
    }

    #[test]
    fn test_module_map_package_init() {
        let files = vec![PathBuf::from("project/my_package/__init__.py")];
        let map = crate::build_module_map(&files);
        assert!(
            map.contains_key("my_package"),
            "must map __init__.py to package name"
        );
    }

    #[test]
    fn test_module_map_nested_module() {
        let files = vec![
            PathBuf::from("project/my_package/__init__.py"),
            PathBuf::from("project/my_package/core.py"),
        ];
        let map = crate::build_module_map(&files);
        assert!(
            map.contains_key("my_package.core"),
            "must map nested module with dot separator"
        );
    }

    #[test]
    fn test_module_map_deeply_nested() {
        let files = vec![
            PathBuf::from("project/a/b/c/d.py"),
            PathBuf::from("project/a/__init__.py"),
        ];
        let map = crate::build_module_map(&files);
        assert!(
            map.contains_key("a.b.c.d"),
            "must handle arbitrary nesting depth"
        );
    }

    #[test]
    fn test_module_map_multiple_files_same_package() {
        let files = vec![
            PathBuf::from("project/pkg/__init__.py"),
            PathBuf::from("project/pkg/models.py"),
            PathBuf::from("project/pkg/views.py"),
        ];
        let map = crate::build_module_map(&files);
        assert_eq!(map.len(), 3, "package init + 2 submodules");
        assert!(map.contains_key("pkg"));
        assert!(map.contains_key("pkg.models"));
        assert!(map.contains_key("pkg.views"));
    }

    #[test]
    fn test_module_map_includes_python_and_js() {
        let files = vec![
            PathBuf::from("project/utils.py"),
            PathBuf::from("project/app.js"),
        ];
        let map = crate::build_module_map(&files);
        assert!(
            map.contains_key("utils"),
            "Python files must be in module map"
        );
        assert!(
            map.contains_key("app"),
            "JS files must be in module map (cross-file resolution)"
        );
    }

    #[test]
    fn test_module_map_empty_input() {
        let files: Vec<PathBuf> = vec![];
        let map = crate::build_module_map(&files);
        assert!(map.is_empty(), "empty input must produce empty map");
    }

    #[test]
    fn test_module_map_name_collision_different_packages() {
        let files = vec![
            PathBuf::from("project/pkg1/utils.py"),
            PathBuf::from("project/pkg2/utils.py"),
        ];
        let map = crate::build_module_map(&files);
        assert!(
            map.contains_key("pkg1.utils"),
            "qualified module names prevent collision"
        );
        assert!(
            map.contains_key("pkg2.utils"),
            "qualified module names prevent collision"
        );
    }

    // -- Category 2: Adapter Annotation Enhancement --------------------------

    #[test]
    fn test_import_from_annotation_stored() {
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("test.py");
        let result = adapter
            .parse_file(&path, "from utils import helper")
            .unwrap();

        let helper_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "helper" && s.annotations.contains(&"import".to_string()))
            .expect("import symbol must exist");

        assert!(
            helper_sym.annotations.contains(&"from:utils".to_string()),
            "must have from:utils annotation, got {:?}",
            helper_sym.annotations
        );
    }

    #[test]
    fn test_plain_import_annotation_stored() {
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("test.py");
        let result = adapter.parse_file(&path, "import os").unwrap();

        let os_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "os" && s.annotations.contains(&"import".to_string()))
            .expect("import symbol must exist");

        assert!(
            os_sym.annotations.contains(&"from:os".to_string()),
            "must have from:os annotation, got {:?}",
            os_sym.annotations
        );
    }

    #[test]
    fn test_dotted_module_import_annotation() {
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("test.py");
        let result = adapter
            .parse_file(&path, "from os.path import join")
            .unwrap();

        let join_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "join" && s.annotations.contains(&"import".to_string()))
            .expect("import symbol must exist");

        assert!(
            join_sym.annotations.contains(&"from:os.path".to_string()),
            "must store dotted module name verbatim, got {:?}",
            join_sym.annotations
        );
    }

    #[test]
    fn test_aliased_import_original_name() {
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("test.py");
        let result = adapter
            .parse_file(&path, "from utils import helper as h")
            .unwrap();

        let h_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "h" && s.annotations.contains(&"import".to_string()))
            .expect("aliased import symbol must exist");

        assert!(
            h_sym.annotations.contains(&"from:utils".to_string()),
            "must have from:utils annotation"
        );
        assert!(
            h_sym
                .annotations
                .contains(&"original_name:helper".to_string()),
            "must have original_name:helper annotation, got {:?}",
            h_sym.annotations
        );
    }

    #[test]
    fn test_multiple_imports_same_module_annotated() {
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("test.py");
        let result = adapter
            .parse_file(&path, "from utils import helper, processor, validator")
            .unwrap();

        let import_syms: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.annotations.contains(&"import".to_string()))
            .collect();

        assert_eq!(import_syms.len(), 3, "must have 3 import symbols");
        for sym in &import_syms {
            assert!(
                sym.annotations.contains(&"from:utils".to_string()),
                "each import must have from:utils, got {:?}",
                sym.annotations
            );
        }
    }

    #[test]
    fn test_star_import_annotation() {
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("test.py");
        let result = adapter.parse_file(&path, "from utils import *").unwrap();

        // Star import creates a reference
        let star_ref = result
            .references
            .iter()
            .find(|r| r.kind == ReferenceKind::Import)
            .expect("star import reference must exist");
        assert_eq!(
            star_ref.resolution,
            ResolutionStatus::Partial("star import".to_string())
        );

        // Star import should also create a symbol with from: annotation
        let star_sym = result
            .symbols
            .iter()
            .find(|s| s.annotations.contains(&"from:utils".to_string()));
        assert!(
            star_sym.is_some(),
            "star import should create symbol with from: annotation"
        );
    }

    #[test]
    fn test_aliased_module_import_annotation() {
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("test.py");
        let result = adapter.parse_file(&path, "import numpy as np").unwrap();

        let np_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "np" && s.annotations.contains(&"import".to_string()))
            .expect("aliased module import symbol must exist");

        assert!(
            np_sym.annotations.contains(&"from:numpy".to_string()),
            "must have from:numpy annotation, got {:?}",
            np_sym.annotations
        );
        assert!(
            np_sym
                .annotations
                .contains(&"original_name:numpy".to_string()),
            "must have original_name:numpy annotation, got {:?}",
            np_sym.annotations
        );
    }

    // -- Category 3: Cross-File Resolution Pass ------------------------------

    #[test]
    fn test_cross_file_import_resolves_basic() {
        let graph = cross_file_graph(&[
            ("main.py", "from utils import helper\nhelper()"),
            ("utils.py", "def helper():\n    pass"),
        ]);

        let (import_id, import_sym) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "helper" && s.annotations.contains(&"import".to_string()))
            .expect("import symbol must exist");

        assert_eq!(
            import_sym.resolution,
            ResolutionStatus::Resolved,
            "import must be resolved after cross-file pass"
        );

        // The cross-file edge is from the import symbol to the target definition.
        // Check incoming edges on the target definition in utils.py.
        let (target_id, _target_sym) = graph
            .all_symbols()
            .find(|(_, s)| {
                s.name == "helper"
                    && s.kind == SymbolKind::Function
                    && s.qualified_name.contains("utils.py")
            })
            .expect("utils.py::helper definition must exist");

        // The cross-file reference creates an edge from import_id -> target_id
        // Check that target has incoming References edge from import
        let incoming = graph.edges_to(target_id);
        let cross_file_incoming = incoming
            .iter()
            .find(|e| e.kind == EdgeKind::References && e.target == import_id);

        assert!(
            cross_file_incoming.is_some(),
            "target must have incoming cross-file edge from import symbol"
        );

        // Also verify outgoing edge from import symbol
        let outgoing = graph.edges_from(import_id);
        let cross_file_out = outgoing
            .iter()
            .find(|e| e.kind == EdgeKind::References && e.target == target_id);

        assert!(
            cross_file_out.is_some(),
            "import must have outgoing cross-file edge to target. \
             Outgoing edges: {:?}",
            outgoing
        );
    }

    #[test]
    fn test_cross_file_aliased_import_resolves() {
        let graph = cross_file_graph(&[
            ("main.py", "from utils import helper as h\nh()"),
            ("utils.py", "def helper():\n    pass"),
        ]);

        let (import_id, import_sym) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "h" && s.annotations.contains(&"import".to_string()))
            .expect("aliased import symbol must exist");

        assert_eq!(
            import_sym.resolution,
            ResolutionStatus::Resolved,
            "aliased import must resolve via original_name"
        );

        // Verify the definition in utils.py has an incoming edge from the import
        let (target_id, _) = graph
            .all_symbols()
            .find(|(_, s)| {
                s.name == "helper"
                    && s.kind == SymbolKind::Function
                    && s.qualified_name.contains("utils.py")
            })
            .expect("utils.py::helper must exist");

        let incoming = graph.edges_to(target_id);
        let cross_file_edge = incoming
            .iter()
            .find(|e| e.kind == EdgeKind::References && e.target == import_id);
        assert!(
            cross_file_edge.is_some(),
            "aliased import must create cross-file edge to 'helper' definition"
        );
    }

    #[test]
    fn test_cross_file_module_import_resolves() {
        let graph = cross_file_graph(&[
            ("main.py", "import utils"),
            ("utils.py", "def helper():\n    pass"),
        ]);

        let import_sym = graph
            .all_symbols()
            .find(|(_, s)| s.name == "utils" && s.annotations.contains(&"import".to_string()));

        assert!(import_sym.is_some(), "import utils symbol must exist");
        let (_, sym) = import_sym.unwrap();
        // Module-level import — the import name IS the module name.
        // Resolution depends on whether "utils" is in the module map AND
        // the lookup_name matches module_name.
        assert_ne!(
            sym.resolution,
            ResolutionStatus::Partial("import".to_string()),
            "resolution must have been updated from initial state"
        );
    }

    #[test]
    fn test_cross_file_missing_module_stays_external() {
        let graph = cross_file_graph(&[("main.py", "from nonexistent import foo")]);

        let (_, import_sym) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "foo" && s.annotations.contains(&"import".to_string()))
            .expect("import symbol must exist");

        // Import symbol starts with Partial("import") from the adapter.
        // When the module is not found in the map, the resolution stays unchanged.
        assert!(
            matches!(
                import_sym.resolution,
                ResolutionStatus::Partial(ref s) if s == "import" || s == "external"
            ),
            "missing module import must stay partial, got {:?}",
            import_sym.resolution
        );
    }

    #[test]
    fn test_cross_file_module_found_symbol_missing() {
        let graph = cross_file_graph(&[
            ("main.py", "from utils import nonexistent_func"),
            ("utils.py", "def helper():\n    pass"),
        ]);

        let (_, import_sym) = graph
            .all_symbols()
            .find(|(_, s)| {
                s.name == "nonexistent_func" && s.annotations.contains(&"import".to_string())
            })
            .expect("import symbol must exist");

        assert_eq!(
            import_sym.resolution,
            ResolutionStatus::Partial("module resolved, symbol not found".to_string()),
            "module found but symbol missing must be partial"
        );
    }

    #[test]
    fn test_cross_file_star_import_partially_resolves() {
        let graph = cross_file_graph(&[
            ("main.py", "from utils import *"),
            (
                "utils.py",
                "def helper():\n    pass\ndef processor():\n    pass",
            ),
        ]);

        let star_sym = graph.all_symbols().find(|(_, s)| {
            s.name.starts_with("*:") && s.annotations.contains(&"import".to_string())
        });

        if let Some((_, sym)) = star_sym {
            assert!(
                matches!(sym.resolution, ResolutionStatus::Partial(ref s) if s.contains("star import")),
                "star import must be partial, not resolved. Got {:?}",
                sym.resolution
            );
        }
    }

    #[test]
    fn test_cross_file_multiple_imports_same_module() {
        let graph = cross_file_graph(&[
            (
                "main.py",
                "from utils import helper, processor\nhelper()\nprocessor()",
            ),
            (
                "utils.py",
                "def helper():\n    pass\ndef processor():\n    pass",
            ),
        ]);

        let helper_import = graph
            .all_symbols()
            .find(|(_, s)| s.name == "helper" && s.annotations.contains(&"import".to_string()));
        let processor_import = graph
            .all_symbols()
            .find(|(_, s)| s.name == "processor" && s.annotations.contains(&"import".to_string()));

        assert!(helper_import.is_some(), "helper import must exist");
        assert!(processor_import.is_some(), "processor import must exist");

        let (_, h) = helper_import.unwrap();
        let (_, p) = processor_import.unwrap();
        assert_eq!(h.resolution, ResolutionStatus::Resolved);
        assert_eq!(p.resolution, ResolutionStatus::Resolved);
    }

    #[test]
    fn test_cross_file_import_class_resolves() {
        let graph = cross_file_graph(&[
            ("main.py", "from models import User"),
            (
                "models.py",
                "class User:\n    def __init__(self):\n        pass",
            ),
        ]);

        let (_, import_sym) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "User" && s.annotations.contains(&"import".to_string()))
            .expect("User import must exist");

        assert_eq!(
            import_sym.resolution,
            ResolutionStatus::Resolved,
            "class import must resolve"
        );
    }

    #[test]
    fn test_cross_file_dotted_module_resolves() {
        let graph = cross_file_graph(&[
            ("main.py", "from pkg.sub import func"),
            ("pkg/__init__.py", ""),
            ("pkg/sub.py", "def func():\n    pass"),
        ]);

        let (_, import_sym) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "func" && s.annotations.contains(&"import".to_string()))
            .expect("func import must exist");

        assert_eq!(
            import_sym.resolution,
            ResolutionStatus::Resolved,
            "dotted module import must resolve"
        );
    }

    #[test]
    fn test_cross_file_resolution_idempotent() {
        let adapter = PythonAdapter::new();
        let mut graph = Graph::new();
        let mut paths = Vec::new();

        for (filename, content) in &[
            ("main.py", "from utils import helper\nhelper()"),
            ("utils.py", "def helper():\n    pass"),
        ] {
            let path = PathBuf::from(format!("project/{}", filename));
            let result = adapter.parse_file(&path, content).unwrap();
            populate_graph(&mut graph, &result);
            paths.push(path);
        }

        let module_map = crate::build_module_map(&paths);

        // First pass
        resolve_cross_file_imports(&mut graph, &module_map);
        let ref_count_1 = graph.reference_count();

        // Second pass — should be a no-op
        resolve_cross_file_imports(&mut graph, &module_map);
        let ref_count_2 = graph.reference_count();

        assert_eq!(
            ref_count_1, ref_count_2,
            "idempotent: second pass must not create duplicate edges"
        );
    }

    // -- Category 4: False Positive Suppression ------------------------------

    #[test]
    fn test_data_dead_end_suppressed_cross_file() {
        use crate::analyzer::patterns::data_dead_end;
        use std::path::Path;

        let graph = cross_file_graph(&[
            ("main.py", "from utils import helper\nhelper()"),
            ("utils.py", "def helper():\n    return 42"),
        ]);

        let diagnostics = data_dead_end::detect(&graph, Path::new("project"));

        let helper_dead_end = diagnostics
            .iter()
            .find(|d| d.entity.contains("helper") && d.entity.contains("utils"));

        assert!(
            helper_dead_end.is_none(),
            "data_dead_end must NOT fire on utils.py::helper with cross-file caller"
        );
    }

    #[test]
    fn test_phantom_dependency_suppressed_cross_file() {
        use crate::analyzer::patterns::phantom_dependency;
        use std::path::Path;

        let graph = cross_file_graph(&[
            ("main.py", "from utils import helper\nhelper()"),
            ("utils.py", "def helper():\n    pass"),
        ]);

        let diagnostics = phantom_dependency::detect(&graph, Path::new("project"));

        let helper_phantom = diagnostics.iter().find(|d| d.entity == "helper");

        assert!(
            helper_phantom.is_none(),
            "phantom_dependency must NOT fire on resolved cross-file import"
        );
    }

    #[test]
    fn test_data_dead_end_true_positive_preserved() {
        use crate::analyzer::patterns::data_dead_end;
        use std::path::Path;

        let graph = cross_file_graph(&[
            ("main.py", "from utils import helper\nhelper()"),
            (
                "utils.py",
                "def helper():\n    pass\ndef unused_func():\n    pass",
            ),
        ]);

        let diagnostics = data_dead_end::detect(&graph, Path::new("project"));

        let unused_dead_end = diagnostics
            .iter()
            .find(|d| d.entity.contains("unused_func"));

        assert!(
            unused_dead_end.is_some(),
            "data_dead_end MUST fire on truly unused utils.py::unused_func"
        );

        let helper_dead_end = diagnostics
            .iter()
            .find(|d| d.entity.contains("helper") && d.entity.contains("utils"));

        assert!(
            helper_dead_end.is_none(),
            "data_dead_end must NOT fire on utils.py::helper (has caller)"
        );
    }

    #[test]
    fn test_phantom_dependency_true_positive_preserved() {
        use crate::analyzer::patterns::phantom_dependency;

        let adapter = PythonAdapter::new();
        let path = PathBuf::from("project/main.py");
        let result = adapter.parse_file(&path, "import os").unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);
        // No cross-file resolution (single file, os is stdlib)

        let diagnostics = phantom_dependency::detect(&graph, std::path::Path::new("project"));

        let os_phantom = diagnostics.iter().find(|d| d.entity == "os");
        assert!(
            os_phantom.is_some(),
            "phantom_dependency MUST fire on unused 'import os'"
        );
    }

    #[test]
    fn test_data_dead_end_multiple_cross_file_callers() {
        use crate::analyzer::patterns::data_dead_end;
        use std::path::Path;

        let graph = cross_file_graph(&[
            ("a.py", "from utils import helper\nhelper()"),
            ("b.py", "from utils import helper\nhelper()"),
            ("utils.py", "def helper():\n    pass"),
        ]);

        let diagnostics = data_dead_end::detect(&graph, Path::new("project"));

        let helper_dead_end = diagnostics
            .iter()
            .find(|d| d.entity.contains("helper") && d.entity.contains("utils"));

        assert!(
            helper_dead_end.is_none(),
            "data_dead_end must NOT fire with multiple cross-file callers"
        );
    }

    #[test]
    fn test_phantom_dependency_missing_module_import() {
        use crate::analyzer::patterns::phantom_dependency;
        use std::path::Path;

        let graph = cross_file_graph(&[("main.py", "from nonexistent import foo\nfoo()")]);

        let diagnostics = phantom_dependency::detect(&graph, Path::new("project"));

        // foo is used (called), so phantom_dependency should NOT fire
        // even though the import is unresolved cross-file
        let foo_phantom = diagnostics.iter().find(|d| d.entity == "foo");
        assert!(
            foo_phantom.is_none(),
            "phantom_dependency should not fire on used import even if unresolved"
        );
    }

    // -- Category 5: Adversarial Edge Cases ----------------------------------

    #[test]
    fn test_circular_cross_file_imports_no_infinite_loop() {
        let graph = cross_file_graph(&[
            ("a.py", "from b import func_b\ndef func_a():\n    func_b()"),
            ("b.py", "from a import func_a\ndef func_b():\n    func_a()"),
        ]);

        // Both should resolve without infinite loop
        let a_import = graph
            .all_symbols()
            .find(|(_, s)| s.name == "func_b" && s.annotations.contains(&"import".to_string()));
        let b_import = graph
            .all_symbols()
            .find(|(_, s)| s.name == "func_a" && s.annotations.contains(&"import".to_string()));

        assert!(a_import.is_some(), "a.py import must exist");
        assert!(b_import.is_some(), "b.py import must exist");

        let (_, a_sym) = a_import.unwrap();
        let (_, b_sym) = b_import.unwrap();
        assert_eq!(a_sym.resolution, ResolutionStatus::Resolved);
        assert_eq!(b_sym.resolution, ResolutionStatus::Resolved);
    }

    #[test]
    fn test_self_import_resolves() {
        let graph = cross_file_graph(&[(
            "main.py",
            "from main import helper\ndef helper():\n    pass",
        )]);

        let (_, import_sym) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "helper" && s.annotations.contains(&"import".to_string()))
            .expect("self-import symbol must exist");

        assert_eq!(
            import_sym.resolution,
            ResolutionStatus::Resolved,
            "self-import must resolve to same-file definition"
        );
    }

    #[test]
    fn test_import_from_empty_module() {
        let graph =
            cross_file_graph(&[("main.py", "from empty import something"), ("empty.py", "")]);

        let (_, import_sym) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "something" && s.annotations.contains(&"import".to_string()))
            .expect("import symbol must exist");

        assert_eq!(
            import_sym.resolution,
            ResolutionStatus::Partial("module resolved, symbol not found".to_string()),
            "import from empty module must be partial"
        );
    }

    #[test]
    fn test_deeply_nested_package_resolution() {
        let graph = cross_file_graph(&[
            ("main.py", "from a.b.c.d import deep_func"),
            ("a/__init__.py", ""),
            ("a/b/__init__.py", ""),
            ("a/b/c/__init__.py", ""),
            ("a/b/c/d.py", "def deep_func():\n    pass"),
        ]);

        let (_, import_sym) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "deep_func" && s.annotations.contains(&"import".to_string()))
            .expect("deep_func import must exist");

        assert_eq!(
            import_sym.resolution,
            ResolutionStatus::Resolved,
            "deeply nested module import must resolve"
        );
    }

    #[test]
    fn test_import_and_call_resolution_independent() {
        let graph = cross_file_graph(&[
            (
                "main.py",
                "from utils import helper\ndef main():\n    helper()\n    local_func()\ndef local_func():\n    pass",
            ),
            ("utils.py", "def helper():\n    pass"),
        ]);

        // Cross-file import should resolve
        let (_import_id, import_sym) = graph
            .all_symbols()
            .find(|(_, s)| s.name == "helper" && s.annotations.contains(&"import".to_string()))
            .expect("helper import must exist");
        assert_eq!(import_sym.resolution, ResolutionStatus::Resolved);

        // Intra-file call to local_func should still work
        let main_sym = graph
            .all_symbols()
            .find(|(_, s)| s.name == "main" && s.kind == SymbolKind::Function);
        assert!(main_sym.is_some(), "main function must exist");

        let local_func = graph.all_symbols().find(|(_, s)| {
            s.name == "local_func"
                && s.kind == SymbolKind::Function
                && s.qualified_name.contains("main.py")
        });
        assert!(local_func.is_some(), "local_func must exist");

        // Verify local_func has an inbound call edge (from main)
        if let Some((lf_id, _)) = local_func {
            let callers = graph.callers(lf_id);
            assert!(
                !callers.is_empty(),
                "local_func must have callers (intra-file resolution still works)"
            );
        }
    }

    // -- Category 6: Regression Guards ---------------------------------------

    #[test]
    fn test_single_file_analysis_unchanged() {
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("project/single.py");
        let content = "def foo():\n    pass\ndef bar():\n    foo()";

        let result = adapter.parse_file(&path, content).unwrap();
        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        let sym_count_before = graph.symbol_count();
        let ref_count_before = graph.reference_count();

        // Run cross-file resolution with empty module map (single file)
        let module_map = crate::build_module_map(&[path]);
        resolve_cross_file_imports(&mut graph, &module_map);

        assert_eq!(
            graph.symbol_count(),
            sym_count_before,
            "single file: symbol count must not change"
        );
        assert_eq!(
            graph.reference_count(),
            ref_count_before,
            "single file: reference count must not change (no cross-file imports)"
        );
    }

    #[test]
    fn test_populate_graph_multiple_calls_still_additive() {
        let adapter = PythonAdapter::new();
        let path1 = PathBuf::from("project/file1.py");
        let path2 = PathBuf::from("project/file2.py");

        let result1 = adapter
            .parse_file(&path1, "def func1():\n    pass")
            .unwrap();
        let result2 = adapter
            .parse_file(&path2, "def func2():\n    pass")
            .unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result1);
        let count1 = graph.symbol_count();
        populate_graph(&mut graph, &result2);
        let count2 = graph.symbol_count();

        assert_eq!(
            count2,
            count1 + 1,
            "second populate must be additive, not replace"
        );
    }
}
