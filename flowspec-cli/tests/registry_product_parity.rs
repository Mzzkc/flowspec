use std::{path::Path, process::Command};
#[test]
fn oracle_product_name_parity_consumes_the_actual_packaged_registry() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let out = Command::new("python3")
        .arg(root.join("scripts/validate-oracle-artifacts.py"))
        .args([
            "--registry-only",
            "--flowspec-bin",
            env!("CARGO_BIN_EXE_flowspec"),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "actual product-name parity must pass: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("REGISTRY_PRODUCT_PARITY_PASS"));
}
