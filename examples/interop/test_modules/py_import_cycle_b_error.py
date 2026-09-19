# py_import_cycle_b_error.py — `py_import_cycle_a_error.py` と相互に import する側。

import test_modules.py_import_cycle_a_error as a


def b_val():
    return 2
