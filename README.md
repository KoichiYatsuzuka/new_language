# Arrow
僕の考えた最強のプログラミング言語。
The best language what I imagine.

## 名前の由来（Origin of the name）
ギリシャ神話で蛇の怪物を倒したのが矢による射撃だったから。
Because the monster of python was beaten by a shoot of an arrow in the Greek mythos.

## 概要（Overview）
Pythonのとっつきやすさ・汎用性と型安全の両立を目指したスクリプト言語です。
Python, C++, Rustの影響を受けています。
あと意図せずJuliaに似た特徴をもちます。
ASTの展開にすべての情熱を注いています。

---
## 特長（Features）
### 一応スクリプト言語（This is a script, maybe）
この言語はPython的な見た目をしたスクリプト言語ですが、静的に強く型付けをします。Pythonでいうところの構文解析時のエラー（syntax error）を送出するタイミングで型検査も行い、その時点で型が合わないと判断される場合には実行を始めません。

また、確定している属性のみをアクセス可能にしているため、Any型、Union型、Optional型はtype_guard節以外では属性参照をまともにできないようにしています。

ソースコードの終わりの方になってTypeErrorが上がってきて、それまでの実行時間が無駄になった、ということがないように、そもそも実行する前に修正を強います。

### 部分コンパイルによる高速化（Accelaration by partial compile）
構文解析時の情報を派生させ、高速化が容易な部分に関してをネイティブな機械語に落とし込む機能を備えています。仕様を決めたモジュールはコンパイルしてから読み込むことで、その部分を高速化できます。最適化されていないC言語くらいの速度です（関数呼び出しオーバーヘッドが10 nsくらい）。同じ言語内でコンパイル済みライブラリを扱えるスクリプト言語であることが特徴です。計算やイテレーションが主な内容なら最高で10万倍以上の高速化が見込めます（./examples/interop/importation.arを参照）。
Pythonの高速化にはノウハウが必要ですが、この言語は標準機能ですぐ試せます。

C ABIに対応する内容でASTを展開し、メンバアクセスはクラス先頭からのポインタオフセット参照、関数は関数ポインタへのアクセスへと置き換えることで、それをネイティブコードに落とし込むときにC言語並みの速度になるようにしました。また、テンプレートクラスやテンプレート関数はAST展開の時に候補となる型で新しく定義を複製することで、AST展開時に挙動を確定させ、ネイティブコードへのコンパイルを実現しています。

### タダ乗り（Free ride）
Python製のライブラリ、Cのdll、Rustのcrate、C#のdll、JavaScriptを使えます。
Pythonインタープリタを呼び出してライブラリを処理させることで、大抵のライブラリが使えます。速度最適化などは今後の検討課題。そのため、標準ライブラリを持ちませんし、作るつもりもあんまりないです。

C ABIに対応するASTを展開するため、C ABIを介して大抵の言語が呼べるようになる……と思う。

Pythonのライブラリの多さがゆえに後発言語が超えられなかった汎用性を同レベルまでに引き上げています。Pythonでなければ〇〇ができない、を封じ、むしろこれまでの様々な言語のライブラリを使用可能にします。

### 二種類の実装（2 implementations）
この言語は速度優先のRust製インタープリタと頒布性優先のPython製インタープリタの両方を実装予定です。
Pythonが使えるなら後者の実装ごと配れば即座に使えるようにします。

### 変数の自動管理（Automatic management of mutabilitis of variables）
変数と関数の引数は定数属性、可変属性、非可変属性に分かれています。値が変わらないことが保証される場合には参照を渡すことで、コピーによるオーバーヘッドを防ぎ、速度を上げます。逆に値が変更されうる時には値をコピーして渡すようにします。これにより、借用権の管理を行わずに、値が変更されうることを防ぎます。可変引数も値の編集が終われば固定化（feeze）することもできます。その他、インスタンスのメンバ変数を変更しうるメンバ関数と変更しないメンバ関数が明示されるなど、値が変更されうるタイミングを追跡しやすくしています。

定数は本当に定数として振る舞い、構文解析時に値が定まっていなければエラーとしています。

ある制御構文内で定義された変数はそのブロックを抜けると自動的に破棄されます。また、特に何もしないブロックを作ることもでき、意図的に寿命の短い変数を宣言可能です（Rustに似ている）。

これらは変数に関する情報を整理し、コードの読み手が抱えるべき情報を減らします。

### typing
Pythonでは標準ライブラリであるAny, Union, Optional, NewTypeをbuilt-inにしています（ただし、使い勝手はPythonのそれとは異なり、強い制約を与えています）。これにより、Pythonのように場合によって型が変わるような場合も対応しつつ、型安全なコードを強制します。

また、クラスの継承は許可しておらず、trait（C++やPythonでいう抽象クラス、C#のインターフェース、Rustのtraitをよりクラスっぽくしたもの）の継承のみを許可しています。つまり、すべてのクラスはインスタンス化されることが前提で、traitはインスタンス化を許可されていません。

templateを実装しています（C++のようなintのtemplateは未実装）。

overloadを実装しています。型注釈用のものではなく、インスタンスを複数個定義可能です。

型推論により、変数の型は明示する必要がありません。また、型推論によって得られた情報により静的な型検査を行います。これにより、型注釈を書く煩わしさを防ぎつつ、静的な型付けを行います。

これらはポリモーフィズムと耐久性の両立のために実装を選定しました。

---

## クイックスタート（Quick Start）

```bash
cargo build
cargo run -- -src examples/archived/showcase.ar
cargo run -- examples/archived/hello.ar
cargo test
```

`-src <file.ar>` と位置引数のどちらでも実行できます。引数がない場合は標準入力からソースを読みます。/ You can run using either `-src <file.ar>` or positional arguments. If no arguments are provided, the source is read from standard input.

### ビルド後の実行ファイルの実行方法
ビルドされた実行ファイルがあるディレクトリにパスを通したあとで
```bash
havakirie.exe examples/basics/variable.ar //ファイル実行
havakirie.exe --repl //対話画面起動
havakirie.exe --compile examples/interop/test_modules/physics.ar //モジュールとしてコンパイル

```
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
Arrow/
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
└── vscode-extension/    # .tl 用 VS Code 拡張 (VS Code extension for .ar)
```

---

## 実行フロー（Execution Flow）

```text
source.ar
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

- `examples/archived/showcase.ar`: 主要機能のまとめ / Summary of main features
- `examples/archived/type_errors.ar`: 静的型エラー例 / Static type error examples
- `examples/archived/fn_kwargs_success.ar` / `examples/archived/fn_kwargs_errors.ar`: キーワード引数 / Keyword arguments
- `examples/archived/overload_success.ar` / `examples/archived/overload_errors.ar`: オーバーロード / Overloading
- `examples/archived/trait_sample.ar` / `examples/archived/trait_template.ar`: trait / Traits
- `examples/archived/template_sample.ar` / `examples/archived/template_constraint_error.ar`: template / Templates
- `examples/archived/self_type.ar` / `examples/archived/self_type__errors.ar`: `Self`
- `examples/archived/new_type.ar` / `examples/archived/new_type__errors.ar`: `new_type`
- `examples/archived/freeze.ar`: `freeze`
- `examples/archived/generator.ar`: generator / Generators
- `examples/archived/union_option.ar` / `examples/archived/union_option__errors.ar`: `Union` / `Option`

エラー確認用のサンプルは、ファイル名に `__errors` または `_errors` を含めています。/ Sample files for error verification are named with `__errors` or `_errors` suffix.

---

## 開発コマンド（Development Commands）

```bash
cargo build
cargo run -- -src examples/archived/showcase.ar
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
