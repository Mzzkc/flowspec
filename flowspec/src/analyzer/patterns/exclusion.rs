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

/// Check if a file path indicates a test file.
///
/// Normalizes backslashes for Windows path compatibility, then checks
/// for common test file indicators: `test_` prefix, `/tests/` or `/test/`
/// directory components, and `_test.py` / `_test.rs` suffixes.
pub fn is_test_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized.contains("test_")
        || normalized.contains("/tests/")
        || normalized.contains("/test/")
        || normalized.ends_with("_test.py")
        || normalized.ends_with("_test.rs")
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
    fn test_is_test_path_tests_dir() {
        assert!(is_test_path("src/tests/unit.py"));
    }

    #[test]
    fn test_is_test_path_test_dir() {
        assert!(is_test_path("src/test/integration.py"));
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
    fn test_is_test_path_windows() {
        assert!(is_test_path("src\\tests\\unit.py"));
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
    fn test_is_test_path_latest_version() {
        // "latest_version" contains "test_" as substring ("atest_v")
        // This is a known false positive of the contains("test_") heuristic
        assert!(is_test_path("latest_version.py"));
    }

    #[test]
    fn test_is_test_path_contest() {
        // "contest_results.py" contains "test_" as substring via contains()
        // This is a known limitation — contest_ triggers the test_ check
        assert!(is_test_path("contest_results.py"));
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
    fn test_excluded_test_file_path() {
        assert!(is_excluded_symbol(&make_sym_in_file(
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
}
