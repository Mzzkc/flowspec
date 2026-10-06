//! Canonical report identity/path primitives, independent of orchestration.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub(crate) fn path_key(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.as_os_str()
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub(crate) fn hash(domain: &str, value: &Value) -> String {
    // serde_json::Map uses key ordering without preserve_order; all generated
    // object keys are ASCII and values are integral, boolean, string, or null.
    let bytes = serde_json::to_vec(value).expect("canonical report value");
    let mut hasher = Sha256::new();
    hasher.update(format!("flowspec/{domain}/v1\0").as_bytes());
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}
