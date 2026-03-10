"""Module with dead code for testing data_dead_end diagnostic."""
import os

def active_function():
    """This is called by main."""
    return 42

def dead_function():
    """PLANTED: Never called - data_dead_end (high confidence)."""
    return "unreachable"

def another_dead_function(x):
    """PLANTED: Never called - data_dead_end (high confidence)."""
    return x * 2
