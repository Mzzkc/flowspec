# Flowspec Oracle — the Binding Contract

> **Status:** Wave 1A. This document is the load-bearing contract for the flowspec
> verification oracle. It is reviewed by the conductor + co-conductor **before** any
> baseline, fixture, or harness work (Wave 1B) is allowed to run. Everything
> downstream — `scripts/validate-oracle-artifacts.py`, `scripts/flowspec-oracle.py`,
> the dogfood classifications, the trace expectations — is governed by the rules here.
>
> **Owner (this wave):** Canyon (contract, default policy, boundaries).
> **Validators:** Axiom (invariants, TP/FP math, adversarial proofs).
> **Arbiter:** Conductor (git/remote/publish; narrows spec when code truth demands it).

The oracle is a **reproducible harness, not a report.** Its job is to *fail* loudly the
moment a release-blocking regression in diagnostic accuracy, trace truth, cache
equivalence, or contract honesty appears. A green oracle means: every default
warning/critical diagnostic is reviewed at ≥80% precision, every trace fixture holds,
the cache is provably equivalent, and no public promise is unbacked by behavior.

---

## 1. The 13 canonical diagnostic patterns

The contract uses the **`DiagnosticPattern::name()` snake_case identifiers** emitted in
JSON output (`pattern` field) and accepted by `--checks`. These are the only valid
pattern keys. Note in particular that the public ID is **`orphaned_impl`**, not
`orphaned_implementation` — the enum variant is `OrphanedImplementation` but its emitted
name is `orphaned_impl`. There is exactly one name per diagnostic; spec/code/docs that
contradict this are bugs (plan item #2).

| # | Canonical ID (`name()`) | Enum variant | One-line failure pattern |
|---|-------------------------|--------------|--------------------------|
| 1 | `isolated_cluster` | `IsolatedCluster` | Connected component with zero inbound external edges |
| 2 | `data_dead_end` | `DataDeadEnd` | Symbol defined but never consumed downstream |
| 3 | `partial_wiring` | `PartialWiring` | Error handlers on some parallel paths but not others |
| 4 | `orphaned_impl` | `OrphanedImplementation` | Trait impl with no dispatch point / public method with zero callers |
| 5 | `duplication` | `Duplication` | Structural (IR-level) similarity, not textual |
| 6 | `contract_mismatch` | `ContractMismatch` | Decorator/annotation vs API schema / arity vs call site |
| 7 | `circular_dependency` | `CircularDependency` | Cycle in the module dependency graph |
| 8 | `layer_violation` | `LayerViolation` | Cross-module reference violating user layer rules |
| 9 | `incomplete_migration` | `IncompleteMigration` | Old/new patterns coexisting with split callers |
| 10 | `asymmetric_handling` | `AsymmetricHandling` | Parallel functions with inconsistent treatment |
| 11 | `stale_reference` | `StaleReference` | Import resolving to a re-export/shim/moved definition |
| 12 | `phantom_dependency` | `PhantomDependency` | Import where zero imported symbols are referenced |
| 13 | `missing_reexport` | `MissingReexport` | Public symbol not re-exported through parent module |

Current default severities (evidence-driven, from the fixtures corpus):

| Pattern | Default severity | Confidence observed | Release status (see FIXTURE-COVERAGE.yaml) |
|---------|------------------|---------------------|--------------------------------------------|
| `circular_dependency` | warning | high | implemented |
| `isolated_cluster` | warning | high | implemented |
| `data_dead_end` | warning | low–high | implemented (noisy — precision work in flight) |
| `orphaned_impl` | warning | moderate–high | implemented (noisy — precision work in flight) |
| `missing_reexport` | warning | — | implemented (no fixture firing yet) |
| `contract_mismatch` | warning | moderate | implemented (heuristic-heavy) |
| `incomplete_migration` | warning | high | implemented (heuristic-heavy) |
| `stale_reference` | warning | high | implemented |
| `partial_wiring` | warning | — | implemented (heuristic-heavy; no fixture firing yet) |
| `layer_violation` | warning | — | implemented (requires config layer rules) |
| `phantom_dependency` | info | high | implemented (info-default by design) |
| `duplication` | — | — | **deferred** (needs AST-subtree hashing; see §7) |
| `asymmetric_handling` | — | — | **deferred** (needs full CFG; see §7) |

A detector's *release-default severity* is the severity it emits **without** any CLI
filter. The precision gate (§3) applies to every detector whose release-default severity
is **warning or critical**. Info-default detectors (e.g. `phantom_dependency`) are held
to the fixture-truth gate (§4) but are not subject to the warning/critical precision
gate; they may not, however, be silently reclassified to silence noise.

---

## 2. Classifications — `REAL`, `KNOWN_FP`, `DEFERRED_BOUNDARY`

Every dogfood finding (from diagnosing flowspec itself and marianne-ai-compose) must
carry exactly one classification. An **unclassified finding is a contract failure**
(R001). The three classes are mutually exclusive and exhaustive for reviewed findings.

### Required target coverage

Production diagnostics require exactly the declared dogfood target set:

- `tests/oracle/diagnostics/self.yaml` for the flowspec repository itself.
- `tests/oracle/diagnostics/marianne.yaml` for marianne-ai-compose.

Once any production `diagnostics/*.yaml` file exists, both required target files must
exist, each file's `target` field must match its filename (`self` or `marianne`), and
`BASELINE-PROVENANCE.yaml` must declare the same target set. Unknown diagnostic target
files or unknown provenance targets are contract failures. A partial baseline such as
`self.yaml` without `marianne.yaml` is rejected as `REQUIRED_TARGET_MISSING`; matching
the live multiset for one target is not enough to claim the dogfood baseline is
classified.

### `REAL` — a true positive that describes a genuine structural defect
A finding is `REAL` when, after human/Musician review, the symbol or relationship it
flags is **genuinely** in the failure state the diagnostic claims:

- `data_dead_end` → the symbol is truly never consumed by any reachable caller and is
  not a declared public-API entry point, test helper, or generated artifact.
- `orphaned_impl` → the impl/method truly has no static or config-proven dispatch point.
- `circular_dependency` → a real cross-module cycle exists (not an intra-module call).
- `phantom_dependency` → the import truly contributes zero referenced symbols and is
  not a side-effect import.
- (and so on per pattern.)

`REAL` findings are the signal flowspec sells. They must be **preserved across
releases** unless an accompanying code fix removes the underlying defect. A `REAL`
finding that disappears without a review note explaining the fix is **R003** (a failure).

### `KNOWN_FP` — a false positive that is understood and not yet fixed
A finding is `KNOWN_FP` when the detector fired but review concluded the flagged code is
**correct/intentional** and the diagnostic is wrong about it. Common root causes:

- **Dynamic dispatch boundary:** the symbol is reached via Protocol/ABC/trait/vtable
  dispatch that v1 does not model (a documented boundary, §7) — but unlike
  `DEFERRED_BOUNDARY`, the *detector itself* misfired by claiming high confidence
  without proof.
- **Public API surface:** a genuinely public entry point with no internal callers
  (correct by design) that the detector flagged at high confidence.
- **Test/generated code:** not excluded by config but should be (a config gap, not a
  code defect).
- **Type-only / macro / derive usage:** the symbol is used in a way the graph cannot
  see (annotation, derive, type alias).

A `KNOWN_FP` must carry a `reason` naming the root cause and an `owner`. If the
detector is later fixed to stop emitting that FP, the classification is **retired**
(removed). A `KNOWN_FP` that **remains high-confidence after its detector claims to be
fixed** is **R004** (a failure — the fix is unproven). A `KNOWN_FP` may persist at
lowered confidence only if the detector's default severity for that confidence tier is
info or off.

### `DEFERRED_BOUNDARY` — a finding attributable to a documented v1 limitation
A finding is `DEFERRED_BOUNDARY` when the *reason it appears* is one of the explicit v1
boundaries (§7) and the finding is therefore **expected, not a defect in the target
code, and not a detector bug to fix now**. This class exists so the oracle does not
penalize flowspec for not modelling things it explicitly defers.

- Runtime reflection / monkey-patching / dynamic imports (Python) that no static tool
  can see.
- Prototype metaprogramming / dynamic `require` (JS/TS).
- Macro-expanded Rust call graphs beyond the source AST.
- Full TS type semantics routed through the JS grammar.
- Unconstrained dynamic dispatch (vtable / trait objects) where the concrete target is
  unknowable statically.

`DEFERRED_BOUNDARY` findings are **expected noise from v1's scope**, documented as
boundaries, and excluded from the precision numerator/denominator (see §3). They still
require a `reason` citing the specific boundary and an `expires` date (boundary
classifications expire — if a later wave closes the boundary, they must be re-reviewed).

### When a finding is which — decision order
1. If review confirms the defect is real → **`REAL`**.
2. Else if the detector misfired on correct code (config gap, confidence bug,
   type/macro blindness) → **`KNOWN_FP`**.
3. Else if the finding exists solely because of a §7 boundary → **`DEFERRED_BOUNDARY`**.
4. Else it is **unclassified** → **R001** (the oracle fails until reviewed).

There is no fourth classification. "Unsure" is not permitted at release time.

---

## 3. The ≥80% reviewed-precision gate (release-default warning/critical)

**Precision** = (reviewed `REAL` findings) / (reviewed `REAL` + reviewed `KNOWN_FP`)
for a given diagnostic, **restricted to findings at the diagnostic's release-default
severity tier (warning or critical)**. `DEFERRED_BOUNDARY` findings are excluded from
both numerator and denominator — they are expected v1 noise, not detector error.

**The gate:** every detector whose release-default severity is **warning or critical**
must reach **≥ 80% reviewed precision** on the dogfood sample (flowspec-self +
marianne-ai-compose). Formally, for each such pattern `p`:

```
precision(p) = count(REAL, p, warning|critical)
             / ( count(REAL, p, warning|critical)
               + count(KNOWN_FP, p, warning|critical) )
precision(p) >= 0.80
```

If a detector cannot meet 80% at warning/critical default, it **must** be either:

- lowered to **info** default (and the changelog/spec record the demotion), or
- made **off-by-default** (`status: off-by-default` in FIXTURE-COVERAGE.yaml, with the
  spec/help text marking it experimental), or
- **deferred** (`status: deferred`, see `duplication` / `asymmetric_handling`).

Silently disabling a detector to pass the gate is **forbidden** (plan RISKS). Every
demotion/default-disable needs an explicit spec note + changelog entry. The validator's
fixture-truth gate (§4) still applies regardless of severity tier.

**`DEFERRED_BOUNDARY` is not a precision escape hatch.** You may not mass-classify
noise as `DEFERRED_BOUNDARY` to inflate precision: a `DEFERRED_BOUNDARY` entry must cite
a specific §7 boundary in its `reason`, and the boundary must be documented. Mass
deferral without a boundary citation is itself a contract failure (the validator
requires a non-empty `reason`; Axiom audits boundary citations during review).

---

## 4. Fixture-truth gate

Every one of the 13 patterns has, at release:

- **≥ 1 positive integration fixture** — a small Python/JS-TS/Rust project where the
  pattern **must** fire on the planted defect. Fixture positives are **100% TP**.
- **≥ 1 adversarial false-positive fixture** — a small project where the pattern
  **must not** fire on code that looks similar but is correct. Adversarial fixtures are
  **0 FP** at the pattern's release-default severity.

Fixtures live under `tests/oracle/fixtures/<lang>/<scenario>/`. Coverage is declared in
`FIXTURE-COVERAGE.yaml` (all 13 patterns, each with `status`, `positive_fixture`,
`adversarial_fixture`, `notes`). Wave 1A declares status + planned paths; Wave 1B
plants and proves the fixtures. A pattern may not be marked `implemented` at release
without both fixture types passing.

---

## 5. Comparator failure rules (the harness fails the build when any fires)

The oracle comparator (`scripts/flowspec-oracle.py`) enforces the following rules. The
validator (`scripts/validate-oracle-artifacts.py`) verifies the comparator **registers
every rule** (by parsing its `--list-rules` output, never by grepping a word). Each rule
has a stable ID; the comparator must emit all of them.

| Rule ID | Failure condition |
|---------|--------------------|
| **R001** `UNCLASSIFIED_FINDING` | A dogfood finding has no classification entry. |
| **R002** `PRECISION_BELOW_THRESHOLD` | A release-default warning/critical detector has < 80% reviewed precision (§3). |
| **R003** `REAL_DISAPPEARED_UNREVIEWED` | A previously-classified `REAL` finding disappears and no review note explains the underlying code fix. |
| **R004** `KNOWN_FP_STILL_HIGH_CONF` | A `KNOWN_FP` remains at high confidence after its detector claims to be fixed. |
| **R005** `TRACE_MISSING_PATH` | A trace fixture lacks the exact expected path and edge kinds. |
| **R006** `SCRATCH_IN_DIAGNOSTICS` | Scratch, stash, `workspaces/build`, `target`, generated reports, or gitignored files appear in diagnostics. |
| **R007** `FULL_VS_INCREMENTAL_MISMATCH` | `--full` and `--incremental` manifests differ after normalizing timestamps, duration, cache metadata, and absolute temp paths. |
| **R008** `COUNT_DELTA_UNREVIEWED` | A count delta between baseline and current requires review (new findings unclassified, or classified findings no longer emitted) and no review record exists (§6). |

The exit-code contract for `flowspec diagnose` is: **0 = no findings, 2 = findings
present** (both acceptable for the harness run; any other code is a crash). The harness
must `set +e` around diagnose and accept 0 or 2.

---

## 6. When a count delta requires review

Diagnostic counts **will** change between runs as code and detectors evolve. Raw count
movement is **not** a failure — only *unreviewed* movement is. A count delta requires a
review record when any of these hold:

- **A new finding appears** that has no classification → **R001** (always a failure
  until classified).
- **A `REAL` finding disappears** → requires a review note tying the disappearance to a
  real code fix (commit reference or explanation). Without it → **R003**.
- **A `KNOWN_FP` disappears** because the detector stopped emitting it → good; the
  classification is retired (removed from the file). If the same FP *reappears* later
  at high confidence after the detector "fixed" it → **R004**.
- **Counts shift by > 10%** with no classification delta (no new REAL/KNOWN_FP/DEFERRED
  entries added or retired) → **R008** (the movement is unexplained; reviewer must
  classify the newcomers or document the reclassification).

The classification file is the review record. Each entry's `owner`, `reason`, and
`expires` fields are the audit trail. The validator enforces that every classified
finding matches a real emitted finding (no fabricated entries) and every emitted finding
is classified (no orphans) — see `SCHEMA.md` and the anti-fabrication check.

---

## 7. Documented v1 boundaries (not hidden failures)

These are explicit v1 scope limits. They appear in README, CLI help, and the spec. A
finding attributable to one of these is `DEFERRED_BOUNDARY` (§2), not a detector bug.

- **Runtime reflection / monkey-patching / dynamic imports** (Python `importlib`,
  `__import__`, `setattr`-based dispatch) — not modelled.
- **Prototype / metaprogramming** (JS `Proxy`, dynamic `require`, symbol-keyed
  dispatch) — not modelled.
- **Macro-expanded Rust call graphs** beyond the source AST — `macro_rules!` and proc
  macro expansion are out of v1 scope.
- **Full TypeScript type semantics** when routed through the JS grammar — type narrowing
  and conditional types are approximate.
- **Unconstrained dynamic dispatch** — Python duck-typed calls without assignable
  annotations, and Rust `dyn Trait` / vtable targets unknowable statically. Flowspec
  infers *directly provable* assignments only (plan item #4 RISK).
- **AST-subtree duplication** and **full-CFG asymmetric handling** — `duplication` and
  `asymmetric_handling` are **deferred** (plan RISKS); they do not ship at
  warning-default in v1.

Boundary claims must be **honest**: if a boundary is closed by a later wave, the
corresponding `DEFERRED_BOUNDARY` findings must be re-reviewed (they carry an `expires`
date for exactly this reason).

---

## 8. How the pieces fit together

```
tests/oracle/
├── README.md                 ← this file (the binding contract)
├── SCHEMA.md                 ← entry shapes: classification / trace / provenance
├── BASELINE-PROVENANCE.yaml  ← binary + git + command provenance (template in 1A)
├── FIXTURE-COVERAGE.yaml     ← all 13 patterns: status + fixture evidence
├── diagnostics/*.yaml        ← reviewed dogfood classifications (self + marianne)
├── fixtures/<lang>/...       ← planted positive + adversarial FP projects
├── traces/*.yaml             ← expected trace paths by symbol/direction/edge kind
└── _selftest/                ← planted good/bad samples proving the validator works

scripts/
├── flowspec-oracle.py        ← the comparator harness (rule registry + run)
└── validate-oracle-artifacts.py  ← the un-gameable contract gate (this wave's core)
```

**Wave 1A ships:** this README, SCHEMA.md, BASELINE-PROVENANCE.yaml (template),
FIXTURE-COVERAGE.yaml (status + planned paths), `scripts/flowspec-oracle.py` (rule
registry + `--list-rules`), `scripts/validate-oracle-artifacts.py` (the gate), and the
`_selftest/` samples. **Wave 1B fills:** the reviewed dogfood baseline, the planted
fixtures, the trace expectations, and wires the comparator's full execution.
