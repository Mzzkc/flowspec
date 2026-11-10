//! Red-first regression tests for Python method-caller completeness.
//!
//! Fixture mirrors the consumer-reported shape (`mini-fixture`): one method,
//! four call sites reaching it through four receiver shapes. A backward trace
//! must list every one; the receiver whose type cannot be proven is reported
//! as a *possible* caller rather than dropped.

use crate::commands::{run_trace, TraceDirection};
use crate::OutputFormat;
use std::path::Path;

fn write_fixture(root: &Path) {
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    std::fs::write(root.join("pkg/__init__.py"), "").unwrap();
    std::fs::write(
        root.join("pkg/sheet.py"),
        "class Sheet:\n    def template_variables(self, n):\n        return {\"n\": n}\n\ndef make():\n    return Sheet()\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pkg/user.py"),
        "from pkg.sheet import Sheet, make\n\nLIMIT = 3\n\ndef direct(sheet: Sheet):\n    return sheet.template_variables(LIMIT)\n\ndef constructed():\n    s = Sheet()\n    return s.template_variables(1)\n\ndef untyped(sheet):\n    return sheet.template_variables(2)\n\nclass Holder:\n    def __init__(self):\n        self.sheet = make()\n    def go(self):\n        return self.sheet.template_variables(4)\n",
    )
    .unwrap();
}

fn trace_json(root: &Path, symbol: &str, direction: TraceDirection) -> serde_json::Value {
    let out = root.join("trace-out.json");
    let code = run_trace(
        root,
        symbol,
        &[],
        10,
        direction,
        OutputFormat::Json,
        Some(out.as_path()),
        None,
    )
    .unwrap_or_else(|e| panic!("trace {symbol} failed: {e}"));
    assert_eq!(code, 0);
    serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap()
}

/// Returns (caller entity, action) for the first step of every flow.
fn first_steps(flows: &serde_json::Value) -> Vec<(String, String)> {
    flows
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["steps"].as_array()?.first().cloned())
        .map(|s| {
            (
                s["entity"].as_str().unwrap().to_string(),
                s["action"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn backward_trace_lists_all_four_receiver_shapes() {
    let dir = tempfile::TempDir::new().unwrap();
    write_fixture(dir.path());
    let flows = trace_json(
        dir.path(),
        "sheet.py::Sheet::template_variables",
        TraceDirection::Backward,
    );
    let steps = first_steps(&flows);
    for caller in ["direct", "constructed", "untyped", "Holder::go"] {
        assert!(
            steps.iter().any(|(e, _)| e.ends_with(caller)),
            "caller {caller} missing from backward trace; got {steps:?}"
        );
    }
}

#[test]
fn backward_trace_marks_unproven_receiver_as_possible_with_site() {
    let dir = tempfile::TempDir::new().unwrap();
    write_fixture(dir.path());
    let flows = trace_json(
        dir.path(),
        "sheet.py::Sheet::template_variables",
        TraceDirection::Backward,
    );
    let steps = first_steps(&flows);
    let find = |name: &str| {
        steps
            .iter()
            .find(|(e, _)| e.ends_with(name))
            .unwrap_or_else(|| panic!("{name} missing: {steps:?}"))
            .1
            .clone()
    };
    // Proven receivers carry the call site and no "possible" marker.
    let direct = find("direct");
    assert!(direct.contains("user.py:6"), "direct action: {direct}");
    assert!(!direct.contains("possible"), "direct action: {direct}");
    let constructed = find("constructed");
    assert!(constructed.contains("user.py:10"), "{constructed}");
    assert!(!constructed.contains("possible"), "{constructed}");
    // Unproven receivers are reported, marked lower confidence.
    let untyped = find("untyped");
    assert!(untyped.contains("user.py:13"), "untyped action: {untyped}");
    assert!(untyped.contains("possible"), "untyped action: {untyped}");
    let holder = find("Holder::go");
    assert!(holder.contains("user.py:18"), "holder action: {holder}");
    assert!(holder.contains("possible"), "holder action: {holder}");
}

#[test]
fn zero_callers_states_what_was_searched() {
    let dir = tempfile::TempDir::new().unwrap();
    write_fixture(dir.path());
    let out = dir.path().join("summary.txt");
    // `direct` is called by nothing in the project.
    let code = run_trace(
        dir.path(),
        "user.py::direct",
        &[],
        10,
        TraceDirection::Backward,
        OutputFormat::Summary,
        Some(out.as_path()),
        None,
    )
    .unwrap();
    assert_eq!(code, 0);
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(
        text.contains("searched"),
        "zero-flow summary must say what was searched; got:\n{text}"
    );
    assert!(
        text.contains("call site"),
        "zero-flow summary must count call sites; got:\n{text}"
    );
}

/// Oracle-proved false positive: `self.sheet = make()` types the attribute by a
/// *function* name, so `self.sheet.template_variables()` must not make `make`
/// a callee of `Holder.go`. Only `Holder.__init__` calls `make()`.
#[test]
fn function_named_attribute_type_does_not_become_a_caller_edge() {
    let dir = tempfile::TempDir::new().unwrap();
    write_fixture(dir.path());
    let flows = trace_json(dir.path(), "sheet.py::make", TraceDirection::Backward);
    let steps = first_steps(&flows);
    assert!(
        steps.iter().any(|(e, _)| e.ends_with("Holder::__init__")),
        "Holder.__init__ really calls make(); got {steps:?}"
    );
    assert!(
        !steps.iter().any(|(e, _)| e.ends_with("Holder::go")),
        "Holder.go only calls a method on the result; got {steps:?}"
    );
}
