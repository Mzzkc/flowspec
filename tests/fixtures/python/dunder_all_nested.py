# PLANTED FACTS:
# - __all__ inside a class is NOT a module-level __all__
# - Module-level import should still be phantom (no module-level __all__)
from utils import helper

class Config:
    __all__ = ["helper"]  # This is a class attribute, NOT module __all__
