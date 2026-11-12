//! Manifest sections for a Python-only analysis, built from the index.
//!
//! Entities, flows, module summaries, boundaries and the dependency graph
//! are projections of the posting lists and the export table; nothing here
//! builds or walks the whole-program graph.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::analyzer::extraction::{DependencyDirection, ModuleDependency};
use crate::analyzer::patterns::exclusion::is_test_path;
use crate::manifest::types::{
    BoundaryEntry, CrossingPoint, EntityEntry, FlowEntry, FlowStep, ModuleSummary,
};

use super::facts::{DefKind, SiteKind};
use super::resolve::GId;
use super::store::Index;

const MAX_FLOW_DEPTH: usize = 64;
const MAX_FLOWS: usize = 5000;

fn kind_name(kind: DefKind, name: &str) -> &'static str {
    match kind {
        DefKind::Function => "fn",
        DefKind::Method => "method",
        DefKind::Class => "class",
        DefKind::Module => "module",
        DefKind::Variable => {
            if is_constant_name(name) {
                "const"
            } else {
                "var"
            }
        }
    }
}

/// Upper-case names with digits and underscores are constants by convention.
pub(crate) fn is_constant_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit())
        && name.chars().any(|c| c.is_alphabetic())
}

fn is_private(name: &str) -> bool {
    name.starts_with('_') && !(name.starts_with("__") && name.ends_with("__"))
}

/// Entity ids keyed by definition: `file.py::Class::method` (the file name
/// with its extension, as the graph producer spelled qualified names),
/// disambiguated by directory when two files define the same qualified name.
pub(crate) fn entity_ids(index: &Index) -> HashMap<GId, String> {
    let files = &index.exports.files;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for f in files {
        for d in &f.defs {
            if d.kind != DefKind::Module {
                *counts.entry(d.qual.as_str()).or_default() += 1;
            }
        }
    }
    let mut ids = HashMap::new();
    for (fi, f) in files.iter().enumerate() {
        let file_name = f.rel.rsplit('/').next().unwrap_or(&f.rel);
        let parent_dir = f
            .rel
            .rsplit_once('/')
            .map(|(d, _)| d.rsplit('/').next().unwrap_or(d));
        for (di, d) in f.defs.iter().enumerate() {
            if d.kind == DefKind::Module {
                continue;
            }
            let base = format!("{}::{}", file_name, d.qual);
            let id = if counts.get(d.qual.as_str()).copied().unwrap_or(0) > 1 {
                match parent_dir {
                    Some(p) if !p.is_empty() => format!("{p}/{base}"),
                    _ => base,
                }
            } else {
                base
            };
            ids.insert((fi as u32, di as u32), id);
        }
    }
    ids
}

fn signature(index: &Index, g: GId) -> String {
    let d = index.exports.def(g);
    if matches!(d.kind, DefKind::Class | DefKind::Variable) {
        return String::new();
    }
    let params: Vec<String> = d
        .params
        .iter()
        .map(|(n, a)| match a {
            Some(t) => format!("{n}: {t}"),
            None => n.clone(),
        })
        .collect();
    match &d.returns {
        Some(r) => format!("({}) -> {}", params.join(", "), r),
        None => format!("({})", params.join(", ")),
    }
}

/// The entity list.
pub fn entities(index: &Index) -> Vec<EntityEntry> {
    let ids = index.entity_ids();
    let files = &index.exports.files;
    let mut out = Vec::new();
    for (fi, f) in files.iter().enumerate() {
        for (di, d) in f.defs.iter().enumerate() {
            if d.kind == DefKind::Module {
                continue;
            }
            let g = (fi as u32, di as u32);
            let mut calls: Vec<String> = Vec::new();
            for s in index.sites_within(g) {
                if s.kind != SiteKind::Call || !s.certain {
                    continue;
                }
                if let Some(id) = ids.get(&s.target) {
                    if !calls.contains(id) {
                        calls.push(id.clone());
                    }
                }
            }
            let mut called_by: Vec<String> = Vec::new();
            for s in index.sites_reaching(g) {
                if !s.certain || s.owner == g {
                    continue;
                }
                let Some(id) = ids.get(&s.owner) else {
                    continue;
                };
                if !called_by.contains(id) {
                    called_by.push(id.clone());
                }
            }
            let mut annotations: Vec<String> = Vec::new();
            if d.name == "main" || d.name == "__main__" {
                annotations.push("entry_point".to_string());
            }
            for dec in &d.decorators {
                annotations.push(format!("decorator:{dec}"));
            }
            out.push(EntityEntry {
                id: ids[&g].clone(),
                kind: kind_name(d.kind, &d.name).to_string(),
                vis: if is_private(&d.name) { "priv" } else { "pub" }.to_string(),
                sig: signature(index, g),
                loc: format!("{}:{}", f.rel, d.line),
                calls,
                called_by,
                annotations,
            });
        }
    }
    out
}

/// Module summaries, one per file, sorted by entity count.
pub fn modules(index: &Index) -> Vec<ModuleSummary> {
    let files = &index.exports.files;
    let mut out: Vec<ModuleSummary> = files
        .iter()
        .map(|f| {
            let defs: Vec<_> = f
                .defs
                .iter()
                .filter(|d| d.kind != DefKind::Module)
                .collect();
            let count = defs.len() as u64;
            let has_main = defs
                .iter()
                .any(|d| d.name == "main" || d.name == "__main__");
            let test_fns = defs.iter().filter(|d| d.name.starts_with("test_")).count();
            let role = if count == 0 {
                "Empty module"
            } else if has_main {
                "Entry point module"
            } else if is_test_path(&f.rel) || test_fns > 0 {
                "Test module"
            } else {
                let top: Vec<_> = defs.iter().filter(|d| d.kind != DefKind::Method).collect();
                let classes = top.iter().filter(|d| d.kind == DefKind::Class).count();
                if top.is_empty() {
                    "Empty module"
                } else if classes * 2 >= top.len() {
                    "Data model module"
                } else if classes == 0 {
                    "Utility module"
                } else {
                    "Service module"
                }
            };
            let name = f
                .rel
                .rsplit('/')
                .next()
                .unwrap_or(&f.rel)
                .trim_end_matches(".py")
                .to_string();
            ModuleSummary {
                name,
                entity_count: count,
                role: role.to_string(),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.entity_count
            .cmp(&a.entity_count)
            .then(a.name.cmp(&b.name))
    });
    out
}

/// Flows from every entry point, following certain call sites depth-first,
/// each callee expanded once.
pub fn flows(index: &Index) -> Vec<FlowEntry> {
    let ids = index.entity_ids();
    let files = &index.exports.files;
    let mut entries: Vec<GId> = Vec::new();
    for (fi, f) in files.iter().enumerate() {
        for (di, d) in f.defs.iter().enumerate() {
            if d.kind != DefKind::Module && (d.name == "main" || d.name == "__main__") {
                entries.push((fi as u32, di as u32));
            }
        }
    }
    let mut out = Vec::new();
    let mut expanded: HashSet<GId> = HashSet::new();
    for entry in entries {
        let mut stack: Vec<(GId, Vec<GId>)> = index
            .sites_within(entry)
            .iter()
            .filter(|s| s.kind == SiteKind::Call && s.certain && s.target != entry)
            .map(|s| s.target)
            .collect::<HashSet<_>>()
            .into_iter()
            .map(|t| (t, vec![t]))
            .collect();
        // Depth-first in source order: the stack pops its last element.
        stack.sort();
        stack.reverse();
        while let Some((node, path)) = stack.pop() {
            if out.len() >= MAX_FLOWS {
                break;
            }
            let mut next: Vec<GId> = Vec::new();
            if path.len() < MAX_FLOW_DEPTH && expanded.insert(node) {
                for s in index.sites_within(node) {
                    if s.kind == SiteKind::Call
                        && s.certain
                        && !path.contains(&s.target)
                        && s.target != entry
                        && !next.contains(&s.target)
                    {
                        next.push(s.target);
                    }
                }
            }
            if next.is_empty() {
                let steps: Vec<FlowStep> = path
                    .iter()
                    .map(|g| FlowStep {
                        entity: ids.get(g).cloned().unwrap_or_default(),
                        action: "call".to_string(),
                        in_type: "unknown".to_string(),
                        out_type: "unknown".to_string(),
                    })
                    .collect();
                let entry_id = ids.get(&entry).cloned().unwrap_or_default();
                let exit_id = ids.get(&node).cloned().unwrap_or_default();
                out.push(FlowEntry {
                    id: String::new(),
                    description: format!("{} → {}", entry_id, exit_id),
                    entry: entry_id,
                    exit: exit_id,
                    steps,
                    issues: Vec::new(),
                });
            } else {
                next.sort();
                for t in next.into_iter().rev() {
                    let mut p = path.clone();
                    p.push(t);
                    stack.push((t, p));
                }
            }
        }
    }
    for (i, f) in out.iter_mut().enumerate() {
        f.id = format!("F{:03}", i + 1);
    }
    out
}

/// Cross-file edges by file pair.
fn cross_file_edges(
    index: &Index,
) -> BTreeMap<(String, String), (usize, bool, Vec<CrossingPoint>)> {
    let ids = index.entity_ids();
    let files = &index.exports.files;
    let mut map: BTreeMap<(String, String), (usize, bool, Vec<CrossingPoint>)> = BTreeMap::new();
    for s in index.all_sites() {
        if !s.certain || s.owner.0 == s.target.0 {
            continue;
        }
        let from = files[s.owner.0 as usize].rel.clone();
        let to = files[s.target.0 as usize].rel.clone();
        let (key, reverse) = if from <= to {
            ((from, to), false)
        } else {
            ((to, from), true)
        };
        let e = map.entry(key).or_insert_with(|| (0, false, Vec::new()));
        e.0 += 1;
        if reverse {
            e.1 = true;
        }
        if s.kind == SiteKind::Call && e.2.len() < 50 {
            let func = ids.get(&s.owner).cloned().unwrap_or_default();
            if !e.2.iter().any(|c| c.func == func) {
                e.2.push(CrossingPoint {
                    func,
                    data_in: "unknown".to_string(),
                    data_out: "unknown".to_string(),
                });
            }
        }
    }
    map
}

/// Boundaries between files, with crossing points.
pub fn boundaries(index: &Index) -> Vec<BoundaryEntry> {
    cross_file_edges(index)
        .into_iter()
        .enumerate()
        .map(|(i, ((from, to), (_, _, crossing_points)))| BoundaryEntry {
            id: format!("B{:03}", i + 1),
            boundary_type: "module".to_string(),
            from,
            to,
            crossing_points,
            issues: Vec::new(),
        })
        .collect()
}

/// The module dependency graph.
pub fn dependency_graph(index: &Index) -> Vec<ModuleDependency> {
    cross_file_edges(index)
        .into_iter()
        .map(
            |((from, to), (weight, bidirectional, _))| ModuleDependency {
                from,
                to,
                weight,
                direction: if bidirectional {
                    DependencyDirection::Bidirectional
                } else {
                    DependencyDirection::Unidirectional
                },
                issues: Vec::new(),
            },
        )
        .collect()
}

/// Public functions and methods that call nothing in the project.
pub fn exit_points(index: &Index) -> Vec<String> {
    let ids = index.entity_ids();
    let files = &index.exports.files;
    let mut out = Vec::new();
    for (fi, f) in files.iter().enumerate() {
        for (di, d) in f.defs.iter().enumerate() {
            if !matches!(d.kind, DefKind::Function | DefKind::Method) || is_private(&d.name) {
                continue;
            }
            if d.name == "main" || d.name == "__main__" {
                continue;
            }
            let g = (fi as u32, di as u32);
            let calls_project = index
                .sites_within(g)
                .iter()
                .any(|s| s.kind == SiteKind::Call && s.certain);
            if !calls_project {
                out.push(ids[&g].clone());
            }
        }
    }
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

    #[test]
    fn entities_carry_cross_file_callers_and_disambiguated_ids() {
        let (_d, idx) = build(&[
            ("backend.py", "class Backend:\n    def execute(self):\n        return 'ok'\n"),
            ("service.py", "from backend import Backend\n\nclass Service:\n    def __init__(self):\n        self.backend: Backend = Backend()\n    def run(self):\n        return self.backend.execute()\n\ndef main():\n    Service().run()\n"),
            ("a/util.py", "def helper():\n    pass\n"),
            ("b/util.py", "def helper():\n    pass\n"),
        ]);
        let ents = entities(&idx);
        let execute = ents
            .iter()
            .find(|e| e.id == "backend.py::Backend::execute")
            .unwrap();
        assert_eq!(execute.called_by, vec!["service.py::Service::run"]);
        assert_eq!(execute.kind, "method");
        assert_eq!(execute.sig, "(self)");
        assert_eq!(execute.loc, "backend.py:2");
        let run = ents
            .iter()
            .find(|e| e.id == "service.py::Service::run")
            .unwrap();
        assert_eq!(run.calls, vec!["backend.py::Backend::execute"]);
        let helpers: Vec<&str> = ents
            .iter()
            .filter(|e| e.id.ends_with("util.py::helper"))
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(helpers, vec!["a/util.py::helper", "b/util.py::helper"]);
        let main = ents.iter().find(|e| e.id == "service.py::main").unwrap();
        assert!(main.annotations.contains(&"entry_point".to_string()));
    }

    #[test]
    fn flows_follow_certain_calls_from_main_and_terminate() {
        let (_d, idx) = build(&[
            ("app.py", "from lib import load, save\n\ndef main():\n    data = load()\n    save(data)\n"),
            ("lib.py", "def load():\n    return parse()\n\ndef parse():\n    return 1\n\ndef save(d):\n    return d\n"),
        ]);
        let fl = flows(&idx);
        let paths: Vec<String> = fl
            .iter()
            .map(|f| {
                f.steps
                    .iter()
                    .map(|s| s.entity.as_str())
                    .collect::<Vec<_>>()
                    .join(">")
            })
            .collect();
        assert_eq!(paths, vec!["lib.py::load>lib.py::parse", "lib.py::save"]);
        assert_eq!(fl[0].id, "F001");
        assert_eq!(fl[0].entry, "app.py::main");
        assert_eq!(fl[0].exit, "lib.py::parse");
    }

    #[test]
    fn dependency_graph_and_boundaries_count_cross_file_sites() {
        let (_d, idx) = build(&[
            (
                "a.py",
                "from b import f\n\ndef g():\n    return f() + f()\n",
            ),
            (
                "b.py",
                "from a import g\n\ndef f():\n    return 1\n\ndef h():\n    return g()\n",
            ),
        ]);
        let deps = dependency_graph(&idx);
        assert_eq!(deps.len(), 1);
        assert_eq!(
            (deps[0].from.as_str(), deps[0].to.as_str()),
            ("a.py", "b.py")
        );
        assert_eq!(deps[0].direction, DependencyDirection::Bidirectional);
        assert!(
            deps[0].weight >= 3,
            "two calls + imports both ways: {}",
            deps[0].weight
        );
        let b = boundaries(&idx);
        assert_eq!(b[0].boundary_type, "module");
        let funcs: Vec<&str> = b[0]
            .crossing_points
            .iter()
            .map(|c| c.func.as_str())
            .collect();
        assert!(
            funcs.contains(&"a.py::g") && funcs.contains(&"b.py::h"),
            "{funcs:?}"
        );
    }

    #[test]
    fn module_roles_and_exit_points() {
        let (_d, idx) = build(&[
            ("models.py", "class A:\n    pass\n\nclass B:\n    pass\n"),
            (
                "util.py",
                "def one():\n    return 1\n\ndef two():\n    return one()\n",
            ),
            ("test_x.py", "def test_it():\n    pass\n"),
            ("cli.py", "def main():\n    pass\n"),
        ]);
        let m = modules(&idx);
        let role = |n: &str| m.iter().find(|x| x.name == n).unwrap().role.clone();
        assert_eq!(role("models"), "Data model module");
        assert_eq!(role("util"), "Utility module");
        assert_eq!(role("test_x"), "Test module");
        assert_eq!(role("cli"), "Entry point module");
        let ex = exit_points(&idx);
        assert!(
            ex.contains(&"util.py::one".to_string()) && !ex.contains(&"util.py::two".to_string()),
            "{ex:?}"
        );
    }
}
