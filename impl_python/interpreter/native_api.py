# git SHA: 08f19f554735e8588bc1f4bd2e2b300b43e4a31a
"""Native API stubs (mirrors src/interpreter/native_api.rs).

The Rust implementation provides a handle-based ABI for native compiled functions.
This module is a stub — loading native C libraries is not implemented in Python.
"""


def load_native_module(path: str) -> None:
    raise NotImplementedError(
        "native module loading has not been implemented in the Python interpreter"
    )
