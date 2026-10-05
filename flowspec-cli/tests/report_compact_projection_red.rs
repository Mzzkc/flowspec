//! Small-source emitted limits retain authority and legacy consumer data.
use flowspec::{analyze, Config};
use serde_json::Value;
use std::{fs, process::Command};

fn run(root: &std::path::Path, format: &str, extra: &[&str]) -> (i32, Value, usize) {
    let out = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(root)
        .args(["--format", format])
        .args(extra)
        .output()
        .unwrap();
    assert!(
        matches!(out.status.code(), Some(0 | 3)),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value = if format == "yaml" {
        serde_yaml::from_slice(&out.stdout).unwrap()
    } else {
        serde_json::from_slice(&out.stdout).unwrap()
    };
    (out.status.code().unwrap(), value, out.stdout.len())
}

#[test]
fn small_incomplete_projection_preserves_catalog_and_legacy() {
    for source in [
        "\nimport os\n\ndef used_function():\n    return 42\n\ndef dead_function():\n    \"\"\"This function is never called by anything.\"\"\"\n    return \"unreachable\"\n\ndef main():\n    result = used_function()\n    print(result)\n",
        "\nimport os\n\ndef active_function():\n    return 42\n\ndef dead_function():\n    \"\"\"This function is never called by anything.\"\"\"\n    return \"unreachable\"\n\ndef another_dead_function(x):\n    return x * 2\n\ndef main():\n    result = active_function()\n    print(result)\n",
    ] {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("dead_code.py"), source).unwrap();
        let native = analyze(root.path(), &Config::default(), &[]).unwrap();
        let original_graph = flowspec::commands::expand_graph_tables(&native.manifest.authority.as_ref().unwrap()["analysis_graph"]).unwrap();
        assert_eq!(original_graph["entities"].as_array().unwrap().len(), native.graph.all_symbols().count());
        let baseline = flowspec::commands::expand_report_records(&serde_json::to_value(native.manifest).unwrap()).unwrap();
        let mut identity = None;
        for format in ["json", "yaml"] {
            let (code, wire, bytes) = run(root.path(), format, &[]);
            let report = flowspec::commands::expand_report_records(&wire).unwrap();
            assert!(bytes <= 10240, "actual {format} output {bytes} exceeds binding 10240");
            assert_eq!(code, 3);
            for field in ["entities", "flows", "boundaries", "dependency_graph", "type_flows", "diagnostics", "summary", "completeness"] {
                assert_eq!(report[field], baseline[field], "legacy {field} was erased by compression");
            }
            for field in ["findings", "inventory", "scope", "producer", "provenance"] {
                assert_eq!(report["authority"][field], baseline["authority"][field], "authority {field} changed");
            }
            let a = &report["authority"];
            assert_eq!(flowspec::commands::expand_graph_tables(&a["analysis_graph"]).unwrap(), original_graph);
            assert_eq!(a["losses"], baseline["authority"]["losses"]);
            assert_eq!(a["pattern_state"], baseline["authority"]["pattern_state"]);
            if let Some(id) = &identity { assert_eq!(&a["report_identity"], id); }
            identity = Some(a["report_identity"].clone());
        }
    }
}

#[test]
fn clean_success_and_real_private_finding_survive() {
    for (source, findings) in [
        ("def greet(name):\n    return f'Hello, {name}'\ndef main():\n    print(greet('world'))\nif __name__ == '__main__':\n    main()\n", 0),
        ("def _unused():\n    return 1\n", 1),
    ] {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("clean.py"), source).unwrap();
        for format in ["json", "yaml"] {
            let (code, report, _) = run(root.path(), format, &[]);
            assert_eq!(code, 0);
            assert_eq!(report["authority"]["execution"], "complete");
            assert!(report["authority"]["losses"].as_array().unwrap().is_empty());
            assert_eq!(report["authority"]["findings"]["total"], findings * 2);
            let report = flowspec::commands::expand_report_records(&report).unwrap();
            let catalog = report["authority"]["findings"]["all"].as_array().unwrap();
            for pattern in ["data_dead_end", "isolated_cluster"] {
                assert_eq!(catalog.iter().filter(|f| f["pattern_id"] == pattern).count(), findings);
            }
        }
    }
}

#[test]
fn incomplete_type_debt_keeps_real_private_catalog_under_filter() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("module.py"), "def _unused():\n    return 1\ndef main(x: int) -> int:\n    return x\nif __name__ == '__main__':\n    main(1)\n").unwrap();
    let baseline = serde_json::to_value(
        analyze(root.path(), &Config::default(), &[])
            .unwrap()
            .manifest,
    )
    .unwrap();
    let baseline = flowspec::commands::expand_report_records(&baseline).unwrap();
    assert_eq!(baseline["authority"]["findings"]["total"], 2);
    let catalog = baseline["authority"]["findings"]["all"].as_array().unwrap();
    for pattern in ["data_dead_end", "isolated_cluster"] {
        assert_eq!(
            catalog
                .iter()
                .filter(|f| f["pattern_id"] == pattern)
                .count(),
            1
        );
    }
    assert_eq!(catalog[0]["subject"], catalog[1]["subject"]);
    assert_eq!(catalog[0]["range"], catalog[1]["range"]);
    assert_ne!(catalog[0]["semantic_id"], catalog[1]["semantic_id"]);
    let mut identity = None;
    for (format, extra) in [
        ("json", vec![]),
        ("yaml", vec![]),
        ("json", vec!["--severity", "critical"]),
    ] {
        let (code, wire, _) = run(root.path(), format, &extra);
        let report = flowspec::commands::expand_report_records(&wire).unwrap();
        assert_eq!(code, 3);
        let a = &report["authority"];
        assert_eq!(a["findings"], baseline["authority"]["findings"]);
        assert_eq!(
            a["findings"]["all"][0]["subject"]["qualified_name"],
            "module.py::_unused"
        );
        assert_eq!(a["gate"]["verdict"], "not_evaluated");
        assert_eq!(report["entities"], baseline["entities"]);
        assert_eq!(
            flowspec::commands::expand_graph_tables(&a["analysis_graph"]),
            flowspec::commands::expand_graph_tables(&baseline["authority"]["analysis_graph"])
        );
        assert!(
            !flowspec::commands::expand_graph_tables(&a["analysis_graph"]).unwrap()["entities"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        if let Some(id) = &identity {
            assert_eq!(&a["report_identity"], id);
        }
        identity = Some(a["report_identity"].clone());
        assert_eq!(
            a["continuation"]["matched_total"],
            if extra.is_empty() { 2 } else { 0 }
        );
    }
}
