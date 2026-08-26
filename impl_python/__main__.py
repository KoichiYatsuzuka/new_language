# git SHA: 3361350159cad1a7fa5cd30901ff27a8f46bc688
"""Command-line entry point for the Python implementation (mirrors src/main.rs)."""
from __future__ import annotations
import sys
import pathlib


def _add_python_search_paths(source_dir: pathlib.Path) -> None:
    """Walk up from source_dir looking for ar_config.json and add its
    python.search_paths entries to sys.path (mirrors src/main.rs)."""
    import json
    try:
        d = source_dir.resolve()
    except OSError:
        d = source_dir
    while True:
        cfg_path = d / "ar_config.json"
        if cfg_path.exists():
            try:
                root = json.loads(cfg_path.read_text(encoding="utf-8"))
                paths = root.get("python", {}).get("search_paths", [])
                for p in paths:
                    pb = pathlib.Path(p)
                    abs_p = pb if pb.is_absolute() else d / pb
                    sys.path.insert(0, str(abs_p))
            except (OSError, json.JSONDecodeError, AttributeError):
                pass
            break
        parent = d.parent
        if parent == d:
            break
        d = parent


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

    # ar_config.json の python.search_paths を sys.path に追加する
    # （source_dir から上位へウォーク — mirrors src/main.rs）
    _add_python_search_paths(source_dir)

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
