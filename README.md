# test_lang

LLVM IR をターゲットとする独自スクリプト言語の実装プロジェクト。

Python の構文を基盤としつつ、静的型チェックや独自拡張を加えた言語を目指している。

## 特徴

- **Python ベースの構文** — インデントによるブロック構造
- **静的型チェック** — 構文解析時に TypeError を検出
- **明示的な変数宣言** — `let`（不変）/ `mut`（可変）/ `const`（定数）
- **`fn` による関数定義**（`def` ではない）
- **`template` サポート** — 独自のテンプレート機能
- **安全な引数変更** — 変更しうる引数には `mut` を明示

## クイックスタート

```bash
# Rust をインストール済みであること

cargo run -- -src examples/hello.tl
```

### サンプル

```python
let name = "world"
mut x = 10
x += 5
print("Hello,", name)   # Hello, world
print("x =", x)         # x = 15
print("2 ** 8 =", 2 ** 8)  # 2 ** 8 = 256
```

## 言語仕様

| 分類 | 詳細 |
|---|---|
| ファイル拡張子 | `.tl` |
| インデント | Python スタイル（スペース4つ推奨） |
| コメント | `#` 行コメント |
| 変数宣言 | `let x = 1`（不変）/ `mut x = 1`（可変）/ `const X = 1`（定数） |
| 関数定義 | `fn name(args):` |
| クラス定義 | `class Name:` |
| コード生成 | LLVM IR（実装予定）|

### 変数宣言

```python
let x = 42          # 不変（再代入不可）
mut y = 3.14        # 可変（再代入可）
const MAX = 100     # 定数
y = y + 1.0         # OK
x = 0               # TypeError: immutable
```

### 演算子

Python と同じ演算子セット。主な演算子:

```python
# 算術
x + y   x - y   x * y   x / y   x // y   x % y   x ** y

# 比較
x == y   x != y   x < y   x > y   x <= y   x >= y

# 論理
x and y   x or y   not x

# ビット
x & y   x | y   x ^ y   ~x   x << n   x >> n

# 代入
x += 1   x -= 1   x *= 2   # など
```

## 実装状況

### 完了

- [x] 字句解析器（全キーワード・演算子・リテラル）
- [x] 再帰下降パーサー（式・変数宣言・代入）
- [x] ツリーウォークインタープリタ
- [x] 組み込み関数 `print()`
- [x] VS Code 拡張（シンタックスハイライト・型推論インレイヒント）

### 未実装

- [ ] ブロック構文（`if` / `for` / `while` / `match`）
- [ ] 関数定義（`fn`）
- [ ] クラス定義（`class`）
- [ ] 型アノテーション・静的型チェック
- [ ] コレクション型（リスト・辞書・セット）
- [ ] 例外処理（`try` / `except`）
- [ ] インポート（`import` / `from`）
- [ ] LLVM IR コード生成
- [ ] Python 実装

## 開発

```bash
cargo build          # コンパイル
cargo test           # テスト実行
cargo clippy         # lint
cargo fmt            # フォーマット
```

### VS Code 拡張のインストール

```bash
cd vscode-extension
npm install
npm run compile

# 開発モード: VS Code で F5
# インストール: vsce package → .vsix から「VSIXからインストール」
```

## 実装言語

| 言語 | 目的 | 状態 |
|---|---|---|
| Rust | メイン実装（保守性・速度） | 開発中 |
| Python | 頒布性のための追加実装 | 未着手 |
