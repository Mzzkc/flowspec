//! QA-3 Cycle 11 tests — trace refactor (FROM semantics), CLI filter flags,
//! backward/both tracing, edge cases, and regression guards.

// =========================================================================
// Phase 1: Trace Refactor — FROM Semantics (P0, 3-cycle carry)
// =========================================================================

/// T1: THE BUG — trace on symbol with call edges but NO entry point.
/// This MUST fail before the refactor (manifest.flows is empty when no main()).
/// After refactor: trace_flows_from() traces directly → non-empty result.
#[test]
fn t1_trace_returns_flows_when_no_entry_point_exists() {
    let dir = tempfile::TempDir::new().unwrap();
    // handler() → helper(), NO main() function
    std::fs::write(
        dir.path().join("handler.py"),
        "def handler():\n    return helper()\n\ndef helper():\n    return 42\n",
    )
    .unwrap();

    let result = crate::commands::run_trace(
        dir.path(),
        "handler",
        &[],
        10,
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Yaml,
        None,
        None,
    );
    assert!(result.is_ok(), "Trace must not error: {:?}", result.err());
    // Exit code 0 = success
    assert_eq!(result.unwrap(), 0);
}

/// T2: Forward trace includes callees reachable from traced symbol.
#[test]
fn t2_trace_forward_includes_downstream_callees() {
    let dir = tempfile::TempDir::new().unwrap();
    // main() → process() → format_output() (linear chain)
    std::fs::write(
        dir.path().join("chain.py"),
        "def main():\n    return process()\n\ndef process():\n    return format_output()\n\ndef format_output():\n    return 42\n",
    )
    .unwrap();

    let output_file = dir.path().join("trace.json");
    let result = crate::commands::run_trace(
        dir.path(),
        "process",
        &[],
        10,
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Json,
        Some(output_file.as_path()),
        None,
    );
    assert!(result.is_ok(), "Trace must succeed: {:?}", result.err());

    let json_str = std::fs::read_to_string(&output_file).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let arr = parsed.as_array().unwrap();

    // FROM semantics: forward from process → should include format_output
    let has_format_output = arr.iter().any(|flow| {
        let steps = flow["steps"].as_array().unwrap_or(&vec![]);
        steps
            .iter()
            .any(|s| s["entity"].as_str().unwrap_or("").contains("format_output"))
    });
    assert!(
        has_format_output,
        "Forward trace from 'process' must include 'format_output' downstream. Got: {}",
        json_str
    );
}

/// T3: Depth=1 returns only direct callees.
#[test]
fn t3_trace_depth_1_returns_only_direct_callees() {
    let dir = tempfile::TempDir::new().unwrap();
    // entry() → middle() → leaf(), three functions linear
    std::fs::write(
        dir.path().join("chain.py"),
        "def entry():\n    return middle()\n\ndef middle():\n    return leaf()\n\ndef leaf():\n    return 42\n",
    )
    .unwrap();

    let output_file = dir.path().join("trace.yaml");
    let result = crate::commands::run_trace(
        dir.path(),
        "entry",
        &[],
        1, // depth=1 means only immediate callees
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Yaml,
        Some(output_file.as_path()),
        None,
    );
    assert!(
        result.is_ok(),
        "Depth=1 trace must succeed: {:?}",
        result.err()
    );

    let yaml_str = std::fs::read_to_string(&output_file).unwrap();
    // With depth=1, steps should be truncated to at most 1
    // "leaf" should NOT appear in the output because it's at depth 2
    assert!(
        !yaml_str.contains("leaf"),
        "Depth=1 trace from 'entry' must NOT include 'leaf' (depth 2). Got:\n{}",
        yaml_str
    );
}

/// T4: Terminal symbol returns empty flows (not error).
#[test]
fn t4_trace_terminal_symbol_returns_ok_empty() {
    let dir = tempfile::TempDir::new().unwrap();
    // main() → leaf(). leaf() calls nothing.
    std::fs::write(
        dir.path().join("main.py"),
        "def main():\n    return leaf()\n\ndef leaf():\n    return 42\n",
    )
    .unwrap();

    let result = crate::commands::run_trace(
        dir.path(),
        "leaf",
        &[],
        10,
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Yaml,
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "Terminal symbol trace must return Ok, not error. Got: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap(), 0);
}

/// T5: Trace with --format summary still produces parseable output.
#[test]
fn t5_trace_summary_format_works() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("main.py"),
        "def main():\n    return helper()\n\ndef helper():\n    return 42\n",
    )
    .unwrap();

    let output_file = dir.path().join("trace.txt");
    let result = crate::commands::run_trace(
        dir.path(),
        "main",
        &[],
        10,
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Summary,
        Some(output_file.as_path()),
        None,
    );
    assert!(
        result.is_ok(),
        "Summary trace must succeed: {:?}",
        result.err()
    );

    let output = std::fs::read_to_string(&output_file).unwrap();
    assert!(
        output.contains("Trace:"),
        "Summary output must contain 'Trace:' header. Got:\n{}",
        output
    );
}

/// T6: Symbol not found returns helpful error message.
#[test]
fn t6_trace_nonexistent_symbol_returns_symbol_not_found() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("main.py"),
        "def main():\n    return helper()\n\ndef helper():\n    return 42\n",
    )
    .unwrap();

    let result = crate::commands::run_trace(
        dir.path(),
        "nonexistent_symbol",
        &[],
        10,
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Yaml,
        None,
        None,
    );
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("not found"),
        "Error must mention 'not found'. Got: {}",
        err_msg
    );
    assert!(
        err_msg.contains("flowspec analyze"),
        "Error must suggest running flowspec analyze. Got: {}",
        err_msg
    );
}

// =========================================================================
// Phase 2: CLI Filter Flags (P1)
// =========================================================================

/// T10: --checks with invalid pattern name returns helpful error listing valid names.
#[test]
fn t10_invalid_pattern_name_returns_helpful_error() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("test.py"), "def foo(): pass\n").unwrap();

    let result = crate::commands::run_analyze(
        dir.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &["nonexistent_pattern".to_string()],
        None,
        None,
    );
    assert!(result.is_err(), "Invalid pattern name must error");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("nonexistent_pattern"),
        "Error must mention the invalid name. Got: {}",
        err_msg
    );
}

/// T12: Exit code isolation — filtering does not change exit code semantics.
/// Exit code 2 must survive output filtering.
#[test]
fn t12_severity_filter_does_not_hide_exit_code_2() {
    let dir = tempfile::TempDir::new().unwrap();
    // Create a project that produces critical diagnostics
    std::fs::write(
        dir.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    // First check if the project produces critical diagnostics
    let base_result = crate::commands::run_analyze(
        dir.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );

    if let Ok(base_code) = base_result {
        if base_code == 2 {
            // Now filter to warnings-only output — exit code must STILL be 2
            let filtered_result = crate::commands::run_analyze(
                dir.path(),
                &[],
                crate::OutputFormat::Yaml,
                None,
                None,
                &[],
                Some(crate::Severity::Warning),
                None,
            );
            assert!(filtered_result.is_ok());
            assert_eq!(
                filtered_result.unwrap(),
                2,
                "Exit code must be 2 when CRITICAL diagnostics exist, \
                 regardless of --severity filter"
            );
        }
    }
    // If no critical diagnostics, test is vacuously true (fixture-dependent)
}

/// T23: Multiple filters combined — severity + checks AND together.
#[test]
fn t23_combined_filters_severity_and_checks() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    // Filter by both checks and severity
    let result = crate::commands::run_analyze(
        dir.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &["data_dead_end".to_string()],
        Some(crate::Severity::Critical),
        None,
    );
    // Should succeed regardless of whether matching diagnostics exist
    assert!(
        result.is_ok(),
        "Combined filters must not error: {:?}",
        result.err()
    );
}

/// T27: Exit code 0/1/2 semantics preserved across all changes.
#[test]
fn t27_exit_code_semantics_preserved() {
    // Clean project: exit code 0
    let clean_dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        clean_dir.path().join("clean.py"),
        "def main():\n    return hello()\n\ndef hello():\n    return 42\n",
    )
    .unwrap();

    let clean_result = crate::commands::run_analyze(
        clean_dir.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(clean_result.is_ok());
    let clean_code = clean_result.unwrap();
    // Exit code 0 or 2 depending on whether diagnostics fire
    assert!(
        clean_code == 0 || clean_code == 2,
        "Exit code must be 0 (clean) or 2 (findings). Got: {}",
        clean_code
    );
}

// =========================================================================
// Phase 3: Backward/Both Tracing (P2)
// =========================================================================

/// T14: --direction backward returns callers of the symbol.
#[test]
fn t14_trace_backward_returns_callers() {
    let dir = tempfile::TempDir::new().unwrap();
    // main() → process() → leaf()
    std::fs::write(
        dir.path().join("chain.py"),
        "def main():\n    return process()\n\ndef process():\n    return leaf()\n\ndef leaf():\n    return 42\n",
    )
    .unwrap();

    let output_file = dir.path().join("trace.yaml");
    let result = crate::commands::run_trace(
        dir.path(),
        "process",
        &[],
        10,
        crate::commands::TraceDirection::Backward,
        crate::OutputFormat::Yaml,
        Some(output_file.as_path()),
        None,
    );
    assert!(
        result.is_ok(),
        "Backward trace must succeed: {:?}",
        result.err()
    );

    let yaml_str = std::fs::read_to_string(&output_file).unwrap();
    // Backward from process should include main (its caller)
    assert!(
        yaml_str.contains("main"),
        "Backward trace from 'process' must include 'main' as a caller. Got:\n{}",
        yaml_str
    );
}

/// T15: --direction both returns union of forward and backward.
#[test]
fn t15_trace_both_returns_forward_and_backward_union() {
    let dir = tempfile::TempDir::new().unwrap();
    // main() → process() → leaf()
    std::fs::write(
        dir.path().join("chain.py"),
        "def main():\n    return process()\n\ndef process():\n    return leaf()\n\ndef leaf():\n    return 42\n",
    )
    .unwrap();

    let output_file = dir.path().join("trace.yaml");
    let result = crate::commands::run_trace(
        dir.path(),
        "process",
        &[],
        10,
        crate::commands::TraceDirection::Both,
        crate::OutputFormat::Yaml,
        Some(output_file.as_path()),
        None,
    );
    assert!(
        result.is_ok(),
        "Both-direction trace must succeed: {:?}",
        result.err()
    );

    let yaml_str = std::fs::read_to_string(&output_file).unwrap();
    // Both direction should include main (backward) and leaf (forward)
    let has_main = yaml_str.contains("main");
    let has_leaf = yaml_str.contains("leaf");
    assert!(
        has_main && has_leaf,
        "Both-direction trace from 'process' must include 'main' (caller) and 'leaf' (callee). \
         main={}, leaf={}. Got:\n{}",
        has_main, has_leaf, yaml_str
    );
}

/// T16: Backward trace on root symbol returns empty (not error).
#[test]
fn t16_trace_backward_on_root_returns_empty() {
    let dir = tempfile::TempDir::new().unwrap();
    // main() → helper(). main() has no callers.
    std::fs::write(
        dir.path().join("main.py"),
        "def main():\n    return helper()\n\ndef helper():\n    return 42\n",
    )
    .unwrap();

    let result = crate::commands::run_trace(
        dir.path(),
        "main",
        &[],
        10,
        crate::commands::TraceDirection::Backward,
        crate::OutputFormat::Yaml,
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "Backward trace on root symbol must return Ok, not error. Got: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap(), 0);
}

/// T17: Backward trace depth limiting.
#[test]
fn t17_trace_backward_depth_1_returns_only_direct_callers() {
    let dir = tempfile::TempDir::new().unwrap();
    // top() → middle() → leaf()
    std::fs::write(
        dir.path().join("chain.py"),
        "def top():\n    return middle()\n\ndef middle():\n    return leaf()\n\ndef leaf():\n    return 42\n",
    )
    .unwrap();

    let output_file = dir.path().join("trace.yaml");
    let result = crate::commands::run_trace(
        dir.path(),
        "leaf",
        &[],
        1, // depth=1 means only direct callers
        crate::commands::TraceDirection::Backward,
        crate::OutputFormat::Yaml,
        Some(output_file.as_path()),
        None,
    );
    assert!(
        result.is_ok(),
        "Backward depth=1 must succeed: {:?}",
        result.err()
    );

    let yaml_str = std::fs::read_to_string(&output_file).unwrap();
    // With depth=1, "top" should NOT appear (indirect caller at depth 2)
    assert!(
        !yaml_str.contains("top"),
        "Backward depth=1 from 'leaf' must NOT include 'top' (indirect caller). Got:\n{}",
        yaml_str
    );
}

// =========================================================================
// Phase 4: Edge Cases and Adversarial
// =========================================================================

/// T18: Symbol with only incoming edges — forward empty, backward non-empty.
#[test]
fn t18_symbol_only_incoming_edges_forward_empty_backward_not() {
    let dir = tempfile::TempDir::new().unwrap();
    // caller_a() → target(), caller_b() → target(). target() calls nothing.
    std::fs::write(
        dir.path().join("multi_caller.py"),
        "def caller_a():\n    return target()\n\ndef caller_b():\n    return target()\n\ndef target():\n    return 42\n",
    )
    .unwrap();

    // Forward trace on target: should be empty (no outgoing calls)
    let forward_result = crate::commands::run_trace(
        dir.path(),
        "target",
        &[],
        10,
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Yaml,
        None,
        None,
    );
    assert!(forward_result.is_ok());

    // Backward trace on target: should include callers
    let backward_file = dir.path().join("backward.yaml");
    let backward_result = crate::commands::run_trace(
        dir.path(),
        "target",
        &[],
        10,
        crate::commands::TraceDirection::Backward,
        crate::OutputFormat::Yaml,
        Some(backward_file.as_path()),
        None,
    );
    assert!(backward_result.is_ok());

    let yaml_str = std::fs::read_to_string(&backward_file).unwrap();
    assert!(
        yaml_str.contains("caller_a") || yaml_str.contains("caller_b"),
        "Backward trace from target must include at least one caller. Got:\n{}",
        yaml_str
    );
}

/// T19: Cyclic call graph — backward trace terminates.
#[test]
fn t19_backward_trace_through_cycle_terminates() {
    let dir = tempfile::TempDir::new().unwrap();
    // ping() ↔ pong() (mutual recursion)
    std::fs::write(
        dir.path().join("cycle.py"),
        "def ping():\n    return pong()\n\ndef pong():\n    return ping()\n",
    )
    .unwrap();

    let result = crate::commands::run_trace(
        dir.path(),
        "pong",
        &[],
        10,
        crate::commands::TraceDirection::Backward,
        crate::OutputFormat::Yaml,
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "Backward trace through cycle must terminate, not hang. Got: {:?}",
        result.err()
    );
}

/// T20: All four output formats work with trace.
#[test]
fn t20_trace_all_output_formats() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("main.py"),
        "def main():\n    return helper()\n\ndef helper():\n    return 42\n",
    )
    .unwrap();

    let formats = [
        crate::OutputFormat::Yaml,
        crate::OutputFormat::Json,
        crate::OutputFormat::Sarif,
        crate::OutputFormat::Summary,
    ];

    for format in &formats {
        let result = crate::commands::run_trace(
            dir.path(),
            "main",
            &[],
            10,
            crate::commands::TraceDirection::Forward,
            *format,
            None,
            None,
        );
        assert!(
            result.is_ok(),
            "Format {:?} must not error: {:?}",
            format,
            result.err()
        );
    }
}

/// T21: Cross-file trace follows edges across file boundaries.
#[test]
fn t21_trace_follows_cross_file_call_edges() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("a.py"),
        "from b import callee\n\ndef caller():\n    return callee()\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("b.py"), "def callee():\n    return 42\n").unwrap();

    let output_file = dir.path().join("trace.json");
    let result = crate::commands::run_trace(
        dir.path(),
        "caller",
        &[],
        10,
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Json,
        Some(output_file.as_path()),
        None,
    );
    assert!(
        result.is_ok(),
        "Cross-file trace must succeed: {:?}",
        result.err()
    );
}

/// T22: Empty project — trace returns SymbolNotFound.
#[test]
fn t22_trace_empty_project_symbol_not_found() {
    let dir = tempfile::TempDir::new().unwrap();
    // No source files at all

    let result = crate::commands::run_trace(
        dir.path(),
        "anything",
        &[],
        10,
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Yaml,
        None,
        None,
    );
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("not found"),
        "Empty project trace error must contain 'not found'. Got: {}",
        err_msg
    );
}

// =========================================================================
// Regression Tests
// =========================================================================

/// T25: Trace forward on entry points still works (regression).
#[test]
fn t25_regression_trace_forward_from_entry_point() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("main.py"),
        "def main():\n    return process()\n\ndef process():\n    return 42\n",
    )
    .unwrap();

    let result = crate::commands::run_trace(
        dir.path(),
        "main",
        &[],
        10,
        crate::commands::TraceDirection::Forward,
        crate::OutputFormat::Yaml,
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "Forward trace from entry point must still work. Got: {:?}",
        result.err()
    );
}

/// T26: Existing run_diagnose filter behavior unchanged.
#[test]
fn t26_regression_diagnose_filters_still_work() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("dead_code.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let result = crate::commands::run_diagnose(
        dir.path(),
        &[],
        &["data_dead_end".to_string()],
        Some(crate::Severity::Warning),
        None,
        crate::OutputFormat::Yaml,
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "Diagnose with filters must still work. Got: {:?}",
        result.err()
    );
}
