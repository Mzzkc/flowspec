//! The memory budget is a product feature: an over-budget analysis stops with a
//! clear error and a non-zero exit instead of exhausting the host.

use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn flowspec() -> Command {
    Command::cargo_bin("flowspec").unwrap()
}

/// A generated Python tree large enough to hold well over 1 MiB in the graph.
fn mid_sized_project() -> TempDir {
    generated_project(60)
}

/// `files` modules of twenty caller/callee pairs each.
///
/// The reference index keeps only the export table resident, so the tree
/// behind a `trace` must be several times larger than the graph needs
/// before 1 MiB is exceeded on every host; 60 files sits at the boundary.
fn generated_project(files: usize) -> TempDir {
    let dir = TempDir::new().unwrap();
    for n in 0..files {
        let mut src = String::new();
        for m in 0..20 {
            src.push_str(&format!("def f{n}_{m}(x):\n    return g{n}_{m}(x) + 1\n\ndef g{n}_{m}(x):\n    return x\n\n"));
        }
        fs::write(dir.path().join(format!("m{n}.py")), src).unwrap();
    }
    dir
}

#[test]
fn analysis_over_budget_stops_with_named_stage_and_exit_1() {
    let dir = mid_sized_project();
    let out = flowspec()
        .args(["analyze", "--memory-limit", "1M", "-f", "summary", "-q"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout={}",
        String::from_utf8_lossy(&out.stdout)
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("memory budget exceeded after"), "{err}");
    assert!(err.contains("--memory-limit"), "{err}");
}

#[test]
fn trace_over_budget_stops_the_same_way() {
    let dir = generated_project(400);
    let out = flowspec()
        .args([
            "trace",
            "--symbol",
            "g0_0",
            "--direction",
            "backward",
            "--memory-limit",
            "1M",
            "-f",
            "summary",
            "-q",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("memory budget exceeded after"));
}

#[test]
fn env_var_sets_the_budget_and_zero_disables_it() {
    let dir = mid_sized_project();
    let out = flowspec()
        .env("FLOWSPEC_MEMORY_LIMIT", "1M")
        .args(["analyze", "-f", "summary", "-q"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let out = flowspec()
        .args(["analyze", "--memory-limit", "0", "-f", "summary", "-q"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_ne!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_bad_size_is_a_config_error() {
    let out = flowspec()
        .args(["analyze", "--memory-limit", "plenty", "."])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a size"));
}

/// The dense caller lattice that exhausted memory on 2026-11-10 stays bounded.
#[test]
fn dense_method_lattice_fits_the_default_budget() {
    let dir = TempDir::new().unwrap();
    for n in 0..120 {
        fs::write(
            dir.path().join(format!("filler_{n}.py")),
            "def value():\n    return 1\n",
        )
        .unwrap();
    }
    for layer in 0..12 {
        let mut source = String::new();
        for branch in 0..3 {
            source.push_str(&format!(
                "class C{layer}_{branch}:\n    def step_{layer}(self, next):\n"
            ));
            if layer == 11 {
                source.push_str("        return 1\n\n");
            } else {
                source.push_str(&format!("        return next.step_{}()\n\n", layer + 1));
            }
        }
        fs::write(dir.path().join(format!("L{layer}.py")), source).unwrap();
    }
    let out = flowspec()
        .args(["analyze", "--memory-limit", "256M", "-f", "summary", "-q"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_ne!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = flowspec()
        .args([
            "trace",
            "--symbol",
            "C0_0::step_0",
            "--direction",
            "forward",
            "--memory-limit",
            "256M",
            "-f",
            "summary",
            "-q",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
