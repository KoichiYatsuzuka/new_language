"""Command-line entry point for the Python implementation."""
import sys
import pathlib


def main() -> None:
    if len(sys.argv) < 2:
        print("Usage: python -m impl_python <file.tl>", file=sys.stderr)
        sys.exit(1)

    source_file = pathlib.Path(sys.argv[1])
    if not source_file.exists():
        print(f"Error: file not found: {source_file}", file=sys.stderr)
        sys.exit(1)

    source = source_file.read_text(encoding="utf-8")
    source_dir = str(source_file.parent)

    from .parser import parse, ParseError
    from .interpreter import run

    try:
        stmts = parse(source, source_file.name, source_dir)
    except ParseError as e:
        print(f"ParseError: {e}", file=sys.stderr)
        sys.exit(1)

    run(stmts, source_file.name)


if __name__ == "__main__":
    main()
