# VSCODE_LEXER_UNIFICATION_PLAN.md — 拡張の自前字句解析を撤去する（2026-09-03 起票）

**きっかけ**: 「VS Code 拡張が `int` などの型名を、時折キャスト関数と誤認する」。
調査の結果、症状は表層で、**フロントエンドの流用が着色経路まで届いていない**ことが原因と判明した。
本書はその是正計画（#V1〜#V3）を扱う。

> ## ✍️ 記載規約（`implementation_logs/FUTURE_FEATURE.md` に準拠）
>
> 1. **1 タスク = 意義 / 前提 / 手法 / 留意点 / 検証 / 参照** を必ず書く。
> 2. **外部にある情報は複製しない。** パスと「そこに何があるか」の 1 行だけを書く。
> 3. 着手したら §7 の状態表を更新する。

## 外部参照（ここには複製しない）

| 参照先 | そこにあるもの |
|---|---|
| skill `vscode-extension-dev` | 拡張の機能追加手順と **VSIX 梱包（`make-vsix.ps1`）** |
| skill `vscode-debug-runner` | `run_debug.js` / `stress.js` の使い方と再ビルド範囲 |
| skill `language-dev-principles` | 「挙動不変」を主張する前に回すゲートの選び方 |
| skill `parser-internals` | `src/parser/` のモジュール構成と precedence chain |
| skill `codebase-map` | ファイルの所在（行数つき） |
| [`src/parser/editor_index.rs`](../src/parser/editor_index.rs) 冒頭 doc | **AST を変えずに副次テーブルを持つ**という設計判断とその理由 |
| [`src/parser/editor_hooks.rs`](../src/parser/editor_hooks.rs) 冒頭 doc | `#[cfg(feature = "editor")]` フックの書き方（引数に `String` を渡さない規約） |
| [`vscode-extension/src/wasm_providers.ts`](../vscode-extension/src/wasm_providers.ts) 冒頭 doc | 旧 `analysis.ts` を捨てた理由（TS 側に言語仕様の判断を置かない） |

---

## 1. 調査で確定した事実

着手時に再調査しなくて済むよう、**根拠となる位置**だけを残す。

### 1-a. 型名と組み込み関数が兼用の名前は 9 個

[`vscode-extension/builtins.ars`](../vscode-extension/builtins.ars) の `fn` 名と、
[`syntaxes/arrow.tmLanguage.json`](../vscode-extension/syntaxes/arrow.tmLanguage.json) の
`builtin-type` リストの積集合：

```
bool, float, int, path, set, slice, str, type, uint
```

`list` / `dict` / `tuple` などは `fn` 宣言が無いので症状が出ない。
「**時折**」という体感の正体はこの 9 個に限られること。
実体としての兼用は本物で、`int(x)` は `src/interpreter/eval/calls.rs:423` が処理する組み込み呼び出し。

### 1-b. 誤認の直接原因

`vscode-extension/src/wasm_providers.ts:403-418` のセマンティックトークン生成は、
行を `/[A-Za-z_]\w*/g` で総なめし、**名前だけ**で宣言表（`visibleSymbols()`、末尾に prelude を連結）を引く。
`int` は prelude に `kind: "function"` としてしか存在しないため、
`let x: int` / `-> int` / `list[int]` のすべてが `function` トークンになる。
hover（`lookup()` 経由）・補完も同じ理由で関数として出る。

TextMate 文法は**正しい**（`builtin-type` が `builtin-func` / `function-call` より前に include されている）。
VS Code はセマンティックトークンを TextMate より優先するため、正しい下地が誤った意味解析で塗り潰されている。

⚠ **切り分け手順**: `settings.json` に `"[arrow]": { "editor.semanticHighlighting.enabled": false }`
を入れて誤認が消えれば、原因はセマンティックトークン層で確定する。

### 1-c. 構造的原因は「情報が 2 段階で失われている」こと

| 層 | 型位置の情報を持つか |
|---|---|
| lexer | **持たない**。`int` は `Token::Ident("int")`。`src/lexer/keyword.rs` に型名は 1 つも無い（`src/lexer/` で `"int"` に触れるのは `math.rs` の `\int → ∫` だけ） |
| parser `parse_type_expr`（`src/parser/types.rs:199`） | **持つ**。`let x: T` / `-> T` / `list[T]` / `Union[...]` / テンプレート引数が通る唯一の関門 |
| AST | **失う**。型注釈は `Option<String>` に潰れ、書かれた位置が消える |
| `EditorIndex` | **持たない**。`decls` / `scopes` / `node_spans` のみ。型参照の欄が無い |
| `analyze_json` の JSON | **持たない**。キーは `ok, parseError, diagnostics, symbols, scopes, exprTypes, members` のみ |

### 1-d. トークン列は作られているが捨てられている

`crates/arrow-frontend/src/analyze.rs:200` で本物の `Lexer` を回しているが、
結果は `Parser::new(tokens, None)` に渡されてそこで消費されて終わり。
`Spanned { token, span }` として行・列つきの完全なトークン列が手元にあるのに JSON に出ていない。

その結果、TS 側には **2 つ目の字句解析器**（`maskLine()` ＋ 識別子正規表現）が残っている。
これは本物の lexer と実際にズレている：

| 構文 | 本物の lexer | `maskLine()` |
|---|---|---|
| 複数行 `"""…"""` | 三重引用符を認識（`src/lexer/literal.rs:24`） | 行単位で状態を持たない → **docstring の 2 行目以降の識別子が色付けされる** |
| `r"…\"` | raw では `\` はエスケープでない | `\` を常にエスケープ扱い → 引用符追跡がずれ、以降が誤マスク |
| `f"{x + 1}"` | 補間部を式としてトークン化 | 丸ごと空白化 → 補間内に色が付かない |
| `m"…"` / `$…$` | 数学文字列（`src/lexer/scan.rs:176`, `:194`） | 存在を知らない |

### 1-e. トークン列は構文エラー中でも必ず得られる

`Lexer::tokenize()`（`src/lexer/scan.rs:100`）は `-> Vec<Spanned>` で**失敗を表現しない**。
未終端文字列も `src/lexer/literal.rs:29` の `None => break` で EOF まで飲んで正常終了する。

一方 AST は `parse_program()` が `Err` なら存在せず、`analyze.rs:206-213` は
`symbols` / `scopes` / `exprTypes` を全部空にする。TS 側は `lastGood`
（**古いテキストの解析結果**）の位置を現在のバッファに当てている。
**これが自前スキャナが残っていた実質的な理由**であり、#V2 で消える。

---

## 2. #V1 — `parse_type_expr` に型参照フックを足す（`int` 誤認の直接修正）

- **意義**: 症状に直結する唯一のタスク。#V2 / #V3 をやっても、これが無ければ
  `int` は関数のまま（トークン列だけでは型/関数を区別できない — §1-c の lexer 行）。
- **前提**: なし。単独で着手・出荷できる。
- **手法**:
  1. `EditorIndex` に `pub type_refs: Vec<(Pos, String)>` を追加。
  2. `editor_hooks.rs` に既存の `note_*` と同じ体裁で
     `note_type_ref(&mut self, name: &str)` を追加。本体は `#[cfg(feature = "editor")]`、
     位置は既存の `prev_pos()` から取る。
  3. `parse_type_expr` が型名トークンを消費する箇所で 1 回呼ぶ。
     **再帰するので `dict[str, int]` の内側や `Union[...]` の各引数も自動で拾える。**
  4. `analyze.rs` の JSON に `"typeRefs": [{ at, name }]` を追加。
  5. TS 側：
     - セマンティックトークンのループで、`line:col` が `typeRefs` にあれば
       **名前引きより先に** `type`（同名のクラス/トレイトが見えていれば `class` / `interface`）
       を出して `continue`。
     - `provideHover` も同様に分岐し、`renderSignature` に型参照用の枝（`type int` 等）を足す。
     - `provideCompletionItems` は、カーソルが型位置なら型系の宣言だけに絞る。
- **留意点**:
  - ⚠ **AST を 1 バイトも変えないこと。** `editor_index.rs` 冒頭が書いているとおり、
    `Stmt::Let` のようなタプルバリアントに 1 フィールド足すだけで 237 箇所以上に波及する。
    副次テーブル方式を崩さない。
  - ⚠ `note_type_ref` の引数は `&str` に留める。`String` や `format!` を渡すと
    通常ビルドでも確保コストだけが残る（`editor_hooks.rs` 冒頭の規約）。
  - ⚠ この変更は**診断を増減させない**。増減したら型位置の判定を間違えている。
- **検証**: `compare_wasm_frontend.ps1`（診断が `arrow.exe` と一致すること）／
  パーサに触るので `scan_examples.ps1`・`force_gate.ps1`・`compare_outputs.ps1`／
  `make-vsix.ps1`（wasm 再ビルド込み）。
- **完了条件**: `let x: int = int(n)` の 1 つ目の `int` が type 色、2 つ目が function 色。
  hover もそれぞれ `type int` / `fn int(x: any) -> int`。

---

## 3. #V2 — トークン列を公開し、TS 側の自前スキャナ 7 箇所を撤去する

- **意義**: 「拡張だけ解釈がずれる」構造を着色経路にも初めて適用する。
  §1-d のズレが一括で消え、さらに §1-e により `lastGood` の弱点（古いテキストの位置を
  現バッファに当てる）が改善する。
- **前提**: なし（#V1 とは独立だが、§6 の理由で #V1 の**後**に回す）。
- **撤去対象**: TS 側の走査箇所は 8 個、うち 7 個が消える。

  | 箇所（`wasm_providers.ts`） | 用途 | 置き換え |
  |---|---|---|
  | `maskLine()` (L370) | 文字列・コメントを潰す | 不要（トークンが分離済み） |
  | 識別子スキャン `/[A-Za-z_]\w*/g` (L403) | 識別子列挙 | `Ident` トークンを走る |
  | `wordAt()` (L190) | カーソル下の語 | 位置を含むトークンを二分探索 |
  | `receiverTypeAt()` (L446) | `x.` の受け手 | 末尾から `Dot` ← `Ident` |
  | `callContext()` (L525) | 呼び出し名と引数 index | トークンで括弧深さを数える |
  | `provideDefinition` の `/\.\s*$/` (L591) | 同上 | 同上 |
  | `toDiagnostic()` の語長取得 (L687) | 波線の長さ | そのトークンの実長 |
  | `parseErrorPosition()` (L678) | エラー文字列から行・列 | **不可** → #V3 |

- **副次的に直るもの**（現時点で既に壊れている）:
  - `callContext()` / `receiverTypeAt()` は `document.lineAt(position.line)` で
    **1 行しか見ていない** → 複数行にまたがる関数呼び出しでシグネチャヘルプが出ない。
  - `receiverTypeAt()` の `/([A-Za-z_]\w*)\s*\.\s*$/` は**直前の 1 識別子しか見ない**
    → `a.b.c.` の連鎖で受け手を誤る。

- **手法**:
  1. `#[cfg(feature = "editor")]` な `Lexer::tokenize_spans()` を lexer 側に足す
     （通常ビルドには存在しない ＝ インタプリタへのコストゼロ）。
  2. `analyze.rs` に `"tokens": [{ line, col, endLine, endCol, kind }]` を追加。
     `kind` は `ident` / `keyword` / `str` / `num` / `comment` / `op` / `layout` 程度の粗い分類で足りる。
  3. **`ok: false` のときも `tokens` は現バージョンのものを返す**（ここが肝）。
  4. TS 側の 7 箇所をトークン走査に置換し、`maskLine()` を削除する。

- **留意点（着手前に必ず読む — 実装を止める 4 点）**:
  1. ⚠ **コメントはトークンにならない。** `src/lexer/scan.rs:155-158` が `skip_comment()` して
     `next_token()` を再帰するだけで、`Token::Comment` は `src/token.rs` に存在しない。
     マスク用途にはコメント範囲が要るので、`editor` feature 下でコメント範囲を記録する追加が必須。
     **これが最大の落とし穴。**
  2. ⚠ **`Spanned` は開始位置しか持たない。** `src/token.rs:66` は `{ token, span }` のみ。
     ただし `pos` は**文字インデックス**（`scan.rs:55-57` の `chars: Vec<char>` /
     `positions: Vec<(usize, usize)>`）なので、`tokenize()` のループ内で `next_token()` 前後の
     `self.pos` を `span_at()` に通せば終端が取れる。
     ⚠ `pending`（INDENT/DEDENT）は `pos` を動かさずに返るため終端が当てにならない。
     視覚的な幅が無いので除外する扱いにすること。
  3. ⚠ **列の単位が違う。** `Span.col` は「文字単位」（`src/token.rs:18`）、
     VS Code の `Position.character` は UTF-16 コードユニット。BMP 内なら一致するが絵文字等でずれる。
     既存の問題ではあるが、自前スキャナを消すとこれが**唯一の真実源**になるので、
     `analyze.rs` で UTF-16 オフセットに変換しておくのが安全。
  4. ⚠ **`Lexer.pos` は `pub(super)`。** `crates/arrow-frontend/src/analyze.rs` からは読めない。
     手法 1 のとおり lexer 側にメソッドを足すのが素直（`analyze.rs` から覗こうとして詰まらないこと）。

- **検証**: `compare_wasm_frontend.ps1`／`stress.js`（全例題で hover・補完・トークンが落ちないこと）／
  `scan_examples.ps1`・`force_gate.ps1`（lexer に触るため）／`make-vsix.ps1`。
- **完了条件**: `wasm_providers.ts` に `maskLine` と識別子スキャン正規表現が存在しない。
  複数行 docstring の中身が色付けされない。複数行の関数呼び出しでシグネチャヘルプが出る。

---

## 4. #V3 — パーサの構造化エラー化（`parseErrorPosition()` の撤去）

- **意義**: 残る 1 箇所を消す。現在は `parse_program()` が `Result<_, String>` を返し、
  **位置を人間向け文字列に埋め込んで捨てている**ため、TS 側が
  `/line\s+(\d+)[,:]?\s*(?:col(?:umn)?\s+(\d+))?/i` でメッセージを読み直している。
- **前提**: なし。ただし#V1 / #V2 より**影響範囲が広い**（パーサの `Err` 生成箇所すべて）。
- **手法**: パーサのエラー型に `Span` を持たせ、`analyze.rs` が
  `parseError` を `{ message, at }` の形で返す。
- **留意点**:
  - ⚠ **端末出力のフォーマットを変えないこと。** `compare_outputs.ps1` 等のゲートは
    stderr を比較対象にしているので、表示文字列が変わると既存の基準が全部ずれる。
    構造化は**内部表現の追加**に留め、`Display` は現状を維持する。
  - 字句解析の話ではないので、#V2 とは**必ず別コミットにする**。
- **検証**: `scan_examples.ps1`・`compare_outputs.ps1`（エラーメッセージが byte-identical）。

---

## 5. 消さないもの（削除候補に見えるが残す）

| 対象 | 判断 |
|---|---|
| [`syntaxes/arrow.tmLanguage.json`](../vscode-extension/syntaxes/arrow.tmLanguage.json) | **残す。** activate 前と wasm 読み込み失敗時にこれしか色が無い。ただし責務は「セマンティックトークンが来るまでの下地」に縮小してよい。食い違えば後から来るセマンティック側が必ず勝つので、**文法側は粗い方が安全**。 |
| [`language-configuration.json`](../vscode-extension/language-configuration.json) | **残す。** 括弧対応・インデント規則は VS Code が同期的に要求するもので wasm では代替不能。そもそも字句解析ではない。 |
| [`src/parser/imports_editor.rs`](../src/parser/imports_editor.rs) | **残す。** `CLAUDE.md` に明記された既知の唯一の例外（fs に触れない import 解析）。 |

---

## 6. 順序と分割の理由

**#V1 → #V2 → #V3** の順。理由は切り分け可能性。

#V1 と #V2 を同時にやると、着色が変わった原因が「型参照フック」なのか
「トークン列への切り替え」なのか判別できなくなる。#V1 は差分が小さく
`compare_wasm_frontend.ps1` で単独検証しやすいので、症状の修正を先に出荷する。

---

## 7. 状態

| # | タスク | 状態 |
|---|---|---|
| #V1 | `parse_type_expr` に型参照フック | 未着手 |
| #V2 | トークン列公開＋TS 自前スキャナ 7 箇所撤去 | 未着手 |
| #V3 | パーサの構造化エラー化 | 未着手 |

---

## 8. 検証環境の制約（着手時に効いてくる）

⚠ **この調査時点で `node --version` は v11.5.0** であり、
`node vscode-extension/run_debug.js <file.ar>` は wasm のコンパイルに失敗する
（`expected table index 0, found 128`）。skill `vscode-debug-runner` の手順を使うには
**Node 16 以降が必要**。

⇒ 本書 §1 の内容は**すべてコード経路の読み取りに基づく**もので、
実行による裏取りは取れていない。着手時はまず Node を上げ、
`run_debug.js` の 1 節目（セマンティックトークン着色）で
`int` が function 色になることを**先に再現**してから直すこと。
