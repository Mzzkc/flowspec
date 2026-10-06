use flowspec::{analyze, validate_manifest_size, Config, OutputFormatter, SarifFormatter};
use std::path::{Path, PathBuf};
#[test]
fn legacy_collision_projection_preserves_unique_rust_module_bindings() {
    let files: Vec<_> = [
        "project/src/lib.rs",
        "project/src/main.rs",
        "project/src/parser/mod.rs",
        "project/src/parser/rust.rs",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    let map = flowspec::build_module_map(&files);
    let canonical = PathBuf::from("project/src/parser/mod.rs");
    assert_eq!(
        map.values().filter(|path| *path == &canonical).count(),
        1,
        "a unique module binding cannot acquire a competing compatibility alias"
    );
    assert_eq!(map.get("crate::parser"), Some(&canonical));
}
#[test]
fn full_authority_sarif_obeys_preserved_rust_fixture_output_budget() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/rust");
    let result = analyze(&root, &Config::default(), &["rust".into()]).unwrap();
    let raw_output = SarifFormatter::new()
        .format_manifest(&result.manifest)
        .unwrap();
    let destination = tempfile::tempdir().unwrap();
    let output_path = destination.path().join("report.sarif");
    let exit = flowspec::commands::run_analyze(
        &root,
        &["rust".into()],
        flowspec::OutputFormat::Sarif,
        Some(&output_path),
        None,
        &[],
        None,
        None,
    )
    .unwrap();
    assert!(matches!(exit, 0 | 2 | 3));
    let output = std::fs::read_to_string(output_path).unwrap();
    let raw_wire: serde_json::Value = serde_json::from_str(&raw_output).unwrap();
    let raw = flowspec::commands::expand_report_records(&raw_wire).unwrap();
    let rendered_wire: serde_json::Value = serde_json::from_str(&output).unwrap();
    let rendered = flowspec::commands::expand_report_records(&rendered_wire).unwrap();
    let mut expected_ids: Vec<_> = result.manifest.authority.as_ref().unwrap()["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| finding["semantic_id"].as_str().unwrap().to_owned())
        .collect();
    let rows = rendered["runs"][0]["results"].as_array().unwrap();
    let mut actual_ids = Vec::new();
    let mut legacy_rows = 0;
    for row in rows {
        if row.get("partialFingerprints").is_some() {
            let id = row["partialFingerprints"]["flowspec.semantic_id"]
                .as_str()
                .expect("qualified row must carry an actual semantic ID");
            let finding = result.manifest.authority.as_ref().unwrap()["findings"]["all"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["semantic_id"] == id)
                .expect("fingerprint must bind a full authoritative finding");
            assert_eq!(row["ruleId"], finding["pattern_id"]);
            actual_ids.push(id.to_owned());
        } else {
            // Legacy display rows have no authoritative cause proof. Check each
            // against the real diagnostic; never invent a semantic fingerprint.
            assert!(result
                .manifest
                .diagnostics
                .iter()
                .any(|d| row["ruleId"] == d.pattern && row["message"]["text"] == d.message));
            legacy_rows += 1;
        }
    }
    assert_eq!(rows.len(), actual_ids.len() + legacy_rows);
    // Exact diagnostic display projection is a multiset, not any-match membership.
    // Qualifying an existing diagnostic adds a fingerprint; appended authoritative
    // rows have no legacy diagnostic counterpart. Match the documented first
    // pattern/entity diagnostic binding, preserving every remaining row in full.
    let diagnostic_wire: serde_json::Value = serde_json::from_str(
        &SarifFormatter::new()
            .format_diagnostics(&result.manifest.diagnostics)
            .unwrap(),
    )
    .unwrap();
    let qualified_indexes: std::collections::HashSet<usize> =
        result.manifest.authority.as_ref().unwrap()["findings"]["all"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|finding| {
                result.manifest.diagnostics.iter().position(|diagnostic| {
                    finding["pattern_id"] == diagnostic.pattern
                        && finding["subject"]["qualified_name"] == diagnostic.entity
                })
            })
            .collect();
    let mut expected_legacy: Vec<String> = diagnostic_wire["runs"][0]["results"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(index, _)| !qualified_indexes.contains(index))
        .map(|(_, row)| serde_json::to_string(row).unwrap())
        .collect();
    let mut observed_legacy: Vec<String> = rows
        .iter()
        .filter(|row| row.get("partialFingerprints").is_none())
        .map(|row| serde_json::to_string(row).unwrap())
        .collect();
    expected_legacy.sort();
    observed_legacy.sort();
    assert_eq!(expected_legacy, observed_legacy, "every legacy diagnostic row, including multiplicity, locations, severity, evidence and message, must be conserved");
    assert_eq!(
        rendered["runs"][0]["properties"]["flowspec"],
        result.manifest.authority.as_ref().unwrap().clone(),
        "complete Graph/causes/ranges/identity must reach the public SARIF authority"
    );
    let rules = rendered["runs"][0]["tool"]["driver"]["rules"]
        .as_array()
        .unwrap();
    assert_eq!(rules.len(), 13);
    let mut ids: Vec<_> = rules
        .iter()
        .map(|r| {
            assert_eq!(r.as_object().unwrap().len(), 2);
            let id = r["id"].as_str().expect("actual SARIF descriptor id");
            assert_eq!(r["shortDescription"]["text"], id.replace('_', " "));
            id.to_owned()
        })
        .collect();
    ids.sort();
    let mut required: Vec<_> = result.manifest.authority.as_ref().unwrap()["scope"]["pattern_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    required.sort();
    assert_eq!(ids, required);
    expected_ids.sort();
    actual_ids.sort();
    assert_eq!(expected_ids, actual_ids);
    assert_eq!(
        raw["runs"][0]["tool"]["driver"]["rules"],
        rendered["runs"][0]["tool"]["driver"]["rules"]
    );
    assert!(
        validate_manifest_size(&output, result.source_bytes, "sarif").is_ok(),
        "full SARIF authority must retain the existing20x/floor budget; bytes={} source={}",
        output.len(),
        result.source_bytes
    );
}

#[test]
fn actual_source_findings_and_full_cause_range_authority_reach_public_sarif() {
    let root = tempfile::tempdir().unwrap();
    let source = "def _work():\n    return 1\n";
    std::fs::write(root.path().join("unit.py"), source).unwrap();
    let result = analyze(root.path(), &Config::default(), &["python".into()]).unwrap();
    let authority = result.manifest.authority.as_ref().unwrap();
    let findings = authority["findings"]["all"].as_array().unwrap();
    assert_eq!(findings.len(), 2);
    assert!(findings.iter().any(|f| f["pattern_id"] == "data_dead_end"));
    assert!(findings
        .iter()
        .any(|f| f["pattern_id"] == "isolated_cluster"));
    for finding in findings {
        assert_eq!(finding["range"]["start_byte"], 0);
        assert_eq!(finding["range"]["end_byte"], source.trim_end().len());
        assert!(finding["cause"]["parts"]
            .as_object()
            .is_some_and(|parts| !parts.is_empty()));
        assert!(finding["subject"]["binding_id"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
    }
    let destination = tempfile::tempdir().unwrap();
    let output_path = destination.path().join("positive.sarif");
    let code = flowspec::commands::run_analyze(
        root.path(),
        &["python".into()],
        flowspec::OutputFormat::Sarif,
        Some(&output_path),
        None,
        &[],
        None,
        None,
    )
    .unwrap();
    assert_eq!(code, 0);
    let raw = std::fs::read_to_string(&output_path).unwrap();
    assert!(validate_manifest_size(&raw, result.source_bytes, "sarif").is_ok());
    let wire: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let public = flowspec::commands::expand_report_records(&wire).unwrap();
    assert_eq!(
        public["runs"][0]["properties"]["flowspec"],
        authority.clone()
    );
    let mut expected: Vec<_> = findings
        .iter()
        .map(|f| f["semantic_id"].as_str().unwrap())
        .collect();
    let mut observed: Vec<_> = public["runs"][0]["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            f["partialFingerprints"]["flowspec.semantic_id"]
                .as_str()
                .expect("every real positive must have a fingerprint")
        })
        .collect();
    expected.sort();
    observed.sort();
    assert_eq!(expected, observed);
    assert_eq!(
        public["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap()
            .len(),
        13
    );
}
