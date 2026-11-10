//! Python reference index: who calls, references or imports a Python symbol.
//!
//! This module answers one question directly from tree-sitter parses, without
//! building the whole-program analysis graph: *which sites reach this function,
//! method or class?* It keeps a real (if deliberately small) resolver:
//!
//! - per-scope name bindings, including function-local imports;
//! - absolute, relative, aliased and re-exported imports;
//! - class hierarchy with base-class method lookup and override dispatch;
//! - receiver types from parameter and variable annotations, constructor
//!   calls, return annotations, `self.attr` assignments and `super()`.
//!
//! Every site is reported as **certain** (the receiver or name was resolved to
//! exactly one project definition) or **possible** (the receiver's type could
//! not be proven, or dispatch may reach an override). Nothing is dropped
//! silently: a call whose receiver type is unknown is a possible caller of
//! every project method with that name.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tree_sitter::{Node, Parser, Tree};

use crate::error::FlowspecError;

/// Identifier of a definition inside a [`PyIndex`].
pub type DefId = usize;

/// The kind of a Python definition tracked by the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    /// The module-level scope of one file.
    Module,
    /// A function that is not directly inside a class body.
    Function,
    /// A function defined directly inside a class body.
    Method,
    /// A class definition.
    Class,
}

/// One Python definition (module scope, function, method or class).
#[derive(Debug, Clone)]
pub struct Def {
    /// Index of the file that holds the definition.
    pub file: usize,
    /// Simple name (`template_variables`).
    pub name: String,
    /// Name path inside the file (`Sheet::template_variables`).
    pub qual: String,
    /// What kind of definition this is.
    pub kind: DefKind,
    /// 1-based line of the definition.
    pub line: usize,
    /// Enclosing definition; `None` only for module scopes.
    pub parent: Option<DefId>,
    /// Owning class for methods and for functions nested inside methods.
    pub class: Option<DefId>,
    decorators: Vec<String>,
    returns: Option<String>,
    bases: Vec<String>,
    start: usize,
    end: usize,
}

/// How a site refers to its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SiteKind {
    /// A call expression.
    Call,
    /// The name is used as a value (callback, stored reference, base class).
    Reference,
    /// The name is applied as a decorator.
    Decorator,
    /// The name is imported.
    Import,
}

/// One place in the source that reaches a definition.
#[derive(Debug, Clone)]
pub struct Site {
    /// The definition being reached.
    pub target: DefId,
    /// The definition that contains the site.
    pub owner: DefId,
    /// Index of the file that contains the site.
    pub file: usize,
    /// 1-based line of the site.
    pub line: usize,
    /// What kind of use this is.
    pub kind: SiteKind,
    /// `true` when the target was resolved to exactly one definition.
    pub certain: bool,
    /// Why the site is attributed to the target (shown for possible sites).
    pub why: String,
}

/// A call whose receiver type could not be determined.
#[derive(Debug, Clone)]
struct UnknownCall {
    owner: DefId,
    file: usize,
    line: usize,
}

#[derive(Debug, Clone)]
struct ImportTarget {
    module: String,
    name: Option<String>,
    line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Ty {
    Inst(DefId),
    Class(DefId),
    Super(DefId),
    Func(DefId),
    Module(usize),
    External,
}

/// The set of things an expression may evaluate to.
#[derive(Clone, Debug, Default)]
struct Val {
    tys: Vec<Ty>,
    unknown: bool,
}

impl Val {
    fn unknown() -> Self {
        Val {
            tys: Vec::new(),
            unknown: true,
        }
    }
    fn external() -> Self {
        Val::one(Ty::External)
    }
    fn none() -> Self {
        Val::default()
    }
    fn one(t: Ty) -> Self {
        Val {
            tys: vec![t],
            unknown: false,
        }
    }
    fn merge(&mut self, other: &Val) {
        for t in &other.tys {
            if !self.tys.contains(t) {
                self.tys.push(t.clone());
            }
        }
        self.unknown |= other.unknown;
    }
    /// Exactly one project type and nothing else.
    fn sure(&self) -> bool {
        !self.unknown && self.tys.len() == 1 && self.tys[0] != Ty::External
    }
    fn has_project(&self) -> bool {
        self.tys.iter().any(|t| *t != Ty::External)
    }
}

type Env = HashMap<String, Val>;

struct FileInfo {
    rel: String,
    module: String,
    is_package: bool,
    module_def: DefId,
    src: Arc<String>,
    tree: Tree,
    stars: Vec<String>,
    has_errors: bool,
}

/// Counts describing what an index covers.
#[derive(Debug, Clone, Copy, Default)]
pub struct IndexStats {
    /// Python files parsed.
    pub files: usize,
    /// Files tree-sitter reported syntax errors in (still indexed).
    pub files_with_errors: usize,
    /// Files that could not be read or parsed at all.
    pub files_skipped: usize,
    /// Functions, methods and classes found.
    pub definitions: usize,
    /// Call expressions examined.
    pub call_sites: usize,
    /// Calls whose receiver type could not be determined.
    pub unknown_receiver_calls: usize,
}

/// A queryable index of Python definitions and the sites that reach them.
pub struct PyIndex {
    files: Vec<FileInfo>,
    defs: Vec<Def>,
    by_start: HashMap<(usize, usize), DefId>,
    children: HashMap<DefId, HashMap<String, DefId>>,
    imports: HashMap<DefId, HashMap<String, ImportTarget>>,
    bound: HashMap<DefId, HashSet<String>>,
    modules: HashMap<String, usize>,
    root_tail: Vec<String>,
    class_bases: HashMap<DefId, Vec<DefId>>,
    subclasses: HashMap<DefId, Vec<DefId>>,
    protocols: HashSet<DefId>,
    envs: HashMap<DefId, Env>,
    class_attrs: HashMap<(DefId, String), Val>,
    inferred_ret: HashMap<DefId, DefId>,
    methods_by_name: HashMap<String, Vec<DefId>>,
    sites: Vec<Site>,
    sites_to: HashMap<DefId, Vec<usize>>,
    sites_from: HashMap<DefId, Vec<usize>>,
    unknown_calls: HashMap<String, Vec<UnknownCall>>,
    stats: IndexStats,
}

/// Upper bound on (file, name) pairs tried while resolving one imported name.
const MAX_EXPORT_STEPS: usize = 64;

const BUILTINS: &[&str] = &[
    "abs",
    "all",
    "any",
    "ascii",
    "bin",
    "bool",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "complex",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "hasattr",
    "hash",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "tuple",
    "type",
    "vars",
    "zip",
    "Exception",
    "BaseException",
    "ValueError",
    "TypeError",
    "KeyError",
    "RuntimeError",
    "OSError",
    "AttributeError",
    "IndexError",
    "NotImplementedError",
    "StopIteration",
    "True",
    "False",
    "None",
    "NotImplemented",
    "Ellipsis",
    "__name__",
    "__file__",
];

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

fn line_of(node: Node) -> usize {
    node.start_position().row + 1
}

/// Split `s` on `sep` at bracket depth zero.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth -= 1,
            _ if c == sep && depth == 0 => {
                out.push(s[start..i].trim());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(s[start..].trim());
    out
}

impl PyIndex {
    /// Build an index over `files` (absolute paths), reporting ids relative to `root`.
    ///
    /// Files that cannot be read are counted in [`IndexStats::files_skipped`]
    /// and otherwise ignored; files with syntax errors are indexed as far as
    /// tree-sitter recovered them.
    pub fn build(root: &Path, files: &[PathBuf]) -> Self {
        let mut idx = PyIndex {
            files: Vec::new(),
            defs: Vec::new(),
            by_start: HashMap::new(),
            children: HashMap::new(),
            imports: HashMap::new(),
            bound: HashMap::new(),
            modules: HashMap::new(),
            root_tail: Vec::new(),
            class_bases: HashMap::new(),
            subclasses: HashMap::new(),
            protocols: HashSet::new(),
            envs: HashMap::new(),
            class_attrs: HashMap::new(),
            inferred_ret: HashMap::new(),
            methods_by_name: HashMap::new(),
            sites: Vec::new(),
            sites_to: HashMap::new(),
            sites_from: HashMap::new(),
            unknown_calls: HashMap::new(),
            stats: IndexStats::default(),
        };
        idx.root_tail = root
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(n) => n.to_str().map(str::to_string),
                _ => None,
            })
            .collect();

        // Packages above the root: `src/marianne` with an `__init__.py` is `marianne`.
        let mut prefix: Vec<String> = Vec::new();
        let mut dir = Some(root);
        while let Some(d) = dir {
            if d.join("__init__.py").is_file() {
                if let Some(n) = d.file_name().and_then(|n| n.to_str()) {
                    prefix.insert(0, n.to_string());
                }
                dir = d.parent();
            } else {
                break;
            }
        }

        let mut parser = Parser::new();
        if parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .is_err()
        {
            idx.stats.files_skipped = files.len();
            return idx;
        }
        let mut sorted: Vec<&PathBuf> = files
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e == "py"))
            .collect();
        sorted.sort();
        for path in sorted {
            let Ok(src) = std::fs::read_to_string(path) else {
                idx.stats.files_skipped += 1;
                continue;
            };
            let Some(tree) = parser.parse(&src, None) else {
                idx.stats.files_skipped += 1;
                continue;
            };
            // Every walker below recurses along the tree; bounding the tree's
            // depth here bounds all of them. A deeper file is skipped and counted.
            if tree_depth_exceeds(&tree, crate::parser::MAX_AST_DEPTH) {
                idx.stats.files_skipped += 1;
                continue;
            }
            let rel_path = path.strip_prefix(root).unwrap_or(path);
            let rel = rel_path.to_string_lossy().replace('\\', "/");
            let mut parts: Vec<String> = prefix.clone();
            let comps: Vec<String> = rel_path
                .components()
                .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
                .collect();
            let is_package = comps.last().is_some_and(|l| l == "__init__.py");
            for (i, c) in comps.iter().enumerate() {
                if i + 1 == comps.len() {
                    if !is_package {
                        parts.push(c.trim_end_matches(".py").to_string());
                    }
                } else {
                    parts.push(c.clone());
                }
            }
            let module = parts.join(".");
            let file = idx.files.len();
            let module_def = idx.defs.len();
            let root_node = tree.root_node();
            idx.defs.push(Def {
                file,
                name: "<module>".to_string(),
                qual: "<module>".to_string(),
                kind: DefKind::Module,
                line: 1,
                parent: None,
                class: None,
                decorators: Vec::new(),
                returns: None,
                bases: Vec::new(),
                start: root_node.start_byte(),
                end: root_node.end_byte(),
            });
            let has_errors = root_node.has_error();
            idx.modules.entry(module.clone()).or_insert(file);
            idx.files.push(FileInfo {
                rel,
                module,
                is_package,
                module_def,
                src: Arc::new(src),
                tree,
                stars: Vec::new(),
                has_errors,
            });
        }
        idx.stats.files = idx.files.len();
        idx.stats.files_with_errors = idx.files.iter().filter(|f| f.has_errors).count();

        for f in 0..idx.files.len() {
            let tree = idx.files[f].tree.clone();
            let src = idx.files[f].src.clone();
            let md = idx.files[f].module_def;
            idx.collect(tree.root_node(), &src, f, md, &mut Vec::new());
        }
        idx.stats.definitions = idx
            .defs
            .iter()
            .filter(|d| d.kind != DefKind::Module)
            .count();
        idx.resolve_classes();
        idx.infer_returns();
        for _ in 0..2 {
            idx.compute_envs();
        }
        idx.collect_all_sites();
        idx
    }

    /// Coverage counts for this index.
    pub fn stats(&self) -> IndexStats {
        self.stats
    }

    /// The definition with the given id.
    pub fn def(&self, id: DefId) -> &Def {
        &self.defs[id]
    }

    /// Root-relative path of a file in the index.
    pub fn file_path(&self, file: usize) -> &str {
        &self.files[file].rel
    }

    /// Stable entity id: `path/to/file.py::Class::method`.
    pub fn entity_id(&self, id: DefId) -> String {
        let d = &self.defs[id];
        format!("{}::{}", self.files[d.file].rel, d.qual)
    }

    // ------------------------------------------------------------------
    // Collection
    // ------------------------------------------------------------------

    fn add_def(
        &mut self,
        node: Node,
        src: &str,
        file: usize,
        owner: DefId,
        decorators: Vec<String>,
    ) -> DefId {
        let is_class = node.kind() == "class_definition";
        let name = node
            .child_by_field_name("name")
            .map(|n| text(n, src).to_string())
            .unwrap_or_default();
        let parent = &self.defs[owner];
        let qual = if parent.kind == DefKind::Module {
            name.clone()
        } else {
            format!("{}::{}", parent.qual, name)
        };
        let kind = if is_class {
            DefKind::Class
        } else if parent.kind == DefKind::Class {
            DefKind::Method
        } else {
            DefKind::Function
        };
        let class = if is_class {
            None
        } else if parent.kind == DefKind::Class {
            Some(owner)
        } else {
            parent.class
        };
        let returns = node
            .child_by_field_name("return_type")
            .map(|n| text(n, src).to_string());
        let mut bases = Vec::new();
        if let Some(sup) = node.child_by_field_name("superclasses") {
            let mut c = sup.walk();
            for b in sup.named_children(&mut c) {
                if b.kind() != "keyword_argument" {
                    bases.push(text(b, src).to_string());
                }
            }
        }
        let id = self.defs.len();
        self.defs.push(Def {
            file,
            name: name.clone(),
            qual,
            kind,
            line: line_of(node),
            parent: Some(owner),
            class,
            decorators,
            returns,
            bases,
            start: node.start_byte(),
            end: node.end_byte(),
        });
        self.by_start.insert((file, node.start_byte()), id);
        self.children
            .entry(owner)
            .or_default()
            .insert(name.clone(), id);
        if kind == DefKind::Method {
            self.methods_by_name.entry(name).or_default().push(id);
        }
        id
    }

    fn bind(&mut self, owner: DefId, node: Node, src: &str) {
        match node.kind() {
            "identifier" => {
                self.bound
                    .entry(owner)
                    .or_default()
                    .insert(text(node, src).to_string());
            }
            "pattern_list"
            | "tuple_pattern"
            | "list_pattern"
            | "list_splat_pattern"
            | "as_pattern_target"
            | "parenthesized_expression"
            | "tuple"
            | "list" => {
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    self.bind(owner, ch, src);
                }
            }
            _ => {}
        }
    }

    fn package_of(&self, file: usize) -> Vec<String> {
        let f = &self.files[file];
        let mut parts: Vec<String> = if f.module.is_empty() {
            Vec::new()
        } else {
            f.module.split('.').map(str::to_string).collect()
        };
        if !f.is_package {
            parts.pop();
        }
        parts
    }

    fn collect_import(&mut self, node: Node, src: &str, file: usize, owner: DefId) {
        let line = line_of(node);
        if node.kind() == "import_statement" {
            let mut c = node.walk();
            for n in node.children_by_field_name("name", &mut c) {
                let (local, target) = if n.kind() == "aliased_import" {
                    let module = n
                        .child_by_field_name("name")
                        .map(|x| text(x, src).to_string())
                        .unwrap_or_default();
                    let alias = n
                        .child_by_field_name("alias")
                        .map(|x| text(x, src).to_string())
                        .unwrap_or_default();
                    (alias, module)
                } else {
                    let full = text(n, src).to_string();
                    let first = full.split('.').next().unwrap_or("").to_string();
                    (first.clone(), first)
                };
                if !local.is_empty() {
                    self.imports.entry(owner).or_default().insert(
                        local,
                        ImportTarget {
                            module: target,
                            name: None,
                            line,
                        },
                    );
                }
            }
            return;
        }
        if node.kind() != "import_from_statement" {
            return;
        }
        let Some(mn) = node.child_by_field_name("module_name") else {
            return;
        };
        let raw = text(mn, src).trim().to_string();
        let level = raw.chars().take_while(|c| *c == '.').count();
        let rest = raw[level..].to_string();
        let module = if level == 0 {
            rest
        } else {
            let mut pkg = self.package_of(file);
            for _ in 1..level {
                pkg.pop();
            }
            if !rest.is_empty() {
                pkg.extend(rest.split('.').map(str::to_string));
            }
            pkg.join(".")
        };
        let mut c = node.walk();
        let mut any = false;
        for n in node.children_by_field_name("name", &mut c) {
            any = true;
            let (name, local) = if n.kind() == "aliased_import" {
                (
                    n.child_by_field_name("name")
                        .map(|x| text(x, src).to_string())
                        .unwrap_or_default(),
                    n.child_by_field_name("alias")
                        .map(|x| text(x, src).to_string())
                        .unwrap_or_default(),
                )
            } else {
                (text(n, src).to_string(), text(n, src).to_string())
            };
            if !local.is_empty() {
                self.imports.entry(owner).or_default().insert(
                    local,
                    ImportTarget {
                        module: module.clone(),
                        name: Some(name),
                        line,
                    },
                );
            }
        }
        if !any {
            self.files[file].stars.push(module);
        }
    }

    fn collect(
        &mut self,
        node: Node,
        src: &str,
        file: usize,
        owner: DefId,
        pending: &mut Vec<String>,
    ) {
        match node.kind() {
            "decorated_definition" => {
                let mut decs = Vec::new();
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    if ch.kind() == "decorator" {
                        decs.push(text(ch, src).trim_start_matches('@').trim().to_string());
                    }
                }
                if let Some(def) = node.child_by_field_name("definition") {
                    self.collect(def, src, file, owner, &mut decs);
                }
                return;
            }
            "function_definition" | "class_definition" => {
                let decorators = std::mem::take(pending);
                let id = self.add_def(node, src, file, owner, decorators);
                if node.kind() == "function_definition" {
                    if let Some(params) = node.child_by_field_name("parameters") {
                        let mut c = params.walk();
                        for p in params.named_children(&mut c) {
                            let name_node = match p.kind() {
                                "identifier" => Some(p),
                                "default_parameter" | "typed_default_parameter" => {
                                    p.child_by_field_name("name")
                                }
                                _ => p.named_child(0),
                            };
                            if let Some(n) = name_node {
                                let n = if n.kind() == "identifier" {
                                    Some(n)
                                } else {
                                    n.named_child(0).filter(|x| x.kind() == "identifier")
                                };
                                if let Some(n) = n {
                                    self.bind(id, n, src);
                                }
                            }
                        }
                    }
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.collect(body, src, file, id, &mut Vec::new());
                }
                return;
            }
            "import_statement" | "import_from_statement" => {
                // Imports in a class body are attributed to the enclosing scope.
                let mut scope = owner;
                while self.defs[scope].kind == DefKind::Class {
                    match self.defs[scope].parent {
                        Some(p) => scope = p,
                        None => break,
                    }
                }
                self.collect_import(node, src, file, scope);
                return;
            }
            "assignment" | "augmented_assignment" | "for_statement" | "for_in_clause" => {
                if let Some(left) = node.child_by_field_name("left") {
                    self.bind(owner, left, src);
                }
            }
            "as_pattern" => {
                if let Some(alias) = node.child_by_field_name("alias") {
                    self.bind(owner, alias, src);
                }
            }
            "named_expression" => {
                if let Some(n) = node.child_by_field_name("name") {
                    self.bind(owner, n, src);
                }
            }
            _ => {}
        }
        let mut c = node.walk();
        for ch in node.named_children(&mut c) {
            self.collect(ch, src, file, owner, &mut Vec::new());
        }
    }

    // ------------------------------------------------------------------
    // Module and name resolution
    // ------------------------------------------------------------------

    fn module_file(&self, module: &str) -> Option<usize> {
        if let Some(&f) = self.modules.get(module) {
            return Some(f);
        }
        // `import <rootname>.x` when the root itself is not a package.
        let parts: Vec<&str> = module.split('.').collect();
        for k in 1..parts.len() {
            let stripped = &parts[..k];
            if self.root_tail.len() >= k
                && self.root_tail[self.root_tail.len() - k..]
                    .iter()
                    .map(String::as_str)
                    .eq(stripped.iter().copied())
            {
                if let Some(&f) = self.modules.get(&parts[k..].join(".")) {
                    return Some(f);
                }
            }
        }
        None
    }

    fn def_ty(&self, d: DefId) -> Ty {
        if self.defs[d].kind == DefKind::Class {
            Ty::Class(d)
        } else {
            Ty::Func(d)
        }
    }

    fn resolve_import(&self, t: &ImportTarget, depth: u8) -> Val {
        self.resolve_import_in(t, depth, &mut Vec::new())
    }

    fn resolve_export(&self, file: usize, name: &str, depth: u8) -> Val {
        self.resolve_export_in(file, name, depth, &mut Vec::new())
    }

    fn resolve_import_in(
        &self,
        t: &ImportTarget,
        depth: u8,
        seen: &mut Vec<(usize, String)>,
    ) -> Val {
        match (&t.name, self.module_file(&t.module)) {
            (None, Some(f)) => Val::one(Ty::Module(f)),
            (None, None) => Val::external(),
            (Some(n), Some(f)) => self.resolve_export_in(f, n, depth, seen),
            (Some(n), None) => match self.module_file(&format!("{}.{}", t.module, n)) {
                Some(f) => Val::one(Ty::Module(f)),
                None => Val::external(),
            },
        }
    }

    /// Resolve `name` as exported by `file`.
    ///
    /// `seen` holds every (file, name) already tried in this resolution, so a
    /// chain or fan of star imports is walked at most once per pair and the
    /// whole resolution is capped at [`MAX_EXPORT_STEPS`] steps.
    fn resolve_export_in(
        &self,
        file: usize,
        name: &str,
        depth: u8,
        seen: &mut Vec<(usize, String)>,
    ) -> Val {
        if depth > 8
            || seen.len() >= MAX_EXPORT_STEPS
            || seen.iter().any(|(f, n)| *f == file && n == name)
        {
            return Val::unknown();
        }
        seen.push((file, name.to_string()));
        let md = self.files[file].module_def;
        if let Some(&d) = self.children.get(&md).and_then(|m| m.get(name)) {
            return Val::one(self.def_ty(d));
        }
        if let Some(v) = self.envs.get(&md).and_then(|e| e.get(name)) {
            return v.clone();
        }
        if let Some(t) = self.imports.get(&md).and_then(|m| m.get(name)) {
            return self.resolve_import_in(t, depth + 1, seen);
        }
        if self.files[file].is_package {
            let sub = if self.files[file].module.is_empty() {
                name.to_string()
            } else {
                format!("{}.{}", self.files[file].module, name)
            };
            if let Some(f) = self.module_file(&sub) {
                return Val::one(Ty::Module(f));
            }
        }
        for star in &self.files[file].stars {
            if let Some(f) = self.module_file(star) {
                let v = self.resolve_export_in(f, name, depth + 1, seen);
                if v.has_project() {
                    return v;
                }
            }
        }
        Val::unknown()
    }

    fn lookup(&self, name: &str, def: DefId, w: &Env, r: &Env) -> Val {
        let mut cur = Some(def);
        let mut first = true;
        while let Some(d) = cur {
            let de = &self.defs[d];
            if de.kind != DefKind::Class {
                let hit = if first {
                    w.get(name).or_else(|| r.get(name))
                } else {
                    self.envs.get(&d).and_then(|e| e.get(name))
                };
                if let Some(v) = hit {
                    return v.clone();
                }
                if self.bound.get(&d).is_some_and(|b| b.contains(name)) {
                    return Val::unknown();
                }
                if let Some(t) = self.imports.get(&d).and_then(|m| m.get(name)) {
                    return self.resolve_import(t, 0);
                }
                if let Some(&c) = self.children.get(&d).and_then(|m| m.get(name)) {
                    return Val::one(self.def_ty(c));
                }
            }
            first = false;
            cur = de.parent;
        }
        if BUILTINS.contains(&name) {
            Val::external()
        } else {
            Val::unknown()
        }
    }

    fn attr_static(&self, v: &Val, attr: &str) -> Val {
        let mut out = Val::none();
        out.unknown = v.unknown;
        for t in &v.tys {
            match t {
                Ty::Module(f) => out.merge(&self.resolve_export(*f, attr, 0)),
                Ty::Class(c) => match self.children.get(c).and_then(|m| m.get(attr)) {
                    Some(&d) => out.merge(&Val::one(self.def_ty(d))),
                    None => out.unknown = true,
                },
                Ty::External => out.merge(&Val::external()),
                _ => out.unknown = true,
            }
        }
        out
    }

    fn resolve_dotted(&self, path: &str, def: DefId) -> Val {
        let empty = Env::new();
        let mut parts = path.split('.').map(str::trim);
        let Some(first) = parts.next() else {
            return Val::unknown();
        };
        let mut v = self.lookup(first, def, &empty, &empty);
        for p in parts {
            v = self.attr_static(&v, p);
        }
        v
    }

    fn enclosing_class(&self, def: DefId) -> Option<DefId> {
        let d = &self.defs[def];
        if d.kind == DefKind::Class {
            Some(def)
        } else {
            d.class
        }
    }

    fn ann_val(&self, ann: &str, def: DefId, depth: u8) -> Val {
        let t = ann.trim().trim_matches(|c| c == '"' || c == '\'').trim();
        if t.is_empty() || depth > 6 {
            return Val::unknown();
        }
        if t == "None" {
            return Val::none();
        }
        let parts = split_top(t, '|');
        if parts.len() > 1 {
            let mut v = Val::none();
            for p in parts {
                v.merge(&self.ann_val(p, def, depth + 1));
            }
            return v;
        }
        if let Some(open) = t.find('[') {
            if !t.ends_with(']') {
                return Val::unknown();
            }
            let head = t[..open].trim();
            let inner = &t[open + 1..t.len() - 1];
            let short = head.rsplit('.').next().unwrap_or(head);
            return match short {
                "Optional" => self.ann_val(inner, def, depth + 1),
                "Union" => {
                    let mut v = Val::none();
                    for p in split_top(inner, ',') {
                        v.merge(&self.ann_val(p, def, depth + 1));
                    }
                    v
                }
                "Annotated" | "Final" | "ClassVar" | "Required" | "NotRequired" => {
                    self.ann_val(split_top(inner, ',')[0], def, depth + 1)
                }
                "type" | "Type" => {
                    let mut v = self.ann_val(inner, def, depth + 1);
                    for ty in v.tys.iter_mut() {
                        if let Ty::Inst(c) = ty {
                            *ty = Ty::Class(*c);
                        }
                    }
                    v
                }
                _ => {
                    let v = self.resolve_dotted(head, def);
                    match v.tys.iter().find_map(|x| match x {
                        Ty::Class(c) => Some(*c),
                        _ => None,
                    }) {
                        Some(c) => Val::one(Ty::Inst(c)),
                        None if v.unknown => Val::unknown(),
                        None => Val::external(),
                    }
                }
            };
        }
        if t == "Self" {
            return match self.enclosing_class(def) {
                Some(c) => Val::one(Ty::Inst(c)),
                None => Val::unknown(),
            };
        }
        if t == "Any" || t == "object" || t.ends_with(".Any") {
            return Val::unknown();
        }
        let v = self.resolve_dotted(t, def);
        let mut out = Val::none();
        out.unknown = v.unknown;
        for ty in &v.tys {
            match ty {
                Ty::Class(c) => out.merge(&Val::one(Ty::Inst(*c))),
                _ => out.merge(&Val::external()),
            }
        }
        out
    }

    // ------------------------------------------------------------------
    // Classes
    // ------------------------------------------------------------------

    fn resolve_classes(&mut self) {
        let mut bases_map = HashMap::new();
        let mut protocols = HashSet::new();
        for (id, d) in self.defs.iter().enumerate() {
            if d.kind != DefKind::Class {
                continue;
            }
            let scope = d.parent.unwrap_or(id);
            let mut resolved = Vec::new();
            for b in &d.bases {
                let head = b.split('[').next().unwrap_or(b).trim();
                if head.rsplit('.').next() == Some("Protocol") {
                    protocols.insert(id);
                }
                for ty in self.resolve_dotted(head, scope).tys {
                    if let Ty::Class(c) = ty {
                        if c != id && !resolved.contains(&c) {
                            resolved.push(c);
                        }
                    }
                }
            }
            bases_map.insert(id, resolved);
        }
        let mut subs: HashMap<DefId, Vec<DefId>> = HashMap::new();
        for (c, bs) in &bases_map {
            for b in bs {
                subs.entry(*b).or_default().push(*c);
            }
        }
        self.class_bases = bases_map;
        self.subclasses = subs;
        self.protocols = protocols;
    }

    /// Linearised ancestors of `class`, the class itself first.
    fn mro(&self, class: DefId) -> Vec<DefId> {
        let mut out = Vec::new();
        let mut stack = vec![class];
        while let Some(c) = stack.pop() {
            if out.contains(&c) {
                continue;
            }
            out.push(c);
            if let Some(bs) = self.class_bases.get(&c) {
                for b in bs.iter().rev() {
                    stack.push(*b);
                }
            }
        }
        out
    }

    fn find_member(&self, class: DefId, name: &str, skip_self: bool) -> Option<DefId> {
        self.mro(class)
            .into_iter()
            .skip(usize::from(skip_self))
            .find_map(|c| self.children.get(&c).and_then(|m| m.get(name)).copied())
    }

    fn descendants(&self, class: DefId) -> Vec<DefId> {
        let mut out = Vec::new();
        let mut queue: VecDeque<DefId> = VecDeque::from([class]);
        while let Some(c) = queue.pop_front() {
            for s in self.subclasses.get(&c).into_iter().flatten() {
                if !out.contains(s) && *s != class {
                    out.push(*s);
                    queue.push_back(*s);
                }
            }
        }
        out
    }

    fn class_attr(&self, class: DefId, name: &str) -> Option<&Val> {
        self.mro(class)
            .into_iter()
            .find_map(|c| self.class_attrs.get(&(c, name.to_string())))
    }

    fn is_property(&self, d: DefId) -> bool {
        self.defs[d]
            .decorators
            .iter()
            .any(|x| x == "property" || x.ends_with("cached_property") || x.ends_with(".property"))
    }

    fn ret_val(&self, d: DefId) -> Val {
        let de = &self.defs[d];
        if de.kind == DefKind::Class {
            return Val::one(Ty::Inst(d));
        }
        if let Some(r) = &de.returns {
            return self.ann_val(r, d, 0);
        }
        match self.inferred_ret.get(&d) {
            Some(c) => Val::one(Ty::Inst(*c)),
            None => Val::unknown(),
        }
    }

    fn def_node<'t>(&self, d: DefId, tree: &'t Tree) -> Option<Node<'t>> {
        let de = &self.defs[d];
        if de.kind == DefKind::Module {
            return Some(tree.root_node());
        }
        let mut n = tree
            .root_node()
            .descendant_for_byte_range(de.start, de.end)?;
        loop {
            if n.start_byte() == de.start
                && matches!(n.kind(), "function_definition" | "class_definition")
            {
                return Some(n);
            }
            n = n.parent()?;
        }
    }

    /// A function with no return annotation and a single `return Cls(...)` returns `Cls`.
    fn infer_returns(&mut self) {
        let mut out = HashMap::new();
        for id in 0..self.defs.len() {
            let de = &self.defs[id];
            if !matches!(de.kind, DefKind::Function | DefKind::Method) || de.returns.is_some() {
                continue;
            }
            let tree = self.files[de.file].tree.clone();
            let src = self.files[de.file].src.clone();
            let Some(node) = self.def_node(id, &tree) else {
                continue;
            };
            let Some(body) = node.child_by_field_name("body") else {
                continue;
            };
            let mut returns = Vec::new();
            collect_returns(body, &mut returns);
            if returns.len() != 1 {
                continue;
            }
            let Some(value) = returns[0].named_child(0) else {
                continue;
            };
            if value.kind() != "call" {
                continue;
            }
            let Some(func) = value.child_by_field_name("function") else {
                continue;
            };
            if func.kind() != "identifier" {
                continue;
            }
            let v = self.resolve_dotted(text(func, &src), id);
            if let (true, Some(Ty::Class(c))) = (v.sure(), v.tys.first()) {
                out.insert(id, *c);
            }
        }
        self.inferred_ret = out;
    }

    // ------------------------------------------------------------------
    // Expression evaluation
    // ------------------------------------------------------------------

    fn value_of(&self, node: Node, src: &str, def: DefId, w: &Env, r: &Env) -> Val {
        match node.kind() {
            "identifier" => self.lookup(text(node, src), def, w, r),
            "call" => {
                let Some(func) = node.child_by_field_name("function") else {
                    return Val::unknown();
                };
                if func.kind() == "identifier" && text(func, src) == "super" {
                    return match self.enclosing_class(def) {
                        Some(c) => Val::one(Ty::Super(c)),
                        None => Val::unknown(),
                    };
                }
                let f = self.value_of(func, src, def, w, r);
                let mut out = Val::none();
                out.unknown = f.unknown;
                for ty in &f.tys {
                    match ty {
                        Ty::Class(c) => out.merge(&Val::one(Ty::Inst(*c))),
                        Ty::Func(d) => out.merge(&self.ret_val(*d)),
                        Ty::External => out.merge(&Val::external()),
                        _ => out.unknown = true,
                    }
                }
                out
            }
            "attribute" => {
                let (Some(obj), Some(attr)) = (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("attribute"),
                ) else {
                    return Val::unknown();
                };
                let v = self.value_of(obj, src, def, w, r);
                self.attr_val(&v, text(attr, src))
            }
            "await" | "parenthesized_expression" => match node.named_child(0) {
                Some(inner) => self.value_of(inner, src, def, w, r),
                None => Val::unknown(),
            },
            "conditional_expression" => {
                let mut out = Val::none();
                for i in [0, 2] {
                    match node.named_child(i) {
                        Some(n) => out.merge(&self.value_of(n, src, def, w, r)),
                        None => out.unknown = true,
                    }
                }
                out
            }
            "boolean_operator" => {
                let mut out = Val::none();
                for field in ["left", "right"] {
                    match node.child_by_field_name(field) {
                        Some(n) => out.merge(&self.value_of(n, src, def, w, r)),
                        None => out.unknown = true,
                    }
                }
                out
            }
            "named_expression" => match node.child_by_field_name("value") {
                Some(n) => self.value_of(n, src, def, w, r),
                None => Val::unknown(),
            },
            "none" => Val::none(),
            "string"
            | "concatenated_string"
            | "integer"
            | "float"
            | "true"
            | "false"
            | "list"
            | "dictionary"
            | "set"
            | "tuple"
            | "list_comprehension"
            | "dictionary_comprehension"
            | "set_comprehension"
            | "generator_expression"
            | "comparison_operator"
            | "not_operator"
            | "binary_operator"
            | "unary_operator" => Val::external(),
            _ => Val::unknown(),
        }
    }

    fn attr_val(&self, v: &Val, attr: &str) -> Val {
        let mut out = Val::none();
        out.unknown = v.unknown;
        for ty in &v.tys {
            match ty {
                Ty::Inst(c) | Ty::Class(c) => {
                    if let Some(a) = self.class_attr(*c, attr) {
                        out.merge(a);
                    } else if let Some(d) = self.find_member(*c, attr, false) {
                        if self.defs[d].kind == DefKind::Class {
                            out.merge(&Val::one(Ty::Class(d)));
                        } else if self.is_property(d) {
                            out.merge(&self.ret_val(d));
                        } else {
                            out.merge(&Val::one(Ty::Func(d)));
                        }
                    } else {
                        out.unknown = true;
                    }
                }
                Ty::Super(c) => match self.find_member(*c, attr, true) {
                    Some(d) => out.merge(&Val::one(self.def_ty(d))),
                    None => out.unknown = true,
                },
                Ty::Module(f) => out.merge(&self.resolve_export(*f, attr, 0)),
                Ty::External => out.merge(&Val::external()),
                Ty::Func(_) => out.unknown = true,
            }
        }
        out
    }

    // ------------------------------------------------------------------
    // Environments (flow-insensitive local types)
    // ------------------------------------------------------------------

    fn seed_params(&self, d: DefId, node: Node, src: &str) -> Env {
        let mut env = Env::new();
        let de = &self.defs[d];
        let Some(params) = node.child_by_field_name("parameters") else {
            return env;
        };
        let is_static = de.decorators.iter().any(|x| x == "staticmethod");
        let is_classmethod = de.decorators.iter().any(|x| x == "classmethod");
        let mut c = params.walk();
        for (i, p) in params.named_children(&mut c).enumerate() {
            match p.kind() {
                "identifier" => {
                    if i == 0 && de.kind == DefKind::Method && !is_static {
                        if let Some(cls) = de.class {
                            let ty = if is_classmethod {
                                Ty::Class(cls)
                            } else {
                                Ty::Inst(cls)
                            };
                            env.insert(text(p, src).to_string(), Val::one(ty));
                        }
                    }
                }
                "typed_parameter" | "typed_default_parameter" => {
                    let name = p
                        .child_by_field_name("name")
                        .or_else(|| p.named_child(0))
                        .filter(|n| n.kind() == "identifier");
                    if let (Some(n), Some(t)) = (name, p.child_by_field_name("type")) {
                        env.insert(text(n, src).to_string(), self.ann_val(text(t, src), d, 0));
                    }
                }
                _ => {}
            }
        }
        env
    }

    fn scan_bindings(
        &self,
        node: Node,
        src: &str,
        def: DefId,
        w: &mut Env,
        r: &Env,
        attrs: &mut Vec<(DefId, String, Val)>,
    ) {
        match node.kind() {
            "function_definition" | "lambda" => return,
            "class_definition" => {
                let file = self.defs[def].file;
                let Some(&cls) = self.by_start.get(&(file, node.start_byte())) else {
                    return;
                };
                let Some(body) = node.child_by_field_name("body") else {
                    return;
                };
                let mut c = body.walk();
                for stmt in body.named_children(&mut c) {
                    let Some(asg) = (stmt.kind() == "expression_statement")
                        .then(|| stmt.named_child(0))
                        .flatten()
                        .filter(|a| a.kind() == "assignment")
                    else {
                        continue;
                    };
                    let Some(left) = asg.child_by_field_name("left") else {
                        continue;
                    };
                    if left.kind() != "identifier" {
                        continue;
                    }
                    let v = if let Some(t) = asg.child_by_field_name("type") {
                        self.ann_val(text(t, src), cls, 0)
                    } else if let Some(right) = asg.child_by_field_name("right") {
                        self.value_of(right, src, def, w, r)
                    } else {
                        Val::unknown()
                    };
                    attrs.push((cls, text(left, src).to_string(), v));
                }
                return;
            }
            "assignment" => {
                if let Some(left) = node.child_by_field_name("left") {
                    let v = if let Some(t) = node.child_by_field_name("type") {
                        self.ann_val(text(t, src), def, 0)
                    } else if let Some(right) = node.child_by_field_name("right") {
                        self.value_of(right, src, def, w, r)
                    } else {
                        Val::unknown()
                    };
                    match left.kind() {
                        "identifier" => {
                            w.entry(text(left, src).to_string()).or_default().merge(&v);
                        }
                        "attribute" => {
                            if let (Some(obj), Some(attr)) = (
                                left.child_by_field_name("object"),
                                left.child_by_field_name("attribute"),
                            ) {
                                let o = self.value_of(obj, src, def, w, r);
                                if let (true, Some(Ty::Inst(c))) = (o.sure(), o.tys.first()) {
                                    attrs.push((*c, text(attr, src).to_string(), v));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "with_item" => {
                if let Some(value) = node.child_by_field_name("value") {
                    if value.kind() == "as_pattern" {
                        let target = value
                            .child_by_field_name("alias")
                            .and_then(|a| a.named_child(0))
                            .filter(|n| n.kind() == "identifier");
                        if let (Some(expr), Some(t)) = (value.named_child(0), target) {
                            let v = self.value_of(expr, src, def, w, r);
                            w.entry(text(t, src).to_string()).or_default().merge(&v);
                        }
                    }
                }
            }
            _ => {}
        }
        let mut c = node.walk();
        for ch in node.named_children(&mut c) {
            self.scan_bindings(ch, src, def, w, r, attrs);
        }
    }

    fn compute_envs(&mut self) {
        let mut new_attrs: HashMap<(DefId, String), Val> = HashMap::new();
        for id in 0..self.defs.len() {
            if self.defs[id].kind == DefKind::Class {
                continue;
            }
            let file = self.defs[id].file;
            let tree = self.files[file].tree.clone();
            let src = self.files[file].src.clone();
            let Some(node) = self.def_node(id, &tree) else {
                continue;
            };
            let seed = if self.defs[id].kind == DefKind::Module {
                Env::new()
            } else {
                self.seed_params(id, node, &src)
            };
            let body = if self.defs[id].kind == DefKind::Module {
                Some(node)
            } else {
                node.child_by_field_name("body")
            };
            let Some(body) = body else {
                self.envs.insert(id, seed);
                continue;
            };
            let mut scratch = Vec::new();
            let mut first = seed.clone();
            let empty = Env::new();
            let mut c = body.walk();
            for stmt in body.named_children(&mut c) {
                self.scan_bindings(stmt, &src, id, &mut first, &empty, &mut scratch);
            }
            let mut second = seed;
            let mut attrs = Vec::new();
            let mut c = body.walk();
            for stmt in body.named_children(&mut c) {
                self.scan_bindings(stmt, &src, id, &mut second, &first, &mut attrs);
            }
            for (cls, name, v) in attrs {
                new_attrs.entry((cls, name)).or_default().merge(&v);
            }
            self.envs.insert(id, second);
        }
        self.class_attrs = new_attrs;
    }

    // ------------------------------------------------------------------
    // Sites
    // ------------------------------------------------------------------

    fn collect_all_sites(&mut self) {
        let mut sites: Vec<Site> = Vec::new();
        let mut unknown: Vec<(String, UnknownCall)> = Vec::new();
        let mut calls = 0usize;
        let empty = Env::new();
        for id in 0..self.defs.len() {
            if self.defs[id].kind == DefKind::Class {
                continue;
            }
            let file = self.defs[id].file;
            let tree = self.files[file].tree.clone();
            let src = self.files[file].src.clone();
            let Some(node) = self.def_node(id, &tree) else {
                continue;
            };
            let env = self.envs.get(&id).unwrap_or(&empty);
            let mut cx = SiteCx {
                idx: self,
                src: &src,
                file,
                env,
                sites: &mut sites,
                unknown: &mut unknown,
                calls: &mut calls,
            };
            if self.defs[id].kind == DefKind::Module {
                cx.walk(node, id, id);
            } else if let Some(body) = node.child_by_field_name("body") {
                cx.walk(body, id, id);
            }
            if let Some(imports) = self.imports.get(&id) {
                for t in imports.values() {
                    if t.name.is_none() {
                        continue;
                    }
                    for ty in self.resolve_import(t, 0).tys {
                        if let Ty::Func(d) | Ty::Class(d) = ty {
                            sites.push(Site {
                                target: d,
                                owner: id,
                                file,
                                line: t.line,
                                kind: SiteKind::Import,
                                certain: true,
                                why: String::new(),
                            });
                        }
                    }
                }
            }
        }
        let mut seen = HashSet::new();
        sites.retain(|s| seen.insert((s.target, s.owner, s.file, s.line, s.kind)));
        sites.sort_by_key(|s| (s.file, s.line));
        for (i, s) in sites.iter().enumerate() {
            self.sites_to.entry(s.target).or_default().push(i);
            self.sites_from.entry(s.owner).or_default().push(i);
        }
        self.stats.call_sites = calls;
        self.stats.unknown_receiver_calls = unknown.len();
        for (name, u) in unknown {
            self.unknown_calls.entry(name).or_default().push(u);
        }
        self.sites = sites;
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    /// Resolve a user-supplied symbol to one definition.
    ///
    /// Accepts `path/file.py::Class::method`, `file.py::Class::method`,
    /// `file.py::method`, `Class.method`, `Class::method` and a bare name.
    /// An ambiguous or unknown symbol is an error that lists what is available.
    pub fn resolve_symbol(&self, query: &str) -> Result<DefId, FlowspecError> {
        let (file_part, sym_part) = match query.find("::") {
            Some(i) if query[..i].ends_with(".py") => (Some(&query[..i]), &query[i + 2..]),
            _ => (None, query),
        };
        let sym = sym_part.replace('.', "::");
        let file_matches = |f: usize| match file_part {
            None => true,
            Some(p) => {
                let rel = &self.files[f].rel;
                rel == p || rel.ends_with(&format!("/{p}"))
            }
        };
        let mut exact = Vec::new();
        let mut suffix = Vec::new();
        for (id, d) in self.defs.iter().enumerate() {
            if d.kind == DefKind::Module || !file_matches(d.file) {
                continue;
            }
            if d.qual == sym {
                exact.push(id);
            } else if d.qual.ends_with(&format!("::{sym}")) {
                suffix.push(id);
            }
        }
        let pool = if exact.is_empty() { suffix } else { exact };
        match pool.len() {
            1 => Ok(pool[0]),
            0 => {
                let mut msg = format!("Symbol '{query}' not found.");
                let in_file: Vec<String> = match file_part {
                    Some(_) => self
                        .defs
                        .iter()
                        .enumerate()
                        .filter(|(_, d)| d.kind != DefKind::Module && file_matches(d.file))
                        .map(|(id, _)| self.entity_id(id))
                        .collect(),
                    None => Vec::new(),
                };
                let last = sym.rsplit("::").next().unwrap_or(&sym).to_string();
                if !in_file.is_empty() {
                    msg.push_str(&format!(
                        " Available in this file: {}",
                        list_capped(&in_file, 40)
                    ));
                } else {
                    let near: Vec<String> = self
                        .defs
                        .iter()
                        .enumerate()
                        .filter(|(_, d)| d.kind != DefKind::Module && d.name == last)
                        .map(|(id, _)| self.entity_id(id))
                        .collect();
                    if near.is_empty() {
                        msg.push_str(&format!(
                            " No Python definition named '{last}' in {} file(s) searched. Run `flowspec analyze` to see available entities.",
                            self.stats.files
                        ));
                    } else {
                        msg.push_str(&format!(" Did you mean: {}", list_capped(&near, 20)));
                    }
                }
                Err(FlowspecError::SymbolNotFound(msg))
            }
            _ => {
                let names: Vec<String> = pool.iter().map(|id| self.entity_id(*id)).collect();
                Err(FlowspecError::SymbolNotFound(format!(
                    "Symbol '{query}' matches multiple entities: {}. Use a qualified name to disambiguate.",
                    list_capped(&names, 20)
                )))
            }
        }
    }

    /// Every recorded site that reaches `target`, certain sites first by position.
    ///
    /// For a method this includes calls whose receiver type is unknown, as
    /// possible sites.
    pub fn sites_reaching(&self, target: DefId) -> Vec<Site> {
        let mut out: Vec<Site> = self
            .sites_to
            .get(&target)
            .into_iter()
            .flatten()
            .map(|i| self.sites[*i].clone())
            .collect();
        let d = &self.defs[target];
        if d.kind == DefKind::Method {
            for u in self.unknown_calls.get(&d.name).into_iter().flatten() {
                if out
                    .iter()
                    .any(|s| s.file == u.file && s.line == u.line && s.owner == u.owner)
                {
                    continue;
                }
                out.push(Site {
                    target,
                    owner: u.owner,
                    file: u.file,
                    line: u.line,
                    kind: SiteKind::Call,
                    certain: false,
                    why: "receiver type not proven".to_string(),
                });
            }
        }
        out.sort_by_key(|s| (!s.certain, s.file, s.line));
        out
    }

    /// Every recorded site inside `owner` that reaches another definition.
    pub fn sites_within(&self, owner: DefId) -> Vec<Site> {
        self.sites_from
            .get(&owner)
            .into_iter()
            .flatten()
            .map(|i| self.sites[*i].clone())
            .collect()
    }

    /// Callers reachable through certain call sites, breadth-first.
    ///
    /// Returns one shortest path per caller: `[direct caller, its caller, ...]`.
    /// The result is bounded by the number of definitions in the index.
    pub fn transitive_callers(&self, target: DefId, max_depth: usize) -> Vec<Vec<DefId>> {
        self.bfs(target, max_depth, true)
    }

    /// Callees reachable through certain call sites, breadth-first.
    pub fn transitive_callees(&self, target: DefId, max_depth: usize) -> Vec<Vec<DefId>> {
        self.bfs(target, max_depth, false)
    }

    fn bfs(&self, target: DefId, max_depth: usize, backward: bool) -> Vec<Vec<DefId>> {
        let mut parent: HashMap<DefId, DefId> = HashMap::new();
        let mut order: Vec<(DefId, usize)> = Vec::new();
        let mut queue: VecDeque<(DefId, usize)> = VecDeque::from([(target, 0)]);
        let mut seen: HashSet<DefId> = HashSet::from([target]);
        while let Some((cur, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            let index = if backward {
                &self.sites_to
            } else {
                &self.sites_from
            };
            for i in index.get(&cur).into_iter().flatten() {
                let s = &self.sites[*i];
                if s.kind != SiteKind::Call || !s.certain {
                    continue;
                }
                let next = if backward { s.owner } else { s.target };
                if !seen.insert(next) {
                    continue;
                }
                parent.insert(next, cur);
                order.push((next, depth + 1));
                if self.defs[next].kind != DefKind::Module {
                    queue.push_back((next, depth + 1));
                }
            }
        }
        order
            .into_iter()
            .map(|(node, _)| {
                let mut path = vec![node];
                let mut cur = node;
                while let Some(&p) = parent.get(&cur) {
                    if p == target {
                        break;
                    }
                    path.push(p);
                    cur = p;
                }
                path.reverse();
                path
            })
            .collect()
    }
}

/// Whether any node of `tree` sits deeper than `limit`, without recursion.
fn tree_depth_exceeds(tree: &Tree, limit: usize) -> bool {
    let mut cursor = tree.walk();
    let mut depth = 0usize;
    loop {
        if cursor.goto_first_child() {
            depth += 1;
            if depth > limit {
                return true;
            }
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return false;
            }
            depth -= 1;
        }
    }
}

fn list_capped(items: &[String], cap: usize) -> String {
    if items.len() <= cap {
        items.join(", ")
    } else {
        format!(
            "{}, … and {} more",
            items[..cap].join(", "),
            items.len() - cap
        )
    }
}

fn collect_returns<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
    match node.kind() {
        "function_definition" | "class_definition" | "lambda" => return,
        "return_statement" => out.push(node),
        _ => {}
    }
    let mut c = node.walk();
    for ch in node.named_children(&mut c) {
        collect_returns(ch, out);
    }
}

struct SiteCx<'a> {
    idx: &'a PyIndex,
    src: &'a str,
    file: usize,
    env: &'a Env,
    sites: &'a mut Vec<Site>,
    unknown: &'a mut Vec<(String, UnknownCall)>,
    calls: &'a mut usize,
}

impl SiteCx<'_> {
    fn push(
        &mut self,
        target: DefId,
        owner: DefId,
        node: Node,
        kind: SiteKind,
        certain: bool,
        why: &str,
    ) {
        self.sites.push(Site {
            target,
            owner,
            file: self.file,
            line: line_of(node),
            kind,
            certain,
            why: if certain {
                String::new()
            } else {
                why.to_string()
            },
        });
    }

    fn site_value(&self, node: Node, scope: DefId) -> Val {
        self.idx.value_of(node, self.src, scope, self.env, self.env)
    }

    /// Record sites for a value that names a function or class.
    fn record_named(&mut self, v: &Val, owner: DefId, node: Node, kind: SiteKind) {
        let certain = v.sure();
        for ty in &v.tys {
            match ty {
                Ty::Func(d) => self.push(
                    *d,
                    owner,
                    node,
                    kind,
                    certain,
                    "more than one possible target",
                ),
                Ty::Class(c) => {
                    self.push(
                        *c,
                        owner,
                        node,
                        kind,
                        certain,
                        "more than one possible target",
                    );
                    if kind == SiteKind::Call {
                        if let Some(init) = self.idx.find_member(*c, "__init__", false) {
                            self.push(
                                init,
                                owner,
                                node,
                                kind,
                                certain,
                                "more than one possible target",
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn record_method(&mut self, recv: &Val, name: &str, owner: DefId, node: Node, kind: SiteKind) {
        let certain = recv.sure();
        for ty in &recv.tys {
            let (class, skip_self) = match ty {
                Ty::Inst(c) | Ty::Class(c) => (*c, false),
                Ty::Super(c) => (*c, true),
                Ty::Module(f) => {
                    let v = self.idx.resolve_export(*f, name, 0);
                    self.record_named(&v, owner, node, kind);
                    continue;
                }
                _ => continue,
            };
            let found = self.idx.find_member(class, name, skip_self);
            if let Some(d) = found {
                if self.idx.defs[d].kind == DefKind::Class {
                    self.record_named(&Val::one(Ty::Class(d)), owner, node, kind);
                } else {
                    self.push(
                        d,
                        owner,
                        node,
                        kind,
                        certain,
                        "receiver has more than one possible type",
                    );
                }
            }
            if skip_self || kind != SiteKind::Call {
                continue;
            }
            // Dynamic dispatch: a receiver typed as a base may be a subclass.
            for sub in self.idx.descendants(class) {
                if let Some(&o) = self.idx.children.get(&sub).and_then(|m| m.get(name)) {
                    if Some(o) != found {
                        let why =
                            format!("receiver typed as base class {}", self.idx.defs[class].name);
                        self.push(o, owner, node, kind, false, &why);
                    }
                }
            }
            if self.idx.protocols.contains(&class) {
                for &m in self.idx.methods_by_name.get(name).into_iter().flatten() {
                    if Some(m) != found {
                        let why =
                            format!("receiver typed as protocol {}", self.idx.defs[class].name);
                        self.push(m, owner, node, kind, false, &why);
                    }
                }
            }
        }
    }

    fn handle_call(&mut self, node: Node, scope: DefId, owner: DefId) {
        *self.calls += 1;
        let Some(func) = node.child_by_field_name("function") else {
            return;
        };
        match func.kind() {
            "identifier" => {
                let v = self.site_value(func, scope);
                self.record_named(&v, owner, node, SiteKind::Call);
            }
            "attribute" => {
                let (Some(obj), Some(attr)) = (
                    func.child_by_field_name("object"),
                    func.child_by_field_name("attribute"),
                ) else {
                    return;
                };
                let name = text(attr, self.src);
                let recv = self.site_value(obj, scope);
                if recv.has_project() {
                    self.record_method(&recv, name, owner, node, SiteKind::Call);
                } else if recv.unknown && self.idx.methods_by_name.contains_key(name) {
                    self.unknown.push((
                        name.to_string(),
                        UnknownCall {
                            owner,
                            file: self.file,
                            line: line_of(node),
                        },
                    ));
                }
            }
            _ => {}
        }
    }

    /// Walk `node`, evaluating names in `scope` and attributing sites to `owner`.
    fn walk(&mut self, node: Node, scope: DefId, owner: DefId) {
        match node.kind() {
            "import_statement"
            | "import_from_statement"
            | "type"
            | "parameters"
            | "lambda_parameters"
            | "comment" => return,
            "function_definition" => {
                // Only default values run in the enclosing scope.
                if let Some(params) = node.child_by_field_name("parameters") {
                    let mut c = params.walk();
                    for p in params.named_children(&mut c) {
                        if let Some(v) = p.child_by_field_name("value") {
                            self.walk(v, scope, owner);
                        }
                    }
                }
                return;
            }
            "class_definition" => {
                if let Some(sup) = node.child_by_field_name("superclasses") {
                    let cls = self
                        .idx
                        .by_start
                        .get(&(self.file, node.start_byte()))
                        .copied();
                    let mut c = sup.walk();
                    for b in sup.named_children(&mut c) {
                        if b.kind() == "keyword_argument" {
                            continue;
                        }
                        let v = self.site_value(b, scope);
                        if let Some(cls) = cls {
                            self.record_named(&v, cls, b, SiteKind::Reference);
                        }
                    }
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.walk(body, scope, owner);
                }
                return;
            }
            "decorated_definition" => {
                let def_node = node.child_by_field_name("definition");
                let decorated = def_node
                    .and_then(|d| self.idx.by_start.get(&(self.file, d.start_byte())))
                    .copied()
                    .unwrap_or(owner);
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    if ch.kind() != "decorator" {
                        continue;
                    }
                    let Some(expr) = ch.named_child(0) else {
                        continue;
                    };
                    if expr.kind() == "identifier" {
                        let v = self.site_value(expr, scope);
                        self.record_named(&v, decorated, expr, SiteKind::Decorator);
                    } else if expr.kind() == "attribute" {
                        self.reference_attribute(expr, scope, decorated, SiteKind::Decorator);
                    } else {
                        self.walk(expr, scope, decorated);
                    }
                }
                if let Some(d) = def_node {
                    self.walk(d, scope, owner);
                }
                return;
            }
            "call" => {
                self.handle_call(node, scope, owner);
                if let Some(func) = node.child_by_field_name("function") {
                    match func.kind() {
                        "identifier" => {}
                        "attribute" => {
                            if let Some(obj) = func.child_by_field_name("object") {
                                self.walk(obj, scope, owner);
                            }
                        }
                        _ => self.walk(func, scope, owner),
                    }
                }
                if let Some(args) = node.child_by_field_name("arguments") {
                    self.walk(args, scope, owner);
                }
                return;
            }
            "keyword_argument" => {
                if let Some(v) = node.child_by_field_name("value") {
                    self.walk(v, scope, owner);
                }
                return;
            }
            "assignment" | "augmented_assignment" => {
                if let Some(left) = node.child_by_field_name("left") {
                    if left.kind() != "identifier" {
                        self.walk_target(left, scope, owner);
                    }
                }
                if let Some(right) = node.child_by_field_name("right") {
                    self.walk(right, scope, owner);
                }
                return;
            }
            "attribute" => {
                self.reference_attribute(node, scope, owner, SiteKind::Reference);
                return;
            }
            "identifier" => {
                let v = self.site_value(node, scope);
                self.record_named(&v, owner, node, SiteKind::Reference);
                return;
            }
            _ => {}
        }
        let mut c = node.walk();
        for ch in node.named_children(&mut c) {
            self.walk(ch, scope, owner);
        }
    }

    /// Assignment targets: only their sub-expressions are uses.
    fn walk_target(&mut self, node: Node, scope: DefId, owner: DefId) {
        match node.kind() {
            "identifier" => {}
            "attribute" => {
                if let Some(obj) = node.child_by_field_name("object") {
                    self.walk(obj, scope, owner);
                }
            }
            "subscript" => self.walk(node, scope, owner),
            _ => {
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    self.walk_target(ch, scope, owner);
                }
            }
        }
    }

    fn reference_attribute(&mut self, node: Node, scope: DefId, owner: DefId, kind: SiteKind) {
        let (Some(obj), Some(attr)) = (
            node.child_by_field_name("object"),
            node.child_by_field_name("attribute"),
        ) else {
            return;
        };
        let recv = self.site_value(obj, scope);
        if recv.has_project() {
            let name = text(attr, self.src);
            // A property read runs the getter: that is a call.
            let prop = recv.tys.iter().any(|ty| match ty {
                Ty::Inst(c) => self
                    .idx
                    .find_member(*c, name, false)
                    .is_some_and(|d| self.idx.is_property(d)),
                _ => false,
            });
            let k = if prop && kind == SiteKind::Reference {
                SiteKind::Call
            } else {
                kind
            };
            self.record_method(&recv, name, owner, node, k);
        }
        self.walk(obj, scope, owner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn index(files: &[(&str, &str)]) -> (tempfile::TempDir, PyIndex) {
        let dir = tempfile::TempDir::new().unwrap();
        let mut paths = Vec::new();
        for (rel, body) in files {
            let p = dir.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, body).unwrap();
            paths.push(p);
        }
        let idx = PyIndex::build(dir.path(), &paths);
        (dir, idx)
    }

    fn callers(idx: &PyIndex, symbol: &str) -> Vec<(String, usize, bool)> {
        let id = idx.resolve_symbol(symbol).unwrap();
        idx.sites_reaching(id)
            .into_iter()
            .filter(|s| s.kind == SiteKind::Call)
            .map(|s| (idx.entity_id(s.owner), s.line, s.certain))
            .collect()
    }

    const SHEET: &str = "class Sheet:\n    def template_variables(self, n):\n        return {\"n\": n}\n\ndef make():\n    return Sheet()\n";
    const USER: &str = "from pkg.sheet import Sheet, make\n\nLIMIT = 3\n\ndef direct(sheet: Sheet):\n    return sheet.template_variables(LIMIT)\n\ndef constructed():\n    s = Sheet()\n    return s.template_variables(1)\n\ndef untyped(sheet):\n    return sheet.template_variables(2)\n\nclass Holder:\n    def __init__(self):\n        self.sheet = make()\n    def go(self):\n        return self.sheet.template_variables(4)\n";

    #[test]
    fn method_callers_cover_annotation_constructor_attribute_and_untyped() {
        let (_d, idx) = index(&[
            ("pkg/__init__.py", ""),
            ("pkg/sheet.py", SHEET),
            ("pkg/user.py", USER),
        ]);
        let got = callers(&idx, "sheet.py::Sheet::template_variables");
        assert_eq!(
            got,
            vec![
                ("pkg/user.py::direct".to_string(), 6, true),
                ("pkg/user.py::constructed".to_string(), 10, true),
                ("pkg/user.py::Holder::go".to_string(), 19, true),
                ("pkg/user.py::untyped".to_string(), 13, false),
            ]
        );
    }

    #[test]
    fn factory_function_is_called_only_by_its_real_caller() {
        let (_d, idx) = index(&[
            ("pkg/__init__.py", ""),
            ("pkg/sheet.py", SHEET),
            ("pkg/user.py", USER),
        ]);
        assert_eq!(
            callers(&idx, "sheet.py::make"),
            vec![("pkg/user.py::Holder::__init__".to_string(), 17, true)]
        );
    }

    #[test]
    fn symbol_spellings_resolve_and_errors_list_the_file() {
        let (_d, idx) = index(&[("pkg/__init__.py", ""), ("pkg/sheet.py", SHEET)]);
        let want = idx
            .resolve_symbol("pkg/sheet.py::Sheet::template_variables")
            .unwrap();
        for q in [
            "sheet.py::Sheet::template_variables",
            "sheet.py::template_variables",
            "Sheet.template_variables",
            "Sheet::template_variables",
            "template_variables",
        ] {
            assert_eq!(idx.resolve_symbol(q).unwrap(), want, "{q}");
        }
        let err = idx
            .resolve_symbol("sheet.py::nope")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Available in this file"), "{err}");
        assert!(
            err.contains("pkg/sheet.py::Sheet::template_variables"),
            "{err}"
        );
    }

    #[test]
    fn aliases_super_classmethod_static_and_same_name_methods() {
        let shapes = "def plain():\n    return 1\n\nclass Base:\n    def ping(self):\n        return 1\n\nclass Child(Base):\n    def ping(self):\n        return super().ping()\n\nclass One:\n    def same(self):\n        return 1\n\nclass Two:\n    def same(self):\n        return 2\n\nclass Modes:\n    @classmethod\n    def class_run(cls):\n        return Child().ping()\n    @staticmethod\n    def static_run():\n        return One().same()\n\ndef use(b: Base):\n    Two().same()\n    Modes.class_run()\n    Modes.static_run()\n    return b.ping()\n";
        let user = "import pkg.shapes as sh\nfrom pkg.shapes import plain as p\n\ndef through_module_alias():\n    return sh.plain()\n\ndef through_from_alias():\n    return p()\n";
        let (_d, idx) = index(&[
            ("pkg/__init__.py", ""),
            ("pkg/shapes.py", shapes),
            ("pkg/user.py", user),
        ]);
        let owners = |sym: &str, certain: bool| -> Vec<String> {
            callers(&idx, sym)
                .into_iter()
                .filter(|c| c.2 == certain)
                .map(|c| c.0)
                .collect()
        };
        assert_eq!(
            owners("shapes.py::plain", true),
            vec![
                "pkg/user.py::through_module_alias",
                "pkg/user.py::through_from_alias"
            ]
        );
        assert_eq!(
            owners("shapes.py::Base::ping", true),
            vec!["pkg/shapes.py::Child::ping", "pkg/shapes.py::use"]
        );
        // `b: Base` may be a Child at run time: possible, never certain.
        assert_eq!(
            owners("shapes.py::Child::ping", true),
            vec!["pkg/shapes.py::Modes::class_run"]
        );
        assert_eq!(
            owners("shapes.py::Child::ping", false),
            vec!["pkg/shapes.py::use"]
        );
        assert_eq!(
            owners("shapes.py::One::same", true),
            vec!["pkg/shapes.py::Modes::static_run"]
        );
        assert_eq!(
            owners("shapes.py::Two::same", true),
            vec!["pkg/shapes.py::use"]
        );
        assert_eq!(
            owners("shapes.py::Modes::class_run", true),
            vec!["pkg/shapes.py::use"]
        );
        assert_eq!(
            owners("shapes.py::Modes::static_run", true),
            vec!["pkg/shapes.py::use"]
        );
    }

    #[test]
    fn function_local_and_relative_imports_resolve() {
        let a = "def helper():\n    return 1\n";
        let b = "def run():\n    from .a import helper\n    return helper()\n\ndef shadow(helper):\n    return helper()\n";
        let (_d, idx) = index(&[("pkg/__init__.py", ""), ("pkg/a.py", a), ("pkg/b.py", b)]);
        assert_eq!(
            callers(&idx, "a.py::helper"),
            vec![("pkg/b.py::run".to_string(), 3, true)]
        );
    }

    #[test]
    fn star_import_cycles_and_fans_terminate() {
        let mut files: Vec<(String, String)> = vec![("pkg/__init__.py".into(), String::new())];
        for n in 0..12 {
            let body: String = (0..12)
                .filter(|m| *m != n)
                .map(|m| format!("from pkg.m{m} import *\n"))
                .collect();
            files.push((format!("pkg/m{n}.py"), body));
        }
        files.push((
            "pkg/use.py".into(),
            "from pkg.m0 import missing\n\ndef f():\n    return missing()\n".into(),
        ));
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let started = std::time::Instant::now();
        let (_d, idx) = index(&refs);
        assert!(
            started.elapsed().as_secs() < 5,
            "star-import fan must stay bounded"
        );
        assert!(idx.resolve_symbol("use.py::f").is_ok());
    }

    #[test]
    fn a_file_nested_deeper_than_the_limit_is_skipped_and_counted() {
        let deep = format!("x = {}1{}\n", "(".repeat(400), ")".repeat(400));
        let (_d, idx) = index(&[
            ("ok.py", "def f():\n    return 1\n"),
            ("deep.py", deep.as_str()),
        ]);
        assert_eq!(idx.stats().files, 1);
        assert_eq!(idx.stats().files_skipped, 1);
        assert!(idx.resolve_symbol("ok.py::f").is_ok());
    }

    #[test]
    fn calls_inside_f_strings_are_call_sites() {
        let src =
            "def fmt(x):\n    return str(x)\n\ndef show(v):\n    return f\"took {fmt(v)} total\"\n";
        let (_d, idx) = index(&[("m.py", src)]);
        assert_eq!(
            callers(&idx, "fmt"),
            vec![("m.py::show".to_string(), 5, true)]
        );
    }

    #[test]
    fn transitive_callers_are_one_shortest_path_each() {
        let src = "def leaf():\n    return 1\n\ndef mid():\n    return leaf()\n\ndef top():\n    return mid()\n\ndef also():\n    return mid() + leaf()\n";
        let (_d, idx) = index(&[("m.py", src)]);
        let leaf = idx.resolve_symbol("leaf").unwrap();
        let paths: Vec<Vec<String>> = idx
            .transitive_callers(leaf, 10)
            .into_iter()
            .map(|p| p.into_iter().map(|d| idx.def(d).name.clone()).collect())
            .collect();
        assert_eq!(paths, vec![vec!["mid"], vec!["also"], vec!["mid", "top"]]);
    }
}
