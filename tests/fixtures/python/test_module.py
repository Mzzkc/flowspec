# PLANTED FACTS:
# - Test functions that call into production code.
# - This module has zero external callers (tests don't get called by production).
# - MUST NOT be flagged as isolated_cluster (it's test infrastructure).
# - Test functions MUST NOT be flagged as data_dead_end.

from dead_code import active_function, main_handler

def test_active_function():
    assert active_function("  HELLO ") == "hello"

def test_main_handler():
    class MockRequest:
        body = "test data"
    result = main_handler(MockRequest())
    assert result["status"] == "ok"
