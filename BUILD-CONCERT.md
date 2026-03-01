# TASK: Build a Mozart Concert to Implement Flowspec

You are building a complete Mozart AI Compose concert — a set of YAML score files that, when executed by Mozart, will produce a working Flowspec tool. Flowspec is a standalone Rust CLI tool that produces structured, AI-readable data flow manifests for codebases.

Before writing ANY scores, you MUST:

1. Read the Mozart score authoring skill at `/home/emzi/.claude/skills/mozart-score-authoring.md` — this is the definitive guide for how to write scores. **Read it completely.** It covers syntax, validation engineering, prompt design, Jinja mastery, fan-out architecture, and common pitfalls.
2. Read ALL example scores in `~/Projects/mozart-ai-compose/examples/` — understand the patterns (especially `quality-continuous.yaml`, `parallel-research-fanout.yaml`, and `prelude-cadenza-example.yaml` for advanced patterns)
3. Read scores in `~/Projects/claude-compositions/scores/` — more examples of fan-out, synthesis, and creative score design
4. Read the CLAUDE.md at `~/Projects/mozart-ai-compose/CLAUDE.md` — project instructions including how to run Mozart
5. Read the configuration reference at `~/Projects/mozart-ai-compose/docs/configuration-reference.md` — every config field documented
6. Read the Flowspec spec at `~/Projects/flowspec/spec.md` — the specification for what you're building

Study these sources. Understand how Mozart actually works from the source of truth. Then write scores that follow the skill's principles — particularly "Scores are programs for minds, not machines" (describe outcomes, not keystrokes) and the validation engineering section (outcome validations, not process validations).

---

## WHAT IS MOZART AI COMPOSE

Mozart orchestrates long-running AI coding sessions by breaking work into stages — atomic units of work that an LLM agent (Claude Code via CLI) executes. Stages can run sequentially or in parallel based on a dependency DAG. Stages can fan out into multiple parallel instances for independent perspectives, then fan back in for synthesis.

### Score Anatomy

A score is a YAML file with these top-level sections. **Every field shown here is real and verified against Mozart's actual config schema.**

```yaml
name: "job-name"                       # Unique identifier for this job
workspace: "/absolute/path/to/workspace"  # REQUIRED: Output directory (must be absolute)

sheet:
  size: 1                              # Items per sheet (1 = each stage is unique)
  total_items: N                       # Total number of stages
  dependencies:                        # DAG — which stages must complete before which
    2: [1]                             # Stage 2 depends on stage 1
    3: [1]                             # Stage 3 depends on stage 1 (parallel with 2)
    4: [2, 3]                          # Stage 4 depends on both 2 and 3
  # fan_out:                           # Optional: fan-out for parallel instances
  #   2: 3                             # Stage 2 → 3 parallel instances
  # prelude:                           # Optional: files injected into ALL stages
  #   - file: "{{ workspace }}/shared-context.md"
  #     as: context                    # context | skill | tool
  # cadenzas:                          # Optional: per-stage file injections
  #   1:
  #     - file: "{{ workspace }}/setup.md"
  #       as: skill

backend:
  type: claude_cli                     # Execution backend
  skip_permissions: true               # REQUIRED for unattended execution
  disable_mcp: true                    # ~2x speedup, prevents contention
  timeout_seconds: 7200                # Max time per stage (2 hours)
  timeout_overrides:                   # Per-stage timeout overrides
    3: 10800                           # Stage 3 gets extra time (3 hours)
  allowed_tools:                       # What tools the Claude Code instance can use
    - Read
    - Write
    - Edit
    - Grep
    - Glob
    - Bash
    - TodoRead
    - TodoWrite

parallel:
  enabled: true                        # Enable parallel execution
  max_concurrent: 3                    # Max simultaneous stages
  fail_fast: true                      # Stop on first failure

cross_sheet:
  auto_capture_stdout: true            # Capture output for subsequent stages
  max_output_chars: 3000               # Per-stage truncation limit
  lookback_sheets: 3                   # How many previous stages' output to include
  # capture_files:                     # Optional: capture specific files
  #   - "{{ workspace }}/stage-{{ sheet_num - 1 }}-result.md"

retry:
  max_retries: 3                       # Total retry budget per stage
  base_delay_seconds: 10.0             # Initial backoff delay
  max_delay_seconds: 3600.0            # Max backoff (1 hour cap)
  exponential_base: 2.0                # Backoff multiplier
  jitter: true                         # Randomize delays
  max_completion_attempts: 3           # If >50% validations pass, retry with focused prompt
  completion_threshold_percent: 50.0   # % passing to trigger completion mode

rate_limit:
  wait_minutes: 60                     # Wait time on rate limit detection
  max_waits: 24                        # Max wait cycles (24 hours at 60min/wait)

circuit_breaker:
  enabled: true                        # Prevent cascade failures
  failure_threshold: 5                 # Consecutive failures before circuit opens
  recovery_timeout_seconds: 300        # Wait before retrying after circuit opens

stale_detection:
  enabled: true                        # Detect hung stages
  idle_timeout_seconds: 1800           # 30min — safe for cargo builds/tests
  check_interval_seconds: 30           # How often to check

validations:                           # Acceptance criteria — how Mozart knows work is done
  # CRITICAL: Validation paths use {workspace} NOT {{ workspace }}
  # This is a different template system from the prompt template (Python .format() vs Jinja2)

  - type: command_succeeds             # Run a command, pass if exit code 0
    command: 'cd {workspace}/flowspec && cargo test --all 2>&1'
    description: "All tests pass"
    stage: 1                           # Validation stage (fail-fast between stages)

  - type: command_succeeds
    command: 'cd {workspace}/flowspec && cargo clippy -- -D warnings 2>&1'
    description: "Clippy clean"
    stage: 1

  - type: file_exists                  # Check a file was created
    path: "{workspace}/flowspec/stage{sheet_num}-result.md"
    description: "Result file exists"
    stage: 2

  - type: content_contains             # Check file contains expected content
    path: "{workspace}/flowspec/stage{sheet_num}-result.md"
    pattern: "IMPLEMENTATION_COMPLETE: yes"
    description: "Stage reports completion"
    stage: 2

  # Conditional validations — only run for specific stages
  - type: command_succeeds
    command: '{workspace}/flowspec/target/debug/flowspec --help'
    description: "CLI is functional"
    condition: "stage >= 2"            # Only after CLI exists
    stage: 1

prompt:
  variables:                           # Reusable variables available in template
    preamble: |
      Shared context that every stage sees.
      Project description, conventions, constraints.

  template: |                          # Jinja2 template — the actual prompt sent to the agent
    {{ preamble }}

    {% if stage == 1 %}
    STAGE 1: [Title]

    [Detailed instructions for stage 1]

    {% elif stage == 2 %}
    STAGE 2: [Title]

    [Detailed instructions for stage 2]

    {% endif %}

    ## Result File
    Write {{ workspace }}/flowspec/stage{{ stage }}-result.md containing:
    - IMPLEMENTATION_COMPLETE: yes/no
    - FILES_CREATED: [list]
    - ISSUES_ENCOUNTERED: [any escalation triggers, or "none"]

# Concert chaining to next phase
concert:
  enabled: true
  max_chain_depth: 5                   # Safety limit for chain depth
  cooldown_between_jobs_seconds: 60    # Pause between phases

on_success:
  - type: run_job
    job_path: "/absolute/path/to/next-phase.yaml"  # MUST be absolute
    detached: true                     # Route through daemon
    fresh: false                       # CRITICAL: Do NOT use fresh:true between phases
                                       # fresh:true archives/clears workspace, destroying
                                       # code built by previous phases
```

### Critical Score Writing Principles

**1. The prompt template is the specification, but specify OUTCOMES, not STEPS.** The agent starts with zero context — no previous conversation, no memory. The prompt IS the complete brief. But describe what should be different when the stage completes, not the keystrokes to get there. The agent is a reasoning, planning entity — it knows its tools. Give it goals, constraints, and quality criteria.

**2. Validations are outcome-based acceptance criteria.** Mozart doesn't trust the agent's self-report. Validation gates check that the work is actually done. The pattern is: `command_succeeds` (tests pass, build succeeds, lint clean) for strong verification, `file_exists` and `content_contains`/`content_regex` for structural checks. **For every goal in your prompt, ask: "Can the agent pass all my validations without achieving this goal?" If yes, your validations are decorative.** See the score-authoring skill's "Process validations vs outcome validations" section.

**3. Use `stage` not `sheet_num` for template conditionals.** After fan-out expansion, `sheet_num` changes but `stage` remains stable. Even without fan-out, `stage` is the conceptually correct variable — it represents the logical stage of work.

**4. Two template systems — don't mix them up.** Prompt templates use Jinja2 (`{{ workspace }}`). Validation paths and commands use Python format strings (`{workspace}`). This is the #1 source of broken configs.

**5. Cross-stage context is limited.** `lookback_sheets: 3` means each stage sees stdout from the 3 most recent prior stages. Don't rely on an agent "remembering" stage 1's output when it's working on stage 8. If stage 8 needs information from stage 1, put it in the preamble variable or in a file the agent can read.

**6. Dependencies control parallelism and ordering.** If stage 3 depends on stage 1 and stage 2, it won't run until both complete. If stages 2 and 3 both only depend on 1, they run in parallel (up to `parallel.max_concurrent`). Design your DAG to maximize parallelism where work is independent. **But**: you must also set `parallel.enabled: true`.

**7. The workspace is shared memory.** Files in `{{ workspace }}` are how stages communicate beyond `previous_outputs`. Write structured output (markdown with consistent headers, JSON) so downstream stages can parse reliably.

**8. Timeout overrides for dense stages.** Some stages are heavier than others. Use `backend.timeout_overrides` to give complex stages more time rather than increasing the global timeout.

**9. The template uses Jinja2.** Available variables: `stage`, `sheet_num`, `start_item`, `end_item`, `total_sheets`, `instance`, `fan_count`, `total_stages`, `workspace`, plus any variables defined in `prompt.variables`. Use `{% if stage == N %}` blocks to give each stage unique instructions.

**10. Write prompts as if briefing a senior developer who just joined the project today.** They're skilled but have zero context. Be explicit about what exists, what to create, coding conventions, constraints. Don't assume they'll infer what you want. But DON'T micromanage — they know how to use their tools.

**11. `skip_permissions: true` and `disable_mcp: true` are required for unattended execution.** Without `skip_permissions`, the agent prompts for permission and hangs. Without `disable_mcp`, MCP server startup adds latency and can cause contention.

### Template Variables Reference

| Variable | Type | Description | When Available |
|---|---|---|---|
| `stage` | int | Logical stage (1-indexed, stable) | Always |
| `sheet_num` | int | Physical sheet number (changes with fan-out) | Always |
| `total_sheets` | int | Total sheets after expansion | Always |
| `total_stages` | int | Total logical stages | Always |
| `start_item` | int | First item for this sheet | Always |
| `end_item` | int | Last item for this sheet | Always |
| `workspace` | str | Absolute workspace path | Always |
| `instance` | int | Instance within fan-out group (1-indexed) | Always (1 without fan-out) |
| `fan_count` | int | Total instances in this stage | Always (1 without fan-out) |
| `previous_outputs` | dict[int, str] | Stdout from previous sheets | When `cross_sheet.auto_capture_stdout` |
| `previous_files` | dict[str, str] | Captured file contents | When `cross_sheet.capture_files` |
| (user variables) | any | From `prompt.variables` | Always |

### Common Score Writing Mistakes

These are real issues found in existing scores. Learn from them:

| # | Mistake | Fix |
|---|---|---|
| 1 | `{{ workspace }}` in validation paths | Use `{workspace}` — validations use Python `.format()`, not Jinja2 |
| 2 | `{workspace}` in prompt templates | Use `{{ workspace }}` — prompts use Jinja2 |
| 3 | No `skip_permissions: true` | Agent hangs waiting for permission prompts |
| 4 | No `disable_mcp: true` | Slow startup, potential contention |
| 5 | Using `sheet_num` for conditionals | Use `stage` — stable across fan-out expansion |
| 6 | Missing `parallel.enabled: true` | Stages run sequentially even with dependencies |
| 7 | `fresh: true` between different phases | Destroys code built by previous phases |
| 8 | Relative paths for `job_path` | Must be absolute — relative paths resolve from daemon CWD |
| 9 | Prescriptive step-by-step instructions | Describe outcomes — the agent reasons and adapts |
| 10 | File-existence-only validations | Combine with content checks and `command_succeeds` for real verification |
| 11 | `max_attempts` (wrong field name) | Use `max_retries` |
| 12 | `backoff_multiplier` (wrong field name) | Use `exponential_base` |
| 13 | Self-report markers as primary validation | `cargo test` is a stronger signal than `IMPLEMENTATION_COMPLETE: yes` |
| 14 | Stale detection timeout too short | Use `idle_timeout_seconds: 1800` for stages with cargo builds |

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

AI coding tools operate on codebases at the text level. They read files as strings. They don't have a structural map of how data flows through a system. When a human developer joins a team, they spend weeks building a mental model of data flow. Flowspec generates that mental model as a structured artifact.

The manifest is designed to be consumed by AI tools (Mozart's planner, Claude Code, any LLM-based coding tool) to provide structural understanding of a codebase without requiring the AI to re-derive it from source files.

### Architecture

```
┌─────────────────────┐
│   Flowspec CLI       │
│                      │
│  flowspec analyze    │   ← Main command: full analysis
│  flowspec diagnose   │   ← Diagnostics only
│  flowspec diff       │   ← Compare two manifests
│  flowspec trace      │   ← Follow one flow path
│  flowspec lint       │   ← Check architectural rules
│  flowspec watch      │   ← Incremental re-analysis
└──────────┬───────────┘
           │
┌──────────▼───────────┐
│    Orchestrator       │   ← Detects project language, starts LSP, controls crawl
└──────────┬───────────┘
           │
     ┌─────┼──────────────────┐
     │     │                  │
┌────▼──┐ ┌▼────────┐  ┌─────▼─────┐
│rust-   │ │pyright  │  │typescript │   ← Language servers (LSP protocol)
│analyzer│ │         │  │-language- │
│        │ │         │  │server     │
└────┬───┘ └────┬────┘  └─────┬─────┘
     │          │             │
┌────▼──────────▼─────────────▼───┐
│      AST Enrichment Layer        │   ← Beyond LSP: pattern detection,
│                                  │     serialization boundaries,
│  - tree-sitter parsing          │     error path analysis,
│  - serde/derive detection       │     convention inference
│  - error chain analysis         │
│  - naming convention inference  │
└─────────────┬────────────────────┘
              │
      ┌───────▼──────────┐
      │  Manifest Writer  │   ← Outputs YAML/JSON/summary
      │                   │
      │  - Full manifest  │
      │  - Diagnostics    │
      │  - Diff support   │
      └───────────────────┘
```

### Language Support (Phase 1: Rust Only)

Phase 1 implements Rust analysis only (via rust-analyzer LSP + tree-sitter). This is both the most immediately useful and the language with the richest type information for static analysis.

Future phases would add Python (pyright), TypeScript (tsserver), Go (gopls).

### CLI Interface

```bash
# Full analysis
flowspec analyze ./my-project --output manifest.yaml

# Diagnostics only
flowspec diagnose ./my-project --checks dead-ends,orphans,duplications

# Diff two manifests
flowspec diff before.yaml after.yaml

# Trace one flow
flowspec trace ./my-project --from "api::handlers::search::handle_search"

# Architectural lint
flowspec lint ./my-project --rules .flowspec/rules.yaml

# Watch mode (incremental)
flowspec watch ./my-project --output manifest.yaml
```

### Configuration

```yaml
# .flowspec/config.yaml
project:
  name: "my-project"
  languages: ["rust"]

analysis:
  entry_points:
    - "src/main.rs::main"
    - "src/api/**::handle_*"
  ignore:
    - "target/"
    - "tests/fixtures/"
  max_call_depth: 20
  max_type_chain: 10

diagnostics:
  enabled:
    - dead_ends
    - orphan_consumers
    - duplications
    - contract_mismatches
    - missing_error_paths
    - circular_dependencies
    - unreachable_code
    - layer_violations

  layer_rules:
    - name: "api should not access database directly"
      from: "api::**"
      to: "sqlx::*"
      allowed_through: ["repository::*"]

  suppressions:
    - entity: "src/legacy.rs::OldEngine"
      diagnostic: "dead_end"
      reason: "Kept for migration rollback"
      expires: "2026-06-01"
```

### Output Format (Manifest)

See `spec.md` in this repository for the complete manifest format specification, including examples of all six sections: entity registry, flow paths, boundary map, diagnostics, dependency graph, and type flow matrix.

---

## CONSTRAINTS

### Musts
- Rust, using Cargo. Single binary, no runtime dependencies beyond what Cargo provides.
- Use `lsp-types` crate for LSP protocol types. For the LSP client transport, use raw JSON-RPC over stdin/stdout (the `lsp-server` crate provides helpers, or build a thin async client with `tokio` + `serde_json`). **Note: `tower-lsp` is an LSP *server* framework, not a client. Do not use it for the client side.**
- Use `tree-sitter` with `tree-sitter-rust` for AST enrichment beyond LSP
- Use `clap` for CLI argument parsing
- Use `serde` + `serde_yaml` for manifest serialization
- All output is valid YAML that can be parsed by any YAML library
- The tool must work on any Rust project with a valid Cargo.toml
- Every public function must have documentation
- Error handling must use typed errors (thiserror), not anyhow for library code (anyhow is fine for CLI main)
- Integration tests must use real Rust projects (small fixture projects in tests/fixtures/)

### Must Nots
- Do NOT use nightly Rust features. Stable toolchain only.
- Do NOT require the user to install rust-analyzer separately — download or locate it automatically, or use it as a library dependency if feasible. **Escalation trigger: if embedding/auto-downloading proves too complex, document the blocker and require rust-analyzer on PATH as a fallback.**
- Do NOT produce manifests larger than 10x the source code size (summarize, don't dump)
- Do NOT attempt to analyze generated code (target/, build artifacts)
- Do NOT use `unsafe` unless absolutely required for FFI

### Preferences
- Prefer streaming analysis over loading everything into memory (large codebases)
- Prefer tree-sitter over syn for initial parsing (tree-sitter is error-tolerant, syn requires valid Rust)
- Prefer explicit module structure over deeply nested files
- Prefer integration tests that test the full pipeline over unit tests of internals
- Prefer YAML output over JSON (more readable for humans and AI)

### Escalation Triggers (mark these in result files if encountered)
- If rust-analyzer LSP proves too complex to embed, document the blocker and suggest alternative approaches
- If tree-sitter-rust grammar doesn't cover a needed syntactic construct, document what's missing
- If the manifest format needs to change from what's specified here, document why and propose the change
- If a diagnostic type proves infeasible to implement with static analysis alone, mark it as "requires dynamic analysis" and stub it

---

## CONCERT STRUCTURE

This concert has 5 phases. Each phase is a separate score file. Phases chain automatically via `on_success` hooks.

**CRITICAL: Concert chaining between phases must use `fresh: false` (the default).** Using `fresh: true` would archive/clear the workspace, destroying the Flowspec Rust project built by previous phases. `fresh: true` is only for self-chaining (same score repeating). For a multi-phase build where each phase extends the previous phase's work, the workspace must persist.

### Phase 1: Foundation (5 stages)
Project scaffolding, core types, CLI skeleton, configuration parsing, manifest serialization.
After this phase: `flowspec --help` works, config files parse, manifest types exist and serialize to valid YAML.

### Phase 2: LSP Client (5 stages)
LSP client implementation — connect to rust-analyzer, send requests, parse responses, build entity registry and call graphs from LSP data.
After this phase: `flowspec analyze ./test-project` produces an entity registry with functions, structs, traits, and their relationships.

### Phase 3: AST Enrichment (5 stages)
Tree-sitter integration — parse Rust files for information LSP doesn't provide: serialization boundaries, error chain analysis, derive macro detection, naming convention inference.
After this phase: the manifest includes boundary detection, error path analysis, and type flow tracking beyond what LSP gives.

### Phase 4: Diagnostics Engine (5 stages)
The diagnostic analysis layer — dead end detection, orphan consumers, duplications, contract mismatches, missing error paths, circular dependencies, layer violations.
After this phase: `flowspec diagnose ./test-project` produces actionable diagnostics.

### Phase 5: Diff, Watch, and Polish (4 stages)
Manifest diffing, incremental watch mode, integration tests against real projects, documentation, release preparation.
After this phase: the tool is complete, tested, documented, and ready to use.

---

## ACCEPTANCE CRITERIA PER PHASE

### Phase 1: Foundation
- `cargo build` succeeds with no warnings
- `cargo test` passes all tests
- `cargo clippy -- -D warnings` is clean
- `flowspec --help` prints usage
- `flowspec --version` prints version
- `.flowspec/config.yaml` parsing works with all fields
- Manifest types serialize to valid YAML matching the format spec
- A round-trip test exists: create manifest in code → serialize to YAML → deserialize → compare

### Phase 2: LSP Client
- rust-analyzer starts and connects via LSP
- Entity registry populates with functions, structs, traits, impls
- Call hierarchy (incoming + outgoing) is captured
- References and definitions are resolved
- `flowspec analyze tests/fixtures/simple-project/` produces a manifest with entities
- LSP server is cleanly shut down after analysis
- Timeout handling for LSP requests that hang

### Phase 3: AST Enrichment
- Tree-sitter parses all .rs files in a project
- Serde derive macros detected → serialization boundaries identified
- Error type chains traced (Result → map_err → ? → handler)
- Type flow matrix populated (created_at, transformed_to, consumed_by)
- Boundary map includes module, crate, and network boundaries
- Flow paths traced from entry points through the system

### Phase 4: Diagnostics
- Dead end detection: finds entities with zero consumers
- Orphan consumer detection: finds consumers with no producers
- Duplication detection: finds overlapping logic in different locations
- Missing error path detection: finds error types not handled at boundaries
- Circular dependency detection: finds cycles in module graph
- Layer violation detection: checks architectural rules from config
- All diagnostics include severity, evidence, and suggestions
- `flowspec diagnose` with `--checks` flag filters diagnostic types

### Phase 5: Diff, Watch, Polish
- `flowspec diff a.yaml b.yaml` shows structural changes between manifests
- `flowspec watch` re-analyzes on file changes (inotify/fswatch)
- Integration tests pass against 2+ real (fixture) Rust projects
- README.md with usage, examples, and architecture description
- All public APIs documented
- `cargo clippy -- -D warnings` clean, `cargo fmt --check` clean
- CI workflow file (GitHub Actions) for test + clippy + fmt

---

## SCORE FILE LOCATIONS

Write the concert scores to these paths:

```
{workspace}/concert/
├── CONCERT-README.md                  # Overview of the concert, how to run it
├── flowspec-phase1-foundation.yaml
├── flowspec-phase2-lsp-client.yaml
├── flowspec-phase3-ast-enrichment.yaml
├── flowspec-phase4-diagnostics.yaml
└── flowspec-phase5-polish.yaml
```

The workspace for Flowspec itself (where the Rust code lives) is at `{workspace}/flowspec/`:

```
{workspace}/flowspec/                  # The actual Rust project
├── Cargo.toml
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
│   │   ├── simple-project/            # Minimal Rust project for testing
│   │   └── complex-project/           # Multi-module project for testing
│   └── integration/
├── .flowspec/
│   └── config.yaml
└── README.md
```

---

## WRITING THE SCORES

For each phase, write a complete YAML score file following the anatomy described above and, more importantly, following the principles in the score-authoring skill. Each score must have:

1. A descriptive `name` field
2. A `workspace` field with an absolute path (use `/home/emzi/Projects/flowspec/workspace` or let the user override)
3. `sheet.size: 1` with `total_items` matching the number of stages
4. A `dependencies` DAG that maximizes parallelism where stages are independent
5. `backend.type: claude_cli` with `skip_permissions: true`, `disable_mcp: true`, and appropriate `timeout_seconds` (3600-7200)
6. `parallel.enabled: true` with `max_concurrent: 3`
7. `cross_sheet.auto_capture_stdout: true` with `lookback_sheets: 3`
8. `retry` with correct field names: `max_retries: 3`, `exponential_base: 2.0`, `base_delay_seconds: 10.0`
9. `stale_detection.enabled: true` with `idle_timeout_seconds: 1800` (cargo builds are slow)
10. Validations that prioritize **outcome verification**: `cargo test`, `cargo clippy`, functional checks (CLI runs, produces output), with self-report markers as secondary
11. Conditional validations using `condition` field where stages need different checks
12. A `prompt.variables.preamble` that contains ALL project context — what Flowspec is, project structure, coding conventions, constraints
13. A `prompt.template` with `{% if stage == N %}` blocks giving each stage specific instructions focused on outcomes, not steps

For scores that are part of the concert chain, include:

```yaml
concert:
  enabled: true
  max_chain_depth: 5
  cooldown_between_jobs_seconds: 60

on_success:
  - type: run_job
    job_path: "/home/emzi/Projects/flowspec/workspace/concert/next-phase.yaml"  # ABSOLUTE path
    detached: true
    fresh: false    # CRITICAL: preserve workspace across phases
```

The **last phase** (phase 5) should NOT have `on_success` chaining — it's the final phase.

### Validation Strategy

**Primary validations (outcome-based):**
- `cargo test --all` — tests pass
- `cargo clippy -- -D warnings` — lint clean
- `cargo build` — builds successfully
- Functional tests — run the CLI and verify output

**Secondary validations (structural):**
- Result file exists with completion markers
- Specific output files exist
- Content checks on generated artifacts

**Conditional validations by stage:**
```yaml
validations:
  # Stage 1: Build compiles
  - type: command_succeeds
    command: 'cd {workspace}/flowspec && cargo build 2>&1'
    description: "Project builds"
    stage: 1

  # Stage 2: Tests pass (only after test infrastructure exists)
  - type: command_succeeds
    command: 'cd {workspace}/flowspec && cargo test --all 2>&1'
    description: "All tests pass"
    stage: 1
    condition: "stage >= 2"

  # Stage 2: Clippy clean
  - type: command_succeeds
    command: 'cd {workspace}/flowspec && cargo clippy -- -D warnings 2>&1'
    description: "Clippy clean"
    stage: 1
    condition: "stage >= 2"

  # All stages: result file
  - type: file_exists
    path: "{workspace}/flowspec/stage{stage}-result.md"
    description: "Result file exists"
    stage: 2

  - type: content_contains
    path: "{workspace}/flowspec/stage{stage}-result.md"
    pattern: "IMPLEMENTATION_COMPLETE: yes"
    description: "Stage reports completion"
    stage: 2
```

**Note on validation variable expansion:** In validation `path` and `command` fields, available variables are: `{workspace}`, `{sheet_num}`, `{start_item}`, `{end_item}`, and (with fan-out) `{stage}`, `{instance}`. User-defined variables from `prompt.variables` are NOT available in validations. Use `command_succeeds` for complex validation logic.

### Preamble Template

The preamble for each phase should include (adapt as phases progress):

```
You are building Flowspec — a static data flow analysis tool for Rust codebases
that produces AI-readable YAML manifests.

The full specification is at: {{ workspace }}/spec.md
Read it if you need details about manifest format, CLI interface, or configuration.

PROJECT LOCATION: {{ workspace }}/flowspec/

CODING CONVENTIONS:
- Typed errors with thiserror for library code, anyhow for main.rs only
- All public functions documented with /// doc comments
- Module structure mirrors the architecture: cli/, lsp/, ast/, analysis/, manifest/, config/
- Tests go in tests/ (integration) and inline #[cfg(test)] mod tests (unit)
- Use `tracing` for logging, not println! or eprintln!
- Prefer &str over String in function parameters where possible
- All LSP communication goes through the lsp/ module — no direct LSP calls elsewhere
- Stable Rust only — no nightly features

WHAT EXISTS (update per phase):
[Describe what previous phases built — file locations, key types, interfaces]

WHAT YOU'RE BUILDING:
[Phase-specific description]

RESULT FILE:
After completing your work, write {{ workspace }}/flowspec/stage{{ stage }}-result.md containing:
- IMPLEMENTATION_COMPLETE: yes/no
- TESTS_PASS: yes/no
- CLIPPY_CLEAN: yes/no
- FILES_CREATED: [list of files you created or modified]
- ISSUES_ENCOUNTERED: [any escalation triggers hit, or "none"]
- DECISIONS_MADE: [any architectural decisions not specified in the brief]
```

### Stage Instruction Quality

Each stage's instructions (inside the `{% if stage == N %}` block) should give a senior developer with zero project context everything they need to succeed. This means:

- **Clear outcome description** — what should be different when this stage completes?
- **Context about what exists** — what files, types, interfaces from prior stages?
- **Constraints and quality criteria** — what must be true about the output?
- **Output specification** — where should artifacts be written?
- **Key interfaces to implement** — signatures for critical types (but don't micromanage the agent's workflow)

**Good:**
```
## Stage 2: LSP Client Core

Build the LSP client that communicates with rust-analyzer.

**What should exist when this stage completes:**
- `src/lsp/client.rs` — an async LSP client that can start rust-analyzer,
  initialize the LSP connection, send requests, and shut down cleanly
- `src/lsp/error.rs` — typed error handling with variants for connection,
  timeout, invalid response, and server errors
- `src/lsp/mod.rs` — public module interface
- Tests that verify: start rust-analyzer against tests/fixtures/simple-project/,
  request document symbols, verify at least one symbol returned, shutdown cleanly

**Key interface:**
The client should expose at minimum:
- Starting rust-analyzer as a child process
- LSP initialize/shutdown lifecycle
- textDocument/documentSymbol
- textDocument/references
- textDocument/definition
- callHierarchy/incomingCalls and outgoingCalls

Use `lsp-types` crate for all LSP protocol types. Communication is via
stdin/stdout JSON-RPC with the rust-analyzer process.

**Constraints:**
- Timeout on all LSP requests (configurable, default 30s)
- Clean shutdown — don't leave rust-analyzer zombies
- thiserror for error types, not anyhow

**Prior work (from Stage 1):**
Read src/lib.rs and src/config/ to understand the project structure and
config types. The Config struct has an `analysis` field with `entry_points`
and `ignore` patterns.
```

**Bad:**
```
Implement the LSP client.
```

---

## OUTPUT

Write the complete concert: CONCERT-README.md + 5 phase YAML files. Each YAML file must:

1. Be valid YAML (the `prompt.template` field contains Jinja2 which is just a string to the YAML parser)
2. Use correct Mozart field names (verified against the schema above)
3. Use `stage` (not `sheet_num`) for template conditionals
4. Use `{workspace}` in validations, `{{ workspace }}` in templates
5. Have `skip_permissions: true` and `disable_mcp: true`
6. Have `parallel.enabled: true` where stages can run concurrently
7. Have outcome-based validations (cargo test/clippy/build, not just marker files)
8. Have conditional validations where appropriate
9. Use absolute paths for `on_success.job_path`
10. Use `fresh: false` for phase-to-phase chaining (preserve workspace)
11. Have `stale_detection.idle_timeout_seconds: 1800` (cargo builds need time)

After writing all files:
1. Validate each YAML file parses correctly with `python3 -c "import yaml; yaml.safe_load(open('file.yaml'))"`
2. Verify `on_success` chains: phase1 → phase2 → phase3 → phase4 → phase5 (no chain on phase5)
3. Verify dependency DAGs within each phase are acyclic and sensible
4. Count total stages across all phases: 5+5+5+5+4 = 24 stages
5. Copy the flowspec spec.md into the workspace so agents can reference it

This concert, when run with `mozart start && mozart run flowspec-phase1-foundation.yaml`, should produce a working Flowspec tool at the end of the chain.
