# CLAUDE.md

Guidelines for Claude Code when working in this repository.

## Commands

### Rust implementation (primary)

```bash
cargo build                        # Compile
cargo run -- -src <file.hv>        # Execute a .hv file
cargo run -- <file.hv>             # Positional argument also supported
cargo run -- --repl                # Start interactive REPL
cargo test                         # Run all tests
cargo test <name>                  # Filter tests by partial name match
cargo clippy                       # Lint
cargo fmt                          # Format
cargo run -- --compile <file.hv>   # Partially compile a module (see below)
```

### Python implementation (`impl_python/`)

```bash
# Run from the repository root
python -m impl_python <file.hv>

# Examples
python -m impl_python examples/variable.hv
python -m impl_python examples/control_flow.hv
```

## Regulations
- When a new grammer is implemented, an example code to check if it works must be generated in example folder. And if error pattern is implemented, an error example is also neede, whose name has "_error" at the last.
- When Python implementation is updated, also update the git SHA to track and syncronize the versions between the Rust-implementation and Python-implementation. 
- If running the same script(s) many times, make them .ps1 file to ease command permission.
- When VS code extension is updated, the compilation and the generation of VSIX file are required. To generate VSIX filee, run make-vsix.ps1.

## Project Overview

**Havakyrie** is a custom scripting language targeting LLVM IR.  
It aims to provide a Python-based syntax with static type checking and additional custom extensions.

- File extension: `.hv`
- Implementation language: **Rust** (main) and **Python** (`impl_python/`)
- Indentation-based block structure (Python-style)

## Directory Structure

```text
Havakyrie/
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
│   │   ├── module_compiler.rs # Writes .hvc (v0/v1) and .hvs; v1 embeds DLL inside .hvc
│   │   ├── rs_loader.rs     # Loads and links compiled Rust shared libraries
│   │   └── stub_gen.rs      # .hvs stub file generator
│   └── built_in_stab/       # Built-in type stubs (used by VS Code extension)
│       ├── built_in_const.hvs
│       ├── built_in_type.hvs
│       ├── basic_traits.hvs
│       └── error.hvs
├── spec.md                  # Language specification
├── examples/
│   ├── variable.hv            # let / mut / const / static mut declarations
│   ├── variable_error.hv      # declaration error examples
│   ├── control_flow.hv        # if / for / while / match (value-case and type-pattern)
│   ├── control_flow_error.hv  # control flow error examples (StaticTypeError, ParseError)
│   ├── functions.hv           # basic fns, typed sigs, default params, closures, generators, decorators
│   ├── functions_errors.hv    # ParseError (non-default after default) and TypeError (freeze captured mut)
│   ├── class_trait.hv         # class, trait, inheritance, Self, new_type, access control
│   ├── class_trait_error.hv   # immutable field and access control errors
│   ├── collection.hv          # list, dict, tuple, set
│   ├── collection_error.hv    # KeyError (missing dict key, set.remove on absent element)
│   ├── polymorphism.hv        # templates, type guards, Union/Optional
│   ├── polymorphism_error.hv
│   ├── other_typing.hv        # Any, Union, Option, is/is not narrowing, enum
│   ├── other_typing_errors.hv # StaticTypeError (is not on non-Union) and enum value type error
│   ├── built_in.hv            # id(), enumerate(), zip(), file I/O (path/open/close/modes)
│   ├── built_in_error.hv      # built-in function error examples
│   ├── try_except.hv          # try / except / finally, raise, built-in exception types
│   ├── try_except_errors.hv   # exception error examples
│   ├── importation.hv         # all import styles: auto/[hv]/[hvc]/[py-int]/from
│   ├── importation_errors.hv  # ParseError: import[hvc] when no .hvc exists
│   ├── async_demo.hv          # DEMO: AsyncManager, <- operator, raise_immediately, Async enum
│   ├── async_bench.hv         # benchmark: sequential vs async parallel (prime counting)
│   ├── debug_demo.hv          # break_point debugger demo
│   ├── test_modules/          # modules used by importation.hv
│   │   ├── native_ops.hv      # typed int/float functions for native compilation
│   │   ├── native_ops.hvc     # compiled module (v1, with embedded DLL)
│   │   ├── native_ops.hvs     # type stub
│   │   └── prime_factors.hv   # prime factorization module
│   ├── geometry/              # example package
│   │   ├── __init__.hv
│   │   ├── __init__.hvc
│   │   └── __init__.hvs
│   └── archived/              # older examples (kept for reference)
├── impl_python/               # Python implementation of the interpreter
│   ├── __main__.py            # CLI entry point: python -m impl_python <file.hv>
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

A `.hv` module can be partially compiled to native machine code with `--compile`.  
This produces two files next to the source:

| Output file | Contents |
|-------------|----------|
| `{stem}.hvc` | Compiled module: binary header + embedded source text, optionally with a native shared library embedded (v1 format) |
| `{stem}.hvs` | Type stub: function/class/trait signatures with `...` bodies (used by the VS Code extension and the static type checker) |

### Canonical demo

`examples/test_modules/native_ops.hv` is the canonical module for demonstrating native compilation.  
`examples/importation.hv` is the corresponding runner that shows the full workflow.

```bash
# Step 1 — run interpreted
cargo run --release -- examples/importation.hv

# Step 2 — compile the module
cargo run --release -- --compile examples/test_modules/native_ops.hv
# Output:
#   NativeLib: compiling 6 function(s): fib, count_divisors, digit_sum, ...
#   NativeLib: 6 function(s) embedded in examples\test_modules\native_ops.hvc
#   Compiled : examples\test_modules\native_ops.hvc
#   Stub     : examples\test_modules\native_ops.hvs

# Step 3 — run again with native dispatch (same command as Step 1)
cargo run --release -- examples/importation.hv
```

### How the compiled module is used

When a `.hv` file imports a module, the parser prefers `.hvc` over `.hv`.  
If the `.hvc` is v1, the embedded DLL is extracted to a temp file at runtime and loaded via `libloading`.  
Eligible functions are dispatched natively; all other functions tree-walk as usual.

```
import test_modules.native_ops               # loads test_modules/native_ops.hvc (parser)
test_modules.native_ops.fib(60)              # calls native code — ~100× faster for typed int/float
```

### Type-specialized codegen

Functions with `int` or `float` parameter and return type annotations generate direct Rust
arithmetic rather than routing each operation through the callback ABI.

| What is typed | Generated code | vs. untyped |
|---|---|---|
| `mut i = 0`, `i += 1` | `_v_i = _v_i + 1i64` | was `cb_binop(OP_ADD, ...)` |
| `i * i <= n` (while cond) | `(_v_i * _v_i) <= _v_n` | was `cb_is_truthy(cb_binop(...))` |
| `fn f(x: float) -> float` param | `let _v_x: f64 = cb_to_float(_v_x)` | was handle pass-through |

Speedup for pure int/float loops: **100–200×**. Speedup for handle-heavy workloads (lists, class instances): **2–5×**.

### Native eligibility

A function is compiled natively when **all** of the following are true:

- No template parameters
- Not abstract
- The body contains **none** of: `yield`, inner function/generator defs (closures), `try`/`raise`, `block_return`/`loop_yield`, keyword-argument calls, `static mut` variables, `import` statements, or class/trait definitions

All value types (`int`, `bool`, `float`, `str`, `list`, `dict`, `tuple`, class instances) can cross the native boundary via the handle-based ABI.  Functions that do not meet these criteria are silently skipped; they continue to be executed by the tree-walk interpreter.  
If `rustc` is not found in `PATH`, native compilation is skipped entirely and only `.hvc` + `.hvs` are written.

#### Handle-based ABI

Every value crossing the native boundary is an `i64` handle:

| Handle | Meaning |
|--------|---------|
| `0` | `None` |
| `1` | `True` |
| `2` | `False` |
| `-1` | `StopIteration` |
| `≥ 3` | Dynamic value stored in `VALUE_ARENA` |

The interpreter owns all values; native code operates on opaque handles via C callbacks (`TlCallbacks`) injected at load time via `tl_init`. Speedup for handle-heavy workloads (class instances, lists) is 2–5×; for typed int/float arithmetic loops it reaches 100–200× (direct Rust arithmetic, no callbacks).

### Output file roles

- **`.hvc`** — imported instead of `.hv` by the parser (preferred when both exist); v1 contains embedded native code, v0 is source-only
- **`.hvs`** — read by the VS Code extension for type hints; never executed

## About Testing

When adding a specification, testing must follow these rules:

- When the specification is completed:
  - Add interpreter tests
  - Create sample code in the `examples` folder that successfully uses the feature and test it
  - Create sample code in the `examples` folder that intentionally triggers the expected error and verify that the expected error is raised and execution terminates correctly. The filename must end with `_errors`. However, if the specification does not mention error behavior, this step may be omitted.

- During incremental implementation before completion:
  - Only interpreter tests are required

## Execution Flow

```text
Source File
  → Lexer         Generates Vec<Spanned> (each token includes file/line/column)
  → Parser        Generates AST (Vec<Stmt>). Embeds Span into Expr::BinOp / Stmt::Assign etc.
  → TypeChecker   Collects StaticTypeError. If any exist, prints all and exits with exit(1)
  → Interpreter   Executes via tree-walk evaluation
```

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

- Variable declarations: `let` (immutable), `mut` (mutable), `const` (immutable), `static mut` (mutable, shared across all calls)
- Assignment: `x = expr`, compound assignment: `x += expr`, etc.
- Expressions: operator precedence implemented according to spec (including right-associative `**`)
- Function calls: `f(args)`, attribute access: `obj.attr`
- List literals: `[a, b, c]`
- Tuple literals, dictionary literals, subscript operators, control flow, classes, templates, `Self`, and `new_type`
- **Set literals**: `{val, val, ...}` → `Expr::Set`; disambiguated from dict `{k: v}` by lookahead after first expression; `{}` → empty dict; `set()` constructor always produces empty set
- **Slice syntax**: `obj[begin:end]`, `obj[begin:end:step]`, `obj[::step]`, etc. — generates `Expr::Slice`; `begin`/`end` are `Optional[Index]`, `step` is `Optional[int]`
- **Membership operators**: `x in y` and `x not in y` — parsed in `parse_comparison` as `BinOp::In` / `BinOp::NotIn`; work for list, set, dict (key), str, tuple
- Type guard expressions: `expr is TypeName` and `expr is not TypeName` (parsed as `Expr::IsType`)
- Function type annotations: `function`, `function[let T]->R`, `function{let name:T}->R`, `function[]->R`
- Function parameters support optional `let` / `mut` qualifiers (`let` = immutable, `mut` = mutable, absent = immutable)
- **`match` statement**: `match (expr): case val: body` with wildcard `case _:` and type-pattern `is Type:` arms; arms must be uniform (all value-case or all type-case, not mixed)
- **Control flow as expressions** with optional `->Type` annotation:
  - `if cond ->T: body [elif/else]` — evaluates to the value of the taken branch (via `block_return`)
  - `for target in iter ->list[T]: body` — evaluates to a list built by `loop_yield`, or a single value via `block_return`
  - `while cond ->T: body` — same as for expression
  - `match (expr) ->T: arms` — evaluates to the matched arm's `block_return` value
  - `block ->T: body` — inline block expression with `block_return`
  - `->Type` annotation is optional; without it the expression still works but yields untyped results
  - `parse_opt_return_type()` helper parses the optional `->Type` before `:` in all forms
- **Access control sections**: `public:`, `private:`, `protected:` markers inside class/trait bodies switch the accessibility of subsequent members; default is `public`
- **Async task submission**: `target <- async->T: body` — parses as `Stmt::AsyncAssign`; `target` must be an `AsyncManager` variable; `body` is a block executed in a separate thread

### Static Type Checking (`src/type_check/`)

Traverses the AST after parsing and before execution, collecting and reporting `StaticTypeError`s together.

- **Type guard narrowing**: when an `if` branch condition is `x is T` or `x is not T`, the variable `x` is re-declared with a narrowed type inside that branch's scope.
  - `x is T` → narrows `x` to `T` (works for primitives, classes, new_types, traits)
  - `x is not T` → requires `x` to be `Union` / `Optional`; narrows by removing `T` from the union members (e.g. `Option[int]` with `is not None` → `int`)
  - `x is not T` on a non-Union type → `StaticTypeError: IsNotOnNonUnion`
- **Function type checking**: typed function values (`function[let T]->R`, `function{let name:T}->R`) are statically checked at call sites for argument count, argument types, keyword argument names, and mutability (`mut` param requires a mutable variable argument)

### Interpreter (`src/interpreter/`)

- Runtime mutability checking
- Arithmetic, comparison, logical, and bitwise operators
- Function execution
- Class execution
- Iterator protocol
- Dictionary type
- Tuple type
- **Slice type** (`Value::Slice`): `obj[begin:end:step]` syntax and `slice(begin, end[, step])` constructor; `begin`/`end` are `Index` or `None`, `step` is `int` or `None`; supports list/str/tuple slicing with Python-compatible semantics; `.begin`, `.end`, `.step` attribute access
- **Set type** (`Value::Set`): `{a, b, c}` literal (deduplicated); `set()` constructor (from list/str/tuple/set); methods: `add`, `remove`, `discard`, `pop`, `clear`, `copy`, `union`, `intersection`, `difference`, `symmetric_difference`, `issubset`, `issuperset`; operators `|`, `&`, `-`, `^`; `in`/`not in` membership; iteration; `len()`; equality (`==`/`!=`); static type annotation `set` / `set[T]`
- **Exception handling** (`src/interpreter/exceptions.rs`): `try`/`except`/`finally` blocks; `raise ExcType(msg)` and `raise ExcType(msg) from cause`; `except ExcType as e:` binds the exception object with `.message`; built-in exception types (`ValueError`, `TypeError`, `IndexError`, `KeyError`, etc.); unhandled exceptions propagate as `RaiseSignal` until caught or reported at the top level
- `import[py]`
- `import[py-int]`
- `import[hv]` — force `.hv` source, always tree-walk (ignores `.hvc`)
- `import[hvc]` — force `.hvc` compiled (parse error if no `.hvc` exists)
- `import` (no qualifier) — auto: prefer `.hvc` if present, fall back to `.hv`
- Type guard (`is` / `is not`): runtime instance-of check against primitive types, class names, trait membership (via `bases`), and `function`
- `function` primitive type: `Value::Function`, `Value::OverloadedFn`, `Value::GeneratorFn` all match `x is function`
- **`match` statement**: pattern matching with value-case arms (`case val:`), wildcard (`case _:`), and type-pattern arms (`is Type:`)
- **Closures**: inner functions capture variables from outer scopes
  - Immutable (`let`) variables: deep-copied at closure creation time
  - Mutable (`mut`) variables: captured as a shared `Rc<RefCell<Value>>` cell — inner functions can read and write the same value as the outer scope
  - Each call to the outer function produces an independent closure environment
  - `static mut` variables: a single persistent cell keyed by source position, shared across all calls to the outer function
  - `freeze` is disallowed on a `mut` variable that has been captured by an inner function (`TypeError: cannot freeze '...' because it is captured by a closure`)
- **`block:` expression** (`Expr::Block`): `let x = block ->T: block_return val` — inline block that returns a single value via `block_return`; returns `None` if `block_return` is never reached
- **Control flow as expressions** (`Expr::IfExpr`, `Expr::ForExpr`, `Expr::WhileExpr`, `Expr::MatchExpr`):
  - `block_return val` — exits the immediately enclosing control-flow expression and yields `val` as its result; runtime error if used outside any block/if/for/while/match expression
  - `loop_yield val` — accumulates `val` into a list inside a `for`/`while` expression; runtime error if used outside a `for`/`while` expression (i.e. not valid in `block:`, `if`, or `match` expressions)
  - `break` — exits the innermost `for`/`while` loop (statement or expression form); propagates through nested `if`/`match`/`block:` expression bodies via `BREAK_SENTINEL` error signal until caught by the enclosing loop; in a loop expression, `break` returns the accumulated `loop_yield` list (or `None` if none); differs from `block_return None` which explicitly returns `None` even when yields exist; runtime error if used outside any `for`/`while`
  - For expressions: if `loop_yield` is used, the expression evaluates to the accumulated list; if `block_return val` is used, evaluates to `val`; if `break` is used, evaluates to the accumulated list or `None`; if none of these is reached, evaluates to `None`
  - Thread-local `BLOCK_YIELDS` (set to `Some(Vec)` inside for/while expression bodies) collects `loop_yield` values without interrupting control flow
  - Thread-local `LOOP_DEPTH` (incremented for every for/while, statement or expression form) guards `break` usage; reset to 0 on function entry to prevent break from crossing function boundaries
  - `BREAK_SENTINEL` (`"\x00__break__"`) — internal error string that propagates `break` through `eval()` channels (e.g., through `if`/`match`/`block:` expression bodies); caught and consumed by the enclosing loop handler
- **Access control** (`public` / `private` / `protected`):
  - Section-style markers (`public:`, `private:`, `protected:`) inside class or trait bodies apply to all subsequent member declarations
  - `public` (default): accessible from anywhere
  - `private`: accessible only from methods of the same class
  - `protected`: accessible from methods of the same class or any class that implements the same trait
  - Violation raises `AccessError` at runtime; `current_class` is tracked on `Interpreter` and set/restored around each method call
  - Trait field access is inherited into class `field_access` maps with namespaced keys (`"TraitName::field"`)
- **Async system** (`src/interpreter/async_mgr.rs`):
  - `AsyncManager(num_thread=N [, raise_immediately=bool])` — built-in class managing a pool of OS threads
  - `mng <- async->T: body` — submits a task; all visible variables are deep-cloned into the thread at submission time
  - Each task runs in a dedicated OS thread (`std::thread::spawn`); results returned via `mpsc` channel
  - `AsyncManager` fields: `num_thread` (`uint`), `raise_immediately` (`bool`), `results` (list), `error_list` (list of `Optional[str]`), `progress_status` (list of `Async.*`), `thread_status` (list of running task indices)
  - `AsyncManager` methods: `all_done() -> bool`, `wait_for_finish([await_interval_msec=100])`
  - `wait_for_finish()` blocks until all tasks complete; if `raise_immediately` is true and any task raised an error, propagates the first error as a catchable `raise` (use `try/except:`)
  - `Async` namespace: `Async.Waiting`, `Async.Running`, `Async.Done` — task progress states
  - Capture semantics: at `<-` time, `mut` variables are captured by shared reference (Rc clone) so the task can propagate mutations back to the caller's scope; `let` variables are deep-cloned (independent copies)
  - `Value::deep_clone()` creates fully independent copies of all value types (new `Rc`s, no sharing across threads)
- **Debugger** (`src/interpreter/debugger.rs`): `break_point` statement pauses execution and drops into an interactive debug session

### VS Code Extension (`vscode-extension/`)

- Syntax highlighting for `.hv`
- Type inference inline hints


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
