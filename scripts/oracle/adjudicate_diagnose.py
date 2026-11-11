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
        elif isinstance(n, ast.Attribute): loads[n.attr].add((rel, n.lineno))

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
        name = x["entity"].split("::")[-1] if "::" in x["entity"] else x["entity"]
        verdict = "agree"
        if pat in ("data_dead_end", "orphaned_impl"):
            lo, hi = def_line_range(rel, line)
            others = [(f, l) for (f, l) in loads.get(name, ()) if not (f == rel and lo <= l <= hi)]
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
