// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Cycle 15 QA-1 (Foundation) tests — proximity-based resolve_import_by_name fix.
//!
//! TDD tests validating that resolve_import_by_name uses location proximity
//! to resolve duplicate-name imports instead of always returning the first match.

use crate::graph::resolve_import_by_name;
use crate::graph::Graph;
use crate::parser::ir::*;
use crate::test_utils::{make_import, make_symbol};

// ===========================================================================
// Helper: build a symbol_id_map by adding symbols to a graph and tracking IDs.
// ===========================================================================

fn build_symbol_map(
    graph: &mut Graph,
    symbols: Vec<Symbol>,
) -> (Vec<Symbol>, Vec<(usize, SymbolId)>) {
    let mut id_map = Vec::new();
    for (idx, sym) in symbols.iter().enumerate() {
        let real_id = graph.add_symbol(sym.clone());
        id_map.push((idx, real_id));
    }
    (symbols, id_map)
}

// ===========================================================================
// Category 1: Regression — Single Import Per Name (R1-R4)
// ===========================================================================

/// R1: Single import resolves correctly (baseline).
#[test]
fn r1_single_import_resolves_correctly() {
    let mut graph = Graph::new();
    let symbols = vec![make_import("Path", "test.rs", 5)];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("Path", &map, &syms, 20);
    assert_eq!(result, map[0].1, "Single import 'Path' must resolve");
}

/// R2: No matching import returns default.
#[test]
fn r2_no_matching_import_returns_default() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_import("os", "test.rs", 1),
        make_import("sys", "test.rs", 2),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("json", &map, &syms, 10);
    assert_eq!(
        result,
        SymbolId::default(),
        "No matching import for 'json' must return default"
    );
}

/// R3: Empty symbol_id_map returns default.
#[test]
fn r3_empty_symbol_map_returns_default() {
    let result = resolve_import_by_name("anything", &[], &[], 1);
    assert_eq!(result, SymbolId::default(), "Empty map must return default");
}

/// R4: Non-import symbol with matching name is skipped.
#[test]
fn r4_non_import_symbol_skipped() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_symbol(
            "Path",
            SymbolKind::Function,
            Visibility::Public,
            "test.rs",
            5,
        ),
        make_import("Path", "test.rs", 10),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    // ref_line=5 is AT the function, but import is at line 10 — should still find the import
    let result = resolve_import_by_name("Path", &map, &syms, 5);
    assert_eq!(
        result, map[1].1,
        "Must return the import's SymbolId, not the function's"
    );
}

// ===========================================================================
// Category 2: Core Fix — Duplicate Import Name Resolution (D1-D6)
// ===========================================================================

/// D1: Two imports of same name, reference nearest to second.
#[test]
fn d1_two_imports_reference_nearest_second() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_import("Path", "test.rs", 10),
        make_import("Path", "test.rs", 50),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("Path", &map, &syms, 55);
    assert_eq!(
        result, map[1].1,
        "Reference at line 55 must resolve to import at line 50 (nearest preceding)"
    );
}

/// D2: Two imports of same name, reference nearest to first.
#[test]
fn d2_two_imports_reference_nearest_first() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_import("Path", "test.rs", 10),
        make_import("Path", "test.rs", 50),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("Path", &map, &syms, 15);
    assert_eq!(
        result, map[0].1,
        "Reference at line 15 must resolve to import at line 10 (nearest preceding)"
    );
}

/// D3: Three imports of same name, reference between second and third.
#[test]
fn d3_three_imports_reference_between_second_and_third() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_import("Graph", "test.rs", 10),
        make_import("Graph", "test.rs", 100),
        make_import("Graph", "test.rs", 200),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("Graph", &map, &syms, 150);
    assert_eq!(
        result, map[1].1,
        "Reference at line 150 must resolve to import at line 100 (nearest preceding)"
    );
}

/// D4: Five imports of same name (realistic test file pattern).
#[test]
fn d4_five_imports_each_resolves_to_nearest() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_import("Path", "test.rs", 20),
        make_import("Path", "test.rs", 80),
        make_import("Path", "test.rs", 140),
        make_import("Path", "test.rs", 200),
        make_import("Path", "test.rs", 260),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);

    let test_cases = vec![
        (25, 0, "line 25 → import at line 20"),
        (85, 1, "line 85 → import at line 80"),
        (145, 2, "line 145 → import at line 140"),
        (205, 3, "line 205 → import at line 200"),
        (265, 4, "line 265 → import at line 260"),
    ];

    for (ref_line, expected_idx, desc) in test_cases {
        let result = resolve_import_by_name("Path", &map, &syms, ref_line);
        assert_eq!(result, map[expected_idx].1, "D4: {}", desc);
    }
}

/// D5: Duplicate function imports (not just types).
#[test]
fn d5_duplicate_function_imports() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_import("relativize_path", "test.rs", 30),
        make_import("relativize_path", "test.rs", 90),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("relativize_path", &map, &syms, 95);
    assert_eq!(
        result, map[1].1,
        "Function import at line 90 must win for reference at line 95"
    );
}

/// D6: Mixed imports, only some duplicated.
#[test]
fn d6_mixed_imports_only_some_duplicated() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_import("os", "test.rs", 5),
        make_import("Path", "test.rs", 10),
        make_import("Path", "test.rs", 50),
        make_import("sys", "test.rs", 55),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("Path", &map, &syms, 52);
    assert_eq!(
        result, map[2].1,
        "Duplicate 'Path' at line 50 must win for reference at line 52"
    );
}

// ===========================================================================
// Category 3: Adversarial — Over-Resolution and Wrong-Scope (A1-A5)
// ===========================================================================

/// A1: Reference before any import of that name — fallback to any match.
#[test]
fn a1_reference_before_any_import_falls_back() {
    let mut graph = Graph::new();
    let symbols = vec![make_import("Path", "test.rs", 50)];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("Path", &map, &syms, 10);
    assert_eq!(
        result, map[0].1,
        "When no preceding import exists, fall back to any matching import"
    );
}

/// A2: Reference at exact same line as import.
#[test]
fn a2_reference_at_same_line_as_import() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_import("Path", "test.rs", 10),
        make_import("Path", "test.rs", 50),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("Path", &map, &syms, 50);
    assert_eq!(
        result, map[1].1,
        "Same-line match (line 50) should match that import"
    );
}

/// A3: Multiple equidistant imports (same line number) — deterministic.
#[test]
fn a3_equidistant_imports_deterministic() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_import("Path", "test.rs", 10),
        make_import("Path", "test.rs", 10),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("Path", &map, &syms, 15);
    // Either import is acceptable — just must not panic or return default
    assert_ne!(
        result,
        SymbolId::default(),
        "Must resolve to one of the imports, not default"
    );
    // Deterministic: first encountered at same line wins
    assert_eq!(
        result, map[0].1,
        "First import at same line wins (deterministic tiebreak)"
    );
}

/// A4: Proximity must not cross file boundaries (inherent from per-file symbol_id_map).
#[test]
fn a4_proximity_per_file_only() {
    let mut graph = Graph::new();
    let symbols_a = vec![make_import("Path", "file_a.rs", 10)];
    let (syms_a, map_a) = build_symbol_map(&mut graph, symbols_a);

    let symbols_b = vec![make_import("Path", "file_b.rs", 5)];
    let (_syms_b, _map_b) = build_symbol_map(&mut graph, symbols_b);

    let result = resolve_import_by_name("Path", &map_a, &syms_a, 15);
    assert_eq!(
        result, map_a[0].1,
        "File A's import must resolve within file A's symbol_id_map"
    );
}

/// A5: Over-resolution guard: don't resolve to wrong-name import even if closer.
#[test]
fn a5_name_match_overrides_proximity() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_import("os", "test.rs", 48),
        make_import("Path", "test.rs", 10),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("Path", &map, &syms, 50);
    assert_eq!(
        result, map[1].1,
        "Name match ('Path' at line 10) must override proximity ('os' at line 48)"
    );
}

// ===========================================================================
// Category 4: Edge Cases — Location Boundaries (E1-E4)
// ===========================================================================

/// E1: Reference at line 0 (invalid but shouldn't crash).
#[test]
fn e1_reference_at_line_zero() {
    let mut graph = Graph::new();
    let symbols = vec![make_import("Path", "test.rs", 10)];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("Path", &map, &syms, 0);
    assert_eq!(
        result, map[0].1,
        "Line 0 reference must fallback to any matching import (line 10)"
    );
}

/// E2: Reference at u32::MAX line.
#[test]
fn e2_reference_at_u32_max() {
    let mut graph = Graph::new();
    let symbols = vec![make_import("Path", "test.rs", 10)];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("Path", &map, &syms, u32::MAX);
    assert_eq!(
        result, map[0].1,
        "u32::MAX reference line must resolve to import at line 10"
    );
}

/// E3: Import with empty name.
#[test]
fn e3_import_with_empty_name() {
    let mut graph = Graph::new();
    let symbols = vec![make_import("", "test.rs", 10)];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("", &map, &syms, 15);
    assert_eq!(
        result, map[0].1,
        "Empty name match must still work (exact match)"
    );
}

/// E4: Large number of duplicate imports (stress test).
#[test]
fn e4_100_duplicate_imports_stress() {
    let mut graph = Graph::new();
    let symbols: Vec<Symbol> = (1..=100)
        .map(|i| make_import("Path", "test.rs", i * 10))
        .collect();
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("Path", &map, &syms, 555);
    assert_eq!(
        result, map[54].1,
        "Reference at line 555 must resolve to import at line 550 (index 54)"
    );
}

// ===========================================================================
// Category 5: Integration — End-to-End Pipeline (I1-I3)
// ===========================================================================

/// I1: Rust file with duplicate Path imports produces fewer phantom_dependency.
///
/// Simulates a file with two test functions, each importing Path and using it.
/// After proximity fix, both imports should have incoming edges — no phantom.
#[test]
fn i1_duplicate_imports_no_phantom_dependency() {
    use crate::analyzer::patterns::phantom_dependency;
    use crate::graph::populate_graph;
    use std::path::PathBuf;

    let mut graph = Graph::new();

    // Build a ParseResult simulating two test functions each importing Path
    let result = ParseResult {
        scopes: vec![Scope {
            id: ScopeId::default(),
            name: "test.rs".to_string(),
            kind: ScopeKind::Module,
            parent: None,
            location: Location {
                file: PathBuf::from("test.rs"),
                line: 1,
                column: 1,
                end_line: 100,
                end_column: 1,
            },
        }],
        symbols: vec![
            // First test function's import at line 5
            {
                let s = Symbol {
                    id: SymbolId::default(),
                    kind: SymbolKind::Variable,
                    name: "Path".to_string(),
                    qualified_name: "test::Path".to_string(),
                    visibility: Visibility::Public,
                    signature: None,
                    location: Location {
                        file: PathBuf::from("test.rs"),
                        line: 5,
                        column: 1,
                        end_line: 5,
                        end_column: 20,
                    },
                    resolution: ResolutionStatus::Resolved,
                    scope: ScopeId::default(),
                    annotations: vec!["import".to_string()],
                };
                s
            },
            // First test function at lines 4-20
            Symbol {
                id: SymbolId::default(),
                kind: SymbolKind::Function,
                name: "test_one".to_string(),
                qualified_name: "test::test_one".to_string(),
                visibility: Visibility::Public,
                signature: None,
                location: Location {
                    file: PathBuf::from("test.rs"),
                    line: 4,
                    column: 1,
                    end_line: 20,
                    end_column: 1,
                },
                resolution: ResolutionStatus::Resolved,
                scope: ScopeId::default(),
                annotations: vec![],
            },
            // Second test function's import at line 25
            {
                let s = Symbol {
                    id: SymbolId::default(),
                    kind: SymbolKind::Variable,
                    name: "Path".to_string(),
                    qualified_name: "test::Path".to_string(),
                    visibility: Visibility::Public,
                    signature: None,
                    location: Location {
                        file: PathBuf::from("test.rs"),
                        line: 25,
                        column: 1,
                        end_line: 25,
                        end_column: 20,
                    },
                    resolution: ResolutionStatus::Resolved,
                    scope: ScopeId::default(),
                    annotations: vec!["import".to_string()],
                };
                s
            },
            // Second test function at lines 24-40
            Symbol {
                id: SymbolId::default(),
                kind: SymbolKind::Function,
                name: "test_two".to_string(),
                qualified_name: "test::test_two".to_string(),
                visibility: Visibility::Public,
                signature: None,
                location: Location {
                    file: PathBuf::from("test.rs"),
                    line: 24,
                    column: 1,
                    end_line: 40,
                    end_column: 1,
                },
                resolution: ResolutionStatus::Resolved,
                scope: ScopeId::default(),
                annotations: vec![],
            },
        ],
        references: vec![
            // First usage: attribute_access:Path at line 10 (inside test_one)
            Reference {
                id: ReferenceId::default(),
                from: SymbolId::default(),
                to: SymbolId::default(),
                kind: ReferenceKind::Read,
                location: Location {
                    file: PathBuf::from("test.rs"),
                    line: 10,
                    column: 5,
                    end_line: 10,
                    end_column: 15,
                },
                resolution: ResolutionStatus::Partial("attribute_access:Path".to_string()),
            },
            // Second usage: attribute_access:Path at line 30 (inside test_two)
            Reference {
                id: ReferenceId::default(),
                from: SymbolId::default(),
                to: SymbolId::default(),
                kind: ReferenceKind::Read,
                location: Location {
                    file: PathBuf::from("test.rs"),
                    line: 30,
                    column: 5,
                    end_line: 30,
                    end_column: 15,
                },
                resolution: ResolutionStatus::Partial("attribute_access:Path".to_string()),
            },
        ],
        boundaries: vec![],
    };

    populate_graph(&mut graph, &result);

    let diagnostics = phantom_dependency::detect(&graph, std::path::Path::new("."));
    let phantom_paths: Vec<&str> = diagnostics
        .iter()
        .filter(|d| d.entity == "Path")
        .map(|d| d.location.as_str())
        .collect();

    assert!(
        phantom_paths.is_empty(),
        "Neither Path import should be phantom — both have edges from their respective usages. \
         Got phantom at: {:?}",
        phantom_paths
    );
}

/// I2: Rust file with single import still has zero phantom_dependency for that import.
#[test]
fn i2_single_import_no_phantom_regression() {
    use crate::analyzer::patterns::phantom_dependency;
    use crate::graph::populate_graph;
    use std::path::PathBuf;

    let mut graph = Graph::new();

    let result = ParseResult {
        scopes: vec![Scope {
            id: ScopeId::default(),
            name: "single.rs".to_string(),
            kind: ScopeKind::Module,
            parent: None,
            location: Location {
                file: PathBuf::from("single.rs"),
                line: 1,
                column: 1,
                end_line: 20,
                end_column: 1,
            },
        }],
        symbols: vec![
            {
                let s = Symbol {
                    id: SymbolId::default(),
                    kind: SymbolKind::Variable,
                    name: "Path".to_string(),
                    qualified_name: "single::Path".to_string(),
                    visibility: Visibility::Public,
                    signature: None,
                    location: Location {
                        file: PathBuf::from("single.rs"),
                        line: 1,
                        column: 1,
                        end_line: 1,
                        end_column: 20,
                    },
                    resolution: ResolutionStatus::Resolved,
                    scope: ScopeId::default(),
                    annotations: vec!["import".to_string()],
                };
                s
            },
            Symbol {
                id: SymbolId::default(),
                kind: SymbolKind::Function,
                name: "use_path".to_string(),
                qualified_name: "single::use_path".to_string(),
                visibility: Visibility::Public,
                signature: None,
                location: Location {
                    file: PathBuf::from("single.rs"),
                    line: 3,
                    column: 1,
                    end_line: 10,
                    end_column: 1,
                },
                resolution: ResolutionStatus::Resolved,
                scope: ScopeId::default(),
                annotations: vec![],
            },
        ],
        references: vec![Reference {
            id: ReferenceId::default(),
            from: SymbolId::default(),
            to: SymbolId::default(),
            kind: ReferenceKind::Read,
            location: Location {
                file: PathBuf::from("single.rs"),
                line: 5,
                column: 5,
                end_line: 5,
                end_column: 15,
            },
            resolution: ResolutionStatus::Partial("attribute_access:Path".to_string()),
        }],
        boundaries: vec![],
    };

    populate_graph(&mut graph, &result);

    let diagnostics = phantom_dependency::detect(&graph, std::path::Path::new("."));
    let phantom_paths: Vec<&str> = diagnostics
        .iter()
        .filter(|d| d.entity == "Path")
        .map(|d| d.location.as_str())
        .collect();

    assert!(
        phantom_paths.is_empty(),
        "Single import with usage must not be phantom. Got: {:?}",
        phantom_paths
    );
}

/// I3: Genuinely unused import is still detected as phantom.
#[test]
fn i3_unused_import_still_detected_as_phantom() {
    use crate::analyzer::patterns::phantom_dependency;
    use crate::graph::populate_graph;
    use std::path::PathBuf;

    let mut graph = Graph::new();

    let result = ParseResult {
        scopes: vec![Scope {
            id: ScopeId::default(),
            name: "unused.rs".to_string(),
            kind: ScopeKind::Module,
            parent: None,
            location: Location {
                file: PathBuf::from("unused.rs"),
                line: 1,
                column: 1,
                end_line: 10,
                end_column: 1,
            },
        }],
        symbols: vec![
            {
                let s = Symbol {
                    id: SymbolId::default(),
                    kind: SymbolKind::Variable,
                    name: "Path".to_string(),
                    qualified_name: "unused::Path".to_string(),
                    visibility: Visibility::Public,
                    signature: None,
                    location: Location {
                        file: PathBuf::from("unused.rs"),
                        line: 1,
                        column: 1,
                        end_line: 1,
                        end_column: 20,
                    },
                    resolution: ResolutionStatus::Resolved,
                    scope: ScopeId::default(),
                    annotations: vec!["import".to_string()],
                };
                s
            },
            Symbol {
                id: SymbolId::default(),
                kind: SymbolKind::Function,
                name: "do_something".to_string(),
                qualified_name: "unused::do_something".to_string(),
                visibility: Visibility::Public,
                signature: None,
                location: Location {
                    file: PathBuf::from("unused.rs"),
                    line: 3,
                    column: 1,
                    end_line: 8,
                    end_column: 1,
                },
                resolution: ResolutionStatus::Resolved,
                scope: ScopeId::default(),
                annotations: vec![],
            },
        ],
        // NO references — Path is never used
        references: vec![],
        boundaries: vec![],
    };

    populate_graph(&mut graph, &result);

    let diagnostics = phantom_dependency::detect(&graph, std::path::Path::new("."));
    let phantom_count = diagnostics.iter().filter(|d| d.entity == "Path").count();

    assert_eq!(
        phantom_count, 1,
        "Unused import 'Path' MUST still be detected as phantom. Got {} findings.",
        phantom_count
    );
}

// ===========================================================================
// Category 6: Regression Tests from Previous Cycles (REG1-REG2)
// ===========================================================================

/// REG1: C13 dotted callee rejection still works.
#[test]
fn reg1_dotted_callee_still_rejected() {
    use crate::graph::populate_graph;
    use std::path::PathBuf;

    let mut graph = Graph::new();

    // Create a parse result with a dotted callee reference
    let result = ParseResult {
        scopes: vec![Scope {
            id: ScopeId::default(),
            name: "dotted.rs".to_string(),
            kind: ScopeKind::Module,
            parent: None,
            location: Location {
                file: PathBuf::from("dotted.rs"),
                line: 1,
                column: 1,
                end_line: 20,
                end_column: 1,
            },
        }],
        symbols: vec![
            make_import("module", "dotted.rs", 1),
            Symbol {
                id: SymbolId::default(),
                kind: SymbolKind::Function,
                name: "func".to_string(),
                qualified_name: "dotted::func".to_string(),
                visibility: Visibility::Public,
                signature: None,
                location: Location {
                    file: PathBuf::from("dotted.rs"),
                    line: 1,
                    column: 1,
                    end_line: 20,
                    end_column: 1,
                },
                resolution: ResolutionStatus::Resolved,
                scope: ScopeId::default(),
                annotations: vec![],
            },
        ],
        references: vec![Reference {
            id: ReferenceId::default(),
            from: SymbolId::default(),
            to: SymbolId::default(),
            kind: ReferenceKind::Call,
            location: Location {
                file: PathBuf::from("dotted.rs"),
                line: 5,
                column: 5,
                end_line: 5,
                end_column: 20,
            },
            // Dotted callee name — should be rejected by resolve_callee
            resolution: ResolutionStatus::Partial("call:module.func".to_string()),
        }],
        boundaries: vec![],
    };

    populate_graph(&mut graph, &result);

    // Dotted call should not create an edge (resolve_callee rejects dotted names)
    let import_id = graph
        .all_symbols()
        .find(|(_, s)| s.name == "module" && s.annotations.contains(&"import".to_string()))
        .map(|(id, _)| id);

    if let Some(id) = import_id {
        let incoming = graph.edges_to(id);
        assert!(
            incoming.is_empty(),
            "Dotted callee 'module.func' must NOT create an edge to the import. Got {} edges.",
            incoming.len()
        );
    }
}

/// REG2: C14 type reference emission still works (attribute_access references resolve).
#[test]
fn reg2_type_reference_emission_still_works() {
    use crate::graph::populate_graph;
    use std::path::PathBuf;

    let mut graph = Graph::new();

    let result = ParseResult {
        scopes: vec![Scope {
            id: ScopeId::default(),
            name: "types.rs".to_string(),
            kind: ScopeKind::Module,
            parent: None,
            location: Location {
                file: PathBuf::from("types.rs"),
                line: 1,
                column: 1,
                end_line: 20,
                end_column: 1,
            },
        }],
        symbols: vec![
            {
                let s = Symbol {
                    id: SymbolId::default(),
                    kind: SymbolKind::Variable,
                    name: "Path".to_string(),
                    qualified_name: "types::Path".to_string(),
                    visibility: Visibility::Public,
                    signature: None,
                    location: Location {
                        file: PathBuf::from("types.rs"),
                        line: 1,
                        column: 1,
                        end_line: 1,
                        end_column: 20,
                    },
                    resolution: ResolutionStatus::Resolved,
                    scope: ScopeId::default(),
                    annotations: vec!["import".to_string()],
                };
                s
            },
            Symbol {
                id: SymbolId::default(),
                kind: SymbolKind::Function,
                name: "foo".to_string(),
                qualified_name: "types::foo".to_string(),
                visibility: Visibility::Public,
                signature: None,
                location: Location {
                    file: PathBuf::from("types.rs"),
                    line: 3,
                    column: 1,
                    end_line: 10,
                    end_column: 1,
                },
                resolution: ResolutionStatus::Resolved,
                scope: ScopeId::default(),
                annotations: vec![],
            },
        ],
        references: vec![Reference {
            id: ReferenceId::default(),
            from: SymbolId::default(),
            to: SymbolId::default(),
            kind: ReferenceKind::Read,
            location: Location {
                file: PathBuf::from("types.rs"),
                line: 5,
                column: 15,
                end_line: 5,
                end_column: 19,
            },
            resolution: ResolutionStatus::Partial("attribute_access:Path".to_string()),
        }],
        boundaries: vec![],
    };

    populate_graph(&mut graph, &result);

    // The Path import should have an incoming edge from foo's attribute_access reference
    let import_id = graph
        .all_symbols()
        .find(|(_, s)| {
            s.name == "Path"
                && s.annotations.contains(&"import".to_string())
                && s.location.file == PathBuf::from("types.rs")
        })
        .map(|(id, _)| id);

    assert!(import_id.is_some(), "Path import must exist in graph");
    let id = import_id.unwrap();
    let incoming = graph.edges_to(id);
    assert!(
        !incoming.is_empty(),
        "Path import must have incoming edges from attribute_access reference. Got 0 edges."
    );
}
