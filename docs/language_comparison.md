# Language Comparison: Rust · Python · tl

---

## Variable Declaration

| Concept | Rust | Python | tl |
|---|---|---|---|
| Immutable binding | `let x = 5` | `x = 5` (all mutable) | `let x = 5` |
| Mutable binding | `let mut x = 5` | `x = 5` | `mut x = 5` |
| Compile-time constant | `const X: i32 = 5` | `X = 5` (convention only) | `const X = 5` |
| Shared-across-calls state | — | class attribute / `global` | `static mut x = 0` (inside fn) |
| Demote to immutable at runtime | — | — | `freeze x` |
| Compound assign | `x += 1` | `x += 1` | `x += 1` |

---

## Primitive Types

| Type | Rust | Python | tl |
|---|---|---|---|
| Integer | `i64`, `i32`, `usize`, … | `int` | `int` |
| Float | `f64`, `f32` | `float` | `float` |
| Boolean | `bool` (`true`/`false`) | `bool` (`True`/`False`) | `bool` (`True`/`False`) |
| String | `String`, `&str` | `str` | `str` |
| None/unit | `()`, `Option::None` | `None` | `None` |
| Numeric literals | `1_000`, `0xFF`, `0b1010`, `0o77` | `1_000`, `0xFF`, `0b1010`, `0o77` | `1_000`, `0xFF`, `0b1010`, `0o77` |

---

## Collections

| Collection | Rust | Python | tl |
|---|---|---|---|
| Dynamic array | `Vec<T>` | `list` | `list` / `list[T]` |
| Hash map | `HashMap<K,V>` | `dict` | `dict` / `dict[K,V]` |
| Fixed tuple | `(T1, T2, …)` | `tuple` | `(v1, v2, …)` |
| Hash set | `HashSet<T>` | `set` | `set` / `set[T]` |
| Empty typed collection | `Vec::<i32>::new()` | `[]` | `list[int]()` |
| Empty dict | `HashMap::new()` | `{}` | `dict[K,V]()` or `{}` |
| Empty set | `HashSet::new()` | `set()` | `set()` (note: `{}` is empty dict) |
| Set literal | — | `{1, 2, 3}` | `{1, 2, 3}` |
| Tuple unpack | `let (a, b) = t` | `a, b = t` | `let a, mut b = t` |
| Nested generics | `Vec<Vec<i32>>` | `list[list[int]]` | `list[list[int]]` |

---

## Type System

| Concept | Rust | Python | tl |
|---|---|---|---|
| Optional value | `Option<T>` | `Optional[T]` / `T \| None` | `Option[T]` |
| Union / sum type | `enum Foo { A(i32), B(str) }` | `Union[int, str]` | `Union[int, str]` |
| Any type | `dyn Any` | `Any` | `Any` |
| Newtype wrapper | `struct Meters(f64)` (newtype pattern) | `NewType('Meters', float)` | `new_type Meters: float` |
| Type cast | `x as i64`, `From::from(x)` | `int(x)`, `float(x)` | `x=>int`, `x=>float` |
| Custom cast | `impl From<T> for U` | `__int__`, `__float__` | `fn __cast__[TargetType](self)` |
| Runtime type check | `x.is::<T>()` | `isinstance(x, T)` | `x is T` |
| Negative type check | — | `not isinstance(x, T)` | `x is not T` |
| Type narrowing | `if let Some(v) = x` | `isinstance` guard (mypy) | `if x is T:` (narrows scope) |

---

## Functions

| Concept | Rust | Python | tl |
|---|---|---|---|
| Function definition | `fn f(x: i32) -> i32 { … }` | `def f(x: int) -> int: …` | `fn f(let x: int) -> int: …` |
| Param mutability | `fn f(mut x: i32)` | — | `fn f(mut x: int)` (caller must pass `mut`) |
| Default params | — (use `Option` / builder) | `def f(x=1)` | `fn f(let x: int = 1)` |
| Keyword args | — | `f(x=1)` | `f(x=1)` |
| Return type | `-> T` | `-> T` (optional) | `-> T` |
| No return | `-> ()` | `-> None` | `-> None` |
| Closure / lambda | `\|x\| x * 2` | `lambda x: x * 2` | inner `fn` capturing outer variables |
| Immutable capture | move semantics | cell-based | deep copy at definition time |
| Mutable capture | `Rc<RefCell<T>>` | cell (implicitly) | `mut` → shared `Rc<RefCell>` cell |
| Generator | custom `Iterator` impl | `def f(): yield …` | `gen f(): yield …` |
| Decorator | macro / wrapper | `@decorator` | `@decorator` |
| Function type annotation | `fn(i32) -> i32` / `impl Fn` | `Callable[[int], int]` | `function[let int]->int` |
| Named-param function type | — | `Callable` | `function{let name: int}->int` |
| Zero-param function type | `fn() -> i32` | `Callable[[], int]` | `function[]->int` |
| Bare function (any sig) | `dyn Fn` | `Callable[…, Any]` | `function` |

---

## Classes & Structs

| Concept | Rust | Python | tl |
|---|---|---|---|
| Definition | `struct Foo { x: i32 }` + `impl Foo { … }` | `class Foo: …` | `class Foo: …` |
| Constructor | `impl Foo { fn new(…) -> Self }` | `def __init__(self, …)` | auto-generated from fields; or explicit `fn __init__(mut self, …)` |
| Immutable field | `x: T` (immutable by default) | `self.x = …` | `let x: T` |
| Mutable field | `x: T` + `mut` binding | `self.x = …` | `mut x: T` |
| Field default | — | `x = default` | `mut x: int = 0` |
| Instance method | `fn method(&self) -> T` | `def method(self) -> T` | `fn method(self) -> T` |
| Mutable method | `fn method(&mut self)` | `def method(self)` | `fn method(mut self)` |
| Self type | `Self` | `Self` (typing) | `Self` |
| Static method | `fn static_method()` inside `impl` | `@staticmethod` | `static fn method()` |
| Class method | — | `@classmethod def f(cls)` | `class_method fn f(cls: type[Self])` |
| Static field | — | class-level variable | `static mut field: T = …` |
| Operator overload | `impl Add for Foo` | `def __add__(self, other)` | `fn __add__(self, other)` |
| Subscript get | `impl Index` | `def __getitem__(self, k)` | `fn __getitem__(self, idx: int)` |
| Subscript set | `impl IndexMut` | `def __setitem__(self, k, v)` | `fn __setitem__(mut self, idx, val)` |
| Freeze hook | — | — | `fn __freeze__(mut self)` |
| Repr | `impl Display` | `def __repr__(self)` | `fn __repr__(self) -> str` |

---

## Traits / Interfaces

| Concept | Rust | Python | tl |
|---|---|---|---|
| Trait / interface | `trait Foo { fn method(&self); }` | `class Foo(Protocol): …` or ABC | `trait Foo: fn method(self) -> T: ...` |
| Abstract method | default: must implement | `@abstractmethod` | method body is `...` |
| Implementing trait | `impl Foo for Bar { … }` | `class Bar(Foo): …` | `class Bar(Foo): …` |
| Multiple traits | `T: Foo + Bar` | `class Bar(A, B): …` | `class Bar(A, B): …` |
| Trait-qualified access | `<Self as Trait>::method()` | — | `self::TraitName.field` |
| Trait as constraint | `fn f<T: Foo>(x: T)` | `TypeVar('T', bound=Foo)` | `fn f[T: Foo](x: T)` |
| Compound constraint | `T: Foo + Bar` | `T: TypeVar …` | `T: Foo and Bar` |
| Check membership | `x: &dyn Foo` / `Any::downcast` | `isinstance(x, Foo)` | `x is Foo` |

---

## Generics / Templates

| Concept | Rust | Python | tl |
|---|---|---|---|
| Generic function | `fn f<T>(x: T) -> T` | `def f(x: T) -> T` (TypeVar) | `fn f[T: Trait](x: T) -> T` |
| Generic class | `struct Box<T> { … }` | `class Box(Generic[T]): …` | `class Box[T: Trait]: …` |
| Instantiate | `Box::<i32> { … }` / inferred | `Box[int](…)` | `Box[MyInt](…)` |
| Call with explicit type | `foo::<i32>(x)` | `foo[int](x)` (PEP 695) | `foo[MyInt](x)` |

---

## Enums

| Concept | Rust | Python | tl |
|---|---|---|---|
| Basic enum | `enum Color { Red, Green, Blue }` | `class Color(Enum): Red=0 …` | `enum Color: Red; Green; Blue` |
| Auto-value | starts at 0 (discriminant) | `auto()` | starts at 0 |
| Explicit value | `Red = 5` (discriminant) | `Red = 5` | `Red = 5` |
| Access variant | `Color::Red` | `Color.Red` | `Color.Red` |
| Get int value | `Color::Red as i32` | `Color.Red.value` | `Color.Red.value` |
| Data-carrying variant | `enum Msg { Move { x: i32, y: i32 } }` | — | — (not supported) |
| Type-check variant | `matches!(x, Color::Red)` | `x == Color.Red` | `x is enum_item_Color` |

---

## Control Flow

| Concept | Rust | Python | tl |
|---|---|---|---|
| If / else if / else | `if … { } else if … { } else { }` | `if …: elif …: else:` | `if …: elif …: else:` |
| While loop | `while cond { }` | `while cond:` | `while cond:` |
| For loop | `for x in iter { }` | `for x in iter:` | `for x in iter:` |
| Range | `0..n`, `(0..n).step_by(2)` | `range(n)`, `range(0,n,2)` | `range(n)`, `range(0,n,2)` |
| Break / Continue | `break`, `continue` | `break`, `continue` | `break`, `continue` |
| Pass (no-op) | — | `pass` | `pass` |
| Pattern matching | `match x { pat => expr, … }` | `match x: case pat: …` (3.10+) | `match (x): case val: … / is Type: …` |
| Wildcard arm | `_ => …` | `case _: …` | `case _: …` |
| Type-pattern arm | `if let Foo(v) = x` | `case Foo(): …` | `is Foo: …` |
| If as expression | `if cond { val } else { val }` | — | `if cond ->T: block_return v else: block_return v` |
| For as expression | — | `[x for x in …]` (list comp) | `for x in iter ->list[T]: loop_yield x` |
| While as expression | — | — | `while cond ->list[T]: loop_yield v` |
| Match as expression | `match x { … }` (always expr) | — | `match (x) ->T: case …: block_return v` |
| Inline block expression | `{ let x = 1; x + 1 }` | — | `block ->T: block_return val` |
| Yield from loop body | — | `yield` in generator | `loop_yield val` |
| Exit block with value | `break val` (loop-break) | — | `block_return val` |
| Custom iterator | `impl Iterator for Foo` | `def __iter__` / `__next__` | `gen __iter__(self): yield …` |

---

## Error Handling

| Concept | Rust | Python | tl |
|---|---|---|---|
| Try/catch | — | `try: … except E as e: …` | `try: … except E as e: …` |
| Finally | — | `finally:` | `finally:` |
| Bare catch | — | `except:` | `except:` |
| Raise | `panic!()` / `return Err(…)` | `raise ValueError("msg")` | `raise ValueError("msg")` |
| Error propagation | `?` operator | re-raise | manual re-raise |
| Built-in errors | `std::io::Error`, etc. | `TypeError`, `ValueError`, … | `TypeError`, `ValueError`, `IndexError`, `KeyError`, `ZeroDivisionError`, `NameError`, `AccessError` |

---

## Access Control

| Level | Rust | Python | tl |
|---|---|---|---|
| Public | `pub` on each item | convention (`_` prefix for private) | `public:` section marker (default) |
| Private | no modifier (crate-private) | `_name` convention | `private:` section marker |
| Protected | — | `__name` name mangling | `protected:` section marker |
| Scope of protection | module / crate boundary | convention only | class / same-trait classes |

---

## Async / Concurrency

| Concept | Rust | Python | tl |
|---|---|---|---|
| Async function | `async fn f() -> T` | `async def f() -> T` | — (uses submit model, not async fn) |
| Await | `.await` | `await expr` | — |
| Thread pool | `tokio::spawn` / `rayon` | `concurrent.futures.ThreadPoolExecutor` | `AsyncManager(num_thread=N)` |
| Submit task | `pool.spawn(async { … })` | `executor.submit(fn, …)` | `mgr <- async->T: body` |
| Task capture | move closure | closure | `mut` → shared ref; `let` → deep clone |
| Wait for all | `join_all(…).await` | `executor.map` / `wait` | `mgr.wait_for_finish()` |
| Check done | `.is_finished()` | `.done()` | `mgr.all_done()` |
| Task states | — | `Future.RUNNING`, etc. | `Async.Waiting`, `Async.Running`, `Async.Done` |

---

## Module / Import

| Concept | Rust | Python | tl |
|---|---|---|---|
| Import module | `use crate::foo;` | `import foo` | `import foo` |
| Import item | `use foo::bar;` | `from foo import bar` | `from foo import bar` |
| Force source | — | — | `import[ar] module` |
| Force compiled | — | — | `import[arc] module` |
| Python interop | — | native | `import[py] module` / `import[py-int]` |
| Auto (prefer compiled) | — | — | `import module` (prefers `.arc` if present) |

---

## Special / Unique to tl

| Feature | Syntax | Notes |
|---|---|---|
| Freeze binding | `freeze x` | Promotes `mut` to permanently immutable; calls `__freeze__` hook if defined |
| Newtype from primitive | `new_type Meters: float` | Distinct type sharing all methods; inner value via `.value` |
| Newtype cast | `5.0=>Meters` | `=>` operator; reverse with `m=>float` |
| Static shared closure state | `static mut n = 0` inside `fn` | Single cell shared across all invocations of the outer function |
| Trait-qualified field access | `self::Trait.field` | Disambiguates when multiple traits define the same field name |
| Partial compile to native | `cargo run -- --compile mod.ar` | Emits `.arc` (compiled binary) + `.ars` (type stub for IDE) |
| Native dispatch | auto on import | `import` prefers `.arc`; eligible fns run as native code (~100–200× faster for typed int/float) |
