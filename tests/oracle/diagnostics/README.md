# Dogfood Diagnostic Classifications

Reviewed `REAL` / `KNOWN_FP` / `DEFERRED_BOUNDARY` classifications for every live dogfood
finding from the flowspec and marianne-ai-compose repositories. One file per target
(`self.yaml`, `marianne.yaml`), following the entry schema in
[`../SCHEMA.md`](../SCHEMA.md). Classification rules and the precision gate live in
[`../README.md`](../README.md).

## Wave 1A status: intentionally empty

Wave 1A ships the **contract + validator + provenance template** — no classification
entries. Dogfood classification is Wave 1B work: it requires the reviewed baseline run
recorded in [`../BASELINE-PROVENANCE.yaml`](../BASELINE-PROVENANCE.yaml) (every
`by_pattern` count is still `TBD-FILL-IN-1B`) and a built release binary.

Populating this directory with unreviewed or fabricated entries would violate the
anti-fabrication rule: every cited `path` must exist on disk, `line` must be inside the
file, and the classified count must exactly equal the live raw count. The validator
(`scripts/validate-oracle-artifacts.py`) enforces both.

## Wave 1B entry plan

1. Build the release binary; capture `raw_findings` and `by_pattern` counts for `self`
   and `marianne` into `BASELINE-PROVENANCE.yaml`.
2. For each live finding, add one entry with the required fields (`pattern`, `path`,
   `line`, `symbol`, `classification`, `reason`, `owner`, `expires`) — see `SCHEMA.md §1`.
3. Run `python3 scripts/validate-oracle-artifacts.py` until it reports PASS with zero
   SKIPs here: every emitted finding classified (`R001`), counts matching, and each
   release-default warning/critical detector at >=80% reviewed precision (`R002`).
