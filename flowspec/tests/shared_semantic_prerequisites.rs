//! Public semantic controls: produced values need a resolved binding, not an unread name.
use flowspec::{analyze, Config};
use serde_json::Value;

fn report(source: &str, extension: &str) -> Value {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(format!("source.{extension}")), source).unwrap();
    analyze(root.path(), &Config::default(), &[])
        .unwrap()
        .manifest
        .authority
        .unwrap()
}
fn locals(report: &Value) -> Vec<&Value> {
    report["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| {
            f["pattern_id"] == "data_dead_end"
                && f["cause"]["parts"]["produced_kind"] == "local_value"
        })
        .collect()
}
#[test]
fn boolean_production_has_an_exact_unconsumed_binding_proof() {
    for ext in ["js", "ts"] {
        let r = report(
            "export function check(input) { const discarded = input === null; return input; }",
            ext,
        );
        let fs = locals(&r);
        assert_eq!(
            fs.len(),
            1,
            "unconsumed strict-comparison Boolean requires a positive: {r}"
        );
        assert_eq!(fs[0]["subject"]["name"], "discarded");
        assert_eq!(fs[0]["evidence"][0]["consumer_closure"], "complete");
    }
}
#[test]
fn returned_aliased_contained_and_registered_values_are_consumed() {
    for tail in [
        "return discarded;",
        "const alias = discarded; return alias;",
        "return [discarded];",
        "register(discarded); return input;",
    ] {
        let r = report(
            &format!("export function check(input) {{ const discarded = input === null; {tail} }}"),
            "js",
        );
        assert!(
            locals(&r)
                .iter()
                .all(|f| f["subject"]["name"] != "discarded"),
            "consumer lost: {r}"
        );
    }
}
#[test]
fn shadowed_binding_read_does_not_consume_the_outer_value() {
    let r = report("export function check(input) { const discarded = input === null; { const discarded = true; return discarded; } }", "js");
    let fs = locals(&r);
    assert_eq!(
        fs.len(),
        1,
        "one dead outer binding, one consumed inner binding: {r}"
    );
    assert!(fs[0]["range"]["start_byte"].as_u64().unwrap() < 70);
}
#[test]
fn reflection_prevents_an_absence_proof_but_does_not_erase_other_scope_positives() {
    let r = report("export function check(input) { const discarded = input === null; eval('discarded'); } export function other(input) { const abandoned = input === null; return input; }", "js");
    let fs = locals(&r);
    assert_eq!(
        fs.len(),
        1,
        "reflection is local proof loss, not global suppression: {r}"
    );
    assert_eq!(fs[0]["subject"]["name"], "abandoned");
    assert_eq!(r["execution"], "incomplete");
}
