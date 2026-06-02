# git SHA: 4a937ed4f6e246e10a462c337360a817357c060c
"""Native API stubs (mirrors src/interpreter/native_api.rs).

The Rust implementation provides a handle-based ABI for native compiled functions.
This module is a stub — loading native C libraries is not implemented in Python.
"""


def load_native_module(path: str) -> None:
    raise NotImplementedError(
        "native module loading has not been implemented in the Python interpreter"
    )
