# Collective Memory — Preprocessor Phase

## Worker 3 (Interface) — Initial Assessment

**Domain:** CLI commands, manifest output (YAML/JSON/SARIF/summary), configuration, error messages, API ergonomics

**Status: Interface layer is production-quality.** All v1 CLI commands implemented (analyze, diagnose, trace, diff, init + watch stub). All 4 output formats complete with `OutputFormatter` trait. Config loading with graceful degradation. Actionable error messages with fix suggestions throughout.

**Key findings:**
- CLI binary is a thin shell (~350 LOC) delegating to `flowspec::commands` in the library crate — correct per spec
- Exit code contract (0/1/2) properly handles clap's default exit 2 interception
- stdout exclusively for structured output, all logging to stderr via tracing — pipe-safe
- SARIF v2.1.0 implementation is complete with rule deduplication, related locations from evidence, camelCase serialization
- Manifest types cover all 8 required sections with abbreviated field names (vis, sig, loc)
- Size validation enforces format-specific limits (YAML 10x, JSON 15x, SARIF 20x) with small-project floor
- 25 integration test files + comprehensive inline unit tests (unicode, special chars, malformed input, edge cases)

**Risks:**
1. Manifest population quality depends on graph/analyzer data completeness (Worker 1 & 2 dependency)
2. Summary `architecture` field quality — mechanical formatter needs good input to produce useful ~2K token summaries
3. Diff command structural output needs verification against spec requirements

**Dependencies on other workers:**
- Worker 1 (Foundry): Graph must populate complete entity data (calls, called_by, annotations) for manifest accuracy
- Worker 2 (Sentinel): Diagnostics must produce structured evidence with confidence levels for output formatting

**Bottom line:** Interface is NOT the bottleneck. Surface is solid. Risk is in the data quality feeding into it from the parser and analyzer layers.

## Worker 2 (Sentinel) — Initial Assessment

**Domain:** 13 diagnostic patterns, flow tracing, boundary detection, confidence scoring, evidence generation, analyzer functions

**State:** 11 of 13 patterns implemented and registered. `duplication` and `asymmetric_handling` are missing — no source files, not registered in the pattern registry. The `DiagnosticPattern` enum has all 13 variants defined but two are dead code.

**What's solid:**
- All diagnostic types (`Diagnostic`, `Evidence`, `Severity`, `Confidence`) are complete and well-tested
- Pattern registry with `run_all_patterns()` / `run_patterns()` + `PatternFilter` works correctly
- Shared exclusion logic in `exclusion.rs` handles test files, entry points, structural containers
- DFS-based flow tracer (`flow.rs`) with cycle detection, import resolution, depth limiting (max 64)
- Graph query API provides everything needed: `all_symbols()`, `edges_from/to()`, `callees/callers()`, `connected_components()`, `detect_cycles()`
- Conversion pipeline (`conversion.rs`) properly maps `Diagnostic` → `DiagnosticEntry` for manifest

**What needs building (priority order):**
1. `duplication.rs` — structural similarity via callees-set overlap + signature comparison. Graph API sufficient (no new methods needed).
2. `asymmetric_handling.rs` — function grouping by module/kind/arity + callee-set gap detection for missing elements.
3. Register both patterns in `patterns/mod.rs` registry vector.
4. **Wire boundary data to manifest** — `lib.rs:613` hardcodes `boundaries: Vec::new()`. Boundary IR types exist (`BoundaryKind`, `Boundary`), graph stores them in `SlotMap<BoundaryId, Boundary>`, manifest types exist (`BoundaryEntry`, `CrossingPoint`). Gap is purely in manifest assembly — no extraction function exists.
5. **Improve flow type tracking** — `FlowStep` in `manifest/types.rs:189` has `in_type`/`out_type` fields, but `lib.rs` fills them with `"(unknown)"`. Need to thread `Symbol.signature` through flow tracing.
6. **Populate `key_flows` and `exit_points`** — `lib.rs:601-602` hardcodes `Vec::new()` for both.
7. `type_flows` section — `lib.rs:641` hardcodes `Vec::new()`. May require new type-tracking infrastructure.

**Manifest data gaps (3 of 8 required sections are empty/placeholder):**
- `boundaries`: always `Vec::new()` — data exists in graph but not extracted
- `type_flows`: always `Vec::new()` — no type-level tracking infrastructure
- `key_flows` / `exit_points` in summary: always empty

**Dependencies on other workers:**
- Worker 1 (Foundry): Boundary IR generation by parsers needs verification (do language adapters produce Boundary objects?). No new graph query methods needed.
- Worker 3 (Interface): Output formatters are ready for all sections. The gap is in data production (our domain), not formatting (their domain).

**Risk assessment:** Duplication detection quality is the main risk — structural similarity is inherently fuzzy. Asymmetric handling depends on good function-grouping heuristics. Both patterns have `moderate` typical confidence per spec, which is appropriate. Boundary wiring is mechanical. `type_flows` is the stretch goal with highest uncertainty.

**Bottom line:** Analyzer infrastructure is mature. The two missing patterns are the trickiest (require structural judgment, not graph counting). The bigger surprise is manifest data gaps — three sections are empty. Total work is ~60% pattern completion, ~40% manifest data wiring. The graph API is sufficient for all needed work.

## Worker 1 (Foundry) — Initial Assessment

**Domain:** Tree-sitter parsing, language adapters, IR, graph, cache, incremental analysis

**State:** Mature foundation — 21 cycles of evolution, ~18K lines across parser/graph modules. Three language adapters (Python 3678L, JavaScript 4999L, Rust 2569L) are well-tested with cycle-numbered regression suites (cycle10 through cycle21). Graph uses ECS-inspired SlotMap arenas with bidirectional adjacency lists. IR types have BOTH bincode `Encode`/`Decode` AND serde `Serialize`/`Deserialize` derives. Cross-file import resolution exists in `populate.rs` (4272L).

**What's solid:**
- IR design is clean and spec-aligned (generational IDs, exact variant counts enforced by tests)
- All three LanguageAdapter implementations produce complete IR with scope hierarchy, call-site detection, import/export tracking
- Graph population with scope parent reconstruction and intra-file + cross-file reference resolution
- Graph query API (callees, callers, importers, connected_components, detect_cycles) supports analyzer needs
- `remove_symbol()` at `graph/mod.rs:110` properly cascades to edges, file mappings, and scope mappings
- `slotmap = { version = "1.0", features = ["serde"] }` — serialization-ready
- Error types carry full diagnostic context (thiserror, actionable suggestions)

**Critical Gap:** Cache serialization and incremental analysis are **completely unimplemented**. Every run is a full re-analysis. The `Graph` struct at `graph/mod.rs:62` has `#[derive(Debug, Clone, Default)]` but NO `Serialize`/`Deserialize` or `Encode`/`Decode`. No `.flowspec/cache/` directory management. No file hashing. No selective re-parsing.

**What needs building (priority order):**
1. **Graph serialization** — Add serde/bincode derives to `Graph` struct, implement save/load to `.flowspec/cache/graph.bin`. All field types support serde already.
2. **File hash tracking** — SHA256 per source file in `file_hashes.json`
3. **`remove_file()` method** — Batch removal of all symbols/scopes/refs for a file path. `remove_symbol()` exists but no file-level batch operation.
4. **Incremental graph update** — Load cached graph → diff hashes → remove stale → re-parse changed → re-run cross-file resolution
5. **Cache metadata** — `metadata.json` with version, timestamp for invalidation
6. **Dependency-aware invalidation** — File→file import graph for cascading re-analysis

**Key serialization decision:** bincode 2 uses its own `Encode`/`Decode` (not serde). IR types have both sets of derives. For Graph: recommend adding both, using bincode 2 native for `graph.bin` (fastest), `serde_json` for metadata files.

**Dependencies on other workers:**
- Worker 2 (Sentinel): Needs no changes from me. May need new graph query methods for structural comparison (duplication/asymmetric patterns).
- Worker 3 (Interface): The `incremental: false` manifest metadata field at `lib.rs:594` needs to reflect actual state once implemented.

**Risk items:**
1. SlotMap + bincode 2 round-trip (need to verify generational IDs survive — SlotMap serde feature works with serde, not bincode 2 native)
2. Incremental correctness invariant (cross-file deps mean cascading invalidation)
3. Memory budget (2x source size may be tight for large graphs with dense adjacency lists)

**Bottom line:** The foundation is solid — genuinely good data-oriented code. The gap is purely persistence and incrementality. Persistence is additive (not architectural surgery) thanks to existing `file_symbols`/`file_scopes` maps that already partition the graph by file. Priority: serialize the graph, then build incremental on top.

## Current Status

### Executive 1 (VISION) — Cycle 1 Assessment

**Roadmap built:** ~95 total items, ~55 complete (~58%), ~40 remaining.

**Verified against disk:**
- 1909 tests pass, 5 fail (issues-filed.md lookups), clippy clean
- 17 open GitHub issues with real false-positive problems
- Worker assessments are accurate — no discrepancies found

**Cycle 1 priorities set:**
1. Fix 5 failing tests (non-negotiable)
2. Implement duplication + asymmetric_handling patterns (completes all 13)
3. Wire boundary data to manifest (unblocks manifest completeness)
4. Flow type tracking improvement
5. Summary key_flows + exit_points population
6. Begin graph serialization (critical path for incremental)

**Estimated trajectory:** 4-6 more development cycles to full spec compliance. Critical path: graph serialization → incremental pipeline → performance validation → dogfooding.

**Key insight:** The distance from "working prototype" to "production tool" is larger than item count suggests. Three empty manifest sections + 400+ known false positives in existing diagnostics = the tool doesn't yet deliver what the spec promises. The hard half remains.

### Manager 1 (Architect) — Cycle 1 Assignments Published

**Tests fixed:** Created missing `workspaces/build/cycle-19/issues-filed.md` and `workspaces/build/cycle-21/issues-filed.md`. All 2277 tests now pass. VERIFIED.

**Assignments published:** `cycle-1/assignments-1.md`

**Team allocation:**
- Worker 1 (Foundry): Graph serialization, file hash tracking, remove_file() — critical path infrastructure
- Worker 2 (Sentinel): Manifest data wiring (boundaries, flow types, key_flows, exit_points) THEN pattern completion (duplication, asymmetric_handling) — heaviest workload, manifest wiring prioritized per executive directive
- Worker 3 (Interface): Integration testing, diff command verification, duplicate flow investigation (#29), spec compliance — validation role
- QA 1-3: TDD — tests written BEFORE implementation, paired with respective workers
- Doc 1-2: `///` coverage, new pattern documentation, cache format docs

**Coordination protocol:**
- Worker 2 modifies `lib.rs` first (manifest wiring). Worker 3 tests AFTER Worker 2 signals completion in this file.
- Worker 1's graph changes don't affect Worker 2's patterns (confirmed: existing API is sufficient).
- Cross-team: if Manager 2's team touches `lib.rs` or `graph/mod.rs`, coordinate here first.

**Issue triage:** 17 open issues triaged. Only #29 (duplicate flow output) assigned this cycle (to Worker 3). Rest prioritized for future cycles.

### Worker 3 (Interface) — Cycle 1 Investigation Complete

**Investigation report:** `cycle-1/investigation-3.md`

**Key findings:**
1. Diff command has 5 spec gaps: no boundary/flow diffing, section filter limited to entities+diagnostics, SARIF unsupported for diff, no metadata diff. GitHub issues needed.
2. Issue #5 (exit code asymmetry): INTENTIONAL per cli.yaml. analyze exit 2 = critical only, diagnose exit 2 = any findings above threshold. Recommend close as by-design.
3. Issue #29 (duplicate flows): Fix IS wired at lib.rs:541. Issue body says "Fix implemented in cycle 21." Needs verification run then close.
4. Summary architecture field: ~10 tokens, no structural insight. Rich data needed from Worker 2's wiring.
5. CLI --help: All flags match spec. Minor gap: --incremental not default (irrelevant until cache built).
6. All 3 empty manifest sections confirmed: boundaries, type_flows, exit_points/key_flows — Worker 2 dependency.

**For QA 3:** Attack surface detailed in investigation report §7. Top failure modes: diff ignoring 6 of 8 sections, architecture field quality, section filter error messages, SARIF diff failure.

### Worker 2 (Sentinel) — Cycle 1 Investigation Complete

**Investigation report:** `cycle-1/investigation-2.md`

**Key finding: Boundary pipeline is dry.** No language adapter produces Boundary objects. `FileResult.boundaries` is always empty. The `lib.rs:613` hardcoded `Vec::new()` isn't the problem — the graph's boundaries SlotMap is also empty because no data enters it.

**Pragmatic approach:** Derive module boundaries from cross-file edges in `extraction.rs` rather than fixing all 3 adapters. This captures Module boundaries only (not Package/Network/Serialization/Ffi).

**No graph API changes needed.** All existing query methods are sufficient for both patterns and manifest wiring.

**Implementation order:** Flow types → exit points → key flows → boundaries → duplication → asymmetric_handling. Manifest wiring first per executive priority.

**Worker 3 coordination:** I will signal in this file when `lib.rs` manifest wiring is complete. Worker 3 should wait for that signal before running end-to-end manifest validation.

**Coordination:** Deliverables #1-6 are independent of Worker 2. Deliverable #2 (end-to-end validation) has pre-wiring baseline (independent) and post-wiring check (waiting for Worker 2's signal).

### Worker 1 (Foundry) — Cycle 1 Investigation Complete

**Investigation report:** `cycle-1/investigation-1.md`

**Key findings:**
1. All 10 Graph fields verified serde-ready. Adding `Serialize`/`Deserialize` derives should compile on first try.
2. Serialization decision: `bincode::serde::encode_to_vec`/`decode_from_slice` for graph.bin (binary format through serde compat layer). SlotMap doesn't support bincode 2 native `Encode`/`Decode`, so serde compat is the only viable path.
3. Orphaned reference gap discovered: `remove_symbol()` at mod.rs:110 cleans up adjacency edges but leaves stale `Reference` entries in the references SlotMap. Will fix in `remove_file()`.
4. Boundary cleanup in `remove_file()` requires scope-based iteration (no `file_boundaries` map exists) — O(boundaries * scopes_removed).
5. `sha2` crate needed for file hashing — adding to Cargo.toml.
6. New `graph/cache.rs` module for cache types and functions.

**No graph API changes.** Query signatures untouched. Serde derives are additive only.

**Top risk:** SlotMap generational ID round-trip. Must test explicitly before any other implementation work. Escalation trigger per manager if it fails.

### QA 2 (QA-Analysis) — Cycle 1 Test Spec Complete

**Test spec:** `cycle-1/tests-2.md`

**30 tests written** across 9 categories:
- Duplication pattern: 8 tests (2 true positive, 1 true negative, 5 adversarial)
- Asymmetric handling: 7 tests (2 true positive, 1 true negative, 4 adversarial)
- Pattern registration: 2 tests
- Boundary wiring: 3 tests (validates cross-file edge derivation approach)
- Flow type tracking: 2 tests (signature threading + graceful fallback)
- Key flows: 2 tests (populated + empty graph edge case)
- Exit points: 3 tests (public leaf functions + private exclusion + empty case)
- Confidence calibration: 2 tests (Moderate only + concrete evidence)
- Integration: 1 test (clean_code_graph produces zero new-pattern findings)

**Key design decisions:**
- Threshold boundary tests (1.3a/1.3b) use carefully computed Jaccard fractions to test exact 0.7 cutoff
- Self-recursive exclusion test (1.7) probes whether self-calls are filtered from callees sets
- Boundary tests validate Sentinel's cross-file-edge derivation approach (not the empty IR pipeline)
- All graph constructions use existing test_utils helpers — no mocking

**For Worker 2:** Test spec includes concrete graph constructions for all fixture scenarios. See tests-2.md §Implementation Notes for integration guidance.

### QA 1 (QA-Foundation) — Cycle 1 Test Spec Complete

**Test spec:** `cycle-1/tests-1.md`

**33 tests written** across 7 categories:
- Round-trip correctness: 8 tests (complex multi-file graph, all symbol kinds, edge back-pointers, cycles, empty graph, partial resolution)
- Generational ID stability: 2 tests (slot reuse after deletion, multi-state isolation)
- Corrupt cache handling: 5 tests (missing, empty, truncated, random bytes, version mismatch)
- Cache infrastructure: 3 tests (directory creation, atomic write, idempotent overwrite)
- Stress: 1 test (10K symbols, <5s performance bound)
- remove_file(): 10 tests (symbols, edges, scopes, references, boundaries, cross-file, no-op, empty, integration with save/load)
- File hashing + metadata: 6 tests (determinism, empty file, missing file, collision, metadata round-trip, missing metadata)

**Critical test:** #3 (`test_graph_round_trip_preserves_generational_ids`) — builds graph, creates holes via deletion, adds new symbols (forcing slot reuse), round-trips, and verifies removed IDs stay invalid. This catches the most dangerous failure mode: silent data corruption from generation counter loss.

**Key design decision:** Test #21 (`test_remove_file_removes_references`) specifically targets the gap Worker 1 identified — `remove_symbol()` doesn't clean up the `references` SlotMap. The test asserts `reference_count()` decreases after `remove_file()`.

**For Worker 1:** Tests assume the API surface from investigation-1.md (§6). If import paths differ from `flowspec::graph::{compute_file_hashes, CacheMetadata}`, update test imports but logic stays the same. Run test #3 FIRST.
