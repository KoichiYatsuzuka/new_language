# enum のメンバーを enum 型にする（`Color.BLUE` の型は `Color`）

enum のメンバー（`Color.BLUE`）の型が、型検査でも実行時でも enum 型（`Color`）とは別の内部の型
`enum_item_Color` になっている。そのため **`Color` 型の値は 1 つも存在しない**。
`Color` 型の引数とメンバーを比べるとエラーになり、`x is Color` は偽を返す。
本書は、メンバーの型を enum 型そのものに統一するための計画書である。

- 起票: 2026-10-07（外部プロジェクト `TeX_editor/letter_processor.ar:305` の `if c == ProofColor.BLUE:` が
  静的エラーになったのが発端）
- 調査: Arrow `2045111`
- 別件: 同じ「can never be true」の誤検出が trait でも起きるが、原因が違うので
  [trait_equality_plan.md](trait_equality_plan.md) に分けた
- **状態: 完了**（2026-10-08・各タスクのコミットは §4 の表）

## 0. 何が起きるか

```
enum Color:
    PLANE
    BLUE

fn label(let c: Color) -> str:
    if c == Color.BLUE:          # StaticTypeError: '==' between 'Color' and 'enum_item_Color' can never be true
        return "blue"
    return "plane"
```

| 書き方 | 型検査 | 実行時（Rust / Python とも） |
|---|---|---|
| `c == Color.BLUE`（`c: Color`）・逆向き・`!=`・`c in [Color.BLUE]` | ❌ `CrossTypeEquality` | （型検査で止まる） |
| `k == Color.BLUE`（`let k = Color.BLUE`・注釈なし） | ✅ | ✅ |
| `let m: Color = Color.BLUE` | ✅（アップキャスト） | ✅ |
| `Color.BLUE is Color` | ✅ | ❌ `False` |
| `match` の `is Color:` の腕 | ✅ | ❌ 当たらない |
| `Color.BLUE mustbe Color` | ✅ | ❌ `expected Color, got enum_item_Color` |
| `Pen(Color.BLUE)`（`mut color: Color` のフィールド） | ✅ | ❌ `value does not match declared type of field` |
| `Color.BLUE == Other.X`（別の enum） | ❌（正しい） | — |

⇒ 型検査は `let m: Color = Color.BLUE` を通すのに、実行時はその `m` を `Color` と認めない。

## 1. 原因

### 1.1 型検査

- `Color.BLUE` の型は `NamedInstance("enum_item_Color")`、`c: Color` の型は `NamedInstance("Color")`
  （`src/type_check/registry/builder.rs:452-472`）。
- 代入は「`enum_item_X` → `X` はアップキャスト」の規則で通る（`src/type_check/type_utils.rs:508-518`）。
- 等値の検査 `check_equality`（`src/type_check/binop.rs:120-162`、タスク 7.6・`57f81c0`）は
  「同型」「数値族」「`__eq__` が相手を受ける」しか見ず、部分型を見ない。⇒ `Color` と `enum_item_Color` は異型として弾かれる。

### 1.2 実行時

- メンバーは `enum_item_Color` という名前の合成クラスのインスタンス（`src/interpreter/exec/definitions.rs:456-543`）。
  `bases` は空。
- `x is Color` は `ClassValue::is_a`（クラス名か `bases` の一致・`src/interpreter/value/callables.rs:334`）で
  判定するので偽になる。`mustbe`・`match` の `is Color:` の腕・クラスのフィールドの実行時の型検査も同じ判定を使う。
- ⚠ 起票時は `case Color:` も挙げていたが、`case X:` は**値の比較**（`subject == X`）の構文で、
  通常のクラスでも型では分岐しない。型で分岐するのは `is X:` の腕。

### 1.3 なぜ別の型だったのか

- メンバーに型を付けたのはタスク 2.3（`implementation_logs/type_check_redesign.md` の「2.3 の記録」）。
  **実行時のクラス名に合わせた**だけで、enum 型と別の型にする設計上の理由は記録に無い。
  アップキャストの規則は「別の型にしたせいで `let m: Color = Color.Green` が書けなくなる」のを避けるために後から足した。
- VS Code 拡張のメンバー一覧は、すでにメンバーの型を enum 名で返している
  （`crates/arrow-frontend/src/analyze.rs:214-221` の `"type": name`）。

### 1.4 ゲートが見逃した理由

enum の比較を書いている例題は 2 件（`examples/typing/enum_in_function.ar:70`・`examples/typing/other_typing.ar:183`）。
どちらも `let m = Mode.Write` と**注釈なし**なので、両辺とも `enum_item_*` 型の同型比較になる。
**片側を enum 型で注釈した比較**はどの例題にも無い。

### 1.5 比較の検査を緩める案を採らない理由

`check_equality` に部分型の判定を足せば比較のエラーは消える。
しかし `is Color` / `is Color:` の腕 / `mustbe Color` が実行時に偽になる問題は残る。
型検査で `Color` と呼んでいる値が、実行時に `Color` でないことが根本の原因である。

## 2. 決定

- **D-1**: enum のメンバーの型は **enum 型そのもの**（`Color.BLUE : Color`）。型検査・実行時・拡張の表示をすべて揃える。
- **D-2**: `enum_item_X` は**ソースから廃止**する（2026-10-07 利用者決定）。
  実行時の大域にも置かない。`x is enum_item_Color` は書けなくなり、`x is Color` と書く。
  - ソースで実際に使っていたのは例題 1 行（`examples/typing/other_typing.ar:178`）だけ。
    `TeX_editor` ではコメントの中（`archive/test_git_diff_text.ar:26`、repr の説明）に出てくるだけだった。
- **D-3**: trait の比較の誤検出（`a: Shape` と `Sq`）は**別タスク**（[trait_equality_plan.md](trait_equality_plan.md)）。

## 3. 型を変える前に塞ぐ穴

型検査は enum の表を 1 つしか持っていない。`class_field_details["Color"]` に
**メンバー（`BLUE` …）とインスタンスのフィールド（`value`）が同居**している
（`builder.rs:454-473`）。属性の推論は、インスタンス（`NamedInstance("Color")`）と型の値
（`TypeValOf(NamedInstance("Color"))`）のどちらからでもこの表を引く（`src/type_check/infer.rs:818-838`）。

そのため現状でも次が型検査を通り、実行時に失敗する。

| 書き方 | 型検査 | 実行時 |
|---|---|---|
| `let m: Color = Color.BLUE` の後の `m.PLANE` | ✅ 通る | ❌ `AttributeError: 'enum_item_Color' object has no attribute 'PLANE'` |
| `Color.value` | ✅ 通る | ❌ `AttributeError: class 'Color' has no attribute 'value'` |

⚠⚠ いまは `m` を `Color` と**注釈したときだけ**の穴だが、D-1 でメンバーの型を `Color` にすると
**注釈なしの `let k = Color.BLUE; k.PLANE` にも広がる**（現状は `'enum_item_Color' has no member 'PLANE'` で止まっている）。
⇒ **型を変える前に表を分ける**（タスク 2-1）。

## 4. 実装予定

- フェーズ 1: 実行時を先に直す（型検査が `Color` と言う値を、実行時も `Color` と認めるようにする）
- フェーズ 2: 型検査（表を分けてから型を変える）
- フェーズ 3: Python 実装を追随させる
- フェーズ 4: 例題・文書・拡張

⚠ 型検査だけを先に変えると、`x is Color` を型検査は「常に真」、実行時は「偽」と答える期間ができる。
そのためフェーズ 1 を先に行う。

⚠ **実施時の変更**: `compare_python_impl` は 2 実装の stdout を突き合わせるので、Python 実装の**実行時**の
変更は 3-1 を待たず、1-2 の直後の追随コミット（`2b504f9`）で行った。3-1 は型検査の部分だけになった。

| # | 状態 | コミット |
|---|---|---|
| 1-1 | ✅ | `71df0b5` |
| 1-2 | ✅ | `a96a236`（Python 実装の実行時の追随は `2b504f9`） |
| 2-1 | ✅ | `f04874a` |
| 2-2 | ✅ | `dd4bb5e` |
| 3-1 | ✅ | `d41228d` |
| 4-1 | ✅ | `f80f9f4` |
| 4-2 | ✅ | `57e95b7` |
| 4-3 | ✅ | `41cb92e` |
| 5-1 | ✅ | `81b944f` |
| 5-2 | ✅ | `4d27cc1` |
| 5-3 | ✅ | `139f32b` |
| 6-1 | ✅ | `a58b81a` |
| 6-2 | ✅ | `ed1e3c0` |
| 6-3 | ✅ | `1be5893` |
| 6-4 | ✅ | （本書の更新と同じコミット） |

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| **1-1** | **「enum のメンバーか」を名前ではなく印で判定する**（挙動不変）。`ClassValue` にメンバーの印（例: `enum_of: Option<String>` ＝ 属する enum の名前）を足し、名前の接頭辞で判定している 3 箇所を置き換える: 等値 `src/interpreter/ops/equality.rs:93`・ハッシュ `src/interpreter/ops/hash.rs:200`・`open()` の引数 `src/interpreter/eval/mod.rs:165`（`extract_enum_int`、呼び出し側 `src/interpreter/eval/builtins.rs:834-848`）。⚠ `ClassValue::synthetic` と `deep_clone` は exhaustive なリテラルなので、フィールドを足すとまずここで止まる（既定値はここで決める）。⚠ 等値とハッシュは**同じ印**で判定すること（ずれると「入れたのに引けない辞書」になる） | なし | 小 |
| **1-2** | **メンバーのクラス名を enum 名にし、`enum_item_X` の束縛をやめる**（D-1・D-2）。名前の付け替え: `build_enum_classes`（`definitions.rs:463`）・`make_builtin_enum_class`（`src/interpreter/built_in_types.rs:212`）。束縛をやめる: `exec_enum_def`（`definitions.rs:416-419`）・`vm_enum_def`（`definitions.rs:438-441`）・組み込み enum の大域登録（`built_in_types.rs:332-334`）・`RUNTIME_BUILTIN_NAMES`（`src/type_check/names.rs:39`）。VM の doc（`src/vm/chunk.rs:116`・`src/vm/op.rs:339`・`src/vm/run.rs:655`・`src/vm/compiler/entry.rs:345`）も直す。⚠ モジュールの enum: メンバーのクラスに `module_name` を付けないと `is tags.Color` が当たらない（`qualified_class_matches`）。別名経由の `is t.Color` は `class_id` で比べる（`src/interpreter/ops/typecheck.rs:326`）ので、enum のクラスとメンバーのクラスの `class_id` が違う点に手当てが要る。⚠ テスト: `src/interpreter/tests/mod.rs:309-312`（`enum_item_X` の有無で組み込み enum を見分けている）・`src/interpreter/tests/enum_defaults.rs:21,115-122`。⚠ 例題 `other_typing.ar:178` の `x is enum_item_Color` は**同じコミットで** `x is Color` に書き換える（実行時の束縛が消えるので出力が変わる）。⚠ repr が `<enum_item_Color object …>` から `<Color object …>` に変わる | 1-1 | 中 |
| **2-1** | **enum の表を「型の値から引くメンバー」と「インスタンスから引くフィールド」に分ける**（§3）。`Color.BLUE` は型の値（`TypeValOf`）からだけ、`value` はインスタンスからだけ引けるようにする（`infer_attr`・`member_exists`）。§3 の 2 つの穴が静的エラーになる。⚠ 通常のクラスも型の値とインスタンスで同じ表を引いている（`infer.rs:818-838`）。`const` クラス変数などで同じ穴があるかは未調査（本書の対象外） | なし | 中 |
| **2-2** | **メンバーの型を `NamedInstance("Color")` にする**（D-1）。`builder.rs:464-472`（モジュールの enum は `tags.Color`）。`enum_item_X` の登録をすべて撤去する（D-2）: `builder.rs:433-451`（既知のクラス名・`value` の表）・`builder.rs:111`（モジュール名での修飾）・`src/type_check/stmt/check.rs:617-622`（宣言）・`src/type_check/mod.rs:238`（組み込み enum の宣言）・アップキャスト規則 `type_utils.rs:508-518`・補助関数 `enum_item_type_name` / `enum_of_item_type`（`src/type_check/types.rs:261-276`）。⚠ `check_equality` は**変更しない**。両辺が同じ `Color` になるので通り、別の enum 同士は今までどおり弾かれる。⚠ `match` の死ぬ腕の検査（タスク 5.4）が `case Color.BLUE:` / `is Color:` の腕を誤って弾かないか確かめる | 1-2・2-1 | 中 |
| **3-1** | **Python 実装を追随させる**。実行時 `impl_python/interpreter/interpreter.py:188, 543-544, 1874-1885`、型検査 `impl_python/type_check/stmt.py:277, 384`。追跡用の git SHA を更新する。⚠ Python 実装は異型の等値を静的に検査しないので、発端のエラーは出ない。ただし `is Color` が偽になる問題は同じようにある | 1-2・2-2 | 小 |
| **4-1** | **例題**。成功例（新規・`examples/typing/`）: enum 型の引数・注釈つき `let` とメンバーの `==` / `!=` / `in`、`is Color`・`is Color:` の腕・`mustbe Color`、モジュールの enum（`examples/basics/namespace_modules/tags.ar` の `Level`）、組み込み enum（`let m: FileOpenMode = FileOpenMode.read` との比較）。エラー例（`_error`）: 別の enum 同士の比較・`m.PLANE`・`Color.value`・`x is enum_item_Color`。既存の `examples/typing/enum_member_type.ar:39`（「メンバーの型は enum_item_<名前>」）と `enum_member_type_error.ar:5` のコメントを直す | 2-2 | 小 |
| **4-2** | **文書**。`docs/grammar/06_classes_traits.md:449`・`docs/language_comparison.md:145`（`x is enum_item_Color` を `x is Color` に）・`type-checking` スキルの enum の節。`implementation_logs/type_check_redesign.md` の「2.3 の記録」に、本書で置き換えた旨を追記する | 2-2 | 小 |
| **4-3** | **VS Code 拡張**。型検査が変わると wasm も変わるので `make-vsix.ps1` で VSIX を作り直す。ホバーでメンバーの型が `Color` と出ることを確かめる | 2-2 | 小 |

### フェーズ 5: メンバーを値として扱う（2026-10-07 追加・利用者の指示）

D-1 の後も、型検査はメンバー（`Color.BLUE`）を**代入できる場所**として扱っていた。

| 書き方 | 5-1 の前（型検査） | 5-1 の前（実行時） |
|---|---|---|
| `Color.BLUE = Color.PLANE` / `Color.BLUE += 1` | 通る | `TypeError` |
| `Color.BLUE.value = 5` | 通る | **通り、共有のメンバーそのものが 5 になる** |
| `mut m = Color.BLUE` の後の `m.value = 5` / `m.value += 1` | 通る | 通る（写しが書き換わる） |

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| **5-1** | **メンバーは値で、変数ではない**。①メンバーへの代入・複合代入を `AssignToEnumMember` で弾く（`check_immutable_field_assign`）。②`value` を不変のフィールドとして登録する（`class_fields[enum] = {value: false}`）ので、`Color.BLUE.value = 5` / `m.value = 5` は既存の `AssignToImmutableField` になる。③実行時もメンバーのクラスの `value` を不変にする（`build_enum_classes` / `make_builtin_enum_class`）。型義務 `N6`〜`N8`。⚠ 変数の付け替え（`mut c = Color.BLUE` の後の `c = Color.RED`）は値の書き換えではないので通す。⚠ Python 実装は実行時にはすでに弾いている（メンバーは不変のインスタンス・名前空間への代入は `AttributeError`）。型検査にはフィールドの書き換えの検査自体が無いので足していない | 2-2 | 小 |
| **5-2** | **`const` への代入を、どの経路でも同じ規則で弾く**（利用者の指示）。enum のメンバーを**暗黙に `const`** として扱い、クラスの `const` と同じ `AssignToConst` にする（5-1 の `AssignToEnumMember` は統合して廃止）。判定は `is_const_member`（クラスの `FieldKind::Const`・enum のメンバー）の 1 つで、クラス名経由（`Counter.LIMIT = ..`）・インスタンス経由（`c.LIMIT = ..`）・`__init__` の中の `self.LIMIT = ..`・複合代入のどれにも効く。⚠ 以前はインスタンス経由だけが（`AssignToImmutableField` で）弾かれ、クラス名経由と `__init__` の中は実行時の `TypeError` まで通っていた（`__init__` の中は `let` フィールドの初回代入のために検査を丸ごと免除していた・`const` の判定はその免除より先に見る）。⚠ enum の `value` は `const` ではなく不変のフィールド（`FieldKind::Let`）に登録し直した（`m.value = 5` は `AssignToImmutableField` のまま）。型義務 `M11` / `M12`・例題 `examples/classes/const_member_assign_error.ar` | 5-1 | 小 |
| **5-3** | **あらゆる `const` を書き換えられなくする**（利用者の指示）。①**`const` の中身**（要素・フィールド・書き換えるメソッド）も書き換えられない: 書き込み先の経路のどこに `const` があっても `ModifyConst`（`const_on_path`）。`path_is_mutable` もその経路を不変と答えるので、`mut` の仮引数への受け渡し・`for` のループ変数も同じ規則に乗る。②**trait の `const`**: `is_const_member` が基底の trait（`trait_field_details`）も見る。③**モジュールの `const`**（`m.K = ..` / 別名 `t.K` / 再エクスポート）: `module_member_types` が `module_consts` を集める（エディタはモジュールのメンバーが確定しないので見ない）。④経路の型は**診断を出さずに**推論する（`infer_quietly`・`Diagnostics::mark` / `rollback`）。⚠ 副産物: `path_is_mutable` が型の値・モジュールの根を「値の束縛」と扱っていたため、`static mut` のリストへの `Counter.items.append(..)` が誤って弾かれていた（実行時は通る）。代入の検査と同じ区別にして直した。⚠ enum のメンバーの中身（`Color.BLUE.value = 5`）は `ModifyConst` になった（5-1 では `AssignToImmutableField`）。型義務 `M13`〜`M17`・例題 `const_member_assign_error.ar`（拡張）/ `examples/basics/module_const_assign_error.ar` / `static_mut_assign.ar`（拡張） | 5-2 | 中 |

### フェーズ 6: import したモジュールのグローバル変数の型と属性（2026-10-08 追加・利用者の指示）

利用者の指摘: **`const` への書き換え禁止は型ではなく属性によるもの**。import した後も、モジュールの
グローバル変数の型と属性は正しく認識されなければならない。5-3 はモジュールの `const` を、メンバーが
確定したモジュールの「型」（`Namespace` の 2 つ目）を経由した別表で見ていたので、型の分からない
VS Code 拡張では効かず、しかも `let` / `mut` の属性も型も持っていなかった。実測（6-1 の前）:

| モジュールでの宣言 | `g.X` の型 | `g.X = 9` | `g.X[0] = ..` / `g.X.append(..)` |
|---|---|---|---|
| `const`（注釈あり） | ✅ | ✅ 静的エラー | ✅ 静的エラー |
| `const` / `let`（注釈なし） | ❌ 不明 | — | — |
| `let`（注釈あり） | ✅ | ❌ 実行時エラー | ❌ **完走して書き換わる** |
| `mut`（注釈ありでも） | ❌ 不明 | ❌ 実行時に `cannot set attribute on non-instance`（`mut` なのに書けない） | 型が不明で無検査 |

利用者の決定: ①`mut` は外から代入できる（モジュールの大域そのものを書き換える・CPython と同じ）。
`let` / `const` は静的エラー。②`from g import X` の `X` は付け替え不可のまま、中身は元の宣言の属性に従う。

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| **6-1** | **型検査**: `VarAttr`（`Const` / `Let` / `Mut` / `Imported`）と `ModuleVars` を新設し、名前空間の型の 3 つ目にグローバル変数の属性を載せる。型と属性はモジュールの本体を検査したスコープから取る（`annotate_module_body` が捨てる前に `module_globals` へ控える）ので、注釈の無い変数・`mut` にも型が付く。書き込みの判定を属性で行う（`g.X = ..`: `const` → `AssignToConst`、`let`・取り込んだ名前・関数など → `AssignToImmutable`、`mut` → 可・値は変数の型で検査）。`VarInfo::contents_mutable` で `from g import X` の中身を元の属性に従わせる。5-3 の `module_consts` は撤去。⚠ `freeze` と型ガードの宣言し直しも両方の可否を引き継ぐ（最初は `freeze` が `contents_mutable` を倒さず、テストが 1 件落ちた）。副産物: モジュールの `mut` に型が付き、`c.count + c.STEP` が `IBIN_SS Add` に型特化された | 5-3 | 中 |
| **6-2** | **実行時**: `attr_assign_evaled`（ツリーウォーク・VM 共通）に名前空間の腕を足し、`mut` の名前はモジュールの大域を書き換える（`namespace_member` が読む所）。⚠ impl_python は名前空間が import 時の写しで今の値を読む仕組みも無いので追随しない（既知差分）。型義務 `I4`〜`I7`・例題 `module_mut_state.ar`（拡張）/ `module_global_assign_error.ar` | 6-1 | 小 |
| **6-3** | **拡張**: `--emit-stubs` が展開・型検査をして（`TypeChecker::module_globals`）、Arrow のモジュールのスタブにグローバル変数を `const LIMIT: int = Undefined` の形で書く（`stub_gen::generate_editor_stub`）。型は必ず注釈で書く（注釈の型で束縛される）。注釈の無い変数は推論した型（注釈として読み戻して同じ型になるものだけ）。型の分からない変数は書かない。部分コンパイル・js-proc のスタブは変えない。確認: スタブを読ませたデバッグランナーの診断の件数が、モジュールの例題 6 本で CLI と一致。⚠ **撤去済み**（editor_import_resolution_plan.md 5-1）: 拡張も import 先を CLI と同じ処理で読むようになり、スタブの仕組みごと不要になった | 6-1 | 小 |
| **6-4** | 文書（`09_imports.md` の「グローバル変数の型と属性」・`06_classes_traits.md`・`language-differences.md`・type-checking / vscode-extension-dev スキル）・VSIX・codebase-map | 6-3 | 小 |

## 5. ゲート

| タスク | 走らせるもの | 期待 |
|---|---|---|
| 全タスク | `scan_examples` / `force_gate` / `compare_python_impl`（`test_build_gate` は前段で自動） | 緑 |
| 1-1 | `compare_outputs -A <直前のコミットの exe>`・`hash_eq_identity` | **差分 0**（挙動不変の主張） |
| 1-2 | `compare_outputs -A`・`compare_bytecode -A`（VM の `Op::EnumDef` を触るため）・`hash_eq_identity` | 差分は enum の表示（repr・メッセージ中の型名）と `other_typing.ar` だけ |
| 1-2・2-2 | `stale_doc_refs` | 消した識別子（`enum_item_type_name` など）がコメントに残っていない |
| 2-1・2-2 | `type_obligations`・`compare_wasm_frontend` | 退行なし・2 実装の診断が一致 |
| 4-1 | `syntax_cov` | 新しい例題が注釈つき enum の比較を埋めている |

⚠ A/B 系は**直前のタスクのコミット**からビルドした exe を基準にし、同じ exe 同士で差分 0 になることを先に確かめる。

## 6. 対象外・未調査

- trait 型の値と、その trait を実装するクラスの値の比較の誤検出 → [trait_equality_plan.md](trait_equality_plan.md)
- 別のモジュールにある同名の enum（`a.Color` と `b.Color`）のメンバーは、今もクラス名だけで等値を判定している。
  そのため値が同じなら等しくなる（既存の挙動）。1-1 の印で区別できるようになるが、変えるならハッシュも同時に変えること
- 通常のクラスで、型の値とインスタンスが同じ表を引くことによる穴（2-1 の ⚠）
- ~~メンバーへの代入（`Color.BLUE = Color.PLANE`）は型検査を通り、実行時の
  `TypeError: cannot assign to class variable 'BLUE' (declared const)` で止まる~~ → **5-1 で対応**
- ~~通常のクラスの `const` クラス変数への代入（`Counter.LIMIT = 5`）は、今も型検査を通って実行時の
  `TypeError` で止まる~~ → **5-2 で対応**
- ~~trait の `const` と import したモジュールの `const` への代入は 5-2 の対象外~~ → **5-3 で対応**
- ⚠ **実行時は `const` の中身を守っていない**（5-3 は静的な検査）。型検査を通らない経路
  （`Any` を経由した値など）から `C.L.append(..)` すると、実行時はクラスで共有する `const` が書き換わる。
  守るには値そのものに不変の印が要る（今はインスタンスのフィールドの可変フラグしか無い）
- trait の `const` は実行時にクラス名・インスタンスから読めない（`class 'C' has no static field 'TK'` /
  `'C' object has no attribute 'TL'`）。書き込みとは別の既存の問題
- ~~import したモジュールのメンバーへの書き込みは、`mut` でも実行時に失敗する~~ → **6-2 で対応**
- 型の分からないモジュールのグローバル変数（推論できない初期値）は、拡張のスタブに書かれない（6-3）。
  CLI では属性が分かるので弾ける
- `compare_wasm_frontend` はスタブを使わない比較なので、拡張でモジュールの変数を検査できることは
  このゲートでは見ていない（6-3 は作業用ディレクトリで `--emit-stubs` → デバッグランナーで確認した）
- ⚠ Python 実装では enum のメンバーを辞書のキーに使うと `KeyError` になる（`2045111` でも同じ。
  Rust 実装は正しく引ける）。例題では辞書のキーに使っていない
