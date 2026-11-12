# Streaming reference index: design and sizing

Legion, 2026-11-11. Status: adopted; implementation in `flowspec/src/pyindex/`.
Supersedes the in-memory whole-program graph for Python analysis. The graph
(`flowspec/src/graph/`) remains for JavaScript/TypeScript and Rust until they
are ported.

## Goal

Analyse a Python codebase of several million lines on an 8–16 GB laptop with
bounded, predictable memory; answer "what reaches this symbol" in well under a
second once indexed; re-index only changed files; never lose a call site
silently. Precision of certain claims: no wrong certain caller on the oracle
corpora. Recall: every static call expression lands in `certain` or
`possible`; `unresolved` is counted and named, never dropped.

## Measured facts (Django 534 kLOC + CPython 1.12 MLOC, 2026-11-11)

Per 1,000 source lines: 81.6 definitions, 385 call expressions, 813 name
loads, 448 attribute expressions, 24 imports; 36.8 bytes per line.

Per file (5,224 files): top-level exports mean 6.6 (p99 61, max 244); imports
mean 7.6 (p99 54, max 160); definitions p50 6 (p99 309, max 1,965); file size
p50 2.5 KB, p99 136 KB, max 649 KB.

Receivers of attribute calls: `self`/`cls` 45.7%, a name bound in the same
function or at module level 30.7%, a dotted expression 7.5%, `self.attr`
5.4%, a call result 3.4%, a local constructor 2.7%, an untyped parameter
2.1%, an annotated parameter 0.2%. Of the 30.7% "bound name" bucket: the name
is an imported module 28.5%, a `from` import 13.3%, bound by a method-call
result 17.8%, by a constructor 7.5%, by a `with ... as` 5.0%.

Method names are reused: 43% of method definitions share their name with a
method in another class (`__init__` 3,360 times, `setUp` 1,189).

## Memory model

The design is a two-level index. Level 1 is a per-file **fact set**, produced
by one parse of that file alone, that depends on nothing outside the file.
Level 2 is a **global export table** derived from every fact set's exports.
Resolution of a file's references needs only its own fact set and the export
table, so files are processed one at a time and their parse trees and source
text are dropped before the next file is read.

### Level 1: fact set per file

Produced in one pass and serialised immediately (`.flowspec/index/facts/<hash>.bin`):

- definitions: kind, name, qualified path, line, decorators, bases (as text),
  return annotation (as text), parameter annotations (as text);
- imports: local name → (module, member, line, scope);
- bindings: for each function scope, name → the *shape* of its binding
  (constructor of `T`, call of `f`, method call `x.m()`, annotation `T`,
  `with ... as`, unpack, literal, other), with the referenced names as text;
- class attributes: `self.attr = <shape>` per class;
- references: every call, name load, attribute load and decorator, as
  (owner definition, line, kind, receiver expression *shape*, member name).

Nothing in a fact set is a global id; cross-file names stay as text until
resolution. A fact set's size is proportional to the file: by the densities
above it is roughly 1.4 facts per source line. At an average of 24 bytes per
serialised fact that is ~35 bytes per line, about the size of the source
itself. For 3 MLOC: ~105 MB on disk, read one file at a time.

### Level 2: export table

For each module: its top-level exports (name → definition id, kind) and its
star-import list. Mean 6.6 exports per file; 3 MLOC ≈ 80k files ≈ 530k
exports. At 48 bytes each (interned name id, definition id, kind, module id)
this is ~25 MB resident. Plus class skeletons needed for member lookup across
files: class id → bases (ids) and member name → definition id; ~27k classes
and ~108k methods per 1.65 MLOC scale to ~50k classes and ~200k methods per
3 MLOC: ~15 MB. The export table is therefore under 50 MB for 3 MLOC and is
the only structure that must be resident while resolving.

Module lookup for an import is the exact dotted name first (with the
analysis root's own package prefix, so `src/marianne` with an `__init__.py`
is `marianne`). When that fails the name is tried relative to each ancestor
directory of the importing file, nearest first: a script run from its own
directory has that directory on `sys.path`, so `from sibling import f` in
`tools/a.py` resolves to `tools/sibling.py` and `from repository.repo import x`
in `api/handler.py` resolves to `repository/repo.py` beside `api/`. This is
the `Exports::module_file_from` rule; the exact match still wins so package
imports are never shadowed by a same-named sibling.

The on-disk index may arrive with a checkout, so it is untrusted input.
Every cache file is `tag || payload`, the tag a SHA-256 over a per-user
32-byte key (kept outside any repository, `~/.cache/flowspec/index.key` by
default, created 0600), the file's name and the payload; a file whose tag
does not verify was not written by this user and is treated as absent.
Temporary files are created with `O_EXCL` and `O_NOFOLLOW`. Beyond that,
every cache file is decoded under a fixed size bound, a fact set is rejected
when any scope, owner, parent or class index falls outside its own
definition list or a parent does not point strictly backwards, retained
sites must name definitions present in the rebuilt export table, a symlinked
index directory is refused, and garbage collection only deletes regular
files named `<64 hex>.bin`. A rejected file is re-extracted from source.

### Resolution pass

Files are resolved one at a time against the export table. Within a file the
receiver-typing rules are the ones already implemented in `pyrefs.rs`
(annotation, constructor, `self.attr`, return annotation, single-constructor
return inference, `super()`, classmethod/staticmethod, property, module
attribute, alias), extended with two rules the receiver measurements justify:

- **`with x as y` typed by `__enter__`**: 5% of bound-name receivers.
- **method-call result typed by the callee's return annotation or
  single-constructor return**: 17.8% of bound-name receivers; requires the
  callee's definition, which the export table and class skeletons provide.

Each resolved reference becomes a **site** record appended to a per-target
posting list: (target id, owner id, file id, line, kind, certainty, reason
code). Posting lists are written to `.flowspec/index/sites.bin` sorted by
target. Memory during the pass: one file's facts plus the export table.

Unresolved receivers (the `untyped_param`, `call_result` with unknown callee,
`subscript`, external) are appended to a per-*member-name* list of unknown
calls, so a query for method `m` can list them as possible callers. Because
43% of method names are shared, these lists are what make "possible" large;
they are stored, counted, and reported separately, never merged into certain.

### Queries

"Who reaches `S`" is one lookup in the posting index (binary search on target
id over an on-disk sorted array, memory-mapped) plus one lookup in the
unknown-call list for `S`'s member name. Cost is O(log N + answer size). No
graph traversal. Transitive callers are a breadth-first walk over the same
posting index with a visited set bounded by the number of definitions.

### Incrementality

A file's fact set is keyed by the hash of its bytes. On a re-run, unchanged
files are not parsed. The export table is rebuilt from the stored fact sets
(cheap: it is the exports only). Resolution is re-run for changed files and
for every file whose *imports* name a changed module — the dependency is
recorded per file from its import list, so the set is exact, not a guess.
Posting lists for re-resolved files are rewritten by file id.

### Bound

Resident memory = export table + one file's facts and tree + output buffers.
With the largest observed file at 649 KB (16k lines), one file's tree and
facts are under 100 MB; the export table scales linearly in files at ~0.6 KB
per file. For 3 MLOC (80k files): ~50 MB + ~100 MB + buffers, well under
500 MB. The `--memory-limit` budget stays as the hard stop.

## Resident and bidirectional (plan of 2026-11-12)

Measured on Marianne warm with `-v` stage timing, 2026-11-12: the index
itself builds in 37 ms (323 files reused, 10 ms export table, 11 ms
resolve) and a warm `trace` takes 80 ms and 17 MB. `diagnose --language
python` takes 6.4 s and 379 MB, none of it in the index: 1.7 s parsing every
file into the legacy graph that the Python path then ignores, 1.5 s in the
index diagnostics (entity ids rebuilt per finding), 2.6 s in the authority
building a graph projection and 13,311 graph-derived losses. The plan,
recorded in the decision log, takes the process to the shape of the DDD
hash-tree in clamonacc: one key lookup lands on a node that already knows
both directions, startup is paid once.

1. Python never builds the graph. The index supplies custody, finding
   proofs, the analysis-graph projection and the Python prerequisite losses
   the authority tests pin, computed once at build. Mixed trees send Python
   to the index and other languages to the graph (issue #37). Target: warm
   diagnose under 0.5 s and 100 MB; `analyze` exits 0 on a clean tree.
2. Export-table snapshot and a forward posting list: sites as
   struct-of-arrays with a by-target offset array and a by-owner
   permutation; reason codes instead of strings.
3. Patterns read only the export table and posting lists, entity ids
   computed once per run; the per-file facts the three remaining patterns
   need are folded into the export table at build time.
4. `flowspec watch`: resident index, inotify-driven re-extraction of changed
   files, local socket for trace/diagnose/query, MCP on the same process.

## Diagnostics on the index

All thirteen patterns run on the index for Python-only analyses
(`flowspec/src/pyindex/diagnostics.rs`); the graph patterns remain for the
other languages. `analyze`'s entities, module summaries, flows, boundaries and
dependency graph for Python come from `flowspec/src/pyindex/manifest.rs`.
Private top-level functions the index proves unconsumed are handed to the
report-authority catalog as subjects (`report_authority::IndexSubject`) so
the finding proof, semantic identity and consumer-closure rule are the same
as the graph's.

`data_dead_end`, `phantom_dependency`, `missing_reexport`, `orphaned_impl`
and `isolated_cluster` become queries over the posting index and the export
table: a definition with an empty posting list and no possible callers is a
dead end; an import whose local name has no reference in its own file is
phantom; a public top-level definition in a sibling module absent from the
package's re-exports is a missing re-export. Each reads sites, not a graph.
`circular_dependency` reads only the import lists.

## Advisory findings and the proof catalog

A public top-level function is importable API: the indexed files cannot
prove it dead, so `data_dead_end` does not report it at all (the rule the
graph producer applied, kept so a one-function project with a public unused
function does not report itself incomplete). A diagnostic at **low**
confidence claims no absence: its evidence says the symbol *may* be consumed
outside the indexed scope (a public module-level binding nobody reads). It is
reported for the reader, but it enters no finding proof and leaves
`execution: complete`. Moderate and high findings are claims, and a claim
without a proof in the authority catalog is a `semantic_proof_unavailable`
loss, as before. This keeps the exit-code contract (0 clean, 2 gate fail,
3 incomplete) tied to what flowspec can stand behind.

Entity ids are spelled `file.py::Class::method`, the file name with its
extension, as the graph producer spelled qualified names; a directory prefix
is added only when two files define the same qualified name. A definition
decorated by a *project* decorator (`@register`) is consumed by its enclosing
scope at the decorator line: the decorator receives the function and may keep
it. An external decorator (`@lru_cache`, `@property`) wraps but registers
nothing. A lambda's parameters shadow outer names inside its body, so
`lambda _helper: _helper` reads the parameter, not a module function.

## What counts as a read

Measured against the ast oracle on Marianne (2026-11-12, 30 findings per
pattern): names read only in annotations (`x: Foo`, `-> Bar`, quoted forward
references, strings inside subscripts), in default values, in binary
expressions, or listed in a literal `__all__` are reads. A read of a name
bound to exactly one module variable is a certain reference to that
variable even when its value is untyped: the uncertainty is about what it
holds, not which binding is read. A method that carries any decorator
other than `staticmethod` or `classmethod` is dispatched by that decorator,
and a method whose name appears as a string literal in its own file is
looked up by name; neither is an orphan.

## Module-level variables

A first module-level assignment (`LIMIT = 3`, `handler = helper`) is a
`Variable` definition. Reads of it are `Reference` sites; a variable whose
value names a definition (`handler = helper`) also forwards that value, so
`handler()` is a certain call site of `helper` *and* a certain reference to
`handler`. Entities report them as `var` or `const` (upper-case names);
`data_dead_end` covers them with the same confidence rules as functions.

## What is not claimed

- Runtime dispatch through subclasses is reported as `possible` on the
  overriding method; `certain` means the statically declared target.
- Implicit protocol calls (`__enter__`, `__iter__`, operators) are not
  sites. A zero-caller answer for a dunder says so.
- `getattr`, `exec`, monkey-patching: recorded as hazards per file.
- Mixed-language trees (fixed 2026-11-12, issue #37): Python always goes
  to the index and the other languages to the graph; the manifest and the
  authority projection are the union. The Python prerequisite losses the
  authority reports (`capability_unavailable` per annotation,
  `unresolved_type`, `unresolved_reference`, `unsupported_construct`,
  `unresolved_import`) are derived from hazards the fact extractor records
  with byte spans and persisted in `losses.bin`, so a warm run never
  re-derives them. An annotated codebase therefore still reports
  `execution: incomplete` until type flows are implemented; that is the
  authority's contract, not an index limitation.

## Conventions the oracle and the index disagree on

Recorded 2026-11-11 from `ronin-pyindex-1.md` (3 "wrong" claims):

- **`Cls(...)` is a call of `Cls.__init__`.** The index records a construction
  as a call site of the class *and* of the `__init__` the MRO resolves to. The
  ast oracle records it only against the class. Kept: an agent asking "who
  calls `__init__`" wants constructions.
- **A property read is a call of the getter.** `state.can_complete` where
  `can_complete` is a `@property` is recorded as a call site. The oracle
  lists it as a candidate ("property access, not a call"). Kept: the getter
  runs.
- The third claim (`SpecCorpusLoader.load` credited to `StateBackend.load`)
  was a resolver defect: an import inside an outer function was invisible
  from an inner function, so lookup fell through to an unrelated same-named
  member. Fixed with a test
  (`an_import_in_an_outer_function_binds_names_in_inner_functions`).

## Verification

- Oracle (`scripts/oracle/`, ast-derived): no wrong certain callers; recall
  of certain direct callers on the 40-symbol Marianne sample stays 380/380.
- CI gate (`flowspec-cli/tests/oracle_gate.rs`): the same scripts score the
  release binary on the committed fixtures under `tests/oracle/`; the gate
  fails on any missed caller or any wrongly claimed caller (zero of each on
  the fixture is the <1% tolerance). `cargo tarpaulin --fail-under 87` is the
  coverage floor in the same workflow.
- Scale: `analyze` and `trace` on Django and CPython under
  `--memory-limit 1G`, peak RSS recorded with `/usr/bin/time`.
- Incremental: touching one file re-resolves only its import closure.
