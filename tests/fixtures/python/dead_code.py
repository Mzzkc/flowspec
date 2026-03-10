# PLANTED FACTS:
# - unused_helper: private function, zero callers anywhere. TRUE POSITIVE for data_dead_end (HIGH confidence).
# - _private_util: underscore-prefix private, zero callers. TRUE POSITIVE for data_dead_end (HIGH confidence).
# - active_function: called by main_handler. TRUE NEGATIVE for data_dead_end.
# - main_handler: entry point (called from outside or as main). TRUE NEGATIVE for data_dead_end.

def unused_helper(x):
    """This function is never called from anywhere."""
    return x * 2

def _private_util():
    """Private utility, also never called."""
    return 42

def active_function(data):
    """Called by main_handler — not dead code."""
    return data.strip().lower()

def main_handler(request):
    """Entry point — called externally. Not dead code."""
    result = active_function(request.body)
    return {"status": "ok", "data": result}
