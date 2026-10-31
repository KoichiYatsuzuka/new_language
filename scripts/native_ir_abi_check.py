"""native_ir_abi_check.py - check that every direct call to an internal function (@X_impl / @X_fast) in a generated
LLVM IR module matches the function's definition (return type and every argument type).

    python scripts/native_ir_abi_check.py <file.ll> [<file.ll> ...]

Driven by scripts/check_native_ir_abi.ps1 (native_object_access_plan.md 4-1). LLVM accepts a mismatched direct
call through an opaque pointer, so a codegen bug here does not fail clang; it reads garbage at run time
(task 3-2 hit this as `float * None`). Exit code 1 if any mismatch is found.
"""
import re
import sys

DEF = re.compile(r'^define\s+(?:internal\s+|dllexport\s+)?(\S+)\s+@(\w+_(?:impl|fast))\((.*)\)\s*\{')
CALL = re.compile(r'call\s+(\S+)\s+@(\w+_(?:impl|fast))\((.*)\)')


def split_args(s):
    out, depth, cur = [], 0, ''
    for ch in s:
        if ch in '([{':
            depth += 1
        elif ch in ')]}':
            depth -= 1
        if ch == ',' and depth == 0:
            out.append(cur.strip())
            cur = ''
        else:
            cur += ch
    if cur.strip():
        out.append(cur.strip())
    return out


def arg_type(a):
    return a.split()[0] if a else ''


bad = 0
for path in sys.argv[1:]:
    text = open(path, encoding='utf-8', errors='replace').read().splitlines()
    defs = {}
    for line in text:
        m = DEF.match(line)
        if m:
            defs[m.group(2)] = (m.group(1), [arg_type(a) for a in split_args(m.group(3))])
    calls = 0
    for n, line in enumerate(text, 1):
        m = CALL.search(line)
        if not m:
            continue
        calls += 1
        ret, name, args = m.group(1), m.group(2), [arg_type(a) for a in split_args(m.group(3))]
        if name not in defs:
            print(f'{path}:{n}: call to undefined {name}')
            bad += 1
            continue
        dret, dargs = defs[name]
        if ret != dret or args != dargs:
            print(f'{path}:{n}: {name}: call {ret}({", ".join(args)}) vs define {dret}({", ".join(dargs)})')
            bad += 1
    print(f'{path}: {len(defs)} internal function(s), {calls} direct call(s)')
print(f'ABI-CHECK: {bad} mismatch(es)')
sys.exit(1 if bad else 0)
