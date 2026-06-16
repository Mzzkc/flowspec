# Flowspec Field Test Report: Dreambot

**Date:** 2026-04-04
**Flowspec version:** 0.1.0
**Tester:** Claude Opus 4.6 (AI agent, invoked via Claude Code CLI)
**Target codebase:** Dreambot — Discord bot (Python, discord.py framework)
**Codebase size:** 58 files, 860 entities, ~4,500 lines of application code
**Task context:** Two tasks were performed: (1) familiarize with an unknown codebase, (2) diagnose a specific bug where user data was not being recorded

---

## Executive Summary

Flowspec was used as the primary investigation tool across two tasks against Dreambot, a medium-sized Python Discord bot built on the discord.py framework with a Supabase (PostgreSQL) backend.

**Task 1 (codebase familiarization):** Flowspec provided useful high-level orientation (file count, entry points, module listing) but could not follow the application's actual execution path due to discord.py's runtime extension loading pattern. All 18 high-confidence diagnostic findings were false positives. The YAML manifest was comprehensive but the module role classifications were inaccurate. I ultimately learned the codebase by reading ~15 source files directly.

**Task 2 (bug diagnosis):** Flowspec surfaced two signals that drew my attention to the right area of code — a `contract_mismatch` on `record_interaction` and an `orphaned_impl` on the `my_data` command. However, neither finding identified the actual bug. The bug was a schema mismatch: `record_interaction` wrote a column (`max_tier_achieved`) that didn't exist in the PostgreSQL schema, causing every upsert to fail silently. Flowspec cannot compare Python dict keys against SQL schema definitions, so this class of bug is outside its detection capability. The bug was found by reading `database.py` and `schema.sql` side by side.

**Diagnostic accuracy (high-confidence only):**

| Pattern | Findings | True Positives | TP Rate |
|---------|----------|----------------|---------|
| `isolated_cluster` | 10 | 0 | **0%** |
| `data_dead_end` | 8 | 0 | **0%** |
| **Total (high-confidence)** | **18** | **0** | **0%** |

**Diagnostic accuracy (all findings):**

| Pattern | Findings | Sampled | Estimated TP Rate | Notes |
|---------|----------|---------|-------------------|-------|
| `isolated_cluster` | 10 | 10 (all) | **0%** | All discord.py cogs loaded via `load_extension()` |
| `data_dead_end` | 136 | 18 (high-conf) | **0%** sampled | Module-level vars used via `global`/mutation |
| `phantom_dependency` | 325 | 0 | Not sampled | Known broken per README |
| `orphaned_impl` | ~250 | ~5 | **0%** | discord.py decorator dispatch |
| `contract_mismatch` | 42 | 42 (all) | **0%** | All class method vs module wrapper (self param) |
| `missing_reexport` | ~44 | 0 | Not sampled | Expected for flat module structure |

**Flow tracing:** 71 flows detected, all within `tools/chat_tester.py` (a standalone test harness). 0 flows traced through the actual application path (`main.py` -> `bot.py` -> `message_events.py` -> `pools/draw.py`). The main entry point trace (`main.py::main`) returned 3 immediate calls but could not follow into the bot's runtime.

---

## Task 1: Codebase Familiarization

### Goal

Understand the architecture of an unfamiliar Discord bot codebase using flowspec as the primary tool, supplemented by source reading.

### What Flowspec Provided

The `analyze --format summary` output gave a useful starting point:
- File count (58), entity count (860), language (Python-only)
- Four entry points correctly identified: `main.py::main`, `chat_tester.py::main`, `analyze.py::main`, `analyze2.py::main`
- Module listing sorted by entity count, giving a rough size map

### What Flowspec Got Wrong

**Module role classification** was consistently inaccurate:
- `database.py` (77 entities) labeled "Utility module" — it's the persistence/infrastructure layer
- `bot.py` (6 entities) labeled "Data model module" — it's the application core and bot factory
- `constants.py` (67 entities) labeled "Service module" — it's static configuration data
- `message_events.py` (15 entities) labeled "Service module" — it's the primary event handler and application logic entry point

These labels would actively mislead an agent trusting them for navigation.

**All 18 high-confidence findings were false positives.** The 10 `isolated_cluster` findings flagged discord.py cogs — classes loaded at runtime via `bot.load_extension('cogs.moderation')` in `bot.py:34`. Flowspec sees the class definitions and their `setup()` functions but cannot connect them to the dynamic loading call. The 8 `data_dead_end` findings flagged module-level variables (`_global_recent_responses`, `_prison_glimpsed`, `_profile_cache`, etc.) that are all actively used through `global` declarations and mutation (`.append()`, `del`, TTLCache lookups).

**Flow tracing could not follow the application's main execution path.** The trace of `main.py::main` showed 3 flows to `keep_alive` and `create_bot`, but couldn't follow `create_bot()` -> `DreambotClient.setup_hook()` -> `load_extension()` -> `MessageEvents.on_message()` -> the 15-function-deep response pipeline. The 71 detected flows were all within `chat_tester.py`, a standalone CLI tool, not the actual bot application.

### How I Actually Learned the Codebase

Reading `bot.py` (77 lines), `message_events.py` (593 lines), `pools/draw.py` (234 lines), and `pools/registry.py` (811 lines). Four files, approximately 90 seconds of reading. This gave me the full architecture — entry point, extension loading, message handling pipeline, pool selection system, and data flow. Flowspec's 19,122-line YAML manifest did not add to this understanding.

---

## Task 2: Bug Diagnosis

### Goal

Determine why `!mydata` returns "no data" for users who have interacted with the bot.

### Flowspec's Contribution

Flowspec provided two indirect signals:

1. **`contract_mismatch` on `record_interaction`** (42 total contract_mismatch findings) — This drew attention to `record_interaction` existing at two locations: `database.py:844` (class method) and `database.py:1480` (module-level wrapper). While the finding itself was a false positive (the param count difference is just `self`), it put `record_interaction` in my field of view early.

2. **`orphaned_impl` on `Utilities::my_data`** — Flagged the `my_data` method as having "no dispatch points." This is a false positive (it's dispatched by `@commands.command(name='mydata')`), but it confirmed the entry point I needed to trace.

3. **`data_dead_end` on `delete_user_data`** — Found as a dead-end function. Also a false positive (it's called from the module-level wrapper), but it appeared in the same search results as `my_data`, helping me map the data lifecycle.

### What Flowspec Could Not Do

The actual bug was a **cross-system schema mismatch**: `record_interaction` (Python) wrote `max_tier_achieved` into a Supabase upsert, but the `user_profiles` PostgreSQL table (defined in `schema.sql`) has no such column — it has `dedication_tier` instead. PostgREST rejects unknown columns, so every upsert failed. The error was caught by a broad `except Exception` and logged as a warning, making the failure silent.

Flowspec cannot:
- Compare Python dict keys against SQL DDL column definitions
- Detect that a try/except is swallowing a critical failure
- Trace data flow from Python through a REST API to a database schema
- Identify that `profile_data['max_tier_achieved']` references a non-existent column

This class of bug (cross-system contract mismatch) is fundamentally outside flowspec's scope as a single-language static analyzer.

### How the Bug Was Actually Found

By reading `record_interaction` (database.py:844-932) and `schema.sql` (lines 513-526) side by side. The column name mismatch (`max_tier_achieved` vs `dedication_tier`) was immediately visible. The silent failure pattern (broad `except Exception` returning `{}`) explained why no error surfaced to users.

---

## Complete Command Log

### Command 1: Full Analysis (Summary)
```bash
flowspec analyze ./dreambot --format summary
```
**Purpose:** Get high-level codebase overview.
**Expected:** File count, module structure, entry points, top diagnostic issues.
**Actual:** Returned useful overview. 58 files, 860 entities, 71 flows, 767 diagnostics. Correctly identified 4 entry points. Module role classifications were inaccurate (see Task 1). Top issues were all `isolated_cluster` — all false positives.
**Exit code:** 0

### Command 2: High-Confidence Diagnostics
```bash
flowspec diagnose ./dreambot --severity warning --confidence high --format summary
```
**Purpose:** Filter to actionable findings only.
**Expected:** A small set of likely-true findings worth investigating.
**Actual:** 18 findings (10 `isolated_cluster`, 8 `data_dead_end`). All 18 were false positives. The confidence filter reduced volume (767 -> 18) but did not improve precision — TP rate was 0%.
**Exit code:** 2 (findings detected)

### Command 3: Full YAML Manifest
```bash
flowspec analyze ./dreambot --format yaml
```
**Purpose:** Get structured machine-readable output for deeper analysis.
**Expected:** Complete entity/flow/diagnostic data.
**Actual:** 19,122 lines of YAML. Entities and flows were present. Used `grep` to search for `mydata`/`user_data` keywords within the manifest — this was the most productive use of the manifest, as it surfaced the `orphaned_impl` on `my_data` and the `contract_mismatch` on `delete_user_data`.
**Exit code:** 0

### Command 4: Trace main.py Entry Point
```bash
flowspec trace ./dreambot --symbol "main.py::main" --format summary
```
**Purpose:** Follow the main execution path through the codebase.
**Expected:** A chain from `main` through `create_bot` into cog loading and event handling.
**Actual:** 3 flows, all immediate calls (`keep_alive`, `create_bot` x2). Could not follow through `create_bot()` into the bot's runtime behavior. The trace stopped at the first function boundary.
**Exit code:** 0

### Command 5: Trace record_interaction (Ambiguous)
```bash
flowspec trace ./dreambot --symbol "record_interaction" --format summary
```
**Purpose:** Trace the data recording path.
**Expected:** Flow from `on_message` through `record_interaction` to database operations.
**Actual:** Error — "Symbol 'record_interaction' matches multiple entities." Flowspec correctly identified the ambiguity between the class method (line 844) and module-level wrapper (line 1480) and asked for disambiguation. This was reasonable behavior.
**Exit code:** 1

### Command 6: Trace record_interaction (Disambiguated — Module Level)
```bash
flowspec trace ./dreambot --symbol "database.py::record_interaction" --format summary
```
**Purpose:** Trace the module-level `record_interaction` wrapper.
**Expected:** At minimum, a flow showing it delegates to `db.record_interaction(...)`.
**Actual:** "0 flow(s) matched." The module-level wrapper function (`def record_interaction(...): return db.record_interaction(...)`) produced no flows. Flowspec could not trace through instance method dispatch (`db.method()`).
**Exit code:** 0

### Command 7: Trace record_interaction (Disambiguated — Class Method)
```bash
flowspec trace ./dreambot --symbol "database.py::BotDatabase::record_interaction" --format summary
```
**Purpose:** Trace the class method implementation.
**Expected:** Internal call graph of `record_interaction`.
**Actual:** 2 flows — calls to `get_user_profile` and `calculate_dedication_tier`. This was partially useful; it confirmed the method's internal dependencies. However, it missed the Supabase upsert call (external library) and the dict construction that contained the buggy column name.
**Exit code:** 0

### Command 8: Trace mydata
```bash
flowspec trace ./dreambot --symbol "mydata" --format summary
```
**Purpose:** Trace the `!mydata` command handler.
**Expected:** Flow from the command through `get_user_profile` to response construction.
**Actual:** Error — "Symbol 'mydata' not found." The command name is registered via `@commands.command(name='mydata')` decorator, not as a Python symbol. The method name is `my_data`, not `mydata`. This is a discord.py framework pattern that flowspec cannot resolve.
**Exit code:** 1

### Command 9: Trace my_data (Correct Symbol)
```bash
flowspec trace ./dreambot --symbol "utilities.py::Utilities::my_data" --format summary
```
**Purpose:** Trace the `my_data` method after finding the correct symbol name.
**Expected:** Flow showing data retrieval and response construction.
**Actual:** 2 flows — calls to `get_user_profile` and `calculate_dedication_tier`. Partially useful. Could not trace into `get_user_profile`'s implementation or the embed construction logic.
**Exit code:** 0

### Command 10: Trace get_user_profile
```bash
flowspec trace ./dreambot --symbol "database.py::BotDatabase::get_user_profile" --format summary
```
**Purpose:** Trace the profile retrieval path that returns empty data.
**Expected:** Internal logic showing cache check, Supabase query, return paths.
**Actual:** "0 flow(s) matched." The method's internal logic (cache lookup, Supabase query, multiple return paths) produced no traceable flows. This is the most critical function in the bug investigation and flowspec provided zero visibility into it.
**Exit code:** 0

### Command 11: Contract Mismatch Diagnostics
```bash
flowspec diagnose ./dreambot --checks contract_mismatch --format summary
```
**Purpose:** Examine all contract mismatches to find potential signature bugs.
**Expected:** A small number of genuine mismatches between function definitions.
**Actual:** 42 findings. Every single one was the same false positive pattern: class method (`self, arg1, arg2`) vs module-level wrapper (`arg1, arg2`). The `self` parameter difference is inherent to Python's class system, not a contract mismatch. 0% TP rate. However, scanning these results for `record_interaction` and `delete_user_data` entries drew attention to the right files.
**Exit code:** 2

### Command 12: Manifest Keyword Search
```bash
flowspec analyze ./dreambot --format yaml 2>/dev/null | grep -i "mydata\|my_data\|user_data"
```
**Purpose:** Search the full manifest for data-related entities and diagnostics.
**Expected:** Entity IDs, flow references, and diagnostics mentioning the data path.
**Actual:** This was the single most productive flowspec interaction. It surfaced:
- `utilities.py::Utilities::my_data` with an `orphaned_impl` diagnostic
- `database.py::delete_user_data` with both a `data_dead_end` and `contract_mismatch`
- The decorator annotation `commands.command(name='mydata')` confirming the command registration
These results helped map the data lifecycle and identify the relevant code locations.
**Exit code:** 0

---

## Constructive Feedback

### Functionality Improvements

**1. Framework-aware analysis plugins**

The single biggest gap. Discord.py, Flask, Django, FastAPI — all use decorator-based dispatch and runtime loading patterns that are invisible to pure static analysis. A plugin system that teaches flowspec about common framework patterns would eliminate the majority of false positives seen in this test:
- `@commands.command(name='X')` registers a dispatch path
- `bot.load_extension('module.name')` creates a runtime import
- `@app.route('/path')` registers an HTTP handler
- `@tasks.loop(hours=N)` registers a background task

Even a simple heuristic — "if a method is decorated with `@commands.command`, it has a dispatch path" — would have eliminated all 10 `isolated_cluster` false positives and the `orphaned_impl` on `my_data`.

**2. Module-level mutation tracking**

All 8 high-confidence `data_dead_end` findings were on module-level variables that are mutated via `global` declarations, `.append()`, `del`, or framework patterns (TTLCache). Flowspec sees the initial assignment but not the subsequent reads/writes. Tracking `global X` declarations and method calls on module-level objects (`.append()`, `.__getitem__()`, `.__contains__()`) would eliminate this class of false positive.

**3. Cross-module flow tracing**

The README acknowledges this limitation, but it's worth emphasizing: the inability to trace flows across module boundaries makes the `trace` command nearly useless for real applications. In this test, `get_user_profile` — the single most important function for the investigation — returned 0 flows. The main entry point trace stopped at the first function call boundary. For the trace feature to deliver on its promise, it needs to follow `import` edges and resolve `self.method()` calls at minimum.

**4. Python class method vs wrapper false positives**

42 out of 42 `contract_mismatch` findings were the same false positive: `BotDatabase.method(self, x, y)` vs `def method(x, y): return db.method(x, y)`. This is the standard Python pattern for exposing class methods as module-level functions. Flowspec should recognize that when function A's body is `return instance.A(args)`, the parameter count difference of 1 (the `self` parameter) is expected, not a mismatch.

**5. Confidence calibration**

"High confidence" should mean "likely true." In this test, 0 of 18 high-confidence findings were true positives. The confidence score appears to be based on the diagnostic pattern type and finding characteristics, but it doesn't account for framework context. If the confidence system can't reliably separate true from false positives, it erodes trust — users who filter to `--confidence high` expect signal, not a smaller volume of the same noise.

### UX Improvements

**6. Better error messages for symbol-not-found**

When `flowspec trace --symbol "mydata"` failed, the error was: "Symbol 'mydata' not found. Run `flowspec analyze` to see available entities." A more helpful message would suggest fuzzy matches: "Did you mean `utilities.py::Utilities::my_data`?" The analyze output is 19K lines of YAML — telling the user to search it manually is not actionable guidance.

**7. Entity search / list command**

There's no way to ask "what entities exist matching pattern X?" without grepping the full YAML manifest. A command like `flowspec entities --pattern "*mydata*"` or `flowspec search mydata` would save significant time. The manifest grep workaround works but feels like using a screwdriver as a hammer.

**8. Trace should show why 0 flows were found**

When `trace` returns "0 flow(s) matched," there's no indication whether:
- The symbol exists but has no outgoing calls
- The symbol exists but its calls couldn't be resolved
- The symbol's calls go to external libraries (Supabase, discord.py) that aren't analyzed

A brief explanation ("0 flows: 3 calls detected but unresolved — `self.supabase.table(...)`, `_profile_cache.__contains__(...)`, `logger.debug(...)`) would tell the user what flowspec can and cannot see, rather than leaving them to guess.

**9. Summary format should show diagnostic TP rate context**

The summary output presents findings as authoritative facts. Given the known TP rates (documented honestly in the README), the summary should include a note like: "isolated_cluster: 10 findings (~33% historical TP rate)" so users calibrate their trust appropriately. The README's accuracy table is excellent — that same data should be surfaced at the point of consumption.

**10. Diff command for schema files**

For projects with SQL schemas, the most valuable diagnostic would be comparing Python code against database DDL — exactly the bug class found in this test. This is likely out of scope for a static code analyzer, but even a simple "columns referenced in code vs columns defined in .sql files" comparison would catch the `max_tier_achieved` bug instantly.

---

## Overall Assessment

Flowspec's vision — giving AI agents structural understanding of codebases — is the right goal. The CLI design is clean, the diagnostic pattern catalog targets real problems, and the README's honesty about accuracy is exemplary. The `--confidence high` filter and `--format` options show thoughtful UX design.

In practice, against this codebase, flowspec provided useful orientation data (file count, entry points, module listing) and one productive interaction (manifest keyword search that surfaced relevant entities). But its core promises — "how data moves through a system" and "what's broken" — were not delivered. Flow tracing couldn't follow the application's actual execution path. All high-confidence diagnostics were false positives. The bug that was found required reading source code and SQL schema side by side — a task that flowspec cannot assist with.

The tool is most useful today as a **fast structural indexer** (file counts, entity counts, entry point detection) and a **keyword-searchable entity database** (grepping the YAML manifest). It is not yet useful as a diagnostic tool or flow tracer for framework-heavy Python codebases. The gap between what the tool claims and what it delivers is significant, but the path forward is clear: framework-aware plugins, cross-module flow tracing, and mutation tracking would address the majority of false positives and tracing failures observed in this test.

For an agent performing the tasks I was given today, reading source files directly was faster and more accurate than flowspec's output for both familiarization and debugging. Flowspec's contribution was marginal — it pointed me toward the right neighborhood but couldn't identify the house.
