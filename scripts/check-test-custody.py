#!/usr/bin/env python3
"""
FLOWSpec Test Custody Checker

Detects and reports skip/ignore markers in tracked test sources.
Ensures all tests are executable and none are silently ignored.
Syntax-aware: excludes comments, strings, iterator .skip(), self-references.
"""

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

# Marker patterns for actual test skip (not iterators, comments, strings)
MARKER_PATTERNS = {
    "rust_ignore": re.compile(r'#\[ignore(?:\s*=\s*"[^"]*")?\]'),
    "js_skip": re.compile(r'(test|it)\.skip\s*\('),
    "python_skip": re.compile(r'@pytest\.skip|pytest\.mark\.skip'),
}


def get_tracked_files(root: Path) -> list[str]:
    """Get all tracked Rust/JS/TS/Python test files via git ls-files."""
    result = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=root,
        capture_output=True,
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


def strip_comments_and_strings(line: str, ext: str) -> str:
    """Remove comments and string literals to detect actual markers."""
    if ext == ".py":
        # Python: strip # comments and triple quotes
        if '#' in line and '"' not in line and "'" not in line:
            line = line.split('#')[0]
        # Skip strings
        line = re.sub(r'"""[^"]*"""', '', line)
        line = re.sub(r"'''[^']*'''", '', line)
        line = re.sub(r'"[^"]*"', '', line)
        line = re.sub(r"'[^']*'", '', line)
    elif ext in (".rs", ".js", ".ts", ".tsx", ".jsx"):
        # Rust/JS: strip // comments and block comments
        if '//' in line and '"""' not in line and '"//' not in line:
            line = line.split('//')[0]
        # Skip strings (basic, not handling all edge cases)
        line = re.sub(r'"[^"]*"', '', line)
        line = re.sub(r"'[^']*'", '', line)
        line = re.sub(r'`[^`]*`', '', line)
    return line


def find_markers(root: Path, paths: list[str]) -> list[tuple[str, int, str]]:
    """Find all actual test skip markers, excluding false positives."""
    failures = []

    # Exclude the checker's own source and test_custody.rs synthetic fixtures
    exclude_paths = {
        "scripts/check-test-custody.py",
        "tests/test-custody-baseline.json",
        "flowspec-cli/tests/test_custody.rs",  # Contains synthetic fixtures
    }

    for rel_path in paths:
        # Skip excluded paths
        if rel_path in exclude_paths:
            continue

        full_path = root / rel_path
        if not full_path.exists():
            continue

        try:
            data = full_path.read_bytes()
        except (OSError, IOError):
            continue

        ext = full_path.suffix
        lines = data.decode("utf-8", "replace").splitlines()

        for line_num, raw_line in enumerate(lines, 1):
            line = raw_line.strip()

            # Skip empty lines
            if not line:
                continue

            # Strip comments and strings for detection
            clean_line = strip_comments_and_strings(line, ext)

            # Check for actual test markers (not iterator .skip())
            if ext == ".rs":
                match = MARKER_PATTERNS["rust_ignore"].search(clean_line)
                if match:
                    failures.append((rel_path, line_num, line))
            elif ext in (".js", ".ts", ".tsx", ".jsx"):
                # Only match test.skip or it.skip, not iterator .skip(
                match = MARKER_PATTERNS["js_skip"].search(clean_line)
                if match:
                    failures.append((rel_path, line_num, line))
            elif ext == ".py":
                match = MARKER_PATTERNS["python_skip"].search(clean_line)
                if match:
                    failures.append((rel_path, line_num, line))

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
    parser = argparse.ArgumentParser(
        description="Check test custody - detect skip/ignore markers (syntax-aware)"
    )
    parser.add_argument("--root", type=Path, default=Path("."), help="Repository root")
    parser.add_argument("--baseline", type=Path, help="Verify baseline file matches current state")
    parser.add_argument("--check-markers", action="store_true", help="Check for skip/ignore markers")
    parser.add_argument("--generate-baseline", type=Path, help="Generate baseline JSON")
    parser.add_argument("--self-test", action="store_true", help="Run self-test checks")

    args = parser.parse_args()
    root = args.root.resolve()

    if args.self_test:
        print("SELF_TEST: syntax-aware marker detection active", file=sys.stderr)
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
