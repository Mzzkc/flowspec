use assert_cmd::Command;
use serde_json::Value;
#[test]
fn trace_report_has_exact_hops_and_depth_continuation() {
    let dir=tempfile::tempdir().unwrap();std::fs::write(dir.path().join("app.py"),"def main():\n    first()\ndef first():\n    last()\ndef last():\n    return 1\n").unwrap();
    let output=Command::cargo_bin("flowspec").unwrap().args(["--format","json","trace",dir.path().to_str().unwrap(),"--symbol","main","--depth","1","--report"]).output().unwrap();
    assert_eq!(output.status.code(),Some(3),"depth loss is exit 3, not clean or unsupported-argument exit");
    let report:Value=serde_json::from_slice(&output.stdout).unwrap();assert_eq!(report["execution"],"incomplete");assert_eq!(report["paths"][0]["hops"][0]["edge_kind"],"calls");assert_eq!(report["paths"][0]["hops"].as_array().unwrap().len(),1);assert!(report["continuation"].as_array().unwrap().len()>0);
}
