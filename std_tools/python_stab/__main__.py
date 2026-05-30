"""Entry point: python -m std_tools.python_stab <file.py> [output.hvs]

If no output path is given, writes <file>.hvs alongside the source file.
Pass -p / --print to print the stub to stdout instead.
"""

import sys
from pathlib import Path
from .stub_gen import generate_stub


def main() -> None:
    args = sys.argv[1:]
    print_only = False
    filtered: list[str] = []
    for a in args:
        if a in ("-p", "--print"):
            print_only = True
        else:
            filtered.append(a)
    args = filtered

    if not args:
        print(
            "Usage: python -m std_tools.python_stab <file.py> [output.hvs] [-p|--print]",
            file=sys.stderr,
        )
        sys.exit(1)

    src = Path(args[0])
    if not src.exists():
        print(f"Error: {src} does not exist", file=sys.stderr)
        sys.exit(1)
    if src.suffix != ".py":
        print(f"Warning: {src} does not have a .py extension", file=sys.stderr)

    stub = generate_stub(str(src))

    if print_only:
        print(stub, end="")
        return

    out_path = Path(args[1]) if len(args) >= 2 else src.with_suffix(".hvs")
    out_path.write_text(stub, encoding="utf-8")
    print(f"Stub written to {out_path}")


if __name__ == "__main__":
    main()
