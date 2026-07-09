---
name: architecture-overview
description: Use before modifying the lexer, parser, static type checker, interpreter, or VS Code extension — gives a compact map of what each subsystem already supports (keywords, control flow, type-guard narrowing, value types, closures, exceptions, async, native modules) and which dedicated skill to open for the full implementation reference.
---

# Implemented Features

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

For full details see the `parser-internals` skill.

### Static Type Checking (`src/type_check/`)

Traverses the AST after parsing and before execution, collecting and reporting `StaticTypeError`s together.

- **Type guard narrowing**: when an `if` branch condition is `x is T` or `x is not T`, the variable `x` is re-declared with a narrowed type inside that branch's scope.
  - `x is T` → narrows `x` to `T` (works for primitives, classes, new_types, traits)
  - `x is not T` → requires `x` to be `Union` / `Optional`; narrows by removing `T` from the union members (e.g. `Option[int]` with `is not None` → `int`)
  - `x is not T` on a non-Union type → `StaticTypeError: IsNotOnNonUnion`
- **Function type checking**: typed function values (`function[let T]->R`, `function{let name:T}->R`) are statically checked at call sites for argument count, argument types, keyword argument names, and mutability (`mut` param requires a mutable variable argument)

For full details (all `InferredType` variants, inference rules, compatibility/coercion rules, every `TypeErrorKind`) see the `type-checking` skill.

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

For full details see the `interpreter-internals` skill.

### VS Code Extension (`vscode-extension/`)

- Syntax highlighting for `.ar`
- Type inference inline hints

For adding/modifying extension features and packaging the VSIX, see the `vscode-extension-dev` skill. For debugging this extension's analysis code without launching VS Code, see the `vscode-debug-runner` skill.
