# 型束縛検査の実装計画（フィールド型チェックのバグ修正）

Arrow の型注釈が**呼び出し引数以外のどこでも強制されていない**問題の修正計画。

- 対象: `src/type_check/`（検査の追加）＋ 実行経路 2 本（`int`→`float` 昇格）＋ `src/interpreter/templates.rs`（実体化キャッシュ）＋ 属性 IC の実測
- 作成: 2026-09-08
- 状態: **全フェーズ完了**（Phase 0 / Phase T / R3）
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
| **テンプレート実体化呼び出し** `Box[int]("s")` / `add[int]("x","y")` | ❌ | ❌ | `func_name` が `Expr::Ident`/`Expr::Attr` しか拾わず `_ => None` に落ちる |
| **仮引数の既定値** `fn f(let n: int = "s")` | ❌ | ❌ | `check_fn_def` は `param.default` に触れず、推論すらしない |
| **`const` / `static mut` の既定値** | ❌ | ❌ | `Stmt::Field` が `infer` の結果を捨てる |
| trait のフィールド型・メソッドシグネチャ適合 | ❌ | ❌ | protocol と違い trait 版の適合検査が無い |

⚠⚠ **テンプレート実体化の行は当初の調査で見つけていたのに、この計画表から落としていた。**
そのためフェーズが割り当てられず、Phase 0 完了後も**静的にも実行時にも素通り**していた
（ユーザ報告で発覚・0-6 で修正）。

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
| D7 | **型注釈は補助的なもの。注釈が無い束縛にも推論型で同じ厳格さを適用する**（緩めない） | 2026-09-09 |
| D8 | **trait メソッドはデフォルト実装を持てる**。仮想メソッド（`...`）は **trait 専用**で、クラスに置くのはエラー。未実装もエラー。**仮想でも型注釈は必須** | 2026-09-10 |

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
| **0-6** | **テンプレート実体化呼び出しの引数型検査**（計画表からの落とし分） | **✅ 完了**（2026-09-09） |
| **0-7** | **protocol 適合検査を全束縛点へ ＋ 再代入の型検査** | **✅ 完了**（2026-09-09） |
| **0-8** | **trait 適合検査（同名・中身違いを弾く）** | **✅ 完了**（2026-09-10） |
| **0-9** | **trait フィールドの再宣言を禁止**（既存バグ A の修正） | **✅ 完了**（2026-09-10） |
| **0-10** | **trait のデフォルト実装**（既存バグ B の修正）＋ **クラスの仮想メソッド禁止** | **✅ 完了**（2026-09-10） |
| **0-11** | **テンプレート実体化検査の穴 2 件**（`__init__` オーバーロード・型引数省略） | **✅ 完了**（2026-09-11） |
| **0-12** | **既定値の型検査**（仮引数・`const`・`static mut`） | **✅ 完了**（2026-09-11） |
| **A-2** | **具体化済みジェネリクスを型注釈として扱う**（テンプレートの偽陽性修正） | **✅ 完了**（2026-09-11） |
| **A-3** | **boxed レイアウトのフィールドを実行時に検査する**（黙って不整合な型が入る穴） | **✅ 完了**（2026-09-11） |
| **A-4** | **クラス型・trait 型・protocol 型のフィールドも検査する**（`TypeTag::Other` の素通り） | **✅ 完了**（2026-09-11） |
| R3 | フィールドのオフセット化 | **✅ 完了として閉じた**（2026-09-08。原計画に未実行分は無かった） |

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

### 0-6 — テンプレート実体化呼び出しの引数型 【✅ 完了 2026-09-09】

⚠⚠ **計画表から落としていた行。** 当初の調査では見つけていたが §0 の表に書かなかったため
フェーズが付かず、Phase 0 を完了と報告した後も素通りしていた（ユーザ報告で発覚）。

**原因**: [call_check.rs](../src/type_check/call_check.rs) の `func_name` は呼び先の名前を
`Expr::Ident` / `Expr::Attr` からしか拾わず、`Box[int](…)` の形
（`Expr::TemplateInstantiate`）は `_ => None` に落ちる。⇒ `check_call_args`（自由関数）も
0-5 のコンストラクタ検査も**走らない**。`infer` 側も `type_args` を捨てて `Unresolved` を返す。

**素通りしていた形（実測）**

| | 修正前 |
|---|---|
| `Box[int]("wrong")` | 静的 ❌ / 実行時 ❌ |
| `add[int]("x", "y")` | 静的 ❌ / 実行時 ❌ |
| `Mixed[int](1, 999)`（`tag: str`） | 静的 ❌ / 実行時 ❌ |

**実装したもの**

| 対象 | 内容 |
|---|---|
| `TypeRegistry::template_params` | 型変数名を**宣言順**で保持（クラス・関数）。置換表を作るのに名前と並びが要る |
| `subst_type_params` | `T` → `int`、`list[T]` → `list[int]` の再帰置換 |
| `check_template_call_args` | 置換後のシグネチャと実引数を突き合わせる |
| `infer_call_inner` | `Expr::TemplateInstantiate` の分岐を追加 |

⚠ **検査するのは型引数で置換した型。** 置換しないとシグネチャ側が `T`
（`NamedInstance("T")`）のままで、具体型の実引数と必ず食い違う。

⚠ **戻り値型は従来どおり `Unresolved` のまま**にした。`NamedInstance(base)` に変えると
フィールド型が `T`（呼び出し点では未知のクラス名）として下流へ流れ、`a.v = "s"` などが
偽エラーになる。型変数の実体化を型検査側でも行うのは別タスク（§7）。

⚠ **個数の不一致は報告しない。** 型引数・実引数どちらも実行時に `TemplateError` /
`TypeError` で捕まるうえ、個数が合っていないと置換表が作れず対応付け自体が嘘になる。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 68/68 identical |
| `compare_outputs.ps1 -A <R3>` | **168/168 identical** |
| `compare_bytecode.ps1 -A <R3>` | **187/187 identical** |
| `compare_wasm_frontend.ps1` | **239/239 agreed**・INVENTED 0 |

**追加した例題**: `examples/typing/template_instantiate_error.ar`（正常系は
`template_type_param.ar` に追記）

### 0-7 — protocol 適合を全束縛点へ ＋ 再代入の型検査 【✅ 完了 2026-09-09】

**⚠⚠ protocol は構造的適合**（`class C(Pr)` のように基底へ書かない）。そのため
`type_matches` は protocol 期待型に対して**任意の `NamedInstance` を通す**作りで、
適合の判定は `check_protocol_conformance` が別に担っている。
⇒ その呼び出しが無い束縛点は**不適合が素通り**する。

しかも protocol 名が `Protocol(...)` へ解決されるのは `fn_sigs`（自由関数）だけで、
`class_method_sigs`（メソッド・自動生成 `__init__`）・戻り値注釈・フィールド型は
`NamedInstance("Pr")` のままだった。⇒ そこでは `type_matches` が**継承関係として**
照合し（`class_implements_trait`）、protocol は基底に現れないので**必ず false**。

**つまり同じ機能が束縛点によって逆方向に壊れていた（実測）**

| 束縛点 | 適合するクラス | 不適合クラス |
|---|---|---|
| `let`/`mut` の注釈 | ✅ 通る | ✅ 弾く |
| 自由関数の引数 | ✅ 通る | ✅ 弾く |
| メソッド引数 / コンストラクタ引数 / 戻り値 / テンプレート型引数 | ❌ **偽陽性で弾く** | △ 別種のエラー |
| フィールド書き込み | ❌ 偽陽性 | △ |
| **再代入** | — | ❌ **素通り** |

**実装したもの**

| 対象 | 内容 |
|---|---|
| `resolve_protocols` | 型の中の `NamedInstance(n)` を、`n` が protocol なら `Protocol(n)` へ寄せる（再帰） |
| `check_expected` | 期待型が protocol なら適合検査へ回し、それ以外は `type_matches`。`mut` 引数の厳格判定も内包 |
| `Stmt::Assign` | 再代入の型検査を新設（0-1 で束縛型が正しくなって初めて成立する） |

⚠ **収集パスではなく検査時に寄せる。** `registry/builder.rs` の `resolve_protocol_type` は
`known_protocols` が育つ途中に走るので**宣言順に依存する**。検査時点ではレジストリが
完成しているので確実。

⚠⚠ **`got` 側も寄せる必要があった。** protocol 名で注釈された仮引数は `declare_param` が
`NamedInstance("Pr")` として束縛するため、寄せずに適合検査へ渡すと「`Pr` という名のクラス」を
探しに行き、**`type 'Pr' does not satisfy protocol 'Pr'`**（自分自身に不適合）という
嘘のエラーになった（自動生成 `__init__` の `self.p = p` で実際に出た）。

**⚠⚠ D7: 注釈の有無で緩めない**

再代入検査を入れた当初、`equality_depth_limit_error.ar` の
`mut y = [1]` → `y = [y]`（`list[int]` → `list[list[int]]`）が落ちたため
「注釈つきの変数だけ検査する」と絞ったが、**方針として誤り**（ユーザ指摘）。
Arrow では型注釈は補助的なもので、**注釈が無い束縛にも推論型で同じ厳格さを適用する**。
⇒ 絞り込み（`VarInfo::annotated`）は撤回し、**例題側を直した**
（`mut y: list = [1]` — 要素型を問わないリストという意図を型で表す）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 69/69 identical |
| `compare_outputs.ps1` / `compare_bytecode.ps1` | 差分は新規例題 3 本のみ（`protocol_sites.ar` は基準側が偽陽性で止まるため大きく違う） |
| `compare_import_paths.ps1` | 13/13 identical |
| `compare_wasm_frontend.ps1` | 241/241 agreed・INVENTED 0 |

**追加した例題**: `examples/classes/protocol_sites.ar` ／ `protocol_sites_error.ar`
（8 束縛点 × 2 メンバーで 16 件の適合エラーを確認）

### 0-8 — trait 適合検査 【✅ 完了 2026-09-10】

**protocol との役割分担**: protocol は構造的適合なので「その型が要求を満たすか」を
**使用箇所**で見る（0-7）。trait は基底に書く名目的な継承なので、見るべきは
**クラス宣言そのもの**。

「実装し忘れ」は以前からパーサが検出していた
（[parser/classes.rs](../src/parser/classes.rs) の `collect_trait_fields_and_check_virtuals`
→ `class C must override virtual method m from trait T`）。ただしあれは**名前の有無しか見ない**。

**⚠⚠ 素通りしていた形（実測）**

| | 修正前 |
|---|---|
| trait フィールドを**別の型で再宣言**（`mut hp: int` → `mut hp: str`） | 静的 ❌ / 実行時 ❌ |
| メソッドの**戻り値型**が違う（`-> str` → `-> int`） | 静的 ❌ / 実行時 ❌ |
| メソッドの**引数型**が違う | 静的 ❌ / 実行時 ❌ |
| メソッドの**引数個数**が違う | 静的 ❌ / 実行時 ❌ |
| メソッドの**引数名**が違う | 静的 ❌ / 実行時 ❌ |

**実装したもの**

| 対象 | 内容 |
|---|---|
| `TypeErrorKind::TraitConformanceFailed` | 新設。`class 'W' does not satisfy trait 'Creature': …` |
| `check_trait_conformance` | `Stmt::ClassDef` の本体検査後に基底 trait ごとに照合 |
| `trait_sig_matches` | `FnSig` 同士の比較（`self` は除外） |
| `template_params` に `TraitDef` を追加 | trait 自身の型変数を登録 |

⚠ **クラスが再宣言していないフィールド・実装していないデフォルトメソッドは対象外**
（trait のものを継承するだけなので照合する相手が無い）。

⚠ **引数名も一致させる。** Arrow はキーワード引数を持つので、名前が変わると
trait 越しの呼び出し（`obj.speak(volume = 1)`）が壊れる。

⚠⚠ **テンプレート trait で偽陽性を踏んだ。** `trait Holder[T]: fn get(self) -> T` に対し
`class IntBox(Holder[int])` が `-> int` で実装するのは正しいが、trait 自身の型変数を
スコープへ積まずに照合すると `int != NamedInstance("T")` で弾いてしまう。
⇒ `check_one_trait` の入口で trait の型変数を積む。
⚠ 具体型引数（`Holder[int]` の `int`）は `class_bases` が**名前しか持たない**ため置換できない。
型変数を含む要求は**照合を見送る**（判らないものを通す）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 71/71 identical |
| `compare_outputs.ps1` | 差分は新規例題のみ（`trait_conformance.ar` は基準と一致） |
| `compare_import_paths.ps1` | 13/13 identical |
| `compare_wasm_frontend.ps1` | 243/243 agreed・INVENTED 0 |

**追加した例題**: `examples/classes/trait_conformance.ar` ／ `trait_conformance_error.ar`

#### ⚠⚠ 作業中に見つけた trait の**既存バグ 2 件**（本変更とは無関係・基準でも再現）

| # | 内容 |
|---|---|
| A | **trait フィールドを同じ型で再宣言すると自動 `__init__` の引数が重複する。** `trait Creature: mut hp: int; mut name: str` を `class Bear(Creature): mut hp: int; mut name: str` が再宣言すると `Bear(50, "Bruno")` が `'Bear.__init__' takes 4 argument(s) but 2 were given` になる。`build_field_index` は同名を**同一スロットへ畳んでいる**ので、コンストラクタの引数列とフィールドのレイアウトが食い違っている |
| B | **trait のデフォルト実装がクラスへ継承されない。** `trait Greeter: fn greet(self) -> str: return "hello"` を実装せずに `class Plain(Greeter)` とすると実行時 `AttributeError: 'Plain' has no method 'greet'`。⇒ 現状の trait は実質**抽象メソッド（`...`）だけ**が機能する |

どちらも正常系の例題に置けないので、`trait_conformance.ar` にコメントで記録した。

### 0-9 — trait フィールドの再宣言を禁止 【✅ 完了 2026-09-10】

0-8 の作業中に見つけた**既存バグ A** の修正。

**バグの内容**: trait のフィールドをクラスが再宣言すると、自動生成される `__init__` の引数が
**重複して数えられていた**。`build_field_index` は同名フィールドを**同一スロットへ畳む**のに
`generate_auto_init_if_needed` は trait 側と own 側を別々に数えるため、

```arrow
trait Creature:
    mut hp: int
    mut name: str
class Bear(Creature):
    mut hp: int       # 同じ型で再宣言
    mut name: str
Bear(50, "Bruno")     # 'Bear.__init__' takes 4 argument(s) but 2 were given
```

**採った方針（ユーザ決定）**: 畳み方を揃えるのではなく、**宣言そのものを禁止する**。

- trait のメンバーは **`obj::Trait.field` で修飾アクセス**できるので、同名をクラス側に置く
  必要がない（名前重複を回避する手段が既にある）
- **オーバーライドはメソッド専用の機能**。フィールドに同名を許すと「どちらの宣言が効くのか」が
  二重になる ⇒ 変数の重複は**再宣言**として扱い、許可しない
- ⇒ 畳み方の不一致に**構造的に到達しない**

**実装**: `collect_trait_fields_and_check_virtuals`（[parser/classes.rs](../src/parser/classes.rs)）に
own フィールド名との衝突検査を追加。**パース時**に弾く（自動 `__init__` 生成より前）。
既存の「複数 trait が同名フィールドを持つ」検査の隣に置いた。

⚠ **既存メッセージの構文誤りも直した。** trait 間衝突のメッセージが
`obj:{trait}::{field}` という**パースできない形**を案内していた（実際の構文は
`obj::{trait}.{field}`）。

⚠ 0-8 で入れた型検査側のフィールド照合は、この禁止により通常経路では到達しなくなる。
パーサを通らない経路（外部言語スタブ等）の backstop として残してある。

**⚠⚠ 既存例題 2 本がこのパターンを使っていた**（D1 に従い例題側を修正）:

| 例題 | 修正 |
|---|---|
| `classes/attr_access_paths.ar` | `class Base(HasPub)` / `class Other(HasPub)` の `mut pub_f: int` を削除（trait から継承） |
| `bench/bench_field_access.ar` | `class Ball(HasEnergy)` の `mut mass` / `mut speed` を削除 |

どちらも無修飾 `self.pub_f` / `obj.mass` でそのまま読み書きでき、**出力は基準と一致**（挙動不変）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 71/71 identical |
| `compare_outputs.ps1` | 差分は新規例題のみ（修正した 2 例題は基準と一致） |
| `compare_import_paths.ps1` | 13/13 identical |
| `compare_wasm_frontend.ps1` | 244/244 agreed・INVENTED 0 |

**追加した例題**: `examples/classes/trait_field_redeclare_error.ar`
⚠ フィールド再宣言は **ParseError** で全体が中断するため、`trait_conformance_error.ar`
（メソッド側）とは**別ファイルに分けた**。

### 0-10 — trait のデフォルト実装 ＋ クラスの仮想メソッド禁止 【✅ 完了 2026-09-10】

#### 調査: 仮想関数の仕組みは**既に存在していた**

判定は**構文のみ** — 本体がちょうど `...` なら `is_abstract: true`
（[functions.rs:299](../src/parser/stmts/functions.rs#L299) の `is_abstract_body` が
`NEWLINE INDENT ELLIPSIS` を見る）。`abstract` キーワードもデコレータも無い。

| 機構 | 状態 |
|---|---|
| trait の `...` メソッド ＝ 仮想メソッド | ✅ 既存 |
| 未実装ならエラー | ✅ 既存（`class C must override virtual method 'f' from trait 'T'`） |
| 仮想メソッドも**型注釈必須** | ✅ 既存（`virtual method 'f' is missing a return type annotation`） |
| `protocol` — 全メンバー抽象（`...` を**強制**）・インスタンス化禁止 | ✅ 既存 |
| trait のインスタンス化禁止 | ✅ 既存（実行時 `'trait' object is not callable`） |
| クラス継承の禁止（基底は trait のみ） | ✅ 既存 ⇒ 抽象基底クラスの役割は trait が担う |

⇒ **要件「trait に仮想関数を持たせる」「未実装はエラー」「注釈必須」は既に満たしていた。**

#### バグ B の修正: デフォルト実装の継承

**原因**: `exec_trait_def`（[definitions.rs:227](../src/interpreter/exec/definitions.rs#L227)）は
trait の**フィールド順とアクセス修飾子しか保持せず、メソッド本体を捨てている**。
`lookup_method_in_class` も `class.methods` を 1 段引くだけで基底を辿らない。
⇒ 実行時に引き継ぐ先が存在しなかった。

**実装（案 a）**: パース時にクラス本体へ注入する。

| 対象 | 内容 |
|---|---|
| `TraitInfo`（新設） | `known_traits` の 3 要素タプルを名前付き構造体へ。`default_methods` を追加 |
| `inject_trait_default_methods` | クラスが同名を定義していないデフォルト実装を body へ注入 |

⚠ 注入後はクラスが**全メソッドを持つ普通のクラス**になるので、メソッド解決・VM コンパイル・
属性 IC が**既存のまま動く**（新しい機構が増えない）。自動 `__init__` 生成と同じ方式。
⚠ `generate_auto_init_if_needed` より**前**に注入する（trait が `__init__` のデフォルト実装を
持つ場合に自動生成を抑止させるため）。
⚠ クラス側の定義が勝つ（override）。複数 trait が同名のデフォルト実装を持つ場合は
どちらが効くか決められないので静的エラー（フィールドの名前衝突と同じ方針）。
⚠ `node_id` は trait と各実装クラスで共有される（テンプレート実体化と同じ状況）。
注釈は最適化ヒントなので正しさには影響せず、`SlotCache`/`AttrCache` は `Clone` が空を返すので
クラスごとに再解決される。

#### クラスの仮想メソッド禁止

クラス本体・自由関数の `...` は**何も強制していなかった** — 黙って `None` を返す no-op で、
`-> int` と宣言しているのに `None` が返り、`return` 文が無いため 0-3 の戻り値検査も素通りした。

**実装**: `TypeErrorKind::VirtualMethodInClass` を新設し、`Stmt::ClassDef` の検査で
body の `is_abstract: true` を弾く。

⚠⚠ **外部言語のスタブクラスを誤検出してはいけない。** `import[cs-dll]` のスタブ生成
（[stub_gen.rs](../src/parser/cs_assembly/stub_gen.rs)）と `.ars` スタブ（`libm.ars` 等）は
`...` 本体のメソッド・関数を**正当に使う**（本体が向こう側にある）。
⇒ 判定には **`arrow_class_names`**（#27-a）を使う。あれは「Arrow ソースで `class` 宣言された」
集合で、外部言語スタブ由来を除いてある。**まさにこの区別のために用意されていた**ので、
新しいフラグを足さずに済んだ。

⚠ **自由関数の `...` は手を付けていない**（`.ars` スタブの主要な形。`libm.ars` は
数十個の `fn … : ...` で構成される）。→ §7

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` のみ（**FFI スタブは無事**） |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 70/70 identical |
| `compare_outputs.ps1` | 差分は新規・変更した例題のみ |
| `compare_import_paths.ps1` | 13/13 identical |
| `compare_wasm_frontend.ps1` | 245/245 agreed・INVENTED 0 |

**追加した例題**: `examples/classes/class_virtual_method_error.ar`
（正常系は `trait_conformance.ar` に仮想メソッド＋デフォルト実装の節を追記）

### 0-11 — テンプレート実体化検査の穴 2 件 【✅ 完了 2026-09-11】

テンプレートクラスのコンストラクタ引数検査（0-6）を改めて総点検したところ、
**7 形態は正しく捕捉**していたが 2 件の穴が残っていた。

#### 正しく捕捉できていた形（実測）

自動 ctor / 明示 `__init__` / trait 基底つき / 型引数がクラス / 入れ子型引数
（`Box[list[int]]`）/ キーワード引数 / `const` フィールド併存 — いずれも
`argument N of 'Box[int]' expects … but got …` を出す。
正常系（`Box[float]("t", 2)` の拡大・`Pair[int, str]`・trait フィールドの並び）も素通しする。

#### 穴 1: `__init__` がオーバーロードされていると素通りしていた

`check_template_call_args` が `sigs.len() == 1` のときだけ検査していたため、
`__init__` が 2 つあるだけで `Box[int]("wrong")` が通っていた。

⇒ **個数で絞って 1 本に決まれば検査する**（`check_self_type_params` と同じ規約）。

#### 穴 2: 型引数を省略した呼び出しのメッセージが読めなかった

`Box(1)` / `add(1, 2)`（型引数なし）は Arrow では不正だが、シグネチャの型変数が
具体型と突き合わされて **`argument 0 of 'Box.__init__' expects 'T' but got 'int'`** という
**型変数を漏らしたメッセージ**になっていた。実行時には
`TemplateError: template must be called with explicit type arguments` という良い文言があるのに、
静的エラーが先に出るので到達しない。

⇒ `TypeErrorKind::TemplateMissingTypeArgs` を新設し、実行時と同じことを静的に言って打ち切る。

#### 意図的に見送っている形（変更なし）

| 形 | 理由 |
|---|---|
| 型引数が外側の型変数（`fn f[U](): Box[U](x)`） | 具体型に決まらないので照合できない |
| 可変長 `__init__` | 個数が固定でないと対応付けが作れない |
| 型引数・実引数の**個数**不一致 | 実行時に `TemplateError` / `TypeError` で捕まる。個数が合わないと置換表が作れず対応付け自体が嘘になる |

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 70/70 identical |
| `compare_outputs.ps1` | 差分は 0-6〜0-10 で追加・変更した例題のみ |
| `compare_import_paths.ps1` | 13/13 identical |
| `compare_wasm_frontend.ps1` | 245/245 agreed・INVENTED 0 |

**例題**: `examples/typing/template_instantiate_error.ar` に Section 5（オーバーロード）と
Section 6（型引数省略）を追記。

### 0-12 — 既定値の型検査 【✅ 完了 2026-09-11】

**素通りしていた 6 形態**（すべて静的にも実行時にも通っていた・実測）

| 形 | 原因 |
|---|---|
| 自由関数の仮引数 `fn f(let n: int = "wrong")` | `check_fn_def` は注釈の**有無**しか見ず `param.default` に触れない。`declare_param` も無視 ⇒ **推論すらされていなかった** |
| メソッドの仮引数 | 同上 |
| `__init__` の仮引数 | 同上 |
| テンプレート関数の仮引数 | 同上 |
| `const` フィールドの既定値 | `Stmt::Field` が `self.infer(expr)` の結果を**捨てていた**（0-2 と同じ形の漏れ） |
| `static mut` フィールドの既定値 | 同上 |

**実装したもの**

| 対象 | 内容 |
|---|---|
| `TypeErrorKind::ParamDefaultTypeMismatch` | 新設。`default value of parameter 'n' of 'f' is declared 'int' but got 'str'` |
| `check_param_defaults` | 仮引数の既定値を推論し宣言型と突き合わせる |
| `Stmt::Field` | `infer` の結果を `check_expected` へ通す（エラーは `FieldTypeMismatch` を再利用） |

⚠ **既定値の検査は仮引数を宣言する前**に行う。既定値は他の仮引数を参照できない
（できると評価順に依存する）ので、まだ見えていない状態で推論する。
⚠ **型変数は積んだ後**に検査する（`fn g[T](let n: T = …)` を照合しないため）。
⚠ `mut` 引数でも既定値は**呼び元の記憶域ではない**ので拡大を許す（write-back の相手が居ない）。

**⚠ あわせて `const` / `static mut` の昇格も揃えた（0-B2 の残り）**

`const F: float = 3` が **`3` を返していた**。フィールド**書き込み**は `store_field` の
raw レイアウト経路で昇格するが、`const` はクラス変数・`static mut` は `static_cells` で
どちらもその経路を通らない。⇒ `exec_class_def` で `coerce_binding` を通して揃えた
（`3` → `3.0`）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 71/71 identical |
| `compare_outputs.ps1` / `compare_bytecode.ps1` | 差分は 0-7〜0-11 の例題のみ（**既存例題は不変**） |
| `compare_import_paths.ps1` | 13/13 identical |
| `compare_wasm_frontend.ps1` | 247/247 agreed・INVENTED 0 |

**追加した例題**: `examples/typing/default_value_type.ar` ／ `default_value_type_error.ar`

### A-2 — 具体化済みジェネリクスを型注釈として扱う 【✅ 完了 2026-09-11】

#### 何が壊れていたか

**`parse_type_expr` の末尾が `[...]` を消費して捨てていた。** そのため `let x: Box[int]` の
注釈が `"Box"` になり、型検査はフィールド型を `class_field_details("Box")` から引いて
**置換前の `T`** を得ていた。`T` は使用箇所では見えない型変数なので、0-2/0-3 の照合が

```arrow
mut x: Box = Box[str]("a")
x.v = "ok"                 # ❌ field 'v' of class 'Box' is declared 'T' but got 'str'
fn take(let b: Box) -> int:
    return b.v             # ❌ 'take' is declared to return 'int' but returns 'T'
```

という**偽陽性**を出していた（`alias IB: Box[int]` 経由でも同じ。alias は型位置で
`parse_type_expr` に再パースさせるので、同じ地点で型引数が落ちていた）。

#### 実装したもの

| 層 | 対象 | 内容 |
|---|---|---|
| 構文 | `parse_type_expr` | `known_templates` の名前に続く `[...]` を取り込み `Box[int]` を 1 つの型名にする |
| 型表現 | `InferredType::GenericInstance { name, args }` | 具体化済みジェネリクスの構造表現（新設） |
| 解釈 | `InferredType::from_ann` | `Name[Args]` を `GenericInstance` へ |
| 置換 | `class_and_subst` | 型からクラス名と「型変数 → 具体型」の置換表を取り出す（新設） |
| 消費 | `infer_attr` / `check_attr_assign` | メンバーの型を置換表で置換してから返す・比較する |

⚠ **`known_templates` に載る名前に限って厳密パースする。** この関数はキャスト
（`expr => Type`）からも呼ばれるので、無条件に厳密化すると `x => list[0]` のような
**型でない中身**でパースエラーになる。テンプレート名以外は従来どおり読み飛ばす。

⚠⚠ **素のテンプレート名（`mut x: Box`）では「型変数だけを `Unresolved` へ写す」表を作る。**
最初は `class_and_subst` を `None`（解決を諦める）にしたが、それだと同じクラスの
**具体型フィールド**（`Mixed[T]` の `count: int`）の検査まで消えた
（`template_type_param_error.ar` の検出が 2 → 0 になって実測で気づいた）。
型変数だけを `Unresolved` にすれば、具体型のメンバーは検査され、型変数のメンバーは
各検査の `matches!(expected, Unresolved | Any)` ガードで見送られる。

⚠ `declared_field_type` のフォールバック経路**にも置換表を通す**こと。`infer_attr` だけ
直すと、そちらが `Unresolved` を返したときに置換前の型が比較されて偽陽性が戻る（実測）。

#### 確認した範囲

| 形 | 結果 |
|---|---|
| `Box[int]` / `Box[str]` のフィールド代入 | ✅ 正しい型で照合 |
| `Pair[int, str]`（複数引数） | ✅ |
| `Box[list[int]]`（入れ子引数） | ✅ |
| `alias IB: Box[int]` 経由 | ✅ 型引数が届く |
| 仮引数 `let b: Box[int]` | ✅ |
| 素の `Box` | 型変数のメンバーは見送り・具体型のメンバーは検査 |

#### ⚠ 作業中に見つけた既存バグ（本変更とは無関係・基準でも再現）

**テンプレートクラスのインスタンスは `int` → `float` の昇格をしない。**
`Box[float]` に `b.v = 7` を入れると `7` のまま（通常クラスの `mut v: float` なら `7.0`）。
値の昇格は `store_field` の **raw レイアウト経路**だけが行うが、`build_template_class` は
`raw_layout` を設定しないためテンプレートクラスは常に boxed 経路を通る。→ §7

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 72/72 identical |
| `compare_outputs.ps1` / `compare_bytecode.ps1` | 差分は 0-7〜A-2 で追加した例題のみ（**既存例題は不変**） |
| `compare_import_paths.ps1` | 13/13 identical |
| `compare_wasm_frontend.ps1` | 249/249 agreed・INVENTED 0 |

**追加した例題**: `examples/typing/generic_type_ann.ar` ／ `generic_type_ann_error.ar`

#### 残り（B / C・未着手）

| 項目 | 内容 |
|---|---|
| **C** | `new_type` の右辺に型式を許す。現在 `exec_new_type_def` が `get_val(original)` で**名前引き**するため `new_type Ints: list[int]` が実行時 `NameError` になる |
| **B の宣言側** | `new_type X[T]: …` / `alias X[T]: …`。どちらも `parse_template_params()` を呼んでいないので ParseError。実体化の意味論（`new_type` なら新しい型・`alias` なら置換）を決める必要がある |

### A-3 — boxed レイアウトのフィールドを実行時に検査する 【✅ 完了 2026-09-11】

> 「インスタンスで型を不正なまま取り扱うバグを優先して潰してください。黙って不整合な型が
> 扱われる原因になります」

#### 何が壊れていたか

フィールド書き込みは `InstanceData::store_field` に**集約されている**（後述のとおり
全経路がここを通る）。ところが型を見ていたのは **raw レイアウト経路だけ**で、
boxed 経路は受け取った値をそのまま `boxed_fields[idx]` へ入れていた。

raw レイアウトが付くのは `RawLayout::from_fields` が `Some` を返すとき、つまり

- trait を継承しておらず、かつ
- **全**フィールドが int/float 系プリミティブ（`RawWidth::from_ann` が通る）、かつ
- フィールド数 ≤ 24

のときだけ。したがって

| 条件 | 結果 |
|---|---|
| `str` フィールドが 1 つ混ざる | 検査が**全フィールド**消える |
| trait を 1 つ実装する | 同上 |
| テンプレートクラス | 同上（`build_template_class` は `raw_layout` を作らない） |

静的検査が要素型を追い切れない経路から、**黙って**不整合な型が居座っていた:

```arrow
class Tagged:
    mut n: int
    mut label: str
mut t = Tagged(1, "a")
mut xs: list = [1, "wrong"]   # 要素型なし ⇒ xs[1] は Unresolved
t.n = xs[1]                   # 静的には通る
print(t.n)                    # 基準: wrong ← int フィールドに str が入っている
```

同じ穴で **`int` → `float` の昇格も効いていなかった**（A-2 節末で「既存の別バグ」として
記録したもの）。しかも同一フィールドで経路によって結果が食い違っていた —
`F(3, "n")` は仮引数昇格（0-B2）のおかげで `3.0`、直後の `c.f = 4` は無検査なので `4`。

#### 実装したもの

| 層 | 対象 | 内容 |
|---|---|---|
| 型情報 | `ClassValue.field_tags: Vec<TypeTag>` | スロット順の**宣言型タグ**（新設。`synthetic()` / `deep_clone` にも追加） |
| 構築 | `build_field_index` | 戻り値を `(field_index, field_mutability, field_tags, field_count)` へ。タグは `field_mutability` と**同じループ**で push する |
| 宣言順 | `trait_field_order` / `py_class_field_order` | `Vec<(String, bool)>` → `Vec<(String, bool, String)>`（型注釈を運ぶ） |
| 収集 | `exec_trait_def` / `exec_class_def` / `build_template_class` | `own_field_order` を 3 つ組へ。py_class の平坦化は**自前宣言が可変性も型も勝つ** |
| 検査 | `store_field` の boxed 分岐 | `TypeTag` で照合し、`(Float, Int)` は昇格してから格納 |

⚠ **スロット順がずれると「別のフィールドの型」で検査してしまう**（黙って誤った値を通す／
正しい値を弾く、どちらにも転ぶ）。だから `field_tags` は `field_mutability` と同じループで
積む — 2 箇所に分けると将来の編集でずれる。

⚠ **判定できる種別だけを見る。** `TypeTag::Other`（`list[T]`・クラス名・`Union[...]`）と
`Any` は通す。誤検知で正しいコードを落とすより取りこぼす方へ倒す（`ffi_boundary` の
「保守的側」と同じ方針）。

⚠⚠ **`int` と `uint` は相互に許容する。** `TypeTag::matches` は `uint` に `Value::UInt` を
要求するが、Arrow で `Value::UInt` が生まれるのは**ハンドル値と uint 同士の除算だけ**で、
`let x: uint = 5` も `f(v: uint)` も値としては `Value::Int` を束縛する（変数・引数・
戻り値のどの束縛点も `uint` 注釈に対して `Value::Int` を通す）。ここだけ厳格にすると
**実バグを 1 つも塞がずに正常なコードを落とす**（`Counter(5, "c")` が落ちるのを実測）。
よって整数族は通し、`str`/`bool` の混入だけを弾く。
⚠ `TypeTag::matches` 自体は VM の `MustBe`/`IsType` と共有なので**変えない** — `store_field`
の中で処理する。`uint` 注釈と `Value::UInt` のずれ自体は別タスク → §7

#### 全書き込み経路が `store_field` を通ることの確認

| 経路 | 通り道 |
|---|---|
| `o.f = v`（ツリーウォーク） | `attr_assign` → `store_field`（`attrs.rs:370`） |
| `o.f = v`（VM） | `Op::SetAttr` → `attr_assign_evaled` → `store_field`（`attrs.rs:434`） |
| `o::Trait.f = v` | `trait_attr_assign` → `store_field`（`attrs.rs:541`） |
| 明示 `__init__` | 本体の `self.f = arg` なので上の 3 つに帰着 |
| **合成 `__init__`** | `inject_auto_init` が `Stmt::AttrAssign` / `TraitAccess` を生成 ⇒ 同上 |
| 既定値 | `instantiate_evaled` → `store_field` |
| FFI 書き戻し | `apply_shadow_raw` → `store_field`（戻り値を検査している） |

`boxed_fields` へ直接書くのは `freeze`（可変性フラグのみ）と `deep_copy` / `Clone`
（既に検査済みの値の複製）だけで、新しい値は入らない。**3 つの代入地点はいずれも
`false` を `TypeError: value does not match declared type of field '<f>'` に変換済み**。

#### 確認した範囲

| 形 | 結果 |
|---|---|
| `str` が混ざるクラス（raw が付かない） | ✅ 検査される |
| `int` → `float` 昇格（boxed） | ✅ `5` → `5.0` |
| テンプレートクラス `Box[float]` | ✅ `7` → `7.0`（A-2 で記録した既存バグの解消） |
| テンプレートクラス `Box[str]` に int | ✅ 弾く（**静的検査が見送る経路**を実行時が捕まえた） |
| trait フィールド `p::Named.name` | ✅ 検査される |
| `uint` フィールドに整数リテラル | ✅ 通る（退行なし） |
| `Any` / `list[int]` フィールド | ✅ 通す（判定対象外） |

#### ゲート結果

⚠ 基準は **HEAD = `cbbf07f`（A-2）から作り直した**。最初は古い基準を使って 13 件の
差分が出たが、大半は「基準バイナリが 0-7〜A-2 の検査をまだ持たない」ための見かけの差分
だった。A-3 だけを切り分けるには直前コミットの基準が必要（CLAUDE.md の注意どおり）。
⚠ worktree は作れない（追跡された `crates/arrow-frontend/target/` が
`Filename too long` を起こす → §7）。変更ファイルを退避 → `git show HEAD:<path>` で
差し替え → ビルド → md5 照合して復元した。

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 72/72 identical・stale 0（新規 2 例題を `$knownDiff` へ登録） |
| `compare_outputs.ps1 -A <A-2>` | 178/181・差分 3 件はすべて**意図したもの**（下表） |
| `compare_bytecode.ps1 -A <A-2>` | 200/200 identical（負の対照 0 / **陽性対照 4 件検出**） |
| `compare_import_paths.ps1 -A <A-2>` | 13/13 identical |
| `compare_wasm_frontend.ps1` | **不要**（A-3 は interpreter だけで lexer/parser/type_check に触っていない） |

**意図した出力差分（3 件）**

| 例題 | 基準 → 現在 | 理由 |
|---|---|---|
| `typing/generic_type_ann.ar` | `7` → `7.0` | テンプレートクラスでも float へ昇格する |
| `classes/field_type_runtime.ar` | `5` → `5.0`, `7` → `7.0` | 同上（boxed 経路の昇格） |
| `classes/field_type_runtime_error.ar` | `ここには到達しない: wrong` → `TypeError` | **塞いだ穴そのもの** |

#### 副産物 — `compare_bytecode.ps1` の偽差分を直した

新しいエラー例題で `compare_bytecode.ps1` が「A=41 / B=42 lines」と報告した。
**バイトコードは完全に同一**で、原因は `AR_VM_DUMP=1` のダンプと**実行時エラーの
traceback が同じ stderr に出る**こと。このゲートの主張は「バイトコードが同一か」であって
stderr の一致ではない（それは `compare_outputs.ps1` の担当）。よって `Get-Dump` が
**ダンプ行だけを残す**ようにした（chunk ヘッダと命令行のみ）。

⚠ 併せて「実行時エラーで停止すると**後続 chunk がコンパイルされない**」ことも判明した
（chunk は遅延コンパイルなので、基準だけが末尾 chunk をダンプして差分になる）。
エラー例題は**落ちる文を最後に置く**こと — `field_type_runtime_error.ar` にコメントで残した。

⚠ フィルタ追加でゲートが緩んでいないことは**陽性対照**で確認した（古い基準に対して
実差分 4 件を検出）。負の対照（同一 exe）は 0 件。

**追加した例題**: `examples/classes/field_type_runtime.ar` ／ `field_type_runtime_error.ar`

### A-4 — クラス型・trait 型・protocol 型のフィールドも検査する 【✅ 完了 2026-09-11】

#### 何が壊れていたか

A-3 で boxed 経路の検査を入れたが、判定に使う `TypeTag` は
`int` / `uint` / `float` / `str` / `bool` / `Any` しか区別しない。**クラス名・trait 名・
protocol 名は `TypeTag::Other` に落ちて素通り**していたので、A-3 の後もまだこれが通った:

```arrow
class Dog:
    mut n: int
class Cat:
    mut m: int
class Holder:
    mut pet: Dog
    mut label: str

mut h = Holder(Dog(1), "h")
mut xs: list = [Cat(2), 3]   # 要素型なし ⇒ xs[0] は Unresolved（静的には通る）
h.pet = xs[0]
print(h.pet.m)               # 2 ← Dog に無いフィールドが読めてしまう
```

⚠ **静的検査では届かない。** `mut xs: list` は要素型を持たないので `xs[0]` は
`Unresolved`。要素型を書けば静的に弾ける（実測: `list[Cat]` にすると
`field 'pet' of class 'Holder' is declared 'Dog' but got 'Cat'`）が、
注釈は補助であって前提にはできない（D ポリシー）。

#### 実装したもの

| 層 | 対象 | 内容 |
|---|---|---|
| 型情報 | `FieldCheck`（新設 enum） | `None` / `Class(name)` / `Trait(name)` / `Protocol(required)` |
| 型情報 | `ClassValue.field_checks: Vec<FieldCheck>` | スロット順。`field_tags` と**同じループ**で積む |
| 分類 | `Interpreter::classify_field`（新設） | 型注釈を**クラス定義時に 1 度だけ**分類する |
| 検査 | `store_field` の boxed 分岐 | 値が `Value::Instance` のときだけ `field_checks` を見る |

⚠⚠ **Arrow はクラス継承を許していない** — `class C(Base)` の `Base` は trait だけで、
`parser/classes.rs` が `cannot inherit from ... (only traits are allowed as bases)` で弾く。
だから**クラス型フィールドはクラス名の完全一致で判定でき、祖先を遡る必要が無い**。
⚠ この前提が崩れたら（クラス継承を入れたら）`FieldCheck::Class` の判定を継承チェーン
探索へ変えること。`FieldCheck` の doc にも同じ注意を置いた。

⚠ **分類はクラス定義時に 1 度だけ。** 実行時にはクラス名のレジストリが無く（`ClassValue.bases`
は名前の `Vec<String>` だけ）、代入ごとにスコープを引くのは hot path には重い。
定義時に protocol / trait / クラス / 判定不能へ畳んでおけば `store_field` は O(1) で済む。

⚠⚠ **判定できないものは必ず通す。** 型変数（`T`）・`list[T]`・`Union[...]`・型引数つきの名前・
未定義名・**このクラスより後に定義されるクラス**は `FieldCheck::None` に落ちる。
特に後者は「定義順に依存して検査が消える」ので、**取りこぼす方へ倒す**のが必須
（逆に倒すと正しいコードが定義順で落ちる）。

⚠ **判定するのは値が `Value::Instance` のときだけ。** `CsObject`・ネイティブハンドル・
`None` などは判定材料が無いので通す。
⚠ `new_type` ラッパは `new_type_base` が一致すれば受ける（`new_type M: Dog` を `Dog` へ）。
⚠ protocol は**構造的**に見る（必須メンバーが全て在るか）。trait の名前的判定との違いを
例題 `field_class_type.ar` に残した（`Dog` は `HasN` を宣言していないが受理される）。

#### 型引数つきの名前を対象外にした理由

`list[int]` / `Union[...]` / `Box[int]` は `FieldCheck::None`。要素型まで見るのは代入ごとには
重く、さらに `Box[int]` は**実体化後のクラス名が `Box[int]` とは別**なので名前一致では
判定できない。⚠ ただしテンプレートクラスの**フィールド**は実体化時に型引数で置換される
ので、`Box[Dog]` の `v` は `Dog` として分類され検査される（実測で確認）。

#### 確認した範囲

| 形 | 結果 |
|---|---|
| クラス型フィールドへ同一クラス | ✅ 通る |
| クラス型フィールドへ別クラス（`Unresolved` 経由） | ✅ 弾く ← **塞いだ穴** |
| trait 型フィールドへ実装クラス | ✅ 通る |
| protocol 型フィールドへ構造一致クラス（宣言なし） | ✅ 通る |
| `Box[Dog]` の `v` へ `Dog` / `Cat` | ✅ 通る / ✅ 弾く（置換後も検査される） |
| `Any` / `list[int]` フィールド | ✅ 通す（判定対象外） |

#### ゲート結果

基準は HEAD = `ef95d03`（A-3）から作成（A-3 と同じ退避 → `git show HEAD:<path>` → 復元手順）。

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` のみ |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 75/75 identical・stale 0 |
| `compare_outputs.ps1 -A <A-3>` | 182/183・差分は新規 `field_class_type_error.ar` のみ（**既存例題は完全に不変**） |
| `compare_bytecode.ps1 -A <A-3>` | 202/202 identical |
| `compare_import_paths.ps1 -A <A-3>` | 13/13 identical |
| `stale_doc_refs.ps1` | OK |
| `compare_wasm_frontend.ps1` | **不要**（interpreter だけの変更） |

⚠ `field_type_runtime_error.ar` の `$knownDiff` 登録は**外した**。到達不能行を末尾から
削った（`compare_bytecode` の偽差分対策）結果、stdout が impl_python と一致するように
なったため。STALE 報告で気づいた。

**追加した例題**: `examples/classes/field_class_type.ar` ／ `field_class_type_error.ar`

#### ⚠ 調査中に見つけた別件（型検査の穴ではない）

**`Optional[T]` は Arrow の綴りとして存在しない。** キーワードは `Union` と `Option` だけで
（`src/lexer/keyword.rs`）、`Optional` は素の識別子なので `parse_type_expr` が
`[...]` を読み飛ばし、注釈が `"Optional"` になる。そのため
`let a: Optional[int] = 1` が `'a' is declared 'Optional' but initialized with 'int'` という
**分かりにくいエラー**になる（「未知の型」と言うべき）。
⚠ 一方 `value_matches_type_ann` は `"Optional["` を `Option[` の別名として受ける分岐を
持っており、`type_check/types.rs` の `from_ann` は持たない — **実装間で食い違っている**。
正しいコードは壊れていない（`Optional` は元から無効）ので本計画の対象外。→ §7

### R3 — フィールドのオフセット化 【✅ 完了として閉じた 2026-09-08】

**⚠ 着手時の前提が誤っていた。** 「計画は多相 IC・実装は単相 IC ＝ 差分が未実装」と報告したが、
[BYTECODE_VM_PLAN.md §4.3](../implementation_logs/BYTECODE_VM_PLAN.md#L499) の R3 行が指定している機構は

> 判らなければ **多相 IC**: `InstanceData.class_id` で
> 「前回と同じ class_id ならオフセット再利用、違えば `field_index` 引き直してキャッシュ更新」

で、これは**エントリ 1 個・ミス時に上書き**する inline cache そのもの。`AttrCache` の実装と一致する。
計画の「多相 IC」は「**多相な呼び出し点でも（引き直して）動く IC**」の意で、N-way を指していない。
`ast.rs` が同じ機構を「単相 IC」と呼ぶのは標準用語（monomorphic = エントリ 1 個）に従ったもので、
**同じコードに呼び名が 2 つ付いていた**のが読み違えの原因。
⇒ **原計画に未実行の R3 作業は無い。** N-way 化は原計画の外側の新規提案だった。

再発防止として、R3 行の側にこの注記を足した（呼び名が 2 つある旨と再測結果）。

#### 完了を裏づけるために入れた計測基盤

| 対象 | 内容 |
|---|---|
| `prof::IC_HIT` / `IC_COLD` / `IC_POLY` | ミスを **cold**（空＝初回）と **poly**（class_id 違い＝多相）に分ける |
| `AttrCache::is_empty` | cold / poly の判別に要る（`--features prof` 専用の読み手） |
| `prof::note_ic` | `Op::GetAttr` / `Op::GetAttrLocal` の probe 地点で計上 |
| [`scripts/prof_attr_ic.ps1`](../scripts/prof_attr_ic.ps1) | 全例題を回して集計（`-Filter` / `-Top`） |

⚠ 分けずに「ミス率」だけ見ても判断できない（N-way で救えるのは poly だけ）。
⚠ **Phase T より後でないと測れない**。メモ化前はテンプレートクラスが実体化ごとに新 `class_id` を
取っていたので、多相性ではなくキャッシュ欠落を測ってしまう。

#### 実測（例題 224 本）

```
probes       : 77,400,461
  hit        :   77,400,077  (99.9995%)
  cold miss  :          375  ( 0.0005%)
  poly miss  :            9  ( 0.0000%)
```

多相ミスは全例題あわせて **9 件**。属性アクセスが最も重い `bench_method_hot.ar` は
**2400 万 probe すべてヒット**（poly 0）。⇒ R3 は設計どおり効いている。

合成ベンチ（意図的に多相な地点を作る）では、多相地点は **hit 0% / 約 1.8x 遅い**
（4 クラス交替 709ms vs 同 probe 数の単相 383ms）。病理は実在するが、R3 行が
「できねば辞書アクセス」と明示的に許容していた範囲。

#### 再開の条件（起票せず記録のみ）

判定基準は「**消費者が居るか**」（#11 R2-c / #14 / #15b を保留にしたのと同じ基準）。
現時点で居ない（9 / 7,740 万）。多相な属性アクセスがホットなワークロードが現れたら、
`prof_attr_ic.ps1` で**先に測ってから**規模を決める。

設計案（未着手）: **2-way** ならヒット経路のコストは不変（エントリ 0 を先に見る）で
`AttrCache` は 8 → 16 バイト。N-way にするなら mono → poly(N) → megamorphic の状態遷移が要る。
⚠ どちらも `AttrCache::Clone` が空を返す不変条件（AST コピーごとの再解決）を壊さないこと。

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

---

## 6. 実装したもの（まとめ）

### コミット

| コミット | 内容 |
|---|---|
| `22a20c8` | 調査結果と本計画 |
| `15aa677` | **0-B** `int`→`float` の暗黙拡大と束縛時昇格 |
| `0c06c60` | **0-2** フィールド書き込みの型検査 ← **報告されたバグの本体** |
| `335ac46` | **0-3** `return` と宣言戻り値の照合 |
| `2084f33` | **0-B2** 戻り値・仮引数でも昇格（案 B-i） |
| `4a1d4ea` | **0-4** メソッド引数の型検査 |
| `62ac4d2` | **0-5** コンストラクタ引数の型検査 |
| `fdd771a` | **0-1** `let`/`mut`/`const` の注釈採用と照合 |
| `0afaea0` | **Phase T** テンプレート実体化クラスのメモ化 ＋ 型変数の取り違え修正 |
| `1690523` | **R3** 属性 IC の計測基盤と実測 |
| `1ebedec` | R3 を完了として閉じ、文書へ追記 |
| `797f920` | **0-6** テンプレート実体化呼び出しの引数型検査 |
| `ed8b985` | **0-7** protocol 適合検査を全束縛点へ ＋ 再代入の型検査 |
| `06a4321` | **0-8** trait 適合検査を静的に行う |
| `7a45d66` | **0-9** trait フィールドの再宣言を禁止（既存バグ A） |
| `6782113` | **0-10** trait のデフォルト実装の継承 ＋ クラスの仮想メソッド禁止 |
| `569ae5a` | **0-11** テンプレート実体化検査の穴 2 件 |
| `efa8012` | **0-12** 既定値の型検査（仮引数・`const`・`static mut`） |
| `cbbf07f` | **A-2** 具体化済みジェネリクスを型注釈として扱う |
| `ef95d03` | **A-3** boxed レイアウトのフィールドを実行時に検査する |
| `ca7625a` | `stale_doc_refs` の誤検知 2 件を直す（副産物） |
| （本コミット） | **A-4** クラス型・trait 型・protocol 型のフィールドも検査する |

### 型検査に足した検査

| 束縛点 | エラー種別 | 実装箇所 |
|---|---|---|
| フィールド書き込み | `FieldTypeMismatch`（新設） | `check_attr_assign` ＋ `declared_field_type` |
| `return` | `ReturnTypeMismatch`（新設） | `check_return_type` ＋ `CheckState::current_fn_return` |
| `let`/`mut`/`const` | `VarTypeMismatch`（新設） | `resolve_declared_type` |
| メソッド引数 | `CallArgTypeMismatch`（既存） | `check_self_type_params` |
| コンストラクタ引数 | 同上 | `infer_call_inner`（`__init__` へ委譲） |
| テンプレート実体化の引数 | 同上 | `check_template_call_args` ＋ `subst_type_params` |

### 実行時に足した昇格（`int` → `float`）

| 経路 | 実装 |
|---|---|
| `let`/`mut`/`const` | `coerce_binding`（ツリーウォーク）／`Op::CoerceFloat`（VM） |
| 仮引数 | `try_fast_bind`（高速）／`bind_args` 後のループ（一般） |
| 戻り値 | `exec_fn_evaled` のラッパ **1 箇所**（両経路がここへ来る） |
| フィールド（raw レイアウト） | 既存（`store_field` の raw レイアウト経路） |
| フィールド（**boxed レイアウト**） | `store_field` の boxed 分岐（**A-3 で新設** — `str` が混ざる／trait を実装する／テンプレートクラスのときは raw が付かないので、昇格も検査も無かった） |

### 実装中に見つけた・直したもの

| # | 内容 |
|---|---|
| 1 | **`mut` 引数（write-back）へ拡大を許してはいけない** — C ABI の `double*` に int 変数を渡すと記憶域の型詐称になる。既存テスト `cpp_prim_ptr_int_arg_type_mismatch` が検出 |
| 2 | **要素型へ拡大を伝播させてはいけない** — 実行時の昇格はスカラだけなので `list[int]` を `list[float]` に通すと静的と実行時がずれる |
| 3 | **フィールドの宣言は 2 つのテーブルに分かれていた** — trait 由来は `trait_field_details`。`collect_class_field_details` も見ないので `declared_field_type` を足した |
| 4 | **既存例題 3 本が実際に誤っていた** — `/` は int 同士でも float を返すのに `-> int` と宣言していた |
| 5 | **0-B が新しい穴を開けた** — 受理だけ広げて値を揃えず、戻り値と仮引数で「注釈が嘘をつく」状態が移動していた（0-B2 で解消） |
| 6 | **0-2 で追加した自分の例題が間違っていた** — trait フィールドの宣言順を取り違えており、0-5 の検査が検出 |
| 7 | **型変数を具象型と取り違えていた** — `from_ann` が大文字始まりの未知の識別子をクラス名にするため。`mentions_type_param` で除外（Phase T で発見） |
| 8 | **テンプレートクラスにキャッシュが無かった** — 実体化ごとに新 `class_id` を発行しており、同じ `Box[int]` が別の型になっていた |
| 9 | **R3 の「多相 IC」の読み違え** — 原計画と実装は同じ機構。呼び名が 2 つあった |
| 17 | **型注釈が型引数を捨てていた** — `parse_type_expr` が `[...]` を消費して破棄しており、テンプレートのフィールド型が置換前の `T` のまま比較されて偽陽性になっていた（A-2）。素の名前で解決を諦めると逆に検査が消えることも実測で発見 |
| 16 | **既定値が 6 箇所で未検査だった** — 仮引数の既定値は**推論すらされておらず**、`const`/`static mut` は `infer` の結果を捨てていた（0-12）。`const F: float = 3` が `3` を返す昇格漏れも同時に修正 |
| 15 | **テンプレート実体化検査に穴が 2 件あった** — `__init__` のオーバーロードで素通り、型引数省略時に型変数を漏らすメッセージ（0-11） |
| 14 | **trait のデフォルト実装が継承されなかった**（既存バグ B）。`exec_trait_def` がメソッド本体を捨てていた。パース時にクラス本体へ注入して解決（0-10）。あわせて**クラスの仮想メソッドを禁止**（以前は黙って `None` を返す no-op）。⚠ FFI スタブの `...` を誤検出しないよう `arrow_class_names` で絞った |
| 13 | **trait フィールドの再宣言で自動 `__init__` の引数が重複していた**（既存バグ A）。畳み方を揃えるのではなく**宣言を禁止**して解決（0-9）。既存メッセージがパースできない構文を案内していたのも直した |
| 12 | **trait の同名・中身違いが素通りしていた** — パーサの検査は名前の有無しか見ていなかった（0-8）。作業中に trait の既存バグ 2 件（自動 `__init__` の引数重複・デフォルト実装が継承されない）も発見 |
| 11 | **protocol 適合が束縛点ごとに逆方向へ壊れていた** — 適合するクラスを弾く偽陽性と、不適合が素通りする偽陰性が同居していた（0-7） |
| 10 | **テンプレート実体化の呼び出し点が丸ごと未検査だった** — 当初の調査で見つけていたのに計画表から落としており、Phase 0 完了報告後にユーザ報告で発覚（0-6） |

### 意図的な非互換

| # | 内容 |
|---|---|
| 1 | **`Any` が本当に `Any` になった** — `let x: Any = 5; x + 1` が明示ダウンキャスト要求のエラーに。以前は注釈が捨てられ `int` として束縛されていた。⚠ 例題は 1 本も壊れなかった |
| 2 | **`float` 注釈の束縛で値が昇格する** — `let b: float = 3` が `3` ではなく `3.0` |
| 3 | **テンプレートのクラスレベル初期化子の評価回数** — 「実体化のたび」から「型引数の組ごとに 1 回」へ（通常のクラス定義と同じ回数） |

### 追加した例題

`typing/int_float_widening{,_error}.ar` ／ `classes/field_type{,_error}.ar` ／
`typing/return_type{,_error}.ar` ／ `classes/method_arg_type_error.ar` ／
`classes/ctor_arg_type_error.ar` ／ `typing/var_annotation{,_error}.ar` ／
`typing/template_type_param{,_error}.ar` ／ `typing/template_instantiate_error.ar` ／
`classes/protocol_sites{,_error}.ar` ／ `classes/trait_conformance{,_error}.ar` ／
`classes/trait_field_redeclare_error.ar` ／ `classes/class_virtual_method_error.ar` ／
`typing/default_value_type{,_error}.ar` ／ `typing/generic_type_ann{,_error}.ar` ／
`classes/field_type_runtime{,_error}.ar` ／ `classes/field_class_type{,_error}.ar`

### 副産物

- **VM の型特化が改善** — `raise_span_fields.ar` の `mut lines: list[int] = []` で
  `lines[0] != lines[1]` が `BIN NotEq` → `IBIN_SS NotEq` に（出力不変）。
  0-B でも「注釈は float・実値は Int」で特化 op が毎回汎用へ落ちていた状態が解消。
- **テンプレート関数の戻り値昇格** — `TemplateFnValue.return_type` を足したので
  `conv[float](3)` → `3.0`（修正前は偽エラーで実行すらできなかった）。
- **op を足すときの注意が 1 つ増えた** — `vm/op_prof.rs` が宣言順のインデックス表を持つので
  **新しい op は `enum Op` の末尾へ足す**。`op.rs` に明記した。

## 7. 残っている宿題（本計画の対象外・記録のみ）

| # | 内容 |
|---|---|
| 2 | **複合代入 `o.f += v` の型検査** — 格納されるのは `f <op> v` の結果なので、二項演算の結果型を求める必要がある |
| ~~9~~ | ~~**テンプレートクラスのインスタンスが `int`→`float` 昇格をしない**（A-2 で発見）~~ → **A-3 で解消**（boxed 経路に昇格と検査を入れた） |
| 10 | **`uint` 注釈と `Value::UInt` がずれている** — `let x: uint = 5` も `f(v: uint)` も束縛するのは `Value::Int` で、`Value::UInt` はハンドル値と uint 同士の除算からしか生まれない。一方 `value_matches_type_ann` / `TypeTag::matches` は `uint` に `Value::UInt` を要求するので、**注釈どおりの値が流れていない**。A-3 は `store_field` 内で整数族を相互許容して回避しているが、本筋は「`uint` 注釈の束縛点で `Value::UInt` を作る」か「`uint` を `int` の別名に落とす」かの決着 |
| 12 | **`Optional[T]` の扱いが実装間で食い違っている** — キーワードは `Union` / `Option` だけなので `Optional` は素の識別子になり、`parse_type_expr` が `[...]` を捨てて注釈が `"Optional"` になる。`value_matches_type_ann`（実行時）は `"Optional["` を `Option[` の別名として受ける分岐を持つのに `from_ann`（静的）は持たない。`Optional` を正式な綴りにするか、`Optional` を未知の型として明確に弾くかの決着が要る（今は分かりにくい型不一致エラーになる） |
| 11 | **`crates/arrow-frontend/target/` が追跡されているせいで `git worktree add` が失敗する**（`Filename too long`）。A/B の基準ビルドを worktree で作れない（#5 と同根） |
| 8 | **自由関数の `...` 本体が no-op のまま** — `fn g() -> int: ...` が `None` を返す。⚠ `.ars` スタブの主要な形（`libm.ars` は数十個）なので、禁止するならスタブ文脈の判別が必要（自由関数には `arrow_class_names` に相当する集合が無い） |
| 4 | **添字代入 `a[i] = v`** — 要素型と値の照合をしていない |
| 6 | **テンプレート実体化の戻り値型** — 呼び出し点では `Unresolved` のまま。`NamedInstance(base)` に変えるにはフィールド型の型変数も置換する必要がある（型検査側での実体化）|
| 7 | **型引数・実引数の個数検査** — 実行時の `TemplateError` / `TypeError` のみ。静的化は可能だが例題の期待が変わる |
| 5 | **`crates/arrow-frontend/target/` の wasm 成果物が tracked のまま gitignore に載っている** — `git rm --cached` で追跡から外すかは要判断 |
