# impl_python 同期計画（バイトコード化以降）

調査日: 2026-08-27 / 対象: `0a0097e`(phase 5C, 2026-07-21) .. `35b5aac`(HEAD)

---

## 0. 先に結論 — 起点の前提を 3 点訂正する

### 訂正 1: impl_python の真の同期点は `0a0097e` ではなく `85e852f`

`0a0097e..HEAD` で `impl_python/` に入った変更は **2 ファイル・実質 1 件だけ**で、
内容は `.tl`→`.ar` の別名追加（`"tl-auto"` → `"ar-auto"`）である。意味論の同期は 1 度も走っていない。

```
git diff --name-status 0a0097e..HEAD -- impl_python/
M  impl_python/__main__.py        # git SHA 行のみ
M  impl_python/parser/imports.py  # tl-auto -> ar-auto
```

意味論を伴う最後の同期コミットは **`85e852f "Apply to Python implementations"`（`0a0097e` より前）**。
つまり **ドリフトはバイトコード化以前から始まっている**ので、`0a0097e` を起点に据えると取りこぼす。

### 訂正 2: `__main__.py` の `git SHA:` は現状を表していない

```
impl_python/__main__.py:1  # git SHA: 3361350159cad1a7fa5cd30901ff27a8f46bc688
```

`3361350` は 3 コミット前（"ReadMe update"）だが、上のとおり同期の実体は無い。
**SHA 行だけが更新されている**＝ `.claude/rules/regulations.md` の
「Python 実装を更新したら SHA も更新」というルールが逆流し、
「同期していないのに SHA が進む」形で形骸化している。§5 で対処する。

### 訂正 3（最重要）: バイトコード VM は **移植対象ではない**

`0a0097e..HEAD` の `src/` 変更 131 ファイルのうち、Python に写すべきものは**ほぼ無い**。

- バイトコード VM は **Rust の実行速度**の話であって、言語の意味論ではない。
- `impl_python` の役割は `scripts/compare_python_impl.ps1` が明記するとおり
  **「Rust とは独立に書かれた参照実装」＝意味論の網**。
- ここに VM を作ると **両実装が同じ間違いをする**構造になり、網の独立性が失われる。
  これは `compare_vm_modes.ps1` が `#27` の `for-target-shadow` を取り逃した失敗そのもの。

> **方針: impl_python はツリーウォークのまま据え置く。同期するのは「意味論」だけ。**

---

## 1. 現状の実測 — ゲートはグリーン

```
./scripts/compare_python_impl.ps1 -TimeoutSec 25
checked: 54   identical: 54   unexpected diff: 0   timeout: 0
known diff (skipped): 45   stale entries: 0
PYTHON-DIFF: clean
```

**未知のズレは 0 件。** 意味論の差分は `compare_python_impl.ps1` の `$knownDiff` **45 件に
すべて捕捉されている**。したがって本計画の実体は「131 ファイルを移植する」ことではなく、
**「45 件の knownDiff を減らす」**ことである。

### ゲートの被覆範囲（＝この 45 件が全数かどうか）

| ディレクトリ | .ar 数 | ゲート対象 | 備考 |
|---|---|---|---|
| basics / collections / classes / typing / exceptions / async / apps / interop / repl | 111 | ✅ | 54 一致 + 45 known + 約 12 skip |
| `bench/` | 24 | ❌ | 速度計測用。意味論の例題ではない |
| `debugger/` | 5 | ❌ | `.in`/`.out` golden・対話。`debug_session.ps1` の担当 |
| `archived/` | 72 | ❌ | archive |

⚠ `bench/` と `debugger/` が対象外なのは妥当だが、**明示的な根拠がスクリプトに書かれていない**
（`$categoryDirs` に無いだけ）。§5 でコメントを足す。

---

## 2. 変更ファイル一覧（`0a0097e..HEAD`, `src/` 131 ファイル）

### Class A — 移植しない（VM / 解決層 / 計測 / 内部リファクタ）: 約 105 ファイル

| 群 | 数 | ファイル | 移植しない理由 |
|---|---|---|---|
| バイトコード VM | 18 | `src/vm/**`（`chunk.rs` `op.rs` `run.rs` `peephole.rs` `disasm.rs` `op_prof.rs` `compiler/*` 12 本） | Rust の実行形。py はツリーウォーク据え置き |
| AST 解決層 (Phase R) | 3 | `interpreter/resolver.rs` `interpreter/vm_toplevel.rs` `type_check/annotations.rs` | slot 採番・注釈テーブル＝速度のための前処理 |
| 走査骨格の 1 本化 | 3 | `expr_walk.rs` `stmt_walk.rs` `decl_names.rs` | Rust の walker ドリフト対策（#59/#81/#84）。py に同じ問題は無い |
| 計測 / 診断 | 3 | `prof.rs` `syntax_cov.rs` `interpreter/tw_stats.rs` | 計測専用 |
| Rust 単体テスト | 19 | `interpreter/tests/**` `frontend_tests/**` | Rust 側のテスト |
| ネイティブ codegen | 7 | `partial_compiler/**` | LLVM。py 版は別物 |
| 削除 | 1 | `interpreter/eval/control_expr.rs` (D) | VM 化に伴い解体・**機能の削除ではない** |
| 純リファクタ | 約 51 | `interpreter/exec/**` `eval/**` `classes/**` `value/**` `ops/**` `type_check/**` `parser/imports/**` ほか | #58〜#88 の大半。**外から見える挙動は不変**（`compare_outputs` / `compare_bytecode` で byte-identical を確認済み） |

> ⚠ Class A の「純リファクタ」を移植しないと言えるのは、**ゲートが緑だから**であって
> 「Rust が変えていないから」ではない。根拠は §1 の実測。

### Class B — 意味論を含み移植を検討する: 下記のみ

| ファイル | タスク | 中身 | py への影響 |
|---|---|---|---|
| `interpreter/ops/operators.rs` | **#18** | `<=` `>=` の `(Int,Float)`/`(Float,Int)`、`<` `>` `<=` `>=` の `(Str,Str)` を追加（型検査の `ordered_comparable` に実行時を合わせた） | **不要**。Python が host 言語として同じ比較を持つので py は偶然すでに一致（`comparison_matrix.ar` はゲート緑） |
| `ar_config.rs` (新規) + `parser/imports/**` | **#72 #73 #74** | `ar_config.json` の読み取りを 1 本化。**#74 は意図的な挙動変更** — `python.search_paths` の探索を祖先ウォークへ統一 | **要移植**（S5）。py は `__main__.py` に独自実装を持つ |
| `interpreter/ffi_boundary.rs` (新規) | #16(b)(ii) | 外部（py/js）関数の戻り値をスタブ宣言型と突き合わせる境界検査 | 要移植（P3・大きい） |
| `interpreter/exceptions.rs` | **#77** | raise した例外へ `file`/`line`/`col`/`code_context` を焼き込む | 要移植（P1） |
| `lexer/math.rs` | #78 | `render_math_str` のリファクタ（意味論不変） | py は `m"..."` **自体が未実装**（P2）。移植は #78 後の形を写せばよい |
| `interpreter/classes/string_methods.rs` | #65 | `ljust` が文字列内の空白まで置換／`rjust`・`center` がバイト長判定でマルチバイト非対応、と判明 | py はこの 3 メソッドを持たないので**現時点では無風**。実装するときに #65 の指摘を織り込む |

### Class C — ツーリング（移植ではなく py 側の同等物を検討）

`scripts/compare_python_impl.ps1`（#31 で新設・本計画の土台）、`syntax_cov.ps1`、`stale_doc_refs.ps1`。

---

## 3. 実バックログ = `$knownDiff` 45 件（優先度順）

優先度の基準は **「py が黙って違う答えを出すか」**。参照実装の価値は正しさなので、
未実装（＝落ちる・目立つ）より **誤答（＝気づかない）** が上。

### P0 — 誤答: py が例外も出さず違う結果を返す（4 件）★最優先

| 例題 | 実測した差 |
|---|---|
| `mut_to_let_copy` | `mut`→`let` でコピーされない。`top a=[1,2,3,4] b=[1,2,3]`（Rust）に対し py は `b=[1,2,3,4]`。dict も同様（`e={'k':1}` vs `{'k':99}`）。**#15e で Rust を修正済み** |
| `copy_method` | 同上。`read: Named(original_copy, 6)`（Rust）が py では `Named(original, 5)` |
| `block_return_typecheck_error` | `block_return` の実行時型検査が無い。Rust は `TypeError: block_return value has type 'str', but 'int' was expected`、py は **`hello` を返して正常終了** |
| `variable` | `freeze` 後のフィールド不変化。py は `Config locked: ...` と `caught after freeze: cannot assign to immutable field 'debug'` の **2 行を出さない** |

> ⚠ **`variable` の `$knownDiff` 理由は誤り。**「py 古い: static mut の扱い」と書かれているが、
> 実測すると `static mut` セクション（`1`/`2`/`3`）は**一致**しており、
> 実際にズレるのは **`freeze`（インスタンスのフィールド凍結）**。§5 で理由を訂正する。

**進め方**: P0 は 4 件とも「Rust に既にある正解」が判っているので、
`impl_python/interpreter/` の代入・束縛経路（`env.py` / `interpreter.py`）と
`type_check/stmt.py` に絞って直せる。**1 件ずつ直して毎回ゲートを回す**
（直ると `$knownDiff` から STALE として報告されるので、そこで行を消す）。

### P1 — 未実装だが意味論の中核（9 件）

| 例題 | 内容 |
|---|---|
| `freeze_collection` | `freeze` の伝播（P0 の `variable` と同根。**まとめて着手すべき**） |
| `fixed_list` / `fixed_list_error` | `fixed_list` 未実装 |
| `attr_access_paths` | 属性アクセス経路の一部 |
| `enum_in_function_error` | enum バリアント値の int 検査（#68） |
| `raise_span_fields` | 例外への `file`/`line`/`col`/`code_context` 焼き込み（#77） |
| `builtin_shadow` | 組み込みのシャドウ規則 |
| `import_py_search_path` / `import_py_int_search_path` | `import[py]`/`import[py-int]` の束縛自体が未対応（#61/#69/#74） |

### P2 — 組み込み・機能の穴（7 件）

`parse_ar` / `parse_ar_error`（#56 の `parse_ar` 組み込み）、`math_string`（`m"..."` 自体が未実装・#78）、
`built_in`（`id`/`repr` ほか）、`collection` / `collection_error`、`complex_error`、
`unregistered_type_call_error`（`tuple` が Python の組み込みなので `NameError` にならない）。

### P3 — 出力形式・FFI・非同期（25 件）— **やらない判断も妥当**

- **repr の差**(3): `block_return_typecheck` `other_typing` `result`
- **エラー出力形式**(7): `runtime_error` `traceback_frame_names` `try_except` `try_except_errors` `global_assign_from_fn_error` `mustbe_error` `polymorphism_error`
- **AsyncManager 未実装**(3): `async_string_share` `async_closure_share` `async_vm_body`
- **FFI / 外部ブリッジ**(9): `ffi_boundary_*`(3) `cs_interop_test` `event_cs_fire` `event_external_handler` `import_py_json` `bench_ab_cdll` ほか
- **バイナリを UTF-8 で読んで落ちる**(3): `stale_arc_check` `swd_nested_runner` `typed_abi`
  → ⚠ これは**機能ではなく数行のバグ**（`read_text(encoding="utf-8")` → `read_bytes()`）。
  **P3 の中で唯一、費用対効果が高いので先に潰してよい。**

> ゲートは **stdout だけ**を比べる設計なので、「エラー出力形式」群は
> **合わせにいかず `$knownDiff` に残すのが正しい**（Rust は色付きの表・py は 1 行）。

---

## 4. 推奨する実施順

| 段 | 内容 | 完了条件 |
|---|---|---|
| **S0** | `__main__.py` の SHA 行を **`85e852f` へ差し戻す**（現状を正しく表す） | §5 のルール変更とセット |
| **S1** | `$knownDiff` の**理由の訂正**（`variable`）＋ ゲートの被覆範囲コメント追記 | `compare_python_impl.ps1` が現実と一致 |
| **S2** | **P0 の 4 件**（コピー意味論 2・`block_return` 実行時検査 1・`freeze` 1） | 該当 4 行が STALE で落ちる → 削除 |
| **S3** | P3 のバイナリ読み 3 件（`read_bytes()` 化） | 3 行削除 |
| **S4** | P1 の `freeze_collection`（S2 の `freeze` と同根）→ `fixed_list` → `raise_span_fields` | 都度 STALE 確認 |
| **S5** | P1 の `import[py]` 束縛 ＋ `ar_config` 祖先ウォーク（#74） | `import_py_*_search_path` 2 件 |
| **S6** | P2（`parse_ar` / `m"..."` / 組み込みの穴） | — |
| **S7** | P3 の残り（**着手しない判断を `$knownDiff` に理由つきで固定する**のが正解） | — |

⚠ **1 タスク = 1 コミット、毎回 `compare_python_impl.ps1` を回す。**
まとめて直すと、どの変更がどの行を緑にしたか判らなくなる。

---

## 5. ルールの手当て（再発防止）

1. **SHA 行の意味を決める。** 現状「同期していないのに進む」ので無意味になっている。
   `.claude/rules/regulations.md` の
   「When Python implementation is updated, also update the git SHA」を
   **「`impl_python/` の*意味論*を同期したときだけ SHA を更新する。
   `.tl`→`.ar` のような機械的改名では更新しない」**へ明確化する。
2. **`variable` の `$knownDiff` 理由を訂正**（static mut → freeze）。
   ⚠ 理由が間違った knownDiff は、**間違ったまま永久に緑**になるので網を殺す。
   他の 44 件も、着手時に必ず実測で理由を確かめてから直すこと。
3. **`$categoryDirs` に `bench`/`debugger`/`archived` を除外する理由をコメントで書く。**
4. **`compare_python_impl.ps1` を「毎回」ゲートに残す。** CLAUDE.md の表のとおり、
   これが **意味論を守る唯一の網**。

---

## 6. 実務上の落とし穴

- **CRLF**: Rust 側は CRLF、py 側は LF で出力する。手で `diff` を取ると**全行が差分に見える**。
  ゲートの `Normalize()` は正規化しているので、手検査でも `sed 's/\r$//'` を通すこと。
- **`head`/`tail` での突き合わせ禁止**: 出力の反対側を切るため、無関係な箇所が差分に見える。
  必ず全文を取ってから `diff`。
- **STALE 報告を無視しない**: 直したのに `$knownDiff` に残っていると網が緩む。
  スクリプトは STALE として黄色で報告するので、都度行を消す。
