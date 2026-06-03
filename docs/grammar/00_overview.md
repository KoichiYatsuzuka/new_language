# Havakyrie 言語 — パイプライン全体図

## 概要

Havakyrie はインデントベースの静的型付きスクリプト言語です。  
ソースファイル (`.hv`) はつぎの 4 段階のパイプラインで実行されます。

```
.hv ファイル
    ↓ ファイル読み込み
ソーステキスト (String)
    ↓ 字句解析 (Lexer)
トークン列 (Vec<Spanned>)
    ↓ 構文解析 (Parser)
AST (Vec<Stmt>)  ← import 先モジュールの AST も再帰的に埋め込まれる
    ↓ 静的型検査 (TypeChecker)
StaticTypeError 一覧  ← 1 件以上あれば実行せずに全件報告して終了
    ↓ 実行 (Interpreter)
副作用 / 戻り値
```

---

## 各段階の詳細

### 1. ファイル読み込み

```
src/main.rs — read_file() / Mode::Run
```

- `cargo run -- <file.hv>` または `-src <file.hv>` で起動
- `std::fs::read_to_string` でソーステキスト全体を `String` に読み込む
- `.hvc` (コンパイル済みモジュール) の場合は先頭ヘッダを解析してソーステキストを取り出す
- 標準入力モード (`Mode::Stdin`) では `stdin.read_to_string` を使用
- エラー時は `stderr` に出力してプロセス終了 (exit code 1)

### 2. 字句解析 (Lexer)

```
src/lexer/scan.rs — Lexer::new(source, filename).tokenize()
```

- ソーステキストを文字単位でスキャンして `Vec<Spanned>` を生成
- 各 `Spanned` は `Token` (トークン種別と値) と `Span` (ファイル名・行・列) のペア
- インデントベースの INDENT/DEDENT 生成
- 括弧内 (`()`, `[]`, `{}`) での改行は無視
- 詳細は [01_lexer.md](01_lexer.md) を参照

### 3. 構文解析 (Parser)

```
src/parser/ — Parser::new(tokens, source_dir).parse_program()
```

- 再帰下降パーサー
- `Vec<Spanned>` を受け取り `Vec<Stmt>` (AST) を返す
- import 文に遭遇すると対象ファイルを再帰的にパースして `Stmt::Import.body` に埋め込む
- パースエラーは最初の 1 件で停止して `Err(String)` を返す
- 詳細は [03_expressions.md](03_expressions.md) / [04_control_flow.md](04_control_flow.md) を参照

### 4. 静的型検査 (TypeChecker)

```
src/type_check/ — TypeChecker::check(&stmts)
```

- AST を走査して型エラーを収集する
- エラーは 1 件でも即停止せず **全件収集してから** まとめて報告する
- エラーが 0 件であれば次段階へ進む
- 詳細は [08_type_system.md](08_type_system.md) を参照

### 5. 実行 (Interpreter)

```
src/interpreter/ — Interpreter::new() → .exec(stmt)
```

- ツリーウォーク型インタープリタ
- 文の実行: `exec(stmt) → Result<ExecResult, String>`
- 式の評価: `eval(expr) → Result<Value, String>`
- 詳細は [02_variables.md](02_variables.md) 以降の各ドキュメントを参照

---

## エラーチャンネルの設計

実行時エラーには 2 種類のチャンネルがあります。

| チャンネル | 型 | 用途 |
|---|---|---|
| `exec()` の戻り値 | `Ok(ExecResult::Raise(e))` | 言語レベルの例外伝播 |
| `eval()` の戻り値 | `Err("\x00__raise__")` | 式評価中の例外伝播 (センチネル文字列) |
| その他 `Err(msg)` | `Err(String)` | インタープリタ内部エラー |

例外が `eval()` で発生すると `Err(RAISE_SENTINEL)` を返しつつ  
`self.current_exception` に `RaisedError` を格納します。  
呼び出し元は `take_current_exception()` で取り出して再送出または捕捉します。

---

## CLI オプション

```
cargo run -- <file.hv>             # 通常実行
cargo run -- -src <file.hv>        # -src フラグ指定
cargo run -- --repl                # 対話型 REPL
cargo run -- --compile <file.hv>   # .hvc / .hvs の生成
cargo run --                       # 標準入力から実行
cargo run -- --key value <file.hv> # ユーザー定義 CLI パラメータ
```

ユーザー定義パラメータは `args["key"]` (dict) としてスクリプト内から参照できます。
