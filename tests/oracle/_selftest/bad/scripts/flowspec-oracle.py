#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial
"""flowspec-oracle.py — the verification oracle comparator.

This is the comparator half of the oracle. Its job is to RUN flowspec against the
dogfood targets + fixtures + traces, then APPLY the failure rules below to the actual
output. The other half — the contract gate — is `validate-oracle-artifacts.py`, which
verifies the *structure* of the reviewed artifacts and confirms this comparator
registers every required rule.

WAVE 1A STATUS: the rule registry + `--list-rules` + the run scaffolding are wired and
authoritative. Full execution (invoking the binary, diffing manifests, applying each
rule to live output) is wired by Wave 1B once the reviewed baseline + planted fixtures
exist. The `--list-rules` output is contract-stable from 1A onward: the validator keys
off these exact rule IDs.

The failure rules are defined ONCE here (the source of truth) and documented in
tests/oracle/README.md §5. Changing a rule ID is a contract break.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass, asdict
from typing import List

# ─────────────────────────────────────────────────────────────────────────────
# THE FAILURE RULES — the oracle fails the build when any of these fires.
# The validator (validate-oracle-artifacts.py) verifies EVERY rule here is registered
# by parsing this registry's --list-rules output. Do not delete or rename an ID; add
# new rules with new IDs instead.
# ─────────────────────────────────────────────────────────────────────────────

PRECISION_THRESHOLD = 0.80  # README §3: >=80% reviewed precision for warning/critical


@dataclass(frozen=True)
class OracleRule:
    id: str
    description: str
    applies_to: str  # "diagnostics" | "trace" | "cache" | "structure"


ORACLE_RULES: List[OracleRule] = [
    OracleRule(
        id="R001_UNCLASSIFIED_FINDING",
        description=(
            "A dogfood finding has no classification entry. Every emitted finding must "
            "map to a REAL/KNOWN_FP/DEFERRED_BOUNDARY classification."
        ),
        applies_to="diagnostics",
    ),
    OracleRule(
        id="R002_PRECISION_BELOW_THRESHOLD",
        description=(
            "A release-default warning/critical detector has < "
            f"{int(PRECISION_THRESHOLD * 100)}% reviewed precision "
            "(REAL / (REAL + KNOWN_FP), DEFERRED_BOUNDARY excluded). Demote to "
            "info/off-by-default/deferred, or fix the detector."
        ),
        applies_to="diagnostics",
    ),
    OracleRule(
        id="R003_REAL_DISAPPEARED_UNREVIEWED",
        description=(
            "A previously-classified REAL finding disappeared and no review note "
            "explains the underlying code fix. The signal must be preserved or the fix "
            "documented."
        ),
        applies_to="diagnostics",
    ),
    OracleRule(
        id="R004_KNOWN_FP_STILL_HIGH_CONF",
        description=(
            "A KNOWN_FP remains at high confidence after its detector claims to be "
            "fixed. The fix is unproven while the FP persists at high confidence."
        ),
        applies_to="diagnostics",
    ),
    OracleRule(
        id="R005_TRACE_MISSING_PATH",
        description=(
            "A trace fixture lacks the exact expected path and edge kinds. The actual "
            "trace output must contain every expected hop (entity + edge_kind) in order."
        ),
        applies_to="trace",
    ),
    OracleRule(
        id="R006_SCRATCH_IN_DIAGNOSTICS",
        description=(
            "Scratch, stash, workspaces/build, target, generated reports, or gitignored "
            "files appear in diagnostics. File discovery/exclusion must keep these out."
        ),
        applies_to="diagnostics",
    ),
    OracleRule(
        id="R007_FULL_VS_INCREMENTAL_MISMATCH",
        description=(
            "--full and --incremental manifests differ after normalizing timestamps, "
            "duration, cache metadata, and absolute temp paths. A stale incremental "
            "manifest is worse than a slow full run."
        ),
        applies_to="cache",
    ),
    OracleRule(
        id="R008_COUNT_DELTA_UNREVIEWED",
        description=(
            "A count delta between baseline and current requires review (new findings "
            "unclassified, REAL findings gone without a note, or >10% shift with no "
            "classification delta) and no review record exists."
        ),
        applies_to="diagnostics",
    ),
]


def list_rules() -> List[dict]:
    """Return the rule registry as JSON-serializable dicts (stable contract)."""
    return [asdict(r) for r in ORACLE_RULES]


# ─────────────────────────────────────────────────────────────────────────────
# Run scaffolding (Wave 1B wires full execution).
# ─────────────────────────────────────────────────────────────────────────────

# Scratch/stash/generated path patterns that must NEVER appear in diagnostics (R006).
FORBIDDEN_PATH_FRAGMENTS = (
    "/.git/",
    "/scratch/",
    "/stash/",
    "/workspaces/build/",
    "/target/",          # rust build output
    "/__pycache__/",
    "/.marianne-observer",
    "tarpaulin-report",
    "/node_modules/",
)


def run_oracle(flowspec_bin: str, flowspec_repo: str, marianne_repo: str) -> int:
    """Run the full oracle comparator.

    Wave 1A: returns a clear "not yet wired" status (exit 2) so CI treats it as
    pending rather than falsely green. Wave 1B implements:
      1. diagnose self + marianne -> compare against classifications (R001..R004,R006,R008)
      2. trace fixtures -> compare against traces/*.yaml (R005)
      3. analyze --full vs --incremental -> normalized diff (R007)
      4. compute per-pattern precision -> enforce >=80% (R002)
    """
    print(
        "flowspec-oracle: full execution is wired in Wave 1B (rule registry + "
        "--list-rules are authoritative from Wave 1A).",
        file=sys.stderr,
    )
    print(
        f"  binary     : {flowspec_bin}\n"
        f"  flowspec   : {flowspec_repo}\n"
        f"  marianne   : {marianne_repo}\n"
        f"  rules      : {len(ORACLE_RULES)} registered "
        f"({', '.join(r.id for r in ORACLE_RULES)})",
        file=sys.stderr,
    )
    return 2  # pending — not a false green


def main(argv: List[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="flowspec-oracle",
        description="Flowspec verification oracle comparator.",
    )
    parser.add_argument(
        "--list-rules",
        action="store_true",
        help="Print the failure-rule registry as JSON and exit (used by the validator).",
    )
    parser.add_argument(
        "--flowspec-bin",
        default="target/release/flowspec",
        help="Path to the flowspec binary (for full runs).",
    )
    parser.add_argument(
        "--flowspec-repo",
        default="<flowspec-repo>",
        help="flowspec repository root (self dogfood target).",
    )
    parser.add_argument(
        "--marianne-repo",
        default="<marianne-repo>",
        help="marianne-ai-compose repository root (dogfood target).",
    )
    args = parser.parse_args(argv)

    if args.list_rules:
        json.dump(list_rules(), sys.stdout, indent=2)
        sys.stdout.write("\n")
        return 0

    return run_oracle(args.flowspec_bin, args.flowspec_repo, args.marianne_repo)


if __name__ == "__main__":
    sys.exit(main())
