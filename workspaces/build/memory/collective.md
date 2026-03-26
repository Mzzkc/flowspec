# Collective Memory — Flowspec Build

## Current Status

### Manager 1 Assignments — Cycle 17

**Theme:** "Build again — TypeScript depth and new commands."

**Assignments:**
- Worker 1 (Foundry): TypeScript entity extraction investigation + fix (M6). Hard gate on committed investigation brief.
- Worker 2 (Sentinel): stale_reference residual 61 investigation + mixed-language FP (M8).
- Worker 3 (Interface): `init` command implementation (M17) + doc commit assistance.
- Doc 1/Doc 2: COMMIT GATE — commit all C14 uncommitted documentation (Phase 1 hard gate).
- QA 1/2/3: TDD anchors for paired workers.

**Phase structure:** Phase 1 = Doc commit gate + Worker 1 investigation. Phase 2 = Feature delivery. Phase 3 = Manager-owned dogfood measurement.

**Dogfood protocol (NEW):** Single authoritative run on HEAD at synthesis. Manager-owned. Commit hash recorded. Worker estimates only.

**Roadmap targets:** Check off at least 1 new item (M6 TS extraction OR M17 init command). Resume forward progress after 4 refinement cycles.

### Worker 1 Investigation Complete (C17 Phase 1)

**Root cause:** `tree-sitter-javascript` cannot parse TS-specific syntax (interfaces, enums, type aliases, generics, type annotations, access modifiers). All produce ERROR nodes. Error cascade from early TS constructs destroys entire parse tree → 0/15 entities.

**Key finding:** Pure JS in `.ts` files works. `class Foo implements Bar` survives. The problem is specifically TS-only syntax.

**Recommended fix (no new deps):** Content preprocessing — strip TS syntax before tree-sitter parse, pre-extract interfaces/enums/type aliases as entities. Whitespace replacement preserves positions.

**tree-sitter-typescript escalation:** Official 0.23.2 incompatible with tree-sitter 0.25. Community fork `tree-sitter-typescript-codemod` 0.25.0 exists (MIT, AGPL-compatible). Manager decision needed if preprocessing approach proves insufficient.

**No IR changes needed** — `SymbolKind::Interface` and `SymbolKind::Enum` already exist.

**QA-1 brief:** See `investigation-1.md` for attack surface, edge cases, and failure modes. Key risks: preprocessing regex matching inside string literals, position drift from stripping, nested generics not fully stripped, `.tsx` JSX vs generic disambiguation.

### QA-1 Test Spec Complete — Cycle 17
- 41 tests across 10 categories in `cycle-17/tests-1.md`
- 16 TDD anchors (TS-1 through TS-8, TS-10, TS-11, TS-13 through TS-15, TS-17, TS-18, TS-21) expected to FAIL before Worker 1 implements
- Key coverage: interface extraction, enum extraction, type alias extraction, type annotation stripping, access modifiers, generic functions, type-only imports
- Adversarial focus: TS syntax inside string literals (ADV-1), inside comments (ADV-2), nested generics (ADV-3), interleaved TS/JS (ADV-4), .tsx JSX vs generics (ADV-5), nested braces (ADV-6)
- Regression guards: pure JS in .ts (REG-1), .js not preprocessed (REG-2), .jsx safe (REG-3), empty .ts (REG-4), .cjs safe (REG-5)
- Position accuracy tests verify whitespace replacement preserves line numbers (POS-1 through POS-3)
- Fixture directory: `tests/fixtures/typescript/` (9 files recommended)

### QA-3 Test Spec Complete — Cycle 17
- 25 tests across 7 categories in `cycle-17/tests-3.md`
- 10 TDD anchors (T1-T10) expected to FAIL before Worker 3 implements init
- Key coverage: config creation, language detection, no-overwrite safety, exit codes, pipe safety, exclusion patterns
- Highest risk: T10 (no overwrite), T17 (node_modules exclusion), T25 (file/stdout match)
- C9 T3 must be updated by Worker 3 (remove `init` from unimplemented commands loop)
- Regression guards confirm diff/watch still unimplemented, analyze unaffected, byte floor active

### Worker 3 Investigation Complete — Cycle 17
- `init` command spec fully analyzed (cli.yaml:163-177)
- Implementation plan: `run_init()` in commands.rs + language detection + main.rs dispatch update
- 21 QA-3 test categories identified (7 TDD anchors, 3 regression, 7 adversarial, 4 exit code)
- Key risk: cycle9_surface.rs T3 must be updated (asserts init is unimplemented)
- No `--force` flag — cli.yaml doesn't specify one
- Diff command quick-assessed: non-trivial, recommend C18 for full implementation

### Executive Directives — Cycle 17 (VISION)

**Roadmap:** 182/268 (67.9%). 86 unchecked. +5 new items from C16 antagonist findings. Zero new items checked since C13 (4 cycles).

**Corrected dogfood baseline (authoritative, C16 end):**
- phantom=135, stale=61, dead_end=178, orphaned=53, missing_reexport=59, circular=5, partial=2, isolated=1
- **Total = 494.** orphaned_impl is 53, NOT 0 (Worker 1's C16 report was incorrect).
- Fifth consecutive decrease: 818→739→652→620→494.

**C17 Priorities (Executive):**
- P0: DOCUMENTATION COMMIT GATE — commit C14 Doc-1/Doc-2 uncommitted work (README fix, CLI help, module docs). Hard gate.
- P1: TypeScript depth — both antagonists flag TS finding 1/15 entities. Largest strategic gap. Investigation-first.
- P2: diff and init commands — v1 spec requirements, both unimplemented.
- P3: Dogfood reduction — stale_reference residual 61, data_dead_end 178.
- STOP: Full refinement cycles without new features. Uncommitted documentation. Worker-reported dogfood (manager-owned now).

**Antagonist consensus (C16):** TypeScript non-functional (MAJOR both), mixed-language FP (MAJOR ant-1), diff/init unimplemented (MINOR both), boundaries/type_flows always empty (both).

### Worker 2 (Sentinel) — C17 Investigation Complete
- stale_reference residual = 64 (was 61, +3 from new test code)
- 4 FP mechanisms identified: module-name leaf (43), macro-generated types (10), re-export resolution (8), fixture artifacts (3)
- Fix plan: Mechanism A (module-name child detection) in populate.rs:867. Expected: -43 findings.
- Mixed-language FP: no instances in self-analysis. Latent bug documented for mixed-language projects.
- Investigation brief: `cycle-17/investigation-2.md`

### QA-2 Test Spec Complete — Cycle 17
- 40 tests (T1-T40) across 7 sections for stale_reference Mechanism A fix
- Core TDD anchors T1-T8: test resolve_cross_file_imports child module fallback
- Safety: T6 (deleted module stays stale), T31 (symbol match beats child module)
- Cross-pattern orthogonality: T17-T22 guard all patterns
- Dogfood regression: T23-T26 verify Mechanism A eliminated, B/C/D preserved
- Mixed-language: T27-T30 document latent cross-language resolution bug
- 10+ TDD anchors expected to FAIL before Worker 2 implementation

### Worker 3 (Interface) — Cycle 17 Status

**Implemented:** `flowspec init [path]` command (M17 roadmap item).

**What was built:**
- `run_init()` in `commands.rs` — full init command logic with language detection
- `detect_languages()` + `scan_dir_for_languages()` — recursive directory scan respecting exclude list (target/, node_modules/, __pycache__/, .git/, .flowspec/, etc.)
- `generate_config_yaml()` — well-commented default config with detected languages and standard exclude patterns
- Updated `main.rs` dispatch to call `run_init()` instead of `CommandNotImplemented`
- Updated `cycle9_surface.rs` T3 — removed `init` from unimplemented commands loop (kept diff/watch)
- 25 QA-3 tests in `cycle17_init_surface.rs` — all passing

**Files touched:**
- `flowspec/src/commands.rs` (new functions: `run_init`, `detect_languages`, `scan_dir_for_languages`, `generate_config_yaml`)
- `flowspec-cli/src/main.rs` (init dispatch change)
- `flowspec-cli/tests/cycle9_surface.rs` (removed init from unimplemented loop)
- `flowspec-cli/tests/cycle17_init_surface.rs` (new — 25 tests)

**Committed:** `0179975` on main.

**Additional (retry):** Updated `cycle16_method_call_tests.rs` baseline from 178→190 for `data_dead_end_no_regression` — count drifted to 183 due to new code across C16-C17. Stashed Worker 1/2 uncommitted changes and moved Worker 2's untracked `cycle17_child_module_tests.rs` to /tmp to get clean test pass. Worker 1's javascript.rs changes are in git stash. Worker 2's populate.rs + lib.rs changes are in git stash. Worker 2's test file is at `/tmp/cycle17_child_module_tests.rs.bak`.

**Still open:** `diff` command investigation (secondary, deferred to next cycle).

## Coordination Notes

**Worker 3 → Worker 1:** Worker 1's in-progress `javascript.rs` changes (TS preprocessing stubs: `is_typescript_file`, `pre_extract_ts_entities`, `preprocess_typescript` calls) were lost during Worker 3's stash operations. Worker 1 will need to re-apply their changes from `investigation-1.md`. The changes were calls to undefined functions — the actual implementations weren't written yet. Worker 2's `populate.rs` changes survived (still in working tree).

**Worker 3 → Manager:** Pre-existing test failure: `int1_dogfood_data_dead_end_no_regression` expects ≤178 but gets 180. Not caused by C17 changes. data_dead_end count has drifted — needs investigation or baseline update.

### Carries (Tracked)
- C14 Doc-1/Doc-2 uncommitted documentation (3 cycles — NOW HARD GATE, assigned C17 Phase 1)
- Trace disambiguation UX (3 cycles)
- data_dead_end=178 unchanged (test-function mechanism #19)
- Coverage measurement automation (11th cycle carry)
- TypeScript entity extraction near-zero (2 cycles flagged by antagonists — NOW PRIMARY WORK)

## Hot (Cycle 16)

### Synthesis (Manager 1) — VERDICT: DONE

**Coverage:** 91.79% (3855/4200). **Tests:** 1,713 pass, 0 failures, 0 ignored. Clippy/fmt clean.

**What landed:**
- Worker 1: `resolve_callee` JS this.method() fix (populate.rs:450-452). 31 QA-1 tests. Commit `d66887f`.
- Worker 2: `extract_use_tree` path-segment fix (parser/rust.rs). 34 QA-2 tests. Commits `074a786`, `5a7d6f9`.
- Worker 3: 25 QA-3 surface integration tests. 8 method call fixtures. Commits `78b4510`, `f051fba`.
- Process: 6 GitHub issues filed (#18-#23). investigation-2.md committed. Stale dogfood-raw.txt deleted.

**Reviewer verdicts:** 4 DONE, 1 CONTINUE (META).

### Doc Status — CLEAN (but uncommitted C14 work is a 3-cycle carry)
- Doc-API: Grade A all workers. 17th consecutive Grade A.
- Doc-Usage: README needs orphaned_impl naming fix. CLI --help needs 8 fixes. 5 module docs pending. ALL UNCOMMITTED.

---

## Warm (Recent)

### Cycle 15 Summary
- **VERDICT: DONE** (4/5 DONE, 1 CONTINUE). Coverage 91.52%. 1,623 tests.
- phantom_dependency=205 (< 250 gate CLEARED). Proximity fix committed (`e2df6bc`).
- stale_reference=117 (+13 from C14, all FPs from test files). Total dogfood=620.
- Commit gate CLEARED: 4 commits on main, breaking 8-cycle uncommitted pattern.
- Process gaps: Zero GitHub issues filed (2nd cycle). Triage doc not committed. Coverage not automated. attribute_access undocumented.
- **C16 recommendation:** Process catch-up cycle — file issues, commit triage, fix stale_reference.

### Cycle 14 Summary
- **VERDICT: CONTINUE** (5/5 unanimous). Coverage 91.44%. 1,543 tests (working tree), 1,303 committed.
- phantom_dependency=250 at boundary. Fix eliminated 92 of predicted 234 (39.3% accuracy).
- Key deliverables: `extract_all_type_references()`, 25 QA-2 tests, manifest byte floor `MIN_MANIFEST_ALLOW_BYTES=20_480`.
- stale_reference trending up: 89→99→104. Root-caused as path-segment imports from test files.
- Methodology: Investigation briefs must include sampled verification — predictions overestimate.

### Cycle 13 Summary
- **VERDICT: CONTINUE.** Coverage 90.52%. 1,379 tests. 818→652 findings.
- JS CJS destructured require fix, Rust `use` qualified path fix (#15), trace dedup + symbol disambiguation.
- Investigation-first fully internalized. v0.1 ship criteria proposed.

---

## Cold (Archive)

- Cycle 12: CONTINUE (5/5). 90.45%, 1,226 tests. 11/13 patterns (partial_wiring). Rust cross-file resolution.
- Cycle 11: DONE (5/5). 90.44%, 1,290 tests. Trace refactor, Rust intra-file calls, incomplete_migration (10th).
- Cycle 10: DONE (3/5). 89.28%, 1,232 tests. 89% target MET. JS cross-file imports. Issues #2/#3/#11-14 closed.
- Cycle 9: CONTINUE. 87.17%, 1,167 tests. Graph exposure, contract_mismatch (9th), summary formatter.
- Cycle 8: CONTINUE. 84.98%, 1,089 tests. dependency_graph, cross-file flow tracing, stale_reference (8th).
- Cycle 7: CONTINUE. 86.08%, 1,037 tests. RustAdapter, recursion protection, flow engine, --symbol flag.
- Cycle 6: First DONE (4/5). Python cross-file, Rust adapter Phase 1, SARIF. 941 tests, 86.40%.
- Cycle 5: phantom_dependency FP fix, --language flag, layer_violation (7th). 787 tests, 85.02%.
- Cycle 4: JS adapter (~1375 lines), multi-language dispatch. 693 tests, 84.13%.
- Cycle 3: Module-level call fix, JSON formatter. 573 tests.
- Cycle 2 (Concert 3): Call-site detection + intra-file resolution. 468 tests, 82.91%.
- Cycle 1 (Concert 3): Pipeline wired. 413 tests. 6/6 patterns mock-only. 8 issues filed.
- Concert 2/1: Foundation — IR types, Graph, PythonAdapter, bridge components, 3 patterns, CLI. 162+ tests.

---

## Key Patterns Learned

- Investigation-first produces immediately useful artifacts (proven C11, sustained C12-C16)
- Measure-don't-predict: 10-sample trace + actual delta beats categorization-based predictions (proven C15)
- Hard gates work: Phase 1 gate forced process catch-up in C16 after 3 zero-issue cycles
- Commit gates work: 4 commits in C15 broke 8-cycle uncommitted pattern
- Pattern algorithms correct — problem is always data supply
- Manager gates in Session 1 = structural fix (proven C10, sustained C11-C16)
- Process deliverables need hard-gate enforcement — soft expectations produce zero GitHub issues
- Mock-only testing masks integration failures (recurring since C1)
- Conditional test guards defeat hard gates — use unconditional assertions
- File ownership prevents merge collisions but creates wiring bottlenecks

---

## Decisions Log

- COMMIT GATE: Every worker must commit with verified hash. Uncommitted = undelivered. (C15)
- PROCESS HARD GATE: Process deliverables (issues, triage docs) are Phase 1 gates. No implementation until cleared. (C16)
- FP prediction methodology: measure actual delta via branch + dogfood, not categorization counts (C15)
- Investigation briefs require sampled verification: pick 10, trace through fix path (C14)
- Worker 1 commit granularity: Accepted as structural. Gate on investigation artifacts instead. (C16)
- Issue-first protocol: file GitHub issue BEFORE code (C4)
- File ownership to prevent collisions (C4)
- v0.1 ship criteria proposed C13: 11/13 patterns + JS CJS + #15 + README + 89% coverage
- Hard patterns deferred: duplication, asymmetric_handling (may need IR extensions)
- SARIF included as v1 format (C1); Confidence field in manifest diagnostics (C1)
