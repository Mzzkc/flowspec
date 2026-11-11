//! The accuracy gate: an `ast`-derived oracle scores `trace` on fixtures with
//! known answers. False negatives and false positives must each stay below
//! 1% (decision of 2026-11-11); on these fixtures that means zero of either.
//!
//! The scripts in `scripts/oracle/` are the same ones used against real
//! corpora (Marianne, Django, CPython); this test runs them on the committed
//! fixture so every change is measured.

use std::path::Path;
use std::process::Command;

fn python() -> Option<&'static str> {
    for p in ["python3", "python"] {
        if Command::new(p).arg("--version").output().is_ok() {
            return Some(p);
        }
    }
    None
}

#[test]
fn oracle_scores_zero_misses_and_zero_wrong_callers_on_fixture() {
    let Some(py) = python() else {
        eprintln!("python not available; oracle gate skipped");
        return;
    };
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixture = repo.join("tests/oracle");
    let work = tempfile::tempdir().unwrap();
    let cases = work.path().join("cases.json");
    let derive = Command::new(py)
        .arg(repo.join("scripts/oracle/ast_callers.py"))
        .arg(&fixture)
        .args(["--output", cases.to_str().unwrap(), "--limit", "40"])
        .output()
        .unwrap();
    assert!(
        derive.status.success(),
        "{}",
        String::from_utf8_lossy(&derive.stderr)
    );
    let _ = std::fs::remove_dir_all(fixture.join(".flowspec"));
    let score = Command::new(py)
        .arg(repo.join("scripts/oracle/score_callers.py"))
        .args([
            "--manifest",
            cases.to_str().unwrap(),
            "--label",
            "gate",
            "--plain",
        ])
        .env("FLOWSPEC_BIN", env!("CARGO_BIN_EXE_flowspec"))
        .current_dir(work.path())
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(fixture.join(".flowspec"));
    assert!(
        score.status.success(),
        "{}",
        String::from_utf8_lossy(&score.stderr)
    );
    let report = std::fs::read_to_string(work.path().join("gate.md")).unwrap();
    let mut found = 0u32;
    let mut total = 0u32;
    let mut missed = 0u32;
    let mut wrong = 0u32;
    let mut failures = 0u32;
    for line in report.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 8 || !cells[1].starts_with('`') {
            continue;
        }
        let Some((f, t)) = cells[2].split_once('/') else {
            failures += 1;
            continue;
        };
        found += f.parse::<u32>().unwrap();
        total += t.parse::<u32>().unwrap();
        missed += cells[3].parse::<u32>().unwrap();
        wrong += cells[4].parse::<u32>().unwrap();
    }
    assert!(total > 0, "oracle derived no certain call sites:\n{report}");
    assert_eq!(failures, 0, "trace failed on some symbols:\n{report}");
    assert_eq!(
        missed, 0,
        "false negatives (missed certain call sites):\n{report}"
    );
    assert_eq!(
        wrong, 0,
        "false positives (wrong certain callers):\n{report}"
    );
    assert_eq!(found, total, "{report}");
}
