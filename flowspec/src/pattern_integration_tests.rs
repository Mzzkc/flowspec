// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Real-data integration tests for diagnostic patterns.
//!
//! Every test uses the real parser pipeline (PythonAdapter → populate_graph →
//! detect_<pattern>), NOT mock graphs. This is the critical test gap identified
//! by Doc-2: mock tests pass (98/98) while real data must also produce correct
//! diagnostics.
//!
//! With Worker 1's call-site detection and intra-file resolution (Concert 3,
//! Cycle 2), `ReferenceKind::Call` edges are now produced and resolved within
//! files. Patterns fire correctly: true positives on dead code, true negatives
//! on clean code, isolated clusters on self-referencing modules.
//!
//! **Remaining limitations:**
//! - phantom_dependency: PythonAdapter does not create import Symbols (only
//!   References), so no symbols have the "import" annotation → zero results.
//! - circular_dependency: Blocked by M5 (cross-file resolution).
//! - missing_reexport: Blocked by fixture gap (no `__init__.py`).

use std::path::{Path, PathBuf};

use crate::analyzer::patterns;
use crate::analyzer::patterns::circular_dependency;
use crate::analyzer::patterns::data_dead_end;
use crate::analyzer::patterns::isolated_cluster;
use crate::analyzer::patterns::layer_violation;
use crate::analyzer::patterns::missing_reexport;
use crate::analyzer::patterns::orphaned_implementation;
use crate::analyzer::patterns::phantom_dependency;
use crate::config::Config;
use crate::graph::populate_graph;
use crate::graph::Graph;
use crate::parser::ir::ReferenceKind;
use crate::parser::python::PythonAdapter;
use crate::parser::LanguageAdapter;
use crate::test_utils::{add_ref, make_symbol};
use crate::{analyze, Confidence, DiagnosticPattern, Severity};

/// Parse a fixture file through the real pipeline and return the populated graph.
///
/// Uses a synthetic path (`fixtures/<name>`) instead of the real fixture path
/// (`tests/fixtures/python/<name>`) to avoid triggering the `/tests/` path-based
/// test file exclusion heuristic in patterns. The fixture content represents
/// production code, not test code — the storage path is an artifact.
fn fixture_graph(fixture_name: &str) -> Graph {
    let real_path = fixture_path(fixture_name);
    let content = std::fs::read_to_string(&real_path)
        .unwrap_or_else(|_| panic!("Fixture {} not found at {:?}", fixture_name, real_path));
    // Use a clean synthetic path that doesn't contain "/tests/"
    let clean_path = PathBuf::from(format!("fixtures/{}", fixture_name));
    let adapter = PythonAdapter::new();
    let parse_result = adapter
        .parse_file(&clean_path, &content)
        .unwrap_or_else(|e| panic!("Failed to parse {}: {:?}", fixture_name, e));
    let mut graph = Graph::new();
    populate_graph(&mut graph, &parse_result);
    graph
}

/// Parse a fixture using its REAL path (including /tests/).
/// Used for test file exclusion tests where we WANT the path check to trigger.
fn fixture_graph_with_real_path(fixture_name: &str) -> Graph {
    let path = fixture_path(fixture_name);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Fixture {} not found at {:?}", fixture_name, path));
    let adapter = PythonAdapter::new();
    let parse_result = adapter
        .parse_file(&path, &content)
        .unwrap_or_else(|e| panic!("Failed to parse {}: {:?}", fixture_name, e));
    let mut graph = Graph::new();
    populate_graph(&mut graph, &parse_result);
    graph
}

/// Parse multiple fixture files into a single graph.
fn multi_fixture_graph(fixtures: &[&str]) -> Graph {
    let adapter = PythonAdapter::new();
    let mut graph = Graph::new();
    for fixture_name in fixtures {
        let real_path = fixture_path(fixture_name);
        let content = std::fs::read_to_string(&real_path)
            .unwrap_or_else(|_| panic!("Fixture {} not found at {:?}", fixture_name, real_path));
        let clean_path = PathBuf::from(format!("fixtures/{}", fixture_name));
        let parse_result = adapter
            .parse_file(&clean_path, &content)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {:?}", fixture_name, e));
        populate_graph(&mut graph, &parse_result);
    }
    graph
}

/// Resolve fixture path from CARGO_MANIFEST_DIR.
fn fixture_path(fixture_name: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .unwrap()
        .join("tests/fixtures/python")
        .join(fixture_name)
}

// =========================================================================
// Category 1: Per-Pattern Real-Data True Positives (P0)
// =========================================================================

/// data_dead_end MUST fire on dead_code.py — unused_helper and _private_util
/// have zero callers anywhere in the codebase.
#[test]
fn test_data_dead_end_real_data_true_positive() {
    let graph = fixture_graph("dead_code.py");
    let diagnostics = data_dead_end::detect(&graph, Path::new(""));

    assert!(
        !diagnostics.is_empty(),
        "data_dead_end MUST fire on dead_code.py — unused_helper and _private_util \
        have zero callers. Got 0 diagnostics."
    );

    let entities: Vec<&str> = diagnostics.iter().map(|d| d.entity.as_str()).collect();

    // True positives: unused_helper and _private_util have zero callers
    assert!(
        entities.iter().any(|e| e.contains("unused_helper")),
        "Must flag unused_helper as dead end. Found entities: {:?}",
        entities
    );
    assert!(
        entities.iter().any(|e| e.contains("_private_util")),
        "Must flag _private_util as dead end. Found entities: {:?}",
        entities
    );

    // main_handler is excluded by name heuristic (starts_with("main_"))
    assert!(
        !entities.iter().any(|e| e.contains("main_handler")),
        "main_handler starts with 'main_' — excluded by name heuristic. Entities: {:?}",
        entities
    );

    // All diagnostics must have correct pattern and severity
    for d in &diagnostics {
        assert_eq!(d.pattern, DiagnosticPattern::DataDeadEnd);
        assert_eq!(d.severity, Severity::Warning);
    }
}

/// orphaned_implementation MUST fire on classes.py — Dog.speak, Dog.species,
/// Animal.speak all have zero callers. __init__ methods are excluded (dunder).
#[test]
fn test_orphaned_implementation_real_data_true_positive() {
    let graph = fixture_graph("classes.py");
    let diagnostics = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(
        !diagnostics.is_empty(),
        "orphaned_implementation MUST fire on classes.py — Dog.speak, Dog.species, \
        Animal.speak all have zero callers. Got 0 diagnostics. \
        If SymbolKind is Function instead of Method, the pattern skips them."
    );

    let entities: Vec<&str> = diagnostics.iter().map(|d| d.entity.as_str()).collect();

    // Dunder methods must be excluded
    assert!(
        !entities.iter().any(|e| e.contains("__init__")),
        "__init__ is a dunder method — must be excluded. Entities: {:?}",
        entities
    );

    // At least one method should be flagged (speak or species)
    let has_real_method = entities
        .iter()
        .any(|e| e.contains("speak") || e.contains("species"));
    assert!(
        has_real_method,
        "Expected at least one orphaned method (speak, species). Got: {:?}",
        entities
    );

    for d in &diagnostics {
        assert_eq!(d.pattern, DiagnosticPattern::OrphanedImplementation);
        assert_eq!(d.severity, Severity::Warning);
    }
}

// =========================================================================
// Category 2: Per-Pattern Real-Data True Negatives (P1)
// =========================================================================

/// orphaned_implementation must NOT fire on clean_code.py — it has
/// only functions, no methods. Trivial true negative.
#[test]
fn test_orphaned_implementation_clean_code_true_negative() {
    let graph = fixture_graph("clean_code.py");
    let diagnostics = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(
        diagnostics.is_empty(),
        "orphaned_implementation must not fire on clean_code.py (no methods). Got: {:?}",
        diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
    );
}

/// data_dead_end must exclude main_handler by name heuristic on dead_code.py.
/// active_function must NOT be flagged — it's called by main_handler (resolved
/// via intra-file call resolution).
#[test]
fn test_data_dead_end_name_heuristic_exclusions() {
    let graph = fixture_graph("dead_code.py");
    let diagnostics = data_dead_end::detect(&graph, Path::new(""));

    let entities: Vec<&str> = diagnostics.iter().map(|d| d.entity.as_str()).collect();

    // main_handler excluded: starts_with("main_")
    assert!(
        !entities.iter().any(|e| e.contains("main_handler")),
        "main_handler should be excluded by starts_with('main_') heuristic"
    );

    // active_function is called by main_handler — must NOT be flagged
    assert!(
        !entities.iter().any(|e| e.contains("active_function")),
        "active_function is called by main_handler — must NOT be flagged as dead end. \
        If flagged, intra-file call resolution failed. Entities: {:?}",
        entities
    );
}

/// data_dead_end true negative: clean_code.py should produce ZERO data_dead_end
/// diagnostics because all functions are connected via call chain.
#[test]
fn test_data_dead_end_clean_code_true_negative() {
    let graph = fixture_graph("clean_code.py");
    let diagnostics = data_dead_end::detect(&graph, Path::new(""));

    // clean_code.py: main (excluded by name) → transform_data → read_file
    // All functions have callers or are excluded. Zero diagnostics expected.
    assert!(
        diagnostics.is_empty(),
        "data_dead_end must NOT fire on clean_code.py — all functions are connected. \
        Got {} diagnostics: {:?}. If transform_data or read_file are flagged, \
        intra-file call resolution is broken.",
        diagnostics.len(),
        diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
    );
}

/// isolated_cluster true negative: clean_code.py should produce zero results.
/// The connected component contains main (entry point) → excluded.
#[test]
fn test_isolated_cluster_clean_code_true_negative() {
    let graph = fixture_graph("clean_code.py");
    let diagnostics = isolated_cluster::detect(&graph, Path::new(""));

    // clean_code.py: main (entry point) → transform_data → read_file
    // The component contains "main" which matches is_entry_point(), so excluded.
    assert!(
        diagnostics.is_empty(),
        "isolated_cluster must not fire on clean_code.py — contains entry point 'main'. \
        Got {} diagnostics: {:?}",
        diagnostics.len(),
        diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
    );
}

/// run_all_patterns on clean_code.py should produce ZERO diagnostics.
/// Global false-positive guard — any diagnostic on clean code is a bug.
#[test]
fn test_clean_code_false_positive_guard() {
    let graph = fixture_graph("clean_code.py");
    let diagnostics = patterns::run_all_patterns(&graph, Path::new(""));

    assert!(
        diagnostics.is_empty(),
        "run_all_patterns MUST produce ZERO diagnostics on clean_code.py. \
        Every function is called, every import is used, main is an entry point. \
        Got {} diagnostics: {:?}",
        diagnostics.len(),
        diagnostics
            .iter()
            .map(|d| format!("{:?}: {}", d.pattern, d.entity))
            .collect::<Vec<_>>()
    );
}

// =========================================================================
// Category 3: End-to-End CLI Tests (P0)
// =========================================================================

/// analyze() MUST produce non-empty diagnostics on dead_code.py.
/// This is THE P0 hard gate — proves diagnostics flow from parser to manifest.
#[test]
fn test_end_to_end_cli_diagnostics_non_empty() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    assert!(
        !result.manifest.diagnostics.is_empty(),
        "THE P0 HARD GATE: analyze() MUST produce non-empty diagnostics on dead_code.py. \
        Got 0 diagnostics. Pipeline: PythonAdapter → populate_graph → run_all_patterns → \
        Manifest conversion. One of these stages is dropping data."
    );

    assert!(
        result.has_findings,
        "has_findings must be true when diagnostics exist"
    );

    // Verify at least one diagnostic is data_dead_end
    let has_dead_end = result
        .manifest
        .diagnostics
        .iter()
        .any(|d| d.pattern.contains("data_dead_end") || d.pattern.contains("DataDeadEnd"));
    assert!(
        has_dead_end,
        "Expected at least one data_dead_end diagnostic. Got patterns: {:?}",
        result
            .manifest
            .diagnostics
            .iter()
            .map(|d| &d.pattern)
            .collect::<Vec<_>>()
    );
}

/// clean_code.py FALSE-POSITIVE GUARD — THE most important regression test.
///
/// clean_code.py has: main → transform_data → read_file → Path import.
/// All functions are called. All imports are used. ZERO diagnostics expected.
/// If ANY diagnostic fires on clean_code.py, we have a false positive bug.
#[test]
fn test_end_to_end_cli_clean_code_false_positive_guard() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("clean_code.py"),
        include_str!("../../tests/fixtures/python/clean_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    // With call-site detection: main (excluded by name) calls transform_data,
    // transform_data calls read_file. Both have inbound Call edges → NOT dead ends.
    assert!(
        result.manifest.diagnostics.is_empty(),
        "clean_code.py MUST produce ZERO diagnostics — all functions are called, \
        main is an entry point. Got {} diagnostics: {:?}",
        result.manifest.diagnostics.len(),
        result
            .manifest
            .diagnostics
            .iter()
            .map(|d| format!("{}: {}", d.pattern, d.entity))
            .collect::<Vec<_>>()
    );
}

// =========================================================================
// Category 4: Cross-Pattern Real-Data Tests (P1)
// =========================================================================

/// run_all_patterns fires multiple patterns on multi-fixture graph.
/// With dead_code.py + classes.py, both data_dead_end and
/// orphaned_implementation should fire.
#[test]
fn test_run_all_patterns_real_data_multi_pattern() {
    let graph = multi_fixture_graph(&["dead_code.py", "classes.py"]);

    let diagnostics = patterns::run_all_patterns(&graph, Path::new(""));

    assert!(
        !diagnostics.is_empty(),
        "run_all_patterns MUST produce diagnostics on combined fixtures"
    );

    // Collect distinct patterns
    let distinct_patterns: std::collections::HashSet<_> =
        diagnostics.iter().map(|d| d.pattern).collect();

    assert!(
        distinct_patterns.len() >= 2,
        "Expected at least 2 distinct patterns (DataDeadEnd + OrphanedImplementation). Got: {:?}",
        distinct_patterns
    );

    // Verify sequential IDs
    for (i, d) in diagnostics.iter().enumerate() {
        let expected_id = format!("D{:03}", i + 1);
        assert_eq!(
            d.id, expected_id,
            "Diagnostic IDs must be sequential. Expected '{}', got '{}'",
            expected_id, d.id
        );
    }

    // Verify each diagnostic has non-empty evidence
    for d in &diagnostics {
        assert!(
            !d.evidence.is_empty(),
            "Diagnostic {} ({:?}) has empty evidence",
            d.id,
            d.pattern
        );
    }
}

/// data_dead_end and orphaned_implementation can fire on the same Method symbol.
/// This is intentional — different diagnostic angle, different messages.
#[test]
fn test_overlap_dead_end_and_orphaned_on_same_method() {
    let graph = fixture_graph("classes.py");

    let dead_end_diags = data_dead_end::detect(&graph, Path::new(""));
    let orphaned_diags = orphaned_implementation::detect(&graph, Path::new(""));

    // Both patterns should fire on uncalled methods in classes.py
    assert!(
        !dead_end_diags.is_empty() || !orphaned_diags.is_empty(),
        "At least one pattern should fire on classes.py (uncalled methods exist)"
    );

    // If both fire, check they have different pattern identity
    if !dead_end_diags.is_empty() && !orphaned_diags.is_empty() {
        let dead_end_entities: std::collections::HashSet<_> =
            dead_end_diags.iter().map(|d| d.entity.clone()).collect();
        let orphaned_entities: std::collections::HashSet<_> =
            orphaned_diags.iter().map(|d| d.entity.clone()).collect();
        let overlap: Vec<_> = dead_end_entities.intersection(&orphaned_entities).collect();

        // Overlapping entities should have distinct diagnostics
        for entity in &overlap {
            let de = dead_end_diags
                .iter()
                .find(|d| &d.entity == *entity)
                .unwrap();
            let oi = orphaned_diags
                .iter()
                .find(|d| &d.entity == *entity)
                .unwrap();
            assert_ne!(
                de.pattern, oi.pattern,
                "Overlapping diagnostics must have different patterns"
            );
            assert_ne!(
                de.message, oi.message,
                "Overlapping diagnostics must have different messages"
            );
        }
    }
}

// =========================================================================
// Category 5: Exclusion Guards (P0/P1)
// =========================================================================

/// test_module.py must produce ZERO diagnostics from all patterns.
/// Test files are excluded by path-based heuristic in data_dead_end
/// and orphaned_implementation. Uses the REAL path to trigger the
/// `/tests/` exclusion check.
#[test]
fn test_test_file_exclusion_guard() {
    let graph = fixture_graph_with_real_path("test_module.py");
    let diagnostics = patterns::run_all_patterns(&graph, Path::new(""));

    // test_module.py: file path contains "test_" → excluded from all patterns
    assert!(
        diagnostics.is_empty(),
        "test_module.py must produce ZERO diagnostics — test files are excluded. \
        Got {}: {:?}",
        diagnostics.len(),
        diagnostics
            .iter()
            .map(|d| format!("{:?}: {}", d.pattern, d.entity))
            .collect::<Vec<_>>()
    );
}

/// Dunder methods (__init__) must be excluded from both data_dead_end
/// and orphaned_implementation on classes.py.
#[test]
fn test_dunder_exclusion_on_real_data() {
    let graph = fixture_graph("classes.py");

    let dead_end_diags = data_dead_end::detect(&graph, Path::new(""));
    let orphaned_diags = orphaned_implementation::detect(&graph, Path::new(""));

    let all_entities: Vec<&str> = dead_end_diags
        .iter()
        .chain(orphaned_diags.iter())
        .map(|d| d.entity.as_str())
        .collect();

    assert!(
        !all_entities.iter().any(|e| e.contains("__init__")),
        "__init__ must be excluded from BOTH data_dead_end and orphaned_implementation. \
        Found in entities: {:?}",
        all_entities
    );
}

// =========================================================================
// Category 6: Blocked Pattern Validation (P2)
// =========================================================================

/// circular_dependency produces zero results on single-file data.
/// Intra-file references are excluded at line 98. Without cross-file
/// resolution (M5), no module adjacency graph forms.
#[test]
fn test_circular_dependency_blocked_single_file() {
    let graph = fixture_graph("dead_code.py");
    let diagnostics = circular_dependency::detect(&graph, Path::new(""));

    assert!(
        diagnostics.is_empty(),
        "circular_dependency should NOT fire on single-file data — requires M5. Got: {:?}",
        diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
    );
}

/// missing_reexport produces zero results without __init__.py fixture.
/// No package structure exists in the fixtures.
#[test]
fn test_missing_reexport_blocked_no_init() {
    let graph = fixture_graph("dead_code.py");
    let diagnostics = missing_reexport::detect(&graph, Path::new(""));

    assert!(
        diagnostics.is_empty(),
        "missing_reexport should NOT fire without __init__.py. Got: {:?}",
        diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
    );
}

/// phantom_dependency produces zero results on dead_code.py.
/// dead_code.py has no imports, so no import symbols exist.
#[test]
fn test_phantom_dependency_no_imports_silent() {
    let graph = fixture_graph("dead_code.py");
    let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

    assert!(
        diagnostics.is_empty(),
        "phantom_dependency should not fire on dead_code.py (no imports). Got: {:?}",
        diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
    );
}

/// isolated_cluster fires on isolated_module.py now that call-site detection
/// produces intra-file Call edges. Connected components form real clusters
/// (e.g., Processor + run + process + validate with internal Call edges).
///
/// Updated by Worker 1 per TODO(worker-1) after call-site detection landed.
#[test]
fn test_isolated_cluster_fires_with_call_edges() {
    let graph = fixture_graph("isolated_module.py");
    let diagnostics = isolated_cluster::detect(&graph, Path::new(""));

    // With Call edges from call-site detection, connected_components() finds
    // real clusters. isolated_cluster should fire on groups with internal edges
    // but zero external inbound edges.
    assert!(
        !diagnostics.is_empty(),
        "isolated_cluster should fire on isolated_module.py now that Call edges exist. \
        Got zero diagnostics — call-site detection may not be producing edges for this fixture."
    );
}

// =========================================================================
// Category 7: Confidence Calibration (P1)
// =========================================================================

/// data_dead_end confidence must match visibility on public_api.py:
/// - _internal_helper: Private/underscore → High confidence
/// - format_timestamp, parse_duration: Public → Low confidence
#[test]
fn test_data_dead_end_confidence_calibration() {
    let graph = fixture_graph("public_api.py");
    let diagnostics = data_dead_end::detect(&graph, Path::new(""));

    assert!(
        diagnostics.len() >= 2,
        "public_api.py should have at least 2 dead-end functions. Got {}",
        diagnostics.len()
    );

    for d in &diagnostics {
        if d.entity.contains("_internal_helper") {
            assert_eq!(
                d.confidence,
                Confidence::High,
                "_internal_helper is private/underscore — must be High confidence"
            );
        } else if d.entity.contains("format_timestamp") || d.entity.contains("parse_duration") {
            assert_eq!(
                d.confidence,
                Confidence::Low,
                "Public function '{}' with no underscore — must be Low confidence",
                d.entity
            );
        }
    }
}

/// Verify the confidence split: private functions get High, public get Low.
/// This is a focused test on the visibility-based confidence heuristic
/// using real parsed data (tree-sitter visibility extraction).
#[test]
fn test_confidence_split_private_vs_public_real_data() {
    let graph = fixture_graph("public_api.py");
    let diagnostics = data_dead_end::detect(&graph, Path::new(""));

    let high_confidence: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.confidence == Confidence::High)
        .collect();
    let low_confidence: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.confidence == Confidence::Low)
        .collect();

    assert!(
        !high_confidence.is_empty(),
        "Must have at least one High confidence diagnostic (_internal_helper)"
    );
    assert!(
        !low_confidence.is_empty(),
        "Must have at least one Low confidence diagnostic (public functions)"
    );
}

// =========================================================================
// Category 8: Infrastructure Tests
// =========================================================================

/// Verify the fixture_graph helper correctly parses and populates.
/// This is the foundation all other tests depend on.
#[test]
fn test_fixture_graph_helper_produces_symbols() {
    let graph = fixture_graph("basic_functions.py");

    assert_eq!(
        graph.symbol_count(),
        3,
        "basic_functions.py has exactly 3 functions"
    );

    let names: Vec<String> = graph.all_symbols().map(|(_, s)| s.name.clone()).collect();
    assert!(names.contains(&"greet".to_string()));
    assert!(names.contains(&"add".to_string()));
    assert!(names.contains(&"_private_helper".to_string()));
}

/// multi_fixture_graph produces a combined graph with symbols from all files.
#[test]
fn test_multi_fixture_graph_combines_files() {
    let graph = multi_fixture_graph(&["basic_functions.py", "classes.py"]);

    // basic_functions: 3 functions, classes: 2 classes + 4+ methods
    assert!(
        graph.symbol_count() >= 9,
        "Combined graph should have >= 9 symbols, got {}",
        graph.symbol_count()
    );
}

// =========================================================================
// Category 9: Diagnostic Loc Relative Paths (P0 — D1 fix validation)
// =========================================================================

/// All patterns must produce relative diagnostic loc on directory analysis.
/// This is the exact bug 5/5 reviewers flagged in cycle 2.
#[test]
fn test_diagnostic_loc_relative_directory_analysis() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    for diag in &result.manifest.diagnostics {
        assert!(
            !diag.loc.starts_with('/'),
            "Diagnostic loc must be relative, not absolute. Got: '{}'",
            diag.loc
        );
        // Must match pattern: filename.py:N
        assert!(
            diag.loc.contains(':'),
            "Diagnostic loc must contain ':' separator. Got: '{}'",
            diag.loc
        );
    }
}

/// Single-file analysis must produce relative loc with just filename.
#[test]
fn test_diagnostic_loc_relative_single_file_analysis() {
    let tmp = tempfile::tempdir().unwrap();
    let file_path = tmp.path().join("dead_code.py");
    std::fs::write(
        &file_path,
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(&file_path, None).unwrap();
    let result = analyze(&file_path, &config, &["python".to_string()]).unwrap();

    for diag in &result.manifest.diagnostics {
        assert!(
            !diag.loc.is_empty(),
            "Diagnostic loc must not be empty on single-file analysis"
        );
        assert!(
            !diag.loc.starts_with(':'),
            "Diagnostic loc must not start with ':'. Got: '{}'",
            diag.loc
        );
        assert!(
            diag.loc.contains("dead_code.py"),
            "Single-file loc must contain filename. Got: '{}'",
            diag.loc
        );
    }
}

/// Diagnostic loc format must match entity loc format for the same file.
#[test]
fn test_diagnostic_loc_matches_entity_loc_format() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    // Extract file prefixes from entity locs and diagnostic locs
    let entity_prefixes: std::collections::HashSet<String> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next().map(|s| s.to_string()))
        .collect();

    for diag in &result.manifest.diagnostics {
        let diag_prefix = diag.loc.split(':').next().unwrap_or("");
        if !diag_prefix.is_empty() {
            assert!(
                entity_prefixes.contains(diag_prefix),
                "Diagnostic loc file prefix '{}' not found in entity locs {:?}",
                diag_prefix,
                entity_prefixes
            );
        }
    }
}

/// Evidence location fields must also use relative paths.
#[test]
fn test_evidence_location_fields_relative() {
    let graph = fixture_graph("dead_code.py");
    let project_root = Path::new("");

    let dead_end = data_dead_end::detect(&graph, project_root);
    let orphaned = orphaned_implementation::detect(&graph, project_root);
    let isolated = isolated_cluster::detect(&graph, project_root);

    for diag in dead_end
        .iter()
        .chain(orphaned.iter())
        .chain(isolated.iter())
    {
        for ev in &diag.evidence {
            if let Some(ref loc) = ev.location {
                assert!(
                    !loc.starts_with('/'),
                    "Evidence location must be relative, got: '{}'",
                    loc
                );
            }
        }
    }
}

/// Diagnostic loc line numbers must be preserved correctly.
#[test]
fn test_diagnostic_loc_preserves_line_numbers() {
    let graph = fixture_graph("dead_code.py");
    let results = data_dead_end::detect(&graph, Path::new(""));

    for d in &results {
        let parts: Vec<&str> = d.location.rsplitn(2, ':').collect();
        assert!(
            parts.len() == 2,
            "Loc must be file:line format. Got: '{}'",
            d.location
        );
        let line_num: u32 = parts[0]
            .parse()
            .unwrap_or_else(|_| panic!("Line number must be numeric. Got: '{}'", parts[0]));
        assert!(line_num > 0, "Line number must be > 0");
    }
}

// =========================================================================
// Category 10: Exclusion Consolidation — Uniform Behavior (P0 — D2 validation)
// =========================================================================

/// Entry point names must be excluded uniformly across all patterns.
#[test]
fn test_entry_point_names_excluded_uniformly() {
    use crate::graph::Graph;
    use crate::parser::ir::*;
    use crate::test_utils::make_symbol;

    let mut graph = Graph::new();
    let project_root = Path::new("");

    // Add symbols with entry point names — all Function, zero callers
    for name in &[
        "main",
        "__main__",
        "if_name_main",
        "main_handler",
        "setup_main",
    ] {
        graph.add_symbol(make_symbol(
            name,
            SymbolKind::Function,
            Visibility::Public,
            "module.py",
            1,
        ));
    }

    let dead_end = data_dead_end::detect(&graph, project_root);
    let orphaned = orphaned_implementation::detect(&graph, project_root);

    let all_entities: Vec<&str> = dead_end
        .iter()
        .chain(orphaned.iter())
        .map(|d| d.entity.as_str())
        .collect();

    for name in &[
        "main",
        "__main__",
        "if_name_main",
        "main_handler",
        "setup_main",
    ] {
        assert!(
            !all_entities.iter().any(|e| e.contains(name)),
            "Entry point '{}' should be excluded from all patterns",
            name
        );
    }
}

/// Test file suffix detection (_test.py, _test.rs) works for all patterns.
#[test]
fn test_file_suffix_exclusion() {
    use crate::graph::Graph;
    use crate::parser::ir::*;
    use crate::test_utils::make_symbol;

    let mut graph = Graph::new();
    let project_root = Path::new("");

    // Symbols in _test.py and _test.rs files
    graph.add_symbol(make_symbol(
        "helper",
        SymbolKind::Function,
        Visibility::Public,
        "utils_test.py",
        1,
    ));
    graph.add_symbol(make_symbol(
        "handler",
        SymbolKind::Function,
        Visibility::Public,
        "handler_test.rs",
        1,
    ));

    let dead_end = data_dead_end::detect(&graph, project_root);
    let orphaned = orphaned_implementation::detect(&graph, project_root);

    assert_eq!(
        dead_end.len(),
        0,
        "data_dead_end must exclude _test.py suffix files"
    );
    assert_eq!(
        orphaned.len(),
        0,
        "orphaned_implementation must exclude _test.py suffix files"
    );
}

/// Windows path normalization must work in exclusion checks.
#[test]
fn test_windows_path_normalization_in_exclusion() {
    use crate::graph::Graph;
    use crate::parser::ir::*;
    use crate::test_utils::make_symbol;

    let mut graph = Graph::new();
    let project_root = Path::new("");

    graph.add_symbol(make_symbol(
        "my_func",
        SymbolKind::Function,
        Visibility::Public,
        "src\\tests\\my_module.py",
        5,
    ));

    let dead_end = data_dead_end::detect(&graph, project_root);
    assert_eq!(
        dead_end.len(),
        1,
        "Directory-based /tests/ exclusion removed — only filename patterns like test_*.py trigger exclusion"
    );
}

/// Dunder methods must be excluded uniformly.
#[test]
fn test_dunder_methods_excluded_uniformly() {
    use crate::graph::Graph;
    use crate::parser::ir::*;
    use crate::test_utils::make_symbol;

    let mut graph = Graph::new();
    let project_root = Path::new("");

    for name in &["__init__", "__str__", "__repr__", "__enter__", "__exit__"] {
        graph.add_symbol(make_symbol(
            name,
            SymbolKind::Method,
            Visibility::Public,
            "classes.py",
            1,
        ));
    }

    let dead_end = data_dead_end::detect(&graph, project_root);
    let orphaned = orphaned_implementation::detect(&graph, project_root);

    for diag in dead_end.iter().chain(orphaned.iter()) {
        assert!(
            !(diag.entity.starts_with("__") && diag.entity.ends_with("__")),
            "Dunder method '{}' must be excluded",
            diag.entity
        );
    }
}

// =========================================================================
// Category 11: Regression Guards (P1)
// =========================================================================

/// Regression: data_dead_end true positive unchanged.
#[test]
fn test_regression_data_dead_end_true_positive() {
    let graph = fixture_graph("dead_code.py");
    let project_root = Path::new("");
    let results = data_dead_end::detect(&graph, project_root);

    assert!(
        !results.is_empty(),
        "data_dead_end must still fire on dead_code.py"
    );
    let entities: Vec<&str> = results.iter().map(|d| d.entity.as_str()).collect();
    assert!(
        entities.iter().any(|e| e.contains("unused_helper")),
        "Must flag unused_helper"
    );
    assert!(
        entities.iter().any(|e| e.contains("_private_util")),
        "Must flag _private_util"
    );
    assert!(
        !entities.iter().any(|e| e.contains("main_handler")),
        "main_handler must NOT be detected (entry point exclusion)"
    );
}

/// Regression: clean_code.py zero false positives.
#[test]
fn test_regression_clean_code_zero_false_positives() {
    let graph = fixture_graph("clean_code.py");
    let project_root = Path::new("");

    let all = patterns::run_all_patterns(&graph, project_root);
    assert_eq!(
        all.len(),
        0,
        "clean_code.py must produce ZERO diagnostics after exclusion consolidation. Got: {:?}",
        all.iter().map(|d| &d.entity).collect::<Vec<_>>()
    );
}

/// Regression: confidence calibration unchanged.
#[test]
fn test_regression_confidence_calibration() {
    let graph = fixture_graph("public_api.py");
    let project_root = Path::new("");
    let results = data_dead_end::detect(&graph, project_root);

    for diag in &results {
        if diag.entity.contains("_internal") {
            assert_eq!(diag.confidence, Confidence::High);
        }
        if diag.entity.contains("format_timestamp") || diag.entity.contains("parse_duration") {
            assert_eq!(diag.confidence, Confidence::Low);
        }
    }
}

// =========================================================================
// Category 12: API Contract Tests (P1)
// =========================================================================

/// run_all_patterns accepts and works with project_root parameter.
#[test]
fn test_run_all_patterns_new_signature() {
    let graph = fixture_graph("dead_code.py");
    let project_root = Path::new("");
    let results = patterns::run_all_patterns(&graph, project_root);
    assert!(
        !results.is_empty(),
        "run_all_patterns with project_root must work"
    );
}

/// All 6 individual detect functions accept project_root parameter.
#[test]
fn test_individual_detect_signatures() {
    let graph = fixture_graph("dead_code.py");
    let project_root = Path::new("");

    let _ = data_dead_end::detect(&graph, project_root);
    let _ = orphaned_implementation::detect(&graph, project_root);
    let _ = isolated_cluster::detect(&graph, project_root);
    let _ = phantom_dependency::detect(&graph, project_root);
    let _ = circular_dependency::detect(&graph, project_root);
    let _ = missing_reexport::detect(&graph, project_root);
}

// =========================================================================
// Category 13: Adversarial Tests (P1)
// =========================================================================

/// Empty graph produces zero diagnostics from all patterns.
#[test]
fn test_adversarial_empty_graph() {
    use crate::graph::Graph;

    let graph = Graph::new();
    let project_root = Path::new("/some/project");

    let all = patterns::run_all_patterns(&graph, project_root);
    assert_eq!(all.len(), 0, "Empty graph must produce zero diagnostics");
}

/// Path with spaces in directory name handled correctly.
#[test]
fn test_adversarial_path_with_spaces() {
    use crate::analyzer::patterns::exclusion::relativize_path;

    let project_root = Path::new("/home/user/my project/src");
    let file = Path::new("/home/user/my project/src/module.py");
    let rel = relativize_path(file, project_root);
    assert_eq!(rel, "module.py");
    assert!(!rel.starts_with('/'));
}

/// Single-file analysis where project_root equals the file path.
#[test]
fn test_adversarial_single_file_project_root() {
    use crate::analyzer::patterns::exclusion::relativize_path;

    let project_root = Path::new("/home/user/dead_code.py");
    let file = Path::new("/home/user/dead_code.py");
    let rel = relativize_path(file, project_root);
    assert!(!rel.is_empty(), "Loc must not be empty");
    assert!(!rel.starts_with(':'), "Loc must not start with ':'");
    assert!(rel.contains("dead_code.py"), "Must fall back to filename");
}

/// Deeply nested path preserves full relative structure.
#[test]
fn test_adversarial_deeply_nested_path() {
    use crate::analyzer::patterns::exclusion::relativize_path;

    let project_root = Path::new("/repo");
    let file = Path::new("/repo/src/packages/core/internal/utils/helpers/deep.py");
    let rel = relativize_path(file, project_root);
    assert_eq!(rel, "src/packages/core/internal/utils/helpers/deep.py");
}

/// Mismatched project_root must not panic.
#[test]
fn test_adversarial_mismatched_project_root() {
    use crate::analyzer::patterns::exclusion::relativize_path;

    let project_root = Path::new("/home/alice/project");
    let file = Path::new("/home/bob/other/file.py");
    let rel = relativize_path(file, project_root);
    assert!(!rel.is_empty(), "Must not be empty");
    // Fallback returns the original path — acceptable behavior
}

// =========================================================================
// Category 14: phantom_dependency — Module.Attribute Access (Cycle 5 D2)
// =========================================================================

/// Parse inline Python content through the full pipeline and return the populated graph.
fn graph_from_python_content(filename: &str, content: &str) -> Graph {
    let clean_path = PathBuf::from(format!("fixtures/{}", filename));
    let adapter = PythonAdapter::new();
    let parse_result = adapter
        .parse_file(&clean_path, content)
        .unwrap_or_else(|e| panic!("Failed to parse {}: {:?}", filename, e));
    let mut graph = Graph::new();
    populate_graph(&mut graph, &parse_result);
    graph
}

/// Core false positive fix: `import os; os.path.join()` must NOT be flagged.
#[test]
fn test_phantom_dependency_module_attribute_call_not_flagged() {
    let content = "import os\n\ndef make_path():\n    return os.path.join(\"/tmp\", \"file\")\n";
    let graph = graph_from_python_content("attr_call_os.py", content);
    let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

    let os_findings: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.entity.contains("os"))
        .collect();

    assert!(
        os_findings.is_empty(),
        "os must NOT be flagged as phantom dependency when os.path.join() is used. \
         Got: {:?}",
        os_findings.iter().map(|d| &d.entity).collect::<Vec<_>>()
    );
}

/// Bare attribute access: `import sys; sys.argv[1:]`
#[test]
fn test_phantom_dependency_bare_attribute_not_flagged() {
    let content = "import sys\n\ndef get_args():\n    return sys.argv[1:]\n";
    let graph = graph_from_python_content("attr_sys.py", content);
    let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

    let sys_findings: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.entity.contains("sys"))
        .collect();

    assert!(
        sys_findings.is_empty(),
        "sys must NOT be flagged when sys.argv is accessed"
    );
}

/// Single-level attribute call: `import json; json.dumps(data)`
#[test]
fn test_phantom_dependency_single_attribute_call_not_flagged() {
    let content = "import json\n\ndef serialize(data):\n    return json.dumps(data)\n";
    let graph = graph_from_python_content("attr_json.py", content);
    let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

    let json_findings: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.entity.contains("json"))
        .collect();

    assert!(
        json_findings.is_empty(),
        "json must NOT be flagged when json.dumps() is called"
    );
}

/// from-import direct call must still work (regression guard).
#[test]
fn test_phantom_dependency_from_import_direct_call_regression() {
    let content =
        "from pathlib import Path\n\ndef resolve():\n    p = Path(\".\")\n    return p.resolve()\n";
    let graph = graph_from_python_content("from_path.py", content);
    let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

    let path_findings: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.entity.contains("Path"))
        .collect();

    assert!(
        path_findings.is_empty(),
        "Path must NOT be flagged — direct call already works, this is a regression guard"
    );
}

/// TRUE POSITIVE GUARD: `import os` with NO usage MUST still be flagged.
#[test]
fn test_phantom_dependency_truly_unused_import_still_flagged() {
    let content = "import os\n\ndef process():\n    return \"done\"\n";
    let graph = graph_from_python_content("unused_os.py", content);
    let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

    let os_findings: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.entity.contains("os"))
        .collect();

    assert!(
        !os_findings.is_empty(),
        "os MUST be flagged as phantom when it is imported but never referenced. \
         If this test fails, the fix is too aggressive."
    );
}

/// ADVERSARIAL: Name collision — `import os; os_count = 5`
/// `os_count` is an identifier, NOT an attribute access on `os`.
#[test]
fn test_phantom_dependency_name_collision_not_false_negative() {
    let content = "import os\n\ndef count():\n    os_count = 5\n    return os_count\n";
    let graph = graph_from_python_content("name_collision.py", content);
    let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

    let os_findings: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.entity.contains("os"))
        .collect();

    assert!(
        !os_findings.is_empty(),
        "os MUST be flagged — 'os_count' is a different identifier, not an attribute access on 'os'."
    );
}

/// Fixture test: unused_import.py — sys must NOT be flagged, os MUST be flagged.
#[test]
fn test_phantom_dependency_unused_import_fixture_accuracy() {
    let graph = fixture_graph("unused_import.py");
    let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

    let entity_names: Vec<&str> = diagnostics.iter().map(|d| d.entity.as_str()).collect();

    // TRUE POSITIVES: os and OrderedDict are never used
    assert!(
        entity_names.iter().any(|e| e.contains("os")),
        "os MUST be flagged — it is imported but never referenced in unused_import.py"
    );
    assert!(
        entity_names.iter().any(|e| e.contains("OrderedDict")),
        "OrderedDict MUST be flagged — imported but never used"
    );

    // TRUE NEGATIVES: sys (sys.argv), Path (Path()) are used
    assert!(
        !entity_names.iter().any(|e| e.contains("sys")),
        "sys must NOT be flagged — sys.argv is used on line 16"
    );
    assert!(
        !entity_names.iter().any(|e| e.contains("Path")),
        "Path must NOT be flagged — Path() is called on line 20"
    );
}

/// Multiple attribute accesses on same import.
#[test]
fn test_phantom_dependency_multiple_attribute_accesses_same_import() {
    let content =
        "import os\n\ndef check():\n    os.path.join(\"/a\", \"b\")\n    os.path.exists(\"/c\")\n    os.getcwd()\n";
    let graph = graph_from_python_content("multi_attr.py", content);
    let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

    let os_findings: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.entity.contains("os"))
        .collect();

    assert!(
        os_findings.is_empty(),
        "os must NOT be flagged when accessed via multiple attributes"
    );
}

/// Deeply nested attribute chain: `import a; a.b.c.d.e.f()`
#[test]
fn test_phantom_dependency_deep_attribute_chain() {
    let content = "import a\n\ndef deep():\n    return a.b.c.d.e.f()\n";
    let graph = graph_from_python_content("deep_chain.py", content);
    let diagnostics = phantom_dependency::detect(&graph, Path::new(""));

    let a_findings: Vec<_> = diagnostics.iter().filter(|d| d.entity == "a").collect();

    assert!(
        a_findings.is_empty(),
        "a must NOT be flagged — a.b.c.d.e.f() is a deeply nested attribute access on 'a'"
    );
}

// =========================================================================
// Category 15: Entity ID Uniqueness (Cycle 5 D4)
// =========================================================================

/// Mixed-language project: app.py::hello and app.js::hello must produce distinct entity IDs.
#[test]
fn test_entity_id_unique_across_languages() {
    let py_adapter = PythonAdapter::new();
    let js_adapter = crate::parser::javascript::JsAdapter::new();

    let py_content = "def hello():\n    return 'hi'\n";
    let js_content = "function hello() { return 'hi'; }\n";

    let py_result = py_adapter
        .parse_file(Path::new("app.py"), py_content)
        .unwrap();
    let js_result = js_adapter
        .parse_file(Path::new("app.js"), js_content)
        .unwrap();

    let py_hello = py_result
        .symbols
        .iter()
        .find(|s| s.name == "hello")
        .unwrap();
    let js_hello = js_result
        .symbols
        .iter()
        .find(|s| s.name == "hello")
        .unwrap();

    assert_ne!(
        py_hello.qualified_name, js_hello.qualified_name,
        "Python and JS symbols with same name in same-stem files must have DIFFERENT qualified names. \
         Python: {}, JS: {}",
        py_hello.qualified_name, js_hello.qualified_name
    );
}

/// Regression guard: single-language project IDs must still work.
#[test]
fn test_entity_id_single_language_regression() {
    let graph = fixture_graph("basic_functions.py");
    let symbols: Vec<_> = graph
        .all_symbols()
        .filter(|(_id, s)| s.kind != crate::parser::ir::SymbolKind::Module)
        .collect();

    let qualified_names: Vec<&str> = symbols
        .iter()
        .map(|(_id, s)| s.qualified_name.as_str())
        .collect();

    let unique: std::collections::HashSet<&&str> = qualified_names.iter().collect();
    assert_eq!(
        qualified_names.len(),
        unique.len(),
        "All qualified names must be unique within a single file. Got: {:?}",
        qualified_names
    );
}

/// Edge case: file without extension — no panic.
#[test]
fn test_entity_id_no_extension_no_panic() {
    let py_adapter = PythonAdapter::new();
    let content = "def hello():\n    pass\n";
    let result = py_adapter.parse_file(Path::new("script"), content).unwrap();

    let hello = result.symbols.iter().find(|s| s.name == "hello");
    assert!(
        hello.is_some(),
        "Must extract function even from extensionless file"
    );
}

// =========================================================================
// Category 15: layer_violation Fixture Integration Tests (Cycle 6)
// =========================================================================

/// Resolve fixture path for layered Python fixtures.
fn layer_fixture_path(relative: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .unwrap()
        .join("tests/fixtures/python")
        .join(relative)
}

/// True positive: API→Data cross-file reference MUST produce a layer violation.
///
/// Uses Approach B: real parsing through PythonAdapter + synthetic cross-file edge
/// (simulating M5 resolution). Tests the detect() function works through the real
/// pipeline once M5 supplies resolved cross-file edges.
#[test]
fn test_layer_violation_fixture_true_positive() {
    let adapter = PythonAdapter::new();
    let mut graph = Graph::new();

    // Parse api/routes.py with synthetic path fixtures/api/routes.py
    let api_path = layer_fixture_path("api/routes.py");
    let api_content = std::fs::read_to_string(&api_path).unwrap();
    let api_result = adapter
        .parse_file(Path::new("fixtures/api/routes.py"), &api_content)
        .unwrap();
    populate_graph(&mut graph, &api_result);

    // Parse db/models.py with synthetic path fixtures/db/models.py
    let db_path = layer_fixture_path("db/models.py");
    let db_content = std::fs::read_to_string(&db_path).unwrap();
    let db_result = adapter
        .parse_file(Path::new("fixtures/db/models.py"), &db_content)
        .unwrap();
    populate_graph(&mut graph, &db_result);

    // Find real SymbolIds for the cross-file edge (non-import function symbols)
    let api_fn_id = graph
        .all_symbols()
        .find(|(_, s)| {
            s.location.file.to_string_lossy().contains("api/")
                && !s.annotations.contains(&"import".to_string())
                && s.kind == crate::parser::ir::SymbolKind::Function
        })
        .map(|(id, _)| id)
        .expect("Must have at least one function symbol from api/routes.py");

    let db_fn_id = graph
        .all_symbols()
        .find(|(_, s)| {
            s.location.file.to_string_lossy().contains("db/")
                && !s.annotations.contains(&"import".to_string())
                && s.kind == crate::parser::ir::SymbolKind::Function
        })
        .map(|(id, _)| id)
        .expect("Must have at least one function symbol from db/models.py");

    // Add synthetic cross-file edge (simulating M5 resolution)
    add_ref(
        &mut graph,
        api_fn_id,
        db_fn_id,
        ReferenceKind::Call,
        "fixtures/api/routes.py",
    );

    let diagnostics = layer_violation::detect(&graph, Path::new(""));
    let violations: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.pattern == DiagnosticPattern::LayerViolation)
        .collect();

    assert!(
        !violations.is_empty(),
        "API→Data cross-file reference MUST produce a layer violation"
    );
    assert_eq!(
        violations[0].confidence,
        Confidence::Moderate,
        "Convention-based detection must be Moderate confidence"
    );
    assert_eq!(
        violations[0].evidence.len(),
        3,
        "Must have 3 evidence entries"
    );
}

/// True negative: API→Service (valid layering) should produce zero violations.
#[test]
fn test_layer_violation_fixture_true_negative() {
    let adapter = PythonAdapter::new();
    let mut graph = Graph::new();

    // Parse api/routes.py
    let api_path = layer_fixture_path("api/routes.py");
    let api_content = std::fs::read_to_string(&api_path).unwrap();
    let api_result = adapter
        .parse_file(Path::new("fixtures/api/routes.py"), &api_content)
        .unwrap();
    populate_graph(&mut graph, &api_result);

    // Parse service/logic.py
    let svc_path = layer_fixture_path("service/logic.py");
    let svc_content = std::fs::read_to_string(&svc_path).unwrap();
    let svc_result = adapter
        .parse_file(Path::new("fixtures/service/logic.py"), &svc_content)
        .unwrap();
    populate_graph(&mut graph, &svc_result);

    // Add synthetic cross-file edge api → service (valid layering)
    let api_fn_id = graph
        .all_symbols()
        .find(|(_, s)| {
            s.location.file.to_string_lossy().contains("api/")
                && !s.annotations.contains(&"import".to_string())
                && s.kind == crate::parser::ir::SymbolKind::Function
        })
        .map(|(id, _)| id)
        .expect("Must have function symbol from api/");

    let svc_fn_id = graph
        .all_symbols()
        .find(|(_, s)| {
            s.location.file.to_string_lossy().contains("service/")
                && !s.annotations.contains(&"import".to_string())
                && s.kind == crate::parser::ir::SymbolKind::Function
        })
        .map(|(id, _)| id)
        .expect("Must have function symbol from service/");

    add_ref(
        &mut graph,
        api_fn_id,
        svc_fn_id,
        ReferenceKind::Call,
        "fixtures/api/routes.py",
    );

    let diagnostics = layer_violation::detect(&graph, Path::new(""));
    let violations: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.pattern == DiagnosticPattern::LayerViolation)
        .collect();

    assert!(
        violations.is_empty(),
        "API→Service reference must NOT produce a layer violation, got {} violations",
        violations.len()
    );
}

/// Adversarial: SymbolId::default() edges must be skipped (no false positives
/// from unresolved cross-file imports).
#[test]
fn test_layer_violation_skips_default_symbol_id_edges() {
    use crate::parser::ir::*;

    let mut g = Graph::new();

    let handler = g.add_symbol(make_symbol(
        "handle",
        SymbolKind::Function,
        Visibility::Public,
        "api/handler.py",
        1,
    ));

    // Add edge with SymbolId::default() as target (unresolved import)
    g.add_reference(Reference {
        id: ReferenceId::default(),
        from: handler,
        to: SymbolId::default(),
        kind: ReferenceKind::Import,
        location: Location {
            file: PathBuf::from("api/handler.py"),
            line: 2,
            column: 1,
            end_line: 2,
            end_column: 30,
        },
        resolution: ResolutionStatus::Unresolved,
    });

    let diagnostics = layer_violation::detect(&g, Path::new(""));
    assert!(
        diagnostics.is_empty(),
        "Edges targeting SymbolId::default() must be skipped — no false positives from unresolved imports"
    );
}

/// Same-layer references (API→API) must NOT be flagged.
#[test]
fn test_layer_violation_same_layer_no_finding() {
    use crate::parser::ir::*;

    let mut g = Graph::new();
    let a = g.add_symbol(make_symbol(
        "handler_a",
        SymbolKind::Function,
        Visibility::Public,
        "api/routes.py",
        1,
    ));
    let b = g.add_symbol(make_symbol(
        "handler_b",
        SymbolKind::Function,
        Visibility::Public,
        "api/utils.py",
        1,
    ));
    add_ref(&mut g, a, b, ReferenceKind::Call, "api/routes.py");

    let diagnostics = layer_violation::detect(&g, Path::new(""));
    assert!(
        diagnostics
            .iter()
            .all(|d| d.pattern != DiagnosticPattern::LayerViolation),
        "Same-layer (API→API) reference must NOT be flagged"
    );
}

/// Regression: mock-based and pipeline-based layer_violation tests must agree.
/// The mock test `test_layer_violation_api_imports_db_directly` fires at Moderate.
/// The fixture test must also fire at Moderate for the same logical scenario.
#[test]
fn test_layer_violation_mock_vs_pipeline_parity() {
    use crate::parser::ir::*;

    // Mock graph (same as unit test in layer_violation.rs)
    let mut mock_g = Graph::new();
    let mock_handler = mock_g.add_symbol(make_symbol(
        "handle_request",
        SymbolKind::Function,
        Visibility::Public,
        "api/handler.py",
        1,
    ));
    let mock_db = mock_g.add_symbol(make_symbol(
        "query_db",
        SymbolKind::Function,
        Visibility::Public,
        "db/models.py",
        1,
    ));
    add_ref(
        &mut mock_g,
        mock_handler,
        mock_db,
        ReferenceKind::Call,
        "api/handler.py",
    );
    let mock_diags = layer_violation::detect(&mock_g, Path::new(""));

    // Pipeline graph
    let adapter = PythonAdapter::new();
    let mut pipeline_g = Graph::new();

    let api_path = layer_fixture_path("api/routes.py");
    let api_content = std::fs::read_to_string(&api_path).unwrap();
    let api_result = adapter
        .parse_file(Path::new("fixtures/api/routes.py"), &api_content)
        .unwrap();
    populate_graph(&mut pipeline_g, &api_result);

    let db_path = layer_fixture_path("db/models.py");
    let db_content = std::fs::read_to_string(&db_path).unwrap();
    let db_result = adapter
        .parse_file(Path::new("fixtures/db/models.py"), &db_content)
        .unwrap();
    populate_graph(&mut pipeline_g, &db_result);

    let api_fn_id = pipeline_g
        .all_symbols()
        .find(|(_, s)| {
            s.location.file.to_string_lossy().contains("api/")
                && !s.annotations.contains(&"import".to_string())
                && s.kind == crate::parser::ir::SymbolKind::Function
        })
        .map(|(id, _)| id)
        .expect("Must have function symbol from api/");

    let db_fn_id = pipeline_g
        .all_symbols()
        .find(|(_, s)| {
            s.location.file.to_string_lossy().contains("db/")
                && !s.annotations.contains(&"import".to_string())
                && s.kind == crate::parser::ir::SymbolKind::Function
        })
        .map(|(id, _)| id)
        .expect("Must have function symbol from db/");

    add_ref(
        &mut pipeline_g,
        api_fn_id,
        db_fn_id,
        ReferenceKind::Call,
        "fixtures/api/routes.py",
    );
    let pipeline_diags = layer_violation::detect(&pipeline_g, Path::new(""));

    // Both must fire
    let mock_violations: Vec<_> = mock_diags
        .iter()
        .filter(|d| d.pattern == DiagnosticPattern::LayerViolation)
        .collect();
    let pipeline_violations: Vec<_> = pipeline_diags
        .iter()
        .filter(|d| d.pattern == DiagnosticPattern::LayerViolation)
        .collect();

    assert!(
        !mock_violations.is_empty(),
        "Mock graph must produce layer violation"
    );
    assert!(
        !pipeline_violations.is_empty(),
        "Pipeline graph must produce layer violation"
    );

    // Both must be Moderate confidence
    assert_eq!(
        mock_violations[0].confidence, pipeline_violations[0].confidence,
        "Mock and pipeline must agree on confidence level"
    );
}
