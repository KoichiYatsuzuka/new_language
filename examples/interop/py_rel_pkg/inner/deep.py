# deep.py — 1 つ上（py_rel_pkg/）を `..` で指す（Python の相対 import の例題）。

from ..helper import double


def quad(x):
    return double(double(x))
