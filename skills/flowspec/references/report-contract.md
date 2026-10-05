# Report contract

Schema v2 separates execution (`complete`, `incomplete`, `error`) from findings.
Outcome precedence is `error > incomplete > findings > clean`. An incomplete
run can retain valid positive findings, but cannot prove absent findings absent.

Read `authority` as the canonical envelope. It includes producer provenance,
registry/config/input digests, scope, execution, inventory, losses, per-pattern
cells, complete findings, gate, display indices and continuation. Compatibility
fields at the root are projections. Do not infer full counts from the displayed
`diagnostics` array. Keep producer SHA/tree and report identity beside evidence.

Losses include parsing/recovery, unreadable inputs, races, unsupported semantics,
unresolved prerequisites and output truncation. Read reason codes and affected
ranges; a rejected selected file is not a policy exclusion. Exclusions define a
smaller declared scope and remain visible. Missing capability makes the affected
pattern incomplete; absent user policy can make a policy-dependent pattern visibly
not applicable. Neither is permission to claim a true negative.

The gate applies declared severity/confidence thresholds independently of display
filters. An exit 0 below the gate threshold can still have a `findings` outcome.
Use `--gate-severity info --gate-confidence low` when requiring zero findings.
Exit 1 is an error; exit 2 carries complete usable findings; exit 3 takes priority
when execution is incomplete.

Diff must refuse incomplete/error inputs and incompatible schema, scope, config,
adapter/capability and input provenance with exit 3. Stable finding IDs and exact
subject/relation/ranges/evidence identify changes. A missing finding in an
incomparable report is not resolved. A development binary's behavior must be
checked; desired semantics alone do not establish implementation.

SARIF results retain positive findings. Inspect invocation success, execution
notifications and FLOWSpec properties for incomplete execution even when results
are empty. Never treat an empty SARIF results array as a clean assertion.

The registered catalog and capability descriptors come from `flowspec registry`.
Registration is not proof of semantic completeness across the immutable oracle.
Any release reliability claim binds the exact binary and independently adjudicated
content-addressed acceptance corpus. A development skill archive carries
`release_qualified: false`; its checksum establishes identity, not release approval.
