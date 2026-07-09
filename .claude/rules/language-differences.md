# Key Language Differences from Python

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
