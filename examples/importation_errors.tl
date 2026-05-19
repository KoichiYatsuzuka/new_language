# importation__errors.tl — expected import errors.
#
# Each block causes the interpreter to exit with an error.
# Comment out earlier blocks to test later ones.

# ── import[tlc] when no .tlc file exists ─────────────────────────────────────
# ParseError: no .tlc file found for "nonexistent_module"
import[tlc] nonexistent_module
