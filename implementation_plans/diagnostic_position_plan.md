# 静的な誤りの位置が文の先頭に寄る（波線が原因の式に引かれない）

型検査の誤りの多くが、原因の式ではなく**文の先頭のトークン**の位置で報告される。
VS Code 拡張はその位置のトークン 1 個に波線を引くので、`while running:` の `running` が未定義でも
波線は `while` に引かれる。CLI の表（`Line:Col`）も同じ位置を出す（拡張だけの問題ではない）。

- 起票: 2026-11-11（`TeX_editor/main.ar` の 98 行目の `while running:` で見つけた）
- 調査: Arrow `bbd6fda6`
- 方針: **B 案**（識別子に位置を持たせる＋「今推論している式」の位置を誤りの既定の位置にする）

## 0. 何が起きるか

```
fn main() -> None:
    let x = 1
    while running:                    # name 'running' is not defined → 3:5（while）
        print(x)
    let y: int = x + undefined_thing  # name 'undefined_thing' is not defined → 5:5（let）
```

`AR_CHECK_ONLY=1 arrow.exe` の表でも、どちらも名前ではなく文の先頭の位置になる。

## 1. 原因

| 段 | 場所 | 何をしているか |
|---|---|---|
| ① | `src/ast.rs` の `Expr::Ident { name, node_id, res }` | 識別子が**位置を持たない** |
| ② | `src/type_check/infer.rs` の `check_name_defined` | そのため `UndefinedName` を `span: None` で報告する |
| ③ | `src/type_check/scope.rs` の `report_error` / `report_warning` | `span` が無い誤りには**今の文の位置**（`stmt_pos`・フェーズ10 10-17）を付ける |
| ④ | `vscode-extension/src/wasm_providers.ts` の診断の変換 | 受け取った位置のトークン 1 個を波線の範囲にする |

④は正しい（位置のトークンに引くだけ）。直すのは①〜③。

**名前の誤りに限らない。** `src/type_check` で `span: None` の誤り・警告は **91 か所**
（`stmt/check.rs` 34・`call_check.rs` 33・`infer.rs` 11・`stmt/resolve.rs` 7・`decorator.rs` 5・`stmt/protocol.rs` 1）、
位置付きは約 66 か所。`span: None` の誤りはどれも③で文の先頭に寄る。

AST で位置を持つ式は 6 種類だけで、指しているトークンは次のとおり（`Expr::own_span`）。

| 式 | `span` が指すトークン |
|---|---|
| `Call` | `(` |
| `Attr` | `.` |
| `BinOp` | 演算子 |
| `Cast` | `=>` |
| `IsType` / `MustBe` | `is` / `mustbe` |

`Ident`・`Subscript`・`UnaryOp`・リテラル・`List` / `Dict` / `Tuple` / `Set`・`TraitAccess`・`TemplateInstantiate`・
`if` / `for` / `while` / `match` / `block` の式は位置を持たない。

⚠ 拡張の `EditorIndex::node_spans`（node-id → 位置）は使えない。エディタのビルドでしか作られず、記録するのは
**式の末尾**のトークン（`Parser::note_node_span`）で、node-id もモジュールごとの採番が混じる。

## 2. 方針（B 案）

1. **識別子に位置を持たせる**（`Expr::Ident` に `span: Span`）。未定義の名前は名前そのものを指す。
2. **型検査器が「今推論している式の位置」を持つ。** `infer` の入口でその式の**錨**（下の表）を控え、
   出口で元に戻す。`report_error` / `report_warning` は `span` が無ければ
   **錨 → `stmt_pos`** の順に使う。こうすると 91 か所を個別に直さなくても、誤りを出した時点で推論中の
   一番内側の式に寄る。
3. 波線として意味のある位置にするため、**錨は式ごとに決める**。`Call` の `(` や `Attr` の `.` は 1 文字だけの波線になって、
   今の `while` と同じくらい分かりにくい。

### 2.1 錨の規則（`Expr::anchor_span`・新設）

| 式 | 錨 | 理由 |
|---|---|---|
| `Ident` | 名前 | 未定義・型の不一致の主語 |
| `Attr` | **属性名** | `obj.missing` の `missing`。⚠ 今の `span`（`.`）は残す（実行時の表示が使う・3-1 で確かめる） |
| `Call` | **呼ばれる側の錨**（`f(..)` → `f`・`o.m(..)` → `m`）。無ければ `(` | 引数の数・型の誤りは「どの関数か」を指す |
| `BinOp` | 演算子 | `+` を支えない型の組 |
| `Cast` / `IsType` / `MustBe` | 今の `span` | 同上 |
| `Subscript` | 対象の錨 | `xs[i]` の `xs` |
| `TemplateInstantiate` | 対象の錨 | `Box[int]` の `Box`（1-3 で足した） |
| その他（リテラル・`UnaryOp`・コンテナ・制御構文の式…） | **なし**（外側の錨を引き継ぐ） | 位置を持たない式で錨を消さない |

### 2.2 守ること

- ⚠⚠ **式の中の文は錨を引き継がない。** `if` 式・`for` 式・`block:`・ラムダ（関数）本体の中の文は `check_stmt` を通る。
  `check_stmt` の入口で錨を**空にして**控え、出口で戻す。空にしないと、本体の中の誤りが外側の式（`let r = if ..` の `if` 式の外の呼び出しなど）を指す。
- ⚠ **推論が終わってから出す誤りは錨が戻った後に出る。** `let z: str = s` の不一致は `infer(s)` が返った後に
  `check_var_decl` が出すので、今までどおり文の位置になる（10-17 のテスト `error_without_expression_position_uses_statement_position` は変わらない）。
  初期化式・引数を指すべき誤りは、フェーズ 3 で**明示的に** `span` を渡す。
- ⚠ **同じ文の未定義の名前の重複除去**（`already_reported_undefined`）は今、誤りの位置と `stmt_pos` が一致するかで見ている。
  位置が名前ごとに変わると `n = n + 1`（左辺は文の位置・右辺は `n` の位置）が 2 件になる。
  「今の文で出した（名前）」を位置と別に控える形に変える（2-3）。
- ⚠ `Span` は始点だけ（`file` / `line` / `col`）。**波線は 1 トークン**のまま（式全体に引くには終点が要る・本書の範囲外・§5）。
- ⚠ 錨の位置は `line == 0`（`Span::unknown()`・合成した式）なら無いものとして扱う（`own_span` と同じ）。
- ⚠⚠ **`alias` の展開で入った式は、alias を定義した位置を持つ**（1-1 で気付いた）。パーサは式位置の alias を
  保存した右辺の AST で置き換える（`parse_primary` の `Token::Ident`）ので、その中の識別子の `span` は定義側
  （別のファイルのこともある）。錨の**ファイルが `stmt_pos` のファイルと違えば錨を使わない**（2-2）。
  同じファイルの alias は定義の位置を指すことになるが、誤りの原因はそこに書いてあるので許す。
- ⚠ CLI の `Line:Col` も変わる。**エラー例題の出力が変わるのは意図した差分**（4-1 で全件を目で確かめる）。
  拡張と CLI は同じ型検査器なので `compare_wasm_frontend` は一致したままのはず。

## 3. タスク

### フェーズ 1: 識別子に位置を持たせる（AST）

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| **1-1** | `Expr::Ident` に `span: Span` を足す。パーサの `Token::Ident` の分岐（`parse_primary`）は名前のトークンの位置を入れる。`Any` / `Union` / `Option` / `Self` も同様。合成する識別子（`parser/classes.rs` の `self` / フィールド名、`exprs.rs:816` / `:1207` の呼び出し、`stmts/definitions.rs`、`interpreter/` の `built_in_types.rs` / `exec/definitions.rs` / `exec/modules.rs`、`python_converter/`、`template_subst.rs`、`meta_expand`）は**元になった位置**があればそれを、無ければ `Span::unknown()`。`Expr::Ident {` が出てくる 175 か所のうち、作る 23 か所以外は `..` で読んでいるだけで、`..` の無いパターンはコンパイラが拾う | なし | 中 |
| **1-2** | `size_of::<Expr>()` が増えていないか確かめる（`Ident` は一番大きい変種より小さいはず）。増えたら `ab_bench.ps1` で解釈側の速度を測る（`vm-pitfalls` §1。負の対照を同じセッションで） | 1-1 | 小 |
| **1-3** | `Expr::anchor_span()` を足す（§2.1 の表）。`Attr` は属性名の位置が要るので、`Attr` に `attr_span`（属性名のトークン）を足す。⚠ 今の `span`（`.`）を使っている箇所（実行時の誤りの表示・VM の行テーブル）を洗い出して、**変えない**ことを確かめてから足す | 1-1 | 小 |

ゲート: `test_build_gate` / `scan_examples` / `force_gate` / `compare_outputs -A`（**差分 0**・まだ型検査は位置を使わない）/
`compare_bytecode -A`（差分 0）/ `compare_wasm_frontend`

**結果（2026-11-11・フェーズ 1 完了）**

- 1-1: 合成する識別子は、パーサの `self` の補完・`interpreter/` の合成した `__init__`・`python_converter/`
  （Python の名前は位置を持たない）が `Span::unknown()`、f-string の脱糖の呼び出しは f-string の位置、
  `template_subst` と VM の `TemplateInstantiate` は元の識別子の位置を引き継ぐ。
- 1-2: `size_of::<Expr>()` は **112 のまま**（`Ident` の中身は 48 → 80 で、一番大きい変種より小さい）。速度は測っていない。
- 1-3: `Attr` に `attr_span` を足した（`.` の `span` は変えていない）。Python からの変換は `span` と同じ値を入れる。
  `Expr::anchor_span()` を足し、位置のテストを `frontend_tests/parser_tests.rs` の
  `expression_anchor_points_at_the_subject_token` に置いた。
- ゲート: `cargo test` 1303 件 ok・`scan_examples`（`bench_ab_native.ar` の TIMEOUT は基準の exe でも 120 秒を超える。
  本書の変更とは関係ない）・`force_gate`（`VmForceError` 0）・`compare_outputs -A` **643 / 643 一致**・
  `compare_bytecode -A` **662 / 662 一致**・`compare_wasm_frontend` 777 / 777 一致

### フェーズ 2: 型検査器が錨を使う

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| **2-1** | `TypeChecker` に `expr_anchor: Option<Span>` を足す。`infer` を「錨を控える → 今の `infer` の本体（`infer_inner` に改名） → 戻す」の薄い包みにする（錨の無い式は張り替えない）。`check_stmt` の入口で錨を空にして控え、出口で戻す（§2.2 の 1 つ目） | 1-3 | 小 |
| **2-2** | `report_error` / `report_warning` の既定の位置を **錨 → `stmt_pos`** にする（錨のファイルが `stmt_pos` と違えば `stmt_pos`・§2.2 の alias の注記）。doc（10-17 の注記）を書き換える | 2-1 | 小 |
| **2-3** | `already_reported_undefined` を位置の比較から「今の文で出した名前の集合」（`check_stmt` が張り替える）に変える。`n = n + 1`・`mh.enable(1)`（受け手と呼び出しの 2 経路）が 1 件のままであることをテストする | 2-2 | 小 |
| **2-4** | 代入の左辺の未定義（`stmt/check.rs` の 2 か所の `check_name_defined`）は文の位置のまま（左辺の名前＝文の先頭）で正しいことを確かめる。`AttrAssign` の対象の錨も確かめる | 2-2 | 小 |
| **2-5** | 位置のテストを足す（`frontend_tests/type_check_tests/`）: 未定義の名前（`while` の条件・二項演算の右辺・引数・入れ子の関数の中）、存在しない属性（属性名）、引数の数の誤り（呼ばれる側の名前）、演算子の型（演算子）、`if` 式の本体の中の誤り（本体の文を指す・外側の式を指さない）、合成した識別子（位置 0 → 文の位置に戻る） | 2-2 | 小 |

ゲート: `test_build_gate` / `scan_examples` / `force_gate` / `compare_python_impl` / `type_obligations` /
`compare_wasm_frontend` / `compare_outputs -A`（**エラー例題の `Line:Col` だけが変わる**こと。それ以外の差分は不具合）

### フェーズ 3: 推論が終わった後に出す誤りに位置を渡す

2-2 の後も文の位置に残る誤りを `span: None` の 91 か所から洗い出し、価値の高いものに明示の `span` を渡す。

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| **3-1** | 91 か所を「推論中に出す（錨で足りる）」「推論の後に出す（文の位置に残る）」「文が主語なので文の位置が正しい」に分けた表を本書の §4 に作る。例題とテストで実際の位置を見て分ける（推測で分けない） | 2-2 | 中 |
| **3-2** | 引数の型の不一致（`call_check.rs`）は**その引数**の錨を渡す | 3-1 | 小 |
| **3-3** | 宣言・代入・`return` の型の不一致（`stmt/check.rs`）は**右辺 / 戻り値の式**の錨を渡す（錨が無い式は文の位置のまま）。⚠ 10-17 のテスト `let z: str = s` の期待位置が (2, 1) から `s` の位置に変わる。期待値を書き換え、理由をテストの doc に書く | 3-1 | 小 |
| **3-4** | 3-1 の表の残り（デコレータ・protocol・resolve）を表に従って直す。直さないものは理由を表に書く | 3-1 | 小〜中 |

ゲート: フェーズ 2 と同じ

### フェーズ 4: 確認・文書・拡張

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| **4-1** | `compare_outputs -A <フェーズ1の直前のコミットの exe>` の差分を全件見て、位置が原因の式を指しているかを確かめる（表の `Line:Col` 以外が変わっていないこと） | 2-5, 3-4 | 小 |
| **4-2** | VS Code 拡張で `TeX_editor/main.ar` の 98 行目と `vscode-extension/test_fixtures/` を開き、波線が名前に引かれることを確かめる（`vscode-debug-runner` の `run_debug.js` で診断の範囲を出してもよい）。拡張の TS は変えない見込み。VSIX を作り直す（`make-vsix.ps1`） | 4-1 | 小 |
| **4-3** | 文書: `type-checking` スキル（誤りの位置の規則・錨）、`parser-internals`（`Ident` の `span`・`Attr` の `attr_span`）、`stale_doc_refs.ps1`（`infer` を改名したので）。ファイルを増やしたら `generate-codebase-map.ps1` | 4-1 | 小 |

## 4. フェーズ 3 の分類表

（3-1 で埋める）

## 5. 範囲外

- **波線を式全体に引く**（`f(x, y)` 全体など）。`Span` に終点が要り、字句・パーサ・拡張の全部に及ぶ。
  1 トークンに正しく引けるようになってから別に起票する。
- **実行時の誤り**（`NameError` など）の位置。VM の行テーブルが文の位置を持っており、本書では変えない。
- `EditorIndex::node_spans` を `Ident` の `span` で置き換える整理（拡張の hover / 定義へ移動の位置は今も正しい）。
