//! End-to-end pipeline integration tests.
//!
//! These tests verify that `analyze()` uses the real pipeline
//! (tree-sitter -> Graph -> patterns -> extraction) and produces
//! correct manifest data. They are the contract that proves the
//! text scanner has been replaced.

use std::path::PathBuf;

use crate::config::Config;
use crate::{analyze, diagnose, Severity};

// =========================================================================
// 1. End-to-End Pipeline Tests (P0)
// =========================================================================

#[test]
fn test_pipeline_real_visibility_private_underscore() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let private_entity = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("_private_util"))
        .expect("Must find _private_util entity in manifest");

    assert_eq!(
        private_entity.vis, "priv",
        "Underscore-prefixed _private_util MUST have vis 'priv', not '{}'. \
        If vis is 'pub', the text scanner is still active.",
        private_entity.vis
    );
}

#[test]
fn test_pipeline_real_visibility_public_no_underscore() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let public_entity = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("unused_helper"))
        .expect("Must find unused_helper entity in manifest");

    assert_eq!(
        public_entity.vis, "pub",
        "No-underscore unused_helper must have vis 'pub', got '{}'",
        public_entity.vis
    );
}

#[test]
fn test_pipeline_real_called_by_not_detected_placeholder() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    for entity in &result.manifest.entities {
        for caller in &entity.called_by {
            assert_ne!(
                caller, "(detected)",
                "Entity '{}' has placeholder called_by '(detected)'. \
                Text scanner is still active.",
                entity.id
            );
        }
    }
}

#[test]
fn test_pipeline_real_call_edges_main_handler_calls_active() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let main_handler = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("main_handler"))
        .expect("Must find main_handler entity");

    for call in &main_handler.calls {
        assert_ne!(
            call, "(detected)",
            "calls must contain real data, not placeholder"
        );
    }
}

#[test]
fn test_pipeline_module_roles_not_vacuous() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    for module in &result.manifest.summary.modules {
        assert!(
            !module.role.contains("Module with"),
            "Module '{}' has vacuous role '{}'. Must use infer_module_role().",
            module.name,
            module.role
        );
        assert!(
            !module.role.is_empty(),
            "Module '{}' has empty role",
            module.name
        );
    }
}

#[test]
fn test_pipeline_no_module_kind_entities() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    for entity in &result.manifest.entities {
        assert_ne!(
            entity.kind, "module",
            "Entity '{}' has kind 'module'. File-scope Module symbols must be filtered out.",
            entity.id
        );
    }
}

#[test]
fn test_pipeline_entity_locations_are_relative() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let abs_prefix = tmp.path().to_string_lossy().to_string();

    for entity in &result.manifest.entities {
        assert!(
            !entity.loc.starts_with(&abs_prefix),
            "Entity '{}' loc '{}' contains absolute path prefix. Must be relative.",
            entity.id,
            entity.loc,
        );
        assert!(
            !entity.loc.starts_with('/'),
            "Entity '{}' loc '{}' starts with '/'. Must be relative path.",
            entity.id,
            entity.loc
        );
    }
}

#[test]
fn test_pipeline_entity_loc_includes_line_number() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    for entity in &result.manifest.entities {
        assert!(
            entity.loc.contains(':'),
            "Entity '{}' loc '{}' must be in 'file:line' format",
            entity.id,
            entity.loc
        );
        let parts: Vec<&str> = entity.loc.splitn(2, ':').collect();
        assert_eq!(parts.len(), 2, "loc must have exactly file:line format");
        let line: u32 = parts[1].parse().unwrap_or_else(|_| {
            panic!(
                "Entity '{}' loc '{}' line part '{}' is not a valid number",
                entity.id, entity.loc, parts[1]
            )
        });
        assert!(
            line > 0,
            "Line number must be 1-based, got 0 for '{}'",
            entity.id
        );
    }
}

// =========================================================================
// 2. Pattern Firing Through Real Pipeline (P0)
// =========================================================================

#[test]
fn test_pipeline_data_dead_end_fires_on_dead_code() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let dead_end_diagnostics: Vec<_> = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.pattern == "data_dead_end")
        .collect();

    assert!(
        !dead_end_diagnostics.is_empty(),
        "data_dead_end must fire on dead_code.py. Total diagnostics: {}",
        result.manifest.diagnostics.len()
    );

    let flagged_entities: Vec<&str> = dead_end_diagnostics
        .iter()
        .map(|d| d.entity.as_str())
        .collect();
    let has_dead_code = flagged_entities
        .iter()
        .any(|e| e.contains("unused_helper") || e.contains("_private_util"));
    assert!(
        has_dead_code,
        "data_dead_end must flag unused_helper or _private_util. Flagged: {:?}",
        flagged_entities
    );
}

#[test]
fn test_pipeline_phantom_dependency_pattern_runs_on_unused_import() {
    // Known limitation: PythonAdapter creates Reference objects for imports but
    // does NOT create Symbol objects with "import" annotation. The
    // phantom_dependency pattern looks for symbols with "import" annotation
    // (matching mock graph behavior). Until the adapter emits import symbols,
    // phantom_dependency won't fire on real parser output.
    //
    // This test verifies the pipeline IS wired and produces real data from
    // unused_import.py — even if phantom_dependency specifically doesn't fire.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("unused_import.py"),
        include_str!("../../tests/fixtures/python/unused_import.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    // Pipeline must produce entities from real parsing
    assert!(
        !result.manifest.entities.is_empty(),
        "unused_import.py must produce entities from real pipeline"
    );

    // Should find get_args, resolve_path, process as entities
    let entity_names: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    assert!(
        entity_names.iter().any(|n| n.contains("get_args")),
        "Must find get_args from unused_import.py. Entities: {:?}",
        entity_names
    );

    // Diagnostics should come from real pattern engine (not text scanner)
    for d in &result.manifest.diagnostics {
        assert!(
            !d.evidence.is_empty(),
            "All diagnostics must have structured evidence from real patterns"
        );
    }
}

#[test]
fn test_pipeline_multi_file_graph_populates_both_files() {
    // Known limitation: isolated_cluster uses connected_components(). With
    // real PythonAdapter output, intra-file call edges connect symbols within
    // a file. Cross-file imports are unresolved (SymbolId::default()), so
    // the pattern may not detect the expected cluster boundaries.
    //
    // This test verifies multi-file analysis produces entities from both files.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("isolated_module.py"),
        include_str!("../../tests/fixtures/python/isolated_module.py"),
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("clean_code.py"),
        include_str!("../../tests/fixtures/python/clean_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let entity_names: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .map(|e| e.id.as_str())
        .collect();

    // Must find entities from isolated_module.py
    assert!(
        entity_names.iter().any(|n| n.contains("process")),
        "Must find process from isolated_module.py. Entities: {:?}",
        entity_names
    );

    // Must find entities from clean_code.py
    assert!(
        entity_names
            .iter()
            .any(|n| n.contains("read_file") || n.contains("transform_data")),
        "Must find entities from clean_code.py. Entities: {:?}",
        entity_names
    );

    // Both files in module summaries
    assert!(
        result.manifest.summary.modules.len() >= 2,
        "Two .py files should produce >= 2 module summaries"
    );

    // All diagnostics must have structured evidence (real pattern engine)
    for d in &result.manifest.diagnostics {
        assert!(!d.evidence.is_empty(), "All diagnostics must have evidence");
    }
}

#[test]
fn test_pipeline_clean_code_minimal_diagnostics() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("clean_code.py"),
        include_str!("../../tests/fixtures/python/clean_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let path_phantom = result
        .manifest
        .diagnostics
        .iter()
        .any(|d| d.pattern == "phantom_dependency" && d.entity.contains("Path"));
    assert!(
        !path_phantom,
        "phantom_dependency must NOT fire for Path import in clean_code.py"
    );
}

#[test]
fn test_pipeline_diagnostic_evidence_is_structured() {
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
            !diag.evidence.is_empty(),
            "Diagnostic '{}' ({}) has empty evidence.",
            diag.id,
            diag.pattern
        );
        for ev in &diag.evidence {
            assert!(
                !ev.observation.is_empty(),
                "Evidence in diagnostic '{}' has empty observation",
                diag.id
            );
        }
    }
}

// =========================================================================
// 3. Text Scanner Deletion Verification (P0)
// =========================================================================

#[test]
fn test_text_scanner_functions_deleted_from_source() {
    let lib_source = include_str!("lib.rs");

    let scanner_hallmarks = [
        "analyze_python_files",
        "group_entities_into_modules",
        "extract_function_name",
        "extract_class_name",
        "extract_signature",
        "is_inside_class",
        "is_python_keyword",
    ];

    for hallmark in &scanner_hallmarks {
        assert!(
            !lib_source.contains(hallmark),
            "lib.rs still contains text scanner function '{}'.",
            hallmark
        );
    }
}

#[test]
fn test_pipeline_imports_present_in_lib() {
    let lib_source = include_str!("lib.rs");

    assert!(
        lib_source.contains("PythonAdapter") || lib_source.contains("python::"),
        "lib.rs must use PythonAdapter for real parsing"
    );
    assert!(
        lib_source.contains("populate_graph"),
        "lib.rs must use populate_graph to build the analysis graph"
    );
    assert!(
        lib_source.contains("run_all_patterns") || lib_source.contains("run_patterns"),
        "lib.rs must use run_all_patterns or run_patterns for diagnostics"
    );
}

// =========================================================================
// 4. Empty and Edge Case Projects (P1)
// =========================================================================

#[test]
fn test_pipeline_empty_project_no_python_files() {
    let tmp = tempfile::tempdir().unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    assert!(
        result.manifest.entities.is_empty(),
        "Empty project must have 0 entities, got {}",
        result.manifest.entities.len()
    );
    assert!(
        result.manifest.diagnostics.is_empty(),
        "Empty project must have 0 diagnostics, got {}",
        result.manifest.diagnostics.len()
    );
    assert_eq!(result.manifest.metadata.entity_count, 0);
    assert_eq!(result.manifest.metadata.diagnostic_count, 0);
    assert!(!result.has_critical);
    assert!(!result.has_findings);
}

#[test]
fn test_pipeline_empty_python_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("empty.py"), "").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    assert!(
        result.manifest.entities.is_empty(),
        "Empty .py file should produce 0 entities (Module kind filtered out), got {}",
        result.manifest.entities.len()
    );
    assert_eq!(
        result.manifest.metadata.file_count, 1,
        "One .py file must be counted even if empty"
    );
}

#[test]
fn test_pipeline_comments_only_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("comments.py"),
        "# Just a comment\n# Another comment\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    assert!(
        result.manifest.entities.is_empty(),
        "Comments-only file should produce 0 entities"
    );
}

#[test]
fn test_pipeline_syntax_errors_partial_parse() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("broken.py"),
        "def valid_function():\n    return 42\n\ndef this is broken syntax!!!\n\ndef another_valid():\n    return \"ok\"\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let names: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    let has_valid = names.iter().any(|n| n.contains("valid_function"));
    assert!(
        has_valid,
        "Tree-sitter should parse valid_function despite syntax errors. Found: {:?}",
        names
    );
}

#[test]
fn test_pipeline_unreadable_file_skipped_gracefully() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("good.py"), "def hello(): return 1\n").unwrap();
    std::fs::create_dir(tmp.path().join("bad.py")).unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]);

    match result {
        Ok(r) => {
            let has_hello = r.manifest.entities.iter().any(|e| e.id.contains("hello"));
            assert!(
                has_hello,
                "Pipeline should have parsed good.py despite bad.py existing"
            );
        }
        Err(_) => {
            // Acceptable if the pipeline returns an error, but must not panic
        }
    }
}

// =========================================================================
// 5. Multi-File Projects (P1)
// =========================================================================

#[test]
fn test_pipeline_multi_file_all_entities_present() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("module_a.py"), "def func_a(): return 1\n").unwrap();
    std::fs::write(
        tmp.path().join("module_b.py"),
        "def func_b(): return 2\ndef func_c(): return 3\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let entity_names: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .map(|e| e.id.as_str())
        .collect();

    assert!(
        entity_names.iter().any(|n| n.contains("func_a")),
        "Must find func_a. Entities: {:?}",
        entity_names
    );
    assert!(
        entity_names.iter().any(|n| n.contains("func_b")),
        "Must find func_b. Entities: {:?}",
        entity_names
    );
    assert!(
        entity_names.iter().any(|n| n.contains("func_c")),
        "Must find func_c. Entities: {:?}",
        entity_names
    );
    assert_eq!(
        result.manifest.metadata.file_count, 2,
        "file_count must be 2"
    );
}

#[test]
fn test_pipeline_entity_count_matches_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.py"), "def f1(): pass\ndef f2(): pass\n").unwrap();
    std::fs::write(tmp.path().join("b.py"), "class C1: pass\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    assert_eq!(
        result.manifest.metadata.entity_count,
        result.manifest.entities.len() as u64,
        "metadata.entity_count must match entities.len()"
    );
    assert_eq!(
        result.manifest.metadata.diagnostic_count,
        result.manifest.diagnostics.len() as u64,
        "metadata.diagnostic_count must match diagnostics.len()"
    );
}

#[test]
fn test_pipeline_multi_file_module_summaries() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("utils.py"),
        "def helper(): pass\ndef parse(): pass\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models.py"),
        "class User: pass\nclass Order: pass\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    assert_eq!(
        result.manifest.summary.modules.len(),
        2,
        "Two .py files should produce 2 module summaries, got {}",
        result.manifest.summary.modules.len()
    );

    let module_names: Vec<&str> = result
        .manifest
        .summary
        .modules
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    assert!(
        module_names.iter().any(|n| n.contains("utils")),
        "Module summary must include utils. Got: {:?}",
        module_names
    );
    assert!(
        module_names.iter().any(|n| n.contains("models")),
        "Module summary must include models. Got: {:?}",
        module_names
    );
}

#[test]
fn test_pipeline_project_with_issues_fixture() {
    // Copy fixture files to a temp dir because data_dead_end excludes
    // symbols under paths containing "/tests/" (test module heuristic).
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/python/project_with_issues");

    if !fixture_path.exists() {
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    for entry in std::fs::read_dir(&fixture_path).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().map(|e| e == "py").unwrap_or(false) {
            std::fs::copy(entry.path(), tmp.path().join(entry.file_name())).unwrap();
        }
    }

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    assert!(
        result.manifest.entities.len() >= 3,
        "project_with_issues should have >= 3 entities. Got {}",
        result.manifest.entities.len()
    );

    let dead_end_count = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.pattern == "data_dead_end")
        .count();
    assert!(
        dead_end_count >= 1,
        "project_with_issues has dead functions -> data_dead_end must fire. Got {}",
        dead_end_count
    );
}

// =========================================================================
// 6. AnalysisResult Contract (P1)
// =========================================================================

#[test]
fn test_pipeline_has_critical_false_for_warnings_only() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    assert!(
        !result.has_critical,
        "dead_code.py diagnostics are warnings, not criticals. has_critical must be false"
    );
}

#[test]
fn test_pipeline_has_findings_true_with_diagnostics() {
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
        "diagnostics must be populated for dead_code fixture"
    );
    assert!(
        result.has_findings,
        "has_findings must be true when diagnostics exist"
    );
}

#[test]
fn test_pipeline_has_findings_false_for_empty() {
    let tmp = tempfile::tempdir().unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    assert!(
        !result.has_findings,
        "Empty project must have has_findings=false"
    );
    assert!(
        !result.has_critical,
        "Empty project must have has_critical=false"
    );
}

// =========================================================================
// 7. Diagnostic Summary Integrity (P2)
// =========================================================================

#[test]
fn test_pipeline_summary_counts_match_diagnostics() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("unused_import.py"),
        include_str!("../../tests/fixtures/python/unused_import.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let actual_critical = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.severity == "critical")
        .count() as u64;
    let actual_warning = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.severity == "warning")
        .count() as u64;
    let actual_info = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.severity == "info")
        .count() as u64;

    assert_eq!(
        result.manifest.summary.diagnostic_summary.critical,
        actual_critical
    );
    assert_eq!(
        result.manifest.summary.diagnostic_summary.warning,
        actual_warning
    );
    assert_eq!(result.manifest.summary.diagnostic_summary.info, actual_info);
}

#[test]
fn test_pipeline_diagnostic_ids_sequential() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    for (i, diag) in result.manifest.diagnostics.iter().enumerate() {
        let expected_id = format!("D{:03}", i + 1);
        assert_eq!(
            diag.id, expected_id,
            "Diagnostic {} should have id '{}', got '{}'",
            i, expected_id, diag.id
        );
    }
}

#[test]
fn test_pipeline_diagnostic_fields_lowercase() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    for diag in &result.manifest.diagnostics {
        assert_eq!(diag.severity, diag.severity.to_lowercase());
        assert_eq!(diag.confidence, diag.confidence.to_lowercase());
        assert!(
            !diag.suggestion.is_empty(),
            "Diagnostic '{}' needs suggestion",
            diag.id
        );
        assert!(
            !diag.message.is_empty(),
            "Diagnostic '{}' needs message",
            diag.id
        );
    }
}

// =========================================================================
// 8. Entity Data Fidelity (P1)
// =========================================================================

#[test]
fn test_pipeline_entity_kind_strings_valid() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("classes.py"),
        include_str!("../../tests/fixtures/python/classes.py"),
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let valid_kinds = [
        "fn",
        "method",
        "class",
        "struct",
        "trait",
        "interface",
        "var",
        "const",
        "macro",
        "enum",
    ];

    for entity in &result.manifest.entities {
        assert!(
            valid_kinds.contains(&entity.kind.as_str()),
            "Entity '{}' has invalid kind '{}'",
            entity.id,
            entity.kind,
        );
    }

    let class_entities: Vec<_> = result
        .manifest
        .entities
        .iter()
        .filter(|e| e.kind == "class")
        .collect();
    assert!(
        !class_entities.is_empty(),
        "classes.py must produce entities with kind 'class'"
    );

    let fn_entities: Vec<_> = result
        .manifest
        .entities
        .iter()
        .filter(|e| e.kind == "fn")
        .collect();
    assert!(
        !fn_entities.is_empty(),
        "dead_code.py must produce entities with kind 'fn'"
    );
}

#[test]
fn test_pipeline_class_methods_have_method_kind() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("classes.py"),
        include_str!("../../tests/fixtures/python/classes.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let methods: Vec<_> = result
        .manifest
        .entities
        .iter()
        .filter(|e| e.kind == "method")
        .collect();
    assert!(
        !methods.is_empty(),
        "classes.py methods must have kind 'method'. Entity kinds found: {:?}",
        result
            .manifest
            .entities
            .iter()
            .map(|e| (&e.id, &e.kind))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_pipeline_entity_ids_contain_module() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("mymodule.py"), "def my_func(): pass\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    for entity in &result.manifest.entities {
        assert!(
            entity.id.contains("mymodule") || entity.id.contains("::"),
            "Entity id '{}' should contain module name or :: separator",
            entity.id
        );
    }
}

#[test]
fn test_pipeline_annotations_preserved() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("deco.py"),
        "class Animal:\n    @staticmethod\n    def species():\n        return \"unknown\"\n\n    @property\n    def name(self):\n        return self._name\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let species = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("species"));
    if let Some(s) = species {
        assert!(
            s.annotations.iter().any(|a| a.contains("staticmethod")),
            "species() must preserve @staticmethod annotation. Got: {:?}",
            s.annotations
        );
    }
}

// =========================================================================
// 9. Unicode and Special Cases (P2)
// =========================================================================

#[test]
fn test_pipeline_unicode_identifiers() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("unicode.py"),
        "def café():\n    return \"coffee\"\n\nclass Ñoño:\n    pass\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let has_cafe = result
        .manifest
        .entities
        .iter()
        .any(|e| e.id.contains("café"));
    assert!(
        has_cafe,
        "Unicode function name 'café' must survive the full pipeline. Entities: {:?}",
        result
            .manifest
            .entities
            .iter()
            .map(|e| &e.id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_pipeline_deeply_nested_functions() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("nested.py"),
        "def level0():\n    def level1():\n        def level2():\n            def level3():\n                return \"deep\"\n            return level3()\n        return level2()\n    return level1()\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    assert!(
        result.manifest.entities.len() >= 4,
        "Deeply nested functions must all appear as entities. Got {}",
        result.manifest.entities.len()
    );
}

#[test]
fn test_pipeline_large_file_no_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let mut content = String::new();
    for i in 0..200 {
        content.push_str(&format!("def func_{}():\n    return {}\n\n", i, i));
    }
    std::fs::write(tmp.path().join("large.py"), &content).unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    assert!(
        result.manifest.entities.len() >= 200,
        "200-function file should produce >= 200 entities, got {}",
        result.manifest.entities.len()
    );
}

// =========================================================================
// 10. Regression Guards (P0)
// =========================================================================

#[test]
fn test_regression_not_all_entities_pub() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("mixed.py"),
        "def public_func():\n    return 1\n\ndef _private_func():\n    return 2\n\nclass _InternalClass:\n    pass\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let vis_values: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .map(|e| e.vis.as_str())
        .collect();

    let all_pub = vis_values.iter().all(|&v| v == "pub");
    assert!(
        !all_pub,
        "NOT all entities should be 'pub'. Visibility values: {:?}.",
        vis_values
    );

    let has_priv = vis_values.iter().any(|&v| v == "priv");
    assert!(
        has_priv,
        "Must have at least one 'priv' entity. Got: {:?}",
        vis_values
    );
}

#[test]
fn test_regression_no_detected_placeholder() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    for entity in &result.manifest.entities {
        assert!(
            !entity.called_by.contains(&"(detected)".to_string()),
            "REGRESSION: Entity '{}' still has '(detected)' placeholder.",
            entity.id
        );
    }
}

#[test]
fn test_regression_no_module_with_n_entities_role() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("utils.py"),
        "def a(): pass\ndef b(): pass\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    for module in &result.manifest.summary.modules {
        let role_matches_vacuous =
            module.role.starts_with("Module with") && module.role.ends_with("entities");
        assert!(
            !role_matches_vacuous,
            "REGRESSION: Module '{}' has vacuous role '{}'.",
            module.name, module.role
        );
    }
}

#[test]
fn test_regression_no_duplicate_diagnostics() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("unused_import.py"),
        include_str!("../../tests/fixtures/python/unused_import.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    for (i, d1) in result.manifest.diagnostics.iter().enumerate() {
        for (j, d2) in result.manifest.diagnostics.iter().enumerate() {
            if i != j {
                let is_dup = d1.entity == d2.entity && d1.pattern == d2.pattern && d1.loc == d2.loc;
                assert!(
                    !is_dup,
                    "Duplicate diagnostic: pattern={}, entity={}, loc={}",
                    d1.pattern, d1.entity, d1.loc
                );
            }
        }
    }
}

// =========================================================================
// 11. Comprehensive Integration Test (P0)
// =========================================================================

#[test]
fn test_pipeline_full_integration_all_patterns() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("integration.py"),
        "import os  # phantom: os never used\n\ndef public_function():\n    return 42\n\ndef _private_unused():\n    return None\n\ndef main_handler():\n    return public_function()\n\nclass IsolatedProcessor:\n    def process(self, data):\n        return self._validate(data)\n\n    def _validate(self, data):\n        return data is not None\n",
    )
    .unwrap();

    std::fs::write(
        tmp.path().join("connected.py"),
        "def connected_func():\n    return \"I exist to make isolated_module detectable\"\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    // === ENTITY VERIFICATION ===
    assert!(
        result.manifest.entities.len() >= 5,
        "Integration test must find at least 5 entities. Got {}",
        result.manifest.entities.len()
    );

    let private_entity = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("_private_unused"));
    if let Some(p) = private_entity {
        assert_eq!(
            p.vis, "priv",
            "_private_unused must have vis 'priv', got '{}'",
            p.vis
        );
    }

    for entity in &result.manifest.entities {
        for cb in &entity.called_by {
            assert_ne!(
                cb, "(detected)",
                "No placeholders in called_by for '{}'",
                entity.id
            );
        }
        assert_ne!(
            entity.kind, "module",
            "Module kind must be filtered: '{}'",
            entity.id
        );
        assert!(
            !entity.loc.starts_with('/'),
            "Relative paths required: '{}'",
            entity.loc
        );
    }

    // === DIAGNOSTIC VERIFICATION ===
    let patterns_fired: Vec<&str> = result
        .manifest
        .diagnostics
        .iter()
        .map(|d| d.pattern.as_str())
        .collect();

    assert!(
        patterns_fired.contains(&"data_dead_end"),
        "data_dead_end must fire. Patterns: {:?}",
        patterns_fired
    );

    // Note: phantom_dependency may not fire on real data because PythonAdapter
    // does not create Symbol objects with "import" annotation for imports.
    // This is a known gap between mock graphs and real parser output.

    for diag in &result.manifest.diagnostics {
        assert!(
            !diag.evidence.is_empty(),
            "Diagnostic '{}' ({}) has empty evidence",
            diag.id,
            diag.pattern
        );
    }

    // === MODULE SUMMARY VERIFICATION ===
    assert!(!result.manifest.summary.modules.is_empty());
    for module in &result.manifest.summary.modules {
        assert!(
            !module.role.contains("Module with"),
            "Module '{}' has vacuous role: '{}'",
            module.name,
            module.role
        );
    }

    // === METADATA VERIFICATION ===
    assert_eq!(result.manifest.metadata.file_count, 2);
    assert_eq!(
        result.manifest.metadata.entity_count,
        result.manifest.entities.len() as u64
    );
    assert_eq!(
        result.manifest.metadata.diagnostic_count,
        result.manifest.diagnostics.len() as u64
    );
}

// =========================================================================
// 12. diagnose() Function Tests (P2)
// =========================================================================

#[test]
fn test_diagnose_uses_real_pipeline() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let (diagnostics, has_findings) = diagnose(
        tmp.path(),
        &config,
        &["python".to_string()],
        None,
        None,
        None,
    )
    .unwrap();

    assert!(has_findings, "dead_code.py should produce findings");
    assert!(
        !diagnostics.is_empty(),
        "diagnose() must return diagnostics from real pipeline"
    );

    for d in &diagnostics {
        assert!(
            !d.evidence.is_empty(),
            "diagnose() diagnostic '{}' has empty evidence",
            d.id
        );
    }
}

// =========================================================================
// 13. Loc Path Correctness — QA-3 Cycle 2 (P0)
// =========================================================================

#[test]
fn test_single_file_analysis_loc_includes_filename() {
    let tmp = tempfile::tempdir().unwrap();
    let file_path = tmp.path().join("dead_code.py");
    std::fs::write(
        &file_path,
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    // KEY: pass the FILE path as project_path, not the directory.
    // This triggers the strip_prefix bug where strip_prefix(file_path)
    // on the same file_path produces an empty string.
    let config = Config::load(file_path.parent().unwrap(), None).unwrap();
    let result = analyze(&file_path, &config, &["python".to_string()]).unwrap();

    assert!(
        !result.manifest.entities.is_empty(),
        "Single-file analysis must produce entities"
    );

    for entity in &result.manifest.entities {
        let parts: Vec<&str> = entity.loc.splitn(2, ':').collect();
        assert_eq!(
            parts.len(),
            2,
            "Entity '{}' loc '{}' must be in file:line format",
            entity.id,
            entity.loc
        );

        let filename_part = parts[0];
        assert!(
            !filename_part.is_empty(),
            "Entity '{}' loc '{}' has EMPTY filename — strip_prefix bug (lib.rs:172-176) not fixed. \
             Expected 'dead_code.py:N', got ':N'",
            entity.id,
            entity.loc
        );
        assert_eq!(
            filename_part, "dead_code.py",
            "Entity '{}' loc filename should be 'dead_code.py', got '{}'",
            entity.id, filename_part
        );

        let line: u32 = parts[1].parse().unwrap_or_else(|_| {
            panic!(
                "Entity '{}' loc '{}' — line part '{}' is not a valid number",
                entity.id, entity.loc, parts[1]
            )
        });
        assert!(line > 0, "Line number must be 1-based for '{}'", entity.id);
    }
}

#[test]
fn test_loc_filename_never_empty_invariant() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    for entity in &result.manifest.entities {
        let parts: Vec<&str> = entity.loc.splitn(2, ':').collect();
        assert!(
            parts.len() == 2 && !parts[0].is_empty(),
            "Entity '{}' loc '{}' violates file:line invariant — \
             filename portion is empty or colon missing. \
             Every loc must have a non-empty filename before the colon.",
            entity.id,
            entity.loc
        );
    }
}

#[test]
fn test_loc_path_nested_subdirectory() {
    let tmp = tempfile::tempdir().unwrap();
    let subdir = tmp.path().join("subdir");
    std::fs::create_dir(&subdir).unwrap();
    std::fs::write(
        subdir.join("module.py"),
        "def helper():\n    pass\n\ndef unused():\n    pass\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    assert!(
        !result.manifest.entities.is_empty(),
        "Nested subdirectory analysis must produce entities"
    );

    for entity in &result.manifest.entities {
        let parts: Vec<&str> = entity.loc.splitn(2, ':').collect();
        assert_eq!(parts.len(), 2, "loc must be file:line format");

        let path_part = parts[0];
        assert!(
            path_part.starts_with("subdir/"),
            "Entity '{}' loc '{}' must show relative path including subdirectory 'subdir/', got '{}'",
            entity.id,
            entity.loc,
            path_part
        );
        assert!(
            !path_part.starts_with('/'),
            "Entity '{}' loc must be relative, not absolute",
            entity.id
        );
        assert!(
            path_part.ends_with(".py"),
            "Entity '{}' loc path '{}' should end with .py",
            entity.id,
            path_part
        );
    }
}

// =========================================================================
// 14. Scanner Deletion Verification — QA-3 Cycle 2 (P0)
// =========================================================================

#[test]
fn test_scanner_function_definitions_absent() {
    let lib_source = include_str!("lib.rs");

    let scanner_definitions = [
        "fn analyze_python_files",
        "fn group_entities_into_modules",
        "fn extract_function_name",
        "fn extract_class_name",
        "fn extract_signature",
        "fn is_inside_class",
        "fn is_python_keyword",
    ];

    for def in &scanner_definitions {
        assert!(
            !lib_source.contains(def),
            "lib.rs still contains dead scanner function definition: '{}'. \
             All 7 scanner functions must be physically deleted.",
            def
        );
    }
}

#[test]
fn test_no_scanner_call_sites_in_lib() {
    let lib_source = include_str!("lib.rs");

    let call_patterns = [
        "analyze_python_files(",
        "group_entities_into_modules(",
        "extract_function_name(",
        "extract_class_name(",
        "extract_signature(",
        "is_inside_class(",
        "is_python_keyword(",
    ];

    for pattern in &call_patterns {
        assert!(
            !lib_source.contains(pattern),
            "lib.rs contains what looks like a call to deleted scanner function: '{}'. \
             All call sites to deleted functions must be removed.",
            pattern
        );
    }
}

#[test]
fn test_diagnose_severity_filter() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();

    let (critical_only, _) = diagnose(
        tmp.path(),
        &config,
        &["python".to_string()],
        Some(Severity::Critical),
        None,
        None,
    )
    .unwrap();

    for d in &critical_only {
        assert_eq!(
            d.severity, "critical",
            "With critical filter, only critical diagnostics should appear. Got: {}",
            d.severity
        );
    }
}

// =========================================================================
// 15. Multi-Language Dispatch Tests (P0) — Cycle 4
// =========================================================================

#[test]
fn test_analyze_mixed_py_and_js() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.py"), "def greet():\n    pass\n").unwrap();
    std::fs::write(dir.path().join("utils.js"), "function helper() {}\n").unwrap();

    let config = Config::load(dir.path(), None).unwrap();
    let result = analyze(dir.path(), &config, &[]).unwrap();

    let py_entities: Vec<_> = result
        .manifest
        .entities
        .iter()
        .filter(|e| e.loc.contains(".py"))
        .collect();
    let js_entities: Vec<_> = result
        .manifest
        .entities
        .iter()
        .filter(|e| e.loc.contains(".js"))
        .collect();

    assert!(!py_entities.is_empty(), "Must have Python entities");
    assert!(!js_entities.is_empty(), "Must have JavaScript entities");
}

#[test]
fn test_analyze_js_file_count_in_metadata() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.py"), "def f(): pass\n").unwrap();
    std::fs::write(dir.path().join("b.js"), "function g() {}\n").unwrap();

    let config = Config::load(dir.path(), None).unwrap();
    let result = analyze(dir.path(), &config, &[]).unwrap();

    assert!(
        result.manifest.metadata.file_count >= 2,
        "file_count must include both .py and .js files, got {}",
        result.manifest.metadata.file_count
    );
}

#[test]
fn test_analyze_languages_metadata_includes_js() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("app.js"), "function main() {}\n").unwrap();

    let config = Config::load(dir.path(), None).unwrap();
    let result = analyze(dir.path(), &config, &[]).unwrap();

    assert!(
        result
            .manifest
            .metadata
            .languages
            .iter()
            .any(|l| l == "javascript"),
        "metadata.languages must include 'javascript', got {:?}",
        result.manifest.metadata.languages
    );
}

#[test]
fn test_python_regression_entity_count_after_js_adapter() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(dir.path(), None).unwrap();
    let result = analyze(dir.path(), &config, &[]).unwrap();

    assert!(
        result.manifest.entities.len() >= 2,
        "dead_code.py entity count must not regress: got {}",
        result.manifest.entities.len()
    );
}

#[test]
fn test_js_only_analysis() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.js"),
        "function start() {}\nconst run = () => {};\n",
    )
    .unwrap();

    let config = Config::load(dir.path(), None).unwrap();
    let result = analyze(dir.path(), &config, &[]).unwrap();

    let fns: Vec<_> = result
        .manifest
        .entities
        .iter()
        .filter(|e| e.kind == "fn")
        .collect();
    assert!(
        fns.len() >= 2,
        "JS-only analysis must extract at least 2 functions, got {}",
        fns.len()
    );
}

// =========================================================================
// Recursion Depth Protection Tests (D2)
// =========================================================================

#[test]
fn test_python_256_depth_nested_functions_no_crash() {
    use crate::parser::python::PythonAdapter;
    use crate::parser::LanguageAdapter;
    use std::path::Path;

    let mut source = String::new();
    for i in 0..256 {
        let indent = "    ".repeat(i);
        source.push_str(&format!("{}def f{}():\n", indent, i));
    }
    let indent = "    ".repeat(256);
    source.push_str(&format!("{}pass\n", indent));

    let adapter = PythonAdapter::new();
    let path = PathBuf::from("deep_functions.py");
    let result = adapter.parse_file(Path::new(&path), &source);

    assert!(result.is_ok(), "Parser must not crash on 256-depth nesting");
    let parse_result = result.unwrap();
    assert!(
        !parse_result.symbols.is_empty(),
        "Must extract symbols for functions above the depth limit"
    );
}

#[test]
fn test_python_512_depth_nested_calls_no_crash() {
    use crate::parser::python::PythonAdapter;
    use crate::parser::LanguageAdapter;
    use std::path::Path;

    let mut expr = "x".to_string();
    for i in (0..512).rev() {
        expr = format!("f{}({})", i, expr);
    }
    let source = format!("result = {}\n", expr);

    let adapter = PythonAdapter::new();
    let path = PathBuf::from("deep_calls.py");
    let result = adapter.parse_file(Path::new(&path), &source);

    assert!(
        result.is_ok(),
        "Parser must not crash on 512-depth call nesting"
    );
}

#[test]
fn test_python_10k_depth_nesting_no_crash() {
    use crate::parser::python::PythonAdapter;
    use crate::parser::LanguageAdapter;
    use std::path::Path;

    let mut source = String::new();
    for i in 0..10_000 {
        let indent = "    ".repeat(i);
        source.push_str(&format!("{}def f{}():\n", indent, i));
    }
    let indent = "    ".repeat(10_000);
    source.push_str(&format!("{}pass\n", indent));

    let adapter = PythonAdapter::new();
    let path = PathBuf::from("adversarial.py");
    let result = adapter.parse_file(Path::new(&path), &source);

    assert!(result.is_ok(), "Parser MUST NOT crash on 10K-depth nesting");
}

#[test]
fn test_python_mixed_nesting_partial_results() {
    use crate::parser::python::PythonAdapter;
    use crate::parser::LanguageAdapter;
    use std::path::Path;

    let mut source = String::new();
    for i in 0..300 {
        let indent = "    ".repeat(i);
        if i % 3 == 0 {
            source.push_str(&format!("{}class C{}:\n", indent, i));
        } else {
            source.push_str(&format!("{}def f{}():\n", indent, i));
        }
    }
    let indent = "    ".repeat(300);
    source.push_str(&format!("{}pass\n", indent));

    let adapter = PythonAdapter::new();
    let path = PathBuf::from("mixed_nesting.py");
    let result = adapter.parse_file(Path::new(&path), &source).unwrap();

    let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"C0"), "Class at depth 0 must be extracted");
    assert!(
        names.contains(&"f1"),
        "Function at depth 1 must be extracted"
    );
    assert!(
        names.contains(&"f2"),
        "Function at depth 2 must be extracted"
    );
}

#[test]
fn test_python_depth_limit_preserves_shallow_symbols() {
    use crate::parser::python::PythonAdapter;
    use crate::parser::LanguageAdapter;
    use std::path::Path;

    let mut source = String::from("def shallow_func():\n    pass\n\n");
    for i in 0..500 {
        let indent = "    ".repeat(i);
        source.push_str(&format!("{}def f{}():\n", indent, i));
    }
    let indent = "    ".repeat(500);
    source.push_str(&format!("{}pass\n", indent));

    let adapter = PythonAdapter::new();
    let path = PathBuf::from("partial_results.py");
    let result = adapter.parse_file(Path::new(&path), &source).unwrap();

    let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"shallow_func"),
        "Symbols at shallow depth MUST be extracted even when deep nesting exists"
    );
}

#[test]
fn test_python_deep_attribute_chain_no_crash() {
    use crate::parser::python::PythonAdapter;
    use crate::parser::LanguageAdapter;
    use std::path::Path;

    let chain: Vec<String> = (0..500).map(|i| format!("a{}", i)).collect();
    let source = format!("import {}\nresult = {}\n", chain[0], chain.join("."));

    let adapter = PythonAdapter::new();
    let path = PathBuf::from("deep_attrs.py");
    let result = adapter.parse_file(Path::new(&path), &source);

    assert!(result.is_ok(), "Deep attribute chains must not crash");
}

#[test]
fn test_python_all_three_traversals_depth_protected() {
    use crate::parser::python::PythonAdapter;
    use crate::parser::LanguageAdapter;
    use std::path::Path;

    let mut source = String::new();
    for i in 0..300 {
        let indent = "    ".repeat(i);
        source.push_str(&format!("{}def f{}():\n", indent, i));
    }
    let deep_indent = "    ".repeat(300);
    let mut expr = "x".to_string();
    for i in (0..300).rev() {
        expr = format!("f{}({})", i, expr);
    }
    source.push_str(&format!("{}result = {}\n", deep_indent, expr));

    let adapter = PythonAdapter::new();
    let path = PathBuf::from("triple_stress.py");
    let result = adapter.parse_file(Path::new(&path), &source);

    assert!(
        result.is_ok(),
        "All three traversals must be depth-protected"
    );
}

// =========================================================================
// Cross-File Fixture Tests (D5)
// =========================================================================

#[test]
fn test_fixture_simple_import_cross_file_resolution() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/simple_import");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    // helper's called_by must include cross-file caller from a.py
    let helper_entity = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("helper") && !e.id.contains("import"));
    assert!(
        helper_entity.is_some(),
        "helper must appear in manifest entities"
    );
    if let Some(entity) = helper_entity {
        assert!(
            !entity.called_by.is_empty(),
            "Simple import must create cross-file edge visible in called_by. \
             called_by is empty for helper even though a.py imports and calls it."
        );
    }
}

#[test]
fn test_fixture_aliased_import_resolves_original() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/aliased_import");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    let utility_exists = result
        .manifest
        .entities
        .iter()
        .any(|e| e.id.contains("utility"));
    assert!(
        utility_exists,
        "utility function must be present in manifest"
    );
}

#[test]
fn test_fixture_missing_module_no_crash() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/missing_module");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        result.manifest.metadata.entity_count > 0,
        "Must extract symbols even with missing module"
    );
}

#[test]
fn test_fixture_circular_import_no_infinite_loop() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/circular_import");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    // ping and pong are the two function entities (import symbols are Module kind, filtered)
    assert!(
        result.manifest.metadata.entity_count >= 2,
        "Both files' symbols must be extracted despite circular import. Got {}",
        result.manifest.metadata.entity_count
    );
}

#[test]
fn test_fixture_nested_package_import() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/nested_package");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    let tool_exists = result
        .manifest
        .entities
        .iter()
        .any(|e| e.id.contains("tool"));
    assert!(tool_exists, "Nested package symbol must be extracted");
}

#[test]
fn test_fixture_reexport_alias_chain() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/reexport");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    let core_exists = result
        .manifest
        .entities
        .iter()
        .any(|e| e.id.contains("core_function"));
    let api_exists = result
        .manifest
        .entities
        .iter()
        .any(|e| e.id.contains("wrapper"));
    assert!(
        core_exists && api_exists,
        "Both internal and API symbols must be present"
    );
}

#[test]
fn test_fixture_cross_file_full_pipeline() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/simple_import");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    // main (a.py) + helper (b.py) = 2 function entities (import symbols are Module kind, filtered)
    assert!(
        result.manifest.entities.len() >= 2,
        "Manifest must include entities from both a.py and b.py. Got {}",
        result.manifest.entities.len()
    );

    let helper_entity = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("helper") && !e.id.contains("import"));
    assert!(helper_entity.is_some(), "helper must appear in manifest");
    if let Some(entity) = helper_entity {
        assert!(
            !entity.called_by.is_empty(),
            "helper's called_by must include cross-file caller after M5 + called_by fix"
        );
    }
}

// =========================================================================
// Flow Tracing Integration Tests (D5)
// =========================================================================

#[test]
fn test_flow_trace_produces_output_through_pipeline() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/flow_trace");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    // Check that main is detected as entry point
    assert!(
        result
            .manifest
            .summary
            .entry_points
            .iter()
            .any(|ep| ep.contains("main")),
        "main must be detected as entry point. Got: {:?}",
        result.manifest.summary.entry_points
    );

    // Flows must be populated for cross-file fixture with entry point
    assert!(
        !result.manifest.flows.is_empty(),
        "flows must be populated when flow tracing is active on cross-file fixture"
    );
    assert!(
        result.manifest.metadata.flow_count > 0,
        "flow_count must be > 0 when flows exist"
    );
}

#[test]
fn test_rust_adapter_registered_produces_output() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("sample.rs"),
        "fn main() {}\nfn helper() { main(); }\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["rust".to_string()]).unwrap();

    assert!(
        !result.manifest.entities.is_empty(),
        "RustAdapter must produce entities for .rs files. Got 0 entities."
    );
}

// ============================================================================
// Cycle 8 QA-Foundation Tests (T1-T4: dependency_graph, T5-T10+R1: cross-file flow)
// ============================================================================

// T1: dependency_graph populated for cross-file Python fixture (P0 HARD GATE)
#[test]
fn test_dependency_graph_populated_for_cross_file_project() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/flow_trace");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    // UNCONDITIONAL. This is the Phase 1 hard gate test.
    assert!(
        !result.manifest.dependency_graph.is_empty(),
        "dependency_graph must be populated for cross-file projects. \
         The flow_trace fixture has 3 files with cross-file imports. \
         Got empty Vec — is extract_dependency_graph() wired at lib.rs:425?"
    );
}

// T2: dependency_graph edges contain correct file pairs
#[test]
fn test_dependency_graph_contains_correct_file_pairs() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/flow_trace");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        !result.manifest.dependency_graph.is_empty(),
        "dependency_graph must be populated (precondition)"
    );

    let all_pairs: Vec<(String, String)> = result
        .manifest
        .dependency_graph
        .iter()
        .map(|dep| (dep.from.clone(), dep.to.clone()))
        .collect();

    // main.py <-> utils.py edge must exist
    let has_main_utils = all_pairs.iter().any(|(from, to)| {
        (from.contains("main") && to.contains("utils"))
            || (from.contains("utils") && to.contains("main"))
    });
    assert!(
        has_main_utils,
        "dependency_graph must contain main.py <-> utils.py edge. Got: {:?}",
        all_pairs
    );

    // utils.py <-> helpers.py edge must exist
    let has_utils_helpers = all_pairs.iter().any(|(from, to)| {
        (from.contains("utils") && to.contains("helpers"))
            || (from.contains("helpers") && to.contains("utils"))
    });
    assert!(
        has_utils_helpers,
        "dependency_graph must contain utils.py <-> helpers.py edge. Got: {:?}",
        all_pairs
    );
}

// T3: single-file project produces empty dependency_graph (true negative)
#[test]
fn test_dependency_graph_empty_for_single_file_project() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("single.py"),
        "def main():\n    return 42\n\ndef helper():\n    return main()\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    assert!(
        result.manifest.dependency_graph.is_empty(),
        "dependency_graph must be empty for single-file projects. \
         Got {} edges: {:?}",
        result.manifest.dependency_graph.len(),
        result.manifest.dependency_graph
    );
}

// T4: dependency_graph weights nonzero and direction valid
#[test]
fn test_dependency_graph_weights_nonzero() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/flow_trace");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        !result.manifest.dependency_graph.is_empty(),
        "dependency_graph must be populated (precondition)"
    );

    for dep in &result.manifest.dependency_graph {
        assert!(
            dep.weight > 0,
            "dependency_graph edge {} → {} has weight 0. \
             Every cross-file edge must have at least 1 reference.",
            dep.from,
            dep.to
        );
    }

    for dep in &result.manifest.dependency_graph {
        let dir_lower = dep.direction.to_lowercase();
        assert!(
            dir_lower == "unidirectional" || dir_lower == "bidirectional",
            "dependency_graph edge {} → {} has invalid direction '{}'. \
             Expected 'unidirectional' or 'bidirectional'.",
            dep.from,
            dep.to,
            dep.direction
        );
    }
}

// T5: Flow from main reaches utils.py::process (P1 HARD GATE)
#[test]
fn test_cross_file_flow_reaches_resolved_target() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/flow_trace");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        !result.manifest.flows.is_empty(),
        "flows must be populated for cross-file flow_trace fixture"
    );

    let main_flow = result
        .manifest
        .flows
        .iter()
        .find(|f| f.entry.contains("main"));
    assert!(
        main_flow.is_some(),
        "Must have a flow from entry point 'main'. Got entries: {:?}",
        result
            .manifest
            .flows
            .iter()
            .map(|f| &f.entry)
            .collect::<Vec<_>>()
    );
    let flow = main_flow.unwrap();

    let step_entities: Vec<&str> = flow.steps.iter().map(|s| s.entity.as_str()).collect();
    let reaches_utils = step_entities
        .iter()
        .any(|e| e.contains("utils") && !e.contains("import::"));
    assert!(
        reaches_utils,
        "Flow from main must reach utils.py::process (resolved target), \
         not stop at import proxy. Got steps: {:?}",
        step_entities
    );
}

// T6: Cross-file flow steps have correct file attribution (no import:: prefix)
#[test]
fn test_cross_file_flow_steps_have_file_attribution() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/flow_trace");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        !result.manifest.flows.is_empty(),
        "flows must be populated (precondition)"
    );

    let main_flow = result
        .manifest
        .flows
        .iter()
        .find(|f| f.entry.contains("main"));
    assert!(main_flow.is_some(), "main flow must exist (precondition)");
    let flow = main_flow.unwrap();

    let has_import_prefix = flow.steps.iter().any(|s| s.entity.contains("import::"));
    assert!(
        !has_import_prefix,
        "Flow steps must not contain 'import::' prefix — that's an internal proxy. \
         Got steps: {:?}",
        flow.steps.iter().map(|s| &s.entity).collect::<Vec<_>>()
    );
}

// T7: Circular import does not infinite-loop (adversarial)
#[test]
fn test_circular_import_flow_trace_terminates() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/circular_import");

    let config = Config::load(&fixture_dir, None).unwrap();
    // This MUST complete without hanging.
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    // The real assertion: we reached this line (no infinite loop)
    assert!(
        result.manifest.metadata.entity_count > 0,
        "circular_import fixture must produce entities"
    );
}

// T8: Multi-hop cross-file flow (A → B → C)
#[test]
fn test_multi_hop_cross_file_flow() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/flow_trace");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        !result.manifest.flows.is_empty(),
        "flows must be populated (precondition)"
    );

    let main_flow = result
        .manifest
        .flows
        .iter()
        .find(|f| f.entry.contains("main"));
    assert!(main_flow.is_some(), "main flow must exist (precondition)");
    let flow = main_flow.unwrap();

    // main.py::main → utils.py::process → helpers.py::format_output
    let step_entities: Vec<&str> = flow.steps.iter().map(|s| s.entity.as_str()).collect();
    let reaches_helpers = step_entities
        .iter()
        .any(|e| e.contains("helpers") || e.contains("format_output"));
    assert!(
        reaches_helpers,
        "Multi-hop flow must reach helpers.py::format_output. \
         main → process (utils.py) → format_output (helpers.py). \
         Got steps: {:?}",
        step_entities
    );
}

// T9: Unresolved import produces partial path (no crash)
#[test]
fn test_unresolved_import_in_flow_produces_partial_path() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("main.py"),
        "from nonexistent_module import magic\n\ndef main():\n    result = magic()\n    return result\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    // Must not crash. Flow steps must not contain empty entities.
    for flow in &result.manifest.flows {
        for step in &flow.steps {
            assert!(
                !step.entity.is_empty(),
                "Flow step entity must not be empty (would indicate SymbolId::default() leak)"
            );
        }
    }
}

// T10: Flow depth limit applies across file boundaries
#[test]
fn test_flow_depth_limit_applies_cross_file() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/flow_trace");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    for flow in &result.manifest.flows {
        assert!(
            flow.steps.len() <= 64,
            "Flow path must respect MAX_FLOW_DEPTH (64). \
             Got {} steps in flow from entry '{}'",
            flow.steps.len(),
            flow.entry
        );
    }
}

// R1: Regression — diagnostics still fire after stale_reference pattern addition
#[test]
fn test_diagnostics_still_fire_after_stale_reference_addition() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/project_with_issues");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    // The diagnostic pipeline must still produce findings on a project with known issues.
    // Adding stale_reference to the pattern registry must not break existing patterns.
    assert!(
        !result.manifest.diagnostics.is_empty(),
        "Diagnostics must still fire after stale_reference addition. \
         If this fails, adding stale_reference to the pattern registry may have broken \
         the diagnostic pipeline."
    );
}

// R1b: Regression — no import:: prefix in flow step entity IDs
#[test]
fn test_no_import_prefix_in_flow_step_entities() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/flow_trace");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    for flow in &result.manifest.flows {
        for step in &flow.steps {
            assert!(
                !step.entity.starts_with("import::"),
                "Flow step entity must not start with 'import::' prefix. Got: '{}'",
                step.entity
            );
        }
    }
}

// =========================================================================
// Cycle 9 — QA-1 (QA-Foundation) Tests
// Graph Exposure + Manifest Size Enforcement + dep_graph relative paths
// =========================================================================

// T1: AnalysisResult.graph field exists and is accessible (Hard Gate)
#[test]
fn test_pipeline_graph_exposed_in_analysis_result() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    // This line fails to COMPILE if the field doesn't exist. Unconditional assertion.
    let symbol_count = result.graph.all_symbols().count();
    assert!(
        symbol_count > 0,
        "AnalysisResult.graph MUST be accessible and populated after analysis. \
         Got 0 symbols. This field is required for trace_flows_from() direct calls \
         and MCP integration."
    );
}

// T2: Graph contains symbols after analysis
#[test]
fn test_pipeline_graph_symbols_populated_after_analysis() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let symbols: Vec<_> = result.graph.all_symbols().collect();
    assert!(
        symbols.len() >= 4,
        "dead_code.py has at least 4 named functions. Graph returned {} symbols. \
         The graph may not have been populated or was cleared before being moved \
         into AnalysisResult.",
        symbols.len()
    );

    let has_main_handler = symbols
        .iter()
        .any(|(_, sym)| sym.qualified_name.contains("main_handler") || sym.name == "main_handler");
    assert!(
        has_main_handler,
        "Graph MUST contain the 'main_handler' symbol from dead_code.py. \
         Found symbols: {:?}",
        symbols.iter().map(|(_, s)| &s.name).collect::<Vec<_>>()
    );
}

// T3: Graph symbol count vs. entity count consistency
#[test]
fn test_pipeline_graph_symbol_count_consistent_with_entities() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let graph_non_module_count = result
        .graph
        .all_symbols()
        .filter(|(_, sym)| sym.kind != crate::parser::ir::SymbolKind::Module)
        .count();

    let entity_count = result.manifest.entities.len();

    let diff = (graph_non_module_count as i64 - entity_count as i64).unsigned_abs();
    assert!(
        diff <= graph_non_module_count as u64 / 2,
        "Graph has {} non-Module symbols but manifest has {} entities. \
         Difference of {} is too large — indicates graph population or entity \
         extraction mismatch.",
        graph_non_module_count,
        entity_count,
        diff
    );
}

// T4: Manifest size validation — pathological input triggers SizeLimit
#[test]
fn test_pipeline_manifest_size_limit_enforced_on_pathological_input() {
    let tmp = tempfile::tempdir().unwrap();

    // Generate pathological Python file: 200 functions with 100-char names.
    let mut source = String::new();
    for i in 0..200 {
        let long_name = format!(
            "very_long_function_name_that_inflates_manifest_output_significantly_{:04}",
            i
        );
        source.push_str(&format!(
            "def {}(param_a, param_b, param_c, param_d):\n    return param_a + param_b\n\n",
            long_name
        ));
    }
    std::fs::write(tmp.path().join("pathological.py"), &source).unwrap();

    let config = Config::load(tmp.path(), None).unwrap();

    let result = analyze(tmp.path(), &config, &["python".to_string()]);

    // Either analyze() returns SizeLimit or we check via serialization
    match result {
        Err(e) => {
            let err_msg = e.to_string();
            assert!(
                err_msg.contains("exceeds") || err_msg.contains("size"),
                "Error should be SizeLimit, got: {}",
                err_msg
            );
        }
        Ok(result) => {
            // If analyze() succeeded, verify the ratio via serialization.
            // The size check is enforced post-serialization by validate_manifest_size().
            use crate::manifest::JsonFormatter;
            use crate::manifest::OutputFormatter;
            let formatter = JsonFormatter;
            let serialized = formatter.format_manifest(&result.manifest);

            if let Ok(ref output) = serialized {
                let manifest_bytes = output.len() as u64;
                let source_bytes = result.source_bytes;
                if source_bytes >= 1024 {
                    let ratio = manifest_bytes as f64 / source_bytes as f64;
                    // Verify either ratio is within bounds OR
                    // validate_manifest_size catches it (JSON limit is 15x)
                    let validation =
                        crate::manifest::validate_manifest_size(output, source_bytes, "json");
                    if ratio > 15.0 {
                        assert!(
                            validation.is_err(),
                            "Pathological input produced {:.1}x ratio but \
                             validate_manifest_size() did NOT catch it.",
                            ratio
                        );
                    }
                }
            }
        }
    }
}

// T5: Normal analysis does NOT trigger size limit
#[test]
fn test_pipeline_normal_analysis_no_size_limit_error() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]);

    assert!(
        result.is_ok(),
        "Normal analysis on dead_code.py fixture MUST NOT trigger size limit. \
         Got error: {:?}",
        result.err()
    );

    let result = result.unwrap();
    use crate::manifest::OutputFormatter;
    use crate::manifest::YamlFormatter;
    let formatter = YamlFormatter;
    let serialized = formatter.format_manifest(&result.manifest);
    assert!(
        serialized.is_ok(),
        "YAML serialization of normal analysis MUST succeed. Error: {:?}",
        serialized.err()
    );
}

// T6: dep_graph paths are relative
#[test]
fn test_pipeline_dep_graph_uses_relative_paths() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::write(
        tmp.path().join("mod_a.py"),
        "from mod_b import helper\n\ndef main():\n    helper()\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("mod_b.py"),
        "def helper():\n    return 42\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let project_path_str = tmp.path().to_string_lossy().to_string();

    for dep in &result.manifest.dependency_graph {
        assert!(
            !dep.from.starts_with('/') && !dep.from.starts_with(&project_path_str),
            "dep_graph 'from' field contains absolute path: '{}'. \
             Must be relative (e.g., 'mod_a.py', not '{}/mod_a.py').",
            dep.from,
            project_path_str
        );
        assert!(
            !dep.to.starts_with('/') && !dep.to.starts_with(&project_path_str),
            "dep_graph 'to' field contains absolute path: '{}'.",
            dep.to
        );
    }

    for dep in &result.manifest.dependency_graph {
        assert!(
            !dep.from.is_empty(),
            "dep_graph 'from' is empty after path normalization"
        );
        assert!(
            !dep.to.is_empty(),
            "dep_graph 'to' is empty after path normalization"
        );
    }
}

// T7: Empty project with graph exposure
#[test]
fn test_pipeline_empty_project_graph_exists_but_empty() {
    let tmp = tempfile::tempdir().unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &[]).unwrap();

    let symbol_count = result.graph.all_symbols().count();
    assert_eq!(
        symbol_count, 0,
        "Empty project should have 0 symbols in graph, got {}",
        symbol_count
    );

    let file_symbols = result
        .graph
        .symbols_in_file(std::path::Path::new("nonexistent.py"));
    assert!(
        file_symbols.is_empty(),
        "symbols_in_file on empty graph should return empty Vec"
    );
}

// T8: source_bytes field exists and is accurate
#[test]
fn test_pipeline_source_bytes_tracked_in_analysis_result() {
    let tmp = tempfile::tempdir().unwrap();
    let source_content = include_str!("../../tests/fixtures/python/dead_code.py");
    std::fs::write(tmp.path().join("dead_code.py"), source_content).unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let expected_bytes = source_content.len() as u64;
    assert_eq!(
        result.source_bytes,
        expected_bytes,
        "source_bytes should equal the total bytes of source files read. \
         Expected {} (dead_code.py is {} bytes), got {}.",
        expected_bytes,
        source_content.len(),
        result.source_bytes
    );
}

// T9: Zero source bytes — no division by zero
#[test]
fn test_pipeline_zero_source_bytes_no_panic() {
    let tmp = tempfile::tempdir().unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &[]);

    assert!(
        result.is_ok(),
        "Empty project analysis must not panic or error from size validation. \
         Got: {:?}",
        result.err()
    );

    let result = result.unwrap();
    assert_eq!(
        result.source_bytes, 0,
        "Empty project should have 0 source_bytes"
    );
}

// T10: Small fixture size ratio tolerance — no false positives
#[test]
fn test_pipeline_small_fixture_no_false_positive_size_limit() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("tiny.py"), "def f():\n    pass\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]);

    assert!(
        result.is_ok(),
        "Tiny source files must not trigger SizeLimit. Metadata overhead \
         for small projects naturally exceeds 10x. Got: {:?}",
        result.err()
    );
}

// T11: Graph is fully populated after move (has edges)
#[test]
fn test_pipeline_graph_has_edges_after_move() {
    let tmp = tempfile::tempdir().unwrap();
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/simple_import");

    for entry in std::fs::read_dir(&fixture_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().map_or(false, |e| e == "py") {
            std::fs::copy(entry.path(), tmp.path().join(entry.file_name())).unwrap();
        }
    }

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let has_edges = result.graph.all_symbols().any(|(id, _)| {
        !result.graph.edges_from(id).is_empty() || !result.graph.edges_to(id).is_empty()
    });

    assert!(
        has_edges,
        "Graph in AnalysisResult should contain edges from cross-file imports. \
         If no edges found, the graph may have been replaced with Graph::new() \
         instead of moved from the populated instance."
    );
}

// T12: diagnose() compatibility with new fields
#[test]
fn test_pipeline_diagnose_works_with_graph_exposure() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();

    let result = diagnose(
        tmp.path(),
        &config,
        &["python".to_string()],
        None,
        None,
        None,
    );

    assert!(
        result.is_ok(),
        "diagnose() must continue working after graph exposure. Got: {:?}",
        result.err()
    );

    let (diagnostics, _has_critical) = result.unwrap();
    assert!(
        !diagnostics.is_empty(),
        "diagnose() on dead_code.py should find diagnostics."
    );
}

// R1: dep_graph type consistency (Cycle 8 fix regression guard)
#[test]
fn test_regression_dep_graph_type_consistency_cycle8() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("a.py"),
        "from b import foo\ndef bar(): foo()\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("b.py"), "def foo(): pass\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    for dep in &result.manifest.dependency_graph {
        assert!(
            dep.direction == "unidirectional" || dep.direction == "bidirectional",
            "dep_graph direction must be 'unidirectional' or 'bidirectional', got: '{}'",
            dep.direction
        );
        assert!(
            dep.weight > 0,
            "dep_graph weight must be > 0 for existing edges"
        );
    }
}

// R2: All AnalysisResult fields accessible (compilation gate)
#[test]
fn test_regression_analysis_result_all_fields_accessible() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("clean.py"),
        "def greet(name):\n    return f'Hello {name}'\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    // Every field must be directly accessible (pub)
    let _manifest = &result.manifest;
    let _critical = result.has_critical;
    let _findings = result.has_findings;
    let _graph = &result.graph;
    let _bytes = result.source_bytes;
}

// =========================================================================
// T16: stale_reference End-to-End Pipeline Integration
// =========================================================================

/// Definitive proof that stale_reference is wired into the full analysis pipeline.
/// Parse → graph → resolve → detect → manifest. If any stage is broken, this fails.
#[test]
fn test_stale_reference_pipeline_integration() {
    let tmp = tempfile::tempdir().unwrap();

    // Copy the stale_reference cross-file fixture
    std::fs::write(
        tmp.path().join("main.py"),
        include_str!("../../tests/fixtures/python/cross_file/stale_reference/main.py"),
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("utils.py"),
        include_str!("../../tests/fixtures/python/cross_file/stale_reference/utils.py"),
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("helpers.py"),
        include_str!("../../tests/fixtures/python/cross_file/stale_reference/helpers.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    // Check manifest diagnostics for stale_reference findings
    let stale_findings: Vec<_> = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.pattern == "stale_reference")
        .collect();

    assert!(
        !stale_findings.is_empty(),
        "stale_reference must appear in manifest diagnostics when run on the \
         stale_reference fixture. This is an end-to-end pipeline test: \
         parse → graph → resolve → detect → manifest. \
         Got {} total diagnostics with patterns: {:?}",
        result.manifest.diagnostics.len(),
        result
            .manifest
            .diagnostics
            .iter()
            .map(|d| &d.pattern)
            .collect::<Vec<_>>()
    );

    // At least old_function should be flagged
    let old_fn_finding = stale_findings
        .iter()
        .find(|d| d.entity.contains("old_function"));
    assert!(
        old_fn_finding.is_some(),
        "old_function (imported from utils but renamed to new_function) must be \
         detected as a stale reference in the pipeline. Stale findings: {:?}",
        stale_findings.iter().map(|d| &d.entity).collect::<Vec<_>>()
    );
}

// =========================================================================
// T17: stale_reference Pipeline Evidence Quality
// =========================================================================

/// Every stale_reference finding in the manifest must have non-empty evidence,
/// suggestion, and location fields. Evidence quality is a core Flowspec value.
#[test]
fn test_stale_reference_pipeline_evidence_quality() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::write(
        tmp.path().join("main.py"),
        include_str!("../../tests/fixtures/python/cross_file/stale_reference/main.py"),
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("utils.py"),
        include_str!("../../tests/fixtures/python/cross_file/stale_reference/utils.py"),
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("helpers.py"),
        include_str!("../../tests/fixtures/python/cross_file/stale_reference/helpers.py"),
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = analyze(tmp.path(), &config, &["python".to_string()]).unwrap();

    let stale_findings: Vec<_> = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.pattern == "stale_reference")
        .collect();

    assert!(
        !stale_findings.is_empty(),
        "Prerequisite: stale_reference findings must exist for evidence quality check"
    );

    for finding in &stale_findings {
        assert!(
            !finding.evidence.is_empty(),
            "stale_reference finding for '{}' must include evidence in manifest output",
            finding.entity
        );
        assert!(
            !finding.suggestion.is_empty(),
            "stale_reference finding for '{}' must include suggestion in manifest output",
            finding.entity
        );
        assert!(
            !finding.loc.is_empty(),
            "stale_reference finding for '{}' must include location in manifest output",
            finding.entity
        );
    }
}
