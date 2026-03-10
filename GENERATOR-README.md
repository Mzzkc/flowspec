# Flowspec Build Score Generator

Generates a Mozart score for building Flowspec using an iterative development loop with fan-out agents.

## Files

- `generate-iterative-dev-loop.py` — Project-agnostic score generator (copied from Company-Simulator-9000)
- `generator-config.yaml` — Flowspec-specific configuration for the generator
- `score.yaml` — Generated output (run the generator to produce this)

## Prerequisites

- Python 3.10+ with PyYAML (`pip install pyyaml`)
- Mozart AI Compose installed and configured
- Flowspec spec corpus at `.flowspec/spec/` (9 YAML files)

## Usage

### Dry Run (stats only)

```bash
python generate-iterative-dev-loop.py generator-config.yaml --dry-run
```

### Generate Score

```bash
python generate-iterative-dev-loop.py generator-config.yaml -o score.yaml
```

### Generate with Fewer Cycles (testing)

```bash
python generate-iterative-dev-loop.py generator-config.yaml -o score.yaml --cycles 5
```

### Run the Score

```bash
mozart run score.yaml
```

## Team Composition

| Role | Count | Names | Responsibility |
|------|-------|-------|----------------|
| Executive | 1 | VISION | Full spec compliance gatekeeper |
| Manager | 1 | Architect | Pipeline sequencing, quality enforcement |
| Workers | 3 | Foundry, Sentinel, Interface | Parser/graph, analyzer, CLI/manifest |
| QA | 3 | QA-Foundation, QA-Analysis, QA-Surface | 1:1 paired with workers (TDD) |
| Docs | 2 | Doc-API, Doc-Usage | Internal reference, user-facing guides |
| Reviewers | 5 | COMP/SCI/CULT/EXP/META | TDF 5-way independent review |
| Antagonists | 2 | Newcomer, Static Analysis Expert | Blind adversarial validation |
| Dreamers | 5 | Workers/QA/Leadership/Reviewers/Collective | Memory consolidation |

### Worker Specializations

- **Foundry** (Worker 1): Tree-sitter integration, language adapters (Python/JS/Rust), IR design, persistent graph, cache serialization, incremental analysis
- **Sentinel** (Worker 2): 13 diagnostic patterns, flow tracing, boundary detection, confidence scoring, evidence generation
- **Interface** (Worker 3): CLI commands, manifest output (YAML/JSON/SARIF/summary), configuration, error messages

### QA Pairings

- QA-Foundation → Foundry (parser/graph edge cases, cache integrity)
- QA-Analysis → Sentinel (diagnostic true/false positive, fixture validation)
- QA-Surface → Interface (CLI testing, manifest format, pipe safety)

## Cycle Structure

Each cycle (26 sheets after fan-out):

1. **Executive** — Reads roadmap + previous synthesis, writes directives, issues DONE/CONTINUE verdict
2. **Manager** — Translates directives into per-agent assignments with intent directives
3. **Investigation** (×3) — Workers investigate their assigned areas
4. **Test Design** (×3) — QA agents write tests BEFORE implementation (TDD)
5. **Implementation** (×3) — Workers implement, making QA tests pass
6. **Documentation** (×2) — Light doc pass to keep docs in sync
7. **TDF Review** (×5) — Independent 5-perspective review
8. **Synthesizer** — Consolidates reviews, creates prioritized action list
9. **Antagonist** (×2) — Blind adversarial validation
10. **Dreamers** (×5) — Memory consolidation (hot/warm/cold tiering)

Pre-loop: intent alignment + preprocessing. Post-loop: 5-stage documentation pipeline.

Cycles 2+ skip automatically when all verdicts agree DONE (executive + synthesizer + antagonists).

## Custom Validations

After implementation and antagonist stages, these cargo commands run:

| Validation | Applies To | Purpose |
|-----------|------------|---------|
| `cargo build` | implementer, antagonist | Code compiles |
| `cargo test --all` | implementer, antagonist | All tests pass |
| `cargo clippy -- -D warnings` | implementer | No lint warnings |
| `cargo fmt --check` | implementer | Code formatting correct |

## Generator Extension Evaluation

The generator was evaluated for Flowspec-specific needs. **No modifications were required.** The config-based approach covers all Flowspec needs:

- **Validation system**: `command_succeeds` type handles cargo commands. Commands use absolute paths to the project root.
- **Spec injection**: Agents read `.flowspec/spec/` directory directly. The detailed spec quality level means no gap-filling guidance needed.
- **Prelude**: `CLAUDE.md` injected into all agents for project conventions and constraints.
- **Personas**: Fully configured via config — no generator changes needed for Rust/static-analysis domain.
- **Custom validations**: The `applies_to` mechanism (implementer, antagonist, executive) covers the needed validation points.

## Workspace

The workspace (`workspaces/build/`) stores Mozart orchestration artifacts:
- `intent-brief.md` — Structured intent brief
- `memory/` — Agent memory files (personal + collective)
- `cycle-N/` — Per-cycle artifacts (assignments, investigations, tests, reviews, synthesis, verdicts)
- `executive-roadmap-1.md` — Executive's spec compliance checklist
- `post-loop/` — Documentation pipeline artifacts

Add `workspaces/` to `.gitignore` — these are orchestration artifacts, not source code.

## Key Design Decisions

- **3 workers (not 4)**: Flowspec's pipeline has 3 natural layers (parser/graph, analyzer, CLI/manifest). Each worker owns a coherent slice.
- **1 manager (not 2)**: Single team, simpler coordination. The project is more focused than Company-Simulator-9000.
- **3 QA (1:1 with workers)**: Each QA agent writes tests specifically for their paired worker's domain. Enables strict TDD.
- **100 cycles**: Safety buffer. Skip-when mechanism terminates early. Flowspec scope estimates 55-85 cycles.
- **spec_quality: outline**: The 9-file spec corpus defines WHAT comprehensively, but there's zero implementation. Agents fill tactical details (crate versions, type signatures, test harness) using their expertise.
- **CLAUDE.md prelude**: Ensures all agents know Flowspec conventions (thiserror, tracing, no unsafe, etc.) from the start.
