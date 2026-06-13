# git SHA: 4a937ed4f6e246e10a462c337360a817357c060c
"""REPL support (mirrors src/repl.rs).

Reads blocks from stdin, executes on the ##REPL_EXEC## sentinel,
and keeps the interpreter alive between blocks.
"""
from __future__ import annotations
import sys

REPL_EXEC_SENTINEL = "##REPL_EXEC##"


def run_repl() -> None:
    """Run an interactive REPL that reads Arrow source from stdin."""
    from .parser import parse, ParseError
    from .type_check import TypeChecker, StaticTypeError
    from .interpreter import Interpreter
    from .interpreter.exceptions import RaiseSignal, InterpreterError

    interp = Interpreter()
    accumulated_lines: list[str] = []

    print("Arrow REPL (Python impl) — enter code, finish block with ##REPL_EXEC##")

    for raw_line in sys.stdin:
        line = raw_line.rstrip("\n")
        if line == REPL_EXEC_SENTINEL:
            source = "\n".join(accumulated_lines)
            accumulated_lines.clear()
            if not source.strip():
                continue
            try:
                stmts = parse(source, "<repl>")
            except ParseError as e:
                print(f"ParseError: {e}", file=sys.stderr)
                continue

            errors = TypeChecker.check(stmts)
            if errors:
                for err in errors:
                    print(str(err), file=sys.stderr)
                continue

            try:
                interp.exec_stmts(stmts)
            except RaiseSignal as e:
                print(f"Uncaught exception: {e}", file=sys.stderr)
            except InterpreterError as e:
                print(f"RuntimeError: {e}", file=sys.stderr)
            except SystemExit:
                raise
            except Exception as e:
                print(f"InternalError: {e}", file=sys.stderr)
        else:
            accumulated_lines.append(line)
