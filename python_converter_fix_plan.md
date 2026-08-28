# python_converter 修正 実装計画（作業指示書）

Python→Arrow 変換器（`import[py]` が使う、PyO3 を介さない翻訳器）の修正を、**まっさらなコンテキストからでも即着手できる**ように整理した作業指示書。
分類の背景・根拠は [python_converter_coverage.md](python_converter_coverage.md)（判断記録）を参照。本書は「どのファイルをどう直すか」に特化する。

- 対象サブシステム: [`src/python_converter/`](src/python_converter/)
- 作成: 2026-07-22（main pull 後の phase 0〜5C リファクタリング反映済み）

---

## 0. 事前確認（着手前に必ず読む）

### 0.1 コードベースの現状（pull 反映済み）
- `git log` に phase 0〜5C（refactoring / native compiler / 神クラス分割）が入っている。
- **`src/python_converter/` の6ファイルは構造不変**（`use` 整理のみ）。本書の主要編集対象は安定している。
- **interpreter 側は god-class 分割で再編済み。行番号は移動している** → 本書は行番号でなく**関数名・`match` アーム・シンボル名**で位置を指す。着手時は `Grep` で現在位置を取り直すこと。
- **`impl_python/` に python_converter 相当は無い**（`import[py]` のソース翻訳は Rust 専用機能）。→ **変換器の変更は impl_python ミラー同期・git SHA 更新の対象外**。interpreter 側に手を入れる項目（7・25）のみ、impl_python に並行実装があるかを個別確認する。

### 0.2 変換器の構造（エントリポイント）
```
convert_python_source(source, filename)          … src/python_converter/mod.rs
  └ convert_stmts / convert_stmts_fn_body         … statements.rs
      └ convert_stmt(stmt, filename)              … statements.rs  ★文の match 本体
          └ convert_expr(expr, filename)          … expressions.rs ★式の match 本体
          └ convert_constant / convert_binop / convert_cmpop / convert_augop … expressions.rs
          └ convert_class / convert_params / collect_self_fields / extract_param_types … classes.rs
          └ convert_annotation / map_type_name    … annotations.rs
          └ is_self / is_main_guard / expr_to_name … utils.rs
```
呼び出し元（再帰ロード側）: `Parser::load_python_module` … [`src/parser/imports/py_modules.rs`](src/parser/imports/py_modules.rs)

### 0.3 参照専用（現在位置・grep アンカー）
| 用途 | シンボル | 現在ファイル |
|---|---|---|
| Python 関数の kwargs 自動注入（項目7） | `extra_kwargs` / `declare_var("kwargs"...)` / `is_python` | `src/interpreter/functions/execution.rs`, `args.rs`（`bind_args_relaxed`） |
| ファイルの Drop クローズ（項目25 根拠） | `impl Drop for FileData` / `fn close` | `src/interpreter/value/objects.rs` |
| デコレータ適用（項目20 根拠） | 逆順適用ループ / method decorator | `src/interpreter/exec/definitions.rs` |
| f-string 脱糖の参照形（項目19） | `fn desugar_fstring` | `src/parser/exprs.rs` |
| 再帰ロード基盤（項目27） | `fn load_python_module`（`module_cache`/`self.loading`） | `src/parser/imports/py_modules.rs` |
| AST ノード定義（全般） | `Stmt` / `Expr` / `Param` / `FieldKind` / `BinOp` enum | `src/ast.rs` |

### 0.4 テスト手順（実機）
1. `cargo build`（変換器は Rust 側のみ）。
2. `.py`（変換対象）と、それを `import[py]` する `.ar`（ドライバ）を同一ディレクトリに置く。
3. 実行: `cargo run -- -src driver.ar`（またはビルド済み `target/debug/arrow.exe -src driver.ar`）。
4. 期待どおりの出力／期待どおりの明示エラーを確認。

### 0.5 規約（このプロジェクト固有）
- **新文法を通したら `examples/` に確認用 `.ar` を追加**。エラー化する項目は `_error` サフィックスの失敗例も追加（`.claude/rules/regulations.md`）。`.py` 側は `examples/interop/test_modules/` 等に置く。
- 同じ実行を繰り返すなら `.ps1` 化する。
- ファイル新規作成後は `./generate-codebase-map.ps1` を実行。
- **VS Code 拡張・VSIX 再生成は原則不要**（本修正は「Python→Arrow 変換」内部処理であり、Arrow の新文法追加ではない）。
- 変換器は Rust 専用のため **impl_python 同期不要**。項目7・25 の interpreter 変更のみ impl_python 並行実装の有無を確認。
- `git commit` はユーザー許可を得るまで行わない。挙動が変わる変更は sub-branch 提案を検討。

---

## 1. 共通基盤（先に用意すると複数項目が楽になる）

複数項目が依存するため、着手順の先頭で整備する。

### INF-A: スコープ単位の再代入対応（＝項目2の中核）✅ **実装済（2026-08-28）**
> 実装: `convert_scope` / `convert_stmts(…, declared)` / `collect_assigned_names` / `assign_or_declare`
> （`statements.rs`）。旧 `convert_stmts_with_hoist` / `convert_stmt_in_hoist_ctx` /
> `convert_stmts_hoisted_branch` / `collect_if_branch_assigns` は**削除**。詳細は coverage 項目 2。

- ファイル: `statements.rs`
- 現状: 既存の巻き上げは `collect_if_branch_assigns`（`if` ブランチのみ）＋ `convert_stmts_with_hoist`。全代入を `Stmt::Mut`（新規宣言）にしているため再代入で `NameError: already declared`。
- 方針: **スコープ（関数本体／モジュール本体）ごとに、単純名前代入される全変数を再帰収集 → スコープ先頭で `mut name = None` を一度だけ宣言 → 以降の `x = expr` はすべて `Stmt::Assign`（再代入）に変換**。
  - 収集は `if`/`for`/`while`/`try`/`block` の全ネストを走査（既存 `collect_if_branch_assigns` を汎用化して置換）。
  - 除外: パラメータ名、`for` ループ変数（これらは別途宣言済み）。
- 影響: `convert_stmt` の `Assign`/`AnnAssign`（Name ターゲット）を `Stmt::Mut` 固定でなく「初回=Mut/以降=Assign」判定に変更。→ 判定用に「宣言済み集合」を引数で引き回すか、`convert_stmts_with_hoist` で全 hoist して常に `Assign` にする（後者が単純で堅牢）。

### INF-B: 式コンテキストから囲みスコープへ文を注入する機構
- ファイル: `statements.rs` / `expressions.rs`
- 用途: 項目15（複数代入の一時変数）・23（walrus）・26（lambda lifting）。
- 方針: `convert_expr` が「この式の前に実行すべき補助文」を外へ持ち出せるようにする。実装案:
  - `convert_expr` 群に `&mut Vec<Stmt> hoist_out` を引数追加し、補助文（`fn __lambda_N ...` / `x = expr`）を push、式側はその参照（`Ident`）を返す。
  - `convert_stmt` は各文を変換する際にローカル `hoist_out` を用意し、生成された補助文を**当該文の直前**に挿入する。
- 難易度: 中（シグネチャ変更が広域に及ぶ）。**先に用意してから 15/23/26 に着手**。

### INF-C: `convert_stmt` の複数文返却
- ファイル: `statements.rs`
- 用途: 項目15（`a = b = c` を2文へ）。
- 方針: `convert_stmt` の戻り値 `Result<Option<Stmt>, String>` を `Result<Vec<Stmt>, String>` 化（呼び出し側 `convert_stmts_*` の push を extend に）。または INF-B の `hoist_out` に追加文を積んで対応。どちらか一方で足りる。

---

## 2. 実装カード

各カードの見方:
- **編集**: `ファイル` — アンカー（関数/`match` アーム）／変更内容。
- rustpython の型名・フィールド名は着手時に実物で確認（例: `ExprSlice{lower,upper,step}`）。

### フェーズ1: 独立・低リスク（式/文の単純マッピング）

#### [3] 添字/キー代入 `a[i]=x`, `d[k]=v`（+ `a[i]+=1`）✅ **実装済（2026-08-28）**
- 計画どおり。`Attribute` を受けていた 3 アーム（`Assign`/`AugAssign`/`AnnAssign`）を
  `Subscript` にも広げるだけ。入れ子もそのまま通る。8 ケース CPython 一致。
- ⚠ これで `test_modules/py_calculator.py` が `import[py]` で読めるようになった。
- ⚠ 併せて項目 2 の不具合を修正: `collect_assigned_names` が `if __name__ == "__main__":` の
  中まで降りて巻き上げ、取り込み側と `already declared` で衝突していた。
- 例題: `examples/interop/py_subscript.ar` + `test_modules/py_subscript.py`。
- 編集: `statements.rs` `convert_stmt`
  - `Assign` アーム: `match target` に `py::Expr::Subscript(_) => { ... }` を追加し、`Attribute` と同じく `Stmt::AttrAssign { target: convert_expr(target)?, value }` を生成。
  - `AugAssign` アーム: `target` が `Subscript` のとき `Stmt::AttrCompoundAssign { target, op, value }` を生成。
- 根拠: Arrow は添字代入を `AttrAssign`(target=Subscript式) で表現（パーサ `finish_expr_stmt`）。
- テスト: `d["k"]=5; return d` / `xs[0]+=1`。

#### [4] スライス `a[1:2]` / `a[::2]` ✅ **実装済（2026-08-28）**
- 計画どおり 1 対 1 の写し替え（`lower`/`upper`/`step` の `Option` をそのまま `begin`/`end`/`step` へ）。
- ⚠ Arrow のスライス意味論は Python 互換だった（負インデックス・負ステップ・範囲外切り詰め・
  str/tuple・`step==0` の `ValueError` 文言まで一致）。18 ケース CPython 一致。
- ⚠ スライス代入 `xs[1:3] = [...]` は**項目 3 と揃って初めて**成立する（例題 ⑧ で固定）。
- 例題: `examples/interop/py_slice.ar` + `test_modules/py_slice.py`。
- 編集: `expressions.rs` `convert_expr`
  - `py::Expr::Slice(_) => Err(...)` を、`Expr::Slice { begin: lower.map(convert→Box), end: upper.map(...), step: step.map(...) }` へ差し替え。
- テスト: `xs[1:3]`, `xs[::2]`, `xs[:-1]`。

#### [12] `in` / `not in` ✅ **実装済（2026-08-28）**
- `convert_cmpop` で 2 行返すだけ。コンテナごとの意味（list/tuple/set=要素、dict=キー、
  str=部分文字列）も Python と一致。11 ケース CPython 一致。
- 例題: `examples/interop/py_membership.ar` + `test_modules/py_membership.py`。
- 編集: `expressions.rs` `convert_cmpop`（または `Compare` アーム）
  - `CmpOp::In => BinOp::In`、`CmpOp::NotIn => BinOp::NotIn` を返す（現状は Err）。
- テスト: `t in xs`, `t not in xs`。

#### [13] `is` / `is not`（★文法差異）
- 編集: `expressions.rs` `Compare` アーム（`convert_cmpop` では `not` ラップを表現できないため Compare 側で特別扱い）
  - `CmpOp::Is` → `Expr::BinOp{ op: RefEq, .. }`（Arrow の `===`。Python `is` は識別比較で **Arrow の `is`(=型ガード) とは別物**）。
  - `CmpOp::IsNot` → `Expr::UnaryOp{ Not, BinOp{RefEq} }`（`!==` は存在しない）。
- テスト: `x is None`, `x is not None`, オブジェクト同一性。

#### [11] 三項演算子 `a if cond else b` ✅ **実装済（2026-08-28）**
- 計画どおり。`return_type: None` でも評価できることを実機確認。遅延評価（選ばれた腕しか
  評価しない）が CPython と一致することも例題で固定した。17 ケース CPython 一致。
- 例題: `examples/interop/py_ternary.ar` + `test_modules/py_ternary.py`。
- 編集: `expressions.rs` `convert_expr`
  - `py::Expr::IfExp(_)` を `Expr::IfExpr { branches: vec![(convert(test), vec![Stmt::BlockReturn(convert(body), span)])], else_body: Some(vec![Stmt::BlockReturn(convert(orelse), span)]), return_type: None }` へ。
- 根拠: `return_type: None` でも式評価可（実機確認済み）。
- テスト: `x = (1 if c else 2)`、呼び出し引数内 `f(a if c else b)`。

#### [18] 定数タプル
- 編集: `expressions.rs` `convert_constant`
  - `Constant::Tuple(items) =>` 各要素を（`convert_constant` 相当で）`Expr` 化し `Expr::Tuple(...)` を返す（現状 Err）。
- テスト: 定数タプルが出る文脈（デフォルト値等）。

#### [19] f-string
- 編集: `expressions.rs` `convert_expr`
  - `py::Expr::JoinedStr(j) =>` 各要素を変換して左結合 `BinOp::Add` で連結: `Constant(str)`→`Expr::Str`、`FormattedValue{value}`→`Expr::Call{ func: Ident("str"), args:[convert(value)] }`。
  - 参照実装: `desugar_fstring`（`src/parser/exprs.rs`）と**同形**にする。
- 制約: `format_spec`（`{x:.2f}`）・`conversion`（`!r`/`!s`）付きは当面 Err（要追加検討）。
- テスト: `f"hi {name} n={n}"`。

#### [22] 集合リテラル `{1,2,3}` / set 内包
- 編集: `expressions.rs` `convert_expr`
  - `py::Expr::Set(s) => Expr::Set(s.elts.map(convert))`（現状 Err）。
  - `SetComp` は項目17 の for 式を `set(...)` で包む（後回し可）。
- 根拠: Arrow は set 型実在（実機確認）。
- テスト: `{1,2,3,2}` → `{1,2,3}`, `2 in s`。

#### [20] デコレータ `@decorator` ✅ **実装済（2026-08-27）**
- 実装: 新設 `src/python_converter/decorators.rs` の `convert_decorators()` に集約。
  `statements.rs` の `FunctionDef` アーム / `classes.rs` の `convert_class`（クラス本体・メソッド）
  の 3 箇所の Err 分岐を、この関数の呼び出しに置き換えた。
- **単純な素通しでは不十分**だった: Arrow に `staticmethod` / `classmethod` という**組込関数が無い**ため、
  `decorators` に積むと実行時 `NameError` になる。`convert_decorators` は
  `@staticmethod`→`is_static` / `@classmethod`→`is_class_method` /
  `@abstractmethod`(`abc.` 付きも)→`is_abstract` の**フラグに振り替える**。
- 明示エラー: `@property` / `@cached_property` / `@x.setter` / `@x.getter` / `@x.deleter`、
  モジュール直下の `@staticmethod` 等、`@staticmethod` と `@classmethod` の併用。
- 例題: `examples/interop/py_decorators.ar` + `test_modules/py_decorators.py`（CPython と出力一致）/
  `examples/interop/py_decorators_error.ar` + `test_modules/py_dec_*_error.py`。
- ⚠ 意味差: Arrow の `static` / `class_method` は**クラス経由でしか呼べない**（Python は
  インスタンス経由も可）。
- ⚠ **本項目の外で見つかった別バグ 2 件**（詳細は coverage の 20 節 †）:
  ① `mut` パラメータが入れ子 `fn` にキャプチャされない（**純 Arrow で再現**）→
  「クロージャで包む」典型的な Python デコレータが動かない。
  ② `.py` のモジュール直下から同モジュールの関数を呼べない（`NameError`）。

#### [21] `...`（Ellipsis）→ 文位置は `pass`
- 編集: `statements.rs` `convert_stmt` `Expr` アーム
  - 式文の中身が `Constant::Ellipsis` なら `Stmt::Pass` を返す。
- 値位置の `...` は現状維持（`convert_constant` の `Ellipsis => Expr::None`、変更不要）。
- テスト: `def f(): ...` / `class C: ...`。

#### [1] デフォルト引数 `def f(x, y=10)` ✅ **実装済（2026-08-28）**
- 実装（変換器）: `classes.rs` `convert_params` の 2 ループで `arg.default` を `convert_expr` して
  `Param.default` へ。rustpython 0.4 の `ArgWithDefault` は**引数ごと**に `default` を持つので、
  計画に書いた「末尾詰めの対応づけ」は**不要だった**。
- ⚠⚠ **変換器だけでは動かない**。静的型検査 `check_fn_type_call`（`src/type_check/call_check.rs`）が
  `arg_data.len() != params.len()` で弾いていた。`FnTypeParam` に `has_default` を足し、
  必要数を数えるよう修正（`types.rs` / `stmt/resolve.rs` / `call_check.rs`）。
  **`.ar` ネイティブモジュールでも同じく壊れていた**ので、そちらも同時に直った。
  ⚠ `impl_python` にも同じ経路があるが**触らない方針**（古いため）。踏む例題が無く
  `compare_python_impl.ps1` は緑のまま。**同期時の積み残し**として記録。
- ⚠ 意味差: デフォルトは Python が **def 時 1 回**、Arrow は**呼び出しごと**に評価する。
  リテラルは同じ。**可変デフォルト（`def f(xs=[])`）だけ結果が違う**（許容と判断）。
- ⚠ 副作用: 未対応のデフォルト式（lambda・f-string 等）が**明示エラー**になる（従来はサイレント欠落）。
- 例題: `examples/interop/py_defaults.ar` / `py_defaults_error.ar` + `test_modules/py_defaults*.py`。
- ゲート: `compare_python_impl.ps1` は変換器の例題 5 本を **knownDiff に登録**した
  （impl_python に python_converter 相当が無く原理的に一致しない。理由は実測確認済み）。

#### [24] bare `*`（キーワード専用引数区切り）✅ **実装済（2026-08-28）**
- **コード変更なし**。既存の `convert_params` が既に kwonly を通常引数へ平坦化しており、
  6 形（bare `*` 1個/2個、`*` のみ、`/` 併用、`__init__`、位置渡し）すべて実機で期待どおりだった。
- 実施したのは ①`convert_params` の doc コメントに**平坦化の方針と意味の緩和**を明文化、
  ②例題で挙動を固定、の 2 点。
- ⚠ 緩和: Python はキーワード専用引数の位置渡しを `TypeError` にするが、Arrow は通す
  （`Param` に「位置渡し禁止」フラグが無いため）。**受け入れる Python が広がる方向**なので許容。
- ⚠ 干渉: kwonly の**デフォルト値は落ちる**（項目 1）／**実体のある `*args` と併用すると壊れる**
  （項目 6。bare `*` 単体は無害）。
- 例題: `examples/interop/py_kwonly.ar` + `test_modules/py_kwonly.py`。エラー化なしのため `_error` 例は無し。

#### [5] クラス変数 → `StaticMut`
- 編集: `classes.rs` `convert_class`（クラス本体の `Assign`/`AnnAssign` アーム）
  - `FieldKind::Const` を `FieldKind::StaticMut` に変更（Python の可変クラス属性に合わせる）。`type_ann` は注釈があればそれ、無ければ `"Any"`。
- テスト: `class C: count = 0` をインスタンス/クラス経由で読み書き。

### フェーズ2: 中リスク（本体リライト／サブセット）

#### [2] 変数の再代入 … **INF-A** を実施（上記 §1）✅ **実装済（2026-08-28）**
- 旧実装は「トップレベルの `if` のブランチ内代入だけ」を巻き上げており、`for`/`while`/`try` の
  本体に降りると巻き上げ集合を捨てていた（＝ネストで壊れるドリフト）。スコープ単位の完全巻き上げに置換。
- ⚠ 残る意味差 1 件: `=` した名前を `for` のループ変数にも使い**ループ後に読む**と、Arrow は
  代入時の値に戻る（`for` が自前スコープで束縛するため）。エラー化はしていない。
- 例題: `examples/interop/py_reassign.ar` + `test_modules/py_reassign.py`（12 ケース中 11 件 CPython 一致）。

#### [6] `*args`
- 編集: `classes.rs` `convert_params` ＋ 本体リライト
  - vararg を `Param { name:"...", variadic:true, mutable:true, type_ann:Some("list[Any]") }` に（現状 `name:"*args", variadic:false`）。
  - **関数本体内の vararg 名参照を `Expr::LocalVar("args")` に書き換え**る識別子リライト（`statements.rs`/`expressions.rs` に本体走査を追加、または変換後 AST を後処理）。
- 根拠: Arrow の可変長は `local::args` 参照（`args.rs` の `bind_args`）。
- テスト: `def f(*xs): return xs[0]` を `f(10,20)`。

#### [7] `**kwargs`
- 編集:
  - `classes.rs` `convert_params`: Python kwarg 名が `kwargs` 以外なら本体の当該 `Ident` を `Ident("kwargs")` にリライト。
  - `src/interpreter/functions/execution.rs`: 余剰キーワードが空でも `kwargs` を空 dict で注入するよう `!extra_kwargs.is_empty()` 条件を緩和（未注入時の `NameError` 回避）。← **interpreter 変更。impl_python 並行実装の有無を確認**。
- テスト: `def f(**kw): return kw` を `f(a=1)` と `f()` の両方。

#### [8] `match` 文（値/`_` サブセット）
- 編集: `statements.rs` `convert_stmt`
  - `py::Stmt::Match(_) => Err` を、`Stmt::Match { subject, arms }` 生成に。各 `case <リテラル/値>:`→`MatchPattern::Case(convert)`、`case _:`→`MatchPattern::Case(Expr::Ident("_"))`。
  - クラス/キャプチャ/シーケンス/マッピング/OR/ガードパターンは**明示 Err**。
- テスト: リテラル match、`case _`。`_error` 例にキャプチャパターン。

#### [9] ジェネレータ（`def`+`yield`）
- 編集: `statements.rs`
  - `FunctionDef` 変換時、本体に `yield` 文を含むなら `Stmt::FnDef` でなく `Stmt::GenDef`（`yield_type` は `Generator[T]`/`Iterator[T]` 注釈から抽出、無ければ None）を生成。
  - `yield x` 文（`Expr` 文中の `py::Expr::Yield`）→ `Stmt::Yield(convert)`。`yield from`・yield 式の値利用は**明示 Err**。
- テスト: `def g(): yield 1; yield 2` を for で回す。`_error` 例に `yield from`。

#### [10] 型エイリアス `type X = ...`
- 編集: `statements.rs`（`TypeAlias`）＋ `annotations.rs`
  - **推奨: 変換器内エイリアステーブル**（透過展開）。`type X = <型式>` を検出したら `X → convert_annotation(rhs)` を map に登録し、`convert_annotation`/`map_type_name` が `X` を展開。
  - map をどこに持つか（スレッド越しの状態 or 引数引き回し）を設計。`new_type` 出力案は名目的別型で偽陽性のため非推奨。
- テスト: `type V = list[int]` を注釈に使用。

#### [16] 連鎖比較 `a<b<c`
- 編集: `expressions.rs` `Compare` アーム
  - `ops.len()>=1` を許可し、隣接ペアを `and` で連結: `a op1 b op2 c` → `(a op1 b) and (b op2 c)`。
- 注意: 中間オペランド2回評価は**許容**（ユーザー方針）。
- テスト: `1 < x < 10`。

#### [17] 内包表記（単一 for / 多重 for / フィルタ）
- 編集: `expressions.rs` `ListComp` アーム
  - `[elt for x in it if c ...]` → 先頭 generator を外側 `Expr::ForExpr{ target, iter, body, return_type:Some("list[Any]") }`、2つ目以降 generator を body 内の入れ子 `Stmt::For`、各 `ifs` を `Stmt::If` ラップ、`elt` を最深部の `Stmt::LoopYield`。
- 根拠: `loop_yield` は入れ子 for 文/if 文を透過して最外 for 式へ積まれフラット化（実機確認: 2重/3重/フィルタ）。
- サブセット: set/dict/generator/async 内包は当面 Err（set は `set(...)` 包みで後追い可）。
- テスト: `[x*y for x in a for y in b]`、フィルタ付き。

### フェーズ3: INF-B/INF-C 依存（式→文注入）

#### [15] 複数代入 `a=b=c`
- 前提: INF-B または INF-C。
- 編集: `statements.rs` `Assign` アーム（`targets.len()!=1` の Err を置換）
  - 忠実版: `let __t = c; a = __t; b = __t`（RHS 1回評価）。単純版 `a=c; b=c` はエイリアス差（`a=b=[]`）に注意。
- テスト: `a=b=0`、`a=b=[]` の共有確認。

#### [23] walrus `:=`
- 前提: INF-B。
- 編集: `expressions.rs` `NamedExpr` アーム
  - `(x := expr)` → 補助文 `x = expr` を hoist_out に push、式は `Ident("x")` を返す。
- テスト: `if (n := len(xs)) > 10:`。

#### [26] lambda → 名前付き関数持ち上げ
- 前提: INF-B。
- 編集: `expressions.rs` `Lambda` アーム
  - `fn __lambda_N(params) -> Ret: return <body>` を hoist_out に push、式は `Ident("__lambda_N")` を返す。連番 N はカウンタ。
- 注意: `fn` は戻り型注釈が要る（`MissingReturnTypeAnn`）→ 推論 or `Any` 補完。デフォルト引数は項目1と併用。
- テスト: `sorted(xs, key=lambda x: -x)` 相当、クロージャ捕捉。

### フェーズ4: 大きめ／実行時ガード

#### [25] `with`（`__exit__` 無しのみ block 脱糖）
- 編集: `statements.rs` `With` アーム（現状 Err）
  - `with EXPR as x: body` → `Stmt::Block([ mut x = EXPR, <runtime __exit__ guard>, body... ])`。
  - runtime guard: `x` のクラスに `__exit__` メソッドがあれば `RuntimeError` を raise（実行時判定。同一モジュール定義クラスは変換時静的検出も可）。
  - `__enter__` が別値を返すケースの束縛は要考慮。
- 根拠: block 退出で `FileData::drop`→close（実機根拠）。詳細は coverage 🟡。
- テスト: `with open(p) as f: f.write(...)`（成功）、`__exit__` 定義クラスの with（`_error`）。

#### [27] Python モジュール内 import（再帰ロード）
- 編集:
  - `statements.rs`: `Import`/`ImportFrom` の `Ok(None)` を、Arrow の `Stmt::Import`/`FromImport`（lang="py"、body は空）生成に変更。相対 import・`as`・`a.b.c` をマッピング。
  - `src/parser/imports/py_modules.rs` `load_python_module`: `convert_python_source` の戻り body を走査し、`Import`/`FromImport` を `self.load_python_module` で**再帰充填**（既存の `module_cache`/`self.loading` 循環検出を再利用）。
- 制約: stdlib/native（`os`/`numpy` 等）は翻訳不能 → `import[py-int]` フォールバック or 明示 Err。
- テスト: ローカル `.py` 同士の import 連鎖、循環 import。

### フェーズ5: 明示エラー化・警告（🟠 / 🔴 / del）

`statements.rs` / `expressions.rs` に集約。**現状サイレント欠落しているものはエラー化が必要**（黙って壊れる解消）。

| 項目 | 編集箇所 | 変更 |
|---|---|---|
| for/else | `For` アーム | `!f.orelse.is_empty()` で明示 Err |
| while/else | `While` アーム | `!w.orelse.is_empty()` で明示 Err |
| try/else | `Try` アーム | `!t.orelse.is_empty()` で明示 Err |
| except (A,B)/属性型 | `Try` の handler | `eh.type_` が単純 `Name` 以外なら明示 Err |
| raise X from Y | `Raise` アーム | `r.cause.is_some()` で明示 Err |
| assert | 新規 `Assert` アーム | 専用メッセージで Err（現状は汎用 catch-all） |
| global/nonlocal | `Global`/`Nonlocal` アーム | `Ok(None)` を明示 Err に（🔴。`mut` 外側変数で代替する旨） |
| del | 新規 `Delete` アーム | Name ターゲット→`eprintln!` 警告＋`Ok(None)`。Subscript/Attribute ターゲット→明示 Err |

- 各エラー化は `_error.ar` 例を追加。del は警告動作の確認例を追加。

### 仕様確定（コード変更なし or 任意）
- 整数 i64 切り詰め（`convert_constant`）: 仕様。変更不要（将来「範囲外は Err」への変更余地のみ注記）。
- 全パラメータ `mutable:true`（`convert_params`）: 仕様。**任意対応**: `is_python` 関数の呼び出しで immutable 実引数を許容するよう型検査 `CallMutParamWithImmutableArg` を緩和（`src/type_check/`）すると UX 改善。

---

## 3. 推奨着手順

1. **フェーズ1**（~~3~~・~~4~~・~~12~~・13・~~11~~・18・19・22・~~20~~・21・~~1~~・~~24~~・5）— 独立・低リスク。1件ずつ通して examples を積む。
   （20 は 2026-08-27、24・1 は 2026-08-28 に完了）
2'. ~~**INF-A → 項目2**（再代入）~~ — 2026-08-28 に完了。
2. **INF-A → 項目2**（再代入）— 影響大・頻出。フェーズ1 と並行可。
3. **フェーズ2**（6・7・8・9・10・16・17）。7 は interpreter 変更を含む。
4. **INF-B/INF-C → フェーズ3**（15・23・26）— 共通基盤を整えてから。
5. **フェーズ4**（25・27）— 設計比重大。
6. **フェーズ5**（明示エラー化・del）— まとめて。サイレント欠落の解消を優先すると安全側に倒れる。

## 4. 各項目の Done 条件
- [ ] `cargo build` / `cargo clippy` 通過。
- [ ] 成功例 `.ar`（＋必要なら `.py`）が期待出力。エラー化項目は `_error.ar` が期待エラー。
- [ ] `examples/` に配置し `./generate-codebase-map.ps1` 実行。
- [ ] interpreter を触った項目（7・25・仕様緩和）は impl_python 並行実装の要否を確認、必要なら git SHA 更新。
- [ ] 挙動変更が大きい項目は sub-branch 提案の要否を確認。
