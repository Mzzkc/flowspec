use flowspec::{analyze, Config};
fn findings(source: &str) -> Vec<serde_json::Value> {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("unit.mjs"), source).unwrap();
    let a = analyze(d.path(), &Config::default(), &[]).unwrap();
    a.manifest.authority.unwrap()["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["pattern_id"] == "contract_mismatch")
        .cloned()
        .collect()
}
#[test]
fn adjacent_jsdoc_return_contradiction_and_attachment_rivals() {
    let source = "/** @returns {number} */\nexport function produce() { return true; }\n";
    let f = findings(source);
    assert_eq!(
        f.len(),
        1,
        "explicit number versus boolean contract must survive"
    );
    assert_eq!(f[0]["cause"]["parts"]["expected"], "number");
    assert_eq!(f[0]["cause"]["parts"]["actual"], "boolean");
    assert_eq!(f[0]["range"]["start_byte"], source.find("true").unwrap());
    assert_eq!(f[0]["range"]["end_byte"], source.find("true").unwrap() + 4);
    for rival in [
        "/** @returns {number} */\nexport function produce() { return 7; }\n",
        "export function produce() { return true; }\n",
        "/** @returns {number} */\nconst unrelated = 1;\nexport function produce() { return true; }\n",
        "/* @returns {number} */\nexport function produce() { return true; }\n",
        "/** @returns {number} */\nexport function produce() { function nested() { return true; } return 7; }\n",
        "/** @returns {boolean} */\nexport function produce() { return true; }\n",
        "/** @returns {string} */\nexport function produce() { return 'ok'; }\n",
    ] { assert!(findings(rival).is_empty(), "unbound or matching rival: {rival}"); }
    assert_eq!(
        findings("/** @return {boolean} */\nfunction produce() { return 'wrong'; }\n").len(),
        1
    );
}
