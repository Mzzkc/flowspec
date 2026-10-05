# Reliability boundary

Static analysis can prove facts only within an explicit finite scope and modeled
semantics. Unsupported or unresolved prerequisites must produce incomplete
execution or visibly justified non-applicability, never guessed negatives.

Schema v2 separates execution from findings and derives outcome with precedence
`error > incomplete > findings > clean`. Exit 0 passes the declared gate and can
still contain below-threshold findings. Exit 1 is an error, 2 carries complete
usable findings, and 3 carries incomplete execution. Inspect scope, inventory,
losses, pattern cells, full counts and continuation even when details are filtered.
See the [report reference](../skills/flowspec/references/report-contract.md).

No exact reliability claim is earned by this developmental candidate. A release
may use only the following claim after independent exact-binary/corpus acceptance:

> For this release binary and published immutable acceptance corpus, all thirteen
> default diagnostics exactly matched every independently adjudicated expected and
> forbidden finding. No maintained case was incomplete, erroneous, skipped,
> unclassified, or non-comparable.

The corpus must bind binary, registry/config, toolchain/build and corpus digests;
all applicable thirteen-diagnostic/four-language slices need exact expected and
forbidden findings, maintained mutations, adversarial and interaction cases,
pinned licensed real-world sources and independent adjudication. An unsealed
corpus or a permissive precision threshold cannot establish that claim.

Cached uncertainty remains uncertainty. Correct cache identity includes source
content, tool/schema, adapter/grammar/capabilities, normalized config, exclusions,
language and parser settings. Atomic writes, corrupt-cache rebuild reporting,
file-race incompleteness and full/warm/incremental equivalence require executable
proof. Their description here is a release requirement, not a claim that this
candidate has passed. Summary or truncated detail must preserve trust metadata;
when continuation is unsupported, retain the report and hold complete-detail claims.

Diff must reject incomplete/error or incomparable inputs with exit 3 and never
call an absent finding resolved. Exact trace requires complete ordered edge paths,
not same-name heuristics or subsequences. Developmental trace/diff output remains
subject to the packaged workflow gate.
