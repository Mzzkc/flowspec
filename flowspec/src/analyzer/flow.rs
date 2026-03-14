// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Flow tracing engine — DFS-based path tracing through the analysis graph.
//!
//! Traces data flow paths from entry points (or any symbol) through the call
//! graph. Walks outgoing `EdgeKind::Calls` edges, records each step, detects
//! cycles via per-path visited sets, and respects a maximum path depth to
//! bound computation on large graphs.
//!
//! The graph is treated as read-only — tracing is a pure query with no side
//! effects.

use std::collections::HashSet;

use crate::graph::Graph;
use crate::parser::ir::{EdgeKind, SymbolId, SymbolKind};

/// Maximum flow path depth before stopping traversal.
///
/// Prevents runaway DFS on very deep call chains. When exceeded, the path
/// is truncated and a `tracing::warn!` is emitted.
const MAX_FLOW_DEPTH: usize = 64;

/// A single step in a traced flow path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowStep {
    /// The symbol at this step.
    pub symbol: SymbolId,
    /// The edge kind that led to this step.
    pub edge_kind: EdgeKind,
}

/// A complete traced flow path from an entry point.
#[derive(Debug, Clone)]
pub struct FlowPath {
    /// The entry point symbol where this path begins.
    pub entry: SymbolId,
    /// The steps in the path (excluding the entry point itself).
    pub steps: Vec<FlowStep>,
    /// Whether the path contains a cycle.
    pub is_cyclic: bool,
}

/// Traces data flow paths from the given symbol through the analysis graph.
///
/// Walks outgoing `EdgeKind::Calls` edges from `start_symbol`, recording each
/// step. Detects cycles via per-path visited sets and stops at terminal symbols
/// (no outgoing call edges). Returns all discovered flow paths.
///
/// Returns an empty `Vec` if the start symbol has no outgoing call edges.
pub fn trace_flows_from(graph: &Graph, start_symbol: SymbolId) -> Vec<FlowPath> {
    let mut paths = Vec::new();

    // Get outgoing call edges from start
    let call_targets: Vec<SymbolId> = graph
        .edges_from(start_symbol)
        .iter()
        .filter(|e| e.kind == EdgeKind::Calls)
        .map(|e| e.target)
        .filter(|&t| t != SymbolId::default())
        .collect();

    if call_targets.is_empty() {
        // Terminal node — no outgoing calls, no paths
        return paths;
    }

    // DFS: for each outgoing call edge, trace a path
    for target in &call_targets {
        let mut visited = HashSet::new();
        visited.insert(start_symbol);
        let mut current_steps = Vec::new();

        dfs_trace(
            graph,
            *target,
            start_symbol,
            &mut visited,
            &mut current_steps,
            &mut paths,
            0,
        );
    }

    paths
}

/// Traces flows from all detected entry points in the graph.
///
/// Entry points are symbols named `main` or `__main__`, matching the
/// detection logic in the analyze pipeline. Returns all flow paths
/// from all entry points.
pub fn trace_all_flows(graph: &Graph) -> Vec<FlowPath> {
    let mut all_paths = Vec::new();

    for (sym_id, symbol) in graph.all_symbols() {
        let is_entry = (symbol.name == "main" || symbol.name == "__main__")
            && symbol.kind != SymbolKind::Module;
        if is_entry {
            let mut paths = trace_flows_from(graph, sym_id);
            all_paths.append(&mut paths);
        }
    }

    all_paths
}

/// DFS recursive helper for flow tracing.
///
/// Explores the call graph from `current`, building up `current_steps`.
/// When a terminal (no outgoing calls) or cycle is detected, records
/// the complete path.
fn dfs_trace(
    graph: &Graph,
    current: SymbolId,
    entry: SymbolId,
    visited: &mut HashSet<SymbolId>,
    current_steps: &mut Vec<FlowStep>,
    paths: &mut Vec<FlowPath>,
    depth: usize,
) {
    // Max depth enforcement
    if depth >= MAX_FLOW_DEPTH {
        tracing::warn!("Flow trace depth limit ({}) reached", MAX_FLOW_DEPTH,);
        // Record the path as-is (truncated)
        paths.push(FlowPath {
            entry,
            steps: current_steps.clone(),
            is_cyclic: false,
        });
        return;
    }

    // Cycle detection
    if visited.contains(&current) {
        // Record the cyclic path
        current_steps.push(FlowStep {
            symbol: current,
            edge_kind: EdgeKind::Calls,
        });
        paths.push(FlowPath {
            entry,
            steps: current_steps.clone(),
            is_cyclic: true,
        });
        current_steps.pop();
        return;
    }

    // Add current to path
    visited.insert(current);
    current_steps.push(FlowStep {
        symbol: current,
        edge_kind: EdgeKind::Calls,
    });

    // Find outgoing call edges
    let call_targets: Vec<SymbolId> = graph
        .edges_from(current)
        .iter()
        .filter(|e| e.kind == EdgeKind::Calls)
        .map(|e| e.target)
        .filter(|&t| t != SymbolId::default())
        .collect();

    if call_targets.is_empty() {
        // Terminal node — record the complete path
        paths.push(FlowPath {
            entry,
            steps: current_steps.clone(),
            is_cyclic: false,
        });
    } else {
        // Continue DFS into each call target
        for target in &call_targets {
            dfs_trace(
                graph,
                *target,
                entry,
                visited,
                current_steps,
                paths,
                depth + 1,
            );
        }
    }

    // Backtrack
    current_steps.pop();
    visited.remove(&current);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ir::*;
    use std::path::PathBuf;

    fn make_symbol(name: &str, kind: SymbolKind, file: &str, line: u32) -> Symbol {
        Symbol {
            id: SymbolId::default(),
            kind,
            name: name.to_string(),
            qualified_name: format!("{}::{}", file, name),
            visibility: Visibility::Public,
            signature: None,
            location: Location {
                file: PathBuf::from(file),
                line,
                column: 1,
                end_line: line,
                end_column: 1,
            },
            resolution: ResolutionStatus::Resolved,
            scope: ScopeId::default(),
            annotations: vec![],
        }
    }

    fn add_call(graph: &mut Graph, from: SymbolId, to: SymbolId, file: &str) {
        graph.add_reference(Reference {
            id: ReferenceId::default(),
            from,
            to,
            kind: ReferenceKind::Call,
            location: Location {
                file: PathBuf::from(file),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            },
            resolution: ResolutionStatus::Resolved,
        });
    }

    // T15: Linear path A→B→C
    #[test]
    fn test_flow_trace_linear_path() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol("func_a", SymbolKind::Function, "main.py", 1));
        let b = g.add_symbol(make_symbol("func_b", SymbolKind::Function, "main.py", 5));
        let c = g.add_symbol(make_symbol("func_c", SymbolKind::Function, "main.py", 10));
        add_call(&mut g, a, b, "main.py");
        add_call(&mut g, b, c, "main.py");

        let paths = trace_flows_from(&g, a);

        assert_eq!(paths.len(), 1, "Linear chain produces one path");
        assert_eq!(paths[0].steps.len(), 2, "Two steps: B, C");
        assert!(!paths[0].is_cyclic, "Linear path is not cyclic");
        assert_eq!(paths[0].entry, a, "Path starts at A");
    }

    // T16: Branching A→B, A→C
    #[test]
    fn test_flow_trace_branching_path() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol("main", SymbolKind::Function, "app.py", 1));
        let b = g.add_symbol(make_symbol("handler_b", SymbolKind::Function, "app.py", 5));
        let c = g.add_symbol(make_symbol("handler_c", SymbolKind::Function, "app.py", 10));
        add_call(&mut g, a, b, "app.py");
        add_call(&mut g, a, c, "app.py");

        let paths = trace_flows_from(&g, a);

        let all_symbols: Vec<SymbolId> = paths
            .iter()
            .flat_map(|p| p.steps.iter().map(|s| s.symbol))
            .collect();
        assert!(all_symbols.contains(&b), "Branch to B must be traced");
        assert!(all_symbols.contains(&c), "Branch to C must be traced");
    }

    // T17: Cycle detection A→B→A
    #[test]
    fn test_flow_trace_cycle_detection() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol("ping", SymbolKind::Function, "cycle.py", 1));
        let b = g.add_symbol(make_symbol("pong", SymbolKind::Function, "cycle.py", 5));
        add_call(&mut g, a, b, "cycle.py");
        add_call(&mut g, b, a, "cycle.py");

        let paths = trace_flows_from(&g, a);

        assert!(
            !paths.is_empty(),
            "Cyclic graph must produce at least one path"
        );
        assert!(
            paths.iter().any(|p| p.is_cyclic),
            "Path through cycle MUST be marked is_cyclic: true"
        );
    }

    // T18: Cross-file flow
    #[test]
    fn test_flow_trace_cross_file() {
        let mut g = Graph::new();
        let main_fn = g.add_symbol(make_symbol("main", SymbolKind::Function, "a.py", 1));
        let helper = g.add_symbol(make_symbol("helper", SymbolKind::Function, "b.py", 1));
        add_call(&mut g, main_fn, helper, "a.py");

        let paths = trace_flows_from(&g, main_fn);

        assert_eq!(paths.len(), 1);
        let step_symbols: Vec<SymbolId> = paths[0].steps.iter().map(|s| s.symbol).collect();
        assert!(
            step_symbols.contains(&helper),
            "Flow must trace across file boundaries"
        );
    }

    // T19: Terminal node
    #[test]
    fn test_flow_trace_terminal_node() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol("entry", SymbolKind::Function, "main.py", 1));
        let b = g.add_symbol(make_symbol("terminal", SymbolKind::Function, "main.py", 5));
        add_call(&mut g, a, b, "main.py");

        let paths = trace_flows_from(&g, a);

        assert_eq!(paths.len(), 1);
        assert!(!paths[0].is_cyclic);
    }

    // T20: Deep call chain (15 functions)
    #[test]
    fn test_flow_trace_deep_call_chain() {
        let mut g = Graph::new();
        let mut ids = Vec::new();
        for i in 0..15 {
            let id = g.add_symbol(make_symbol(
                &format!("f{}", i),
                SymbolKind::Function,
                "deep.py",
                i as u32 + 1,
            ));
            ids.push(id);
        }
        for i in 0..14 {
            add_call(&mut g, ids[i], ids[i + 1], "deep.py");
        }

        let paths = trace_flows_from(&g, ids[0]);

        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].steps.len(), 14);
    }

    // T21: Empty graph
    #[test]
    fn test_flow_trace_empty_graph() {
        let g = Graph::new();
        let paths = trace_all_flows(&g);
        assert!(paths.is_empty(), "Empty graph must produce no flows");
    }

    // T22: Single-symbol graph
    #[test]
    fn test_flow_trace_single_symbol() {
        let mut g = Graph::new();
        let solo = g.add_symbol(make_symbol("main", SymbolKind::Function, "solo.py", 1));

        let paths = trace_flows_from(&g, solo);

        // Terminal entry point with no outgoing calls — no flow paths
        assert!(paths.is_empty());
    }

    // T23: Star import flow (same as cross-file — just call edges)
    #[test]
    fn test_flow_trace_through_star_import() {
        let mut g = Graph::new();
        let main_fn = g.add_symbol(make_symbol("main", SymbolKind::Function, "a.py", 1));
        let helper = g.add_symbol(make_symbol("helper", SymbolKind::Function, "b.py", 1));
        add_call(&mut g, main_fn, helper, "a.py");

        let paths = trace_flows_from(&g, main_fn);

        assert_eq!(paths.len(), 1);
        let step_symbols: Vec<SymbolId> = paths[0].steps.iter().map(|s| s.symbol).collect();
        assert!(step_symbols.contains(&helper));
    }

    // T24: Idempotent
    #[test]
    fn test_flow_trace_idempotent() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol("func_a", SymbolKind::Function, "main.py", 1));
        let b = g.add_symbol(make_symbol("func_b", SymbolKind::Function, "main.py", 5));
        let c = g.add_symbol(make_symbol("func_c", SymbolKind::Function, "main.py", 10));
        add_call(&mut g, a, b, "main.py");
        add_call(&mut g, b, c, "main.py");

        let paths1 = trace_flows_from(&g, a);
        let paths2 = trace_flows_from(&g, a);

        assert_eq!(paths1.len(), paths2.len(), "Trace must be idempotent");
        for (p1, p2) in paths1.iter().zip(paths2.iter()) {
            assert_eq!(p1.entry, p2.entry);
            assert_eq!(p1.steps.len(), p2.steps.len());
            assert_eq!(p1.is_cyclic, p2.is_cyclic);
        }
    }

    // T25: Unresolved target
    #[test]
    fn test_flow_trace_unresolved_target_stops_path() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol("caller", SymbolKind::Function, "main.py", 1));
        // Add an edge to SymbolId::default() (unresolved)
        g.add_reference(Reference {
            id: ReferenceId::default(),
            from: a,
            to: SymbolId::default(),
            kind: ReferenceKind::Call,
            location: Location {
                file: PathBuf::from("main.py"),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            },
            resolution: ResolutionStatus::Unresolved,
        });

        let paths = trace_flows_from(&g, a);

        // Unresolved targets are filtered — no valid call targets, so no paths
        for path in &paths {
            for step in &path.steps {
                assert_ne!(
                    step.symbol,
                    SymbolId::default(),
                    "Unresolved targets must not appear as flow steps"
                );
            }
        }
    }

    // T26: Max depth enforcement
    #[test]
    fn test_flow_trace_max_depth_enforcement() {
        let mut g = Graph::new();
        let mut ids = Vec::new();
        for i in 0..100 {
            let id = g.add_symbol(make_symbol(
                &format!("f{}", i),
                SymbolKind::Function,
                "huge.py",
                i as u32 + 1,
            ));
            ids.push(id);
        }
        for i in 0..99 {
            add_call(&mut g, ids[i], ids[i + 1], "huge.py");
        }

        let paths = trace_flows_from(&g, ids[0]);

        assert!(!paths.is_empty());
        for path in &paths {
            assert!(
                path.steps.len() <= MAX_FLOW_DEPTH,
                "Flow path must not exceed max depth ({}). Got: {}",
                MAX_FLOW_DEPTH,
                path.steps.len()
            );
        }
    }

    // T27: Diamond pattern
    #[test]
    fn test_flow_trace_diamond_pattern() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol("entry", SymbolKind::Function, "diamond.py", 1));
        let b = g.add_symbol(make_symbol("path_b", SymbolKind::Function, "diamond.py", 5));
        let c = g.add_symbol(make_symbol(
            "path_c",
            SymbolKind::Function,
            "diamond.py",
            10,
        ));
        let d = g.add_symbol(make_symbol("sink", SymbolKind::Function, "diamond.py", 15));
        add_call(&mut g, a, b, "diamond.py");
        add_call(&mut g, a, c, "diamond.py");
        add_call(&mut g, b, d, "diamond.py");
        add_call(&mut g, c, d, "diamond.py");

        let paths = trace_flows_from(&g, a);

        assert!(
            paths.len() >= 2,
            "Diamond pattern must produce at least 2 paths (via B and via C). Got: {}",
            paths.len()
        );
    }

    // T28: trace_all_flows uses entry point detection
    #[test]
    fn test_trace_all_flows_finds_entry_points() {
        let mut g = Graph::new();
        let main_fn = g.add_symbol(make_symbol("main", SymbolKind::Function, "__main__.py", 1));
        let helper = g.add_symbol(make_symbol("helper", SymbolKind::Function, "utils.py", 1));
        add_call(&mut g, main_fn, helper, "__main__.py");

        let paths = trace_all_flows(&g);

        assert!(!paths.is_empty(), "trace_all_flows must find entry points");
        assert!(
            paths.iter().any(|p| p.entry == main_fn),
            "main() must be detected as entry point"
        );
    }
}
