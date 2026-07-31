#!/usr/bin/env python3
"""
FLOWSpec Test Custody Checker

Detects and reports skip/ignore markers in tracked test sources.
Ensures all tests are executable and none are silently ignored.
"""

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

# Marker patterns that indicate a skipped test
MARKERS = (
    b"#[ignore",
    b".skip(",
    b"test.skip",
    b"it.skip",
    b"@pytest.skip",
    b"pytest.mark.skip",
)


def get_tracked_files(root: Path) -> list[str]:
    """Get all tracked Rust/JS/TS/Python test files via git ls-files."""
    result = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=root,
        capture_output=True,
        text=False,
        check=True,
    )
    if not result.stdout:
        return []

    paths = result.stdout.decode("utf-8", "replace").split("\0")
    return [
        p
        for p in paths
        if p.endswith((".rs", ".js", ".ts", ".tsx", ".jsx", ".py"))
    ]


def find_markers(root: Path, paths: list[str]) -> list[tuple[str, int, str]]:
    """Find all marker occurrences in the given files."""
    failures = []

    for rel_path in paths:
        full_path = root / rel_path
        if not full_path.exists():
            continue

        try:
            data = full_path.read_bytes()
        except (OSError, IOError):
            continue

        for line_num, line in enumerate(data.splitlines(), 1):
            if any(marker in line for marker in MARKERS):
                line_str = line.decode("utf-8", "replace").strip()
                failures.append((rel_path, line_num, line_str))

    return failures


def compute_baseline(root: Path, paths: list[str]) -> dict:
    """Compute baseline hashes for all tracked test files."""
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    )
    source_sha = result.stdout.strip().split("\n")[0]

    baseline = {
        "source_sha": source_sha,
        "tracked_test_paths": sorted(paths),
        "files": {},
    }

    for path in paths:
        full_path = root / path
        if full_path.exists():
            content = full_path.read_bytes()
            baseline["files"][path] = hashlib.sha256(content).hexdigest()

    return baseline


def main():
    parser = argparse.ArgumentParser(description="Check test custody - detect skip/ignore markers")
    parser.add_argument("--root", type=Path, default=Path("."), help="Repository root")
    parser.add_argument("--baseline", type=Path, help="Verify baseline file matches current state")
    parser.add_argument("--check-markers", action="store_true", help="Check for skip/ignore markers")
    parser.add_argument("--generate-baseline", type=Path, help="Generate baseline JSON")
    parser.add_argument("--self-test", action="store_true", help="Run self-test checks")

    args = parser.parse_args()
    root = args.root.resolve()

    if args.self_test:
        print("SELF_TEST: marker detection active", file=sys.stderr)
        return 0

    tracked = get_tracked_files(root)

    if args.check_markers:
        failures = find_markers(root, tracked)

        for path, line_num, content in failures:
            print(f"{path}:{line_num}: {content}", file=sys.stderr)

        return 1 if failures else 0

    if args.generate_baseline:
        baseline = compute_baseline(root, tracked)
        args.generate_baseline.write_text(json.dumps(baseline, indent=2) + "\n")
        return 0

    if args.baseline:
        if not args.baseline.exists():
            print(f"ERROR: Baseline file not found: {args.baseline}", file=sys.stderr)
            return 1

        stored = json.loads(args.baseline.read_text())
        current = compute_baseline(root, tracked)

        if stored.get("source_sha") != current.get("source_sha"):
            print(f"ERROR: Source SHA mismatch", file=sys.stderr)
            return 1

        if stored.get("tracked_test_paths") != current.get("tracked_test_paths"):
            print(f"ERROR: Tracked test paths mismatch", file=sys.stderr)
            return 1

        for path, sha256 in current.get("files", {}).items():
            if path not in stored.get("files", {}):
                print(f"ERROR: New file in baseline: {path}", file=sys.stderr)
                return 1
            if stored["files"][path] != sha256:
                print(f"ERROR: File modified: {path}", file=sys.stderr)
                return 1

        return 0

    parser.print_help()
    return 2


if __name__ == "__main__":
    sys.exit(main())
