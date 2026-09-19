# python_converter 修正 実装計画（作業指示書）

Python→Arrow 変換器（`import[py]` が使う、PyO3 を介さない翻訳器）の修正を、**まっさらなコンテキストからでも即着手できる**ように整理した作業指示書。
分類の背景・根拠は [python_converter_coverage.md](python_converter_coverage.md)（判断記録）を参照。本書は「どのファイルをどう直すか」に特化する。

- 対象サブシステム: [`src/python_converter/`](../src/python_converter/)
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
呼び出し元（再帰ロード側）: `Parser::load_python_module` … [`src/parser/imports/py_modules.rs`](../src/parser/imports/py_modules.rs)

### 0.3 参照専用（現在位置・grep アンカー）
| 用途 | シンボル | 現在ファイル |
|---|---|---|
| Python 関数の kwargs 束縛（項目7） | `fn bind_args_relaxed` の `kwargs_idx` 分岐（`PY_KWARGS_PARAM` で束縛）| `src/interpreter/functions/args.rs` |
| ファイルの Drop クローズ（項目25 根拠） | `impl Drop for FileData` / `fn close` | `src/interpreter/value/objects.rs` |
| デコレータ適用（項目20 根拠） | 逆順適用ループ / method decorator | `src/interpreter/exec/definitions.rs` |
| f-string 脱糖の参照形（項目19） | `fn desugar_fstring` | `src/parser/exprs.rs` |
| 再帰ロード基盤（項目27） | `fn load_python_module`（`module_cache`/`self.loading`） | `src/parser/imports/py_modules.rs` |
| AST ノード定義（全般） | `Stmt` / `Expr` / `Param` / `FieldKind` / `BinOp` enum | `src/ast.rs` |
| `.pyi` 解決（群6） | `fn load_python_interface_module` / `fn load_py_type_body` | `src/parser/imports/py_modules.rs` |
| スタブの行ベース抽出（群6） | `fn extract_py_type_stubs` / `fn py_type_to_arrow` | `src/parser/imports/mod.rs` |
| 検索パス（群6 S2） | `Parser::python_search_dirs`（**`Interpreter` 側と別物**） | `src/parser/imports/cs_js_modules.rs` |
| `mut` 実引数検査（INF-D） | `fn check_mut_param_arg`（緩い側） / `fn is_mutable_expr`（厳しい側） | `src/type_check/call_check.rs` |

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

### INF-B: 式コンテキストから囲みスコープへ文を注入する機構 ✅ **実装済（2026-09-19）**
> 実装: 新設 [`hoist.rs`](src/python_converter/hoist.rs)。⚠ **`convert_expr` に
> `&mut Vec<Stmt>` を足す案は採らなかった** —— 呼び出しが **59 箇所**あり、
> `supers.rs` / `param_rewrite.rs` が同じ理由で既にスレッドローカルを採っている。
> `convert_stmts` / `convert_scope` が**文ごと**にバッファを積み、`convert_one_into` が
> 補助文を**本体の前**に並べる。
- ファイル: `statements.rs` / `expressions.rs`
- 用途: 項目15（複数代入の一時変数）・23（walrus）・26（lambda lifting）。
- 方針: `convert_expr` が「この式の前に実行すべき補助文」を外へ持ち出せるようにする。実装案:
  - `convert_expr` 群に `&mut Vec<Stmt> hoist_out` を引数追加し、補助文（`fn __lambda_N ...` / `x = expr`）を push、式側はその参照（`Ident`）を返す。
  - `convert_stmt` は各文を変換する際にローカル `hoist_out` を用意し、生成された補助文を**当該文の直前**に挿入する。
- 難易度: 中（シグネチャ変更が広域に及ぶ）。**先に用意してから 15/23/26 に着手**。

### INF-C: `convert_stmt` の複数文返却 ✅ **実装済（2026-09-19）**
> `Result<Option<Stmt>, String>` → `Result<Vec<Stmt>, String>`。呼び出しは 2 箇所だけで安かった。
- ファイル: `statements.rs`
- 用途: 項目15（`a = b = c` を2文へ）。
- 方針: `convert_stmt` の戻り値 `Result<Option<Stmt>, String>` を `Result<Vec<Stmt>, String>` 化（呼び出し側 `convert_stmts_*` の push を extend に）。または INF-B の `hoist_out` に追加文を積んで対応。どちらか一方で足りる。

### INF-D: `mut` パラメータへの一時値渡しを、ネイティブと同じ規則に揃える ✅ **実装済（2026-09-18）**

> **この文書で唯一「変換器の外」が主因の基盤タスク**。§2 の「仕様確定」に *任意対応* として
> 書いていたものを、**群6（同梱スタブ）の前提として必須に格上げ**した。

- ファイル: [`src/type_check/call_check.rs`](../src/type_check/call_check.rs)
- **現状は 2 つの検査が食い違っている**（実測。同じ `CallMutParamWithImmutableArg` を出す）:

| 経路 | 述語 | リテラル実引数 |
|---|---|---|
| ネイティブ `fn`（`FnSig`）| `check_mut_param_arg` → `path_is_mutable(e) != Some(false)` | **通る**（`None` を返すため） |
| モジュールメンバ（`FnTypeParam`）| `is_mutable_expr(e)` → `Expr::Ident` 以外は `false` | **弾かれる** |

  ```text
  fn f(mut x: int) -> int: return x * 3
  print(f(5))                    # → 15（通る）

  # 同じ形を .py / .pyi 経由にすると
  m.triple(5)                    # → StaticTypeError: parameter 'x' of 'triple'
                                 #    expects a mutable argument, but got an immutable value
  ```
- ⚠ `check_mut_param_arg` 側の doc が正しい根拠を書いている —— **「根が識別子でない式
  （リテラル・呼び出しの戻り値）は一時値で誰とも共有していないので通す。ここを弾くと
  `g([1, 2])` のような正しい呼び出しまで落ちる」**。B11 が守りたいのは「`let` 変数を
  `mut` パラメータへ渡して呼び出し元が書き換わる」形であって、一時値ではない。
- 方針: `FnTypeParam` 側の 2 箇所（`call_check.rs` の `param.mutable && !self.is_mutable_expr(arg_expr)`）を
  **`path_is_mutable(arg_expr) == Some(false)` 判定へ寄せる**。`is_mutable_expr` は他に消費者が
  無ければ削除する（`stale_doc_refs.ps1` が拾う）。
  - ⚠ **`is_python` 限定の緩和にはしない**。食い違いは py 固有ではなく `FnTypeParam` 経路
    全体（`import[ar]` のモジュールメンバ・cs/js スタブ）に効いており、片側だけ緩めると
    「同じ関数が呼び方で通ったり落ちたりする」形が残る。
- 影響: **受け入れる呼び出しが広がる方向のみ**（今まで通っていた形は落ちない）。
- ゲート: `type_obligations.ps1`（型義務の退行検出）＋ `scan_examples.ps1` ＋
  `compare_wasm_frontend.ps1`（型検査を触るため必須）。
  `frontend_tests/type_check_tests/bridge_mutability.rs` は **書き換えが要る**
  （リテラルを弾くことを固定しているテストが残っていないか確認する）。

---

## 2. 実装カード

各カードの見方:
- **編集**: `ファイル` — アンカー（関数/`match` アーム）／変更内容。
- rustpython の型名・フィールド名は着手時に実物で確認（例: `ExprSlice{lower,upper,step}`）。

### 群1: 式・文の単純マッピング

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

#### [13] `is` / `is not`（★文法差異）✅ **実装済（2026-08-28）**
- 計画どおり `Compare` アーム側で処理（`is not` は `Not(RefEq)` ラップが要るため）。
- ⚠ 残る意味差 2 件: 不変プリミティブ（計算で作った str / 256 超の int）は Arrow が `True`、
  CPython は `False`（インターン依存）。CPython 自身が SyntaxWarning を出す使い方なので
  エラー化はしない。13 ケース中 11 件 CPython 一致。
- 例題: `examples/interop/py_identity.ar` + `test_modules/py_identity.py`。
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

#### [18] 定数タプル ✅ **実装済（2026-08-28）／ただし現構成では到達しない経路**
- ⚠ `Constant::Tuple` を作るのは rustpython の `ConstantOptimizer` だけ（`constant-optimization`
  フィーチャ限定）で、`Suite::parse` は畳み込みをしない。通常のタプルは常に `py::Expr::Tuple`。
- 将来フィーチャを有効にしても壊れないよう `constant_value_to_expr` を切り出して再帰変換を実装。
  例題は実際に通る経路（`Expr::Tuple`）を 16 ケースで固定。
- ⚠ 検査中に **Arrow 本体のタプルの穴 3 件**を発見（未修正・詳細は coverage 項目 18）:
  ①タプルを dict キーにすると**黙って消える** ②タプル同士の `+` が未対応
  ③`list` の `==` が値比較でない（タプルは正しい）。
- 例題: `examples/interop/py_tuple.ar` + `test_modules/py_tuple.py`。
- 編集: `expressions.rs` `convert_constant`
  - `Constant::Tuple(items) =>` 各要素を（`convert_constant` 相当で）`Expr` 化し `Expr::Tuple(...)` を返す（現状 Err）。
- テスト: 定数タプルが出る文脈（デフォルト値等）。

#### [19] f-string ✅ **実装済（2026-08-28・書式指定を除く）**
- `desugar_fstring` と同形に脱糖（リテラル片 + `str(...)` を左結合 `+` で連結）。14 ケース CPython 一致。
- ⚠ 計画より広く対応: `!s` → `str()`、`!r` → `repr()` も通る（Arrow に組込があるため）。
  明示エラーは `!a`（ascii）と **書式指定 `{x:.2f}`**（Arrow に書式指定の構文・組込が無い）。
- ⚠ 書式指定の将来方針は `implementation_logs/FUTURE_FEATURE.md` §4 (3) に残した。
  **モジュール単位変換なので、1 箇所でも含む `.py` は import 全体が落ちる**点が効く。
- 例題: `examples/interop/py_fstring.ar` / `py_fstring_error.ar` + `test_modules/py_fstring*.py`。
- 編集: `expressions.rs` `convert_expr`
  - `py::Expr::JoinedStr(j) =>` 各要素を変換して左結合 `BinOp::Add` で連結: `Constant(str)`→`Expr::Str`、`FormattedValue{value}`→`Expr::Call{ func: Ident("str"), args:[convert(value)] }`。
  - 参照実装: `desugar_fstring`（`src/parser/exprs.rs`）と**同形**にする。
- 制約: `format_spec`（`{x:.2f}`）・`conversion`（`!r`/`!s`）付きは当面 Err（要追加検討）。
- テスト: `f"hi {name} n={n}"`。

#### [22] 集合リテラル `{1,2,3}` / set 内包 ✅ **実装済（2026-08-28・内包は項目 17 で完了）**
- リテラルは `py::Expr::Set` アームで要素を積むだけ。13 ケース CPython 一致。
- **set 内包も通る**。本項目の時点では `SetComp` を独立アームに分けて専用のエラー文言に
  していたが、項目 17 で `set(<リスト内包>)` への脱糖が入り解消した。
- ⚠ セットの repr 順は当てにしない（CPython は str のハッシュを実行ごとにランダム化する）。
- 例題: `examples/interop/py_set.ar` + `test_modules/py_set.py`。
  ⚠ **`py_set_error.ar` / `py_setcomp_error.py` は実在しない**（項目 17 で削除済み）。
  集合内包は `py_comprehension.ar` の ⑦ が持つ。
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

#### [21] `...`（Ellipsis）→ 文位置は `pass` ✅ **実装済（2026-08-28）**
- ⚠ **挙動ではなく AST の形の修正**（どちらでも実行結果は同じ）。検証は例題の出力ではなく
  AST の形で行った（`parser_tests.rs::test_python_statement_ellipsis_becomes_pass`）。
- ⚠ 値位置（`x = ...`）は `Expr::None` のまま据え置き（承認済み）。波及していないことを
  `test_python_value_ellipsis_stays_none` で固定。⇒ そこだけ CPython（`Ellipsis`）と表示が違う。
- 例題: `examples/interop/py_ellipsis.ar` + `test_modules/py_ellipsis.py`。
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

#### [5] クラス変数 → `StaticMut` ✅ **実装済（2026-08-28）**
- `Assign` / `AnnAssign` 両アームで `FieldKind::Const` → `StaticMut` に変えるだけ。
  以前は `C.count = ...` が `cannot assign to class variable (declared const)` で落ちていた。
- ⚠ 残る意味差: `self.x = ...` は Python では**インスタンス属性の新設**だが、Arrow の
  `static mut` は単一の記憶場所なので共有変数を書き換える。変換器では埋められないモデル差。
- ⚠ 作業中に **クラス継承が黙って落ちる**ことを発見（未トリアージ／coverage 参照）。
  Arrow はクラス継承を持たず（トレイトのみ）、変換器はパーサを通らないので
  エラーも出ずメソッド・フィールドが引き継がれない。**Python では継承が非常に多いので優先度高**。
- 例題: `examples/interop/py_classvar.ar` + `test_modules/py_classvar.py`。
- 編集: `classes.rs` `convert_class`（クラス本体の `Assign`/`AnnAssign` アーム）
  - `FieldKind::Const` を `FieldKind::StaticMut` に変更（Python の可変クラス属性に合わせる）。`type_ann` は注釈があればそれ、無ければ `"Any"`。
- テスト: `class C: count = 0` をインスタンス/クラス経由で読み書き。

### 群2: 本体リライト／サブセット

#### [2] 変数の再代入 … **INF-A** を実施（上記 §1）✅ **実装済（2026-08-28）**
- 旧実装は「トップレベルの `if` のブランチ内代入だけ」を巻き上げており、`for`/`while`/`try` の
  本体に降りると巻き上げ集合を捨てていた（＝ネストで壊れるドリフト）。スコープ単位の完全巻き上げに置換。
- ⚠ 残る意味差 1 件: `=` した名前を `for` のループ変数にも使い**ループ後に読む**と、Arrow は
  代入時の値に戻る（`for` が自前スコープで束縛するため）。エラー化はしていない。
- 例題: `examples/interop/py_reassign.ar` + `test_modules/py_reassign.py`（12 ケース中 11 件 CPython 一致）。

#### [6] `*args` ✅ **実装済（2026-08-28・項目 7 と同時）**
- ⚠ 計画は「変換器 + interpreter」だったが、**3 層**必要だった（変換器 / 静的型検査 / 束縛）。
  詳細は coverage 項目 7 にまとめてある。
- 本体の識別子差し替えは**変換中に**行う（`param_rewrite.rs` 新設。AST 再帰ウォーカを避けた）。
- ⚠ `*args` は list（CPython は tuple）。⚠ 入れ子 `fn` からの参照は VM 非適格（純 Arrow でも同じ）。
- 編集: `classes.rs` `convert_params` ＋ 本体リライト
  - vararg を `Param { name:"...", variadic:true, mutable:true, type_ann:Some("list[Any]") }` に（現状 `name:"*args", variadic:false`）。
  - **関数本体内の vararg 名参照を `Expr::LocalVar("args")` に書き換え**る識別子リライト（`statements.rs`/`expressions.rs` に本体走査を追加、または変換後 AST を後処理）。
- 根拠: Arrow の可変長は `local::args` 参照（`args.rs` の `bind_args`）。
- テスト: `def f(*xs): return xs[0]` を `f(10,20)`。

#### [7] `**kwargs` ✅ **実装済（2026-08-28・項目 6 と同時）**
- ⚠ 計画の「余剰キーワードが `kwargs` dict に自動注入される仕組みが既存」は**古い**。
  その仕組みは #33 で削除済みで、`extra_kwargs` は捨てられていた。今回あらためて束縛する。
- 番兵パラメータ名 `ast::PY_KWARGS_PARAM`（`"**kwargs"`）を使う。⚠ パラメータ名・束縛名・
  本体の参照を**同じ名前で揃える**こと（別名だとリゾルバがスロットに解決できず `NameError`）。
- ⚠ 定数は `src/ast.rs` に置く。wasm フロントエンドは `python_converter` を取り込まないため。
- 編集:
  - `classes.rs` `convert_params`: Python kwarg 名が `kwargs` 以外なら本体の当該 `Ident` を `Ident("kwargs")` にリライト。
  - ~~`src/interpreter/functions/execution.rs` の `!extra_kwargs.is_empty()` 条件を緩和~~
    ⚠ **計画時の見立て。実際の実装先は [`args.rs`](../src/interpreter/functions/args.rs) の
    `bind_args_relaxed`（`kwargs_idx` 分岐）**で、余剰が 0 個でも空 dict を束縛する。
    `execution.rs` 側は `extra_kwargs` を捨てるだけになっている（#33 以降）。
- テスト: `def f(**kw): return kw` を `f(a=1)` と `f()` の両方。

#### [8] `match` 文（値/`_` サブセット）
- 編集: `statements.rs` `convert_stmt`
  - `py::Stmt::Match(_) => Err` を、`Stmt::Match { subject, arms }` 生成に。各 `case <リテラル/値>:`→`MatchPattern::Case(convert)`、`case _:`→`MatchPattern::Case(Expr::Ident("_"))`。
  - クラス/キャプチャ/シーケンス/マッピング/OR/ガードパターンは**明示 Err**。
- テスト: リテラル match、`case _`。`_error` 例にキャプチャパターン。

#### [9] ジェネレータ（`def`+`yield`）

> ⚠⚠ **本カードは B13（2026-09-05）より前に書かれている。前提が 2 つ変わった**（A4 で更新）。

- **① `yield` の置き場所に静的な制約が入った**（`TypeErrorKind::YieldOutsideGenerator`・
  [`type_check/stmt/check.rs`](../src/type_check/stmt/check.rs)）。`yield` は **`gen` 本体の
  自フレーム直下だけ**。判定は `in_gen_body()` なので:
  - `if` / `for` / `while` / `try` の**入れ子の中は可**（Python の普通の書き方は通る）。
  - **入れ子の `def` の中の `yield` は不可** → **変換器で明示エラーにする必要がある**
    （Python では内側の `def` が別のジェネレータになるが、Arrow では表現できない）。
  - ⚠ これは体裁ではなく**コルーチン化の前提**（`yield` が自分のフレームにしか現れないから
    「中断＝`run_dispatch` を 1 回抜けるだけ」で済む）。緩められない。
- **② ジェネレータが真に遅延になった**。B13 以前は先行評価（本体を最後まで走らせて
  全 `yield` を `Vec` に集める）だったが、今は中断・再開する。
  実測: 無限ジェネレータ + `break` が正しく止まる。
  ⇒ カードの「サブセット」という位置づけは**緩められる**。`.send()` と `yield from` は
  依然サブセット外だが、**普通のジェネレータは意味論まで CPython と揃う**。
  ⇒ ついでに **FUTURE_FEATURE §4 (4-b) のジェネレータ式**の前提も満たされた
    （「`gen` を使った脱糖を設計すること」が実行可能になった）。再検討の価値あり。
- 編集: `statements.rs`
  - `FunctionDef` 変換時、本体に `yield` 文を含むなら `Stmt::FnDef` でなく `Stmt::GenDef`（`yield_type` は `Generator[T]`/`Iterator[T]` 注釈から抽出、無ければ None）を生成。
  - `yield x` 文（`Expr` 文中の `py::Expr::Yield`）→ `Stmt::Yield(convert)`。`yield from`・yield 式の値利用は**明示 Err**。
  - **入れ子 `def` の中に `yield` があれば明示 Err**（①。型検査まで行かせず変換時に止める）。
- テスト: `def g(): yield 1; yield 2` を for で回す／`if` の中の `yield`／**無限ジェネレータ +
  `break`**（②の遅延を固定する）。`_error` 例に `yield from` と**入れ子 `def` の `yield`**。

#### [10] 型エイリアス `type X = ...`
- 編集: `statements.rs`（`TypeAlias`）＋ `annotations.rs`
  - **推奨: 変換器内エイリアステーブル**（透過展開）。`type X = <型式>` を検出したら `X → convert_annotation(rhs)` を map に登録し、`convert_annotation`/`map_type_name` が `X` を展開。
  - map をどこに持つか（スレッド越しの状態 or 引数引き回し）を設計。`new_type` 出力案は名目的別型で偽陽性のため非推奨。
- テスト: `type V = list[int]` を注釈に使用。

#### [16] 連鎖比較 `a<b<c` ✅ **実装済（2026-08-28）**
- 計画どおり隣接ペアを `and` 連結。項目 13 の `is`/`is not` 特別扱いを `build_comparison` に
  切り出して連鎖の各ペアでも効くようにした。
- ⚠ **短絡は CPython と一致**（Arrow の `and` も短絡）。差は**中間オペランドの二重評価**だけで、
  これは INF-B（式→文の注入）が入れば 1 回評価に直せる。12 ケース中 11 件 CPython 一致。
- 例題: `examples/interop/py_chained_compare.ar` + `test_modules/py_chained_compare.py`。
- 編集: `expressions.rs` `Compare` アーム
  - `ops.len()>=1` を許可し、隣接ペアを `and` で連結: `a op1 b op2 c` → `(a op1 b) and (b op2 c)`。
- 注意: 中間オペランド2回評価は**許容**（ユーザー方針）。
- テスト: `1 < x < 10`。

#### [17] 内包表記（単一 for / 多重 for / フィルタ）✅ **実装済（2026-08-28・list / set）**
- ⚠ **Arrow 側の言語仕様としても同時に実装**（ユーザー指示）。ネイティブ構文
  `[elt for x in it if c]` / `{elt for ...}` をパース時に脱糖する。
- ★ **脱糖器は `ast::build_list_comprehension` の 1 箇所**。ネイティブ構文
  （`parser/exprs.rs` の `parse_comprehension_tail`）と Python 変換が同じ関数を通るので
  **AST が必ず一致**する（`parser_tests.rs::test_python_list_comprehension_matches_native_ast`）。
- set 内包は `set(<リスト内包>)` に脱糖 ⇒ **項目 22 の制限を解消**（その `_error` 例題は削除）。
- ⚠ 辞書内包・ジェネレータ式は明示エラー。方針は `FUTURE_FEATURE.md` §4 (4)。
- ⚠ impl_python は**触らない**方針のため `comprehension.ar` を knownDiff に登録（実測確認済み）。
- 例題: `examples/collections/comprehension.ar` / `comprehension_error.ar` /
  `examples/interop/py_comprehension.ar` / `py_comprehension_error.ar`。
- 編集: `expressions.rs` `ListComp` アーム
  - `[elt for x in it if c ...]` → 先頭 generator を外側 `Expr::ForExpr{ target, iter, body, return_type:Some("list[Any]") }`、2つ目以降 generator を body 内の入れ子 `Stmt::For`、各 `ifs` を `Stmt::If` ラップ、`elt` を最深部の `Stmt::LoopYield`。
- 根拠: `loop_yield` は入れ子 for 文/if 文を透過して最外 for 式へ積まれフラット化（実機確認: 2重/3重/フィルタ）。
- サブセット: set/dict/generator/async 内包は当面 Err（set は `set(...)` 包みで後追い可）。
- テスト: `[x*y for x in a for y in b]`、フィルタ付き。

### 群3: 式→文注入が要るもの（INF-B / INF-C 依存）

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

### クラス継承（`import[py]` 限定）✅ **実装済（2026-08-28）**

計画外の追加項目（項目 5 の作業中に「基底が黙って捨てられる」ことを発見して着手）。

- **方針**: Arrow 本体では class 継承は**今後も不許可**（基底はトレイトのみ）。
  **Python を読み込むときだけ**の特別処置として、**トレイト継承のロジックを流用**する。
- **編集 3 箇所**:
  - `src/interpreter/exec/definitions.rs` `exec_class_def` — `in_python_module` 限定で
    基底クラスのメンバを**定義時に平坦化**して取り込む（オーバーライド優先）。
  - `src/interpreter.rs` `build_field_index` + 新フィールド `py_class_field_order` —
    基底フィールドを先頭に置く既存ロジックを流用。トレイトを先に見て無ければこちら。
  - `src/python_converter/supers.rs`（新設）+ `expressions.rs` —
    `super().m(a)` → `<第1基底>.m(self, a)` に**変換時**脱糖。受け側は
    `classes/class_methods.rs` が `FnValue::is_python` 限定でアンバウンド呼び出しを許可。
    ⚠ `in_python_module` で判定してはいけない（ドライバから呼ばれた時点で false）。
- **境界の固定**: `examples/classes/class_inherit_error.ar`（ネイティブ `.ar` は今も ParseError）。
- **挙動不変**: `compare_outputs.ps1 -A <HEAD>` が 114/116 一致・差分は新規例題 2 本のみ
  （負の対照 116/116 取得済み）。
- 例題: `examples/interop/py_inherit.ar` / `py_inherit_error.ar` + `test_modules/py_inherit.py`。

### 群4: 大物（設計比重大／実行時ガード）

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

### 群5: 明示エラー化・警告（🟠 / 🔴 / del）

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

### 群6: 同梱 `.pyi` スタブによる静的型予測（BYTECODE_VM_PLAN #19）

[BYTECODE_VM_PLAN.md](../implementation_logs/BYTECODE_VM_PLAN.md) の別レーン
**#19「py 組み込みスタブ整備（`time`/`math`）— 同梱 `.pyi` ＋ `python_search_dirs()` に置き場を追加」**
をここに展開する。対象は `import[py-int]`（PyO3 経路）だが、**`.pyi` の読み込みは
`python_converter::convert_python_source` を通る**ため本書の管轄に入る。

#### S0. 現状（実測で確定・2026-09-17）

機構は**すでに全部ある**。欠けているのは「スタブの中身」と「置き場」だけ。

```text
import[py-int] math
let s: str = math.sqrt(2.0)

  スタブ無し  → 1.4142135623730951 を印字（★型検査がまったく走らない）
  math.pyi 有 → StaticTypeError: 's' is declared 'str' but initialized with 'float'
```

- 解決の入口は [`load_python_interface_module`](../src/parser/imports/py_modules.rs) —
  `module.pyi` → `module/__init__.pyi` → `module.py` → `module/__init__.py` の順に
  **全検索ディレクトリ**を見て、**1 つも無ければ `Ok(vec![])`**（＝型検査を丸ごと放棄）。
- `.pyi` は [`load_py_type_body`](../src/parser/imports/py_modules.rs) が
  `convert_python_source` で変換し、**変換できなかった名前だけ**
  `extract_py_type_stubs`（[`imports/mod.rs`](../src/parser/imports/mod.rs) の行ベース抽出）で補う。
- ⚠ `time` / `math` は **C 組み込みモジュールで `.py` が存在しない**。だから
  `Parser::python_search_dirs()` が CPython の stdlib ディレクトリまで辿っても**必ず空振り**する。
  ＝ 現状 `time.*` / `math.*` の戻り値型は**恒久的に検査されない**。
- 既に [`examples/interop/test_modules/time.pyi`](../examples/interop/test_modules/time.pyi) が
  4 関数ぶんだけ存在する（**そのディレクトリから実行したときしか効かない**）。

#### S1. ⚠⚠ 前提: INF-D を先に入れる（**これ無しでは #19 は純粋な退行**）

同梱スタブを置いた瞬間、**最も普通の呼び出しが落ちる**。実測:

```text
# time.pyi の `def sleep(secs: float) -> None: ...` を置いた状態で
time.sleep(0.01)
  → StaticTypeError: parameter 'secs' of 'sleep' expects a mutable argument,
    but got an immutable value
```

`convert_params` が全パラメータを `mutable: true` にする（🔵 仕様）ためで、
**今は「スタブが無いから検査が走らず、たまたま通っている」**。スタブを足すと
`time.sleep(0.01)` / `math.sqrt(2.0)` が軒並みエラーになる。
⇒ **INF-D（§1）が S2 以降の着手前提**。順序を逆にしないこと。

#### S2. 同梱スタブの置き場と解決順（★設計判断が要る箇所）

VM plan の一行は「`python_search_dirs()` に置き場を追加」だが、**素直にディレクトリを
足す案は採らないことを推奨する**。理由と代案:

| 案 | 内容 | 評価 |
|---|---|---|
| A. `include_str!` で**バイナリに埋め込む** | `src/py_stubs/` に `pub fn builtin_stub(module: &[String]) -> Option<&'static str>`、実体は `stubs/*.pyi` | **推奨** |
| B. `current_exe()` 相対の `stubs/` を検索パスへ追加 | VM plan の字面どおり | 非推奨 |

- **B を採らない理由**:
  - `src/` に `current_exe()` の使用は**現在 0 箇所**。`cargo run`（`target/debug/`）と
    配布レイアウトでスタブの相対位置が変わり、「開発中だけ効く／配布すると効かない」を作り込む。
  - `python_search_dirs()` に 1 ディレクトリ足すと、**モジュール解決 1 回あたり
    `exists()` が 4 回増える**（`load_python_interface_module` が dir × 4 候補を生成するため）。
    ⚠ ここは #69 が「`exists()` の syscall 連打が `interp_init` の 48〜53% を占めていた」と
    実測して遅延化した場所で、**同じ轍を踏む**。
- **A の利点**:
  - syscall ゼロ・パス依存ゼロ。
  - ⭐ **エディタ（wasm）にも効かせられる**。`editor` feature は `imports` を
    `imports_editor` へ差し替えて fs に触れないので、**ディレクトリ方式では原理的に届かない**。
    埋め込みなら「`import[py-int] time` の `time.time()` が VS Code 上でも `float` になる」が
    射程に入る（⚠ ただし `imports_editor` 側の対応は**別タスク**。S5 で任意とする）。
- **解決順（重要）**: 埋め込みは**ファイルシステム探索が全部空振りしたときの最後**に見る。
  ユーザーが自分で置いた `time.pyi` が**必ず勝つ**ようにする（上書き可能性を残す）。
  ＝ `load_python_interface_module` の `Ok(vec![])` フォールバック**直前**に 1 分岐足すだけ。
- 編集:
  - 新設 `stubs/time.pyi` / `stubs/math.pyi`（リポジトリ直下）。
  - 新設 `src/py_stubs.rs` — `include_str!` のテーブルと `builtin_stub()`。
    ⚠ `src/python_converter/` には置かない（wasm フロントエンドが取り込まないため。項目 7 で
    `PY_KWARGS_PARAM` が同じ理由で `src/ast.rs` へ行った）。
  - `py_modules.rs` `load_python_interface_module` — 末尾の `Ok(vec![])` の前に
    `builtin_stub()` を引き、当たれば `load_py_type_body` 相当（ソース文字列版）へ流す。
    ⚠ `load_py_type_body` は `abs_path` でキャッシュキーを作るので、埋め込み用に
    **疑似パス**（例 `<builtin>/time.pyi`）を使うか、キャッシュキーを `(lang, String)` に広げる。

#### S3. `time` / `math` のスタブを書く

- `stubs/time.pyi` … 既存の [`test_modules/time.pyi`](../examples/interop/test_modules/time.pyi) を
  出発点に（`time` / `monotonic` / `perf_counter` / `sleep`）、`time_ns` / `monotonic_ns` を追加。
- `stubs/math.pyi` … `sqrt` / `floor` / `ceil` / `fabs` / `pow` / `exp` / `log` / `log2` / `log10` /
  `sin` / `cos` / `tan` / `atan2` / `hypot` / `isnan` / `isinf` / `gcd` と定数 `pi` / `e` / `inf` / `nan`。
- **書ける型の上限**（`py_type_to_arrow` の実装で確定。超えたぶんは黙って `Any` に落ちる）:
  - 通る: `int` / `float` / `str` / `bool` / `None` / `bytes` / `list[T]` / `set[T]` /
    `Optional[T]` / `List[T]` / `Set[T]` / `X | None` / `X | Y`（角括弧を含まない場合のみ）。
  - **要素型が落ちる**: `dict[K,V]` → `dict`、`tuple[A,B]` → `tuple`。
  - `Any` に落ちる: `Callable[...]`・`@overload`・ジェネリック・`Literal[...]`。
  - ⚠ **モジュール定数（`pi: float`）は現状どちらの経路も拾わない** ——
    `extract_py_type_stubs` は `def` と `class` しか見ず、`convert_python_source` は
    値なしの `x: float` を `Ok(None)` で捨てる（[`statements.rs`](../src/python_converter/statements.rs) の `AnnAssign` アーム）。
    ⇒ **定数に型を付けたいなら別途手当てが要る**。S3 では**関数だけを対象にし、
    定数は次のカード（S4）に切り出す**。

#### S4. スタブ経路のサイレント劣化を可視化する

`load_py_type_body` は `convert_python_source(...).unwrap_or_default()` —— **変換エラーを
握り潰して**行ベース抽出へ落ちる。同梱スタブを持つ以上、ここは「自分で書いたファイルが
静かに精度を失う」経路になる。

- `.pyi` の変換に失敗したら、**同梱スタブのときだけ** `eprintln!` で警告する
  （ユーザーの `.pyi` は今までどおり黙って劣化 —— 外部ファイルを硬いエラーにはしない）。
- `cargo test` に「同梱スタブは全部 `convert_python_source` を**エラー無しで**通る」
  という回帰テストを 1 本置く（`frontend_tests/`）。⇒ スタブを足したときに
  未対応構文を入れてしまったら、その場で落ちる。
- ⚠ 併せて**クラスを含むスタブは `Any` 止まり**であることを doc に明記する（実測）:
  `class Fraction: ...` を書いても `Fraction(1,2)` は `Any` を返す
  （`extract_py_type_stubs` の「クラス名を戻り値型にしてはいけない」＝タスク 7.8 の帰結。
  **意図どおりで直さない**）。⇒ 同梱スタブは**関数中心**に設計する。

#### S5. 例題・ゲート

- `examples/interop/py_int_stub.ar` — `time` / `math` を**検索パスに何も置かずに**呼び、
  戻り値型が効いていること（`let x: float = math.sqrt(...)` が通り、`let s: str = ...` が落ちる）。
- `examples/interop/py_int_stub_error.ar` — 同梱スタブが型を予測して**弾く**形。
- ⚠ **`scan_examples.ps1` だけでは足りない**。以下を必ず走らせる:
  - `compare_import_paths.ps1 -A <base.exe>` — import 解決順を変えるため**この変更の本丸**。
  - `type_obligations.ps1` — INF-D が型義務を落としていないか。
  - `compare_wasm_frontend.ps1` — `call_check.rs` を触るため必須。
  - `generate-codebase-map.ps1` — `stubs/` と `src/py_stubs.rs` の新設ぶん。
- 任意（別タスク化してよい）: `imports_editor` から `builtin_stub()` を引いて、
  VS Code 上でも `py-int` の戻り値型を出す。⇒ 実施するなら VSIX 再生成まで（`make-vsix.ps1`）。

#### S6. スコープ外（意図的に含めない）

- `os` / `sys` / `json` など**本体が `.py` で存在する** stdlib。これらは既に
  `extract_py_type_stubs` が実ソースからシグネチャを抜けるので、#19 の対象ではない。
- typeshed の取り込み。ライセンスと量の判断が別に要る。**まず `time` / `math` の 2 本**。
- 項目 27（Python モジュール内 import の再帰ロード）との接続。
  27 が「stdlib は翻訳不能 → `py-int` フォールバック」を選ぶなら本フェーズが受け皿になるが、
  **27 の設計が固まるまで結合しない**。

### 群7: アンパック（タプル / 辞書）

coverage の 🟠「コレクション型のアンパック」を実装カードに展開する。
**8 形すべて現状は明示エラー**（サイレント欠落は無い・実測で確認）。

| Python | 現在のエラー文言 | カード |
|---|---|---|
| `f(*xs)` | `starred expression is not supported in this context` | U4 |
| `f(**d)` | `**kwargs unpacking in call is not supported` | U5 |
| `a, b = t` / `a, *rest = t` | `tuple/list unpacking in assignment is not supported` | U2 |
| `for k, v in pairs:` | `tuple unpacking in for-loop target is not supported` | U1 |
| `[*a, 9]` / `{*a, 9}` | `starred expression …` | U3 |
| `{**d, "z": 1}` | `**dict unpacking in dict literal is not supported` | U3 |

#### ⚠⚠ coverage の根拠を 1 つ訂正する（2026-09-18・実測）

coverage 🟠 は「Arrow には `LetTuple`・多ターゲット `For`・呼び出しの `f(...=a,b,c)`
（`CallArg::Variadic`）が実在するため将来対応可能」と書いているが、**3 つ目は
`f(*xs)` の受け皿にならない**。

```text
fn count(let ...: Any) -> int: return len(local::args)
mut xs = [1, 2, 3]
count(... = xs)        → 1     ★リスト 1 個として渡る（展開されない）
count(... = 1, 2, 3)   → 3
# 型を付けると静的に弾かれる:
#   the variadic argument of 'count' expects 'int' but got 'list[int]'
# 固定長 fn に渡すと: 'add' takes 2 argument(s) but 0 were given
```

`... =` は**引数を並べる構文**であって展開演算子ではない。**Arrow に splat / spread は
字句・構文とも 1 つも無い**（lexer / parser を grep して 0 件）。
⇒ **U1 / U2 / U3 は Arrow 本体の追加なしで載るが、U4 / U5 は言語追加が要る**。
文書の並びから受ける印象と難易度が逆なので注意。

#### [U1] `for` ターゲットのアンパック `for k, v in pairs:`
- Arrow 側は**既に対応済み**（`Stmt::For.targets` が `Vec<String>`）。実機で確認:
  `for k, v in pairs:` / `for i, v in enumerate(xs):` / `for p, q in zip(xs, ys):` すべて動く。
- 編集: `statements.rs` `For` アーム — `py::Expr::Tuple(t)` のとき各要素が `Name` なら
  `targets` に並べる。`Name` 以外（入れ子タプル・添字）は明示エラーのまま。
- ⚠⚠ **`d.items()` が存在しない**。Arrow の dict のメソッドは `keys()`/`key()` と
  `values()`/`item()` だけ（[`method_call.rs`](../src/interpreter/classes/method_call.rs) の `Value::Dict` アーム）。
  `for k, v in d.items():` は Python で最頻出の形なので、**`items()` の追加をセットで行う**
  （`all_keys()` / `all_items()` の隣に `all_pairs()` を足して `List<Tuple>` を返す）。
  ⇒ これは interpreter 変更。impl_python 並行実装の要否を確認すること。
- テスト: `pairs` / `enumerate` / `zip` / `d.items()` の 4 形。

#### [U2] 代入のアンパック `a, b = t` / `a, *rest = t`
- 前提: **INF-C**（複数文返却）。
- ⚠ **`LetTuple` は使えない**。Arrow の `let a, let b = t` は**宣言**で、しかも
  **宣言済みの名前への `a, b = t` は構文として存在しない**（実測: `ParseError: unexpected token: ','`）。
  項目 2 のスコープ巻き上げが全代入名を先に宣言して以降を `Stmt::Assign` にする以上、
  変換器から `LetTuple` は出せない。
- 方針: **添字への脱糖**（Arrow 本体の追加が不要）。
  `a, b = t` → `__unpack_N = t; a = __unpack_N[0]; b = __unpack_N[1]`
  `a, *rest = t` → 末尾は `rest = __unpack_N[1:]`（項目 4 のスライスが既に効く）。
  - 一時変数名は衝突しない連番（`__unpack_N`）。巻き上げ対象に含めること。
  - RHS を**1 回だけ**評価するために一時変数は必須（`a = t[0]; b = t[1]` は 2 回評価になる）。
- 明示エラーのまま残す: 入れ子タプル（`a, (b, c) = t`）・`*rest` が末尾以外。
- テスト: 2 要素／3 要素／`*rest`／RHS が関数呼び出し（1 回評価の確認）／要素数不一致。

#### [U3] リテラルのアンパック `[*a, 9]` / `{*a, 9}` / `{**d, "z": 1}`
- Arrow 本体の追加は不要。B5 で `list` / `tuple` の `+` が入ったので連結で書ける。
- 方針:
  - `[*a, 9]` → `a + [9]`（`Expr::BinOp{Add}`）。複数の `*` も左結合で連結。
  - `{*a, 9}` → `set(<list 版>)`（群6 の set 内包と同じ手）。
  - `{**d, "z": 1}` → **要検討**。Arrow に dict のマージ手段が無い可能性が高い。
    無ければ `dict()` コンストラクタ（FUTURE_FEATURE §4 (4-a) で辞書内包の前提として
    既に起票済み）と合わせて片付ける。⇒ **U3 の dict 部分だけ後回しにしてよい**。
- テスト: 先頭／中間／末尾の `*`、複数の `*`、空リストの展開。

#### [U4] 呼び出しのアンパック `f(*xs)` ★Arrow 本体への言語追加
- ⚠ **本カードは Arrow の言語仕様追加**。変換器だけでは閉じない。
  方針決定は [FUTURE_FEATURE.md](../implementation_logs/FUTURE_FEATURE.md) 側に置き、
  本書はそれを消費する側として扱う。
- **配管は既にある**（[`args.rs`](../src/interpreter/functions/args.rs) の `CallArg::Variadic` 経路）:
  各式を評価 → `Value::List` に束ねる → 特殊キー `"..."` で渡す → `bind_args` が
  `variadic_value` として拾い `local::args` に直結。
  ⇒ **「リスト 1 本を `local::args` に渡す」経路は通っている**。
- 実装の段取り（見積り順）:
  1. `CallArg::Spread(Expr)` を追加。⚠ **消費者 11 箇所**が全部コンパイルエラーになる
     （`expr_walk` / `resolver` / `templates` / `ast_value` / `type_check` / `vm/compiler/calls.rs` /
     `vm/compiler/diag.rs` / `partial_compiler/llvm_codegen` / `eval/builtins.rs` / `parser/exprs.rs`）。
     網羅 `match` による強制が効くので**むしろ安全**（`language-dev-principles` 参照）。
  2. **可変長関数への `f(*xs)`** … `BuildList(n)` を省いて `xs` をそのまま `"..."` で渡すだけ。
     VM 側（[`calls.rs`](../src/vm/compiler/calls.rs)）は**コードが短くなる**。
  3. **固定長関数への `f(*xs)`** … 束縛時に位置引数列へ展開する。ここが本体。
  4. 静的型検査: spread を含む呼び出しは**引数個数検査を降りる**。項目 6/7 で
     `params: None` / `variadic_type` を入れた前例に乗る。
- 変換器側: `expressions.rs` `Call` アームで `py::Expr::Starred` を `CallArg::Spread` に。
- ゲート: `compare_bytecode.ps1`（VM コンパイラを触る）＋ `force_gate.ps1` ＋ `syntax_cov.ps1`。

#### [U5] 呼び出しのアンパック `f(**d)` ★Arrow 本体への言語追加
- 前提: U4 と同じ設計（`CallArg` の拡張）。U4 と**同時に決める**こと。
- `bind_args_relaxed` は既に余剰キーワードを集めて `extra_kwargs` にしているので、
  dict を `(name, value)` のキーワード列へ展開するのは U4 と対称に書ける。
- ⚠ キーが `str` でない dict は実行時 `TypeError`。

#### [U6] 転送パターンの成立（U4 + U5 + INF-D + 項目20†1 が揃って初めて動く）
```python
def wrapper(*args, **kwargs):
    return inner(*args, **kwargs)     # デコレータの定番形
```
- これが通ることを**1 本の例題で固定する**。現状は 2 重にブロックされている:
  ① `f(*args)` が変換器で明示エラー（U4 / U5）
  ② 関数値の `mut` パラメータが入れ子 `fn` から見えず `NameError: 'f' is not defined`
     （項目 20 †1。B3 で int は直ったが関数値は残っている・実測）
- 例題: `examples/interop/py_forward.ar` + `test_modules/py_forward.py`。

### 仕様確定（コード変更なし or 任意）
- 整数 i64 切り詰め（`convert_constant`）: 仕様。変更不要（将来「範囲外は Err」への変更余地のみ注記）。
- 全パラメータ `mutable:true`（`convert_params`）: 仕様。
  ⚠ **「任意対応」から格上げした** —— 呼び出し側の検査の食い違い（**INF-D**）は
  フェーズ6 の前提であり、項目 26（lambda）・デコレータの実用性にも効く。§1 INF-D を見ること。

---

## 3. 実行フェーズ（依存関係による割り当て）

> **§2 の「群」は話題別のカタログ、本節の「フェーズ」は実行順**。別の軸なので混同しないこと。
> 各フェーズは「**前のフェーズが終わっていないと着手できないもの**」だけを後ろに置く。
> 同じフェーズ内のタスクは**互いに独立で、順不同・並行可**。

### 依存グラフ（これだけ守れば順序は自由）

```text
                 ┌─► B  INF-D ──────────────► E  同梱スタブ（群6）
                 │                    │
A 地ならし ──────┼─► C  独立実装（群5 + 14 / U1 / U3 / 8 / 9 / 10）
（全部の前提）   │                    │
                 ├─► D1 INF-B / INF-C ┼─► D' 15・23・26・U2
                 │                    │
                 ├─► F  言語追加（U4・U5）──┐
                 │                          ├─► G  U6 転送パターンが成立
                 ├─► A7 関数値 mut の捕捉 ──┤     （B・F・A7 の 3 つ揃いで初めて動く）
                 │                          │
                 │         B ───────────────┘
                 │
                 └─► H  大物（25・27）… E と結合するかは 27 の設計で決める

実線 = 硬い依存（先に終わっていないと着手できない）
B から C / D' / F へ伸びる破線的な関係は「無くても着手できるが、
無いと `m.f(5)` が落ちるので実用性が出ない」＝ 柔らかい依存。
```

---

### フェーズ A — 地ならし（依存なし・**すべての前提**）✅ **完了（2026-09-18）**

計画文書が実態とずれたままだと、後続がその嘘を前提に設計してしまう。**最優先**。

> **結果**: A1〜A7 すべて完了（コミット 3 本）。
> - **A6 は実バグだった** — `def f(a, *rest, b)` が呼び方によって通ったり壊れたりしていた。
>   `Param` にキーワード専用の概念が無く表現できないため**明示エラー**に倒した。
>   起票時の見立て「項目 1・6 が未実装だから」は**外れ**（両方実装済み。原因は平坦化の順序）。
> - **A1 で 4 件が既に解決済みと判明**（項目18 のタプル 3 件 = B1/B5/B2、項目6 の VM = B7、
>   項目20 †2 = B6）。実機で 1 件ずつ確認した。
> - **A7 は範囲が縮んでいた** — `mut` パラメータの捕捉は B3 で int が直り、
>   残るのは**関数値**のみ。症状も `None` → `NameError` に変わっていた。
> - **A5 は想定より広かった** — 破損リンクは 199 件で、`bug_fix.md` と
>   `comptime_metafn_design.md` も同じ移動の巻き添えになっていた。

| # | タスク | 対象 |
|---|---|---|
| A1 | **文書追随**: B1〜B13 で解決済みの「未修正」記述を訂正（項目18 のタプル 3 件／項目6 の入れ子 `fn` × `*args`／項目20 †2 モジュール自己呼び出し） | coverage |
| A2 | 項目22 の例題欄から実在しない `py_set_error.ar` / `py_setcomp_error.py` を削除（項目17 で解消済み） | coverage / 本書 群1 |
| A3 | **§0.3 の死んだアンカー**を差し替え（`declare_var("kwargs"...)` は消滅。実体は `bind_args_relaxed`） | 本書 §0.3 |
| A4 | **項目9 のカードを B13 前提へ更新**: `YieldOutsideGenerator`（`yield` は `gen` 本体の自フレーム直下のみ）／ジェネレータが**真に遅延**になった旨 | 本書 群2 |
| A5 | 文書移動の後始末: coverage の相対リンク総崩れ、`BUGFIX_B1_B13.md` / `FUTURE_FEATURE.md` からの逆参照、`generate-codebase-map.ps1` 再実行 | 全体 |
| A6 | **実バグ**: `def f(a, *rest, b)` が `TypeError: argument 'b' given twice`。`convert_params` が kwonly を**可変長より先に**平坦化するため。項目24 の「項目1・6 が未実装だから」という記述と [`classes.rs`](../src/python_converter/classes.rs) のコメントも同時に訂正 | 変換器 |
| A7 | **項目20 †1 の再トリアージ**: B3 で `mut n: int` は直ったが、**関数値の `mut` パラメータ**が入れ子 `fn` から見えず `NameError`。症状（`None` → `NameError`）も記述と違う。⇒ 起票し直す（修正は F/G で効いてくる） | coverage / bug_fix |

**Done**: `scan_examples.ps1` 緑・`stale_doc_refs.ps1` 緑・codebase-map 再生成済み。

---

### フェーズ B — 型検査の食い違い解消（A のみに依存）✅ **完了（2026-09-18）**

> **結果**: `call_check.rs` の 2 箇所を `path_is_mutable(e) == Some(false)` へ寄せ、
> 消費者の無くなった `is_mutable_expr` を削除した。
> ⚠ **弾かれていたのはリテラルだけではなかった** —— 起票時は「リテラルが通らない」と
> 書いていたが、実測では **`xs[0]`（`mut` コレクションの要素）と呼び出しの戻り値**も
> 落ちていた（4 形中 3 形）。`is_mutable_expr` が `Expr::Ident` 以外を一律 `false` に
> していたため。`path_is_mutable` はパスの根を辿るので 3 形とも一度に直った。
> ⚠ **B11 の防御は無傷** —— `let` 束縛と `let` の要素（規則 1）は今も弾かれる。
> `bridge_mutability.rs`（C ABI の書き込み用ポインタ）も書き換え不要だった
> ＝ リテラル拒否を仕様として固定したテストは**存在しなかった**。
> ⭐ 副産物: 動機だった `time.sleep(0.01)`（フェーズ E）が通るようになった。


| # | タスク |
|---|---|
| B1 | **INF-D**（§1）— `FnTypeParam` 経路の `mut` 実引数検査をネイティブ側（`path_is_mutable`）に揃える |

- **単独で完結**するが、**E の絶対前提**（先に E をやると `time.sleep(0.01)` が落ちる状態を自作する）。
- C / D / F の**実用性**にも効く（`m.f(5)` のようなリテラル渡しが通るようになる）。
- ゲート: `type_obligations.ps1` ＋ `compare_wasm_frontend.ps1` ＋ `scan_examples.ps1`。

---

### フェーズ C — 独立実装（A のみに依存・**互いに並行可**）✅ **完了（2026-09-19）**

> **進捗**: ✅ **フェーズ C 完了**（C1 = 2026-09-18、C2〜C6 = 2026-09-19）。
> ⚠ C6 で **`dict()` コンストラクタが 2 箇所の前提**として立った —— `{**d1, **d2}`（U3 の dict 部分）と
> 辞書内包（FUTURE_FEATURE §4 (4-a)）。`items()` は C5 で入ったので、`dict(d1.items() + d2.items())`
> の形で両方いっぺんに片付く。**別カードとして起票する価値がある**。
> ⚠ C1 で潰したのは**すべてサイレント欠落**だった —— `for/while/try ... else` は節を
> 黙って破棄し、`except (A, B)` は**型を `"Exception"` に潰して何でも捕まえるハンドラ**に
> 化けており、`global` / `nonlocal` は**黙って無視**されて内側の代入が外側に届かないまま
> 動いていた。エラーも出ないので気付けない形だった。

他の何にも依存せず、他の何もブロックしない。手が空いたらここから取る。

| # | タスク | 備考 |
|---|---|---|
| C1 ✅ | **群5 の明示エラー化 8 件**（`for/while/try else`・`except (A,B)`・`raise from`・`assert`・`global/nonlocal`）＋ **項目14 `del`**（Name は警告付き無視／Subscript・Attribute は明示エラー） | ✅ **完了（2026-09-18）**。`del <name>` のみ警告して無視、他 7 形は明示エラー |
| C2 ✅ | 項目8 `match`（値 / `_` サブセット） | ✅ **完了（2026-09-19）**。⚠ 意味論は Python と一致（フォールスルーしない・不一致なら何もしない）。キャプチャ / OR / シーケンス / マッピング / クラス / ガードは明示エラー |
| C3 ✅ | 項目9 ジェネレータ | ✅ **完了（2026-09-19）**。⚠ A4 の「入れ子 `def` の `yield` は明示エラーが要る」は**外れ**だった — 判定を自フレームだけにすれば入れ子 `gen` として自然に通る。⚠ **`gen` メソッドが型検査のクラスメンバ収集から漏れていた**（純 Arrow でも再現）ので同時に修正 |
| C4 ✅ | 項目10 型エイリアス | ✅ **完了（2026-09-19）**。**変換器内エイリアス表**で透過展開（`alias` はパース時構文で AST に出せず、`new_type` は名目的別型で不適）。表はモジュール単位でリセット。型パラメータ付き `type L[T]` は明示エラー |
| C5 ✅ | **U1** `for k, v in …` ＋ **`dict.items()` の追加** | ✅ **完了（2026-09-19）**。⚠ `items()` は **impl_python には最初から在った**（同形のタプルのリスト）— Rust 側だけが欠けていた。⚠ 内包表記の中（`[k for k, v in d.items()]`）は**まだ通らない**（`ComprehensionClause.target` が単一 `String`・別項目）|
| C6 ✅ | **U3** リテラルのアンパック `[*a]` / `{*a}` | ✅ **完了（2026-09-19）**。list は連結、set は `set([...]).union(...)` へ脱糖。⚠ **list 版だけ展開元がリストに限られる**（`list()` 組込みが無く `list(a)+[...]` と書けない）— 他は `TypeError` で loud に落ちるので黙って壊れはしない。set 版は種類を選ばない。`{**d}` は `dict()` 待ちで未対応のまま |

---

### フェーズ D — 式→文注入の基盤と、その消費者

> **進捗**: ✅ **D1〜D5 完了**（2026-09-19）。残りは D6（項目16 の二重評価解消・任意）のみ。
> ⚠ D5 で **Arrow 側の制限を 1 件発見**: `for` の**ループ変数を捕捉する**入れ子 `fn` が
> VM に載らない（`VmForceError`・純 Arrow で再現）。`while` や他の捕捉は通る。
> FUTURE_FEATURE §5 (z) に起票候補として記録した。
> ⚠ D5 で `py_defaults_error.ar` が**陳腐化**した（lambda と f-string が通るように
> なり `_error` として成立しなくなった）。書式指定つき f-string と bytes へ入れ替えた。
> ⚠⚠ **coverage 項目 15 の「一時変数化すれば `a = b = []` の共有も直せる」は now 失効**。
> Arrow は **代入で複製する**（B8/B9/L4「格納も複製する」）ので、一時変数を挟んでも
> a と b は別オブジェクトになる。純 Arrow で再現する**モデル差**で、変換器では埋められない。

| 段 | # | タスク |
|---|---|---|
| D-前 | D1 | **INF-B**（式から囲みスコープへ文を注入）＋ **INF-C**（複数文返却）。どちらか一方で足りる項目もあるが、**まとめて入れる** |
| D-後 | D2 ✅ | 項目15 複数代入 `a = b = c`（2026-09-19 完了） |
| | D3 ✅ | **U2** 代入のアンパック `a, b = t` / `a, *rest = t`（2026-09-19 完了・添字＋スライスへ脱糖） |
| | D4 ✅ | 項目23 walrus `:=`（2026-09-19 完了）⚠ **持ち上げると評価回数が変わる位置**（`while` の条件・`and`/`or` の右辺・三項の腕・内包の中）は明示エラー。入れ子でも効くよう深さで数える |
| | D5 ✅ | 項目26 lambda lifting（2026-09-19 完了）⚠ 戻り型は **注釈なし**が正解だった（計画の「推論 or `Any` 補完」は誤り。`-> Any` は伝染して算術で落ちる）。入れ子 lambda は**外側の本体へ**持ち上げる |
| | D6 | （任意）項目16 の中間オペランド二重評価を 1 回評価へ |

**D-後 の 5 件は互いに独立**。D1 が入ってから並行で進めてよい。

---

### フェーズ E — 同梱 `.pyi` スタブ（**B1 が前提**）✅ **完了（2026-09-19）**

> **結果**: E1〜E4 完了。`include_str!` の埋め込み表（[`src/py_stubs.rs`](src/py_stubs.rs)）＋
> [`stubs/time.pyi`](../stubs/time.pyi) / [`stubs/math.pyi`](../stubs/math.pyi)。
> 検索ディレクトリを全部見て空振りしたときだけ引くので、**利用者のファイルが常に勝つ**（実測確認）。
> ⚠ **未宣言メンバは壊れない**（`math.pi` が従来どおり動く）—— 着手前に一番心配した点。
> ⚠ A/B（`compare_outputs -A <前コミットの exe>`）は **280 例題中の差分が新しい `_error`
> 例題 1 本だけ**。ベースラインは黙って通し、新版が捕まえる＝狙いどおり。
> ⚠ `compare_import_paths -A` は 13/13 一致（import 解決は不変）。
> ⚠ 積み残し: **モジュール定数**（`math.pi` の型）は拾えないまま（S3 の注記どおり）。


| # | タスク |
|---|---|
| E1 | 群6 S2 — 置き場（`include_str!` 埋め込み・検索パス追加は不採用）と解決順 |
| E2 | 群6 S3 — `time` / `math` のスタブを書く（**関数だけ**。定数は別カード） |
| E3 | 群6 S4 — スタブ経路のサイレント劣化を可視化（警告＋回帰テスト） |
| E4 | 群6 S5 — 例題・ゲート（`compare_import_paths.ps1 -A` が本丸） |

⚠ **E1 → E2 の順序は崩せない**（群6 S1）。

---

### フェーズ F — Arrow 本体への言語追加（呼び出し側アンパック）

**本書で唯一「Arrow の言語仕様そのものを増やす」フェーズ**。方針決定は
[FUTURE_FEATURE.md](../implementation_logs/FUTURE_FEATURE.md) に置き、本書は消費側として扱う。

| # | タスク |
|---|---|
| F1 | `CallArg::Spread` の設計と追加（**消費者 11 箇所**が網羅 `match` で止まる） |
| F2 | **U4** `f(*xs)` — ①可変長関数向け（`BuildList` を省くだけ）→ ②固定長関数向け（束縛時展開）→ ③型検査の個数検査を降りる |
| F3 | **U5** `f(**d)` — dict をキーワード列へ展開。F2 と**同時に設計する** |

- A / B にしか依存しないので、**D と並行してよい**。
- ゲート: `compare_bytecode.ps1` ＋ `force_gate.ps1` ＋ `syntax_cov.ps1`。

---

### フェーズ G — 転送パターンの成立（A7 + B1 + F）

| # | タスク |
|---|---|
| G1 | **U6** `def wrapper(*args, **kwargs): return inner(*args, **kwargs)` が動くことを例題で固定 |

Python で最頻出のデコレータ形。**A7（関数値の `mut` 捕捉）・B1（INF-D）・F（spread）の
3 つが揃って初めて成立する**ので、独立フェーズに切り出してある。ここが通れば
「実在する Python のデコレータが読める」と言ってよい。

---

### フェーズ H — 大物（A のみに依存・設計比重が大きい）

| # | タスク | 備考 |
|---|---|---|
| H1 | 項目25 `with`（`__exit__` 無しのみ block 脱糖） | 実行時ガードの設計 |
| H2 | 項目27 Python モジュール内 import（再帰ロード） | ⚠ **E と結合するか**は 27 の設計が固まってから決める（群6 S6） |

いつ着手してもよいが、**着手したら他と並行しない**（設計判断が多く、中断コストが高い）。

---

### まとめ（どれから取るか迷ったら）

1. **A**（地ならし）→ 短い。文書が嘘をついている状態を先に消す。
2. **B1**（INF-D）→ 1 箇所。E の前提で、C/D/F の使い勝手も上がる。
3. あとは **C（並行可・すぐ価値が出る）** と **D1（基盤）** を並べて進める。
4. ユーザーの関心が呼び出し側アンパックにあるなら **F を D と並行**で始める。

## 4. 各項目の Done 条件
- [ ] `cargo build` / `cargo clippy` 通過。
- [ ] 成功例 `.ar`（＋必要なら `.py`）が期待出力。エラー化項目は `_error.ar` が期待エラー。
- [ ] `examples/` に配置し `./generate-codebase-map.ps1` 実行。
- [ ] interpreter を触った項目（7・25・仕様緩和）は impl_python 並行実装の要否を確認、必要なら git SHA 更新。
- [ ] 挙動変更が大きい項目は sub-branch 提案の要否を確認。
- [ ] **型検査（`src/type_check/`）を触った項目（INF-D）は `type_obligations.ps1` と
      `compare_wasm_frontend.ps1` を走らせる**。import 解決を触った項目（フェーズ6・27）は
      `compare_import_paths.ps1 -A <base.exe>`。

---

## 5. 作業中に見つけたこと（発見ログ）

> 実装を進める中で**計画の前提が外れていた**点と、**この作業の外**で見つかった
> Arrow 本体の不具合・制限をここに積む。新しいものを下に足す。
> ⚠ 「起票候補」は別文書（[FUTURE_FEATURE.md](../implementation_logs/FUTURE_FEATURE.md) §5 /
> [bug_fix.md](../implementation_logs/bug_fix.md)）へ、ここには**この計画に効く要約**だけを書く。

### 5.1 計画の前提が外れていたもの

| # | 計画の記述 | 実際 | 出所 |
|---|---|---|---|
| 1 | 項目24「`def f(a, *rest, b)` は項目 1・6 が未実装だから壊れる」 | **両方実装済み**。原因は `convert_params` が kwonly を可変長より**先に**平坦化していたこと。`Param` にキーワード専用の概念が無く**表現不能**なので明示エラーへ | A6 |
| 2 | 項目20 †1「`mut` パラメータが入れ子 `fn` にキャプチャされない（**None になる**）」 | int などの値は **B3 で解決済み**。残るのは**関数値**のみで、症状も `NameError` | A7 |
| 3 | INF-D「リテラルが弾かれる」 | **4 形中 3 形**が落ちていた（リテラル・`mut` の要素・呼び出しの戻り値）。`is_mutable_expr` が `Expr::Ident` 以外を一律 false にしていたため | B |
| 4 | 項目9「入れ子 `def` の `yield` は明示エラーが要る」 | **不要**。判定を自フレームだけにすれば入れ子 `gen` として自然に通る（Arrow も対応済み） | C3 |
| 5 | 項目15「`a = b = []` の共有は**一時変数化すれば回避できる**」 | **直らない**。Arrow は**代入で複製する**（B8/B9/L4・この計画より後に決まった規則）。純 Arrow で再現するモデル差 | D2 |
| 6 | 項目26「`fn` は戻り型注釈が要る → 推論 or `Any` 補完」 | **注釈なしが正解**。`-> Any` は**伝染**して `cannot apply '+' to 'Any'` になる。py 由来の関数はどれも注釈が無い | D5 |
| 7 | 🟠「`CallArg::Variadic` があるので `f(*xs)` は将来対応可能」 | **受け皿にならない**。`... = xs` はリスト 1 個として渡り**展開されない**。Arrow に splat / spread は字句・構文とも 0 件 | 群7 調査 |

### 5.2 この作業の外で見つかった Arrow 本体の不具合・制限

| # | 内容 | 状態 | 出所 |
|---|---|---|---|
| 1 | **`gen` メソッドが型検査のクラスメンバ収集から漏れる**（`'Bag' has no member 'each'`）。純 Arrow で再現 | ✅ C3 で修正（`registry/builder.rs` に `Stmt::GenDef` を追加） | C3 |
| 2 | **`dict.items()` が Rust 側に無い**（`impl_python` には最初からあった＝ Rust 側のドリフト） | ✅ C5 で追加 | C5 |
| 3 | **`for` のループ変数を捕捉する入れ子 `fn` が VM に載らない**（`VmForceError`）。純 Arrow で再現。`while` 本体・ループ変数以外の捕捉・ループ外は通る | ⏳ 未修正。**FUTURE_FEATURE §5 (z)** に起票候補として記録 | D5 |
| 4 | **`compare_wasm_frontend.ps1` は wasm を再ビルドしない**（存在確認のみ）＝ **古い成果物で緑になる**。型検査を触ったら `cd crates/arrow-frontend && cargo build --release --target wasm32-unknown-unknown` を明示的に回すこと | ⏳ 未修正。`vm-pitfalls` §3 に該当 | C3 |
| 5 | **`list` / `dict` を Arrow から呼べないのは仕様**。⚠⚠ 私が一度「配線漏れ」と誤認して露出させ、`4299c88` を **revert した**。Arrow 側は**要素型の指定を必須**にしており（`mut xs: list[int] = []`）、素の `list()` / `dict()` で作れるとその規則を迂回する。`eval_type_call` に実装が残っているのは **Python 翻訳専用**に使うため | 🔒 **仕様。Arrow 側へ露出させないこと** | C6 → 誤認 → 撤回 |
| 6 | `{**d1, **d2}`（U3 の dict 部分）と**辞書内包**は dict の構築手段を要するが、#5 のとおり **Arrow 側へ出してはいけない**。⇒ **Python 翻訳からのみ届く呼び出し経路の設計**が要る（内部専用名にするか、py 由来のフレームでだけ許すか） | ⏳ **要設計** | C6 |
| 7 | **内包表記が多ターゲットを取れない**（`[k for k, v in d.items()]`）。`ComprehensionClause.target` が単一 `String` で、ネイティブ構文側の変更も要る | ⏳ 未着手・**起票候補** | C5 |
| 8 | **`for k in d:`（dict の直接反復）が `'dict[..]' is not iterable`**。Python では既定でキーを回す | ⏳ 未着手・**起票候補** | C5 |
| 9 | ~~`open()` が Python と非互換なので項目25 は通らない~~ → **私の誤り（訂正済み）**。Arrow の `open` は `file_path` / `open_mode` の**2 引数で通り**、`start_point` / `byte_recognizing` / `encoding` が任意（3 引数でも通る）。`block` 退出で閉じることも実測済み。⇒ **項目25 の障害ではない**。残る論点は「`.py` の `open(p, "w")` をどう解決するか」＝ **Python の `open` を呼ぶ（`py-int` 経路）か、変換器がモードを写すか**という別問題 | ✅ 誤認を訂正。H1 は着手可 | H1 調査 |

### 5.3 やらないと決めたこと

| # | 項目 | 理由 |
|---|---|---|
| 1 | **D6**（項目16 の中間オペランド二重評価の解消） | **短絡があるので完全には直せない**。`a<b<c<d` の `c` は「2 回現れる」が「`a<b` が真のときだけ評価される」ので、持ち上げると**条件付き評価が無条件になる**。直せるのは 3 オペランド（`a<b<c`）の `a`・`b` だけで、4 つ以上との**不整合が残る**。現状の二重評価は既にユーザー方針で許容済みなので据え置く |
| 2 | **フェーズ F**（`f(*xs)` / `f(**d)`） | 別スレッドが [comptime_metafn_design.md](comptime_metafn_design.md) の **D34「引数展開構文を入れる」**として設計中。二重実装を避ける |
