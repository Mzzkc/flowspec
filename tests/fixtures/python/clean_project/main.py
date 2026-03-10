"""A clean, well-structured Python module for testing."""

def greet(name: str) -> str:
    """Generate a greeting message."""
    return f"Hello, {name}!"

def farewell(name: str) -> str:
    """Generate a farewell message."""
    return f"Goodbye, {name}!"

def main():
    """Entry point."""
    print(greet("world"))
    print(farewell("world"))

if __name__ == "__main__":
    main()
