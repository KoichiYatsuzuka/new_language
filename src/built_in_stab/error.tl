# Built-in error classes pre-registered by the interpreter.
# Every class implicitly bases on the `Error` trait.
# Fields `code_context`, `file`, `line`, and `col` are populated
# automatically by the interpreter at raise time.

class Exception(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class ValueError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class TypeError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class NameError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class AttributeError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class IndexError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class KeyError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class ZeroDivisionError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class RuntimeError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class StopIteration(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class NotImplementedError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class OverflowError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class IOError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class OSError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class AssertionError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class ArithmeticError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...


class AccessError(Error):
    let message: str
    mut code_context: str
    mut file: str
    mut line: int
    mut col: int

    fn __init__(mut self, message: str):
        ...
