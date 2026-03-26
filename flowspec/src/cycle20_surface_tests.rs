// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Cycle 20 QA-3 (Surface) tests — config file exclusion wiring, `.gitignore`
//! integration, language filtering from config, and full pipeline integration.
//!
//! These tests prove that `discover_source_files()` respects all three exclusion
//! sources (hardcoded skip_dirs, config exclude patterns, .gitignore) and that
//! config languages flow through the analysis pipeline correctly.

use crate::config::Config;

/// Helper: call discover_source_files (private) through analyze() by examining results.
/// For unit tests that need direct access to discover_source_files, we test through
/// the public analyze() API or reconstruct the discovery logic in tests.

// ===========================================================================
// Category 2: File Exclusion — discover_source_files Respects Config (T11–T19)
// ===========================================================================

#[test]
fn t11_config_exclude_patterns_skip_directories() {
    let tmp = tempfile::tempdir().unwrap();

    // Create dirs: src/, archive/, vendor/
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::create_dir_all(tmp.path().join("archive")).unwrap();
    std::fs::create_dir_all(tmp.path().join("vendor")).unwrap();

    // Create source files
    std::fs::write(tmp.path().join("src/main.py"), "x = 1\n").unwrap();
    std::fs::write(tmp.path().join("archive/old.py"), "y = 2\n").unwrap();
    std::fs::write(tmp.path().join("vendor/dep.py"), "z = 3\n").unwrap();

    // Config: exclude archive/ and vendor/
    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(
        flowspec_dir.join("config.yaml"),
        "exclude:\n  - \"archive/\"\n  - \"vendor/\"\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    // main.py should produce entities, archive/old.py and vendor/dep.py should not
    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "src/main.py must be analyzed"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("old.py")),
        "archive/old.py must be excluded by config"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("dep.py")),
        "vendor/dep.py must be excluded by config"
    );
}

#[test]
fn t12_hardcoded_skip_dirs_survive_with_config() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join("target")).unwrap();
    std::fs::create_dir_all(tmp.path().join("archive")).unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();

    std::fs::write(tmp.path().join("target/gen.rs"), "fn gen() {}\n").unwrap();
    std::fs::write(tmp.path().join("archive/old.py"), "y = 2\n").unwrap();
    std::fs::write(tmp.path().join("src/main.py"), "x = 1\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(
        flowspec_dir.join("config.yaml"),
        "exclude:\n  - \"archive/\"\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "src/main.py must be analyzed"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("gen.rs")),
        "target/ must still be skipped (hardcoded)"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("old.py")),
        "archive/ must be skipped (config)"
    );
}

#[test]
fn t13_hardcoded_skip_dirs_work_without_config() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join("target")).unwrap();
    std::fs::create_dir_all(tmp.path().join("node_modules")).unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();

    std::fs::write(tmp.path().join("target/gen.rs"), "fn gen() {}\n").unwrap();
    std::fs::write(
        tmp.path().join("node_modules/pkg.js"),
        "function pkg() {}\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("src/main.py"), "x = 1\n").unwrap();

    // No config file — empty exclude
    let config = Config::load(tmp.path(), None).unwrap();
    assert!(config.exclude.is_empty());

    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "src/main.py must be analyzed"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("gen.rs")),
        "target/ must be skipped (hardcoded) even without config"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("pkg.js")),
        "node_modules/ must be skipped (hardcoded) even without config"
    );
}

#[test]
fn t14_glob_pattern_exclude_matching() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join("tests")).unwrap();
    std::fs::create_dir_all(tmp.path().join("test_data")).unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();

    std::fs::write(tmp.path().join("tests/test_main.py"), "def test(): pass\n").unwrap();
    std::fs::write(tmp.path().join("test_data/data.py"), "x = 1\n").unwrap();
    std::fs::write(tmp.path().join("src/main.py"), "y = 2\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(
        flowspec_dir.join("config.yaml"),
        "exclude:\n  - \"test*\"\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "src/main.py must be analyzed"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("test_main.py")),
        "tests/ must be excluded by glob pattern test*"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("data.py")),
        "test_data/ must be excluded by glob pattern test*"
    );
}

#[test]
fn t15_nested_directory_exclusion() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join("src/generated")).unwrap();
    std::fs::create_dir_all(tmp.path().join("src/main")).unwrap();

    std::fs::write(
        tmp.path().join("src/generated/auto.py"),
        "def auto(): pass\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("src/main/app.py"), "def app(): pass\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(
        flowspec_dir.join("config.yaml"),
        "exclude:\n  - \"generated/\"\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("app.py")),
        "src/main/app.py must be analyzed"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("auto.py")),
        "src/generated/auto.py must be excluded by 'generated/' pattern"
    );
}

#[test]
fn t16_empty_exclude_list() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::create_dir_all(tmp.path().join("target")).unwrap();

    std::fs::write(tmp.path().join("src/main.py"), "x = 1\n").unwrap();
    std::fs::write(tmp.path().join("target/gen.rs"), "fn gen() {}\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(flowspec_dir.join("config.yaml"), "exclude: []\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    assert!(config.exclude.is_empty());

    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "src/main.py must be analyzed"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("gen.rs")),
        "target/ still skipped by hardcoded dirs with empty exclude list"
    );
}

#[test]
fn t17_duplicate_exclude_entries() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join("archive")).unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();

    std::fs::write(tmp.path().join("archive/old.py"), "x = 1\n").unwrap();
    std::fs::write(tmp.path().join("src/main.py"), "y = 2\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(
        flowspec_dir.join("config.yaml"),
        "exclude:\n  - \"target/\"\n  - \"target/\"\n  - \"archive/\"\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        !entity_files.iter().any(|f| f.contains("old.py")),
        "archive/ excluded despite duplicate target/ entries"
    );
}

#[test]
fn t18_exclude_file_patterns() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join("src")).unwrap();

    std::fs::write(tmp.path().join("src/main.py"), "def main(): pass\n").unwrap();
    std::fs::write(tmp.path().join("src/conftest.py"), "def conftest(): pass\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(
        flowspec_dir.join("config.yaml"),
        "exclude:\n  - \"conftest.py\"\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "src/main.py must be analyzed"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("conftest.py")),
        "conftest.py must be excluded by file pattern"
    );
}

#[test]
fn t19_exclude_does_not_affect_config_discovery() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/main.py"), "x = 1\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(
        flowspec_dir.join("config.yaml"),
        "exclude:\n  - \".flowspec/\"\nlanguages:\n  - python\n",
    )
    .unwrap();

    // Config loads successfully even though .flowspec/ is in exclude
    let config = Config::load(tmp.path(), None).unwrap();
    assert_eq!(config.languages, vec!["python".to_string()]);
    assert!(config.exclude.contains(&".flowspec/".to_string()));

    // Analysis still works
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();
    assert!(
        !result.manifest.entities.is_empty(),
        "Analysis should produce entities from src/"
    );
}

// ===========================================================================
// Category 3: .gitignore Integration (T20–T28)
// ===========================================================================

#[test]
fn t20_gitignore_excludes_directories() {
    let tmp = tempfile::tempdir().unwrap();

    // Create minimal git repo structure
    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    std::fs::write(tmp.path().join(".gitignore"), "archive/\n").unwrap();

    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::create_dir_all(tmp.path().join("archive")).unwrap();

    std::fs::write(tmp.path().join("src/main.py"), "def main(): pass\n").unwrap();
    std::fs::write(tmp.path().join("archive/old.py"), "def old(): pass\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "Non-gitignored files must be found"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("old.py")),
        "Gitignored files must NOT be found"
    );
}

#[test]
fn t21_no_gitignore_no_crash() {
    let tmp = tempfile::tempdir().unwrap();

    // No .gitignore, no .git
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/main.py"), "x = 1\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]);

    assert!(result.is_ok(), "Missing .gitignore must not cause errors");
    let result = result.unwrap();
    assert!(
        !result.manifest.entities.is_empty(),
        "Files should still be discovered without .gitignore"
    );
}

#[test]
fn t22_no_git_directory_no_crash() {
    let tmp = tempfile::tempdir().unwrap();

    // .gitignore present but NO .git/
    std::fs::write(tmp.path().join(".gitignore"), "archive/\n").unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/main.py"), "x = 1\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]);

    assert!(result.is_ok(), "Non-git project must not crash");
}

#[test]
fn t23_nested_gitignore_files_respected() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    std::fs::write(tmp.path().join(".gitignore"), "*.log\n").unwrap();

    std::fs::create_dir_all(tmp.path().join("lib")).unwrap();
    std::fs::write(tmp.path().join("lib/.gitignore"), "generated/\n").unwrap();

    std::fs::create_dir_all(tmp.path().join("lib/generated")).unwrap();
    std::fs::write(tmp.path().join("lib/core.py"), "def core(): pass\n").unwrap();
    std::fs::write(
        tmp.path().join("lib/generated/auto.py"),
        "def auto(): pass\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("core.py")),
        "lib/core.py must be found"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("auto.py")),
        "lib/generated/auto.py must be excluded by nested .gitignore"
    );
}

#[test]
fn t24_gitignore_negation_patterns() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    // Git behavior: to negate a file inside an ignored directory, you must
    // first un-ignore the directory with "archive/*" (not "archive/") and then
    // negate the specific file. "archive/" ignores the directory itself, making
    // negation of contents impossible.
    std::fs::write(
        tmp.path().join(".gitignore"),
        "archive/*\n!archive/important.py\n",
    )
    .unwrap();

    std::fs::create_dir_all(tmp.path().join("archive")).unwrap();
    std::fs::write(tmp.path().join("archive/junk.py"), "def junk(): pass\n").unwrap();
    std::fs::write(
        tmp.path().join("archive/important.py"),
        "def important(): pass\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("important.py")),
        "archive/important.py must be found (gitignore negation !)"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("junk.py")),
        "archive/junk.py must be excluded by gitignore"
    );
}

#[test]
fn t25_config_exclude_and_gitignore_combined() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    std::fs::write(tmp.path().join(".gitignore"), "logs/\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(
        flowspec_dir.join("config.yaml"),
        "exclude:\n  - \"vendor/\"\n",
    )
    .unwrap();

    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::create_dir_all(tmp.path().join("logs")).unwrap();
    std::fs::create_dir_all(tmp.path().join("vendor")).unwrap();

    std::fs::write(tmp.path().join("src/main.py"), "def main(): pass\n").unwrap();
    std::fs::write(tmp.path().join("logs/debug.py"), "x = 1\n").unwrap();
    std::fs::write(tmp.path().join("vendor/dep.py"), "y = 2\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "src/ files must be found"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("debug.py")),
        "logs/ excluded by gitignore"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("dep.py")),
        "vendor/ excluded by config"
    );
}

#[test]
fn t26_config_exclude_overrides_gitignore_include() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    // .gitignore does NOT mention "tests/"
    std::fs::write(tmp.path().join(".gitignore"), "").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(
        flowspec_dir.join("config.yaml"),
        "exclude:\n  - \"tests\"\n",
    )
    .unwrap();

    std::fs::create_dir_all(tmp.path().join("tests")).unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();

    std::fs::write(tmp.path().join("tests/test.py"), "def test(): pass\n").unwrap();
    std::fs::write(tmp.path().join("src/main.py"), "def main(): pass\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        !entity_files.iter().any(|f| f.contains("test.py")),
        "tests/ excluded by config even though gitignore allows it"
    );
    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "src/ must be found"
    );
}

#[test]
fn t27_gitignore_wildcard_patterns() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    std::fs::write(tmp.path().join(".gitignore"), "*.pyc\nbuild_out/\n").unwrap();

    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::create_dir_all(tmp.path().join("build_out")).unwrap();

    std::fs::write(tmp.path().join("src/main.py"), "def main(): pass\n").unwrap();
    std::fs::write(tmp.path().join("build_out/out.py"), "x = 1\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "src/main.py found"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("out.py")),
        "build_out/ excluded by gitignore"
    );
}

#[test]
fn t28_symlink_into_excluded_directory() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    std::fs::write(tmp.path().join(".gitignore"), "archive/\n").unwrap();

    std::fs::create_dir_all(tmp.path().join("archive")).unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();

    std::fs::write(tmp.path().join("archive/code.py"), "def archived(): pass\n").unwrap();
    std::fs::write(tmp.path().join("src/main.py"), "def main(): pass\n").unwrap();

    // Try to create symlink — skip if platform doesn't support it
    #[cfg(unix)]
    {
        let link_path = tmp.path().join("src/link.py");
        let target = tmp.path().join("archive/code.py");
        if std::os::unix::fs::symlink(&target, &link_path).is_ok() {
            let config = Config::load(tmp.path(), None).unwrap();
            let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

            let entity_files: Vec<&str> = result
                .manifest
                .entities
                .iter()
                .filter_map(|e| e.loc.split(':').next())
                .collect();

            // The symlink target is in an excluded dir — should be excluded or handled gracefully
            assert!(
                entity_files.iter().any(|f| f.contains("main.py")),
                "main.py must still be found"
            );
        }
    }

    // Non-unix: just verify basic analysis works
    #[cfg(not(unix))]
    {
        let config = Config::load(tmp.path(), None).unwrap();
        let result = crate::analyze(tmp.path(), &config, &[]).unwrap();
        assert!(result
            .manifest
            .entities
            .iter()
            .any(|e| e.loc.contains("main.py")));
    }
}

// ===========================================================================
// Category 4: Language Filtering from Config (T29–T33)
// ===========================================================================

#[test]
fn t29_config_languages_filter_file_discovery() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::write(tmp.path().join("main.py"), "def main(): pass\n").unwrap();
    std::fs::write(tmp.path().join("app.js"), "function app() {}\n").unwrap();
    std::fs::write(tmp.path().join("lib.rs"), "fn lib_fn() {}\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(flowspec_dir.join("config.yaml"), "languages:\n  - python\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    // Empty CLI languages → should use config languages
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "Python files must be analyzed when config says python"
    );
    // JS and Rust should not be in entities because config limits to python
    assert!(
        !entity_files.iter().any(|f| f.contains("app.js")),
        "JS files must not be analyzed when config says python only"
    );
}

#[test]
fn t30_cli_language_overrides_config_languages() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::write(tmp.path().join("main.py"), "def main(): pass\n").unwrap();
    std::fs::write(tmp.path().join("lib.rs"), "fn lib_fn() {}\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(flowspec_dir.join("config.yaml"), "languages:\n  - python\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    // CLI says rust — should override config's python
    let result = crate::analyze(tmp.path(), &config, &["rust".to_string()]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("lib.rs")),
        "Rust files must be analyzed when CLI says rust"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("main.py")),
        "Python files must NOT be analyzed when CLI overrides to rust"
    );
}

#[test]
fn t31_empty_config_languages_auto_detect() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::write(tmp.path().join("main.py"), "def main(): pass\n").unwrap();
    std::fs::write(tmp.path().join("app.js"), "function app() {}\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(flowspec_dir.join("config.yaml"), "languages: []\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    assert!(config.languages.is_empty());

    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    // Both should be auto-detected and analyzed
    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "Python auto-detected when config languages empty"
    );
    assert!(
        entity_files.iter().any(|f| f.contains("app.js")),
        "JS auto-detected when config languages empty"
    );
}

#[test]
fn t32_config_languages_invalid_handled_gracefully() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::write(tmp.path().join("main.py"), "def main(): pass\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(
        flowspec_dir.join("config.yaml"),
        "languages:\n  - python\n  - cobol\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    // Config has invalid "cobol" — analyze should handle gracefully.
    // Since config languages feed into active_languages which then
    // filter adapters (not validate), invalid names are silently skipped
    // (no adapter matches "cobol"). No crash.
    let result = crate::analyze(tmp.path(), &config, &[]);
    assert!(
        result.is_ok(),
        "Invalid language in config must not crash — got: {:?}",
        result.err()
    );
}

#[test]
fn t33_config_language_filtering_detection_reporting() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::write(tmp.path().join("main.py"), "def main(): pass\n").unwrap();
    std::fs::write(tmp.path().join("app.js"), "function app() {}\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(flowspec_dir.join("config.yaml"), "languages:\n  - python\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    // The manifest metadata reports which languages were analyzed
    let langs = &result.manifest.metadata.languages;
    assert!(
        langs.contains(&"python".to_string()),
        "Python should be in analyzed languages"
    );
}

// ===========================================================================
// Category 5: Integration — Full Pipeline (T34–T38)
// ===========================================================================

#[test]
fn t34_full_pipeline_config_gitignore_exclude_dirs_produce_zero_output() {
    let tmp = tempfile::tempdir().unwrap();

    // Git repo
    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    std::fs::write(tmp.path().join(".gitignore"), "archive/\n").unwrap();

    // Config
    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(
        flowspec_dir.join("config.yaml"),
        "languages:\n  - python\nexclude:\n  - \"vendor/\"\n",
    )
    .unwrap();

    // Source files
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::create_dir_all(tmp.path().join("archive")).unwrap();
    std::fs::create_dir_all(tmp.path().join("vendor")).unwrap();

    std::fs::write(tmp.path().join("src/main.py"), "def hello(): pass\n").unwrap();
    std::fs::write(tmp.path().join("archive/old.py"), "def legacy(): pass\n").unwrap();
    std::fs::write(tmp.path().join("vendor/dep.py"), "def external(): pass\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_locs: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .map(|e| e.loc.as_str())
        .collect();

    assert!(
        entity_locs.iter().any(|l| l.contains("main.py")),
        "hello from src/main.py must be in entities"
    );
    assert!(
        !entity_locs.iter().any(|l| l.contains("old.py")),
        "legacy from archive/old.py must NOT be in entities (gitignored)"
    );
    assert!(
        !entity_locs.iter().any(|l| l.contains("dep.py")),
        "external from vendor/dep.py must NOT be in entities (config excluded)"
    );

    // Verify no location references to excluded dirs
    for loc in &entity_locs {
        assert!(
            !loc.contains("archive/"),
            "No entity should reference archive/ — got: {}",
            loc
        );
        assert!(
            !loc.contains("vendor/"),
            "No entity should reference vendor/ — got: {}",
            loc
        );
    }
}

#[test]
fn t35_backward_compatibility_no_config_no_gitignore() {
    let tmp = tempfile::tempdir().unwrap();

    // No .flowspec/, no .gitignore, no .git/
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::create_dir_all(tmp.path().join("target")).unwrap();

    std::fs::write(tmp.path().join("src/main.py"), "def main(): pass\n").unwrap();
    std::fs::write(tmp.path().join("target/build.rs"), "fn build() {}\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    assert!(config.config_path.is_none());
    assert!(config.languages.is_empty());
    assert!(config.exclude.is_empty());

    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "src/main.py must be analyzed"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("build.rs")),
        "target/build.rs must be skipped by hardcoded dirs"
    );
}

#[test]
fn t36_all_three_exclusion_mechanisms_active() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    std::fs::write(tmp.path().join(".gitignore"), "logs/\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(
        flowspec_dir.join("config.yaml"),
        "exclude:\n  - \"data_raw\"\n",
    )
    .unwrap();

    std::fs::create_dir_all(tmp.path().join("target")).unwrap();
    std::fs::create_dir_all(tmp.path().join("logs")).unwrap();
    std::fs::create_dir_all(tmp.path().join("data_raw")).unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();

    std::fs::write(tmp.path().join("target/gen.rs"), "fn gen() {}\n").unwrap();
    std::fs::write(tmp.path().join("logs/debug.py"), "x = 1\n").unwrap();
    std::fs::write(tmp.path().join("data_raw/raw.py"), "y = 2\n").unwrap();
    std::fs::write(tmp.path().join("src/main.py"), "def main(): pass\n").unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "src/ included"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("gen.rs")),
        "target/ excluded (hardcoded)"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("debug.py")),
        "logs/ excluded (gitignore)"
    );
    assert!(
        !entity_files.iter().any(|f| f.contains("raw.py")),
        "data_raw/ excluded (config)"
    );
}

#[test]
fn t37_large_exclude_list_performance() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/main.py"), "def main(): pass\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();

    // Generate a config with 50 exclude patterns
    let mut yaml = String::from("exclude:\n");
    for i in 0..50 {
        yaml.push_str(&format!("  - \"exclude_dir_{}/\"\n", i));
    }
    std::fs::write(flowspec_dir.join("config.yaml"), &yaml).unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    assert_eq!(config.exclude.len(), 50);

    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();
    assert!(
        !result.manifest.entities.is_empty(),
        "Analysis must complete with large exclude list"
    );
}

#[test]
fn t38_analyze_passes_config_to_discover() {
    let tmp = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join("tests")).unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();

    std::fs::write(
        tmp.path().join("tests/test_main.py"),
        "def test_main(): pass\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("src/main.py"), "def main(): pass\n").unwrap();

    let flowspec_dir = tmp.path().join(".flowspec");
    std::fs::create_dir_all(&flowspec_dir).unwrap();
    std::fs::write(
        flowspec_dir.join("config.yaml"),
        "exclude:\n  - \"tests/\"\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    let entity_files: Vec<&str> = result
        .manifest
        .entities
        .iter()
        .filter_map(|e| e.loc.split(':').next())
        .collect();

    assert!(
        !entity_files.iter().any(|f| f.contains("test_main.py")),
        "tests/ must be excluded — proves analyze() passes config to discover_source_files"
    );
    assert!(
        entity_files.iter().any(|f| f.contains("main.py")),
        "src/main.py must be analyzed"
    );
}
