use flowspec::{analyze, commands, Config, OutputFormat};
use std::{fs, path::Path};
use tempfile::TempDir;

#[test]
fn incomplete_rust_report_preserves_entities_within_requested_output_bounds() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/rust");
    let result = analyze(&root, &Config::default(), &["rust".into()]).unwrap();
    let expected = result
        .graph
        .all_symbols()
        .filter(|(_, symbol)| symbol.kind != flowspec::parser::ir::SymbolKind::Module)
        .count();
    assert_eq!(result.manifest.entities.len(), expected);
    assert!(expected > 0);
    let output = TempDir::new().unwrap();
    for (format, name, ratio) in [
        (OutputFormat::Yaml, "yaml", 10),
        (OutputFormat::Json, "json", 15),
    ] {
        let target = output.path().join(name);
        commands::run_analyze(&root, &[], format, Some(&target), None, &[], None, None).unwrap();
        let bytes = fs::read(&target).unwrap();
        assert!(bytes.len() < 20480 || bytes.len() as u64 <= result.source_bytes * ratio);
        let report: serde_json::Value = if name == "yaml" {
            serde_yaml::from_slice(&bytes).unwrap()
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        let report = flowspec::commands::expand_report_records(&report).unwrap();
        assert_eq!(report["entities"].as_array().unwrap().len(), expected);
        assert_eq!(report["authority"]["execution"], "incomplete");
    }
}

#[test]
fn javascript_ordinary_report_preserves_four_original_basic_symbols() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript");
    let result = analyze(&fixture, &Config::load(&fixture, None).unwrap(), &[]).unwrap();
    let mut names: Vec<_> = result
        .manifest
        .entities
        .iter()
        .filter(|e| e.loc.starts_with("basic_functions.js:"))
        .map(|e| e.id.rsplit("::").next().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["add", "greet", "main", "multiply"]);
    assert_eq!(
        result
            .graph
            .all_symbols()
            .filter(|(_, s)| s.location.file.ends_with("basic_functions.js")
                && s.kind != flowspec::parser::ir::SymbolKind::Module)
            .count(),
        4
    );
}

#[test]
fn projected_yaml_retains_all_eight_legacy_root_sections_in_order() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("entry.py"),
        "def _hidden():\n    return 1\n",
    )
    .unwrap();
    let output = dir.path().join("report.yaml");
    commands::run_analyze(
        dir.path(),
        &[],
        OutputFormat::Yaml,
        Some(&output),
        None,
        &[],
        None,
        None,
    )
    .unwrap();
    let yaml = fs::read_to_string(output).unwrap();
    let expected = [
        "metadata",
        "summary",
        "diagnostics",
        "entities",
        "flows",
        "boundaries",
        "dependency_graph",
        "type_flows",
    ];
    let actual: Vec<_> = yaml
        .lines()
        .filter_map(|line| {
            let (key, _) = line.split_once(':')?;
            expected.contains(&key).then_some(key)
        })
        .collect();
    assert_eq!(actual, expected);
    let _: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
}
