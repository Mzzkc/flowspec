# PLANTED FACTS:
# - Non-string items (42, None) must be skipped without error
# - "Valid" should still be extracted as an export
# - Non-existent "Ghost" creates a reference but won't resolve to anything
from utils import Valid

__all__ = [42, None, "Valid", "Ghost"]
