# Oracle harness: measured false negatives and false positives

Flowspec's accuracy bar is below 1% false negatives and below 1% false
positives on labelled samples from real codebases (decision of 2026-11-11).
These scripts measure it without using flowspec's own resolver: ground truth
comes from Python's `ast`.

## Callers

```sh
# derive certain/candidate call sites for the N most-called symbols of a tree
python3 scripts/oracle/ast_callers.py /path/to/project --output cases.json --limit 40
# score the binary's backward traces against them (2 GB cap, 120 s per query)
FLOWSPEC_BIN=target/release/flowspec python3 scripts/oracle/score_callers.py --manifest cases.json --label run-1
```

`<label>.md` reports per symbol: certain callers found/total, missed sites,
wrong certain callers, timing. Recall = found/total; a wrong certain caller
is a false positive. The two known convention differences (constructions
count as `__init__` calls; property reads count as calls) are documented in
`docs/design/streaming-index.md`.

## Diagnostics

```sh
flowspec diagnose /path/to/project -f json -o diag.json
python3 scripts/oracle/adjudicate_diagnose.py /path/to/project diag.json 30
```

For each pattern it samples findings and derives its own verdict from the
`ast` (is the name loaded anywhere? is the import used? does `__init__`
re-export it?). A `disagree` row is a candidate false positive to read by
hand; the per-pattern count divided by the sample is the measured FP rate.
False negatives for diagnostics need planted fixtures; see
`flowspec/src/pyindex/diagnostics.rs` tests for the planted shapes.

Corpora used on 2026-11-11: Marianne `src/marianne` (132 kLOC), Django
(534 kLOC), CPython `Lib/` (1.12 MLOC).
