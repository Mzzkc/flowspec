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
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

# Supported test-bearing source extensions. Kept identical to the original
# custody inventory so the baseline path set is stable.
TEST_EXTENSIONS = (".rs", ".js", ".ts", ".tsx", ".jsx", ".py")
POLICY_MAX_BYTES = 1_048_576
POLICY_MAX_DEPTH = 64


class PolicyLoadError(Exception):
    """Stable fail-closed custody policy load failure."""

    def __init__(self, reason: str, pointer: str | None = None) -> None:
        super().__init__(reason)
        self.reason = reason
        self.pointer = pointer


_POLICY_SCHEMA = {
    "schema": "string",
    "schema_version": "integer",
    "policy_version": "string",
    "deny_unknown_fields": "boolean",
    "marker_scan": {
        "tracked_regular_utf8_source": "boolean",
        "path_exclusions_allowed": "boolean",
        "extensions": ("array", "string"),
    },
    "whole_file_roots": ("array", "string"),
    "whole_file_harness_paths": ("array", "string"),
    "mixed_rust_roots": ("array", "string"),
    "rust_region_rules": {
        "cfg_test_predicate": "boolean",
        "test_attributes": ("array", "string"),
        "policy_test_attributes": ("array", "string"),
        "external_modules_under_test_cfg": "boolean",
        "executable_rustdoc_fences": "boolean",
        "local_macro_invocations_inside_custodied_regions": "boolean",
        "fail_on_unclassified_test_constructs": "boolean",
    },
    "rust_include_edges": {
        "resolve_constant_literal_include_str": "boolean",
        "resolve_constant_literal_include_bytes": "boolean",
        "repository_contained_regular_files_only": "boolean",
        "unresolved_include_is_error": "boolean",
    },
    "control_baseline": {
        "path": "string",
        "excluded_from_own_file_hash_map": "boolean",
        "self_exclusion_reason": "string",
    },
    "cargo_inventory": {
        "platform_contract": {
            "os": "string",
            "workspace": "boolean",
            "all_features": "boolean",
        },
        "enumeration_commands": ("array", "string"),
        "exact_identity_set_required": "boolean",
        "target_qualified_identities": "boolean",
    },
}

_POLICY_ARRAYS = (
    (
        "/marker_scan/extensions",
        ("marker_scan", "extensions"),
        (".rs", ".js", ".ts", ".tsx", ".jsx", ".py"),
    ),
    (
        "/whole_file_roots",
        ("whole_file_roots",),
        ("tests/", "flowspec/tests/", "flowspec-cli/tests/"),
    ),
    (
        "/whole_file_harness_paths",
        ("whole_file_harness_paths",),
        (
            "scripts/check-test-custody.py",
            "scripts/flowspec-oracle.py",
            "scripts/validate-oracle-artifacts.py",
        ),
    ),
    (
        "/mixed_rust_roots",
        ("mixed_rust_roots",),
        ("flowspec/src/", "flowspec-cli/src/"),
    ),
    (
        "/rust_region_rules/test_attributes",
        ("rust_region_rules", "test_attributes"),
        ("test",),
    ),
    (
        "/rust_region_rules/policy_test_attributes",
        ("rust_region_rules", "policy_test_attributes"),
        (),
    ),
    (
        "/cargo_inventory/enumeration_commands",
        ("cargo_inventory", "enumeration_commands"),
        (
            "cargo test --workspace --all-features --no-run",
            "cargo test --workspace --all-features -- --list",
            "cargo test --workspace --all-features --doc -- --list",
        ),
    ),
)

_POLICY_PATH_ARRAYS = (
    ("/whole_file_roots", ("whole_file_roots",), True),
    ("/whole_file_harness_paths", ("whole_file_harness_paths",), False),
    ("/mixed_rust_roots", ("mixed_rust_roots",), True),
)

_POLICY_PATH_SCALARS = (
    ("/control_baseline/path", ("control_baseline", "path"), False),
)

_POLICY_UNSAFE_BOOLEAN_BINDINGS = (
    (
        "/marker_scan/tracked_regular_utf8_source",
        ("marker_scan", "tracked_regular_utf8_source"),
        True,
    ),
    (
        "/marker_scan/path_exclusions_allowed",
        ("marker_scan", "path_exclusions_allowed"),
        False,
    ),
    (
        "/rust_region_rules/cfg_test_predicate",
        ("rust_region_rules", "cfg_test_predicate"),
        True,
    ),
    (
        "/rust_region_rules/external_modules_under_test_cfg",
        ("rust_region_rules", "external_modules_under_test_cfg"),
        True,
    ),
    (
        "/rust_region_rules/executable_rustdoc_fences",
        ("rust_region_rules", "executable_rustdoc_fences"),
        True,
    ),
    (
        "/rust_region_rules/local_macro_invocations_inside_custodied_regions",
        (
            "rust_region_rules",
            "local_macro_invocations_inside_custodied_regions",
        ),
        True,
    ),
    (
        "/rust_region_rules/fail_on_unclassified_test_constructs",
        ("rust_region_rules", "fail_on_unclassified_test_constructs"),
        True,
    ),
    (
        "/rust_include_edges/resolve_constant_literal_include_str",
        ("rust_include_edges", "resolve_constant_literal_include_str"),
        True,
    ),
    (
        "/rust_include_edges/resolve_constant_literal_include_bytes",
        ("rust_include_edges", "resolve_constant_literal_include_bytes"),
        True,
    ),
    (
        "/rust_include_edges/repository_contained_regular_files_only",
        ("rust_include_edges", "repository_contained_regular_files_only"),
        True,
    ),
    (
        "/rust_include_edges/unresolved_include_is_error",
        ("rust_include_edges", "unresolved_include_is_error"),
        True,
    ),
    (
        "/cargo_inventory/platform_contract/workspace",
        ("cargo_inventory", "platform_contract", "workspace"),
        True,
    ),
    (
        "/cargo_inventory/platform_contract/all_features",
        ("cargo_inventory", "platform_contract", "all_features"),
        True,
    ),
    (
        "/cargo_inventory/exact_identity_set_required",
        ("cargo_inventory", "exact_identity_set_required"),
        True,
    ),
    (
        "/cargo_inventory/target_qualified_identities",
        ("cargo_inventory", "target_qualified_identities"),
        True,
    ),
)

_POLICY_VERSION = "phase-01-custody-closeout-red"

_POLICY_BINDINGS = (
    ("/schema", ("schema",), "flowspec.test_custody_policy"),
    ("/schema_version", ("schema_version",), 1),
    ("/deny_unknown_fields", ("deny_unknown_fields",), True),
    (
        "/control_baseline/path",
        ("control_baseline", "path"),
        "tests/test-custody-baseline.json",
    ),
    (
        "/control_baseline/excluded_from_own_file_hash_map",
        ("control_baseline", "excluded_from_own_file_hash_map"),
        True,
    ),
    (
        "/control_baseline/self_exclusion_reason",
        ("control_baseline", "self_exclusion_reason"),
        "baseline self hash has no fixed point",
    ),
    (
        "/cargo_inventory/platform_contract/os",
        ("cargo_inventory", "platform_contract", "os"),
        "linux",
    ),
)

_COMMAND_PREFIX = "cargo test --workspace --all-features"
_COMMAND_FILTER_TOKENS = ("--ignored", "--skip", "--exact")
_COMMAND_SHELL_METACHARACTERS = ";|&$()`<>"
_UNICODE_PATH_SEPARATORS = "\u2044\u2215\u29f5\uff0f"
_BIDI_PATH_CONTROLS = "\u202a\u202b\u202c\u202d\u202e\u2066\u2067\u2068\u2069"

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

# Reason names are part of the permanent adversarial contract.  Patterns run
# only over executable text returned by the language lexers below.
_MARKER_SPECS.extend([
    ("rust_ignore_spaced", ".rs", re.compile(r"#\s+\[\s*ignore\b")),
    ("rust_ignore_cfg_attr", ".rs", re.compile(r"#\s*\[\s*cfg_attr\s*\([^\]\n]*,\s*ignore\b")),
])
for _ext, _prefix in ((".js", "javascript"), (".ts", "typescript"),
                      (".jsx", "jsx"), (".tsx", "tsx")):
    _MARKER_SPECS.extend([
        (f"{_prefix}_test_skip", _ext, re.compile(r"(?<![\w$])(?:test|it)\.skip\s*\(")),
        (f"{_prefix}_suite_skip", _ext, re.compile(r"(?<![\w$])(?:describe|context|suite)\.skip\s*\(")),
        (f"{_prefix}_conditional_skip", _ext, re.compile(r"(?<![\w$])(?:test|it|describe|context|suite)\.(?:skipIf|runIf)\s*\(")),
        (f"{_prefix}_todo", _ext, re.compile(r"(?<![\w$])(?:test|it|describe|context|suite)\.todo\s*\(")),
        (f"{_prefix}_focus", _ext, re.compile(r"(?<![\w$])(?:test|it|describe|context|suite)\.only\s*\(")),
        (f"{_prefix}_disabled_alias", _ext, re.compile(r"(?<![\w$])x(?:test|it|describe|context|suite)\s*\(")),
        (f"{_prefix}_focus_alias", _ext, re.compile(r"(?<![\w$])f(?:test|it|describe|context|suite)\s*\(")),
        (f"{_prefix}_runtime_skip", _ext, re.compile(r"(?<![\w$])(?:pending|this\.skip)\s*\(")),
    ])
_MARKER_SPECS.extend([
    ("python_pytest_skip", ".py", re.compile(r"(?<![\w.])pytest\.skip\s*\(")),
    ("python_pytest_importorskip", ".py", re.compile(r"(?<![\w.])pytest\.importorskip\s*\(")),
    ("python_pytest_xfail", ".py", re.compile(r"(?<![\w.])pytest\.xfail\s*\(")),
    ("python_pytest_mark_skipif", ".py", re.compile(r"(?<![\w.])pytest\.mark\.skipif\b")),
    ("python_pytest_mark_xfail", ".py", re.compile(r"(?<![\w.])pytest\.mark\.xfail\b")),
    ("python_unittest_skip", ".py", re.compile(r"(?<![\w.])unittest\.skip\s*\(")),
    ("python_unittest_skip_if", ".py", re.compile(r"(?<![\w.])unittest\.skipIf\s*\(")),
    ("python_unittest_skip_unless", ".py", re.compile(r"(?<![\w.])unittest\.skipUnless\s*\(")),
    ("python_unittest_expected_failure", ".py", re.compile(r"(?<![\w.])unittest\.expectedFailure\b")),
    ("python_unittest_skiptest", ".py", re.compile(r"\braise\s+unittest\.SkipTest\b")),
])


_TRACKED_PATH_ERRORS: list[str] = []


def get_tracked_files(root: Path) -> list[str]:
    """Return every tracked path; reject lossy path decoding."""
    result = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=root,
        capture_output=True,
        check=True,
    )
    if not result.stdout:
        return []
    paths = []
    _TRACKED_PATH_ERRORS.clear()
    for raw in result.stdout.split(b"\0"):
        if not raw:
            continue
        try:
            path = raw.decode("utf-8")
        except UnicodeDecodeError:
            safe = "".join(chr(b) if 32 <= b < 127 else f"{b:02x}" for b in raw)
            _TRACKED_PATH_ERRORS.append(f"{safe}:0: invalid_path_utf8")
            continue
        if _is_noncanonical_policy_path(path, False):
            raise CustodyError("noncanonical_path", raw.hex())
        paths.append(path)
    return sorted(paths)


class CustodyError(Exception):
    def __init__(self, reason: str, path: str = "") -> None:
        super().__init__(reason)
        self.reason, self.path = reason, path


def read_source(root: Path, rel: str) -> bytes:
    """Linux regular-file read, with parent and final no-follow checks."""
    full = root / rel
    cursor = root
    for part in Path(rel).parts[:-1]:
        cursor = cursor / part
        try:
            st = os.lstat(cursor)
        except FileNotFoundError:
            raise CustodyError("source_missing", rel) from None
        except OSError:
            raise CustodyError("source_read_failed", rel) from None
        if stat.S_ISLNK(st.st_mode):
            raise CustodyError("source_symlink_component", rel)
        if not stat.S_ISDIR(st.st_mode):
            raise CustodyError("source_not_regular", rel)
    try:
        before = os.lstat(full)
    except FileNotFoundError:
        raise CustodyError("source_missing", rel) from None
    except OSError:
        raise CustodyError("source_read_failed", rel) from None
    if stat.S_ISLNK(before.st_mode):
        raise CustodyError("source_final_symlink", rel)
    if not stat.S_ISREG(before.st_mode):
        reason = "source_not_regular_fifo" if stat.S_ISFIFO(before.st_mode) else "source_not_regular"
        raise CustodyError(reason, rel)
    if not before.st_mode & 0o444:
        raise CustodyError("source_mode_unreadable", rel)
    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NONBLOCK", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    fd = -1
    try:
        fd = os.open(full, flags)
        opened = os.fstat(fd)
        if not stat.S_ISREG(opened.st_mode):
            raise CustodyError("source_not_regular", rel)
        chunks = []
        while True:
            piece = os.read(fd, 1024 * 1024)
            if not piece:
                break
            chunks.append(piece)
        after_fd = os.fstat(fd)
        after_path = os.lstat(full)
    except CustodyError:
        raise
    except OSError:
        raise CustodyError("source_read_failed", rel) from None
    finally:
        if fd >= 0:
            os.close(fd)
    identity = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns)
    if identity(before) != identity(opened) or identity(before) != identity(after_fd) or identity(before) != identity(after_path):
        raise CustodyError("source_changed_during_read", rel)
    raw = b"".join(chunks)
    if len(raw) != before.st_size:
        raise CustodyError("source_changed_during_read", rel)
    return raw


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
        byte_prefix = (i > 0 and text[i - 1] == "b" and
                       (i < 2 or not (text[i - 2].isalnum() or text[i - 2] == "_")))
        if (c == "r" or c == "R") and (not prev_ident or byte_prefix):
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


def mask_javascript(text: str) -> str:
    """Finite JS lexer: regular expressions and template expressions are distinct."""
    n = len(text)
    out = ["\n" if c == "\n" else " " for c in text]

    def quoted(i: int, quote: str) -> int:
        i += 1
        while i < n:
            if text[i] == "\\":
                i += 2
            elif text[i] == quote:
                return i + 1
            else:
                i += 1
        return n

    def regex_literal(i: int) -> int:
        i += 1
        bracket = False
        while i < n and text[i] != "\n":
            if text[i] == "\\":
                i += 2
            elif text[i] == "[":
                bracket = True
                i += 1
            elif text[i] == "]":
                bracket = False
                i += 1
            elif text[i] == "/" and not bracket:
                i += 1
                while i < n and text[i].isalpha():
                    i += 1
                return i
            else:
                i += 1
        return i

    def template(i: int) -> int:
        i += 1
        while i < n:
            if text[i] == "\\":
                i += 2
            elif text[i] == "`":
                return i + 1
            elif text.startswith("${", i):
                i = code(i + 2, True)
            else:
                i += 1
        return n

    def code(i: int, interpolation: bool = False) -> int:
        depth = 1 if interpolation else 0
        prev = "="
        while i < n:
            c = text[i]
            if interpolation and c == "}":
                depth -= 1
                if depth == 0:
                    return i + 1
            elif interpolation and c == "{":
                depth += 1
            if text.startswith("//", i):
                j = text.find("\n", i)
                i = n if j < 0 else j
                continue
            if text.startswith("/*", i):
                j = text.find("*/", i + 2)
                i = n if j < 0 else j + 2
                continue
            if c in "\"'":
                i = quoted(i, c)
                prev = "v"
                continue
            if c == "`":
                i = template(i)
                prev = "v"
                continue
            if c == "/" and prev in "=([{,:;!?+-*%&|^~<>" :
                i = regex_literal(i)
                prev = "v"
                continue
            out[i] = c
            if not c.isspace():
                prev = c if not (c.isalnum() or c in "_$") else "v"
            i += 1
        return n

    code(0)
    return "".join(out)


def mask_python(text: str) -> str:
    """Mask Python literals but retain executable f-string replacement fields."""
    n = len(text)
    out = ["\n" if c == "\n" else " " for c in text]

    def string(i: int, quote_at: int, f_string: bool) -> int:
        quote = text[quote_at]
        triple = text.startswith(quote * 3, quote_at)
        width = 3 if triple else 1
        j = quote_at + width
        closer = quote * width
        while j < n:
            if text[j] == "\\":
                j += 2
            elif text.startswith(closer, j):
                return j + width
            elif f_string and text.startswith("{{", j):
                j += 2
            elif f_string and text[j] == "{":
                j = code(j + 1, True)
            elif not triple and text[j] == "\n":
                return j
            else:
                j += 1
        return n

    def code(i: int, expression: bool = False) -> int:
        depth = 1 if expression else 0
        while i < n:
            c = text[i]
            if expression and c == "}":
                depth -= 1
                if depth == 0:
                    return i + 1
            elif expression and c == "{":
                depth += 1
            if c == "#":
                j = text.find("\n", i)
                i = n if j < 0 else j
                continue
            prefix = re.match(r"(?i:[rubf]{1,4})(?=['\"])", text[i:]) if (i == 0 or not (text[i-1].isalnum() or text[i-1] == "_")) else None
            if prefix:
                p = prefix.group()
                i = string(i, i + len(p), "f" in p.lower())
                continue
            if c in "\"'":
                i = string(i, i, False)
                continue
            out[i] = c
            i += 1
        return n

    code(0)
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


def _policy_path(root: Path, supplied: Path) -> Path:
    if supplied.is_absolute():
        return supplied
    return root / supplied


def _reject_parent_symlink_components(path: Path) -> None:
    parts = path.parent.parts
    if not parts:
        return
    current = Path(parts[0])
    for part in parts[1:]:
        current = current / part
        try:
            info = os.lstat(current)
        except FileNotFoundError:
            raise PolicyLoadError("policy_not_found") from None
        except OSError:
            raise PolicyLoadError("policy_read_failed") from None
        if stat.S_ISLNK(info.st_mode):
            raise PolicyLoadError("policy_symlink_component_rejected")


def _reason_for_non_regular(mode: int) -> str:
    if stat.S_ISFIFO(mode):
        return "policy_non_regular_fifo"
    if stat.S_ISSOCK(mode):
        return "policy_non_regular_socket"
    return "policy_non_regular"


def _read_policy_bytes(path: Path) -> bytes:
    _reject_parent_symlink_components(path)
    try:
        info = os.lstat(path)
    except FileNotFoundError:
        raise PolicyLoadError("policy_not_found") from None
    except OSError:
        raise PolicyLoadError("policy_read_failed") from None

    mode = info.st_mode
    if stat.S_ISLNK(mode):
        raise PolicyLoadError("policy_symlink_rejected")
    if not stat.S_ISREG(mode):
        raise PolicyLoadError(_reason_for_non_regular(mode))
    if info.st_size > POLICY_MAX_BYTES:
        raise PolicyLoadError("policy_too_large")

    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NONBLOCK
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW

    fd = -1
    try:
        fd = os.open(path, flags)
        opened_info = os.fstat(fd)
        if not stat.S_ISREG(opened_info.st_mode):
            raise PolicyLoadError("policy_read_failed")
        data = os.read(fd, POLICY_MAX_BYTES + 1)
    except PolicyLoadError:
        raise
    except OSError:
        raise PolicyLoadError("policy_read_failed") from None
    finally:
        if fd >= 0:
            try:
                os.close(fd)
            except OSError:
                pass

    if len(data) > POLICY_MAX_BYTES:
        raise PolicyLoadError("policy_too_large")
    return data


def _object_without_duplicates(pairs: list[tuple[str, object]]) -> dict:
    seen: set[str] = set()
    obj = {}
    for key, value in pairs:
        if key in seen:
            raise PolicyLoadError("policy_duplicate_field")
        seen.add(key)
        obj[key] = value
    return obj


def _reject_policy_too_deep(value: object) -> None:
    if not isinstance(value, (dict, list)):
        return
    stack: list[tuple[object, int]] = [(value, 1)]
    while stack:
        item, depth = stack.pop()
        if depth > POLICY_MAX_DEPTH:
            raise PolicyLoadError("policy_too_deep")
        if isinstance(item, dict):
            children = item.values()
        else:
            children = item
        for child in children:
            if isinstance(child, (dict, list)):
                stack.append((child, depth + 1))


def _join_json_pointer(parent: str, token: str) -> str:
    escaped = token.replace("~", "~0").replace("/", "~1")
    return f"{parent}/{escaped}"


def _matches_policy_type(value: object, schema: object) -> bool:
    if isinstance(schema, dict):
        return isinstance(value, dict)
    if isinstance(schema, tuple):
        return isinstance(value, list)
    if schema == "string":
        return isinstance(value, str)
    if schema == "boolean":
        return isinstance(value, bool)
    if schema == "integer":
        return isinstance(value, int) and not isinstance(value, bool)
    raise AssertionError(f"unknown policy schema node: {schema!r}")


def _validate_policy_structure(value: object, schema: object, pointer: str = "") -> None:
    if isinstance(schema, dict):
        assert isinstance(value, dict)
        for key in value:
            if key not in schema:
                raise PolicyLoadError(
                    "policy_unknown_field", _join_json_pointer(pointer, key)
                )
        for key in schema:
            if key not in value:
                raise PolicyLoadError(
                    "policy_missing_required", _join_json_pointer(pointer, key)
                )
        for key, child_schema in schema.items():
            if not _matches_policy_type(value[key], child_schema):
                raise PolicyLoadError(
                    "policy_invalid_type", _join_json_pointer(pointer, key)
                )
        for key, child_schema in schema.items():
            if isinstance(child_schema, (dict, tuple)):
                _validate_policy_structure(
                    value[key], child_schema, _join_json_pointer(pointer, key)
                )
        return

    if isinstance(schema, tuple):
        assert isinstance(value, list)
        item_schema = schema[1]
        for index, item in enumerate(value):
            item_pointer = _join_json_pointer(pointer, str(index))
            if not _matches_policy_type(item, item_schema):
                raise PolicyLoadError("policy_invalid_type", item_pointer)
            if isinstance(item_schema, (dict, tuple)):
                _validate_policy_structure(item, item_schema, item_pointer)
        return

    raise AssertionError(f"non-container policy schema node: {schema!r}")


def _policy_pointer(value: object, pointer: str) -> object:
    current = value
    for token in pointer.split("/")[1:]:
        assert isinstance(current, dict)
        current = current[token.replace("~1", "/").replace("~0", "~")]
    return current


def _semantic_value(policy: dict, keys: tuple[str, ...]) -> object:
    current = policy
    for key in keys:
        current = current[key]
    return current


def _is_noncanonical_policy_path(value: str, directory: bool) -> bool:
    if not value:
        return True
    if value[0] == "/" or "\\" in value or "//" in value:
        return True
    if len(value) >= 3 and value[1:3] == ":/" and (
        "A" <= value[0] <= "Z" or "a" <= value[0] <= "z"
    ):
        return True
    for character in value:
        codepoint = ord(character)
        if codepoint < 32 or codepoint == 127:
            return True
        if character in _UNICODE_PATH_SEPARATORS:
            return True
        if character in _BIDI_PATH_CONTROLS:
            return True
    if directory != (value[-1] == "/"):
        return True
    segment_start = 0
    for index, character in enumerate(value):
        if character != "/":
            continue
        segment = value[segment_start:index]
        if segment == "." or segment == "..":
            return True
        segment_start = index + 1
    final_segment = value[segment_start:]
    return final_segment == "." or final_segment == ".."


def _is_invalid_command_grammar(command: str) -> bool:
    prefix_length = len(_COMMAND_PREFIX)
    if len(command) <= prefix_length + 1:
        return True
    if command[:prefix_length] != _COMMAND_PREFIX:
        return True
    if command[prefix_length] != " " or command[-1] == " ":
        return True
    if "  " in command:
        return True
    for character in command:
        if not 32 <= ord(character) <= 126:
            return True
        if character in _COMMAND_SHELL_METACHARACTERS:
            return True
    for token in _COMMAND_FILTER_TOKENS:
        if token in command:
            return True
    return False


def _validate_policy_semantics(policy: dict) -> None:
    # Specific semantic classes precede generic exact-value binding.
    for pointer, keys, directory in _POLICY_PATH_ARRAYS:
        values = _semantic_value(policy, keys)
        assert isinstance(values, list)
        for index, value in enumerate(values):
            assert isinstance(value, str)
            if _is_noncanonical_policy_path(value, directory):
                raise PolicyLoadError(
                    "policy_noncanonical_path",
                    pointer + "/" + str(index),
                )
    for pointer, keys, directory in _POLICY_PATH_SCALARS:
        value = _semantic_value(policy, keys)
        assert isinstance(value, str)
        if _is_noncanonical_policy_path(value, directory):
            raise PolicyLoadError("policy_noncanonical_path", pointer)

    for pointer, keys, _expected in _POLICY_ARRAYS:
        values = _semantic_value(policy, keys)
        assert isinstance(values, list)
        if len(values) != len(set(values)):
            raise PolicyLoadError("policy_duplicate_item", pointer)

    for pointer, keys, expected in _POLICY_ARRAYS:
        values = _semantic_value(policy, keys)
        assert isinstance(values, list)
        if len(values) != len(expected):
            raise PolicyLoadError("policy_invalid_cardinality", pointer)

    commands_pointer = "/cargo_inventory/enumeration_commands"
    commands = _semantic_value(policy, ("cargo_inventory", "enumeration_commands"))
    assert isinstance(commands, list)
    for index, command in enumerate(commands):
        assert isinstance(command, str)
        if _is_invalid_command_grammar(command):
            raise PolicyLoadError(
                "policy_invalid_command",
                commands_pointer + "/" + str(index),
            )

    for pointer, keys, expected in _POLICY_ARRAYS:
        values = _semantic_value(policy, keys)
        assert isinstance(values, list)
        if set(values) == set(expected) and tuple(values) != expected:
            raise PolicyLoadError("policy_invalid_order", pointer)

    for pointer, keys, expected in _POLICY_UNSAFE_BOOLEAN_BINDINGS:
        if _semantic_value(policy, keys) != expected:
            raise PolicyLoadError("policy_unsafe_value", pointer)

    if _semantic_value(policy, ("policy_version",)) != _POLICY_VERSION:
        raise PolicyLoadError("policy_unsupported_version", "/policy_version")

    for pointer, keys, expected in _POLICY_ARRAYS:
        values = _semantic_value(policy, keys)
        assert isinstance(values, list)
        for index, (value, expected_value) in enumerate(zip(values, expected)):
            if value != expected_value:
                raise PolicyLoadError(
                    "policy_invalid_entry",
                    pointer + "/" + str(index),
                )

    for pointer, keys, expected in _POLICY_BINDINGS:
        if _semantic_value(policy, keys) != expected:
            raise PolicyLoadError("policy_invalid_binding", pointer)


def load_policy(root: Path, supplied: Path) -> dict:
    raw = _read_policy_bytes(_policy_path(root, supplied))
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        raise PolicyLoadError("policy_invalid_utf8") from None

    try:
        parsed = json.loads(text, object_pairs_hook=_object_without_duplicates)
    except PolicyLoadError:
        raise
    except RecursionError:
        raise PolicyLoadError("policy_too_deep") from None
    except json.JSONDecodeError:
        raise PolicyLoadError("policy_malformed_json") from None

    if not isinstance(parsed, dict):
        raise PolicyLoadError("policy_schema_type")
    _reject_policy_too_deep(parsed)
    _validate_policy_structure(parsed, _POLICY_SCHEMA)
    _validate_policy_semantics(parsed)
    return parsed


def find_failures(root: Path, paths: list[str]) -> list[str]:
    """Return stable path:line: reason evidence for loss and marker failures."""
    failures: list[str] = list(_TRACKED_PATH_ERRORS)
    for rel in paths:
        ext = Path(rel).suffix
        if ext not in TEST_EXTENSIONS:
            continue
        try:
            raw = read_source(root, rel)
        except CustodyError as error:
            aliases = {
                "source_missing": "tracked_source_missing",
                "source_final_symlink": "tracked_source_symlink",
                "source_mode_unreadable": "source_read_failed",
            }
            primary = "source_non_regular" if error.reason == "source_not_regular" else error.reason
            failures.append(f"{rel}:0: {primary} {error.reason}")
            if primary != error.reason:
                failures.append(f"{rel}:0: {error.reason}")
            if error.reason in aliases:
                failures.append(f"{rel}:0: {aliases[error.reason]}")
            continue
        try:
            data = raw.decode("utf-8")
        except UnicodeDecodeError:
            failures.append(f"{rel}:0: invalid_content_utf8 invalid_utf8")
            failures.append(f"{rel}:0: invalid_utf8")
            continue

        masker = _masker_for(ext)
        specs = _specs_for(ext)
        if masker is None or not specs:
            continue

        masked = masker(data)
        if ext == ".rs":
            for line_no, line in enumerate(data.splitlines(), 1):
                fence = re.match(r"\s*//[/!]\s*```\s*(.*)", line)
                if fence:
                    tags = re.split(r"[,\s]+", fence.group(1))
                    if "ignore" in tags:
                        failures.append(f"{rel}:{line_no}: rustdoc_ignore_fence")
                    if "no_run" in tags:
                        failures.append(f"{rel}:{line_no}: rustdoc_no_run_fence")
        original_lines = data.splitlines()
        masked_lines = masked.splitlines()
        # Keep lengths aligned even if a trailing line has no newline.
        while len(masked_lines) < len(original_lines):
            masked_lines.append("")

        for idx, masked_line in enumerate(masked_lines):
            if ext == ".py" and re.search(r"\bpytest\.skip\s*\(", masked_line):
                if re.search(r"\b[fF][rR]?[\"']|\b[rR][fF][\"']", original_lines[idx]):
                    failures.append(f"{rel}:{idx + 1}: python_f_string_skip")
            for reason, rx in specs:
                if rx.search(masked_line):
                    line_no = idx + 1
                    content = ""
                    if line_no - 1 < len(original_lines):
                        content = original_lines[line_no - 1].strip()
                    aliases = []
                    if reason == "rust_ignore_spaced":
                        aliases.append("rust_ignore")
                    if reason == "python_pytest_skip":
                        aliases.append("python_pytest_skip")
                        reason = "python_test_skip"
                    if reason == "python_test_skip" and "pytest.mark.skip" in content:
                        aliases.append("python_pytest_mark_skip")
                    extra = f" {' '.join(aliases)}" if aliases else ""
                    failures.append(f"{rel}:{line_no}: {reason}: {content}{extra}")
                    if reason == "python_test_skip" and "python_pytest_skip" in aliases:
                        failures.append(f"{rel}:{line_no}: python_pytest_skip: {content}")
                    break
    return failures



# Schema v2 deliberately keeps the policy parser above as the bootstrap authority.
LEGACY_SHA256 = "c579c2a68b65be17b2ae3356b3936fe98cdd974016d5f99dfc8003b04391abfe"
LEGACY_CANDIDATE = "a102deb681f0180985b4fdfb9cc544ed7d8db90e"
LEGACY_RED = "a3939034923a9a2deaeeb65e36affa26665c4fbf"
SHA256_RE = re.compile(r"[0-9a-f]{64}\Z")
SHA_RE = re.compile(r"[0-9a-f]{40}\Z")


def git_bytes(root: Path, *args: str) -> bytes:
    result = subprocess.run(["git", *args], cwd=root, capture_output=True)
    if result.returncode:
        raise CustodyError("git_authority_failed", " ".join(args))
    return result.stdout


def current_head(root: Path) -> str:
    return git_bytes(root, "rev-parse", "HEAD").decode("ascii").strip()


def is_ancestor(root: Path, ancestor: str, descendant: str) -> bool:
    return subprocess.run(["git", "merge-base", "--is-ancestor", ancestor, descendant],
                          cwd=root, capture_output=True).returncode == 0


def sha256(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def safe_text(root: Path, rel: str) -> str:
    raw = read_source(root, rel)
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError:
        raise CustodyError("invalid_content_utf8", rel) from None


def rust_regions(rel: str, source: str) -> tuple[list[dict], list[str]]:
    """Fingerprint complete outer test items in the finite Rust lexical universe."""
    mask = mask_rust(source)
    entries: list[dict] = []
    externals: list[str] = []
    occupied = 0
    attribute = re.compile(r"#\s*\[\s*(?:cfg\s*\([^\]]*\btest\b[^\]]*\)|(?:(?:\w+::)*test)\b[^\]]*)\]", re.S)
    for found in attribute.finditer(mask):
        if found.start() < occupied:
            continue
        start = found.start()
        pos = found.end()
        while True:
            whitespace = re.match(r"\s*", mask[pos:])
            pos += len(whitespace.group())
            next_attr = re.match(r"#\s*\[[^\]]*\]", mask[pos:])
            if not next_attr:
                break
            pos += len(next_attr.group())
        opening = mask.find("{", pos)
        semicolon = mask.find(";", pos)
        if semicolon >= 0 and (opening < 0 or semicolon < opening):
            end = semicolon + 1
            declaration = mask[pos:end]
            module = re.search(r"\bmod\s+(\w+)\s*;", declaration)
            if module and "cfg" in found.group():
                externals.append(str(Path(rel).with_name(module.group(1) + ".rs")))
        elif opening >= 0:
            depth = 1
            end = opening + 1
            while depth and end < len(mask):
                if mask[end] == "{":
                    depth += 1
                elif mask[end] == "}":
                    depth -= 1
                end += 1
            if depth:
                raise CustodyError("unclassified_test_construct", rel)
        else:
            raise CustodyError("unclassified_test_construct", rel)
        occupied = end
        raw = source[start:end].encode("utf-8")
        entries.append({"path": rel, "start": len(source[:start].encode("utf-8")),
                        "kind": "rust_test_item", "sha256": sha256(raw)})
    return entries, externals


def rustdoc_fences(rel: str, source: str) -> list[dict]:
    entries = []
    fence = None
    offset = 0
    for line in source.splitlines(keepends=True):
        match = re.match(r"\s*//[/!]\s*```(.*)", line)
        if match:
            if fence is None:
                fence = (offset, match.group(1).strip())
            else:
                start, tags = fence
                if not any(tag in tags.split(",") for tag in ("ignore", "no_run", "text", "sh")):
                    entries.append({"path": rel, "start": start, "kind": "rustdoc_fence",
                                    "sha256": sha256(source.encode("utf-8")[start:offset + len(line.encode("utf-8"))])})
                fence = None
        offset += len(line.encode("utf-8"))
    return entries


def includes(root: Path, rel: str, source: str, tracked: set[str]) -> set[str]:
    result = set()
    for match in re.finditer(r'\binclude_(?:str|bytes)!\s*\(\s*"([^"\n]+)"\s*\)', source):
        resolved = os.path.normpath(os.path.join(os.path.dirname(rel), match.group(1)))
        if resolved.startswith("../") or resolved.startswith("/") or resolved not in tracked:
            raise CustodyError("unresolved_include", rel)
        read_source(root, resolved)
        result.add(resolved)
    return result


def inventory(root: Path, policy: dict, paths: list[str]) -> dict:
    tracked = set(paths)
    control = policy["control_baseline"]["path"]
    whole_paths = {p for p in paths if any(p.startswith(prefix) for prefix in policy["whole_file_roots"])
                   or p in policy["whole_file_harness_paths"]}
    whole_paths.discard(control)
    regions = []
    rustdoc = []
    external = set()
    for rel in paths:
        if rel.endswith(TEST_EXTENSIONS):
            source = safe_text(root, rel)
            if rel.endswith(".rs"):
                whole_paths.update(includes(root, rel, source, tracked))
                rustdoc.extend(rustdoc_fences(rel, source))
                if any(rel.startswith(prefix) for prefix in policy["mixed_rust_roots"]):
                    new_regions, modules = rust_regions(rel, source)
                    regions.extend(new_regions)
                    external.update(modules)
    for rel in external:
        if rel not in tracked:
            raise CustodyError("external_test_module_missing", rel)
    whole_paths.update(external)
    whole = {rel: sha256(read_source(root, rel)) for rel in sorted(whole_paths)}
    return {"whole_files": whole, "rust_regions": sorted(regions, key=lambda x: (x["path"], x["start"])),
            "external_test_modules": {rel: whole[rel] for rel in sorted(external)},
            "rustdoc_inventory": sorted(rustdoc, key=lambda x: (x["path"], x["start"])),
            "tracked_test_paths": sorted(p for p in paths if p.endswith(TEST_EXTENSIONS))}


def executable_inventory(root: Path, paths: list[str]) -> tuple[list[str], list[str]]:
    if (root / "Cargo.toml").is_file():
        result = subprocess.run(["cargo", "test", "--workspace", "--all-features",
                                 "--no-run", "--message-format=json"],
                                cwd=root, capture_output=True, text=True)
        if result.returncode:
            raise CustodyError("cargo_inventory_failed", result.stderr[-300:])
        identities = []
        for line in result.stdout.splitlines():
            try:
                artifact = json.loads(line)
            except json.JSONDecodeError:
                raise CustodyError("cargo_inventory_malformed") from None
            if (artifact.get("reason") != "compiler-artifact" or
                    not artifact.get("executable") or
                    not artifact.get("profile", {}).get("test")):
                continue
            target = artifact["target"]
            package_id = artifact["package_id"]
            location, _, fragment = package_id.partition("#")
            package = fragment.split("@", 1)[0] if "@" in fragment else location.rstrip("/").rsplit("/", 1)[-1]
            qualifier = f"{package}|{','.join(target['kind'])}|{target['name']}"
            listed = subprocess.run([artifact["executable"], "--list"], cwd=root,
                                    capture_output=True, text=True)
            if listed.returncode:
                raise CustodyError("cargo_inventory_failed", qualifier)
            for entry in listed.stdout.splitlines():
                if entry.endswith(": test"):
                    identities.append(f"{qualifier}|{entry[:-6]}")
        if len(identities) != len(set(identities)):
            raise CustodyError("cargo_inventory_duplicate_identity")
        docs = subprocess.run(["cargo", "test", "--doc", "--workspace", "--all-features",
                               "--", "--list"], cwd=root, capture_output=True, text=True)
        if docs.returncode:
            raise CustodyError("rustdoc_inventory_failed", docs.stderr[-300:])
        doc_identities = sorted(line[:-6] for line in docs.stdout.splitlines()
                                if line.endswith(": test"))
        return sorted(identities), doc_identities
    identities = []
    for rel in paths:
        if rel.endswith(".rs"):
            source = safe_text(root, rel)
            mask = mask_rust(source)
            for match in re.finditer(r"#\s*\[\s*test\s*\]\s*(?:pub\s+)?fn\s+(\w+)", mask):
                identities.append("synthetic::" + match.group(1))
    return sorted(identities), []


def clean_source(root: Path, destination: Path) -> bool:
    result = git_bytes(root, "status", "--porcelain=v1", "-z")
    try:
        destination_rel = str(destination.resolve().relative_to(root.resolve()))
    except ValueError:
        destination_rel = None
    for record in result.split(b"\0"):
        if not record:
            continue
        name = record[3:].decode("utf-8", "surrogateescape")
        if name != destination_rel:
            return False
    return True


def compute_baseline(root: Path, policy: dict, paths: list[str], policy_path: Path) -> dict:
    source_sha = current_head(root)
    details = inventory(root, policy, paths)
    identities, docs = executable_inventory(root, paths)
    policy_rel = str(policy_path.resolve().relative_to(root.resolve())) if policy_path.resolve().is_relative_to(root.resolve()) else str(policy_path)
    policy_raw = _read_policy_bytes(policy_path)
    baseline = {
        "schema": "flowspec.test_custody_baseline", "schema_version": 2,
        "generator_version": "2", "policy_version": policy["policy_version"],
        "source_sha": source_sha, "policy_path": policy_rel,
        "policy_sha256": sha256(policy_raw),
        "legacy": {"candidate_sha": LEGACY_CANDIDATE, "red_sha": LEGACY_RED, "baseline_sha256": LEGACY_SHA256},
        "control_self_exclusion": policy["control_baseline"],
        **details,
        "cargo_inventory": {"command": "cargo test --workspace --all-features -- --list",
                            "platform_contract": policy["cargo_inventory"]["platform_contract"],
                            "executable_tests": identities},
        "executable_identities": identities, "rustdoc_identities": docs,
    }
    baseline["counts"] = {"whole_files": len(details["whole_files"]),
                          "rust_regions": len(details["rust_regions"]),
                          "executable_identities": len(identities)}
    return baseline

BASELINE_KEYS = {"schema", "schema_version", "generator_version", "policy_version", "source_sha",
                 "policy_path", "policy_sha256", "legacy", "control_self_exclusion", "whole_files",
                 "rust_regions", "external_test_modules", "rustdoc_inventory", "tracked_test_paths",
                 "cargo_inventory", "executable_identities", "rustdoc_identities", "counts"}


def baseline_object(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise CustodyError("schema_duplicate_path baseline_duplicate_path", key)
        result[key] = value
    return result


def validate_baseline(stored: object) -> dict:
    if not isinstance(stored, dict):
        raise CustodyError("baseline_schema_type")
    # Legacy adversarial controls still need stable, specific diagnostics.
    legacy_paths = stored.get("tracked_test_paths")
    if isinstance(legacy_paths, list):
        if len(legacy_paths) != len(set(map(str, legacy_paths))):
            raise CustodyError("baseline_duplicate_path")
        for path in legacy_paths:
            if not isinstance(path, str) or _is_noncanonical_policy_path(path, False):
                raise CustodyError("baseline_noncanonical_path", str(path))
    if isinstance(stored.get("files"), dict) and isinstance(legacy_paths, list):
        for path in stored["files"]:
            if path not in legacy_paths:
                raise CustodyError("baseline_phantom_path", path)
    unknown = set(stored) - BASELINE_KEYS
    if unknown:
        raise CustodyError("schema_unknown_field", sorted(unknown)[0])
    missing = BASELINE_KEYS - set(stored)
    if missing:
        raise CustodyError("baseline_missing_required_key", sorted(missing)[0])
    if not isinstance(stored["schema_version"], int) or isinstance(stored["schema_version"], bool):
        raise CustodyError("schema_invalid_type", "schema_version")
    if stored["schema_version"] != 2 or stored["schema"] != "flowspec.test_custody_baseline":
        raise CustodyError("schema_invalid_binding", "schema_version")
    for key, expected_type in (("schema", str), ("generator_version", str),
                               ("policy_version", str), ("policy_path", str),
                               ("policy_sha256", str), ("source_sha", str),
                               ("whole_files", dict), ("rust_regions", list),
                               ("rustdoc_inventory", list), ("rustdoc_identities", list),
                               ("external_test_modules", dict), ("cargo_inventory", dict),
                               ("counts", dict), ("legacy", dict),
                               ("control_self_exclusion", dict),
                               ("tracked_test_paths", list), ("executable_identities", list)):
        if not isinstance(stored[key], expected_type):
            raise CustodyError("schema_invalid_type", key)
    if not SHA_RE.fullmatch(stored["source_sha"]):
        raise CustodyError("schema_invalid_type", "source_sha")
    if not SHA256_RE.fullmatch(stored["policy_sha256"]):
        raise CustodyError("schema_invalid_sha256", "policy_sha256")
    for key, fields in (("cargo_inventory", {"command", "platform_contract", "executable_tests"}),
                        ("legacy", {"candidate_sha", "red_sha", "baseline_sha256"}),
                        ("counts", {"whole_files", "rust_regions", "executable_identities"}),
                        ("control_self_exclusion", {"path", "excluded_from_own_file_hash_map", "self_exclusion_reason"})):
        if set(stored[key]) - fields:
            raise CustodyError("schema_unknown_field", key)
        if fields - set(stored[key]):
            raise CustodyError("baseline_missing_required_key", key)
    if not isinstance(stored["cargo_inventory"]["platform_contract"], dict):
        raise CustodyError("schema_invalid_type", "platform_contract")
    if set(stored["cargo_inventory"]["platform_contract"]) - {"os", "workspace", "all_features"}:
        raise CustodyError("schema_unknown_field", "platform_contract")
    if (stored["cargo_inventory"]["platform_contract"] !=
            {"os": "linux", "workspace": True, "all_features": True}):
        raise CustodyError("schema_invalid_binding", "platform_contract")
    if (stored["cargo_inventory"]["command"] != "cargo test --workspace --all-features -- --list"
            or stored["generator_version"] != "2"):
        raise CustodyError("schema_invalid_binding", "command")
    for key in ("whole_files", "external_test_modules"):
        for path, digest in stored[key].items():
            if not isinstance(path, str) or _is_noncanonical_policy_path(path, False):
                raise CustodyError("schema_noncanonical_path baseline_noncanonical_path", str(path))
            if not isinstance(digest, str) or not SHA256_RE.fullmatch(digest):
                raise CustodyError("schema_invalid_sha256", path)
    for path in stored["tracked_test_paths"]:
        if not isinstance(path, str) or _is_noncanonical_policy_path(path, False):
            raise CustodyError("schema_noncanonical_path baseline_noncanonical_path", str(path))
    if len(stored["tracked_test_paths"]) != len(set(stored["tracked_test_paths"])):
        raise CustodyError("schema_duplicate_path baseline_duplicate_path")
    regions = stored["rust_regions"]
    if any(not isinstance(item, dict) for item in regions):
        raise CustodyError("schema_invalid_type", "rust_regions")
    if len(regions) != len({json.dumps(item, sort_keys=True) for item in regions}):
        raise CustodyError("schema_duplicate_region")
    identities = stored["executable_identities"]
    if any(not isinstance(item, str) for item in identities):
        raise CustodyError("schema_invalid_type", "executable_identities")
    if len(identities) != len(set(identities)):
        raise CustodyError("schema_duplicate_identity")
    return stored


def verify_baseline(root: Path, paths: list[str], stored: object, policy: dict,
                    policy_path: Path) -> list[str]:
    stored = validate_baseline(stored)
    errors = []
    source_sha = stored["source_sha"]
    if not is_ancestor(root, source_sha, current_head(root)):
        return [f"ERROR: baseline_source_not_ancestor: {source_sha}"]
    if stored["policy_sha256"] != sha256(_read_policy_bytes(policy_path)):
        errors.append("ERROR: policy_hash_mismatch")
    if stored["policy_version"] != policy["policy_version"] or stored["legacy"] != {
            "candidate_sha": LEGACY_CANDIDATE, "red_sha": LEGACY_RED,
            "baseline_sha256": LEGACY_SHA256}:
        errors.append("ERROR: schema_invalid_binding")
    current = inventory(root, policy, paths)
    for key in ("whole_files", "rust_regions"):
        if not stored[key] and current[key]:
            raise CustodyError("schema_empty_universe", key)
    if not stored["executable_identities"]:
        identities_for_empty, _ = executable_inventory(root, paths)
        if identities_for_empty:
            raise CustodyError("schema_empty_universe", "executable_identities")
    for key in ("whole_files", "external_test_modules"):
        stored_map = stored[key]
        current_map = current[key]
        if set(stored_map) != set(current_map):
            for path in sorted(set(stored_map) - set(current_map)):
                errors.append(f"ERROR: baseline_phantom_path: {path}")
            for path in sorted(set(current_map) - set(stored_map)):
                errors.append(f"ERROR: baseline_missing_path: tracked path not in baseline: {path}")
        for path in sorted(set(stored_map) & set(current_map)):
            try:
                blob_hash = sha256(git_bytes(root, "show", f"{source_sha}:{path}"))
            except CustodyError:
                errors.append(f"ERROR: stored_hash_forged: {path}")
                continue
            if stored_map[path] != blob_hash:
                errors.append(f"ERROR: baseline_hash_mismatch: baseline_source_mismatch: stored_hash_forged: {path}: {source_sha}")
            if stored_map[path] != current_map[path]:
                errors.append(f"ERROR: test byte drift: {path}")
                if path in current["external_test_modules"]:
                    errors.append(f"ERROR: external_test_module_drift: {path}")
    for key in ("rust_regions", "rustdoc_inventory"):
        if stored[key] != current[key]:
            old_by_path = {}
            new_by_path = {}
            for entry in stored[key]:
                old_by_path.setdefault(entry["path"], []).append(entry)
            for entry in current[key]:
                new_by_path.setdefault(entry["path"], []).append(entry)
            for path in sorted(set(old_by_path) | set(new_by_path)):
                if old_by_path.get(path) != new_by_path.get(path):
                    errors.append(f"ERROR: region_drift: {path} rust_region_drift")
    if stored["tracked_test_paths"] != current["tracked_test_paths"]:
        errors.append("ERROR: tracked path drift: " + ", ".join(sorted(set(stored["tracked_test_paths"]) ^ set(current["tracked_test_paths"]))))
    if not errors:
        for key in ("whole_files", "rust_regions", "executable_identities"):
            count = stored["counts"].get(key)
            if not isinstance(count, int) or isinstance(count, bool) or count != len(stored[key]):
                errors.append(f"ERROR: schema_count_mismatch: {key}")
    identities, docs = executable_inventory(root, paths)
    if stored["executable_identities"] != identities or stored["cargo_inventory"].get("executable_tests") != identities:
        errors.append("ERROR: executable_identity_mismatch")
    if stored["rustdoc_identities"] != docs:
        errors.append("ERROR: rustdoc_identity_mismatch")
    return errors


def write_atomic(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".custody-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def main() -> int:
    parser = argparse.ArgumentParser(description="FLOWSpec sacred-test custody checker")
    parser.add_argument("--root", type=Path, default=Path("."))
    parser.add_argument("--policy", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--generate-baseline", type=Path)
    parser.add_argument("--check-markers", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    count = sum((args.baseline is not None, args.generate_baseline is not None,
                 args.check_markers, args.self_test))
    if count != 1:
        reason = "exactly_one_action_required: action_ambiguous" if count > 1 else "exactly_one_action_required"
        print(f"ERROR: {reason}", file=sys.stderr)
        return 2
    if args.policy is None:
        print("ERROR: policy_required", file=sys.stderr)
        return 2
    if args.self_test:
        print("ERROR: self_test_noop_rejected", file=sys.stderr)
        return 2
    root = args.root.resolve()
    try:
        policy = load_policy(root, args.policy)
        policy_path = _policy_path(root, args.policy)
        paths = get_tracked_files(root)
        if args.check_markers:
            findings = find_failures(root, paths)
            for finding in findings:
                print(finding, file=sys.stderr)
            return 1 if findings else 0
        destination = args.generate_baseline
        if destination is not None:
            if not destination.is_absolute():
                destination = root / destination
            if not clean_source(root, destination):
                raise CustodyError("baseline_generation_dirty_source dirty_source_tree")
            baseline = compute_baseline(root, policy, paths, policy_path)
            write_atomic(destination, (json.dumps(baseline, indent=2, sort_keys=True) + "\n").encode())
            return 0
        assert args.baseline is not None
        baseline_path = args.baseline if args.baseline.is_absolute() else root / args.baseline
        try:
            stored = json.loads(baseline_path.read_text(), object_pairs_hook=baseline_object)
        except (OSError, UnicodeDecodeError, json.JSONDecodeError):
            raise CustodyError("baseline_malformed_json") from None
        errors = verify_baseline(root, paths, stored, policy, policy_path)
        for error in errors:
            print(error, file=sys.stderr)
        return 1 if errors else 0
    except PolicyLoadError as error:
        detail = f": {error.pointer}" if error.pointer is not None else ""
        print(f"ERROR: {error.reason}{detail}", file=sys.stderr)
        return 1
    except CustodyError as error:
        print(f"ERROR: {error.path}:0: {error.reason}" if error.path else f"ERROR: {error.reason}", file=sys.stderr)
        return 1
    except (OSError, ValueError, TypeError, RecursionError) as error:
        print(f"ERROR: custody_internal_loss: {type(error).__name__}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
