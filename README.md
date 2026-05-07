# test_lang

`test_lang` は、Python 風のインデント構文を土台に、明示的な変数宣言、静的型検査、trait、template、`Self` 型、`new_type` などを試している独自スクリプト言語です。

将来的なターゲットは LLVM IR ですが、現在の実装は Rust 製の tree-walk interpreter です。実行時は `.tl` ソースを字句解析、構文解析、静的型検査したあとにインタープリタで評価します。

## クイックスタート

```bash
cargo build
cargo run -- -src examples/showcase.tl
cargo run -- examples/hello.tl
cargo test
```

`-src <file.tl>` と位置引数のどちらでも実行できます。引数がない場合は標準入力からソースを読みます。

## サンプル

```tl
let name = "world"
mut x = 10
x += 5

fn greet(who: str) -> None:
    print("Hello,", who)

greet(name)
print("x =", x)
```

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

## プロジェクト構成

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

## 実行フロー

```text
source.tl
  -> Lexer        Vec<Spanned> を生成
  -> Parser       Vec<Stmt> の AST を生成
  -> TypeChecker  StaticTypeError をまとめて報告
  -> Interpreter  AST を実行
```

静的型検査でエラーが 1 件でも見つかった場合は、全件を表示して実行せずに終了します。

## 実装済みの主な機能

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

## 型検査

`src/type_check.rs` は、AST を実行前に走査して `StaticTypeError` を収集します。現在は主に以下を検査します。

- `let` / `const` への再代入
- 互換性のない大小比較
- 関数呼び出しの引数個数・型不一致
- パラメータや戻り値の型アノテーション欠如
- 不明なキーワード引数
- オーバーロード候補不一致
- `Self` 型引数の不一致
- `Union` / `Option` まわりの型不一致

型が静的に分からない箇所は、原則として実行時に委ねます。

## examples

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

## 開発コマンド

```bash
cargo build
cargo run -- -src examples/showcase.tl
cargo test
cargo test <name>
cargo clippy
cargo fmt
```

仕様を追加した場合は、実装のユニットテストに加えて、正常系の example を追加してください。エラー動作が仕様に含まれる場合は、期待したエラーで終了する `__errors` example も追加します。

## 現在の主な制限

- LLVM IR 生成は未実装です。
- Python 実装は未着手です。
- `import` / `from ... import` は未実装です。
- `try` / `except` / `finally` / `raise` は未実装です。
- `match` は未実装です。
- 辞書・セットリテラルは未実装です。
- template の静的型検査は限定的で、制約チェックの多くは実行時に行われます。
- trait は現在、主に parse/type-check/interpreter のための構造で、完全な runtime object ではありません。

## VS Code 拡張

```bash
cd vscode-extension
npm install
npm run compile
```

開発モードでは VS Code で拡張プロジェクトを開き、F5 で Extension Development Host を起動します。
