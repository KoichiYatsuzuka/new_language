# git SHA: 33ef765a635dee99b50fccb937129e07ae6bdefb
"""Scope / environment management for the interpreter."""
from __future__ import annotations
from .value import Value, MISSING, CapturedImm, CapturedMut, CapturedVar


class Environment:
    """Lexical scope stack.

    Each scope maps name → [value, is_mutable, Optional[mutable_cell]].
    The mutable_cell (a single-element list) is shared between the scope entry
    and any closure CapturedMut that refers to the same variable.
    """

    def __init__(self) -> None:
        # Each entry: dict[str, [Value, bool, list | None]]
        #   [0] = current value  (irrelevant when cell is not None)
        #   [1] = is_mutable
        #   [2] = shared mutable cell, or None for non-captured locals
        self._scopes: list[dict[str, list]] = [{}]

    # ------------------------------------------------------------------
    # Scope lifecycle
    # ------------------------------------------------------------------

    def push_scope(self) -> None:
        self._scopes.append({})

    def pop_scope(self) -> None:
        if len(self._scopes) > 1:
            self._scopes.pop()

    # ------------------------------------------------------------------
    # Variable declaration
    # ------------------------------------------------------------------

    def declare(self, name: str, value: Value, mutable: bool) -> None:
        """Declare a new variable in the current (innermost) scope."""
        self._scopes[-1][name] = [value, mutable, None]

    def declare_cell(self, name: str, cell: list, mutable: bool) -> None:
        """Declare a variable backed by an existing shared mutable cell."""
        self._scopes[-1][name] = [None, mutable, cell]

    # ------------------------------------------------------------------
    # Variable read
    # ------------------------------------------------------------------

    def get(self, name: str) -> Value:
        for scope in reversed(self._scopes):
            if name in scope:
                entry = scope[name]
                return entry[2][0] if entry[2] is not None else entry[0]
        raise RuntimeError(f"NameError: name '{name}' is not defined")

    def get_info(self, name: str) -> tuple[Value, bool]:
        """Return (value, is_mutable)."""
        for scope in reversed(self._scopes):
            if name in scope:
                entry = scope[name]
                val = entry[2][0] if entry[2] is not None else entry[0]
                return val, entry[1]
        raise RuntimeError(f"NameError: name '{name}' is not defined")

    def get_cell(self, name: str) -> list | None:
        """Return the shared mutable cell for a variable, or None."""
        for scope in reversed(self._scopes):
            if name in scope:
                return scope[name][2]
        return None

    def contains(self, name: str) -> bool:
        return any(name in scope for scope in self._scopes)

    # ------------------------------------------------------------------
    # Variable write
    # ------------------------------------------------------------------

    def assign(self, name: str, value: Value) -> None:
        """Assign to an existing variable. Raises if immutable or undefined."""
        for scope in reversed(self._scopes):
            if name in scope:
                entry = scope[name]
                if not entry[1]:
                    raise RuntimeError(
                        f"TypeError: cannot assign to immutable variable '{name}'"
                    )
                if entry[2] is not None:
                    entry[2][0] = value
                else:
                    entry[0] = value
                return
        raise RuntimeError(f"NameError: name '{name}' is not defined")

    def freeze(self, name: str) -> None:
        """Make a variable immutable (freeze)."""
        for scope in reversed(self._scopes):
            if name in scope:
                entry = scope[name]
                # Check if captured by a closure
                if entry[2] is not None:
                    raise RuntimeError(
                        f"TypeError: cannot freeze '{name}' because it is captured by a closure"
                    )
                entry[1] = False
                return
        raise RuntimeError(f"NameError: cannot freeze '{name}': not defined")

    # ------------------------------------------------------------------
    # Capture helpers (for closure creation)
    # ------------------------------------------------------------------

    def capture_all(self) -> dict[str, CapturedVar]:
        """Snapshot the current environment for closure capture."""
        captured: dict[str, CapturedVar] = {}
        for scope in self._scopes:
            for name, entry in scope.items():
                if name in captured:
                    continue
                if entry[1]:  # mutable
                    # Upgrade to shared cell if not already
                    if entry[2] is None:
                        cell: list = [entry[0]]
                        entry[2] = cell
                        entry[0] = None  # value now lives in cell
                    captured[name] = CapturedMut(entry[2])
                else:
                    val = entry[2][0] if entry[2] is not None else entry[0]
                    from .value import deep_clone
                    captured[name] = CapturedImm(deep_clone(val))
        return captured

    def apply_captured(self, captured: dict[str, CapturedVar]) -> None:
        """Install captured variables into the current scope."""
        for name, cv in captured.items():
            if isinstance(cv, CapturedMut):
                self._scopes[-1][name] = [None, True, cv.cell]
            else:
                self._scopes[-1][name] = [cv.value, False, None]
