//! Normal library driver proof with an injected partial-capability reporting adapter.
//! This does not qualify built-in native adapters; their principal join is a separate gate.
use flowspec::parser::LanguageAdapter;
use flowspec::registry::LanguageDescriptor;
use flowspec::{
    analyze_with_options,
    cache::{AnalysisMode, ParsedFile},
    AnalysisOptions, Config,
};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};
static PARSES: AtomicUsize = AtomicUsize::new(0);
fn structured(
    adapter: &dyn LanguageAdapter,
    path: &Path,
    source: &str,
    descriptor: &LanguageDescriptor,
) -> Result<ParsedFile, flowspec::FlowspecError> {
    PARSES.fetch_add(1, Ordering::SeqCst);
    let key = flowspec::cache::CacheKey::new(&json!({}), source.as_bytes());
    // This provider deliberately certifies no diagnostic prerequisites. Unknowns must survive hits.
    Ok(ParsedFile {
        ir: adapter.parse_file(path, source)?,
        report: json!({"schema_version":1,"file":path,"source_bytes":source.len(),"source_sha256":key.source_sha256,"language":descriptor.id,"adapter_version":descriptor.adapter_version,"grammar_version":descriptor.grammar_version,"capabilities":{},"events":[]}),
    })
}
fn differences(a: &Value, b: &Value, path: &str, out: &mut Vec<String>) {
    if a == b {
        return;
    }
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in a {
                differences(
                    v,
                    b.get(k).unwrap_or(&Value::Null),
                    &format!("{path}/{k}"),
                    out,
                );
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                differences(a, b, &format!("{path}/{i}"), out);
            }
        }
        _ => out.push(format!("{path}: {a} != {b}")),
    }
}
fn equal(a: Value, b: Value) {
    let mut diff = vec![];
    differences(&a, &b, "", &mut diff);
    assert!(a == b, "normalized equality drift: {diff:?}");
}
fn normal(report: Value) -> Value {
    // Compare the complete public value, not its encoded record-node numbering.
    let mut report = flowspec::commands::expand_report_records(&report).unwrap();
    let authority = &mut report["authority"];
    assert_eq!(authority.as_object().unwrap().len(), 17);
    // Independently check all fourteen result-determining fields before normalization.
    use sha2::Digest;
    let fields = ["schema_version", "producer_contract", "producer", "execution", "outcome", "findings_present", "gate", "findings", "scope", "provenance", "inventory", "losses", "pattern_state", "analysis_graph"];
    let body: serde_json::Map<String, Value> = fields.iter().map(|key| ((*key).into(), authority[*key].clone())).collect();
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"flowspec/report/v1\0");
    hasher.update(serde_json::to_vec(&Value::Object(body)).unwrap());
    assert_eq!(authority["report_identity"], json!(format!("sha256:{:x}", hasher.finalize())));
    assert_eq!(authority["execution"], "incomplete");
    assert!(matches!(authority["provenance"]["cache"]["executed_mode"].as_str(), Some("full" | "incremental")));
    // Physical cache execution is independently checked in the driver below.
    // Only that observation and its identity are excluded from semantic equivalence.
    authority["provenance"]["cache"]["executed_mode"] = json!("normalized-execution");
    authority.as_object_mut().unwrap().remove("report_identity");
    report["metadata"]["analyzed_at"] = json!("clock");
    report["metadata"]["incremental"] = json!(false);
    report["metadata"]["files_changed"] = json!(0);
    report
}
#[test]
fn ordinary_library_full_warm_edit_same_timestamp_rename_delete_and_policy_equivalence() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("app.py");
    std::fs::write(&file, "def _first():\n    return 1\n").unwrap();
    let options = AnalysisOptions {
        mode: AnalysisMode::Incremental,
        structured_parser: Some(structured),
        structured_parser_identity: Some("partial-capability-test-provider/v1"),
        cache_dir: None,
    };
    let full_options = AnalysisOptions {
        mode: AnalysisMode::Full,
        ..options.clone()
    };
    let compare = |config: &Config| {
        let full = analyze_with_options(dir.path(), config, &[], &full_options).unwrap();
        assert_eq!(full.manifest.authority.as_ref().unwrap()["provenance"]["cache"]["executed_mode"], "full");
        let before = PARSES.load(Ordering::SeqCst);
        let warm = analyze_with_options(dir.path(), config, &[], &options).unwrap();
        assert_eq!(
            PARSES.load(Ordering::SeqCst),
            before,
            "a cache hit must actually bypass parsing"
        );
        assert_eq!(warm.manifest.authority.as_ref().unwrap()["provenance"]["cache"]["executed_mode"], "incremental");
        equal(
            normal(serde_json::to_value(&full.manifest).unwrap()),
            normal(serde_json::to_value(&warm.manifest).unwrap()),
        );
        assert_eq!(
            warm.manifest.execution,
            Some(flowspec::Execution::Incomplete),
            "cached capability debt never becomes complete"
        );
        assert!(warm.cache_stats.fallbacks.is_empty());
        warm
    };
    let config = Config::default();
    let warm = compare(&config);
    assert_eq!(warm.cache_stats.hits, 1);
    let old_modified = std::fs::metadata(&file).unwrap().modified().unwrap();
    std::fs::write(&file, "def _other():\n    return 2\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old_modified))
        .unwrap();
    let changed = analyze_with_options(dir.path(), &config, &[], &options).unwrap();
    assert_eq!(changed.cache_stats.hits, 0);
    assert_eq!(changed.cache_stats.parsed, 1);
    compare(&config);
    std::fs::rename(&file, dir.path().join("renamed.py")).unwrap();
    let renamed = analyze_with_options(dir.path(), &config, &[], &options).unwrap();
    assert_eq!(renamed.cache_stats.hits, 0);
    compare(&config);
    std::fs::write(dir.path().join("added.py"), "def _added():\n    return 3\n").unwrap();
    compare(&config);
    let mut changed_policy = config.clone();
    changed_policy.exclude = vec!["added.py".into()];
    compare(&changed_policy);
    std::fs::remove_file(dir.path().join("added.py")).unwrap();
    compare(&config);
    let blobs: Vec<_> = std::fs::read_dir(dir.path().join(".flowspec/cache/ir-v1"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|s| s == "ir"))
        .collect();
    for blob in blobs {
        std::fs::write(blob, b"corrupt").unwrap();
    }
    let rebuilt = analyze_with_options(dir.path(), &config, &[], &options).unwrap();
    assert_eq!(rebuilt.cache_stats.hits, 0);
    assert!(!rebuilt.cache_stats.fallbacks.is_empty());
    let full = analyze_with_options(dir.path(), &config, &[], &full_options).unwrap();
    assert_eq!(
        normal(serde_json::to_value(&full.manifest).unwrap()),
        normal(serde_json::to_value(&rebuilt.manifest).unwrap())
    );
    let registry = flowspec::registry::Registry::default();
    let mut languages = registry.languages().to_vec();
    languages
        .iter_mut()
        .find(|l| l.id == "python")
        .unwrap()
        .adapter_version
        .push_str("/changed");
    let revised =
        flowspec::registry::Registry::new(languages, registry.patterns().to_vec()).unwrap();
    let invalidated =
        flowspec::analyze_with_registry_and_options(dir.path(), &config, &[], &revised, &options)
            .unwrap();
    assert_eq!(
        invalidated.cache_stats.hits, 0,
        "registry/adapter provenance changes require fresh IR"
    );
    let revised_warm =
        flowspec::analyze_with_registry_and_options(dir.path(), &config, &[], &revised, &options)
            .unwrap();
    assert_eq!(revised_warm.cache_stats.hits, 1);
    let concurrent = std::thread::scope(|scope| {
        let mut jobs = vec![];
        for _ in 0..4 {
            jobs.push(
                scope.spawn(|| analyze_with_options(dir.path(), &config, &[], &options).unwrap()),
            );
        }
        jobs.into_iter()
            .map(|j| j.join().unwrap())
            .collect::<Vec<_>>()
    });
    for report in concurrent {
        equal(
            normal(serde_json::to_value(&full.manifest).unwrap()),
            normal(serde_json::to_value(&report.manifest).unwrap()),
        );
    }
}
