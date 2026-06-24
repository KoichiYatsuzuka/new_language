# git SHA: 08f19f554735e8588bc1f4bd2e2b300b43e4a31a
"""Arrow interpreter package."""
from .interpreter import Interpreter
from .value import Value, display, type_name
from .exceptions import RaiseSignal, InterpreterError


def run(stmts: list, filename: str = "<input>") -> None:
    """Execute a parsed program."""
    from pathlib import Path
    interp = Interpreter()
    # Populate search dirs so cs-dll (and rs) bridge DLLs can be found
    src_path = Path(filename)
    if src_path.exists():
        interp._python_search_dirs = [src_path.parent]
    elif filename not in ("<input>", "<repl>"):
        # Non-existent path; try parent directory anyway
        interp._python_search_dirs = [src_path.parent]
    try:
        interp.exec_stmts(stmts)
    except RaiseSignal as exc:
        msg = exc.message or display(exc.exception_value) if exc.exception_value is not None else "Exception"
        print(f"Unhandled exception: {msg}")
        raise SystemExit(1)
    except RuntimeError as e:
        print(f"RuntimeError: {e}")
        raise SystemExit(1)
    except InterpreterError as e:
        print(f"InterpreterError: {e}")
        raise SystemExit(1)


__all__ = ["Interpreter", "run", "Value", "display", "type_name"]
