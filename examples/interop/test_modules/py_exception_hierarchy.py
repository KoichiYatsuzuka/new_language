# py_exception_hierarchy.py — Python の例外クラスの継承（フェーズ10 10-19）。
#
# 使う側: examples/interop/py_exception_hierarchy.ar


class AppError(Exception):
    pass


class ConfigError(ValueError):
    pass


class MissingKey(ConfigError):
    pass


def classify(kind):
    try:
        if kind == 1:
            raise MissingKey("missing key")
        elif kind == 2:
            raise ConfigError("bad config")
        elif kind == 3:
            raise AppError("app failure")
        else:
            raise KeyError("k")
    except ValueError as e:
        return "ValueError branch"
    except Exception as e:
        return "Exception branch"


def run():
    return ", ".join([classify(1), classify(2), classify(3), classify(4)])


def fail():
    raise MissingKey("raised in python")
