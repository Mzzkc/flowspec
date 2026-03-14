// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Shared exclusion logic for diagnostic pattern detectors.
//!
//! Consolidates three separate `is_excluded()` implementations
//! (data_dead_end, orphaned_implementation, isolated_cluster) into
//! a single source of truth. All exclusion rules are the superset
//! of the three previous implementations — additions only, never
//! removing an existing exclusion.

use std::path::Path;

use crate::parser::ir::Symbol;

/// Check if a file path indicates a test file by filename or directory convention.
///
/// Normalizes backslashes for Windows path compatibility, then checks:
///
/// **Filename-based patterns** (extracted via `rsplit('/')`):
/// - `test_` prefix (e.g., `test_module.py`) — Python convention
/// - `conftest` prefix — pytest convention
/// - `_test.py` / `_test.rs` suffix (e.g., `utils_test.py`)
/// - `_tests.py` / `_tests.rs` suffix (plural variant)
/// - `.test.js` / `.test.ts` / `.test.jsx` / `.test.tsx` — Jest/Vitest convention
/// - `.spec.js` / `.spec.ts` / `.spec.jsx` / `.spec.tsx` — Angular/Jasmine convention
///
/// **Directory-based pattern** (exception to filename-only rule):
/// - `__tests__/` as a path segment — Jest convention where ALL files inside
///   are test files by definition. Unlike generic `/tests/`, this is unambiguous.
///
/// Generic directory names like `/tests/` do NOT trigger this function.
/// A file at `tests/fixtures/dead_code.py` is a fixture, not a test file.
pub fn is_test_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let filename = normalized.rsplit('/').next().unwrap_or(&normalized);

    // Filename-based patterns (Python/Rust)
    filename.starts_with("test_")
        || filename.starts_with("conftest")
        || filename.ends_with("_test.py")
        || filename.ends_with("_test.rs")
        || filename.ends_with("_tests.py")
        || filename.ends_with("_tests.rs")
        // Filename-based patterns (JS/TS)
        || filename.ends_with(".test.js")
        || filename.ends_with(".test.ts")
        || filename.ends_with(".test.jsx")
        || filename.ends_with(".test.tsx")
        || filename.ends_with(".spec.js")
        || filename.ends_with(".spec.ts")
        || filename.ends_with(".spec.jsx")
        || filename.ends_with(".spec.tsx")
        // Directory-based pattern: __tests__/ as a path segment (Jest convention)
        || has_tests_dir_segment(&normalized)
}

/// Check if a normalized path contains `__tests__/` as a proper path segment.
///
/// Matches `__tests__/` at the start of the path or after a `/` separator.
/// Does NOT match `__tests__` as a bare filename (no child path) or as part
/// of another directory name (e.g., `my__tests__dir/`).
fn has_tests_dir_segment(normalized: &str) -> bool {
    // Must have something after __tests__/ (it must be a directory, not a file)
    if normalized.starts_with("__tests__/") && normalized.len() > "__tests__/".len() {
        return true;
    }
    // __tests__/ as a segment within the path
    if let Some(pos) = normalized.find("/__tests__/") {
        // Verify there's content after /__tests__/
        let after = pos + "/__tests__/".len();
        return after < normalized.len();
    }
    false
}

/// Check if a symbol should be excluded from diagnostic detection.
///
/// Applies the unified superset of exclusion rules across all patterns:
/// - Entry point annotations (`entry_point`)
/// - Import annotations (`import`)
/// - Dunder methods (`__init__`, `__str__`, etc.)
/// - Test function names (`test_*`)
/// - Test file paths (via [`is_test_path`])
/// - Entry point names (`main`, `__main__`, `if_name_main`, `main_*`, `*_main`)
///
/// Pattern-specific exclusions (e.g., Module/Class/Struct kind filtering
/// in data_dead_end, Method-only targeting in orphaned_implementation)
/// remain in each pattern's detect function.
pub fn is_excluded_symbol(symbol: &Symbol) -> bool {
    // Skip entry points (explicitly marked as called from outside analysis scope)
    if symbol.annotations.contains(&"entry_point".to_string()) {
        return true;
    }

    // Skip import symbols (handled by phantom_dependency)
    if symbol.annotations.contains(&"import".to_string()) {
        return true;
    }

    // Skip dunder methods (Python special methods — runtime-dispatched)
    if symbol.name.starts_with("__") && symbol.name.ends_with("__") {
        return true;
    }

    // Skip test functions (name starts with test_)
    if symbol.name.starts_with("test_") {
        return true;
    }

    // Skip symbols in test files
    let path = symbol.location.file.to_string_lossy();
    if is_test_path(&path) {
        return true;
    }

    // Skip entry point names
    if symbol.name == "main"
        || symbol.name == "__main__"
        || symbol.name == "if_name_main"
        || symbol.name.starts_with("main_")
        || symbol.name.ends_with("_main")
    {
        return true;
    }

    false
}

/// Produce a relative path string from an absolute file path and a project root.
///
/// Uses `strip_prefix` to remove the project root, falling back to the
/// filename component when the prefix is the entire path (single-file
/// analysis), or the original path when `strip_prefix` fails (mismatched
/// roots).
pub fn relativize_path(file: &Path, project_root: &Path) -> String {
    let rel = file.strip_prefix(project_root).unwrap_or(file);

    // When analyzing a single file, strip_prefix removes the entire path,
    // producing an empty string. Fall back to the filename component.
    if rel.as_os_str().is_empty() {
        file.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| file.display().to_string())
    } else {
        rel.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;
    use crate::parser::ir::*;
    use std::path::PathBuf;

    fn make_sym(name: &str) -> Symbol {
        Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Function,
            name: name.to_string(),
            qualified_name: name.to_string(),
            visibility: Visibility::Public,
            signature: None,
            location: Location {
                file: PathBuf::from("module.py"),
                line: 1,
                column: 1,
                end_line: 2,
                end_column: 1,
            },
            resolution: ResolutionStatus::Resolved,
            scope: ScopeId::default(),
            annotations: vec![],
        }
    }

    fn make_sym_in_file(name: &str, file: &str) -> Symbol {
        let mut s = make_sym(name);
        s.location.file = PathBuf::from(file);
        s
    }

    fn make_sym_with_annotation(name: &str, annotation: &str) -> Symbol {
        let mut s = make_sym(name);
        s.annotations.push(annotation.to_string());
        s
    }

    // -- is_test_path tests --

    #[test]
    fn test_is_test_path_prefix() {
        assert!(is_test_path("test_module.py"));
    }

    #[test]
    fn test_is_test_path_tests_dir_filename_not_test() {
        // Filename "unit.py" has no test pattern — directory should not matter
        assert!(!is_test_path("src/tests/unit.py"));
    }

    #[test]
    fn test_is_test_path_test_dir_filename_not_test() {
        // Filename "integration.py" has no test pattern — directory should not matter
        assert!(!is_test_path("src/test/integration.py"));
    }

    #[test]
    fn test_is_test_path_suffix_py() {
        assert!(is_test_path("utils_test.py"));
    }

    #[test]
    fn test_is_test_path_suffix_rs() {
        assert!(is_test_path("handler_test.rs"));
    }

    #[test]
    fn test_is_test_path_windows_filename_not_test() {
        // Filename "unit.py" has no test pattern even through Windows path
        assert!(!is_test_path("src\\tests\\unit.py"));
    }

    #[test]
    fn test_is_test_path_normal_file() {
        assert!(!is_test_path("src/utils.py"));
    }

    #[test]
    fn test_is_test_path_main() {
        assert!(!is_test_path("main.py"));
    }

    #[test]
    fn test_is_test_path_latest_version_not_test() {
        // "latest_version.py" does NOT start with "test_" — not a test file
        assert!(!is_test_path("latest_version.py"));
    }

    #[test]
    fn test_is_test_path_contest_not_test() {
        // "contest_results.py" does NOT start with "test_" — not a test file
        assert!(!is_test_path("contest_results.py"));
    }

    // -- is_excluded_symbol tests --

    #[test]
    fn test_excluded_entry_point_annotation() {
        assert!(is_excluded_symbol(&make_sym_with_annotation(
            "func",
            "entry_point"
        )));
    }

    #[test]
    fn test_excluded_import_annotation() {
        assert!(is_excluded_symbol(&make_sym_with_annotation(
            "os", "import"
        )));
    }

    #[test]
    fn test_excluded_dunder_init() {
        assert!(is_excluded_symbol(&make_sym("__init__")));
    }

    #[test]
    fn test_excluded_dunder_str() {
        assert!(is_excluded_symbol(&make_sym("__str__")));
    }

    #[test]
    fn test_excluded_test_function() {
        assert!(is_excluded_symbol(&make_sym("test_login")));
    }

    #[test]
    fn test_excluded_main() {
        assert!(is_excluded_symbol(&make_sym("main")));
    }

    #[test]
    fn test_excluded_main_handler() {
        assert!(is_excluded_symbol(&make_sym("main_handler")));
    }

    #[test]
    fn test_excluded_setup_main() {
        assert!(is_excluded_symbol(&make_sym("setup_main")));
    }

    #[test]
    fn test_excluded_test_file_path_filename_not_test() {
        // mod.py is NOT a test file — filename has no test pattern
        assert!(!is_excluded_symbol(&make_sym_in_file(
            "func",
            "src/tests/mod.py"
        )));
    }

    #[test]
    fn test_not_excluded_normal_function() {
        assert!(!is_excluded_symbol(&make_sym("process_data")));
    }

    #[test]
    fn test_not_excluded_private_helper() {
        assert!(!is_excluded_symbol(&make_sym("_private_helper")));
    }

    #[test]
    fn test_not_excluded_maintain() {
        // "maintain" contains "main" but isn't "main" — starts_with doesn't trigger
        // because we check exact match or starts_with("main_") / ends_with("_main")
        assert!(!is_excluded_symbol(&make_sym("maintain")));
    }

    #[test]
    fn test_excluded_double_underscore() {
        // "__" starts with "__" and ends with "__" → matches dunder heuristic
        assert!(is_excluded_symbol(&make_sym("__")));
    }

    #[test]
    fn test_excluded_test_underscore_only() {
        // "test_" starts_with("test_") → excluded
        assert!(is_excluded_symbol(&make_sym("test_")));
    }

    // -- is_test_path: fixture paths are NOT test paths (regression core) --

    #[test]
    fn test_fixture_dead_code_is_not_test_path() {
        assert!(
            !is_test_path("tests/fixtures/python/dead_code.py"),
            "Fixture file dead_code.py must NOT be classified as test path"
        );
    }

    #[test]
    fn test_fixture_clean_code_is_not_test_path() {
        assert!(
            !is_test_path("tests/fixtures/python/clean_code.py"),
            "Fixture file clean_code.py must NOT be classified as test path"
        );
    }

    #[test]
    fn test_fixture_isolated_module_is_not_test_path() {
        assert!(
            !is_test_path("tests/fixtures/python/isolated_module.py"),
            "Fixture file isolated_module.py must NOT be classified as test path"
        );
    }

    // -- is_test_path: substring false positives fixed --

    #[test]
    fn test_protest_is_not_test_path() {
        assert!(
            !is_test_path("protest.py"),
            "'protest' does not start with 'test_'"
        );
    }

    #[test]
    fn test_attest_is_not_test_path() {
        assert!(
            !is_test_path("attest.py"),
            "'attest' does not start with 'test_'"
        );
    }

    #[test]
    fn test_testing_utils_is_not_test_path() {
        assert!(
            !is_test_path("testing_utils.py"),
            "testing_utils.py must NOT match — 'testing_' != 'test_'"
        );
    }

    #[test]
    fn test_my_test_utils_is_not_test_path() {
        assert!(
            !is_test_path("my_test_utils.py"),
            "my_test_utils.py must NOT match — filename-based check only"
        );
    }

    // -- is_test_path: directory-based matching removed --

    #[test]
    fn test_mod_py_in_tests_dir_is_not_test_path() {
        assert!(
            !is_test_path("src/tests/mod.py"),
            "mod.py under /tests/ is NOT a test file"
        );
    }

    // -- is_test_path: true positive guards --

    #[test]
    fn test_test_prefix_rs() {
        assert!(
            is_test_path("test_handler.rs"),
            "test_ prefix must match for .rs"
        );
    }

    #[test]
    fn test_conftest_py() {
        assert!(
            is_test_path("conftest.py"),
            "conftest.py is pytest convention"
        );
    }

    #[test]
    fn test_test_prefix_in_deep_path() {
        assert!(
            is_test_path("deeply/nested/path/to/test_module.py"),
            "test_ prefix on filename must match regardless of directory depth"
        );
    }

    #[test]
    fn test_suffix_test_in_deep_path() {
        assert!(
            is_test_path("src/api/routes_test.py"),
            "_test.py suffix must match regardless of directory depth"
        );
    }

    #[test]
    fn test_plural_suffix_tests_py() {
        assert!(
            is_test_path("utils_tests.py"),
            "_tests.py plural suffix should match"
        );
    }

    #[test]
    fn test_plural_suffix_tests_rs() {
        assert!(
            is_test_path("handler_tests.rs"),
            "_tests.rs plural suffix should match"
        );
    }

    // -- is_test_path: adversarial edge cases --

    #[test]
    fn test_empty_string_is_not_test_path() {
        assert!(
            !is_test_path(""),
            "Empty string must return false, not panic"
        );
    }

    #[test]
    fn test_just_slash_is_not_test_path() {
        assert!(!is_test_path("/"), "Bare slash must return false");
    }

    #[test]
    fn test_bare_test_underscore_is_test_path() {
        assert!(
            is_test_path("test_"),
            "Bare 'test_' filename is a test file"
        );
    }

    #[test]
    fn test_bare_underscore_test_dot_py_is_test_path() {
        assert!(
            is_test_path("_test.py"),
            "_test.py ends with _test.py suffix"
        );
    }

    #[test]
    fn test_windows_backslash_test_file() {
        assert!(
            is_test_path("src\\tests\\test_handler.py"),
            "test_handler.py has test_ prefix even through Windows path"
        );
    }

    #[test]
    fn test_windows_backslash_fixture_not_test() {
        assert!(
            !is_test_path("src\\tests\\unit.py"),
            "unit.py is not a test file even with Windows backslashes"
        );
    }

    #[test]
    fn test_no_extension_test_prefix() {
        assert!(
            is_test_path("test_something"),
            "No extension, test_ prefix still matches"
        );
    }

    #[test]
    fn test_path_with_only_extension() {
        assert!(!is_test_path(".py"), "Bare extension is not a test file");
    }

    #[test]
    fn test_double_slash_path() {
        assert!(
            !is_test_path("src//fixtures//dead_code.py"),
            "Double slashes — dead_code.py is not a test file"
        );
    }

    #[test]
    fn test_test_in_directory_name_only() {
        assert!(
            !is_test_path("test_data/config.py"),
            "config.py is not a test file — test_ is in directory, not filename"
        );
    }

    #[test]
    fn test_deeply_nested_test_dirs_non_test_file() {
        assert!(
            !is_test_path("project/tests/integration/test_data/fixtures/sample.py"),
            "sample.py is not a test file despite test directories in path"
        );
    }

    // -- is_excluded_symbol: integration with is_test_path fix --

    #[test]
    fn test_symbol_in_fixture_not_excluded() {
        let sym = make_sym_in_file("unused_helper", "tests/fixtures/python/dead_code.py");
        assert!(
            !is_excluded_symbol(&sym),
            "Symbol in fixture file must NOT be excluded — fixture is not a test"
        );
    }

    #[test]
    fn test_symbol_in_test_file_still_excluded() {
        let sym = make_sym_in_file("helper_fn", "test_module.py");
        assert!(
            is_excluded_symbol(&sym),
            "Symbol in test_module.py must be excluded — test_ prefix on filename"
        );
    }

    #[test]
    fn test_contest_results_symbol_not_excluded() {
        let sym = make_sym_in_file("score_calc", "contest_results.py");
        assert!(
            !is_excluded_symbol(&sym),
            "Symbol in contest_results.py must NOT be excluded"
        );
    }

    // -- Full pipeline regression: diagnostics fire on fixture files --

    #[test]
    fn test_dead_code_fixture_produces_diagnostics() {
        use crate::analyzer::patterns::data_dead_end;

        let graph = crate::test_utils::build_dead_code_graph();
        let diagnostics = data_dead_end::detect(&graph, Path::new(""));

        assert!(
            !diagnostics.is_empty(),
            "dead_code.py fixture MUST produce data_dead_end diagnostics"
        );

        let dead_end_names: Vec<&str> = diagnostics.iter().map(|d| d.entity.as_str()).collect();
        assert!(
            dead_end_names.iter().any(|n| n.contains("unused_helper")),
            "unused_helper must be flagged as dead end"
        );
        assert!(
            dead_end_names.iter().any(|n| n.contains("_private_util")),
            "_private_util must be flagged as dead end"
        );
    }

    #[test]
    fn test_clean_code_fixture_produces_zero_diagnostics() {
        use crate::analyzer::patterns::run_all_patterns;

        let graph = crate::test_utils::build_clean_code_graph();
        let diagnostics = run_all_patterns(&graph, Path::new(""));

        assert!(
            diagnostics.is_empty(),
            "clean_code.py must produce ZERO diagnostics, got {}: {:?}",
            diagnostics.len(),
            diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_fixture_symbols_with_test_path_prefix_in_directory() {
        use crate::analyzer::patterns::data_dead_end;
        use crate::parser::ir::{SymbolKind, Visibility};

        let mut g = Graph::new();
        let _dead = g.add_symbol(crate::test_utils::make_symbol(
            "unreachable_fn",
            SymbolKind::Function,
            Visibility::Private,
            "tests/fixtures/python/dead_code.py",
            7,
        ));

        let diagnostics = data_dead_end::detect(&g, Path::new(""));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.entity.contains("unreachable_fn")),
            "unreachable_fn under tests/fixtures/ MUST be detected after is_test_path fix"
        );
    }

    // -- Confidence calibration spot checks --

    #[test]
    fn test_private_dead_end_has_high_confidence() {
        use crate::analyzer::diagnostic::Confidence;
        use crate::analyzer::patterns::data_dead_end;

        let graph = crate::test_utils::build_dead_code_graph();
        let diagnostics = data_dead_end::detect(&graph, Path::new(""));

        let unused_helper_diag = diagnostics
            .iter()
            .find(|d| d.entity.contains("unused_helper"))
            .expect("unused_helper must be detected");

        assert_eq!(
            unused_helper_diag.confidence,
            Confidence::High,
            "Private function with zero callers must be HIGH confidence"
        );
    }

    #[test]
    fn test_public_dead_end_has_low_confidence() {
        use crate::analyzer::diagnostic::Confidence;
        use crate::analyzer::patterns::data_dead_end;

        let graph = crate::test_utils::build_public_api_graph();
        let diagnostics = data_dead_end::detect(&graph, Path::new(""));

        let public_diag = diagnostics
            .iter()
            .find(|d| d.entity.contains("format_timestamp"))
            .expect("format_timestamp must be detected");

        assert_eq!(
            public_diag.confidence,
            Confidence::Low,
            "Public function with zero callers must be LOW confidence"
        );
    }

    // -- relativize_path tests --

    #[test]
    fn test_relativize_path_directory() {
        let path = Path::new("/home/user/project/src/module.py");
        let root = Path::new("/home/user/project");
        assert_eq!(relativize_path(path, root), "src/module.py");
    }

    #[test]
    fn test_relativize_path_single_file() {
        let path = Path::new("/home/user/dead_code.py");
        let root = Path::new("/home/user/dead_code.py");
        assert_eq!(relativize_path(path, root), "dead_code.py");
    }

    #[test]
    fn test_relativize_path_mismatched_root() {
        let path = Path::new("/home/bob/other/file.py");
        let root = Path::new("/home/alice/project");
        // strip_prefix fails → returns original absolute path
        assert_eq!(relativize_path(path, root), "/home/bob/other/file.py");
    }

    #[test]
    fn test_relativize_path_at_root() {
        let path = Path::new("/repo/script.py");
        let root = Path::new("/repo");
        assert_eq!(relativize_path(path, root), "script.py");
    }

    #[test]
    fn test_relativize_path_deeply_nested() {
        let path = Path::new("/repo/src/packages/core/internal/utils/helpers/deep.py");
        let root = Path::new("/repo");
        assert_eq!(
            relativize_path(path, root),
            "src/packages/core/internal/utils/helpers/deep.py"
        );
    }

    #[test]
    fn test_relativize_path_empty_root() {
        let path = Path::new("module.py");
        let root = Path::new("");
        // strip_prefix("") on "module.py" → "module.py"
        assert_eq!(relativize_path(path, root), "module.py");
    }

    // =========================================================================
    // QA-2: JS test conventions in is_test_path (Cycle 5)
    // =========================================================================

    // -- Standard JS/TS test patterns — true positives --

    #[test]
    fn test_is_test_path_jest_test_js() {
        assert!(is_test_path("App.test.js"), "Jest .test.js convention");
    }

    #[test]
    fn test_is_test_path_jest_test_ts() {
        assert!(is_test_path("App.test.ts"), "Jest .test.ts convention");
    }

    #[test]
    fn test_is_test_path_jest_test_jsx() {
        assert!(
            is_test_path("Component.test.jsx"),
            "Jest .test.jsx convention"
        );
    }

    #[test]
    fn test_is_test_path_jest_test_tsx() {
        assert!(
            is_test_path("Component.test.tsx"),
            "Vitest .test.tsx convention"
        );
    }

    #[test]
    fn test_is_test_path_angular_spec_js() {
        assert!(
            is_test_path("app.spec.js"),
            "Angular/Jasmine .spec.js convention"
        );
    }

    #[test]
    fn test_is_test_path_angular_spec_ts() {
        assert!(is_test_path("app.spec.ts"), "Angular .spec.ts convention");
    }

    #[test]
    fn test_is_test_path_angular_spec_jsx() {
        assert!(is_test_path("Component.spec.jsx"), ".spec.jsx convention");
    }

    #[test]
    fn test_is_test_path_angular_spec_tsx() {
        assert!(is_test_path("Component.spec.tsx"), ".spec.tsx convention");
    }

    // -- __tests__/ directory convention — true positives --

    #[test]
    fn test_is_test_path_jest_tests_dir() {
        assert!(
            is_test_path("__tests__/App.js"),
            "Files inside __tests__/ are test files by Jest convention"
        );
    }

    #[test]
    fn test_is_test_path_jest_tests_dir_nested() {
        assert!(
            is_test_path("__tests__/helpers/setup.js"),
            "Nested files inside __tests__/ are still test files"
        );
    }

    #[test]
    fn test_is_test_path_jest_tests_dir_deep_nested() {
        assert!(
            is_test_path("src/components/__tests__/Button.tsx"),
            "__tests__/ can appear anywhere in path — Jest convention"
        );
    }

    #[test]
    fn test_is_test_path_jest_tests_dir_windows() {
        assert!(
            is_test_path("src\\components\\__tests__\\Button.tsx"),
            "__tests__/ detection must work with Windows backslashes"
        );
    }

    // -- JS substring false positive guards — true negatives --

    #[test]
    fn test_is_test_path_testament_js_not_test() {
        assert!(
            !is_test_path("testament.js"),
            "testament.js does NOT end with .test.js — not a test file"
        );
    }

    #[test]
    fn test_is_test_path_testify_ts_not_test() {
        assert!(
            !is_test_path("testify.ts"),
            "testify.ts does NOT end with .test.ts — not a test file"
        );
    }

    #[test]
    fn test_is_test_path_app_test_module_js_not_test() {
        assert!(
            !is_test_path("app.test.module.js"),
            "app.test.module.js — .test. is NOT immediately before final extension"
        );
    }

    #[test]
    fn test_is_test_path_spectacle_js_not_test() {
        assert!(
            !is_test_path("spectacle.js"),
            "spectacle.js does not end with .spec.js"
        );
    }

    #[test]
    fn test_is_test_path_inspect_ts_not_test() {
        assert!(
            !is_test_path("inspect.ts"),
            "inspect.ts does not end with .spec.ts"
        );
    }

    #[test]
    fn test_is_test_path_my_tests_dir_not_test() {
        assert!(
            !is_test_path("my__tests__dir/file.js"),
            "my__tests__dir is not __tests__/ as a path segment"
        );
    }

    // -- Edge cases and boundary conditions --

    #[test]
    fn test_is_test_path_bare_dot_test_js() {
        assert!(
            is_test_path(".test.js"),
            ".test.js ends with .test.js — technically matches"
        );
    }

    #[test]
    fn test_is_test_path_tests_dir_at_root() {
        assert!(
            is_test_path("__tests__/file.js"),
            "__tests__/ at root must match"
        );
    }

    #[test]
    fn test_is_test_path_file_named_underscore_tests_underscore() {
        assert!(
            !is_test_path("__tests__"),
            "Bare __tests__ without a child file path is not meaningful as test path"
        );
    }

    // -- Integration: is_excluded_symbol with JS test conventions --

    #[test]
    fn test_excluded_symbol_in_jest_test_file() {
        let sym = make_sym_in_file("renderButton", "src/components/__tests__/Button.test.tsx");
        assert!(
            is_excluded_symbol(&sym),
            "Symbol in __tests__/Button.test.tsx must be excluded from diagnostics"
        );
    }

    #[test]
    fn test_excluded_symbol_in_spec_file() {
        let sym = make_sym_in_file("setupTest", "src/App.spec.ts");
        assert!(
            is_excluded_symbol(&sym),
            "Symbol in App.spec.ts must be excluded from diagnostics"
        );
    }

    #[test]
    fn test_not_excluded_symbol_in_testament_file() {
        let sym = make_sym_in_file("processInput", "testament.js");
        assert!(
            !is_excluded_symbol(&sym),
            "Symbol in testament.js must NOT be excluded — not a test file"
        );
    }
}
