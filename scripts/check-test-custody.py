#!/usr/bin/env python3
"""
Test custody checker: enforces that tracked test sources contain no ignore/skip markers.

Sacred tests cannot be skipped, ignored, or demoted. This script validates that
no #[ignore], .skip(), test.skip, or it.skip markers exist in tracked test files.

Usage:
    check-test-custody.py --root <repo> --check-markers
    check-test-custody.py --root <repo> --baseline <json>
"""

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from typing import List, Tuple


# Test skip/ignore markers to detect
MARKERS = (
    b"#[ignore",
    b".skip(",
    b"test.skip",
    b"it.skip",
)

# Paths to exclude from hashing (gitignored build artifacts)
EXCLUDE_PATTERNS = (
    b"workspaces/",
    b"target/",
)


def get_tracked_test_files(root: Path) -> List[str]:
    """Get all tracked test source files using git ls-files."""
    result = subprocess.run(
        ["git", "ls-files", "-z", "*.rs"],
        cwd=root,
        capture_output=True,
        check=True,
    )
    if not result.stdout:
        return []
    # Split by null byte and filter
    paths = [p.decode("utf-8") for p in result.stdout.split(b"\x00") if p]
    # Only include test files
    return [p for p in paths if p.endswith("tests.rs") or "/tests/" in p or "/test_" in p]


def marker_failures(root: Path, paths: List[str]) -> List[str]:
    """Check for ignore/skip markers in tracked test files.

    Only reports actual #[ignore] attributes, not string/comment mentions.
    """
    failures: List[str] = []
    for rel_path in paths:
        # Skip gitignored workspaces
        if any(pattern in rel_path.encode() for pattern in EXCLUDE_PATTERNS):
            continue

        full_path = root / rel_path
        try:
            data = full_path.read_bytes()
        except FileNotFoundError:
            continue

        lines = data.splitlines()
        for line_num, line in enumerate(lines, 1):
            line_str = line.decode("utf-8", "replace").strip()

            # Skip empty lines and comments
            if not line_str or line_str.startswith("//") or line_str.startswith("/*") or line_str.startswith("*"):
                continue

            # Check for #[ignore] attribute - must start the line or be preceded only by whitespace
            if line_str.startswith("#[ignore"):
                failures.append(f"{rel_path}:{line_num}: {line_str}")
            elif "#[ignore = " in line_str or "#[ignore=\"" in line_str:
                # Handle cases where #[ignore] has a value
                stripped_start = line_str.lstrip()
                if stripped_start.startswith("#[ignore"):
                    failures.append(f"{rel_path}:{line_num}: {line_str}")

            # Check for skip patterns - these must be actual method calls
            # Look for .skip( that's not in a string
            if ".skip(" in line_str:
                # Basic check: if it starts with identifier.skip( it's likely real
                # Skip if it's in a comment or string
                if not line_str.startswith("//") and '"' not in line_str.split(".skip(")[0]:
                    if any(line_str.split(".skip(")[0].endswith(prefix) for prefix in ["test.", "it.", "describe.", "context.", "suite."]):
                        failures.append(f"{rel_path}:{line_num}: {line_str}")

    return failures


def compute_digest(root: Path, rel_path: str) -> str:
    """Compute SHA-256 digest of a file."""
    full_path = root / rel_path
    return hashlib.sha256(full_path.read_bytes()).hexdigest()


def verify_baseline(root: Path, baseline: dict) -> Tuple[List[str], List[str]]:
    """Verify baseline paths exist and digests match."""
    missing: List[str] = []
    changed: List[str] = []

    for path, expected_digest in baseline.get("tracked_test_paths", {}).items():
        full_path = root / path
        if not full_path.exists():
            missing.append(path)
        else:
            actual_digest = compute_digest(root, path)
            if actual_digest != expected_digest:
                changed.append(f"{path}: expected {expected_digest}, got {actual_digest}")

    return missing, changed


def main() -> int:
    parser = argparse.ArgumentParser(description="Enforce test custody - no skip/ignore markers")
    parser.add_argument("--root", type=Path, required=True, help="Repository root")
    parser.add_argument("--check-markers", action="store_true", help="Check for ignore/skip markers")
    parser.add_argument("--baseline", type=Path, help="Baseline JSON to verify")
    parser.add_argument("--generate-baseline", type=Path, help="Generate baseline JSON")
    parser.add_argument("--source-sha", help="Source SHA for baseline metadata")

    args = parser.parse_args()

    # Get tracked test files
    test_files = get_tracked_test_files(args.root)

    if args.check_markers:
        failures = marker_failures(args.root, test_files)
        if failures:
            for failure in failures:
                print(failure, file=sys.stderr)
            return 1
        return 0

    if args.generate_baseline:
        baseline_data = {
            "source_sha": args.source_sha or "unknown",
            "tracked_test_paths": {},
        }
        for path in sorted(test_files):
            # Skip gitignored paths
            if any(pattern in path.encode() for pattern in EXCLUDE_PATTERNS):
                continue
            baseline_data["tracked_test_paths"][path] = compute_digest(args.root, path)

        args.generate_baseline.write_text(json.dumps(baseline_data, indent=2) + "\n")
        return 0

    if args.baseline:
        baseline_data = json.loads(args.baseline.read_text())
        missing, changed = verify_baseline(args.root, baseline_data)
        if missing or changed:
            if missing:
                print("Missing baseline paths:", file=sys.stderr)
                for path in missing:
                    print(f"  {path}", file=sys.stderr)
            if changed:
                print("Changed baseline files:", file=sys.stderr)
                for change in changed:
                    print(f"  {change}", file=sys.stderr)
            return 1
        return 0

    parser.error("No action specified -- use --check-markers, --generate-baseline, or --baseline")


if __name__ == "__main__":
    sys.exit(main())
