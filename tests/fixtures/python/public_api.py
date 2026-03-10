# PLANTED FACTS:
# - Public functions with zero internal callers — they are library API surface.
# - data_dead_end should assign LOW confidence, NOT high.
# - _internal_helper: private, zero callers, should be HIGH confidence dead end.

def format_timestamp(ts):
    """Public API function. Zero internal callers. Intended for external use."""
    return ts.isoformat()

def parse_duration(s):
    """Public API function. Zero internal callers. Intended for external use."""
    parts = s.split(":")
    return int(parts[0]) * 3600 + int(parts[1]) * 60 + int(parts[2])

def _internal_helper():
    """Private. Zero callers. Should be HIGH confidence dead end."""
    return None
