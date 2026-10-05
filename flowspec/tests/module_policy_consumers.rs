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
    if let Some(output) = std::env::var_os("FLOWSPEC_MODULE_CONTROL_REPORT_DIR") {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let output = std::path::PathBuf::from(output);
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join(format!("control-{number:03}.json")), serde_json::to_vec_pretty(&json!({"authority":authority,"source_files":files,"semantic_policy":config.semantic_policy})).unwrap()).unwrap();
    }
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
            "def old():\n    return 1\ndef upgraded():\n    return 2\n",
            "def consumer():\n    return old() + upgraded()\n",
            "def consumer():\n    return upgraded()\n",
            "def api():\n    return 1\n",
        ),
        (
            "js",
            "export function old(){return 1;}\nexport function upgraded(){return 2;}\n",
            "export function consumer(){return old() + upgraded();}\n",
            "export function consumer(){return upgraded();}\n",
            "export function api(){return 1;}\n",
        ),
        (
            "ts",
            "export function old():number{return 1;}\nexport function upgraded():number{return 2;}\n",
            "export function consumer():number{return old() + upgraded();}\n",
            "export function consumer():number{return upgraded();}\n",
            "export function api():number{return 1;}\n",
        ),
        (
            "rs",
            "pub fn old()->i32{1}\npub fn upgraded()->i32{2}\n",
            "pub fn consumer()->i32{old() + upgraded()}\n",
            "pub fn consumer()->i32{upgraded()}\n",
            "pub fn api()->i32{1}\n",
        ),
    ]
}
#[test]
fn declared_migration_requires_exact_live_old_binding_in_four_languages() {
    for (ext, declarations, live, satisfied, _) in languages() {
        let name = format!("subject.{ext}");
        let policy = json!({"version":1,"migrations":[{"old":format!("{name}::old"),"new":format!("{name}::upgraded")}]});
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
            json!({"version":1,"migrations":[{"old":format!("{name}::absent"),"new":format!("{name}::upgraded") }]}),
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

#[test]
fn migration_shadowed_names_unused_and_unrelated_declarations_are_clean() {
    for (ext, declarations, _, _, _) in languages() {
        let path = format!("subject.{ext}");
        let shadow = match ext {
            "py" => "def consumer(old):\n    return old()\n",
            "rs" => "pub fn consumer(old:fn()->i32)->i32{old()}\n",
            _ => "export function consumer(old){return old();}\n",
        };
        let source = format!("{declarations}{shadow}");
        let policy = json!({"version":1,"migrations":[{"old":format!("{path}::old"),"new":format!("{path}::upgraded")}]});
        let a = run(&[(&path, &source)], policy.clone());
        assert!(
            findings(&a, "incomplete_migration").is_empty(),
            "{ext}: shadow must not target declaration: {a}"
        );
        assert_eq!(state(&a, "incomplete_migration"), "complete", "{ext}");
        let a = run(&[(&path, declarations)], policy);
        assert!(findings(&a, "incomplete_migration").is_empty());
        assert_eq!(state(&a, "incomplete_migration"), "complete");
    }
}
#[test]
fn migration_multifile_alias_live_call_has_exact_call_span() {
    for (ext, declarations, _, _, _) in languages() {
        let child = format!("child.{ext}");
        let parent = format!("parent.{ext}");
        let source=match ext {"py"=>"from child import old as legacy\ndef consumer():\n    return legacy()\n", "rs"=>"use crate::child::old as legacy;\npub fn consumer()->i32{legacy()}\n", _=>"import {old as legacy} from './child';\nexport function consumer(){return legacy();}\n"};
        let a = run(
            &[(&child, declarations), (&parent, source)],
            json!({"version":1,"migrations":[{"old":format!("{child}::old"),"new":format!("{child}::upgraded") }]}),
        );
        let f = findings(&a, "incomplete_migration");
        assert_eq!(f.len(), 1, "{ext}: {a}");
        let consumers = f[0]["evidence"][0]["live_old_consumers"]
            .as_array()
            .unwrap();
        assert_eq!(consumers.len(), 1, "{ext}: {consumers:?}");
        let r = &consumers[0]["range"];
        let start = r["start_byte"].as_u64().unwrap() as usize;
        let end = r["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(&source[start..end], "legacy()", "{ext}");
        assert_eq!(
            state(&a, "incomplete_migration"),
            "complete",
            "{ext}: {}",
            a["losses"]
        );
    }
}
#[test]
fn export_unknown_dynamic_private_and_unrelated_binding_rivals_fail_closed() {
    for (ext, _, _, _, api) in languages() {
        let child = format!("child.{ext}");
        let parent = format!("parent.{ext}");
        let unknown = json!({"version":1,"exports":[{"module":parent,"symbols":[format!("{child}::absent")]}]});
        let empty = if ext == "py" {
            "# surface\n"
        } else {
            "// surface\n"
        };
        let a = run(&[(&child, api), (&parent, empty)], unknown);
        assert!(findings(&a, "missing_reexport").is_empty());
        assert_eq!(state(&a, "missing_reexport"), "incomplete");
        let private = match ext {
            "py" => "def _private():\n    return 1\n",
            "rs" => "fn _private()->i32{1}\n",
            _ => "function _private(){return 1;}\n",
        };
        let a = run(
            &[(&child, private), (&parent, empty)],
            json!({"version":1,"exports":[{"module":parent,"symbols":[format!("{child}::_private")]}]}),
        );
        assert!(findings(&a, "missing_reexport").is_empty());
        assert_eq!(state(&a, "missing_reexport"), "incomplete");
        let a = run(
            &[(&child, api), (&parent, api)],
            json!({"version":1,"exports":[{"module":parent,"symbols":[format!("{child}::api")]}]}),
        );
        assert_eq!(
            findings(&a, "missing_reexport").len(),
            1,
            "{ext}: unrelated same spelling is not child export"
        );
    }
    let a = run(
        &[
            ("child.py", "def api():\n    return 1\n"),
            (
                "parent.py",
                "from child import api\n__all__ = build_surface()\n",
            ),
        ],
        json!({"version":1,"exports":[{"module":"parent.py","symbols":["child.py::api"]}]}),
    );
    assert!(findings(&a, "missing_reexport").is_empty());
    assert_eq!(state(&a, "missing_reexport"), "incomplete");
}
