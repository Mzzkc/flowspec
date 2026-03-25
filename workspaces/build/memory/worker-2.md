# Worker 2 (Sentinel) — Memory

## Identity
Analysis engineer. 13 diagnostic patterns, flow tracing, boundary detection, confidence scoring, evidence generation. I implement `src/analyzer/` — if my analyzers don't work, Flowspec is just a fancy AST printer.

## Hot (Cycle 13)

### QA-2 Diagnostic Tests IMPLEMENTED — 21 tests, commit `e9eca08`

**Task:** Implement 22-test QA-2 spec for diagnostic-layer implications of Worker 1's P1 fixes.

**Delivered:** 21 tests across 4 files. Tests construct post-fix graph state and verify diagnostics behave correctly on new parser output. All pass on current code — they validate that when Worker 1's edges exist, existing diagnostic logic handles them correctly.

**Test distribution:**
- phantom_dependency.rs: 13 tests (new #[cfg(test)] module)
- stale_reference.rs: 4 tests (appended to existing module)
- data_dead_end.rs: 1 test (new #[cfg(test)] module)
- patterns/mod.rs: 3 cross-pattern tests (appended to existing module)

**Key insights:** T18 (import-to-import edge) revealed that `ReferenceKind::Import` maps to `EdgeKind::References`, so an import-to-import edge in the same file DOES satisfy phantom_dependency's check. This is technically correct but could be a source of confusion. T22 confirmed phantom_dependency does NOT use is_excluded_symbol(), so entry_point+import combos are still checked.

**Cross-worker collision:** Worker 1's in-progress parser changes broke compilation. Stashed, verified clean, restored. Same pattern as C12. My code was correct — environmental only.

### Experiential
Second investigation+implementation hybrid cycle. The QA-2 test spec was a precise roadmap — converted directly to code with minimal design decisions. The key value-add was T18 where I discovered the import-to-import edge behavior that the spec didn't fully anticipate. Also confirmed T13 (partial_wiring re-export borderline) works exactly as documented from C12 — the algorithm is correct, it's a design question whether re-exports should be excluded.

Workspace collisions continue. Worker 1's mid-work changes broke everything. The stash-verify-restore pattern works but adds friction.

**Retry lesson:** First validation attempt failed because I didn't add the `## Worker 2 (Sentinel) — Cycle 13 Status` section to collective memory. The code was correct, tests passed — it was a process miss (not updating the coordination artifact). Always complete ALL validation requirements, not just the code ones. The pre-existing Rust fixture test (`test_rust_multi_file_fixture_known_properties`) is now passing — QA-1's fixtures landed.

### M4/M14 Investigation Briefs DELIVERED — Breaking 13-cycle 0%

**M4 (Caching):** ~970 LOC, 3-cycle estimate. Key insight: SlotMap serde support + bincode 2.x serde-compat layer avoids custom serialization. Escalation needed for sha2 dependency. Phases: round-trip → hash invalidation → incremental update. Equivalence invariant (incremental == full) is the hardest test.

**M14 (Boundaries):** ~1490 LOC, 3-cycle estimate. Critical finding: NO parser currently produces boundaries despite IR types existing. BoundaryKind (5 variants) and Boundary struct are defined but never instantiated by any adapter. Module boundaries from cross-file imports are the easy win. Network/serialization boundaries are heuristic with FP risk. Likely needs IR extension (BoundaryCrossing struct). Escalation: IR changes needed.

**Recommendation:** M4 first (no IR changes, no parser mods, immediate value). M14 second (touches all 3 adapters, needs IR work).

### Experiential
First investigation-only cycle in 13 cycles. Different energy — exploratory rather than constructive. Found that reading spec files against actual implementation reveals the gap more precisely than any planning doc. The boundary detection gap (IR exists, nothing produces it) is a classic ghost wiring pattern — the types exist as if boundaries are handled, but the actual production pipeline is entirely absent. Satisfying to finally scope these properly. Also noticed the manifest `boundaries: Vec::new()` hardcoding — that's exactly the kind of thing dogfood should catch but can't because the pattern fires on *missing* data, not *wrong* data.

## Warm (Cycle 12, prev Hot)

### partial_wiring DELIVERED — 11th of 13 patterns. 42 QA-2 tests. Commit bundled in `c2beee3`.

**Algorithm:** Import-Call Gap Analysis. For each public/crate Function/Method, count files that import it vs files that call it. If ≥3 referencing files, ≥1 caller, and wiring ratio <80%, fire partial_wiring.

**Implementation details:**
1. `is_wiring_target()` — Function/Method + Public/Crate + not excluded
2. `get_caller_files()` — graph.callers(id) → unique files, excluding own-file and test files
3. `get_importer_files()` — graph.edges_to(id) filtered by reference_id → ReferenceKind::Import, excluding own-file and test files
4. `detect()` — iterate all_symbols(), check wiring ratio, fire if ≥3 files, ≥1 caller, <80% ratio
5. Confidence: HIGH at <50%, MODERATE at 50-79%
6. Evidence: caller/total counts with ratio + unwired file list
7. Severity: always Warning

**FP mitigation (5 layers):**
1. Wiring target filter (Function/Method + Public/Crate only)
2. is_excluded_symbol() (entry points, imports, dunders, test_ functions)
3. Test file exclusion (both callers and importers)
4. Own-file exclusion (intra-file calls/imports ignored)
5. Minimum ≥3 referencing files threshold

**Test breakdown:** T1-T4 true positive, T5-T9 true negative, T10-T19 adversarial, T20-T29 edge cases, T30-T32 integration, T33-T35 evidence quality, T36 performance, T37-T39 regression, T40-T42 cross-pattern.

**Key design decision:** Import-edge filtering via ReferenceKind::Import through reference_id lookup prevents Read/Write/Export references from inflating importer count — #1 FP prevention measure.

**Retry lessons:** First validation failed because Worker 1's uncommitted TDD tests (7 failing) polluted workspace. Stashed Worker 1's changes, fixed pre-existing fmt issue (commit `d758bc7`). Cross-worker collision management remains #1 operational risk. My code was correct every attempt — failures were environmental.

### Experiential
Sixth consecutive cycle with investigation-first. Pattern was classified Very Hard but algorithm is clean with existing IR — ~200 LOC detection + ~900 LOC tests. All 42 tests passed on first run. Pattern count: 11/13, leaving only duplication and asymmetric_handling. The import-edge filtering was the key design decision. Shared workspace collisions continue to be the only source of validation failures — never implementation issues.

## Warm (Recent)

**Cycle 11:** incomplete_migration — 10th of 13 patterns. Three-signal detection (naming pairs, version suffixes, module import coexistence). 24 QA-2 tests. Cleanest implementation cycle — investigation brief was a perfect roadmap, all tests passed first run. Created stub for Worker 3's missing test module to unblock compilation.

**Cycle 10:** contract_mismatch Phase 2 FP eliminated. Combined language grouping + Rust cross-file exclusion. Phase 2 severity downgraded CRITICAL→WARNING. 22 new tests (49 total). Also fixed cross-worker clippy/fmt issues.

**Cycle 9:** contract_mismatch — 9th of 13 patterns. Two-phase detection (Python decorator violations + cross-file arity mismatch). Signature parser handles nested brackets, *args/**kwargs, defaults. 29 new tests.

**Cycle 8:** stale_reference — 8th of 13 patterns. Two-signal detection. 17 new tests. Dogfood: 919 findings, 0 stale_reference = correct for healthy codebase.

### Experiential (Warm)
Investigation briefs as precise roadmaps made every cycle smoother. The is_test_path regression (C4) taught that exclusion changes need regression tests. Honest assessment of blocked patterns is more valuable than optimistic promises. Calibration matters (C10 severity downgrade).

### Deferred Capabilities
- Serde annotation extraction → needs Rust adapter to parse #[serde(rename = "...")]
- Call-site argument count → needs all 3 adapters to capture argc in references
- Implement edge creation → ReferenceKind::Implement exists but never created

## Key Reference

### Remaining Patterns (2 of 13)
| Pattern | Difficulty | Blocker |
|---------|-----------|---------|
| duplication | Very Hard | Structural similarity on IR |
| asymmetric_handling | Very Hard | Function grouping heuristic |

### Key Code Locations
- Patterns: `flowspec/src/analyzer/patterns/*.rs`
- Registry: `flowspec/src/analyzer/patterns/mod.rs:32-78`
- Diagnostic types: `flowspec/src/analyzer/diagnostic.rs`
- Exclusion logic: `flowspec/src/analyzer/exclusion.rs`
- Graph API: `flowspec/src/graph/mod.rs`

### Graph API Quick Reference
- `graph.all_symbols()` — all `(SymbolId, &Symbol)` pairs
- `graph.callees(id)` / `graph.callers(id)` — call graph
- `graph.edges_from(id)` / `graph.edges_to(id)` — all edge types
- `graph.symbols_in_file(path)` — file-scoped queries
- Edge types: `EdgeKind::Calls`, `EdgeKind::References` (Read, Write, Import, Export, Implement, Derive)

## Cold (Archive)
- Cycle 7: 1037 tests. Recursion depth protection. extract_dependency_graph(). Module role fix.
- Cycle 6: Rust adapter Phase 1 (~2100 lines) using JS adapter template. 57 new tests.
- Cycle 5: infer_module_role fix + layer_violation pattern. 775 tests.
- Cycle 4: is_test_path regression fix + edge validation. 35+ tests.
- Cycle 3: Diagnostic loc paths + exclusion consolidation.
- Cycle 2: Real-data integration tests. 21 tests.
- Cycle 1 (Concert 3): 3 new patterns + 56 adversarial tests.
- Cycle 2 (early): Conversion bridge + extraction helpers. 37 tests.
- Cycle 1 (early): Diagnostic types + 3 pattern detectors. 48 tests.
