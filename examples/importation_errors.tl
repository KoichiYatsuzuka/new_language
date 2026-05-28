# importation_errors.tl — expected import errors.
#
# Each block causes the interpreter to exit with an error.
# Comment out earlier blocks to test later ones.

# ── Error 1: import[tlc] when no .tlc file exists ────────────────────────────
# ParseError: no .tlc file found for "nonexistent_module"
import[tlc] nonexistent_module

# ── Error 2: import[rs] — wrong argument type ─────────────────────────────────
# import[rs] libm[0.2]
# print(libm.sin("hello"))   # TypeError: wrong arg type (str instead of float)

# ── Error 3: import[rs] — wrong argument count ────────────────────────────────
# import[rs] libm[0.2]
# print(libm.pow(2.0))       # TypeError: pow expects 2 args, got 1
