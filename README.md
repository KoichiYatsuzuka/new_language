# test_lang

## 概要（Overview）

`test_lang` は、Python 風のインデント構文を土台に、明示的な変数宣言、静的型検査、trait、template などを試している独自スクリプト言語です。

将来的なターゲットは LLVM IR ですが、現在の実装は Rust 製の tree-walk interpreter です。実行時は `.tl` ソースを字句解析、構文解析、静的型検査したあとにインタープリタで評価します。将来的には部分的にコンパイルすることで高速化を可能にする予定です。また、Python などで書かれた外部ライブラリを使用可能にする予定です。

`test_lang` is a custom scripting language built on Python-style indented syntax, exploring explicit variable declarations, static type checking, traits, templates, and more.

The ultimate target is LLVM IR, but the current implementation is a Rust-based tree-walk interpreter. At runtime, `.tl` source code is lexically analyzed, parsed, statically type-checked, and then evaluated by the interpreter. In the future, we plan to enable faster execution through partial compilation and integration with external libraries written in Python and other languages.

---

## クイックスタート（Quick Start）

```bash
cargo build
cargo run -- -src examples/showcase.tl
cargo run -- examples/hello.tl
cargo test
```

`-src <file.tl>` と位置引数のどちらでも実行できます。引数がない場合は標準入力からソースを読みます。/ You can run using either `-src <file.tl>` or positional arguments. If no arguments are provided, the source is read from standard input.

---

## サンプル（Examples）

基本的な変数宣言と関数定義 / Basic variable declarations and function definitions:

```tl
let name = "world"
mut x = 10
x += 5

fn greet(who: str) -> None:
    print("Hello,", who)

greet(name)
print("x =", x)
```

Trait と Template を用いたジェネリック型の例 / Example using traits and templates for generic types:

```tl
trait Printable:
    fn to_str(self) -> str:
        ...

class Point(Printable):
    mut x: int
    mut y: int

    fn to_str(self) -> str:
        return "Point"

fn describe[T: Printable](item: T) -> str:
    return item.to_str()

let p = Point(1, 2)
print(describe[Point](p))
```

---

## プロジェクト構成（Project Structure）

```text
test_lang/
├── src/
│   ├── main.rs          # CLI と実行フロー (CLI and execution flow)
│   ├── token.rs         # Token / Span / Spanned
│   ├── lexer.rs         # 字句解析器 (Lexical analyzer)
│   ├── ast.rs           # AST 定義 (AST definitions)
│   ├── parser.rs        # 再帰下降パーサー (Recursive descent parser)
│   ├── type_check.rs    # 静的型検査 (Static type checker)
│   └── interpreter.rs   # tree-walk interpreter
├── spec/                # 言語仕様メモ (Language specification notes)
├── examples/            # 正常系・エラー系サンプル (Success and error case samples)
├── stdlib/              # 標準 trait の実験置き場 (Standard trait experiment area)
└── vscode-extension/    # .tl 用 VS Code 拡張 (VS Code extension for .tl)
```

---

## 実行フロー（Execution Flow）

```text
source.tl
  -> Lexer        Vec<Spanned> を生成 (Generate Vec<Spanned>)
  -> Parser       Vec<Stmt> の AST を生成 (Generate AST (Vec<Stmt>))
  -> TypeChecker  StaticTypeError をまとめて報告 (Collect and report StaticTypeErrors)
  -> Interpreter  AST を実行 (Execute AST)
```

静的型検査でエラーが 1 件でも見つかった場合は、全件を表示して実行せずに終了します。/ If any errors are found during static type checking, all errors are displayed and the program exits without running.

---

## 実装済みの主な機能（Implemented Features）

- Python 風のインデントブロック、コメント、複数行文字列 / Python-style indented blocks, comments, and multi-line strings
- `let` / `mut` / `const` による変数宣言と可変性チェック / Variable declarations with `let` / `mut` / `const` and mutability checking
- 算術、比較、論理、ビット演算、複合代入 / Arithmetic, comparison, logical, bitwise operations, and compound assignments
- `if` / `elif` / `else`、`while`、`for ... in ...`、`block` / `if` / `elif` / `else`, `while`, `for ... in ...`, `block`
- `break` / `continue` / `pass` / `return` / `block_return` / `block_yield`
- リストリテラルと組み込み関数 `print`、`range`、`len` / List literals and built-in functions: `print`, `range`, `len`
- 関数定義、再帰、位置引数、キーワード引数 / Function definitions, recursion, positional arguments, keyword arguments
- 関数・メソッドのオーバーロード / Function and method overloading
- クラス、インスタンスフィールド、クラス変数、メソッド、`self` / Classes, instance fields, class variables, methods, `self`
- trait 定義、trait フィールド、virtual method、trait 実装チェック / Trait definitions, trait fields, virtual methods, trait implementation checking
- template 関数、template class、template generator、trait 制約 / Template functions, template classes, template generators, trait constraints
- `Self` 型と `Self(...)` コンストラクタ / `Self` type and `Self(...)` constructors
- `new_type NewName: OriginalType`
- `freeze` による `mut` 変数・インスタンスの凍結 / `freeze` for immutability of `mut` variables and instances
- `gen` / `yield` による eager generator と `next()` / `gen` / `yield` for eager generators and `next()`
- 型注釈としての `Union[...]` / `Option[...]` / Type annotations using `Union[...]` / `Option[...]`
- VS Code 拡張によるシンタックスハイライトと簡易 inlay hint / VS Code extension with syntax highlighting and basic inlay hints

---

## 型検査（Type Checking）

`src/type_check.rs` は、AST を実行前に走査して `StaticTypeError` を収集します。現在は主に以下を検査します。 / `src/type_check.rs` traverses the AST before execution and collects `StaticTypeError` instances. It currently checks mainly the following:

- `let` / `const` への再代入 / Re-assignment to `let` / `const` variables
- 互換性のない大小比較 / Incompatible comparison operations (e.g., `<`, `>`)
- 関数呼び出しの引数個数・型不一致 / Function call argument count and type mismatches
- パラメータや戻り値の型アノテーション欠如 / Missing type annotations for parameters or return values
- 不明なキーワード引数 / Unknown keyword arguments
- オーバーロード候補不一致 / Mismatched overload candidates
- `Self` 型引数の不一致 / Mismatched `Self` type arguments
- `Union` / `Option` まわりの型不一致 / Type mismatches related to `Union` / `Option`

型が静的に分からない箇所は、原則として機能を厳しく制限し、明示的なダウンキャストを要求します。 / For code where types cannot be statically determined, functionality is generally restricted and explicit downcasting is required.

---

## 例（Examples Directory）

代表的な動作確認ファイルです。/ Representative test files for verifying functionality.

- `examples/showcase.tl`: 主要機能のまとめ / Summary of main features
- `examples/type_errors.tl`: 静的型エラー例 / Static type error examples
- `examples/fn_kwargs_success.tl` / `examples/fn_kwargs_errors.tl`: キーワード引数 / Keyword arguments
- `examples/overload_success.tl` / `examples/overload_errors.tl`: オーバーロード / Overloading
- `examples/trait_sample.tl` / `examples/trait_template.tl`: trait / Traits
- `examples/template_sample.tl` / `examples/template_constraint_error.tl`: template / Templates
- `examples/self_type.tl` / `examples/self_type__errors.tl`: `Self`
- `examples/new_type.tl` / `examples/new_type__errors.tl`: `new_type`
- `examples/freeze.tl`: `freeze`
- `examples/generator.tl`: generator / Generators
- `examples/union_option.tl` / `examples/union_option__errors.tl`: `Union` / `Option`

エラー確認用のサンプルは、ファイル名に `__errors` または `_errors` を含めています。/ Sample files for error verification are named with `__errors` or `_errors` suffix.

---

## 開発コマンド（Development Commands）

```bash
cargo build
cargo run -- -src examples/showcase.tl
cargo test
cargo test <name>
cargo clippy
cargo fmt
```

仕様を追加した場合は、実装のユニットテストに加えて、正常系の example を追加してください。エラー動作が仕様に含まれる場合は、期待したエラーで終了する `__errors` example も追加します。/ When adding new specifications, include unit tests in the implementation and add corresponding example files. If error behavior is part of the specification, also add a `__errors` example that demonstrates the expected error termination.

---

## 現在の主な制限（Current Limitations）

- LLVM IR 生成は未実装です。 / LLVM IR code generation is not yet implemented.
- Python 実装は未着手です。 / Python implementation has not been started.
- `import` / `from ... import` は未実装です。 / `import` / `from ... import` are not implemented.
- `try` / `except` / `finally` / `raise` は未実装です。 / `try` / `except` / `finally` / `raise` are not implemented.
- `match` は未実装です。 / `match` statements are not implemented.
- 辞書・セットリテラルは未実装です。 / Dictionary and set literals are not implemented.
- template の静的型検査は限定的で、制約チェックの多くは実行時に行われます。 / Static type checking for templates is limited, with most constraint checks performed at runtime.
- trait は現在、主に parse/type-check/interpreter のための構造で、完全な runtime object ではありません。 / Traits are currently structures primarily for parse/type-check/interpreter use, not complete runtime objects.

---

## VS Code 拡張（VS Code Extension）

```bash
cd vscode-extension
npm install
npm run compile
```

開発モードでは VS Code で拡張プロジェクトを開き、F5 で Extension Development Host を起動します。/ For development mode, open the extension project in VS Code and launch the Extension Development Host by pressing F5.
