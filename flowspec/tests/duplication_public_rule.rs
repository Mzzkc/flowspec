//! Rule-derived controls frozen before normalizer implementation; no oracle labels.
use flowspec::{analyze, Config};
use serde_json::Value;
fn report(source: &str, language: &str, extension: &str) -> Value {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(format!("source.{extension}")), source).unwrap();
    let cfg = root.path().join("policy.yaml");
    let (version, target, overflow) = match language {
        "python" => ("Python/3.14", "Python", ""),
        "rust" => (
            "Rust/2024",
            "x86_64-unknown-linux-gnu",
            "      overflow: checked\n",
        ),
        _ => ("ECMA-262/2025", "ECMAScript", ""),
    };
    std::fs::write(&cfg,format!("semantic_policy:\n  version: 1\n  environments:\n    {language}:\n      version: {version}\n      target: {target}\n{overflow}")).unwrap();
    let config = Config::load(root.path(), Some(&cfg)).unwrap();
    analyze(root.path(), &config, &[])
        .unwrap()
        .manifest
        .authority
        .unwrap()
}
fn duplicates(r: &Value) -> Vec<&Value> {
    r["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["pattern_id"] == "duplication")
        .collect()
}
fn js(name: &str, param: &str, local: &str, literal: i32) -> String {
    format!("function {name}({param}) {{ const {local} = (({param} + {literal}) * 2) - 3; if ({local} > 0) {{ return ({local} + 4) * 5; }} return {local} - 6; }}")
}
#[test]
fn exact_threshold_alpha_renaming_emits_all_three_pairs() {
    let r = report(
        &[
            js("a", "x", "t", 1),
            js("b", "y", "u", 1),
            js("c", "z", "v", 1),
        ]
        .join("\n"),
        "javascript",
        "js",
    );
    let pairs = duplicates(&r);
    assert_eq!(
        pairs.len(),
        3,
        "exact W8 subjects form three unordered pairs: {r}"
    );
    for f in pairs {
        assert_eq!(f["evidence"][0]["weight"], 8);
        assert_eq!(
            f["evidence"][0]["rule_sha256"],
            "8da06b2a92262e831d20a728d3f9fc8c62853b39e727009e16871a69f07f05a8"
        );
        assert!(f["semantic_id"]
            .as_str()
            .unwrap()
            .starts_with("duplication:"));
    }
}
#[test]
fn literal_declaration_and_binding_rivals_are_distinct() {
    let a = js("a", "x", "t", 1);
    for b in [
        js("b", "y", "u", 2),
        js("b", "y", "u", 1).replace("const ", "let "),
        js("b", "y", "u", 1).replace("u > 0", "y > 0"),
    ] {
        let r = report(&format!("{a}\n{b}"), "javascript", "js");
        assert!(duplicates(&r).is_empty(), "exact structural mismatch: {r}");
    }
}
#[test]
fn formatting_and_parentheses_do_not_refuse_a_proven_pair() {
    let a = js("a", "x", "t", 1);
    let b = js("b", "y", "u", 1).replace("((y + 1) * 2)", "( ( (y + 1) ) * 2 )");
    let r = report(&format!("{a}\n{b}"), "javascript", "js");
    assert_eq!(
        duplicates(&r).len(),
        1,
        "grouping trivia must normalize: {r}"
    );
}
#[test]
fn below_threshold_and_disconnected_shape_are_nonqualifying() {
    let a = js("a", "x", "t", 1).replace("(t + 4) * 5", "t + 4");
    let b = js("b", "y", "u", 1).replace("(u + 4) * 5", "u + 4");
    assert!(duplicates(&report(&format!("{a}\n{b}"), "javascript", "js")).is_empty());
    let body = "x + 1; x + 2; x + 3; x + 4; x + 5; x + 6; x + 7; x + 8; return x;";
    assert!(duplicates(&report(
        &format!("function a(x) {{{body}}} function b(x) {{{body}}}"),
        "javascript",
        "js"
    ))
    .is_empty());
}
#[test]
fn python_positional_only_can_rename_but_keyword_slots_cannot() {
    let a="def a(x, /):\n    t = ((x + 1) * 2) - 3\n    if t > 0:\n        return (t + 4) * 5\n    return t - 6\n";
    let b = a
        .replace("a(", "b(")
        .replace("x", "y")
        .replace("t =", "u =")
        .replace("if t", "if u")
        .replace("(t +", "(u +")
        .replace("return t", "return u");
    assert_eq!(
        duplicates(&report(&format!("{a}\n{b}"), "python", "py")).len(),
        1
    );
    assert!(duplicates(&report(
        &format!("{}\n{}", a.replace(", /", ""), b.replace(", /", "")),
        "python",
        "py"
    ))
    .is_empty());
}
#[test]
fn unsupported_subject_keeps_a_complete_pair_and_locates_its_loss() {
    let source = format!(
        "{}\n{}\nfunction mystery(x) {{ try {{ return x; }} finally {{ cleanup(); }} }}",
        js("a", "x", "t", 1),
        js("b", "y", "u", 1)
    );
    let r = report(&source, "javascript", "js");
    assert_eq!(duplicates(&r).len(), 1);
    assert_eq!(r["execution"], "incomplete");
    assert!(r["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["reason"] == "DUP_UNSUPPORTED_CONSTRUCT"
            && !l["ranges"].as_array().unwrap().is_empty()));
}
