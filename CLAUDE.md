# CLAUDE.md

Guidelines for Claude Code when working in this repository.

## Commands

```bash
cargo build                        # Compile
cargo run -- -src <file.tl>        # Execute a .tl file
cargo run -- <file.tl>             # Positional argument also supported
cargo test                         # Run all tests
cargo test <name>                  # Filter tests by partial name match
cargo clippy                       # Lint
cargo fmt                          # Format
cargo run -- --compile <file.tl>   # Partially compile a module (see below)
```

## Project Overview

**test_lang** is a custom scripting language targeting LLVM IR.  
It aims to provide a Python-based syntax with static type checking and additional custom extensions.

- File extension: `.tl`
- Implementation language: **Rust** (main), with Python planned in the future
- Indentation-based block structure (Python-style)

## Directory Structure

```text
test_lang/
├── src/
│   ├── main.rs          # Entry point / argument parsing
│   ├── token.rs         # Token enum / Span / Spanned definitions
│   ├── lexer.rs         # Lexer (returns Vec<Spanned>)
│   ├── ast.rs           # AST node definitions (with embedded Span)
│   ├── parser.rs        # Recursive descent parser (input: Vec<Spanned>)
│   ├── type_check.rs    # Static type checker (after parsing, before execution)
│   ├── interpreter.rs   # Tree-walk interpreter
│   ├── partial_compiler/ # --compile subsystem
│   │   ├── mod.rs         # Entry point: orchestrates compile pipeline
│   │   ├── codegen.rs     # Rust source code generator (tl fn → handle-based i64 ABI)
│   │   ├── module_compiler.rs # Writes .tlc (v0/v1) and .tls; v1 embeds DLL inside .tlc
│   │   └── stub_gen.rs    # .tls stub file generator
│   └── interpreter/
│       └── native_api.rs  # Handle arena, TlCallbacks struct, 19 C callback impls
├── spec/
│   ├── general.md       # General language specification
│   ├── keywords.md      # Keyword list
│   └── operator.md      # Operator list / precedence
├── examples/
│   ├── variable.tl            # let / mut / const / static mut declarations
│   ├── variable_error.tl      # declaration error examples
│   ├── control_flow.tl        # if / for / while / match (value-case and type-pattern)
│   ├── control_flow_error.tl  # control flow error examples (StaticTypeError, ParseError)
│   ├── control_flow_expr.tl   # if/for/while/match as expressions (->Type)
│   ├── block_expr.tl          # block: expression with block_return
│   ├── functions.tl           # basic fns, typed sigs, default params, closures, generators, decorators
│   ├── functions__errors.tl   # ParseError (non-default after default) and TypeError (freeze captured mut)
│   ├── class_trait.tl         # class, trait, inheritance, Self, new_type, access control
│   ├── class_trait_error.tl   # immutable field and access control errors
│   ├── collection.tl          # list, dict, tuple, set
│   ├── collection_error.tl    # KeyError (missing dict key, set.remove on absent element)
│   ├── polymorphism.tl        # templates, type guards, Union/Optional
│   ├── polymorphism_error.tl
│   ├── other_typing.tl        # Any, Union, Option, is/is not narrowing, enum
│   ├── other_typing__errors.tl  # StaticTypeError (is not on non-Union) and enum value type error
│   ├── subscript.tl           # subscript / indexing behavior
│   ├── subscript__errors.tl
│   ├── slice.tl               # slice syntax and slice() constructor
│   ├── slice__errors.tl       # TypeError when non-Index used as slice bound
│   ├── file_io.tl             # import[py] file I/O
│   ├── file_io__errors.tl
│   ├── native_ops.tl              # module: typed int/float functions for native compilation
│   ├── native_ops_demo.tl         # DEMO: full --compile workflow (start here)
│   ├── import_qualifier_demo.tl   # DEMO: import / import[tl] / import[tlc] comparison
│   ├── heavy_ops.tl           # module: heavier benchmarks (all value types)
│   ├── bench_heavy.tl         # benchmark: speedup across int/float/str/class
│   └── archived/              # older examples (kept for reference)
└── vscode-extension/    # VS Code extension (type inference inline hints)
    └── src/
        ├── extension.ts
        └── type_infer.ts
```

## Partial Compilation

A `.tl` module can be partially compiled to native machine code with `--compile`.  
This produces two files next to the source:

| Output file | Contents |
|-------------|----------|
| `{stem}.tlc` | Compiled module: binary header + embedded source text, optionally with a native shared library embedded (v1 format) |
| `{stem}.tls` | Type stub: function/class/trait signatures with `...` bodies (used by the VS Code extension and the static type checker) |

### Canonical demo

`examples/native_ops.tl` is the canonical module for demonstrating native compilation.  
`examples/native_ops_demo.tl` is the corresponding runner that shows the full workflow.

```bash
# Step 1 — run interpreted
cargo run --release -- examples/native_ops_demo.tl

# Step 2 — compile the module
cargo run --release -- --compile examples/native_ops.tl
# Output:
#   NativeLib: compiling 6 function(s): fib, count_divisors, digit_sum, ...
#   NativeLib: 6 function(s) embedded in examples\native_ops.tlc
#   Compiled : examples\native_ops.tlc
#   Stub     : examples\native_ops.tls

# Step 3 — run again with native dispatch (same command as Step 1)
cargo run --release -- examples/native_ops_demo.tl
```

### How the compiled module is used

When a `.tl` file imports a module, the parser prefers `.tlc` over `.tl`.  
If the `.tlc` is v1, the embedded DLL is extracted to a temp file at runtime and loaded via `libloading`.  
Eligible functions are dispatched natively; all other functions tree-walk as usual.

```
import native_ops               # loads native_ops.tlc (parser)
native_ops.fib(60)              # calls native code — ~100× faster for typed int/float
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
If `rustc` is not found in `PATH`, native compilation is skipped entirely and only `.tlc` + `.tls` are written.

#### Handle-based ABI

Every value crossing the native boundary is an `i64` handle:

| Handle | Meaning |
|--------|---------|
| `0` | `None` |
| `1` | `True` |
| `2` | `False` |
| `-1` | `StopIteration` |
| `≥ 3` | Dynamic value stored in `VALUE_ARENA` |

The interpreter owns all values; native code operates on opaque handles via 19 callbacks (`TlCallbacks`) injected at load time via `tl_init`. Speedup for handle-heavy workloads (class instances, lists) is 2–5×; for typed int/float arithmetic loops it reaches 100–200× (direct Rust arithmetic, no callbacks).

### Output file roles

- **`.tlc`** — imported instead of `.tl` by the parser (preferred when both exist); v1 contains embedded native code, v0 is source-only
- **`.tls`** — read by the VS Code extension for type hints; never executed

## About Testing

When adding a specification, testing must follow these rules:

- When the specification is completed:
  - Add interpreter tests
  - Create sample code in the `examples` folder that successfully uses the feature and test it
  - Create sample code in the `examples` folder that intentionally triggers the expected error and verify that the expected error is raised and execution terminates correctly. The filename must end with `__errors`. However, if the specification does not mention error behavior, this step may be omitted.

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

### Lexical Analysis (`src/lexer.rs`)

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

### Parsing (`src/parser.rs`)

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

### Static Type Checking (`src/type_check.rs`)

Traverses the AST after parsing and before execution, collecting and reporting `StaticTypeError`s together.

- **Type guard narrowing**: when an `if` branch condition is `x is T` or `x is not T`, the variable `x` is re-declared with a narrowed type inside that branch's scope.
  - `x is T` → narrows `x` to `T` (works for primitives, classes, new_types, traits)
  - `x is not T` → requires `x` to be `Union` / `Optional`; narrows by removing `T` from the union members (e.g. `Option[int]` with `is not None` → `int`)
  - `x is not T` on a non-Union type → `StaticTypeError: IsNotOnNonUnion`
- **Function type checking**: typed function values (`function[let T]->R`, `function{let name:T}->R`) are statically checked at call sites for argument count, argument types, keyword argument names, and mutability (`mut` param requires a mutable variable argument)

### Interpreter (`src/interpreter.rs`)

- Runtime mutability checking
- Arithmetic, comparison, logical, and bitwise operators
- Function execution
- Class execution
- Iterator protocol
- Dictionary type
- Tuple type
- **Slice type** (`Value::Slice`): `obj[begin:end:step]` syntax and `slice(begin, end[, step])` constructor; `begin`/`end` are `Index` or `None`, `step` is `int` or `None`; supports list/str/tuple slicing with Python-compatible semantics; `.begin`, `.end`, `.step` attribute access
- **Set type** (`Value::Set`): `{a, b, c}` literal (deduplicated); `set()` constructor (from list/str/tuple/set); methods: `add`, `remove`, `discard`, `pop`, `clear`, `copy`, `union`, `intersection`, `difference`, `symmetric_difference`, `issubset`, `issuperset`; operators `|`, `&`, `-`, `^`; `in`/`not in` membership; iteration; `len()`; equality (`==`/`!=`); static type annotation `set` / `set[T]`
- `import[py]`
- `import[py-int]`
- `import[tl]` — force `.tl` source, always tree-walk (ignores `.tlc`)
- `import[tlc]` — force `.tlc` compiled (parse error if no `.tlc` exists)
- `import` (no qualifier) — auto: prefer `.tlc` if present, fall back to `.tl`
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
  - `break` — alias for `block_return None`; exits the innermost `for`/`while` loop (statement or expression form); runtime error if used outside any `for`/`while`
  - For expressions: if `loop_yield` is used, the expression evaluates to the accumulated list; if `block_return` is used, evaluates to that single value; if neither is reached, evaluates to `None`
  - Thread-local `BLOCK_YIELDS` (set to `Some(Vec)` inside for/while expression bodies) collects `loop_yield` values without interrupting control flow
  - Thread-local `LOOP_DEPTH` (incremented for every for/while, statement or expression form) guards `break` usage
- **Access control** (`public` / `private` / `protected`):
  - Section-style markers (`public:`, `private:`, `protected:`) inside class or trait bodies apply to all subsequent member declarations
  - `public` (default): accessible from anywhere
  - `private`: accessible only from methods of the same class
  - `protected`: accessible from methods of the same class or any class that implements the same trait
  - Violation raises `AccessError` at runtime; `current_class` is tracked on `Interpreter` and set/restored around each method call
  - Trait field access is inherited into class `field_access` maps with namespaced keys (`"TraitName::field"`)

### VS Code Extension (`vscode-extension/`)

- Syntax highlighting for `.tl`
- Type inference inline hints

## Major Unimplemented Features

- Full preservation of type annotations
- Runtime return type checking for `block_return`/`loop_yield` against `->Type` annotations
- Mixing check: `block_return` and `loop_yield` in the same block expression (currently not statically detected)
- Static access checking for `private`/`protected` (currently runtime `AccessError` only; no `StaticTypeError` at parse/type-check time)
- Imports (`import` / `from ... import`)
- Native compilation: closures (inner functions capturing outer variables), generators, `try`/`raise`, `block_return`/`loop_yield`, and `static mut` are not yet supported in compiled functions
- Python implementation

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
- `break` is an alias of `block_return None` and is only valid inside `for`/`while` loops
- Access control uses section markers (`public:` / `private:` / `protected:`) rather than per-member keywords; default accessibility is `public`

## Next Features to Implement (Priority Order)

1. **Imports** (`import` / `from ... import`)
2. **Expand native compilation** — support closures, generators, `try`/`raise`, and `block_return`/`loop_yield` in compiled functions

