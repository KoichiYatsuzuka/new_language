# Python→Arrow 変換機能（`python_converter`）の Python 文法カバレッジ調査

- 初版: 2026-07-21 / 更新: 2026-07-22（フィードバックを受けてトリアージ）
- 対象: [`src/python_converter/`](src/python_converter/)（`import[py]` が使用する、PyO3 インタープリタを介さない Python→Arrow AST 翻訳器）
- 手法: 全ソース精査 + 代表ケースを `target/debug/arrow.exe` で実機検証 + Arrow 側 AST/パーサ/インタープリタの対応文法確認
- 検証列凡例: ✔実機 = 実際に実行して確認 / ソース = ソースコード読解により確定

---

## 対象機能の特定

「Python コードをインタープリタを介さずに翻訳する機能」＝ [`src/python_converter/`](src/python_converter/)。
Python ソースを rustpython-parser でパースし、Arrow の AST（`Stmt`）へ**直接変換**する。`import[py]` がこれを使う。実行時に PyO3（CPython）を呼ぶ `import[py-int]`（[`src/interpreter/py_interop.rs`](src/interpreter/py_interop.rs)）とは別物であり、本調査の対象外。

- エントリポイント: `convert_python_source()` [`mod.rs`](src/python_converter/mod.rs)
- 文変換: [`statements.rs`](src/python_converter/statements.rs) / クラス: [`classes.rs`](src/python_converter/classes.rs) / 式: [`expressions.rs`](src/python_converter/expressions.rs) / 型注釈: [`annotations.rs`](src/python_converter/annotations.rs) / 補助: [`utils.rs`](src/python_converter/utils.rs)

## 結論

すべての Python 文法には未対応。ただし Arrow 側 AST を精査した結果、**当初「未対応」とした項目の多くは Arrow に対応する文法・AST ノードが実在し、変換器を拡張すれば対応可能**であることが判明した。以下にトリアージ結果を示す。

補足: 変換は**モジュール単位**で、1 箇所でも未対応構文があると `import` 全体が失敗する。

---

## ステータス凡例

| 記号 | 意味 |
|---|---|
| 🟢 対応予定 | Arrow に対応文法が実在し、変換方針が確定。下記「修正予定リスト」へ |
| 🟠 暫定エラー | 当面は明示エラーで拒否。Arrow 言語仕様の検討後に再検討 |
| 🔴 非対応確定 | 明示エラー。サポート予定なし（代替手段あり／モデル差） |
| 🔵 仕様確定 | 意図的動作として仕様化（バグではない） |
| 🟡 懸念/要判断 | 対応可能だが設計上の懸念がある、または提案された代替策が不適切 |
| ⚪ 未トリアージ | 今回の指示対象外。開いたままの課題 |

---

## 🟢 修正予定リスト（対応方針確定分）

各項目 `[ ]` は未着手。実装時にチェックする。

### [x] 1. デフォルト引数 `def f(x, y=10)`【実装済 2026-08-28】

- 対象: [`classes.rs` `convert_params()`](src/python_converter/classes.rs)（+ `extract_param_types` 近傍）
- 現状: `Param { default: None, ... }` を常に生成し、デフォルト値を捨てている。
- Arrow 側: `Param.default: Option<Expr>` が実在（[`ast.rs:156`](src/ast.rs#L156)）。`bind_args` は未割当スロットをデフォルトで埋める（[`args.rs:180`](src/interpreter/functions/args.rs#L180)）。
- 変換方針:
  - `py::Arguments` の `args`/`posonlyargs` は末尾から `args.defaults` に、`kwonlyargs` は `kw_defaults` に対応する。各 `ArgWithDefault.default: Option<Box<Expr>>` を `convert_expr` して `Param.default` に載せる。
  - 位置引数のデフォルトは「後ろ詰め」で対応付く点に注意（`f(a, b=1, c=2)` なら defaults は `[1,2]` が末尾2個に対応）。
- 難易度: 低。
- テスト: `def greet(name, greeting="hi")` → `m.greet("x")` が `"hi x"` を返すこと（現状は引数不足エラー）。

**実装結果**: 変換器側は**想定より簡単**だった。rustpython 0.4 の `ArgWithDefault` は
`{ def: Arg, default: Option<Box<Expr>> }` と**引数ごと**にデフォルト式を持つので、
「位置引数のデフォルトは末尾詰めで対応づける」処理は不要。`convert_expr` して
`Param.default` に載せるだけ（`convert_params` の 2 ループ）。

**⚠⚠ 変換器だけでは動かなかった — 静的型検査にもう 1 層あった**:
`check_fn_type_call`（[`call_check.rs`](src/type_check/call_check.rs)）が
`arg_data.len() != params.len()` で弾いており、**デフォルトの有無を見ていなかった**
（`FnTypeParam` に情報自体が無かった）。`FnTypeParam` に `has_default` を追加し、
必要数を `!has_default` の個数で数えるよう修正した。

- **これは py 固有の問題ではなく、`.ar` のネイティブモジュールでも同じく壊れていた**。
  `import[ar] lib` して `lib.greet("bob")` が `'greet' takes 2 argument(s) but 1 were given`
  で落ちる（インタープリタは `evaluated_defaults` で正しく埋めるので、**静的検査だけが嘘をつく**形）。
  ⇒ 本項目の副作用としてこちらも直った。
- エラー文言も `takes 1 to 3 argument(s) but 0 were given` と範囲表示になる（上限も従来どおり検査）。
- ⚠ `impl_python` にも同じ経路（`type_check/call_check.py` の `_check_fn_type_call`）があるが、
  **ユーザー方針により impl_python は触らない**（同期点が約 100 コミット前で古いため）。
  `compare_python_impl.ps1` は**この差分を踏む例題が無いので緑のまま**（54/54 一致・実測確認済み）。
  ⇒ 将来 impl_python を同期するときの積み残しとして記録しておく。

**⚠ 意味差: デフォルト値の評価時期**:
Python は `def` の実行時に**1 回**評価してその値を共有する。Arrow は
`exec_fn_evaled` が**呼び出しごと**に評価する（[`execution.rs`](src/interpreter/functions/execution.rs) の
`evaluated_defaults`）。

- リテラル（`0` / `"hi"` / `None` / `(1,2)`）は**完全に同じ**。実用上ほぼこれ。
- ⚠ **可変デフォルト**（`def f(xs=[])`）だけ結果が違う。CPython は `1, 2, 3…`（同じリストを共有＝
  有名な罠）、Arrow は毎回 `1`。**Arrow 側が「普通に期待される」挙動**で、罠に依存したコードだけが
  差を踏む。⇒ **エラーにはせず許容**（エラー化すると無害な `def f(xs=[])` を大量に拒否するため）。
  必要なら後から「可変リテラルのデフォルトは明示エラー」に倒せる。
- ⚠ 名前参照のデフォルト（`def f(x=CONST)`）も Arrow は呼び出し時に読み直す。

**⚠ 副作用: 未対応のデフォルト式が表に出る**:
実装前は `default: None` 固定＝**デフォルト式を丸ごと捨てて**いたので、`def h(k=lambda: 1)` は
黙って読み込めていた（呼ぶと引数不足で落ちる）。今はデフォルト式も `convert_expr` に通すので、
lambda（項目 26）・f-string（項目 19）などがその場で明示エラーになる。**サイレント欠落の解消**。

**例題**: [`examples/interop/py_defaults.ar`](examples/interop/py_defaults.ar) +
[`test_modules/py_defaults.py`](examples/interop/test_modules/py_defaults.py)
（⑦の可変デフォルトを除き CPython と出力一致を突き合わせ済）/
[`examples/interop/py_defaults_error.ar`](examples/interop/py_defaults_error.ar)（lambda / f-string デフォルト）。

### [x] 2. 変数の再代入 `x = 1; x = 2` / ループ内カウンタ【実装済 2026-08-28】

- 対象: [`statements.rs`](src/python_converter/statements.rs) の文変換全体（`Assign`/`AnnAssign` と巻き上げ処理）
- 現状: すべての `x = expr` を `Stmt::Mut`（新規宣言）に変換 → Arrow は再宣言を禁止するため実行時 `NameError: variable 'x' is already declared`。既存の巻き上げは `if` ブランチ内代入のみに限定。
- Arrow 側: 宣言は `Stmt::Mut`、再代入は `Stmt::Assign`（[`ast.rs:580`](src/ast.rs#L580)）。両者は別ノード。
- 変換方針（**スコープ単位の完全巻き上げに置き換える**）:
  1. 関数本体（およびモジュール本体）ごとに、その中で単純名前代入されるすべての変数名を再帰収集（`if`/`for`/`while`/`try`/`block` の全ネストを走査）。
  2. 収集した名前を、パラメータ名・`for` ループ変数を除外したうえで、スコープ先頭に `mut name = None` として一度だけ宣言（hoist）。
  3. 以降、すべての `x = expr` を `Stmt::Assign`（再代入）に変換する（`Mut` は使わない）。
  - これで Python の「関数内で一度でも代入された名前は関数全体で可視」というスコープ規則を、Arrow のブロックスコープ上で正しく再現できる。既存の `collect_if_branch_assigns` ベースの部分巻き上げはこの汎用版で置換する。
- 難易度: 中。
- 懸念: hoist 初期値 `None` により型が `Any`/nullable 化し静的型検査が緩む。機能的には正しいが型精度は下がる（許容範囲と判断）。
- テスト: `x=1; x=2; x=x+10` → `13`。`while n>0: n=n-1` が回ること。

**実装結果**: 計画（INF-A）どおり**スコープ単位の完全巻き上げ**に置き換えた。
`statements.rs` の巻き上げ機構を丸ごと差し替え:

| 旧 | 新 |
|---|---|
| `convert_stmts_with_hoist` / `convert_stmt_in_hoist_ctx` / `convert_stmts_hoisted_branch` / `collect_if_branch_assigns` | `convert_scope` / `convert_stmts(…, declared)` / `collect_assigned_names` / `assign_or_declare` |

- `convert_scope(stmts, filename, params)` … **スコープの入口**（モジュール本体・関数本体・メソッド本体）。
  代入名を再帰収集 → 先頭で `mut name = None` → 以降すべて `Stmt::Assign`。
- `convert_stmts(stmts, filename, declared)` … **同一スコープ内**の本体（if/for/while/try）。
  `declared` をそのまま引き回す。
- `seen` をパラメータ名で初期化するので、**パラメータは巻き上げないが再代入扱いにはなる**。

**⚠ 旧実装のドリフト（これが項目 2 の本体）**: 巻き上げは「**トップレベルの `if` のブランチ内代入だけ**」で、
`for` / `while` / `try` の本体に降りた時点で `convert_stmts(…)` が巻き上げ集合を捨てていた。
そのため同じ名前がまた `Stmt::Mut` になって `already declared` で落ちていた。
⇒ 例題 ①〜④・⑥〜⑧ はすべて**旧実装では動かない**形。

**収集しないもの（意図的）**:
- `for` のループ変数・`except ... as e` … `=` ではない。`Stmt::For` / ハンドラ側が自前で宣言する。
- `x += 1`（`AugAssign`）… Python でも事前の束縛が必要なので、その `=` から拾われる。
- 入れ子の `def` / `class` の本体 … **別スコープ**（⑧ で確認済み）。

**⚠ 残る意味差（1 件・実測）**: `=` で代入した名前を **`for` のループ変数にも使い、ループ後に読む**とき。
Arrow の `for` は自前スコープでループ変数を束縛する（＝巻き上げた外側の変数は隠れるだけ）ため、
ループ後は代入時の値に戻る。CPython は最後の要素。

```python
def f(xs):
    i = -1
    for i in xs: pass
    return i        # Arrow: -1 / CPython: 3
```

⇒ **エラー化していない**（`i` をループ後に読まない限り無害で、`i = 0` の後に `for i in …` と書く
コードを丸ごと拒否することになるため）。必要なら「同名衝突は明示エラー」に後から倒せる。
なお `=` が無い純粋なループ変数（⑩）は巻き上げ対象外なので **CPython と一致**する。

**例題**: [`examples/interop/py_reassign.ar`](examples/interop/py_reassign.ar) +
[`test_modules/py_reassign.py`](examples/interop/test_modules/py_reassign.py)
（12 ケース中 ⑨ の 1 件のみ CPython と相違。他 11 件は一致を突き合わせ済）。
エラー化した経路が無いため `_error` 例は無し。

### [x] 3. 添字/キー代入 `a[i] = x` / `d[k] = v`（+ 複合 `a[i] += 1`）【実装済 2026-08-28】

- 対象: [`statements.rs` `convert_stmt()`](src/python_converter/statements.rs) の `Assign` / `AugAssign` アーム
- 現状: 代入ターゲットが `Subscript` の場合 `unsupported assignment target` エラー。
- Arrow 側: 添字代入は `Stmt::AttrAssign { target: <Subscript式>, value }` で表現する（パーサ `finish_expr_stmt` のコメント「`d["k"] = v`」参照 [`assignment.rs:92`](src/parser/stmts/assignment.rs#L92)）。複合は `Stmt::AttrCompoundAssign`。
- 変換方針:
  - `Assign` のターゲット `py::Expr::Subscript` を `Attribute` と同様に扱い、`target = convert_expr(subscript)`（= `Expr::Subscript`）として `Stmt::AttrAssign` を生成する。
  - `AugAssign` のターゲット `Subscript` も同様に `Stmt::AttrCompoundAssign` を生成する。
- 難易度: 低（既存 `Attribute` アームの分岐を Subscript にも広げるだけ）。
- テスト: `d["k"]=5; return d`、`xs[0]+=1`。

**実装結果**: 計画どおり。Arrow は添字代入の専用ノードを持たず**属性代入と同じ `Stmt::AttrAssign`**
（`target` に代入先の**式**を置く）で表すので、`Attribute` を受けていた 3 アーム
（`Assign` / `AugAssign` / `AnnAssign`）を `Subscript` にも広げるだけで済んだ。
入れ子（`box["k"][0] = v`）も target が入れ子の `Expr::Subscript` になるだけでそのまま通る。

**確認（8 ケース・すべて CPython と出力一致）**: dict キー代入／読んで書き戻し／list 添字代入／
複合代入 `+=` `*=` `-=`／入れ子／属性+添字（`self.data[i] += 1`）／ループ内での dict 構築。

**⚠ 実在モジュールが読めるようになった**: `test_modules/py_calculator.py` は
`Container.__setitem__` の `self.data[key] = value` 1 行のせいで `import[py]` が
**丸ごと失敗**していた（モジュール単位変換なので 1 箇所の未対応構文が import 全体を殺す）。
本項目で変換が通り、`Calculator` が使えるようになった
（`sum_dict` だけは Arrow に `sum` 組込が無いため別途 `NameError`）。

**⚠ 本項目の作業中に項目 2 の不具合を 1 件修正**:
`collect_assigned_names` が `if __name__ == "__main__":` の中まで降りていた。
`convert_stmt` はこのブロックを**丸ごと捨てる**ので、中の代入を巻き上げると
「代入されないのに `mut name = None` だけ残る」モジュール変数が生まれ、取り込み側の同名変数と
衝突して `already declared` になる（`py_calculator.py` のガード内 `c = Calculator(...)` で実際に踏んだ）。
`is_main_guard` で降りないように修正し、退行例を `py_reassign.ar` の ⑪ に追加した。

**例題**: [`examples/interop/py_subscript.ar`](examples/interop/py_subscript.ar) +
[`test_modules/py_subscript.py`](examples/interop/test_modules/py_subscript.py)。
新しいエラー経路が無いため `_error` 例は無し（スライス代入 `a[1:2] = xs` は項目 4 のエラーになる）。

### [x] 4. スライス `a[1:2]` / `a[::2]`【実装済 2026-08-28】

- 対象: [`expressions.rs` `convert_expr()`](src/python_converter/expressions.rs) の `py::Expr::Slice` アーム
- 現状: `slice expression is not supported` エラー。
- Arrow 側: `Expr::Slice { begin, end, step }` が実在（[`ast.rs:387`](src/ast.rs#L387)）。添字式の内側で生成される想定。
- 変換方針: `py::Expr::Slice { lower, upper, step }` → `Expr::Slice { begin: lower.map(convert), end: upper.map(convert), step: step.map(convert) }`。`Subscript` のインデックスとしてそのまま入る。
- 難易度: 低。
- テスト: `xs[1:3]`, `xs[::2]`, `xs[:-1]`。

**実装結果**: 計画どおり 1 対 1 の写し替えで済んだ。rustpython の `ExprSlice` は
`lower` / `upper` / `step` を 3 つとも `Option` で持ち、省略部分は `None`。
Arrow の `Expr::Slice { begin, end, step }` も同じ形なのでそのまま変換できる。

**⚠ Arrow のスライス意味論は Python 互換だった**（18 ケースすべて CPython と出力一致）:
負のインデックス（`a[:-1]` / `a[-2:]`）・負のステップ（`a[::-1]` / `a[4:1:-1]`）・
範囲外の切り詰め（`a[1:100]`）・逆転した境界（`a[3:1]` → 空）・`str` / `tuple` への適用・
境界が定数でない式・スライス結果への再スライス。
`a[::0]` の `ValueError: slice step cannot be zero` は**文言まで一致**する。

**⚠ スライス代入 `xs[1:3] = [...]` は項目 3 と揃って初めて成立する**:
項目 3 が代入 target に**式**（`Expr::Subscript`）を置けるようにし、本項目がその index を
`Expr::Slice` にする。例題 ⑧ で固定した。

**例題**: [`examples/interop/py_slice.ar`](examples/interop/py_slice.ar) +
[`test_modules/py_slice.py`](examples/interop/test_modules/py_slice.py)。
新しいエラー経路が無いため `_error` 例は無し。

### [x] 5. クラス変数 `class C: count = 0`【実装済 2026-08-28】

- 対象: [`classes.rs` `convert_class()`](src/python_converter/classes.rs) のクラス本体 `Assign`/`AnnAssign` アーム
- 現状: クラス直下の `x = value` を `FieldKind::Const`（不変・共有）に変換 → Python の可変クラス属性が const になる。`__` 始まりは無視。
- Arrow 側: 共有**可変**クラス変数は `FieldKind::StaticMut`（`static mut name: Type [= default]`、[`ast.rs:1003`](src/ast.rs#L1003)）。共有不変は `Const`。
- 変換方針:
  - クラス直下の `x = value` および `x: T = value` を、Python 意味論（可変・共有）に合わせて `FieldKind::StaticMut` にマップする（`default: Some(...)`、`type_ann` は注釈があればそれ、無ければ `"Any"`）。
  - 定数として使いたいものと区別できないため、既定は可変（`StaticMut`）に倒す。
- 難易度: 低〜中。
- 懸念: `type_ann` は `Field` で必須（`String`）。注釈なしは `"Any"` で埋める。定数意図の属性も `StaticMut` になる（Python 上は可変なので忠実）。
- テスト: `class C: count = 0` 定義後にインスタンス/クラス経由で `count` を読み書き。

**実装結果**: `convert_class` のクラス本体 `Assign` / `AnnAssign` 両アームで
`FieldKind::Const` → `FieldKind::StaticMut` に変更しただけ（`type_ann` は注釈があればそれ、
無ければ `"Any"`）。以前は `Counter.count = ...` が
`TypeError: cannot assign to class variable 'count' (declared const)` で落ちていた。

**確認（17 ケース中 15 件 CPython 一致）**: クラス経由の読み取り／ドライバ側からの書き換え／
メソッド内からの読み書き（全インスタンス共有）／`self` 経由の読み取り／
可変オブジェクト（`items = []`）の中身の共有／注釈つきクラス変数。

**⚠ 残る意味差 1 件（2 ケース）— `self.x = ...` のインスタンス属性新設**:
Python の `self.count = 99` は**クラス属性を隠すインスタンス属性を新設**するので
`Counter.count` は変わらない。Arrow の `static mut` は**単一の記憶場所**でインスタンス側の層が
無いため、共有変数そのものを書き換える。

| | Arrow | CPython |
|---|---|---|
| `(self.count, Counter.count)` | `(99, 99)` | `(99, 102)` |
| その後の `Counter.count` | `99` | `102` |

⇒ **変換器では埋められないモデル差**（Arrow に「インスタンス属性の動的追加」が無い）。

**例題**: [`examples/interop/py_classvar.ar`](examples/interop/py_classvar.ar) +
[`test_modules/py_classvar.py`](examples/interop/test_modules/py_classvar.py)。
新しいエラー経路が無いため `_error` 例は無し。

### [ ] 6. `*args`（可変長位置引数）

- 対象: [`classes.rs` `convert_params()`](src/python_converter/classes.rs) + 本体の識別子書き換え
- 現状: `Param { name: "*args", variadic: false }` という不正なパラメータに化けている。
- Arrow 側: `Param.variadic = true`・名前 `"..."`、本体からは `local::args`（`Expr::LocalVar("args")`）で参照する（[`args.rs:196`](src/interpreter/functions/args.rs#L196)）。
- 変換方針:
  - vararg を `Param { name: "...", variadic: true, mutable: true, type_ann: Some("list[Any]") }` に変更。
  - **関数本体を走査し、Python の vararg 名（例 `args`/`rest`）への `Ident` 参照を `Expr::LocalVar("args")` に書き換える**識別子リライトを追加する。
- 難易度: 中（本体リライトが必要）。
- 懸念: Python の `*args` はタプル、`local::args` はリスト。添字/反復は互換だが `tuple` 固有操作は差異あり。
- テスト: `def f(*xs): return xs[0]` を `f(10,20)` で呼び `10`。

### [ ] 7. `**kwargs`（可変長キーワード引数）

- 対象: [`classes.rs` `convert_params()`](src/python_converter/classes.rs) + 本体の識別子書き換え
- 現状: `**kwargs` をパラメータから除外。余剰キーワードは `kwargs` dict に自動注入される仕組みが既存（[`execution.rs:143`](src/interpreter/functions/execution.rs#L143), `bind_args_relaxed`）。既存例 [`py_additional_param.ar`](examples/archived/py_additional_param.ar) で動作実績あり。
- 変換方針:
  - Python の kwarg 名が `kwargs` 以外（例 `**opts`）の場合、本体の `Ident("opts")` を `Ident("kwargs")` に書き換える。
- 難易度: 中。
- 懸念: 余剰キーワードが 1 つも渡されないと `kwargs` 変数が未定義になり、本体参照で `NameError`。**空でも空 dict を注入する**ようインタープリタ側 [`execution.rs`](src/interpreter/functions/execution.rs) の条件（`!extra_kwargs.is_empty()`）緩和が別途必要。
- テスト: `def f(**kw): return kw` を `f(a=1,b=2)` と `f()` の両方で呼ぶ。

### [ ] 8. `match` 文（値/ワイルドカードパターンのサブセット）

- 対象: [`statements.rs` `convert_stmt()`](src/python_converter/statements.rs) の `py::Stmt::Match`
- 現状: `'match' statement is not supported` エラー。
- Arrow 側: `Stmt::Match { subject, arms }`、`MatchPattern::Case(Expr)`（`==` 比較）/ `IsType(String)`（型検査）、`case _:` はワイルドカード（[`ast.rs:492`](src/ast.rs#L492)）。1 つの match 内で case と is の混在は不可。
- 変換方針:
  - Python の `case <リテラル/値>:` → `MatchPattern::Case(convert_expr)`、`case _:` → `MatchPattern::Case(Expr::Ident("_"))`。
  - クラスパターン・キャプチャ・シーケンス/マッピングパターン・OR パターン・ガード（`if`）は**明示エラー**にする。
- 難易度: 中。
- 懸念: Arrow の match は値等価/型検査のみ。Python の構造的パターンの大半は非対応（サブセット対応）。

### [ ] 9. ジェネレータ（`def` + `yield`、サブセット）

- 対象: [`statements.rs`](src/python_converter/statements.rs) の関数定義変換 + [`expressions.rs`](src/python_converter/expressions.rs) の `Yield`
- 現状: `yield` 式を `yield expression ... is not supported` でエラー。
- Arrow 側: ジェネレータは `gen` キーワード=`Stmt::GenDef`、本体で `Stmt::Yield(Expr)`（[`ast.rs:676`](src/ast.rs#L676),[`ast.rs:721`](src/ast.rs#L721)）。呼び出しで `Value::Generator` を返す。
- 変換方針:
  - 関数本体に `yield` 文を含む `FunctionDef` は `Stmt::FnDef` ではなく `Stmt::GenDef` として生成し、`yield x` 文を `Stmt::Yield` に変換する。
  - `yield_type` は戻り注釈 `Generator[T]`/`Iterator[T]` から `T` を抽出、無ければ `None`。
  - `yield from` と yield 式の値利用（`x = yield`）は**明示エラー**。
- 難易度: 中。
- 懸念: `.send()`/双方向通信・`yield from` は非対応（サブセット）。

### [ ] 10. 型エイリアス `type X = ...`

- 対象: [`statements.rs`](src/python_converter/statements.rs) の `py::Stmt::TypeAlias` + [`annotations.rs`](src/python_converter/annotations.rs)
- 現状: `Ok(None)`（黙って無視）。
- Arrow 側: (a) `alias name: RHS`＝パース時展開の透過エイリアス。ただし**AST ノードは持たず**（`Stmt::Pass` を返し `parser.aliases` に登録）、変換器からは出力できない。(b) `Stmt::NewTypeDef { name, original }`＝名目的別型（[`ast.rs:814`](src/ast.rs#L814)）。
- 変換方針（推奨: 変換器内エイリアステーブル）:
  - `type X = <型式>` を検出したら、変換器が保持するマップに `X → convert_annotation(rhs)` を登録し、以降の型注釈解決（`convert_annotation`/`map_type_name`）で `X` を展開する（`alias` と同等の透過展開を変換器内で行う）。
  - 単純名エイリアスに限り `NewTypeDef` へ出力する案もあるが、名目的別型のため型検査で偽陽性を生む懸念があり非推奨。
- 難易度: 中。
- 懸念: パラメータ化エイリアス（`type V = list[float]`）や値としての利用は限定的。透過 vs 名目的の意味差に注意。

### [x] 11. 三項演算子 `a if cond else b`【実装済 2026-08-28】

- 対象: [`expressions.rs` `convert_expr()`](src/python_converter/expressions.rs) の `py::Expr::IfExp` アーム
- 現状: `inline 'if' expression is not supported` エラー。
- Arrow 側: `Expr::IfExpr { branches, else_body, return_type }`（[`ast.rs:413`](src/ast.rs#L413)）。各ブランチ本体は `block_return` で値を返す。**`return_type: None` でも式として評価可能**（実機検証済み）。任意の式位置にネスト可能。
- 変換方針: `IfExp { test, body, orelse }` → `Expr::IfExpr { branches: [(convert(test), [Stmt::BlockReturn(convert(body), span)])], else_body: Some([Stmt::BlockReturn(convert(orelse), span)]), return_type: None }`。
- 難易度: 低。
- 検証: ✔実機（`if c -> int: block_return 1 else: block_return 2` および注釈なし版の双方が動作）。

**実装結果**: 計画どおり `convert_expr` の `py::Expr::IfExp` アームで Arrow の `if` 式へ写す。
Arrow の `if` 式は分岐本体が**文の列**なので、各腕を `BlockReturn(<値>)` 1 文だけのブロックにする。
`return_type: None`（`-> T` 注釈なし）でも式として評価できることを実機で確認済み。

**確認した位置（17 ケース・すべて CPython と出力一致）**: 素朴／入れ子（括弧つき）／
括弧なしの連鎖（右結合＝ elif 相当）／代入の右辺／呼び出し引数／リスト要素／dict の値／
`while` の条件式の中／腕の型が違う場合。

**⚠ 遅延評価が一致する**のが要点: Python の三項式は選ばれた腕しか評価しない。Arrow の `if` 式も
同じなので、副作用のある呼び出しを腕に置いたときの**評価回数まで揃う**（例題 ⑨ で固定）。

**例題**: [`examples/interop/py_ternary.ar`](examples/interop/py_ternary.ar) +
[`test_modules/py_ternary.py`](examples/interop/test_modules/py_ternary.py)。
新しいエラー経路が無いため `_error` 例は無し（腕の式が未対応構文ならその構文自身のエラーが出る）。

### [x] 12. `in` / `not in`（メンバシップ）【実装済 2026-08-28】

- 対象: [`expressions.rs`](src/python_converter/expressions.rs) の `py::Expr::Compare` 変換（`convert_cmpop` の `In`/`NotIn` アーム）
- 現状: `'in' operator is not supported in expression context` エラー。
- Arrow 側: `BinOp::In` / `BinOp::NotIn` が実在（[`ast.rs:248`](src/ast.rs#L248)）。**ソース構文も Python と同一**（`x in xs` / `x not in xs`）。
- 変換方針: `CmpOp::In` → `Expr::BinOp{ op: In }`、`CmpOp::NotIn` → `Expr::BinOp{ op: NotIn }`。単一 BinOp で表現できる。
- 難易度: 低。
- 検証: ✔実機（`2 in [1,2,3]`→True、`9 not in [1,2,3]`→True）。

**実装結果**: `convert_cmpop` で `CmpOp::In => BinOp::In` / `CmpOp::NotIn => BinOp::NotIn` を返すだけ。

**⚠ コンテナごとの意味も Python と一致**（11 ケース CPython 出力一致）:
list / tuple / set は**要素**、dict は**キー**、str は**部分文字列**。
条件式・`and` との組み合わせ・ループ内フィルタも確認済み。

**例題**: [`examples/interop/py_membership.ar`](examples/interop/py_membership.ar) +
[`test_modules/py_membership.py`](examples/interop/test_modules/py_membership.py)。
新しいエラー経路が無いため `_error` 例は無し。

### [x] 13. `is` / `is not`（識別比較）— ★文法差異に注意★【実装済 2026-08-28】

- 対象: [`expressions.rs`](src/python_converter/expressions.rs) の `py::Expr::Compare` 変換
- 現状: `'is' operator is not supported` エラー。
- **文法差異（重要）**:
  - Python の `is`/`is not` は**識別比較（オブジェクト同一性）**。
  - Arrow の `is` キーワードは**別物** — `expr is TypeName` という**型ガード**（`Expr::IsType`、`isinstance` 相当。実機 `x is int`→True）。したがって Python の `is` を Arrow の `is` に写すと**誤変換**になる。
  - Python の識別比較に対応する Arrow 演算子は `===`（`BinOp::RefEq`、参照等値。実機で `a===b`（別オブジェクト同値リスト）=False、`a===c`（同一オブジェクト）=True）。`!==`（RefNotEq）は**存在しない**。
- 変換方針:
  - `x is y` → `Expr::BinOp{ op: RefEq }`。
  - `x is not y` → `Expr::UnaryOp{ Not, BinOp{ RefEq } }`（`!==` が無いため単項 `not` で包む）。
  - `x is None` / `x is not None` も同様（実機で `x === None`→True、`not (y === None)`→True）。
  - 実装上は `convert_cmpop`（`BinOp` を返す）では `is not` の Not ラップを表現できないため、`Compare` アーム側で `In`/`NotIn`/`Is`/`IsNot` を特別扱いする。
- 難易度: 低〜中。
- 懸念: Python の小整数/文字列インターン等の `is` 挙動差はあるが、一般用途（`x is None`・オブジェクト同一性）は `===` で一致。

**実装結果**: 計画どおり `Compare` アーム側で処理した（`convert_cmpop` には置けない。
Arrow に `!==` が無く `is not` は `Not(RefEq)` ラップが要るので `BinOp` 1 個では表せない）。
`convert_cmpop` の `Is`/`IsNot` は到達しない内部エラーに変えてある。

**⚠ 名前の衝突**: Arrow にも `is` キーワードがあるが**型ガード**（`x is int`）で別物。
Python の `is` は `===`（`BinOp::RefEq`）に写す。

**一致した形（11 ケース）**: `x is None` / `x is not None`（最頻出）・デフォルト値ガード・
別名（`b = a` のあと `a is b` → True）・別々に作った同値オブジェクト（False）・
インスタンスの別名・`f is True`・`and` との組み合わせ。
⚠ 別名が True になるのは、`import[py]` の関数が **Python の値渡し規則（deep copy しない）** を
保っているため。純 Arrow の `mut b = a` は deep copy されるので同じにはならない。

**⚠ 残る意味差（2 ケース・実測）— 不変プリミティブのインターン**:
Arrow の `===` は str / int を**値で**比べるが、CPython の `is` はオブジェクト識別なので
**インターンの有無**に依存する。

| 式 | Arrow | CPython |
|---|---|---|
| `(a+b) is "hi"` 相当（計算で作った str） | `True` | `False` |
| `(500+500) is 1000` 相当（256 超の int） | `True` | `False` |

⇒ **エラー化していない**。これは CPython 自身が `SyntaxWarning: "is" with a literal` で
警告する使い方の系列（本来 `==` を使う場面）であり、**Arrow の答えの方が書き手の意図に近い**。
モジュール単位変換では 1 箇所の拒否が import 全体を殺すので、警告相当を硬いエラーにするのは
釣り合わない。将来「警告を出す」方向なら足せる。

**例題**: [`examples/interop/py_identity.ar`](examples/interop/py_identity.ar) +
[`test_modules/py_identity.py`](examples/interop/test_modules/py_identity.py)
（13 ケース中 11 件 CPython 一致・⑤ の 2 件が上記の差）。

### [ ] 14. `del` 文（警告付き無視）

- 対象: [`statements.rs` `convert_stmt()`](src/python_converter/statements.rs) — 新規 `py::Stmt::Delete` アーム
- 現状: 汎用 catch-all エラー `unsupported Python statement`。
- 方針（ユーザー指示）: **警告を出したうえで無視**（ローカルスタックの破棄に任せる）。`del <name>` は変数束縛の削除であり、Arrow ではスコープ終了時に破棄されるため無視でおおむね許容。
- 実装: `Delete { targets }` の各ターゲットについて警告を出して `Ok(None)` を返す。変換器に警告チャネルが無いため、当面 `eprintln!("Warning: ...")`（main.rs の型検査警告と同じ stderr 出力）で対応。将来は警告収集ベクタの導入を検討。
- 難易度: 低。
- 懸念（要判断）: `del d[k]`（Subscript）・`del obj.attr`（Attribute）は**意味のある削除**であり、無視すると挙動が失われる（警告があっても誤り）。→ **Name ターゲットのみ警告付き無視、Subscript/Attribute は明示エラー**を推奨（あるいは `d.pop(k)` 等への変換を別途検討）。

### [ ] 15. 複数代入 `a = b = c`

- 対象: [`statements.rs`](src/python_converter/statements.rs) の `Assign` アーム（現在 `targets.len() != 1` でエラー）
- 現状: `multiple assignment targets are not supported` エラー。
- 方針（ユーザー指示）: 単文に分割（`a = c` と `b = c`）。
- 実装: `convert_stmt` が複数文を返せるようにする（戻り値の `Vec<Stmt>` 化、または `convert_stmts` 側で展開）。各ターゲットについて再代入ロジック（🟢2）に従い `Mut`/`Assign` を生成。
- 難易度: 中（複数文返却の配管）。
- 懸念（限定的）: RHS が **mutable 値を指す変数/式**なら、Arrow の代入は参照を共有するためエイリアスは崩れない（ユーザー確認済み。実機でも `let c = a; a === c`→True）。残る差異は **`a = b = <新規リテラル>`**（例 `a = b = []`）のみ — Python は単一オブジェクトを共有するが分割 `a=[]; b=[]` は別オブジェクトになる。この稀なケースが問題なら一時変数化 `__t=[]; a=__t; b=__t` で回避可能。

### [ ] 16. 連鎖比較 `a < b < c`

- 対象: [`expressions.rs`](src/python_converter/expressions.rs) の `Compare` アーム（現在 `ops.len() != 1` でエラー）
- 現状: `chained comparisons are not supported` エラー。
- 方針（ユーザー指示）: `and` で分割（`a < b and b < c`）。
- 実装: `Compare { left, ops, comparators }` を隣接ペアに展開し `Expr::BinOp{ And }` で連結。`a op1 b op2 c` → `(a op1 b) and (b op2 c)`。式のまま完結（文分割不要）。
- 難易度: 低。
- 懸念: `and` 展開で中間オペランド `b` が2回評価される（副作用のある中間式で差異）。→ **ユーザー方針によりこの副作用は許容**。

### [ ] 17. 内包表記 → `for` 式 + `loop_yield`

- 対象: [`expressions.rs`](src/python_converter/expressions.rs) の `ListComp`/`SetComp`/`DictComp`/`GeneratorExp` アーム
- 現状: `comprehensions are not supported` エラー。
- Arrow 側: `Expr::ForExpr { target, iter, body, return_type }` + `Stmt::LoopYield`（[`ast.rs:422`](src/ast.rs#L422)）。実機で `for x in xs -> list[int]: if cond: loop_yield x*x` → `[4,16]` 確認。
- 方針: リスト内包 `[expr for x in it if cond]` → `Expr::ForExpr { target:"x", iter:convert(it), body:[If{[(cond,[LoopYield(expr)])],None}]（if 無しなら [LoopYield(expr)]）, return_type:Some("list[Any]") }`。
- 難易度: 中。
- **多重 for（検証済み・対応可能）**: `[expr for x in xs for y in ys if cond]` は「外側 for **式** + 内側 for **文** + `loop_yield`」で**フラットなリスト**になる（`loop_yield` は入れ子 for 文・if 文を透過して最外の for 式のアキュムレータへ積まれる）。実機で `[11,21,12,22]`（2重）・`[111,121,112,122]`（3重）・フィルタ付き `[22,12]` を確認。
  - マッピング: 先頭 generator → 外側 `Expr::ForExpr`、2つ目以降の generator → body 内の入れ子 `Stmt::For`、各 generator の `ifs` → `Stmt::If` ラップ、`elt` → 最深部の `Stmt::LoopYield`。
- 懸念（残り）:
  - set 内包は結果の set 化（`set(...)`、set は 🟢22 で対応）、dict 内包は dict 化（ペア構築→dict）が別途必要。
  - generator 式は遅延評価だが ForExpr は即時（list 構築）。async 内包（`async for`）は非対応。
  - → 単一 for / 多重 for のリスト内包を対応。set/dict/generator/async は追加検討 or 明示エラー。

### [x] 18. 定数タプル【実装済 2026-08-28 / ただし現構成では到達しない経路】

- 対象: [`expressions.rs` `convert_constant()`](src/python_converter/expressions.rs) の `Constant::Tuple` アーム
- 現状: `constant tuple is not supported` エラー。
- Arrow 側: `Expr::Tuple(Vec<Expr>)` 実在。
- 方針: `Constant::Tuple(Vec<Constant>)` の各要素を定数変換して `Expr::Tuple` を構築。
- 難易度: 低。

**⚠⚠ 調査結果: このアームは現在の構成では到達しない**。
`Constant::Tuple` を作るのは rustpython の `ConstantOptimizer` だけで、それは
`constant-optimization` フィーチャ有効時にしか実装されず、`Suite::parse` は畳み込みをしない。
通常のタプル `(1, 2)` は**常に** `py::Expr::Tuple` として来る（そちらは元から対応済み）。
6 通りの書き方（代入・デフォルト引数・添字・注釈付き・return・型注釈つき return）で確認済み。

**実装結果**: 将来フィーチャを有効にしても黙って壊れないよう、`convert_constant` から
`constant_value_to_expr(&py::Constant)` を切り出して**再帰変換**を実装した（入れ子の定数タプルも通る）。
例題は「実際に通る経路」（`Expr::Tuple`）を 16 ケースで固定した（CPython と出力一致）。

**⚠⚠ 検査中に Arrow 本体側のタプルの穴を 3 つ発見（変換器の外・未修正）**:

1. **タプルを dict のキーにすると黙って消える**（最悪の失敗形）。
   `{(1, 2): "x"}` は**空の dict** になり、`d[(1,2)] = "y"` も入らない（`len` が 0）。
   str / int / bool のキーは正常。純 Arrow で再現。
2. **タプル同士の `+`（連結）が未対応**。`(1,2) + (3,)` が
   `TypeError: unsupported operand types for Add: tuple and tuple`。純 Arrow で再現。
3. **`list` の `==` が値比較でない**（`[1,2] == [1,2]` が `False`）。
   ⚠ **タプルの `==` は値比較で正しい**ので、list 側だけがおかしい。純 Arrow で再現。

**例題**: [`examples/interop/py_tuple.ar`](examples/interop/py_tuple.ar) +
[`test_modules/py_tuple.py`](examples/interop/test_modules/py_tuple.py)。
到達しないアームなので `_error` 例は無し。

### [ ] 19. f-string

- 対象: [`expressions.rs`](src/python_converter/expressions.rs) の `py::Expr::JoinedStr` アーム
- 現状: `f-strings are not supported` エラー。
- Arrow 側: f-string を **`"lit" + str(expr) + "lit"`** の左結合 `BinOp::Add` 連結に脱糖する（`desugar_fstring` [`exprs.rs:592`](src/parser/exprs.rs#L592)）。実機 `f"hi {name} number {n}"` → `hi bob number 5` 確認。
- 方針: Python `JoinedStr { values }` を同形へ変換。各 `Constant(str)` → `Expr::Str`、各 `FormattedValue{value}` → `Expr::Call{ func:Ident("str"), args:[convert(value)] }`。全体を `BinOp::Add` で連結。
- 難易度: 低〜中。
- 懸念: 書式指定 `{x:.2f}`（format_spec）・変換 `!r`/`!s`（conversion）は `str()` 単純ラップでは再現不可。書式なし f-string を対応し、format_spec 付きは追加検討 or 明示エラー。

### [x] 20. デコレータ `@decorator`【実装済 2026-08-27】

- 対象: [`statements.rs`](src/python_converter/statements.rs) の `FunctionDef` アーム + [`classes.rs`](src/python_converter/classes.rs)（クラス／メソッドの計3箇所のエラー分岐）
- 現状: `decorators are not yet implemented` エラー（関数・クラス・メソッド）。
- Arrow 側: **`@decorator fn` / `@decorator class` を実装済み**。`Stmt::FnDef.decorators` / `Stmt::ClassDef.decorators`（`Vec<Expr>`）に保持し、インタープリタが逆順適用（[`definitions.rs:82`](src/interpreter/exec/definitions.rs#L82)）。メソッドデコレータも対応（[`definitions.rs:509`](src/interpreter/exec/definitions.rs#L509)）。実機で関数デコレータ・スタックデコレータの動作を確認。
- 方針: Python の `decorator_list` を各々 `convert_expr` して `decorators` フィールドへ格納。3箇所のエラー分岐を削除。
- 制約: Arrow は `@` の直後が `fn`/`class` のみ（Python も関数/クラスのみで一致）。デコレータ関数は `fn d(let f: function) -> function:` 形のシグネチャが必要。
- 難易度: 低。
- 検証: ✔実機（`@log`、`@double_log @log` が正しく動作）。

**実装結果**: [`decorators.rs`](src/python_converter/decorators.rs) を新設し、`convert_decorators()` で
`decorator_list` を 2 つに振り分ける形にした（単純な素通しでは不十分だった）:

- **通常のデコレータ** → `FnDef.decorators` / `ClassDef.decorators`（`convert_expr` で変換）。
  `@f(x)` 形のデコレータファクトリもそのまま通る。
- **定義種別マーカ** → Arrow のフラグに振り替える。⚠ Arrow には `staticmethod` /
  `classmethod` という**組込関数が存在しない**ため、素通しすると実行時 `NameError` になる。
  - `@staticmethod` → `is_static`（Arrow の `static fn`）
  - `@classmethod` → `is_class_method`（Arrow の `class_method fn`。第 1 引数にクラス自身）
  - `@abstractmethod` / `@abc.abstractmethod` → `is_abstract`（本体はそのまま残す）
- **明示エラー**: `@property` / `@cached_property` / `@x.setter` / `@x.getter` / `@x.deleter`
  （Arrow にプロパティ構文が無い）、モジュール直下の `@staticmethod` 等、
  `@staticmethod` と `@classmethod` の併用。

**意味差（要留意）**: Arrow の `static` / `class_method` は**クラス経由でしか呼べない**
（インスタンス経由は `AttributeError`：[`method_call.rs`](src/interpreter/classes/method_call.rs) の
`static_method_names` 判定）。Python は `obj.stat()` も許すので、ここだけ差が残る。

**例題**: [`examples/interop/py_decorators.ar`](examples/interop/py_decorators.ar)（成功・CPython と出力一致を突き合わせ済）/
[`examples/interop/py_decorators_error.ar`](examples/interop/py_decorators_error.ar)（明示エラー 3 種）。

**† 実装中に見つかった別バグ（本項目の外）**:
1. **`mut` パラメータが入れ子 `fn` にキャプチャされない**（None になる）。**純 Arrow で再現**する。
   `fn f(let n: int)` なら通るが `fn f(mut n: int)` だと壊れる。変換器は全パラメータを
   `mutable: true` にするため、**Python で最も普通の「クロージャで包むデコレータ」が使えない**。
2. **モジュール直下で同じモジュールの関数を「呼ぶ」ことができない**（項目 2 の作業中に切り分け完了）。
   `g = hello`（**参照**）は通るが `MSG = hello("bob")`（**呼び出し**）が
   `NameError: 'hello' is not defined` になる。⚠ **純 Arrow の `.ar` モジュールでも再現**する
   （`import[ar] lib` した先の `let MSG = hello("bob")`）。⇒ 変換器ではなく
   **モジュール本体の実行時の名前解決**の問題。デコレータは `eval_definition_expr` 経由なので通る。

### [ ] 21. `...`（Ellipsis）→ 文位置は `pass`

- 対象: [`statements.rs`](src/python_converter/statements.rs) の `py::Stmt::Expr` アーム（中身の Ellipsis 判定）
- 現状: `Constant::Ellipsis` を式として `Expr::None` に変換（黙って None 化）。
- 方針（ユーザー提案）: **文としての `...`**（スタブ本体の `...`）を `Stmt::Pass` に読み替える。`Expr` 文で中身が `Constant::Ellipsis` なら `Stmt::Pass` を返す。
- 難易度: 低。
- 懸念: **値位置の `...`**（`x = ...`、`Callable[..., int]`、`a[...]`）は文（pass）にできない。値位置は現状どおり `Expr::None` を維持（**副作用が無いためユーザー承認済み・変更不要**）。文位置のみ pass 化。

### [x] 22. 集合リテラル `{1, 2, 3}` / set 内包【リテラルのみ実装済 2026-08-28】

- 対象: [`expressions.rs`](src/python_converter/expressions.rs) の `py::Expr::Set` アーム（＋ `SetComp`）
- 現状: `set literal is not supported` エラー。
- Arrow 側: **set 型は実在**（`Expr::Set(Vec<Expr>)`、`set()` コンストラクタ、`set_type.rs`）。実機で `{1,2,3,2}`→`{1, 2, 3}`、`2 in s`→True、`set()`→空集合 を確認。
- 方針: `py::Expr::Set { elts }` → `Expr::Set(elts.map(convert))`。set 内包は `set(<for式>)` 相当（ForExpr を set 化）。
- 難易度: 低（リテラル）〜中（内包）。
- 補足: ユーザーの「set 型が無いかも」という想定に反し、**Arrow は set をサポート**していた（実機確認済み）。

**実装結果**: セットリテラルは `convert_expr` の `py::Expr::Set` アームで要素を順に変換して
`Expr::Set` に積むだけ（13 ケース CPython 出力一致）。リテラル／重複除去／単要素／`set()` の空集合／
メンバシップ／`len`／`add`／要素が式／リスト内の入れ子／tuple 要素／str 要素を確認。

**⚠ 空セットは `set()`**（`{}` は空辞書）。これは Python の規則そのままなので、
空セットが `py::Expr::Set` として来ることはない。

**⚠ set 内包 `{x for x in xs}` は未対応のまま**（`SetComp` という**別ノード**で、
内包表記＝項目 17 の担当）。「セットは対応したのに落ちる」と読み違えやすいので、
`SetComp` を独立アームに分けて**専用の文言**にした:
`set comprehension is not supported (set literals like `{1, 2}` are supported)`。
項目 17 が入れば `set(<for 式>)` として通せる。

**⚠ セットの repr 順は当てにしないこと**: CPython は文字列のハッシュを実行ごとに
ランダム化するので `{"a","b","c"}` の表示順は**実行のたびに変わる**。例題では int / tuple の
セットだけを表示し、str のセットはメンバシップで確認している。

**例題**: [`examples/interop/py_set.ar`](examples/interop/py_set.ar) +
[`test_modules/py_set.py`](examples/interop/test_modules/py_set.py) /
[`examples/interop/py_set_error.ar`](examples/interop/py_set_error.ar) +
[`test_modules/py_setcomp_error.py`](examples/interop/test_modules/py_setcomp_error.py)（set 内包）。

### [ ] 23. walrus 演算子 `:=`

- 対象: [`expressions.rs`](src/python_converter/expressions.rs) の `py::Expr::NamedExpr` アーム（＋文レベルの補助文注入）
- 現状: `walrus operator ':=' is not supported` エラー。
- 方針（ユーザー指示）: **2文に分割**。`(x := expr)` を、先行する代入文 `x = expr`（🟢2 の再代入ロジック）＋ 元の式位置での `x` 参照に変換。
  - 例: `if (n := len(xs)) > 10:` → `n = len(xs)` を if の前に出し、条件を `n > 10` にする（＝代入演算子＋左辺値を使った比較文）。
- 難易度: 中（式コンテキストから囲みスコープへ文を注入する仕組みが必要。🟢15 複数代入と共通の配管）。
- 懸念: if/while の条件など**文レベルの walrus は素直**。内包表記内や深いネスト式内の walrus は注入位置の決定が要る。

### [x] 24. bare `*`（キーワード専用引数の区切り）【実装済 2026-08-28】

- 対象: [`classes.rs` `convert_params()`](src/python_converter/classes.rs)（既に概ね対応済み）
- 背景: `def f(a, *, b)` の bare `*` は rustpython では「vararg なし＋`kwonlyargs`」として表現され、`convert_params` は `kwonlyargs` を通常引数へ平坦化している（＝bare `*` は実質無視済み）。
- 方針（ユーザー指示）: bare `*` は**無視して引数リストを切り詰める**（キーワード専用引数を通常引数として平坦化）。現状挙動を正式化・確認する。
- 難易度: 低（ほぼ現状どおり）。
- 懸念: Python のキーワード専用引数は位置渡し不可だが、平坦化後の Arrow では位置渡しも可能になる（意味の緩和）。ユーザー方針で許容。

**実装結果**: **コード変更は不要**だった（既存の `convert_params` が既に平坦化していた）。
実機で全形を確認したうえで、`convert_params` の doc コメントに**方針と緩和の理由を明文化**し、
例題で挙動を固定した。`/`（位置専用マーカ）も同じく無視される。

確認した形（すべて期待どおり動作）:

| 形 | 呼び出し | 結果 |
|---|---|---|
| `def f(a, *, b)` | `f(1, b=2)` | ✔ |
| 同上 | `f(1, 2)` | ✔ **CPython は `TypeError`**（緩和・許容済み） |
| `def g(a, *, b, c)` | `g(1, b=2, c=3)` | ✔ |
| `def only_kw(*, b)` | `only_kw(b=5)` | ✔ |
| `def both(a, /, b, *, c)` | `both(1, 2, c=3)` | ✔（`/` も無視） |
| `def __init__(self, x, *, y)` | `Point(1, y=2)` | ✔ |

**他項目との干渉（本項目では直さない）**:
- キーワード専用引数の**デフォルト値は落ちる**（項目 1 未実装）。`def f(a, *, b=3)` を `f(1)` で
  呼ぶと `'f' takes 2 argument(s) but 1 were given`。
- **実体のある `*args` と併用**すると壊れる（項目 6 未実装。`*args` が不正な `Param` になるため
  `def f(a, *rest, b)` が `takes 3 argument(s)` という紛らわしいエラーになる）。
  **bare `*`（名前なし）単体はこの影響を受けない。**

**例題**: [`examples/interop/py_kwonly.ar`](examples/interop/py_kwonly.ar) +
[`test_modules/py_kwonly.py`](examples/interop/test_modules/py_kwonly.py)
（②の位置渡しを除き CPython と出力一致を突き合わせ済）。エラー化した項目が無いため `_error` 例は無し。

### [ ] 25. `with` 文（`__exit__` 実行を伴わない場合のみ）

- 対象: [`statements.rs`](src/python_converter/statements.rs) の `py::Stmt::With` アーム（現在エラー）
- 方針（ユーザー決定）: **`__exit__` 実行を伴う with は明示エラー、伴わない場合のみ block 脱糖**で代替。
  - `with EXPR as x: body` → `block: mut x = EXPR; body`（`x` は退出時に Drop され、ファイル等のリソース系は `FileData::drop` で自動後始末。詳細は 🟡「with 文」参照）。
- 検出: `__exit__` の有無は変換時に静的判定できない（EXPR の実体型は実行時決定）ため、**実行時ガード**で切り分ける — 脱糖 block 内で `x` のクラスに `__exit__` メソッドがあれば `RuntimeError` を raise（同一モジュール内で定義されたクラスは変換時にも静的検出可能）。
- 難易度: 中。
- 懸念: `__enter__` の戻り値束縛（Python は `x = EXPR.__enter__()`）— ファイル等は `open()` が直接オブジェクトを返すため `mut x = EXPR` で足りるが、`__enter__` が別値を返すケースは要考慮。`__exit__` の例外抑制（True 返却で握り潰し）は error 対象なので非対応で問題なし。

### [ ] 26. lambda 式 → 名前付き関数への持ち上げ（lambda lifting）

- 対象: [`expressions.rs`](src/python_converter/expressions.rs) の `py::Expr::Lambda` アーム（＋文注入機構）
- 方針（ユーザー決定）: 各 lambda を、囲みスコープへ持ち上げた名前付きネスト関数 `fn __lambda_N(params) -> Ret: return <body>` に変換し、lambda 式をその関数名参照 `Ident("__lambda_N")` に置換。
- Arrow 側: 名前付きネスト関数は環境をキャプチャする（`CapturedVar`、デコレータ wrapper の実機動作で確認済み）→ lambda のクロージャ意味論を保持できる。
- 難易度: 中（式コンテキストから囲みスコープへ `fn` 定義を注入する仕組みが必要。🟢15 複数代入・🟢23 walrus と共通の配管）。
- 懸念: Arrow の `fn` は戻り型注釈が要る（`return_type: None` は `MissingReturnTypeAnn`）。lambda は無注釈なので戻り型を推論するか `Any` を補う運用が必要。デフォルト引数付き lambda は 🟢1 と併用。

### [ ] 27. Python モジュール内の import（再帰ロード）

- 対象: [`statements.rs`](src/python_converter/statements.rs) の `Import`/`ImportFrom`（現在 `Ok(None)` で破棄）＋ [`imports/py_modules.rs` `load_python_module`](src/parser/imports/py_modules.rs) の後処理
- 方針（ユーザー提案・肯定）: `import[py]` は「Python ソースを AST 展開したものをロード」する方針とし、**Python モジュール内の `import`/`from import` を Arrow の `Stmt::Import`/`FromImport`（lang="py"）に変換**、その body を **`load_python_module` の再帰呼び出しで充填**する。
- 実現可能性: **可能**。`load_python_module` は既に検索ディレクトリ・`module_cache`・循環検出（`self.loading`）を持ち、内部で `convert_python_source` を呼ぶ。現状の唯一の欠落は「変換器が Python 内部 import を捨てている」点のみ。変換器が import 文を（空 body で）emit し、`load_python_module` が返り値 body を走査して各 import を再帰解決すれば、キャッシュ・循環検出をそのまま再利用できる。
- 難易度: 中。
- 残る制約: **標準ライブラリ・サードパーティ（`os`/`numpy` 等）は翻訳対象の `.py` が無い/C 実装のため、この再帰では解決不能** → `import[py-int]`（PyO3）へのフォールバックまたは明示エラーが必要。相対 import（`from . import x`）・`import X as Y`・`import a.b.c` のマッピングも要対応。

---

## 🟠 明示エラー化（暫定）＋ Arrow 言語仕様の検討後に再検討

ユーザー方針: これらは**現状すべて明示的な「非対応文法」エラーとして raise** する。Arrow 側に相当構文（ループ `else`・例外グループ・例外連鎖・アサーション）を導入するかは**言語仕様の検討事項として保留**し、確定後に再検討する。

> 注意: 下表のうち `for/else`・`while/else`・`try/else`・`except (A,B)`・`raise X from Y` は**現状サイレントに欠落**している。方針に合わせるには、まず**明示エラー化するコード修正が必要**（「黙って壊れる」状態の解消）。

| 項目 | 現状 | 必要な暫定対応（コード） |
|---|---|---|
| `for ... else` | `else` 節を黙って破棄 | `For` アームで `!f.orelse.is_empty()` なら明示エラー |
| `while ... else` | `else` 節を黙って破棄 | `While` アームで `!w.orelse.is_empty()` なら明示エラー |
| `try ... else` | `else`（`t.orelse`）を黙って破棄 | `Try` アームで `!t.orelse.is_empty()` なら明示エラー |
| `except (A, B)` / `except mod.Err` | 具体型を捨て `"Exception"` に潰す | `except` 型が単純 `Name` 以外なら明示エラー |
| `raise X from Y` | `from Y`（cause）を黙って破棄 | `Raise` アームで `r.cause.is_some()` なら明示エラー |
| `assert` 文 | 汎用エラー | 専用の `Assert` アームで明示エラー（メッセージ具体化） |
| コレクション型のアンパック（`*` / `**` 展開を統合） | 明示エラー（現状維持・変更不要） | 代入 `a,b=` / `a,*rest=`、`for a,b in`、呼び出し `f(*xs)` / `f(**d)`、リテラル `[*a]` / `{*a}` / `{**d}` を**すべてこの項目に統合**。Arrow には `LetTuple`・多ターゲット `For`・呼び出しの `f(...=a,b,c)`（`CallArg::Variadic`）が実在するため将来対応可能 |
| `@`（行列積） | 明示エラー（現状維持・変更不要） | Arrow に相当演算子なし（言語追加が前提） |
| bytes リテラル | 明示エラー（現状維持・変更不要） | Arrow に bytes 型なし。再検討 |
| 複素数リテラル | 明示エラー（現状維持・変更不要） | ※Arrow に `ImaginaryLit`/`Value::Complex` は実在するため将来対応可。再検討 |
---

## 🔴 明示エラー（非対応確定・サポート予定なし）

ユーザー方針により、以下は明示的な非対応エラーとして扱う（Arrow に別の代替手段があるか、モデルが異なるため）。

| 項目 | 現状 | 必要な対応 | 備考 |
|---|---|---|---|
| `global` / `nonlocal` | **黙って無視**（`Ok(None)`） | **silent→明示エラー化のコード修正が必要** | Arrow は「外側変数を `mut` 宣言」で代替（`nonlocal` キーワードなし） |
| `async def` / `async for` / `async with` / `await` | 明示エラー | 現状維持 | Arrow の非同期は `mng <- async->T:` / AsyncManager モデルで別体系 |

---

## 🔵 仕様として確定（意図的動作・バグではない）

### 整数の切り詰め

- `convert_constant` の `Int` は i64 を超える値を `i64::MAX` にクランプする（[`expressions.rs:199`](src/python_converter/expressions.rs#L199)）。
- **仕様**: Arrow の整数は i64。i64 を超える整数リテラルは i64::MAX に丸める（多倍長整数は非対応）。
- 補足: サイレントなクランプは気付きにくいため、将来的に「範囲外は明示エラー」への変更余地はあるが、現時点では仕様として据え置く。

### 変換パラメータは常に `mutable: true`

- `convert_params` は全パラメータを `mutable: true` として生成する（Python に不変引数の概念がないため）。
- **仕様**: Python 由来関数の引数はすべて可変扱い。
- 補足（要留意）: 現状のままだと、変換関数へイミュータブル値（リテラルや `let` 変数）を渡すと型検査 `CallMutParamWithImmutableArg` で弾かれる（`m.double(5)` が通らない）。この UX 問題を避けるには、**Python 由来関数に対してこの検査を緩和する**（`is_python` 関数は immutable 実引数を許容）対応を別途検討する価値がある。仕様維持と UX のトレードオフとして残す。

---

## 🟡 懸念・要判断

### lambda 式 — 提案された `block_return` 代替は不適切

- Python の `lambda` は**呼び出し可能値（callable）を生成する式**。一方 `block_return` は「ブロック式から値を返して抜ける」もので、callable を作らない。したがって `block_return` では代替できない。
- Arrow には**無名関数式が存在しない**（`Expr` に Lambda/Closure バリアントなし。クロージャは名前付きネスト `fn` 定義のみで実現され、環境捕捉に対応）。
- 忠実な変換は「**ラムダリフティング**」＝各 lambda を直前スコープの名前付き `fn __lambda_N(...)` に持ち上げ、その名前を参照する形。ただし lambda は任意の式位置（例 `sorted(xs, key=lambda ...)`）に現れ、式単位の `convert_expr`（単一 `Expr` を返す）からは囲みスコープへ文（`fn` 定義）を注入できない。実装には convert_expr が補助文を外に持ち出せるよう変換器の構造変更が必要。
- **判断（決定）**: `block_return` 代替は採用しない。**ラムダリフティング（名前付き関数への持ち上げ）を採用** → 🟢26。式コンテキストからの文注入機構（🟢15/🟢23 と共通）で実装する。

### モジュール import（`import X` / `from X import Y`）— アーキ変更が必要

- Arrow 側に `Stmt::Import` / `Stmt::FromImport` は実在するが、その `body`（モジュール内容の AST）は**パーサの import 機構がパース時に充填**する（`load_python_module` 等）。`convert_python_source` は search dir やモジュールキャッシュを持たない自由関数で、単体では import を解決できない。
- 加えて、対象が純 `.py` ローカルモジュールなら `import[py]` に翻訳できるが、`os`/`numpy` 等の標準・サードパーティは対応する `.py` を翻訳する意味がなく、`import[py-int]`（PyO3 実行時）でしか扱えない。翻訳器がタグ（`py` vs `py-int`）を一意に決められない。
- **判断（決定）**: Python 内部 import を Arrow の `import[py]` 文に変換し、body を **`load_python_module` の再帰呼び出しで充填**する方針を採用 → 🟢27。`load_python_module` は既に search dirs・`module_cache`・循環検出（`self.loading`）を持つため再利用可能。**解決不能な stdlib/native（`os`/`numpy` 等）は `py-int` フォールバックまたは明示エラー**。

### `with` 文 — `block` によるスコープ破棄で「一部」表現可能（前回の断定を訂正）

- **検証結果（訂正）**: Arrow の block は退出時にローカル変数を破棄する仕様が**実在した**。`exec_scoped_block` が `push_scope`→実行→`pop_scope` を行い、`pop_scope` でスコープのバインディングを破棄、値は Rc 参照が 0 になった時点で Rust の `Drop` が走る。さらに**ネイティブリソース型は `Drop` で実クリーンアップ**する — `FileData::drop` は `close()`（フラッシュ＋書き戻し＋ハンドル解放）を呼ぶ（[`objects.rs:137`](src/interpreter/value/objects.rs#L137)、コメントにも「スコープを抜けるとき自動 close」）。C#/Node プロセスブリッジも同様。
- したがって **`with open(p) as f: body` は `block: mut f = open(p); body` へ脱糖可能** — `f` がスコープを抜けて無参照になると `FileData::drop` がファイルを閉じる。組込みリソース系の `with` は block で概ね表現できる。
- **参照カウント基準の懸念 → Python→Arrow では非該当（削除）**: block+Drop の破棄は参照カウント基準（最後の参照が消えた時点）だが、**Python→Arrow の変換方向ではこれは問題にならない**。Python の `with` はブロック退出でリソースが後始末される前提であり、退出後にそのリソースを**使う**コードは Python 自身が実行時エラーになる（例: closed file への I/O）。したがって Arrow の遅延破棄との差異が表在化するのは「Python 側で元々エラーになる with 文を Arrow で動かしたとき」に限られ、Python ライブラリを外部モジュールとして使うユースケースでは無視できる（ユーザー確認済み）。（理論上の残差は「書込み with + 未使用の逃げ参照 + ブロック後の外部読取り」という作為的なフラッシュ時機のみで、実質無視可能。）
- **残る制約**: **ユーザー定義クラスには終了フック（`__del__`/`__exit__`）が無い**。リソース解放以外の副作用を持つカスタムコンテキストマネージャ（ロック解放・トランザクション commit/rollback 等）の `__exit__` は、block 脱糖では**再現できない**。
- **判断（決定）**: **`__exit__` 実行を伴う with は明示エラー、伴わない場合のみ block 脱糖**で代替 → 🟢25。`__exit__` の有無は静的に一般判定できないため、実行時ガード（`x` のクラスに `__exit__` メソッドがあれば `RuntimeError`）で切り分ける。

### 横断的な注意点

- `*args` / `**kwargs`（🟢 6・7）は**本体の識別子リライト**を伴い、`**kwargs` は空時の `kwargs` 未定義問題でインタープリタ側修正も要する。
- `match`（🟢 8）・ジェネレータ（🟢 9）は Arrow 側機能のサブセットのみ対応。非対応パターンは明示エラーにして「黙って誤変換」を防ぐ。

---

## ⚪ 未トリアージ（今回の指示対象外の残課題）

以下は今回のフィードバックで言及されなかった、開いたままの「サイレント欠落／誤変換」項目。別途方針決定が必要:

**⚪ 新規（2026-08-28・項目 5 の作業中に発見）: クラス継承が黙って落ちる**

`class Sub(Base):` を変換すると `Stmt::ClassDef.bases = ["Base"]` は載るが、
**Arrow はクラス継承をサポートしていない**（ネイティブ `.ar` では
`ParseError: class Sub cannot inherit from Base (only traits are allowed as bases)`。
Arrow の継承はトレイトのみ）。変換器はパーサを通らないのでこのエラーが出ず、
基底クラスの**メソッドもフィールドも引き継がれない**まま実行される:

- `s.hello()` → `AttributeError: 'Sub' has no method 'hello'`
- 基底の `__init__` が動かないので `s.v` も無い（`'Sub' object has no attribute 'v'`）

⇒ **サイレント欠落**（1 つ前の項目でいう「黙って壊れる」形）。方針決定が必要:
①明示エラーにする（フェーズ 5 と同じ扱い）／②基底のメソッド・フィールドを
**変換時に平坦化**して取り込む／③Arrow 側にクラス継承を入れる。
⚠ **Python コードでクラス継承は非常に多い**ので、優先度は高い。

分類済みの参照:
- 🟢 対応予定: 三項式/`in`/`is`（11〜13）、複数代入/連鎖比較/内包表記(単一・多重for)/定数タプル/f-string（15〜19）、デコレータ/Ellipsis(文)/集合（20〜22）、walrus/bare`*`（23〜24）、`with`(no __exit__)/lambda/モジュール import（25〜27）、他（1〜10）
- 🟠 再検討: `for/while/else`・`try/else`・`except (A,B)`・`raise from`・`assert`・**コレクションアンパック（`*`/`**` 展開を統合）**・行列積・bytes・複素数
- 🔴 非対応確定: `global`/`nonlocal`・async 系
- 🟢14: `del`、値位置 `...` は現状維持（None）で承認済み

---

## 検証メモ

検証に用いた `.py` / `.ar` はセッションのスクラッチパッドに作成。リグレッションテスト化する場合は 🟢 各項目の成功例を `examples/interop/`、明示エラー化する項目の失敗例を `*_error.ar` として配置するのが妥当。
