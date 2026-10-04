//! Actual consumers and presentation must preserve one analysis authority.
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
        matches!(out.status.code(), Some(0 | 2 | 3)),
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
fn decorator_registration_consumes_function_and_plain_private_survives() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("module.py"), "registry = []\ndef register(fn):\n    registry.append(fn)\n    return fn\n@register\ndef _callback():\n    return 1\ndef _unused():\n    return 2\n").unwrap();
    let (_, report, _) = run(dir.path(), "json", &[]);
    assert!(
        report["authority"]["findings"]["all"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["subject"]["qualified_name"] != "module.py::_callback"),
        "registered binding must not be a dead end: {report}"
    );
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["entity"] != "module.py::_callback"),
        "known consumer must not remain in legacy output"
    );
    let plain = tempfile::tempdir().unwrap();
    fs::write(
        plain.path().join("module.py"),
        "def _unused():\n    return 2\n",
    )
    .unwrap();
    let (_, report, _) = run(plain.path(), "json", &[]);
    assert_eq!(report["authority"]["findings"]["total"], 1);
}

#[test]
fn near_bound_full_authority_is_independent_of_format_and_display() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = String::from("def _one():\n    return 1\ndef _two():\n    return 2\ndef _three():\n    return 3\ndef main():\n    return 0\nif __name__ == '__main__':\n    main()\n");
    source.push('#');
    source.push_str(&"x".repeat(1255 - source.len()));
    source.push('\n');
    assert_eq!(source.len(), 1256);
    fs::write(dir.path().join("module.py"), &source).unwrap();
    let controls = [
        ("json", vec![]),
        ("json", vec!["--checks", "isolated_cluster"]),
        ("yaml", vec![]),
        ("yaml", vec!["--severity", "critical"]),
    ];
    let mut identity = None;
    let mut exit = None;
    for (format, extra) in controls {
        let (code, report, bytes) = run(dir.path(), format, &extra);
        let a = &report["authority"];
        assert_eq!(a["findings"]["total"], 3);
        if let Some(ref id) = identity {
            assert_eq!(
                &a["report_identity"], id,
                "format/display changed analysis authority"
            );
        }
        if let Some(prior) = exit {
            assert_eq!(code, prior, "display changed analysis exit");
        }
        identity = Some(a["report_identity"].clone());
        exit = Some(code);
        assert!(bytes < 20480 || bytes <= source.len() * if format == "json" { 15 } else { 10 });
        let matched = if extra.is_empty() { 3 } else { 0 };
        assert_eq!(a["continuation"]["matched_total"], matched);
        assert_eq!(
            report["metadata"]["entity_count"].as_u64().unwrap() as usize,
            report["entities"].as_array().unwrap().len()
        );
        for loss in a["losses"].as_array().unwrap() {
            if loss["reason"] == "output_truncation" {
                for pattern in a["pattern_state"].as_array().unwrap() {
                    for cell in pattern["cells"].as_array().unwrap() {
                        assert_eq!(cell["state"], "incomplete");
                        assert!(cell["reason_codes"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|r| r == "output_truncation"));
                    }
                }
            }
        }
    }
}
