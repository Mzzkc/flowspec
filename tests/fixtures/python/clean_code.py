# PLANTED FACTS:
# - All functions are called. All imports are used. Module is connected.
# - ZERO diagnostics expected from ANY pattern.

from pathlib import Path

def read_file(path: str) -> str:
    """Called by transform_data — not dead code."""
    return Path(path).read_text()

def transform_data(raw: str) -> list:
    """Called by main — not dead code. Calls read_file."""
    content = read_file(raw)
    return content.split("\n")

def main():
    """Entry point. Calls transform_data."""
    result = transform_data("input.txt")
    return result
