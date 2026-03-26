# PLANTED FACTS:
# - Integration: both __all__ and TYPE_CHECKING in same file
# - helper in __all__ → NOT phantom
# - TYPE_CHECKING itself → NOT phantom
# - PathLike in TYPE_CHECKING block → NOT phantom (type-only)
# - unused_thing → TRUE phantom
from typing import TYPE_CHECKING
from utils import helper, unused_thing

if TYPE_CHECKING:
    from os import PathLike

__all__ = ["helper"]
