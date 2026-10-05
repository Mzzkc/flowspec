# Contributing

Preserve existing test assertions and fixtures. Changes to test custody require
independent review; a filtered pass cannot replace complete validation. Record a
reachable test-only RED commit before a repair or feature GREEN commit.

Run formatting, strict lint, and the locked complete Rust test matrix:

```sh
cargo +1.91.0 fmt --all --check
cargo +1.91.0 clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo +1.91.0 test --locked --workspace --all-features
python3 scripts/check-test-custody.py --root . --policy tests/test-custody-policy.json --check-markers
python3 scripts/check-test-custody.py --root . --policy tests/test-custody-policy.json --baseline tests/test-custody-baseline.json
```

The release gate additionally requires inventory proving zero ignored, skipped,
filtered or retry-only cases; canonical production line coverage at least 90.00%;
the immutable exact thirteen-by-four oracle with no FP/FN/error/incomplete/skip/
unclassified/mutation-survivor; cache equivalence; security/dependency/license
review; dogfood/performance acceptance; and reproducible release builds.

Package only a committed clean skill source with an explicitly selected immutable
binary and reviewed SHA-256:

```sh
python3 scripts/package-skill.py --binary CANDIDATE --binary-sha256 REVIEWED_SHA256 --out PACKAGE_DIR
```

The ZIP contains `flowspec/SKILL.md`, two references, `LICENSE`, `LICENSE-AGPL`,
`bin/flowspec`, `package.json`, and `SHA256SUMS`. Its sidecar checksum binds the
archive. `package.json` binds the actual source HEAD/tree separately from the
binary producer HEAD/tree. Development packaging always records
`release_qualified: false`; it cannot certify a public release. Extract only after
checking member paths/types and checksums; restore executable mode for the binary.

Run new focused tests with `FLOWSPEC_SKILL_BINARY` and `FLOWSPEC_SKILL_ARCHIVE`
bound to the exact candidate and archive:

```sh
python3 -m unittest discover -s tests/skill -p 'test_*.py'
python3 tests/skill/run-workflow.py --archive ARCHIVE --out EVIDENCE_DIR
python3 scripts/check-product-contract.py --binary CANDIDATE --check
```

The strict workflow test fails on every HOLD. Do not change it to count a held
operation as passing. The registry checker can verify catalog/help/table agreement;
`--require-corpus` remains fail closed pending admitted corpus-schema integration.
Generate the two tables with `--write` before review when the registry changes.

CI uses the same commands. Required helpers are listed in
`scripts/check-ci-prerequisites.py`; availability is never qualification. Each
real helper must be implemented and independently reviewed before a green launch
claim. The component author must not invent oracle or coverage success to satisfy
CI. CI publication and the single joined whole qualification belong to the release
owner; never push, tag or publish as part of these local package checks.
