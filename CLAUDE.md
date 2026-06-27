# CLAUDE.md

Guidelines for Claude Code when working in this repository.

## Commands

### Rust implementation (primary)

```bash
cargo build                        # Compile
cargo run -- -src <file.ar>        # Execute a .ar file
cargo run -- <file.ar>             # Positional argument also supported
cargo run -- --repl                # Start interactive REPL
cargo test                         # Run all tests
cargo test <name>                  # Filter tests by partial name match
cargo clippy                       # Lint
cargo fmt                          # Format
cargo run -- --compile <file.ar>   # Partially compile a module (see below)
cargo run -- --compile-cs <file.dll> # Generate .ars stub from a .NET DLL (import[cs-dll]/import[cs-proc])
```

### Python implementation (`impl_python/`)

```bash
# Run from the repository root
python -m impl_python <file.ar>

# Examples
python -m impl_python examples/variable.ar
python -m impl_python examples/control_flow.ar
```

## Regulations
- Read .claude/settings.json and .claude/setting.local.json to check the permitted commands and avoid asking permissions by using the commands in the file.
- When a new grammer is implemented, an example code to check if it works must be generated in example folder. And if error pattern is implemented, an error example is also neede, whose name has "_error" at the last.
- When Python implementation is updated, also update the git SHA to track and syncronize the versions between the Rust-implementation and Python-implementation. 
- If running the same script(s) many times, make them .ps1 file to ease command permission.
- When VS code extension is updated, the compilation and the generation of VSIX file are required. To generate VSIX filee, run make-vsix.ps1.

## Project Overview

**Arrow** is a custom scripting language targeting LLVM IR.  
It aims to provide a Python-based syntax with static type checking and additional custom extensions.

- File extension: `.ar`
- Implementation language: **Rust** (main) and **Python** (`impl_python/`)
- Indentation-based block structure (Python-style)

## Directory Structure

```text
Arrow/
├── src/
│   ├── main.rs              # Entry point / argument parsing / error formatting
│   ├── token.rs             # Token enum / Span / Spanned definitions
│   ├── ast.rs               # AST node definitions (with embedded Span)
│   ├── python_converter.rs  # Python source converter utilities
│   ├── repl.rs              # Interactive REPL implementation
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

## Partial Compilation

A `.ar` module can be partially compiled to native machine code with `--compile`.  
This produces two files next to the source:

| Output file | Contents |
|-------------|----------|
| `{stem}.arc` | Compiled module: binary header + embedded source text, optionally with a native shared library embedded (v1 format) |
| `{stem}.ars` | Type stub: function/class/trait signatures with `...` bodies (used by the VS Code extension and the static type checker) |

### Canonical demo

`examples/test_modules/physics.ar` is the canonical module for demonstrating native compilation.  
`examples/importation.ar` (section 1) is the corresponding runner that shows the full workflow.

```bash
# Step 1 — run interpreted
cargo run --release -- examples/importation.ar

# Step 2 — compile the module
cargo run --release -- --compile examples/test_modules/physics.ar
# Output:
#   NativeLib: compiling 6 function(s): potential, kinetic, vel_dot, ...
#   NativeLib: 6 function(s) embedded in examples\test_modules\physics.arc
#   Compiled : examples\test_modules\physics.arc
#   Stub     : examples\test_modules\physics.ars

# Step 3 — run again with native dispatch (same command as Step 1)
cargo run --release -- examples/importation.ar
```

### How the compiled module is used

When a `.ar` file imports a module, the parser prefers `.arc` over `.ar`.  
If the `.arc` is v1, the embedded DLL is extracted to a temp file at runtime and loaded via `libloading`.  
Eligible functions are dispatched natively; all other functions tree-walk as usual.

```
import test_modules.physics                  # loads test_modules/physics.arc (parser)
test_modules.physics.total_energy(a, b, N)   # calls native code
```
The details for how it is implemented, refer to ./for_claude/partial_compile.md

## Importation of .ar, .py, .dll (C language), .lib, and .rs

Import syntax: `import[lang] module.path as alias` / `from module import[lang] Name`.  
The `[lang]` tag selects the source type; omitting it defaults to `ar-auto`.

| Tag | Loads |
|-----|-------|
| *(none)* | `.arc` preferred, falls back to `.ar` or `__init__.ar` |
| `hv` / `hvc` | Force `.ar` source only / force `.arc` compiled only |
| `py` | Python `.py` via converter |
| `py-int` | `.pyi`→`.py` for type checking only; runtime via PyO3 |
| `rs` | Rust crate — auto-compiles a wrapper DLL (requires `ar_config.json` with `rust.crates_path`) |
| `cpp-dll` / `cpp-lib` | C header (`Dir.Name` → `Dir/Name.h`) for type stubs; runtime via `cpp_bridge` |

For implementation details see `./for_claude/importation.md`.

## Implemented Features

### Lexical Analysis (`src/lexer/`)

- Supports all keywords, operators, and literals
- Indentation tracking (`INDENT` / `DEDENT` token generation)
- Ignores newlines inside parentheses
- Compound keywords: `not in`, `is not`, `yield from`
- Numeric literals: decimal, hexadecimal, octal, binary, underscore separators
- Strings: single quote, double quote, triple quote, escapes
- Adds `Span` (filename, line number, column number) to every token
- `Token::SelfType` (`Self`), `Token::NewType` (`new_type`), `Token::Static` (`static`)
- `Token::BlockReturn` (`block_return`), `Token::LoopYield` (`loop_yield`), `Token::Block` (`block`)
- `Token::Public` (`public`), `Token::Private` (`private`), `Token::Protected` (`protected`)
- `Token::LeftArrow` (`<-`)

### Parsing (`src/parser/`)

Recursive-descent parser. Input: `Vec<Spanned>` tokens. Output: `Vec<Stmt>` AST. Module loading also happens here — imported module ASTs are embedded in `Stmt::Import.body` at parse time.

- Declarations: `let` / `mut` / `const` / `static mut`; tuple unpack `let x, mut y = expr`
- Assignments: `x = expr`, compound `x += expr`, attribute `obj.x = expr`
- Expressions: full precedence chain (`or`→`and`→`not`→comparison→bitwise→arithmetic→unary→`**`→postfix); set `{a,b}` disambiguated from dict `{k:v}` by lookahead; slice `a[i:j:k]`; `is` / `is not` type guards; `in` / `not in` membership
- Control flow: `if/elif/else`, `for`, `while`, `match` (value-case and type-pattern arms, not mixed), `try/except/finally`, `raise`
- `if` / `for` / `while` / `match` / `block` can appear as expressions with optional `->Type` annotation; `parse_opt_return_type()` handles the `->` before `:`
- Functions: `fn` / `gen`; template params `[T: Trait]`; `let`/`mut` param qualifiers; default params (non-default after default is a parse error); abstract body `...`
- Classes / traits: inheritance, access-control sections (`public:` / `private:` / `protected:`), `Self`, `new_type`
- Async: `target <- async->T: body` → `Stmt::AsyncAssign`
- Parse-time checks: mixed `case`/`is` arms, `return` in `gen`, `mut` param in `gen`, `new_type` reassignment, missing trait method annotations

For full details see `./for_claude/parser.md`.

### Static Type Checking (`src/type_check/`)

Traverses the AST after parsing and before execution, collecting and reporting `StaticTypeError`s together.

- **Type guard narrowing**: when an `if` branch condition is `x is T` or `x is not T`, the variable `x` is re-declared with a narrowed type inside that branch's scope.
  - `x is T` → narrows `x` to `T` (works for primitives, classes, new_types, traits)
  - `x is not T` → requires `x` to be `Union` / `Optional`; narrows by removing `T` from the union members (e.g. `Option[int]` with `is not None` → `int`)
  - `x is not T` on a non-Union type → `StaticTypeError: IsNotOnNonUnion`
- **Function type checking**: typed function values (`function[let T]->R`, `function{let name:T}->R`) are statically checked at call sites for argument count, argument types, keyword argument names, and mutability (`mut` param requires a mutable variable argument)

### Interpreter (`src/interpreter/`)

Tree-walk interpreter. `exec(stmt)` / `eval(expr)` dispatch on the AST recursively. Lexical scopes are a `Vec<HashMap>` searched tail-to-head.

- **Values**: `Int`, `Float`, `Bool`, `Str`, `None`, `List`, `Dict`, `Tuple`, `Set`, `Slice`, `Function`, `Generator`, `Class`, `Instance`, `Namespace`, `NativeFn`, `PyObject`
- **Mutability**: `let` → immutable; `mut` → mutable (deep-copied on declaration); `freeze` makes a variable immutable; `static mut` → single shared cell keyed by source position
- **Closures**: `let` captures are deep-copied; `mut` captures share an `Rc<RefCell<Value>>` cell with the outer scope
- **Control-flow signals**: `return` via `ExecResult`; `break` via `BREAK_SENTINEL` error string propagating through `eval()`; `block_return` / `loop_yield` via thread-locals `BLOCK_YIELDS` and `RAISE_SENTINEL`
- **Exceptions**: `raise` / `try/except/finally` use a sentinel error string `"\x00__raise__:..."`; built-in classes (`ValueError`, `TypeError`, `KeyError`, …) pre-registered at startup
- **Access control**: `public` / `private` / `protected` enforced at runtime via `current_class` tracked on `Interpreter`; violation raises `AccessError`
- **Async**: `mng <- async->T: body` spawns an OS thread (`std::thread::spawn`); `mut` captures share Rc, `let` captures are deep-cloned before crossing the thread boundary
- **Native modules**: values cross the ABI as `i64` handles into a thread-local `VALUE_ARENA`; `ArCallbacks` struct passed to DLLs via `ar_init()`
- **Debugger**: `break_point` enters an interactive REPL; step mode driven by `DBG_MODE` thread-local

For full details see `./for_claude/interpreter.md`.

### VS Code Extension (`vscode-extension/`)

- Syntax highlighting for `.ar`
- Type inference inline hints

#### Standalone Debug Runner

A CLI tool that exercises the extension's analysis code against a `.ar` file without running VS Code. It outputs ANSI-coloured source with inlay hints inserted inline, hover balloon content for every symbol, and a diagnostics list.

**Setup (one-time after changing analysis code):**

```bash
cd vscode-extension
npm run compile:debug     # compiles to out_debug/ with ES2019 target
```

**Run against any `.ar` file:**

```bash
node run_debug.js <path/to/file.ar>
# example
node run_debug.js ../examples/importation.ar
```

**Output sections:**

| Section | What it shows |
|---------|---------------|
| `SOURCE` | Every source line with inlay hints inserted in cyan and semantic token colours (yellow = class/import, cyan = built-in type) |
| `HOVER REFERENCE` | Every symbol with its hover balloon content |
| `DIAGNOSTICS` | Errors / warnings with line:col |

**Colour legend (ANSI — requires a terminal that renders escape codes):**

- Yellow `■` — class name or imported identifier
- Cyan `■` — built-in type (`int`, `float`, `str`, …)
- Bright-cyan `■` — inlay hint (type inserted by inference)
- Red `■` / Yellow `■` — diagnostic error / warning

**How `import[rs]` types are resolved:**

The extension reads `ar_config.json` (walked up from the document directory) to find `rust.crates_path`, then parses the crate's `src/lib.rs` directly.  No `.ars` stub file is needed for Rust crates — the type information is read live from the Rust source.

**Re-compile reminder:**  Run `npm run compile:debug` whenever `analysis.ts`, `type_infer.ts`, or `native_module.ts` change.  The VSIX for the real extension uses `npm run compile` (separate `out/` directory).


## Key Language Differences from Python

- Variable declarations require `let` / `mut` / `const`
- Functions use `fn` instead of `def`
- Static type checking occurs after parsing and before execution
- Supports templates
- Mutable arguments must explicitly use `mut`
- Empty collections require explicit typing
- No `nonlocal` keyword: declare the outer variable as `mut` to allow inner functions to modify it
- `static mut` instead of a class-level attribute for shared closure state across calls
- `if` / `for` / `while` / `match` / `block` can be used as expressions with a `->Type` annotation
- `block_return val` exits a block/if/match/for/while expression with a value (not a function return)
- `loop_yield val` accumulates values in a `for`/`while` expression into a list (only valid inside `for`/`while` expressions)
- `break` exits the innermost `for`/`while` loop; it propagates through nested `if`/`match`/`block:` expressions to reach the enclosing loop; in a `for`/`while` expression using `loop_yield`, `break` returns the accumulated list; differs from `block_return None` which explicitly sets the expression result to `None`
- Access control uses section markers (`public:` / `private:` / `protected:`) rather than per-member keywords; default accessibility is `public`
- `mng <- async->T: body` submits a concurrent task to an `AsyncManager`; variables are deep-cloned at submission time (no shared mutable state)

## Next Features to Implement (Priority Order)

1. **Expand native compilation** — support closures, generators, and `block_return`/`loop_yield` in compiled functions
2. **Async enhancements** — `async` blocks inside native-compiled functions; shared mutable state via explicit `Mutex`-style primitives
