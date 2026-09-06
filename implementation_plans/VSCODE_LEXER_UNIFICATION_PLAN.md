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

| 構文 | 本物の lexer | `maskLine()` | ズレるか |
|---|---|---|---|
| 複数行 `"""…"""` | 三重引用符を認識（`src/lexer/literal.rs:24`） | 行単位で状態を持たない → **docstring の 2 行目以降の識別子が色付けされる** | **ズレる** |
| `m"…"` / `$…$` | 数学文字列（`src/lexer/scan.rs:176`, `:194`） | 存在を知らない → 中身をコードとして扱う | **ズレる** |
| `r"…"` / `b"…"` のプレフィックス | 文字列トークンの一部（`r` から始まる） | 引用符しか見ないので `r` が裸で残り、識別子として拾われる | **ズレる** |
| `r"a\"` | raw でも `\` は次の 1 文字を巻き込むので閉じない | 同じく `\` の次を読み飛ばす | ズレない |
| `f"{x + 1}"` | f-string 全体で 1 トークン（補間内に別トークンは出ない） | 丸ごと空白化 | ズレない |

⚠ 下 2 行は**当初ズレると書いていたが誤り**。raw 文字列の `\` は
`src/lexer/literal.rs` の raw 分岐でも次の 1 文字を消費する（Python と同じ）ので
`maskLine()` と同じ結果になる。f-string も lexer 側が 1 トークンにするので、
どちらも補間内にコードトークンは出ない。3 件のテストでこの挙動を固定してある
（`crates/arrow-frontend/tests/tokens.rs`）。

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
     - `provideHover` も同様に分岐し、型参照位置では `type int` を出す。
       ただしユーザー定義のクラス・トレイト等は**この分岐を通さず**通常経路へ流す
       （継承元・docstring も含めてそのまま見せたいため）。
     - ~~`provideCompletionItems` は、カーソルが型位置なら型系の宣言だけに絞る~~
       → **#V2 へ移した。** 補完は入力途中に呼ばれるので構文エラー中がほとんどで、
       そのとき `typeRefs` は `lastGood`（古いテキスト）由来になり位置が合わない。
       いま実装するとしたら「直前が `:` か `->` か」を正規表現で見るしかなく、それは
       `wasm_providers.ts` 冒頭が消したはずのヒューリスティックそのもの。
       トークン列（#V2）が入れば `Colon` / `Arrow` の直後かを**現バージョンのテキストで**
       確実に判定できるので、そこまで待つ。
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
| #V1 | `parse_type_expr` に型参照フック | **完了**（§9 に結果） |
| #V2 | トークン列公開＋TS 自前スキャナ 7 箇所撤去 | **完了**（§10 に結果） |
| #V3 | パーサの構造化エラー化 | 未着手 |

## 9. #V1 の結果

### 実装

`EditorIndex.type_refs` ＋ `Parser::note_type_ref()` を追加し、`parse_type_expr` の
識別子アーム・読み飛ばす型引数・`expect_guard_type_name`（`x is int`）の 3 箇所で呼ぶ。
alias 展開中は `enter_alias` / `leave_alias` で記録を止める。
`analyze_json` は `typeRefs` を出し、拡張はセマンティックトークンと hover で
**名前引きより先に**それを見る。

### 実測（`scripts/run_extension_debug.ps1`、ANSI 色コードで判定）

`let v: int = int(a)` の 2 つの `int`:

| | 型注釈の `int` | 呼び出しの `int(` |
|---|---|---|
| 修正前 | `[92m` = **function**（症状） | `[92m` = function |
| 修正後 | `[36m` = **type** | `[92m` = function |

hover:

| 位置 | 結果 |
|---|---|
| `let a: int` の `int` | `type int` |
| `int(a)` の `int` | `fn int(let x: any) -> int` ＋ docstring |
| `let b: Box` の `Box` | `class Box`（ユーザー型は宣言をそのまま保持） |

### ⚠ 副産物: デバッグ環境の穴を 2 つ塞いだ

1. **`run_debug.js` と `stress.js` が `loadPrelude()` を呼んでいなかった。**
   `debug_runner.ts` は `loadPrelude` を import しているのに**一度も呼んでいない**、
   `stress.js` は import すらしていなかった。つまり両者は「組み込みの無い世界」を
   調べており、**この不具合は原理的に再現できなかった**（`int` がどの宣言にも当たらず
   無色になるだけ）。両方で `activate()` と同じく `builtins.ars` を読むよう修正した。
   ⇒ これを直すまで A/B は「無色 → 型色」に見え、症状の再現になっていなかった。

2. **`scripts/run_extension_debug.ps1` を新設。** PATH の node は v11 で wasm を
   コンパイルできない。`compare_wasm_frontend.ps1` と同じく VS Code 同梱の Node
   （`Code.exe` ＋ `ELECTRON_RUN_AS_NODE=1`、実測 v24）を探して使う。
   ⇒ §8 の制約は解消。`skill vscode-debug-runner` の手順はこの環境ではこの scripts 経由で使う。

### ゲート結果

| ゲート | 結果 |
|---|---|
| `cargo test`（ルート） | 772 passed / 0 failed |
| `cd crates/arrow-frontend; cargo test` | 6 passed / 0 failed（新規 `tests/type_refs.rs`） |
| `compare_wasm_frontend.ps1` | compared 226 / agreed 226 / INVENTED 0 / mismatch 0 |
| `scan_examples.ps1` | FAIL 0（`bench_ab_native.ar` の TIMEOUT は既知・環境要因） |
| `force_gate.ps1` | 0 件 |
| `compare_python_impl.ps1` | identical 65 / unexpected diff 0 |
| `stress.js`（全 226 例題） | threw 0 / hover misses 0 / def misses 0 |
| `stale_doc_refs.ps1` | ⚠ **1 件 FAIL（本タスクとは無関係・既存）**。`src/python_converter/param_rewrite.rs:6` の `opts` は Python 側の引数名を説明する表の一部で、Arrow の識別子ではない＝スクリプトの誤検出。本ブランチでは触っていない。 |

---

## 8. 検証環境の制約（#V1 で解消済み）

~~PATH の `node` は v11.5.0 で wasm をコンパイルできない~~
→ **`scripts/run_extension_debug.ps1` で解消。** VS Code 同梱の Node を使う。
デバッグランナーは以下で回す:

```powershell
./scripts/run_extension_debug.ps1 <file.ar> -Build   # -Build は .ts を触ったとき
```

⚠ `-Build` を忘れると `out_debug/` の**前の版**を調べることになる（黙って古い結果が出る）。

## 10. #V2 の結果

### 実装

- `src/lexer/editor_tokens.rs`（新規・`editor` feature 専用）に `tokenize_with_spans()`。
  トークン列と**その範囲**を同時に返す。種別は ident / keyword / str / num / comment / op の 6 つだけ
  （細かくすると Rust 側の分類と拡張側の解釈を同期し続ける必要が生まれ、消したはずの問題が戻る）。
  キーワードか演算子かは `Token::keyword_str()` に判定させるので、キーワードが増えても追随不要。
- `Lexer` に `comment_spans`（`editor` 限定）。**コメントはトークンにならない**ので
  `skip_comment()` でしか捕まえられない。
- `analyze.rs` に `Utf16Cols` を追加し、**全出力の列を UTF-16 に統一**。
  拡張が自前走査をやめた以上ここが唯一の真実源になるため。
- 拡張側は `maskLine()` と識別子正規表現を**削除**。7 箇所をトークン走査へ置換。
- `getAnalysis()` は構文エラー時に「`lastGood` の宣言表 ＋ **現在のテキストの** `tokens`」を
  合成した `view` を返す。合成は `CacheEntry` に 1 つ持つ（毎回作ると WeakMap がミスする）。

### 撤去結果

`wasm_providers.ts` に残る走査は **`parseErrorPosition()` の 1 つだけ**（#V3 の担当）。
`maskLine` / `getWordRangeAtPosition` / 識別子正規表現は完全に消えた。

### 実測

| 項目 | 修正前 | 修正後 |
|---|---|---|
| docstring 2 行目以降の `int str float` | function 色で着色 | 無着色 |
| 複数行にまたがる呼び出しのシグネチャヘルプ | 出ない（1 行しか見ていない） | 出る（`activeParameter=2`） |
| 打ちかけ（構文エラー）中のシグネチャヘルプ | 出ない | **出る**（新しいトークン＋lastGood の宣言表） |
| コメント／文字列の中の hover | 語を拾って出た | 出ない |
| コメントの中の補完 | スコープ補完が 40 件出た | 出ない |
| `let q: ` の補完 | 変数・関数も混ざる | `Shape` だけ（#V1 から繰り越した型位置フィルタ） |
| 診断の波線の長さ | 常に識別子長 or 1 文字 | そのトークンの実長 |

### ゲート結果

| ゲート | 結果 |
|---|---|
| `cargo test`（ルート） | 772 passed / 0 failed |
| `cd crates/arrow-frontend; cargo test` | 16 passed / 0 failed（`tokens.rs` 10 件を新規追加） |
| `compare_wasm_frontend.ps1` | compared 226 / agreed 226 / INVENTED 0 / mismatch 0（**UTF-16 化で診断位置は壊れていない**） |
| `scan_examples.ps1` | FAIL 0（`bench_ab_native.ar` の TIMEOUT は既知） |
| `force_gate.ps1` | 0 件 |
| `compare_python_impl.ps1` | identical 65 / unexpected diff 0 |
| `stress.js`（全 226 例題） | threw 0 / hover misses 0 / def misses 0 |
| `cargo clippy` | 新規警告 0（既存 52 件は変更前から） |

### ⚠ 作業中に踏んだ落とし穴

1. **ブロックコメントに正規表現を書くと `*/` でコメントが閉じる。**
   `` `/[A-Za-z_]\w*/g` `` を `/** … */` の中に書いて TS がパースエラーになった。
   このファイルは正規表現の話をする機会が多いので、`//` 行コメントを使うか literal を避けること。
2. **`.ps1` は UTF-8 BOM 必須。** BOM 無しで日本語コメントを書くと PS 5.1 が ANSI として
   読み、パースが壊れる（`scripts/*.ps1` は全て BOM 付き）。
3. **`Code.exe` を Node として使うときは `ProcessStartInfo` 直叩き。** `&` も `Start-Process` も
   stdout を取りこぼす（`compare_wasm_frontend.ps1` と同じ）。
4. **プローブスクリプトで `uri` を使い回さない。** providers は uri+version でキャッシュするので、
   同じ uri の別ドキュメントを続けて調べると最初の結果に汚染されて**全部空**になる。
   これで「補完が全滅した」と誤読しかけた。