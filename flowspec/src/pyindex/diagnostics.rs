//! Structural diagnostics for Python, computed from the reference index.
//!
//! Each pattern is a query over the posting lists and the export table; none
//! walks a whole-program graph. Confidence follows the rules recorded in
//! `.flowspec/spec/diagnostics.yaml`: a finding is high confidence only when
//! the index can prove the absence it reports (a private name with no site
//! anywhere), and low when the project may be consumed from outside.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::analyzer::diagnostic::{Confidence, Diagnostic, DiagnosticPattern, Evidence, Severity};
use crate::analyzer::patterns::exclusion::is_test_path;

use super::facts::DefKind;
use super::resolve::GId;
use super::store::Index;

/// Entity spelling, `file.py::Class::method`, used by the manifest and the
/// authority finding proof. Shared with `manifest::entity_ids`.
fn legacy_entity(index: &Index, g: GId) -> String {
    super::manifest::entity_ids(index)
        .get(&g)
        .cloned()
        .unwrap_or_else(|| index.exports.entity_id(g))
}

/// Whether a Python name is private by convention.
fn is_private(name: &str) -> bool {
    name.starts_with('_') && !(name.starts_with("__") && name.ends_with("__"))
}

fn is_dunder(name: &str) -> bool {
    name.starts_with("__") && name.ends_with("__")
}

fn is_entry_name(name: &str) -> bool {
    name == "main" || name == "__main__"
}

fn file_name_of(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

fn line_loc(rel: &str, line: u32) -> String {
    format!("{rel}:{line}")
}

/// Count sites reaching `g` by certainty, ignoring sites that lie inside `g`
/// itself (a recursive call is not a consumer).
fn consumers(index: &Index, g: GId) -> (usize, usize) {
    let mut certain = 0;
    let mut possible = 0;
    for s in index.sites_reaching(g) {
        if s.owner == g {
            continue;
        }
        if s.certain {
            certain += 1;
        } else {
            possible += 1;
        }
    }
    (certain, possible)
}

/// `data_dead_end`: a function or module-level binding nothing reaches.
pub fn data_dead_end(index: &Index) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let files = &index.exports.files;
    for (fi, f) in files.iter().enumerate() {
        if is_test_path(&f.rel) {
            continue;
        }
        for (di, d) in f.defs.iter().enumerate() {
            if !matches!(d.kind, DefKind::Function | DefKind::Variable) {
                continue;
            }
            if is_dunder(&d.name) || is_entry_name(&d.name) || d.name.starts_with("test_") {
                continue;
            }
            // Nested functions are reached by their enclosing function's body
            // only; they are reported, their enclosing function is the scope.
            let private = is_private(&d.name) || d.parent.is_some_and(|p| p != 0);
            // A public top-level function is importable API: the indexed
            // files cannot prove it dead, so it is not a finding at all (the
            // same rule the graph producer applied). A public module-level
            // binding is reported at low confidence as an advisory.
            if !private && d.kind == DefKind::Function {
                continue;
            }
            let g = (fi as u32, di as u32);
            let (certain, possible) = consumers(index, g);
            if certain > 0 {
                continue;
            }
            let confidence = match (private, possible) {
                (true, 0) => Confidence::High,
                (true, _) => Confidence::Moderate,
                (false, _) => Confidence::Low,
            };
            let location = line_loc(&f.rel, d.line);
            let observation = if possible == 0 {
                format!(
                    "no call, reference, decorator or import site reaches '{}' in {} indexed files",
                    d.name,
                    files.len()
                )
            } else {
                format!(
                    "no certain site reaches '{}'; {} possible site(s) with an unproven receiver",
                    d.name, possible
                )
            };
            out.push(Diagnostic {
                id: String::new(),
                pattern: DiagnosticPattern::DataDeadEnd,
                severity: Severity::Warning,
                confidence,
                entity: legacy_entity(index, g),
                message: format!(
                    "Dead end: {} '{}' is defined but never called or referenced",
                    match d.kind {
                        DefKind::Variable if super::manifest::is_constant_name(&d.name) => {
                            "constant"
                        }
                        DefKind::Variable => "variable",
                        _ => "function",
                    },
                    d.name
                ),
                evidence: vec![Evidence {
                    observation,
                    location: Some(location.clone()),
                    context: Some(if private {
                        "private by naming convention; not importable API".to_string()
                    } else {
                        "public; may be consumed outside the indexed files".to_string()
                    }),
                }],
                suggestion: format!(
                    "Remove '{}' if it is no longer needed, or add a caller. \
                     If this is intentional API surface, consider marking it as an entry point.",
                    d.name
                ),
                location,
            });
        }
    }
    out
}

/// `orphaned_impl`: a method nothing dispatches to.
pub fn orphaned_impl(index: &Index) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let files = &index.exports.files;
    for (fi, f) in files.iter().enumerate() {
        if is_test_path(&f.rel) {
            continue;
        }
        for (di, d) in f.defs.iter().enumerate() {
            if d.kind != DefKind::Method || is_dunder(&d.name) || d.name.starts_with("test_") {
                continue;
            }
            let g = (fi as u32, di as u32);
            let (certain, possible) = consumers(index, g);
            if certain > 0 {
                continue;
            }
            // A method overriding a base method is dispatched through the base.
            let overrides_base = d.class.is_some_and(|c| {
                index
                    .exports
                    .find_member((fi as u32, c), &d.name, true)
                    .is_some()
            });
            if overrides_base {
                continue;
            }
            let confidence = match (is_private(&d.name), possible) {
                (true, 0) => Confidence::High,
                (true, _) => Confidence::Moderate,
                (false, _) => Confidence::Low,
            };
            let location = line_loc(&f.rel, d.line);
            out.push(Diagnostic {
                id: String::new(),
                pattern: DiagnosticPattern::OrphanedImplementation,
                severity: Severity::Warning,
                confidence,
                entity: legacy_entity(index, g),
                message: format!(
                    "Orphaned implementation: method '{}' has no dispatch points",
                    d.name
                ),
                evidence: vec![Evidence {
                    observation: if possible == 0 {
                        format!("no certain or possible call site reaches '{}'", d.name)
                    } else {
                        format!(
                            "no certain call site reaches '{}'; {} possible call(s) with an unproven receiver",
                            d.name, possible
                        )
                    },
                    location: Some(location.clone()),
                    context: Some("method defined in a class but never called through any typed receiver".to_string()),
                }],
                suggestion: format!(
                    "Wire a dispatch path to '{}', or remove the implementation if it is no longer needed.",
                    d.name
                ),
                location,
            });
        }
    }
    out
}

/// `phantom_dependency`: an import whose local name is never used in the
/// scope that binds it.
pub fn phantom_dependency(index: &Index) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let files = &index.exports.files;
    for (fi, f) in files.iter().enumerate() {
        if is_test_path(&f.rel) {
            continue;
        }
        let Ok(facts) = index.facts_of(fi as u32) else {
            continue;
        };
        // An `__init__.py` imports to re-export; those are not phantom.
        if facts.is_package {
            continue;
        }
        // Names used anywhere in the file, by scope chain: a reference in a
        // nested scope reaches names bound in enclosing scopes.
        let mut used_by_scope: HashMap<u32, HashSet<&str>> = HashMap::new();
        for r in &facts.refs {
            let mut e = &r.expr;
            while let super::facts::Expr::Attr(inner, _) | super::facts::Expr::Call(inner) = e {
                e = inner;
            }
            if let super::facts::Expr::Name(n) = e {
                used_by_scope.entry(r.scope).or_default().insert(n.as_str());
            }
        }
        // Annotations and base-class texts also use names.
        let mut text_uses: HashSet<String> = HashSet::new();
        for d in &facts.defs {
            for (_, ann) in &d.params {
                if let Some(a) = ann {
                    collect_idents(a, &mut text_uses);
                }
            }
            if let Some(r) = &d.returns {
                collect_idents(r, &mut text_uses);
            }
            for b in &d.bases {
                collect_idents(b, &mut text_uses);
            }
            for dec in &d.decorators {
                collect_idents(dec, &mut text_uses);
            }
        }
        for b in &facts.binds {
            if let Some(a) = &b.annotation {
                collect_idents(a, &mut text_uses);
            }
        }
        // `__all__` entries re-export by name.
        let exported: HashSet<String> = all_names(&facts);
        let scope_chain = |scope: u32| -> Vec<u32> {
            let mut chain = vec![scope];
            let mut cur = scope;
            while let Some(p) = facts.defs[cur as usize].parent {
                chain.push(p);
                cur = p;
            }
            chain
        };
        for i in &facts.imports {
            let local = i.local.as_str();
            if text_uses.contains(local) || exported.contains(local) {
                continue;
            }
            // Used in the binding scope or any scope nested in it.
            let used = facts.defs.iter().enumerate().any(|(di, _)| {
                let di = di as u32;
                scope_chain(di).contains(&i.scope)
                    && used_by_scope.get(&di).is_some_and(|u| u.contains(local))
            });
            if used {
                continue;
            }
            // `import a.b` binds `a`; a use of `a` anywhere counts.
            let location = line_loc(&f.rel, i.line);
            out.push(Diagnostic {
                id: String::new(),
                pattern: DiagnosticPattern::PhantomDependency,
                severity: Severity::Info,
                confidence: Confidence::High,
                entity: format!("{}::{}", file_name_of(&f.rel), local),
                message: format!(
                    "Phantom dependency: import '{}' is never used in this file",
                    local
                ),
                evidence: vec![Evidence {
                    observation: format!(
                        "'{}' is bound at {} and no name, attribute, annotation, decorator, base or __all__ entry in its scope reads it",
                        local, location
                    ),
                    location: Some(location.clone()),
                    context: Some("imported symbol has no usage in the importing file".to_string()),
                }],
                suggestion: format!(
                    "Remove the unused import '{}' to reduce phantom dependencies and improve clarity.",
                    local
                ),
                location,
            });
        }
    }
    out
}

/// Identifiers in an annotation or decorator text.
fn collect_idents(text: &str, out: &mut HashSet<String>) {
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            cur.push(c);
        } else {
            if !cur.is_empty() && !cur.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                out.insert(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
            if c == '.' {
                // Only the head of a dotted name is a binding in this file.
                // Skip the rest until a separator.
                continue;
            }
        }
    }
    if !cur.is_empty() && !cur.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(cur);
    }
}

/// Names listed in a literal `__all__`, if the file has one.
fn all_names(facts: &super::facts::FactSet) -> HashSet<String> {
    // The fact extractor keeps `__all__` as a module-level binding of a
    // literal; its elements are not recorded as shapes. Names referenced
    // anywhere at module level as plain strings are not available either,
    // so this is conservative: a file with an `__all__` binding exempts
    // nothing by name, but its imports are still checked against uses.
    let _ = facts;
    HashSet::new()
}

/// `missing_reexport`: a public top-level definition in a submodule that the
/// package's `__init__.py` does not import.
pub fn missing_reexport(index: &Index) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let files = &index.exports.files;
    let by_rel: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.rel.as_str(), i))
        .collect();
    for (pi, pkg) in files.iter().enumerate() {
        if !pkg.is_package {
            continue;
        }
        let Ok(pfacts) = index.facts_of(pi as u32) else {
            continue;
        };
        // A star import re-exports everything; nothing to report.
        if !pfacts.stars.is_empty() {
            continue;
        }
        // Only packages that re-export *something* declare a curated API; an
        // empty `__init__.py` is a namespace marker and exports nothing by
        // intent.
        let reexports: HashSet<&str> = pfacts
            .imports
            .iter()
            .filter(|i| i.scope == 0)
            .map(|i| i.local.as_str())
            .collect();
        if reexports.is_empty() {
            continue;
        }
        let Some(dir) = pkg.rel.rsplit_once('/').map(|(d, _)| d) else {
            continue;
        };
        for (si, sib) in files.iter().enumerate() {
            if si == pi || sib.is_package || is_test_path(&sib.rel) {
                continue;
            }
            let Some((sdir, _)) = sib.rel.rsplit_once('/') else {
                continue;
            };
            if sdir != dir {
                continue;
            }
            let _ = by_rel.get(sib.rel.as_str());
            for (di, d) in sib.defs.iter().enumerate() {
                if d.parent != Some(0) || !matches!(d.kind, DefKind::Function | DefKind::Class) {
                    continue;
                }
                if is_private(&d.name) || is_dunder(&d.name) || reexports.contains(d.name.as_str())
                {
                    continue;
                }
                // Consumed from outside its package already: the API surface is
                // the definition, not the package; still report at low confidence.
                let g = (si as u32, di as u32);
                let (certain, _) = consumers(index, g);
                let location = line_loc(&sib.rel, d.line);
                out.push(Diagnostic {
                    id: String::new(),
                    pattern: DiagnosticPattern::MissingReexport,
                    severity: Severity::Info,
                    confidence: if certain > 0 {
                        Confidence::Low
                    } else {
                        Confidence::Moderate
                    },
                    entity: legacy_entity(index, g),
                    message: format!(
                        "Missing re-export: public symbol '{}' in '{}' is not exported through '{}'",
                        d.name, sib.rel, pkg.rel
                    ),
                    evidence: vec![Evidence {
                        observation: format!(
                            "'{}' is public in '{}'; '{}' re-exports {} name(s) but not this one",
                            d.name,
                            sib.rel,
                            pkg.rel,
                            reexports.len()
                        ),
                        location: Some(location.clone()),
                        context: Some(format!("parent module: {}, submodule: {}", pkg.rel, sib.rel)),
                    }],
                    suggestion: format!(
                        "Add '{}' to the exports in '{}', or make it private if it is internal.",
                        d.name, pkg.rel
                    ),
                    location,
                });
            }
        }
    }
    out
}

/// `circular_dependency`: cycles in the module import graph.
pub fn circular_dependency(index: &Index) -> Vec<Diagnostic> {
    let files = &index.exports.files;
    let manifest = index.manifest();
    let mut adjacency: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for (fi, e) in manifest.files.iter().enumerate() {
        adjacency.entry(fi as u32).or_default();
        for m in &e.imports {
            // `from pkg import name` names a package whose member may be a
            // module: try both.
            let mut targets = Vec::new();
            if let Some(t) = index.exports.module_file_from(m, fi as u32) {
                targets.push(t);
            }
            for t in targets {
                if t != fi as u32 {
                    adjacency.entry(fi as u32).or_default().insert(t);
                }
            }
        }
    }
    // Tarjan's strongly connected components, iterative.
    let n = files.len();
    let mut index_of = vec![u32::MAX; n];
    let mut low = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut next_index = 0u32;
    let mut sccs: Vec<Vec<u32>> = Vec::new();
    for start in 0..n as u32 {
        if index_of[start as usize] != u32::MAX {
            continue;
        }
        let mut work: Vec<(u32, Vec<u32>, usize)> = vec![(
            start,
            adjacency
                .get(&start)
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default(),
            0,
        )];
        index_of[start as usize] = next_index;
        low[start as usize] = next_index;
        next_index += 1;
        stack.push(start);
        on_stack[start as usize] = true;
        while let Some((v, succ, pos)) = work.last_mut() {
            if *pos < succ.len() {
                let w = succ[*pos];
                *pos += 1;
                if index_of[w as usize] == u32::MAX {
                    index_of[w as usize] = next_index;
                    low[w as usize] = next_index;
                    next_index += 1;
                    stack.push(w);
                    on_stack[w as usize] = true;
                    let ws = adjacency
                        .get(&w)
                        .map(|s| s.iter().copied().collect())
                        .unwrap_or_default();
                    work.push((w, ws, 0));
                } else if on_stack[w as usize] {
                    let v = *v;
                    low[v as usize] = low[v as usize].min(index_of[w as usize]);
                }
            } else {
                let v = *v;
                work.pop();
                if let Some((parent, _, _)) = work.last() {
                    let p = *parent;
                    low[p as usize] = low[p as usize].min(low[v as usize]);
                }
                if low[v as usize] == index_of[v as usize] {
                    let mut comp = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack[w as usize] = false;
                        comp.push(w);
                        if w == v {
                            break;
                        }
                    }
                    if comp.len() > 1 {
                        comp.sort();
                        sccs.push(comp);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for comp in sccs {
        let rels: Vec<&str> = comp
            .iter()
            .map(|&fi| files[fi as usize].rel.as_str())
            .collect();
        let first = comp[0];
        let location = line_loc(&files[first as usize].rel, 1);
        out.push(Diagnostic {
            id: String::new(),
            pattern: DiagnosticPattern::CircularDependency,
            severity: Severity::Warning,
            confidence: Confidence::High,
            entity: rels.join(" -> "),
            message: format!(
                "Circular dependency: {} module(s) import each other: {}",
                rels.len(),
                rels.join(" -> ")
            ),
            evidence: vec![Evidence {
                observation: format!(
                    "the import statements of these {} files form a strongly connected component",
                    rels.len()
                ),
                location: Some(location.clone()),
                context: Some("cycle in the module import graph (function-local imports included)".to_string()),
            }],
            suggestion: "Break the cycle by moving the shared definitions into a module both sides import, or by importing inside the function that needs it.".to_string(),
            location,
        });
    }
    out
}

/// `isolated_cluster`: a group of definitions that only reach each other.
pub fn isolated_cluster(index: &Index) -> Vec<Diagnostic> {
    let files = &index.exports.files;
    // Undirected union-find over definitions linked by any site.
    let mut ids: Vec<GId> = Vec::new();
    let mut pos: HashMap<GId, usize> = HashMap::new();
    for (fi, f) in files.iter().enumerate() {
        for (di, d) in f.defs.iter().enumerate() {
            if d.kind == DefKind::Module {
                continue;
            }
            let g = (fi as u32, di as u32);
            pos.insert(g, ids.len());
            ids.push(g);
        }
    }
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut has_external_in: HashSet<usize> = HashSet::new();
    for s in index.all_sites() {
        if !s.certain {
            continue;
        }
        let (Some(&a), Some(&b)) = (pos.get(&s.owner), pos.get(&s.target)) else {
            // A site owned by a module scope is a top-level use.
            if let Some(&b) = pos.get(&s.target) {
                has_external_in.insert(b);
            }
            continue;
        };
        if a == b {
            continue;
        }
        let ra = find(&mut parent, a);
        let rb = find(&mut parent, b);
        if ra != rb {
            parent[ra] = rb;
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..ids.len() {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut out = Vec::new();
    for members in groups.values() {
        if members.len() < 2 {
            continue;
        }
        if members.iter().any(|&i| has_external_in.contains(&i)) {
            continue;
        }
        let entry = members.iter().any(|&i| {
            let d = index.exports.def(ids[i]);
            is_entry_name(&d.name)
                || d.name.starts_with("test_")
                || is_test_path(&files[ids[i].0 as usize].rel)
        });
        if entry {
            continue;
        }
        // Public API reachable from outside the index: low confidence only.
        let any_public = members.iter().any(|&i| {
            let d = index.exports.def(ids[i]);
            !is_private(&d.name) && d.parent == Some(0)
        });
        let internal: usize = members
            .iter()
            .map(|&i| {
                index
                    .sites_within(ids[i])
                    .iter()
                    .filter(|s| {
                        s.certain && pos.get(&s.target).is_some_and(|t| members.contains(t))
                    })
                    .count()
            })
            .sum();
        let first = ids[members[0]];
        let location = line_loc(&files[first.0 as usize].rel, index.exports.def(first).line);
        let names: Vec<String> = members
            .iter()
            .take(5)
            .map(|&i| index.exports.entity_id(ids[i]))
            .collect();
        out.push(Diagnostic {
            id: String::new(),
            pattern: DiagnosticPattern::IsolatedCluster,
            severity: Severity::Warning,
            confidence: if any_public {
                Confidence::Low
            } else {
                Confidence::Moderate
            },
            entity: legacy_entity(index, first),
            message: format!(
                "Isolated cluster of {} symbols with {} internal references but 0 external callers",
                members.len(),
                internal
            ),
            evidence: vec![Evidence {
                observation: format!(
                    "members: {}{}",
                    names.join(", "),
                    if members.len() > 5 { ", …" } else { "" }
                ),
                location: Some(location.clone()),
                context: Some("no certain site outside the cluster reaches any member".to_string()),
            }],
            suggestion:
                "Connect the cluster to an entry point or a consumer, or remove it if it is dead."
                    .to_string(),
            location,
        });
    }
    out
}

/// `stale_reference`: `from m import name` where the project defines module
/// `m` but `m` exports no `name`.
pub fn stale_reference(index: &Index) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let files = &index.exports.files;
    for fi in 0..files.len() {
        let f = &files[fi];
        if is_test_path(&f.rel) {
            continue;
        }
        let Ok(facts) = index.facts_of(fi as u32) else {
            continue;
        };
        for i in &facts.imports {
            let Some(member) = &i.member else {
                continue;
            };
            let Some(target) = index.exports.module_file_from(&i.module, fi as u32) else {
                continue;
            };
            // A submodule import (`from pkg import mod`) is a module, not a name.
            if index
                .exports
                .module_file_from(&format!("{}.{}", i.module, member), fi as u32)
                .is_some()
            {
                continue;
            }
            let v = index.exports.resolve_export(target, member);
            if v.has_project() {
                continue;
            }
            // Star imports in the target can supply the name from elsewhere; the
            // resolver followed them. External re-exports (`from os import path`
            // in the target) make the name external, not missing.
            if !v.unknown {
                continue;
            }
            // The target module binds the name some other way (assignment to a
            // non-alias value, loop, `try`/`except` import): not stale.
            let Ok(tfacts) = index.facts_of(target) else {
                continue;
            };
            let bound_in_target = tfacts.binds.iter().any(|b| {
                b.scope == 0
                    && matches!(&b.target, super::facts::BindTarget::Local(n) if n == member)
            }) || tfacts
                .imports
                .iter()
                .any(|ti| ti.scope == 0 && ti.local == *member);
            if bound_in_target || !tfacts.stars.is_empty() {
                continue;
            }
            let location = line_loc(&f.rel, i.line);
            out.push(Diagnostic {
                id: String::new(),
                pattern: DiagnosticPattern::StaleReference,
                severity: Severity::Warning,
                confidence: Confidence::High,
                entity: format!("{}::{}", file_name_of(&f.rel), member),
                message: format!(
                    "Stale reference: import '{}' from module '{}' targets a symbol that no longer exists",
                    member, i.module
                ),
                evidence: vec![Evidence {
                    observation: format!(
                        "'{}' defines no '{}' at module scope (no definition, import, assignment or star import supplies it)",
                        files[target as usize].rel, member
                    ),
                    location: Some(location.clone()),
                    context: Some("module resolved, symbol not found".to_string()),
                }],
                suggestion: format!(
                    "Update or remove the import of '{}'. Check if the symbol was renamed or moved to a different module.",
                    member
                ),
                location,
            });
        }
    }
    out
}

/// Private top-level functions the index proves unconsumed: the subjects the
/// authority catalog may promote to findings. Mirrors the graph proof's
/// admission rule (`report_authority::build_finding`): Python, function,
/// private by naming, exactly one definition of that qualified name.
pub fn proven_private_subjects(index: &Index) -> Vec<crate::report_authority::IndexSubject> {
    let ids = super::manifest::entity_ids(index);
    let files = &index.exports.files;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for f in files {
        for d in &f.defs {
            if d.kind == DefKind::Function {
                *counts.entry(d.qual.as_str()).or_default() += 1;
            }
        }
    }
    let mut out = Vec::new();
    for (fi, f) in files.iter().enumerate() {
        for (di, d) in f.defs.iter().enumerate() {
            if d.kind != DefKind::Function || !is_private(&d.name) || d.parent != Some(0) {
                continue;
            }
            let g = (fi as u32, di as u32);
            // The authority subject is the parser's qualified spelling,
            // `file.py::name`; the manifest entity id adds a directory prefix
            // only when two files define the same name.
            let file_name = f.rel.rsplit('/').next().unwrap_or(&f.rel);
            out.push(crate::report_authority::IndexSubject {
                entity: ids
                    .get(&g)
                    .cloned()
                    .unwrap_or_else(|| index.exports.entity_id(g)),
                rel: f.rel.clone(),
                qualified_name: format!("{}::{}", file_name, d.qual),
                range: (d.line, d.column, d.end_line, d.end_column),
            });
        }
    }
    out
}

/// All index-backed patterns, in registry order.
pub fn all(index: &Index) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    out.extend(isolated_cluster(index));
    out.extend(data_dead_end(index));
    out.extend(phantom_dependency(index));
    out.extend(circular_dependency(index));
    out.extend(orphaned_impl(index));
    out.extend(missing_reexport(index));
    out.extend(stale_reference(index));
    out.extend(incomplete_migration(index));
    out.extend(asymmetric_handling(index));
    out.extend(duplication(index));
    out.extend(contract_mismatch(index));
    out.extend(partial_wiring(index));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn build(files: &[(&str, &str)]) -> (tempfile::TempDir, Index) {
        let dir = tempfile::TempDir::new().unwrap();
        let mut paths = Vec::new();
        for (rel, body) in files {
            let p = dir.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, body).unwrap();
            paths.push(p);
        }
        let idx = Index::build(dir.path(), &paths).unwrap();
        (dir, idx)
    }

    fn names(diags: &[Diagnostic]) -> Vec<String> {
        let mut v: Vec<String> = diags.iter().map(|d| d.entity.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn same_file_reads_annotations_and_local_imports_are_not_phantom() {
        let src = "import os\nimport json\nfrom typing import Any, Optional\nfrom pkg.a import A, unused_one\n\n_LIMIT = 3\n\ndef f(x: Optional[A]) -> Any:\n    from pkg.b import helper\n    return helper(_LIMIT) + os.getcwd()\n\nclass C:\n    def m(self):\n        return f\"{json.dumps(1)}\"\n";
        let (_d, idx) = build(&[
            ("pkg/__init__.py", ""),
            (
                "pkg/a.py",
                "class A:\n    pass\n\ndef unused_one():\n    pass\n",
            ),
            ("pkg/b.py", "def helper(n):\n    return n\n"),
            ("pkg/m.py", src),
        ]);
        assert_eq!(names(&phantom_dependency(&idx)), vec!["m.py::unused_one"]);
    }

    #[test]
    fn dead_end_counts_same_file_and_cross_file_consumers() {
        let (_d, idx) = build(&[
            ("pkg/__init__.py", ""),
            ("pkg/a.py", "def _used():\n    return 1\n\ndef _dead():\n    return 2\n\ndef public_dead():\n    return 3\n\ndef caller():\n    return _used()\n"),
            ("pkg/b.py", "from pkg.a import caller\n\ndef run():\n    return caller()\n"),
        ]);
        let diags = data_dead_end(&idx);
        let by: HashMap<String, Confidence> = diags
            .iter()
            .map(|d| (d.entity.clone(), d.confidence))
            .collect();
        assert_eq!(by.get("a.py::_dead"), Some(&Confidence::High));
        // Public top-level functions are importable API, never dead ends.
        assert!(!by.contains_key("a.py::public_dead"));
        assert!(!by.contains_key("b.py::run"));
        assert!(!by.contains_key("a.py::_used"));
        assert!(!by.contains_key("a.py::caller"));
    }

    #[test]
    fn methods_called_through_self_or_typed_receivers_are_not_orphaned() {
        let src = "class Base:\n    def hook(self):\n        return 0\n\nclass K(Base):\n    def hook(self):\n        return 1\n    def _helper(self):\n        return self.hook()\n    def _lonely(self):\n        return 2\n    def run(self):\n        return self._helper()\n\ndef use(k: K):\n    return k.run()\n";
        let (_d, idx) = build(&[("m.py", src)]);
        let diags = orphaned_impl(&idx);
        assert_eq!(names(&diags), vec!["m.py::Base::hook", "m.py::K::_lonely"]);
        let lonely = diags
            .iter()
            .find(|d| d.entity.ends_with("_lonely"))
            .unwrap();
        assert_eq!(lonely.confidence, Confidence::High);
        let hook = diags
            .iter()
            .find(|d| d.entity.ends_with("Base::hook"))
            .unwrap();
        assert_eq!(hook.confidence, Confidence::Low);
    }

    #[test]
    fn reexports_are_checked_only_against_curating_packages() {
        let (_d, idx) = build(&[
            ("curated/__init__.py", "from curated.a import A\n"),
            (
                "curated/a.py",
                "class A:\n    pass\n\nclass B:\n    pass\n\ndef _p():\n    pass\n",
            ),
            ("plain/__init__.py", ""),
            ("plain/x.py", "def visible():\n    pass\n"),
            ("star/__init__.py", "from star.y import *\n"),
            ("star/y.py", "def anything():\n    pass\n"),
        ]);
        assert_eq!(names(&missing_reexport(&idx)), vec!["a.py::B"]);
    }

    #[test]
    fn import_cycles_are_found_including_relative_and_function_local() {
        let (_d, idx) = build(&[
            ("pkg/__init__.py", ""),
            ("pkg/a.py", "from .b import f\n\ndef g():\n    return f()\n"),
            (
                "pkg/b.py",
                "def f():\n    from pkg.a import g\n    return g\n",
            ),
            ("pkg/c.py", "from pkg.a import g\n"),
        ]);
        let diags = circular_dependency(&idx);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].entity, "pkg/a.py -> pkg/b.py");
    }

    #[test]
    fn stale_imports_name_missing_members_but_not_submodules_aliases_or_star_sources() {
        let (_d, idx) = build(&[
            ("main.py", "from utils import old_function\nfrom utils import valid_function\nfrom helpers import removed_helper, another_removed\nfrom pkg import sub\nfrom pkg.sub import ALIAS, starred\n\ndef run():\n    return valid_function()\n"),
            ("utils.py", "def new_function():\n    return 42\n\ndef valid_function():\n    return 'ok'\n"),
            ("helpers.py", "def active_helper():\n    return True\n"),
            ("pkg/__init__.py", ""),
            ("pkg/sub.py", "from os import path\nfrom pkg.other import *\nALIAS = path\n"),
            ("pkg/other.py", "def starred():\n    pass\n"),
        ]);
        assert_eq!(
            names(&stale_reference(&idx)),
            vec![
                "main.py::another_removed",
                "main.py::old_function",
                "main.py::removed_helper"
            ]
        );
    }

    #[test]
    fn ai_codebase_heuristics_fire_on_planted_shapes_and_stay_quiet_on_clean_ones() {
        let (_d, idx) = build(&[
            ("pkg/__init__.py", ""),
            // incomplete migration: old_ and new spelling both with production callers; a shim pair excluded
            ("pkg/db.py", "def query(sql):\n    return 1\n\ndef old_query(sql):\n    return 2\n\ndef legacy_fetch(x):\n    return fetch(x)\n\ndef fetch(x):\n    return x\n"),
            ("pkg/h1.py", "from pkg.db import old_query\n\ndef a():\n    return old_query('x')\n"),
            ("pkg/h2.py", "from pkg.db import query\n\ndef b():\n    return query('x')\n"),
            // asymmetric handling: three handlers, one lacks the auth check
            ("pkg/api.py", "def require_admin():\n    pass\n\ndef log(x):\n    pass\n\ndef create_user(u):\n    require_admin()\n    log(u)\n\ndef update_user(u):\n    require_admin()\n    log(u)\n\ndef delete_user(u):\n    log(u)\n"),
            // duplication: two functions sharing 3 of 3 callees with one differing step each
            ("pkg/norm.py", "def lower(s):\n    return s\n\ndef strip(s):\n    return s\n\ndef collapse(s):\n    return s\n\ndef decode(s):\n    return s\n\ndef normalize_query(q):\n    return collapse(strip(lower(q)))\n\ndef clean_query_string(q):\n    return collapse(strip(lower(decode(q))))\n"),
            // contract mismatch: staticmethod with self
            ("pkg/svc.py", "class S:\n    @staticmethod\n    def run(self, x):\n        return x\n    @classmethod\n    def make(cls):\n        return cls()\n"),
            // partial wiring: imported by three files, used in one
            ("pkg/util.py", "def helper():\n    return 1\n"),
            ("pkg/u1.py", "from pkg.util import helper\n\ndef go():\n    return helper()\n"),
            ("pkg/u2.py", "from pkg.util import helper\n"),
            ("pkg/u3.py", "from pkg.util import helper\n"),
        ]);
        let mig = incomplete_migration(&idx);
        assert_eq!(names(&mig), vec!["db.py::old_query, db.py::query"]);
        let asym = asymmetric_handling(&idx);
        assert_eq!(names(&asym), vec!["api.py::delete_user"]);
        assert!(
            asym[0].message.contains("require_admin"),
            "{}",
            asym[0].message
        );
        let dup = duplication(&idx);
        assert_eq!(
            names(&dup),
            vec!["norm.py::normalize_query, norm.py::clean_query_string"]
        );
        let cm = contract_mismatch(&idx);
        assert_eq!(names(&cm), vec!["run"]);
        assert_eq!(cm[0].severity, Severity::Critical);
        let pw = partial_wiring(&idx);
        assert_eq!(names(&pw), vec!["util.py::helper"]);
        assert_eq!(pw[0].confidence, Confidence::High);
    }

    #[test]
    fn isolated_clusters_exclude_groups_reached_from_module_level_or_outside() {
        let (_d, idx) = build(&[
            ("m.py", "def _a():\n    return _b()\n\ndef _b():\n    return _a()\n\ndef _c():\n    return _d()\n\ndef _d():\n    return 1\n\n_c()\n"),
        ]);
        let diags = isolated_cluster(&idx);
        assert_eq!(diags.len(), 1);
        assert!(diags[0]
            .message
            .starts_with("Isolated cluster of 2 symbols"));
        assert_eq!(diags[0].entity, "m.py::_a");
    }
}

// ---------------------------------------------------------------------------
// AI-codebase heuristics
// ---------------------------------------------------------------------------

/// Owners of certain call sites reaching `g`, excluding test files and `g` itself.
fn production_callers(index: &Index, g: GId) -> Vec<GId> {
    let files = &index.exports.files;
    let mut out: Vec<GId> = Vec::new();
    for s in index.sites_reaching(g) {
        if s.kind != super::facts::SiteKind::Call || !s.certain || s.owner == g {
            continue;
        }
        if is_test_path(&files[s.owner.0 as usize].rel) {
            continue;
        }
        if !out.contains(&s.owner) {
            out.push(s.owner);
        }
    }
    out
}

/// Distinct certain callees of `g`, excluding `g` itself.
fn callees(index: &Index, g: GId) -> HashSet<GId> {
    index
        .sites_within(g)
        .into_iter()
        .filter(|s| s.kind == super::facts::SiteKind::Call && s.certain && s.target != g)
        .map(|s| s.target)
        .collect()
}

fn callee_name_list(index: &Index, set: &[GId]) -> String {
    set.iter()
        .map(|g| index.exports.def(*g).name.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

/// (definition, is old side, marker is explicit) for `incomplete_migration`.
type MigrationMember = (GId, bool, bool);

/// Members of one `asymmetric_handling` group: (definition, arity).
type ArityMember = (GId, usize);

/// `incomplete_migration`: an old and a new spelling of the same operation in
/// the same package, both with production callers.
pub fn incomplete_migration(index: &Index) -> Vec<Diagnostic> {
    const OLD_PREFIX: &[(&str, bool)] = &[
        ("deprecated_", true),
        ("legacy_", true),
        ("old_", false),
        ("sync_", false),
    ];
    const NEW_PREFIX: &[(&str, bool)] = &[("new_", false), ("async_", false)];
    const OLD_SUFFIX: &[(&str, bool)] = &[
        ("_deprecated", true),
        ("_legacy", true),
        ("_old", false),
        ("_sync", false),
    ];
    const NEW_SUFFIX: &[(&str, bool)] = &[("_new", false), ("_async", false)];
    let files = &index.exports.files;
    // base name → [(gid, old_side, explicit)]
    let mut by_base: BTreeMap<(String, String), Vec<MigrationMember>> = BTreeMap::new();
    for (fi, f) in files.iter().enumerate() {
        if is_test_path(&f.rel) {
            continue;
        }
        let dir = f
            .rel
            .rsplit_once('/')
            .map(|(d, _)| d)
            .unwrap_or("")
            .to_string();
        for (di, d) in f.defs.iter().enumerate() {
            if !matches!(d.kind, DefKind::Function | DefKind::Method) || is_dunder(&d.name) {
                continue;
            }
            let g = (fi as u32, di as u32);
            let mut push = |base: &str, old: bool, explicit: bool| {
                if !base.is_empty() {
                    by_base
                        .entry((dir.clone(), base.to_string()))
                        .or_default()
                        .push((g, old, explicit));
                }
            };
            for (p, e) in OLD_PREFIX {
                if let Some(b) = d.name.strip_prefix(p) {
                    push(b, true, *e);
                }
            }
            for (p, e) in NEW_PREFIX {
                if let Some(b) = d.name.strip_prefix(p) {
                    push(b, false, *e);
                }
            }
            for (sfx, e) in OLD_SUFFIX {
                if let Some(b) = d.name.strip_suffix(sfx) {
                    push(b, true, *e);
                }
            }
            for (sfx, e) in NEW_SUFFIX {
                if let Some(b) = d.name.strip_suffix(sfx) {
                    push(b, false, *e);
                }
            }
            // The bare name is the "new" side of `old_x` / `x_deprecated`.
            push(&d.name, false, false);
        }
    }
    let mut out = Vec::new();
    let mut seen: HashSet<(GId, GId)> = HashSet::new();
    for ((_, base), members) in &by_base {
        let olds: Vec<_> = members.iter().filter(|m| m.1).collect();
        let news: Vec<_> = members.iter().filter(|m| !m.1).collect();
        for (og, _, explicit) in &olds {
            for (ng, _, _) in &news {
                if og == ng || !seen.insert((*og, *ng)) {
                    continue;
                }
                // A thin wrapper (old calls new, or new calls old) is a shim, not a migration.
                if callees(index, *og).contains(ng) || callees(index, *ng).contains(og) {
                    continue;
                }
                let oc = production_callers(index, *og);
                let nc = production_callers(index, *ng);
                if oc.is_empty() || nc.is_empty() {
                    continue;
                }
                let od = index.exports.def(*og);
                let nd = index.exports.def(*ng);
                let location = line_loc(&files[og.0 as usize].rel, od.line);
                let name = |gs: &[GId]| -> String {
                    gs.iter()
                        .take(5)
                        .map(|g| index.exports.entity_id(*g))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                out.push(Diagnostic {
                    id: String::new(),
                    pattern: DiagnosticPattern::IncompleteMigration,
                    severity: Severity::Warning,
                    confidence: if *explicit { Confidence::High } else { Confidence::Moderate },
                    entity: format!("{}, {}", legacy_entity(index, *og), legacy_entity(index, *ng)),
                    message: format!(
                        "Incomplete migration: {} call site(s) use '{}' and {} use '{}' for '{}'",
                        oc.len(), od.name, nc.len(), nd.name, base
                    ),
                    evidence: vec![Evidence {
                        observation: format!(
                            "'{}' called from: {}. '{}' called from: {}.",
                            od.name, name(&oc), nd.name, name(&nc)
                        ),
                        location: Some(location.clone()),
                        context: Some("old and new spellings of one operation both have production callers in the same package".to_string()),
                    }],
                    suggestion: format!(
                        "Complete the migration: move the {} caller(s) of '{}' to '{}', then remove '{}'.",
                        oc.len(), od.name, nd.name, od.name
                    ),
                    location,
                });
            }
        }
    }
    out
}

/// `asymmetric_handling`: in a group of sibling functions of similar arity,
/// one lacks a callee that all the others share.
pub fn asymmetric_handling(index: &Index) -> Vec<Diagnostic> {
    const MIN_GROUP: usize = 3;
    let files = &index.exports.files;
    let mut out = Vec::new();
    for (fi, f) in files.iter().enumerate() {
        if is_test_path(&f.rel) {
            continue;
        }
        // Group: same file, same kind, same parent (methods of one class), arity within 1.
        let mut groups: BTreeMap<(DefKind, Option<u32>), Vec<ArityMember>> = BTreeMap::new();
        for (di, d) in f.defs.iter().enumerate() {
            if !matches!(d.kind, DefKind::Function | DefKind::Method)
                || is_dunder(&d.name)
                || d.name.starts_with("test_")
            {
                continue;
            }
            let arity = d
                .params
                .iter()
                .filter(|p| p.0 != "self" && p.0 != "cls")
                .count();
            groups
                .entry((d.kind, d.parent))
                .or_default()
                .push(((fi as u32, di as u32), arity));
        }
        for members in groups.values() {
            // Sub-group by arity: members within 1 of the smallest in the run.
            let mut sorted = members.clone();
            sorted.sort_by_key(|m| m.1);
            let mut i = 0;
            while i < sorted.len() {
                let base = sorted[i].1;
                let mut j = i;
                while j < sorted.len() && sorted[j].1 <= base + 1 {
                    j += 1;
                }
                let group: Vec<GId> = sorted[i..j].iter().map(|m| m.0).collect();
                i = j;
                if group.len() < MIN_GROUP {
                    continue;
                }
                // Leaf functions (no project callees) are helpers, not parallel paths.
                let sets: Vec<(GId, HashSet<GId>)> = group
                    .iter()
                    .map(|g| (*g, callees(index, *g)))
                    .filter(|(_, cs)| !cs.is_empty())
                    .collect();
                let n = sets.len();
                if n < MIN_GROUP {
                    continue;
                }
                let mut counts: HashMap<GId, usize> = HashMap::new();
                for (_, cs) in &sets {
                    for c in cs {
                        *counts.entry(*c).or_default() += 1;
                    }
                }
                let consensus: Vec<GId> = counts
                    .iter()
                    .filter(|(_, &c)| c >= n - 1 && c < n)
                    .map(|(g, _)| *g)
                    .collect();
                if consensus.is_empty() {
                    continue;
                }
                for (g, cs) in &sets {
                    let missing: Vec<GId> = consensus
                        .iter()
                        .filter(|c| !cs.contains(c))
                        .copied()
                        .collect();
                    if missing.is_empty() {
                        continue;
                    }
                    let d = index.exports.def(*g);
                    let siblings: Vec<String> = sets
                        .iter()
                        .filter(|(x, _)| x != g)
                        .map(|(x, _)| index.exports.def(*x).name.clone())
                        .collect();
                    let location = line_loc(&f.rel, d.line);
                    let missing_names = callee_name_list(index, &missing);
                    out.push(Diagnostic {
                        id: String::new(),
                        pattern: DiagnosticPattern::AsymmetricHandling,
                        severity: Severity::Warning,
                        confidence: Confidence::Moderate,
                        entity: legacy_entity(index, *g),
                        message: format!(
                            "Asymmetric handling: '{}' lacks {} present in its {} sibling(s)",
                            d.name, missing_names, siblings.len()
                        ),
                        evidence: vec![Evidence {
                            observation: format!("{} all call {}; '{}' does not", siblings.join(", "), missing_names, d.name),
                            location: Some(location.clone()),
                            context: Some("sibling functions of similar arity in the same scope share a callee this one lacks".to_string()),
                        }],
                        suggestion: format!("Add the {} call to '{}', or document why this path differs.", missing_names, d.name),
                        location,
                    });
                }
            }
        }
    }
    out
}

/// `duplication`: two functions in one file whose certain callee sets overlap
/// by Jaccard >= 0.7 over at least three callees.
pub fn duplication(index: &Index) -> Vec<Diagnostic> {
    const MIN_UNION: usize = 3;
    const THRESHOLD: f64 = 0.7;
    let files = &index.exports.files;
    let mut out = Vec::new();
    for (fi, f) in files.iter().enumerate() {
        if is_test_path(&f.rel) {
            continue;
        }
        let fns: Vec<(GId, usize, HashSet<GId>)> = f
            .defs
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                matches!(d.kind, DefKind::Function | DefKind::Method) && !is_dunder(&d.name)
            })
            .map(|(di, d)| {
                let g = (fi as u32, di as u32);
                (
                    g,
                    d.params
                        .iter()
                        .filter(|p| p.0 != "self" && p.0 != "cls")
                        .count(),
                    callees(index, g),
                )
            })
            .collect();
        for a in 0..fns.len() {
            for b in a + 1..fns.len() {
                let (ga, aa, ca) = &fns[a];
                let (gb, ab, cb) = &fns[b];
                if aa != ab {
                    continue;
                }
                let union = ca.union(cb).count();
                if union < MIN_UNION {
                    continue;
                }
                let inter: Vec<GId> = ca.intersection(cb).copied().collect();
                let jaccard = inter.len() as f64 / union as f64;
                if jaccard < THRESHOLD {
                    continue;
                }
                let da = index.exports.def(*ga);
                let db = index.exports.def(*gb);
                let location = line_loc(&f.rel, da.line);
                let only_a: Vec<GId> = ca.difference(cb).copied().collect();
                let only_b: Vec<GId> = cb.difference(ca).copied().collect();
                out.push(Diagnostic {
                    id: String::new(),
                    pattern: DiagnosticPattern::Duplication,
                    severity: Severity::Warning,
                    confidence: Confidence::Moderate,
                    entity: format!("{}, {}", legacy_entity(index, *ga), legacy_entity(index, *gb)),
                    message: format!(
                        "Duplication: '{}' and '{}' share {:.0}% of their callees ({} of {})",
                        da.name, db.name, jaccard * 100.0, inter.len(), union
                    ),
                    evidence: vec![Evidence {
                        observation: format!(
                            "both call {}; only '{}' calls [{}]; only '{}' calls [{}]",
                            callee_name_list(index, &inter), da.name, callee_name_list(index, &only_a), db.name, callee_name_list(index, &only_b)
                        ),
                        location: Some(location.clone()),
                        context: Some("structural similarity of certain callee sets, same arity, same file".to_string()),
                    }],
                    suggestion: format!("Consolidate '{}' and '{}' into one function with the differing step as a parameter.", da.name, db.name),
                    location,
                });
            }
        }
    }
    out
}

/// `contract_mismatch`: a `@staticmethod` taking `self`, a `@classmethod`
/// whose first parameter is not `cls`, or a call passing more positional
/// arguments than the certain target accepts.
pub fn contract_mismatch(index: &Index) -> Vec<Diagnostic> {
    let files = &index.exports.files;
    let mut out = Vec::new();
    for f in files {
        for d in &f.defs {
            if d.kind != DefKind::Method {
                continue;
            }
            let first = d.params.first().map(|p| p.0.as_str());
            let is_static = d.decorators.iter().any(|x| x == "staticmethod");
            let is_class = d.decorators.iter().any(|x| x == "classmethod");
            let location = line_loc(&f.rel, d.line);
            if is_static && first == Some("self") {
                out.push(Diagnostic {
                    id: String::new(),
                    pattern: DiagnosticPattern::ContractMismatch,
                    severity: Severity::Critical,
                    confidence: Confidence::High,
                    entity: d.name.clone(),
                    message: format!("Contract mismatch: @staticmethod '{}' has 'self' as first parameter", d.name),
                    evidence: vec![Evidence {
                        observation: format!("@staticmethod '{}' is declared with parameters ({})", d.name, d.params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(", ")),
                        location: Some(location.clone()),
                        context: Some("static methods receive no instance; a 'self' parameter is a caller-facing lie".to_string()),
                    }],
                    suggestion: format!("Remove 'self' from '{}' or drop @staticmethod.", d.name),
                    location,
                });
            } else if is_class
                && first.is_some()
                && first != Some("cls")
                && first != Some("mcs")
                && first != Some("metacls")
            {
                out.push(Diagnostic {
                    id: String::new(),
                    pattern: DiagnosticPattern::ContractMismatch,
                    severity: Severity::Warning,
                    confidence: Confidence::Moderate,
                    entity: d.name.clone(),
                    message: format!("Contract mismatch: @classmethod '{}' names its first parameter '{}' rather than 'cls'", d.name, first.unwrap_or("")),
                    evidence: vec![Evidence {
                        observation: format!("@classmethod '{}' parameters: ({})", d.name, d.params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(", ")),
                        location: Some(location.clone()),
                        context: Some("the first parameter of a classmethod is the class".to_string()),
                    }],
                    suggestion: format!("Rename the first parameter of '{}' to 'cls'.", d.name),
                    location,
                });
            }
        }
    }
    out
}

/// `partial_wiring`: a public function imported by several files but called
/// from fewer than 80% of them.
pub fn partial_wiring(index: &Index) -> Vec<Diagnostic> {
    let files = &index.exports.files;
    let mut out = Vec::new();
    for (fi, f) in files.iter().enumerate() {
        if is_test_path(&f.rel) {
            continue;
        }
        for (di, d) in f.defs.iter().enumerate() {
            if !matches!(d.kind, DefKind::Function | DefKind::Method)
                || is_private(&d.name)
                || is_dunder(&d.name)
            {
                continue;
            }
            let g = (fi as u32, di as u32);
            let mut importers: HashSet<u32> = HashSet::new();
            let mut callers: HashSet<u32> = HashSet::new();
            for s in index.sites_reaching(g) {
                if s.owner.0 == fi as u32 || !s.certain {
                    continue;
                }
                match s.kind {
                    super::facts::SiteKind::Import => {
                        importers.insert(s.owner.0);
                    }
                    super::facts::SiteKind::Call
                    | super::facts::SiteKind::Reference
                    | super::facts::SiteKind::Decorator => {
                        callers.insert(s.owner.0);
                    }
                }
            }
            let all: HashSet<u32> = importers.union(&callers).copied().collect();
            if all.len() < 3 || callers.is_empty() {
                continue;
            }
            let unwired: Vec<u32> = importers.difference(&callers).copied().collect();
            if unwired.is_empty() {
                continue;
            }
            let ratio = callers.len() as f64 / all.len() as f64;
            if ratio >= 0.80 {
                continue;
            }
            let mut unwired_rels: Vec<&str> = unwired
                .iter()
                .map(|u| files[*u as usize].rel.as_str())
                .collect();
            unwired_rels.sort();
            let location = line_loc(&f.rel, d.line);
            out.push(Diagnostic {
                id: String::new(),
                pattern: DiagnosticPattern::PartialWiring,
                severity: Severity::Warning,
                confidence: if ratio < 0.5 { Confidence::High } else { Confidence::Moderate },
                entity: legacy_entity(index, g),
                message: format!(
                    "Partial wiring: '{}' is imported by {} file(s) but used in only {}",
                    d.name, all.len(), callers.len()
                ),
                evidence: vec![Evidence {
                    observation: format!("imported but never used in: {}", unwired_rels.join(", ")),
                    location: Some(location.clone()),
                    context: Some("files that import a function without using it suggest an abandoned integration".to_string()),
                }],
                suggestion: format!("Wire '{}' into {} or remove the dead imports.", d.name, unwired_rels.join(", ")),
                location,
            });
        }
    }
    out
}
