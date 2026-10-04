use std::process::Command;
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
fn main() {
    if let Some(head_path) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head_path}");
    }
    if let Some(sha) = git(&["rev-parse", "HEAD"]) {
        println!("cargo:rustc-env=FLOWSPEC_BUILD_SHA={sha}");
    }
    if let Some(tree) = git(&["rev-parse", "HEAD^{tree}"]) {
        println!("cargo:rustc-env=FLOWSPEC_SOURCE_TREE_SHA={tree}");
    }
}
