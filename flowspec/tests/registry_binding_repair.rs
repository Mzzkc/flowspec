use flowspec::{analyze, Config};
use std::{fs, path::PathBuf};

fn targets(import_path: &str, ambiguous: bool) -> (Vec<PathBuf>, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("package.json"), "{}").unwrap();
    fs::create_dir(dir.path().join("child")).unwrap();
    fs::write(dir.path().join("child/package.json"), "{}").unwrap();
    fs::write(dir.path().join("entry.js"), format!("import {{ target }} from '{import_path}'; export function run() {{ return target(); }}\n")).unwrap();
    let expected = dir.path().join("child/util.js");
    fs::write(&expected, "export function target() { return 2; }\n").unwrap();
    fs::write(
        dir.path().join("child/util.py"),
        "def target():\n    return 3\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("util.js"),
        "export function target() { return 4; }\n",
    )
    .unwrap();
    if ambiguous {
        fs::write(
            dir.path().join("child/util.ts"),
            "export function target() { return 5; }\n",
        )
        .unwrap();
    }
    let result = analyze(dir.path(), &Config::default(), &[]).unwrap();
    let import = result
        .graph
        .all_symbols()
        .find(|(_, s)| {
            s.location.file == dir.path().join("entry.js")
                && s.name == "target"
                && s.annotations.contains(&"import".into())
        })
        .unwrap()
        .0;
    let targets = result
        .graph
        .edges_from(import)
        .iter()
        .filter_map(|e| result.graph.get_symbol(e.target))
        .map(|s| s.location.file.clone())
        .collect();
    (targets, expected)
}

#[test]
fn extensionless_relative_import_crosses_package_marker() {
    let (actual, expected) = targets("./child/util", false);
    assert_eq!(
        actual,
        vec![expected],
        "genuine relative import must retain its exact target across a nested package marker"
    );
}

#[test]
fn explicit_relative_import_crosses_marker_without_foreign_rival() {
    let (actual, expected) = targets("./child/util.js", true);
    assert_eq!(actual, vec![expected]);
}

#[test]
fn extensionless_js_ts_rivals_remain_unresolved() {
    let (actual, _) = targets("./child/util", true);
    assert!(
        actual.is_empty(),
        "ambiguous same-stem candidates cannot acquire an arbitrary binding"
    );
}
