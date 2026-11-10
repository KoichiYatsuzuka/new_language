# 見えない名前が「定義済み」として通る（型検査・VS Code 拡張）

スコープの外にある名前を裸で読んでも、型検査が `name '..' is not defined` を出さない。
VS Code 拡張では、その名前が別の場所の同名の宣言（クラスのフィールドなど）に結び付けられて、
ホバー・定義へ移動に誤った情報が出る。

- 起票: 2026-11-09（利用者の報告: `TeX_editor/initializer.ar` の `wp.open_dummy_pane(.., font, narrow)` の
  `font` が未宣言なのに赤線が出ず、エディタ上は `EditorConfig` のメンバとして表示される）
- 調査: Arrow `b6e1e2af`（release を作り直して測った）・参照実装 `impl_python`
- 方針: **フェーズ 1（型検査）を直してから、フェーズ 2（拡張）を直す**（利用者の指示）。
  拡張の診断は同じ型検査器（wasm）から来るので、フェーズ 1 で赤線は出るようになる。フェーズ 2 は
  拡張だけが持っている「名前 → 宣言」の引き方（ホバー・定義へ移動・色付け）を直す。
- 採番: フェーズ-タスク（`1-1`, `1-2`, ..）。§3 の要判断は 2026-11-09 に決定（D-1 = (a)・D-2 = (b)）。
  フェーズ 3（入れ子の関数の再帰）は 2026-11-09 に追加。進める順はフェーズ 1 → 2 → 3（利用者の指示）。

## 0. 何が起きるか

再現ファイル（作業用フォルダで作ったものを、そのまま例題にできる形で載せる）。

```
class EditorConfig:
    let font: str
    pass

fn loader()->None:
    let font = "a"
    print(font)

fn use_it()->None:
    print(font)      # 実行時 NameError。型検査は通る。拡張のホバーは「member of `EditorConfig`」
```

**実測**（「検査」は `AR_CHECK_ONLY=1`、「実行」は release の exe、「参照」は `python -m impl_python`）

| # | 形 | 検査 | 実行 | 参照 | 本来 |
|---|---|---|---|---|---|
| A1 | 別の関数のローカル（`let font`）を読む | 通る | `NameError` | — | 静的な誤り |
| A2 | クラスのフィールド名だけがある名前を関数から読む | 通る | `NameError` | — | 静的な誤り |
| A3 | メソッドの中でフィールド名を裸で読む（`let n: int = font`） | **`str` 型の変数として見える**（`'n' is declared 'int' but initialized with 'str'`） | — | `NameError` | 静的な誤り（未定義） |
| A4 | 同上・`const K` / `static mut n` / `let v` のクラスメンバー | 通る | `NameError` | `NameError` | 静的な誤り |
| A5 | クラスの `const B = A + 1`（同じクラスの `const A` を裸で） | 通る | `NameError` | `NameError` | 静的な誤り |
| A6 | メソッド名を裸で読む（`print(helper)`） | 通る | `NameError` | `NameError` | 静的な誤り |
| A7 | 別の関数の入れ子の関数を呼ぶ（`nested()`） | 通る | — | — | 静的な誤り |
| A8 | 最上位で、後で宣言する名前を先に読む（`print(H2)` → `let H2`） | 通る | `NameError` | `NameError` | 静的な誤り |
| A9 | `for` の変数をループの後で読む（最上位） | 通る | `NameError` | — | 静的な誤り |
| A10 | `except .. as e` の `e` を `try` の後で読む | 通る | `NameError` | — | 静的な誤り |
| A11 | 内包表記の変数を外で読む | 通る | `NameError` | — | 静的な誤り |
| A12 | 関数の中で、`if` / `for` / `while` の本体の `let` をブロックの後で読む | 通る | **`1` / `None`**（§5 R-1） | `NameError` | 静的な誤り（仕様は**ブロックスコープ**・`docs/grammar/02_variables.md`） |
| A13 | 最上位の `if` の本体の `let` を関数から読む | 通る | `NameError` | — | 静的な誤り |
| B1 | 関数から、**後で宣言する最上位の名前**を読む（`fn f(): print(G)` → `let G = 1` → `f()`） | 通る | `1` | — | **正しい**（前方参照。直した後も通すこと） |
| B2 | 入れ子の関数から、外側の関数で後に宣言するローカルを読む | 通る | **`None`**（§5 R-1） | `1` | 静的な誤り（D-1 = (a)） |
| B3 | 入れ子の関数どうしの前方参照（`a` が後に定義する `b` を呼ぶ） | 通る | **`TypeError: 'NoneType' object is not callable`** | `3` | 静的な誤り（D-1 = (a)） |

`let x: int = font`（A1 の形）は型の食い違いも出ない。引けなかった名前の型が `Unresolved`
（何とでも合う）になるため。未定義の誤りを出せば、こちらも同時に止まる。

**拡張**（`vscode-extension` のヘッドレスランナーで、宣言ではなく**使っている位置**のホバーを取った）

| 形 | ホバー | 定義へ移動 |
|---|---|---|
| A1 / A2（`use_it` の `font`） | `public let font: str` / `member of EditorConfig` | フィールドの行 |
| A3（メソッドの中の裸の `font`） | `public let font: str` / `member of C` | フィールドの行 |
| A12（ブロックの外へ漏れた `y`） | `let y: int` | ブロックの中の宣言 |

診断は exe と同じ（未定義の誤りを出さない）。

## 1. 原因

### 1-A. 型検査：「プログラムのどこかで束縛される名前」なら全部通す

[infer.rs `check_name_defined`](../src/type_check/infer.rs) は、スコープで引けなかった名前を
`declared_anywhere`（[names.rs `collect_declared_anywhere`](../src/type_check/names.rs)）に照らす。
これは**プログラム全体**の、関数の引数・ローカル・`for` の変数・`except` の別名・クラスの本体まで含む
すべての束縛を集めた集合で、どのスコープから読んでも当たる（フェーズ10 10-12 で意図して保守的にした。
コメントにも「同じ名前が別の場所で束縛されていると取りこぼす」とある）。A1・A2・A6〜A13 はすべてこれで通る。

保守的にした理由は 2 つ（names.rs の doc）。
1. **前方参照**（B1）。型検査は 1 回の順次走査で、関数の本体を**定義した時点で**検査するので、
   後で宣言する最上位の名前がまだスコープに無い。
2. **スコープの細部の食い違い**。型検査器が宣言し忘れている束縛があると、偽の「未定義」になる。

### 1-B. 型検査：クラスのメンバーをクラスのスコープに変数として宣言している

[stmt/check.rs](../src/type_check/stmt/check.rs) の `Stmt::Field` の腕が、最後に
`self.declare(name, ty, mutable)` でフィールドを**クラス本体のスコープ**に宣言する。メソッドの本体は
そのスコープの内側で検査されるので、裸の `font` が `str` 型の変数として引ける（A3・A4）。
実行時も参照実装も、クラスのメンバーは `self.x` / `C.K` 以外では見えない。
⚠ メソッド名（A6）がどの経路で通っているか（`declared_anywhere` か、関数表 `fn_sigs` が裸の名前で
引けるのか）は未確認（1-1 で調べる）。

### 1-C. 拡張：見えない名前を「ファイル内で最初の同名の宣言」に結び付ける

[wasm_providers.ts `lookup`](../vscode-extension/src/wasm_providers.ts) は、その行から見える宣言
（`visibleSymbols`）に無ければ、`analysis.symbols.find(s => s.name === name)`、つまり**スコープを無視して
ファイル内で最初の同名の宣言**を返す。`initializer.ar` で最初の `font` は `EditorConfig` のフィールドなので、
そのフィールドとして表示された。この行は wasm に切り替えた最初のコミット（`93eca1db`）からあり、理由は残っていない。
呼び出し元はホバー・定義へ移動・シグネチャヘルプ・受け手の型の推定（補完）の 4 か所。

加えて `visibleSymbols` は、パーサのスコープ木を行の範囲でたどり、そのスコープの宣言を全部見せる。
1-B と同じく、メソッドの中からクラスのフィールドが裸で見える（A3 の表示）。ブロックの外への漏れ（A12）、
同じスコープで後に宣言する名前が見えるかどうか（宣言の順序）も、型検査の規則と揃っていない。

## 2. 修正の方向

### 型検査の新しい規則（フェーズ 1）

裸の名前を読み、スコープ（`lookup`）で引けなかったときに**通す**のは次のどれかだけにする。

1. 今と同じ: 実行時の組み込みの名前・`Self`・`_`・今見えている型変数・クラス / trait / protocol /
   テンプレートの名前・**最上位の**関数の名前。
2. **読んでいる場所が、後で実行される本体（関数・`gen`・メソッド・ラムダ）の中**で、名前が
   **このモジュールの最上位の束縛**であるとき（B1 の前方参照）。最上位の束縛とは、モジュールの最上位の文が
   直接束縛する名前（`let` / `mut` / `const` / `fn` / `class` / `import` の束縛など）。
   ⚠ 最上位の `if` / `for` などの**中**の束縛は含めない（ブロックスコープ・A13）。
   ⚠ 最上位の文そのもの（関数の外）では使わない。最上位は順に実行されるので、後で宣言する名前を
   先に読むのは誤り（A8）。
⚠ 入れ子の関数から、外側の関数で**後に**束縛する名前（B2・B3）は通さない（D-1 = (a)）。外側の関数の本体は
   入れ子の関数の定義より前の分だけが見える（今の順次走査のスコープのまま）。

`declared_anywhere`（プログラム全体の集合）はやめ、2 の「モジュールの最上位の束縛」の集合に置き換える。
クラスのメンバーは、クラスのスコープに変数として宣言しない（1-B）。

⚠ **呼ぶ順番の誤りは捕まえない**（例: `g()` が `H` を読むが、`g()` を `let H` より前に呼ぶ）。静的に
決めるには呼び出しの流れを追う必要があり、本計画の範囲外。今も実行時の `NameError` で止まる。

⚠ **Python から変換したモジュール（`import[py]`）は Python のスコープ規則**（関数単位。`if` / `for` の中の
束縛は関数全体で見える）。そこにブロックスコープの規則を当てると偽の誤りになる。変換したコードが型検査で
どう扱われているか（同じ検査器を通るか、`for` の変数などを変換器が持ち上げているか）を 1-1 で確かめ、
Python のモジュールでは関数単位の集合を使うか、判定を変えない。

⚠ **偽の誤りを出さないことが最優先**（10-12 で保守的にした理由）。新しい規則に切り替える前に、
型検査器が宣言し忘れている束縛を 1-1 で洗い出して 1-2 で埋める。

### 拡張（フェーズ 2）

- `lookup` の「ファイル内で最初の同名の宣言」をやめる。引けない名前は「解決できない」とし、ホバー・定義へ
  移動を出さない（「掬いきれなかった選択肢はエラーにする」。別の宣言に置き換えない）。
- 名前 → 宣言の引き方を型検査の規則に揃える（D-2）。

## 3. 要判断

| # | 内容 | 案 | 推奨 |
|---|---|---|---|
| **D-1** | 入れ子の関数から、外側の関数で**後に**束縛する名前を読む形（B2: 外側のローカル・B3: 後に定義する入れ子の関数）を許すか。今の実行時は値を `None` にしてしまう（§5 R-1）が、参照実装は呼んだ時点の値を読む（CPython と同じ） | (a) 静的な誤りにする（Arrow のクロージャは定義した時点で値を写すので、そのときに無い名前は写せない）。(b) 許す（入れ子の関数の相互再帰が書ける）。実行時を参照実装に合わせる（R-1） | **決定（2026-11-09）: (a)**。⚠ B3（入れ子の関数どうしの相互再帰）を書いている例題が 1-1 で見つかったら、それは誤りの例として直す（入れ子をやめて最上位の関数にする） |
| **D-2** | 拡張の「名前 → 宣言」の引き方をどこで決めるか | (a) TypeScript の `visibleSymbols` をフェーズ 1 の規則に合わせて直す（メソッドからクラスのメンバーを外す・ブロックスコープ・宣言の順序）。(b) 型検査器が、識別子を**使っている位置ごとに**結び付いた宣言を記録し、wasm がそれを返す。拡張はそれを引くだけにする | **決定（2026-11-09）: (b)**。拡張は「wasm 以外に解析ロジックを持たない」方針（CLAUDE.md）で、(a) はスコープ規則を TypeScript に二重に書くことになり、またずれる |

## 4. タスク

### フェーズ 1 — 型検査（exe と拡張の診断）

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| **1-1** | ✅（2026-11-10・結果は表の下）**影響の測定**（コードは残さない）。§2 の規則を試しに入れたビルドで、9 カテゴリ・`archived/`・`practical_examples/`・import 先（`.ar` / `import[py]` の変換結果）すべての例題に型検査をかけ、新しく出る `UndefinedName` を全部並べて 3 つに分ける。① 例題の本当の誤り ② **型検査器の宣言漏れ**（宣言しているはずの束縛を宣言していない。`match` の束縛・組の分解・`with .. as`・関数の中の `static mut`・内包表記・メタ展開が作る名前など、見つかったもの） ③ D-1 の形（B2・B3）。あわせて確かめる: メソッド名（A6）・入れ子の関数（A7）が通る経路（`declared_anywhere` か `fn_sigs` か）、`Stmt::Field` の `declare` に頼っているもの（束縛の記録 `BindingRecord` や拡張のホバー・インレイヒントなど）、Python のモジュールの扱い（§2 の ⚠）、REPL（前の入力の名前が大域に入っているか）。結果を本書に書く | なし | 中 |
| **1-2** | ⇒ **対象なし**（1-1 で ② が 0 件）。1-1 の ② を埋める。型検査器が束縛をスコープに宣言するようにする。宣言の漏れは型の検査の漏れでもあるので、`type_obligations` の退行を見る | 1-1 | 1-1 次第 |
| **1-3** | ✅（2026-11-10・結果は表の下）クラスのメンバーを裸の名前で見せない（1-B）。`Stmt::Field` の腕でクラスのスコープに宣言するのをやめる（頼っているものは 1-1 の結果に従って別の経路にする）。メソッド名が裸で通る経路があれば塞ぐ。例題: 成功例はメソッドから `self.x` / `C.K` で読む形、エラー例（`_error`）は A3〜A6 | 1-1 | 小 |
| **1-4** | ✅（2026-11-10・結果は表の下）`declared_anywhere` を「モジュールの最上位の束縛」に置き換え、§2 の規則 2 を入れる（1-A）。Python のモジュールは 1-1 の結果に従う。`names.rs` / `check_name_defined` の doc の規則を書き直す。例題: 成功例は B1（前方参照）、エラー例（`_error`）は A1・A2・A7〜A13 と、利用者の報告の形（別の関数のローカルを渡す） | 1-2, 1-3 | 中 |
| **1-5** | ✅（2026-11-10）1-4 の規則で B2・B3 とも静的な誤りになった（`fn_sigs` で型が引けても判定を通すので B3 も止まる）。[block_scope_error.ar](../examples/basics/block_scope_error.ar) の④（B2）・⑥（B3）。⚠ ⑥は間に `let` を挟んだ形にした（間を挟まずに続く定義どうしはフェーズ 3 で組として呼べるようになるため、その後も誤りのままの形を残す）。入れ子の関数の自己再帰は型検査を通る（実行時の `NoneType` はフェーズ 3）。ゲート: `scan_examples` FAIL 0・`compare_wasm_frontend` 772 / 772。D-1 = (a): B2・B3 を静的な誤りにする（1-4 の規則で自然に誤りになるはずなので、そうなることを確かめてエラー例（`_error`）に足す） | 1-4 | 小 |
| **1-7** | ✅（2026-11-10・結果は表の下）**関数のシグネチャの表をスコープで分ける**（2026-11-09 に追加）。`fn_sigs`（[registry/builder.rs](../src/type_check/registry/builder.rs)）は名前だけをキーにしたプログラム全体の表で、別々の関数の中の同じ名前の入れ子の関数（`a` の `helper(x: int)` と `b` の `helper(s: str)`）を多重定義としてまとめる。実測: `b` の中の `helper(2)` が静的に止まらず、実行時の `TypeError`。入れ子の関数のシグネチャは、その定義を検査するときにそのスコープへ宣言し、表から引くのは最上位の関数（とモジュールの関数）だけにする。例題: エラー例（`_error`）に上の形 | 1-4 | 小〜中 |
| **1-6** | ✅（2026-11-10）`docs/grammar/02_variables.md` に「見えない名前は静的エラー」の節（見える名前 3 つ・誤りになる形の表）、`06_classes_traits.md` にメンバーを裸で読めないこと（1-3 で）、`type-checking` スキルの Identifiers と `fn_sigs` の節、`.claude/rules/language-differences.md` に 1 項目、`for_target_scope_error.ar` の古い記述は 1-4 で書き直した。VSIX を作り直した（`vscode-extension/make-vsix.ps1`・`arrow-0.5.0.vsix`）。文書を直す。`type-checking` スキルの未定義名の節、`docs/grammar/02_variables.md` のスコープ規則（静的に検査することを書く）、[for_target_scope_error.ar](../examples/basics/for_target_scope_error.ar) の「型検査には未定義変数の検査が無い」（古い記述。ループ変数の誤りが静的になる）。型検査が変わるので VSIX を作り直す（`make-vsix.ps1`） | 1-4, 1-5 | 小 |

**ゲート**（1-2〜1-5 の各タスクで）: `test_build_gate` / `scan_examples` / `force_gate` / `compare_python_impl` /
`type_obligations` / `compare_wasm_frontend` / `repl_session`（REPL の大域）。識別子を改名・削除したら `stale_doc_refs`。
⚠ 例題ゲートは `archived/` と `practical_examples/` を見ない（CLAUDE.md）。1-1 の測定ではこの 2 つも数えるが、
直すかどうかは各ディレクトリの README の方針に従う。

#### 1-1 の結果（2026-11-10・✅ 完了）

**方法**。測定用の印（コミットしていない）を入れた release を別の `target/trial` に作り、`Expr::Ident` を推論するたびに
**今の規則では通り、§2 の規則では落ちる名前**を `TRIAL` 行として stderr に出した（振る舞いは変えない）。試した規則:
- スコープで引けた名前でも、引けた先が**クラスの本体のスコープ**なら落とす（`CLASS-MEMBER`・1-B）。
- 関数表（`fn_sigs`）で引けた名前は、**今のモジュールの最上位の名前**のときだけ通す（`FN-SIGS`・A7）。
- どこにも無い名前は `declared_anywhere` を使わず、§2 の規則 1・2 だけで通す（`ANYWHERE`）。
  「後で実行される本体」は `fn` / `gen` / メソッドの本体（Arrow にラムダ式は無い）。
`examples/` の下の `.ar` を全部（842 本。9 カテゴリ・`archived/`・`practical_examples/`・import 先のモジュール）、
`AR_CHECK_ONLY=1` で検査した（時間切れ 0）。§0 の再現ファイルでは A1〜A13・B2・B3 がすべて `TRIAL` になり、
B1（前方参照）と入れ子の関数の自己再帰はならないことを先に確かめた。

**結果: 新しく落ちる名前は 5 件だけ**。

| 分類 | 件数 | 箇所 |
|---|---|---|
| ① 例題の本当の誤り | 3 | `archived/scope_test.ar:5`（`block:` の中の `inner` を外で読む）・`basics/block_scope_error.ar:49`（`y`）・`basics/for_target_scope_error.ar:41`（`i`）。どれも誤りを見せるための例題で、実行時の `NameError` が静的な誤りになる |
| ② 型検査器の宣言漏れ | **0** | — |
| ③ D-1 の形（B2・B3） | **0** | 入れ子の関数どうしの前方参照を書いている例題は無い（⇒ §3 D-1 の ⚠ の「例題を直す」は対象なし） |
| その他: Python のモジュール | 2 | `interop/py_class_factory.ar` / `_error.ar` が読む Python のモジュールの `tag`（`CLASS-MEMBER`）。下の「Python のモジュール」 |

②が 0 なのを例題の偏りで見落としていないか、束縛の形を全部使った正しいプログラムを別に書いて試した（作業用の `forms.ar`:
`let` / `mut` / `const` / 関数の中の `static mut` / 組の分解（`let a, let b = ..`・`mut`）/ 関数の中の `enum` / 入れ子の `fn` と `gen` /
`for` の組のターゲット / `for` 式と `loop_yield` / ブロック式 / `except .. as` / リスト・集合の内包表記 / `match` / 型ガード /
`freeze` / `import .. as` / `from .. import` / 最上位の前方参照をメソッドから読む / クラスの名前を自分のメソッドから読む）。
`TRIAL` は 0 件。

**名前が通る経路**（1-1 で確かめる項目）
- **A6（メソッド名を裸で読む）**: `declared_anywhere` ではなく**クラスのスコープ**。`check_fn_def` がメソッドの名前も
  クラスの本体のスコープへ `declare` している（`Stmt::Field` と同じ形）。⇒ 1-3 で両方を外す。
- **A7（別の関数の入れ子の関数）・B3**: `fn_value_type` が**プログラム全体の `fn_sigs`** を引いて通す。⇒ 1-4 で
  「今のモジュールの最上位の関数」だけにする。1-7 と同じ根（`check_fn_def` は入れ子の関数の名前も `fn_value_type` の型で
  宣言するので、別の関数に同じ名前があると多重定義として `Unresolved` になる）。
- **`Stmt::Field` の `declare` に頼っているもの**: 見つからない。`Field` の腕は束縛の記録（`note_binding`・拡張のホバー /
  インレイヒント）を呼ばず、`self.x` / `C.K` はレジストリ（`class_field_details`）から引く。拡張のフィールドのホバーは
  パーサの宣言表（`editor_index`）から来る。⚠ メソッド名の `declare` を外すときは、同じ名前のメソッドの再宣言の判定
  （`already_declared`）がそれを使っていないかを 1-3 で確かめる。

**Python のモジュール**（§2 の ⚠）
- 入口のファイルが Python になることは無い。import したモジュールの本体は `annotate_module_body` が**取り込む側の中で**
  検査し、**誤りは捨てる**（具体化の本体の誤りだけを残す・[resolve.rs](../src/type_check/stmt/resolve.rs)）。
  ⇒ 規則を変えても、Python のモジュール（と import した `.ar` のモジュール）の中の誤りは表に出ない。Python 用に
  関数単位の集合を作る必要は無い。
- 見つかった 2 件は、関数の中の Python のクラスのメソッド `tag` が、外側の関数の引数 `tag` を読む形
  （`def make(tag): class Tagged: def tag(self): return tag`）。CPython ではメソッドの本体からクラスの本体の名前は
  見えないので、これは外側の引数。型検査器はクラスのスコープのメソッド名 `tag` を引いていた（型を取り違えている）。
  ⇒ **1-3 で正しくなる側**（誤りが出るのではなく、引く先が直る）。

**REPL**: 型検査の誤りを無視して実行する（[repl.rs `run_block`](../src/repl.rs)）ので、規則を変えても振る舞いは変わらない。

⇒ **1-2 は対象なし**（②が 0 件）。1-3 から進める。

#### 1-3 の結果（2026-11-10・✅ 完了）

- クラス・trait の本体の直下では、メンバー（`Stmt::Field` と、メソッドの `fn` / `gen`）をスコープに宣言しない
  （`CheckState::in_class_body`）。メンバーの型・可変性はもともとレジストリが持っていて、`self.x` / `C.K` はそちらから引く。
- 見えない名前が検査中のクラスのメンバーなら、`declared_anywhere` より先に専用の誤り `BareClassMember` を出す
  （`name 'font' is not defined ('font' is a member of 'C'; write `self.font`)`。`const` / `static mut` は `C.K` を勧める）。
  先に見るのは、メンバーの名前はたいていどこか別の場所（`__init__` の同名の引数など）でも束縛されていて、
  `declared_anywhere` で通ってしまうため。⚠ モジュールの最上位の束縛と同じ名前は対象外（メソッドから最上位の変数を読む・前方参照）。
  そのため**モジュールの最上位の束縛の集合**（`TypeChecker::module_toplevel`・`names::collect_toplevel_bindings`）を先に入れた（1-4 でも使う）。
- **代入の左辺**（`x = ..` / `x += ..`）にも同じ判定を入れた。以前は読みだけを検査していて、`opened += 1`（クラスの `static mut` を
  裸で書く）や、関数の中の宣言していない名前への代入（実行時 `NameError`）が静的に通っていた。
- 振る舞いが**緩くなる**所: メソッドの中で、メンバーと同じ名前のローカル（`let font = ..`）を宣言できるようになった。以前は
  クラスのスコープのメンバーとぶつかって `VariableRedeclaration` だった（実行時は問題なく動く形）。参照実装は今も誤りにする
  ので、`compare_python_impl` の既知の差に `class_member_scope` を足した。
- 例題: [class_member_scope.ar](../examples/classes/class_member_scope.ar)・[class_member_scope_error.ar](../examples/classes/class_member_scope_error.ar)（A3〜A6 と、
  クラスの本体の `const B = A + 1`・`static mut` への裸の `+=`。5 件とも静的な誤り）。
  ⚠ 1-1 の Python のモジュールの `tag`（`py_class_factory`）は、引く先が外側の関数の引数に直った。
- ゲート（release）: `scan_examples` FAIL 1・TIMEOUT 1（`interop/async_native_call.ar` の LINK1104 は一時ファイルの取り合いで、
  単独で再実行して成功。`bench_ab_native.ar` の時間切れは負荷）。`force_gate` 0 件。`type_obligations` 退行なし（149 件中 STATIC 147）。
  `compare_wasm_frontend` 772 / 772 一致。`repl_session` 同一。`cargo test`（debug）1302 passed。`compare_python_impl` 予期しない差は上の 1 件だけ（既知の差に足した）。
  ⚠ 同じ実行で `interop/cpp_raw_pointers.ar` の既知の差が「一致した（古い）」と出た。Rust 側の出力は変わっておらず
  （型検査を通って最後まで動く）、参照実装の側の環境による揺れ。今回は触らない（1-4 の実行では出なかった）。

#### 1-4 の結果（2026-11-10・✅ 完了）

- `declared_anywhere`（プログラム全体で束縛される名前の集合）をやめた。見えない名前を通すのは、§2 の規則 1 の名前
  （組み込み・クラス / trait / protocol / テンプレート・型変数・`Self`・`_`）と、規則 2 の**後で実行される本体の中から読む、
  このモジュールの最上位の束縛**（`CheckState::in_deferred_body`・1-3 で入れた `module_toplevel`）だけ。
- ⚠ §2 の規則 1 の「最上位の関数の名前」は**特別扱いをやめた**。最上位の関数は定義した時点からスコープにあり、その前に
  読めるのは後で実行される本体の中だけ（規則 2 に入る）。実行時も関数を巻き上げない（最上位の `f()` の後の `fn f` は
  `NameError`）ので、規則 1 のままだと A8 と同じ形の取りこぼしが残った。
- 関数表（`fn_sigs`）で型が引けても、スコープに無い名前は判定を通す（`Expr::Ident`）。A7（別の関数の入れ子の関数）・
  B3（後で定義する入れ子の関数）はこれで止まる。
- 同じ文で同じ名前の「見えない名前」の誤りは 1 回だけ出す（`already_reported_undefined`）。`n = n + 1` の左辺と右辺が
  2 回出していたのと、§5 R-2 の 2 回出る件（`mh.enable(1)`）がこれで直った（R-2 の位置が `(` を指す件は残る）。
- 例題: 成功例 [name_resolution.ar](../examples/basics/name_resolution.ar)（メソッドからの前方参照を足した）。エラー例
  [name_resolution_error.ar](../examples/basics/name_resolution_error.ar)（9 件: 利用者の報告の形・A2・A7・A8・関数の巻き上げ・A13・宣言していない名前への代入）、
  [block_scope_error.ar](../examples/basics/block_scope_error.ar)（5 件: A12・B2・`except .. as e`）、
  [for_target_scope_error.ar](../examples/basics/for_target_scope_error.ar)（3 件: A9・A11）。後の 2 本は実行時の `NameError` の例だったので、
  全部の形を本文に書いて静的な誤りの例にした。
- 全例題（842 本・`archived/`・`practical_examples/` を含む）で、1-3 の前の exe と「is not defined」の誤りを比べた: **増えたのは
  22 件ですべて誤りの例**（上の 4 本＋[class_member_scope_error.ar](../examples/classes/class_member_scope_error.ar)・
  [global_assign_from_fn_error.ar](../examples/basics/global_assign_from_fn_error.ar)・`archived/scope_test.ar`）。正しい例題で増えたものは 0、減ったものは 0。
  `global_assign_from_fn_error.ar` は実行時の経路の例だったが静的な誤りになった（実行時の経路は型検査を通さない単体テストが確かめる）。
- ゲート（release）: `scan_examples` FAIL 0（TIMEOUT 1 は `bench_ab_native.ar`・負荷）。`force_gate` 0 件。`type_obligations` 退行なし。
  `compare_wasm_frontend` 772 / 772。`repl_session` 同一。`stale_doc_refs` OK。`compare_python_impl` 予期しない差 0（`for_target_scope_error` は
  参照実装も内包表記で止まって一致したので既知の差から外した。理由の文面を 2 件直した）。`cargo test`（debug）1302 passed。


#### 1-7 の結果（2026-11-10・✅ 完了）

- レジストリの `fn_sigs` には、関数の本体の中の `fn` / `gen`（入れ子の関数）を載せない（`TypeRegistryBuilder::fn_depth`）。
  ⚠ テンプレートの入れ子の関数は今までどおり載せる。最上位の `if` などの中の関数は今までどおり載る（深さ 0）。
- 入れ子の関数のシグネチャは、検査器がその定義を検査するときに作り（`TypeRegistry::sig_of`。ビルダーと同じ
  `TypeRegistryBuilder::sig_from` を通す）、定義したスコープへ積む（`CheckState::declare_local_fn`。スコープと同じ並びの表）。
- 検査器が関数のシグネチャを名前で引く所（呼び出しの引数の検査・戻り値の型・関数値の型・デコレータ。9 か所）は
  `TypeChecker::visible_fn_sigs`（見えている入れ子の関数を先に引き、無ければ最上位の関数）にした。同じ名前の変数が
  より内側で宣言されていれば、その変数が関数を隠す。
- 例題: [nested_fn_signature.ar](../examples/basics/nested_fn_signature.ar)（別々の関数の同じ名前の入れ子の関数・最上位の関数を隠す・既定値・入れ子の `gen`）、
  [nested_fn_signature_error.ar](../examples/basics/nested_fn_signature_error.ar)（2 件。以前はどちらも通った）。
- 全例題（842 本）で、1-3 の前の exe と**すべての静的な誤り**を比べた: 増えたのは 25 件ですべて誤りの例（1-3〜1-5 の 23 件と
  上の 2 件）。減ったのは 1 件で、1-3 で意図して緩めた形（`class_member_scope.ar` のフィールドと同じ名前のローカル）。
- ゲート（release）: `scan_examples` FAIL 1・TIMEOUT 1（`interop/js_proc_values.ar` は 1-3 と同じ LNK1104 の一時ファイルの取り合いで、
  単独で再実行して成功。`bench_ab_native.ar` は負荷）。`force_gate` 0 件。`type_obligations` 退行なし。`compare_wasm_frontend` 774 / 774。
  `repl_session` 同一。`stale_doc_refs` OK。`compare_python_impl` 予期しない差は `nested_fn_signature_error` だけ（参照実装は入れ子の関数を
  静的に検査しない。既知の差に足した）。`cargo test`（debug）1302 passed。
  ⚠ `scan_examples` の LNK1104（`_tl_cpp_bridge` の一時ファイル）は 3 回の実行で 2 回、別々の例題で出た。cpp の中継のビルドを
  並列に走らせたときの取り合いと見られる（本計画の変更とは関係ない・単独では毎回成功）。

### フェーズ 2 — VS Code 拡張（ホバー・定義へ移動・色付け）

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| **2-1** | ✅（2026-11-10・結果は表の下）`lookup` の「ファイル内で最初の同名の宣言」をやめる（1-C）。呼び出し元 4 か所（ホバー・定義へ移動・シグネチャヘルプ・受け手の型の推定）が `undefined` を扱えるか確かめる。`stress.js` の数え上げで、ホバー・定義が消えたものを全部見て、正しく消えた（見えない名前）か、2-2 で拾うべきもの（前方参照など）かを分ける | フェーズ 1 | 小 |
| **2-2** | ✅（2026-11-10・結果は表の下）D-2 = (b): 型検査器が、識別子を使っている位置ごとに結び付いた宣言（の位置）を記録し、`crates/arrow-frontend` の解析結果に載せ、`lookup` と色付け（`provideDocumentSemanticTokens`）がそれを引く。A1〜A3・A12 の形のホバーが出ず、B1 のホバーは出ること | 2-1 | 中 |
| **2-3** | ✅（2026-11-10・結果は表の下）`vscode-extension/test_fixtures/` に §0 の形を足し、`run_debug.js` / `stress.js` で確かめる。`vscode-extension-dev` スキル（名前の引き方）を更新する。wasm を作り直して VSIX を作る（`make-vsix.ps1`） | 2-2 | 小 |

**ゲート**: `compare_wasm_frontend` / `stress.js`（数え上げの退行） / `compare_tm_grammar`（触らないはずだが VSIX を作るので）

#### 2-1 の結果（2026-11-10・✅ 完了）

- `lookup` は見える宣言だけを返す（無ければ `undefined`）。4 か所の呼び出し元はどれも `undefined` を「出さない」として扱っていた。
- **測り方**: `stress.js` は宣言の位置しか調べないので、全例題の**識別子を使っている位置すべて**（約 46 000）で hover の 1 行目と
  定義へ移動の行き先を出す [uses_probe.js](../vscode-extension/uses_probe.js) を作り、変更の前後で全部突き合わせた
  （`vscode-debug-runner` スキルの §3）。
- 外しただけの段階で変わった 861 か所を全部見て、次の 2 つを直した:
  - **メンバー**（`self.x` / `c.f()`・784 か所）: 以前は名前だけで引いた「最初の同名の宣言」が**たまたま**同じメンバーに当たって
    宣言の hover（`public mut x: int`）が出ていた。外すとメンバー表の経路に落ちて表示が変わった（`public` や `self` が消える）。
    ⇒ 受け手の型が分かれば、その型のメンバーとしてこのファイルにある宣言を引く（定義へ移動と同じ）。逆に、以前は同じ名前の
    **別のクラスの**メンバーに当たっていた所（`Override.x` に `Mixed.x` を見せていた）も正しい持ち主になった（261 か所が宣言の hover）。
  - **キーワード引数の名前**（`f(a = 1)` の `a`・114 か所）: 以前は同じ名前の仮引数（たまたま）に当たっていた。外すと、型の付かない
    名前に式の型を補う経路が**呼び出し先の関数の型**を `a` の型として見せた。⇒ キーワード引数の名前には hover を出さない
    （仮引数へ結び付けるのは 2-2 以降の別件）。
- あわせて、型の付かない名前に式の型を補う `exprTypeAt` を、**その識別子で終わる式**の型だけを見る `exprTypeOfToken` に替えた。
  以前は同じ行でカーソルより左で終わる最も近い式の型を拾っていたので、変数 `parts` に `function{let obj:Any}->int`（`len` の型）、
  メソッド名 `split` に受け手の `str`、`pg.display` の `display` にモジュールの型を出していた（763 か所の誤った hover が消えた）。
- 残った変化はどれも「以前の結び付きが誤り」の側: `case _:` の `_` に別の `let _`、型注釈の中の仮引数の名前
  （`function{let value:int}`）・`m.name` の `name`・`import[py-int] time` の `py`・テンプレートの型変数 `K` に別のクラスの `const K`、
  内包表記の変数に離れた行の同じ名前の宣言、見えない名前（`name_resolution_error.ar` の `font` / `title` / `nested` / `inside`）。
  ⚠ 内包表記の変数は宣言表に載っていないので、今は hover に型だけが出て定義へ移動が無い（2-2 で型検査器の結び付きから引く）。
- `stress.js`: threw 0・hover misses 0 / 6713・def misses 0 / 6713・bind misses 0 / 427・typed vars 2855 / 3508（変更前と同じ）・tag misses 0 / 14。
  VSIX を作り直した。

#### 2-2 の結果（2026-11-10・✅ 完了）

- 型検査器（エディタの経路だけ）が、識別子を**使っている位置ごと**に結び付きを記録する（`NameUse`・node-id 単位）。
  - 変数を束縛した記録（`BindingRecord`）に番号を付け、宣言した変数（`VarInfo::binding`）に持たせる。使っている位置は
    その番号を記録する（型ガードの絞り込みは同じ変数なので番号を引き継ぐ）。
  - **見えない名前**（`UndefinedName` / `BareClassMember` になった名前）は「見えない」と記録する（`check_name_defined` が
    見えたかどうかを返すようにした）。
  - 記録しない位置（引数・関数・クラスの名前・組み込み・前方参照・束縛の記録が無いもの）は、拡張が従来どおりスコープ木から引く。
- `crates/arrow-frontend` は、束縛の記録を宣言表と突き合わせる `bound_types` が「記録の番号 → 宣言」も返すようにし、
  解析結果に `uses`（使っている位置 → 宣言の位置、見えなければ `null`）を載せる。数が合わない鍵（どの宣言か決まらない）は出さない。
- 拡張: `lookup` は使っている位置の記録があればそれに従う（見えなければ `undefined`）。hover は見えない位置に何も出さない
  （式の型も補わない）。色付け（`provideDocumentSemanticTokens`）も同じ記録に従う。
- `uses_probe.js` で 2-1 の後と比べた: 消えたのは 15 か所ですべて見えない名前（A12 のブロックの後・宣言より前の読み・
  B2・B3・`for` の変数をループの後で・`except .. as e` の `e` を後で・最上位で後に束縛する名前・メソッドの中の裸のメンバー（A3）・
  クラスの本体の裸の `const`）。変わったのは 4 か所で、どれも正しい側へ: クラスのメンバーと同じ名前の最上位の変数
  （`class_member_scope.ar` の `size` が、以前はフィールドを指していた）、再宣言の誤りの後の読みが後の宣言へ、`if` の中で `freeze`
  した変数の読みが元の宣言へ（`freeze` は変数そのものを書き換えるので、ブロックの中の `freeze` 文ではなく元の宣言が正しい）、
  最上位で定義より前に呼んだ関数（`name_resolution_error.ar` の `later_fn`）に型だけが出ていたのが出なくなった。
- §0 の拡張の表: A1 / A2（`use_it` の `font`）・A3（メソッドの中の裸の `font`）・A12（ブロックの外へ漏れた `y`）とも、hover も
  定義へ移動も出ない。B1（前方参照）は出る（`name_resolution.ar` の `LIMIT` / `helper` / `GREETING`）。
- ゲート: `compare_wasm_frontend` 774 / 774・`stress.js`（threw 0・hover / def misses 0 / 6713・bind misses 0・typed vars 2855 / 3508 で変わらず・
  tag misses 0）・`scan_examples` FAIL 0・`type_obligations` 退行なし・`repl_session` 同一・`stale_doc_refs` OK・`cargo test`（debug）1302 passed。


#### 2-3 の結果（2026-11-10・✅ 完了）

- [test_fixtures/name_scope_cases.ar](../vscode-extension/test_fixtures/name_scope_cases.ar): §0 の形（A1 / A2・A3・A12・宣言より前の読み・B1・B2・
  クラスのメンバーを `self.x` / `C.K` で読む・同じ関数の 2 つの `for i` のそれぞれ）を 14 の印（`use:<name>=<label|none>`）で並べた。
- `stress.js` に `scope misses` を足した（印どおりに定義へ移動が飛ぶか・見えない名前に hover も定義も出ないか）。**0 / 14**。
  負の対照: 2-1 の前の `wasm_providers.ts` では 6 / 14（検査が退行を捕まえることを確かめた）。
- スキル: `vscode-extension-dev` に「使っている位置が宣言を引く順序」（`uses` → スコープ木 → 何もしない）、
  `vscode-debug-runner` に `scope misses` と `uses_probe.js`。
- VSIX を作り直した。`compare_tm_grammar`（`test_fixtures/` を足したので）: SPILL 0・キーワードの一覧が一致。

### フェーズ 3 — 入れ子の関数の再帰（連続する `fn` を 1 つの組として作る）

起票: 2026-11-09（利用者の指示。前方宣言は入れず、前方宣言なしで再帰を書けるようにする）。

**規則**。関数（`fn` / `gen` / メソッド）の本体の中で、**間に別の文を挟まずに続く入れ子の `fn` / `gen` の定義の並び**を
1 つの**組**とする（OCaml の `let rec f = .. and g = ..` と同じ。組は並びで決まり、書き手は何も足さない）。

- 組の中の関数どうしは、互いの名前（自分の名前を含む）を読める。組を全部作ってから、互いの名前を書き込む。
  ⇒ 自己再帰（§5 R-1 の「残り」）は「1 つだけの組」として動く。入れ子の関数どうしの相互再帰（B3）は、
  定義が続いていれば動く。
- 組の**外**の名前は今の D-1 = (a) のまま。組より前に束縛されていなければ静的な誤り。
  組の途中に別の文（`let` など）が入れば、組はそこで切れる（その後の `fn` は別の組）。
- ⚠ デコレータの付いた入れ子の関数は組を切る（デコレータは定義の時点で走り、そのときに組の残りはまだ無い）。
  今の VM は入れ子の関数のデコレータを扱わない（`nested-fn-decorator`）ので、まず型検査と木の実行器の規則として決める。
- ⚠ ブロック（`if` / `for` / `block:` …）の中の定義の並びも同じ規則（そのブロックの中の連続）。
- ⚠ Python から変換したモジュールは CPython の規則（呼んだ時点の値を読む）のままで、本フェーズの対象外。
- やらないこと: JavaScript のような巻き上げ（入れ子の関数をブロックの先頭へ持ち上げる）。持ち上げた関数が
  後ろの `let` を写すことになり、D-1 = (a) とぶつかる。

**実測**（2026-11-09・release の exe と `impl_python`）。`fn outer(): fn fact(n): .. fact(n - 1) ..; fact(5)` は
`TypeError: 'NoneType' object is not callable`（参照 `120`）、`ev` / `od` の相互再帰は `NameError: 'od' is not defined`
（参照 `True`・R-1 より前は `NoneType` の誤り）。最上位の関数の相互再帰とクラスの相互参照は今も動き、型の誤りも
静的に出る（型検査は本体の前に全部のシグネチャを集める・[registry/builder.rs](../src/type_check/registry/builder.rs)）。
⇒ 再帰の問題は**シグネチャではなく実行時の束縛**（入れ子の関数は定義した時点の値を写す）。

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| **3-1** | ✅（2026-11-10・結果は表の下）**測定**（コードは残さない）。全例題（9 カテゴリ・`archived/`・`practical_examples/`・import 先）で、入れ子の関数の並びと、自己再帰・相互再帰をしている箇所を数える。実行器ごとの今の扱いを確かめる: VM（`compile_nested_fn_def` の `MakeFn` と `captures` / `cell_captures`）・最上位の木の実行器（最上位の `if` などの中の `fn`）・ネイティブコンパイル（`--compile`。入れ子の関数を載せるか）・デバッガ。⚠ 関数の値を互いに写すと `Rc` の循環になる（組の関数がずっと解放されない）。今の関数の値の持ち方で循環が起きるか、起きるなら弱い参照・枠の共有（`cell`）のどれで切るかを決めて本書に書く | フェーズ 1 | 小 |
| **3-2** | ✅（2026-11-10）`check_stmts` が組（`fn_group_len`: 間に文を挟まずに続く `fn` / `gen`。デコレータ・テンプレートの付いたものは組を切る）の名前とシグネチャを最初の定義より前に宣言する（`predeclare_fn_group`。各定義の検査は先に宣言した分を飛ばす）。クラスの本体の直下は対象外（メソッド）。`ev` / `od` の相互再帰が型検査を通り、`block_scope_error.ar` の⑥（間に `let`）は誤りのまま（6 件）。⚠ 実行時はまだ動かない（3-3）。ゲート: `scan_examples` FAIL 0・`type_obligations` 退行なし・`compare_wasm_frontend` 774 / 774。型検査: 組の関数の名前とシグネチャを、組の最初の定義より前に宣言してから本体を検査する（組の中の前方参照を通す）。組の外の後の名前は 1-5 のとおり誤りのまま。⚠ 1-5 の B3 の例（`block_scope_error.ar` の⑥）は初めから組を切った形（間に `let`）にしてあるので、誤りのままであることを確かめる | 3-1 | 小 |
| **3-3** | ✅（2026-11-11・結果は表の下）VM: 組の関数を全部作ってから互いの名前を書き込む（3-1 で決めた持ち方）。`compile_nested_fn_def` の自分の名前を除く例外（R-1）は組の規則に置き換える。最上位の木の実行器も同じ規則にする | 3-2 | 中 |
| **3-4** | ✅（2026-11-11・コードの変更なし）ネイティブコンパイルは入れ子の `fn` を含む関数を載せない（`llvm_codegen` の `stmt_eligible` に `FnDef` の腕が無く `false`）。3-3 はこの判定に触れていないので、載らない理由は変わらない（組を含む関数も VM で動く）。デバッガは 3-3 のゲートの `debug_session` が同一。ネイティブコンパイル（3-1 で入れ子の関数を載せていたら）とデバッガを組の規則に合わせる。載せていなければ、載らない理由が変わらないことだけ確かめる | 3-3 | 3-1 次第 |
| **3-5** | 例題（成功例: 自己再帰・相互再帰・ブロックの中の組・`gen` を含む組、エラー例（`_error`）: 組の途中に文を挟んだ前方参照）。文書（`docs/grammar` の関数の節・`language-differences`・`type-checking` / `vm-pitfalls` スキル）。型検査が変わるので VSIX を作り直す | 3-3 | 小 |

**ゲート**: `test_build_gate` / `scan_examples` / `force_gate` / `compare_python_impl` / `compare_bytecode -A`（組を作らない
プログラムは差分 0）/ `compare_outputs -A` / `type_obligations` / `compare_wasm_frontend` / `debug_session` / `repl_session`。

#### 3-1 の結果（2026-11-10・✅ 完了）

**例題**: 846 本の中の入れ子の `fn` / `gen` の定義は 104 か所。自己再帰しているものは **0**、入れ子の関数どうしの相互再帰も
**0**（1-1）。⇒ 組の規則を入れても既存の例題の振る舞いは変わらない（変わるのは今 `NoneType` / `NameError` で落ちる形だけ）。

**今の振る舞い**（release の exe・参照は `impl_python`）

| 置き場所 | 自己再帰 | 参照 |
|---|---|---|
| 関数の中（`fn outer(): fn fact(n): .. fact(n - 1)`） | `TypeError: 'NoneType' object is not callable` | `120` |
| メソッドの中 | 同上 | `24` |
| 関数の中の `gen`（`for x in walk(n - 1)`） | 同上 | `6` |
| 入れ子の関数を返して外で呼ぶ（`let f = make(); f(5)`） | 同上 | `NameError: name 'fact' is not defined`（参照実装も動かない） |
| 最上位の `if` の中（木の実行器・`vm_toplevel`） | 同上 | — |
| 最上位 | `120`（最上位の関数は名前で引くので動く） | `120` |

相互再帰（`ev` / `od`）は 1-4 から静的な誤り（`name 'od' is not defined`・D-1 = (a)）。

**実行器ごとの仕組み**
- VM: `Op::MakeFn`（[run.rs `make_fn`](../src/vm/run.rs)）が、定義した時点の外側の枠を `captures` として**値で写す**
  （可変なものはセルを共有・`cell_captures`）。自分の名前の枠は `MakeFn` の後に書くので、写した値は `None`。呼び出し時は
  `bind_captures`（[execution.rs](../src/interpreter/functions/execution.rs)）が `captured_env` を本体の枠へ書き込む。
- 最上位の木の実行器も、入れ子の関数の値（`FnValue::captured_env`）の作り方が同じで、同じ形で落ちる。呼び出しは
  どちらも VM の本体（関数の本体は必ず VM にコンパイルされる・#33）で、`bind_captures` を通る。
- ネイティブコンパイル: 入れ子の `fn` を含む関数は対象外（`stmt_eligible` に `FnDef` の腕が無い）。⇒ 3-4 は
  「載らない理由が変わらない」ことの確認だけ。
- デバッガ: 入れ子の関数の値の作り方は触らないので、停止時の表示の変化は 3-3 のゲート（`debug_session`）で見る。

**`Rc` の循環**（3-1 で決める項目）。関数の値は `Rc<FnValue>` で、`captured_env` に入れた値は強い参照。自分や組の相手の
**値**を `captured_env` に入れると `FnValue → captured_env → FnValue` の循環になり、外側の関数を呼ぶたびに組の関数が
解放されなくなる（Arrow の値は参照カウントだけで、循環を回収しない）。⇒ **値を写さない**:
- **自分の名前**: 新しい捕捉の種類（`CapturedVar` に「自分」）を足し、`bind_captures` が**呼ばれている関数そのもの**
  （`fn_val` の `Rc`）を枠へ書く。呼び出しのフレームが持つだけなので循環しない。割り当ても増えない。
- **組の相手**: 組の定義（各メンバーの名前・仮引数・本体・外側の捕捉など、関数の値を作る材料）を 1 つの `Rc` に持ち、
  メンバーの関数の値は「組 ＋ 自分の番号」を持つ。相手の名前は `bind_captures` が組から**その場で関数の値を作って**枠へ書く。
  組は関数の値を持たないので循環しない。代わりに、相手を呼ぶ本体に入るたびに相手の数だけ関数の値を作る（相互再帰の
  ときだけの費用。自己再帰と、組でない入れ子の関数は今と同じ）。
- 弱い参照（`Weak`）は使わない。組の相手だけを返して外で呼ぶと（`return ev`）、残りのメンバーが解放されて呼べなくなる。

#### 3-3 の結果（2026-11-11・✅ 完了）

- 捕捉に印を 2 つ足した（[callables.rs `CapturedVar`](../src/interpreter/value/callables.rs)）: `SelfFn`（自分の名前）と `Sibling(j)`（組の
  `j` 番目の相手）。`Op::MakeFn`（[run.rs `make_fn`](../src/vm/run.rs)）は、捕捉のうち自分の名前と組の相手の名前は値を写さず印にする。
  呼び出しの `bind_captures`（`fn`）と `gen` の本体の枠の書き込みが、`SelfFn` には呼ばれている関数そのもの、`Sibling` には組から
  作った相手（`FnGroup::member_value`）を書く。
- 組（`FnGroup`）は組を持たない**写し**を持ち、メンバーの関数の値が `FnValue::group` / `GeneratorFnValue::group` で組を持つ
  （循環しない）。組を繋ぐのは**最後のメンバーの `MakeFn`**（`ChunkFnDef::group_first` → `link_fn_group`）。新しい命令は足していない。
- 組は型検査と**同じ関数**で決める（`decl_names::fn_group_len`。型検査器の `check_stmts` とコンパイラの `note_fn_groups` が使う）。
  コンパイラは本体の文を回す 4 つのループ（`compile_fn` の本体・`compile_block_body` / `compile_loop_body` / `compile_handler_body`）で控える。
  R-1 の「定義の時点で見えていない捕捉は `NameError`」は、自分の名前に加えて組の相手を除く。
- スレッドへ送る複製（`deep_clone`）は組も複製する（`deep_clone_group`）。
- 最上位の `if` の中の組・自己再帰も VM（`vm_toplevel` がブロックを VM でコンパイルする）なので同じ仕組みで動く。
- 確かめた形（release）: 関数の中の自己再帰（`120`）・相互再帰（`True`）・メソッドの中（`24`）・自己再帰する `gen`（`6`）・
  `gen` と `fn` の組・組の関数が外側の束縛済みのローカルを捕まえる形・**返した関数**（自己再帰 `720`・組の片方だけを返して
  `True False`。参照実装はどちらも `NameError`）・最上位の `if` の中の組。
- `Rc` の循環が無いことの確認: 組と自己再帰を持つ外側の関数を 2 万回 / 20 万回呼んだときの最大メモリが 9.9 / 20.6 MB で、
  再帰しない入れ子の関数だけの同じ形（9.9 / 20.7 MB）と同じ（伸びは組と関係なく元から）。
- ゲート（基準は 3-2 の `d61f14bd` の release・同じセッションで負の対照）: `compare_bytecode -A` 658 / 659（負の対照も 658 / 659。ずれるのは
  毎回変わる async の例題 1 本で、負の対照と A/B で別の例題）・`compare_outputs -A` 640 / 640（負の対照 640 / 640）・`scan_examples` FAIL 0・
  `force_gate` 0 件・`compare_python_impl` 予期しない差 0・`type_obligations` 退行なし・`compare_wasm_frontend` 774 / 774・
  `debug_session` / `repl_session` 同一・`stale_doc_refs` OK・`cargo test`（debug）1302 passed。
  ⇒ 組・自己再帰を書いていない例題（全部）のバイトコードと出力は変わらない。

## 5. あわせて見つけた（本計画の範囲外・起票の候補。どのタスクに入れるかは未定）

### R-1. VM が関数の中でブロックスコープを守らない（未束縛のローカルを `None` / 前の値として読む）

「掬いきれなかった選択肢はエラーにする」に反する。調査は 2026-11-09（`b6e1e2af`・release）。

**✅ 修正済み（2026-11-09・利用者の指示で本計画のフェーズより先に直した）**。下の「直し方の案」のとおり。
- [visibility.rs](../src/vm/compiler/visibility.rs)（新規）: コンパイラが「今見えている名前」を持つ。
  本体が宣言する名前（`block_locals`・入口ごとに、宣言を集めた直後の `slots` から引数・キャプチャを除いたもの）は、
  宣言の文をコンパイルし終えたとき（`make_declared_visible`・列挙は `decl_names::each_declared_name`）に見え、
  そのブロックを抜けると見えなくなる。ブロックの本体はすべて `compile_block_body` / `compile_loop_body`（`for` の変数）/
  `compile_handler_body`（`except` の別名）を通す（`if` / `while` / `for` / `match` / `block:` / `try` / `except` /
  `finally` の複製・各ブロック式・async の本体）。
- 見えていない名前の読み（`Expr::Ident` の腕の先頭・呼び出し先・融合の `as_local`）、代入・複合代入・`freeze`、
  入れ子の関数のキャプチャは、`Op::Fail` で `NameError: '<name>' is not defined`（最上位と同じ文言）。
- Python のモジュールの関数・文には当てない（`compile_fn` / `compile_module_stmt` / `compile_definition_expr` に
  `is_python` を渡す。ジェネレータの値 `GeneratorFnValue` にも `is_python` を足した）。
- ⚠ **入れ子の関数の自己再帰は対象外にした**（下の「残り」）。
- 上の表の形はすべて `NameError` になった（関数の中・最上位の 1 文の中の両方。`gen` の本体・メソッドの本体も）。
  Python のモジュールの例（`3` / `4`）は変わらない。
- 例題: [block_scope.ar](../examples/basics/block_scope.ar)（正しいコードが今までどおり動く）・
  [block_scope_error.ar](../examples/basics/block_scope_error.ar)（実行時 `NameError`。フェーズ 1 の後は静的な誤りになる）。
- ゲート（基準は修正前の `b6e1e2af` の release exe・同じセッションで負の対照を取った）:
  - `compare_bytecode -A`: 652 / 653 一致。残る 1 件（`async_cs_calls.ar`）は負の対照でも同じようにずれる
    （毎回変わる出力）。⇒ **正しいプログラムのバイトコードは 1 命令も変わっていない**。
  - `compare_outputs -A`: 635 / 636 一致（負の対照 636 / 636）。違うのは新しい `block_scope_error.ar` だけ（修正前は `1`）。
  - `compare_import_paths -A`: 13 / 13（負の対照 13 / 13）。`compare_python_impl`: 予期しない差 0
    （`block_scope_error` は出力形式だけが違うので既知の差に足した。`for_target_scope_error` と同じ）。
  - `scan_examples`: FAIL 0（`test_build_gate` ok）。`force_gate`: `VmForceError` 0 件。`debug_session` / `repl_session`: 同一。
  - `cargo test`（debug）: 1302 passed。

**残り**
- **入れ子の関数の自己再帰が動かない**（R-1 の調査中に見つけた・修正の前から）。
  `fn outer(): fn fact(n): .. fact(n - 1) ..; fact(5)` が `TypeError: 'NoneType' object is not callable`（参照実装は `120`）。
  `MakeFn` が自分の名前の枠を、名前を書き込む前に写すため。B3 と同じ形だが、「後で束縛する名前」（D-1）ではなく
  自分の名前なので、静的な誤りにするか動かすかは別に決める（⚠ 要判断）。今回は振る舞いを変えていない。
  ⇒ **動かす**（2026-11-09・利用者の決定）。連続する入れ子の関数の組（フェーズ 3）の 1 つだけの組として扱う。
- ネイティブコンパイル（`--compile`）の関数に同じ漏れがあるかは未確認（この環境に clang が無く、`.arc` は VM で動いた）。
- デバッガの停止時のローカルの表示（`declared_slots`）は、ブロックを抜けた枠も見せたまま（実行の意味は変わらない）。

**仕組み**。関数（`fn` / `gen` / メソッド）の本体は VM にコンパイルされ、ローカルは番号付きの枠
（`STORE_LOCAL` / `LOAD_LOCAL`）に置かれる。

1. 枠の割り当て（[decls.rs `collect_nested_decls`](../src/vm/compiler/decls.rs)）は、本体の宣言を**入れ子のブロックまで
   先に全部**集め、関数全体で 1 つの平らな表（`slots`）にする。ブロックを抜けても名前は表に残り、
   宣言より前の位置からも引ける。
2. 枠はフレームを作るときに `Value::None` で埋まる（[execution.rs](../src/interpreter/functions/execution.rs) の
   `vec![Value::None; chunk.n_locals]`）。`LOAD_LOCAL` は枠が書かれたかどうかを見ない。
3. ブロックを抜けたときに表から名前を外しているのは **`for` の変数だけ**（bug_fix.md B4・
   [stmt.rs `compile_for`](../src/vm/compiler/stmt.rs) の `released_for_targets`。読みは `Op::Fail` で `NameError` にする）。
   それ以外の束縛（`let` / `mut` / `const`・`except .. as` の別名・入れ子の `fn`）には同じ処理が無い。

最上位の文（ツリーウォーク・`vm_toplevel`）はスコープを積むので、仕様どおり `NameError` になる。

**実測**（どれも関数の中。「参照」は `impl_python`。型検査はどれも通る＝フェーズ 1 の A12 の形）

| 形 | VM | 参照 | 何が起きているか |
|---|---|---|---|
| `if True: let y = 1` の後で `print(y)` | `1` | `NameError` | ブロックの外へ漏れる |
| `block: let b = 5` の後で `print(b)` | `5` | `NameError` | 同上 |
| `match` の腕の `let m` を後で読む | `matched` | `NameError` | 同上 |
| ブロック式（`block ->int:`）の中の `let inner` を外で読む | `7` | `NameError` | 同上 |
| `except ValueError as e:` の後で `print(e)` | `a` | `NameError` | 同上（`e` は例外の値） |
| `while False: let wv = 1` の後で `print(wv)` | `None` | `NameError` | 一度も書かれていない枠を読む |
| `if False: let z: int = 1` の後で `z + 1` | `TypeError: unsupported operand types for Add: NoneType and int` | `NameError` | **`int` と注釈した値に `None` が入る**（型の保証が崩れる） |
| ループの本体で、`let y = i * 10` より前に `y` を読む（2 周目） | `10` | `NameError` | **前の周回の値を読む**（正しそうな値が出るので気付けない） |
| `while` の本体で同じ形 | `100` | `NameError` | 同上 |
| 入れ子の関数が、外側で後に束縛するローカルを読む（B2） | `None` | `1` | `MAKE_FN` が、まだ書かれていない枠（`None`）を写す |
| 入れ子の関数どうしの前方参照（B3） | `TypeError: 'NoneType' object is not callable` | `3` | 同上（`b` の枠が `None` のまま写される） |

`gen` の本体・メソッドの本体でも同じ（`if True: let y = ..` の後の `y` が読める）。async の本体は未確認。
ネイティブコンパイル（`--compile`）の関数に同じ漏れがあるかも未確認。

**フェーズ 1 との関係**。フェーズ 1 の後は、Arrow のソースのこれらの形は静的な誤りになり、実行まで届かない
（D-1 = (a) で B2・B3 も）。それでも直すべき理由:
- 型検査の誤りを止めない入口がある（`resolver.rs` の doc によれば REPL は型の誤りを**無視して続行**する）。
- 型検査は「通さない」だけで、実行時の意味がずれたまま残ると、検査の漏れがあったときに黙って誤った値になる
  （今回の `declared_anywhere` がまさにその形だった）。

**⚠ Python のモジュールは今の振る舞いに頼っている**。Python は関数単位のスコープなので、

```python
def f(c):
    if c:
        y = 1
    else:
        y = 2
    for i in range(3):
        last = i
    return y + last
```

が `import[py]` で正しく動く（`3` / `4`。CPython と同じ）のは、VM が `if` / `for` の中の束縛を関数全体で
見せているから。Arrow のブロックスコープをそのまま当てると Python のモジュールが壊れる。
⇒ 直すときは**Arrow の関数だけ**（`Chunk::is_python` が偽のもの）に当てる。

**直し方の案**（実行時の負担が無いもの）。B4 の `released_for_targets` を一般化し、コンパイラが
「今見えている名前」を持つ。名前は宣言の書き込みで見えるようになり、そのブロックを抜けると見えなくなる。
`slots` にあっても見えていない名前の読み（`MAKE_FN` が写す名前を含む）は、B4 と同じく `Op::Fail` で
`NameError` にする。宣言は必ずブロックの頭から順に実行されるので、ソースの位置で決めた見え方が
実行時の見え方と一致する（`if False:` の中の宣言も、その後ろからは見えない）。
- ⚠ リゾルバの注釈（`Resolution::Local`）より先に見え方を判定する（B4 のコメントにある、注釈より優先させる理由と同じ）。
- ⚠ 正しいプログラムのバイトコードは変わらないはず（`Op::Fail` が出るのは誤った読みだけ）。
  ⇒ `compare_bytecode` で全例題の差分 0 を確かめるのが「挙動不変」の主張になる。差分が出た例題は、
  漏れに頼っていた例題なので個別に見る。
- ⚠ デバッガの `declared_slots`（[run.rs](../src/vm/run.rs)・停止時に見せるローカル）も、ブロックを抜けた枠を
  見せないよう揃える（`debug_session` ゲート）。
- 代案: 枠の初期値を「未束縛」の印にし、`LOAD_LOCAL` で見て `NameError` にする。一度も書かれていない枠は捕まるが、
  前の周回の値（上の表のループの 2 行）はブロックを抜けるたびに枠を消す命令が要る。どの読みにも検査が
  1 つ増えるので、案の方を推す。

**ゲート**: `test_build_gate` / `scan_examples` / `force_gate` / `compare_bytecode -A`（差分 0）/ `compare_outputs -A` /
`compare_python_impl` / `debug_session` / `repl_session`。Python のモジュールの例題（`interop/py_*`）が変わらないこと。

### R-3. Python のモジュールで、ループの後に `for` の変数を読むと `NameError`

⇒ **[known_bugs.md](known_bugs.md) の #14 に起票した**（2026-11-09・利用者の指示）。R-1 の調査中に見つけた。CPython ではループの後でも `for` の変数（最後の値）が読める。

```python
def g():
    for i in range(3):
        pass
    return i      # CPython: 2。Arrow（import[py]）: NameError: 'i' is not defined
```

B4 の `for` の変数の解放（R-1 の「仕組み」3）が、Python のモジュールの関数にも当たっているため。
R-1 で「Arrow の関数だけ」に分けるときに、こちらも Python の関数から外せば直る。

### R-2. 未定義の受け手でメソッドを呼ぶと、同じ誤りが 2 回出る

⇒ **2 回出る件は 1-4 で直った**（`already_reported_undefined`）。位置が `(` を指す件は残る。

同じ誤りが 2 回出て、位置が受け手ではなく `(` を指す（最上位の `mh.enable(1)` → `1:10 name 'mh' is not defined`
が 2 行。関数の中でも同じ。利用者のファイルでは `80:18` が 2 行）。

### R-4. import したモジュールの本体の型の誤りが表に出ない・取り込む側の大域が見える（1-1 で見つけた）

- モジュールの本体は `annotate_module_body`（[resolve.rs](../src/type_check/stmt/resolve.rs)）が取り込む側の中で検査し、
  誤りは具体化の分を除いて捨てる。実測: モジュールに `let s: str = 1` と未定義の名前を書いても、`import` した入口の
  検査も実行も何も言わない（そのモジュールを入口にすれば出る）。意図した設計かどうかは記録が見当たらない。
- 本体は**取り込む側の大域スコープの上に積んだ**スコープで検査するので、取り込む側が `import` より前に束縛した名前
  （`let MAIN_G = 1` → `import leakmod`）がモジュールの関数から見える。実行時は `NameError: 'MAIN_G' is not defined`。
- 本計画の規則（モジュールの最上位の束縛）とは別の層なので、フェーズ 1 では扱わない。

### R-5. 関数の中の `static mut` を `local::` で読むと `VmForceError`（1-1 で見つけた）

`fn f(): static mut calls: int = 0; print(local::calls)` が `VmForceError: cannot compile function 'f' to bytecode`
（R-1 の前の exe でも同じ）。`local::` は可変長引数の名前空間で、`static mut` に使うのが誤りなら静的な誤りにすべき。
