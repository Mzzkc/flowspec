use flowspec::{analyze, Config, OutputFormatter, SarifFormatter};
use std::collections::BTreeSet;
use std::fs;
use tempfile::TempDir;

#[test]
fn sarif_results_retain_authoritative_findings_after_display_filtering() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("a.py"),
        "def _unconsumed():\n    return 1\n",
    )
    .unwrap();
    let mut report = analyze(dir.path(), &Config::default(), &[])
        .unwrap()
        .manifest;
    let authority = report.authority.as_ref().unwrap().clone();
    // Expected identity/cause/range projections derive from this exact 32-byte
    // private Python source and the v1 domain hashes, independently of display.
    // DDE and a closed singleton component are distinct authoritative relations.
    let expected: serde_json::Value = serde_json::from_str(r#"{"data_dead_end":{"cause":{"key_version":1,"parts":{"produced_binding_id":"sha256:a3af2b17b3ae0955321827459dc22e4f3cf5595e68e24073a230826c7794e37a","produced_kind":"function","unconsumed_relation":"unconsumed_private_symbol"}},"confidence":"high","evidence":[{"kind":"graph_inbound_consumption","observation":"0 callers in 1 analyzed files","range":null}],"identity_status":"stable","observation_id":"sha256:908b85a0c5adf8dc079d0ea177096c3042642b8e489912806f4577bc56e42575","pattern_id":"data_dead_end","range":{"end_byte":31,"end_column":13,"end_line":2,"path_key":"612e7079","start_byte":0,"start_column":1,"start_line":1},"semantic_id":"sha256:2f8780ac8de6d6d2832cd884eea6afc5449280481b355db4c32baf8db71ec276","severity":"warning","subject":{"binding_id":"sha256:a3af2b17b3ae0955321827459dc22e4f3cf5595e68e24073a230826c7794e37a","construct_kind":"function","language":"python","path_key":"612e7079","qualified_name":"a.py::_unconsumed"}},"isolated_cluster":{"cause":{"key_version":1,"parts":{"component_members":["sha256:a3af2b17b3ae0955321827459dc22e4f3cf5595e68e24073a230826c7794e37a"],"produced_binding_id":"sha256:a3af2b17b3ae0955321827459dc22e4f3cf5595e68e24073a230826c7794e37a","produced_kind":"function","unconsumed_relation":"closed_private_executable_component"}},"confidence":"high","evidence":[{"consumer_closure":"complete","external_inbound":0,"internal_executable_edges":0,"kind":"executable_component_closure","lexical_containment_is_consumption":false,"member_binding_ids":["sha256:a3af2b17b3ae0955321827459dc22e4f3cf5595e68e24073a230826c7794e37a"]}],"identity_status":"stable","observation_id":"sha256:992c29ecffb41b4b2d2e4fc23a50d794d46b5fe918f89c824d12193763b7f683","pattern_id":"isolated_cluster","range":{"end_byte":31,"end_column":13,"end_line":2,"path_key":"612e7079","start_byte":0,"start_column":1,"start_line":1},"semantic_id":"sha256:e5ed0b7d5129c0e27eb5d0623625d7088b40bd5f449427bc43267285b009f47b","severity":"warning","subject":{"binding_id":"sha256:a3af2b17b3ae0955321827459dc22e4f3cf5595e68e24073a230826c7794e37a","construct_kind":"function","language":"python","path_key":"612e7079","qualified_name":"a.py::_unconsumed"}}}"#).unwrap();
    assert_eq!(authority["execution"], "complete");
    assert_eq!(authority["findings"]["total"], 2);
    assert_eq!(authority["findings"]["by_pattern"]["data_dead_end"], 1);
    assert_eq!(authority["findings"]["by_pattern"]["isolated_cluster"], 1);
    assert_eq!(authority["findings"]["by_severity"]["warning"], 2);
    assert_eq!(
        authority["gate"],
        serde_json::json!({"qualifying_count": 0, "verdict": "pass"})
    );
    assert!(authority["analysis_graph"].is_object());
    assert_eq!(
        authority["analysis_graph"]["entities"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        authority["analysis_graph"]["dependency_graph"],
        serde_json::json!([])
    );
    assert_eq!(authority["analysis_graph"]["flows"], serde_json::json!([]));
    let findings = authority["findings"]["all"].as_array().unwrap();
    assert_eq!(findings.len(), 2);
    let mut patterns = BTreeSet::new();
    let mut semantic_ids = BTreeSet::new();
    let mut observation_ids = BTreeSet::new();
    for finding in findings {
        let pattern = finding["pattern_id"].as_str().unwrap();
        assert!(patterns.insert(pattern.to_owned()));
        let projection = expected[pattern].as_object().unwrap();
        for (key, value) in projection {
            assert_eq!(&finding[key], value, "{pattern}.{key}");
        }
        assert!(semantic_ids.insert(finding["semantic_id"].as_str().unwrap().to_owned()));
        assert!(observation_ids.insert(finding["observation_id"].as_str().unwrap().to_owned()));
    }
    assert_eq!(
        patterns,
        BTreeSet::from(["data_dead_end".to_owned(), "isolated_cluster".to_owned()])
    );
    for state in authority["pattern_state"].as_array().unwrap() {
        if patterns.contains(state["pattern_id"].as_str().unwrap()) {
            assert_eq!(state["cells"][0]["state"], "complete");
        }
    }
    report.diagnostics.clear();
    let sarif: serde_json::Value =
        serde_json::from_str(&SarifFormatter::new().format_manifest(&report).unwrap()).unwrap();
    let raw_run = &sarif["runs"][0];
    assert_eq!(raw_run["results"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        raw_run["tool"]["driver"]["rules"].as_array().map(Vec::len),
        Some(13)
    );
    assert_eq!(raw_run["invocations"][0]["executionSuccessful"], true);
    let expanded = flowspec::commands::expand_report_records(&sarif).unwrap();
    let run = &expanded["runs"][0];
    assert_eq!(run["results"], raw_run["results"]);
    assert_eq!(run["properties"]["flowspec"], authority);
    let results = run["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    let mut projected_ids = BTreeSet::new();
    for result in results {
        let semantic_id = result["partialFingerprints"]["flowspec.semantic_id"]
            .as_str()
            .unwrap();
        assert!(projected_ids.insert(semantic_id.to_owned()));
        let finding = findings
            .iter()
            .find(|finding| finding["semantic_id"] == semantic_id)
            .unwrap();
        assert_eq!(result["ruleId"], finding["pattern_id"]);
        assert_eq!(result["properties"]["flowspec"], *finding);
        let location = &result["locations"][0]["physicalLocation"];
        assert_eq!(location["artifactLocation"]["uri"], "a.py");
        assert_eq!(
            location["region"],
            serde_json::json!({"startLine": 1, "startColumn": 1, "endLine": 2, "endColumn": 13})
        );
    }
    assert_eq!(projected_ids, semantic_ids);
}
