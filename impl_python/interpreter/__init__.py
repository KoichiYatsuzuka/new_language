"""test_lang interpreter package."""
from .interpreter import Interpreter
from .value import Value, display, type_name
from .exceptions import RaiseSignal, InterpreterError


def run(stmts: list, filename: str = "<input>") -> None:
    """Execute a parsed program."""
    interp = Interpreter()
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
