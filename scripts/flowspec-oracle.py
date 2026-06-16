#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial
"""flowspec-oracle.py - executable verification oracle comparator.

The comparator runs the flowspec release binary against reviewed diagnostic
baselines and trace fixtures, then applies the oracle failure rules to the
actual output. The rule registry is the stable contract consumed by
validate-oracle-artifacts.py.
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import subprocess
import sys
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass
from typing import Any

import yaml

PRECISION_THRESHOLD = 0.80


@dataclass(frozen=True)
class OracleRule:
    id: str
    description: str
    applies_to: str
    status: str = "implemented"
    deferred_reason: str | None = None


ORACLE_RULES: list[OracleRule] = [
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
            "(REAL / (REAL + KNOWN_FP), DEFERRED_BOUNDARY excluded)."
        ),
        applies_to="diagnostics",
    ),
    OracleRule(
        id="R003_REAL_DISAPPEARED_UNREVIEWED",
        description=(
            "A previously-classified REAL finding disappeared and no review note "
            "explains the underlying code fix."
        ),
        applies_to="diagnostics",
    ),
    OracleRule(
        id="R004_KNOWN_FP_STILL_HIGH_CONF",
        description=(
            "A KNOWN_FP remains at high confidence after its detector claims to be fixed."
        ),
        applies_to="diagnostics",
    ),
    OracleRule(
        id="R005_TRACE_MISSING_PATH",
        description=(
            "A trace fixture lacks the exact expected path and edge kinds in actual "
            "flowspec trace output."
        ),
        applies_to="trace",
    ),
    OracleRule(
        id="R006_SCRATCH_IN_DIAGNOSTICS",
        description=(
            "Scratch, stash, workspace/build, target, generated reports, or ignored "
            "files appear in diagnostics."
        ),
        applies_to="diagnostics",
    ),
    OracleRule(
        id="R007_FULL_VS_INCREMENTAL_MISMATCH",
        description=(
            "--full and --incremental manifests differ after normalizing timestamps, "
            "duration, cache metadata, and absolute temp paths."
        ),
        applies_to="cache",
        status="deferred",
        deferred_reason=(
            "This repair gate runs the existing release binary only; the full-vs-"
            "incremental cache equivalence gate is reserved for a separate "
            "oracle-smoke movement with a fresh build."
        ),
    ),
    OracleRule(
        id="R008_COUNT_DELTA_UNREVIEWED",
        description=(
            "A count delta between baseline and current requires review and no review "
            "record exists."
        ),
        applies_to="diagnostics",
    ),
]

IMPLEMENTED_RULE_IDS = frozenset(r.id for r in ORACLE_RULES if r.status == "implemented")

# Semantic gate coverage: run_oracle applies R001 R002 R003 R004 R005 R006 R008; R007 is deferred.
SEMANTIC_GATE_RULE_COVERAGE = (
    "run_oracle applies R001/R002/R003/R004/R005/R006/R008; "
    "R007_FULL_VS_INCREMENTAL_MISMATCH is explicitly deferred"
)

CLASSIFICATIONS = frozenset({"REAL", "KNOWN_FP", "DEFERRED_BOUNDARY"})

FORBIDDEN_PATH_FRAGMENTS = (
    "/.git/",
    "/scratch/",
    "/stash/",
    "/workspace/",
    "/workspaces/build/",
    "/target/",
    "/__pycache__/",
    "/.marianne-observer",
    "tarpaulin-report",
    "/node_modules/",
    "/generated/",
)

FALLBACK_FLOWSPEC_BIN = None  # resolved repo-relative in resolve_binary() (no hardcoded host paths)


@dataclass
class Violation:
    rule_id: str
    detail: str

    def as_dict(self) -> dict[str, str]:
        return {"rule_id": self.rule_id, "detail": self.detail}


def list_rules() -> list[dict[str, Any]]:
    return [asdict(r) for r in ORACLE_RULES]


def repo_root_from_script() -> str:
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def resolve_binary(path: str) -> str:
    if os.path.isfile(path):
        return path
    if not os.path.isabs(path):
        local = os.path.join(repo_root_from_script(), path)
        if os.path.isfile(local):
            return local
    fallback = os.path.join(repo_root_from_script(), "target", "release", "flowspec")
    if os.path.isfile(fallback):
        return fallback
    return path


def load_yaml(path: str) -> Any:
    with open(path, "r", encoding="utf-8") as fh:
        return yaml.safe_load(fh)


def parse_loc(loc: str) -> tuple[str, int] | None:
    if not isinstance(loc, str) or ":" not in loc:
        return None
    path, _, line_s = loc.rpartition(":")
    try:
        return path.strip(), int(line_s.strip())
    except ValueError:
        return None


def finding_symbol(finding: dict[str, Any]) -> str:
    value = finding.get("symbol", finding.get("entity", ""))
    return str(value)


def finding_key(finding: dict[str, Any]) -> tuple[str, str, int, str] | None:
    loc = parse_loc(str(finding.get("loc", "")))
    pattern = finding.get("pattern")
    if not loc or not pattern:
        return None
    return (str(pattern), loc[0], loc[1], finding_symbol(finding))


def entry_key(entry: dict[str, Any]) -> tuple[str, str, int, str] | None:
    pattern = entry.get("pattern")
    path = entry.get("path")
    line = entry.get("line")
    symbol = entry.get("symbol")
    if not pattern or not isinstance(path, str) or not isinstance(line, int) or not symbol:
        return None
    return (str(pattern), path, line, str(symbol))


def optional_fingerprint_mismatches(
    entry: dict[str, Any], finding: dict[str, Any]
) -> list[str]:
    """Check stable optional fields when classifications pin them.

    The matching key is the mandatory multiset key (pattern, path, line, symbol).
    If the review artifact also stores severity/confidence/message/range, those
    values must agree with the emitted finding.
    """
    pairs = (
        ("severity_at_review", "severity"),
        ("severity", "severity"),
        ("confidence_at_review", "confidence"),
        ("confidence", "confidence"),
        ("message_at_review", "message"),
        ("message", "message"),
        ("range_at_review", "range"),
        ("range", "range"),
    )
    bad = []
    for entry_field, raw_field in pairs:
        if entry_field in entry and entry.get(entry_field) not in (None, ""):
            if entry.get(entry_field) != finding.get(raw_field):
                bad.append(f"{entry_field}={entry.get(entry_field)!r} raw {raw_field}={finding.get(raw_field)!r}")
    return bad


def run_diagnose(binary: str, target_root: str) -> tuple[list[dict[str, Any]] | None, str, int]:
    try:
        proc = subprocess.run(
            [binary, "diagnose", target_root, "-f", "json", "-q"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=180,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        return None, f"diagnose invocation failed: {exc}", 127
    if proc.returncode not in (0, 2):
        return None, f"diagnose exited {proc.returncode}: {proc.stderr.strip()[:300]}", proc.returncode
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        return None, f"diagnose emitted invalid JSON: {exc}", proc.returncode
    if not isinstance(data, list):
        return None, "diagnose JSON is not an array", proc.returncode
    return data, f"{len(data)} finding(s)", proc.returncode


def load_classification_files(oracle_dir: str) -> list[tuple[str, dict[str, Any]]]:
    out: list[tuple[str, dict[str, Any]]] = []
    for path in sorted(glob.glob(os.path.join(oracle_dir, "diagnostics", "*.yaml"))):
        try:
            data = load_yaml(path)
        except yaml.YAMLError as exc:
            out.append((path, {"__parse_error__": str(exc)}))
            continue
        if isinstance(data, dict):
            out.append((path, data))
    return out


def resolve_target_root(target_root: str, oracle_dir: str) -> str:
    if os.path.isabs(target_root):
        return os.path.normpath(target_root)
    return os.path.normpath(os.path.join(oracle_dir, target_root))


def counter_examples(counter: Counter[tuple[str, str, int, str]], limit: int = 3) -> list[Any]:
    examples: list[Any] = []
    for key, count in sorted(counter.items()):
        examples.append({"key": key, "count": count})
        if len(examples) >= limit:
            break
    return examples


def find_optional_mismatches(
    raw_findings: list[dict[str, Any]], entries: list[dict[str, Any]]
) -> list[str]:
    raw_by_key: dict[tuple[str, str, int, str], list[dict[str, Any]]] = defaultdict(list)
    for finding in raw_findings:
        key = finding_key(finding)
        if key is not None:
            raw_by_key[key].append(finding)
    mismatches: list[str] = []
    for entry in entries:
        key = entry_key(entry)
        if key is None or not raw_by_key.get(key):
            continue
        finding = raw_by_key[key].pop(0)
        bad = optional_fingerprint_mismatches(entry, finding)
        if bad:
            mismatches.append(f"{key}: {', '.join(bad)}")
    return mismatches


def check_diagnostics(
    binary: str,
    oracle_dir: str,
    default_targets: list[tuple[str, str]],
) -> list[Violation]:
    violations: list[Violation] = []
    files = load_classification_files(oracle_dir)

    if not files:
        for label, target in default_targets:
            findings, note, _ = run_diagnose(binary, target)
            if findings is None:
                violations.append(Violation("R001_UNCLASSIFIED_FINDING", f"{label}: {note}"))
            elif findings:
                violations.append(
                    Violation(
                        "R001_UNCLASSIFIED_FINDING",
                        f"{label}: {len(findings)} live finding(s) but no classification file exists",
                    )
                )
        return violations

    for path, data in files:
        name = os.path.basename(path)
        if "__parse_error__" in data:
            violations.append(Violation("R008_COUNT_DELTA_UNREVIEWED", f"{name}: YAML parse error"))
            continue
        entries = [e for e in data.get("entries", []) if isinstance(e, dict)]
        target_root = resolve_target_root(str(data.get("target_root", ".")), oracle_dir)
        findings, note, _ = run_diagnose(binary, target_root)
        if findings is None:
            violations.append(Violation("R008_COUNT_DELTA_UNREVIEWED", f"{name}: {note}"))
            continue

        raw_counter: Counter[tuple[str, str, int, str]] = Counter()
        by_key: dict[tuple[str, str, int, str], list[dict[str, Any]]] = defaultdict(list)
        for finding in findings:
            if not isinstance(finding, dict):
                continue
            key = finding_key(finding)
            if key is None:
                continue
            raw_counter[key] += 1
            by_key[key].append(finding)
            path_part = f"/{key[1].replace(os.sep, '/')}"
            if any(fragment in path_part for fragment in FORBIDDEN_PATH_FRAGMENTS):
                violations.append(
                    Violation("R006_SCRATCH_IN_DIAGNOSTICS", f"{name}: forbidden diagnostic path {key[1]}")
                )

        cls_counter: Counter[tuple[str, str, int, str]] = Counter()
        entries_by_key: dict[tuple[str, str, int, str], list[dict[str, Any]]] = defaultdict(list)
        for entry in entries:
            key = entry_key(entry)
            if key is None:
                continue
            cls_counter[key] += 1
            entries_by_key[key].append(entry)

        unclassified = raw_counter - cls_counter
        disappeared = cls_counter - raw_counter
        if unclassified:
            violations.append(
                Violation(
                    "R001_UNCLASSIFIED_FINDING",
                    f"{name}: emitted finding(s) lack classifications {counter_examples(unclassified)}",
                )
            )
        if disappeared:
            real_missing = Counter(
                {
                    key: count
                    for key, count in disappeared.items()
                    if any(e.get("classification") == "REAL" for e in entries_by_key.get(key, []))
                }
            )
            other_missing = disappeared - real_missing
            if real_missing:
                violations.append(
                    Violation(
                        "R003_REAL_DISAPPEARED_UNREVIEWED",
                        f"{name}: REAL finding(s) disappeared without review {counter_examples(real_missing)}",
                    )
                )
            if other_missing:
                violations.append(
                    Violation(
                        "R008_COUNT_DELTA_UNREVIEWED",
                        f"{name}: classified finding(s) no longer emitted {counter_examples(other_missing)}",
                    )
                )

        for mismatch in find_optional_mismatches(findings, entries):
            violations.append(Violation("R008_COUNT_DELTA_UNREVIEWED", f"{name}: fingerprint drift {mismatch}"))

        precision_counts: dict[str, dict[str, int]] = defaultdict(lambda: {"REAL": 0, "KNOWN_FP": 0})
        for key, entry_list in entries_by_key.items():
            if not by_key.get(key):
                continue
            finding = by_key[key][0]
            if finding.get("severity") not in ("warning", "critical"):
                continue
            classification = entry_list[0].get("classification")
            if classification in ("REAL", "KNOWN_FP"):
                precision_counts[key[0]][str(classification)] += 1
        for pattern, counts in sorted(precision_counts.items()):
            denom = counts["REAL"] + counts["KNOWN_FP"]
            if denom and (counts["REAL"] / denom) < PRECISION_THRESHOLD:
                violations.append(
                    Violation(
                        "R002_PRECISION_BELOW_THRESHOLD",
                        f"{name}: {pattern} precision {counts['REAL']}/{denom} below {PRECISION_THRESHOLD:.0%}",
                    )
                )

        for key, entry_list in entries_by_key.items():
            if not by_key.get(key):
                continue
            finding = by_key[key][0]
            if finding.get("confidence") != "high":
                continue
            for entry in entry_list:
                if (
                    entry.get("classification") == "KNOWN_FP"
                    and (entry.get("detector_fixed") is True or data.get("detector_fixed") is True)
                ):
                    violations.append(
                        Violation(
                            "R004_KNOWN_FP_STILL_HIGH_CONF",
                            f"{name}: {key} remains high confidence after detector_fixed",
                        )
                    )

    return violations


def trace_hops(data: Any) -> list[dict[str, Any]]:
    if isinstance(data, list):
        return [h for h in data if isinstance(h, dict)]
    if not isinstance(data, dict):
        return []
    for key in ("hops", "expected_hops", "path", "trace", "edges", "nodes"):
        value = data.get(key)
        if isinstance(value, list):
            return [h for h in value if isinstance(h, dict)]
    return []


def hop_identity(hop: dict[str, Any]) -> tuple[str, str]:
    entity = hop.get("entity", hop.get("symbol", hop.get("name", "")))
    edge = hop.get("edge_kind", hop.get("kind", hop.get("edge", "")))
    return (str(entity), str(edge))


def contains_ordered_hops(actual: list[dict[str, Any]], expected: list[dict[str, Any]]) -> bool:
    actual_pairs = [hop_identity(h) for h in actual]
    cursor = 0
    for expected_hop in expected:
        wanted = hop_identity(expected_hop)
        while cursor < len(actual_pairs) and actual_pairs[cursor] != wanted:
            cursor += 1
        if cursor >= len(actual_pairs):
            return False
        cursor += 1
    return True


def check_traces(binary: str, oracle_dir: str) -> list[Violation]:
    violations: list[Violation] = []
    for path in sorted(glob.glob(os.path.join(oracle_dir, "traces", "*.yaml"))):
        name = os.path.basename(path)
        try:
            data = load_yaml(path)
        except yaml.YAMLError as exc:
            violations.append(Violation("R005_TRACE_MISSING_PATH", f"{name}: YAML parse error {exc}"))
            continue
        if not isinstance(data, dict):
            violations.append(Violation("R005_TRACE_MISSING_PATH", f"{name}: not a mapping"))
            continue
        fixture = data.get("fixture")
        symbol = data.get("symbol")
        direction = data.get("direction", "forward")
        expected = data.get("expected_hops")
        if not fixture or not symbol or direction not in ("forward", "backward") or not isinstance(expected, list):
            violations.append(Violation("R005_TRACE_MISSING_PATH", f"{name}: malformed trace expectation"))
            continue
        fixture_path = fixture if os.path.isabs(str(fixture)) else os.path.join(oracle_dir, str(fixture))
        try:
            proc = subprocess.run(
                [
                    binary,
                    "trace",
                    fixture_path,
                    "-s",
                    str(symbol),
                    "--direction",
                    str(direction),
                    "-f",
                    "json",
                    "-q",
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=120,
            )
        except (OSError, subprocess.SubprocessError) as exc:
            violations.append(Violation("R005_TRACE_MISSING_PATH", f"{name}: trace invocation failed {exc}"))
            continue
        if proc.returncode != 0:
            violations.append(
                Violation("R005_TRACE_MISSING_PATH", f"{name}: trace exited {proc.returncode}: {proc.stderr[:200]}")
            )
            continue
        try:
            actual = trace_hops(json.loads(proc.stdout))
        except json.JSONDecodeError as exc:
            violations.append(Violation("R005_TRACE_MISSING_PATH", f"{name}: trace JSON parse error {exc}"))
            continue
        if not contains_ordered_hops(actual, expected):
            violations.append(
                Violation(
                    "R005_TRACE_MISSING_PATH",
                    f"{name}: expected hops absent; expected={list(map(hop_identity, expected))} actual={list(map(hop_identity, actual))[:8]}",
                )
            )
    return violations


def run_oracle(
    flowspec_bin: str,
    flowspec_repo: str,
    marianne_repo: str,
    oracle_dir: str | None = None,
    json_output: bool = False,
) -> int:
    """run_oracle executes diagnose/trace and applies R001-R006/R008.

    R007_FULL_VS_INCREMENTAL_MISMATCH is explicitly deferred in the rule
    registry until the isolated oracle-smoke movement can exercise cache
    equivalence against a fresh build.
    """
    binary = resolve_binary(flowspec_bin)
    oracle_dir = oracle_dir or os.path.join(repo_root_from_script(), "tests", "oracle")
    flowspec_repo = flowspec_repo or repo_root_from_script()
    if marianne_repo is None:
        marianne_repo = os.environ.get("FLOWSPEC_MARIANNE_REPO") or os.path.join(
            repo_root_from_script(), "..", "marianne-ai-compose"
        )
    default_targets = [("flowspec", flowspec_repo), ("marianne", marianne_repo)]

    results: dict[str, Any] = {
        "binary": binary,
        "coverage": SEMANTIC_GATE_RULE_COVERAGE,
        "oracle_dir": oracle_dir,
        "rules": list_rules(),
        "violations": [],
        "deferred": [
            {"rule_id": r.id, "reason": r.deferred_reason}
            for r in ORACLE_RULES
            if r.status == "deferred"
        ],
    }

    if not os.path.isfile(binary):
        results["violations"].append(
            Violation("R001_UNCLASSIFIED_FINDING", f"flowspec binary not found: {binary}").as_dict()
        )
    else:
        violations = check_diagnostics(binary, oracle_dir, default_targets)
        violations.extend(check_traces(binary, oracle_dir))
        results["violations"] = [v.as_dict() for v in violations if v.rule_id in IMPLEMENTED_RULE_IDS]

    if json_output:
        json.dump(results, sys.stdout, indent=2, sort_keys=True)
        sys.stdout.write("\n")
    else:
        print(f"flowspec-oracle: binary={binary}")
        print(f"flowspec-oracle: oracle_dir={oracle_dir}")
        for item in results["deferred"]:
            print(f"DEFERRED {item['rule_id']}: {item['reason']}")
        for item in results["violations"]:
            print(f"VIOLATION {item['rule_id']}: {item['detail']}")
        if not results["violations"]:
            print("flowspec-oracle: PASS")

    return 1 if results["violations"] else 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="flowspec-oracle",
        description="Flowspec verification oracle comparator.",
    )
    parser.add_argument("--list-rules", action="store_true", help="Print rule registry JSON and exit.")
    parser.add_argument(
        "--flowspec-bin",
        default="target/release/flowspec",
        help="Path to the flowspec binary.",
    )
    parser.add_argument(
        "--flowspec-repo",
        default=None,
        help="flowspec repository root (default: this script's repo root).",
    )
    parser.add_argument(
        "--marianne-repo",
        default=None,
        help="marianne-ai-compose repository root (default: $FLOWSPEC_MARIANNE_REPO or <flowspec-repo>/../marianne-ai-compose).",
    )
    parser.add_argument(
        "--oracle-dir",
        default=None,
        help="Oracle artifact directory (default: tests/oracle beside this script).",
    )
    parser.add_argument("--json", action="store_true", help="Emit machine-readable verdict JSON.")
    args = parser.parse_args(argv)

    if args.list_rules:
        json.dump(list_rules(), sys.stdout, indent=2)
        sys.stdout.write("\n")
        return 0

    return run_oracle(
        args.flowspec_bin,
        args.flowspec_repo,
        args.marianne_repo,
        oracle_dir=args.oracle_dir,
        json_output=args.json,
    )


if __name__ == "__main__":
    sys.exit(main())
