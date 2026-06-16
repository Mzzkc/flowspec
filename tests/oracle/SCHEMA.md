# Oracle Artifact Schemas

> The authoritative entry shapes. `scripts/validate-oracle-artifacts.py` parses every
> artifact against these shapes — it never greps. Field names are stable keys: changing
> them is a contract break that fails the validator.
>
> Classification IDs and severity/confidence strings match `flowspec diagnose -f json`
> output exactly: `pattern` is the snake_case `DiagnosticPattern::name()` value,
> `severity` ∈ {`info`,`warning`,`critical`}, `confidence` ∈ {`low`,`moderate`,`high`}.

---

## 1. Classification file — `tests/oracle/diagnostics/<target>.yaml`

One file per dogfood target (`self.yaml`, `marianne.yaml`). A target is a repository
diagnosed by `flowspec diagnose <target_root> -f json -q`.

```yaml
# ── file header (provenance for this classification set) ──────────────
target: self                       # "self" | "marianne" | <stable label>
target_root: <flowspec-repo>   # absolute repo root being diagnosed
raw_source: live                   # "live" = invoke binary; or path to planted raw JSON
binary_provenance:                 # copied from BASELINE-PROVENANCE.yaml at review time
  path: target/release/flowspec
  sha256: <hex>
  version: <flowspec --version output>
  diagnosed_at: <ISO-8601>
reviewed_by: canyon                # owner of this review pass
review_expires: 2026-09-01         # this whole set must be re-reviewed by this date

# ── classified findings ───────────────────────────────────────────────
entries:
  - pattern: orphaned_impl         # REQUIRED. canonical snake_case name() (§1 of README)
    path: flowspec/src/graph/mod.rs # REQUIRED. relative to target_root; must exist on disk
    line: 194                      # REQUIRED. int >= 1; must be within the file's line count
    symbol: populate_graph         # REQUIRED. non-empty entity/symbol name
    classification: REAL           # REQUIRED. REAL | KNOWN_FP | DEFERRED_BOUNDARY (only these)
    reason: >                      # REQUIRED. non-empty; for DEFERRED_BOUNDARY cite a §7 boundary
      Genuinely orphaned; no static caller and not a declared public entry point.
    owner: canyon                  # REQUIRED. non-empty; who is accountable for this entry
    expires: 2026-09-01            # REQUIRED. ISO date or "never"; entry must be re-reviewed by then
    confidence_at_review: high     # OPTIONAL. confidence emitted at review time (for R004)
    # ── anti-fabrication escape hatch (rare) ──────────────────────────
    verified: false                # OPTIONAL. default true. skip path:line check ONLY if false
    verify_skip_reason: line moved # REQUIRED-if verified:false. non-empty reason for the skip
```

### Required fields (per entry)
| Field | Type | Constraint |
|-------|------|-----------|
| `pattern` | string | one of the 13 canonical `name()` IDs |
| `path` | string | non-empty; resolves under `target_root`; file must exist |
| `line` | int | ≥ 1 and ≤ line count of `path` (anti-fabrication) |
| `symbol` | string | non-empty |
| `classification` | enum | `REAL` \| `KNOWN_FP` \| `DEFERRED_BOUNDARY` |
| `reason` | string | non-empty; `DEFERRED_BOUNDARY` must cite a §7 boundary |
| `owner` | string | non-empty |
| `expires` | string | ISO-8601 date or `never` |

### Anti-fabrication
For every entry with `verified` ≠ `false`, the validator resolves
`<target_root>/<path>` and asserts the file exists and `1 ≤ line ≤ line_count`. An entry
with `verified: false` **must** also carry a non-empty `verify_skip_reason`; otherwise it
fails. This is the check that prevents fabricated classifications (citing code that does
not exist or lines that are out of range).

### Count equivalence
The set of `(pattern, path, line)` tuples over all entries must equal the set emitted by
`flowspec diagnose <target_root>` (or the planted `raw_source` in self-test). No orphans
(emitted, unclassified) and no extras (classified, not emitted). See README §5 R001/R008.

---

## 2. Trace expectation — `tests/oracle/traces/<name>.yaml`

```yaml
name: rust-cross-file-backward-helper
fixture: tests/fixtures/rust/cross_file   # project dir passed to `flowspec trace`
symbol: helper                           # the -s argument
direction: backward                      # "forward" | "backward"
command: >                               # exact command provenance (copied to BASELINE-PROVENANCE)
  flowspec trace tests/fixtures/rust/cross_file -s helper --direction backward -f json -q
expected_hops:                           # REQUIRED, non-empty, ordered from the start symbol
  - entity: utils::helper                # symbol/location at this hop
    edge_kind: defined_in                # the graph edge traversed to reach it
    file: utils.rs                       # OPTIONAL evidence file
    line: 1                              # OPTIONAL evidence line
  - entity: handler::handle
    edge_kind: called_by
    file: handler.rs
    line: 3
  - entity: entry_point
    edge_kind: called_by
    file: lib.rs
    line: 2
```

The comparator's **R005** rule fails if the actual trace output does not contain every
`expected_hops` entry (by `entity` + `edge_kind`) in order. The validator enforces that
every trace file has a non-empty `expected_hops` with each hop carrying `entity` and
`edge_kind`.

### Edge kinds (v1 trace contract)
The trace product is built from graph edges flowspec can **prove**. Valid `edge_kind`
values are the edge types actually present in the graph (narrowed if a claimed kind is
unbacked — plan item #6 / trace contract gate):

- `called_by` / `calls` — call edges (forward = calls, backward = called_by)
- `referenced_by` / `references` — reference/read edges
- `imported_by` / `imports` — import/export edges
- `assigned` / `read` / `written` — assignment/read/write edges
- `returned_from` / `returns` — return-value edges
- `parameter_of` / `receives` — parameter edges
- `defined_in` — definition location
- `reexported_from` — re-export edges

If the trace CLI cannot prove an edge kind for v1, the CLI/help/spec are narrowed to the
subset it *can* prove (trace contract gate, README §7). No public text may promise
"everything touching the symbol" unless the oracle proves the full edge set.

---

## 3. Provenance — `tests/oracle/BASELINE-PROVENANCE.yaml`

Template in Wave 1A (values `TBD-FILL-IN-1B`); Wave 1B fills it at review time and the
validator checks structure + that filled values are no longer `TBD-FILL-IN-1B` when a
real baseline exists.

```yaml
git:
  flowspec_sha: <sha>                # git -C <flowspec> rev-parse HEAD
  flowspec_status_clean: true        # `git status --short` is empty at review time
  flowspec_status_summary: ""        # the raw `git status --short` output (empty if clean)
binary:
  path: target/release/flowspec      # path used for the dogfood run
  sha256: <hex>                      # sha256sum of that binary
  mtime: <ISO-8601>                  # file mtime
version: flowspec <x.y.z>            # exact `flowspec --version` output
targets:
  self:
    repo_path: <flowspec-repo>
    repo_sha: <sha>
    diagnose_command: >
      target/release/flowspec diagnose <flowspec-repo> -f json -q
    diagnose_exit_code: 2            # 0 = clean, 2 = findings present (both OK)
    raw_findings: 601                # total emitted findings
    by_pattern:                      # count per canonical pattern
      orphaned_impl: 29
      data_dead_end: 270
      # ...
  marianne:
    repo_path: <marianne-repo>
    repo_sha: <sha>
    diagnose_command: >
      target/release/flowspec diagnose <marianne-repo> -f json -q
    diagnose_exit_code: 2
    raw_findings: <n>
    by_pattern: { ... }
```

The provenance file pins **exactly which binary, on which commit, produced the raw
findings the classifications review**. A classification set reviewed against binary X is
not valid for binary Y without re-review — the comparator checks provenance drift as part
of R008.

---

## 4. Fixture coverage — `tests/oracle/FIXTURE-COVERAGE.yaml`

```yaml
patterns:
  orphaned_impl:
    status: implemented              # implemented | deferred | off-by-default
    positive_fixture: tests/oracle/fixtures/python/orphaned_impl/positive
    adversarial_fixture: tests/oracle/fixtures/python/orphaned_impl/adversarial
    notes: >-
      Wave 1A declares status + planned paths; Wave 1B plants + proves fixtures.
  duplication:
    status: deferred                 # deferred — needs AST-subtree hashing (README §7)
    positive_fixture: none
    adversarial_fixture: none
    notes: Deferred for v1; not shipped at warning-default.
```

`status` meanings:
- **implemented** — detector is active; must have passing positive + adversarial fixtures at release.
- **deferred** — out of v1 scope (`duplication`, `asymmetric_handling`); emits nothing by default.
- **off-by-default** — implemented but experimental; must be opt-in, labelled "experimental", and still fixture-covered.

Wave 1A allows `positive_fixture: none` / `adversarial_fixture: none` for `implemented`
patterns (fixtures are planted in 1B). The validator requires all 13 patterns present
with a valid `status`; the stricter "fixture paths must exist + pass" gate is enforced
once 1B populates them.
