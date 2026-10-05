//! Requested rendering owns refusal; an unrequested projection cannot erase findings.
use std::{fs, process::Command};

#[test]
fn dense_complete_catalog_summary_survives_structured_refusal() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("private.py"),
        (0..80)
            .map(|i| format!("def _dense_{i}():\n    return {i}\n"))
            .collect::<String>(),
    )
    .unwrap();
    for command in ["analyze", "diagnose"] {
        let output = Command::new(env!("CARGO_BIN_EXE_flowspec"))
            .arg(command)
            .arg(root.path())
            .args(["--format", "summary", "--gate-severity", "info"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let summary = String::from_utf8(output.stdout).unwrap();
        assert!(summary.contains("execution: complete"), "{summary}");
        assert!(summary.contains("outcome: findings"), "{summary}");
        for format in ["json", "yaml"] {
            let target = root.path().join(format!("{command}-{format}.out"));
            fs::write(&target, "existing output").unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_flowspec"))
                .arg(command)
                .arg(root.path())
                .args(["--format", format, "--gate-severity", "info", "--output"])
                .arg(&target)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(1),
                "{format} must preserve the actual byte limit"
            );
            assert!(output.stdout.is_empty());
            assert_eq!(
                fs::read_to_string(target).unwrap(),
                "existing output",
                "refusal must precede output replacement"
            );
        }
    }
}

#[test]
fn dense_public_report_retains_complete_catalog_without_serialization_loss() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("private.py"),
        (0..80)
            .map(|i| format!("def _dense_{i}():\n    return {i}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let report = flowspec::diagnose_report(
        root.path(),
        &flowspec::Config::default(),
        &[],
        &flowspec::ReportOptions {
            gate_severity: flowspec::Severity::Info,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report.execution, Some(flowspec::Execution::Complete));
    let authority = report.authority.unwrap();
    assert_eq!(authority["findings"]["total"], 80);
    assert_eq!(authority["findings"]["all"].as_array().unwrap().len(), 80);
    assert!(authority["losses"].as_array().unwrap().is_empty());
    assert_eq!(authority["gate"]["qualifying_count"], 80);
}
