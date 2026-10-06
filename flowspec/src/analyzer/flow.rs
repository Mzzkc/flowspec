//! Exact, bounded traversal of real call and import-reference graph edges.
use crate::graph::Graph;
use crate::parser::ir::{EdgeKind, SymbolId, SymbolKind};
use std::collections::HashSet;
const MAX_FLOW_DEPTH: usize = 64;
const MAX_TRACE_PATHS: usize = 600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowStep {
    pub symbol: SymbolId,
    pub edge_kind: EdgeKind,
}
#[derive(Debug, Clone)]
pub struct FlowPath {
    pub entry: SymbolId,
    pub steps: Vec<FlowStep>,
    pub is_cyclic: bool,
    /// A real next edge was omitted by the depth or path budget.
    pub truncated: bool,
}
/// Structured loss and continuation belong to the query, including zero-path queries.
#[derive(Debug, Clone)]
pub struct TraceReport {
    pub symbol: SymbolId,
    pub paths: Vec<FlowPath>,
    pub unresolved: Vec<crate::parser::ir::Location>,
    pub continuation: Vec<FlowPath>,
    pub incomplete: bool,
}

fn trace_edges(graph: &Graph, current: SymbolId, backward: bool) -> Vec<(SymbolId, EdgeKind)> {
    let import = graph
        .get_symbol(current)
        .is_some_and(|s| s.annotations.iter().any(|a| a == "import"));
    let edges = if backward {
        graph.edges_to(current)
    } else {
        graph.edges_from(current)
    };
    let mut selected: Vec<_> = edges
        .iter()
        .filter(|e| {
            e.kind == EdgeKind::Calls
                || (e.kind == EdgeKind::References
                    && if backward {
                        graph
                            .get_symbol(e.target)
                            .is_some_and(|s| s.annotations.iter().any(|a| a == "import"))
                    } else {
                        import
                    })
        })
        .filter(|e| e.target != SymbolId::default() && graph.get_symbol(e.target).is_some())
        .map(|e| (e.target, e.kind))
        .collect();
    selected.sort_by(|a, b| {
        let key = |id| {
            graph.get_symbol(id).map(|s| {
                (
                    &s.qualified_name,
                    &s.location.file,
                    s.location.line,
                    s.location.column,
                )
            })
        };
        key(a.0)
            .cmp(&key(b.0))
            .then(format!("{:?}", a.1).cmp(&format!("{:?}", b.1)))
    });
    selected.dedup();
    selected
}

struct TraceWalk {
    paths: Vec<FlowPath>,
    continuation: Vec<FlowPath>,
    reached: HashSet<SymbolId>,
}

fn trace_walk(graph: &Graph, start: SymbolId, max_depth: usize, backward: bool) -> TraceWalk {
    let max_depth = max_depth.min(256);
    let mut paths = Vec::new();
    let mut pending = vec![(start, Vec::<FlowStep>::new(), HashSet::from([start]))];
    let mut reached = HashSet::from([start]);
    let mut continuation = Vec::new();
    while let Some((current, steps, visited)) = pending.pop() {
        reached.insert(current);
        if paths.len() >= MAX_TRACE_PATHS - 1 && !pending.is_empty() {
            let frontier = FlowPath {
                entry: start,
                steps,
                is_cyclic: false,
                truncated: true,
            };
            continuation.push(frontier.clone());
            paths.push(frontier);
            break;
        }
        let edges = trace_edges(graph, current, backward);
        if !edges.is_empty() && (steps.len() >= max_depth || paths.len() >= MAX_TRACE_PATHS) {
            let path = FlowPath {
                entry: start,
                steps,
                is_cyclic: false,
                truncated: true,
            };
            continuation.push(path.clone());
            paths.push(path);
            if paths.len() >= MAX_TRACE_PATHS {
                // Keep bounded frontier metadata without walking the omitted branches.
                break;
            }
            continue;
        }
        if edges.is_empty() {
            if !steps.is_empty() {
                paths.push(FlowPath {
                    entry: start,
                    steps,
                    is_cyclic: false,
                    truncated: false,
                });
            }
            continue;
        }
        for (target, kind) in edges.into_iter().rev() {
            let mut next = steps.clone();
            next.push(FlowStep {
                symbol: target,
                edge_kind: kind,
            });
            if visited.contains(&target) {
                paths.push(FlowPath {
                    entry: start,
                    steps: next,
                    is_cyclic: true,
                    truncated: false,
                });
            } else {
                let mut seen = visited.clone();
                seen.insert(target);
                pending.push((target, next, seen));
            }
        }
    }
    let mut identities = HashSet::new();
    paths.retain(|p| {
        identities.insert(
            p.steps
                .iter()
                .map(|s| (s.symbol, s.edge_kind))
                .collect::<Vec<_>>(),
        )
    });
    TraceWalk {
        paths,
        continuation,
        reached,
    }
}

pub fn trace_report(
    graph: &Graph,
    start: SymbolId,
    max_depth: usize,
    backward: bool,
) -> TraceReport {
    let TraceWalk {
        paths,
        continuation,
        reached,
    } = trace_walk(graph, start, max_depth, backward);
    let mut unresolved:Vec<_>=graph.unresolved_calls().iter().chain(graph.prerequisite_losses()).filter(|r| {
        if matches!(&r.resolution,crate::parser::ir::ResolutionStatus::Partial(info) if info.ends_with(":type_flows")) {return false;}
        reached.contains(&r.from) || (r.from==SymbolId::default() && reached.iter().filter_map(|id|graph.get_symbol(*id)).any(|s|
            s.location.file==r.location.file && (s.location.line,s.location.column)<=(r.location.end_line,r.location.end_column)
                && (s.location.end_line,s.location.end_column)>=(r.location.line,r.location.column)))
    }).map(|r|r.location.clone()).collect();
    unresolved.extend(
        reached
            .iter()
            .filter_map(|id| graph.get_symbol(*id))
            .filter(|s| {
                s.annotations.iter().any(|a| a == "import")
                    && s.resolution != crate::parser::ir::ResolutionStatus::Resolved
            })
            .map(|s| s.location.clone()),
    );
    unresolved.sort_by(|a, b| {
        (&a.file, a.line, a.column, a.end_line, a.end_column).cmp(&(
            &b.file,
            b.line,
            b.column,
            b.end_line,
            b.end_column,
        ))
    });
    unresolved.dedup();
    let incomplete = !unresolved.is_empty() || !continuation.is_empty();
    TraceReport {
        symbol: start,
        paths,
        unresolved,
        continuation,
        incomplete,
    }
}
pub fn trace_flows_from(graph: &Graph, start: SymbolId, max_depth: usize) -> Vec<FlowPath> {
    trace_report(graph, start, max_depth, false).paths
}
pub fn trace_flows_to(graph: &Graph, start: SymbolId, max_depth: usize) -> Vec<FlowPath> {
    trace_report(graph, start, max_depth, true).paths
}
pub fn trace_all_flows(graph: &Graph) -> Vec<FlowPath> {
    let mut entries: Vec<_> = graph
        .all_symbols()
        .filter(|(_, s)| {
            s.kind != SymbolKind::Module
                && (s.name == "main"
                    || s.name == "__main__"
                    || s.annotations.iter().any(|a| a == "entry_point"))
        })
        .collect();
    entries.sort_by(|(_, a), (_, b)| a.qualified_name.cmp(&b.qualified_name));
    entries
        .into_iter()
        // The aggregate API returns paths only. Query loss collection belongs
        // to trace_report; the analysis report independently preserves all losses.
        .flat_map(|(id, _)| trace_walk(graph, id, MAX_FLOW_DEPTH, false).paths)
        .collect()
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

    #[test]
    fn aggregate_paths_equal_full_query_paths_without_discarding_public_losses() {
        let mut graph = Graph::new();
        let first = graph.add_symbol(make_symbol("main", SymbolKind::Function, "a.py", 1));
        let second = graph.add_symbol(make_symbol("__main__", SymbolKind::Function, "b.py", 1));
        let leaf = graph.add_symbol(make_symbol("leaf", SymbolKind::Function, "a.py", 5));
        add_call(&mut graph, first, leaf, "a.py");
        add_call(&mut graph, leaf, first, "a.py");
        add_call(&mut graph, second, leaf, "b.py");
        graph.record_unresolved_call(Reference {
            id: ReferenceId::default(),
            from: first,
            to: SymbolId::default(),
            kind: ReferenceKind::Call,
            location: Location {
                file: PathBuf::from("a.py"),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 2,
            },
            resolution: ResolutionStatus::Unresolved,
        });
        let mut entries = [first, second];
        entries.sort_by_key(|id| graph.get_symbol(*id).unwrap().qualified_name.clone());
        let expected: Vec<_> = entries
            .iter()
            .flat_map(|id| trace_report(&graph, *id, MAX_FLOW_DEPTH, false).paths)
            .collect();
        let signature = |paths: Vec<FlowPath>| {
            paths
                .into_iter()
                .map(|p| (p.entry, p.steps, p.is_cyclic, p.truncated))
                .collect::<Vec<_>>()
        };
        assert_eq!(signature(trace_all_flows(&graph)), signature(expected));
        let public = trace_report(&graph, first, MAX_FLOW_DEPTH, false);
        assert_eq!(public.unresolved.len(), 1);
        assert!(public.incomplete);
        assert!(public.paths.iter().any(|p| p.is_cyclic));
    }

    #[test]
    fn aggregate_paths_retain_exact_path_budget_and_continuation_frontier() {
        let mut graph = Graph::new();
        let entry = graph.add_symbol(make_symbol("main", SymbolKind::Function, "a.py", 1));
        for index in 0..MAX_TRACE_PATHS + 5 {
            let leaf = graph.add_symbol(make_symbol(
                &format!("leaf_{index:04}"),
                SymbolKind::Function,
                "a.py",
                5 + index as u32,
            ));
            add_call(&mut graph, entry, leaf, "a.py");
        }
        let public = trace_report(&graph, entry, MAX_FLOW_DEPTH, false);
        let aggregate = trace_all_flows(&graph);
        assert_eq!(public.paths.len(), MAX_TRACE_PATHS);
        assert_eq!(aggregate.len(), MAX_TRACE_PATHS);
        assert!(public.incomplete);
        assert_eq!(public.continuation.len(), 1);
        for (actual, expected) in aggregate.iter().zip(&public.paths) {
            assert_eq!(actual.entry, expected.entry);
            assert_eq!(actual.steps, expected.steps);
            assert_eq!(actual.is_cyclic, expected.is_cyclic);
            assert_eq!(actual.truncated, expected.truncated);
        }
        assert!(aggregate.last().unwrap().truncated);
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

        let paths = trace_flows_from(&g, a, MAX_FLOW_DEPTH);

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

        let paths = trace_flows_from(&g, a, MAX_FLOW_DEPTH);

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

        let paths = trace_flows_from(&g, a, MAX_FLOW_DEPTH);

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

        let paths = trace_flows_from(&g, main_fn, MAX_FLOW_DEPTH);

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

        let paths = trace_flows_from(&g, a, MAX_FLOW_DEPTH);

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

        let paths = trace_flows_from(&g, ids[0], MAX_FLOW_DEPTH);

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

        let paths = trace_flows_from(&g, solo, MAX_FLOW_DEPTH);

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

        let paths = trace_flows_from(&g, main_fn, MAX_FLOW_DEPTH);

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

        let paths1 = trace_flows_from(&g, a, MAX_FLOW_DEPTH);
        let paths2 = trace_flows_from(&g, a, MAX_FLOW_DEPTH);

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

        let paths = trace_flows_from(&g, a, MAX_FLOW_DEPTH);

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

        let paths = trace_flows_from(&g, ids[0], MAX_FLOW_DEPTH);

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

        let paths = trace_flows_from(&g, a, MAX_FLOW_DEPTH);

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

#[cfg(test)]
mod backward_binding_regressions {
    use super::*;
    use crate::parser::ir::{
        Location, Reference, ReferenceId, ReferenceKind, ResolutionStatus, Symbol, Visibility,
    };
    use std::path::PathBuf;

    fn symbol(name: &str, import: bool) -> Symbol {
        Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Function,
            name: name.into(),
            qualified_name: format!("unit.rs::{name}"),
            visibility: Visibility::Private,
            signature: None,
            location: Location {
                file: PathBuf::from("unit.rs"),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 8,
            },
            resolution: ResolutionStatus::Resolved,
            scope: Default::default(),
            annotations: if import {
                vec!["import".into()]
            } else {
                vec![]
            },
        }
    }
    fn edge(graph: &mut Graph, from: SymbolId, to: SymbolId, kind: ReferenceKind) {
        graph.add_reference(Reference {
            id: ReferenceId::default(),
            from,
            to,
            kind,
            location: Location {
                file: PathBuf::from("unit.rs"),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 8,
            },
            resolution: ResolutionStatus::Resolved,
        });
    }
    #[test]
    fn incoming_call_trace_selects_actual_caller_without_fabricated_self_cycle() {
        let mut graph = Graph::new();
        let caller = graph.add_symbol(symbol("caller", false));
        let callee = graph.add_symbol(symbol("callee", false));
        edge(&mut graph, caller, callee, ReferenceKind::Call);
        assert_eq!(graph.edges_to(callee)[0].target, caller);
        let trace = trace_report(&graph, callee, 8, true);
        assert_eq!(trace.paths.len(), 1);
        assert_eq!(
            trace.paths[0].steps,
            vec![FlowStep {
                symbol: caller,
                edge_kind: EdgeKind::Calls
            }]
        );
        assert!(!trace.paths[0].is_cyclic);
        assert!(!trace.incomplete);
    }
    #[test]
    fn incoming_import_reference_preserves_source_proxy_and_forward_relation() {
        let mut graph = Graph::new();
        let caller = graph.add_symbol(symbol("main", false));
        let proxy = graph.add_symbol(symbol("alias", true));
        let callee = graph.add_symbol(symbol("implementation", false));
        edge(&mut graph, caller, proxy, ReferenceKind::Call);
        edge(&mut graph, proxy, callee, ReferenceKind::Read);
        let trace = trace_report(&graph, callee, 8, true);
        assert_eq!(trace.paths.len(), 1);
        assert_eq!(
            trace.paths[0].steps,
            vec![
                FlowStep {
                    symbol: proxy,
                    edge_kind: EdgeKind::References
                },
                FlowStep {
                    symbol: caller,
                    edge_kind: EdgeKind::Calls
                }
            ]
        );
        assert!(!trace.paths[0].is_cyclic);
        let forward = trace_flows_from(&graph, caller, 8);
        assert_eq!(
            forward[0].steps,
            vec![
                FlowStep {
                    symbol: proxy,
                    edge_kind: EdgeKind::Calls
                },
                FlowStep {
                    symbol: callee,
                    edge_kind: EdgeKind::References
                }
            ]
        );
        // A non-import value read is not an eligible trace hop in either direction.
        let rival = graph.add_symbol(symbol("value_reader", false));
        edge(&mut graph, rival, callee, ReferenceKind::Read);
        assert_eq!(
            trace_report(&graph, callee, 8, true).paths[0].steps,
            trace.paths[0].steps
        );
    }
}
