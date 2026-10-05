use flowspec::{analyze, diagnose_report, Config, ReportOptions};
use serde_json::{json, Value};

fn run(files: &[(&str, &str)], policy: Value) -> Value {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let config = Config {
        semantic_policy: Some(serde_json::from_value(policy).unwrap()),
        ..Config::default()
    };
    let a = analyze(dir.path(), &config, &[]).unwrap().manifest;
    let d = diagnose_report(dir.path(), &config, &[], &ReportOptions::default()).unwrap();
    assert_eq!(a.authority, d.authority, "ordinary public paths must agree");
    let authority = a.authority.unwrap();
    assert_eq!(authority["pattern_state"].as_array().unwrap().len(), 13);
    assert_eq!(
        authority["findings"]["by_pattern"]
            .as_object()
            .unwrap()
            .len(),
        13
    );
    eprintln!("MODULE_POLICY_PRODUCER {}", authority["producer"]);
    authority
}
fn state(a: &Value, pattern: &str) -> String {
    a["pattern_state"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pattern_id"] == pattern)
        .unwrap()["cells"][0]["state"]
        .as_str()
        .unwrap()
        .into()
}
fn findings<'a>(a: &'a Value, p: &str) -> Vec<&'a Value> {
    a["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["pattern_id"] == p)
        .collect()
}
fn languages() -> Vec<(
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
)> {
    vec![
        (
            "py",
            "def old():\n    return 1\ndef new():\n    return 2\n",
            "def consumer():\n    return old() + new()\n",
            "def consumer():\n    return new()\n",
            "def api():\n    return 1\n",
        ),
        (
            "js",
            "export function old(){return 1;}\nexport function new(){return 2;}\n",
            "export function consumer(){return old() + new();}\n",
            "export function consumer(){return new();}\n",
            "export function api(){return 1;}\n",
        ),
        (
            "ts",
            "export function old():number{return 1;}\nexport function new():number{return 2;}\n",
            "export function consumer():number{return old() + new();}\n",
            "export function consumer():number{return new();}\n",
            "export function api():number{return 1;}\n",
        ),
        (
            "rs",
            "pub fn old()->i32{1}\npub fn new()->i32{2}\n",
            "pub fn consumer()->i32{old() + new()}\n",
            "pub fn consumer()->i32{new()}\n",
            "pub fn api()->i32{1}\n",
        ),
    ]
}
#[test]
fn declared_migration_requires_exact_live_old_binding_in_four_languages() {
    for (ext, declarations, live, satisfied, _) in languages() {
        let name = format!("subject.{ext}");
        let policy = json!({"version":1,"migrations":[{"old":format!("{name}::old"),"new":format!("{name}::new")}]});
        let source = format!("{declarations}{live}");
        let a = run(&[(&name, &source)], policy.clone());
        assert_eq!(findings(&a, "incomplete_migration").len(), 1, "{ext}: {a}");
        let f = findings(&a, "incomplete_migration")[0];
        assert_eq!(f["subject"]["name"], "old");
        assert!(
            f["range"]["end_byte"].as_u64().unwrap() > f["range"]["start_byte"].as_u64().unwrap()
        );
        assert_eq!(
            state(&a, "incomplete_migration"),
            "complete",
            "{ext}: {}",
            a["losses"]
        );
        let source = format!("{declarations}{satisfied}");
        let clean = run(&[(&name, &source)], policy.clone());
        assert!(findings(&clean, "incomplete_migration").is_empty());
        assert_eq!(state(&clean, "incomplete_migration"), "complete");
        let unknown = run(
            &[(&name, &source)],
            json!({"version":1,"migrations":[{"old":format!("{name}::absent"),"new":format!("{name}::new") }]}),
        );
        assert!(findings(&unknown, "incomplete_migration").is_empty());
        assert_eq!(state(&unknown, "incomplete_migration"), "incomplete");
    }
}
#[test]
fn layer_rules_match_real_module_edges_in_four_languages() {
    for (ext, _, _, _, api) in languages() {
        let provider = format!("child.{ext}");
        let caller = format!("parent.{ext}");
        let source = match ext {
            "py" => "from child import api\ndef consumer():\n    return api()\n",
            "rs" => "use crate::child::api;\npub fn consumer()->i32{api()}\n",
            _ => "import {api} from './child';\nexport function consumer(){return api();}\n",
        };
        let policy = json!({"version":1,"layers":[{"from":caller,"to":provider}]});
        let a = run(&[(&provider, api), (&caller, source)], policy);
        assert_eq!(findings(&a, "layer_violation").len(), 1, "{ext}: {a}");
        assert_eq!(
            state(&a, "layer_violation"),
            "complete",
            "{ext}: {}",
            a["losses"]
        );
        let rival = run(
            &[(&provider, api), (&caller, source)],
            json!({"version":1,"layers":[{"from":provider,"to":caller}]}),
        );
        assert!(findings(&rival, "layer_violation").is_empty());
        assert_eq!(state(&rival, "layer_violation"), "complete");
    }
}
#[test]
fn public_surface_absence_and_actual_alias_star_exports_in_four_languages() {
    for (ext, _, _, _, api) in languages() {
        let child = format!("child.{ext}");
        let parent = format!("parent.{ext}");
        let policy =
            json!({"version":1,"exports":[{"module":parent,"symbols":[format!("{child}::api")]}]});
        let empty = if ext == "py" {
            "# public surface\n"
        } else {
            "// public surface\n"
        };
        let a = run(&[(&child, api), (&parent, empty)], policy.clone());
        assert_eq!(findings(&a, "missing_reexport").len(), 1, "{ext}: {a}");
        assert_eq!(
            state(&a, "missing_reexport"),
            "complete",
            "{ext}: {}",
            a["losses"]
        );
        for exported in match ext {
            "py" => vec![
                "from child import api as exposed\n__all__ = ['exposed']\n",
                "from child import *\n",
            ],
            "rs" => vec![
                "pub use crate::child::api as exposed;\n",
                "pub use crate::child::*;\n",
            ],
            _ => vec![
                "export {api as exposed} from './child';\n",
                "export * from './child';\n",
            ],
        } {
            let clean = run(&[(&child, api), (&parent, exported)], policy.clone());
            assert!(
                findings(&clean, "missing_reexport").is_empty(),
                "{ext}: {clean}"
            );
            assert_eq!(
                state(&clean, "missing_reexport"),
                "complete",
                "{ext}: {}",
                clean["losses"]
            );
        }
    }
}
