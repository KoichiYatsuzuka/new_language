# pandas・numpy を Arrow から使う — Python のコードはすべて CPython で動かす

## 0. 妨げている要因（測定: Arrow `b6e1e2af`・pandas 2.2.0・numpy 1.26.4・CPython 3.12.2・2026-11-01〜02）

pandas を `import[py-int]`（組み込みの CPython）で読めば、`read_csv`・DataFrame の作成・表示・集計は今でも動く（6.5）。
残っているのは型と、CPython の物が Arrow との境界を越えるときの振る舞い・コスト。「対処」は本書のタスク。

| # | 要因 | 対処 |
|---|---|---|
| 1 | Python のソースを変換して動かす `import[py]` では pandas を読めない（C 拡張 42 本・numpy・標準ライブラリ・変換器が拒む構文）。直しても Arrow 側に Python 専用の特例が増え続ける | D-1・[3] |
| ~~2~~ | ✅ pandas の名前がすべて `Any` だった件（直した: 1-3。`pd.DataFrame` はクラス・`pd.read_csv(path)` は `DataFrame`・`df.head()` は `DataFrame`）。⚠ 存在しない名前は今も `Any`（Python のモジュールは実行時に名前が増えうる） | [1-3] |
| ~~3~~ | ✅ `.py` の無いモジュール（`sys`・`itertools`・`_collections`・`select` など）を `import[py-int]` できなかった件（直した: 1-4。CPython の内省で作ったスタブを同梱）。⚠ 戻り値の型は `Any`（`__text_signature__` に型が無い） | [1-4] |
| ~~4~~ | ✅ ユーザーの site-packages を型の探索先にしなかった件（直した: 1-5） | [1-5] |
| ~~5~~ | ✅ 参照の数（直した: 1-1）。以前は Arrow の束縛がいくつあっても CPython の参照は 1 つで、変数への `df["c"] = ..` を pandas が連鎖代入と誤判定した | D-5・[1-1] |
| ~~6~~ | ✅ Python の例外が `ForeignError` になり、`except IndexError:` などで捕まらなかった件（直した: 2-1） | [2-1] |
| 7 | 真偽: numpy の配列の `if a > 2:` が `ValueError` にならず真として通る | [2-2] |
| 8 | 切り出し `a[1:3]` が `TypeError`。`...`（`Ellipsis`）を渡せない | [2-3] |
| 9 | `for` が CPython の反復子から先に全部取り出す（無限の列で止まらない） | [2-4] |
| 10 | Arrow の関数・インスタンスを Python へ渡せない（`df.apply(f)`・`lib.map_infer(arr, f)` が `TypeError`） | [2-5]・[2-6] |
| 11 | list / dict / set は境界で写す。Python 側の書き換えが戻らない（`heapq.heappush(h, 0)` の後も `h` は元のまま） | [2-7] |
| ~~12~~ | ✅ Python の物を `let` で受けても別名から中身が変わる件（直した: 1-2。`mut` で受ける規則にした） | D-3・[1-2] |
| 13 | `is` が `==` を呼ぶ／同じ物どうしが `False` | D-4・[5-1] |
| 14 | コスト: 越境 1 回ごとに +0.5〜1.7 µs。リストは Arrow → Python が 20〜50 ns/要素、Python → Arrow が約 110 ns/要素 | [4] |
| ~~15~~ | ✅ CPython の物・モジュールの属性へ代入できなかった件（直した: 2-9。`df.columns = [..]`・`gm.count = 5`） | [2-9] |

測定の手順と生データは 6 節。

---

## 1. 方針（2026-11-02 に決定）

- **D-1** Python のコードは**すべて CPython で実行**し、Arrow とは値をやり取りする。
  - 理由: Python のソースを Arrow へ変換して直接実行する方式は、Python の資源（C 拡張・標準ライブラリ・動的な仕掛け）を
    十全に扱えず、Arrow 側に Python 専用の例外（`running_python` の分岐・属性の操作の上書き・`object()` など）を足し続ける
    だけになっていた。
  - 旧計画（変換して読む方式。シグネチャだけの AST・遅延 import）は `4bad8c8e` までの本書にある。
- **D-2** 型を**推論できるときはその型で受ける**（`.pyi`・注釈・コンストラクタ。再エクスポートも定義元までたどる）。
  推論できないときは `Any`（今の規則・ffi_value_exchange_plan.md 7-27）。
- **D-3** Python から受け取ったオブジェクトは**すべて `mut` で受ける**（`let` / `const` / `freeze` では受けられない。関数の
  引数も `mut d: pd.DataFrame` と書く・2026-11-02 に確定）。CPython の物は参照を共有するので、Arrow の `let` の
  「変わらない」を保証できない（0 節の 12）。
- **D-4** `is` の修正は後で行う（フェーズ 5）。
- **D-6** `import[py]` は従来の `import[py-int]` と同じ意味にする（CPython で実行する）。**`import[py-int]` は廃止する**
  （2026-11-02 に決定）。
- **D-7** Python の翻訳に固有のコードは**すべて消す**（変換器と、変換したコードのための仕組み・2026-11-02 に決定）。
- **D-8** Python から来た list / dict / set は**写さずにそのまま使う**（2026-11-04 に決定・4 節の 3・2.6）。
- **D-9** Arrow の list / dict / set・インスタンスは、**Python からそのまま使えるように渡す**（包む・2026-11-04 に決定・4 節の 4・2.6）。
- **D-10** 型を書いたスタブ（typeshed）は **`.py` の無いモジュールの分だけ同梱**し、ほかは同梱しない（2026-11-04 に決定・4 節の 7）。
- **D-11** 暗黙の束縛（`for` の変数・内包の変数・`except E as e`・`match` の捕捉）も D-3 に従う: 不変の束縛で Python の物を受けたら
  誤りにし、`mut` で受ける書き方（`for mut f in ..` など）を足す（2026-11-04 に決定・4 節の 6 の ①）。
- **D-5** 参照の数は **Arrow の値の写し 1 つ ＝ CPython の参照 1 つ**にする（2.1）。決まった形の参照の数が CPython と同じで
  あることを試験で守る。
  - 採らなかった案:
    - ① 持ち主ごとに 1 つ ＋ 呼び出しの間に 1 つ: 変数と一時的な物がどちらも 4 になり、連鎖代入を見分けられない（6.4）。
    - ② 連鎖を CPython の補助関数の中で評価する: 補助関数の仮引数の分だけ参照が増え、`a.resize(5)` が誤りになる
      （CPython でも補助関数を通すと誤り・6.4）。
  - **メソッド・関数の呼び出しは CPython の肩代わりの関数で行う**（2026-11-02 に決定・2.1）。C の API から呼ぶと
    呼ばれた側のフレームが参照を 1 つ増やすので、写し方を直すだけでは呼び出しの数が CPython より常に 1 つ多い。

---

## 2. 設計

### 2.1 参照の数（D-5）

- ✅ **済**（1-1）。`Value::PyObject` を写すと CPython の参照を 1 つ増やし、捨てると 1 つ減らす
  （`PyObjHandle` の `Clone` / `Drop`・[py_interop.rs](../src/interpreter/py_interop.rs)）。束縛・欄・コレクションの要素・
  VM のスタックに積んだ写しが、それぞれ 1 つの参照を持つ。以前は `Arc<PyObjHandle>` で 1 つの参照を共有していた。
- これで CPython の数え方とそろう: 変数の形は「束縛 1 ＋ スタックの写し 1」、一時的な物は「スタックの値 1」だけ。
- **GIL**: PyO3 0.22 の `Py::clone` は GIL が無いと panic する。Arrow は格納のたびに値を写すので、そのたびに GIL を
  取らず、増減を待ち行列に積んで、CPython へ入る唯一の入口 `with_py` で反映する（増やす分を先に）。CPython が数を
  見られるのは入口の中だけなので、見える数は正確になる。
- **呼び出しの肩代わり**: C の API（`PyObject_Call` など）から Python の関数を呼ぶと、呼ばれた側のフレームが参照を 1 つ
  増やす（CPython のバイトコードは評価のスタックの参照を引き渡す）。⇒ 呼び出しは
  `lambda __arrow_box__: __arrow_box__.pop().NAME(__arrow_box__.pop(), K=__arrow_box__.pop())` の形の小さな関数に
  行わせ、Arrow は自分の参照をリストに入れて手放す。形（メソッド名・位置引数の数・キーワードの名前）ごとに 1 度だけ作る。
  ソースに書けない名前（予約語のキーワードなど）のときだけ肩代わりを使わない（そのときは 1 つ多い）。
- **添字の読み書き**は C の API（`PyObject_GetItem` / `PyObject_SetItem`）を直接呼び、受け手・鍵・値を借りるだけにする
  （CPython の `x[k] = v` も C の入口を通るので、これで同じ数になる）。
- 等しさとハッシュは CPython のオブジェクトの番地で決める（写しごとに包みが別物になったため）。同じ物を 2 回受け取っても
  同じ値になる。
- ⚠⚠ インタプリタの内部（Rust のコード）で余分に写すと、その数だけ増える。本物の連鎖代入が変数に見えて警告が消え、
  `resize` が誤りになる。⇒ 次の形の参照の数・振る舞いが CPython と同じことを**試験で守る**（ゲート）:

  | 形 | CPython |
  |---|---|
  | 変数に入った物への `p["k"] = 1`（`__setitem__` の中の `sys.getrefcount(self)`） | 4 |
  | 一時的な物への `make()["k"] = 1` | 3 |
  | 束縛を 3 つ・リストの要素を 2 つ足す | 束縛・要素の数だけ増える |
  | `a = np.arange(3); a.resize(5)` | 通る |
  | pandas: 変数への `df["c"] = ..` ／ `df["a"][0] = ..` | 警告なし ／ `ChainedAssignmentError` |
  | numpy: Arrow が持つ大きな配列 `t` の `u = t + 1` | `t` は変わらない |

- 試験: [examples/interop/py_int_refcount.ar](../examples/interop/py_int_refcount.ar)（ゲート対象。上の表の
  `__setitem__`・束縛を足す形に加え、変数・一時的な物のメソッドと関数の引数を、CPython で同じ形を走らせた数と比べる）と
  [examples/practical_examples/pd_numpy_pyplot/refcount_pandas.ar](../examples/practical_examples/pd_numpy_pyplot/refcount_pandas.ar)
  （ゲート対象外・pandas と numpy の警告と `resize` を CPython と比べる）。どちらも直す前の exe では誤りになる。
- 速さ（6.6）: pandas・numpy の操作はノイズの幅。1 要素を読む `arr[k]` だけ +0.08 µs（待ち行列に積む分）。

### 2.2 `mut` で受ける（D-3）

- 静的: 束縛する式の型が Python の型（Python のモジュールのクラス）なら、`let` / `const` / `freeze` と、`mut` でない関数の
  引数は誤り。
- 実行時: `Value::PyObject` を不変の束縛（`let` / `const` / `mut` でない引数）にしたら誤り（型が `Any` のときもここで止まる）。
- 境界で Arrow の値に変わる物（`int` / `str` / list など）は、今は Arrow の値なので `let` で受けられる
  （⚠ 4 節の 3 で包みのまま持つと決めたら、list などは対象に入る）。
- ⚠ 暗黙の束縛（`for` の変数・内包の変数・`except E as e`・`with .. as x`・`match` の束縛）は 4 節の 6 で決める。1-2 では
  明示の束縛と関数の引数だけを対象にする。

### 2.3 型（D-2）

- `import[py-int]` の型の抽出が、`from pandas.core.api import DataFrame` のような再エクスポートを**定義元のファイルまで
  たどる**。たどれない名前は `Any`。
  - ⚠ 今はたどらないので、`pd` のメンバーは存在しない名前まで全部 `Any` になる（6.5）。
- `@overload` のある関数（`read_csv` は `iterator` / `chunksize` で戻り値が `DataFrame` か `TextFileReader` に変わる）は、
  引数の型・リテラルで選ぶ。選べなければ Union。
  - ✅ 1-3 で実装: 形は本体の宣言の本体の中に入れ子で持たせ（モジュールの関数を増やさない）、型検査は `m.f(..)` の呼び出しで
    最初に当てはまる形を選ぶ（位置引数の数・キーワードの名前・省略できない引数・`Literal[X]` の値・`None`）。⚠ `from m import f`
    して `f(..)` と直接呼ぶ形はまだ選ばない（本体の宣言の型）。
- built-in のモジュールのスタブは Arrow に同梱する（`src/py_stubs.rs` に足す。CPython の `inspect.signature` /
  `__text_signature__` から作れる部分は生成する）。
  - ✅ 1-4 で実装: built-in と標準ライブラリの拡張モジュール 84 本を内省で生成（戻り値は `Any`）。import をたどる先にもなる
    （`collections.deque` は `_collections` のスタブ）。注釈のある大域変数も型を持つ（`.py` / `.pyi` とも）。

### 2.4 境界の振る舞いを CPython に合わせる

| 項目 | 今 | 直した後 |
|---|---|---|
| 例外 | `ForeignError`（`message` に `ValueError: ..`） | 同じ名前の Arrow の例外（組み込みは CPython 3.12 の階層を Arrow が持っている）。Python で定義した例外は `except m.Err:` で捕まる |
| 真偽（`if` / `while` / `not` / `and` / `or` / `bool()`） | 常に真 | ✅ `__bool__` / `__len__` を呼ぶ（2-2） |
| 切り出し・`...` | `TypeError` | Python の `slice` / タプルで渡す（`a[1:, 0]`・`df.loc[:, "a"]`）。`Ellipsis` / `NotImplemented` を対応させる |
| `for` | 先に全部取り出す | ✅ 1 つずつ取り出す（2-4） |
| Arrow の関数を渡す | `TypeError` | Python から呼べる物として渡す。中で起きた Arrow の例外は Python の例外になる |
| Arrow のインスタンスを渡す | `TypeError` | ⏸ 保留（4 節の 4） |
| list / dict / set | 境界で写す（書き換えが戻らない） | ⏸ 保留（4 節の 3・2.6） |

### 2.5 `.py` を CPython で実行する（D-1・D-6・D-7）

- `import[py]` は CPython が import して実行する（従来の `import[py-int]` の実行の道）。型は従来の `import[py-int]` と同じ
  抽出器（2.3）。
- `import[py-int]` は廃止: 書くと構文解析の誤りにし、`import[py]` に書き換えるよう促す。`import[py-int]` を使う例題 61 本
  （`interop` 31・`bench` 16・`bench/speed_modes` 4・`apps` 2・`async` 1・`basics` 1・`interop/py_subdir` 1・
  `practical_examples` 5）を `import[py]` に書き換える。
- 変換器（`python_converter`）と、変換したコードのための Python 専用の仕組みは**すべて消す**（D-7）。
  - 仕組みの例: `running_python` / `in_python_module`、格納で写さない規則、属性の操作の上書き、`object()`、クラスを返す関数、
    prelude の `__py_*`。
  - ⚠ `.pyi` の型の抽出は今 `python_converter` の結果を使う（`py_stub_extract.rs` の冒頭）ので、そこは抽出器（2.3）へ
    移してから消す。

### 2.6 写さずに使う（4 節の 3・4 の検討・2026-11-02）

**Python → Arrow（4 節の 3）**

| 種類 | 今 | 写さずに使う案 | 検討 |
|---|---|---|---|
| `int`（i64 に入る）・`float`・`bool`・`None` | Arrow の値に写す | — | 両側とも不変なので、写しても共有と区別がつかない（意味は同じ）。写すのは数バイト。⇒ **今のままでよい** |
| `str`・`bytes` | Arrow の文字列に写す（UTF-8） | CPython が持つ UTF-8 の写し（`PyUnicode_AsUTF8AndSize`）を借りる | 両側とも不変なので意味は同じで、違いは速さだけ（10 万文字で 6.9 µs）。Arrow の文字列に「CPython から借りた」形を足すことになる。⇒ 必要なら速さのタスクとして後で |
| list / dict / set / bytearray | Arrow の値に写す（書き換えが互いに届かない） | **包みのまま持ち**、Arrow の list などの操作（添字・`len`・反復・メソッド・`in`・`+`・切り出し・表示）を CPython の API で行う | 共有されるので CPython と同じになる。⚠ 判断が要る（下の ①〜③） |
| numpy の配列 | 包みのまま（今も写さない） | `tolist()` などで Arrow のリストにするときにバッファを直接読む | 速さのタスク（4-3） |

判断が要ること（4 節の 3）:
- ① Python の list を `list[int]` の型で受けたとき、要素の型をどこで確かめるか（読むたび／受け取るときに全要素を 1 度／確かめずに
  要素を `Any` にする）。Python の list には何でも入るので、静的な型は保証にならない。
- ② Arrow の list の操作と Python の list の操作が違うもの（`+` の型の規則・`==` の比べ方・`sort` の引数・Arrow にしか無い
  メソッド）を、どちらの意味にするか。
- ③ 速さ: 包みのままだと要素の操作が 1 回ごとに越境する（6.3 の目安で +0.5〜1.7 µs）。VM の list 向けの特化も効かない。

⇒ D-8（そのまま使う）での進め方（2026-11-04・2-7 で実装するときの案。違えば直す）:
- ① 要素は**読むたびに**宣言の型と照合する（`FfiTypeError`）。共有されるので、受け取るときに 1 度確かめても後で Python が別の型を
  入れうる。注釈の無い受け取りは要素が `Any`。
- ② 操作は**Python の意味**（中身は Python の list なので CPython の API を呼ぶ・D-1）。Arrow にしか無い操作は誤り（黙って Arrow の
  意味にしない）。
- ③ 速さは測ってから直す（フェーズ 4）。

**Arrow → Python（4 節の 4）**

| 種類 | 今 | 写さずに使う案 | 検討 |
|---|---|---|---|
| `int`・`float`・`str` など | Python の値に写す | — | 不変なので今のままでよい |
| list / dict / set | Python の値に深く写す | Arrow の list を包む Python の型（PyO3 の `#[pyclass]`）を渡し、Python から読み書きすると Arrow の list を直接触る | ⚠ 判断が要る（下の ④〜⑥） |
| Arrow のインスタンス | `TypeError`（渡せない） | Arrow のインスタンスを包む Python の型を渡し、属性・メソッドを Arrow へ中継する（2-5 の呼び戻しの上に作る） | ⚠ 判断が要る（下の ④〜⑥） |

判断が要ること（4 節の 4）:
- ④ 包んだ物は Python から見て `list` / `dict` ではない（`isinstance(x, list)` が偽。C 拡張の速い道（`PyList_Check`）にも乗らない）。
  `type(x) is list` で分けるコードや、`list` しか受けない C 拡張では振る舞いが変わる。それでよいか、受け取る側によって写すか。
- ⑤ Arrow の `let` の値を包んで渡すと、Python から書き換えられる（`let` の保証が崩れる）。`mut` の値だけを包む／`let` は読むだけの
  包みにする／`let` は今どおり写す、のどれにするか。
- ⑥ Python が包みを持ち続けたまま別の Python のスレッドで使うと、Arrow の値（`Rc`・スレッドをまたげない）に別のスレッドから
  触ることになる。そのときは誤りにするか（包みがスレッドを確かめる）。

⇒ D-9（そのまま使えるように渡す）での進め方（2026-11-04・2-6 で実装するときの案。違えば直す）:
- ④ 包みは `list` / `dict` ではないが、`collections.abc` の `MutableSequence` / `MutableMapping` / `MutableSet` に登録して
  反復・添字・`len` に答える（numpy・pandas が受ける「list のような物」になる）。`list` しか受けない C 拡張へは写して渡す。
- ⑤ `let` の値は**読むだけの包み**（Python からの書き換えは `TypeError`）、`mut` の値は書ける包み（書き換えが Arrow へ届く）。
- ⑥ 包みは作ったスレッドを覚え、別のスレッドから触られたら Python の `RuntimeError`。

---

## 3. タスク

各タスクで、例題（誤りの形は `_error` の例題も）を足し、次のゲートを走らせる:
`test_build_gate` / `scan_examples` / `force_gate` / `compare_python_impl` / `compare_import_paths` / `compare_wasm_frontend`
（wasm を先に作り直す）。型検査を触ったら `type_obligations`、識別子を改名・削除したら `stale_doc_refs` も走らせる。
src のファイルを作ったら codebase map を再生成し、型検査・構文解析・インタプリタを触ったら VSIX を作り直す。
値は CPython と突き合わせる（例題は期待と違えば `raise` する形で書く）。

### フェーズ 1: 値の持ち方と型

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| ~~1-1~~ | ✅ **済**（2026-11-02・2.1）参照の数（D-5）。`Value::PyObject` の写し 1 つが CPython の参照 1 つ（GIL が無ければ待ち行列で `with_py` が反映）。呼び出しは肩代わりの関数、添字は C の API で借りるだけ。等しさ・ハッシュはオブジェクトの番地。試験は `py_int_refcount.ar`（ゲート）・`refcount_pandas.ar` | なし | 中 |
| ~~1-2~~ | ✅ **済**（2026-11-02・2.2）Python の物を `let` / `const` / `freeze` / `mut` でない関数の引数で受けたら誤り（D-3）。静的: CPython のクラスのインスタンス（とそれを持つ list など）なら `ImmutablePythonObject`（`TypeRegistry::cpython_class_names`）。実行時: `bind_copy_held` の `CopyFreeze`（コレクションの中も見る）と引数の束縛（値そのものだけ・前置きの生成器を除く）で `TypeError`。例題は `py_int_mut_binding.ar`・`_error`・`_runtime_error`。既存の例題 8 本を `mut` に直した。暗黙の束縛は 4 節の 6 | なし | 中 |
| ~~1-3~~ | ✅ **済**（2026-11-03・2.3）import をたどって型を付ける: ① 再エクスポート ② 注釈のよそのクラス（`TYPE_CHECKING` の中も） ③ ほかのファイルの基底のメンバー（`Self` は派生）④ `@overload` は呼び出しで最初に当てはまる形を選ぶ。import の表は行単位で読む（`py_import_links`）。`x is m.C` は CPython の `isinstance`。ついでに: 引数の多い関数（約 40 行を超えるシグネチャ）が型スタブから落ちていた件を直した。例題は `py_int_links.ar`・`_error`。速さは 6.7 | なし | 大 |
| ~~1-4~~ | ✅ **済**（2026-11-03・2.3）`.py` の無いモジュール（built-in 67 本と標準ライブラリの拡張モジュール。手で書いた `math` / `time` を除く 84 本）のスタブを CPython の内省で作って同梱した（`scripts/gen_builtin_py_stubs.ps1` → `stubs/generated/`・`src/py_stubs_generated.rs`）。引数は `__text_signature__`、大域変数は値の型、ほかは `Any`。import をたどる先（`_collections` の `deque`）にもなる。ついでに: 注釈のある大域変数（`count: int`）を型スタブに入れた（以前はどれも `Any`）。モジュールの大域を import の時点の写し（Python のオブジェクトのまま）で返していたのを、読むたびに `getattr` して Arrow の値にするよう直した（`NsLive::Python`）。例題は `py_int_builtin_modules.ar`・`py_int_module_globals.ar`・各 `_error`。⚠ 型を書いたスタブ（typeshed）を使うかは 4 節の 7 | なし | 中 |
| ~~1-6~~ | ✅ **済**（2026-11-04・D-10）`.py` の無いモジュール 84 本のうち typeshed にある 56 本のスタブを typeshed から作った（Pylance が同梱する typeshed-fallback・commit `289e5d35`）。`scripts/typeshed_flatten.py` が、作った CPython の版と OS の分岐だけを残し、型変数を `Any`・型の別名を中身・`X: Final = 8` を `X: int`・メソッドの `@overload` を 1 つ・`__new__` を `__init__` にする。無い 28 本は内省のまま。`LiteralString` は `str`。同梱のスタブは変換器を通さず行単位の抽出器だけで読む（単体テスト `bundled_stubs_lose_no_declarations`）。ライセンス（Apache-2.0）は `stubs/generated/LICENSE-typeshed.txt`・VSIX・配布先の `third_party/`。例題 `py_int_builtin_modules.ar`・`_error` を戻り値の型つきに直した | 1-4 | 中 |
| ~~1-5~~ | ✅ **済**（2026-11-03）ユーザーの site-packages を型の探索先に足した（`import_fs::python_lib_dirs` と拡張の `queryPythonDirs` を同じ問い合わせに・CPython の `sys.path` と同じく、使えるときだけ標準ライブラリの後・site-packages の前）。`import[py-int] dateutil.parser` が読める。試験は `import_fs` の単体テスト | なし | 小 |

### フェーズ 2: 境界の振る舞いを CPython に合わせる（2.4）

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| ~~2-1~~ | ✅ **済**（2026-11-04）Python が投げた組み込みの例外は同じ名前の Arrow の例外（`args` は Python の `args`）。Python で定義した例外は CPython のオブジェクトのまま `except m.Err:` と継承した組み込みの例外で捕まる（CPython の `isinstance`）・`raise m.Err(..)` で Arrow から投げられる・捕まえなければ CPython と同じ最後の行。型スタブのクラスは継承をたどって組み込みの例外を基底に持つ（`except m.Err:` の型検査）。ついでに: `print(x)` / `str(x)` を CPython の `str`（以前は `repr`）にした。例題は `py_int_exceptions.ar`・`_error`、`py_int_foreign_error.ar` を直した。⚠ `except ValueError as e` で Python で定義した例外を受けると、`e` の型は `ValueError` だが値は CPython の物（`e.message` は無い・`str(e)` / `e.args` を使う） | なし | 中 |
| ~~2-2~~ | ✅ **済**（2026-11-04）CPython の物の真偽（`bool()`・`not`・`and` / `or`・型の分からない値の `if` / `while`）は CPython の `PyObject_IsTrue`（`__bool__`、無ければ `__len__`・`eval_truthy` → `py_truthy`）。`__bool__` が投げた例外は Python の例外（2-1）。ついでに: 単項の `-` / `~` も CPython（`py_unary`・以前は `bad operand type`）、`print(.., flush=x)` も同じ真偽。条件が bool でなければならないのは同じ（型が CPython のクラスと分かる `if df:` は `ConditionNotBool`）。例題は `py_int_truthiness.ar`・`_error`・`_runtime_error` | 2-1 | 小 |
| 2-3 | 切り出し・多次元の添字・`Ellipsis` / `NotImplemented` を Python へ渡す | なし | 小〜中 |
| ~~2-4~~ | ✅ **済**（2026-11-04）`for x in <CPython の物>` は `iter(obj)` の反復子から取り出すたびに 1 つ取る（`GeneratorState::py_iter`・`gen_next` が `PyIter_Next`）。`break` すれば先は取り出さず、途中の例外はそこまでの本体のあとで届く。`Generator` の形のままなので、内包・`list()`・`any()` / `all()`・分解・ネイティブの関数の `for` も同じ道。取り出し途中の反復子は写せない（`deep_clone` は中断中の生成器と同じく印を付ける）。1 要素あたり +0.12 µs（30 万要素の `range` と生成器・負の対照は ±1 ms）。例題は `py_int_lazy_for.ar` | なし | 小 |
| 2-5 | Arrow の関数（クロージャ・束縛メソッド）を Python から呼べる物として渡す。中の Arrow の例外は Python の例外に、Python の例外は 2-1 で受ける。⚠ Python からの呼び戻し中に Arrow の VM へ入り直す（再入）ので、GIL と借用の扱いを先に設計する | 2-1 | 大 |
| 2-6 | Arrow の list / dict / set・インスタンスを包んで Python へ渡す（D-9・2.6 の ④〜⑥） | 2-5 | 中〜大 |
| 2-7 | Python の list / dict / set を写さずに包みのまま使う（D-8・2.6 の ①〜③） | なし | 中〜大 |
| 2-8 | Python の物を `async` のタスクへ渡したときの規則を決めて実装する（未測定。スレッドと GIL） | 1-1 | 中 |
| ~~2-10~~ | ✅ **済**（2026-11-04）`for mut x in ..`・`[.. for mut x in ..]`・`except E as mut e` を足した（AST の `mut_targets` / `name_mut`）。型で Python の物と分かれば静的な `ImmutablePythonObject`、分からなければ実行時の `TypeError`（VM: 1 つの変数は `ForIter` が要素が Python の物のときだけ副表 `implicit_for_targets` を引く・分解と `except` は新しい命令 `StoreLocalImplicit`・`for mut` は `StoreLocalDeepCopy` で写す）。呼んだ側の大域で走る組み込みの実装（`enumerate` など）の中は検査しない（`Chunk::builtin_prelude`）。ネイティブの関数は `for mut` を載せない。例題 `basics/for_mut.ar`・`py_int_implicit_bindings.ar`・`_error`・`_runtime_error`、`py_int_exceptions.ar` を `as mut` に直した。以下は起票時の内容: 暗黙の束縛で Python の物を受ける規則（D-11）。`for mut f in ..`（`for mut k, v in ..` はすべて `mut`）・内包の `for mut f in ..`・`except E as mut e`・`match` の捕捉に `mut` を書けるようにする。不変の束縛で Python の物を受けたら、型で分かれば静的な誤り（`ImmutablePythonObject`）、分からなければ束縛するときに実行時の `TypeError`。⚠ 外の束縛が `mut` でも Python の物を受けるなら `mut` と書く（受け継ぎは実行時に判定できない。VM の slot は可変かどうかを持たない）。Arrow の値を `mut` で受けたら `mut` の宣言と同じく写す。`match` は新しい名前を束縛しない（`case <式>` / `is <型>`）ので対象外 | 1-2 | 中 |
| ~~2-9~~ | ✅ **済**（2026-11-03）CPython の物とモジュールの属性への代入（`df.columns = [..]`・`gm.count = 5`）は CPython の `setattr`（`py_setattr`・受け手と値は借りるだけで、`__setattr__` の中の参照の数は CPython と同じ）。型スタブに型のある大域変数への代入は静的に検査する（属性は従来どおり）。例題は `py_int_attr_assign.ar`・`_error`。⚠ 削除（`del o.x`）は Arrow に構文が無いので対象外 | なし | 小 |

### フェーズ 3: `.py` を CPython で実行する（2.5）

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| 3-1 | `import[py]` を CPython の実行に切り替え、`import[py-int]` を廃止する（書くと誤り・D-6・2.5）。型は従来の `import[py-int]` と同じ抽出器 | 1-3・2-1〜2-5 | 中 |
| 3-2 | 例題の移行: `import[py]` を使う 114 本（`interop` 106・`classes` 4・`collections` 3・`interop/py_subdir` 1）を CPython の出力と突き合わせて直す。`import[py-int]` を使う 61 本を `import[py]` に書き換える。変換器の誤りを確かめる `_error` の例題は消す | 3-1 | 大 |
| 3-3 | Python の翻訳に固有のコードをすべて消す（D-7・2.5）。文書（`language-differences.md` の Python の節・`docs/grammar/09_imports.md`・skills）も直す | 3-2 | 大 |

### フェーズ 4: 速さ（計測してから）

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| 4-1 | Python → Arrow の変換（約 110 ns/要素）を測って直す。原因の候補は `py_to_tl` が要素ごとに型の名前の文字列を作ること。⚠ D-8 で list などは写さなくなるので、2-7 の後に残る変換（`tolist()` など明示の写し・要素を読むときの変換）を測る | 2-7 | 小〜中 |
| 4-2 | 越境 1 回ごとの GIL の取得（+0.5〜1.7 µs）をまとめられるか測る | 1-1 | 中 |
| 4-3 | numpy の数値の配列を Arrow のリストにするとき、バッファを直接読む | 4-1 | 中 |

### フェーズ 5: `is`（D-4）

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| 5-1 | CPython の物どうしの `is` を同一性にする（同じ物なら真。片方だけ CPython の物なら偽） | 1-1 | 小 |

---

## 4. 決定と未決

決定（2026-11-02）:
1. ~~`import[py]` の扱い~~ ⇒ `import[py]` を従来の `import[py-int]` と同じ意味にし、`import[py-int]` は廃止する（D-6）。
2. ~~消す範囲~~ ⇒ Python の翻訳に固有のコードはすべて消す（D-7）。
5. ~~関数の引数~~ ⇒ Python から受け取ったオブジェクトはすべて `mut` で受ける。関数の引数も同じ（D-3）。

決定（2026-11-04）:
3. ~~Python から来たコンテナを写さずに使うか~~ ⇒ そのまま使う（D-8）。①〜③の進め方は 2.6。
4. ~~Arrow の値を写さずに Python へ渡すか~~ ⇒ そのまま使えるように渡す（D-9）。④〜⑥の進め方は 2.6。
7. ~~型を書いたスタブ（typeshed）を同梱するか~~ ⇒ `.py` の無いモジュールの分だけ同梱する（D-10・1-6）。

6. ~~暗黙の束縛~~ ⇒ ①誤りにして `mut` で受ける書き方を足す（D-11・2-10）。⚠ 例（2026-11-04 に実測）: 関数の戻り値を
   `for f in m.chunks():` で回すと `f` は不変で `f["x"] = 1` が静的な誤り、`mut` の変数を回すと通る、型の分からない
   `for chunk in pd.read_csv(.., chunksize=2):` は検査されずに通る、`except m.Err as e: e.retries = 3` も通る、と束縛と型によって割れていた。
   不変のはずの `f` もメソッド（`f.add(1)`）では中身が変わる。

（以下は決定前の記録）
6. **暗黙の束縛**（1-2・D-3）: `for` の変数・内包の変数・`except E as e`・`with .. as x`・`match` の束縛は、今どれも不変で、
   `mut` にする書き方が無い（`for mut x in ..` は無い）。Python の物を受けたとき ①誤りにして新しい書き方（`for mut x in ..`
   など）を足す ②暗黙の束縛だけは Python の物でも不変のまま許す ③暗黙の束縛は Python の物なら自動で `mut` にする、のどれにするか。
   ⇒ 実際に問題になるコードの例を次の節目で示してから決める（2026-11-04 の指示）。

---

## 5. 対象外

- C 拡張を Arrow が直接読むこと。`PyInit` が返すのはモジュールの定義で、関数は CPython の上でモジュールを初期化した後に
  作られる。関数の本体は CPython の API を呼ぶので、CPython なしには動かない（6.1）。

---

## 6. 測定の記録（2026-11-01〜02・Arrow `b6e1e2af`）

### 6.1 C 拡張の実態

- pandas・numpy の `.pyd` 63 本を `llvm-readobj` で調べた:
  - 58 本は `PyInit_<名前>` だけを書き出す。残りの 5 本が書き出すのは、C の構造体へのポインタや `PyObject*` を取る内部の関数
  - 63 本すべてが `python312.dll` に依存する（`pandas._libs.lib` だけで CPython の API を 219 個使う）
- `ctypes` で読んだ結果:
  - `PyInit_lib` は `PyModuleDef` を返す（初期化を 2 段に分ける方式・PEP 489）
  - 関数の表（`m_methods`）は 0 件で、スロットは `Py_mod_create` / `Py_mod_exec` だけ
  - 公開されている 68 個の関数は、実行のスロットが作る
  - 作った後の関数は `FASTCALL|KEYWORDS`（`PyObject* f(PyObject* self, PyObject* const* args, Py_ssize_t nargs, PyObject* kwnames)`）
- メモリ上の形（3.12 に固有）:
  - `float` は `double` が 1 つ
  - `int` は `lv_tag` ＋ 30 ビットずつの桁
  - `str` は文字ごとに 1 / 2 / 4 バイト（UTF-8 ではない）
  - list / tuple は `PyObject*` の配列、dict は内部のハッシュ表
  - numpy の配列は `data` が生のバッファを指す
- ⇒ 読めても、呼ぶには CPython が要る。

### 6.2 境界で正しく使えるか

方法: Arrow で numpy・`pandas._libs.lib`・`heapq` を `import[py-int]` し、Python のコードへ渡して CPython と突き合わせた。

- 同じだったもの: 属性・メソッド・演算子（両向き）・比較・`len`・反復・numpy のスカラーを int として使う・リストを numpy へ渡す。
- 違ったもの: 切り出し、`is`、真偽、例外、関数・インスタンスを渡す、渡したリストの書き換え（0 節の 6〜11・13）。
- pandas の中心のクラス 10 個（`Block`・`BlockManager`・インデクサ・配列など）は C 拡張の型を基底にしている。
  ⇒ Python のコードを一部だけ変換する方式は成り立たない（D-1 の理由の 1 つ）。

### 6.3 越境のコスト（1 回あたり。同じ操作を CPython だけで行った値と比べた）

| 操作 | CPython | Arrow → py-int |
|---|---|---|
| ループ 1 周（参考・境界と無関係） | 0.05 µs | 0.48 µs |
| `df.describe()` | 1,698 µs | 1,807 µs |
| `pd.DataFrame({..})` 2 列 × 1,000 ／ × 100,000 | 332 µs ／ 27.6 ms | 411 µs ／ 37.7 ms |
| `np.array(<Arrow のリスト 1,000>)` | 36 µs | 57 µs |
| `df["a"].sum()` | 9.7 µs | 13.1 µs |
| `arr * 2 + 1`（1,000 要素） | 2.8 µs | 4.4 µs |
| `df.at[5, "a"]` ／ `col.iloc[k]` | 2.2 ／ 2.7 µs | 3.9 ／ 4.4 µs |
| `for row in df.itertuples()`（1 行） | 0.81 µs | 2.4 µs |
| `df["a"].tolist()` 1,000 ／ 100,000 を Arrow のリストへ | 13.5 µs ／ 1.9 ms | 126 µs ／ 13.9 ms |
| C の関数にリスト 1,000 ／ 100,000 要素を渡す | ほぼ 0 | 17 µs ／ 3.0 ms |

### 6.4 参照の数（`sys.getrefcount`）

| 形 | CPython | 今の Arrow | 採らなかった案 ① |
|---|---|---|---|
| 束縛 1 つ | 3 | 4 | — |
| 束縛 3 つ ＋ リストの要素 2 つ | 7 | 4（増えない） | — |
| 変数への `p["k"] = 1`（`__setitem__` の中） | 4 | 3 | 4 |
| 一時的な物への `make()["k"] = 1` | 3 | 3 | 4 |

- 補助関数で `root[k1][k2] = v` を評価する案 ②: 連鎖代入の警告は CPython と同じになった。ただし補助関数を通すと
  `a.resize(5)` が `ValueError`（参照が多い）。CPython でも同じ補助関数を通すと誤りなので、補助関数を通すこと自体が
  振る舞いを変える。
- Windows では、Arrow が持つ大きな配列は numpy の一時配列の再利用で書き換えられなかった。numpy はこの最適化を、呼び出し元を
  調べられる環境でだけ行うと理解している（未確認）。Linux / macOS では 1-1 の試験で確かめる。

### 6.5 DataFrame を Arrow に持つ

- `let df: pd.DataFrame = pd.read_csv("people.csv")` で受けられ、クラスの欄・関数の引数と戻り値・`list[pd.DataFrame]`・
  `dict[str, pd.DataFrame]` に持って後から使えた。写しは作らず、CPython から見て同じ物（`let c = a` の後に `a` から足した列が
  `c` にも見える）。
- 型: `pd.read_csv` を含む `pd` のメンバーは、存在しない名前まで `Any`。`Any` の引数・変数の属性を読むと「明示のダウンキャストが要る」
  静的な誤りになる。`pd.DataFrame` の注釈なら通る。

### 6.6 1-1 の前後（参照の数と速さ）

| 形（呼ばれた側で見た `sys.getrefcount`） | CPython | 直す前 | 直した後 |
|---|---|---|---|
| 変数のメソッド ／ 一時的な物のメソッド | 3 ／ 2 | 4 ／ 4 | 3 ／ 2 |
| 変数への `p[k] = v` ／ 一時的な物への `make()[k] = v` | 4 ／ 3 | 3 ／ 3 | 4 ／ 3 |
| 束縛 1 つ ／ 束縛 3 つ ＋ リストの要素 2 つで関数の引数 | 3 ／ 7 | 4 ／ 4 | 3 ／ 7 |

- pandas（`refcount_pandas.ar` と同じ形・`read_csv` の DataFrame でも確かめた）: 変数への代入の誤った警告が消え、連鎖代入・
  連鎖の `inplace` の警告は CPython と同じに出る。`a.resize(5)` は通る。Windows で numpy の一時配列の再利用は起きない。
- 速さ（6.3 の操作・直す前 ／ 後・2 回ずつ）: `df.describe()` 1,766・1,730 ／ 1,729・1,734 µs、`df["a"].sum()` 11.5・11.5 ／
  12.3・11.7 µs、`arr * 2 + 1` 4.3・4.2 ／ 4.1・4.3 µs、`arr[k]` 0.50・0.50 ／ 0.58・0.59 µs、`df["a"].tolist()` 1,000
  122・122 ／ 125・121 µs。`arr[k]` の +0.08 µs のほかはノイズの幅。

### 6.7 1-3 の速さ（`AR_CHECK_ONLY=1` で型検査まで・3 回）

| | 直す前（1-5 の exe） | 直した後 |
|---|---|---|
| `import[py-int] pandas as pd` だけ | 80〜102 ms | 311〜572 ms |
| `import[py-int] numpy as np` だけ | 106〜152 ms | 176〜185 ms |

- たどる処理（`link_py_types`）の内訳（pandas・直す途中で測った）: 型スタブの抽出 47 ms・import の表 11 ms・たどって写す残り
  約 130 ms（142 ファイル）。
  - ⚠ 最初は import の表を Python の構文木で作り（357 ms）、名前をたどるたびにファイルを探し直していた（約 720 ms・合計
    1,128 ms）。表を行単位で読み、モジュールの名前で引く置き場（`Parser::py_raw_stubs`）を作って今の値になった。
