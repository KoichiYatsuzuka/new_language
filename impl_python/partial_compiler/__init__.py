# git SHA: 08f19f554735e8588bc1f4bd2e2b300b43e4a31a
"""Partial compiler package stubs (mirrors src/partial_compiler/mod.rs).

The Rust implementation compiles .tl modules to native machine code via LLVM/rustc.
This package is a stub — native compilation is not implemented in Python.
"""


def compile_module(source_path: str) -> None:
    raise NotImplementedError(
        "--compile has not been implemented in the Python interpreter"
    )
