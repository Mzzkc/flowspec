use flowspec::{analyze, Config};
use std::fs;

fn check(source: &str, caller: &str, target: &str, excluded: &str) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.py"), source).unwrap();
    let result = analyze(dir.path(), &Config::default(), &["python".into()]).unwrap();
    let find = |qualified: &str| {
        result
            .graph
            .all_symbols()
            .find(|(_, s)| s.qualified_name == qualified)
            .unwrap()
            .0
    };
    let caller = find(caller);
    let target = find(target);
    let excluded = find(excluded);
    assert!(
        result.graph.callees(caller).contains(&target),
        "missing lexical target"
    );
    assert!(
        !result.graph.callees(caller).contains(&excluded),
        "wrong lexical target"
    );
    assert!(!result
        .manifest
        .diagnostics
        .iter()
        .any(|d| d.pattern == "data_dead_end"
            && d.entity == result.graph.get_symbol(target).unwrap().qualified_name));
    assert!(result
        .manifest
        .diagnostics
        .iter()
        .any(|d| d.pattern == "data_dead_end"
            && d.entity == result.graph.get_symbol(excluded).unwrap().qualified_name));
}
#[test]
fn nested_same_name_and_renamed_bindings_select_local_definition() {
    for name in ["_unconsumed", "_renamed"] {
        check(&format!("def {name}():\n    return 1\n\ndef other():\n    def {name}():\n        return 2\n    return {name}()\n"), "a.py::other", &format!("a.py::other::{name}"), &format!("a.py::{name}"));
    }
}
#[test]
fn closure_selects_enclosing_binding_and_excludes_sibling() {
    check("def _value():\n    return 1\ndef other():\n    def _value():\n        return 2\n    def inside():\n        return _value()\n    return inside()\n", "a.py::other::inside", "a.py::other::_value", "a.py::_value");
}
#[test]
fn sibling_nested_binding_is_invisible_to_module_function() {
    check("def _value():\n    return 1\ndef other():\n    def _value():\n        return 2\n    return 3\ndef entry():\n    return _value()\n", "a.py::entry", "a.py::_value", "a.py::other::_value");
}
