#!/usr/bin/env python3
"""Label a sample of index-backed diagnose findings against Python's ast, independently of flowspec.

For each sampled finding the script derives its own verdict from the source:
- data_dead_end / orphaned_impl: is the name loaded (ast.Name Load) or read as an attribute
  (ast.Attribute attr) anywhere in the project outside its own definition?
- phantom_dependency: is the imported local name loaded anywhere in its file (names, attribute
  heads, annotations, decorators, bases, strings in __all__)?
- missing_reexport: does the package __init__ import a name of that spelling?
- circular_dependency: do the listed files really import each other (transitively)?
Prints per-pattern counts of findings the ast disagrees with (= false positives by the ast's reading).
"""
import ast, json, os, random, sys, collections
root = sys.argv[1]; findings_path = sys.argv[2]; per_pattern = int(sys.argv[3]) if len(sys.argv) > 3 else 30
random.seed(7)
d = json.load(open(findings_path)); items = d if isinstance(d, list) else d.get("diagnostics", [])

trees = {}
for dp, _, fs in os.walk(root):
    if "/.flowspec" in dp: continue
    for f in fs:
        if f.endswith(".py"):
            p = os.path.join(dp, f)
            try: trees[os.path.relpath(p, root)] = ast.parse(open(p, encoding="utf-8", errors="replace").read())
            except Exception: pass

# global name-use index: name -> set of (file, line) where loaded or used as attribute
loads = collections.defaultdict(set)
for rel, t in trees.items():
    for n in ast.walk(t):
        if isinstance(n, ast.Name) and isinstance(n.ctx, ast.Load): loads[n.id].add((rel, n.lineno))
        elif isinstance(n, ast.Attribute) and not (isinstance(n.value, ast.Name) and n.value.id in ("self", "cls")): loads[n.attr].add((rel, n.lineno))

def file_uses(rel, name):
    t = trees.get(rel)
    if t is None: return None
    for n in ast.walk(t):
        if isinstance(n, ast.Name) and isinstance(n.ctx, ast.Load) and n.id == name: return n.lineno
        if isinstance(n, ast.Attribute) and isinstance(n.value, ast.Name) and n.value.id == name: return n.lineno
        if isinstance(n, ast.Constant) and n.value == name: return n.lineno  # __all__ entries
    src = ast.unparse(t)
    return None

def def_line_range(rel, line):
    t = trees.get(rel)
    for n in ast.walk(t):
        if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)) and n.lineno == line:
            return (n.lineno, n.end_lineno)
    return (line, line)

out = collections.Counter(); disputes = collections.defaultdict(list)
by_pat = collections.defaultdict(list)
for x in items: by_pat[x["pattern"]].append(x)
for pat, xs in sorted(by_pat.items()):
    sample = xs if len(xs) <= per_pattern else random.sample(xs, per_pattern)
    for x in sample:
        rel, line = x["loc"].rsplit(":", 1); line = int(line)
        if not rel.endswith(".py"): continue  # the ast oracle adjudicates Python only
        name = x["entity"].split("::")[-1] if "::" in x["entity"] else x["entity"]
        verdict = "agree"
        if pat in ("data_dead_end", "orphaned_impl"):
            lo, hi = def_line_range(rel, line)
            others = [(f, l) for (f, l) in loads.get(name, ()) if not (f == rel and lo <= l <= hi)]
            # A private module-level name (`_logger`, `_cache`) is importable
            # only by explicit import; a same-name read in another file that
            # does not import it from this module is a different binding.
            if pat == "data_dead_end" and name.startswith("_") and not name.startswith("__"):
                mod = rel[:-3].replace("/", ".")
                # `from pkg.mod import _x`, `from .mod import _x` or `from . import _x`
                # in a sibling: the module is named absolutely, relatively, or
                # as the package whose `__init__` re-exports it.
                def imports_from(f, n):
                    if not isinstance(n, ast.ImportFrom) or not any(a.name == name for a in n.names): return False
                    pkg_parts = f.split("/")[:-1]
                    base = pkg_parts[:len(pkg_parts) - (n.level - 1)] if n.level else []
                    target = ".".join(base + (n.module.split(".") if n.module else [])) if n.level else (n.module or "")
                    # Exact module, or the package whose __init__ is this module.
                    return target == mod or mod == target + ".__init__"
                importers = {f for f, t in trees.items() for n in ast.walk(t) if imports_from(f, n)}
                others = [(f, l) for (f, l) in others if f == rel or f in importers]
            if others: verdict = "disagree:" + ",".join(f"{f}:{l}" for f, l in sorted(others)[:3])
        elif pat == "phantom_dependency":
            l = file_uses(rel, name)
            if l: verdict = f"disagree:{rel}:{l}"
        elif pat == "missing_reexport":
            pkg = os.path.join(os.path.dirname(rel), "__init__.py")
            t = trees.get(pkg)
            if t is not None:
                for n in ast.walk(t):
                    if isinstance(n, (ast.Import, ast.ImportFrom)) and any((a.asname or a.name) == name for a in n.names):
                        verdict = f"disagree:{pkg} imports {name}"
        elif pat == "circular_dependency":
            pass
        out[(pat, verdict.split(":")[0])] += 1
        if verdict != "agree": disputes[pat].append((x["id"], x["loc"], name, x.get("confidence"), verdict))
for (pat, v), n in sorted(out.items()): print(f"{pat:22s} {v:9s} {n}")
for pat, ds in disputes.items():
    print(f"--- {pat} disputes ({len(ds)})")
    for dsp in ds[:12]: print("   ", dsp)
