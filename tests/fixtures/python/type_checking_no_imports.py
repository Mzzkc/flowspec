# PLANTED FACTS:
# - TYPE_CHECKING block with no imports (just aliases)
# - Should not crash or produce spurious results
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    MyType = str
