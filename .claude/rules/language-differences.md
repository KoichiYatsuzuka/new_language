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
    defaults to `list[Any]` — see `implementation_plans/type_check_redesign.md` D-7 / U-6.
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
- No `nonlocal` keyword: declare the outer variable as `mut` to allow inner functions to modify it
- `static mut` instead of a class-level attribute for shared closure state across calls
- `if` / `for` / `while` / `match` / `block` can be used as expressions with a `->Type` annotation
- `block_return val` exits a block/if/match/for/while expression with a value (not a function return)
- `loop_yield val` accumulates values in a `for`/`while` expression into a list (only valid inside `for`/`while` expressions)
- `break` exits the innermost `for`/`while` loop; it propagates through nested `if`/`match`/`block:` expressions to reach the enclosing loop; in a `for`/`while` expression using `loop_yield`, `break` returns the accumulated list; differs from `block_return None` which explicitly sets the expression result to `None`
- Access control uses section markers (`public:` / `private:` / `protected:`) rather than per-member keywords; default accessibility is `public`
- `mng <- async->T: body` submits a concurrent task to an `AsyncManager`; variables are deep-cloned at submission time (no shared mutable state)
