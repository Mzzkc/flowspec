use flowspec::{analyze, commands::expand_report_records, Config, OutputFormatter, SarifFormatter};
use serde_json::{json, Value};

fn fixture_report() -> flowspec::AnalysisResult {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/rust");
    analyze(&fixtures, &Config::default(), &[]).unwrap()
}

#[test]
fn sarif_records_preserve_authority_and_standard_results_within_original_bound() {
    let report = fixture_report();
    let output = SarifFormatter::new()
        .format_manifest(&report.manifest)
        .unwrap();
    flowspec::validate_manifest_size(&output, report.source_bytes, "sarif").unwrap();
    let wire: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(
        wire["runs"][0]["properties"]["report_records"]["encoding"],
        "flowspec.report.records.v2"
    );
    assert!(
        wire.get("report_records").is_none(),
        "Only the SARIF extension property bag may carry the table"
    );
    let expanded = expand_report_records(&wire).unwrap();
    assert_eq!(
        expanded["runs"][0]["properties"]["flowspec"],
        report.manifest.authority.as_ref().unwrap().clone()
    );
    assert!(expanded["runs"][0]["properties"]
        .get("report_records")
        .is_none());
    let legacy: Value = serde_json::from_str(
        &SarifFormatter::new()
            .format_diagnostics(&report.manifest.diagnostics)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(expanded["runs"][0]["results"], legacy["runs"][0]["results"]);
    assert_eq!(expanded["runs"][0]["results"].as_array().unwrap().len(), 12);
    assert_eq!(
        expanded["runs"][0]["invocations"][0]["executionSuccessful"],
        false
    );
    assert!(!expanded["runs"][0]["properties"]["flowspec"]["losses"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(expand_report_records(&expanded).unwrap(), expanded);
}

#[test]
fn encoded_sarif_rejects_malformed_tables_and_missing_authority() {
    let report = fixture_report();
    let wire: Value = serde_json::from_str(
        &SarifFormatter::new()
            .format_manifest(&report.manifest)
            .unwrap(),
    )
    .unwrap();
    for mutation in 0..5 {
        let mut bad = wire.clone();
        match mutation {
            0 => bad["runs"][0]["properties"]["report_records"]["encoding"] = json!("unsupported"),
            1 => bad["runs"][0]["properties"]["report_records"]["nodes"] = json!([]),
            2 => bad["runs"][0]["properties"]["flowspec"]["losses"] = json!({"$record":999999}),
            3 => bad["runs"][0]["properties"]["flowspec"] = Value::Null,
            4 => {
                let second = bad["runs"][0].clone();
                bad["runs"].as_array_mut().unwrap().push(second);
            }
            _ => unreachable!(),
        }
        assert!(
            expand_report_records(&bad).is_err(),
            "Malformed case {mutation} must fail closed"
        );
    }
}

#[test]
fn ordinary_root_tables_cannot_be_hidden_by_sarif_shaped_fields() {
    let report = fixture_report();
    let wire: Value = serde_json::from_str(
        &flowspec::JsonFormatter::new()
            .format_manifest(&report.manifest)
            .unwrap(),
    )
    .unwrap();
    assert!(wire.get("report_records").is_some());
    for runs in [json!([]), json!([{}])] {
        let mut ordinary = wire.clone();
        ordinary["runs"] = runs;
        let expanded = expand_report_records(&ordinary).unwrap();
        assert_eq!(
            expanded["authority"],
            report.manifest.authority.as_ref().unwrap().clone()
        );
        ordinary["report_records"]["encoding"] = json!("unsupported");
        assert!(expand_report_records(&ordinary).is_err());
    }
    let mut ambiguous = wire.clone();
    ambiguous["runs"] = json!([{"properties":{"report_records":wire["report_records"]}}]);
    assert!(expand_report_records(&ambiguous).is_err());
}
