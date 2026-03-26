# Investigation Brief: `flowspec diff` Command

**Author:** Worker 1 (Foundry)
**Cycle:** 18
**Spec source:** `cli.yaml:141-161`
**Status:** Ready for implementation by Worker 3

---

## Spec Requirements

```
flowspec diff <old> <new> [flags]
```

- **Two required positional args:** paths to manifest files (old and new)
- **Optional flag:** `--section <name>` (repeatable) — filter to specific sections
- **Output:** Structural diff — entities added/removed/changed, new diagnostics, resolved diagnostics, boundary changes, flow changes
- **Exit codes:**
  - 0: Diff completed, manifests are identical or changes shown
  - 1: Diff failed (invalid manifests, missing files)
  - 2: Structural regressions found (new critical diagnostics)

## Architecture Decision: Manifest Comparison, Not Graph Comparison

The diff command compares **serialized manifest files**, NOT analysis graphs. This is the right approach because:

1. Manifests ARE the product — what AI agents consume. Diffing manifests diffs the output contract.
2. Manifests are self-contained serialized data with a known schema (`manifest/types.rs`).
3. No graph infrastructure needed — Worker 3 is unblocked.
4. Manifests already derive `Deserialize` via serde.

## Implementation Location

Following the thin-binary + library-functions pattern established by `analyze`, `diagnose`, `trace`, `init`:

- **Library:** `flowspec/src/commands.rs` — add `pub fn run_diff(...)` with all business logic
- **Binary:** `flowspec-cli/src/main.rs` — add `Diff` subcommand to clap enum, dispatch to `run_diff`

## Key Types

```rust
/// Result of comparing two manifests section by section.
pub struct ManifestDiff {
    pub entities: SectionDiff<String>,      // keyed by EntityEntry.id
    pub diagnostics: SectionDiff<String>,   // keyed by (pattern, entity, loc)
    pub flows: SectionDiff<String>,         // keyed by FlowEntry.id
    pub boundaries: SectionDiff<String>,    // keyed by BoundaryEntry.id
    pub dependency_graph: SectionDiff<String>, // keyed by (from, to)
    pub type_flows: SectionDiff<String>,    // keyed by type name
    pub metadata_changes: Vec<String>,      // human-readable change descriptions
    pub has_critical_regression: bool,      // triggers exit code 2
}

pub struct SectionDiff<K> {
    pub added: Vec<K>,
    pub removed: Vec<K>,
    pub changed: Vec<(K, K)>,  // (old_key, new_key) or (old_desc, new_desc)
}
```

## Matching Strategy

Each manifest section needs a matching key to determine added/removed/changed:

| Section | Match Key | Changed Detection |
|---------|-----------|-------------------|
| entities | `EntityEntry.id` (qualified_name) | kind, vis, sig, calls differ |
| diagnostics | `(pattern, entity, loc)` composite | severity, confidence, message differ |
| flows | `FlowEntry.id` | steps, issues differ |
| boundaries | `BoundaryEntry.id` | type, crossing_points differ |
| dependency_graph | `(from, to)` | weight, direction differ |
| type_flows | `TypeFlowEntry.type` | producers, consumers differ |
| metadata | N/A (single struct) | per-field comparison |
| summary | NOT DIFFED | regenerated each run |

**Implementation:** Use `HashMap<String, &T>` keyed by the match key for O(n) lookup instead of O(n²) naive comparison.

## Format Detection

Manifests can be YAML or JSON. Detection strategy:
1. Check file extension: `.yaml`/`.yml` → YAML, `.json` → JSON
2. Fallback: try YAML first (superset of JSON), then JSON if that fails
3. The two files can be in different formats (comparing YAML old vs JSON new is valid)

## Output Format

The diff output itself should support the same `--format` flag as other commands:

- **summary** (default): Human-readable text diff with section headers and change counts
- **yaml**: Full diff structure as YAML
- **json**: Full diff structure as JSON

SARIF diff is not standard — don't support it for diff output.

## Exit Code Logic

```
if parse_error { exit 1 }
if any NEW diagnostic has severity == "critical" { exit 2 }
exit 0
```

"New" means present in `new` manifest but absent in `old` manifest (by composite key match). Changed severity from non-critical to critical also counts.

## Section Filtering

`--section entities` should only compute and output the entities diff. Multiple `--section` flags are additive. If no `--section` is provided, diff all sections.

Valid section names: `entities`, `diagnostics`, `flows`, `boundaries`, `dependency_graph`, `type_flows`, `metadata`.

## Pipe Safety

- Diff output to stdout
- Logs/errors to stderr via tracing
- Exit codes are machine-readable for CI integration

## Dependencies on Existing Code

- `Manifest` struct in `manifest/types.rs` — already derives `Deserialize`
- `EntityEntry`, `DiagnosticEntry`, etc. — already derive `Serialize, Deserialize`
- `OutputFormat` enum in `commands.rs` — reuse for `--format` flag
- `FlowspecError` in `lib.rs` — reuse for error handling
- May need `PartialEq` derives on some manifest types for "changed" detection, or implement custom comparison by field

## Risk Assessment

- **Low risk:** This is pure data comparison on well-typed structs. No graph mutation, no parse pipeline involvement.
- **Medium risk:** Entity matching stability. `EntityEntry.id` includes the file stem, which should be stable across runs on the same codebase. But renamed files would appear as remove+add rather than rename.
- **Low risk:** Performance. Even large manifests (10K entities) are O(n) with HashMap-based matching.

## Testing Guidance for QA-3

See `cycle-18/investigation-1.md` Phase 2 section and `cycle-18/tests-3.md` for the QA-3 test spec. Key test categories:

1. Happy path: two manifests with known differences
2. Identical manifests: exit 0, empty diff
3. New critical diagnostic: exit 2
4. Invalid manifest: exit 1
5. Section filter: `--section entities` only
6. Format support: `--format json`, `--format yaml`
7. Regression: existing commands unaffected
