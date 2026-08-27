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
この言語は速度優先のRust製実装（`src/`）と頒布性優先のPython製実装（`impl_python/`）の両方を持ちます。
Pythonが使えるなら後者の実装ごと配れば即座に使えます。
両者の出力は `compare_python_impl.ps1` で全例題を突き合わせて一致を確認しています。

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
arrow.exe examples/basics/variable.ar //ファイル実行
arrow.exe --repl //対話画面起動
arrow.exe --compile examples/interop/test_modules/physics.ar //モジュールとしてコンパイル

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
├── src/                    # Rust 実装 (Rust implementation)
│   ├── main.rs             # CLI と実行フロー (CLI and execution flow)
│   ├── token.rs / ast.rs   # Token / Span / Spanned・AST 定義 (AST definitions)
│   ├── lexer/              # 字句解析器 (Lexical analyzer)
│   ├── parser/             # 再帰下降パーサー・import 解決 (Recursive descent parser, import resolution)
│   ├── type_check/         # 静的型検査 ＋ AST 型注釈 (Static type checker + AST annotations)
│   ├── interpreter/        # 実行時の値・クラス・FFI・定義文の実行 (Runtime values, classes, FFI)
│   │   └── resolver.rs     # 解決層: 名前 → slot / グローバル索引 (Resolution layer)
│   ├── vm/                 # バイトコードコンパイラ ＋ VM (Bytecode compiler + VM)
│   ├── partial_compiler/   # LLVM IR 生成・部分ネイティブコンパイル (LLVM IR codegen)
│   ├── decl_names.rs       # 「この文はどの名前を束縛するか」の唯一の定義
│   ├── expr_walk.rs        # 「この式の直下に何があるか」の唯一の定義
│   └── stmt_walk.rs        # 「この文の下に何があるか」の唯一の定義
├── impl_python/            # Python 実装 (Python implementation)
├── docs/                   # 言語仕様 (Language specification)
├── examples/               # 正常系・エラー系サンプル (Success and error case samples)
├── std_tools/              # 標準ツール群 (Standard tools)
├── bridge/                 # 他言語ブリッジ (Bridges to other languages)
└── vscode-extension/       # .ar 用 VS Code 拡張 (VS Code extension for .ar)
```

⚠ `decl_names.rs` / `expr_walk.rs` / `stmt_walk.rs` は「AST を歩く処理が増えても取りこぼしが起きない」
ようにするための仕掛けです。AST に構文を 1 つ足すと、これらを消費する全箇所がコンパイルエラーになり、
対応漏れが**コンパイル時に**分かります。 / These three modules make AST traversal drift impossible:
adding one AST variant breaks compilation at every consumer, so nothing can be silently missed.

---

## 実行フロー（Execution Flow）

```text
source.ar
  -> Lexer         Vec<Spanned> を生成 (Generate Vec<Spanned>)
  -> Parser        Vec<Stmt> の AST を生成 (Generate AST (Vec<Stmt>))
  -> TypeChecker   StaticTypeError をまとめて報告 ＋ AST に型注釈を焼く
                   (Collect StaticTypeErrors + bake type annotations into the AST)
  -> Resolver      名前を slot / グローバル索引へ解決 (Resolve names to slots / global indices)
  -> VM compiler   AST → バイトコード (Chunk) へ翻訳 (Translate AST into bytecode)
  -> VM            バイトコードを実行 (Execute bytecode)
```

静的型検査でエラーが 1 件でも見つかった場合は、全件を表示して実行せずに終了します。/ If any errors are found during static type checking, all errors are displayed and the program exits without running.

### 実行方式（Execution model）

**解釈実行はバイトコード VM 一本です。** 以前は AST をそのまま辿る tree-walk インタープリタでしたが、
`AST → 解決層 → バイトコード VM` に置き換えました。/ **Interpretation runs entirely on a bytecode VM.**
The original tree-walk interpreter was replaced by `AST -> resolution layer -> bytecode VM`.

- **解決層（Resolver）** — 実行のたびに名前をスコープチェーンで辿るのをやめ、
  ローカル変数は**フレーム内の固定 slot 番号**、最上位の変数は**グローバル索引**へ、実行前に解決します。
  属性アクセスもクラス先頭からのオフセットに解決し、外れた場合だけ辞書を引き直します（インラインキャッシュ）。
- **バイトコード VM** — 解決済み AST を `Chunk`（命令列 ＋ 定数プール ＋ 行テーブル）へ翻訳して実行します。
  頻出パターンは 1 命令へ融合してあります（`local[a] + local[b]`、`local.attr` など）。
- **フォールバックはありません。** VM に載せられない構文に出会った場合は
  `VmForceError` で停止します（黙って遅い経路へ落ちることはありません）。
  tree-walk が実行するのは**定義文（`fn` / `class` / `import` など）だけ**です。
- **デバッガと REPL** はバイトコード上でそのまま動きます（`Chunk` が行テーブルと
  slot → 変数名のデバッグ名テーブルを持つため）。

⚠ 実測（tree-walk 版との A/B）: **解釈が支配的なベンチでは約 3.97 倍**。
ただし例題 1 本の中央値では実行そのものが全体の **14%**（0.46ms / 3.40ms）しかなく、
残りはプロセス起動・パース・型検査です。⇒ **短命なスクリプトでは、実行を無限に速くしても
プロセス全体では 1.59 倍が上限**。/ Measured against the tree-walk build: **~3.97x on
execution-dominated benchmarks**, but execution is only **14%** of a median example's wall time,
so short-lived scripts cap at ~1.59x end-to-end.

### 部分ネイティブコンパイル（Partial native compilation）

`--compile` で `.ar` モジュールを LLVM IR 経由でネイティブコードへ落とし、`.arc`（バイナリ）と
`.ars`（型スタブ）を生成します。適格な関数だけがネイティブ化され、import 時にネイティブ側へ
直接ディスパッチされます。/ `--compile` lowers eligible functions to native code via LLVM IR.

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
- 辞書・セット・タプルのリテラル / Dictionary, set and tuple literals
- `match` 文（`case` / `is` パターン）と `enum` / `match` statements (`case` / `is` patterns) and `enum`
- `try` / `except ... as` / `finally` / `raise` による例外処理 / Exception handling
- **式としての制御構文** — `if` / `for` / `while` / `match` / `block` に `->Type` を付けて値を返す
  （`block_return` で値を返し、`loop_yield` で値を積む） / Control constructs usable as expressions
- 型ガード `is` / 動的アサーション `mustbe` / キャスト `=>` と `__cast__` / Type guards, assertions and casts
- **多言語 import** — `import[lang]` で Python・C の DLL/LIB・Rust crate・C# DLL・Node.js を呼び出し /
  Multi-language interop via `import[lang]`
- `async` タスク（`mng <- async->T:`）と `AsyncManager` / Async tasks and `AsyncManager`
- イベント購読（`EventSubscribe` / `EventUnsubscribe`）／`protocol` 定義 / Event subscription and `protocol`
- アクセス制御のセクションマーカー（`public:` / `private:` / `protected:`） / Access-control section markers
- **対話 REPL**（`--repl`）と**ステップ実行デバッガ** / Interactive REPL and a stepping debugger
- **部分ネイティブコンパイル**（`--compile` → LLVM IR → `.arc` / `.ars`） / Partial native compilation
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

- ネイティブコンパイル（`--compile`）の対象は**適格な関数だけ**です。クロージャ・ジェネレータ・
  `block_return` / `loop_yield` を含む関数は対象外で、解釈実行へ回ります。 / Native compilation
  covers eligible functions only; closures, generators and `block_return`/`loop_yield` fall back to interpretation.
- `--compile` でネイティブライブラリまで生成するには `clang` が必要です（見つからない場合は
  `.arc` だけ生成してスキップします）。 / `clang` is required to emit the native library.
- 入れ子の `gen`（ジェネレータ）は VM 非対応で、含む関数は `VmForceError` になります。 /
  Nested generators are not supported by the VM.
- `async` は share-nothing（送出時にディープコピー）で、共有可変状態は持てません。 /
  `async` is share-nothing; there is no shared mutable state.
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
