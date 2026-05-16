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
│   └── interpreter.rs   # Tree-walk interpreter
├── spec/
│   ├── general.md       # General language specification
│   ├── keywords.md      # Keyword list
│   └── operator.md      # Operator list / precedence
├── examples/
│   ├── showcase.tl            # Demonstration of all implemented features
│   ├── type_errors.tl         # Examples generating StaticTypeError
│   ├── self_type.tl           # Self type behavior examples
│   ├── self_type__errors.tl   # Self type parse error examples
│   ├── new_type.tl            # new_type behavior examples
│   ├── new_type__errors.tl    # new_type Self mismatch error examples
│   ├── iterator.tl            # Iterator behavior examples
│   ├── iterator__errors.tl    # EndOfIteration error examples
│   ├── dict.tl                # Dictionary type behavior examples
│   ├── dict__errors.tl        # dict type mismatch error examples
│   ├── tuple.tl               # Tuple type behavior examples
│   ├── py_import.tl           # import[py] examples
│   ├── py_additional_param.tl # Python **kwargs examples
│   ├── py_int_import.tl       # import[py-int] examples
│   ├── typeguard.tl           # is / is not type guard examples
│   ├── typeguard__errors.tl   # is not on non-Union type StaticTypeError examples
│   ├── function_type.tl       # function type annotation examples
│   ├── function_type__errors.tl  # function type StaticTypeError examples
│   ├── closure.tl             # Closure behavior examples (capture, static, nested)
│   ├── closure__errors.tl     # freeze on captured mutable variable TypeError examples
│   ├── match.tl               # match statement behavior examples
│   ├── match__errors.tl       # match mixed-arm parse error examples
│   ├── block_expr.tl          # block: expression with block_return examples
│   └── control_flow_expr.tl   # if/for/while/match as expressions with ->Type examples
└── vscode-extension/    # VS Code extension (type inference inline hints)
    └── src/
        ├── extension.ts
        └── type_infer.ts
```

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

### Parsing (`src/parser.rs`)

- Variable declarations: `let` (immutable), `mut` (mutable), `const` (immutable), `static mut` (mutable, shared across all calls)
- Assignment: `x = expr`, compound assignment: `x += expr`, etc.
- Expressions: operator precedence implemented according to spec (including right-associative `**`)
- Function calls: `f(args)`, attribute access: `obj.attr`
- List literals: `[a, b, c]`
- Tuple literals, dictionary literals, subscript operators, control flow, classes, templates, `Self`, and `new_type`
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
- `import[py]`
- `import[py-int]`
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

### VS Code Extension (`vscode-extension/`)

- Syntax highlighting for `.tl`
- Type inference inline hints

## Major Unimplemented Features

- Full preservation of type annotations
- Runtime return type checking for `block_return`/`loop_yield` against `->Type` annotations
- Mixing check: `block_return` and `loop_yield` in the same block expression (currently not statically detected)
- Set type (`{a, b}`)
- Exception handling (`try` / `except` / `finally` / `raise`)
- Imports (`import` / `from ... import`)
- LLVM IR code generation
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

## Next Features to Implement (Priority Order)

1. **Set type** (`{a, b}`)
2. **Exception handling** (`try` / `except` / `finally` / `raise`)
3. **LLVM IR code generation**

