# 型束縛検査の実装計画（フィールド型チェックのバグ修正）

Arrow の型注釈が**呼び出し引数以外のどこでも強制されていない**問題の修正計画。

- 対象: `src/type_check/`（検査の追加）＋ 実行経路 2 本（`int`→`float` 昇格）＋ `src/interpreter/templates.rs`（実体化キャッシュ）＋ IC の多相化
- 作成: 2026-09-08
- 状態: **実装中**
- ブランチ: `bug-fix_field-type`

---

## 0. 問題の所在（調査結果）

`type_matches`（[type_utils.rs:26](../src/type_check/type_utils.rs#L26)）は Arrow で**唯一の型互換判定**だが、
呼び出し元は **`call_check.rs` だけ**である。したがって型注釈は
「シグネチャの判っている呼び先へ引数を渡すとき」にしか効かない。

### 実測した欠落（すべて実行して確認）

| 束縛点 | 静的 | 実行時 | 該当コード |
|---|---|---|---|
| 通常関数の引数 | ✅ | — | [call_check.rs:370](../src/type_check/call_check.rs#L370) |
| 関数型変数/引数の呼び出し | ✅ | — | [call_check.rs:516](../src/type_check/call_check.rs#L516) |
| `let` の protocol 注釈 | ✅ | — | [resolve.rs:197](../src/type_check/stmt/resolve.rs#L197) |
| `let`/`mut`/`const` の通常注釈 | ❌ | ❌ | [resolve.rs:191,211](../src/type_check/stmt/resolve.rs#L191) |
| 再代入 `x = v` | ❌ | ❌ | [check.rs:57-68](../src/type_check/stmt/check.rs#L57) |
| **フィールド書き込み** `o.f = v` | ❌ | 部分的※ | [check.rs:568](../src/type_check/stmt/check.rs#L568) |
| コンストラクタ引数 | ❌ | 部分的※ | [call_check.rs:370](../src/type_check/call_check.rs#L370) |
| メソッド引数（インスタンス/クラス/static） | ❌ | ❌ | [call_check.rs:264](../src/type_check/call_check.rs#L264) |
| `return` と宣言戻り値 | ❌ | ❌ | [check.rs:224](../src/type_check/stmt/check.rs#L224) |
| trait のフィールド型・メソッドシグネチャ適合 | ❌ | ❌ | protocol と違い trait 版の適合検査が無い |

※ **「部分的」はメモリレイアウトの副作用であって型検査ではない。**
`store_field`（[instance.rs:257](../src/interpreter/value/instance.rs#L257)）は
**raw レイアウトを持つクラスでだけ**値と slot 形式を突き合わせる。raw レイアウトは
「trait 継承なし・全フィールドが int/float 系プリミティブ」のときだけ付く
（[definitions.rs:784](../src/interpreter/exec/definitions.rs#L784)）ので、
`str` フィールドが 1 つ混ざる／trait を 1 つ実装するだけで**検査が消える**。

```arrow
class P:            class P:
    mut x: int          mut x: int
                        mut y: str
mut p = P(1)        mut p = P(1, "a")
p.x = "w"           p.x = "w"
# TypeError ✅      # "w" と出力される ❌
```

---

## 1. 決定事項

| # | 決定 | 日付 |
|---|---|---|
| D1 | 注釈の不一致は **warning ではなく完全にエラー**。例題側を修正対象とする | 2026-09-08 |
| D2 | `int` → `float` は**拡大を許し、束縛時に float へ昇格**する（案 B） | 2026-09-08 |
| D3 | テンプレート実体化は「**呼び出し時に具体型で探し、無ければ作る**」。同じ型引数なら同一クラス（同一 `class_id`） | 2026-09-08 |
| D4 | テンプレートの `is` 判定は**現仕様のまま**（`Box[int]` も `Box[str]` も `is Box` が True） | 2026-09-08 |
| D5 | メタ関数・展開パス・#89 は**本計画の対象外** | 2026-09-08 |

### D2 の根拠（現状の非対称・実測）

| 書き方 | 現在 | D2 適用後 |
|---|---|---|
| `let b: float = 3` | **3** | 3.0 |
| `c = 5`（`c: float`） | **5** | 5.0 |
| `k.x = 7`（フィールド `x: float`） | **7.0** | 7.0（変化なし） |

フィールドだけが昇格しているのは `store_field` の raw レイアウト経路に
`int → float フィールドの自動昇格` アームがあるため。`let`/代入に揃える。

---

## 2. フェーズと進捗

| # | 内容 | 状態 |
|---|---|---|
| 0-B | `Int → Float` 拡大の土台（`type_matches` ＋ 束縛時昇格） | **✅ 完了**（2026-09-08） |
| 0-2 | フィールド書き込みの型検査 | **✅ 完了**（2026-09-08） |
| 0-3 | `return` と宣言戻り値の照合 | 未着手 |
| 0-4 | メソッド引数の型検査 | 未着手 |
| 0-5 | コンストラクタ引数の型検査 | 未着手 |
| 0-1 | `let`/`mut`/`const` の注釈採用と照合 | 未着手 |
| T | テンプレート実体化キャッシュ | 未着手 |
| R3 | 属性アクセスの多相 IC | 未着手 |

---

## 3. 各フェーズの要点

### 0-B — `Int → Float` 拡大の土台 【✅ 完了 2026-09-08】

**実装したもの**

| 対象 | 内容 |
|---|---|
| `type_matches` | 最上位でのみ `(Int, Float)` を受理。本体は `type_matches_exact` へ分離 |
| `coerce_binding` | ツリーウォーク側の昇格（[exec/vars.rs](../src/interpreter/exec/vars.rs)） |
| `Op::CoerceFloat` | VM 側の昇格。`compile_expr` の直後・ストア op の直前に置く |
| `emit_coerce_binding` | 上記 op の発行（[compiler/emit.rs](../src/vm/compiler/emit.rs)） |

**⚠⚠ 実装中に判明した「拡大を伝播させてはいけない 2 箇所」**

1. **要素型（再帰の内側）** — 実行時の昇格はスカラの `float` 注釈だけが対象で
   `list[float] = [1, 2]` の要素は `Int` のまま。伝播させると**静的には通るのに
   実行時は Int** になる。⇒ `type_matches_exact` の内部再帰は自分自身を呼ぶ。
2. **`mut` パラメータ（write-back）** — C ABI の `double*` のように呼び先が呼び元の
   記憶域へ書き戻す引数では、拡大は値の変換ではなく**記憶域の型詐称**になる。
   ⇒ `param_type_matches` が `param_mutable` を見て `type_matches_exact` を使う。
   ⚠ **これは既存テスト `cpp_prim_ptr_int_arg_type_mismatch` が実際に検出した。**

**⚠ 無駄な op を出さない** — 初期化子が既に float リテラルなら `CoerceFloat` を出さない。
入れたままだと `bench_*` 例題の bytecode が変わり、後続フェーズの A/B 計測を濁す
（実測: 差分 9 件 → **1 件**に減り、bench 系は byte-identical に戻った）。

**副作用として VM の型特化が正しくなった** — `slot_prim` は `slot_type`（AST の注釈）で
int/float 特化 op を選ぶが、昇格前は `let b: float = 3` が「注釈は float・実値は Int」
だったため、選ばれた float 特化 op が毎回汎用へフォールバックしていた。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 65/65 identical・unexpected diff 0 |
| `compare_outputs.ps1 -A <base>` | **156/156 identical**（既存例題に該当箇所が無かった） |
| `compare_bytecode.ps1 -A <base>` | 差分 1 件（`other_typing.ar` の `let cast_f: float = temp=>float`。キャスト結果は既に float なので実行時 no-op） |

**追加した例題**: `examples/typing/int_float_widening.ar` ／ `int_float_widening_error.ar`

### 0-2 — フィールド書き込み 【✅ 完了 2026-09-08】

**実装したもの**

| 対象 | 内容 |
|---|---|
| `TypeErrorKind::FieldTypeMismatch` | 新設。`field 'x' of class 'P' is declared 'int' but got 'str'` |
| `check_attr_assign` | 第 3 引数 `check_field_type` を追加し、宣言型と値型を突き合わせる |
| `declared_field_type` | **2 つに分かれたフィールド表**を 1 つに見せる新しいヘルパ |

**⚠⚠ フィールドの宣言は 2 つのテーブルに分かれていた**

`infer_attr` が引く `class_field_details` は**そのクラス自身が宣言したフィールドしか
持たない**。trait 由来のフィールド（`class Wolf(Creature)` の `w.hp`）は
**別テーブル `trait_field_details`** に入っており、`Unresolved` に落ちて検査が素通りしていた。
⚠ `collect_class_field_details`（基底を辿る既存ヘルパ）も `class_field_details` しか
見ないので**これだけでは足りなかった**。⇒ `declared_field_type` で両方を引く。
own の宣言が trait の宣言を上書きする順序は、実行時の `build_field_index` と揃えてある。

⚠ **起票されたバグ報告が trait を挙げていたのはまさにこの形**で、最初の実装では
Section 3 だけ発火せず、エラー例題が漏れを検出した。

**⚠ 複合代入は対象外にした** — `o.f += v` で格納されるのは `v` ではなく `f <op> v` の
結果で、その型は二項演算の規則で決まる。`v` をそのままフィールド型と突き合わせると
嘘の判定になる。⇒ `AttrAssign` と `AttrCompoundAssign` のアームを分けた。

**⚠ オブジェクトを二度推論しない** — 期待型は `infer(target)` の**戻り値**を使い、
クラス名は推論を伴わないスコープ引き（`lookup`）で求める。二度推論すると
`infer_attr` がオブジェクトに対して出す診断（`OperationOnAny` など）が重複する。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 67/67 identical（`field_type_error` は known diff 登録） |
| `compare_outputs.ps1 -A <0-B>` | 159/160（差分は**新規エラー例題そのもの**。基準側に検査が無く通ってしまうため） |
| `compare_bytecode.ps1 -A <0-B>` | 178/179（同上） |

**追加した例題**: `examples/classes/field_type.ar` ／ `field_type_error.ar`

### 0-3 — `return`

⚠ `CheckState` に戻り値型のフィールドが無い（`current_fn_name` のみ・
[state.rs:17](../src/type_check/state.rs#L17)）。`enter_fn`/`exit_fn` と同じ
save/restore で追加する。**名前引きで代用しない**（オーバーロード・メソッド・
入れ子関数で一意に定まらない）。

### 0-4 — メソッド引数

`check_self_type_params` は個数・可変長・`SelfType` だけ見ている。
`check_call_args` にある `type_matches` ループを移植する。
`self`/`cls` の +1 は `effective_count` と同じ規約に揃える。

### 0-5 — コンストラクタ引数

`C(...)` は `check_call_args("C", …)` に入り `fn_sigs("C")` が `None` で即 return する。

⚠⚠ **自動コンストラクタの仮引数列の順序は実行時の `build_field_index` と同一規則**に
すること（[interpreter.rs:702](../src/interpreter.rs#L702)）:
**Step 1: 継承 trait のフィールドを基底順で先頭 → Step 2: own フィールドを宣言順**。
ずれると引数と型が 1 つずれて**別のフィールドを検査**する。

### 0-1 — `let`/`mut`/`const`

`resolve_declared_type` が `rhs_ty` を返して注釈を捨てている。照合を足すだけでなく
**変数を注釈の型で束縛する**よう変える。⚠⚠ 束縛型が変われば下流の推論 → 注釈テーブル →
`binop_kind` → VM の特化 op が変わり得る。`compare_bytecode.ps1` に差分が出る前提。

### T — テンプレート実体化キャッシュ

現状 `instantiate_template_class` は**呼び出しのたびに** `alloc_class_id()`
（[templates.rs:442](../src/interpreter/templates.rs#L442)）を叩き、
`template_class_cache` が**存在しない**（`template_fn_cache`/`template_gen_cache` はある）。

1. `(Rc::as_ptr(&tmpl) as usize, type_args)` キーのキャッシュを追加（既存 2 本と同形）
2. ⚠ **`subst_stmts` をキャッシュミス側へ移す**。現在は呼び出し元が先に置換してから
   渡しているので、キャッシュを足しても clone-walk が毎回走る
3. ⚠ `check_template_constraints` は**キャッシュ引きの前に毎回**走らせたまま（関数側と同じ）

### R3 — 多相 IC

計画（[BYTECODE_VM_PLAN.md:502](../implementation_logs/BYTECODE_VM_PLAN.md#L502)）は
「真に多相な protocol 引数は R3 の**多相 IC** に倒す」だが、実装はエントリ 1 個の
**単相 IC**（[ast.rs:165](../src/ast.rs#L165)）。

⚠ **Phase T の後に測ること。** 現状テンプレートクラスは実体化ごとに新 `class_id` を
取るため構造的に毎回ミスし、多相性ではなくキャッシュ欠落を測ってしまう。

1. **先に測る**（`--features prof`）: ミスを **cold miss**（キャッシュが空）と
   **polymorphic miss**（埋まっていて class_id 違い）に分ける
2. N-way 化。状態は mono → poly(N) → **megamorphic**（埋め直しの空回りを止める）
3. 消費者は**2 本**: `Expr::Attr.cache`（AST 埋め込み）と
   `chunk.attr_caches`（[chunk.rs:225](../src/vm/chunk.rs#L225)・側テーブル）
4. ⚠⚠ **`AttrCache::Clone` が空を返す不変条件を維持**（AST コピーの再解決が依存）

---

## 4. ゲート

| タイミング | スクリプト | 期待 |
|---|---|---|
| 各項ごと | `scan_examples.ps1` / `force_gate.ps1` / `compare_python_impl.ps1` | 緑 |
| 0-B・0-1 の後 | `compare_outputs.ps1 -A <base>` | **差分あり前提**（昇格由来のみ） |
| 0-B・0-1 の後 | `compare_bytecode.ps1 -A <base>` | 差分あり前提（特化 op の変化） |
| T の後 | `compare_outputs.ps1 -A <base>` | 差分 0 |
| R3 の後 | `compare_outputs.ps1` / `compare_bytecode.ps1` | **両方 0**（純粋な最適化） |
| R3 の後 | `ab_bench.ps1` | ⚠ 負の対照を**同一セッション**で取る |
| 全体 | `compare_wasm_frontend.ps1` ＋ wasm 再ビルド ＋ `make-vsix.ps1` | 緑 |

⚠ regulations.md より、**新エラーパターンごとに `_error` 例題が必要**。

### ベースライン

- 基準バイナリ: コミット `9986df1` からビルドしたもの
- `scan_examples.ps1` の既知の非緑: `bench_ab_native.ar`（TIMEOUT・ベンチのため想定内）

---

## 5. 実測メモ

| 測定 | 結果 | 日付 |
|---|---|---|
| 文レベルの注釈付き宣言（archived 除く） | 234 件 | 2026-09-08 |
| うちリテラルで判る型不一致 | **0 件** | 2026-09-08 |
| 実行時フィールド型エラーに依存する例題 | **0 件** | 2026-09-08 |
| 既存 `_error` 例題 | 46 件 | 2026-09-08 |
