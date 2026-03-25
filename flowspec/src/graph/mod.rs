// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Persistent in-memory analysis graph.
//!
//! The graph is the source of truth for all analysis. It stores symbols, scopes,
//! references, and boundaries in flat tables (slotmap arenas) for O(1) lookup
//! and cache-friendly iteration. Adjacency lists provide bidirectional edge
//! traversal. File-to-symbols mappings enable incremental updates.
//!
//! Design follows the ECS-inspired pattern: symbols are IDs, properties are
//! data alongside, and the graph is a passive data store that analyzers query.

mod populate;

pub use populate::populate_graph;
pub use populate::resolve_cross_file_imports;
#[allow(unused_imports)]
// Used by cycle14_surface_tests via crate::graph::resolve_import_by_name
pub(crate) use populate::resolve_import_by_name;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use slotmap::SlotMap;

use crate::parser::ir::*;

/// The core analysis graph — the source of truth for all Flowspec analysis.
///
/// Stores symbols, scopes, boundaries, and references in flat slotmap arenas
/// for O(1) lookup and cache-friendly iteration. Bidirectional adjacency
/// lists (`outgoing`/`incoming`) enable efficient edge traversal in both
/// directions. File-to-symbol mappings support incremental updates.
///
/// # Design
///
/// ECS-inspired data-oriented design: symbols are IDs, properties are data
/// alongside, and the graph is a passive data store that analyzers query.
/// The graph does not contain analysis logic — diagnostic patterns and flow
/// tracers operate on it externally.
///
/// # Key query methods
///
/// | Method | Returns |
/// |---|---|
/// | [`all_symbols()`](Self::all_symbols) | Iterator over all `(SymbolId, &Symbol)` pairs |
/// | [`callees()`](Self::callees) | Symbols called by a given symbol (`EdgeKind::Calls`) |
/// | [`callers()`](Self::callers) | Symbols that call a given symbol |
/// | [`importers()`](Self::importers) | Symbols that import a given symbol (cross-file) |
/// | [`edges_from()`](Self::edges_from) | All outgoing edges (any kind) |
/// | [`edges_to()`](Self::edges_to) | All incoming edges (any kind) |
/// | [`symbols_in_file()`](Self::symbols_in_file) | All symbols defined in a file |
/// | [`connected_components()`](Self::connected_components) | Undirected connected components |
/// | [`detect_cycles()`](Self::detect_cycles) | Directed cycle detection via DFS coloring |
#[derive(Debug, Clone, Default)]
pub struct Graph {
    symbols: SlotMap<SymbolId, Symbol>,
    scopes: SlotMap<ScopeId, Scope>,
    boundaries: SlotMap<BoundaryId, Boundary>,
    references: SlotMap<ReferenceId, Reference>,

    outgoing: HashMap<SymbolId, Vec<Edge>>,
    incoming: HashMap<SymbolId, Vec<Edge>>,

    scope_symbols: HashMap<ScopeId, Vec<SymbolId>>,
    scope_children: HashMap<ScopeId, Vec<ScopeId>>,

    file_symbols: HashMap<PathBuf, Vec<SymbolId>>,
    file_scopes: HashMap<PathBuf, Vec<ScopeId>>,
}

impl Graph {
    /// Creates a new empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    // -- Symbols ------------------------------------------------------------

    /// Inserts a symbol into the graph and returns its assigned ID.
    pub fn add_symbol(&mut self, mut symbol: Symbol) -> SymbolId {
        let scope = symbol.scope;
        let file = symbol.location.file.clone();
        let id = self.symbols.insert_with_key(|key| {
            symbol.id = key;
            symbol
        });
        self.file_symbols.entry(file).or_default().push(id);
        self.scope_symbols.entry(scope).or_default().push(id);
        id
    }

    /// Returns the symbol with the given ID, or `None`.
    pub fn get_symbol(&self, id: SymbolId) -> Option<&Symbol> {
        self.symbols.get(id)
    }

    /// Returns a mutable reference to the symbol, or `None`.
    pub fn get_symbol_mut(&mut self, id: SymbolId) -> Option<&mut Symbol> {
        self.symbols.get_mut(id)
    }

    /// Removes a symbol and cleans up all edges and index entries.
    pub fn remove_symbol(&mut self, id: SymbolId) {
        let Some(symbol) = self.symbols.remove(id) else {
            return;
        };

        if let Some(file_syms) = self.file_symbols.get_mut(&symbol.location.file) {
            file_syms.retain(|&s| s != id);
        }
        if let Some(scope_syms) = self.scope_symbols.get_mut(&symbol.scope) {
            scope_syms.retain(|&s| s != id);
        }

        // Remove outgoing edges and clean up matching incoming entries
        if let Some(out_edges) = self.outgoing.remove(&id) {
            for edge in &out_edges {
                if let Some(inc) = self.incoming.get_mut(&edge.target) {
                    inc.retain(|e| e.target != id);
                }
            }
        }

        // Remove incoming edges and clean up matching outgoing entries
        if let Some(inc_edges) = self.incoming.remove(&id) {
            for edge in &inc_edges {
                if let Some(out) = self.outgoing.get_mut(&edge.target) {
                    out.retain(|e| e.target != id);
                }
            }
        }
    }

    /// Returns the total number of symbols.
    pub fn symbol_count(&self) -> usize {
        self.symbols.len()
    }

    /// Iterates over all (id, symbol) pairs.
    pub fn all_symbols(&self) -> impl Iterator<Item = (SymbolId, &Symbol)> {
        self.symbols.iter()
    }

    // -- Scopes -------------------------------------------------------------

    /// Inserts a scope and returns its assigned ID.
    pub fn add_scope(&mut self, mut scope: Scope) -> ScopeId {
        let parent = scope.parent;
        let file = scope.location.file.clone();
        let id = self.scopes.insert_with_key(|key| {
            scope.id = key;
            scope
        });
        if let Some(parent_id) = parent {
            self.scope_children.entry(parent_id).or_default().push(id);
        }
        self.file_scopes.entry(file).or_default().push(id);
        id
    }

    /// Returns the scope with the given ID, or `None`.
    pub fn get_scope(&self, id: ScopeId) -> Option<&Scope> {
        self.scopes.get(id)
    }

    /// Returns the total number of scopes.
    pub fn scope_count(&self) -> usize {
        self.scopes.len()
    }

    /// Returns child scopes of the given scope.
    pub fn scope_children(&self, id: ScopeId) -> Vec<ScopeId> {
        self.scope_children.get(&id).cloned().unwrap_or_default()
    }

    /// Returns symbols contained in the given scope.
    pub fn symbols_in_scope(&self, id: ScopeId) -> Vec<SymbolId> {
        self.scope_symbols.get(&id).cloned().unwrap_or_default()
    }

    // -- References (creates edges) -----------------------------------------

    /// Inserts a reference and creates bidirectional edges.
    pub fn add_reference(&mut self, mut reference: Reference) -> ReferenceId {
        let from = reference.from;
        let to = reference.to;
        let kind = reference.kind;

        let id = self.references.insert_with_key(|key| {
            reference.id = key;
            reference
        });

        let edge_kind = ref_kind_to_edge_kind(kind);

        self.outgoing.entry(from).or_default().push(Edge {
            kind: edge_kind,
            target: to,
            reference_id: Some(id),
        });

        self.incoming.entry(to).or_default().push(Edge {
            kind: edge_kind,
            target: from,
            reference_id: Some(id),
        });

        id
    }

    /// Returns the reference with the given ID, or `None`.
    pub fn get_reference(&self, id: ReferenceId) -> Option<&Reference> {
        self.references.get(id)
    }

    /// Returns the total number of references.
    pub fn reference_count(&self) -> usize {
        self.references.len()
    }

    // -- Boundaries ---------------------------------------------------------

    /// Inserts a boundary and returns its assigned ID.
    pub fn add_boundary(&mut self, mut boundary: Boundary) -> BoundaryId {
        self.boundaries.insert_with_key(|key| {
            boundary.id = key;
            boundary
        })
    }

    // -- Edge queries -------------------------------------------------------

    /// Returns all symbols called by the given symbol.
    pub fn callees(&self, id: SymbolId) -> Vec<SymbolId> {
        self.outgoing
            .get(&id)
            .map(|edges| {
                edges
                    .iter()
                    .filter(|e| e.kind == EdgeKind::Calls)
                    .map(|e| e.target)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns all symbols that call the given symbol.
    pub fn callers(&self, id: SymbolId) -> Vec<SymbolId> {
        self.incoming
            .get(&id)
            .map(|edges| {
                edges
                    .iter()
                    .filter(|e| e.kind == EdgeKind::Calls)
                    .map(|e| e.target)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns all symbols that import the given symbol (cross-file references).
    ///
    /// Filters incoming edges for `EdgeKind::References` — which includes
    /// `ReferenceKind::Import` edges created by cross-file resolution.
    /// Use this together with `callers()` to get the full set of dependents.
    pub fn importers(&self, id: SymbolId) -> Vec<SymbolId> {
        self.incoming
            .get(&id)
            .map(|edges| {
                edges
                    .iter()
                    .filter(|e| e.kind == EdgeKind::References)
                    .map(|e| e.target)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns the number of incoming edges (all types) for a symbol.
    pub fn incoming_edge_count(&self, id: SymbolId) -> usize {
        self.incoming.get(&id).map(|e| e.len()).unwrap_or(0)
    }

    /// Returns all outgoing edges for a symbol.
    pub fn edges_from(&self, id: SymbolId) -> &[Edge] {
        self.outgoing.get(&id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Returns all incoming edges for a symbol.
    pub fn edges_to(&self, id: SymbolId) -> &[Edge] {
        self.incoming.get(&id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    // -- File queries -------------------------------------------------------

    /// Returns all symbol IDs defined in the given file.
    pub fn symbols_in_file(&self, path: &Path) -> Vec<SymbolId> {
        self.file_symbols.get(path).cloned().unwrap_or_default()
    }

    /// Returns all references originating from symbols in the given file.
    pub fn references_from_file(&self, path: &Path) -> Vec<&Reference> {
        let file_sym_ids: HashSet<SymbolId> = self.symbols_in_file(path).into_iter().collect();
        self.references
            .values()
            .filter(|r| file_sym_ids.contains(&r.from))
            .collect()
    }

    // -- Connected components (undirected BFS) ------------------------------

    /// Finds connected components treating the graph as undirected.
    pub fn connected_components(&self) -> Vec<Vec<SymbolId>> {
        let mut visited: HashSet<SymbolId> = HashSet::new();
        let mut components = Vec::new();

        for (id, _) in &self.symbols {
            if visited.contains(&id) {
                continue;
            }

            let mut component = Vec::new();
            let mut queue = VecDeque::new();
            queue.push_back(id);
            visited.insert(id);

            while let Some(current) = queue.pop_front() {
                component.push(current);

                for edges in [self.outgoing.get(&current), self.incoming.get(&current)]
                    .into_iter()
                    .flatten()
                {
                    for edge in edges {
                        if !visited.contains(&edge.target) && self.symbols.contains_key(edge.target)
                        {
                            visited.insert(edge.target);
                            queue.push_back(edge.target);
                        }
                    }
                }
            }

            components.push(component);
        }

        components
    }

    // -- Cycle detection (directed, iterative DFS with coloring) ------------

    /// Detects cycles in the directed graph.
    pub fn detect_cycles(&self) -> Vec<Vec<SymbolId>> {
        #[derive(Clone, Copy, PartialEq)]
        enum Color {
            White,
            Gray,
            Black,
        }

        let mut color: HashMap<SymbolId, Color> = HashMap::new();
        let mut parent: HashMap<SymbolId, SymbolId> = HashMap::new();
        let mut cycles = Vec::new();

        for (id, _) in &self.symbols {
            color.insert(id, Color::White);
        }

        for (start, _) in &self.symbols {
            if color[&start] != Color::White {
                continue;
            }

            let mut stack: Vec<(SymbolId, bool)> = vec![(start, false)];

            while let Some((node, returning)) = stack.pop() {
                if returning {
                    color.insert(node, Color::Black);
                    continue;
                }

                let c = color.get(&node).copied().unwrap_or(Color::Black);
                if c != Color::White {
                    continue;
                }

                color.insert(node, Color::Gray);
                stack.push((node, true));

                if let Some(edges) = self.outgoing.get(&node) {
                    for edge in edges {
                        let target = edge.target;
                        if !self.symbols.contains_key(target) {
                            continue;
                        }
                        match color.get(&target) {
                            Some(Color::White) => {
                                parent.insert(target, node);
                                stack.push((target, false));
                            }
                            Some(Color::Gray) => {
                                let mut cycle = vec![target];
                                let mut current = node;
                                while current != target {
                                    cycle.push(current);
                                    match parent.get(&current) {
                                        Some(&p) => current = p,
                                        None => break,
                                    }
                                }
                                cycle.reverse();
                                cycles.push(cycle);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        cycles
    }
}

/// Maps a [`ReferenceKind`] to the corresponding [`EdgeKind`] for the graph's
/// adjacency lists. Only `Call` produces `EdgeKind::Calls`; all other reference
/// kinds produce `EdgeKind::References`. This distinction is critical because
/// diagnostic patterns like `data_dead_end` and `isolated_cluster` filter on
/// `EdgeKind::Calls` to detect uncalled functions and disconnected clusters.
fn ref_kind_to_edge_kind(kind: ReferenceKind) -> EdgeKind {
    match kind {
        ReferenceKind::Call => EdgeKind::Calls,
        ReferenceKind::Read | ReferenceKind::Write => EdgeKind::References,
        ReferenceKind::Import | ReferenceKind::Export => EdgeKind::References,
        ReferenceKind::Implement | ReferenceKind::Derive => EdgeKind::References,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_location(file: &str, line: u32, col: u32, end_line: u32, end_col: u32) -> Location {
        Location {
            file: PathBuf::from(file),
            line,
            column: col,
            end_line,
            end_column: end_col,
        }
    }

    fn make_file_scope(graph: &mut Graph, filename: &str) -> ScopeId {
        graph.add_scope(Scope {
            id: ScopeId::default(),
            kind: ScopeKind::File,
            parent: None,
            name: filename.to_string(),
            location: make_location(filename, 1, 1, 9999, 1),
        })
    }

    fn add_function(graph: &mut Graph, name: &str, scope: ScopeId, file: &str) -> SymbolId {
        graph.add_symbol(Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Function,
            name: name.to_string(),
            qualified_name: format!("{}::{}", file.trim_end_matches(".py"), name),
            visibility: Visibility::Public,
            signature: None,
            location: make_location(file, 1, 1, 10, 1),
            resolution: ResolutionStatus::Resolved,
            scope,
            annotations: vec![],
        })
    }

    fn add_function_in_scope(
        graph: &mut Graph,
        name: &str,
        scope: ScopeId,
        file: &str,
        line: u32,
    ) -> SymbolId {
        graph.add_symbol(Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Function,
            name: name.to_string(),
            qualified_name: format!("{}::{}", file.trim_end_matches(".py"), name),
            visibility: Visibility::Public,
            signature: None,
            location: make_location(file, line, 1, line + 5, 1),
            resolution: ResolutionStatus::Resolved,
            scope,
            annotations: vec![],
        })
    }

    fn add_call_edge(graph: &mut Graph, from: SymbolId, to: SymbolId, file: &str, line: u32) {
        graph.add_reference(Reference {
            id: ReferenceId::default(),
            from,
            to,
            kind: ReferenceKind::Call,
            location: make_location(file, line, 1, line, 20),
            resolution: ResolutionStatus::Resolved,
        });
    }

    // -- Basic CRUD ---------------------------------------------------------

    #[test]
    fn test_graph_new_is_empty() {
        let graph = Graph::new();
        assert_eq!(graph.symbol_count(), 0);
        assert_eq!(graph.scope_count(), 0);
        assert_eq!(graph.reference_count(), 0);
    }

    #[test]
    fn test_insert_symbol_and_retrieve() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "test.py");
        let id = add_function(&mut graph, "foo", scope, "test.py");
        let sym = graph.get_symbol(id).expect("Must retrieve inserted symbol");
        assert_eq!(sym.name, "foo");
        assert_eq!(sym.kind, SymbolKind::Function);
    }

    #[test]
    fn test_insert_multiple_symbols_unique_ids() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "multi.py");
        let id1 = add_function(&mut graph, "alpha", scope, "multi.py");
        let id2 = add_function(&mut graph, "beta", scope, "multi.py");
        let id3 = add_function(&mut graph, "gamma", scope, "multi.py");
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
        assert_eq!(graph.get_symbol(id1).unwrap().name, "alpha");
        assert_eq!(graph.get_symbol(id2).unwrap().name, "beta");
        assert_eq!(graph.get_symbol(id3).unwrap().name, "gamma");
    }

    #[test]
    fn test_get_symbol_nonexistent_returns_none() {
        let graph = Graph::new();
        let mut temp = Graph::new();
        let scope = make_file_scope(&mut temp, "x.py");
        let orphan_id = add_function(&mut temp, "orphan", scope, "x.py");
        assert!(graph.get_symbol(orphan_id).is_none());
    }

    #[test]
    fn test_remove_symbol_basic() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "rm.py");
        let id = add_function(&mut graph, "to_remove", scope, "rm.py");
        assert!(graph.get_symbol(id).is_some());
        graph.remove_symbol(id);
        assert!(graph.get_symbol(id).is_none());
    }

    #[test]
    fn test_remove_symbol_cleans_up_outgoing_edges() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "cleanup.py");
        let a = add_function(&mut graph, "a", scope, "cleanup.py");
        let b = add_function(&mut graph, "b", scope, "cleanup.py");
        let c = add_function(&mut graph, "c", scope, "cleanup.py");
        add_call_edge(&mut graph, a, b, "cleanup.py", 5);
        add_call_edge(&mut graph, b, c, "cleanup.py", 10);
        graph.remove_symbol(b);
        assert!(graph.callees(a).is_empty());
        assert!(graph.callers(c).is_empty());
    }

    #[test]
    fn test_remove_symbol_cleans_up_incoming_edges() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "incoming.py");
        let a = add_function(&mut graph, "a", scope, "incoming.py");
        let b = add_function(&mut graph, "b", scope, "incoming.py");
        add_call_edge(&mut graph, a, b, "incoming.py", 5);
        graph.remove_symbol(b);
        assert!(graph.callees(a).is_empty());
    }

    #[test]
    fn test_remove_symbol_with_many_edges() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "hub.py");
        let hub = add_function(&mut graph, "hub", scope, "hub.py");
        let mut spokes = Vec::new();
        for i in 0..500 {
            let spoke = add_function(&mut graph, &format!("spoke_{}", i), scope, "hub.py");
            add_call_edge(&mut graph, hub, spoke, "hub.py", i as u32 + 1);
            spokes.push(spoke);
        }
        assert_eq!(graph.callees(hub).len(), 500);
        graph.remove_symbol(hub);
        for spoke in &spokes {
            assert!(graph.callers(*spoke).is_empty());
        }
    }

    #[test]
    fn test_remove_nonexistent_symbol_no_panic() {
        let mut graph = Graph::new();
        let mut temp = Graph::new();
        let scope = make_file_scope(&mut temp, "phantom.py");
        let phantom_id = add_function(&mut temp, "phantom", scope, "phantom.py");
        graph.remove_symbol(phantom_id);
    }

    // -- Symbol construction ------------------------------------------------

    #[test]
    fn test_symbol_construction_function() {
        let mut graph = Graph::new();
        let scope_id = graph.add_scope(Scope {
            id: ScopeId::default(),
            kind: ScopeKind::File,
            parent: None,
            name: "main.py".to_string(),
            location: make_location("main.py", 1, 1, 100, 1),
        });
        let sym_id = graph.add_symbol(Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Function,
            name: "process_data".to_string(),
            qualified_name: "main::process_data".to_string(),
            visibility: Visibility::Public,
            signature: Some("(data: list) -> dict".to_string()),
            location: make_location("main.py", 5, 1, 20, 1),
            resolution: ResolutionStatus::Resolved,
            scope: scope_id,
            annotations: vec!["staticmethod".to_string()],
        });
        let sym = graph.get_symbol(sym_id).expect("must exist");
        assert_eq!(sym.name, "process_data");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(sym.signature.is_some());
        assert_eq!(sym.annotations.len(), 1);
        assert_eq!(sym.resolution, ResolutionStatus::Resolved);
    }

    #[test]
    fn test_symbol_empty_name() {
        let mut graph = Graph::new();
        let scope_id = make_file_scope(&mut graph, "anon.py");
        let sym_id = graph.add_symbol(Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Function,
            name: String::new(),
            qualified_name: "anon::".to_string(),
            visibility: Visibility::Private,
            signature: None,
            location: make_location("anon.py", 1, 1, 1, 10),
            resolution: ResolutionStatus::Resolved,
            scope: scope_id,
            annotations: vec![],
        });
        assert!(graph.get_symbol(sym_id).unwrap().name.is_empty());
    }

    #[test]
    fn test_symbol_very_long_name() {
        let mut graph = Graph::new();
        let scope_id = make_file_scope(&mut graph, "long.py");
        let long_name = "a".repeat(10_000);
        let sym_id = graph.add_symbol(Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Variable,
            name: long_name.clone(),
            qualified_name: format!("long::{}", long_name),
            visibility: Visibility::Private,
            signature: None,
            location: make_location("long.py", 1, 1, 1, 10),
            resolution: ResolutionStatus::Resolved,
            scope: scope_id,
            annotations: vec![],
        });
        assert_eq!(graph.get_symbol(sym_id).unwrap().name.len(), 10_000);
    }

    #[test]
    fn test_symbol_unicode_name() {
        let mut graph = Graph::new();
        let scope_id = make_file_scope(&mut graph, "unicode.py");
        let sym_id = graph.add_symbol(Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Function,
            name: "café".to_string(),
            qualified_name: "unicode::café".to_string(),
            visibility: Visibility::Public,
            signature: None,
            location: make_location("unicode.py", 1, 1, 1, 15),
            resolution: ResolutionStatus::Resolved,
            scope: scope_id,
            annotations: vec![],
        });
        assert_eq!(graph.get_symbol(sym_id).unwrap().name, "café");
    }

    #[test]
    fn test_symbol_no_signature() {
        let mut graph = Graph::new();
        let scope_id = make_file_scope(&mut graph, "consts.py");
        let sym_id = graph.add_symbol(Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Constant,
            name: "MAX_RETRIES".to_string(),
            qualified_name: "consts::MAX_RETRIES".to_string(),
            visibility: Visibility::Public,
            signature: None,
            location: make_location("consts.py", 1, 1, 1, 20),
            resolution: ResolutionStatus::Resolved,
            scope: scope_id,
            annotations: vec![],
        });
        assert!(graph.get_symbol(sym_id).unwrap().signature.is_none());
    }

    #[test]
    fn test_symbol_multiple_annotations() {
        let mut graph = Graph::new();
        let scope_id = make_file_scope(&mut graph, "deco.py");
        let sym_id = graph.add_symbol(Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Method,
            name: "handle".to_string(),
            qualified_name: "deco::Handler::handle".to_string(),
            visibility: Visibility::Public,
            signature: Some("(self, request) -> Response".to_string()),
            location: make_location("deco.py", 10, 5, 30, 1),
            resolution: ResolutionStatus::Resolved,
            scope: scope_id,
            annotations: vec![
                "staticmethod".to_string(),
                "cache".to_string(),
                "retry(max=3)".to_string(),
            ],
        });
        let sym = graph.get_symbol(sym_id).unwrap();
        assert_eq!(sym.annotations.len(), 3);
        assert!(sym.annotations.contains(&"retry(max=3)".to_string()));
    }

    // -- Reference construction ---------------------------------------------

    #[test]
    fn test_reference_construction_call() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "calls.py");
        let caller = add_function(&mut graph, "caller", scope, "calls.py");
        let callee = add_function(&mut graph, "callee", scope, "calls.py");
        let ref_id = graph.add_reference(Reference {
            id: ReferenceId::default(),
            from: caller,
            to: callee,
            kind: ReferenceKind::Call,
            location: make_location("calls.py", 5, 5, 5, 20),
            resolution: ResolutionStatus::Resolved,
        });
        let r = graph.get_reference(ref_id).expect("must exist");
        assert_eq!(r.from, caller);
        assert_eq!(r.to, callee);
        assert_eq!(r.kind, ReferenceKind::Call);
    }

    #[test]
    fn test_reference_import_with_partial_resolution() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "imports.py");
        let a = add_function(&mut graph, "main", scope, "imports.py");
        let b = add_function(&mut graph, "ext", scope, "imports.py");
        let ref_id = graph.add_reference(Reference {
            id: ReferenceId::default(),
            from: a,
            to: b,
            kind: ReferenceKind::Import,
            location: make_location("imports.py", 1, 1, 1, 30),
            resolution: ResolutionStatus::Partial("external package".to_string()),
        });
        assert!(matches!(
            graph.get_reference(ref_id).unwrap().resolution,
            ResolutionStatus::Partial(_)
        ));
    }

    #[test]
    fn test_reference_all_kinds_constructible() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "rk.py");
        let a = add_function(&mut graph, "a", scope, "rk.py");
        let b = add_function(&mut graph, "b", scope, "rk.py");
        for kind in [
            ReferenceKind::Call,
            ReferenceKind::Read,
            ReferenceKind::Write,
            ReferenceKind::Import,
            ReferenceKind::Export,
            ReferenceKind::Implement,
            ReferenceKind::Derive,
        ] {
            let ref_id = graph.add_reference(Reference {
                id: ReferenceId::default(),
                from: a,
                to: b,
                kind,
                location: make_location("rk.py", 1, 1, 1, 10),
                resolution: ResolutionStatus::Resolved,
            });
            assert!(graph.get_reference(ref_id).is_some());
        }
    }

    // -- Scope nesting ------------------------------------------------------

    #[test]
    fn test_scope_file_has_no_parent() {
        let mut graph = Graph::new();
        let file_scope = graph.add_scope(Scope {
            id: ScopeId::default(),
            kind: ScopeKind::File,
            parent: None,
            name: "root.py".to_string(),
            location: make_location("root.py", 1, 1, 50, 1),
        });
        let scope = graph.get_scope(file_scope).unwrap();
        assert!(scope.parent.is_none());
        assert_eq!(scope.kind, ScopeKind::File);
    }

    #[test]
    fn test_scope_nesting_file_function_block() {
        let mut graph = Graph::new();
        let file = graph.add_scope(Scope {
            id: ScopeId::default(),
            kind: ScopeKind::File,
            parent: None,
            name: "nested.py".to_string(),
            location: make_location("nested.py", 1, 1, 100, 1),
        });
        let func = graph.add_scope(Scope {
            id: ScopeId::default(),
            kind: ScopeKind::Function,
            parent: Some(file),
            name: "outer".to_string(),
            location: make_location("nested.py", 5, 1, 20, 1),
        });
        let block = graph.add_scope(Scope {
            id: ScopeId::default(),
            kind: ScopeKind::Block,
            parent: Some(func),
            name: "if_block".to_string(),
            location: make_location("nested.py", 10, 5, 15, 5),
        });
        assert_eq!(graph.get_scope(block).unwrap().parent, Some(func));
        assert_eq!(graph.get_scope(func).unwrap().parent, Some(file));
        assert!(graph.get_scope(file).unwrap().parent.is_none());
    }

    #[test]
    fn test_scope_deeply_nested_100_levels() {
        let mut graph = Graph::new();
        let mut parent: Option<ScopeId> = None;
        let mut ids = Vec::new();
        for i in 0..100 {
            let kind = if i == 0 {
                ScopeKind::File
            } else {
                ScopeKind::Function
            };
            let id = graph.add_scope(Scope {
                id: ScopeId::default(),
                kind,
                parent,
                name: format!("level_{}", i),
                location: make_location("deep.py", i as u32 + 1, 1, i as u32 + 2, 1),
            });
            ids.push(id);
            parent = Some(id);
        }
        let mut current = Some(ids[99]);
        let mut depth = 0;
        while let Some(id) = current {
            current = graph.get_scope(id).unwrap().parent;
            depth += 1;
        }
        assert_eq!(depth, 100);
    }

    // -- Edge operations ----------------------------------------------------

    #[test]
    fn test_add_edge_creates_bidirectional_link() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "bidir.py");
        let a = add_function(&mut graph, "caller", scope, "bidir.py");
        let b = add_function(&mut graph, "callee", scope, "bidir.py");
        add_call_edge(&mut graph, a, b, "bidir.py", 5);
        assert!(graph.callees(a).contains(&b));
        assert!(graph.callers(b).contains(&a));
    }

    #[test]
    fn test_self_loop_edge() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "recurse.py");
        let f = add_function(&mut graph, "factorial", scope, "recurse.py");
        add_call_edge(&mut graph, f, f, "recurse.py", 5);
        assert!(graph.callees(f).contains(&f));
        assert!(graph.callers(f).contains(&f));
    }

    #[test]
    fn test_multiple_edges_same_pair() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "mc.py");
        let a = add_function(&mut graph, "a", scope, "mc.py");
        let b = add_function(&mut graph, "b", scope, "mc.py");
        add_call_edge(&mut graph, a, b, "mc.py", 5);
        add_call_edge(&mut graph, a, b, "mc.py", 10);
        assert!(graph.callees(a).contains(&b));
    }

    #[test]
    fn test_incoming_edge_count() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "count.py");
        let target = add_function(&mut graph, "target", scope, "count.py");
        let c1 = add_function(&mut graph, "c1", scope, "count.py");
        let c2 = add_function(&mut graph, "c2", scope, "count.py");
        let c3 = add_function(&mut graph, "c3", scope, "count.py");
        add_call_edge(&mut graph, c1, target, "count.py", 1);
        add_call_edge(&mut graph, c2, target, "count.py", 2);
        add_call_edge(&mut graph, c3, target, "count.py", 3);
        assert_eq!(graph.incoming_edge_count(target), 3);
    }

    #[test]
    fn test_incoming_edge_count_zero() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "dead.py");
        let orphan = add_function(&mut graph, "orphan", scope, "dead.py");
        assert_eq!(graph.incoming_edge_count(orphan), 0);
    }

    // -- File-to-symbols mapping --------------------------------------------

    #[test]
    fn test_symbols_in_file() {
        let mut graph = Graph::new();
        let scope_a = make_file_scope(&mut graph, "a.py");
        let scope_b = make_file_scope(&mut graph, "b.py");
        let s1 = add_function(&mut graph, "fa1", scope_a, "a.py");
        let s2 = add_function(&mut graph, "fa2", scope_a, "a.py");
        let s3 = add_function(&mut graph, "fb1", scope_b, "b.py");
        let a_syms = graph.symbols_in_file(Path::new("a.py"));
        assert_eq!(a_syms.len(), 2);
        assert!(a_syms.contains(&s1));
        assert!(a_syms.contains(&s2));
        let b_syms = graph.symbols_in_file(Path::new("b.py"));
        assert_eq!(b_syms.len(), 1);
        assert!(b_syms.contains(&s3));
    }

    #[test]
    fn test_symbols_in_file_nonexistent() {
        let graph = Graph::new();
        assert!(graph.symbols_in_file(Path::new("no.py")).is_empty());
    }

    #[test]
    fn test_symbols_in_file_after_removal() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "rem.py");
        let s1 = add_function(&mut graph, "f1", scope, "rem.py");
        let s2 = add_function(&mut graph, "f2", scope, "rem.py");
        graph.remove_symbol(s1);
        let syms = graph.symbols_in_file(Path::new("rem.py"));
        assert_eq!(syms.len(), 1);
        assert!(syms.contains(&s2));
        assert!(!syms.contains(&s1));
    }

    // -- Connected components -----------------------------------------------

    #[test]
    fn test_connected_components_empty_graph() {
        let graph = Graph::new();
        assert!(graph.connected_components().is_empty());
    }

    #[test]
    fn test_connected_components_single_node() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "solo.py");
        let _solo = add_function(&mut graph, "solo", scope, "solo.py");
        let cc = graph.connected_components();
        assert_eq!(cc.len(), 1);
        assert_eq!(cc[0].len(), 1);
    }

    #[test]
    fn test_connected_components_two_isolated_nodes() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "iso.py");
        let _a = add_function(&mut graph, "a", scope, "iso.py");
        let _b = add_function(&mut graph, "b", scope, "iso.py");
        assert_eq!(graph.connected_components().len(), 2);
    }

    #[test]
    fn test_connected_components_chain() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "ch.py");
        let a = add_function(&mut graph, "a", scope, "ch.py");
        let b = add_function(&mut graph, "b", scope, "ch.py");
        let c = add_function(&mut graph, "c", scope, "ch.py");
        add_call_edge(&mut graph, a, b, "ch.py", 1);
        add_call_edge(&mut graph, b, c, "ch.py", 2);
        let cc = graph.connected_components();
        assert_eq!(cc.len(), 1);
        assert_eq!(cc[0].len(), 3);
    }

    #[test]
    fn test_connected_components_two_separate_chains() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "tc.py");
        let a = add_function(&mut graph, "a", scope, "tc.py");
        let b = add_function(&mut graph, "b", scope, "tc.py");
        let c = add_function(&mut graph, "c", scope, "tc.py");
        let d = add_function(&mut graph, "d", scope, "tc.py");
        add_call_edge(&mut graph, a, b, "tc.py", 1);
        add_call_edge(&mut graph, c, d, "tc.py", 2);
        assert_eq!(graph.connected_components().len(), 2);
    }

    #[test]
    fn test_connected_components_star_topology() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "star.py");
        let center = add_function(&mut graph, "center", scope, "star.py");
        for i in 0..10u32 {
            let spoke = add_function(&mut graph, &format!("s{}", i), scope, "star.py");
            add_call_edge(&mut graph, center, spoke, "star.py", i + 1);
        }
        let cc = graph.connected_components();
        assert_eq!(cc.len(), 1);
        assert_eq!(cc[0].len(), 11);
    }

    #[test]
    fn test_connected_components_diamond() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "dia.py");
        let a = add_function(&mut graph, "a", scope, "dia.py");
        let b = add_function(&mut graph, "b", scope, "dia.py");
        let c = add_function(&mut graph, "c", scope, "dia.py");
        let d = add_function(&mut graph, "d", scope, "dia.py");
        add_call_edge(&mut graph, a, b, "dia.py", 1);
        add_call_edge(&mut graph, a, c, "dia.py", 2);
        add_call_edge(&mut graph, b, d, "dia.py", 3);
        add_call_edge(&mut graph, c, d, "dia.py", 4);
        assert_eq!(graph.connected_components().len(), 1);
        assert_eq!(graph.connected_components()[0].len(), 4);
    }

    #[test]
    fn test_connected_components_with_cycle() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "cyc.py");
        let a = add_function(&mut graph, "a", scope, "cyc.py");
        let b = add_function(&mut graph, "b", scope, "cyc.py");
        let c = add_function(&mut graph, "c", scope, "cyc.py");
        add_call_edge(&mut graph, a, b, "cyc.py", 1);
        add_call_edge(&mut graph, b, c, "cyc.py", 2);
        add_call_edge(&mut graph, c, a, "cyc.py", 3);
        assert_eq!(graph.connected_components().len(), 1);
    }

    // -- Cycle detection ----------------------------------------------------

    #[test]
    fn test_cycle_detection_acyclic() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "ac.py");
        let a = add_function(&mut graph, "a", scope, "ac.py");
        let b = add_function(&mut graph, "b", scope, "ac.py");
        let c = add_function(&mut graph, "c", scope, "ac.py");
        add_call_edge(&mut graph, a, b, "ac.py", 1);
        add_call_edge(&mut graph, b, c, "ac.py", 2);
        assert!(graph.detect_cycles().is_empty());
    }

    #[test]
    fn test_cycle_detection_self_loop() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "sl.py");
        let f = add_function(&mut graph, "rec", scope, "sl.py");
        add_call_edge(&mut graph, f, f, "sl.py", 1);
        assert!(!graph.detect_cycles().is_empty());
    }

    #[test]
    fn test_cycle_detection_two_node_cycle() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "mu.py");
        let a = add_function(&mut graph, "ping", scope, "mu.py");
        let b = add_function(&mut graph, "pong", scope, "mu.py");
        add_call_edge(&mut graph, a, b, "mu.py", 1);
        add_call_edge(&mut graph, b, a, "mu.py", 2);
        assert!(!graph.detect_cycles().is_empty());
    }

    #[test]
    fn test_cycle_detection_three_node_cycle() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "tri.py");
        let a = add_function(&mut graph, "a", scope, "tri.py");
        let b = add_function(&mut graph, "b", scope, "tri.py");
        let c = add_function(&mut graph, "c", scope, "tri.py");
        add_call_edge(&mut graph, a, b, "tri.py", 1);
        add_call_edge(&mut graph, b, c, "tri.py", 2);
        add_call_edge(&mut graph, c, a, "tri.py", 3);
        let cycles = graph.detect_cycles();
        assert!(!cycles.is_empty());
        assert!(cycles[0].len() >= 3);
    }

    #[test]
    fn test_cycle_detection_diamond_no_cycle() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "dmd.py");
        let a = add_function(&mut graph, "a", scope, "dmd.py");
        let b = add_function(&mut graph, "b", scope, "dmd.py");
        let c = add_function(&mut graph, "c", scope, "dmd.py");
        let d = add_function(&mut graph, "d", scope, "dmd.py");
        add_call_edge(&mut graph, a, b, "dmd.py", 1);
        add_call_edge(&mut graph, a, c, "dmd.py", 2);
        add_call_edge(&mut graph, b, d, "dmd.py", 3);
        add_call_edge(&mut graph, c, d, "dmd.py", 4);
        assert!(graph.detect_cycles().is_empty());
    }

    #[test]
    fn test_cycle_detection_empty_graph() {
        let graph = Graph::new();
        assert!(graph.detect_cycles().is_empty());
    }

    // -- Scale tests --------------------------------------------------------

    #[test]
    fn test_graph_10000_symbols_performance() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "big.py");
        let mut ids = Vec::with_capacity(10_000);
        for i in 0..10_000 {
            ids.push(add_function(
                &mut graph,
                &format!("func_{}", i),
                scope,
                "big.py",
            ));
        }
        assert_eq!(graph.symbol_count(), 10_000);
        assert_eq!(graph.get_symbol(ids[5000]).unwrap().name, "func_5000");
        assert_eq!(graph.connected_components().len(), 10_000);
    }

    #[test]
    fn test_graph_long_chain_no_stack_overflow() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "lc.py");
        let mut ids = Vec::with_capacity(1000);
        for i in 0..1000 {
            ids.push(add_function(&mut graph, &format!("n{}", i), scope, "lc.py"));
        }
        for i in 0..999 {
            add_call_edge(&mut graph, ids[i], ids[i + 1], "lc.py", i as u32 + 1);
        }
        let cc = graph.connected_components();
        assert_eq!(cc.len(), 1);
        assert_eq!(cc[0].len(), 1000);
    }

    #[test]
    fn test_graph_complete_graph_small() {
        let mut graph = Graph::new();
        let scope = make_file_scope(&mut graph, "cg.py");
        let n = 5;
        let mut ids = Vec::new();
        for i in 0..n {
            ids.push(add_function(&mut graph, &format!("n{}", i), scope, "cg.py"));
        }
        for i in 0..n {
            for j in 0..n {
                if i != j {
                    add_call_edge(&mut graph, ids[i], ids[j], "cg.py", (i * n + j) as u32);
                }
            }
        }
        assert_eq!(graph.connected_components().len(), 1);
        for id in &ids {
            assert_eq!(graph.incoming_edge_count(*id), n - 1);
            assert_eq!(graph.callees(*id).len(), n - 1);
        }
    }

    // -- Scope-symbol relationships -----------------------------------------

    #[test]
    fn test_symbols_in_scope() {
        let mut graph = Graph::new();
        let file_scope = make_file_scope(&mut graph, "sc.py");
        let class_scope = graph.add_scope(Scope {
            id: ScopeId::default(),
            kind: ScopeKind::Module,
            parent: Some(file_scope),
            name: "MyClass".to_string(),
            location: make_location("sc.py", 5, 1, 50, 1),
        });
        let m1 = add_function_in_scope(&mut graph, "m1", class_scope, "sc.py", 6);
        let m2 = add_function_in_scope(&mut graph, "m2", class_scope, "sc.py", 15);
        let _top = add_function(&mut graph, "top", file_scope, "sc.py");
        let class_syms = graph.symbols_in_scope(class_scope);
        assert_eq!(class_syms.len(), 2);
        assert!(class_syms.contains(&m1));
        assert!(class_syms.contains(&m2));
    }

    #[test]
    fn test_scope_children() {
        let mut graph = Graph::new();
        let file = make_file_scope(&mut graph, "par.py");
        let f1 = graph.add_scope(Scope {
            id: ScopeId::default(),
            kind: ScopeKind::Function,
            parent: Some(file),
            name: "f1".to_string(),
            location: make_location("par.py", 1, 1, 10, 1),
        });
        let f2 = graph.add_scope(Scope {
            id: ScopeId::default(),
            kind: ScopeKind::Function,
            parent: Some(file),
            name: "f2".to_string(),
            location: make_location("par.py", 12, 1, 20, 1),
        });
        let children = graph.scope_children(file);
        assert_eq!(children.len(), 2);
        assert!(children.contains(&f1));
        assert!(children.contains(&f2));
    }
}
