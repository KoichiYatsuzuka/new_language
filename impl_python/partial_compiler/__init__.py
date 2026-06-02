# git SHA: 4a937ed4f6e246e10a462c337360a817357c060c
"""Partial compiler package stubs (mirrors src/partial_compiler/mod.rs).

The Rust implementation compiles .hv modules to native machine code via LLVM/rustc.
This package is a stub — native compilation is not implemented in Python.
"""


def compile_module(source_path: str) -> None:
    raise NotImplementedError(
        "--compile has not been implemented in the Python interpreter"
    )
