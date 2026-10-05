use flowspec::{analyze_with_options, cache::AnalysisMode, AnalysisOptions, Config};
use serde_json::Value;
fn normal(mut v: Value) -> Value {
    v["metadata"]["analyzed_at"] = "clock".into();
    v["metadata"]["incremental"] = false.into();
    v["metadata"]["files_changed"] = 0.into();
    v
}
#[test]
fn actual_native_report_uncertainty_survives_warm_corruption_and_scope_changes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.ts");
    std::fs::write(&path,"type Choice<T> = T extends string ? T : never;\nfunction _unused(): number { return 1; }\n").unwrap();
    let run = |config: &Config, mode| {
        analyze_with_options(
            root.path(),
            config,
            &[],
            &AnalysisOptions {
                mode,
                ..Default::default()
            },
        )
        .unwrap()
    };
    let config = Config::default();
    let full = run(&config, AnalysisMode::Full);
    let warm = run(&config, AnalysisMode::Incremental);
    assert_eq!(warm.cache_stats.hits, 1);
    assert_eq!(warm.cache_stats.parsed, 0);
    assert_eq!(
        normal(serde_json::to_value(&full.manifest).unwrap()),
        normal(serde_json::to_value(&warm.manifest).unwrap())
    );
    assert_eq!(
        warm.manifest.execution,
        Some(flowspec::Execution::Incomplete)
    );
    for entry in std::fs::read_dir(root.path().join(".flowspec/cache/ir-v1")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "ir") {
            std::fs::write(path, b"corrupt framing").unwrap();
        }
    }
    let fallback = run(&config, AnalysisMode::Incremental);
    assert_eq!(fallback.cache_stats.parsed, 1);
    assert!(!fallback.cache_stats.fallbacks.is_empty());
    assert_eq!(
        normal(serde_json::to_value(&full.manifest).unwrap()),
        normal(serde_json::to_value(&fallback.manifest).unwrap())
    );
    let previous = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, "function _changed(): number { return 2; }\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(previous))
        .unwrap();
    let changed = run(&config, AnalysisMode::Incremental);
    assert_eq!(changed.cache_stats.hits, 0);
    let changed_full = run(&config, AnalysisMode::Full);
    assert_eq!(
        normal(serde_json::to_value(changed.manifest).unwrap()),
        normal(serde_json::to_value(changed_full.manifest).unwrap())
    );
    std::fs::rename(&path, root.path().join("renamed.ts")).unwrap();
    let renamed = run(&config, AnalysisMode::Incremental);
    assert_eq!(renamed.cache_stats.hits, 0);
    std::fs::remove_file(root.path().join("renamed.ts")).unwrap();
    assert_eq!(
        run(&config, AnalysisMode::Incremental)
            .graph
            .all_symbols()
            .count(),
        0
    );
}
