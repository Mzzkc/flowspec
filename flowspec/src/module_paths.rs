//! Pure source path and module naming primitives.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
/// Builds a mapping from module names to file paths for both Python and JS/TS.
///
/// **Python:** Maps file paths to dotted module names using standard conventions:
/// - `utils.py` → `"utils"`
/// - `my_package/__init__.py` → `"my_package"`
/// - `my_package/core.py` → `"my_package.core"`
///
/// **JavaScript/TypeScript:** Maps file paths to path-based keys (relative to
/// the common prefix, without extension):
/// - `utils.js` → `"utils"`
/// - `src/helpers.js` → `"src/helpers"`
/// - `components/App.tsx` → `"components/App"`
///
/// Both Python and JS/TS entries coexist in the same map. The resolution pass
/// uses the `"from:<module>"` annotation to determine the key format.
pub fn build_module_map(files: &[PathBuf]) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();
    let mut ambiguous = HashSet::new();
    let mut ambiguous_files = HashSet::new();

    // Phase 1: Python files (dotted module names)
    let py_files: Vec<&PathBuf> = files
        .iter()
        .filter(|f| f.extension().map(|e| e == "py").unwrap_or(false))
        .collect();

    if !py_files.is_empty() {
        let common_prefix = find_common_prefix(&py_files);
        for file in &py_files {
            let rel = file.strip_prefix(&common_prefix).unwrap_or(file);
            let module_name = path_to_module_name(rel);
            if !module_name.is_empty() {
                if let Some(previous) = map.insert(module_name.clone(), (*file).clone()) {
                    ambiguous.insert(module_name);
                    ambiguous_files.insert(previous);
                    ambiguous_files.insert((*file).clone());
                }
            }
        }
    }

    // Phase 2: JS/TS files (path-based keys, no extension)
    let js_files: Vec<&PathBuf> = files
        .iter()
        .filter(|f| {
            f.extension()
                .and_then(|e| e.to_str())
                .map(|_| {
                    matches!(
                        crate::language_metadata::language_id_for_path(f),
                        Some("javascript" | "typescript")
                    )
                })
                .unwrap_or(false)
        })
        .collect();

    if !js_files.is_empty() {
        let js_prefix = find_js_common_prefix(&js_files);
        for file in &js_files {
            let rel = file.strip_prefix(&js_prefix).unwrap_or(file);
            let key = js_path_to_module_key(rel);
            if !key.is_empty() {
                if let Some(previous) = map.insert(key.clone(), (*file).clone()) {
                    ambiguous.insert(key);
                    ambiguous_files.insert(previous);
                    ambiguous_files.insert((*file).clone());
                }
            }
        }
    }

    // Phase 3: Rust files (crate-path keys)
    let rs_files: Vec<&PathBuf> = files
        .iter()
        .filter(|f| f.extension().map(|e| e == "rs").unwrap_or(false))
        .collect();

    if !rs_files.is_empty() {
        let crate_roots = find_rust_crate_roots(&rs_files);
        if crate_roots.is_empty() {
            // No crate root found — use common prefix as fallback
            let common = find_js_common_prefix(&rs_files);
            for file in &rs_files {
                if let Ok(rel) = file.strip_prefix(&common) {
                    let key = rust_path_to_module_key(rel);
                    if !key.is_empty() {
                        if let Some(previous) = map.insert(key.clone(), (*file).clone()) {
                            ambiguous.insert(key);
                            ambiguous_files.insert(previous);
                            ambiguous_files.insert((*file).clone());
                        }
                    }
                }
            }
        } else {
            for file in &rs_files {
                if let Some(root_dir) = closest_rust_crate_root(file, &crate_roots) {
                    if let Ok(rel) = file.strip_prefix(root_dir) {
                        let key = rust_path_to_module_key(rel);
                        if !key.is_empty() {
                            if let Some(previous) = map.insert(key.clone(), (*file).clone()) {
                                ambiguous.insert(key);
                                ambiguous_files.insert(previous);
                                ambiguous_files.insert((*file).clone());
                            }
                        }
                    }
                }
            }
        }
    }

    if !ambiguous.is_empty() {
        for file in &ambiguous_files {
            let language =
                crate::language_metadata::language_id_for_path(file).unwrap_or("unknown");
            map.insert(format!("{language}:{}", file.display()), file.clone());
        }
    }
    map.retain(|key, _| !ambiguous.contains(key));
    map
}

/// Finds directories containing Rust crate roots (`lib.rs` or `main.rs`).
pub(crate) fn find_rust_crate_roots(rs_files: &[&PathBuf]) -> Vec<PathBuf> {
    rs_files
        .iter()
        .filter(|f| {
            f.file_name()
                .map(|n| n == "lib.rs" || n == "main.rs")
                .unwrap_or(false)
        })
        .filter_map(|f| f.parent().map(|p| p.to_path_buf()))
        .collect()
}

/// Finds the closest crate root directory that is an ancestor of the given file.
pub(crate) fn closest_rust_crate_root<'a>(
    file: &Path,
    roots: &'a [PathBuf],
) -> Option<&'a PathBuf> {
    roots
        .iter()
        .filter(|root| file.starts_with(root))
        .max_by_key(|root| root.components().count())
}

/// Converts a relative Rust file path to a `crate::` prefixed module key.
///
/// - `lib.rs` → `"crate"`, `main.rs` → `"crate"`
/// - `utils.rs` → `"crate::utils"`
/// - `parser/mod.rs` → `"crate::parser"`
/// - `parser/rust.rs` → `"crate::parser::rust"`
pub(crate) fn rust_path_to_module_key(rel_path: &Path) -> String {
    let stem = rel_path.with_extension("");
    let parts: Vec<&str> = stem
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();

    if parts.is_empty() {
        return String::new();
    }

    if parts.len() == 1 && (parts[0] == "lib" || parts[0] == "main") {
        return "crate".to_string();
    }

    let mut module_parts: Vec<&str> = parts;
    if module_parts.last() == Some(&"mod") {
        module_parts.pop();
    }

    if module_parts.is_empty() {
        return "crate".to_string();
    }

    format!("crate::{}", module_parts.join("::"))
}

/// Finds the common directory prefix for JS/TS files.
pub(crate) fn find_js_common_prefix(files: &[&PathBuf]) -> PathBuf {
    if files.is_empty() {
        return PathBuf::new();
    }

    let roots: Vec<PathBuf> = files
        .iter()
        .filter_map(|f| f.parent().map(|p| p.to_path_buf()))
        .collect();

    if roots.is_empty() {
        return PathBuf::new();
    }

    let mut prefix = roots[0].clone();
    for root in &roots[1..] {
        while !root.starts_with(&prefix) {
            if !prefix.pop() {
                return PathBuf::new();
            }
        }
    }

    prefix
}

/// Converts a relative JS/TS file path to a module map key.
///
/// Strips the file extension and converts to forward-slash path:
/// - `helpers.js` → `"helpers"`
/// - `src/utils.ts` → `"src/utils"`
/// - `index.js` → `"index"`
pub(crate) fn js_path_to_module_key(rel_path: &Path) -> String {
    let stem = rel_path.with_extension("");
    stem.components()
        .filter_map(|c| {
            if let std::path::Component::Normal(s) = c {
                s.to_str()
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Finds the project root directory for module name computation.
///
/// For `__init__.py` files, uses the grandparent directory (since the parent
/// directory IS part of the package path). For regular `.py` files, uses the
/// parent directory. The result is the common prefix of all these "effective
/// roots", ensuring package structure is preserved in module names.
pub(crate) fn find_common_prefix(files: &[&PathBuf]) -> PathBuf {
    if files.is_empty() {
        return PathBuf::new();
    }

    // For each file, compute its "effective root" — the directory that should
    // NOT be part of the module name.
    let effective_roots: Vec<PathBuf> = files
        .iter()
        .map(|f| {
            let is_init = f
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "__init__.py")
                .unwrap_or(false);

            if is_init {
                // __init__.py: grandparent is the project root
                f.parent()
                    .and_then(|p| p.parent())
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| f.parent().map(|p| p.to_path_buf()).unwrap_or_default())
            } else {
                f.parent().map(|p| p.to_path_buf()).unwrap_or_default()
            }
        })
        .collect();

    if effective_roots.is_empty() {
        return PathBuf::new();
    }

    let mut prefix = effective_roots[0].clone();
    for root in &effective_roots[1..] {
        while !root.starts_with(&prefix) {
            if !prefix.pop() {
                return PathBuf::new();
            }
        }
    }

    prefix
}

/// Converts a relative Python file path to a module name.
///
/// - `utils.py` → `"utils"`
/// - `__init__.py` → `""` (maps to parent package, handled by caller)
/// - `pkg/__init__.py` → `"pkg"`
/// - `pkg/core.py` → `"pkg.core"`
pub(crate) fn path_to_module_name(rel_path: &Path) -> String {
    let file_name = rel_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if file_name == "__init__.py" {
        if let Some(parent) = rel_path.parent() {
            return components_to_module(parent);
        }
        return String::new();
    }

    let stem = rel_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if stem.is_empty() {
        return String::new();
    }

    if let Some(parent) = rel_path.parent() {
        let parent_module = components_to_module(parent);
        if parent_module.is_empty() {
            stem.to_string()
        } else {
            format!("{}.{}", parent_module, stem)
        }
    } else {
        stem.to_string()
    }
}

/// Converts path components to a dot-separated module name.
fn components_to_module(path: &Path) -> String {
    path.components()
        .filter_map(|c| {
            if let std::path::Component::Normal(s) = c {
                s.to_str()
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}
