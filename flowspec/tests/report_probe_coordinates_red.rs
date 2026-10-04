#![cfg(feature = "fault_probe")]
use flowspec::fault_probe::{analyze_with_plan, FaultCase, FaultPlan};
#[test]
fn injected_multiline_node_ranges_use_actual_consumed_byte_coordinates() {
    let dir = tempfile::tempdir().unwrap();
    let source = "match value:\n    case 1:\n        pass\n";
    std::fs::write(dir.path().join("unsupported.py"), source).unwrap();
    let plan = FaultPlan::new(
        FaultCase::UnsupportedRequiredConstruct,
        "unsupported.py",
        "node-coordinate-control",
    )
    .unwrap();
    let report = analyze_with_plan(dir.path(), &flowspec::Config::default(), &[], plan).unwrap();
    assert!(!report.operations.is_empty());
    let a = report.analysis.manifest.authority.unwrap();
    for loss in a["losses"].as_array().unwrap() {
        for range in loss["ranges"].as_array().unwrap() {
            let start = range["start_byte"].as_u64().unwrap() as usize;
            let end = range["end_byte"].as_u64().unwrap() as usize;
            assert!(start <= end && end <= source.len());
            for (offset, line, column) in [
                (start, "start_line", "start_column"),
                (end, "end_line", "end_column"),
            ] {
                let bytes = &source.as_bytes()[..offset];
                let expected_line = bytes.iter().filter(|b| **b == b'\n').count() as u64 + 1;
                let expected_column = bytes
                    .iter()
                    .rposition(|b| *b == b'\n')
                    .map_or(bytes.len(), |n| bytes.len() - n - 1)
                    as u64
                    + 1;
                assert_eq!(range[line], expected_line, "{range}");
                assert_eq!(range[column], expected_column, "{range}");
            }
        }
    }
}
