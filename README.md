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

`-src <file.tl>` と位置引数のどちらでも実行できます。引数がない場合は標準入力からソースを読みます。

```bash
cargo build
cargo run -- -src examples/showcase.tl
cargo run -- examples/hello.tl
cargo test
```

You can run using either `-src <file.tl>` or positional arguments. If no arguments are provided, the source is read from standard input.

---

## サンプル（Examples）

基本的な変数宣言と関数定義：

```tl
let name = "world"
mut x = 10
x += 5

fn greet(who: str) -> None:
    print("Hello,", who)

greet(name)
print("x =", x)
```

Trait と Template を用いたジェネリック型の例：

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

Basic variable declarations and function definitions:

```tl
let name = "world"
mut x = 10
x += 5

fn greet(who: str) -> None:
    print("Hello,", who)

greet(name)
print("x =", x)
```

Example using traits and templates for generic types:

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
│   ├── main.rs          # CLI と実行フロー
│   ├── token.rs         # Token / Span / Spanned
│   ├── lexer.rs         # 字句解析器
│   ├── ast.rs           # AST 定義
│   ├── parser.rs        # 再帰下降パーサー
│   ├── type_check.rs    # 静的型検査
│   └── interpreter.rs   # tree-walk interpreter
├── spec/                # 言語仕様メモ
├── examples/            # 正常系・エラー系サンプル
├── stdlib/              # 標準 trait の実験置き場
└── vscode-extension/    # .tl 用 VS Code 拡張
```

```text
test_lang/
├── src/
│   ├── main.rs          # CLI and execution flow
│   ├── token.rs         # Token / Span / Spanned
│   ├── lexer.rs         # Lexical analyzer
│   ├── ast.rs           # AST definitions
│   ├── parser.rs        # Recursive descent parser
│   ├── type_check.rs    # Static type checker
│   └── interpreter.rs   # Tree-walk interpreter
├── spec/                # Language specification notes
├── examples/            # Success and error case samples
├── stdlib/              # Standard trait experiment area
└── vscode-extension/    # VS Code extension for .tl
```

---

## 実行フロー（Execution Flow）

```text
source.tl
  -> Lexer        Vec<Spanned> を生成
  -> Parser       Vec<Stmt> の AST を生成
  -> TypeChecker  StaticTypeError をまとめて報告
  -> Interpreter  AST を実行
```

静的型検査でエラーが 1 件でも見つかった場合は、全件を表示して実行せずに終了します。

```text
source.tl
  -> Lexer        Generate Vec<Spanned>
  -> Parser       Generate AST (Vec<Stmt>)
  -> TypeChecker  Collect and report StaticTypeErrors
  -> Interpreter  Execute AST
```

If any errors are found during static type checking, all errors are displayed and the program exits without running.

---

## 実装済みの主な機能（Implemented Features）

- Python 風のインデントブロック、コメント、複数行文字列
- `let` / `mut` / `const` による変数宣言と可変性チェック
- 算術、比較、論理、ビット演算、複合代入
- `if` / `elif` / `else`、`while`、`for ... in ...`、`block`
- `break` / `continue` / `pass` / `return` / `block_return` / `block_yield`
- リストリテラルと組み込み関数 `print`、`range`、`len`
- 関数定義、再帰、位置引数、キーワード引数
- 関数・メソッドのオーバーロード
- クラス、インスタンスフィールド、クラス変数、メソッド、`self`
- trait 定義、trait フィールド、virtual method、trait 実装チェック
- template 関数、template class、template generator、trait 制約
- `Self` 型と `Self(...)` コンストラクタ
- `new_type NewName: OriginalType`
- `freeze` による `mut` 変数・インスタンスの凍結
- `gen` / `yield` による eager generator と `next()`
- 型注釈としての `Union[...]` / `Option[...]`
- VS Code 拡張によるシンタックスハイライトと簡易 inlay hint

- Python-style indented blocks, comments, and multi-line strings
- Variable declarations with `let` / `mut` / `const` and mutability checking
- Arithmetic, comparison, logical, bitwise operations, and compound assignments
- `if` / `elif` / `else`, `while`, `for ... in ...`, `block`
- `break` / `continue` / `pass` / `return` / `block_return` / `block_yield`
- List literals and built-in functions: `print`, `range`, `len`
- Function definitions, recursion, positional arguments, keyword arguments
- Function and method overloading
- Classes, instance fields, class variables, methods, `self`
- Trait definitions, trait fields, virtual methods, trait implementation checking
- Template functions, template classes, template generators, trait constraints
- `Self` type and `Self(...)` constructors
- `new_type NewName: OriginalType`
- `freeze` for immutability of `mut` variables and instances
- `gen` / `yield` for eager generators and `next()`
- Type annotations using `Union[...]` / `Option[...]`
- VS Code extension with syntax highlighting and basic inlay hints

---

## 型検査（Type Checking）

`src/type_check.rs` は、AST を実行前に走査して `StaticTypeError` を収集します。現在は主に以下を検査します。

- `let` / `const` への再代入
- 互換性のない大小比較
- 関数呼び出しの引数個数・型不一致
- パラメータや戻り値の型アノテーション欠如
- 不明なキーワード引数
- オーバーロード候補不一致
- `Self` 型引数の不一致
- `Union` / `Option` まわりの型不一致

型が静的に分からない箇所は、原則として機能を厳しく制限し、明示的なダウンキャストを要求します。

`src/type_check.rs` traverses the AST before execution and collects `StaticTypeError` instances. It currently checks mainly the following:

- Re-assignment to `let` / `const` variables
- Incompatible comparison operations (e.g., `<`, `>`)
- Function call argument count and type mismatches
- Missing type annotations for parameters or return values
- Unknown keyword arguments
- Mismatched overload candidates
- Mismatched `Self` type arguments
- Type mismatches related to `Union` / `Option`

For code where types cannot be statically determined, functionality is generally restricted and explicit downcasting is required.

---

## 例（Examples Directory）

代表的な動作確認ファイルです。

- `examples/showcase.tl`: 主要機能のまとめ
- `examples/type_errors.tl`: 静的型エラー例
- `examples/fn_kwargs_success.tl` / `examples/fn_kwargs_errors.tl`: キーワード引数
- `examples/overload_success.tl` / `examples/overload_errors.tl`: オーバーロード
- `examples/trait_sample.tl` / `examples/trait_template.tl`: trait
- `examples/template_sample.tl` / `examples/template_constraint_error.tl`: template
- `examples/self_type.tl` / `examples/self_type__errors.tl`: `Self`
- `examples/new_type.tl` / `examples/new_type__errors.tl`: `new_type`
- `examples/freeze.tl`: `freeze`
- `examples/generator.tl`: generator
- `examples/union_option.tl` / `examples/union_option__errors.tl`: `Union` / `Option`

エラー確認用のサンプルは、ファイル名に `__errors` または `_errors` を含めています。

Representative test files for verifying functionality.

- `examples/showcase.tl`: Summary of main features
- `examples/type_errors.tl`: Static type error examples
- `examples/fn_kwargs_success.tl` / `examples/fn_kwargs_errors.tl`: Keyword arguments
- `examples/overload_success.tl` / `examples/overload_errors.tl`: Overloading
- `examples/trait_sample.tl` / `examples/trait_template.tl`: Traits
- `examples/template_sample.tl` / `examples/template_constraint_error.tl`: Templates
- `examples/self_type.tl` / `examples/self_type__errors.tl`: `Self`
- `examples/new_type.tl` / `examples/new_type__errors.tl`: `new_type`
- `examples/freeze.tl`: `freeze`
- `examples/generator.tl`: Generators
- `examples/union_option.tl` / `examples/union_option__errors.tl`: `Union` / `Option`

Sample files for error verification are named with `__errors` or `_errors` suffix.

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

仕様を追加した場合は、実装のユニットテストに加えて、正常系の example を追加してください。エラー動作が仕様に含まれる場合は、期待したエラーで終了する `__errors` example も追加します。

```bash
cargo build
cargo run -- -src examples/showcase.tl
cargo test
cargo test <name>
cargo clippy
cargo fmt
```

When adding new specifications, include unit tests in the implementation and add corresponding example files. If error behavior is part of the specification, also add a `__errors` example that demonstrates the expected error termination.

---

## 現在の主な制限（Current Limitations）

- LLVM IR 生成は未実装です。
- Python 実装は未着手です。
- `import` / `from ... import` は未実装です。
- `try` / `except` / `finally` / `raise` は未実装です。
- `match` は未実装です。
- 辞書・セットリテラルは未実装です。
- template の静的型検査は限定的で、制約チェックの多くは実行時に行われます。
- trait は現在、主に parse/type-check/interpreter のための構造で、完全な runtime object ではありません。

- LLVM IR code generation is not yet implemented.
- Python implementation has not been started.
- `import` / `from ... import` are not implemented.
- `try` / `except` / `finally` / `raise` are not implemented.
- `match` statements are not implemented.
- Dictionary and set literals are not implemented.
- Static type checking for templates is limited, with most constraint checks performed at runtime.
- Traits are currently structures primarily for parse/type-check/interpreter use, not complete runtime objects.

---

## VS Code 拡張（VS Code Extension）

```bash
cd vscode-extension
npm install
npm run compile
```

開発モードでは VS Code で拡張プロジェクトを開き、F5 で Extension Development Host を起動します。

```bash
cd vscode-extension
npm install
npm run compile
```

For development mode, open the extension project in VS Code and launch the Extension Development Host by pressing F5.
