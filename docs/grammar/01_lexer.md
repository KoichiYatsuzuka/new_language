# 字句解析 (Lexer)

```
src/lexer/
├── scan.rs    — Lexer 本体・インデント処理
├── chars.rs   — 文字参照ヘルパー (ch / ch1 / bump)
├── keyword.rs — 識別子・キーワード解析
├── literal.rs — 文字列・数値リテラル解析
├── symbol.rs  — 演算子・区切り記号解析
└── math.rs    — LaTeX 風数式文字列の Unicode 変換
```

---

## トークンの種類

### 変数宣言キーワード

| トークン | ソース記号 |
|---|---|
| `Let` | `let` |
| `Mut` | `mut` |
| `Const` | `const` |
| `Static` | `static` |
| `Freeze` | `freeze` |

### 制御構文キーワード

| トークン | ソース記号 |
|---|---|
| `If` / `Elif` / `Else` | `if` / `elif` / `else` |
| `Match` / `Case` | `match` / `case` |
| `For` / `While` | `for` / `while` |
| `Break` / `Continue` / `Pass` | `break` / `continue` / `pass` |
| `Return` / `Yield` / `YieldFrom` | `return` / `yield` / `yield from` |
| `BlockReturn` | `block_return` |
| `LoopYield` | `loop_yield` |
| `Block` | `block` |

### 例外処理キーワード

| トークン | ソース記号 |
|---|---|
| `Try` / `Except` / `Finally` | `try` / `except` / `finally` |
| `Raise` | `raise` |

### 定義キーワード

| トークン | ソース記号 |
|---|---|
| `Fn` | `fn` |
| `Gen` | `gen` |
| `Class` | `class` |
| `Trait` | `trait` |
| `Enum` | `enum` |
| `Template` | `template` (予約語) |

### 型関連キーワード

| トークン | ソース記号 |
|---|---|
| `SelfType` | `Self` |
| `NewType` | `new_type` |
| `Any` | `Any` |
| `Union` | `Union` |
| `Option` | `Option` |

### アクセス修飾子

| トークン | ソース記号 |
|---|---|
| `Public` | `public` |
| `Private` | `private` |
| `Protected` | `protected` |
| `ClassMethod` | `class_method` |

### インポート

| トークン | ソース記号 |
|---|---|
| `Import` | `import` |
| `From` | `from` |
| `As` | `as` |

### リテラル値キーワード

| トークン | ソース記号 |
|---|---|
| `True` / `False` | `True` / `False` |
| `None` | `None` |

### 論理・比較演算子キーワード

| トークン | ソース記号 |
|---|---|
| `And` / `Or` / `Not` | `and` / `or` / `not` |
| `In` / `NotIn` | `in` / `not in` |
| `Is` / `IsNot` | `is` / `is not` |

> `not in`・`is not` は複合キーワード。字句解析器が先読みして単一トークンに変換します。

---

## 算術・代入・ビット演算子

```
+   -   *   /   //  %   **  @
+=  -=  *=  /=  //= %=  **= @=
==  !=  <   >   <=  >=
&   |   ^   ~   <<  >>
&=  |=  ^=  <<= >>=
=   :=  ::
->  <-  =>
```

---

## リテラル

### 整数リテラル

```ar
42          # 10 進数
0xFF        # 16 進数
0o77        # 8 進数
0b1010      # 2 進数
1_000_000   # アンダースコア区切り
```

**パース処理**:  
先頭 `0x`/`0X` → 16進、`0o`/`0O` → 8進、`0b`/`0B` → 2進として処理。  
アンダースコアを除去してから `i64::from_str_radix()` でパース。

### 浮動小数点リテラル

```ar
3.14
2.5e10
1.0e-3
```

整数部のあとに `.数字` または `e`/`E` が続く場合に `f64` として解析。

### 文字列リテラル

| 形式 | 説明 |
|---|---|
| `"hello"` / `'hello'` | 通常文字列 (エスケープ処理あり) |
| `"""..."""` / `'''...'''` | トリプルクォート文字列 (改行含む) |
| `r"hello\n"` | Raw 文字列 (バックスラッシュをそのまま保持) |
| `f"x = {expr}"` | f-string (式補間) |
| `fr"..."` / `rf"..."` | Raw f-string |
| `m"\alpha_1^2"` | 数学文字列 (LaTeX 記法 → Unicode 変換) |
| `$\alpha_1^2$` | 数学文字列の短縮記法 |

**エスケープシーケンス** (raw でない文字列で有効):

| シーケンス | 文字 |
|---|---|
| `\n` | 改行 |
| `\t` | タブ |
| `\r` | キャリッジリターン |
| `\\` | バックスラッシュ |
| `\'` / `\"` | クォート |
| `\0` | ヌル文字 |

**f-string の処理**:  
`{expr}` の部分を `FStrPart::Expr(src)` として保存し、  
`{{` / `}}` をエスケープされた `{` / `}` として扱います。  
評価はインタープリタ側で行います。

**数学文字列の変換例** (LaTeX → Unicode):

| 入力 | 出力 |
|---|---|
| `\alpha` | `α` |
| `x^2` | `x²` |
| `v_{n}` | `vₙ` |
| `\times` | `×` |

---

## インデント処理 (INDENT/DEDENT)

Python スタイルのインデントブロックをサポートします。

**動作ルール**:
1. 行頭 (`at_line_start = true`) かつ括弧深さ 0 のとき `handle_indent()` を呼ぶ
2. 現在行のインデントレベル (スペース数、タブは8の倍数に展開) を計算
3. 空行・コメント行はスキップ
4. 前のレベルより **大きい** → `Token::Indent` を生成してスタックに積む
5. 前のレベルより **小さい** → `Token::Dedent` を必要な数だけ生成してスタックを巻き戻す
6. 変化なし → 通常トークン読み取りに進む
7. EOF → スタックに残ったレベル分の `Token::Dedent` を生成

**括弧内の改行無視**:  
`(`/`[`/`{` の入れ子深さ (`bracket_depth`) が 1 以上のとき、  
改行文字は読み飛ばして次のトークンを読み続けます。  
これにより複数行の関数呼び出し・リストリテラルを記述できます。

---

## コメント

`#` から行末までをコメントとして読み飛ばします。  
ブロックコメントはありません。

---

## 特殊トークン

| トークン | 意味 |
|---|---|
| `Newline` | 論理的な行末 (括弧外の改行) |
| `Indent` | インデントレベル増加 |
| `Dedent` | インデントレベル減少 |
| `Eof` | 入力終端 |
| `Unknown(char)` | 未知の文字 (エラー回復用) |

---

## Spanned の構造

すべてのトークンは位置情報 (`Span`) を持ちます。

```rust
struct Span {
    file: Arc<str>,  // ファイル名 (Arc で共有)
    line: usize,     // 1 始まり行番号 (0 は位置不明)
    col:  usize,     // 1 始まり列番号
}

struct Spanned {
    token: Token,
    span:  Span,
}
```
