---
name: parser-internals
description: Use when modifying src/parser/ — the recursive-descent parser (statement/expression precedence chain, class/trait parsing, type/param parsing, or import/module loading during parsing). Gives the module map, Parser struct fields, parse_stmt dispatch table, expression precedence chain, and the full list of parse-time validations.
---

# Parser — Implementation Reference

`src/parser/` is a hand-written recursive-descent parser.
**Input**: `Vec<Spanned>` (token stream from the lexer)
**Output**: `Vec<Stmt>` (the full program AST)

Module loading also happens here — imported modules are parsed recursively and their ASTs are embedded inside `Stmt::Import.body` before the interpreter ever runs. For the module-loading dispatch itself (by `[lang]` tag), see the `importation` skill.

---

## Module Map

```
src/parser/
├── mod.rs      — Parser struct, shared helpers (current/peek1/eat/advance/skip_newlines)
├── stmts.rs    — parse_program / parse_block / parse_stmt and all statement parsers
├── exprs.rs    — expression precedence chain (parse_expr → ... → parse_primary)
├── types.rs    — type annotations, template params, function params
├── classes.rs  — class / trait / access-section body parsing
└── imports.rs  — import / from-import parsing and module loading (see importation skill)
```

---

## Parser Struct (`mod.rs`)

```rust
pub struct Parser {
    tokens:              Vec<Spanned>,
    pos:                 usize,
    known_traits:        HashMap<String, (Vec<TemplateParam>, Vec<(...)>, Vec<String>)>,
    class_or_trait_depth: usize,
    known_new_types:     HashSet<String>,
    source_dir:          PathBuf,   // first search dir for imports
    root_dir:            PathBuf,   // fallback search dir (shared to sub-parsers unchanged)
    module_cache:        HashMap<(String, PathBuf), Vec<Stmt>>,
    loading:             HashSet<PathBuf>,  // circular import detection
}
```

Key parser state:

| Field | Purpose |
|-------|---------|
| `known_traits` | Registered traits → used when parsing class inheritance to auto-generate `__init__` and verify fields |
| `class_or_trait_depth` | `> 0` inside a class/trait body — guards `Self` type usage |
| `known_new_types` | Names declared with `new_type` — reassignment is a parse-time error |
| `module_cache` | Avoids re-parsing the same module twice; shared across sub-parsers via clone+merge |
| `loading` | Path set for circular import detection |

The built-in `Error` trait is pre-registered in `Parser::new()` so user classes can inherit from it without importing.

---

## Primitive helpers (`mod.rs`)

| Helper | What it does |
|--------|-------------|
| `current()` | Returns `&Token` at `pos`; `Token::Eof` past the end |
| `peek1()` | Returns `&Token` at `pos + 1` (one-token lookahead) |
| `current_span()` | Returns `Span` at `pos` (filename / line / column) |
| `advance()` | Increments `pos` |
| `eat(expected)` | Asserts current == expected and advances; error otherwise |
| `skip_newlines()` | Skips `Newline / Indent / Dedent / Semicolon` tokens |

---

## Statement Parsing (`stmts.rs`)

### Entry points

`parse_program()` — loops `parse_stmt()` until `Eof`.
`parse_block()` — consumes `Newline Indent … Dedent` and collects statements inside.

### `parse_stmt()` dispatch table

| Leading token | Result |
|---------------|--------|
| `let` | `Stmt::Let` / `Stmt::LetTuple` / `Stmt::DebugLet` |
| `mut` | `Stmt::Mut` / `Stmt::LetTuple` |
| `const` | `Stmt::Const` |
| `static mut` | `Stmt::Static` |
| `freeze` | `Stmt::Freeze` |
| `return` | `Stmt::Return(None\|Some(expr))` |
| `block_return` | `Stmt::BlockReturn(expr)` |
| `loop_yield` | `Stmt::LoopYield(expr)` |
| `yield` | `Stmt::Yield(expr)` |
| `break / continue / pass` | `Stmt::Break / Continue / Pass` |
| `if` | `Stmt::If { branches, else_body }` |
| `while` | `Stmt::While { cond, body }` |
| `for` | `Stmt::For { targets, iter, body }` |
| `match` | `Stmt::Match { subject, arms }` |
| `block` | `Stmt::Block(body)` |
| `try` | `Stmt::Try { body, handlers, finally_body }` |
| `raise` | `Stmt::Raise { exc, span }` |
| `fn` | `Stmt::FnDef` |
| `gen` | `Stmt::GenDef` |
| `class` | `Stmt::ClassDef` |
| `trait` | `Stmt::TraitDef` |
| `enum` | `Stmt::EnumDef` |
| `new_type` | `Stmt::NewTypeDef` |
| `@` | decorator list → `FnDef` or `ClassDef` |
| `import / from` | `Stmt::Import / FromImport` (module loaded immediately) |
| `Ident` | `parse_ident_stmt()` — assignment, compound-assign, async-assign, or expression statement |
| other | `Stmt::Expr(parse_expr())` |

### Identifier statement disambiguation (`parse_ident_stmt`)

Uses `peek1()` to decide:
- `name <-` → `Stmt::AsyncAssign`
- `name =` → `Stmt::Assign`
- `name +=` etc. → `Stmt::CompoundAssign`
- otherwise: parse full expression, then check if `=` or `+=` follows for attribute assignment; else `Stmt::Expr`

### Abstract body detection (`is_abstract_body`)

Checks `tokens[pos..pos+2]` for `Newline Indent Ellipsis` — used for trait virtual methods and `import[rs]` stubs.

---

## Expression Parsing (`exprs.rs`)

Precedence chain (lowest → highest):

```
parse_expr
  └─ parse_or          (or)
       └─ parse_and         (and)
            └─ parse_not         (not)
                 └─ parse_comparison  (== != < > <= >= is is_not in not_in)
                      └─ parse_bitor       (|)
                           └─ parse_bitxor      (^)
                                └─ parse_bitand      (&)
                                     └─ parse_shift       (<< >>)
                                          └─ parse_add        (+ -)
                                               └─ parse_mul     (* / // % @)
                                                    └─ parse_unary  (- ~ +)
                                                         └─ parse_power  (** right-assoc)
                                                              └─ parse_postfix  (call/attr/subscript/slice)
                                                                   └─ parse_primary
```

`parse_primary` handles: literals (`int`, `float`, `str`, `bool`, `None`), f-strings, identifiers, parenthesised expressions / tuples, list literals `[...]`, dict/set literals `{...}`, `if`/`for`/`while`/`match`/`block` **expressions**, `lambda`, `yield from`.

Set vs. dict disambiguation in `{...}`: after parsing the first expression, lookahead at the next token — `:` means dict entry, `,` or `}` means set.

`is` / `is not` are parsed in `parse_comparison` as `Expr::IsType { negated }`.
`in` / `not in` are parsed as `BinOp::In / BinOp::NotIn`.

---

## Type and Parameter Parsing (`types.rs`)

`parse_type_expr()` — parses a full type string including generics (`list[int]`, `dict[str, float]`, `Optional[T]`, `Union[A, B]`, `function[let T]->R`, etc.) and returns it as a plain `String`.

`parse_template_params()` — parses `[T: Trait1 and Trait2, U]` on `fn`/`class`/`trait` definitions.

`parse_param()` — parses a single function parameter: optional `let`/`mut` qualifier, name, optional `: Type`, optional `= default`. Default value after non-default raises a parse error (`validate_param_defaults`).

`parse_opt_return_type()` — parses optional `-> Type` before `:` in `if`/`for`/`while`/`match`/`block` expressions and at the statement level (where it is consumed and discarded).

---

## Class and Trait Parsing (`classes.rs`)

`parse_trait_def()` — parses `trait Name[T: C]:` body; registers into `known_traits` on success. Enforces that all methods have type annotations (return type + parameter types).

`parse_class_def()` — parses `class Name[T](Base1, Base2):` body. For each inherited trait in `known_traits`, auto-generates the expected `__init__` parameters and field declarations if not already present.

`parse_class_body()` — shared body parser for both class and trait. Handles access-control section markers (`public:` / `private:` / `protected:`) which set the `Accessibility` on all subsequent member declarations.

`parse_fn_def_with_flags(decorators, is_static, is_class_method)` — shared function-definition parser used by `fn`, `static fn`, `class_method fn`, and decorated variants.

---

## Parse-time Validations

| Check | Where |
|-------|-------|
| Non-default param after default param | `validate_param_defaults` (types.rs) |
| `return` inside `gen` body | `body_has_return` (stmts.rs) |
| `mut` param in `gen` (except `self`) | `parse_gen_def` (stmts.rs) |
| `new_type` name reassigned | `parse_ident_stmt` / `parse_compound` (stmts.rs) |
| Trait inheriting from another trait | `parse_trait_def` (classes.rs) |
| Missing type annotations in trait methods | `parse_trait_def` (classes.rs) |
| Mixed `case` / `is` arms in `match` | `parse_match_arms` (stmts.rs) |
| Circular imports | `loading` HashSet in `load_tl_module` (imports.rs) |
