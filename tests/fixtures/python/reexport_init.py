# PLANTED FACTS:
# - `from utils import helper` where helper is re-exported via __all__.
#   Re-export IS the usage. MUST NOT be flagged as phantom.
# - `from utils import internal_only` — neither used nor re-exported. TRUE POSITIVE.

from utils import helper
from utils import internal_only

__all__ = ["helper"]
