use assert_cmd::Command;
use serde_json::Value;
#[test]
fn ordinary_incremental_cli_reuses_files_and_reports_a_warm_run() {
    let root=tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("main.py"),"def main():\n    return 1\nmain()\n").unwrap();
    let run=|| {let output=Command::cargo_bin("flowspec").unwrap().args(["--format","json","analyze",root.path().to_str().unwrap(),"--incremental"]).output().unwrap(); serde_json::from_slice::<Value>(&output.stdout).unwrap()};
    let _=run();let warm=run();
    assert_eq!(warm["metadata"]["incremental"],true,"--incremental must perform actual per-file reuse, not always label the run full");
    assert_eq!(warm["metadata"]["files_changed"],0);
}
