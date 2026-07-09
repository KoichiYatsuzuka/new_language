---
name: codebase-map
description: Use when you need to know where a file, module, or subsystem lives in the Arrow repository (Rust src/, Python impl_python/, examples/, vscode-extension/), or what a given file is responsible for. Read this before navigating an unfamiliar part of the tree instead of globbing blindly.
---

# Directory Structure

```text
Arrow/
├── src/
│   ├── main.rs              # Entry point / argument parsing / error formatting
│   ├── token.rs             # Token enum / Span / Spanned definitions
│   ├── ast.rs                # AST node definitions (with embedded Span)
│   ├── python_converter.rs  # Python source converter utilities
│   ├── repl.rs               # Interactive REPL implementation
│   ├── frontend_tests.rs    # Lexer/parser/type-checker tests
│   ├── lexer/               # Lexer module
│   │   ├── mod.rs           # Lexer entry point (returns Vec<Spanned>)
│   │   ├── chars.rs         # Character utilities
│   │   ├── keyword.rs       # Keyword recognition
│   │   ├── literal.rs       # Numeric / string literal scanning
│   │   ├── math.rs          # Operator scanning
│   │   ├── scan.rs          # Main scan loop
│   │   └── symbol.rs        # Symbol / punctuation scanning
│   ├── parser/              # Parser module
│   │   ├── mod.rs           # Parser entry point (input: Vec<Spanned>)
│   │   ├── stmts.rs         # Statement parsing
│   │   ├── exprs.rs         # Expression parsing
│   │   ├── classes.rs       # Class / trait parsing
│   │   ├── types.rs         # Type annotation parsing
│   │   └── imports.rs       # Import statement parsing
│   ├── type_check/          # Static type checker module
│   │   ├── mod.rs           # Type checker entry point
│   │   ├── stmt.rs          # Statement type checking
│   │   ├── infer.rs         # Type inference
│   │   ├── types.rs         # Type representations
│   │   ├── errors.rs        # StaticTypeError definitions
│   │   ├── scope.rs         # Scope / environment
│   │   ├── type_utils.rs    # Type utility functions
│   │   ├── call_check.rs    # Function call type checking
│   │   ├── binop.rs         # Binary operator type checking
│   │   └── decorator.rs     # Decorator type checking
│   ├── interpreter.rs       # Interpreter entry point (re-exports interpreter/ module)
│   ├── interpreter/         # Interpreter module
│   │   ├── eval.rs          # Expression evaluation
│   │   ├── exec.rs          # Statement execution
│   │   ├── functions.rs     # Function / closure / generator execution
│   │   ├── classes.rs       # Class / trait execution
│   │   ├── templates.rs     # Template instantiation
│   │   ├── ops.rs           # Operator implementations
│   │   ├── str_methods.rs   # String method dispatch
│   │   ├── scope.rs         # Lexical scope / environment
│   │   ├── exceptions.rs    # Exception handling (try/except/finally/raise)
│   │   ├── async_mgr.rs     # AsyncManager and task submission
│   │   ├── native_api.rs    # Handle arena, TlCallbacks struct, C callback impls
│   │   ├── py_interop.rs    # Python interop (import[py] / import[py-int])
│   │   ├── debugger.rs      # break_point debugger support
│   │   ├── msvc_errors.rs   # MSVC-style error formatting
│   │   └── tests.rs         # Interpreter integration tests
│   ├── partial_compiler/    # --compile subsystem
│   │   ├── mod.rs           # Entry point: orchestrates compile pipeline
│   │   ├── codegen.rs       # Rust source code generator (fn → handle-based i64 ABI)
│   │   ├── module_compiler.rs # Writes .arc (v0/v1) and .ars; v1 embeds DLL inside .arc
│   │   ├── rs_loader.rs     # Loads and links compiled Rust shared libraries
│   │   └── stub_gen.rs      # .ars stub file generator
│   └── built_in_stab/       # Built-in type stubs (used by VS Code extension)
│       ├── built_in_const.ars
│       ├── built_in_type.ars
│       ├── basic_traits.ars
│       └── error.ars
├── spec.md                  # Language specification
├── examples/
│   ├── ....ar                 # Example files
│   ├── ..._error.ar           # Error examples
│   ├── test_modules/          # modules used by importation.ar
│   ├── geometry/              # example package
│   └── archived/              # older examples (kept for reference)
├── impl_python/               # Python implementation of the interpreter
│   ├── __main__.py            # CLI entry point: python -m impl_python <file.ar>
│   ├── __init__.py
│   ├── token.py               # Token / Span / Spanned definitions
│   ├── ast.py                 # AST node dataclasses
│   ├── repl.py                # Interactive REPL
│   ├── lexer/                 # Lexer module (mirrors src/lexer/)
│   ├── parser/                # Parser module (mirrors src/parser/)
│   ├── type_check/            # Static type checker module (mirrors src/type_check/)
│   ├── interpreter/           # Tree-walk interpreter package
│   │   ├── __init__.py        # run(stmts) entry point
│   │   ├── interpreter.py     # Interpreter class (exec / eval)
│   │   ├── value.py           # Runtime value types (TlList, TlClass, TlInstance, …)
│   │   ├── env.py             # Lexical scope / Environment
│   │   ├── exceptions.py      # Control-flow signals (ReturnSignal, RaiseSignal, …)
│   │   ├── native_api.py      # Native API bindings
│   │   └── builtins.py        # Built-in functions and collection method dispatch
│   └── partial_compiler/      # Partial compiler (mirrors src/partial_compiler/)
│       ├── codegen.py
│       ├── module_compiler.py
│       └── stub_gen.py
└── vscode-extension/          # VS Code extension (type inference inline hints)
    └── src/
        ├── extension.ts
        └── type_infer.ts
```
