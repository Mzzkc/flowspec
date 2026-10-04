use std::process::Command;
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
fn main() {
    // PROFILE is Cargo's actual build profile, independent of rustc's
    // debug-assertions flag. Optimized release cannot enable the probe by
    // overriding that flag; ordinary release has no probe API at all.
    if std::env::var_os("CARGO_FEATURE_FAULT_PROBE").is_some()
        && std::env::var("PROFILE").as_deref() != Ok("debug")
    {
        panic!("fault_probe is test-only and requires the debug build profile");
    }
    if let Some(packed_refs) = git(&["rev-parse", "--git-path", "packed-refs"]) {
        println!("cargo:rerun-if-changed={packed_refs}");
    }
    if let Some(head_path) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head_path}");
    }
    // A worktree HEAD normally contains only `ref: refs/heads/<branch>`.
    // Watch the referenced file as well: its bytes change at every commit.
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(ref_path) = git(&["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={ref_path}");
        }
    }
    if let Some(sha) = git(&["rev-parse", "HEAD"]) {
        println!("cargo:rustc-env=FLOWSPEC_BUILD_SHA={sha}");
    }
    if let Some(tree) = git(&["rev-parse", "HEAD^{tree}"]) {
        println!("cargo:rustc-env=FLOWSPEC_SOURCE_TREE_SHA={tree}");
    }
}
