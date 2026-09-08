# 型束縛検査の実装計画（フィールド型チェックのバグ修正）

Arrow の型注釈が**呼び出し引数以外のどこでも強制されていない**問題の修正計画。

- 対象: `src/type_check/`（検査の追加）＋ 実行経路 2 本（`int`→`float` 昇格）＋ `src/interpreter/templates.rs`（実体化キャッシュ）＋ IC の多相化
- 作成: 2026-09-08
- 状態: **Phase 0 / Phase T 完了**。残り: Phase R3（多相 IC）
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
| D6 | 0-B2 は **案 B-i**（戻り値・仮引数でも昇格する）を採用 | 2026-09-08 |

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
| 0-B2 | 戻り値・仮引数でも float へ昇格する（案 B-i） | **✅ 完了**（2026-09-08） |
| 0-3 | `return` と宣言戻り値の照合 | **✅ 完了**（2026-09-08） |
| 0-4 | メソッド引数の型検査 | **✅ 完了**（2026-09-08） |
| 0-5 | コンストラクタ引数の型検査 | **✅ 完了**（2026-09-08） |
| 0-1 | `let`/`mut`/`const` の注釈採用と照合 | **✅ 完了**（2026-09-08） |
| T | テンプレート実体化キャッシュ ＋ 型変数の取り違え修正 | **✅ 完了**（2026-09-08） |
| R3-計測 | 属性 IC の当たり外れを実測 | **✅ 完了**（2026-09-08） |
| R3-実装 | 多相 IC（N-way 化） | ⚠ **要判断**（計測の結果、消費者が居ない） |

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

### 0-B2 — 戻り値・仮引数でも float へ昇格する 【✅ 完了 2026-09-08・案 B-i】

**なぜ要ったか**: 0-B で拡大を**受理**した結果、昇格しない箇所が 2 つ残っていた。
0-B 以前は `f(3)`（`f(x: float)`）も `return n`（`-> float`）も**静的エラー**だったので
嘘をつく余地が無く、受理だけ広げたことで D2 が解消しようとした
「注釈が嘘をつく」状態が別の場所へ移っていた。**0-B が開けた穴**である。

| 束縛点 | 0-B 時点 | 0-B2 後 |
|---|---|---|
| `let`/`mut`/`const` | 3.0 | 3.0 |
| フィールド書き込み | 7.0 | 7.0 |
| **`return`（`-> float` に int）** | **3** | **3.0** |
| **仮引数（`x: float` に int）** | **4** | **4.0** |

**実装したもの**

| 対象 | 内容 |
|---|---|
| `exec_fn_evaled` | 薄いラッパにして `exec_fn_evaled_inner` を包み、**戻り値の昇格を 1 箇所へ集約** |
| `try_fast_bind` | 高速経路の仮引数昇格 |
| `bind_args` 後のループ | 一般経路の仮引数昇格 |
| `exec/mod.rs` | `mod vars` を `pub(crate)` へ（`coerce_binding` を関数側から使うため） |

⚠⚠ **戻り値の昇格は `exec_fn_evaled` の 1 箇所だけ**に置いた。ツリーウォークも VM も
関数呼び出しは最終的にここへ来る（VM の `Op::Call` → `call_value_evaled` → ここ）ので、
`Stmt::Return` と `Op::Return` の両方に手を入れるより**ずれようがない**。

⚠ **仮引数は束縛経路が 2 本ある**（`try_fast_bind` と `bind_args`）。片方だけ直すと
「単純シグネチャのときだけ昇格する」という読めない差になる。コードベースにも
「`*_evaled` 版とずれた実装を作らない（実バグ 4 回）」と記録がある。

**確認した経路**（すべて昇格する）: フリー関数の戻り値／仮引数・インスタンスメソッド・
`static` メソッド・テンプレート実体化（仮引数経由）・デフォルト値経由。

**⚠ 既知の残り**: `TemplateFnValue` は **`return_type` を保持していない**ので、
テンプレート関数の**戻り値**だけは昇格しない。仮引数は `subst_params` が具体型へ
置換済みなので効く（`tmpl[float](2, 3)` → `5.0` を実測）。
⇒ Phase T でテンプレート値を触るときに合わせて足す。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 67/67 identical（`return_type` は known diff 登録） |
| `compare_outputs.ps1 -A <0-B>` | 159/162（差分は新規例題 3 本のみ・**既存例題は不変**） |
| `compare_bytecode.ps1 -A <0-B>` | 179/181（差分は新規エラー例題 2 本。`return_type.ar` は byte-identical ＝ codegen 不変） |
| `compare_import_paths.ps1 -A <0-B>` | **13/13 identical**（関数呼び出しに触ったため実施） |
| `compare_wasm_frontend.ps1` | **232/232 agreed**・INVENTED 0・parse mismatch 0（wasm 再ビルド＋VSIX 生成済み） |

### 0-3 — `return` 【✅ 完了 2026-09-08】

**実装したもの**

| 対象 | 内容 |
|---|---|
| `CheckState::current_fn_return` | 新設。`enter_fn`/`exit_fn` で save/restore |
| `TypeErrorKind::ReturnTypeMismatch` | 新設。`'bad' is declared to return 'int' but returns 'str'` |
| `check_return_type` | `Stmt::Return(Some(e))` で照合 |
| `resolve_self_type` | `-> Self` を現在のクラスへ解決（`return Self(...)` を通すため） |

⚠ **名前引きで代用しない** — 戻り値型は注釈から取る。`current_fn_name` でレジストリを
引くとオーバーロード・メソッド・入れ子関数で一意に定まらない。
⚠ `enter_fn` で**張り替える**（継承しない）。入れ子 `fn` の `return` が外側の関数の
戻り値型と照合されると嘘の判定になる。

**⚠ 値なしの `return`（早期脱出）は照合しない。** 「戻り値を返し忘れている」という
別の検査であり、既存コードの早期 return を巻き込む。

**⚠⚠ 既存例題 3 本が実際に誤っていた** — `/` は int 同士でも **float** を返す
（`6 / 3` → `2.0`）のに `-> int` と宣言していた。D1 に従い例題側を修正:

| 例題 | 修正 |
|---|---|
| `exceptions/traceback_frame_names.ar` | `inner`/`middle`/`outer` を `-> float` へ |
| `exceptions/try_except.ar` | `risky_div` を `-> float` へ |
| `exceptions/runtime_error.ar` | `divide`/`compute` と `let result` を float へ |

⚠ `//` に変える案は採らなかった。ゼロ除算のメッセージが
`division by zero` → `integer division by zero` に変わり、例題の出力が動くため
（`runtime_error.ar` と `traceback_frame_names.ar` はその出力が主題）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 68/68 identical（`return_type_error` は known diff 登録） |
| `compare_outputs.ps1 -A <0-B>` | 160/162（差分は新規エラー例題 2 本） |

**追加した例題**: `examples/typing/return_type.ar` ／ `return_type_error.ar`

### 0-4 — メソッド引数 【✅ 完了 2026-09-08】

`check_self_type_params` は個数・可変長・`SelfType` しか見ておらず、`type_matches` に
掛けていたのは自由関数だけだった。⇒ **インスタンス／キーワード／`static`／
`class_method` のすべてで引数型が素通りしていた**（実測）。

**⚠ 添字の扱いが 2 つある**

- `self`/`cls` の分のずれ … `implicit = usize::from(!is_static)` を arity 検査と**同じ規約**で使う。
  ⚠ 既存の `SelfType` 検査は `arg_idx + 1` 決め打ちだったので、`static` メソッドでは
  **別の仮引数を見ていた**。あわせて `implicit` に揃えた。
- キーワード引数 … **名前で引き当てる**。位置で数えると別の仮引数を見る。

**⚠ `Self` 型パラメータは新しい検査から除外する。** 専用の `SelfTypeMismatch` が
見ているので、`type_matches` にも掛けると二重に鳴る。

**⚠ `mut` 引数は `param_type_matches` 経由**（0-B と同じ理由で拡大を許さない）。

エラーメッセージの `param_index` は**呼び出し側から見た位置**へ直して出す（`self` を数えない）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 67/67 identical |
| `compare_outputs.ps1 -A <0-B>` | 159/163（差分は新規例題 4 本のみ・**既存例題は不変**） |
| `compare_import_paths.ps1 -A <0-B>` | 13/13 identical |

**追加した例題**: `examples/classes/method_arg_type_error.ar`

### 0-5 — コンストラクタ引数 【✅ 完了 2026-09-08】

`C(...)` は `check_call_args("C", …)` に入るが `fn_sigs("C")` が `None` で即 return する
（クラス名は関数として登録されていない）ため、**まったく検査されていなかった**。

**⚠⚠ 自前で仮引数列を組み立てる必要は無かった。**
着手前の計画では「`build_field_index` と同じ順序で仮引数列を合成する」としていたが、
実際には**パーサが `__init__` を自動生成している**
（[parser/classes.rs](../src/parser/classes.rs) の `generate_auto_init_if_needed`。
並びは `fn __init__(mut self, trait のフィールド..., クラス自身のフィールド...)`）。
⇒ それは既に `class_method_sigs` に載っているので、**0-4 のメソッド呼び出し経路へ流すだけ**で
並びも自動的に実行時の `build_field_index` と一致する。合成した仮引数列と実行時の
スロット順がずれる risk（計画で最も警戒していた点）が構造的に消えた。

**⚠⚠ 外部言語のクラスは対象外にする**（#27-a の `arrow_class_names`）。
C# スタブは `__init__` を**引数 0 個**で持つが、実行時の生成は `__cs_bridge_path__` 経由の
ブリッジ側コンストラクタが行うのでスタブの `__init__` と実引数は対応しない。
除外しないと `cs_interop_test.ar` が
`'Calculator.__init__' takes 0 argument(s) but 1 were given` で落ちる（**実測**）。

**⚠ 0-2 で追加した例題が実際に間違っていた** — `field_type.ar` の
`Wolf("Grey", 40, 5)` は trait `Creature` が `hp: int` → `name: str` の宣言順なので
`Wolf(40, "Grey", 5)` が正しい。0-5 の検査が自分で書いた例題の誤りを検出した。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | `bench_ab_native.ar` のみ（下記） |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 67/67 identical |
| `compare_outputs.ps1 -A <0-B>` | 159/164（差分は新規例題 5 本のみ・**既存例題は不変**） |
| `compare_import_paths.ps1 -A <0-B>` | 13/13 identical |

**追加した例題**: `examples/classes/ctor_arg_type_error.ar`

### 0-1 — `let`/`mut`/`const` 【✅ 完了 2026-09-08】

`resolve_declared_type` は右辺の推論型をそのまま返しており、**注釈は照合も採用も
されていなかった**。`let a: int = "s"` が通るだけでなく変数が **`str` として束縛**
されていた。Protocol / Intersection / Result の 3 つだけが上で特別扱いされており、
それ以外の注釈は存在しないのと同じだった。

**照合を足すだけでなく、注釈の型で束縛する。** 注釈が右辺より情報量が多いケースが
実在するため（参考C が挙げていた 4 ケースのうち 3 つを例題で固定した）:

| ケース | 右辺の推論型 | 注釈 |
|---|---|---|
| 空コレクション | `List`（要素型なし） | `list[int]` |
| Option | `None` | `Option[int]` |
| トレイトへの拡大 | `Circle` | `Drawable` |

**⚠ 実測できた効果**: `raise_span_fields.ar` の `mut lines: list[int] = []` で
`lines[0] != lines[1]` が **`BIN NotEq` → `IBIN_SS NotEq`** に特化された
（出力は不変）。要素型が判るようになったため。予測どおりの向きの変化。

**⚠⚠ `Any` の意味が変わる（唯一の意図的な非互換）**

```arrow
let x: Any = 5
print(x + 1)   # 以前: 6 / 現在: StaticTypeError（明示ダウンキャストが要る）
```
注釈が採用されるようになったので **`Any` が本当に `Any` になった**。以前は注釈が
捨てられて `int` として束縛されていたため演算が通っていた。`Any` に明示ダウンキャストを
要求するのは言語の既存規則どおりなので、これは規則が**やっと効くようになった**もの。
⚠ 例題は 1 本も壊れなかった（`other_typing.ar` は既に正しくダウンキャストしていた）。

**⚠ 解釈できない注釈文字列では何もしない** — `from_ann` は失敗を `None` で返すので、
そこで `rhs_ty` に倒さないと「型が無い」が「型が違う」に化ける。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | **既存例題は 1 本も壊れなかった** |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 67/67 identical |
| `compare_outputs.ps1 -A <0-B>` | 159/164（差分は新規例題のみ・**既存例題は不変**） |
| `compare_bytecode.ps1 -A <0-B>` | 178/183（新規例題 4 本＋`raise_span_fields.ar` の**特化 1 件**） |
| `compare_wasm_frontend.ps1` | **236/236 agreed**・INVENTED 0（wasm 再ビルド＋VSIX 生成済み） |

**追加した例題**: `examples/typing/var_annotation.ar` ／ `var_annotation_error.ar`

### T — テンプレート実体化キャッシュ 【✅ 完了 2026-09-08】

**実装したもの**

| 対象 | 内容 |
|---|---|
| `template_class_cache` | `(テンプレートの Rc アドレス, 型引数)` キー。既存 2 本と同形 |
| `build_template_class` | `instantiate_template_class` から**構築部だけ**を分離（`Rc<ClassValue>` を返す） |
| `TemplateFnValue.return_type` | 新設。実体化時に `subst_type` で具体型へ置換して `FnValue` へ載せる（0-B2 の残件） |

⚠ 置換（`subst_stmts` の clone-walk）も**キャッシュミス側**へ移した。呼び出し元で先に
置換していたので、キャッシュを足すだけでは毎回 AST を複製したままになる（関数側と同じ形）。
⚠ 制約検証（`check_template_constraints`）は**キャッシュ引きの前に毎回**のまま。
⚠ 引数の評価はクラスが決まった後（既存の順序を保つ）。

**⚠⚠ 意図した意味論の変化: クラスレベル初期化子の評価回数**

`const` / `static mut` / フィールド既定値の初期化子は `build_template_class` の中で
`self.eval` される。メモ化により、これが**型引数の組ごとに 1 回**になった（実測）:

| | 基準 | Phase T 後 |
|---|---|---|
| `Box[int]` ×2 ＋ `Box[str]` ×1 | 3 回評価 | **2 回評価** |

通常のクラス定義（`exec_class_def` は 1 回だけ走る）と同じ回数であり、
以前の「実体化のたびに評価し直す」方が非対称だった。

---

### T-fix — 型変数を具象型と取り違えていた（Phase 0 の偽陽性）

**Phase T の検証中に、Phase 0 で入れた検査の偽陽性が 2 件見つかった。**

```arrow
class Box[T]:
    mut v: T
    fn reset(mut self) -> None:
        self.v = 0          # ❌ field 'v' of class 'Box' is declared 'T' but got 'int'

fn conv[T](let n: int) -> T:
    return n                # ❌ 'conv' is declared to return 'T' but returns 'int'
```

**原因**: `InferredType::from_ann` は**大文字始まりの未知の識別子をクラス名として扱う**
（[types.rs:281](../src/type_check/types.rs#L281) の `NamedInstance(other)`）。
型変数 `T` と実在のクラス名が型の上で区別できず、具体型と突き合わせると偽エラーになる。

⚠ **例題が 1 本も検出しなかった** — `template_specialization.ar` は `-> T` に対して
`a + b`（型変数同士なので `Unresolved`）を返しており、`type_matches` が
`Unresolved` を無条件に通すため素通りしていた。**具体型を返す形の例題が無かった。**

**修正**: `CheckState` に**見えている型変数のスタック**を持たせ、型変数を含む注釈は
照合しない（`mentions_type_param`）。

| 積む場所 | 理由 |
|---|---|
| `Stmt::ClassDef` | クラスの型変数はメソッド本体からも見える（`mut v: T` を `self.v` で書く） |
| `check_fn_def` | 関数自身の型変数（`fn f[T]`） |

適用先は 0-2（フィールド代入）・0-3（`return`）・0-1（`let` の注釈）の 3 箇所。
⚠ 検査したい形は**実体化後の AST**（型変数が具体型へ置換済み）で見られる。

⚠ 副産物: `TemplateFnValue.return_type` を足したことで、実体化後の戻り値注釈が
具体型になり **テンプレート関数の戻り値でも `int`→`float` の昇格が効く**ようになった
（`conv[float](3)` → `3.0`。修正前は偽エラーで実行すらできなかった）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 68/68 identical |
| `compare_outputs.ps1 -A <Phase 0>` | **166/166 identical**（例題追加前）→ 166/168（追加した 2 本のみ） |
| `compare_bytecode.ps1 -A <Phase 0>` | **185/185 identical** |
| `compare_import_paths.ps1 -A <Phase 0>` | 13/13 identical |
| `compare_wasm_frontend.ps1` | **238/238 agreed**・INVENTED 0（wasm 再ビルド＋VSIX 生成済み） |

**追加した例題**: `examples/typing/template_type_param.ar` ／ `template_type_param_error.ar`

### R3 — 多相 IC 【計測完了・実装は要判断】

計画（[BYTECODE_VM_PLAN.md:502](../implementation_logs/BYTECODE_VM_PLAN.md#L502)）は
「真に多相な protocol 引数は R3 の**多相 IC** に倒す」だが、実装はエントリ 1 個の
**単相 IC**（[ast.rs](../src/ast.rs) の `AttrCache`）。その差を埋めるべきかを実測した。

#### 計測基盤（実装済み）

| 対象 | 内容 |
|---|---|
| `prof::IC_HIT` / `IC_COLD` / `IC_POLY` | ミスを **cold**（空＝初回）と **poly**（class_id 違い＝多相）に分ける |
| `AttrCache::is_empty` | cold / poly の判別に要る（prof ビルド専用の読み手） |
| `prof::note_ic` | `Op::GetAttr` / `Op::GetAttrLocal` の probe 地点で計上 |
| [`scripts/prof_attr_ic.ps1`](../scripts/prof_attr_ic.ps1) | 全例題を回して集計 |

⚠ **分けずに「ミス率」だけ見ても判断できない。** 多相化（N-way）で救えるのは poly だけ。
⚠ **Phase T の後に測る必要があった。** テンプレートクラスが実体化ごとに新 class_id を
取っていた間は、多相性ではなくキャッシュ欠落を測ってしまう。

#### 実測結果（2026-09-08・例題 224 本）

```
probes       : 77,400,461
  hit        :   77,400,077  (99.9995%)
  cold miss  :          375  ( 0.0005%)   <- 多相化では救えない
  poly miss  :            9  ( 0.0000%)   <- 多相化で救えるのはここだけ
```

**多相ミスは全例題あわせて 9 件**（`py_inherit.ar` 4・`polymorphism.ar` 3・
`enum_in_function.ar` 2）。属性アクセスが最も重い `bench_method_hot.ar` は
**2400 万 probe すべてヒット**（poly 0）。

#### 合成ベンチによる「多相が起きたときの」コスト

例題が多相ワークロードを含んでいない可能性があるので、意図的に多相な地点を作って測った
（4 クラスを交替させて属性を読む / 同じ probe 数の単相版と比較）:

| | probe | hit | poly miss | 実行時間 |
|---|---|---|---|---|
| 単相（1 クラス） | 3,200,000 | 100% | 0 | 383 ms / 393 ms |
| 多相（4 クラス交替） | 3,200,000 | **0%** | **100%** | 709 ms / 707 ms |

⇒ 多相地点は**約 1.8x 遅い**。病理そのものは実在する。

#### 判断材料

| | |
|---|---|
| 実コードでの発生 | **9 / 77,400,461 = 0.0000%**（消費者が居ない） |
| 起きたときの損失 | 約 1.8x（合成ベンチ・上限） |
| 実装コスト | `AttrCache` は**全 `Expr::Attr` ノードに埋め込まれている**ので 8 バイト → 32 バイト超。消費者 2 本（AST 埋め込みと `chunk.attr_caches`）。`Clone` が空を返す不変条件の維持 |

プロジェクトの明文化された基準は「**消費者が居るか**」（#11 R2-c / #14 / #15b を保留にしたのと
同じ基準）。現時点では**居ない**。

**選択肢**

| 案 | 内容 |
|---|---|
| R3-a | **実装しない**。数値を記録して閉じ、消費者が現れたら再開する |
| R3-b | **2-way だけ入れる**。ヒット経路のコストは不変（エントリ 0 を先に見る）で、`AttrCache` は 8→16 バイト。「2 クラスが交替する」形だけ救う保険 |
| R3-c | 計画どおり N-way ＋ megamorphic 状態 |

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
- `scan_examples.ps1` の既知の非緑: `bench_ab_native.ar`。実行が長いので TIMEOUT になるか、
  完走した場合は `.arc` が stale という警告で FAIL する。⚠ **どちらも本計画とは無関係**
  （基準バイナリでも同じ警告が出ることを実測で確認済み）。

---

## 5. 実測メモ

| 測定 | 結果 | 日付 |
|---|---|---|
| 文レベルの注釈付き宣言（archived 除く） | 234 件 | 2026-09-08 |
| うちリテラルで判る型不一致 | **0 件** | 2026-09-08 |
| 実行時フィールド型エラーに依存する例題 | **0 件** | 2026-09-08 |
| 既存 `_error` 例題 | 46 件 | 2026-09-08 |
