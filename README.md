# FLOWSpec

FLOWSpec supplies structural reports for coding agents. Python, JavaScript,
TypeScript and Rust are registered, with exactly thirteen diagnostics enabled by
default. Registration and a successful plumbing test do not certify detector
correctness. This candidate is developmental; no launch qualification is claimed.

Read [reliability](docs/reliability.md) before interpreting absent findings. A
complete result supports claims only within its declared scope. An incomplete
result can retain useful positive findings, but cannot establish that code is clean.

## Distribution and development

Launch distribution is GitHub Release only. No crates.io publication is authorized.
Release installation instructions and filenames must be finalized against the
qualified, downloaded artifact and its checksums. Until then, build locally:

```sh
cargo +1.91.0 build --locked --release -p flowspec-cli
./target/release/flowspec --version
./target/release/flowspec registry
```

The source URL is the repository's configured origin; this document does not
invent a replacement installation URL or a published release asset.
See [contributing](CONTRIBUTING.md) for validation and packaging.

## Agent use

```sh
flowspec analyze PROJECT --format summary
flowspec analyze PROJECT --gate-severity info --gate-confidence low --format json --output before.json
flowspec diagnose PROJECT --gate-severity info --gate-confidence low --format json
flowspec trace PROJECT --symbol 'main.py::main' --format json
flowspec diff before.json after.json --format json
```

Choose an exact symbol ID from analysis and preserve reports outside the analyzed
source tree. Read execution, outcome, scope, losses and full counts before a claim.
Analyze defaults to a critical finding gate; select an explicit gate threshold
when requiring zero findings. Display filters do not change the gate or justify
ignoring incompleteness.

| Exit | Meaning |
| --- | --- |
| 0 | Complete and passing the declared gate; check full finding counts |
| 1 | Execution error |
| 2 | Complete with findings meeting the declared gate |
| 3 | Incomplete; valid findings retained; clean/regression claims blocked |

The compact [skill](skills/flowspec/SKILL.md) teaches the analyze, completeness,
evidence, smallest-change, diff and native-validation loop. See
[agent workflow](docs/agent-workflow.md), [generated diagnostics](docs/diagnostics.md),
[generated language support](docs/language-support.md), the
[adapter seam](docs/registry.md), and [security](docs/security.md).

## Current qualification limits

The exact four-language acceptance corpus, production-source coverage at least
90.00%, complete zero-skip test inventory, hostile security/dependency and cache
and reproducibility gates are required. Missing implementations fail CI with
named HOLDs; the older permissive oracle is not a replacement. Current CLI
limit/offset pagination and a finding-ID diagnose selector are not exposed by
the retained developmental binary. Trace and diff must be checked with actual
packaged inputs; an unsupported operation remains held. No public CI success,
fresh-install acceptance, full type-flow completeness or speed budget is claimed.

Any joined source change invalidates affected documentation, archive, binary and
review evidence. Run the live documentation checker against the final joined
binary and rerun the packaged workflow before release qualification.
