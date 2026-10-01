# Key Language Differences from Python

- Variable declarations require `let` / `mut` / `const`
- `freeze x` **demotes a `mut` binding to `let`** (task 9.2). After it, every write is a
  static error — rebinding (`x = ..`), mutating methods (`x.append(..)`), subscript
  assignment (`x[0] = ..`) and attribute assignment (`x.f = ..`). Reading still works.
  - ⚠ The demotion does **not** end with the enclosing block: it matches the runtime,
    where `make_var_immutable` rewrites the variable itself, so `x` stays immutable for
    the rest of its lifetime.
  - ⚠ A class may define `fn __freeze__(mut self)`, which runs once at the `freeze`.
- Functions use `fn` instead of `def`
- Static type checking occurs after parsing and before execution
- Supports templates
- Mutable arguments must explicitly use `mut`
- Empty collections require explicit typing
  - ⚠ **Not enforced yet** (`let xs = []` currently passes). The redesign decides this:
    `[]` infers as `list[⊥]` (upcasts to any `list[T]`), and an unannotated `let xs = []`
    defaults to `list[Any]` — see `implementation_logs/type_check_redesign.md` D-7 / U-6.
- **Container annotations must name the element type** (task 8.1). `list` / `dict` / `set` /
  `fixed_list` / `list_like` / `tuple` are rejected in *annotation* position — write
  `list[int]`, `dict[str, int]`, `tuple[int, str]`. Use `list[Any]` when the element type is
  deliberately unconstrained; unlike a bare container it is loud (operating on an `Any`
  element is a static error, so nothing passes silently).
  - ⚠ Type-test position is unaffected: `x is list`, `x mustbe list` and `case list:` still
    work — there the bare name asks "is it a list at all?", which is the correct use.
  - ⚠ The bare container *types* still exist internally, reserved for Python translation
    (Python's `list`/`dict`/`set` carry no element type). They are unreachable from Arrow
    source — see the note on `from_ann`'s primitive table in `src/type_check/types.rs`.
- **Types declared in an imported module are named by the module** (phase 10 task 10-8): the class `Tag` in
  `tags.ar` is the type `tags.Tag` (a nested path gives `a.b.Tag`), distinct from a `Tag` in the main program or in
  another module. Annotations may name it through the import alias (`import tags as t` → `let x: t.Tag`), and
  `from tags import Tag` binds the same type. `is t.Tag` / `mustbe t.Tag` work too.
  - ⚠ A module's name is **per file** (2026-10-02): its path relative to the entry file's directory
    (`pkg.util`; above the entry directory each level is `__parent__`, e.g. `__parent__.util`). The same file
    imported under different spellings is one module; two files never share a name (so `util.ar` and
    `pkg/util.ar` no longer collide). Rule and naming: `src/module_path.rs`.
  - ⚠ A type can only be named through a module **this file imports** (`UnimportedModuleType`): the type
    table is program-wide, but `let t: util.Tag` is an error unless this file imports `util`.
- **Relative imports with leading dots** (2026-10-02, `IMPORT_RESOLUTION_PLAN.md`): `import .a` (this file's
  directory), `import ..a.b` (one level up; each extra dot goes one more level up), `from ..lib import f`.
  Without dots, `import a.b` searches the importing file's directory, then the language's external paths
  (Python `search_paths` / site-packages, C# `lib_paths`, the js-proc bridge). **The same rule applies to
  every `import[lang]`** (`.ar` / `.arc` / py / py-int / cpp / cs / js), both when parsing and at runtime.
  - ⚠ **The entry file's directory is not searched** (unlike Python's `sys.path[0]`). A module in a
    subdirectory reaches up with `..`, never through the entry directory.
  - ⚠ `import[rs]` takes a crate name, so a relative `import[rs]` is an error. Python files translated by
    `import[py]` may use `from .m import x` / `from . import m` (same rule, from the `.py` file's directory).
- **No re-exports** (2026-10-02): names a module binds with `import` / `from … import` are **not** part of
  its namespace. `import wrapper` does not make `wrapper.core` (wrapper's own import) or
  `from wrapper import Item` (wrapper's `from core import Item`) available — both are static errors
  (`ModuleHasNoMember` / `CannotImportName`) and fail at runtime too. Import what you use directly.
  Modules translated from Python (`import[py]`) keep Python's re-export semantics.
- **`class` / `trait` / `protocol` / `new_type` and `import` only at the top level of a module**
  (task 10-16). Inside a function or a block (`if` / `for` / …) they are a `ParseError`.
  `enum` may still be declared inside a function.
  - ⚠ Code translated from Python (`import[py]`) may define classes inside functions (decorators,
    class factories), but a Python function that **returns a class it defines** may only be used
    from Python: calling it, decorating with it or taking it as a value from Arrow is a static
    error. A Python `import` inside a function (or a module-level block) is a conversion error.
- **Exceptions**: a user exception implements the `Error` trait (`class MyErr(Error)` — classes can
  only inherit traits). `Exception` is the base of **every** exception, built-in or user-defined
  (task 10-19), so `except Exception` catches them all, as in Python. `except Error` is a static
  error pointing to `except Exception`.
- No `nonlocal` keyword: declare the outer variable as `mut` to allow inner functions to modify it
- `static mut` instead of a class-level attribute for shared closure state across calls
- `if` / `for` / `while` / `match` / `block` can be used as expressions with a `->Type` annotation
- `block_return val` exits a block/if/match/for/while expression with a value (not a function return)
- `loop_yield val` accumulates values in a `for`/`while` expression into a list (only valid inside `for`/`while` expressions)
- `break` exits the innermost `for`/`while` loop; it propagates through nested `if`/`match`/`block:` expressions to reach the enclosing loop; in a `for`/`while` expression using `loop_yield`, `break` returns the accumulated list; differs from `block_return None` which explicitly sets the expression result to `None`
- Access control uses section markers (`public:` / `private:` / `protected:`) rather than per-member keywords; default accessibility is `public`
- `mng <- async->T: body` submits a concurrent task to an `AsyncManager`; variables are deep-cloned at submission time (no shared mutable state)
