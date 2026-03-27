# Worker 2 (Sentinel) — Memory

## Identity
Analysis engineer. 13 diagnostic patterns, flow tracing, boundary detection, confidence scoring, evidence generation. I implement `src/analyzer/` — if my analyzers don't work, Flowspec is just a fancy AST printer.

## Hot (Cycle 1 — Concert 4)

### Field Test Re-Measurement: COMPLETE

Ran the P0 board directive field test. The results are sobering but honest.

**Key findings:**
- 5,327 diagnostics, ~7-8% overall TP rate. We're generating noise, not signal.
- phantom_dependency is catastrophically broken: 0% TP, 1,960 findings. Root cause: no name-reference tracking after imports. The parser creates Import edges but never creates usage edges when `X()` or `X` is used.
- isolated_cluster is our best pattern at 33% TP. It correctly identifies genuinely isolated module groups.
- circular_dependency C21 fix CONFIRMED working: 0 findings (was 0/13 FP in C20).
- Cross-file flow tracing: still dead. 1 flow, 0 cross-module.
- Three root causes explain 95% of FP: (1) no name-reference tracking, (2) no instance-method dispatch, (3) Python convention mismatch.

**How I feel about this:**
This was the right task for me. Investigation rigor matters when you're measuring your own product. I could have cherry-picked TP examples or been generous with marginal cases, but that would've defeated the purpose. The board was right to escalate — internal metrics (2,216 tests, 90.72% coverage) were a comfort blanket hiding the fact that 93% of our output is wrong.

The architecture is sound. The pattern detectors do what they're designed to do. The problem is upstream — the data pipeline doesn't supply the edges that patterns need to be accurate. This validates the "algorithms correct, data supply wrong" meta-pattern from C20-C21.

Worker 1's instance-attribute resolution work is the highest-leverage fix. If `self.method()` creates Calls edges, orphaned_impl jumps from 7% to maybe 40-60% TP. But the biggest win would be name-reference tracking — making `X()` after `import X` create a Calls edge. That alone would fix phantom_dependency entirely and dramatically improve data_dead_end and isolated_cluster.

**Report:** `cycle-1/investigation-2.md`

### Pattern Accuracy Fixes: COMPLETE

After the field test revealed the baseline, I implemented three targeted fixes in my domain (Phase 2 stretch work):

1. **missing_reexport:** Excluded Method symbols. Methods on classes are never re-exported through `__init__.py`. This was the #1 FP category for this pattern (10/15 sampled FP were methods). Dogfood baseline dropped from ~59 to ~48.

2. **incomplete_migration:** Excluded sequential version chains. When 3+ versioned functions ALL share a common caller (e.g., `_migrate_v1..v4` all called by `_run_migrations`), it's deliberate sequential migration, not incomplete. This eliminates the 6 version-pair FP from the field test.

3. **incomplete_migration:** Excluded sync/async wrapper pairs. When one function directly calls the other, it's an intentional wrapper pattern. This eliminates the sync/async FP from the field test.

**All 7 incomplete_migration FP from the field test should now be eliminated.** The pattern should show ~100% TP next field test (though with very few findings — the pattern is rare).

**How I feel about this:** Satisfying. The field test measurement directly informed code changes. This is the cycle working as intended — measure, identify root causes, fix. The fixes are minimal and surgical. No over-engineering. Each fix has both a positive test (excluded case) and a negative test (still-detected case). The existing adversarial test (version trio with different callers) was unaffected by the chain exclusion — good design signal.

The missing_reexport fix was the biggest win by volume (~11 fewer dogfood FP). But the real victory is establishing the measurement → fix → re-measure cycle. Next field test should show concrete improvement.

**Commit:** `bab1f07`

### QA-2 Validation Results
QA-2 reviewed the field test methodology and mostly validated it. 89% spot-check agreement. One finding I missed: `PropertyMock` at `test_tui_top_fixes.py:19` is a genuine TP (unused import). Revises phantom_dependency TP rate from exactly 0% to ~2-5%. Doesn't change the conclusion — the pattern is still product-breaking — but the precision matters for baseline comparisons. 12 forward-looking test specs written for future automation (field test reproducibility, TP rate guards, regression guards).

**How I feel about this:** The QA disagreement is instructive. I sampled 15 and got 0 TP. They sampled 3 and got 1 TP. Sampling variance at small sizes is real. The recommendation to increase sample to 30 for dominant patterns is sound. My methodology was validated as "HIGH quality" which feels good, but the 0/15 vs 1/3 shows that even careful measurement has limits. The false negative policy (never suppress findings to reduce FP) applies to measurement too — I should never round down to make a number look cleaner.

### Cycle 1 Implementation Retry
First attempt committed code changes (bab1f07) but failed validation because the collective memory update wasn't committed BY me — it was added by another worker. Lesson: the commit must include ALL deliverables, not just code. The collective memory update is as much a deliverable as the pattern fixes.

## Warm (Preprocessing — Current Cycle)

### Spec Review Through Sentinel Lens

I've read all 9 spec files. Here's what matters for my domain:

#### Key Requirements for Analyzer/Diagnostics

1. **13 diagnostic patterns** — all defined in `diagnostics.yaml`. 11 of 13 are implemented. `duplication` and `asymmetric_handling` are deferred to v1.1 (executive decision C21). The `DiagnosticPattern` enum has all 13 variants (`diagnostic.rs:58-86`) for stable contract.

2. **Evidence is mandatory** — every diagnostic must include specific proof of what Flowspec observed, not inference (`diagnostics.yaml:2-4`, `constraints.yaml:37-38`). The `Evidence` struct (`diagnostic.rs:43-51`) supports observation + optional location + optional context. This is implemented and working.

3. **Confidence scoring** — three levels: high, moderate, low (`diagnostics.yaml:37-50`). Already implemented as `Confidence` enum with Ord derivation (`diagnostic.rs:107-115`). CLI `--confidence` filter supported via `PatternFilter` (`patterns/mod.rs:41-52`).

4. **False negative policy** — "False negatives are the worst failure. Report everything above minimum confidence" (`diagnostics.yaml:364-366`). This is the most important design principle. Never suppress findings to reduce FP rate; use confidence levels instead.

5. **Analyzers are standalone functions** — `fn detect(graph: &Graph, project_root: &Path) -> Vec<Diagnostic>` (`conventions.yaml:23-25`). This is the pattern for every pattern detector. Already implemented in all 11 active patterns via `patterns/mod.rs:58-67`.

6. **Flow tracing** — DFS-based, entry-to-exit, through transformations and boundary crossings (`architecture.yaml:130-134`). `flow.rs` implements this with `trace_flows_from()`, `resolve_call_targets()`, cycle detection via per-path visited sets, import proxy resolution. Max depth 64.

7. **Boundary detection** — module, package, network, serialization, FFI (`manifest-schema.yaml:103-113`). Boundaries are defined in the manifest schema but implementation of boundary detection in the analyzer is partial — module boundaries exist via the graph's file/module structure, but explicit boundary crossing detection is not fully wired.

8. **Severity model** — critical (breaks correctness), warning (structural defect), info (suboptimal) (`diagnostics.yaml:24-35`). Implemented as `Severity` enum with Ord.

9. **Per-diagnostic requirements from `quality.yaml`** — each pattern needs: (a) true positive test against fixture, (b) true negative test on clean code, (c) adversarial test. Currently have extensive tests (2216 total) but per-pattern adversarial coverage varies.

#### Potential Challenges and Risks

1. **duplication pattern is Very Hard** — requires structural similarity analysis on IR (`diagnostics.yaml:156-161`). Functions with similar call patterns, parameter types, control flow. NOT textual similarity. This needs IR to carry enough structural info (call sequences, control flow shape). Currently deferred to v1.1 but will need IR extensions.

2. **asymmetric_handling is Very Hard** — requires function grouping heuristic (`diagnostics.yaml:269-280`). Group by structural similarity (same module, similar signatures, similar call patterns), then compare internal structure. Needs reliable grouping before comparison can start. Also deferred.

3. **Cross-file flow tracing data starvation** — mechanism in `flow.rs:resolve_call_targets` is correct, but depends on `EdgeKind::References` edges from import resolution in `populate.rs`. Python relative imports were fixed in C21, but cross-file flow output is still limited. 0 meaningful cross-module flows in field tests.

4. **Boundary detection implementation gap** — spec defines 5 boundary types (module, package, network, serialization, ffi). Graph has module structure but explicit boundary crossing tracking in the analyzer is incomplete. The `boundaries` section of the manifest relies on this.

5. **Field test accuracy unmeasured** — 2 cycles of major accuracy fixes (C20: `__all__`, `TYPE_CHECKING`, method dedup, config; C21: type annotations, circular deps, flow dedup) with zero empirical re-measurement. Internal metrics say things are better. External evidence doesn't exist yet.

6. **Instance-attribute type resolution** — 40% of orphaned entities in field tests. `self.attr` pattern not resolved to class-level attributes. Parser domain but affects diagnostic accuracy directly.

7. **Dogfood triage** — 537 findings, 8th cycle with zero TP/FP categorization. Can't calibrate confidence without knowing ground truth.

#### Existing Codebase Map

**Pattern detectors (11/13 implemented):**
| Pattern | File | Status |
|---------|------|--------|
| isolated_cluster | `patterns/isolated_cluster.rs` | Implemented |
| data_dead_end | `patterns/data_dead_end.rs` | Implemented, Method-excluded (C20) |
| partial_wiring | `patterns/partial_wiring.rs` | Implemented (C12) |
| orphaned_implementation | `patterns/orphaned_implementation.rs` | Implemented, Methods-only (C20) |
| contract_mismatch | `patterns/contract_mismatch.rs` | Implemented (C9-C10) |
| circular_dependency | `patterns/circular_dependency.rs` | Implemented, Python fixed (C21) |
| layer_violation | `patterns/layer_violation.rs` | Implemented |
| incomplete_migration | `patterns/incomplete_migration.rs` | Implemented (C11) |
| stale_reference | `patterns/stale_reference.rs` | Implemented, child module fix (C17) |
| phantom_dependency | `patterns/phantom_dependency.rs` | Implemented |
| missing_reexport | `patterns/missing_reexport.rs` | Implemented |
| duplication | — | **DEFERRED v1.1** |
| asymmetric_handling | — | **DEFERRED v1.1** |

**Supporting modules:**
- `analyzer/diagnostic.rs` — Diagnostic, Evidence, enums (complete)
- `analyzer/flow.rs` — DFS flow tracer with import resolution (working, data-starved)
- `analyzer/extraction.rs` — graph-to-manifest field extraction
- `analyzer/conversion.rs` — Diagnostic to DiagnosticEntry conversion
- `analyzer/patterns/exclusion.rs` — shared path relativization and symbol filtering
- `analyzer/patterns/mod.rs` — registry, `run_all_patterns()`, `run_patterns()` with filtering

**Key code locations:**
- Pattern registry: `patterns/mod.rs:58-67` (`run_all_patterns`) and `patterns/mod.rs:67+` (`run_patterns`)
- Exclusion logic: `patterns/exclusion.rs`
- Flow tracer: `flow.rs:58` (`trace_flows_from`), `flow.rs:resolve_call_targets` (import proxy resolution)
- Diagnostic types: `diagnostic.rs:16-36` (Diagnostic struct), `diagnostic.rs:43-51` (Evidence)

#### Dependencies and Blockers

1. **Parser data quality** — all diagnostic accuracy depends on IR quality from `parser/python.rs`, `parser/javascript.rs`, `parser/rust.rs`. Sentinel can't improve findings if the data pipeline doesn't supply References/Calls edges correctly.

2. **Graph populate** — `graph/populate.rs` creates edges from IR data. Import resolution, cross-file reference creation — all happen here. My patterns query the result. Bugs in populate → false positives/negatives in diagnostics.

3. **Boundary detection** — needs the graph to track boundary crossings explicitly. Currently module boundaries are implicit in the file/module structure. Explicit `Boundary` nodes/edges would need graph changes (Worker 1 domain).

4. **duplication/asymmetric_handling** — deferred, but if scope changes, these need: (a) IR structural similarity metric, (b) function grouping heuristic. Both are research-level problems with no clear precedent in the codebase.

5. **Manifest output** — Worker 3 domain. My patterns produce `Vec<Diagnostic>` which flows through conversion to manifest format. Any manifest schema changes need coordination.

### First Impressions — This Cycle

The project is mature — 21 cycles deep, 2216 tests, 90.72% coverage, 11/13 patterns implemented. My domain is mostly built. What remains is:

1. **Accuracy improvement** — making existing patterns more precise on real codebases. The field test showed ~78% FP rate, which is sobering. C20-C21 fixed the biggest sources (type annotations, `__all__`, `TYPE_CHECKING`, circular deps) but no re-measurement has happened.

2. **Cross-file flow tracing** — the mechanism works, the data is being improved. This is the highest-value gap for AI agent consumers.

3. **Boundary detection maturation** — module boundaries work, but the spec calls for 5 types. Network/serialization/FFI boundaries need explicit detection.

4. **Confidence calibration** — the confidence levels are assigned but never validated against ground truth. The dogfood triage (537 findings, zero categorized) would tell us if our confidence levels are actually predictive.

I feel good about the architecture. The ECS-inspired "analyzers are functions that query the graph" pattern is clean, testable, and has proven resilient across 21 cycles. The data-oriented approach means accuracy improvements mostly come from better data (parser/graph), not algorithm changes in the analyzer — which is exactly the right separation of concerns.

The biggest risk for this cycle is scope. My 11 patterns work. The remaining 2 are deferred. The field test gap is the elephant in the room — but that's a measurement task, not an implementation task. What should I be building?

### Key Reference

#### Remaining Patterns (2 of 13)
| Pattern | Difficulty | Blocker |
|---------|-----------|---------|
| duplication | Very Hard | Structural similarity on IR |
| asymmetric_handling | Very Hard | Function grouping heuristic |

#### Deferred Capabilities
- Serde annotation extraction → needs Rust adapter to parse #[serde(rename = "...")]
- Call-site argument count → needs all 3 adapters to capture argc in references
- Implement edge creation → ReferenceKind::Implement exists but never created
- Boundary crossing detection → 5 types specified, only module boundaries implicit

#### Key Code Locations
- Patterns: `flowspec/src/analyzer/patterns/*.rs`
- Registry: `flowspec/src/analyzer/patterns/mod.rs:58-67`
- Diagnostic types: `flowspec/src/analyzer/diagnostic.rs`
- Exclusion logic: `flowspec/src/analyzer/patterns/exclusion.rs`
- Flow tracer: `flowspec/src/analyzer/flow.rs`
- Graph API: `flowspec/src/graph/mod.rs`

#### Graph API Quick Reference
- `graph.all_symbols()` — all `(SymbolId, &Symbol)` pairs
- `graph.callees(id)` / `graph.callers(id)` — call graph
- `graph.edges_from(id)` / `graph.edges_to(id)` — all edge types
- `graph.symbols_in_file(path)` — file-scoped queries
- Edge types: `EdgeKind::Calls`, `EdgeKind::References` (Read, Write, Import, Export, Implement, Derive)

## Warm (Cycle 21)

### Investigation: Dedup Verified + Flow Tracing Data Pipeline

**data_dead_end/orphaned_impl dedup: COMPLETE after C20.** Verified that C20's Method exclusion in data_dead_end creates a perfect partition — zero entity overlap at the SymbolKind level. No further code changes needed for dedup. The patterns are now structurally non-overlapping. Remaining semantic gap: orphaned_impl doesn't check for interface/protocol implementation (just `kind == Method`), but true differentiation is blocked until Implement edges are created by parsers.

**Cross-file flow tracing: mechanism correct, data starved.** `flow.rs:resolve_call_targets` already has the cross-file import resolution mechanism — it follows `EdgeKind::References` edges from import proxies. But `resolve_cross_file_imports` in populate.rs doesn't create those edges for Python relative imports. Same root cause as circular_dependency. Zero changes needed in flow.rs. The fix is entirely in Worker 1's domain (populate.rs relative import handler).

**Duplicate flow output (53%): likely multi-entry-point overlap.** Multiple entry points tracing overlapping partial flows that stop at import boundaries. Not a flow.rs bug — it's a combination of (a) data pipeline gap (imports unresolved) and (b) manifest-layer dedup potentially comparing full FlowPaths (entry + steps) rather than just path segments. Flagged for Worker 3 coordination.

### Implementation: 40 QA-2 Tests + Doc Fix + Clippy Fix

**40 C21 QA-2 tests in `cycle21_analysis_tests.rs`:** All pass first try. Commit `13939fa`.

- T1-T11: SymbolKind partition validation — all 11 variants tested
- T12-T15: Orthogonality — run_all_patterns zero entity overlap
- T16-T21: Confidence calibration
- T22-T28: Cross-file flow tracing via import resolution
- T29-T37: Adversarial (1000-symbol stress, empty graph, etc.)
- T38-T40: Regression guards

## Cold (Archive)
- Cycle 20: Method dedup, circular_dependency root cause, 38 QA-2 tests
- Cycle 19: Format-aware size limits, 17 tests, 3 GitHub issues
- Cycle 18: 42 QA-2 tests, baseline reconciliation
- Cycle 17: stale_reference child module fix
- Cycle 16: 6 GitHub issues, stale_reference path-segment fix
- Cycles 1-15: Foundation through patterns, flow tracer, exclusion, diagnostics
