# Flowspec Field Test Report: Mozart AI Compose

**Date:** 2026-03-26
**Flowspec version:** 0.1.0
**Target codebase:** Mozart AI Compose — Python orchestration system
**Codebase size:** 228 Python source files, ~30K entities, 3,384+ tests
**Methodology:** Full analysis + 5 parallel validation agents spot-checking every finding category

---

## Executive Summary

Flowspec was run against Mozart AI Compose, a production Python codebase that uses Protocols, async/await, Pydantic v2, mixin composition, pub/sub patterns, and IPC dispatch. This report documents what flowspec gets right, what it gets wrong, and what it misses entirely — with evidence from 200+ manual spot-checks across all diagnostic categories.

**Overall accuracy by pattern:**

| Pattern | Findings | Sampled | True Positive Rate | Notes |
|---------|----------|---------|-------------------|-------|
| `isolated_cluster` | 54 | 54 (all) | **13%** (7/54) | Best signal; the 7 true positives were genuinely valuable |
| `data_dead_end` | 1,641 | 75 | **~8%** | ~90% are Protocol/dynamic dispatch false positives |
| `orphaned_impl` | 1,057 | — | **same as above** | 100% overlap with `data_dead_end` entities |
| `phantom_dependency` | 1,469 | 50 | **~0%** | Misses `__all__`, `TYPE_CHECKING`, type annotations |
| `missing_reexport` | 1,568 | — | Not sampled | Expected noise for deep package structures |
| `contract_mismatch` | 110 | 110 (all) | **0%** | 100% archive contamination (see Config section) |
| `incomplete_migration` | 7 | 7 (all) | **0%** | Sequential DB migrations + async/sync wrappers |
| `circular_dependency` | 0 | — | — | **13 exist, 0 found** (see Missed Patterns) |
| `layer_violation` | 0 | — | — | Correct: 0 exist, 0 found |

**Flow tracing:** 53 flows found, **0 meaningful**. All are within-file traces in standalone scripts. Flowspec never entered `src/mozart/` (the actual application). 53% of flows are duplicates.

---

## Critical Issues

### 1. Config System Is Non-Functional

**Severity: Critical (user-facing feature that silently does nothing)**

The `.flowspec/config.yaml` file is detected but **never read or deserialized**. The `Config::load()` function at `flowspec/src/config/mod.rs:39` contains:

```rust
// For now, just acknowledge the config file exists
return Ok(Self {
    config_path: Some(path.to_path_buf()),
    languages: Vec::new(),  // always empty
});
```

The `Config` struct has only two fields (`config_path` and `languages`), neither populated from the file. The `analyze()` function receives config as `_config: &Config` (underscore prefix = intentionally unused).

**Impact on this test:** All `exclude` patterns, `diagnostics.suppressions`, `analysis.entry_points`, and config-level `languages` were silently ignored. We had to work around this with `--language python` CLI flag and manual post-filtering.

**The `flowspec init` command generates a config template with `exclude` patterns**, giving users the impression the feature works. It does not.

**What's missing in Config struct:**

| Config Field | In YAML Template | In Struct | Deserialized | Used |
|-------------|-------------------|-----------|-------------|------|
| `languages` | Yes | Yes (always empty) | No | No |
| `exclude` | Yes | No | No | No |
| `diagnostics.enabled` | No | No | No | No |
| `diagnostics.suppressions` | No | No | No | No |
| `diagnostics.layer_rules` | No | No | No | No |
| `analysis.entry_points` | No | No | No | No |
| `analysis.max_call_depth` | No | No | No | No |

### 2. No .gitignore Respect

**Severity: High**

Flowspec scans all directories except a hardcoded list (`target`, `node_modules`, `__pycache__`, `.git`, `.flowspec`, `build`, `dist`, `.venv`, `venv`). It does not read `.gitignore`.

Mozart's `workspaces/` directory is gitignored and contains archived copies of `src/mozart/` from previous job runs. Flowspec analyzed 505 files from these archives, producing:

- **10,405 contaminated diagnostics** (59% of all output)
- **110 contract_mismatch findings** (100% were current-code-vs-archived-copy comparisons)
- Duplicate module names in the summary (same module appearing 2-3 times)
- Duplicate entry points

### 3. Flow Tracing Doesn't Cross Module Boundaries

**Severity: High (core feature fundamentally limited for Python)**

Of 53 flows traced:

| Quality | Count | Description |
|---------|-------|-------------|
| Good (cross-module, accurate) | **0** | — |
| Shallow (within-file, stops at boundary) | 31 | Correct but trivially incomplete |
| Trivial ("main calls import") | 13 | Import statement treated as flow exit |
| Duplicate (same flow, different ID) | ~28 | 53% duplication rate |

**Root cause:** Flowspec found entry points in 4 standalone scripts (`scripts/backfill_learning.py`, `scripts/generate-iterative-dev-loop.py`, `scripts/reset-sheet.py`, `scripts/check-dashboard-page.py`) but never followed imports into `src/mozart/`. The actual application — 228 Python files with the CLI, daemon, runner, backends, and learning system — produced zero flows.

**What flowspec should have traced but couldn't:**
- CLI -> IPC -> Daemon -> JobService -> Runner -> Backend (8+ module boundaries, async, IPC)
- Score YAML -> Config parsing -> Validation -> Sheet creation (Pydantic model construction)
- Event publishing through EventBus (pub/sub, callback registration)
- Checkpoint save/load cycle (Protocol-based backend selection)
- Learning store record/query (14-module mixin composition)

---

## Diagnostic Pattern Accuracy

### `isolated_cluster` — 13% True Positive Rate (Best Signal)

**54 findings, 54 validated. 7 true positives, 47 false positives.**

The 7 true positives were genuinely valuable — code clusters that are built and tested but never called from production paths:

1. `RunSummary.to_dict()` + `_format_duration()` — dead serialization methods
2. `DelayOutcome` + `record_delay_outcome()` — unused circuit breaker feedback
3. `ErrorLearningHooks` + `wrap_classifier_with_learning()` — built, not integrated
4. `OutcomeMigrator` + `migrate_existing_outcomes()` — migration utility never called
5. `SchedulerStats` + `get_stats()` — scheduler infrastructure not yet wired
6. `ErrorChain` + `to_error_chain()` — built, not used
7. `TableMapping` + `get_state_registry()` — schema registry not wired

**False positive causes (47/54):**
- Instance method calls `self.method()` / `obj.method()` (primary cause)
- Mixin methods called through class composition
- Framework-registered functions (FastAPI routes, Textual actions, structlog processors)
- Cross-module imports via `__init__.py` re-exports
- Factory pattern instantiation (`Class.from_config()`)

### `data_dead_end` + `orphaned_impl` — ~8-10% True Positive Rate

**2,698 findings (1,641 unique — `orphaned_impl` is 100% subset of `data_dead_end`). 75+ spot-checked.**

The double-counting inflates the apparent finding count by 64%.

**False positive breakdown (by estimated prevalence):**

| Category | % of Findings | Example |
|----------|--------------|---------|
| Protocol/ABC implementations | ~30% | `Backend.execute()`, `ValidationCheck.check()` |
| Dynamic dispatch (IPC, callbacks) | ~25% | `JobManager.start()` via IPC handler registration |
| Same-file internal usage | ~15% | Module constants used 10 lines below definition |
| Framework magic (Pydantic, logging) | ~8% | `@model_validator` methods |
| Inheritance | ~7% | `HttpxClientMixin._get_client()` used by subclasses |
| Re-exports / API surface | ~5% | Exported in `__all__` but callers use through __init__ |

**True positives found (~8%):**
- 6 dead constants in `core/constants.py`
- 7 dead functions in `cli/helpers.py`
- ~20 tested-but-unused API surface (output helpers, notification methods, etc.)

### `phantom_dependency` — ~0% True Positive Rate

**1,469 findings in src/mozart/. 50 sampled. 0 true positives.**

Every single sampled finding was a false positive. The checker has three systematic blind spots:

1. **`__all__` re-exports (40% of sample):** `__init__.py` imports symbols and lists them in `__all__` for public API. Flowspec sees no "usage" in the file.
2. **`TYPE_CHECKING` imports (24% of sample):** Imports guarded by `if TYPE_CHECKING:` for type annotations. Flowspec doesn't look inside the guard block. Also flags `TYPE_CHECKING` itself as unused.
3. **Type annotation usage (28% of sample):** Symbols used only in function signatures, return types, or class field annotations. Flowspec doesn't count annotation positions as references.
4. **Standard usage missed (8% of sample):** `class X(str, Enum)`, `Depends(get_templates)`, `app.command()(modify)` — real runtime references in argument positions.

### `incomplete_migration` — 0% True Positive Rate

**7 findings. All 7 validated. 0 true positives.**

- 6 findings flag sequential SQLite schema migrations (`_migrate_v1` through `_migrate_v4`) as incomplete migrations. These are incremental DB migrations that all must coexist permanently — a database at version 0 needs all four applied.
- 1 finding flags `_enforce_size_cap_sync` vs `_enforce_size_cap_async` as an incomplete migration. The `_sync` version is the core implementation; `_async` is a wrapper that calls `_sync` internally with added error handling. Both are needed for async-loop-present vs sync contexts.

### `contract_mismatch` — 0% True Positive Rate (All Archive Noise)

**110 findings. All 110 were comparisons between current source and archived workspace copies.** With archive paths excluded, zero contract_mismatch findings remain.

---

## What Flowspec Missed Entirely

### 13 Circular Dependencies (0 Found)

Flowspec's `circular_dependency` checker found nothing. Manual analysis found 13 import cycles:

| Severity | Count | Examples |
|----------|-------|---------|
| Benign (TYPE_CHECKING guarded) | 9 | Dashboard app <-> routes (7), healing diagnosis <-> registry/remedies (2) |
| Design smell (deferred runtime imports) | 3 | daemon backpressure/monitor/manager, execution parallel/runner, learning outcomes/patterns |
| Serious (runtime breaking) | 0 | — |

The benign cycles follow standard Flask/framework patterns. The 3 design smells work but rely on in-method deferred imports that are fragile if moved to module level.

### Instance-Attribute Method Dispatch (40% of Entities Disconnected)

Of ~3,087 entities in the call graph, **1,247 (40%) have zero `calls` and zero `called_by`** — completely disconnected. Most are methods called via `self._backend.execute()`, `self._event_bus.publish()`, `self.state.mark_sheet_completed()` patterns.

This is the single largest gap. Resolving instance-attribute types from `__init__` parameter annotations or `self._attr: Type` assignments would connect the majority of these orphaned entities.

### Mixin/MRO Method Resolution

Mozart's `JobRunner` is composed from 8 mixins across 9 files (~4,600 LOC). The class body is `pass`. Flowspec traced **0 flows** from it because all behavior lives in inherited mixins. MRO resolution would make this entire class traceable.

### Abstract Method -> Implementation Linking

`Backend::execute` (abstract) has `calls: [], called_by: []`. Its 4 implementations (`ClaudeCliBackend`, `AnthropicApiBackend`, `OllamaBackend`, `RecursiveLightBackend`) are not connected to the abstract definition. The subclass relationship exists but isn't modeled as a dispatch edge.

---

## Trace Command Evaluation

8 traces tested against key Mozart symbols:

| Symbol | Found? | Flows | Quality | Key Issue |
|--------|--------|-------|---------|-----------|
| `main` (CLI entry) | Yes | 1 | Trivial | Typer dispatch invisible |
| `JobManager` | Yes | 1 | Shallow | 30+ methods, found 1 connection |
| `JobRunner` (8 mixins) | Yes | 0 | **Failed** | Mixin composition = empty class body |
| `ClaudeCliBackend::execute` | Yes | 10 | **Good** | Best result — intra-class calls work |
| `CheckpointState` | Yes | 9 | Moderate | Found construction/load, missed usage |
| `EventBus` | Yes | 0 | **Failed** | `self._event_bus.publish()` invisible |
| `GlobalLearningStore` | Yes | 1 | Shallow | Singleton accessor only |
| `validate` (CLI command) | Yes | 13 | Good | Within-command flow accurate |

**What traces well:** Intra-class method chains (`self.method()` where the method is defined in the same class). `ClaudeCliBackend::execute` → `_execute_impl` → helpers was accurately mapped.

**What consistently fails:** Instance-attribute dispatch, mixin inheritance, abstract→implementation, framework decorators, pub/sub callbacks.

**Positive finding:** Async generators (`SSEManager::connect`, `DaemonClient::stream`) traced correctly. The async iteration pattern doesn't break tracing.

---

## Recommendations

### P0 — Config System

1. **Deserialize the config file.** The YAML is never read. Add `serde_yaml::from_reader` in `Config::load()` and populate the struct.
2. **Wire `exclude` patterns to `discover_source_files()`.** Replace or augment the hardcoded `skip_dirs` array.
3. **Wire `languages` from config** (currently only works via CLI `--language` flag).
4. **Implement `suppressions`** — entity+diagnostic pairs to exclude from output.
5. **Read `.gitignore`** — or at minimum, provide a `--respect-gitignore` flag. The `ignore` crate handles this.

### P1 — Python Semantic Resolution

6. **Instance-attribute type resolution.** When `self._backend: Backend = backend` is set in `__init__`, resolve `self._backend.execute()` through the declared type. This single improvement would connect ~40% of currently-orphaned entities.
7. **`__all__` re-export recognition.** If a symbol appears in `__all__`, it's used (re-exported as public API).
8. **`TYPE_CHECKING` block awareness.** Imports inside `if TYPE_CHECKING:` blocks are used for type annotations. The `TYPE_CHECKING` symbol itself is used when `if TYPE_CHECKING:` appears.
9. **Type annotation positions as references.** Function parameter types, return types, and class field annotations are usages of the imported symbol.

### P2 — Structural Analysis

10. **Mixin/MRO resolution.** When `class X(Mixin1, Mixin2, Base): pass`, resolve all inherited methods as belonging to X. Python's MRO is deterministic and computable from the class definition.
11. **Abstract→implementation linking.** When `Backend::execute` is abstract and `ClaudeCliBackend(Backend)` overrides `execute`, create a polymorphic dispatch edge.
12. **Circular dependency detection for Python.** The import graph is statically extractable from `import`/`from...import` statements. Detect cycles, and note which are guarded by `TYPE_CHECKING`.
13. **Deduplicate `data_dead_end` and `orphaned_impl`.** They flag the same entities — either merge them or ensure they report distinct information.

### P3 — Flow Quality

14. **Cross-file module resolution.** Follow `from mozart.X import Y` into the target module. Currently, flows stop at file boundaries.
15. **Deduplicate flows.** 53% of traced flows were duplicates with different IDs.
16. **Don't treat imports as flow exits.** "main → import::Path" is not a meaningful data flow.
17. **Report trace depth.** Add `depth_reached` vs `depth_limit` to trace output so users know if they're hitting a wall.

### Nice-to-Have

18. **Fuzzy symbol suggestion on not-found.** When `DaemonManager` isn't found, suggest `JobManager` from `daemon/manager.py`.
19. **Sequential migration pattern recognition.** Functions named `_migrate_v1`...`_migrate_vN` called from a version-checking dispatcher are not incomplete migrations.
20. **Framework-aware dispatch.** For well-known frameworks (FastAPI, Typer/Click, Pydantic, pytest), model the registration/dispatch patterns decorators create.

---

## What Flowspec Gets Right

Despite the issues, several things work well:

1. **Entity extraction is comprehensive.** 30,554 entities across 980 files is thorough — classes, functions, methods, constants, imports all found.
2. **Intra-class call tracing is accurate.** The `ClaudeCliBackend` trace produced 10 correct flows through its internal method chain.
3. **Layer violation detection** would work if circular_dependency were implemented — the entity graph has enough data to check import paths between packages.
4. **The diagnostic framework is well-designed.** Severity, confidence, evidence with locations, and suggestions are all structured well. The output format (YAML, JSON, SARIF, summary) is flexible and AI-consumable.
5. **Async generators trace correctly** — a positive surprise given other async gaps.
6. **Symbol disambiguation** lists all matches with file paths and line numbers when multiple symbols match.

---

## Test Setup

The analysis was run from Mozart's project root with:
```bash
flowspec analyze . --full -l python --format json -o manifest.json
```

Config file was written to `.flowspec/config.yaml` but had no effect (see Critical Issue #1). Manual post-filtering excluded `workspaces/archive/` contamination from all validation work.

Five parallel validation agents each spot-checked a different finding category:
- **Agent 1:** All 54 `isolated_cluster` findings (100% coverage)
- **Agent 2:** 75+ `data_dead_end` / `orphaned_impl` findings across 27+ files
- **Agent 3:** 53 flows + circular dependency / layer violation / stale reference checks
- **Agent 4:** 50 `phantom_dependency` + all 7 `incomplete_migration` findings
- **Agent 5:** 8 `flowspec trace` invocations against key architecture symbols
