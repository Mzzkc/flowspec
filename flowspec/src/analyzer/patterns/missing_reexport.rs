// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Missing re-export detection — public symbols not exported through parent module.
//!
//! Scans for parent module files (`__init__.py`, `mod.rs`) and checks
//! whether public symbols in sibling submodule files are re-exported
//! through the parent. Uses name-based matching since cross-file
//! references are currently unresolved (`SymbolId::default()`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::analyzer::diagnostic::*;
use crate::analyzer::patterns::exclusion::relativize_path;
use crate::graph::Graph;
use crate::parser::ir::{SymbolKind, Visibility};

/// Detect public symbols in submodules that are not re-exported through their parent module.
///
/// Identifies parent modules (`__init__.py`, `mod.rs`) and checks sibling
/// files for public symbols not referenced in the parent's import list.
/// Uses name-based matching as the detection mechanism.
///
/// The `project_root` path is used to produce relative file paths in
/// diagnostic locations and evidence, matching the format of entity `loc` fields.
///
/// Severity: Info. Confidence: Moderate (re-export may be intentionally omitted).
pub fn detect(graph: &Graph, project_root: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Step 1: Collect all file paths and identify parent modules
    let mut all_files: HashSet<PathBuf> = HashSet::new();
    for (_, symbol) in graph.all_symbols() {
        if !symbol.location.file.as_os_str().is_empty() {
            all_files.insert(symbol.location.file.clone());
        }
    }

    let parent_modules: Vec<PathBuf> = all_files
        .iter()
        .filter(|p| is_parent_module(p))
        .cloned()
        .collect();

    if parent_modules.is_empty() {
        return diagnostics;
    }

    // Step 2: For each parent module, find sibling submodule files
    for parent_path in &parent_modules {
        let parent_dir = match parent_path.parent() {
            Some(dir) => dir,
            None => continue,
        };

        // Collect names already re-exported by the parent module
        let reexported_names: HashSet<String> = collect_reexported_names(graph, parent_path);

        // Find sibling files in the same directory
        let siblings: Vec<&PathBuf> = all_files
            .iter()
            .filter(|f| *f != parent_path && is_sibling(f, parent_dir) && !is_parent_module(f))
            .collect();

        // Step 3: Check each sibling's public symbols against parent's re-exports
        for sibling_path in siblings {
            let sibling_symbols = collect_public_symbols(graph, sibling_path);

            let sibling_rel = relativize_path(sibling_path, project_root);
            let parent_rel = relativize_path(parent_path, project_root);

            for (name, qualified_name, line) in &sibling_symbols {
                if reexported_names.contains(name) {
                    continue;
                }

                let location = format!("{}:{}", sibling_rel, line);

                diagnostics.push(Diagnostic {
                    id: String::new(),
                    pattern: DiagnosticPattern::MissingReexport,
                    severity: Severity::Info,
                    confidence: Confidence::Moderate,
                    entity: qualified_name.clone(),
                    message: format!(
                        "Missing re-export: public symbol '{}' in '{}' is not exported through '{}'",
                        name, sibling_rel, parent_rel,
                    ),
                    evidence: vec![Evidence {
                        observation: format!(
                            "'{}' is public in '{}' but not found in parent module '{}'",
                            name, sibling_rel, parent_rel,
                        ),
                        location: Some(location.clone()),
                        context: Some(format!(
                            "parent module: {}, submodule: {}",
                            parent_rel, sibling_rel,
                        )),
                    }],
                    suggestion: format!(
                        "Add '{}' to the exports in '{}', or make it private if it is internal.",
                        name, parent_rel,
                    ),
                    location,
                });
            }
        }
    }

    diagnostics
}

/// Check if a file is a parent module file (`__init__.py` or `mod.rs`).
fn is_parent_module(path: &Path) -> bool {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    file_name == "__init__.py" || file_name == "mod.rs"
}

/// Check if a file is a direct sibling (same directory, not nested deeper).
fn is_sibling(file: &Path, parent_dir: &Path) -> bool {
    match file.parent() {
        Some(dir) => dir == parent_dir,
        None => false,
    }
}

/// Collect names of symbols that the parent module re-exports (by import annotation).
fn collect_reexported_names(graph: &Graph, parent_path: &Path) -> HashSet<String> {
    let mut names = HashSet::new();

    for id in graph.symbols_in_file(parent_path) {
        if let Some(symbol) = graph.get_symbol(id) {
            // Import symbols in the parent module indicate re-exports
            if symbol.annotations.contains(&"import".to_string()) {
                names.insert(symbol.name.clone());
            }
        }
    }

    names
}

/// Collect public symbols from a submodule file that are candidates for re-export.
///
/// Returns (name, qualified_name, line) tuples. Excludes Module symbols
/// (structural), private/crate symbols, and import symbols.
fn collect_public_symbols(graph: &Graph, file_path: &Path) -> Vec<(String, String, u32)> {
    let mut symbols = Vec::new();

    // Build a set of symbol names that have "import" annotation in this file
    // to exclude them
    let import_names: HashSet<String> = graph
        .symbols_in_file(file_path)
        .iter()
        .filter_map(|id| graph.get_symbol(*id))
        .filter(|s| s.annotations.contains(&"import".to_string()))
        .map(|s| s.name.clone())
        .collect();

    // Collect symbol names to track what we've already added (dedup by name)
    let mut seen_names: HashMap<String, usize> = HashMap::new();

    for id in graph.symbols_in_file(file_path) {
        let Some(symbol) = graph.get_symbol(id) else {
            continue;
        };

        // Skip non-public symbols
        if symbol.visibility != Visibility::Public {
            continue;
        }

        // Skip Module symbols (structural, not re-exportable)
        if symbol.kind == SymbolKind::Module {
            continue;
        }

        // Skip import symbols (they're imports, not definitions)
        if symbol.annotations.contains(&"import".to_string()) {
            continue;
        }

        // Skip symbols whose names match an import in this file
        // (they might be re-imported, not defined here)
        if import_names.contains(&symbol.name) {
            continue;
        }

        // Deduplicate by name — only report first occurrence
        if let std::collections::hash_map::Entry::Vacant(e) = seen_names.entry(symbol.name.clone())
        {
            e.insert(symbols.len());
            symbols.push((
                symbol.name.clone(),
                symbol.qualified_name.clone(),
                symbol.location.line,
            ));
        }
    }

    symbols
}
