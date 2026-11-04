# Key Language Differences from Python

- Variable declarations require `let` / `mut` / `const`
- `freeze x` **demotes a `mut` binding to `let`** (task 9.2). After it, every write is a
  static error — rebinding (`x = ..`), mutating methods (`x.append(..)`), subscript
  assignment (`x[0] = ..`) and attribute assignment (`x.f = ..`). Reading still works.
  - ⚠ The demotion does **not** end with the enclosing block: it matches the runtime,
    where `make_var_immutable` rewrites the variable itself, so `x` stays immutable for
    the rest of its lifetime.
  - ⚠ A class may define `fn __freeze__(mut self)`, which runs once at the `freeze`.
  - ⚠ `freeze x` gives **the same result as `let x = x`** (task 1-9, one implementation
    `bind_copy(BindCopy::CopyFreeze, ..)`): `x` is rebound to a frozen **deep copy**, so other holders of the object
    (e.g. a caller that passed it to a `mut` parameter) stay mutable. Works inside functions.
- **`const` is immutable as a whole** (top-level, local, class, trait, enum member — implicitly — and an
  imported module's `const`). Assigning to it (`C.K = ..`, `c.K = ..`, `self.K = ..` even in `__init__`,
  `m.K = ..`) and writing into it (`C.L[0] = ..`, `c.L.append(..)`, `c.O.x = ..`, `m.L[0] = ..`) are static
  errors, whatever the root binding. A copy (`mut xs = C.L`) is writable; `static mut` stays writable.
  - ⚠ Whether something is writable is decided by the **attribute** (`const` / `let` / `mut`), not the type.
- **An imported module's globals keep their type and attribute** (`docs/grammar/09_imports.md`):
  `g.X = ..` is allowed only for `mut X` (it rewrites the module's global itself, like CPython);
  `let` / `const` / functions are static errors, and so is writing into a `let` / `const` (`g.L.append(..)`).
  `from g import X` binds a name that cannot be rebound, but its contents follow the original attribute.
  - ⚠ A Python module (`import[py-int]`) has no `let` / `mut`: as in CPython, every global can be assigned from Arrow
    (`m.count = 5` calls `setattr`; checked against the stub's type when it has one), and reads are live (pandas_import_plan.md
    1-4 / 2-9).
- `...` is a value (`Ellipsis`) only as a subscript item (`a[..., 0]`, pandas_import_plan.md 2-3); elsewhere write
  `Ellipsis`. Subscript items may be slices (`a[1:, 0]`), and a slice may hold any bounds (`df.loc["b":"d"]`), but
  slicing an Arrow list / str / bytes / tuple still requires int bounds (static error when the type says so, otherwise
  CPython's runtime `TypeError`).
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
  - ⚠ The same holds for classes of an `import[py]` module (`import[py] zoo as z` → `z.Dog`). A Python
    class also matches the names of its base classes (`p is z.Animal` for `class Dog(Animal)`), as CPython's
    `isinstance` does. `import[py-int]` classes (CPython objects) follow the foreign-stub rule below instead.
  - ⚠ A module's name is **per file** (2026-10-02): its path relative to the entry file's directory
    (`pkg.util`; above the entry directory each level is `__parent__`, e.g. `__parent__.util`). The same file
    imported under different spellings is one module; two files never share a name (so `util.ar` and
    `pkg/util.ar` no longer collide). Rule and naming: `src/module_path.rs`.
  - A foreign stub's class (`import[cs-proc] cs_probe as cp` → `Person`) is still registered under its bare name, but
    type positions may also spell it through the alias (`cp.Person`, `x is cp.Person`; ffi_value_exchange_plan.md 7-6).
  - ⚠ A type can only be named through a module **this file imports** (`UnimportedModuleType`): the type
    table is program-wide, but `let t: util.Tag` is an error unless this file imports `util`.
- **Modules behave like CPython** (2026-10-02, phase 4 of `IMPORT_RESOLUTION_PLAN.md`; rule: `src/module_path.rs`):
  - `import a.b.c` runs package `a` (its `a/__init__.ar`; a directory without one is an empty namespace
    package), then `a.b`, then `a.b.c`, and **binds `a`** — call it as `a.b.c.f()`. `import a.b.c as m` binds
    `m = a.b.c`. A loaded submodule becomes an attribute of its parent package wherever it was imported.
  - `from a import b` imports submodule `a.b` when `b` is not a name in `a`.
  - **Re-exports exist**: names a module binds with `import` / `from … import` are part of its namespace
    (`wrapper.core`, `from wrapper import Item`). Only names that are none of declaration / re-export /
    submodule are errors (`ModuleHasNoMember` / `CannotImportName`).
  - **Search**: `import a.b` (no dots) searches the **entry file's directory** (CPython's `sys.path[0]`), then
    the language's external paths (Python `search_paths` / site-packages, C# `lib_paths`, the js-proc
    bridge). It does **not** search the importing file's directory (no implicit relative imports) — inside a
    package write `from . import util` or `import pkg.util`.
  - **Relative imports**: `from . import x`, `from .m import y`, `from ..m import y` (CPython), plus the Arrow
    extension `import .a` / `import ..a.b`. Each extra dot goes one directory up from the importing file.
    ⚠ Unlike CPython they also work from the entry file and may go above the entry directory.
  - **The same rule applies to every `import[lang]`** (`.ar` / `.arc` / py / py-int / cpp / cs / js), when
    parsing and at runtime. ⚠ cpp / cs / js / rs dotted paths are file locations, not packages: they bind
    the alias or the last part (cpp: the header's stem) as before. `import[rs]` takes a crate name, so a
    relative `import[rs]` is an error.
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
  - Built-in exceptions follow **CPython 3.12's hierarchy** (`except LookupError` catches `KeyError`;
    `GeneratorExit` is not an `Exception`), take any positional arguments (`e.args`, CPython's
    `str(e)` / `repr(e)`), and `except m.Err:` names a module's exception (python_builtins_plan.md 4-1〜4-3).
  - Exceptions raised by CPython (`import[py-int]`; pandas_import_plan.md 2-1): a built-in one arrives as the Arrow exception
    of the same name (`except ValueError:`); one defined in Python stays a CPython object, caught by `except m.Err:` and by
    the built-in exceptions it inherits (CPython's `isinstance`). They used to all be `ForeignError`.
- **Code translated from Python (`import[py]`) shares references; Arrow code copies** (known_bugs.md #1, decision
  A). Arrow copies a value whenever it is stored (field, dict, `append`, `xs[i] = v`, set `add`, `mut` declaration,
  closure capture) so a `let` value can never change through another holder. Inside a Python module's code these
  stores share the object, as in CPython (aliases, parent/child references and registries behave the same). A
  Python class without `__eq__` / `__hash__` compares and hashes by identity; an Arrow class still compares by
  contents. Binding a Python value to an Arrow `let` / `mut` still copies it (the boundary keeps Arrow's rule).
- **Python objects must be bound with `mut`** (`implementation_plans/pandas_import_plan.md` D-3, task 1-2). A CPython
  object (an `import[py-int]` value that stays a Python object: instances, functions, arrays, big ints …) is shared, not
  copied, so other references can change it; `let` / `const` / `freeze` and a parameter that is not `mut` cannot hold one
  (`fn f(mut d: pd.DataFrame)`). Static error `ImmutablePythonObject` when the type says so (a Python class instance, or a
  container of them, or a `py_list` / `py_dict` / `py_set`), otherwise a runtime `TypeError` at the binding. Values
  converted to Arrow values at the boundary (`int`, `str`, `tuple`, the result of `=> list[int]` …) are Arrow values and
  can be bound with `let`.
  - Implicit bindings follow the same rule (D-11, task 2-10): write `for mut f in ..` (all targets of `for mut k, v`),
    `[.. for mut f in ..]` and `except E as mut e`. Without `mut`, receiving a Python object is a static
    `ImmutablePythonObject` when the type says so, otherwise a runtime `TypeError` at the binding — even when the iterated
    binding is itself `mut`. `for mut x in xs` binds a **copy** of an Arrow element (like `mut x = elem`); plain
    `for x in xs` still aliases the element. (`match` binds no new names; Arrow has no `with`.)
- **Collections are exchanged with Python through `py_list[T]` / `py_dict[K, V]` / `py_set[T]`** (pandas_import_plan.md
  D-12, task 2-7). These built-in types hold a CPython list / dict / set; their operations have Python's meaning
  (methods are exactly CPython's, display is `['a']`) and they never mix with Arrow's `list` / `dict` / `set`
  (`pl + [1]` and `let xs: list[int] = pl` are static errors). Convert with the cast `=>`: `xs => py_list[int]`,
  `pl => list[int]`; the cast copies to the depth the target type names (`[[1]] => py_list[py_list[int]]`) and checks the
  elements (`TypeError` on a mismatch). A list / dict / set returned by Python arrives **uncopied** as `py_list` etc.
  (Python's later changes are visible), and a stub's `list[T]` reads as `py_list[T]`. An Arrow collection cannot be an
  element (`py_list[list[int]]` is an error). Like every Python object they are bound with `mut`.
  - Passing an Arrow list / dict / set (or a tuple holding one) to Python **without a cast is an error, literals
    included** (task 2-11): write `np.array([1, 2] => py_list[int])`. It covers call arguments (keywords and `*` / `**`
    spreads too), subscripts, element / attribute assignment and operator operands — static `ArrowCollectionToPython`
    when the types say so, otherwise a runtime `TypeError`. `pyobj in xs` with an Arrow container on the right is
    compared on the Arrow side (nothing is passed).
- **Arrow instances, classes and generators reach Python uncopied** (pandas_import_plan.md 2-6, D-9). An instance is
  shared (passing it twice gives the same Python object; it comes back as the same instance) and appears as a subclass of
  `arrow.ArrowObject` named after the Arrow class, exposing only the special methods the class defines. It is
  **writable only when passed as a `mut` variable (or an expression) argument**, read from a writable one, or returned by
  an Arrow function; otherwise (a `let` variable, stored into a Python object, an operand, a cast element) Python gets a
  read-only view — attribute assignment and `mut self` methods raise `TypeError`. Classes and generator functions are
  callables, generators are Python iterators. Touching them from another Python thread raises `RuntimeError`.
- **`object()` can only be called in Python modules** (`import[py]`; phase 6 task 6-10). In Arrow source it is a
  static error (calling the class value is a runtime error). `object` itself is still the base of every value
  (`isinstance(x, object)` / `x is object` are always true) and annotates like `Any`. An `object()` instance has no
  fields, is never copied on store (a sentinel stays the same object through fields, dicts and lists), and compares
  and hashes by identity.
- **Attribute hooks (`__setattr__` / `__delattr__` / `__getattr__` / `__getattribute__`) exist only in Python modules**
  (`import[py]`; known_bugs.md #10 / #11 / #12). A Python class's `__setattr__` runs for every assignment (`o.x = v`,
  `setattr`, `self.x = ..` in `__init__`, assignments from Arrow); `object.__setattr__(self, k, v)` /
  `super().__setattr__(k, v)` store past it. `__getattribute__` runs for every explicit read and method call (`o.x`,
  `o.m(..)`, `o.__len__()`), but not for the special methods the interpreter looks up itself (`str(o)`, `len(o)`,
  operators, `for`); `__getattr__` runs when the read raises `AttributeError`; `object.__getattribute__(self, k)` reads
  past both. Defining any of these hooks in an Arrow class is a `ParseError` — Arrow members are fixed by the class body.
- **`obj.method` read as a value is a bound method** (the receiver is bound, as in Python; task 1-8).
  Binding never fails; calling a `mut self` method of an immutable receiver (`let` / frozen) is an
  error at the call, exactly as the direct call `obj.method(..)` would be (static via the function type's
  `mutates_immutable` mark; instances are also stopped at run time). `gen` methods and methods of built-in
  values (`push = xs.append`) bind too (task 1-10; readable names: `names::BUILTIN_VALUE_METHODS`).
- No `nonlocal` keyword: declare the outer variable as `mut` to allow inner functions to modify it
- `static mut` instead of a class-level attribute for shared closure state across calls
- `if` / `for` / `while` / `match` / `block` can be used as expressions with a `->Type` annotation
- `block_return val` exits a block/if/match/for/while expression with a value (not a function return)
- `loop_yield val` accumulates values in a `for`/`while` expression into a list (only valid inside `for`/`while` expressions)
- `break` exits the innermost `for`/`while` loop; it propagates through nested `if`/`match`/`block:` expressions to reach the enclosing loop; in a `for`/`while` expression using `loop_yield`, `break` returns the accumulated list; differs from `block_return None` which explicitly sets the expression result to `None`
- Access control uses section markers (`public:` / `private:` / `protected:`) rather than per-member keywords; default accessibility is `public`
- `mng <- async->T: body` submits a concurrent task to an `AsyncManager`. At submission `let` variables are deep-cloned,
  but **`mut` variables are shared by reference**: the task mutating a `mut` list (`xs.append(..)`) changes the caller's
  list, while rebinding (`n = 5`) does not propagate (decided in `implementation_plans/ffi_value_exchange_plan.md` 7-20).
  - ⚠ The shared contents are **not synchronised** — mutating the same `mut` value from the caller and a task at the same
    time is a data race. Return results with `block_return` instead.
  - ⚠ Thread-bound values (file objects, `AsyncManager`, `Signal`) are never shared, even through `mut`: the task gets a
    detached copy and using it is a `RuntimeError`; `EventLoop` is the task's own loop (7-19).
