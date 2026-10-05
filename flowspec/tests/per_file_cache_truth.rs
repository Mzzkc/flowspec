use flowspec::{
    cache::{CacheKey, CacheStore, ParsedFile},
    parser::ir::ParseResult,
};
use serde_json::json;
fn output(key: &CacheKey) -> ParsedFile {
    ParsedFile {
        ir: ParseResult::default(),
        report: json!({"schema_version":1,"source_sha256":key.source_sha256,"source_bytes":key.source_bytes,"grammar_version":"test-grammar-v1","adapter_version":"test-adapter-v1","capabilities":{"types":"unknown"},"events":[{"code":"unsupported_syntax","detail":"unknown is retained"}]}),
    }
}
#[test]
fn cache_roundtrip_retains_unknowns_and_all_identity_inputs_invalidate() {
    let dir = tempfile::tempdir().unwrap();
    let store = CacheStore::open(dir.path()).unwrap();
    let base = json!({"schema":2,"registry":"r1","grammar":"g1","adapter":"a1","config":"c1","scope":"s1","path":"a.py","options":{"depth":256},"tool":"t1"});
    let key = CacheKey::new(&base, b"source");
    let expected = output(&key);
    store.store(&key, expected.clone()).unwrap();
    assert_eq!(store.load(&key).unwrap().unwrap().report, expected.report);
    for field in [
        "schema", "registry", "grammar", "adapter", "config", "scope", "path", "options", "tool",
    ] {
        let mut changed = base.clone();
        changed[field] = json!("changed");
        assert!(store
            .load(&CacheKey::new(&changed, b"source"))
            .unwrap()
            .is_none());
    }
    assert!(store
        .load(&CacheKey::new(&base, b"edited"))
        .unwrap()
        .is_none());
}
#[test]
fn corrupt_checksum_oversize_and_legacy_output_are_never_hits() {
    let dir = tempfile::tempdir().unwrap();
    let store = CacheStore::open(dir.path()).unwrap();
    let key = CacheKey::new(&json!({}), b"source");
    let mut legacy = output(&key);
    legacy.report["schema_version"] = json!(0);
    assert!(store.store(&key, legacy).is_err());
    store.store(&key, output(&key)).unwrap();
    let path = dir.path().join(format!("{}.ir", key.digest));
    let mut bytes = std::fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&path, bytes).unwrap();
    assert!(store.load(&key).is_err());
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(9 * 1024 * 1024).unwrap();
    assert!(store.load(&key).is_err());
}
#[test]
fn concurrent_writers_publish_only_whole_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let key = CacheKey::new(&json!({}), b"source");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let results = std::thread::scope(|scope| {
        let mut jobs = vec![];
        for _ in 0..4 {
            let path = dir.path();
            let key = &key;
            let barrier = barrier.clone();
            jobs.push(scope.spawn(move || {
                let cache = CacheStore::open(path).unwrap();
                barrier.wait();
                cache.store(key, output(key))
            }));
        }
        jobs.into_iter()
            .map(|j| j.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(results.iter().any(|r| r.is_ok()));
    assert_eq!(
        CacheStore::open(dir.path())
            .unwrap()
            .load(&key)
            .unwrap()
            .unwrap()
            .report,
        output(&key).report
    );
}
#[test]
fn symlink_cache_directory_and_blob_are_refused() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), dir.path().join("cache")).unwrap();
    assert!(CacheStore::open(&dir.path().join("cache")).is_err());
    let cache = CacheStore::open(&dir.path().join("regular")).unwrap();
    let key = CacheKey::new(&json!({}), b"source");
    std::fs::write(outside.path().join("blob"), b"anything").unwrap();
    symlink(
        outside.path().join("blob"),
        dir.path()
            .join("regular")
            .join(format!("{}.ir", key.digest)),
    )
    .unwrap();
    assert!(cache.load(&key).is_err());
}
