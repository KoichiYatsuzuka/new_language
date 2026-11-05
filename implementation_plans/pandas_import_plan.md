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
| ~~7~~ | ✅ 真偽: numpy の配列の `if a > 2:` が `ValueError` にならず真として通る件（直した: 2-2・`__bool__` / `__len__`） | [2-2] |
| ~~8~~ | ✅ 切り出し `a[1:3]` が `TypeError`。`...`（`Ellipsis`）を渡せない件（直した: 2-3・`a[1:, 0]`・`a[..., 0]`・`df.loc["b":"d"]`） | [2-3] |
| ~~9~~ | ✅ `for` が CPython の反復子から先に全部取り出す（無限の列で止まらない）件（直した: 2-4） | [2-4] |
| ~~10~~ | ✅ Arrow の関数・インスタンスを Python へ渡せなかった（`df.apply(f)`・`lib.map_infer(arr, f)` が `TypeError`）件（直した: 関数は 2-5、インスタンス・クラス・ジェネレータは 2-6） | [2-5]・[2-6] |
| ~~11~~ | ✅ list / dict / set を境界で写していた（Python 側の書き換えが戻らない・`heapq.heappush(h, 0)` の後も `h` は元のまま）件（直した: 2-7・2-11。Python とのやり取りは `py_list` などの専用の型・Arrow のコレクションはキャストして渡す・D-12・2.6） | [2-7]・[2-11] |
| ~~12~~ | ✅ Python の物を `let` で受けても別名から中身が変わる件（直した: 1-2。`mut` で受ける規則にした） | D-3・[1-2] |
| ~~13~~ | ✅ `is` が `==` を呼ぶ／同じ物どうしが `False`（直した: 5-1・`===` は同一性） | D-4・[5-1] |
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
- **D-8** Python から来た list / dict / set は**写さずにそのまま使う**（2026-11-04 に決定・4 節の 3・2.6）。型は `py_list[T]` / `py_dict[K, V]` /
  `py_set[T]`（D-12）。
- **D-9** Arrow のインスタンス・クラス・ジェネレータは、**Python からそのまま使えるように渡す**（包む・2026-11-04 に決定・4 節の 4・2.6）。
  ⚠ Arrow の list / dict / set は包まない（包むと pandas などが壊れる・6.8）。Python へ渡すには `=> py_list[T]` などで写す（D-12）。
- **D-12** Python とのコレクションのやり取りは、組み込みの型 **`py_list[T]` / `py_dict[K, V]` / `py_set[T]`** で行う（2026-11-04 に決定・
  4 節の 8・2.6）。中身は CPython の list / dict / set で、Arrow の list / dict / set とは**キャスト `=>` で相互に写す**。
  - Arrow の list / dict / set をキャストせずに Python へ渡すのは誤り（**リテラルも**: `np.array([1, 2, 3] => py_list[int])`）。
  - Python が返した list / dict / set は写さずに `py_list` などのまま受ける（Arrow の list にするなら `=> list[int]`）。
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
| 切り出し・`...` | `TypeError` | ✅ Python の `slice` / タプルで渡す（`a[1:, 0]`・`df.loc[:, "a"]`）。`Ellipsis` / `NotImplemented` を対応させる（2-3） |
| `for` | 先に全部取り出す | ✅ 1 つずつ取り出す（2-4） |
| Arrow の関数を渡す | `TypeError` | ✅ Python から呼べる物として渡す。中で起きた Arrow の例外は Python の例外になる（2-5） |
| Arrow のインスタンスを渡す | `TypeError` | ✅ 写さずに包んで渡す（`let` の値は読むだけ・2-6・D-9） |
| list / dict / set | 境界で写す（書き換えが戻らない） | `py_list` などの専用の型で受け渡す（2-7・2-11・D-12・2.6） |

### 2.5 `.py` を CPython で実行する（D-1・D-6・D-7）

- `import[py]` は CPython が import して実行する（従来の `import[py-int]` の実行の道）。型は従来の `import[py-int]` と同じ
  抽出器（2.3）。
- `import[py-int]` は廃止: 書くと構文解析の誤りにし、`import[py]` に書き換えるよう促す。`import[py-int]` を使う例題
  （3-1 で `import[py]` から移した 80 本を含む）を `import[py]` に書き換える。
- 変換器（`python_converter`）と、変換したコードのための Python 専用の仕組みは**すべて消す**（D-7）。
  - 仕組みの例: `running_python` / `in_python_module`、格納で写さない規則、属性の操作の上書き、`object()`、クラスを返す関数、
    prelude の `__py_*`。
  - ⚠ `.pyi` の型の抽出は今 `python_converter` の結果を使う（`py_stub_extract.rs` の冒頭）ので、そこは抽出器（2.3）へ
    移してから消す。

### 2.6 Python とのコレクションのやり取り（D-8・D-9・D-12・2026-11-04）

**経緯**: 4 節の 4（Arrow の値を写さずに渡すか）で「包んで渡す」（PyO3 の型を `collections.abc` に登録する）としたが、同じ形の
Python のクラスで測ると、Python のコードが `isinstance(x, dict)` / C の `PyList_Check` で本物の list / dict かを確かめて道を分ける
ところで壊れた（6.8）。`pd.DataFrame(包んだ dict)` は誤りにならず、鍵の一覧を値とする 1 列の表を返す。
- 包みを本物の list / dict の派生にしても共有できない。C のコードは `ob_item`（`PyObject*` の配列）を直接読むので、Python から
  見える中身は CPython の配列になる。Arrow の list（32 バイトの `Value` の並び・要素は値そのもの）とは作りが違い、同じメモリを
  両方の形で見せられない。
- 写さない利点（呼び出しの中の書き換えが戻る・持ち続けた物の書き換えが届く・同じ物のまま・写す時間が無い）を全部得られるのは、
  中身が初めから CPython の list であるときだけ。
- ⇒ **専用の型 `py_list[T]` / `py_dict[K, V]` / `py_set[T]` を足し、Arrow のコレクションとはキャストで写す**（D-12）。

**型**（組み込み・中身は CPython の物 `Value::PyObject`）

| 型 | 中身 | 得る方法 |
|---|---|---|
| `py_list[T]` | CPython の list | `xs => py_list[T]`・Python が返した list |
| `py_dict[K, V]` | CPython の dict | `d => py_dict[K, V]`・Python が返した dict |
| `py_set[T]` | CPython の set | `s => py_set[T]`・Python が返した set |

- 要素の型は Python に置ける物だけ: `int` / `float` / `bool` / `str` / `bytes` / `None`・それらのタプル・`py_list` など・CPython の
  クラス・関数・`Any`・それらの `Union`。Arrow の list / dict / set は置けない（`py_list[list[int]]` は誤り・`py_list[py_list[int]]`
  と書く）。Arrow のインスタンスは 2-6 の包み。
- キャスト `=>` はキャスト先の型の深さまで写す: `[[1, 2], [3]] => py_list[py_list[int]]`、`pl => list[list[int]]`（中も Arrow の
  list）、`pl => list[py_list[int]]`（外側だけ）。写すときに要素を宣言の型と照合する（違えば `TypeError`）。
- 操作は **Python の意味**（中身は Python の物なので CPython を呼ぶ・D-1）: 添字・切り出し・`len`・反復・`in`・`+`・`==`・表示
  （`['a']`）・メソッドは Python の list / dict / set のもの。型は閉じた表で付ける（無いメソッドは静的な誤り）。
- Python の物なので **`mut` で受ける**（D-3）。`tuple` / `frozenset` / `str` / `bytes` は不変なので、今までどおり境界で写す。

**境界**
- Arrow → Python（2-11）: Arrow の list / dict / set を**キャストせずに**渡すのは誤り（リテラルも・呼び出しの引数・添字・属性への代入・
  演算子・呼び戻しの戻り値）。型で分かれば静的、分からなければ実行時の `TypeError`。
- Python → Arrow（2-7）: Python が返した list / dict / set は写さずに `py_list` などのまま。型スタブの `list[T]` / `dict[K, V]` /
  `set[T]` は `py_list[T]` などと読む。
- 要素の型（4 節の 3 の ①）: 写して受け取るとき（キャスト・戻り値の境界検査）に照合し、共有されるので後で Python が別の型を入れうる
  ⇒ 読むたびにも照合する（2-12）。

**Python → Arrow の不変の値**（4 節の 3 の検討・変わらない）: `int` / `float` / `bool` / `None` / `str` / `bytes` は両側とも不変なので、
写しても共有と区別がつかない（`str` を借りるのは速さのタスク）。numpy の配列は今も写さない（4-3）。

### 2.7 Arrow の関数を Python から呼ぶ（2-5 の設計・2026-11-04。違えば直す）

- **包み**: Arrow の関数（`fn`・クロージャ・束縛メソッド・多重定義・ネイティブの関数）を Python へ渡すと、Python の呼べる物
  `ArrowFunction`（PyO3 の `#[pyclass]`）になる。中身の `Value` はスレッドごとの表に置き、包みは番号と作ったスレッドだけを持つ
  （`Rc` を Python のスレッドへ持ち出さない）。Python が包みを捨てると表から消す（別のスレッドで捨てられたら、作ったスレッドで次に
  表を使うときに消す）。クラス・ジェネレータ関数も同じ包み、インスタンスとジェネレータは別の包み（2-6）。
- **呼び戻し（再入）**: Python が包みを呼ぶと、そのスレッドのインタプリタで関数を呼ぶ（`call_value_evaled`）。インタプリタは
  py-int の import とワーカーの起動のときにスレッドごとに覚え、Python を呼ぶ入口（呼び出し・メソッド呼び出し）でも覚え直す
  （ネイティブの ABI の `interp_ptr` と同じ方式）。GIL は Python が呼んだときに持っているので、そのまま Arrow の本体を走らせる
  （本体が Python を呼べば入れ子の `with_py`）。引数は Python → Arrow の変換（`py_to_tl`）、キーワード引数は名前つき、戻り値は
  Arrow → Python の変換（`ar_to_py`・関数を返せばまた包む）。
- **スレッド**: 作ったスレッド以外から呼ばれたら Python の `RuntimeError`（Arrow の値はスレッドをまたげない・2.6 の ⑥ と同じ）。
- **例外**: 本体で起きた Arrow の例外は Python の例外になる。組み込みの例外は同じ名前の Python の例外（`except ValueError:` で
  捕まる・`args` も渡す）、Arrow で定義した例外は `arrow.ArrowError`（`Exception` の子）。Python の例外が Arrow を通って戻るとき
  （Python の物の例外）は同じ物を投げ直す。⚠ Python の中を通って Arrow へ戻った例外は、**元の Arrow の例外の値**に戻す
  （`except MyErr as e` で捕まり、欄も同じ）。
- 型検査: Python の関数の引数（スタブで `Callable[..]` は `Any`）へ Arrow の関数を渡すのは今までどおり通る。

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
| ~~2-3~~ | ✅ **済**（2026-11-04）CPython の物の切り出しは Python の `slice`（`py_getitem` / `py_setitem`・`ar_to_py` の `Value::Slice`）。添字の項にスライス・`...` を書ける（`a[1:, 0]`・`a[..., 0]`・`parse_subscript_item`・`...` は添字の項だけ）。`Ellipsis` / `NotImplemented` は CPython の同じ値。スライスは境界に何でも持てる（CPython と同じ・`slice_from_values` が唯一の作り手）で、Arrow の list などを切り出すときに `slice_bound` が確かめる（文言は CPython）。型検査は添字の相手が CPython のクラスか `Any` のときだけ境界の型を問わない（`df.loc["b":"d"]`）。ついでに: `xs[1::2]` が `::` をトレイトの指定と読んで構文の誤りだったのを直した（名前と `.` が続くときだけトレイトの指定）、スライスの表示の境界を `repr` にした。例題は `py_int_slices.ar`・`_error`・`_runtime_error`・`collections/slice_begin_step.ar`。⚠ Python から来た `slice` は Python の物のまま | なし | 小〜中 |
| ~~2-4~~ | ✅ **済**（2026-11-04）`for x in <CPython の物>` は `iter(obj)` の反復子から取り出すたびに 1 つ取る（`GeneratorState::py_iter`・`gen_next` が `PyIter_Next`）。`break` すれば先は取り出さず、途中の例外はそこまでの本体のあとで届く。`Generator` の形のままなので、内包・`list()`・`any()` / `all()`・分解・ネイティブの関数の `for` も同じ道。取り出し途中の反復子は写せない（`deep_clone` は中断中の生成器と同じく印を付ける）。1 要素あたり +0.12 µs（30 万要素の `range` と生成器・負の対照は ±1 ms）。例題は `py_int_lazy_for.ar` | なし | 小 |
| ~~2-5~~ | ✅ **済**（2026-11-04・設計は 2.7）Arrow の関数（`fn`・束縛メソッド・多重定義・ネイティブの関数）は Python の呼べる物 `arrow.ArrowFunction`（`#[pyclass]`・中身はスレッドごとの表 `ARROW_FUNCTIONS`、包みは番号と作ったスレッドだけ）。Python が呼ぶと覚えておいたインタプリタ（`register_interpreter`・py-int の import と Python を呼ぶ入口）で `call_value_evaled`（GIL を持ったまま再入・入れ子も可）。別のスレッドからは Python の `RuntimeError`。本体の Arrow の例外は Python の例外（組み込みは同じ名前・`args` も、Arrow で定義した例外は `arrow.ArrowError`）、Python の中を通って Arrow へ戻ったら元の Arrow の例外（`ARROW_RAISED` → `PyRaised::Arrow`・欄もトレースバックの枠も。呼んだ Python の枠もファイル・行・関数名で入る）。例題は `py_int_callbacks.ar`・`_runtime_error` | 2-1 | 大 |
| ~~2-6~~ | ✅ **済**（2026-11-05・D-9）Arrow のインスタンスは写さずに `arrow.ArrowObject` の派生で渡す（Arrow のクラスごとに 1 つ・Arrow のクラスが持つ特殊メソッドだけを持つ・`proxy_type`。属性・メソッドは Arrow のインスタンスを読み、`str` / `repr` / 比較 / `hash` は Arrow の意味）。同じインスタンスは同じ包み（弱参照の表 `PROXIES`）、Python から戻れば元のインスタンス（`unwrap_arrow_value`）。書けるのは引数の `mut` の変数・式、書ける包みから読んだ物、Arrow の関数が返した物だけで、ほかは読むだけ（属性への代入・`mut self` のメソッドは `TypeError`）。クラス・ジェネレータ関数は `ArrowFunction`、ジェネレータは `ArrowIterator`。作ったスレッド以外からは `RuntimeError`（表は関数と共有・`ARROW_VALUES`）。例題 `py_int_arrow_objects.ar`、`py_int_instance_arg_error.ar` を読むだけの包みの負例に直した。⚠ インスタンスの list などの欄は Python から読むと `TypeError`（D-12）。⚠ 見つけたこと: ジェネレータ関数の引数は `mut` にできないので Python の物を取れない（2-13） | 2-5 | 中〜大 |
| ~~2-7~~ | ✅ **済**（2026-11-04・D-8・D-12・2.6）`py_list[T]` / `py_dict[K, V]` / `py_set[T]` を組み込みの型にした（`InferredType::PyList` / `PyDict` / `PySet`・実行時は `Value::PyObject`）。`=>` で Arrow の list / dict / set と相互に写す（キャスト先の型の深さまで・写しながら要素を照合・`py_*` から `py_*` は照合だけ・`cast_to_python` / `cast_from_python`）。操作は Python の意味で、型は `py_collection_binop`・閉じたメソッドの表 `names::PY_COLLECTION_METHODS`・`py_collection_method_return`。要素に Arrow の list などは置けない（`ArrowCollectionInPython`）。`mut` で受ける。Python が返した list / dict / set は写さずに `py_list` などのまま（`py_to_tl`）、型スタブの `list[T]` などは `py_list[T]`（`py_type_to_arrow`・変換器で読んだ `.pyi` は `py_collections_as_python`）。戻り値の境界検査は要素まで（`float` には int も入る・PEP 484）。frozenset は今までどおり Arrow の set に写す。例題 `py_int_collections.ar`・`_error`・`_runtime_error`、既存の例題 9 本を `=> list[T]` / `mut` に直した。⚠ 照合は受け取るときだけ（2-12）。⚠ Arrow のコレクションをキャストせずに渡すと今は写す（2-11） | なし | 中〜大 |
| ~~2-11~~ | ✅ **済**（2026-11-05・D-12）Arrow の list / dict / set（とそれを持つ tuple）をキャストせずに Python へ渡すのは誤り（リテラルも）。関数・メソッド・クラスの引数（キーワード・`*` / `**` も）・添字・要素や属性への代入・演算子の相手。受け手が Python と型で分かれば静的な `ArrowCollectionToPython`（写す先の型を示す・`is_python_value`・`PyNamespace` に CPython のモジュールかの印を足した）、分からなければ実行時の `TypeError`（`ar_to_py`）。`pyobj in xs`（右が Arrow のコレクション）は Arrow の側で比べる。例外の `args` だけは写す（記録）。例題 `py_int_cast_required_error.ar`・`_runtime_error`、既存の例題 8 本（と `practical_examples/pd_numpy_pyplot` の 2 本）にキャストを足した | 2-7 | 中 |
| ~~2-13~~ | ✅ **済**（2026-11-05・4 節の 9）ジェネレータ関数の引数（`let`）は Python の物を受けられる（D-3 の例外・`gen` の引数は `mut` にできないので）。静的な `ImmutablePythonObject` も実行時の `TypeError` も出さない（`declare_param_in`・`exec_generator_evaled`）。受けた物は共有され、取り出すたびに要素を照合する（2-12）。例題 `py_int_generator_params.ar`。以下は起票時の内容: ジェネレータ関数の引数は `mut` にできない（`let` / `const` だけ・構文の誤り）ので、Python の物（`mut` で受ける・D-3）を引数に取れない（`gen each(mut xs: py_list[int])` が書けない・2-6 で見つけた） | なし | 小〜中 |
| ~~2-12~~ | ✅ **済**（2026-11-05・4 節の 3 の ①）`py_list` / `py_dict` の要素を読むたびに宣言の型と照合する（違えば `FfiTypeError`・`float` の所の int は float に寄せる）。型検査が読む所に型を載せ（注釈の `py_reads`: 添字の node・`py_iters`: `for` / 内包の回す式の node。中の `py_list` などは外側の種類だけ・`python_read_check`）、実行時に `check_python_read` で照合する（VM は載っているときだけ `CheckPyRead` / `GetIterChecked` を出す・反復子は `GeneratorState::py_check`）。メソッドの戻り値（`xs.pop()`）は前からある戻り値の境界検査（`check_ffi_return`）が照合する。例題 `py_int_element_reads.ar`・`_runtime_error`。⚠ ネイティブにコンパイルした関数の中の読みは照合しない | 2-7 | 中 |
| ~~2-8~~ | ✅ **済**（2026-11-05）Python の物は `mut` の値と同じくタスクと共有する（写さない・GIL で 1 つずつ走る・提出のときの警告は今までどおり）。**タスクを待つ間（`wait_for_finish`・`EventLoop.run`）は GIL を手放す**（`py_interop::without_gil`）。⚠ 以前は Python が呼んだ Arrow の関数の中で Python の物を使うタスクを待つと、待つ側が GIL を持ったままで止まった（実測）。Arrow が渡した包みは作ったスレッドでだけ使える（タスクから触ると `RuntimeError`・2-6）。例題 `async/async_py_objects.ar`。⚠ 見つけたこと: `async` の本体の `mustbe` が VM に載らない（known_bugs.md の #14・Python と無関係） | 1-1 | 中 |
| ~~2-10~~ | ✅ **済**（2026-11-04）`for mut x in ..`・`[.. for mut x in ..]`・`except E as mut e` を足した（AST の `mut_targets` / `name_mut`）。型で Python の物と分かれば静的な `ImmutablePythonObject`、分からなければ実行時の `TypeError`（VM: 1 つの変数は `ForIter` が要素が Python の物のときだけ副表 `implicit_for_targets` を引く・分解と `except` は新しい命令 `StoreLocalImplicit`・`for mut` は `StoreLocalDeepCopy` で写す）。呼んだ側の大域で走る組み込みの実装（`enumerate` など）の中は検査しない（`Chunk::builtin_prelude`）。ネイティブの関数は `for mut` を載せない。例題 `basics/for_mut.ar`・`py_int_implicit_bindings.ar`・`_error`・`_runtime_error`、`py_int_exceptions.ar` を `as mut` に直した。以下は起票時の内容: 暗黙の束縛で Python の物を受ける規則（D-11）。`for mut f in ..`（`for mut k, v in ..` はすべて `mut`）・内包の `for mut f in ..`・`except E as mut e`・`match` の捕捉に `mut` を書けるようにする。不変の束縛で Python の物を受けたら、型で分かれば静的な誤り（`ImmutablePythonObject`）、分からなければ束縛するときに実行時の `TypeError`。⚠ 外の束縛が `mut` でも Python の物を受けるなら `mut` と書く（受け継ぎは実行時に判定できない。VM の slot は可変かどうかを持たない）。Arrow の値を `mut` で受けたら `mut` の宣言と同じく写す。`match` は新しい名前を束縛しない（`case <式>` / `is <型>`）ので対象外 | 1-2 | 中 |
| ~~2-9~~ | ✅ **済**（2026-11-03）CPython の物とモジュールの属性への代入（`df.columns = [..]`・`gm.count = 5`）は CPython の `setattr`（`py_setattr`・受け手と値は借りるだけで、`__setattr__` の中の参照の数は CPython と同じ）。型スタブに型のある大域変数への代入は静的に検査する（属性は従来どおり）。例題は `py_int_attr_assign.ar`・`_error`。⚠ 削除（`del o.x`）は Arrow に構文が無いので対象外 | なし | 小 |

### フェーズ 3: `.py` を CPython で実行する（2.5）

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| ~~3-1~~ | ✅ **済**（2026-11-06）例題の移行: `import[py]` で読んでいた例題 105 本（`interop` 104・`interop/py_subdir` 1。ほかの 9 本はコメントで触れているだけ）を `import[py-int]`（CPython）に書き換え、CPython の値に合わせて直した。80 本を残し、変換器の誤りだけを確かめていた `_error` 25 本と、それだけが使う Python のモジュール 45 本を消した。移す中で見つけた不備を直した: ① CPython の `print` が、出力先がファイル・パイプのとき消えた（`sys.stdout` / `sys.stderr` を行ごとに書き出し、終わりに残りを書き出す・`py_interop::flush_stdio`） ② Python の `...` / `NotImplemented` が Arrow の同じ値に戻らなかった（`py_to_tl`） ③ Arrow の `isinstance` / `issubclass` / `type` が CPython のクラス・物を受けなかった（`call_py_builtin`） ④ 型スタブ: `@classmethod` の `cls` を外していなかった・`f = staticmethod(f)` を読まなかった・基底の基底のメンバーが無かった・入れ子のクラス（`Tree.Node`）を落としていた・本体に `if` のあるクラスを閉じていた ⑤ CPython のクラスの型をモジュールで修飾し（`z.Dog` と `k.Dog` は別の型・`names_types_by_module`）、基底を型に載せた（派生を基底の型で受ける。ほかのモジュールの基底はそのモジュールで修飾した名前）⑥ CPython のクラスの静的メソッドをインスタンスから呼ぶのが静的な誤りだった。⚠ 旧変換器が CPython と違っていた所（可変の既定値の共有・`is` とインターン・`self.x = ..` がクラス属性を隠す・キーワード専用引数を位置で渡す形）は CPython の値に直した。⚠ 例題の `.py` に注釈を足した所がある（注釈の無い戻り値は `Any`・D-2）。⚠ 3-2 と順を入れ替えた（4 節の 10） | なし | 大 |
| 3-2 | `import[py]` を CPython の実行に切り替え、`import[py-int]` を廃止する（書くと誤り・D-6・2.5）。`import[py-int]` を使う例題（3-1 で移した 80 本を含む約 190 本）を `import[py]` に書き換える。型は従来の `import[py-int]` と同じ抽出器 | 3-1 | 中 |
| 3-3 | Python の翻訳に固有のコードをすべて消す（D-7・2.5）。文書（`language-differences.md` の Python の節・`docs/grammar/09_imports.md`・skills）も直す | 3-2 | 大 |

### フェーズ 4: 速さ（計測してから）

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| 4-1 | Python → Arrow の変換（約 110 ns/要素）を測って直す。原因の候補は `py_to_tl` が要素ごとに型の名前の文字列を作ること。⚠ D-8 で list などは写さなくなるので、2-7 の後に残る変換（`=> list[T]` の明示の写し・要素を読むときの変換）を測る | 2-7 | 小〜中 |
| 4-2 | 越境 1 回ごとの GIL の取得（+0.5〜1.7 µs）をまとめられるか測る | 1-1 | 中 |
| 4-3 | numpy の数値の配列を Arrow のリストにするとき、バッファを直接読む | 4-1 | 中 |

### フェーズ 5: `is`（D-4）

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| ~~5-1~~ | ✅ **済**（2026-11-04）CPython の物どうしの `===`（Python の `is`）は同じオブジェクトか（`PyObjHandle::identity`・`__eq__` を呼ばない）。片方だけ CPython の物なら偽。`apply_binop` が Python へ写す前に `values_ref_eq` で決める（写せない Arrow のインスタンスとでも偽）。例題は `py_int_identity.ar`。以下は起票時の内容: CPython の物どうしの `is` を同一性にする（同じ物なら真。片方だけ CPython の物なら偽） | 1-1 | 小 |

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
8. ~~Arrow の list / dict / set を Python へどう渡すか~~（4 の包みを測ったら pandas などが壊れた・6.8）⇒ 専用の型 `py_list` / `py_dict` /
   `py_set` を組み込みで足し、Arrow のコレクションとはキャストで写す。キャストを書かずに渡すのは誤り（リテラルも）。Python が返した物は
   `py_list` などのまま（D-12・2.6）。

決定（2026-11-06・実施時の判断）:
10. 3-1（切り替え）と 3-2（例題の移行）の順を入れ替えた。例題を先に `import[py-int]` で CPython に合わせておけば、切り替えは
    書き換えだけになり、各コミットでゲートが緑のまま進める（切り替えを先にすると、例題 105 本が直すまで落ちたままになる）。

決定（2026-11-05）:
9. ~~ジェネレータ関数の引数で Python の物を受けるか~~（`gen` の引数は `mut` にできず、D-3 と食い違う・2-6 で発見）⇒ ジェネレータ関数の
   引数（`let`）は Python の物を受けられる（D-3 の例外・2-13）。

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

### 6.8 Arrow の list / dict / set を包んで渡す案（2026-11-04・CPython 3.12.2・pandas 2.2.0・numpy 1.26.4）

方法: 包み（list / dict / set を継承せず、`collections.abc` の `MutableSequence` / `MutableMapping` / `MutableSet` に登録した型）と
同じ形の Python のクラスを書き、よく使う受け手約 60 個に渡した（`wrapper_probe.py`・作業用で保存していない）。

- 黙って違う結果: `pd.DataFrame(包んだ dict)` が `{0: ['a', 'b']}`（鍵の一覧の 1 列）。`DataFrame.__init__` は `isinstance(data, dict)`
  で dict の道に入り（frame.py 765 行）、外れると「list のような物」として `list(data)` にする（826 行）。
- 誤り: `heapq.heappush` / `heapify`（C で本物の list か確かめる）・`json.dumps`（`isinstance` で分ける）・`[0] + 包み`・`{0} | 包み`・
  `{"z": 0} | 包み`・`df.sort_values(by=包み)`（`KeyError`・list でなければ列名 1 つ・frame.py 7142 行）・`df.groupby(包み)`
  （`TypeError: not callable`・grouper.py 939 行）・`eval(.., 包み)`。
- 動いた: `np.array` / `np.sum` / `pd.Series` / `df[包み]` / `df.loc[包み]` / `isin` / `df.rename(columns=)` / `astype` / `agg` / `replace` /
  `map` / `random.shuffle` / `bisect.insort` / `sorted` / `sum` / `str.join` / `f(**包み)` / `copy.deepcopy` / `pickle` / `csv`。
- ⇒ 包みは採らない（D-12・2.6）。
