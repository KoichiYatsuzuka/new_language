# py_qualified_except.py — Python の except m.Err:（python_builtins_plan.md のタスク 4-3）。
#
# 使う側: examples/interop/py_qualified_except.ar

import test_modules.py_exception_hierarchy as h


def guarded(kind):
    try:
        if kind == 1:
            h.fail()
        raise h.AppError("app failure")
    except h.ConfigError as e:
        return "ConfigError: " + str(e)
    except h.AppError as e:
        return "AppError: " + str(e)
