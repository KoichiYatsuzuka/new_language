# git SHA: 33ef765a635dee99b50fccb937129e07ae6bdefb
"""Partial compiler package stubs (mirrors src/partial_compiler/mod.rs).

The Rust implementation compiles .ar modules to native machine code via LLVM/rustc.
This package is a stub — native compilation is not implemented in Python.
"""


def compile_module(source_path: str) -> None:
    raise NotImplementedError(
        "--compile has not been implemented in the Python interpreter"
    )
