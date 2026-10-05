//! Authored policies enter only through the actual validated config API.
use flowspec::{analyze, Config};
#[test]
fn explicit_semantic_policy_is_validated_and_part_of_report_identity() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("source.js"), "export function api(input) { return input; }").unwrap();
    let path = root.path().join("policy.yaml");
    std::fs::write(&path, "semantic_policy:\n  version: 1\n  environments:\n    javascript:\n      version: ECMA-262/2025\n      target: ECMAScript\n  peers:\n    - subjects: [source.js::api, source.js::other]\n      required_operations: [source.js::cleanup]\n").unwrap();
    let config = Config::load(root.path(), Some(&path)).expect("documented explicit semantic policy");
    let actual = analyze(root.path(), &config, &[]).unwrap().manifest.authority.unwrap();
    assert_eq!(actual["provenance"]["config"]["effective"]["semantic_policy"]["version"], 1);
    assert_eq!(actual["execution"], "incomplete", "requested unimplemented peer closure must be visible");
    assert!(actual["losses"].as_array().unwrap().iter().any(|l| l["reason"] == "policy_operation_unavailable"));
}
#[test]
fn arbitrary_policy_json_does_not_become_configuration() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("source.js"), "export function api(input) { return input; }").unwrap();
    std::fs::write(root.path().join("policy.json"), "{\"peers\":[\"invented\"]}").unwrap();
    let config = Config::load(root.path(), None).unwrap();
    assert_eq!(config.origin, "default");
}
#[test]
fn malformed_semantic_policy_is_a_hard_configuration_error() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("policy.yaml");
    for body in ["semantic_policy: {version: 2}", "semantic_policy: {version: 1, peers: [{subjects: [], required_operations: []}]}", "semantic_policy: {version: 1, magic: true}"] {
        std::fs::write(&path, body).unwrap();
        assert!(Config::load(root.path(), Some(&path)).is_err(), "malformed policy accepted: {body}");
    }
}
