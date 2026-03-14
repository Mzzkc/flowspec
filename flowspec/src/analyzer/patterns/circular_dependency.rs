// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Circular dependency detection — cycles in the module dependency graph.
//!
//! Builds a module-level adjacency map by grouping symbols by file path,
//! then aggregating cross-module edges (Calls and References). Runs
//! iterative DFS with white/gray/black coloring to detect cycles.
//! Reports each cycle with per-step evidence showing the specific
//! cross-module references that form the dependency loop.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use crate::analyzer::diagnostic::*;
use crate::graph::Graph;
use crate::parser::ir::EdgeKind;

/// A cross-module reference with source/target details for evidence.
#[derive(Debug, Clone)]
struct CrossModuleRef {
    from_file: PathBuf,
    to_file: PathBuf,
    from_symbol: String,
    to_symbol: String,
    from_line: u32,
}

/// Detect circular dependencies between modules in the analysis graph.
///
/// A circular dependency is a cycle in the module-level dependency graph:
/// module A depends on module B, which depends on module C, which depends
/// back on module A. This pattern uses custom module-level cycle detection
/// (NOT `graph.detect_cycles()`, which operates at the symbol level).
///
/// Severity: Warning. Confidence: High (cycles are structurally verifiable).
pub fn detect(graph: &Graph) -> Vec<Diagnostic> {
    // Step 1: Build module-level adjacency map with cross-module references
    let (adjacency, cross_refs) = build_module_adjacency(graph);

    if adjacency.is_empty() {
        return Vec::new();
    }

    // Step 2: Find all cycles using iterative DFS with coloring
    let raw_cycles = find_cycles(&adjacency);

    // Step 3: Deduplicate cycles by canonical form
    let unique_cycles = deduplicate_cycles(raw_cycles);

    // Step 4: Build diagnostics with evidence
    let mut diagnostics = Vec::new();
    for cycle in unique_cycles {
        if let Some(diag) = build_diagnostic(&cycle, &cross_refs) {
            diagnostics.push(diag);
        }
    }

    diagnostics
}

/// Build a module-level adjacency map from the symbol graph.
///
/// Groups symbols by file path, then for each symbol checks outgoing
/// Calls and References edges. If the target symbol is in a different
/// file, adds a directed module->module edge.
fn build_module_adjacency(
    graph: &Graph,
) -> (BTreeMap<PathBuf, BTreeSet<PathBuf>>, Vec<CrossModuleRef>) {
    let mut adjacency: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
    let mut cross_refs: Vec<CrossModuleRef> = Vec::new();

    for (id, symbol) in graph.all_symbols() {
        let source_file = &symbol.location.file;
        if source_file.as_os_str().is_empty() {
            continue;
        }

        // Ensure every file with symbols appears in the adjacency map
        adjacency.entry(source_file.clone()).or_default();

        for edge in graph.edges_from(id) {
            // Only Calls and References edges establish module dependencies
            if !matches!(edge.kind, EdgeKind::Calls | EdgeKind::References) {
                continue;
            }

            // Resolve target symbol
            let Some(target_sym) = graph.get_symbol(edge.target) else {
                continue;
            };

            let target_file = &target_sym.location.file;
            if target_file.as_os_str().is_empty() {
                continue;
            }

            // Skip intra-module references (same file)
            if source_file == target_file {
                continue;
            }

            adjacency
                .entry(source_file.clone())
                .or_default()
                .insert(target_file.clone());

            cross_refs.push(CrossModuleRef {
                from_file: source_file.clone(),
                to_file: target_file.clone(),
                from_symbol: symbol.name.clone(),
                to_symbol: target_sym.name.clone(),
                from_line: symbol.location.line,
            });
        }
    }

    (adjacency, cross_refs)
}

/// Find all cycles in a directed module adjacency map using iterative DFS.
fn find_cycles(adjacency: &BTreeMap<PathBuf, BTreeSet<PathBuf>>) -> Vec<Vec<PathBuf>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }

    let mut color: HashMap<&PathBuf, Color> = HashMap::new();
    let mut cycles: Vec<Vec<PathBuf>> = Vec::new();

    for key in adjacency.keys() {
        color.insert(key, Color::White);
    }

    for start in adjacency.keys() {
        if color.get(start).copied() != Some(Color::White) {
            continue;
        }

        // DFS with explicit stack tracking the path
        let mut stack: Vec<(&PathBuf, usize)> = vec![(start, 0)];
        let mut path: Vec<&PathBuf> = vec![start];
        color.insert(start, Color::Gray);

        while let Some((node, neighbor_idx)) = stack.last_mut() {
            let neighbors: Vec<&PathBuf> = adjacency
                .get(*node)
                .map(|s| s.iter().collect())
                .unwrap_or_default();

            if *neighbor_idx >= neighbors.len() {
                // All neighbors explored, backtrack
                color.insert(*node, Color::Black);
                stack.pop();
                path.pop();
                continue;
            }

            let neighbor = neighbors[*neighbor_idx];
            *neighbor_idx += 1;

            match color.get(neighbor).copied() {
                Some(Color::Gray) => {
                    // Found a cycle — extract it from path
                    if let Some(cycle_start_idx) = path.iter().position(|p| *p == neighbor) {
                        let cycle: Vec<PathBuf> = path[cycle_start_idx..]
                            .iter()
                            .map(|p| (*p).clone())
                            .collect();
                        if cycle.len() >= 2 {
                            cycles.push(cycle);
                        }
                    }
                }
                Some(Color::White) => {
                    color.insert(neighbor, Color::Gray);
                    path.push(neighbor);
                    stack.push((neighbor, 0));
                }
                _ => {} // Black or not in map — skip
            }
        }
    }

    cycles
}

/// Deduplicate cycles by canonical form (start from lexicographically smallest path).
fn deduplicate_cycles(cycles: Vec<Vec<PathBuf>>) -> Vec<Vec<PathBuf>> {
    let mut seen: HashSet<Vec<PathBuf>> = HashSet::new();
    let mut unique = Vec::new();

    for cycle in cycles {
        let canonical = canonicalize_cycle(&cycle);
        if seen.insert(canonical.clone()) {
            unique.push(canonical);
        }
    }

    unique
}

/// Canonicalize a cycle by rotating so it starts with the lexicographically smallest path.
fn canonicalize_cycle(cycle: &[PathBuf]) -> Vec<PathBuf> {
    if cycle.is_empty() {
        return Vec::new();
    }

    let min_idx = cycle
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.cmp(b))
        .map(|(i, _)| i)
        .unwrap_or(0);

    let mut canonical = Vec::with_capacity(cycle.len());
    for i in 0..cycle.len() {
        canonical.push(cycle[(min_idx + i) % cycle.len()].clone());
    }
    canonical
}

/// Build a diagnostic from a detected cycle with per-step evidence.
fn build_diagnostic(cycle: &[PathBuf], cross_refs: &[CrossModuleRef]) -> Option<Diagnostic> {
    if cycle.is_empty() {
        return None;
    }

    let module_names: Vec<String> = cycle.iter().map(|p| p.display().to_string()).collect();

    let entity = module_names.join(", ");

    let mut evidence = Vec::new();
    for i in 0..cycle.len() {
        let from = &cycle[i];
        let to = &cycle[(i + 1) % cycle.len()];

        // Find the specific cross-reference for this step
        let specific_ref = cross_refs
            .iter()
            .find(|r| &r.from_file == from && &r.to_file == to);

        let observation = if let Some(r) = specific_ref {
            format!(
                "{} references {} ('{}' depends on '{}')",
                from.display(),
                to.display(),
                r.from_symbol,
                r.to_symbol,
            )
        } else {
            format!("{} depends on {}", from.display(), to.display())
        };

        let location = specific_ref.map(|r| format!("{}:{}", r.from_file.display(), r.from_line));

        evidence.push(Evidence {
            observation,
            location,
            context: None,
        });
    }

    let location = format!("{}:1", cycle[0].display());

    let message = format!(
        "Circular dependency: {} modules form a dependency cycle ({})",
        cycle.len(),
        module_names.join(" -> "),
    );

    Some(Diagnostic {
        id: String::new(),
        pattern: DiagnosticPattern::CircularDependency,
        severity: Severity::Warning,
        confidence: Confidence::High,
        entity,
        message,
        evidence,
        suggestion: "Extract shared types into a common module that both can depend on, \
                      or restructure to eliminate the cycle."
            .to_string(),
        location,
    })
}
