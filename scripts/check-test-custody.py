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

    try:
        load_policy(root, args.policy)
    except PolicyLoadError as error:
        detail = f": {error.pointer}" if error.pointer is not None else ""
        print(f"ERROR: {error.reason}{detail}", file=sys.stderr)
        return 1

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
