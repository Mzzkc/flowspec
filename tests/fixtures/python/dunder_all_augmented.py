# PLANTED FACTS:
# - base_exports via __all__ = [...], extended via __all__ += [...]
# - Both Foo and Bar should be marked as used (re-exported)
# - Baz imported but not in any __all__ → TRUE phantom
from models import Foo, Bar, Baz

__all__ = ["Foo"]
__all__ += ["Bar"]
