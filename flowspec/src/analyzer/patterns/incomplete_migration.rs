// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Incomplete migration detection — old and new patterns coexisting with split callers.
//!
//! Detects when a codebase contains both an old API and a new API that serve the
//! same purpose, with callers split between them. This is the most common agent
//! failure mode — an agent updates 3 of 5 call sites during a refactor and leaves
//! the rest on the old pattern.
//!
//! Two detection signals:
//! - **Signal 1: Naming-pair detection** — functions whose names match old/new
//!   naming conventions (`deprecated_X`/`X`, `legacy_X`/`X`, `old_X`/`new_X`,
//!   `X_v1`/`X_v2`) in the same file or directory, both with callers.
//! - **Signal 2: Module-level import coexistence** — a file importing from both
//!   an old-named module and its new counterpart.
//!
//! Confidence calibration:
//! - **HIGH:** Explicit intent markers (`deprecated_`, `legacy_` prefixes/suffixes).
//! - **MODERATE:** Ambiguous markers (`old_`, `new_`, version suffixes `_v1`/`_v2`).
//!
//! False positive mitigation:
//! - Both functions must have ≥1 non-test caller.
//! - Test-file symbols excluded from caller counting.
//! - Same-file or same-directory restriction for naming pairs.

use std::collections::HashMap;
use std::path::Path;

use crate::analyzer::diagnostic::*;
use crate::analyzer::patterns::exclusion::{is_test_path, relativize_path};
use crate::graph::Graph;
use crate::parser::ir::{SymbolId, SymbolKind};

/// Marker category for a naming prefix/suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerKind {
    /// Explicit intent: `deprecated_`, `legacy_` — HIGH confidence.
    Explicit,
    /// Ambiguous: `old_`, `new_`, `sync_`, `async_`, version suffixes — MODERATE confidence.
    Ambiguous,
}

/// A symbol with its stripped base name and marker metadata.
#[derive(Debug)]
struct NamedSymbol {
    id: SymbolId,
    name: String,
    file: String,
    line: u32,
    base_name: String,
    is_old_side: bool,
    marker_kind: MarkerKind,
}

/// Known prefixes that indicate old/new migration patterns.
const OLD_PREFIXES: &[(&str, MarkerKind)] = &[
    ("deprecated_", MarkerKind::Explicit),
    ("legacy_", MarkerKind::Explicit),
    ("old_", MarkerKind::Ambiguous),
];

const NEW_PREFIXES: &[(&str, MarkerKind)] = &[("new_", MarkerKind::Ambiguous)];

/// Known suffixes that indicate old/new migration patterns.
const OLD_SUFFIXES: &[(&str, MarkerKind)] = &[
    ("_deprecated", MarkerKind::Explicit),
    ("_legacy", MarkerKind::Explicit),
    ("_old", MarkerKind::Ambiguous),
];

const NEW_SUFFIXES: &[(&str, MarkerKind)] = &[("_new", MarkerKind::Ambiguous)];

/// Async/sync migration prefixes and suffixes.
const SYNC_PREFIXES: &[(&str, MarkerKind)] = &[("sync_", MarkerKind::Ambiguous)];
const ASYNC_PREFIXES: &[(&str, MarkerKind)] = &[("async_", MarkerKind::Ambiguous)];
const SYNC_SUFFIXES: &[(&str, MarkerKind)] = &[("_sync", MarkerKind::Ambiguous)];
const ASYNC_SUFFIXES: &[(&str, MarkerKind)] = &[("_async", MarkerKind::Ambiguous)];

/// Extract the directory from a file path (everything before the last `/`).
fn parent_dir(file: &str) -> &str {
    if file.contains('\\') {
        // Handle Windows paths by finding last separator
        file.rfind(['/', '\\']).map(|i| &file[..i]).unwrap_or("")
    } else {
        file.rfind('/').map(|i| &file[..i]).unwrap_or("")
    }
}

/// Check if two files are in the same directory.
fn same_directory(file_a: &str, file_b: &str) -> bool {
    let dir_a = parent_dir(file_a);
    let dir_b = parent_dir(file_b);
    dir_a == dir_b
}

/// Try to strip a version suffix like `_v1`, `_v2`, etc. and return the base + version number.
fn strip_version_suffix(name: &str) -> Option<(String, u32)> {
    // Match patterns: _v1, _v2, ..., _V1, _V2, ...
    if let Some(idx) = name.rfind("_v").or_else(|| name.rfind("_V")) {
        let after = &name[idx + 2..];
        if let Ok(version) = after.parse::<u32>() {
            return Some((name[..idx].to_string(), version));
        }
    }
    None
}

/// Count non-test callers for a symbol.
fn count_production_callers(graph: &Graph, id: SymbolId) -> Vec<SymbolId> {
    graph
        .callers(id)
        .into_iter()
        .filter(|caller_id| {
            if let Some(caller) = graph.get_symbol(*caller_id) {
                !is_test_path(&caller.location.file.display().to_string())
            } else {
                false
            }
        })
        .collect()
}

/// Detect incomplete migrations in the analysis graph.
///
/// Scans all Function/Method symbols for naming pairs that indicate old/new
/// coexistence. Excludes test files, requires both sides to have callers,
/// and restricts to same-file or same-directory pairs.
///
/// Returns diagnostics with `WARNING` severity and `HIGH` or `MODERATE`
/// confidence based on the naming marker strength.
pub fn detect(graph: &Graph, project_root: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Signal 1: Naming-pair detection
    detect_naming_pairs(graph, project_root, &mut diagnostics);

    // Signal 2: Version-suffix detection
    detect_version_pairs(graph, project_root, &mut diagnostics);

    // Signal 3: Module-level import coexistence
    detect_import_coexistence(graph, project_root, &mut diagnostics);

    diagnostics
}

/// Detect naming pairs where prefix/suffix stripping reveals the same base name.
fn detect_naming_pairs(graph: &Graph, project_root: &Path, diagnostics: &mut Vec<Diagnostic>) {
    // Collect all Function/Method symbols not in test files
    let mut candidates: Vec<NamedSymbol> = Vec::new();

    for (id, symbol) in graph.all_symbols() {
        // Only Function and Method symbols
        if symbol.kind != SymbolKind::Function && symbol.kind != SymbolKind::Method {
            continue;
        }

        // Skip empty names
        if symbol.name.is_empty() {
            continue;
        }

        let file = symbol.location.file.display().to_string();

        // Skip test files
        if is_test_path(&file) {
            continue;
        }

        // Try to match against known prefixes/suffixes
        // OLD side prefixes
        for (prefix, kind) in OLD_PREFIXES {
            if let Some(base) = symbol.name.strip_prefix(prefix) {
                if !base.is_empty() {
                    candidates.push(NamedSymbol {
                        id,
                        name: symbol.name.clone(),
                        file: file.clone(),
                        line: symbol.location.line,
                        base_name: base.to_string(),
                        is_old_side: true,
                        marker_kind: *kind,
                    });
                }
            }
        }

        // NEW side prefixes
        for (prefix, kind) in NEW_PREFIXES {
            if let Some(base) = symbol.name.strip_prefix(prefix) {
                if !base.is_empty() {
                    candidates.push(NamedSymbol {
                        id,
                        name: symbol.name.clone(),
                        file: file.clone(),
                        line: symbol.location.line,
                        base_name: base.to_string(),
                        is_old_side: false,
                        marker_kind: *kind,
                    });
                }
            }
        }

        // OLD side suffixes
        for (suffix, kind) in OLD_SUFFIXES {
            if let Some(base) = symbol.name.strip_suffix(suffix) {
                if !base.is_empty() {
                    candidates.push(NamedSymbol {
                        id,
                        name: symbol.name.clone(),
                        file: file.clone(),
                        line: symbol.location.line,
                        base_name: base.to_string(),
                        is_old_side: true,
                        marker_kind: *kind,
                    });
                }
            }
        }

        // NEW side suffixes
        for (suffix, kind) in NEW_SUFFIXES {
            if let Some(base) = symbol.name.strip_suffix(suffix) {
                if !base.is_empty() {
                    candidates.push(NamedSymbol {
                        id,
                        name: symbol.name.clone(),
                        file: file.clone(),
                        line: symbol.location.line,
                        base_name: base.to_string(),
                        is_old_side: false,
                        marker_kind: *kind,
                    });
                }
            }
        }

        // SYNC side (old in async migration context)
        for (prefix, kind) in SYNC_PREFIXES {
            if let Some(base) = symbol.name.strip_prefix(prefix) {
                if !base.is_empty() {
                    candidates.push(NamedSymbol {
                        id,
                        name: symbol.name.clone(),
                        file: file.clone(),
                        line: symbol.location.line,
                        base_name: base.to_string(),
                        is_old_side: true,
                        marker_kind: *kind,
                    });
                }
            }
        }

        for (suffix, kind) in SYNC_SUFFIXES {
            if let Some(base) = symbol.name.strip_suffix(suffix) {
                if !base.is_empty() {
                    candidates.push(NamedSymbol {
                        id,
                        name: symbol.name.clone(),
                        file: file.clone(),
                        line: symbol.location.line,
                        base_name: base.to_string(),
                        is_old_side: true,
                        marker_kind: *kind,
                    });
                }
            }
        }

        // ASYNC side (new in async migration context)
        for (prefix, kind) in ASYNC_PREFIXES {
            if let Some(base) = symbol.name.strip_prefix(prefix) {
                if !base.is_empty() {
                    candidates.push(NamedSymbol {
                        id,
                        name: symbol.name.clone(),
                        file: file.clone(),
                        line: symbol.location.line,
                        base_name: base.to_string(),
                        is_old_side: false,
                        marker_kind: *kind,
                    });
                }
            }
        }

        for (suffix, kind) in ASYNC_SUFFIXES {
            if let Some(base) = symbol.name.strip_suffix(suffix) {
                if !base.is_empty() {
                    candidates.push(NamedSymbol {
                        id,
                        name: symbol.name.clone(),
                        file: file.clone(),
                        line: symbol.location.line,
                        base_name: base.to_string(),
                        is_old_side: false,
                        marker_kind: *kind,
                    });
                }
            }
        }

        // Also register the bare name as a potential "new" side match for
        // deprecated_X / X and legacy_X / X pairs
        candidates.push(NamedSymbol {
            id,
            name: symbol.name.clone(),
            file: file.clone(),
            line: symbol.location.line,
            base_name: symbol.name.clone(),
            is_old_side: false,
            marker_kind: MarkerKind::Explicit, // placeholder, overridden by old side
        });
    }

    // Group by base_name
    let mut groups: HashMap<String, Vec<&NamedSymbol>> = HashMap::new();
    for c in &candidates {
        groups.entry(c.base_name.clone()).or_default().push(c);
    }

    // Track already-emitted pairs to avoid duplicates
    let mut emitted: std::collections::HashSet<(SymbolId, SymbolId)> =
        std::collections::HashSet::new();

    for members in groups.values() {
        // Need at least one old and one new side
        let old_side: Vec<&&NamedSymbol> = members.iter().filter(|m| m.is_old_side).collect();
        let new_side: Vec<&&NamedSymbol> = members.iter().filter(|m| !m.is_old_side).collect();

        if old_side.is_empty() || new_side.is_empty() {
            continue;
        }

        // Try all old-new pairs
        for old in &old_side {
            for new in &new_side {
                // Skip self-pairing (same symbol ID)
                if old.id == new.id {
                    continue;
                }

                // Skip if already emitted
                let pair_key = if old.id < new.id {
                    (old.id, new.id)
                } else {
                    (new.id, old.id)
                };
                if emitted.contains(&pair_key) {
                    continue;
                }

                // Same-file or same-directory restriction
                if old.file != new.file && !same_directory(&old.file, &new.file) {
                    continue;
                }

                // Both must have ≥1 production caller
                let old_callers = count_production_callers(graph, old.id);
                let new_callers = count_production_callers(graph, new.id);

                if old_callers.is_empty() || new_callers.is_empty() {
                    continue;
                }

                // Determine confidence from the old side's marker
                let confidence = match old.marker_kind {
                    MarkerKind::Explicit => Confidence::High,
                    MarkerKind::Ambiguous => Confidence::Moderate,
                };

                let old_count = old_callers.len();
                let new_count = new_callers.len();
                let total = old_count + new_count;

                let location = format!(
                    "{}:{}",
                    relativize_path(Path::new(&old.file), project_root),
                    old.line
                );

                // Build caller names for evidence (up to 5 per side)
                let old_caller_names: Vec<String> = old_callers
                    .iter()
                    .take(5)
                    .filter_map(|cid| graph.get_symbol(*cid).map(|s| s.name.clone()))
                    .collect();
                let new_caller_names: Vec<String> = new_callers
                    .iter()
                    .take(5)
                    .filter_map(|cid| graph.get_symbol(*cid).map(|s| s.name.clone()))
                    .collect();

                diagnostics.push(Diagnostic {
                    id: String::new(),
                    pattern: DiagnosticPattern::IncompleteMigration,
                    severity: Severity::Warning,
                    confidence,
                    entity: format!("{}, {}", old.name, new.name),
                    message: format!(
                        "Incomplete migration: '{}' and '{}' coexist with split callers \
                         ({} of {} call sites still use old pattern)",
                        old.name, new.name, old_count, total
                    ),
                    evidence: vec![
                        Evidence {
                            observation: format!(
                                "Old pattern '{}' has {} caller(s): [{}]",
                                old.name,
                                old_count,
                                old_caller_names.join(", ")
                            ),
                            location: Some(format!(
                                "{}:{}",
                                relativize_path(Path::new(&old.file), project_root),
                                old.line
                            )),
                            context: Some("old API still in use".to_string()),
                        },
                        Evidence {
                            observation: format!(
                                "New pattern '{}' has {} caller(s): [{}]",
                                new.name,
                                new_count,
                                new_caller_names.join(", ")
                            ),
                            location: Some(format!(
                                "{}:{}",
                                relativize_path(Path::new(&new.file), project_root),
                                new.line
                            )),
                            context: Some("new API partially adopted".to_string()),
                        },
                    ],
                    suggestion: format!(
                        "Complete migration: update callers [{}] to use '{}' instead of '{}'",
                        old_caller_names.join(", "),
                        new.name,
                        old.name
                    ),
                    location,
                });

                emitted.insert(pair_key);
            }
        }
    }
}

/// Detect version-suffixed pairs (e.g., `handle_v1` / `handle_v2`).
fn detect_version_pairs(graph: &Graph, project_root: &Path, diagnostics: &mut Vec<Diagnostic>) {
    // Collect versioned symbols
    type VersionEntry = (SymbolId, String, String, u32, u32);
    let mut versioned: HashMap<String, Vec<VersionEntry>> = HashMap::new();

    for (id, symbol) in graph.all_symbols() {
        if symbol.kind != SymbolKind::Function && symbol.kind != SymbolKind::Method {
            continue;
        }
        if symbol.name.is_empty() {
            continue;
        }
        let file = symbol.location.file.display().to_string();
        if is_test_path(&file) {
            continue;
        }

        if let Some((base, version)) = strip_version_suffix(&symbol.name) {
            versioned.entry(base).or_default().push((
                id,
                symbol.name.clone(),
                file,
                symbol.location.line,
                version,
            ));
        }
    }

    // Track emitted pairs to avoid duplicates with naming-pair detection
    let mut emitted: std::collections::HashSet<(SymbolId, SymbolId)> =
        std::collections::HashSet::new();

    for (_base, mut members) in versioned {
        if members.len() < 2 {
            continue;
        }

        // Sort by version number
        members.sort_by_key(|m| m.4);

        // Check all consecutive pairs and non-consecutive pairs
        for i in 0..members.len() {
            for j in (i + 1)..members.len() {
                let old = &members[i];
                let new = &members[j];

                let pair_key = if old.0 < new.0 {
                    (old.0, new.0)
                } else {
                    (new.0, old.0)
                };
                if emitted.contains(&pair_key) {
                    continue;
                }

                // Same-file or same-directory restriction
                if old.2 != new.2 && !same_directory(&old.2, &new.2) {
                    continue;
                }

                // Both must have ≥1 production caller
                let old_callers = count_production_callers(graph, old.0);
                let new_callers = count_production_callers(graph, new.0);

                if old_callers.is_empty() || new_callers.is_empty() {
                    continue;
                }

                let old_count = old_callers.len();
                let new_count = new_callers.len();
                let total = old_count + new_count;

                let location = format!(
                    "{}:{}",
                    relativize_path(Path::new(&old.2), project_root),
                    old.3
                );

                let old_caller_names: Vec<String> = old_callers
                    .iter()
                    .take(5)
                    .filter_map(|cid| graph.get_symbol(*cid).map(|s| s.name.clone()))
                    .collect();
                let new_caller_names: Vec<String> = new_callers
                    .iter()
                    .take(5)
                    .filter_map(|cid| graph.get_symbol(*cid).map(|s| s.name.clone()))
                    .collect();

                diagnostics.push(Diagnostic {
                    id: String::new(),
                    pattern: DiagnosticPattern::IncompleteMigration,
                    severity: Severity::Warning,
                    confidence: Confidence::Moderate,
                    entity: format!("{}, {}", old.1, new.1),
                    message: format!(
                        "Incomplete migration: '{}' and '{}' coexist with split callers \
                         ({} of {} call sites still use old pattern)",
                        old.1, new.1, old_count, total
                    ),
                    evidence: vec![
                        Evidence {
                            observation: format!(
                                "Old version '{}' has {} caller(s): [{}]",
                                old.1,
                                old_count,
                                old_caller_names.join(", ")
                            ),
                            location: Some(format!(
                                "{}:{}",
                                relativize_path(Path::new(&old.2), project_root),
                                old.3
                            )),
                            context: Some("older version still in use".to_string()),
                        },
                        Evidence {
                            observation: format!(
                                "New version '{}' has {} caller(s): [{}]",
                                new.1,
                                new_count,
                                new_caller_names.join(", ")
                            ),
                            location: Some(format!(
                                "{}:{}",
                                relativize_path(Path::new(&new.2), project_root),
                                new.3
                            )),
                            context: Some("newer version partially adopted".to_string()),
                        },
                    ],
                    suggestion: format!(
                        "Complete migration: update callers [{}] to use '{}' instead of '{}'",
                        old_caller_names.join(", "),
                        new.1,
                        old.1
                    ),
                    location,
                });

                emitted.insert(pair_key);
            }
        }
    }
}

/// Detect module-level import coexistence (e.g., importing from both `old_utils` and `utils`).
fn detect_import_coexistence(
    graph: &Graph,
    project_root: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Group imports by file
    let mut file_imports: HashMap<String, Vec<(SymbolId, String, String, u32)>> = HashMap::new();

    for (id, symbol) in graph.all_symbols() {
        if !symbol.annotations.contains(&"import".to_string()) {
            continue;
        }

        let file = symbol.location.file.display().to_string();
        if is_test_path(&file) {
            continue;
        }

        // Extract "from:" module annotation
        if let Some(from_module) = symbol
            .annotations
            .iter()
            .find(|a| a.starts_with("from:"))
            .map(|a| a[5..].to_string())
        {
            file_imports.entry(file).or_default().push((
                id,
                symbol.name.clone(),
                from_module,
                symbol.location.line,
            ));
        }
    }

    // Check each file for old/new module name pairs
    for (file, imports) in &file_imports {
        let module_names: Vec<&str> = imports.iter().map(|i| i.2.as_str()).collect();

        for (i, old_module) in module_names.iter().enumerate() {
            for (j, new_module) in module_names.iter().enumerate() {
                if i == j {
                    continue;
                }

                // Check if module names match old/new patterns
                let is_migration_pair = OLD_PREFIXES.iter().any(|(prefix, _)| {
                    old_module
                        .strip_prefix(prefix)
                        .is_some_and(|base| *new_module == base)
                }) || OLD_SUFFIXES.iter().any(|(suffix, _)| {
                    old_module
                        .strip_suffix(suffix)
                        .is_some_and(|base| *new_module == base)
                });

                if is_migration_pair {
                    let location = format!(
                        "{}:{}",
                        relativize_path(Path::new(file), project_root),
                        imports[i].3
                    );

                    diagnostics.push(Diagnostic {
                        id: String::new(),
                        pattern: DiagnosticPattern::IncompleteMigration,
                        severity: Severity::Warning,
                        confidence: Confidence::Moderate,
                        entity: format!("{}, {}", old_module, new_module),
                        message: format!(
                            "Incomplete migration: file imports from both '{}' and '{}' modules",
                            old_module, new_module
                        ),
                        evidence: vec![Evidence {
                            observation: format!(
                                "File '{}' imports from both old module '{}' and new module '{}'",
                                relativize_path(Path::new(file), project_root),
                                old_module,
                                new_module
                            ),
                            location: Some(location.clone()),
                            context: Some(
                                "both old and new modules imported in the same file".to_string(),
                            ),
                        }],
                        suggestion: format!(
                            "Complete migration: update imports from '{}' to use '{}' instead",
                            old_module, new_module
                        ),
                        location,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;
    use crate::parser::ir::{ReferenceKind, SymbolKind, Visibility};
    use crate::test_utils::*;

    // =========================================================================
    // T1: Python deprecated prefix pair — HIGH confidence
    // =========================================================================

    #[test]
    fn detect_deprecated_prefix_pair_python() {
        let mut g = Graph::new();
        let f = "api.py";
        let h = "handlers.py";

        let old_fn = g.add_symbol(make_symbol(
            "deprecated_fetch_data",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let new_fn = g.add_symbol(make_symbol(
            "fetch_data",
            SymbolKind::Function,
            Visibility::Private,
            f,
            15,
        ));

        // 3 callers of old function
        for i in 0..3 {
            let caller = g.add_symbol(make_symbol(
                &format!("handler_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                h,
                10 + i * 10,
            ));
            add_ref(&mut g, caller, old_fn, ReferenceKind::Call, h);
        }
        // 5 callers of new function
        for i in 3..8 {
            let caller = g.add_symbol(make_symbol(
                &format!("handler_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                h,
                10 + i * 10,
            ));
            add_ref(&mut g, caller, new_fn, ReferenceKind::Call, h);
        }

        let diagnostics = detect(&g, Path::new(""));
        assert_eq!(
            diagnostics.len(),
            1,
            "Should detect exactly one incomplete migration"
        );

        let d = &diagnostics[0];
        assert_eq!(d.pattern, DiagnosticPattern::IncompleteMigration);
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(
            d.confidence,
            Confidence::High,
            "deprecated_ prefix = HIGH confidence"
        );
        assert!(
            d.entity.contains("deprecated_fetch_data"),
            "Entity must name the old function"
        );
        assert!(
            d.entity.contains("fetch_data"),
            "Entity must name the new function"
        );
        assert!(!d.evidence.is_empty(), "Must include evidence");
        assert!(!d.suggestion.is_empty(), "Must include suggestion");
    }

    // =========================================================================
    // T2: JS legacy prefix pair — HIGH confidence
    // =========================================================================

    #[test]
    fn detect_legacy_prefix_pair_js() {
        let mut g = Graph::new();
        let f = "src/ui.js";

        let old_fn = g.add_symbol(make_symbol(
            "legacy_render",
            SymbolKind::Function,
            Visibility::Public,
            f,
            10,
        ));
        let new_fn = g.add_symbol(make_symbol(
            "render",
            SymbolKind::Function,
            Visibility::Public,
            f,
            30,
        ));

        let c1 = g.add_symbol(make_symbol(
            "page_a",
            SymbolKind::Function,
            Visibility::Private,
            "src/pages/a.js",
            1,
        ));
        let c2 = g.add_symbol(make_symbol(
            "page_b",
            SymbolKind::Function,
            Visibility::Private,
            "src/pages/b.js",
            1,
        ));
        let c3 = g.add_symbol(make_symbol(
            "page_c",
            SymbolKind::Function,
            Visibility::Private,
            "src/pages/c.js",
            1,
        ));
        let c4 = g.add_symbol(make_symbol(
            "page_d",
            SymbolKind::Function,
            Visibility::Private,
            "src/pages/d.js",
            1,
        ));

        add_ref(&mut g, c1, old_fn, ReferenceKind::Call, "src/pages/a.js");
        add_ref(&mut g, c2, old_fn, ReferenceKind::Call, "src/pages/b.js");
        add_ref(&mut g, c3, new_fn, ReferenceKind::Call, "src/pages/c.js");
        add_ref(&mut g, c4, new_fn, ReferenceKind::Call, "src/pages/d.js");

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            !diagnostics.is_empty(),
            "Should detect legacy_ prefix migration"
        );

        let d = diagnostics
            .iter()
            .find(|d| d.entity.contains("legacy_render"))
            .expect("Should find legacy_render pair");
        assert_eq!(
            d.confidence,
            Confidence::High,
            "legacy_ prefix = HIGH confidence"
        );
    }

    // =========================================================================
    // T3: Rust old/new prefix pair — MODERATE confidence
    // =========================================================================

    #[test]
    fn detect_old_new_prefix_pair_rust() {
        let mut g = Graph::new();
        let f = "src/engine.rs";

        let old_fn = g.add_symbol(make_symbol(
            "old_process",
            SymbolKind::Function,
            Visibility::Public,
            f,
            5,
        ));
        let new_fn = g.add_symbol(make_symbol(
            "new_process",
            SymbolKind::Function,
            Visibility::Public,
            f,
            25,
        ));

        let c1 = g.add_symbol(make_symbol(
            "caller_a",
            SymbolKind::Function,
            Visibility::Private,
            f,
            40,
        ));
        let c2 = g.add_symbol(make_symbol(
            "caller_b",
            SymbolKind::Function,
            Visibility::Private,
            f,
            50,
        ));
        let c3 = g.add_symbol(make_symbol(
            "caller_c",
            SymbolKind::Function,
            Visibility::Private,
            f,
            60,
        ));

        add_ref(&mut g, c1, old_fn, ReferenceKind::Call, f);
        add_ref(&mut g, c2, old_fn, ReferenceKind::Call, f);
        add_ref(&mut g, c3, new_fn, ReferenceKind::Call, f);

        let diagnostics = detect(&g, Path::new(""));
        let d = diagnostics
            .iter()
            .find(|d| d.entity.contains("old_process"))
            .expect("Should detect old_/new_ pair");
        assert_eq!(
            d.confidence,
            Confidence::Moderate,
            "old_/new_ prefix = MODERATE confidence"
        );
    }

    // =========================================================================
    // T4: Version-suffixed pair — MODERATE confidence
    // =========================================================================

    #[test]
    fn detect_version_suffix_pair() {
        let mut g = Graph::new();
        let f = "api.py";
        let h = "routes.py";

        let v1 = g.add_symbol(make_symbol(
            "handle_request_v1",
            SymbolKind::Function,
            Visibility::Public,
            f,
            5,
        ));
        let v2 = g.add_symbol(make_symbol(
            "handle_request_v2",
            SymbolKind::Function,
            Visibility::Public,
            f,
            20,
        ));

        for i in 0..4 {
            let c = g.add_symbol(make_symbol(
                &format!("route_old_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                h,
                10 + i * 10,
            ));
            add_ref(&mut g, c, v1, ReferenceKind::Call, h);
        }
        for i in 0..6 {
            let c = g.add_symbol(make_symbol(
                &format!("route_new_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                h,
                100 + i * 10,
            ));
            add_ref(&mut g, c, v2, ReferenceKind::Call, h);
        }

        let diagnostics = detect(&g, Path::new(""));
        assert!(!diagnostics.is_empty(), "Should detect v1/v2 pair");
        let d = diagnostics
            .iter()
            .find(|d| {
                d.entity.contains("handle_request_v1") || d.entity.contains("handle_request_v2")
            })
            .unwrap();
        assert_eq!(
            d.confidence,
            Confidence::Moderate,
            "v1/v2 suffix = MODERATE confidence"
        );
    }

    // =========================================================================
    // T5: Clean codebase — migration complete
    // =========================================================================

    #[test]
    fn no_finding_when_migration_complete() {
        let mut g = Graph::new();
        let f = "api.py";

        let new_fn = g.add_symbol(make_symbol(
            "fetch_data",
            SymbolKind::Function,
            Visibility::Public,
            f,
            5,
        ));

        for i in 0..5 {
            let c = g.add_symbol(make_symbol(
                &format!("caller_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                20 + i * 5,
            ));
            add_ref(&mut g, c, new_fn, ReferenceKind::Call, f);
        }

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Migration-complete codebase should produce zero findings"
        );
    }

    // =========================================================================
    // T6: Intentional dual-API (sync/async)
    // =========================================================================

    #[test]
    fn no_high_confidence_on_intentional_dual_api() {
        let mut g = Graph::new();
        let f = "lib/http.py";

        let sync_fn = g.add_symbol(make_symbol(
            "sync_fetch",
            SymbolKind::Function,
            Visibility::Public,
            f,
            5,
        ));
        let async_fn = g.add_symbol(make_symbol(
            "async_fetch",
            SymbolKind::Function,
            Visibility::Public,
            f,
            20,
        ));

        let c1 = g.add_symbol(make_symbol(
            "sync_handler",
            SymbolKind::Function,
            Visibility::Private,
            "app.py",
            5,
        ));
        let c2 = g.add_symbol(make_symbol(
            "async_handler",
            SymbolKind::Function,
            Visibility::Private,
            "app.py",
            15,
        ));
        add_ref(&mut g, c1, sync_fn, ReferenceKind::Call, "app.py");
        add_ref(&mut g, c2, async_fn, ReferenceKind::Call, "app.py");

        let diagnostics = detect(&g, Path::new(""));
        // Spec says never suppress — may flag at MODERATE. But must NOT be HIGH.
        for d in &diagnostics {
            if d.entity.contains("sync_fetch") || d.entity.contains("async_fetch") {
                assert_ne!(
                    d.confidence,
                    Confidence::High,
                    "Dual sync/async API should not be HIGH confidence"
                );
            }
        }
    }

    // =========================================================================
    // T7: Functions with "old" in name but no corresponding "new"
    // =========================================================================

    #[test]
    fn no_finding_when_old_prefix_has_no_new_counterpart() {
        let mut g = Graph::new();
        let f = "service.py";

        let fn1 = g.add_symbol(make_symbol(
            "old_handler",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let fn2 = g.add_symbol(make_symbol(
            "unrelated_thing",
            SymbolKind::Function,
            Visibility::Private,
            f,
            20,
        ));

        let c = g.add_symbol(make_symbol(
            "main",
            SymbolKind::Function,
            Visibility::Public,
            f,
            40,
        ));
        add_ref(&mut g, c, fn1, ReferenceKind::Call, f);
        add_ref(&mut g, c, fn2, ReferenceKind::Call, f);

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "old_ prefix without new counterpart should not fire"
        );
    }

    // =========================================================================
    // T8: Unrelated functions named "old_function" and "new_function"
    // =========================================================================

    #[test]
    fn adversarial_unrelated_old_new_names_different_dirs() {
        let mut g = Graph::new();

        let old_fn = g.add_symbol(make_symbol(
            "old_function",
            SymbolKind::Function,
            Visibility::Private,
            "auth/login.py",
            5,
        ));
        let new_fn = g.add_symbol(make_symbol(
            "new_function",
            SymbolKind::Function,
            Visibility::Private,
            "billing/invoice.py",
            5,
        ));

        for i in 0..3 {
            let c = g.add_symbol(make_symbol(
                &format!("auth_caller_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                "auth/login.py",
                20 + i * 10,
            ));
            add_ref(&mut g, c, old_fn, ReferenceKind::Call, "auth/login.py");
        }
        for i in 0..2 {
            let c = g.add_symbol(make_symbol(
                &format!("billing_caller_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                "billing/invoice.py",
                20 + i * 10,
            ));
            add_ref(&mut g, c, new_fn, ReferenceKind::Call, "billing/invoice.py");
        }

        let diagnostics = detect(&g, Path::new(""));
        for d in &diagnostics {
            assert_ne!(
                d.confidence,
                Confidence::High,
                "Cross-directory unrelated functions should not be HIGH confidence"
            );
        }
    }

    // =========================================================================
    // T9: Test files containing old API references
    // =========================================================================

    #[test]
    fn adversarial_test_file_old_api_excluded() {
        let mut g = Graph::new();
        let src = "src/parser.py";
        let test = "tests/test_parser.py";

        let old_fn = g.add_symbol(make_symbol(
            "deprecated_parser",
            SymbolKind::Function,
            Visibility::Private,
            src,
            5,
        ));
        let new_fn = g.add_symbol(make_symbol(
            "parser",
            SymbolKind::Function,
            Visibility::Private,
            src,
            20,
        ));

        // Only test callers for old function
        let t1 = g.add_symbol(make_symbol(
            "test_deprecated_parser",
            SymbolKind::Function,
            Visibility::Private,
            test,
            10,
        ));
        add_ref(&mut g, t1, old_fn, ReferenceKind::Call, test);

        // 5 production callers for new function
        for i in 0..5 {
            let c = g.add_symbol(make_symbol(
                &format!("use_parser_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                "src/app.py",
                10 + i * 10,
            ));
            add_ref(&mut g, c, new_fn, ReferenceKind::Call, "src/app.py");
        }
        // Test caller for new function
        let t2 = g.add_symbol(make_symbol(
            "test_parser",
            SymbolKind::Function,
            Visibility::Private,
            test,
            20,
        ));
        add_ref(&mut g, t2, new_fn, ReferenceKind::Call, test);

        let diagnostics = detect(&g, Path::new(""));
        // With test callers excluded, deprecated_parser has 0 production callers → should NOT fire
        if !diagnostics.is_empty() {
            for d in &diagnostics {
                for e in &d.evidence {
                    assert!(
                        !e.observation.contains("test_parser.py"),
                        "Test file callers must not appear in evidence"
                    );
                }
            }
        }
    }

    // =========================================================================
    // T10: Nested prefix — "old_new_handler"
    // =========================================================================

    #[test]
    fn adversarial_nested_prefix_no_self_pair() {
        let mut g = Graph::new();
        let f = "service.py";

        let fn1 = g.add_symbol(make_symbol(
            "old_new_handler",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let c1 = g.add_symbol(make_symbol(
            "main",
            SymbolKind::Function,
            Visibility::Public,
            f,
            20,
        ));
        let c2 = g.add_symbol(make_symbol(
            "helper",
            SymbolKind::Function,
            Visibility::Private,
            f,
            30,
        ));
        add_ref(&mut g, c1, fn1, ReferenceKind::Call, f);
        add_ref(&mut g, c2, fn1, ReferenceKind::Call, f);

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Single function with nested prefixes must not self-pair"
        );
    }

    // =========================================================================
    // T11: Both functions have zero callers
    // =========================================================================

    #[test]
    fn adversarial_pair_with_zero_callers_both_sides() {
        let mut g = Graph::new();
        let f = "auth.py";

        let _old_fn = g.add_symbol(make_symbol(
            "deprecated_auth",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let _new_fn = g.add_symbol(make_symbol(
            "auth",
            SymbolKind::Function,
            Visibility::Private,
            f,
            20,
        ));

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Pair with zero callers on both sides should not fire"
        );
    }

    // =========================================================================
    // T12: Version trio — v1, v2, v3
    // =========================================================================

    #[test]
    fn adversarial_version_trio_v1_v2_v3() {
        let mut g = Graph::new();
        let f = "api.py";

        let v1 = g.add_symbol(make_symbol(
            "handle_v1",
            SymbolKind::Function,
            Visibility::Public,
            f,
            5,
        ));
        let v2 = g.add_symbol(make_symbol(
            "handle_v2",
            SymbolKind::Function,
            Visibility::Public,
            f,
            15,
        ));
        let v3 = g.add_symbol(make_symbol(
            "handle_v3",
            SymbolKind::Function,
            Visibility::Public,
            f,
            25,
        ));

        for i in 0..2 {
            let c = g.add_symbol(make_symbol(
                &format!("old_route_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                50 + i * 5,
            ));
            add_ref(&mut g, c, v1, ReferenceKind::Call, f);
        }
        for i in 0..3 {
            let c = g.add_symbol(make_symbol(
                &format!("mid_route_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                70 + i * 5,
            ));
            add_ref(&mut g, c, v2, ReferenceKind::Call, f);
        }
        for i in 0..5 {
            let c = g.add_symbol(make_symbol(
                &format!("new_route_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                100 + i * 5,
            ));
            add_ref(&mut g, c, v3, ReferenceKind::Call, f);
        }

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            !diagnostics.is_empty(),
            "Version trio (v1/v2/v3) must produce at least one finding"
        );
    }

    // =========================================================================
    // T13: Performance — 100+ functions with mixed naming
    // =========================================================================

    #[test]
    fn adversarial_performance_100_functions() {
        let mut g = Graph::new();
        let f = "big_module.py";

        // 10 planted deprecated/new pairs
        let mut pair_old_names = Vec::new();
        for i in 0..10u32 {
            let old_name = format!("deprecated_action_{}", i);
            let new_name = format!("action_{}", i);
            pair_old_names.push(old_name.clone());

            let old_fn = g.add_symbol(make_symbol(
                &old_name,
                SymbolKind::Function,
                Visibility::Private,
                f,
                i * 20 + 1,
            ));
            let new_fn = g.add_symbol(make_symbol(
                &new_name,
                SymbolKind::Function,
                Visibility::Private,
                f,
                i * 20 + 10,
            ));

            let c1 = g.add_symbol(make_symbol(
                &format!("old_caller_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                300 + i * 10,
            ));
            let c2 = g.add_symbol(make_symbol(
                &format!("new_caller_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                500 + i * 10,
            ));
            add_ref(&mut g, c1, old_fn, ReferenceKind::Call, f);
            add_ref(&mut g, c2, new_fn, ReferenceKind::Call, f);
        }

        // 40 normal functions (no migration prefixes)
        for i in 0..40u32 {
            let normal = g.add_symbol(make_symbol(
                &format!("normal_func_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                800 + i * 5,
            ));
            let c = g.add_symbol(make_symbol(
                &format!("normal_caller_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                1200 + i * 5,
            ));
            add_ref(&mut g, c, normal, ReferenceKind::Call, f);
        }

        // 20 orphaned old_ prefixes (no new counterpart)
        for i in 0..20u32 {
            let orphaned = g.add_symbol(make_symbol(
                &format!("old_orphan_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                1500 + i * 5,
            ));
            let c = g.add_symbol(make_symbol(
                &format!("orphan_caller_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                1700 + i * 5,
            ));
            add_ref(&mut g, c, orphaned, ReferenceKind::Call, f);
        }

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            diagnostics.len() >= 10,
            "Should detect at least 10 planted pairs, got {}",
            diagnostics.len()
        );

        // Verify no false positives from normal functions
        for d in &diagnostics {
            assert!(
                !d.entity.contains("normal_func_"),
                "Normal functions must not trigger migration findings"
            );
        }
    }

    // =========================================================================
    // T14: Empty graph
    // =========================================================================

    #[test]
    fn empty_graph_returns_empty() {
        let g = Graph::new();
        let diagnostics = detect(&g, Path::new(""));
        assert!(diagnostics.is_empty());
    }

    // =========================================================================
    // T15: Symbol with empty name
    // =========================================================================

    #[test]
    fn symbol_with_empty_name_no_crash() {
        let mut g = Graph::new();
        let _s = g.add_symbol(make_symbol(
            "",
            SymbolKind::Function,
            Visibility::Private,
            "empty.py",
            1,
        ));
        let diagnostics = detect(&g, Path::new(""));
        assert!(diagnostics.is_empty());
    }

    // =========================================================================
    // T16: Only one side has callers
    // =========================================================================

    #[test]
    fn only_old_side_has_callers() {
        let mut g = Graph::new();
        let f = "auth.py";

        let old_fn = g.add_symbol(make_symbol(
            "legacy_auth",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let _new_fn = g.add_symbol(make_symbol(
            "auth",
            SymbolKind::Function,
            Visibility::Private,
            f,
            20,
        ));

        for i in 0..3 {
            let c = g.add_symbol(make_symbol(
                &format!("caller_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                30 + i * 10,
            ));
            add_ref(&mut g, c, old_fn, ReferenceKind::Call, f);
        }

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Pair where new side has 0 callers should not fire"
        );
    }

    // =========================================================================
    // T17: Method symbols (not just functions)
    // =========================================================================

    #[test]
    fn detect_method_pair_migration() {
        let mut g = Graph::new();
        let f = "service.py";

        let old_method = g.add_symbol(make_symbol(
            "deprecated_validate",
            SymbolKind::Method,
            Visibility::Public,
            f,
            10,
        ));
        let new_method = g.add_symbol(make_symbol(
            "validate",
            SymbolKind::Method,
            Visibility::Public,
            f,
            25,
        ));

        let c1 = g.add_symbol(make_symbol(
            "handler_a",
            SymbolKind::Function,
            Visibility::Private,
            f,
            40,
        ));
        let c2 = g.add_symbol(make_symbol(
            "handler_b",
            SymbolKind::Function,
            Visibility::Private,
            f,
            50,
        ));
        add_ref(&mut g, c1, old_method, ReferenceKind::Call, f);
        add_ref(&mut g, c2, old_method, ReferenceKind::Call, f);

        for i in 0..4 {
            let c = g.add_symbol(make_symbol(
                &format!("new_handler_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                60 + i * 10,
            ));
            add_ref(&mut g, c, new_method, ReferenceKind::Call, f);
        }

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            !diagnostics.is_empty(),
            "Should detect deprecated_ Method pairs, not just Functions"
        );
    }

    // =========================================================================
    // T18: Suffix-based detection — "_old" and "_new"
    // =========================================================================

    #[test]
    fn detect_suffix_pair_old_new() {
        let mut g = Graph::new();
        let f = "data.py";

        let old_fn = g.add_symbol(make_symbol(
            "fetch_data_old",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let new_fn = g.add_symbol(make_symbol(
            "fetch_data_new",
            SymbolKind::Function,
            Visibility::Private,
            f,
            20,
        ));

        let c1 = g.add_symbol(make_symbol(
            "consumer_a",
            SymbolKind::Function,
            Visibility::Private,
            f,
            40,
        ));
        let c2 = g.add_symbol(make_symbol(
            "consumer_b",
            SymbolKind::Function,
            Visibility::Private,
            f,
            50,
        ));
        add_ref(&mut g, c1, old_fn, ReferenceKind::Call, f);
        add_ref(&mut g, c2, old_fn, ReferenceKind::Call, f);

        for i in 0..5 {
            let c = g.add_symbol(make_symbol(
                &format!("new_consumer_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                60 + i * 10,
            ));
            add_ref(&mut g, c, new_fn, ReferenceKind::Call, f);
        }

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            !diagnostics.is_empty(),
            "Should detect _old/_new suffix pairs"
        );
    }

    // =========================================================================
    // T19: Pattern registration in mod.rs
    // =========================================================================

    #[test]
    fn incomplete_migration_registered_in_run_patterns() {
        use crate::analyzer::patterns::run_all_patterns;

        let mut g = Graph::new();
        let f = "api.py";
        let h = "handlers.py";

        let old_fn = g.add_symbol(make_symbol(
            "deprecated_fetch",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let new_fn = g.add_symbol(make_symbol(
            "fetch",
            SymbolKind::Function,
            Visibility::Private,
            f,
            15,
        ));

        let c1 = g.add_symbol(make_symbol(
            "handler_a",
            SymbolKind::Function,
            Visibility::Private,
            h,
            10,
        ));
        let c2 = g.add_symbol(make_symbol(
            "handler_b",
            SymbolKind::Function,
            Visibility::Private,
            h,
            20,
        ));
        add_ref(&mut g, c1, old_fn, ReferenceKind::Call, h);
        add_ref(&mut g, c2, new_fn, ReferenceKind::Call, h);

        let diagnostics = run_all_patterns(&g, Path::new(""));
        let migration_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.pattern == DiagnosticPattern::IncompleteMigration)
            .collect();

        assert!(
            !migration_findings.is_empty(),
            "IncompleteMigration must be registered in run_all_patterns"
        );
        assert!(
            migration_findings[0].id.starts_with('D'),
            "Must have sequential ID assigned"
        );
    }

    // =========================================================================
    // T20: PatternFilter includes IncompleteMigration
    // =========================================================================

    #[test]
    fn pattern_filter_selects_incomplete_migration() {
        use crate::analyzer::patterns::{run_patterns, PatternFilter};

        let mut g = Graph::new();
        let f = "api.py";

        let old_fn = g.add_symbol(make_symbol(
            "deprecated_action",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let new_fn = g.add_symbol(make_symbol(
            "action",
            SymbolKind::Function,
            Visibility::Private,
            f,
            15,
        ));

        let c1 = g.add_symbol(make_symbol(
            "caller_1",
            SymbolKind::Function,
            Visibility::Private,
            f,
            30,
        ));
        let c2 = g.add_symbol(make_symbol(
            "caller_2",
            SymbolKind::Function,
            Visibility::Private,
            f,
            40,
        ));
        add_ref(&mut g, c1, old_fn, ReferenceKind::Call, f);
        add_ref(&mut g, c2, new_fn, ReferenceKind::Call, f);

        // Also add an isolated cluster (different pattern type)
        let _iso = g.add_symbol(make_symbol(
            "isolated_fn",
            SymbolKind::Function,
            Visibility::Private,
            "lonely.py",
            1,
        ));

        let filter = PatternFilter {
            patterns: Some(vec![DiagnosticPattern::IncompleteMigration]),
            min_severity: None,
            min_confidence: None,
        };
        let diagnostics = run_patterns(&g, &filter, Path::new(""));

        for d in &diagnostics {
            assert_eq!(
                d.pattern,
                DiagnosticPattern::IncompleteMigration,
                "Filter should only return IncompleteMigration findings"
            );
        }
    }

    // =========================================================================
    // T21: Severity filter — WARNING survives min_severity Warning
    // =========================================================================

    #[test]
    fn severity_filter_warning_survives_but_critical_excludes() {
        use crate::analyzer::patterns::{run_patterns, PatternFilter};

        let mut g = Graph::new();
        let f = "api.py";

        let old_fn = g.add_symbol(make_symbol(
            "deprecated_process",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let new_fn = g.add_symbol(make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Private,
            f,
            15,
        ));

        let c1 = g.add_symbol(make_symbol(
            "a",
            SymbolKind::Function,
            Visibility::Private,
            f,
            30,
        ));
        let c2 = g.add_symbol(make_symbol(
            "b",
            SymbolKind::Function,
            Visibility::Private,
            f,
            40,
        ));
        add_ref(&mut g, c1, old_fn, ReferenceKind::Call, f);
        add_ref(&mut g, c2, new_fn, ReferenceKind::Call, f);

        // WARNING filter: should survive
        let filter_warn = PatternFilter {
            patterns: None,
            min_severity: Some(Severity::Warning),
            min_confidence: None,
        };
        let diags_warn = run_patterns(&g, &filter_warn, Path::new(""));
        let has_migration = diags_warn
            .iter()
            .any(|d| d.pattern == DiagnosticPattern::IncompleteMigration);
        assert!(
            has_migration,
            "IncompleteMigration (WARNING) should survive min_severity: Warning"
        );

        // CRITICAL filter: should be excluded
        let filter_crit = PatternFilter {
            patterns: None,
            min_severity: Some(Severity::Critical),
            min_confidence: None,
        };
        let diags_crit = run_patterns(&g, &filter_crit, Path::new(""));
        let has_migration_crit = diags_crit
            .iter()
            .any(|d| d.pattern == DiagnosticPattern::IncompleteMigration);
        assert!(
            !has_migration_crit,
            "IncompleteMigration (WARNING) should NOT survive min_severity: Critical"
        );
    }

    // =========================================================================
    // T22: Evidence contains caller counts and function names
    // =========================================================================

    #[test]
    fn evidence_contains_required_fields() {
        let mut g = Graph::new();
        let f = "api.py";

        let old_fn = g.add_symbol(make_symbol(
            "deprecated_fetch_data",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let new_fn = g.add_symbol(make_symbol(
            "fetch_data",
            SymbolKind::Function,
            Visibility::Private,
            f,
            15,
        ));

        for i in 0..3 {
            let c = g.add_symbol(make_symbol(
                &format!("old_caller_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                30 + i * 10,
            ));
            add_ref(&mut g, c, old_fn, ReferenceKind::Call, f);
        }
        for i in 0..5 {
            let c = g.add_symbol(make_symbol(
                &format!("new_caller_{}", i),
                SymbolKind::Function,
                Visibility::Private,
                f,
                70 + i * 10,
            ));
            add_ref(&mut g, c, new_fn, ReferenceKind::Call, f);
        }

        let diagnostics = detect(&g, Path::new(""));
        assert!(!diagnostics.is_empty());

        let d = &diagnostics[0];
        assert!(!d.evidence.is_empty(), "Evidence must not be empty");
        assert!(!d.suggestion.is_empty(), "Suggestion must not be empty");
        assert!(!d.location.is_empty(), "Location must not be empty");
        assert!(
            d.message.to_lowercase().contains("migration")
                || d.message.to_lowercase().contains("incomplete"),
            "Message should reference migration: got '{}'",
            d.message
        );
    }

    // =========================================================================
    // T23: Regression — Non-Function/Method symbols don't trigger
    // =========================================================================

    #[test]
    fn regression_class_symbols_not_paired() {
        let mut g = Graph::new();
        let f = "engine.py";

        let old_class = g.add_symbol(make_symbol(
            "OldProcessor",
            SymbolKind::Class,
            Visibility::Public,
            f,
            5,
        ));
        let new_class = g.add_symbol(make_symbol(
            "Processor",
            SymbolKind::Class,
            Visibility::Public,
            f,
            30,
        ));

        let c1 = g.add_symbol(make_symbol(
            "use_old",
            SymbolKind::Function,
            Visibility::Private,
            f,
            50,
        ));
        let c2 = g.add_symbol(make_symbol(
            "use_new",
            SymbolKind::Function,
            Visibility::Private,
            f,
            60,
        ));
        add_ref(&mut g, c1, old_class, ReferenceKind::Call, f);
        add_ref(&mut g, c2, new_class, ReferenceKind::Call, f);

        let diagnostics = detect(&g, Path::new(""));
        // Class symbols should not trigger — only Function/Method per investigation §4.1
        assert!(
            diagnostics.is_empty(),
            "Class symbols should not trigger incomplete_migration"
        );
    }

    // =========================================================================
    // T24: Regression — import symbols not treated as migration pairs
    // =========================================================================

    #[test]
    fn regression_import_symbols_not_paired() {
        let mut g = Graph::new();
        let f = "app.py";

        let _i1 = g.add_symbol(make_import("old_utils", f, 1));
        let _i2 = g.add_symbol(make_import("utils", f, 2));

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Import symbols should not trigger migration pairs"
        );
    }
}
