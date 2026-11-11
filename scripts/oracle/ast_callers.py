#!/usr/bin/env python3
"""Independent, conservative AST caller manifest for Python source trees."""
import argparse
import ast
import json
from collections import defaultdict
from pathlib import Path


def symbols(root):
    trees = {}
    defs = {}
    imports = {}
    bases = {}
    for path in sorted(root.rglob('*.py')):
        rel = path.relative_to(root).as_posix()
        try:
            tree = ast.parse(path.read_text(), filename=rel)
        except (SyntaxError, UnicodeError):
            continue
        trees[rel] = tree
        aliases = {}
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                for a in node.names:
                    aliases[a.asname or a.name.split('.')[0]] = a.name
            elif isinstance(node, ast.ImportFrom) and node.module:
                for a in node.names:
                    aliases[a.asname or a.name] = node.module + '.' + a.name
        imports[rel] = aliases
        for node in tree.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                key = f'{rel}::{node.name}'
                defs[key] = {'file': rel, 'line': node.lineno, 'name': node.name,
                             'class': None, 'decorators': [ast.unparse(d) for d in node.decorator_list]}
            elif isinstance(node, ast.ClassDef):
                bases[(rel, node.name)] = [ast.unparse(b).split('.')[-1] for b in node.bases]
                for child in node.body:
                    if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef)):
                        key = f'{rel}::{node.name}::{child.name}'
                        defs[key] = {'file': rel, 'line': child.lineno, 'name': child.name,
                                     'class': node.name, 'decorators': [ast.unparse(d) for d in child.decorator_list]}
    return trees, defs, imports, bases


def analyze(root):
    trees, defs, imports, bases = symbols(root)
    factories = {}
    by_class = defaultdict(list)
    by_name = defaultdict(list)
    for key, val in defs.items():
        by_name[val['name']].append(key)
        if val['class']:
            by_class[val['class']].append(key)

    def class_key(name, file):
        name = name.split('.')[-1].split('[')[0]
        options = [(f, c) for f, c in bases if c == name]
        if (file, name) in options:
            return (file, name)
        imp = imports[file].get(name, '')
        if imp:
            matches = [(f, c) for f, c in options if f.replace('/', '.').removesuffix('.py').endswith(imp.rsplit('.', 1)[0])]
            if len(matches) == 1:
                return matches[0]
        return options[0] if len(options) == 1 else None

    def method(cls, name, seen=None):
        if cls is None:
            return None
        seen = seen or set()
        if cls in seen:
            return None
        seen.add(cls)
        k = f'{cls[0]}::{cls[1]}::{name}'
        if k in defs:
            return k
        for b in bases.get(cls, []):
            parent = class_key(b, cls[0])
            found = method(parent, name, seen)
            if found:
                return found
        return None

    def direct(name, file):
        here = f'{file}::{name}'
        if here in defs:
            return here
        alias = imports[file].get(name, '')
        if alias:
            matches = [k for k in by_name[alias.split('.')[-1]]
                       if defs[k]['class'] is None and k.split('::')[0].replace('/', '.').removesuffix('.py').endswith(alias.rsplit('.', 1)[0])]
            if len(matches) == 1:
                return matches[0]
        return None

    for file, tree in trees.items():
        for node in tree.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                returned = [n.value for n in ast.walk(node) if isinstance(n, ast.Return)]
                if len(returned) == 1 and isinstance(returned[0], ast.Call) and isinstance(returned[0].func, ast.Name):
                    cls = class_key(returned[0].func.id, file)
                    if cls:
                        factories[f'{file}::{node.name}'] = cls

    sites = defaultdict(lambda: {'certain': [], 'candidate': []})
    class Scanner(ast.NodeVisitor):
        def __init__(self, file):
            self.file = file
            self.cls = None
            self.owner = None
            self.env = {}
            self.self_attrs = defaultdict(dict)
            self.call_targets = {id(n.func) for n in ast.walk(trees[file]) if isinstance(n, ast.Call)}

        def type_of(self, expr):
            if isinstance(expr, ast.Name):
                if expr.id == 'self':
                    return self.cls
                return self.env.get(expr.id) or class_key(expr.id, self.file)
            if isinstance(expr, ast.Call) and isinstance(expr.func, ast.Name):
                return class_key(expr.func.id, self.file) or factories.get(direct(expr.func.id, self.file))
            if isinstance(expr, ast.Attribute) and isinstance(expr.value, ast.Name) and expr.value.id == 'self':
                return self.self_attrs[self.cls].get(expr.attr)
            return None

        def visit_ClassDef(self, node):
            prior = self.cls
            self.cls = (self.file, node.name)
            for child in node.body:
                self.visit(child)
            self.cls = prior

        def visit_FunctionDef(self, node):
            prior_owner, prior_env = self.owner, self.env
            self.owner = (f'{prior_owner}::{node.name}' if prior_owner else
                          f'{self.file}::{self.cls[1]}::{node.name}' if self.cls else
                          f'{self.file}::{node.name}')
            self.env = {}
            for deco in node.decorator_list:
                if isinstance(deco, ast.Name):
                    key = direct(deco.id, self.file)
                    if key:
                        sites[key]['candidate'].append({'file': self.file, 'line': deco.lineno,
                            'caller': self.owner, 'reason': 'decorator reference, not an explicit call', 'expr': deco.id})
            args = node.args.posonlyargs + node.args.args + node.args.kwonlyargs
            for arg in args:
                if arg.annotation:
                    c = class_key(ast.unparse(arg.annotation), self.file)
                    if c:
                        self.env[arg.arg] = c
            for child in node.body:
                self.visit(child)
            self.owner, self.env = prior_owner, prior_env

        visit_AsyncFunctionDef = visit_FunctionDef

        def visit_Assign(self, node):
            cls = self.type_of(node.value)
            if cls:
                for target in node.targets:
                    if isinstance(target, ast.Name):
                        self.env[target.id] = cls
                    elif isinstance(target, ast.Attribute) and isinstance(target.value, ast.Name) and target.value.id == 'self' and self.cls:
                        self.self_attrs[self.cls][target.attr] = cls
            self.generic_visit(node)

        def visit_AnnAssign(self, node):
            if isinstance(node.target, ast.Name):
                cls = class_key(ast.unparse(node.annotation), self.file)
                if cls:
                    self.env[node.target.id] = cls
            self.generic_visit(node)

        def visit_Call(self, node):
            if not self.owner:
                self.generic_visit(node)
                return
            key = None
            name = None
            reason = ''
            f = node.func
            if isinstance(f, ast.Name):
                name = f.id
                key = direct(name, self.file)
                reason = 'local or imported function alias'
            elif isinstance(f, ast.Attribute):
                name = f.attr
                if isinstance(f.value, ast.Call) and isinstance(f.value.func, ast.Name) and f.value.func.id == 'super':
                    for b in bases.get(self.cls, []) if self.cls else []:
                        key = method(class_key(b, self.file), name)
                        if key:
                            break
                    reason = 'super() base method'
                elif isinstance(f.value, ast.Name) and f.value.id in imports[self.file] and '.' in imports[self.file][f.value.id]:
                    module = imports[self.file][f.value.id]
                    matches = [k for k in by_name[name] if k.split('::')[0].replace('/', '.').removesuffix('.py').endswith(module) and defs[k]['class'] is None]
                    key = matches[0] if len(matches) == 1 else None
                    reason = 'module import alias'
                if not key:
                    cls = self.type_of(f.value)
                    key = method(cls, name)
                    reason = 'receiver annotation, constructor, self, or inheritance' if cls else 'untyped receiver'
            if name:
                record = {'file': self.file, 'line': node.lineno, 'caller': self.owner, 'reason': reason, 'expr': ast.unparse(node.func)}
                if key:
                    sites[key]['certain'].append(record)
                else:
                    for candidate in by_name[name]:
                        sites[candidate]['candidate'].append(record)
            self.generic_visit(node)

        def visit_Attribute(self, node):
            if self.owner and id(node) not in self.call_targets:
                cls = self.type_of(node.value)
                key = method(cls, node.attr)
                if key and any(d == 'property' or d.endswith('.property') for d in defs[key]['decorators']):
                    sites[key]['candidate'].append({'file': self.file, 'line': node.lineno,
                        'caller': self.owner, 'reason': 'property access, not a call', 'expr': ast.unparse(node)})
            self.generic_visit(node)

        def visit_Name(self, node):
            if self.owner and isinstance(node.ctx, ast.Load) and id(node) not in self.call_targets:
                key = direct(node.id, self.file)
                if key:
                    sites[key]['candidate'].append({'file': self.file, 'line': node.lineno,
                        'caller': self.owner, 'reason': 'function value, callback/decorator candidate', 'expr': node.id})

    for file, tree in trees.items():
        Scanner(file).visit(tree)
    result = []
    for key in sorted(defs):
        result.append({'symbol': key, 'definition': defs[key],
                       'certain': sorted(sites[key]['certain'], key=lambda s: (s['file'], s['line'])),
                       'candidate': sorted(sites[key]['candidate'], key=lambda s: (s['file'], s['line']))})
    return result


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('root', type=Path)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--limit', type=int, default=40)
    args = ap.parse_args()
    all_cases = analyze(args.root.resolve())
    usable = [c for c in all_cases if c['certain']]
    usable.sort(key=lambda c: (-len(c['certain']), c['symbol']))
    picked = usable[:args.limit]
    fixed = next((c for c in all_cases if c['symbol'].endswith('core/sheet.py::Sheet::template_variables') or c['symbol'].endswith('sheet.py::Sheet::template_variables')), None)
    extras = [fixed,
              next((c for c in usable if any('super()' in s['expr'] for s in c['certain'])), None),
              next((c for c in all_cases if any(s['reason'].startswith('property access') for s in c['candidate'])), None),
              next((c for c in all_cases if any('function value' in s['reason'] for s in c['candidate'])), None),
              next((c for c in all_cases if any('decorator reference' in s['reason'] for s in c['candidate'])), None)]
    additions = [item for item in extras if item and item not in picked]
    if additions:
        picked = picked[:max(0, args.limit - len(additions))] + additions
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps({'root': str(args.root.resolve()), 'cases': picked}, indent=2) + '\n')
    print(f'{len(picked)} cases; {sum(len(c["certain"]) for c in picked)} certain sites; {sum(len(c["candidate"]) for c in picked)} candidates')


if __name__ == '__main__':
    main()
