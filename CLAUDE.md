# CLAUDE.md

Claude Code がこのリポジトリで作業する際の指針。

## コマンド

```bash
cargo build                        # コンパイル
cargo run -- -src <file.tl>        # .tl ファイルを実行
cargo run -- <file.tl>             # 位置引数でも可
cargo test                         # 全テスト実行
cargo test <name>                  # テスト名（部分一致）で絞り込み
cargo clippy                       # lint
cargo fmt                          # フォーマット
```

## プロジェクト概要

**test_lang** は LLVM IR をターゲットとする独自スクリプト言語。
Python の構文を基盤としつつ、静的型チェックや独自拡張を加えた言語を目指している。

- ファイル拡張子: `.tl`
- 実装言語: **Rust**（メイン）、将来的に Python も追加予定
- インデントによるブロック構造（Python スタイル）

## ディレクトリ構成

```
test_lang/
├── src/
│   ├── main.rs          # エントリーポイント・引数パース
│   ├── token.rs         # Token enum・Span・Spanned 定義
│   ├── lexer.rs         # 字句解析器（Vec<Spanned> を返す）
│   ├── ast.rs           # AST ノード定義（Span 埋め込み済み）
│   ├── parser.rs        # 再帰下降パーサー（Vec<Spanned> 入力）
│   ├── type_check.rs    # 静的型検査（パース後・実行前）
│   └── interpreter.rs   # ツリーウォークインタープリタ
├── spec/
│   ├── general.md       # 言語全般仕様
│   ├── keywords.md      # キーワード一覧
│   └── operator.md      # 演算子一覧・優先順位
├── examples/
│   ├── showcase.tl           # 実装済み全機能の動作確認
│   ├── type_errors.tl        # StaticTypeError の発生例
│   ├── self_type.tl          # Self 型の動作確認
│   ├── self_type__errors.tl  # Self 型のパースエラー例
│   ├── new_type.tl           # new_type の動作確認
│   └── new_type__errors.tl   # new_type の Self 型不一致エラー例
└── vscode-extension/    # VS Code 拡張（型推論インレイヒント）
    └── src/
        ├── extension.ts
        └── type_infer.ts
```
## テストについて
仕様を追加した場合、以下のルールに則ってテストを行う
- 仕様が完成したとき:
    - インタープリタのテスト
    - 仕様を使って正常終了するサンプルコードをexampleフォルダに生成し、テストする
    - 仕様を使って、狙った通りにエラーを送出するサンプルコードをexampleフォルダに生成してテストし、期待通りのエラーが送出されて終了することを確認する。ファイル名には末尾に__errorsと入れること。ただし、エラーに関する仕様について言及されなかった場合、このステップは省く。

- 仕様が完成するまえの段階的なテスト:
    - インタープリタのテストのみを行う

## 実行フロー

```
ソースファイル
  → Lexer         Vec<Spanned> を生成（各トークンにファイル名・行・列を付与）
  → Parser        AST（Vec<Stmt>）を生成。Expr::BinOp / Stmt::Assign などに Span を埋め込む
  → TypeChecker   StaticTypeError を収集。1 件でもあれば全件表示して exit(1)
  → Interpreter   ツリーウォーク実行
```

## 実装済みの機能

### 字句解析（`src/lexer.rs`）
- 全キーワード・演算子・リテラル対応
- インデント追跡（INDENT / DEDENT トークン生成）
- 括弧内での改行無視
- 複合キーワード: `not in`, `is not`, `yield from`
- 数値リテラル: 10進・16進・8進・2進・アンダースコア区切り
- 文字列: シングル・ダブル・トリプルクォート・エスケープ
- 各トークンに `Span`（ファイル名・行番号・列番号）を付与
- `Token::SelfType`（`Self`）、`Token::NewType`（`new_type`）

### 構文解析（`src/parser.rs`）
- 変数宣言: `let`（不変）、`mut`（可変）、`const`（不変）
- 代入: `x = expr`、複合代入: `x += expr` など全種
- 式: 演算子優先順位を仕様通りに実装（右結合 `**` 含む）
- 関数呼び出し: `f(args)`、属性アクセス: `obj.attr`
- リストリテラル: `[a, b, c]`
- 制御構文: `if` / `elif` / `else`、`while`、`for ... in ...`、`block`
- ジャンプ文: `break`、`continue`、`pass`、`return`、`block_return`
- 関数定義: `fn name(params) -> RetType:`
- クラス定義: `class Name(Base):` （基底クラス複数可）
- **クラスメンバ変数**: クラス本体では必ず型アノテーション付きで宣言する（`[mut|let|const] name: Type [= default]`）。型アノテーションのない宣言はパースエラー
- **デフォルトコンストラクタ自動生成**: 初期値なしの `mut`/`let` フィールドが 1 つ以上あるとき、パーサーが `__init__(mut self, field1: Type1, ...)` を AST に挿入する。既存の `__init__` と引数の型・個数が完全一致する場合は生成しない（override）。型・個数が異なる場合は共存する（overload）
- **関数オーバーロード**: 同名関数を引数の型・個数で区別して複数定義可能
- **テンプレート**: 関数・クラスの定義時に型変数を宣言し、呼び出し時に具体的な型を指定する。構文: `fn f[T: Trait]()` / `class C[T: Trait]:`。テンプレート呼び出し構文: `f[Type](args)` / `C[Type](args)`
- **`Self` 型**: クラス・trait 本体でのみ使用可能な型キーワード。型アノテーション・戻り値型・`Self(...)` コンストラクタとして利用可能。クラス外での使用はパースエラー
- **`new_type`**: `new_type NewName: OriginalType` 構文。元のクラス／プリミティブ型と構造的に同一の新しい型を定義する。バインドは常に const（再代入はパースエラー）

### 静的型検査（`src/type_check.rs`）
パース後・実行前に AST を走査し、`StaticTypeError` を収集してまとめて報告する。
エラーメッセージには `ファイル名:行:列: StaticTypeError: ...` の形式で位置情報を付与。

現在検査するエラー種別（`TypeErrorKind`）:

| 種別 | 説明 |
|---|---|
| `AssignToImmutable` | `let` / `const` 変数への代入・複合代入 |
| `IncompatibleComparison` | `<` `>` `<=` `>=` で互換性のない型の比較（例: `str < int`） |
| `CallArgCountMismatch` | 関数呼び出し時の引数個数不一致 |
| `CallArgTypeMismatch` | 引数型の不一致 |
| `MissingParamTypeAnn` | パラメータの型アノテーション欠如（`self` は除外） |
| `MissingReturnTypeAnn` | 戻り値型アノテーション欠如 |
| `UnknownKeywordArg` | 存在しないキーワード引数名 |
| `NoMatchingOverload` | オーバーロード候補のどれにも引数の個数が合わない |
| `SelfTypeMismatch` | `Self` 型パラメータに異なるクラス／new_type のインスタンスを渡した |

- `==` / `!=` は異なる型間でも許容（実行時に `False` を返す想定）
- 型が静的に不明（fn パラメータ等）な場合は実行時に委ねる
- キーワード引数（`f(a=1, b=2)`）の名前・型・個数を検査（`collect_fn_sigs` 前処理で前方参照も対応）
- オーバーロード: 同名関数が複数ある場合、引数個数が一致する候補のみ型検査。複数候補が個数一致する場合は型検査をスキップ
- `Self` 型パラメータ検査: メソッド呼び出し時にレシーバのクラス名（`NamedInstance`）と引数の型を照合し、不一致なら `SelfTypeMismatch` を発行
- コンストラクタ呼び出し（`ClassName(args)`）は `InferredType::NamedInstance(ClassName)` を返す。これにより `new_type` で作成した型を静的に区別できる

**拡張方法**: `TypeErrorKind` に variant を追加 → `check_stmt` / `check_binop` に arm を追加。

### インタープリタ（`src/interpreter.rs`）
- 変数の不変/可変チェック（実行時）
- 算術・比較・論理・ビット演算（全演算子）
- 文字列連結 (`+`)
- `and` / `or` の短絡評価
- ゼロ除算エラー
- `if` / `elif` / `else`（スコープ分離済み）
- `while`（`break` / `continue` 対応）
- `for ... in ...`（`break` / `continue` 対応）
- `block:` スコープ（`block_return` / `block_yield` 対応）
- リストリテラル評価
- 組み込み関数: `print()`、`range(stop)` / `range(start, stop)` / `range(start, stop, step)`、`len()`
- **関数の実行**: `fn` 定義・呼び出し（位置引数・キーワード引数）・`return`・再帰
- **クラスの実行**: インスタンス化・フィールド・メソッド呼び出し・`self`・継承（`lookup_method_in_class`）
- `self.x = v` / `self.x += v`（`AttrAssign` / `AttrCompoundAssign`）
- **クラスメンバの種別**:
  - `mut name: Type [= default]` — ミュータブルなインスタンス変数。初期値なしの場合はコンストラクタで必ず設定する
  - `let name: Type [= default]` — イミュータブルなインスタンス変数。コンストラクタ（`__init__`）での初回代入のみ許可
  - `const name: Type = default` — クラス変数（全インスタンスで共有）。必ず初期値が必要。`instance.name` および `ClassName.name` でアクセス可能。代入は不可
- **フィールド可変性の実行時チェック**: `let` フィールドへの再代入は `TypeError`、`const` クラス変数への代入も `TypeError`
- **関数オーバーロード**: 同名関数が複数あるとき `Value::OverloadedFn(Vec<Rc<FnValue>>)` として蓄積。呼び出し時に引数の個数→型で候補を絞り込む
- **テンプレート関数・クラス**: `Value::TemplateFn` / `Value::TemplateClass` として格納。呼び出し時に型引数の trait 制約を検証し、AST 内の型変数を具体型に置換して実行
- **`Self` 型**: メソッド実行時に `exec_fn_evaled()` がレシーバインスタンスのクラスを `"Self"` としてスコープに束縛。`Self(...)` は現在のクラスのコンストラクタとして機能し、`new_type` のインスタンスで呼び出すと正しく new_type 側のインスタンスを生成する
- **`new_type`**: `Stmt::NewTypeDef { name, original }` を実行。元の値が `Value::Class` の場合は `ClassValue` を `name` でコピーして `const` バインド、プリミティブ型の場合は `value` フィールドを持つラッパークラスを自動生成して束縛
- 値型: `Int`, `Float`, `Str`, `Bool`, `None`, `List`, `Function(Rc<FnValue>)`, `OverloadedFn(Vec<Rc<FnValue>>)`, `Class(Rc<ClassValue>)`, `Instance(Rc<RefCell<InstanceData>>)`, `TemplateFn(Rc<TemplateFnValue>)`, `TemplateClass(Rc<TemplateClassValue>)`

### VS Code 拡張（`vscode-extension/`）
- `.tl` ファイルのシンタックスハイライト
- 型推論インレイヒント（変数名の右に `: int` などを表示）
- 対応型: `int`, `float`, `str`, `bool`, `None`, `unknown`

## 未実装の主な機能

- **型アノテーションの完全保存**: パース時に型名は取得済みだが、インタープリタ実行時には使われていない（静的型検査で使用）
- **戻り値型の実行時チェック**: 宣言型と実際の `return` 値の型検証
- **辞書・セット型**: `{k: v}`、`{a, b}`
- **例外処理**: `try` / `except` / `finally` / `raise`
- **インポート**: `import` / `from ... import`
- **`match` 文**
- **LLVM IR コード生成**（現在はツリーウォークインタープリタで動作）
- **Python 実装**

## 言語仕様の要点（Python との違い）

- 変数宣言に `let` / `mut` / `const` が必要
- 関数定義は `def` ではなく `fn`
- パース後・実行前に静的型検査を行い `StaticTypeError` を出す
- テンプレートをサポート: `fn f[T: Trait]()` / `class C[T: Trait]:` で型変数を宣言し、`f[Type](args)` で呼び出す
- 変更しうる引数には `mut` を明示する必要がある
- 空のコレクションは型を明示しなければならない（`list[int]` など）
- `dataclass` / `enum` などを標準でサポート（未実装）
- 値オブジェクト（型キャスト）による誤演算防止（未実装）

### クラスメンバ変数の宣言規則

クラス本体で宣言できるメンバ変数は以下の 3 種類のみ。いずれも **型アノテーションが必須**。

```
mut  name: Type [= default]   # ミュータブルなインスタンス変数
let  name: Type [= default]   # イミュータブルなインスタンス変数
const name: Type = default    # クラス変数（必ず初期値が必要）
```

- `const` フィールドのみ初期値が必須。`mut`/`let` は初期値を省略できる
- `const` はクラスと紐づいた変数（クラス変数）。インスタンス経由・クラス名経由どちらでもアクセス可能。代入は不可
- `let` フィールドは `__init__` 内での初回代入のみ許可。それ以降の代入は `TypeError`

### デフォルトコンストラクタ（auto-init）

初期値なしの `mut`/`let` フィールドが存在するとき、パーサーが自動的に `__init__` を生成する。

```
fn __init__(mut self, field1: Type1, field2: Type2, ...) -> None:
    self.field1 = field1
    self.field2 = field2
    ...
```

生成の可否は既存の `__init__` 定義と比較して決まる:

| 既存の `__init__` | 自動生成 | 備考 |
|---|---|---|
| なし | される | |
| 引数の型・個数が異なる | される | 既存定義とオーバーロードとして共存 |
| 引数の型・個数が完全一致 | されない | 既存定義が優先（override） |

- 初期値ありのフィールドは auto-init のパラメータに含まれない（`field_defaults` 経由で初期化）
- `const` フィールドは auto-init に含まれない

### 関数オーバーロード

同名の関数・メソッドを異なる引数の型・個数で複数定義できる。

```tl
fn add(a: int, b: int) -> int:   return a + b
fn add(a: str, b: str) -> str:   return a + b
```

- 呼び出し時は引数の個数 → 型の順で候補を絞り込み、最初にマッチしたものを実行
- どの候補にも一致しない場合は実行時エラー
- 静的型検査: 個数が合う候補がなければ `NoMatchingOverload` エラー

### テンプレート

関数・クラス・**trait** の定義時に型変数と trait 制約を宣言できる。

```tl
fn func[T1: Trait1, T2: Trait2](a: T1, b: T2) -> T1:
    ...

class MyClass[T: Trait1 and Trait2]:
    mut item: T
    fn get(self) -> T:
        return self.item

trait Container[T: Printable]:
    mut item: T
```

**呼び出し構文**: `func[ConcreteType](args)` / `MyClass[ConcreteType](args)`

**trait テンプレートの継承**: クラスは `class Foo(MyTrait[ConcreteType]):` の形式で template trait を具体化して継承できる。型変数はフィールドの型アノテーションに置換されて auto-init が生成される。

```tl
trait Printable:
    fn to_str(self) -> str:
        ...

class MyInt(Printable):
    mut value: int
    fn to_str(self) -> str: return "MyInt"

fn describe[T: Printable](item: T) -> str:
    return item.to_str()

let a = MyInt(42)
let s = describe[MyInt](a)   # → "MyInt"
```

**制約の仕様**:
- 型変数ごとに `: Trait` で trait 制約を指定する
- `and` で複数 trait を結合できる: `[T: TraitA and TraitB]`
- 呼び出し時に型変数に渡された具体型が制約を満たさない場合、`TemplateError` を送出して停止
- 具体型が trait を実装しているかは、クラス定義の `bases`（継承 trait リスト）で判定する
- 組み込み型（`int`, `str` 等）は trait を実装していないため、trait 制約を持つテンプレートには渡せない

**実装の仕組み**:
1. パース時: テンプレートパラメータ付きの `FnDef` / `ClassDef` を AST に生成し、インタープリタが `Value::TemplateFn` / `Value::TemplateClass` として格納
2. 呼び出し時: 型引数ごとに `type_satisfies_trait()` で制約を検証し、違反があれば即エラー
3. 制約通過後: `subst_*` 関数群で AST 内の型変数名を具体型名に置換した新しい AST を生成して実行

**現在の制限**:
- テンプレート関数のオーバーロードは未対応（同名テンプレート関数を複数定義すると後の定義で上書き）
- `list[T]` のような複合型に含まれる型変数は置換されない（`parse_type_expr` が基底型名のみ返すため）
- テンプレート呼び出しの静的型検査は未対応（制約チェックは実行時のみ）

### `Self` 型

クラス・trait 本体でのみ使用できる特殊な型キーワード。

```tl
class Vec2:
    mut x: int
    mut y: int
    fn clone(self) -> Self:          # 戻り値型として使用
        return Self(self.x, self.y)  # Self(...) でコンストラクタ呼び出し
    fn add(self, other: Self) -> Self:  # パラメータ型として使用
        return Self(self.x + other.x, self.y + other.y)
```

**仕様**:
- クラス・trait の外で `Self` を型アノテーションや式に使うとパースエラー
- 実行時: `exec_fn_evaled()` がレシーバインスタンスのクラスを `"Self"` としてスコープに束縛する
- `Self(...)` コンストラクタはレシーバの実際のクラスを使うため、`new_type` で派生した型でも正しく動作する
- 静的型検査: `other: Self` 型パラメータに異なるクラスのインスタンスを渡すと `SelfTypeMismatch` エラー

### `new_type`

既存のクラス／プリミティブ型と構造的に同一だが **名前が異なる** 新しい型を定義する。Python の `NewType` に近い。

```tl
class Meters:
    mut value: int
    fn add(self, other: Self) -> Self:
        return Self(self.value + other.value)

new_type Kilometers: Meters   # Meters と同じ構造だが別の型
new_type Celsius: int         # プリミティブのラッパー（.value フィールドを持つクラスが生成される）
```

**仕様**:
- 構文: `new_type NewName: OriginalType`
- バインドは常に `const`。再代入は**パースエラー**（型エラーではない）
- 元の型が `class` の場合: `ClassValue` を `name` だけ変えてコピー。メソッドは共有される
- 元の型がプリミティブ（`int`, `str` 等）の場合: `value` フィールドを持つラッパークラスを自動生成
- `Self` との相互作用: `new_type` 由来のインスタンスでメソッドを呼ぶと、`Self` はその new_type のクラスに解決される。同じ元クラスから派生した 2 つの new_type は互いに `Self` 型引数として渡せない
- 静的型検査: `a.method(b)` で `method` の引数が `Self` 型であり、`a` と `b` が異なるクラス／new_type のとき `SelfTypeMismatch` を発行

**エラー例**:
```tl
class Meters:
    mut value: int
    fn add(self, other: Self) -> Self: ...

new_type Kilometers: Meters

let m = Meters(100)
let km = Kilometers(5)
m.add(km)   # StaticTypeError: parameter 'other' of 'add' expects 'Self' = 'Meters' but got 'Kilometers'
```

## 次に実装すべき機能（優先順）

1. **辞書・セット型**（`{k: v}`、`{a, b}`）
2. **例外処理**（`try` / `except` / `finally` / `raise`）
3. **インポートシステム**（`import` / `from ... import`）
4. **`match` 文**
5. **クロージャ**（外側スコープのキャプチャ）
6. **LLVM IR コード生成**
