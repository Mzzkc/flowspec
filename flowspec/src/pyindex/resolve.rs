//! Resolution: from one file's facts plus the global export table to sites.
//!
//! Names are resolved by the rules in `docs/design/streaming-index.md`.
//! Everything cross-file goes through [`Exports`]; nothing here reads a
//! second file's facts, so files resolve independently and in any order.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::facts::{BindTarget, DefKind, Expr, FactSet, HazardKind, SiteKind};

/// Global id of a definition: (file index, definition index in that file).
pub type GId = (u32, u32);

/// A definition's summary as other files see it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportDef {
    /// Simple name.
    pub name: String,
    /// `Class::method` path inside the file.
    pub qual: String,
    /// Kind.
    pub kind: DefKind,
    /// Line.
    pub line: u32,
    /// Column.
    pub column: u32,
    /// End line.
    pub end_line: u32,
    /// End column.
    pub end_column: u32,
    /// Enclosing definition.
    pub parent: Option<u32>,
    /// Owning class for methods.
    pub class: Option<u32>,
    /// Decorators.
    pub decorators: Vec<String>,
    /// Return annotation text.
    pub returns: Option<String>,
    /// Base texts.
    pub bases: Vec<String>,
    /// Name returned by a single `return Name(...)`.
    pub single_call_return: Option<String>,
    /// First parameter name (receiver of a method).
    pub first_param: Option<String>,
    /// Parameter annotations.
    pub params: Vec<(String, Option<String>)>,
}

/// One file's contribution to the export table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileExports {
    /// Root-relative path.
    pub rel: String,
    /// Dotted module.
    pub module: String,
    /// Whether it is a package `__init__`.
    pub is_package: bool,
    /// All definitions (cross-file member lookup needs every class member).
    pub defs: Vec<ExportDef>,
    /// Module-scope imports: local name → (module, member).
    pub imports: HashMap<String, (String, Option<String>)>,
    /// Imports bound inside function scopes: (scope, local name) → (module, member).
    pub scoped_imports: HashMap<(u32, String), (String, Option<String>)>,
    /// Module-scope name bindings whose value names a definition or import:
    /// local name → shape.
    pub aliases: HashMap<String, Expr>,
    /// Star imports.
    pub stars: Vec<String>,
    /// Class-level attribute annotations/values: (class, attr) → (annotation, shape).
    pub class_attrs: Vec<(u32, String, Option<String>, Expr)>,
    /// `self.attr = ...` facts: (class, attr, annotation, shape, scope def).
    pub self_attrs: Vec<(u32, String, Option<String>, Expr, u32)>,
    /// Identifier-shaped string literals (name-based dispatch tables).
    #[serde(default)]
    pub named_strings: HashSet<String>,
}

impl FileExports {
    /// Reduce a fact set to what other files need from it.
    pub fn from_facts(f: &FactSet) -> Self {
        let mut imports = HashMap::new();
        let mut scoped_imports = HashMap::new();
        for i in &f.imports {
            if i.scope == 0 {
                imports.insert(i.local.clone(), (i.module.clone(), i.member.clone()));
            } else {
                scoped_imports.insert(
                    (i.scope, i.local.clone()),
                    (i.module.clone(), i.member.clone()),
                );
            }
        }
        let mut aliases = HashMap::new();
        let mut class_attrs = Vec::new();
        let mut self_attrs = Vec::new();
        for b in &f.binds {
            match &b.target {
                BindTarget::Local(n) if b.scope == 0 => {
                    if matches!(b.value, Expr::Name(_) | Expr::Attr(..)) {
                        aliases.insert(n.clone(), b.value.clone());
                    }
                }
                BindTarget::ClassAttr(a) => {
                    if let Some(c) = b.class {
                        class_attrs.push((c, a.clone(), b.annotation.clone(), b.value.clone()));
                    }
                }
                BindTarget::SelfAttr(a) => {
                    if let Some(c) = b.class {
                        self_attrs.push((
                            c,
                            a.clone(),
                            b.annotation.clone(),
                            b.value.clone(),
                            b.scope,
                        ));
                    }
                }
                _ => {}
            }
        }
        FileExports {
            rel: f.rel.clone(),
            module: f.module.clone(),
            is_package: f.is_package,
            named_strings: f.named_strings.iter().cloned().collect(),
            defs: f
                .defs
                .iter()
                .map(|d| ExportDef {
                    name: d.name.clone(),
                    qual: d.qual.clone(),
                    kind: d.kind,
                    line: d.line,
                    column: d.column,
                    end_line: d.end_line,
                    end_column: d.end_column,
                    parent: d.parent,
                    class: d.class,
                    decorators: d.decorators.clone(),
                    returns: d.returns.clone(),
                    bases: d.bases.clone(),
                    single_call_return: d.single_call_return.clone(),
                    first_param: d.params.first().map(|p| p.0.clone()),
                    params: d.params.clone(),
                })
                .collect(),
            imports,
            scoped_imports,
            aliases,
            stars: f.stars.clone(),
            class_attrs,
            self_attrs,
        }
    }
}

/// What an expression may evaluate to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    /// An instance of a project class.
    Inst(GId),
    /// A project class itself.
    Class(GId),
    /// `super()` inside a method of the class.
    Super(GId),
    /// A project function or method.
    Func(GId),
    /// A project module.
    Module(u32),
    /// A module-level variable or constant.
    Var(GId),
    /// Something outside the project (stdlib, third party, literal).
    External,
}

/// A set of possible types.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Val {
    /// Candidate types.
    pub tys: Vec<Ty>,
    /// Whether some possibility could not be named.
    pub unknown: bool,
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
    /// Exactly one project type and nothing unknown.
    pub fn sure(&self) -> bool {
        !self.unknown && self.tys.len() == 1 && self.tys[0] != Ty::External
    }
    /// At least one project type.
    pub fn has_project(&self) -> bool {
        self.tys.iter().any(|t| *t != Ty::External)
    }
}

/// The resident cross-file table: every file's exports, plus derived
/// indexes for module lookup, class hierarchy and member lookup.
#[derive(Debug, Default)]
pub struct Exports {
    /// Per-file exports, indexed by file id.
    pub files: Vec<FileExports>,
    modules: HashMap<String, u32>,
    root_tail: Vec<String>,
    members: HashMap<GId, HashMap<String, u32>>,
    bases: HashMap<GId, Vec<GId>>,
    subclasses: HashMap<GId, Vec<GId>>,
    protocols: HashSet<GId>,
    methods_by_name: HashMap<String, Vec<GId>>,
    class_attr_vals: HashMap<(GId, String), Val>,
    inferred_returns: HashMap<GId, GId>,
}

const MAX_EXPORT_STEPS: usize = 64;

/// Depth bound shared by every mutually recursive resolution function.
///
/// `ret_val`, `ann_val`, `resolve_dotted`, `lookup`, `eval` and `attr_val`
/// call one another; a self-referential alias (`TimeoutError = TimeoutError`)
/// or a return annotation naming a class whose attribute is typed by the same
/// function would otherwise recurse without end. The counter is per thread
/// and reset by the guard's drop.
const MAX_RESOLVE_DEPTH: u32 = 48;

thread_local! {
    static RESOLVE_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

struct DepthGuard;

impl DepthGuard {
    /// Enter one level; `None` when the bound is reached.
    fn enter() -> Option<DepthGuard> {
        RESOLVE_DEPTH.with(|d| {
            if d.get() >= MAX_RESOLVE_DEPTH {
                None
            } else {
                d.set(d.get() + 1);
                Some(DepthGuard)
            }
        })
    }
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        RESOLVE_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

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
    "super",
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
    "object",
];

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

impl Exports {
    /// Build the table from every file's exports. `root_tail` is the path
    /// components of the analysis root, used for `import <rootname>.x`.
    pub fn build(files: Vec<FileExports>, root_tail: Vec<String>) -> Self {
        let mut ex = Exports {
            files,
            root_tail,
            ..Exports::default()
        };
        for (fi, f) in ex.files.iter().enumerate() {
            ex.modules.entry(f.module.clone()).or_insert(fi as u32);
        }
        // Members and method-name index.
        let mut members: HashMap<GId, HashMap<String, u32>> = HashMap::new();
        let mut methods_by_name: HashMap<String, Vec<GId>> = HashMap::new();
        for (fi, f) in ex.files.iter().enumerate() {
            for (di, d) in f.defs.iter().enumerate() {
                if let Some(p) = d.parent {
                    members
                        .entry((fi as u32, p))
                        .or_default()
                        .insert(d.name.clone(), di as u32);
                }
                if d.kind == DefKind::Method {
                    methods_by_name
                        .entry(d.name.clone())
                        .or_default()
                        .push((fi as u32, di as u32));
                }
            }
        }
        ex.members = members;
        ex.methods_by_name = methods_by_name;
        // Class hierarchy.
        let mut bases: HashMap<GId, Vec<GId>> = HashMap::new();
        let mut protocols = HashSet::new();
        for fi in 0..ex.files.len() as u32 {
            for di in 0..ex.files[fi as usize].defs.len() as u32 {
                let d = &ex.files[fi as usize].defs[di as usize];
                if d.kind != DefKind::Class {
                    continue;
                }
                let scope = ex.lookup_scope_of((fi, di));
                let mut resolved = Vec::new();
                for b in d.bases.clone() {
                    let head = b.split('[').next().unwrap_or(&b).trim().to_string();
                    if head.rsplit('.').next() == Some("Protocol") {
                        protocols.insert((fi, di));
                    }
                    let v = ex.resolve_dotted(&head, fi, scope, &Env::default());
                    for ty in v.tys {
                        if let Ty::Class(c) = ty {
                            if c != (fi, di) && !resolved.contains(&c) {
                                resolved.push(c);
                            }
                        }
                    }
                }
                bases.insert((fi, di), resolved);
            }
        }
        let mut subs: HashMap<GId, Vec<GId>> = HashMap::new();
        for (c, bs) in &bases {
            for b in bs {
                subs.entry(*b).or_default().push(*c);
            }
        }
        ex.bases = bases;
        ex.subclasses = subs;
        ex.protocols = protocols;
        // Return inference, then class attributes (two rounds so attributes
        // typed by other attributes settle).
        let mut inferred = HashMap::new();
        for fi in 0..ex.files.len() as u32 {
            for di in 0..ex.files[fi as usize].defs.len() as u32 {
                let d = &ex.files[fi as usize].defs[di as usize];
                if !matches!(d.kind, DefKind::Function | DefKind::Method) || d.returns.is_some() {
                    continue;
                }
                if let Some(n) = d.single_call_return.clone() {
                    let v = ex.resolve_dotted(&n, fi, di, &Env::default());
                    if let (true, Some(Ty::Class(c))) = (v.sure(), v.tys.first()) {
                        inferred.insert((fi, di), *c);
                    }
                }
            }
        }
        ex.inferred_returns = inferred;
        for _ in 0..2 {
            let mut vals: HashMap<(GId, String), Val> = HashMap::new();
            for fi in 0..ex.files.len() as u32 {
                let f = &ex.files[fi as usize];
                for (c, a, ann, shape) in f.class_attrs.clone() {
                    let v = match &ann {
                        Some(t) => ex.ann_val(t, fi, c, 0),
                        None => ex.eval(&shape, fi, 0, &Env::default()),
                    };
                    vals.entry(((fi, c), a)).or_default().merge(&v);
                }
                for (c, a, ann, shape, scope) in f.self_attrs.clone() {
                    let env = ex.seed_params(fi, scope);
                    let v = match &ann {
                        Some(t) => ex.ann_val(t, fi, scope, 0),
                        None => ex.eval(&shape, fi, scope, &env),
                    };
                    vals.entry(((fi, c), a)).or_default().merge(&v);
                }
            }
            ex.class_attr_vals = vals;
        }
        ex
    }

    /// Stable entity id `path/file.py::Class::method`.
    pub fn entity_id(&self, g: GId) -> String {
        let f = &self.files[g.0 as usize];
        format!("{}::{}", f.rel, f.defs[g.1 as usize].qual)
    }

    /// The definition record.
    pub fn def(&self, g: GId) -> &ExportDef {
        &self.files[g.0 as usize].defs[g.1 as usize]
    }

    /// Number of files.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// File id of a module, if the project defines it.
    pub fn module_file(&self, module: &str) -> Option<u32> {
        if let Some(&f) = self.modules.get(module) {
            return Some(f);
        }
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

    /// File id of `module` as imported from file `from`.
    ///
    /// An exact dotted match wins. Otherwise the name is tried relative to
    /// each ancestor directory of the importing file, nearest first: a script
    /// run from its own directory has that directory on `sys.path`, so
    /// `from sibling import f` and `from subpkg.mod import g` resolve there.
    pub fn module_file_from(&self, module: &str, from: u32) -> Option<u32> {
        if let Some(f) = self.module_file(module) {
            return Some(f);
        }
        let here = &self.files[from as usize];
        let mut parts: Vec<&str> = here.module.split('.').filter(|p| !p.is_empty()).collect();
        if !here.is_package {
            parts.pop();
        }
        while !parts.is_empty() {
            let candidate = format!("{}.{}", parts.join("."), module);
            if let Some(&f) = self.modules.get(&candidate) {
                return Some(f);
            }
            parts.pop();
        }
        None
    }

    /// Nearest enclosing non-class definition of `g` (its lookup scope).
    fn lookup_scope_of(&self, g: GId) -> u32 {
        let f = &self.files[g.0 as usize];
        let mut cur = f.defs[g.1 as usize].parent.unwrap_or(0);
        while f.defs[cur as usize].kind == DefKind::Class {
            cur = f.defs[cur as usize].parent.unwrap_or(0);
        }
        cur
    }

    fn def_ty(&self, g: GId) -> Ty {
        match self.def(g).kind {
            DefKind::Class => Ty::Class(g),
            DefKind::Variable => Ty::Var(g),
            _ => Ty::Func(g),
        }
    }

    fn enclosing_class(&self, fi: u32, d: u32) -> Option<GId> {
        let de = &self.files[fi as usize].defs[d as usize];
        if de.kind == DefKind::Class {
            Some((fi, d))
        } else {
            de.class.map(|c| (fi, c))
        }
    }

    /// Resolve `member` as exported by module file `file`.
    pub fn resolve_export(&self, file: u32, name: &str) -> Val {
        self.resolve_export_in(file, name, 0, &mut Vec::new())
    }

    fn resolve_import(
        &self,
        module: &str,
        member: Option<&str>,
        from: u32,
        depth: u8,
        seen: &mut Vec<(u32, String)>,
    ) -> Val {
        match (member, self.module_file_from(module, from)) {
            (None, Some(f)) => Val::one(Ty::Module(f)),
            (None, None) => Val::external(),
            (Some(n), Some(f)) => self.resolve_export_in(f, n, depth, seen),
            (Some(n), None) => match self.module_file_from(&format!("{module}.{n}"), from) {
                Some(f) => Val::one(Ty::Module(f)),
                None => Val::external(),
            },
        }
    }

    fn resolve_export_in(
        &self,
        file: u32,
        name: &str,
        depth: u8,
        seen: &mut Vec<(u32, String)>,
    ) -> Val {
        let Some(_g) = DepthGuard::enter() else {
            return Val::unknown();
        };
        if depth > 8
            || seen.len() >= MAX_EXPORT_STEPS
            || seen.iter().any(|(f, n)| *f == file && n == name)
        {
            return Val::unknown();
        }
        seen.push((file, name.to_string()));
        let f = &self.files[file as usize];
        if let Some(&d) = self.members.get(&(file, 0)).and_then(|m| m.get(name)) {
            // A module-level variable exports its bound value too (`handler = helper`).
            if f.defs[d as usize].kind == DefKind::Variable {
                if let Some(shape) = f.aliases.get(name) {
                    let self_ref = match shape {
                        Expr::Name(n) => n == name,
                        Expr::Attr(obj, _) => matches!(&**obj, Expr::Name(n) if n == name),
                        _ => false,
                    };
                    if !self_ref {
                        let mut v = self.eval(shape, file, 0, &Env::default());
                        v.merge(&Val::one(Ty::Var((file, d))));
                        return v;
                    }
                }
            }
            return Val::one(self.def_ty((file, d)));
        }
        if let Some((module, member)) = f.imports.get(name) {
            return self.resolve_import(module, member.as_deref(), file, depth + 1, seen);
        }
        if let Some(shape) = f.aliases.get(name) {
            let self_ref = match shape {
                Expr::Name(n) => n == name,
                Expr::Attr(obj, _) => matches!(&**obj, Expr::Name(n) if n == name),
                _ => false,
            };
            return if self_ref {
                Val::external()
            } else {
                self.eval(shape, file, 0, &Env::default())
            };
        }
        if f.is_package {
            let sub = if f.module.is_empty() {
                name.to_string()
            } else {
                format!("{}.{}", f.module, name)
            };
            if let Some(sf) = self.module_file(&sub) {
                return Val::one(Ty::Module(sf));
            }
        }
        for star in &f.stars {
            if let Some(sf) = self.module_file_from(star, file) {
                let v = self.resolve_export_in(sf, name, depth + 1, seen);
                if v.has_project() {
                    return v;
                }
            }
        }
        Val::unknown()
    }

    /// Look a name up from scope `scope` in file `fi`, with `env` holding
    /// the local bindings of the innermost scope.
    pub fn lookup(&self, name: &str, fi: u32, scope: u32, env: &Env) -> Val {
        let Some(_g) = DepthGuard::enter() else {
            return Val::unknown();
        };
        let f = &self.files[fi as usize];
        let mut cur = Some(scope);
        let mut first = true;
        while let Some(d) = cur {
            if first {
                if let Some(v) = env.get(name) {
                    return v.clone();
                }
                if env.bound.contains(name) {
                    return Val::unknown();
                }
            }
            if let Some((module, member)) = f.scoped_imports.get(&(d, name.to_string())) {
                return self.resolve_import(module, member.as_deref(), fi, 0, &mut Vec::new());
            }
            if let Some(&c) = self.members.get(&(fi, d)).and_then(|m| m.get(name)) {
                // A module-level variable evaluates to its bound value when the
                // value names a definition; the reference to the variable itself
                // is recorded by the site walker through `Ty::Var`.
                if self.files[fi as usize].defs[c as usize].kind == DefKind::Variable {
                    if let Some(shape) = f.aliases.get(name) {
                        let self_ref = match shape {
                            Expr::Name(n) => n == name,
                            Expr::Attr(obj, _) => matches!(&**obj, Expr::Name(n) if n == name),
                            _ => false,
                        };
                        if !self_ref {
                            let mut v = self.eval(shape, fi, 0, &Env::default());
                            v.merge(&Val::one(Ty::Var((fi, c))));
                            return v;
                        }
                    }
                }
                return Val::one(self.def_ty((fi, c)));
            }
            if d == 0 {
                if let Some((module, member)) = f.imports.get(name) {
                    return self.resolve_import(module, member.as_deref(), fi, 0, &mut Vec::new());
                }
                if let Some(shape) = f.aliases.get(name) {
                    // `X = X` and `X = X.attr` rebind a builtin or an imported
                    // name to itself; evaluating them would loop.
                    let self_ref = match shape {
                        Expr::Name(n) => n == name,
                        Expr::Attr(obj, _) => matches!(&**obj, Expr::Name(n) if n == name),
                        _ => false,
                    };
                    if !self_ref {
                        return self.eval(shape, fi, 0, &Env::default());
                    }
                    return Val::external();
                }
                for star in &f.stars {
                    if let Some(sf) = self.module_file_from(star, fi) {
                        let v = self.resolve_export(sf, name);
                        if v.has_project() {
                            return v;
                        }
                    }
                }
                break;
            }
            first = false;
            cur = f.defs[d as usize].parent;
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
                Ty::Module(f) => out.merge(&self.resolve_export(*f, attr)),
                Ty::Class(c) => match self.members.get(c).and_then(|m| m.get(attr)) {
                    Some(&d) => out.merge(&Val::one(self.def_ty((c.0, d)))),
                    None => out.unknown = true,
                },
                Ty::External => out.merge(&Val::external()),
                _ => out.unknown = true,
            }
        }
        out
    }

    /// Resolve a dotted path such as `pkg.mod.Class` from a scope.
    pub fn resolve_dotted(&self, path: &str, fi: u32, scope: u32, env: &Env) -> Val {
        let mut parts = path.split('.').map(str::trim);
        let Some(first) = parts.next() else {
            return Val::unknown();
        };
        let mut v = self.lookup(first, fi, scope, env);
        for p in parts {
            v = self.attr_static(&v, p);
        }
        v
    }

    /// The instance type named by an annotation.
    pub fn ann_val(&self, ann: &str, fi: u32, scope: u32, depth: u8) -> Val {
        let Some(_g) = DepthGuard::enter() else {
            return Val::unknown();
        };
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
                v.merge(&self.ann_val(p, fi, scope, depth + 1));
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
                "Optional" => self.ann_val(inner, fi, scope, depth + 1),
                "Union" => {
                    let mut v = Val::none();
                    for p in split_top(inner, ',') {
                        v.merge(&self.ann_val(p, fi, scope, depth + 1));
                    }
                    v
                }
                "Annotated" | "Final" | "ClassVar" | "Required" | "NotRequired" => {
                    self.ann_val(split_top(inner, ',')[0], fi, scope, depth + 1)
                }
                "type" | "Type" => {
                    let mut v = self.ann_val(inner, fi, scope, depth + 1);
                    for ty in v.tys.iter_mut() {
                        if let Ty::Inst(c) = ty {
                            *ty = Ty::Class(*c);
                        }
                    }
                    v
                }
                _ => {
                    let v = self.resolve_dotted(head, fi, scope, &Env::default());
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
            return match self.enclosing_class(fi, scope) {
                Some(c) => Val::one(Ty::Inst(c)),
                None => Val::unknown(),
            };
        }
        if t == "Any" || t == "object" || t.ends_with(".Any") {
            return Val::unknown();
        }
        let v = self.resolve_dotted(t, fi, scope, &Env::default());
        let mut out = Val::none();
        out.unknown = v.unknown;
        for ty in &v.tys {
            match ty {
                Ty::Class(c) => out.merge(&Val::one(Ty::Inst(*c))),
                // A type alias (`Alias = Callable[...]`) is a module variable;
                // the annotation reads it even though it names no class.
                Ty::Var(d) => out.merge(&Val::one(Ty::Var(*d))),
                _ => out.merge(&Val::external()),
            }
        }
        out
    }

    /// Linearised ancestors of `class`, the class first.
    pub fn mro(&self, class: GId) -> Vec<GId> {
        let mut out = Vec::new();
        let mut stack = vec![class];
        while let Some(c) = stack.pop() {
            if out.contains(&c) {
                continue;
            }
            out.push(c);
            if let Some(bs) = self.bases.get(&c) {
                for b in bs.iter().rev() {
                    stack.push(*b);
                }
            }
        }
        out
    }

    /// Find `name` on `class` or its ancestors.
    pub fn find_member(&self, class: GId, name: &str, skip_self: bool) -> Option<GId> {
        self.mro(class)
            .into_iter()
            .skip(usize::from(skip_self))
            .find_map(|c| {
                self.members
                    .get(&c)
                    .and_then(|m| m.get(name))
                    .map(|&d| (c.0, d))
            })
    }

    /// Every class below `class` in the hierarchy.
    pub fn descendants(&self, class: GId) -> Vec<GId> {
        let mut out = Vec::new();
        let mut queue: VecDeque<GId> = VecDeque::from([class]);
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

    /// Methods with this name anywhere in the project.
    pub fn methods_named(&self, name: &str) -> &[GId] {
        self.methods_by_name
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Whether `name` is a method name anywhere in the project.
    pub fn is_method_name(&self, name: &str) -> bool {
        self.methods_by_name.contains_key(name)
    }

    fn class_attr(&self, class: GId, name: &str) -> Option<&Val> {
        self.mro(class)
            .into_iter()
            .find_map(|c| self.class_attr_vals.get(&(c, name.to_string())))
    }

    /// Whether a definition is a property getter.
    pub fn is_property(&self, g: GId) -> bool {
        self.def(g)
            .decorators
            .iter()
            .any(|x| x == "property" || x.ends_with("cached_property") || x.ends_with(".property"))
    }

    /// Whether a definition is a Protocol class.
    pub fn is_protocol(&self, g: GId) -> bool {
        self.protocols.contains(&g)
    }

    /// What calling `g` returns.
    pub fn ret_val(&self, g: GId) -> Val {
        let Some(_g) = DepthGuard::enter() else {
            return Val::unknown();
        };
        let d = self.def(g);
        if d.kind == DefKind::Class {
            return Val::one(Ty::Inst(g));
        }
        if let Some(r) = &d.returns {
            return self.ann_val(r, g.0, g.1, 0);
        }
        match self.inferred_returns.get(&g) {
            Some(c) => Val::one(Ty::Inst(*c)),
            None => Val::unknown(),
        }
    }

    /// The environment a function scope starts with: its typed parameters
    /// and the receiver type of a method's first parameter.
    pub fn seed_params(&self, fi: u32, scope: u32) -> Env {
        let mut env = Env::default();
        let d = &self.files[fi as usize].defs[scope as usize];
        if !matches!(d.kind, DefKind::Function | DefKind::Method) {
            return env;
        }
        let is_static = d.decorators.iter().any(|x| x == "staticmethod");
        let is_classmethod = d.decorators.iter().any(|x| x == "classmethod");
        for (i, (name, ann)) in d.params.iter().enumerate() {
            let v = match ann {
                Some(t) => self.ann_val(t, fi, scope, 0),
                None if i == 0 && d.kind == DefKind::Method && !is_static => match d.class {
                    Some(c) if is_classmethod => Val::one(Ty::Class((fi, c))),
                    Some(c) => Val::one(Ty::Inst((fi, c))),
                    None => Val::unknown(),
                },
                None => Val::unknown(),
            };
            env.insert_param(name.clone(), v);
        }
        env
    }

    /// Evaluate a shape in scope `scope` of file `fi`.
    pub fn eval(&self, e: &Expr, fi: u32, scope: u32, env: &Env) -> Val {
        self.eval_depth(e, fi, scope, env, 0)
    }

    fn eval_depth(&self, e: &Expr, fi: u32, scope: u32, env: &Env, depth: u8) -> Val {
        let Some(_g) = DepthGuard::enter() else {
            return Val::unknown();
        };
        if depth > 32 {
            return Val::unknown();
        }
        match e {
            Expr::Name(n) => self.lookup(n, fi, scope, env),
            Expr::Super => match self.enclosing_class(fi, scope) {
                Some(c) => Val::one(Ty::Super(c)),
                None => Val::unknown(),
            },
            Expr::Call(f) => {
                let fv = self.eval_depth(f, fi, scope, env, depth + 1);
                let mut out = Val::none();
                out.unknown = fv.unknown;
                for ty in &fv.tys {
                    match ty {
                        Ty::Class(c) => out.merge(&Val::one(Ty::Inst(*c))),
                        Ty::Func(d) => out.merge(&self.ret_val(*d)),
                        Ty::External => out.merge(&Val::external()),
                        _ => out.unknown = true,
                    }
                }
                out
            }
            Expr::Attr(obj, attr) => {
                let v = self.eval_depth(obj, fi, scope, env, depth + 1);
                self.attr_val(&v, attr)
            }
            Expr::Enter(inner) => {
                let v = self.eval_depth(inner, fi, scope, env, depth + 1);
                let mut out = Val::none();
                out.unknown = v.unknown;
                for ty in &v.tys {
                    match ty {
                        Ty::Inst(c) => match self.find_member(*c, "__enter__", false) {
                            Some(m) => {
                                let r = self.ret_val(m);
                                // `__enter__` returning Self or nothing known: the object itself.
                                if r.has_project() {
                                    out.merge(&r);
                                } else if self.def(m).returns.is_none() {
                                    out.merge(&Val::one(Ty::Inst(*c)));
                                } else {
                                    out.merge(&r);
                                }
                            }
                            None => out.merge(&Val::one(Ty::Inst(*c))),
                        },
                        Ty::External => out.merge(&Val::external()),
                        _ => out.unknown = true,
                    }
                }
                out
            }
            Expr::Either(items) => {
                let mut out = Val::none();
                for i in items {
                    out.merge(&self.eval_depth(i, fi, scope, env, depth + 1));
                }
                out
            }
            Expr::NoneLit => Val::none(),
            Expr::Known => Val::external(),
            Expr::Unknown => Val::unknown(),
        }
    }

    /// The value of `.attr` on each candidate type.
    pub fn attr_val(&self, v: &Val, attr: &str) -> Val {
        let Some(_g) = DepthGuard::enter() else {
            return Val::unknown();
        };
        let mut out = Val::none();
        out.unknown = v.unknown;
        for ty in &v.tys {
            match ty {
                Ty::Inst(c) | Ty::Class(c) => {
                    if let Some(a) = self.class_attr(*c, attr) {
                        out.merge(a);
                    } else if let Some(d) = self.find_member(*c, attr, false) {
                        if self.def(d).kind == DefKind::Class {
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
                Ty::Module(f) => out.merge(&self.resolve_export(*f, attr)),
                Ty::External => out.merge(&Val::external()),
                Ty::Func(_) | Ty::Var(_) => out.unknown = true,
            }
        }
        out
    }
}

/// Local bindings of one scope: name → value, plus the names bound at all
/// (so an unknown local shadows an outer definition).
#[derive(Clone, Debug, Default)]
pub struct Env {
    vals: HashMap<String, Val>,
    bound: HashSet<String>,
    params: HashSet<String>,
}

impl Env {
    fn get(&self, name: &str) -> Option<&Val> {
        self.vals.get(name)
    }
    fn insert(&mut self, name: String, v: Val) {
        self.bound.insert(name.clone());
        self.vals.entry(name).or_default().merge(&v);
    }
    fn insert_param(&mut self, name: String, v: Val) {
        self.params.insert(name.clone());
        self.insert(name, v);
    }
    fn is_param(&self, name: &str) -> bool {
        self.params.contains(name)
    }
}

/// One resolved site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Site {
    /// Definition reached.
    pub target: GId,
    /// Definition the site is attributed to.
    pub owner: GId,
    /// Line in the owner's file.
    pub line: u32,
    /// Column in that line (1-based); two sites on one line are distinct.
    #[serde(default)]
    pub column: u32,
    /// Kind of use.
    pub kind: SiteKind,
    /// Whether the target was proven.
    pub certain: bool,
    /// Why not certain (empty when certain).
    pub why: String,
}

/// A call whose receiver type could not be determined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownCall {
    /// Member name called.
    pub name: String,
    /// Owner of the call site.
    pub owner: GId,
    /// Line.
    pub line: u32,
}

/// Why a span of a file leaves a question open. Each maps onto one loss
/// reason the report authority already understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LossKind {
    /// An annotation whose contract no type-flow evidence backs
    /// (`capability_unavailable` / `type_flows`).
    TypeFlows,
    /// An annotation naming a type the resolver cannot bind
    /// (`unresolved_type` / `type_resolution`).
    UnresolvedType,
    /// A runtime namespace lookup (`unresolved_reference` / `consumer_closure`).
    DynamicNamespace,
    /// A `match` statement (`unsupported_construct` / `control_flow`).
    MatchStatement,
    /// A relative import the resolver could not place in the project
    /// (`unresolved_import`).
    UnresolvedImport,
}

/// One open question, with the span it is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loss {
    /// File the span is in.
    pub file: u32,
    /// Byte span.
    pub start: u32,
    /// End byte, exclusive.
    pub end: u32,
    /// What is open.
    pub kind: LossKind,
}

/// The output of resolving one file.
#[derive(Debug, Default, Clone)]
pub struct Resolved {
    /// Sites found in this file.
    pub sites: Vec<Site>,
    /// Calls with an unknown receiver, by member name.
    pub unknown_calls: Vec<UnknownCall>,
    /// Open questions in this file.
    pub losses: Vec<Loss>,
}

/// Builtin type names an annotation may use without a project binding.
const ANNOTATION_BUILTINS: &[&str] = &[
    "int",
    "str",
    "float",
    "bool",
    "bytes",
    "bytearray",
    "list",
    "tuple",
    "dict",
    "set",
    "frozenset",
    "object",
    "type",
    "complex",
    "None",
    "Any",
    "Optional",
    "Union",
    "Callable",
    "Iterable",
    "Iterator",
    "Sequence",
    "Mapping",
    "List",
    "Dict",
    "Set",
    "Tuple",
    "Self",
];

/// The root identifier of an annotation text: `Optional[Foo]` → `Optional`,
/// `pkg.mod.Cls[int]` → `pkg`, `"Foo"` → `Foo`.
fn annotation_root(text: &str) -> Option<&str> {
    let t = text.trim().trim_matches(|c| c == '"' || c == '\'').trim();
    let head = t.split(['[', '|', '.', ',', ' ']).next()?.trim();
    (!head.is_empty() && head.chars().all(|c| c.is_alphanumeric() || c == '_')).then_some(head)
}

/// Resolve one file's facts against the export table.
pub fn resolve_file(ex: &Exports, fi: u32, facts: &FactSet) -> Resolved {
    let mut out = Resolved::default();
    // Imports bound inside function scopes are local to that scope; seed
    // them into the scope's environment so lookup sees them before outer names.
    let mut scoped_imports: HashMap<u32, Vec<(String, Val)>> = HashMap::new();
    for i in &facts.imports {
        if i.scope == 0 {
            continue;
        }
        let v = ex.resolve_import(&i.module, i.member.as_deref(), fi, 0, &mut Vec::new());
        scoped_imports
            .entry(i.scope)
            .or_default()
            .push((i.local.clone(), v));
    }
    // Per-scope environments: parameters, then bindings (two rounds so a
    // binding that depends on a later one settles).
    let mut envs: HashMap<u32, Env> = HashMap::new();
    for (di, d) in facts.defs.iter().enumerate() {
        if d.kind != DefKind::Class {
            let mut env = ex.seed_params(fi, di as u32);
            for (local, v) in scoped_imports.get(&(di as u32)).into_iter().flatten() {
                env.insert(local.clone(), v.clone());
            }
            envs.insert(di as u32, env);
        }
    }
    for _ in 0..2 {
        let snapshot = envs.clone();
        for b in &facts.binds {
            let BindTarget::Local(name) = &b.target else {
                continue;
            };
            let env = snapshot.get(&b.scope).cloned().unwrap_or_default();
            let v = match &b.annotation {
                Some(t) => ex.ann_val(t, fi, b.scope, 0),
                None => ex.eval(&b.value, fi, b.scope, &env),
            };
            let target_env = envs.entry(b.scope).or_default();
            // A parameter's shadow binding carries no value of its own; the
            // seeded parameter type stands. Any other binding of the name
            // widens it, including to unknown.
            if b.value == Expr::Unknown && b.annotation.is_none() && target_env.is_param(name) {
                continue;
            }
            let mut v = v;
            // A module-scope binding of a module-level variable definition is
            // that variable: a read of the name is a reference site to it, in
            // this file or any other, so carry its identity with the value.
            if b.scope == 0 {
                if let Some(&c) = ex.members.get(&(fi, 0)).and_then(|m| m.get(name.as_str())) {
                    if ex.files[fi as usize].defs[c as usize].kind == DefKind::Variable {
                        v.merge(&Val::one(Ty::Var((fi, c))));
                    }
                }
            }
            target_env.insert(name.clone(), v);
        }
    }
    let empty = Env::default();
    let mut push =
        |target: GId, owner: GId, at: (u32, u32), kind: SiteKind, certain: bool, why: &str| {
            out.sites.push(Site {
                target,
                owner,
                line: at.0,
                column: at.1,
                kind,
                certain,
                why: if certain {
                    String::new()
                } else {
                    why.to_string()
                },
            });
        };
    for r in &facts.refs {
        let env = envs.get(&r.scope).unwrap_or(&empty);
        let owner = (fi, r.owner);
        // `@register def _callback` passes `_callback` into project code,
        // which may keep it (a registry, a route table). The decorated
        // definition is consumed by its enclosing scope at the decorator
        // line. An external decorator (`@lru_cache`, `@property`) wraps but
        // registers nothing, so it consumes nothing.
        if r.kind == SiteKind::Decorator && ex.eval(&r.expr, fi, r.scope, env).has_project() {
            let enclosing = (fi, facts.defs[r.owner as usize].parent.unwrap_or(0));
            push(
                owner,
                enclosing,
                (r.line, r.column),
                SiteKind::Reference,
                true,
                "",
            );
        }
        match &r.expr {
            Expr::Name(_) => {
                let v = ex.eval(&r.expr, fi, r.scope, env);
                record_named(ex, &v, owner, (r.line, r.column), r.kind, &mut push);
            }
            Expr::Attr(obj, name) => {
                let recv = ex.eval(obj, fi, r.scope, env);
                if recv.has_project() {
                    record_member(
                        ex,
                        &recv,
                        name,
                        owner,
                        (r.line, r.column),
                        r.kind,
                        &mut push,
                    );
                } else if r.kind == SiteKind::Call && recv.unknown && ex.is_method_name(name) {
                    out.unknown_calls.push(UnknownCall {
                        name: name.clone(),
                        owner,
                        line: r.line,
                    });
                }
            }
            _ => {}
        }
    }
    for i in &facts.imports {
        if i.member.is_none() {
            continue;
        }
        for ty in ex
            .resolve_import(&i.module, i.member.as_deref(), fi, 0, &mut Vec::new())
            .tys
        {
            if let Ty::Func(d) | Ty::Class(d) = ty {
                push(d, (fi, i.scope), (i.line, 1), SiteKind::Import, true, "");
            }
        }
    }
    let mut seen = HashSet::new();
    out.sites
        .retain(|s| seen.insert((s.target, s.owner, s.line, s.column, s.kind)));
    // Open questions. A relative import the table cannot place is an
    // obligation the file declared; an unknown annotation root is a contract
    // the resolver cannot bind; every annotation lacks type-flow evidence.
    for i in &facts.imports {
        if i.relative && ex.module_file_from(&i.module, fi).is_none() {
            out.losses.push(Loss {
                file: fi,
                start: i.span.0,
                end: i.span.1,
                kind: LossKind::UnresolvedImport,
            });
        }
    }
    for h in &facts.hazards {
        let kind = match h.kind {
            HazardKind::MatchStatement => LossKind::MatchStatement,
            HazardKind::DynamicNamespace => {
                const NAMESPACE_OPS: &[&str] =
                    &["globals", "locals", "eval", "exec", "getattr", "__import__"];
                let name = h.text.split('(').next().unwrap_or("").trim();
                // The original name: the called name itself, or what an
                // import bound it to (`from builtins import getattr as lookup`).
                let import = facts
                    .imports
                    .iter()
                    .filter(|i| i.local == name)
                    .find(|i| i.scope == h.scope || i.scope == 0);
                let original = import.and_then(|i| i.member.as_deref()).unwrap_or(name);
                if !NAMESPACE_OPS.contains(&original) {
                    continue;
                }
                // A project definition of the name is the provider, not the
                // builtin; so is an import from anywhere but `builtins`.
                let env = envs.get(&h.scope).unwrap_or(&empty);
                if ex.lookup(name, fi, h.scope, env).has_project() {
                    continue;
                }
                if import.is_some_and(|i| i.module != "builtins") {
                    continue;
                }
                LossKind::DynamicNamespace
            }
            HazardKind::Annotation => {
                out.losses.push(Loss {
                    file: fi,
                    start: h.start,
                    end: h.end,
                    kind: LossKind::TypeFlows,
                });
                let Some(root) = annotation_root(&h.text) else {
                    continue;
                };
                if ANNOTATION_BUILTINS.contains(&root) {
                    continue;
                }
                let env = envs.get(&h.scope).unwrap_or(&empty);
                let v = ex.lookup(root, fi, h.scope, env);
                if v.has_project() || !v.unknown {
                    continue;
                }
                LossKind::UnresolvedType
            }
        };
        out.losses.push(Loss {
            file: fi,
            start: h.start,
            end: h.end,
            kind,
        });
    }
    out
}

fn record_named(
    ex: &Exports,
    v: &Val,
    owner: GId,
    at: (u32, u32),
    kind: SiteKind,
    push: &mut impl FnMut(GId, GId, (u32, u32), SiteKind, bool, &str),
) {
    // A variable alongside the value it aliases is one certain reference to
    // each, not an ambiguity.
    let certain = v.sure()
        || (!v.unknown
            && v.tys
                .iter()
                .filter(|t| !matches!(t, Ty::Var(_) | Ty::External))
                .count()
                <= 1
            && v.tys.iter().any(|t| matches!(t, Ty::Var(_))));
    // Reading a name bound to exactly one module variable is a certain
    // reference to that variable even when its value cannot be typed: the
    // uncertainty is about what it holds, not about which binding is read.
    let vars = v.tys.iter().filter(|t| matches!(t, Ty::Var(_))).count();
    let var_certain = certain || vars == 1;
    for ty in &v.tys {
        match ty {
            Ty::Var(d) => push(
                *d,
                owner,
                at,
                SiteKind::Reference,
                var_certain,
                "more than one possible target",
            ),
            Ty::Func(d) => push(
                *d,
                owner,
                at,
                kind,
                certain,
                "more than one possible target",
            ),
            Ty::Class(c) => {
                push(
                    *c,
                    owner,
                    at,
                    kind,
                    certain,
                    "more than one possible target",
                );
                if kind == SiteKind::Call {
                    if let Some(init) = ex.find_member(*c, "__init__", false) {
                        push(
                            init,
                            owner,
                            at,
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

fn record_member(
    ex: &Exports,
    recv: &Val,
    name: &str,
    owner: GId,
    at: (u32, u32),
    kind: SiteKind,
    push: &mut impl FnMut(GId, GId, (u32, u32), SiteKind, bool, &str),
) {
    let certain = recv.sure();
    for ty in &recv.tys {
        let (class, skip_self) = match ty {
            Ty::Inst(c) | Ty::Class(c) => (*c, false),
            Ty::Super(c) => (*c, true),
            Ty::Module(f) => {
                let v = ex.resolve_export(*f, name);
                record_named(ex, &v, owner, at, kind, push);
                continue;
            }
            _ => continue,
        };
        let found = ex.find_member(class, name, skip_self);
        if let Some(d) = found {
            if ex.def(d).kind == DefKind::Class {
                record_named(ex, &Val::one(Ty::Class(d)), owner, at, kind, push);
            } else {
                // A property read runs the getter: that is a call.
                let k = if kind == SiteKind::Reference && ex.is_property(d) {
                    SiteKind::Call
                } else {
                    kind
                };
                push(
                    d,
                    owner,
                    at,
                    k,
                    certain,
                    "receiver has more than one possible type",
                );
            }
        }
        if skip_self || kind != SiteKind::Call {
            continue;
        }
        for sub in ex.descendants(class) {
            if let Some(o) = ex.find_member(sub, name, false) {
                if Some(o) != found && o.0 == sub.0 && ex.def(o).parent == Some(sub.1) {
                    let why = format!("receiver typed as base class {}", ex.def(class).name);
                    push(o, owner, at, kind, false, &why);
                }
            }
        }
        if ex.is_protocol(class) {
            for &m in ex.methods_named(name) {
                if Some(m) != found {
                    let why = format!("receiver typed as protocol {}", ex.def(class).name);
                    push(m, owner, at, kind, false, &why);
                }
            }
        }
    }
}
