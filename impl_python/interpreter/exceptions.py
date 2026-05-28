# git SHA: 08f19f554735e8588bc1f4bd2e2b300b43e4a31a
"""Control-flow signals and language-level error types."""
from __future__ import annotations
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from .value import Value


# ---------------------------------------------------------------------------
# Control-flow signals (used as exceptions internally)
# ---------------------------------------------------------------------------

class ReturnSignal(Exception):
    def __init__(self, value: "Value") -> None:
        self.value = value


class BreakSignal(Exception):
    pass


class ContinueSignal(Exception):
    pass


class BlockReturnSignal(Exception):
    def __init__(self, value: "Value") -> None:
        self.value = value


class LoopYieldSignal(Exception):
    """Raised by loop_yield — caught by for/while expression handlers."""
    def __init__(self, value: "Value") -> None:
        self.value = value


class YieldSignal(Exception):
    """Raised by yield inside a gen function."""
    def __init__(self, value: "Value") -> None:
        self.value = value


class StopIterationSignal(Exception):
    pass


# ---------------------------------------------------------------------------
# Language-level exception (raise / try-except)
# ---------------------------------------------------------------------------

class RaiseSignal(Exception):
    """Carries a language-level exception value through the call stack."""
    def __init__(self, exception_value: "Value", message: str = "") -> None:
        self.exception_value = exception_value
        self.message = message
        super().__init__(message)


# ---------------------------------------------------------------------------
# Interpreter runtime error (bugs / unimplemented)
# ---------------------------------------------------------------------------

class InterpreterError(RuntimeError):
    pass
