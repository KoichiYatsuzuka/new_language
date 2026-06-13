"""Minimal stdin helper for spider_solitaire.hv.

Havakyrie has no built-in input(), so we bridge via Python.
All functions take no arguments to avoid the mut-parameter constraint.
"""

def read_cmd() -> str:
    """Read one command line from stdin (prompts with '> ')."""
    try:
        return input("> ")
    except EOFError:
        return "q"

def wait_enter() -> str:
    """Pause until the user presses Enter."""
    try:
        return input("Press Enter to continue...")
    except EOFError:
        return ""
