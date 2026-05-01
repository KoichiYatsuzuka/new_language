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
│   ├── token.rs         # Token enum（字句の種類）
│   ├── lexer.rs         # 字句解析器
│   ├── ast.rs           # AST ノード定義
│   ├── parser.rs        # 再帰下降パーサー
│   └── interpreter.rs   # ツリーウォークインタープリタ
├── spec/
│   ├── general.md       # 言語全般仕様
│   ├── keywords.md      # キーワード一覧
│   └── operator.md      # 演算子一覧・優先順位
├── examples/
│   └── hello.tl         # サンプルプログラム
└── vscode-extension/    # VS Code 拡張（型推論インレイヒント）
    └── src/
        ├── extension.ts
        └── type_infer.ts
```

## 実装済みの機能

### 字句解析（`src/lexer.rs`）
- 全キーワード・演算子・リテラル対応
- インデント追跡（INDENT / DEDENT トークン生成）
- 括弧内での改行無視
- 複合キーワード: `not in`, `is not`, `yield from`
- 数値リテラル: 10進・16進・8進・2進・アンダースコア区切り
- 文字列: シングル・ダブル・トリプルクォート・エスケープ

### 構文解析（`src/parser.rs`）
- 変数宣言: `let`（不変）、`mut`（可変）、`const`（不変）
- 代入: `x = expr`、複合代入: `x += expr` など全種
- 式: 演算子優先順位を仕様通りに実装（右結合 `**` 含む）
- 関数呼び出し: `f(args)`

### インタープリタ（`src/interpreter.rs`）
- 変数の不変/可変チェック
- 算術・比較・論理・ビット演算
- 文字列連結 (`+`)
- `and` / `or` の短絡評価
- ゼロ除算エラー
- 組み込み関数: `print()`

### VS Code 拡張（`vscode-extension/`）
- `.tl` ファイルのシンタックスハイライト
- 型推論インレイヒント（変数名の右に `: int` などを表示）
- 対応型: `int`, `float`, `str`, `bool`, `None`, `unknown`

## 未実装の主な機能

- **ブロック構文**: `if` / `elif` / `else` / `for` / `while` / `match`
- **関数定義**: `fn`
- **クラス定義**: `class`（`Token::Class` は字句解析済み）
- **型アノテーション**: `x: int`
- **コレクション型**: リスト・辞書・セット
- **例外処理**: `try` / `except` / `finally`
- **インポート**: `import` / `from`
- **LLVM IR コード生成**（現在はツリーウォークインタープリタで動作）
- **Python 実装**

## 言語仕様の要点（Python との違い）

- 変数宣言に `let` / `mut` / `const` が必要
- 関数定義は `def` ではなく `fn`
- 構文解析時に静的型チェックを行い `TypeError` を出す
- `template` キーワードによるテンプレートをサポート
- 変更しうる引数には `mut` を明示する必要がある
- 空のコレクションは型を明示しなければならない（`list[int]` など）
- `dataclass` / `enum` などを標準でサポート
- 値オブジェクト（型キャスト）による誤演算防止

## 次に実装すべき機能（優先順）

1. **インデントブロックのパース**（if・for・fn・class 共通基盤）
2. `if` / `elif` / `else`
3. `fn` 関数定義・呼び出し
4. `for` / `while` ループ
5. `class` 定義
