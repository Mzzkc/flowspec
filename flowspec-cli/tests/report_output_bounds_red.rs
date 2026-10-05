//! The emitted bytes, including authority, must obey the real output budget.
use std::fs;
use std::path::Path;
use std::process::Command;

fn run(root: &Path, format: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(root)
        .args(["--format", format])
        .output()
        .unwrap()
}

fn within_limit(bytes: usize, source: usize, ratio: usize) -> bool {
    bytes < 20_480 || bytes <= source * ratio
}

#[test]
fn ordinary_rust_fixture_emits_bounded_real_yaml_and_json() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/rust");
    let source: usize = fs::read_dir(&root)
        .unwrap()
        .map(|entry| fs::metadata(entry.unwrap().path()).unwrap().len() as usize)
        .sum();
    for (format, ratio) in [("yaml", 10), ("json", 15)] {
        let output = run(&root, format);
        assert!(matches!(output.status.code(), Some(0 | 3)));
        assert!(
            within_limit(output.stdout.len(), source, ratio),
            "actual {format} output {} bytes over {source} source bytes",
            output.stdout.len()
        );
        if output.status.code() == Some(3) {
            let report: serde_json::Value = if format == "json" {
                serde_json::from_slice(&output.stdout).unwrap()
            } else {
                serde_yaml::from_slice(&output.stdout).unwrap()
            };
            let report = flowspec::commands::expand_report_records(&report).unwrap();
            assert_eq!(report["authority"]["execution"], "incomplete");
            let original = flowspec::analyze(&root, &flowspec::Config::default(), &[]).unwrap();
            let authority = original.manifest.authority.as_ref().unwrap();
            assert_eq!(report["authority"]["losses"], authority["losses"]);
            assert_eq!(
                report["authority"]["pattern_state"],
                authority["pattern_state"]
            );
            let graph =
                flowspec::commands::expand_graph_tables(&report["authority"]["analysis_graph"])
                    .unwrap();
            assert_eq!(
                graph["entities"].as_array().unwrap().len(),
                original.graph.all_symbols().count()
            );
            assert!(!graph["entities"].as_array().unwrap().is_empty());
            assert_eq!(
                graph,
                flowspec::commands::expand_graph_tables(&authority["analysis_graph"]).unwrap()
            );
        }
    }
}

#[test]
fn twenty_selected_files_emit_bounded_yaml() {
    let root = tempfile::tempdir().unwrap();
    for i in 0..20 {
        fs::write(
            root.path().join(format!("mod_{i}.py")),
            format!(
                "import os\nimport sys\ndef func_{i}(x):\n    return os.path.join(str(x), sys.argv[0])\n"
            ),
        )
        .unwrap();
    }
    let source = 1630;
    let output = run(root.path(), "yaml");
    assert!(matches!(output.status.code(), Some(0 | 3)));
    assert!(
        within_limit(output.stdout.len(), source, 10),
        "actual YAML output {} bytes over {source} source bytes",
        output.stdout.len()
    );
    let report: serde_json::Value = serde_yaml::from_slice(&output.stdout).unwrap();
    let report = flowspec::commands::expand_report_records(&report).unwrap();
    assert_eq!(report["authority"]["inventory"]["selected"], 20);
    if output.status.code() == Some(3) {
        assert_eq!(report["authority"]["execution"], "incomplete");
        let original = flowspec::analyze(root.path(), &flowspec::Config::default(), &[]).unwrap();
        let authority = original.manifest.authority.as_ref().unwrap();
        assert_eq!(report["authority"]["losses"], authority["losses"]);
        assert_eq!(
            report["authority"]["pattern_state"],
            authority["pattern_state"]
        );
        let graph = flowspec::commands::expand_graph_tables(&report["authority"]["analysis_graph"])
            .unwrap();
        assert_eq!(
            graph["entities"].as_array().unwrap().len(),
            original.graph.all_symbols().count()
        );
        assert!(!graph["entities"].as_array().unwrap().is_empty());
        assert_eq!(
            graph,
            flowspec::commands::expand_graph_tables(&authority["analysis_graph"]).unwrap()
        );
    }
}
