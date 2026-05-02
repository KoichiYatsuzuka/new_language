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
- 関数定義: `fn name(params) -> RetType:`（型アノテーションはスキップ）
- クラス定義: `class Name(Base):` （基底クラス複数可）

### 静的型検査（`src/type_check.rs`）
パース後・実行前に AST を走査し、`StaticTypeError` を収集してまとめて報告する。
エラーメッセージには `ファイル名:行:列: StaticTypeError: ...` の形式で位置情報を付与。

現在検査するエラー種別（`TypeErrorKind`）:

| 種別 | 説明 |
|---|---|
| `AssignToImmutable` | `let` / `const` 変数への代入・複合代入 |
| `IncompatibleComparison` | `<` `>` `<=` `>=` で互換性のない型の比較（例: `str < int`） |

- `==` / `!=` は異なる型間でも許容（実行時に `False` を返す想定）
- 型が静的に不明（fn パラメータ等）な場合は実行時に委ねる

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
- `fn` / `class` 定義はパース・AST 構築まで完了（**実行は未実装**）

### VS Code 拡張（`vscode-extension/`）
- `.tl` ファイルのシンタックスハイライト
- 型推論インレイヒント（変数名の右に `: int` などを表示）
- 対応型: `int`, `float`, `str`, `bool`, `None`, `unknown`

## 未実装の主な機能

- **関数の実行**: `fn` 定義・呼び出し・`return`（パースは完了）
- **クラスの実行**: インスタンス生成・メソッド呼び出し・`self`（パースは完了）
- **型アノテーションの保存**: パース時にスキップしており AST に格納されていない
- **静的型検査の拡張**: 戻り値型・引数型・加算型不一致など
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

## 次に実装すべき機能（優先順）

1. **関数の実行**（`fn` 定義・呼び出し・クロージャ・再帰）
2. **クラスの実行**（インスタンス化・フィールド・メソッド・継承）
3. **型アノテーションの AST 保存**（静的型検査の精度向上に必要）
4. **静的型検査の拡張**（引数型・戻り値型・加算型不一致など）
5. **辞書・セット型**
6. **例外処理**（`try` / `except` / `finally`）
7. **インポートシステム**
