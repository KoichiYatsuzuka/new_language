---
name: type-checking
description: Use when modifying src/type_check/ (the static type checker) — internal type representation (InferredType), annotation syntax, inference rules, type compatibility/coercion, type-guard narrowing, access control, overload resolution, or any StaticTypeError/TypeErrorKind variant. Also use when deciding what the checker currently does NOT verify.
---

# Arrow Type System

Reference for the static type checker (`src/type_check/`). Describes the internal type representation, annotation syntax, inference rules, compatibility rules, and all error kinds.

---

## Internal Type Representation (`InferredType`)

The type checker works with the `InferredType` enum defined in `src/type_check/types.rs`.

| Variant | Meaning |
|---------|---------|
| `Int` | `int` primitive |
| `Float` | `float` primitive |
| `Str` | `str` primitive |
| `Bool` | `bool` primitive |
| `None` | `None` value |
| `List` | Untyped list |
| `ListOf(T)` | Homogeneous list — all elements are `T` |
| `Set` | Untyped set |
| `SetOf(T)` | Homogeneous set |
| `Dict` | Untyped dict |
| `DictOf(K, V)` | Dict with key type `K` and value type `V` |
| `Tuple(Vec<T>)` | Fixed-length tuple with per-element types |
| `Union(Vec<T>)` | Union of two or more types |
| `TypeVal` | A type value (class/type object) with unknown inner type |
| `TypeValOf(T)` | A type value that constructs/represents `T` (e.g. the class `Vec2D` itself) |
| `SelfType` | The `Self` placeholder inside a class body |
| `NamedInstance(name)` | An instance of the named class/newtype |
| `Namespace(map)` | An imported module; members are accessed by name |
| `Function { params, return_type }` | Typed function value |
| `Any` | Escape hatch — matches everything but blocks operations |
| `Unresolved` | Unknown type (inference could not determine); silently passes checks |

### `Option[T]` shorthand

`Option[T]` is syntactic sugar for `Union[T, None]`. The Display implementation shows a `Union[T, None]` back as `Option[T]`.

### `Function` variant

```
Function {
    params: Option<Vec<FnTypeParam>>,   // None = bare `function` (untyped), Some = typed
    return_type: Box<InferredType>,
}
```

Each `FnTypeParam` has `name: String`, `mutable: bool`, and `ty: InferredType`.

---

## Type Annotation Syntax

Annotations appear in source code as strings and are parsed by `InferredType::from_ann()` and the parser's `parse_type_expr()`.

### Primitive types

```
int   float   str   bool   None   Any   Self
```

### Collection types

```
list                      # untyped
list[int]                 # ListOf(Int)
set[str]                  # SetOf(Str)
dict[str, int]            # DictOf(Str, Int)
tuple[int, str, float]    # Tuple([Int, Str, Float])
```

### Union / Optional

```
Union[int, str]           # Union of int and str
Option[int]               # Union[int, None]  (sugar)
```

### Type values

```
type               # bare TypeVal — any class/type object
type[Vec2D]        # TypeValOf(NamedInstance("Vec2D"))
```

### Function types

Two syntaxes — positional (`[...]`) and named (`{...}`):

```
function[int, str]->bool              # positional params, named param1/param2/…
function[let x:int, mut y:str]->bool  # with mut/let qualifiers
function{let x:int, mut y:str}->bool  # named params (keyword-arg style)
function->int                         # no params, returns int
function                              # bare function — no param/return info
```

`let` qualifier = immutable argument. `mut` qualifier = mutable argument (call site must pass a `mut` variable).

### Named class / newtype

Any identifier starting with an uppercase letter and containing only alphanumerics/underscores is parsed as `NamedInstance("Name")`.

### Template parameters

Template params are written with `[T: Trait and Trait2]` syntax on function/class definitions. Multiple constraints are joined with `and`. They are resolved at parse time and stored in `TemplateParam { name, constraints }` on the AST node, but the static type checker currently treats template-parameterised values as `Unresolved`.

---

## Type Inference Rules

Defined in `src/type_check/infer.rs`. The checker calls `infer(expr)` recursively.

### Literals

| Expression | Inferred type |
|------------|--------------|
| `42`, `-1` | `Int` |
| `3.14` | `Float` |
| `"hello"` | `Str` |
| `True` / `False` | `Bool` |
| `None` | `None` |

### Collection literals

- **List**: if empty → `List`; if all elements have the same type `T` → `ListOf(T)`; otherwise → `List`.
- **Set**: same rule as list, yields `Set` or `SetOf(T)`.
- **Tuple**: always `Tuple([t0, t1, …])` with per-element inference.
- **Dict**: if all keys have type `K` and all values have type `V` → `DictOf(K, V)`; otherwise → `Dict`.

### Identifiers

Returns the type recorded in scope. If not found → `Unresolved`.

### Attribute access (`obj.attr`)

- If `obj` is `Any` → `OperationOnAny` error.
- If `obj` is `Union` → `OperationOnUnion` error.
- If `obj` is `Namespace` (imported module) → returns the member's recorded type.
- If `obj` is `NamedInstance(cls)` → access-control check runs (see below) → returns `Unresolved` (field type resolution is partial).

### Unary operators

| Operator | Input | Result |
|----------|-------|--------|
| `-` | `Int` | `Int` |
| `-` | `Float` | `Float` |
| `not` | any | `Bool` |
| `~` | any | `Int` |
| any | `Any` | error `OperationOnAny` |
| any | `Union` | error `OperationOnUnion` |

### Binary operators

See the table below. `Any` or `Union` on either side → error (explicit downcast required).

| Operators | Condition | Result |
|-----------|-----------|--------|
| `==`, `!=`, `<`, `>`, `<=`, `>=`, `and`, `or`, `in`, `not in` | — | `Bool` |
| `+` | `int + int` | `Int` |
| `+` | `float + float`, mixed int/float | `Float` |
| `+` | `str + str` | `Str` |
| `-`, `*`, `**` | int, int | `Int` |
| `-`, `*`, `**` | float or mixed | `Float` |
| `/` | any | `Float` |
| `//`, `%` | `int, int` | `Int` |
| `&`, `\|`, `^`, `<<`, `>>` | — | `Int` |
| `\|`, `&`, `^`, `-` (set–set) | `Set, Set` | `Set` |

Ordered comparisons (`<`, `>`, `<=`, `>=`) are only valid between: `int/int`, `float/float`, `int/float`, `float/int`, `str/str`. Any other pair → `IncompatibleComparison` error.

### Function calls

Handled by `infer_call()`:

1. If the callee's inferred type is `Function { params: Some(ps), return_type }` (a function-type variable), `check_fn_type_call` validates argument count, types, and `mut` requirements. Returns `return_type`.
2. If callee type is `Function { params: None, .. }` → returns `Any` (untyped function).
3. Otherwise, the callee name is looked up in `fn_sigs`. If a single overload matches the argument count, its return type is returned; if multiple overloads exist and exactly one matches count, returns that one's return type; otherwise `Unresolved`.
4. If the callee name is a known class name → returns `NamedInstance(name)`.

### Expression forms (`block:`, `if:`, `for:`, `while:`, `match:` as expressions)

When these appear as expressions with an optional `->Type` annotation:
- If `->Type` is present → `InferredType::from_ann(type)` or `Unresolved` on failure.
- If no annotation → `Unresolved`.

The bodies are still checked recursively.

### Slice (`a[i:j:k]`)

Inferred as `NamedInstance("slice")`.

### Type guard (`x is T`, `x is not T`)

Both yield `Bool`.

### Cast

`value as Type` → `InferredType::from_ann("Type")` or `Unresolved`.

---

## Type Compatibility (`type_matches`)

Defined in `src/type_check/type_utils.rs`. Returns `true` when `arg_ty` satisfies `expected`.

Rules applied in order:

1. `arg_ty == Unresolved` → **compatible** (unknown type passes anything).
2. `expected == Any` → **compatible** (Any accepts everything).
3. `arg_ty == expected` → **compatible** (exact match).
4. `expected == TypeVal` → compatible if `arg_ty` is `TypeValOf(_)` or `TypeVal`.
5. `expected == TypeValOf(E)` → compatible if `arg_ty` is `TypeVal` (untyped) or `TypeValOf(A)` where `A` is compatible with `E` via `type_val_compatible` (see below).
6. Collection coercions:
   - `ListOf(_)` ↔ `List` (both directions)
   - `ListOf(A)` ↔ `ListOf(E)` recursively
   - `SetOf(_)` ↔ `Set` (both directions)
   - `SetOf(A)` ↔ `SetOf(E)` recursively
   - `DictOf(_, _)` ↔ `Dict` (both directions)
   - `DictOf(AK, AV)` ↔ `DictOf(EK, EV)` recursively on both key and value
7. `expected == Union(ts)` → compatible if `arg_ty` is compatible with any member of `ts`.
8. If `arg_ty == NamedInstance(C)` and `C` has a `__cast__[ExpectedType]` method → **compatible** (implicit cast path).
9. Otherwise → **incompatible**.

### `type_val_compatible`

Used when checking `TypeValOf(arg)` against `TypeValOf(expected)`:

1. Exact name match → true.
2. Traverse the `new_type_originals` chain of `arg_name` until `expected_name` is reached → true.
3. Check `class_bases[arg_name]` for direct base match → true.

---

## Type Narrowing (Type Guards)

### `if x is T:` narrowing

When an `if` branch condition is `x is T` where `x` is a simple identifier:
- A new scope is pushed for the branch body.
- `x` is re-declared with type `T` (exactly, not inferred from the Union).

### `if x is not T:` narrowing

Requires `x` to have a `Union` type:
- Removes `T` from the union members.
- If one member remains → narrows to that single type.
- If zero members remain → `Unresolved`.
- If `x` is not `Union` (and not `Unresolved`) → `IsNotOnNonUnion` error.

### `match x:` with type arms

`is Type` arms in a `match` statement narrow `x` inside that arm's scope the same way as `if x is T`.

---

## Function Definitions — Required Annotations

Every parameter except `self` **must** have a type annotation (`: Type`). The function **must** have a return type annotation (`-> Type`). Missing annotations generate:
- `MissingParamTypeAnn { func_name, param_name }`
- `MissingReturnTypeAnn { func_name }`

Generator functions (`gen`) are subject to the same requirement; the missing annotation targets the yield type.

`self` parameters are implicitly typed as `NamedInstance(current_class_name)`.

---

## Class and Trait Type System

### Signature collection (pre-pass)

Before checking, `collect_fn_sigs` scans all `FnDef`, `ClassDef`, `EnumDef`, `TraitDef`, and `NewTypeDef` statements recursively. It populates:

- `fn_sigs`: `name → Vec<FnSig>` for overloaded resolution.
- `class_method_sigs`: `class → method → Vec<FnSig>`.
- `known_class_names`: set of all class/enum names.
- `class_bases`: `class → [base names]`.
- `class_fields`: `class → (field_name → is_mutable)`.
- `class_member_access`: `class → (member_name → Accessibility)` (only non-Public members stored).
- `class_static_methods`: `class → {static method names}`.
- `new_type_originals`: `newtype_name → original_name`.

### Field declarations

Fields are declared with `let`, `mut`, or `const`:

| Kind | Mutable | Default allowed |
|------|---------|-----------------|
| `let` | No | No — `FieldDefaultNotAllowed` error |
| `mut` | Yes | No — `FieldDefaultNotAllowed` error |
| `const` | No | Yes |

### `__cast__` method

A class `C` with method `__cast__[T]` can be passed wherever `T` is expected. The template parameter name (e.g. `T`) maps to the storage key `__cast__[T]` in `class_method_sigs`.

### Access control

Sections `public:` / `private:` / `protected:` set `Accessibility` for all members that follow until the next section. Default is `Public`. Enforcement rules:

| Level | Can access from |
|-------|----------------|
| `Public` | Anywhere |
| `Private` | Only methods of the same class (`current_class_name == class_name`) |
| `Protected` | Same class or a direct subclass (single-level base check) |

Violations produce `PrivateAccessError` or `ProtectedAccessError`.

Access control is checked statically only for attribute accesses on `NamedInstance` values (`Expr::Attr`).

### Immutable field assignment

Assigning to a `let` field (`obj.field = val`) outside `__init__` produces `AssignToImmutableField`. Inside `__init__`, `let` fields can be freely set.

### Static methods

A `static` method called on an instance → `StaticMethodOnInstance` error. Static methods must be called on the class name directly.

### `Self` type

Inside a class, `Self` is used as a type annotation. When the type checker encounters a parameter annotated `Self` in a method, it expects the argument to be `NamedInstance(class_name)`. If the caller passes an instance of a different class → `SelfTypeMismatch`.

---

## New Types

`new_type Name = OriginalType` creates a named alias. The checker:
- Registers `Name` in `known_class_names`.
- Stores `Name → OriginalType` in `new_type_originals`.
- Copies `OriginalType`'s method signatures to `Name`.
- `TypeValOf(NamedInstance("Name"))` is compatible with `TypeValOf(NamedInstance("OriginalType"))` through the chain traversal in `type_val_compatible`.

Built-in new types pre-registered: `path` (original `str`), `Index` (original `int`), `Size` (original `int`).

---

## Built-in Types and Values

Registered at `TypeChecker::new()` in the global scope:

| Name | Scope type |
|------|-----------|
| `int` | `TypeValOf(Int)` |
| `float` | `TypeValOf(Float)` |
| `str` | `TypeValOf(Str)` |
| `bool` | `TypeValOf(Bool)` |
| `Any` | `TypeValOf(Any)` |
| `function` | `TypeValOf(Function { None, Any })` |
| `Error` | `TypeValOf(NamedInstance("Error"))` |
| `begin`, `last` | `NamedInstance("Index")` |
| All exception classes | `TypeValOf(NamedInstance(name))` |

Exception classes registered (all with base `Error`):
`Exception`, `ValueError`, `TypeError`, `NameError`, `AttributeError`, `IndexError`, `KeyError`,
`ZeroDivisionError`, `RuntimeError`, `StopIteration`, `NotImplementedError`, `OverflowError`,
`IOError`, `OSError`, `AssertionError`, `ArithmeticError`, `AccessError`.

---

## Exception Handling

`raise expr` — the type checker calls `is_error_instance_type(ty)`:
- `NamedInstance(cls)` → checks if `cls` transitively implements trait `Error` via `class_bases`.
- `Union(types)` → all members must implement `Error`.
- Anything else → `InvalidRaiseType` error.

`except` handlers: if a handler binds a name (`except E as e:`), the variable `e` is declared as `Unresolved` (catching the exact type is not tracked yet).

---

## Tuple Unpacking

`let x, mut y = expr`:
- Every target must use `let`, `mut`, or `_` (wildcard). Bare names without qualifier → `TupleUnpackMissingQualifier`.
- If the RHS has type `Tuple([t0, t1, …])`:
  - Without wildcard: exact count match required, else `TupleUnpackArityMismatch`.
  - With wildcard (`_`): named targets must be ≤ tuple length.
- Each named target is declared with the corresponding tuple element type (or `Any` if the index is out of range).

---

## Mutability Checking at Call Sites

When calling a `function[mut x:T]->R` variable, the argument for the `mut` parameter must be a mutable variable (i.e. `Expr::Ident(name)` where `name` is declared as `mutable=true`). Any other expression → `CallMutParamWithImmutableArg`. This applies to both positional and keyword arguments.

Ordinary named function calls do not check `mut` at the call site (only `function`-typed variables do).

---

## Overload Resolution

Multiple `fn` definitions with the same name at the same scope form an overload set. Resolution:
- If no overload matches the call's argument count → `CallArgCountMismatch` (single overload) or `NoMatchingOverload` (multiple).
- If exactly one overload matches count → type-check arguments against that overload's parameter types.
- If multiple overloads match count → no type checking performed (ambiguous).

---

## Decorator Checking

Function decorators must:
- Have at least one parameter.
- First parameter must be `function` type.
- Return type (if annotated) must be `function` type.

Class decorators must:
- `__init__`'s second parameter must be `type` (or `type[T]`).
- `__call__`'s return type must be `type`.

Both flavours are checked only when the decorator is a simple identifier (`Expr::Ident`). Anything else is not checked.

---

## `block_return` Restrictions

`block_return` cannot be used directly inside a `for` or `while` expression body. Use `loop_yield` instead (which accumulates values), or nest inside an `if`, `match`, or `block:` expression. Violation → `BlockReturnInLoopExpr`.

The depth counter `block_return_forbidden_depth` is incremented on entering a `ForExpr`/`WhileExpr` body and decremented on exit. It is reset to 0 on entering a function body.

---

## Import Type Tracking

When a module is imported, `collect_module_types` does a shallow scan of the module's top-level AST:

- `ClassDef` → `TypeValOf(NamedInstance(name))`
- `FnDef` → `Function { params: Some([...]), return_type }` built from the function's annotations
- `Let`, `Mut`, `Const`, `Static`, tuple targets → `Unresolved`

The resulting `HashMap<String, InferredType>` is stored as `InferredType::Namespace(map)` under the module's binding name. Member access on a `Namespace` returns the recorded type directly.

---

## All Error Kinds (`TypeErrorKind`)

| Variant | When raised |
|---------|------------|
| `IncompatibleComparison { lhs, rhs, op }` | Ordered comparison (`<`, `>`, `<=`, `>=`) between incompatible types |
| `AssignToImmutable { name }` | Assignment to a `let` or `const` variable |
| `CallArgCountMismatch { func_name, expected_min, expected_max, got }` | Wrong number of arguments (single overload or function-type call) |
| `CallArgTypeMismatch { func_name, param_index, expected, got }` | Argument type does not match parameter type |
| `MissingParamTypeAnn { func_name, param_name }` | Non-`self` parameter has no type annotation |
| `MissingReturnTypeAnn { func_name }` | Function has no return type annotation |
| `UnknownKeywordArg { func_name, arg_name }` | Keyword argument name not found in the signature |
| `NoMatchingOverload { func_name, got, available }` | Overload call — no overload has a matching argument count |
| `SelfTypeMismatch { method, param_name, expected_class, got_class }` | `Self`-typed parameter receives an instance of the wrong class |
| `OperationOnAny { op }` | Any binary/unary op or attribute access on `Any` — explicit downcast required |
| `OperationOnUnion { union_type, op }` | Same as above but on a `Union` type |
| `IsNotOnNonUnion { var_name, var_type }` | `x is not T` guard where `x` is not a `Union` or `Optional` |
| `CallMutParamWithImmutableArg { func_name, param_name }` | `mut` parameter passed an immutable expression |
| `InvalidDecorator { reason }` | Decorator signature is incompatible with the decorated target |
| `TupleUnpackMissingQualifier { name }` | Tuple-unpack target has no `let`/`mut` qualifier |
| `TupleUnpackArityMismatch { tuple_len, target_count, has_wildcard }` | Number of unpack targets does not match tuple length |
| `AssignToImmutableField { field_name, class_name }` | Assigning to a `let` field outside `__init__` |
| `PrivateAccessError { member_name, class_name }` | Accessing a `private` member from outside the class |
| `ProtectedAccessError { member_name, class_name }` | Accessing a `protected` member from a non-subclass |
| `StaticMethodOnInstance { method_name, class_name }` | Calling a `static` method on an instance |
| `BlockReturnInLoopExpr` | `block_return` used directly in a `for`/`while` expression body |
| `InvalidRaiseType { got }` | `raise expr` where `expr` does not implement trait `Error` |
| `FieldDefaultNotAllowed { field_name, kind }` | `let`/`mut` field has a default value (only `const` fields may) |

---

## What the Type Checker Does NOT Check

- **Return type correctness** — the checker records the return type annotation but does not verify that returned values match it.
- **Subscript types** — `a[i]` always yields `Unresolved`.
- **Attribute types beyond `Namespace`** — `obj.field` on a `NamedInstance` yields `Unresolved` (except access-control and immutability checks still run).
- **Template instantiation** — `T[int]` yields `Unresolved`.
- **`TraitAccess`** — trait namespace access yields `Unresolved`.
- **`except` handler type** — the caught variable is always `Unresolved`.
- **`mut` at call sites for regular named functions** — only enforced for `function`-typed variable calls.
