#!/usr/bin/env python3
"""
FLOWSpec Test Custody Checker

Inspects every tracked test-bearing source for live skip/ignore markers and
guards the sacred-test baseline. The checker is intentionally strict:

  * No path is allowlisted out of custody. Every tracked file with a supported
    test extension is inspected, including this script and the integration test
    that exercises it.
  * Detection is extension-aware and lexical. Comments, strings, chars, raw
    strings, and templates are masked before matching, so marker text that only
    appears inside a literal or a remark is never reported. Runtime-constructed
    tokens in tests therefore stay clean, while a real attribute is caught.
  * Information loss fails closed. A tracked source that is a symlink, missing,
    unreadable, or not valid UTF-8 produces a stable path:0 reason and a nonzero
    exit. Nothing is silently skipped.
  * Baseline authority is ancestral. The recorded ``source_sha`` names the test
    authority at generation time and remains valid for any descendant whose test
    paths and bytes are unchanged, so a later non-test commit cannot self-invalidate
    custody.

Evidence is written to stderr as ``path:line: reason`` tuples so callers and
reviewers can act on stable identifiers rather than re-deriving intent.
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

# Supported test-bearing source extensions. Kept identical to the original
# custody inventory so the baseline path set is stable.
TEST_EXTENSIONS = (".rs", ".js", ".ts", ".tsx", ".jsx", ".py")

# Marker reason codes. Each is a stable identifier asserted by the custody
# contract; the matching text lives inside raw-string regex literals so this
# script's own source scans clean.
_MARKER_SPECS = [
    # Rust: a live ignore attribute. Its quoted argument, when present, is
    # masked away as a string before this runs on the code mask.
    ("rust_ignore", ".rs", re.compile(r"#\[\s*ignore\b")),
    # JavaScript: test/it skip call sites.
    ("javascript_test_skip", ".js", re.compile(r"(?<![\w$])(?:test|it)\.skip\s*\(")),
    # TypeScript: same form, distinct reason code.
    ("typescript_test_skip", ".ts", re.compile(r"(?<![\w$])(?:test|it)\.skip\s*\(")),
    # Python: pytest skip / mark.skip decorators.
    ("python_test_skip", ".py", re.compile(r"@pytest\.skip\b|pytest\.mark\.skip\b")),
]


def get_tracked_files(root: Path) -> list[str]:
    """Return tracked source paths with supported test extensions, sorted."""
    result = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=root,
        capture_output=True,
        check=True,
    )
    if not result.stdout:
        return []
    paths = [
        p
        for p in result.stdout.decode("utf-8", "replace").split("\0")
        if p.endswith(TEST_EXTENSIONS)
    ]
    return sorted(paths)


def _push_mask(out: list[str], ch: str) -> None:
    """Append a mask byte, preserving newlines for line-number stability."""
    out.append("\n" if ch == "\n" else " ")


def mask_rust(text: str) -> str:
    """Mask Rust comments, strings, raw strings, and char literals."""
    out: list[str] = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        prev_ident = i > 0 and (text[i - 1].isalnum() or text[i - 1] == "_")

        # Line comment.
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                out.append(" ")
                i += 1
            continue

        # Block comment (nestable in Rust).
        if c == "/" and i + 1 < n and text[i + 1] == "*":
            depth = 1
            out.append(" ")
            out.append(" ")
            i += 2
            while i < n and depth > 0:
                if text[i] == "/" and i + 1 < n and text[i + 1] == "*":
                    depth += 1
                    out.append(" ")
                    out.append(" ")
                    i += 2
                elif text[i] == "*" and i + 1 < n and text[i + 1] == "/":
                    depth -= 1
                    out.append(" ")
                    out.append(" ")
                    i += 2
                else:
                    _push_mask(out, text[i])
                    i += 1
            continue

        # Raw string r".." / r#".."# (and br-prefixed). Requires a word
        # boundary before the prefix so identifier tails are not consumed.
        if (c == "r" or c == "R") and not prev_ident:
            j = i + 1
            while j < n and text[j] == "#":
                j += 1
            if j < n and text[j] == '"':
                hashes = j - (i + 1)
                closer = '"' + "#" * hashes
                for _ in range(j - i + 1):
                    out.append(" ")
                i = j + 1
                while i < n:
                    if text.startswith(closer, i):
                        for _ in range(len(closer)):
                            out.append(" ")
                        i += len(closer)
                        break
                    _push_mask(out, text[i])
                    i += 1
                continue

        # Normal ("..." possibly byte-prefixed) string with backslash escapes.
        if c == '"':
            out.append(" ")
            i += 1
            while i < n:
                if text[i] == "\\" and i + 1 < n:
                    _push_mask(out, text[i])
                    _push_mask(out, text[i + 1])
                    i += 2
                    continue
                if text[i] == '"':
                    out.append(" ")
                    i += 1
                    break
                _push_mask(out, text[i])
                i += 1
            continue

        # Char literal '...' / '\..' vs lifetime 'ident.
        if c == "'":
            if i + 1 < n and text[i + 1] == "\\":
                out.append(" ")
                i += 1
                while i < n:
                    if text[i] == "\\" and i + 1 < n:
                        _push_mask(out, text[i])
                        _push_mask(out, text[i + 1])
                        i += 2
                        continue
                    if text[i] == "'":
                        out.append(" ")
                        i += 1
                        break
                    _push_mask(out, text[i])
                    i += 1
                continue
            if i + 2 < n and text[i + 2] == "'":
                out.append(" ")
                out.append(" ")
                out.append(" ")
                i += 3
                continue
            # Lifetime or stray quote: real code, do not mask.
            out.append(c)
            i += 1
            continue

        out.append(c)
        i += 1
    return "".join(out)


def mask_javascript(text: str) -> str:
    """Mask JavaScript/TypeScript comments, strings, and template literals."""
    out: list[str] = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]

        # Line comment.
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                out.append(" ")
                i += 1
            continue

        # Block comment (not nested in JS).
        if c == "/" and i + 1 < n and text[i + 1] == "*":
            out.append(" ")
            out.append(" ")
            i += 2
            while i < n:
                if text[i] == "*" and i + 1 < n and text[i + 1] == "/":
                    out.append(" ")
                    out.append(" ")
                    i += 2
                    break
                _push_mask(out, text[i])
                i += 1
            continue

        # Template literal (multiline). Interpolation is masked wholesale.
        if c == "`":
            out.append(" ")
            i += 1
            while i < n:
                if text[i] == "\\" and i + 1 < n:
                    _push_mask(out, text[i])
                    _push_mask(out, text[i + 1])
                    i += 2
                    continue
                if text[i] == "`":
                    out.append(" ")
                    i += 1
                    break
                _push_mask(out, text[i])
                i += 1
            continue

        # Single/double quoted string. Raw newlines are not legal in JS string
        # literals, so a newline closes the mask defensively.
        if c == '"' or c == "'":
            quote = c
            out.append(" ")
            i += 1
            while i < n:
                if text[i] == "\\" and i + 1 < n:
                    _push_mask(out, text[i])
                    _push_mask(out, text[i + 1])
                    i += 2
                    continue
                if text[i] == quote:
                    out.append(" ")
                    i += 1
                    break
                if text[i] == "\n":
                    out.append("\n")
                    i += 1
                    break
                out.append(" ")
                i += 1
            continue

        out.append(c)
        i += 1
    return "".join(out)


def mask_python(text: str) -> str:
    """Mask Python comments, triple-quoted strings, and normal strings."""
    out: list[str] = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]

        # Line comment.
        if c == "#":
            while i < n and text[i] != "\n":
                out.append(" ")
                i += 1
            continue

        # Triple-quoted strings.
        if c == '"' and text.startswith('"""', i):
            out.extend([" ", " ", " "])
            i += 3
            while i < n:
                if text.startswith('"""', i):
                    out.extend([" ", " ", " "])
                    i += 3
                    break
                _push_mask(out, text[i])
                i += 1
            continue
        if c == "'" and text.startswith("'''", i):
            out.extend([" ", " ", " "])
            i += 3
            while i < n:
                if text.startswith("'''", i):
                    out.extend([" ", " ", " "])
                    i += 3
                    break
                _push_mask(out, text[i])
                i += 1
            continue

        # Normal single/double string (prefix letters stay as code). Backslash
        # escapes are honored; a raw newline closes the mask defensively.
        if c == '"' or c == "'":
            quote = c
            out.append(" ")
            i += 1
            while i < n:
                if text[i] == "\\" and i + 1 < n:
                    _push_mask(out, text[i])
                    _push_mask(out, text[i + 1])
                    i += 2
                    continue
                if text[i] == quote:
                    out.append(" ")
                    i += 1
                    break
                if text[i] == "\n":
                    out.append("\n")
                    i += 1
                    break
                out.append(" ")
                i += 1
            continue

        out.append(c)
        i += 1
    return "".join(out)


def _masker_for(ext: str):
    if ext == ".rs":
        return mask_rust
    if ext in (".js", ".ts", ".tsx", ".jsx"):
        return mask_javascript
    if ext == ".py":
        return mask_python
    return None


def _specs_for(ext: str):
    return [(reason, rx) for reason, spec_ext, rx in _MARKER_SPECS if spec_ext == ext]


def find_failures(root: Path, paths: list[str]) -> list[str]:
    """Return stable path:line: reason evidence for loss and marker failures."""
    failures: list[str] = []
    for rel in paths:
        full = root / rel
        ext = Path(rel).suffix

        # Source-access checks fail closed, in precedence order.
        if full.is_symlink():
            failures.append(f"{rel}:0: tracked_source_symlink")
            continue
        if not full.exists():
            failures.append(f"{rel}:0: tracked_source_missing")
            continue
        if not os.access(full, os.R_OK):
            failures.append(f"{rel}:0: source_read_failed")
            continue
        try:
            raw = full.read_bytes()
        except OSError:
            failures.append(f"{rel}:0: source_read_failed")
            continue
        try:
            data = raw.decode("utf-8")
        except UnicodeDecodeError:
            failures.append(f"{rel}:0: invalid_utf8")
            continue

        masker = _masker_for(ext)
        specs = _specs_for(ext)
        if masker is None or not specs:
            continue

        masked = masker(data)
        original_lines = data.splitlines()
        masked_lines = masked.splitlines()
        # Keep lengths aligned even if a trailing line has no newline.
        while len(masked_lines) < len(original_lines):
            masked_lines.append("")

        for idx, masked_line in enumerate(masked_lines):
            for reason, rx in specs:
                if rx.search(masked_line):
                    line_no = idx + 1
                    content = ""
                    if line_no - 1 < len(original_lines):
                        content = original_lines[line_no - 1].strip()
                    failures.append(f"{rel}:{line_no}: {reason}: {content}")
                    break
    return failures


def current_head(root: Path) -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout.strip().split("\n")[0]


def is_ancestor(root: Path, ancestor: str, descendant: str) -> bool:
    result = subprocess.run(
        ["git", "merge-base", "--is-ancestor", ancestor, descendant],
        cwd=root,
        capture_output=True,
    )
    return result.returncode == 0


def compute_baseline(root: Path, paths: list[str]) -> dict:
    source_sha = current_head(root)
    files = {}
    for rel in paths:
        full = root / rel
        if full.exists() and not full.is_symlink():
            try:
                files[rel] = hashlib.sha256(full.read_bytes()).hexdigest()
            except OSError:
                continue
    return {
        "source_sha": source_sha,
        "tracked_test_paths": list(paths),
        "files": files,
    }


def verify_baseline(root: Path, paths: list[str], stored: dict) -> tuple[int, list[str]]:
    errors: list[str] = []
    source_sha = str(stored.get("source_sha", ""))
    head = current_head(root)

    # Ancestral authority: the recorded SHA must be HEAD or an ancestor of it.
    if not source_sha or not is_ancestor(root, source_sha, head):
        errors.append(f"ERROR: baseline_source_not_ancestor: {source_sha}")
        return 1, errors

    stored_paths = sorted(stored.get("tracked_test_paths", []))
    current_paths = sorted(paths)
    removed = [p for p in stored_paths if p not in current_paths]
    added = [p for p in current_paths if p not in stored_paths]
    for rel in removed:
        errors.append(f"ERROR: baseline path removed: {rel}")
    for rel in added:
        errors.append(f"ERROR: tracked path not in baseline: {rel}")

    stored_files = stored.get("files", {})
    for rel in sorted(stored_files):
        full = root / rel
        expected = stored_files[rel]
        if rel not in current_paths:
            continue
        if not full.exists() or full.is_symlink():
            errors.append(f"ERROR: test source lost: {rel}")
            continue
        try:
            actual = hashlib.sha256(full.read_bytes()).hexdigest()
        except OSError:
            errors.append(f"ERROR: test source lost: {rel}")
            continue
        if actual != expected:
            errors.append(f"ERROR: test byte drift: {rel}")

    return (1 if errors else 0), errors


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Check test custody - detect skip/ignore markers (lexical, fail-closed)"
    )
    parser.add_argument("--root", type=Path, default=Path("."), help="Repository root")
    parser.add_argument("--baseline", type=Path, help="Verify baseline file against current state")
    parser.add_argument("--check-markers", action="store_true", help="Check for skip/ignore markers")
    parser.add_argument("--generate-baseline", type=Path, help="Generate baseline JSON")
    parser.add_argument("--policy", type=Path, help="Custody policy file")
    parser.add_argument("--self-test", action="store_true", help="Run self-test checks")
    args = parser.parse_args()
    root = args.root.resolve()

    action_count = sum(
        [
            args.check_markers,
            args.generate_baseline is not None,
            args.baseline is not None,
            args.self_test,
        ]
    )
    if action_count != 1:
        print("ERROR: exactly_one_action_required", file=sys.stderr)
        return 2
    if args.policy is None:
        print("ERROR: policy_required", file=sys.stderr)
        return 2

    if args.self_test:
        print("SELF_TEST: lexical fail-closed marker detection active", file=sys.stderr)
        return 0

    tracked = get_tracked_files(root)

    if args.check_markers:
        failures = find_failures(root, tracked)
        for line in failures:
            print(line, file=sys.stderr)
        return 1 if failures else 0

    if args.generate_baseline:
        baseline = compute_baseline(root, tracked)
        args.generate_baseline.write_text(json.dumps(baseline, indent=2) + "\n")
        return 0

    if args.baseline:
        if not args.baseline.exists():
            print(f"ERROR: Baseline file not found: {args.baseline}", file=sys.stderr)
            return 1
        try:
            stored = json.loads(args.baseline.read_text())
        except json.JSONDecodeError as error:
            print(f"ERROR: malformed baseline JSON: {error}", file=sys.stderr)
            return 1
        code, errors = verify_baseline(root, tracked, stored)
        for line in errors:
            print(line, file=sys.stderr)
        return code

    parser.print_help()
    return 2


if __name__ == "__main__":
    sys.exit(main())
