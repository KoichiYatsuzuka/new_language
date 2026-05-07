# stdlib/error.tl — Error trait and built-in exception hierarchy
#
# NOTE: This file is documentation only.
# The Error trait and all built-in exception classes are pre-registered
# in the interpreter (src/interpreter.rs) and parser (src/parser.rs)
# at startup. There is no import system yet, so this file is not loaded
# at runtime — it exists to document the specification.

# ---------------------------------------------------------------------------
# Error trait (built-in, always in scope)
# ---------------------------------------------------------------------------
# Inheriting from Error marks a class as an exception type.
# Fields `code_context`, `file`, `line`, and `col` are populated automatically
# at `raise` time by the interpreter. Only `message` must be set in __init__.
#
# trait Error:
#     let  message:      str   # set by the exception constructor
#     mut  code_context: str   # ~5 surrounding source lines at the raise site
#     mut  file:         str   # source file name of the raise statement
#     mut  line:         int   # line number of the raise statement
#     mut  col:          int   # column number of the raise statement

# ---------------------------------------------------------------------------
# Built-in exception classes (pre-registered, all inherit from Error)
# ---------------------------------------------------------------------------
#
#   Exception            - generic base exception
#   ValueError           - invalid value
#   TypeError            - wrong type
#   NameError            - name not found
#   AttributeError       - attribute not found on object
#   IndexError           - sequence index out of range
#   KeyError             - mapping key not found
#   ZeroDivisionError    - division or modulo by zero
#   RuntimeError         - generic runtime error
#   StopIteration        - raised to signal end of iteration
#   NotImplementedError  - operation not implemented
#   OverflowError        - arithmetic result too large
#   IOError              - I/O operation failed
#   OSError              - OS-level error (alias for IOError)
#   AssertionError       - assertion failed
#   ArithmeticError      - generic arithmetic error

# ---------------------------------------------------------------------------
# User-defined exceptions
# ---------------------------------------------------------------------------
# Inherit from the Error trait to create custom exception types.
# The auto-generated __init__ accepts a `message: str` parameter.
#
# class ValidationError(Error):
#     pass
#
# class NetworkError(Error):
#     pass
#
# raise ValidationError("field 'name' is required")

# ---------------------------------------------------------------------------
# Usage examples
# ---------------------------------------------------------------------------

# Raising a built-in exception:
#   raise ValueError("must be positive")

# Catching a specific exception:
#   try:
#       raise TypeError("wrong")
#   except TypeError as e:
#       print(e.message, e.file, e.line)

# Catching any Error (all built-in and user-defined errors inherit from Error):
#   try:
#       raise RuntimeError("oops")
#   except Error:
#       print("some error occurred")

# Bare except — catches absolutely everything (including non-Error raises):
#   try:
#       raise RuntimeError("oops")
#   except:
#       print("something went wrong")

# finally block (always executed):
#   try:
#       raise ValueError("bad")
#   except ValueError:
#       pass
#   finally:
#       print("cleanup")

# bare raise (re-raise current exception):
#   try:
#       raise ValueError("v")
#   except ValueError as e:
#       raise   # propagates e to the caller
