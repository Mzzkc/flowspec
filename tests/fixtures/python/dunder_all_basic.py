# PLANTED FACTS:
# - helper re-exported via __all__ → NOT phantom
# - internal_only imported but not in __all__ and not used → TRUE phantom
from utils import helper
from utils import internal_only

__all__ = ["helper"]
