use flowspec::graph::populate_graph;
use flowspec::parser::{python::PythonAdapter, LanguageAdapter};
use flowspec::Graph;
use std::path::Path;

fn callees(source: &str, caller: &str) -> Vec<String> {
    let ir = PythonAdapter::new()
        .parse_file(Path::new("a.py"), source)
        .unwrap();
    let mut graph = Graph::new();
    populate_graph(&mut graph, &ir);
    let id = graph
        .all_symbols()
        .find(|(_, s)| s.qualified_name == format!("a.py::{caller}"))
        .unwrap()
        .0;
    let mut names: Vec<_> = graph
        .callees(id)
        .iter()
        .map(|id| graph.get_symbol(*id).unwrap().qualified_name.clone())
        .collect();
    names.sort();
    names
}
#[test]
fn parameter_call_does_not_consume_module_definition() {
    assert!(callees(
        "def _value():\n    return 1\ndef entry(_value):\n    return _value()\n",
        "entry"
    )
    .is_empty());
}
#[test]
fn explicit_global_bypasses_enclosing_definition() {
    assert_eq!(callees("def _value():\n    return 1\ndef outer():\n    def _value():\n        return 2\n    def inner():\n        global _value\n        return _value()\n    return inner()\n", "outer::inner"), ["a.py::_value"]);
}
#[test]
fn closure_parameter_does_not_consume_module_definition() {
    assert!(callees("def _value():\n    return 1\ndef outer(_value):\n    def inner():\n        return _value()\n    return inner()\n", "outer::inner").is_empty());
}
#[test]
fn assignment_shadows_entire_function_including_earlier_call() {
    assert!(callees(
        "def _value():\n    return 1\ndef entry():\n    _value()\n    _value = lambda: 2\n",
        "entry"
    )
    .is_empty());
}
#[test]
fn nonlocal_and_ordinary_closures_keep_true_local_function() {
    for declaration in ["", "        nonlocal _value\n"] {
        let source = format!("def _value():\n    return 1\ndef outer():\n    def _value():\n        return 2\n    def inner():\n{declaration}        return _value()\n    return inner()\n");
        assert_eq!(callees(&source, "outer::inner"), ["a.py::outer::_value"]);
    }
}
#[test]
fn class_namespace_is_excluded_from_method_closure() {
    assert_eq!(callees("def _value():\n    return 1\nclass Box:\n    def _value():\n        return 2\n    def entry(self):\n        return _value()\n", "Box::entry"), ["a.py::_value"]);
}
#[test]
fn global_declaration_in_nested_sibling_does_not_change_lookup() {
    assert_eq!(callees("def _value():\n    return 1\ndef outer():\n    def _value():\n        return 2\n    def sibling():\n        global _value\n        return _value()\n    return _value()\n", "outer"), ["a.py::outer::_value"]);
}
