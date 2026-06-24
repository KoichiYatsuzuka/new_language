# git SHA: a027318a14d1e2c2c3c63f50f37b36bf2c5aa10
"""Command-line entry point for the Python implementation (mirrors src/main.rs)."""
from __future__ import annotations
import sys
import pathlib


def main() -> None:
    args = sys.argv[1:]

    if not args:
        print("Usage: python -m impl_python [-src] <file.ar>", file=sys.stderr)
        print("       python -m impl_python --compile <file.ar>", file=sys.stderr)
        print("       python -m impl_python --repl", file=sys.stderr)
        sys.exit(1)

    # --compile flag: stub (not implemented)
    if args[0] == "--compile":
        print("--compile has not been implemented in the Python interpreter", file=sys.stderr)
        sys.exit(1)

    # --repl flag
    if args[0] == "--repl":
        from .repl import run_repl
        run_repl()
        return

    # Accept -src / --src as optional prefix
    if args[0] in ("-src", "--src") and len(args) >= 2:
        source_path = pathlib.Path(args[1])
    else:
        source_path = pathlib.Path(args[0])

    if not source_path.exists():
        print(f"Error: file not found: {source_path}", file=sys.stderr)
        sys.exit(1)

    source = source_path.read_text(encoding="utf-8")
    source_dir = source_path.parent

    from .parser import parse, ParseError
    from .type_check import TypeChecker
    from .interpreter import run

    try:
        stmts = parse(source, str(source_path), source_dir)
    except ParseError as e:
        print(f"ParseError: {e}", file=sys.stderr)
        sys.exit(1)

    errors = TypeChecker.check(stmts)
    if errors:
        for err in errors:
            print(str(err), file=sys.stderr)
        sys.exit(1)

    run(stmts, str(source_path))


if __name__ == "__main__":
    main()
