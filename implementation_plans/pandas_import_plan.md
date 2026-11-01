# pandas・numpy を Arrow から使う — Python のコードはすべて CPython で動かす

## 0. 妨げている要因（測定: Arrow `b6e1e2af`・pandas 2.2.0・numpy 1.26.4・CPython 3.12.2・2026-11-01〜02）

pandas を `import[py-int]`（組み込みの CPython）で読めば、`read_csv`・DataFrame の作成・表示・集計は今でも動く（6.5）。
残っているのは型と、CPython の物が Arrow との境界を越えるときの振る舞い・コスト。「対処」は本書のタスク。

| # | 要因 | 対処 |
|---|---|---|
| 1 | Python のソースを変換して動かす `import[py]` では pandas を読めない（C 拡張 42 本・numpy・標準ライブラリ・変換器が拒む構文）。直しても Arrow 側に Python 専用の特例が増え続ける | D-1・[3] |
| 2 | pandas の名前がすべて `Any`（`pandas/__init__.py` の再エクスポートを定義元までたどらない）。`pd.read_csv(..)` の型が分からず、存在しない名前も通る | [1-3] |
| 3 | `.py` の無い built-in のモジュール（`builtins`・`sys`・`itertools` など 67 本）は型の出所が無く `import[py-int]` できない（同梱のスタブは `math`・`time` だけ） | [1-4] |
| 4 | ユーザーの site-packages（`AppData\Roaming\Python\..`）を型の探索先にしない。`import[py-int] dateutil.parser` が `cannot find its types` | [1-5] |
| ~~5~~ | ✅ 参照の数（直した: 1-1）。以前は Arrow の束縛がいくつあっても CPython の参照は 1 つで、変数への `df["c"] = ..` を pandas が連鎖代入と誤判定した | D-5・[1-1] |
| 6 | Python の例外が `ForeignError` になり、`except IndexError:` などで捕まらない | [2-1] |
| 7 | 真偽: numpy の配列の `if a > 2:` が `ValueError` にならず真として通る | [2-2] |
| 8 | 切り出し `a[1:3]` が `TypeError`。`...`（`Ellipsis`）を渡せない | [2-3] |
| 9 | `for` が CPython の反復子から先に全部取り出す（無限の列で止まらない） | [2-4] |
| 10 | Arrow の関数・インスタンスを Python へ渡せない（`df.apply(f)`・`lib.map_infer(arr, f)` が `TypeError`） | [2-5]・[2-6] |
| 11 | list / dict / set は境界で写す。Python 側の書き換えが戻らない（`heapq.heappush(h, 0)` の後も `h` は元のまま） | [2-7] |
| 12 | Python の物を `let` で受けても、別名から中身が変わる（`let c = a` の後の `a["z"] = 1` が `c` にも見える） | D-3・[1-2] |
| 13 | `is` が `==` を呼ぶ／同じ物どうしが `False` | D-4・[5-1] |
| 14 | コスト: 越境 1 回ごとに +0.5〜1.7 µs。リストは Arrow → Python が 20〜50 ns/要素、Python → Arrow が約 110 ns/要素 | [4] |

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
- **D-3** Python のオブジェクトは **`let` で受けられない**（`mut` で受ける）。CPython の物は参照を共有するので、Arrow の
  `let` の「変わらない」を保証できない（0 節の 12）。
- **D-4** `is` の修正は後で行う（フェーズ 5）。
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

### 2.2 `let` で受けない（D-3）

- 静的: 束縛する式の型が Python の型（`import[py-int]` のモジュールのクラス、`PyObject`）なら、`let` / `const` は誤り。
- 実行時: `Value::PyObject` を `let` / `const` に束縛したら誤り（型が `Any` のときもここで止まる）。`freeze` も同じ。
- 境界で Arrow の値に変わる物（`int` / `str` / list など）は Arrow の値なので `let` で受けられる。
- ⚠ 関数の引数は既定で不変（`let`）なので、Python の物を受ける引数は `mut d: pd.DataFrame` と書くことになる（4 節の 5）。

### 2.3 型（D-2）

- `import[py-int]` の型の抽出が、`from pandas.core.api import DataFrame` のような再エクスポートを**定義元のファイルまで
  たどる**。たどれない名前は `Any`。
  - ⚠ 今はたどらないので、`pd` のメンバーは存在しない名前まで全部 `Any` になる（6.5）。
- `@overload` のある関数（`read_csv` は `iterator` / `chunksize` で戻り値が `DataFrame` か `TextFileReader` に変わる）は、
  引数の型・リテラルで選ぶ。選べなければ Union。
- built-in のモジュールのスタブは Arrow に同梱する（`src/py_stubs.rs` に足す。CPython の `inspect.signature` /
  `__text_signature__` から作れる部分は生成する）。

### 2.4 境界の振る舞いを CPython に合わせる

| 項目 | 今 | 直した後 |
|---|---|---|
| 例外 | `ForeignError`（`message` に `ValueError: ..`） | 同じ名前の Arrow の例外（組み込みは CPython 3.12 の階層を Arrow が持っている）。Python で定義した例外は `except m.Err:` で捕まる |
| 真偽（`if` / `while` / `not` / `and` / `or` / `bool()`） | 常に真 | `__bool__` / `__len__` を呼ぶ |
| 切り出し・`...` | `TypeError` | Python の `slice` / タプルで渡す（`a[1:, 0]`・`df.loc[:, "a"]`）。`Ellipsis` / `NotImplemented` を対応させる |
| `for` | 先に全部取り出す | 1 つずつ取り出す |
| Arrow の関数を渡す | `TypeError` | Python から呼べる物として渡す。中で起きた Arrow の例外は Python の例外になる |
| Arrow のインスタンスを渡す | `TypeError` | 4 節の 4 で決める |
| list / dict / set | 境界で写す（書き換えが戻らない） | 4 節の 3 で決める |

### 2.5 `.py` を CPython で実行する（D-1）

- `.py` は CPython が import して実行する（`import[py-int]` と同じ実行の道）。型は `import[py-int]` と同じ抽出器（2.3）。
- 変換器（`python_converter`）と、変換したコードのための Python 専用の仕組みは使われなくなる。
  - 仕組みの例: `running_python` / `in_python_module`、格納で写さない規則、属性の操作の上書き、`object()`、クラスを返す関数、
    prelude の `__py_*`。
  - 消す範囲は 4 節の 2 で決める。⚠ `.pyi` の型の抽出は今 `python_converter` の結果を使う（`py_stub_extract.rs` の冒頭）ので、
    そこは抽出器（2.3）へ移してから消す。

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
| 1-2 | Python の物を `let` / `const` / `freeze` で受けたら誤り（D-3・2.2）。静的と実行時の両方 | なし | 中 |
| 1-3 | 再エクスポートを定義元までたどって型を付ける。`@overload` を引数で選ぶ（D-2・2.3） | なし | 大 |
| 1-4 | built-in のモジュールの型スタブを同梱する（2.3） | なし | 中 |
| 1-5 | ユーザーの site-packages を型の探索先に足す（拡張の `wasm_host.ts` の `queryPythonDirs` も同じ問い合わせにする） | なし | 小 |

### フェーズ 2: 境界の振る舞いを CPython に合わせる（2.4）

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| 2-1 | Python の例外を同じ名前の Arrow の例外として受ける。Python で定義した例外は `except m.Err:` で捕まる | なし | 中 |
| 2-2 | 真偽の判定で `__bool__` / `__len__` を呼ぶ（誤りは 2-1 の例外） | 2-1 | 小 |
| 2-3 | 切り出し・多次元の添字・`Ellipsis` / `NotImplemented` を Python へ渡す | なし | 小〜中 |
| 2-4 | `for` で CPython の反復子から 1 つずつ取り出す | なし | 小 |
| 2-5 | Arrow の関数（クロージャ・束縛メソッド）を Python から呼べる物として渡す。中の Arrow の例外は Python の例外に、Python の例外は 2-1 で受ける。⚠ Python からの呼び戻し中に Arrow の VM へ入り直す（再入）ので、GIL と借用の扱いを先に設計する | 2-1 | 大 |
| 2-6 | Arrow のインスタンスを Python へ渡す（4 節の 4） | 2-5 | 中〜大 |
| 2-7 | list / dict / set の受け渡し（4 節の 3） | なし | 中 |
| 2-8 | Python の物を `async` のタスクへ渡したときの規則を決めて実装する（未測定。スレッドと GIL） | 1-1 | 中 |

### フェーズ 3: `.py` を CPython で実行する（2.5）

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| 3-1 | `.py` の import を CPython の実行に切り替える（4 節の 1）。型は `import[py-int]` と同じ抽出器 | 1-3・2-1〜2-5 | 中 |
| 3-2 | 例題の移行: `import[py]` を使う 114 本（`interop` 106・`classes` 4・`collections` 3・`interop/py_subdir` 1）を CPython の出力と突き合わせて直す。変換器の誤りを確かめる `_error` の例題は消すか書き換える | 3-1 | 大 |
| 3-3 | 変換器と Python 専用の仕組みを消す（2.5・4 節の 2）。文書（`language-differences.md` の Python の節・`docs/grammar/09_imports.md`・skills）も直す | 3-2 | 大 |

### フェーズ 4: 速さ（計測してから）

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| 4-1 | Python → Arrow の変換（約 110 ns/要素）を測って直す。原因の候補は `py_to_tl` が要素ごとに型の名前の文字列を作ること | なし | 小〜中 |
| 4-2 | 越境 1 回ごとの GIL の取得（+0.5〜1.7 µs）をまとめられるか測る | 1-1 | 中 |
| 4-3 | numpy の数値の配列を Arrow のリストにするとき、バッファを直接読む | 4-1 | 中 |

### フェーズ 5: `is`（D-4）

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| 5-1 | CPython の物どうしの `is` を同一性にする（同じ物なら真。片方だけ CPython の物なら偽） | 1-1 | 小 |

---

## 4. 未決（要判断）

1. **`import[py]` の扱い**（3-1）: ① `import[py]` を CPython の実行に変え、`import[py-int]` と同じ意味にする（例題の書き換えが
   要らない） ② `import[py]` を廃止して `import[py-int]` に一本化する（114 本を書き換える） ③ 変換器を残し、利用者が選べるようにする。
2. **消す範囲**（3-3）: 変換器と Python 専用の仕組みを全部消すか、一部を残すか（③を選ぶなら残す）。
3. **Python から来たコンテナ**（2-7）: 今どおり境界で写すか（Arrow の値になる。Python 側の書き換えは届かない）、包みのまま持つか
   （CPython と同じく共有する。Arrow の list のメソッドの代わりに Python のメソッドが走る）。
4. **Arrow のインスタンスを Python へ渡すとき**（2-6）: 代理のオブジェクト（属性・メソッドを Arrow へ中継する）にするか、今どおり
   誤りにするか。
5. **関数の引数**（1-2）: 既定で不変の引数も D-3 の対象になるので、Python の物を受ける引数は `mut` と書くことになる。これでよいか。

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
