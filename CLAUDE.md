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
│   ├── showcase.tl      # 実装済み全機能の動作確認
│   └── type_errors.tl   # StaticTypeError の発生例
└── vscode-extension/    # VS Code 拡張（型推論インレイヒント）
    └── src/
        ├── extension.ts
        └── type_infer.ts
```

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

- `==` / `!=` は異なる型間でも許容（実行時に `False` を返す想定）
- 型が静的に不明（fn パラメータ等）な場合は実行時に委ねる
- キーワード引数（`f(a=1, b=2)`）の名前・型・個数を検査（`collect_fn_sigs` 前処理で前方参照も対応）
- オーバーロード: 同名関数が複数ある場合、引数個数が一致する候補のみ型検査。複数候補が個数一致する場合は型検査をスキップ

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
- 値型: `Int`, `Float`, `Str`, `Bool`, `None`, `List`, `Function(Rc<FnValue>)`, `OverloadedFn(Vec<Rc<FnValue>>)`, `Class(Rc<ClassValue>)`, `Instance(Rc<RefCell<InstanceData>>)`

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
- `template` キーワードによるテンプレートをサポート（構文未実装）
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

## 次に実装すべき機能（優先順）

1. **辞書・セット型**（`{k: v}`、`{a, b}`）
2. **例外処理**（`try` / `except` / `finally` / `raise`）
3. **インポートシステム**（`import` / `from ... import`）
4. **`match` 文**
5. **クロージャ**（外側スコープのキャプチャ）
6. **LLVM IR コード生成**
