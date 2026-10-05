//! Import uses have genuine lexical/value/export origins.
use flowspec::{analyze, Config};
#[test]
fn exported_aliased_contained_and_inherited_imports_have_consumers() {
    for consumer in ["export { utility };", "const alias = utility; export { alias };", "export function api() { return [utility]; }", "export class Sub extends utility {}"] {
        let root=tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("peer.js"),"export function utility() { return true; }").unwrap();
        std::fs::write(root.path().join("source.js"),format!("import {{ utility }} from './peer.js'; {consumer}")).unwrap();
        let actual=analyze(root.path(),&Config::default(),&[]).unwrap();
        assert!(actual.manifest.diagnostics.iter().all(|d| !(d.pattern=="phantom_dependency" && d.entity=="utility")),"real imported binding consumer lost: {:?}",actual.manifest.diagnostics);
    }
}
#[test]
fn shadowed_parameter_is_not_a_use_of_the_import() {
 let root=tempfile::tempdir().unwrap();
 std::fs::write(root.path().join("peer.js"),"export function utility() { return true; }").unwrap();
 std::fs::write(root.path().join("source.js"),"import { utility } from './peer.js'; export function api(utility) { return utility; }").unwrap();
 let actual=analyze(root.path(),&Config::default(),&[]).unwrap();
 assert!(actual.manifest.diagnostics.iter().any(|d|d.pattern=="phantom_dependency" && d.entity=="utility"));
}

