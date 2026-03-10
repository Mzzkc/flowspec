# TASK: Build a Mozart Concert to Implement Flowspec

You are building a complete Mozart AI Compose concert — a set of YAML score files that, when executed by Mozart, will produce a working Flowspec tool. Flowspec is a standalone Rust CLI tool that produces structured, AI-readable data flow manifests for codebases.

Before writing ANY scores, you MUST:

1. Read the Mozart score authoring skill at `/home/emzi/.claude/skills/mozart-score-authoring.md` — this is the **definitive guide** for how to write scores. Read it completely. It covers syntax, validation engineering, prompt design, Jinja mastery, fan-out architecture, and common pitfalls. The skill is the source of truth; this prompt supplements it with Flowspec-specific context.
2. Read ALL example scores in `~/Projects/mozart-ai-compose/examples/` — understand the patterns (especially `quality-continuous.yaml` for multi-stage coding workflows, `parallel-research-fanout.yaml` for fan-out, and `prelude-cadenza-example.yaml` for file injection)
3. Read scores in `~/Projects/claude-compositions/scores/` — more examples of fan-out, synthesis, and creative score design
4. Read the CLAUDE.md at `~/Projects/mozart-ai-compose/CLAUDE.md` — project instructions including how to run Mozart
5. Read the configuration reference at `~/Projects/mozart-ai-compose/docs/configuration-reference.md` — every config field documented
6. Read the Flowspec spec at `~/Projects/flowspec/spec.md` — the specification for what you're building

Study these sources. The score authoring skill is the canonical reference. This prompt adds Flowspec-specific decisions that the skill can't know about.

---

## WHAT IS MOZART AI COMPOSE

Mozart orchestrates long-running AI coding sessions by breaking work into stages — atomic units of work that an LLM agent (Claude Code via CLI) executes. Stages can run sequentially or in parallel based on a dependency DAG. Stages can fan out into multiple parallel instances for independent perspectives, then fan back in for synthesis.

The score authoring skill covers the full anatomy and every field. Here, only the fields and principles specific to this concert are highlighted.

### Key Principles for This Concert

**1. Validations must be unfudgeable.** The central question for every validation: **"Could an AI agent pass this without actually achieving the goal?"** If yes, the validation is theater. Self-report markers like `IMPLEMENTATION_COMPLETE: yes` are trivially fudgeable — the agent writes the marker without doing the work. `cargo test` is better but still gameable if the agent writes tests that assert `true`. The strongest validation is: **run the tool against a fixture project with known properties and verify the output contains those properties.** The fixture projects are the test oracle. Design them at score-writing time with specific, known entities so validations can check for facts the agent can't fabricate without doing real work.

**2. Specify outcomes and contracts, not steps.** For coding stages, specify WHAT should exist when the stage completes (interfaces, types, behaviors) but not HOW to build it (which tools to call, what order). The agent reasons and adapts. However, DO specify the interfaces and contracts precisely — function signatures, error types, module boundaries. Vague outcomes are as bad as prescriptive steps.

**3. Two template systems — don't mix them up.** Prompt templates use Jinja2 (`{{ workspace }}`). Validation paths and commands use Python format strings (`{workspace}`). The score authoring skill covers this in detail.

**4. Use `stage` not `sheet_num` for template conditionals.** Even without fan-out, `stage` is the semantically correct variable. It stays stable if you later add fan-out.

**5. `skip_permissions: true` and `disable_mcp: true` are required for unattended execution.** Without `skip_permissions`, the agent prompts for permission and hangs. Without `disable_mcp`, MCP server startup adds latency and can cause contention.

### Template Variables Available in Validations

Confirmed from Mozart source code (`src/mozart/execution/validation/engine.py`): validation `path`, `command`, and `working_directory` fields support these variables via `{variable}` syntax:

| Variable | Available |
|---|---|
| `{workspace}` | Always |
| `{sheet_num}` | Always |
| `{start_item}` | Always |
| `{end_item}` | Always |
| `{stage}` | Always (equals sheet_num without fan-out) |
| `{instance}` | Always (1 without fan-out) |
| `{fan_count}` | Always (1 without fan-out) |
| `{total_sheets}` | Always |
| `{total_stages}` | Always |

User-defined variables from `prompt.variables` are NOT available in validations. For complex validation logic that needs custom data, use `command_succeeds` with inline scripts.

### Common Score Writing Mistakes

Refer to the score authoring skill's "Common Pitfalls" table for the comprehensive list. The most relevant for this concert:

| Mistake | Fix |
|---|---|
| `{{ workspace }}` in validation paths | Use `{workspace}` |
| `{workspace}` in prompt templates | Use `{{ workspace }}` |
| No `skip_permissions: true` | Agent hangs |
| No `disable_mcp: true` | Slow startup |
| `fresh: true` between different phases | Destroys code built by previous phases |
| Relative paths for `job_path` | Must be absolute |
| `max_attempts` (wrong field) | Use `max_retries` |
| `backoff_multiplier` (wrong field) | Use `exponential_base` |
| Self-report markers as primary validation | Use `cargo test`, functional CLI checks, manifest content verification |
| Stale detection timeout too short | Use `idle_timeout_seconds: 1800` for cargo builds |

---

## WHAT FLOWSPEC IS

Flowspec is a standalone Rust CLI tool that crawls a codebase using language server protocols and AST analysis, then produces a structured YAML manifest describing:

1. **Entity Registry** — Every meaningful unit (functions, structs, traits, modules) with their signatures, visibility, relationships
2. **Flow Paths** — Traced routes data takes from entry points to exit points, step by step
3. **Boundary Map** — Every interface where data crosses a meaningful boundary (module, crate, network, serialization)
4. **Diagnostics** — Dead ends, orphan consumers, duplications, contract mismatches, missing error paths, circular dependencies
5. **Dependency Graph** — Module-level and crate-level dependency structure with direction and weight
6. **Type Flow Matrix** — Where each significant type is created, transformed, and consumed

### Why This Tool Exists

AI coding tools operate on codebases at the text level. They don't have a structural map of how data flows through a system. Flowspec generates that mental model as a structured artifact that any AI tool can consume directly.

See `spec.md` for the complete specification: manifest format, CLI interface, architecture diagram, configuration, and integration points.

---

## CONSTRAINTS

### Musts
- Rust, using Cargo. Single binary. Commit `Cargo.lock` (it's a binary project).
- Use `lsp-types` crate for LSP protocol types. For the LSP client transport, use raw JSON-RPC over stdin/stdout (`tokio` + `serde_json`). **Note: `tower-lsp` is an LSP *server* framework, not a client. Do not use it for the client side.**
- Use `tree-sitter` with `tree-sitter-rust` for AST enrichment beyond LSP
- Use `clap` for CLI argument parsing
- Use `serde` + `serde_yaml` for manifest serialization
- Use `tracing` for logging (not println/eprintln)
- All output is valid YAML parseable by any YAML library
- The tool must work on any Rust project with a valid Cargo.toml
- Every public function must have `///` documentation
- Error handling: `thiserror` for library code, `anyhow` only in `main.rs`
- Integration tests must use the fixture projects defined below
- rust-analyzer must be available: first check PATH, then try `rustup component add rust-analyzer`. **Escalation trigger: if automatic acquisition proves too complex, document the blocker and require rust-analyzer on PATH.**

### Must Nots
- Do NOT use nightly Rust features. Stable toolchain only.
- Do NOT produce manifests larger than 10x the source code size
- Do NOT attempt to analyze generated code (target/, build artifacts)
- Do NOT use `unsafe` unless absolutely required for FFI

### Preferences
- Prefer tree-sitter over syn for initial parsing (tree-sitter is error-tolerant)
- Prefer streaming analysis over loading everything into memory
- Prefer integration tests that test the full pipeline over unit tests of internals
- Prefer YAML output over JSON

### Escalation Triggers (mark in result files if encountered)
- If rust-analyzer LSP proves too complex to embed, document the blocker and suggest alternatives
- If tree-sitter-rust grammar doesn't cover a needed construct, document what's missing
- If the manifest format needs changes from what's specified, document why and propose the change
- If a diagnostic type proves infeasible with static analysis alone, mark it "requires dynamic analysis" and stub it

---

## FIXTURE PROJECTS — THE TEST ORACLE

The fixture projects are how validations verify real work was done. They must be created in Phase 1 Stage 1 with **exact, known properties** that later validations check against. The agent cannot fudge a validation that says "the manifest for simple-project must contain an entity named `math::add` with two i32 parameters" — it either built a working analyzer or it didn't.

### `tests/fixtures/simple-project/`

A minimal Rust project with known, verifiable properties.

```
simple-project/
├── Cargo.toml          # [package] name = "simple-project"
├── src/
│   ├── main.rs         # fn main() calls greet::hello("world") and math::add(1, 2)
│   ├── lib.rs          # pub mod math; pub mod greet;
│   ├── math.rs         # pub fn add(a: i32, b: i32) -> i32 { a + b }
│   │                   # pub fn multiply(a: i32, b: i32) -> i32 { a * b }
│   └── greet.rs        # pub fn hello(name: &str) -> String { format!("Hello, {name}!") }
```

**Known properties for validation:**
- Entity `math::add`: public function, takes `(i32, i32)`, returns `i32`
- Entity `math::multiply`: public function, takes `(i32, i32)`, returns `i32`
- Entity `greet::hello`: public function, takes `&str`, returns `String`
- Entity `main`: calls `greet::hello` and `math::add`
- `math::multiply` is a dead end (defined but never called from main or greet)
- Module boundary: `main` → `math`, `main` → `greet`
- Total: at least 4 public functions across 3 modules

### `tests/fixtures/complex-project/`

A multi-module project with deliberate structural issues for diagnostic testing.

```
complex-project/
├── Cargo.toml          # [package] name = "complex-project"
│                       # [dependencies] serde = { version = "1", features = ["derive"] }
│                       #                serde_json = "1"
│                       #                thiserror = "1"
├── src/
│   ├── main.rs         # fn main() calls api::handle_request(...)
│   ├── lib.rs          # pub mod api; pub mod service; pub mod model; pub mod errors; pub mod unused;
│   ├── api/
│   │   ├── mod.rs      # pub mod handler;
│   │   └── handler.rs  # pub fn handle_request(req: model::Request) -> Result<model::Response, errors::AppError>
│   │                   #   calls service::process(&req.data)
│   ├── service/
│   │   ├── mod.rs      # pub mod processor;
│   │   └── processor.rs # pub fn process(data: &str) -> Result<String, errors::AppError>
│   ├── model/
│   │   ├── mod.rs      # pub mod request; pub mod response;
│   │   ├── request.rs  # #[derive(serde::Deserialize)] pub struct Request { pub data: String }
│   │   └── response.rs # #[derive(serde::Serialize)] pub struct Response { pub result: String }
│   ├── errors.rs       # #[derive(thiserror::Error)] pub enum AppError { #[error("processing failed")] ProcessingError(String) }
│   │                   # NOTE: missing variant for serialization errors — deliberate gap
│   └── unused.rs       # pub fn orphaned_function() -> String { "never called".into() }
│                       # pub struct DeadStruct { pub field: i32 }
```

**Known properties for validation (diagnostics):**
- **Dead ends:** `unused::orphaned_function` and `unused::DeadStruct` — zero callers outside their module
- **Serialization boundary:** `model::Request` derives `Deserialize`, `model::Response` derives `Serialize`
- **Module boundaries:** `api` → `service` → cross-module call via `process()`
- **Missing error path:** `AppError` has no serialization error variant, but `model::Response` derives `Serialize` (serde errors unhandled)
- **Flow path:** `main` → `api::handle_request` → `service::process` → return through `model::Response`
- **Type flow:** `Request` created via deserialization, consumed by `handle_request`; `Response` created by `handle_request`, consumed via serialization

These properties are the ground truth. Validations in Phases 2-4 check that Flowspec's output contains them.

---

## CONCERT STRUCTURE

This concert has 5 phases. Each phase is a separate score file. Phases chain via `on_success` hooks.

**CRITICAL: Concert chaining between phases uses `fresh: false` (the default).** `fresh: true` would archive/clear the workspace, destroying the Flowspec project built by previous phases. `fresh: true` is only for self-chaining.

### Workspace & Paths

All scores use the same workspace. The definitive paths:

```
WORKSPACE:     /home/emzi/Projects/flowspec/workspace
CONCERT DIR:   /home/emzi/Projects/flowspec/workspace/concert/
FLOWSPEC CODE: /home/emzi/Projects/flowspec/workspace/flowspec/
SPEC FILE:     /home/emzi/Projects/flowspec/workspace/spec.md
```

The `backend.working_directory` in each score should be set to the flowspec code directory so agents start in the right place:

```yaml
backend:
  working_directory: "/home/emzi/Projects/flowspec/workspace/flowspec"
```

### Pre-Concert Setup

Before running the concert, the spec must be placed in the workspace:

```bash
mkdir -p /home/emzi/Projects/flowspec/workspace
cp /home/emzi/Projects/flowspec/spec.md /home/emzi/Projects/flowspec/workspace/spec.md
```

Include this in the CONCERT-README.md as a prerequisite step. Phase 1 Stage 1 should also verify the spec file exists and copy it if missing.

### Phase 1: Foundation (5 stages)
Project scaffolding, core types, CLI skeleton, configuration parsing, manifest serialization.
After this phase: `flowspec --help` works, config files parse, manifest types serialize to valid YAML, fixture projects exist, `cargo test` passes.

### Phase 2: LSP Client (5 stages)
LSP client implementation — connect to rust-analyzer, send requests, parse responses, build entity registry and call graphs from LSP data.
After this phase: `flowspec analyze tests/fixtures/simple-project/` produces a manifest containing the known entities from the fixture project.

### Phase 3: AST Enrichment (5 stages)
Tree-sitter integration — parse Rust files for information LSP doesn't provide: serialization boundaries, error chain analysis, derive macro detection, naming convention inference.
After this phase: the manifest includes boundary detection, serialization boundaries from serde derives, and type flow tracking.

### Phase 4: Diagnostics Engine (5 stages)
The diagnostic analysis layer — dead end detection, orphan consumers, duplications, contract mismatches, missing error paths, circular dependencies, layer violations.
After this phase: `flowspec diagnose tests/fixtures/complex-project/` finds the dead ends, missing error paths, and serialization boundaries planted in the fixture.

### Phase 5: Diff, Watch, and Polish (4 stages)
Manifest diffing, incremental watch mode, integration tests against both fixture projects, documentation, release preparation.
After this phase: the tool is complete, tested, documented, and ready to use.

---

## ACCEPTANCE CRITERIA PER PHASE (Unfudgeable)

Each criterion below is designed so an agent cannot pass the validation without doing real work.

### Phase 1: Foundation
- `cargo build` succeeds
- `cargo test` passes all tests
- `cargo clippy -- -D warnings` is clean
- `flowspec --help` exits 0 and stdout contains "analyze" and "diagnose" (proves real CLI, not empty binary)
- `flowspec --version` prints a version string
- `.flowspec/config.yaml` parsing works — a test loads a config and asserts fields parse correctly
- Manifest types round-trip: create in code → serialize to YAML → deserialize → assert equality
- Both fixture projects exist and `cargo check` succeeds in each

### Phase 2: LSP Client
- `flowspec analyze tests/fixtures/simple-project/ --output /tmp/test.yaml` succeeds
- The output manifest YAML contains entity `math::add` (proves real analysis, not hardcoded output)
- The output manifest contains entity `greet::hello`
- The output manifest contains at least 4 entities total
- rust-analyzer process is not left running after analysis (no zombies)

### Phase 3: AST Enrichment
- `flowspec analyze tests/fixtures/complex-project/ --output /tmp/test.yaml` succeeds
- The output manifest identifies `model::Request` as having a `Deserialize` derive
- The output manifest identifies `model::Response` as having a `Serialize` derive
- The output manifest contains a boundary between `api` and `service` modules
- Type flow section exists and contains `Request` type

### Phase 4: Diagnostics
- `flowspec diagnose tests/fixtures/complex-project/` succeeds
- Diagnostics output identifies `unused::orphaned_function` as a dead end
- Diagnostics output identifies `unused::DeadStruct` as a dead end
- Diagnostics output identifies the missing error path (AppError missing serialization variant)
- `flowspec diagnose --checks dead-ends` filters to only dead end diagnostics

### Phase 5: Diff, Watch, Polish
- `flowspec diff a.yaml b.yaml` exits 0 when comparing two manifests
- `flowspec analyze` succeeds against both fixture projects
- `cargo clippy -- -D warnings` clean, `cargo fmt --check` clean
- README.md exists and contains "Usage" section
- All public items have doc comments (validated via `cargo doc --no-deps 2>&1 | grep -c warning` = 0)
- CI workflow file exists at `.github/workflows/ci.yml`

---

## VALIDATION STRATEGY

### The Anti-Gaming Principle

For every validation, ask: **"Could an agent fudge this?"**

| Validation | Fudgeable? | Verdict |
|---|---|---|
| `IMPLEMENTATION_COMPLETE: yes` in result file | Trivially — agent writes marker without doing work | Worthless as primary validation |
| `cargo test` passes | Partially — agent can write `assert!(true)` tests | Necessary but insufficient |
| `cargo clippy` clean | No — but proves code quality, not functionality | Good secondary check |
| `flowspec --help` contains "analyze" | No — requires real CLI with real subcommands | Good |
| Manifest from fixture contains `math::add` | No — requires working analysis pipeline that actually found the function | Strong |
| Diagnostics find `orphaned_function` as dead end | No — requires working dead-end detection against real code | Strong |

**Hierarchy:** Functional checks against known fixtures > build/test/lint > structural content checks > self-report markers.

### Result Files: Context, Not Validation

Result files (`stage{stage}-result.md`) serve a purpose: they provide structured context for cross-sheet communication and human debugging. But they should NEVER be the primary validation gate. They're supplementary. Keep them — they're useful for `previous_outputs` and for debugging when a stage fails — but don't rely on them to prove work was done.

### Validation Structure Per Phase

Each phase score should use staged validations with conditions:

```yaml
validations:
  # ── VALIDATION STAGE 1: Build (fast, fail-fast) ──
  - type: command_succeeds
    command: 'cd {workspace}/flowspec && cargo build 2>&1'
    description: "Project builds"
    stage: 1

  - type: command_succeeds
    command: 'cd {workspace}/flowspec && cargo clippy -- -D warnings 2>&1'
    description: "Clippy clean"
    stage: 1

  # ── VALIDATION STAGE 2: Tests pass ──
  - type: command_succeeds
    command: 'cd {workspace}/flowspec && cargo test --all 2>&1'
    description: "All tests pass"
    stage: 2

  # ── VALIDATION STAGE 3: Functional checks (phase-specific, unfudgeable) ──
  # These are the phase-specific validations that prove the actual goal was achieved.
  # Examples for Phase 2:
  - type: command_succeeds
    command: |
      cd {workspace}/flowspec && cargo run -- analyze tests/fixtures/simple-project/ --output /tmp/flowspec-test-output.yaml 2>&1 && grep -q "math::add" /tmp/flowspec-test-output.yaml
    description: "Analyzer finds math::add in simple-project"
    stage: 3
    condition: "stage >= 3"

  # ── VALIDATION STAGE 4: Result file (supplementary context, not gate) ──
  - type: file_exists
    path: "{workspace}/flowspec/phase{stage}-result.md"
    description: "Result file exists for debugging"
    stage: 4
```

**Note on `condition` field:** The `condition` field controls which stages a validation applies to within a single phase. Use it to avoid running Phase 2-specific functional checks on Phase 1 stages (which wouldn't have the analyzer built yet). The `stage` field on validations controls fail-fast ordering (stage 1 validations run before stage 2, etc.).

### Result File Naming

Use `phase{N}-stage{stage}-result.md` to avoid collisions across phases:

```
In template:  {{ workspace }}/flowspec/phase1-stage{{ stage }}-result.md
In validation: {workspace}/flowspec/phase1-stage{stage}-result.md
```

Each phase uses its own prefix (phase1, phase2, etc.).

---

## SCORE FILE LOCATIONS

Write the concert scores to these paths:

```
/home/emzi/Projects/flowspec/workspace/concert/
├── CONCERT-README.md
├── flowspec-phase1-foundation.yaml
├── flowspec-phase2-lsp-client.yaml
├── flowspec-phase3-ast-enrichment.yaml
├── flowspec-phase4-diagnostics.yaml
└── flowspec-phase5-polish.yaml
```

The Flowspec Rust project lives at:

```
/home/emzi/Projects/flowspec/workspace/flowspec/
├── Cargo.toml
├── Cargo.lock                         # Committed (binary project)
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── cli/
│   ├── lsp/
│   ├── ast/
│   ├── analysis/
│   ├── manifest/
│   └── config/
├── tests/
│   ├── fixtures/
│   │   ├── simple-project/            # Known entities for validation (see above)
│   │   └── complex-project/           # Known diagnostics for validation (see above)
│   └── integration/
├── .flowspec/
│   └── config.yaml
└── README.md
```

---

## WRITING THE SCORES

For each phase, write a complete YAML score following the score authoring skill's principles. The checklist:

1. `name` field (descriptive, e.g., `"flowspec-phase1-foundation"`)
2. `description` field (human-readable summary of the phase)
3. `workspace: "/home/emzi/Projects/flowspec/workspace"` (absolute)
4. `sheet.size: 1` with `total_items` matching stage count
5. `sheet.dependencies` DAG — maximize parallelism for independent stages
6. `backend.type: claude_cli` with:
   - `skip_permissions: true`
   - `disable_mcp: true`
   - `timeout_seconds: 7200` (2 hours, with overrides for heavy stages)
   - `working_directory: "/home/emzi/Projects/flowspec/workspace/flowspec"`
7. `parallel.enabled: true` with `max_concurrent: 3`
8. `cross_sheet.auto_capture_stdout: true` with `lookback_sheets: 5` (full visibility within a phase)
9. `retry.max_retries: 3`, `retry.exponential_base: 2.0`, `retry.base_delay_seconds: 10.0`
10. `stale_detection.enabled: true` with `idle_timeout_seconds: 1800`
11. **Unfudgeable validations** — functional checks against fixture projects as primary gates, cargo test/clippy as secondary, result files as supplementary
12. Conditional validations using `condition` field
13. `prompt.variables.preamble` with full project context
14. `prompt.template` with `{% if stage == N %}` blocks — outcome-focused, contract-precise

### Concert Chaining

Phases 1-4 chain to the next phase:

```yaml
concert:
  enabled: true
  max_chain_depth: 5
  cooldown_between_jobs_seconds: 60

on_success:
  - type: run_job
    job_path: "/home/emzi/Projects/flowspec/workspace/concert/flowspec-phase{N+1}-{name}.yaml"
    detached: true
    fresh: false    # Preserve workspace across phases
```

Phase 5 has NO `on_success` chain — it's the final phase. It should also omit `concert.enabled`.

### Preamble Template

The preamble for each phase should include (adapt as phases progress):

```
You are building Flowspec — a static data flow analysis tool for Rust codebases
that produces AI-readable YAML manifests.

The full specification is at: {{ workspace }}/spec.md
Read it for details about manifest format, CLI interface, and configuration.

PROJECT LOCATION: {{ workspace }}/flowspec/
WORKING DIRECTORY: You are already in the flowspec project root.

RUST-ANALYZER REQUIREMENT:
This project uses rust-analyzer for LSP analysis. Verify it's available:
  which rust-analyzer || rustup component add rust-analyzer

CODING CONVENTIONS:
- thiserror for library errors, anyhow only in main.rs
- All public functions documented with /// doc comments
- Module structure: cli/, lsp/, ast/, analysis/, manifest/, config/
- Tests: tests/ (integration) and inline #[cfg(test)] mod tests (unit)
- tracing for logging, not println!/eprintln!
- Prefer &str over String in function parameters
- All LSP communication through lsp/ module only
- Stable Rust only — no nightly features
- Commit Cargo.lock

FIXTURE PROJECTS (test oracle — DO NOT MODIFY their structure):
- tests/fixtures/simple-project/ — known entities: math::add(i32,i32)->i32,
  math::multiply(i32,i32)->i32, greet::hello(&str)->String, main calls hello+add,
  multiply is dead code (never called)
- tests/fixtures/complex-project/ — known diagnostics: unused::orphaned_function
  and unused::DeadStruct are dead ends, model::Request derives Deserialize,
  model::Response derives Serialize, api→service module boundary,
  errors::AppError missing serialization error variant

WHAT EXISTS (update per phase):
[Describe what previous phases built — file locations, key types, interfaces]

WHAT YOU'RE BUILDING:
[Phase-specific description]

RESULT FILE:
After completing your work, write {{ workspace }}/flowspec/phase{N}-stage{{ stage }}-result.md containing:
- IMPLEMENTATION_COMPLETE: yes/no
- FILES_CREATED: [list of files you created or modified]
- ISSUES_ENCOUNTERED: [any escalation triggers hit, or "none"]
- DECISIONS_MADE: [any architectural decisions not specified in the brief]
Note: this file is for debugging and cross-sheet context, not the primary validation.
```

Replace `{N}` with the phase number (1-5) in each phase's preamble.

### Stage Instruction Quality

Each stage's `{% if stage == N %}` block should provide:

- **Clear outcome** — what should be different when this stage completes?
- **Context about what exists** — files, types, interfaces from prior stages
- **Interface contracts** — key type signatures, trait definitions, module APIs
- **Quality criteria** — what the validations will check (so the agent aims right)
- **Constraints** — what must NOT happen (error handling strategy, no unsafe, etc.)

Don't prescribe workflow (which tools to use, what order to read files). DO prescribe contracts (function signatures, error types, module structure).

**Good example:**
```
## Stage 2: LSP Client Core

Build the LSP client that communicates with rust-analyzer.

**What should exist when this stage completes:**
- src/lsp/client.rs — an async LSP client that starts rust-analyzer,
  initializes the LSP connection, sends requests, and shuts down cleanly
- src/lsp/error.rs — typed error handling (thiserror)
- src/lsp/mod.rs — public module interface
- Tests proving: start rust-analyzer against tests/fixtures/simple-project/,
  request document symbols, verify at least one symbol returned, shutdown cleanly

**Key interface (the agent can expand but must support at minimum):**
- Start rust-analyzer as child process given a project root path
- LSP initialize/shutdown lifecycle
- textDocument/documentSymbol
- textDocument/references
- textDocument/definition
- callHierarchy/incomingCalls and outgoingCalls

Use lsp-types crate for all LSP protocol types. Communication is via
stdin/stdout JSON-RPC with the rust-analyzer process.

**Constraints:**
- Timeout on all LSP requests (configurable, default 30s)
- Clean shutdown — don't leave rust-analyzer zombie processes
- thiserror for error types

**What the validation will check:**
- cargo test passes (including the fixture project test)
- cargo clippy clean
- flowspec analyze tests/fixtures/simple-project/ produces output containing "math::add"
```

### CONCERT-README.md Contents

The CONCERT-README.md should contain:
- What Flowspec is (one paragraph)
- Prerequisites: Rust stable toolchain, rust-analyzer
- Pre-concert setup commands (mkdir workspace, copy spec.md)
- How to start: `mozart start && mozart run /home/emzi/Projects/flowspec/workspace/concert/flowspec-phase1-foundation.yaml`
- How to monitor: `mozart status flowspec-phase1-foundation`
- Phase overview with expected duration estimates per phase
- How to resume if a phase fails: `mozart resume <job-id> --workspace /home/emzi/Projects/flowspec/workspace`
- Where outputs end up

---

## OUTPUT

Write the complete concert: CONCERT-README.md + 5 phase YAML files.

Each YAML file must:

1. Be valid YAML
2. Use correct Mozart field names per the score authoring skill
3. Use `stage` (not `sheet_num`) for template conditionals
4. Use `{workspace}` in validations, `{{ workspace }}` in templates
5. Have `skip_permissions: true` and `disable_mcp: true`
6. Have `working_directory` pointing to the flowspec project root
7. Have `parallel.enabled: true` where stages can run concurrently
8. Have `lookback_sheets: 5` (full phase visibility)
9. Have unfudgeable validations — functional checks against fixture projects with known properties
10. Use absolute paths for `on_success.job_path`
11. Use `fresh: false` for phase-to-phase chaining
12. Have `stale_detection.idle_timeout_seconds: 1800`
13. Have `description` field on each score

After writing all files:
1. Validate each YAML file parses with `python3 -c "import yaml; yaml.safe_load(open('file.yaml'))"`
2. Verify `on_success` chains: phase1 → phase2 → phase3 → phase4 → phase5 (no chain on phase5)
3. Verify dependency DAGs are acyclic
4. Count total stages: 5+5+5+5+4 = 24
5. Verify all `job_path` values are absolute and correct
6. Verify all validation paths use `{workspace}` not `{{ workspace }}`
7. Verify all prompt templates use `{{ workspace }}` not `{workspace}`
8. Verify functional validations reference specific known properties from fixture projects

This concert, when run with `mozart start && mozart run /home/emzi/Projects/flowspec/workspace/concert/flowspec-phase1-foundation.yaml`, should produce a working Flowspec tool at the end of the chain.
