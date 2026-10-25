# known_bugs 実装ログ — Python の読み込み（`import[py]`）の残りの不具合（2026-10-22 〜 2026-10-26）

[python_builtins_plan.md](../implementation_plans/python_builtins_plan.md) の残りと、その作業中に見つけた不具合を
[known_bugs.md](../implementation_plans/known_bugs.md) に起票して直した記録。済んだ 10 件（#1〜#6・#9〜#12）の実装を
ここへ移した。**番号は known_bugs.md のまま**（ソースのコメントの「known_bugs.md の #N」はこの文書の #N を指す）。

- 残り: **#7 / #8**（`import[py-int]` 経由・CPython との値のやり取り）は 2026-10-26 に「今回は扱わない」と決定して
  known_bugs.md に残した
- 基準: どの項目も CPython 3.12.2（Windows）と同じ出力になる例題で確かめた（release ビルド）。項目ごとに全ゲート
  （`test_build_gate` / `scan_examples` / `force_gate` / `compare_python_impl` / `compare_outputs` / `type_obligations` /
  `stale_doc_refs` / `compare_import_paths` / `compare_bytecode` / `compare_wasm_frontend`）を、直前のコミットから
  作った exe を基準に走らせた
- 対象: `src/python_converter/`（変換器）・`src/interpreter/`（実行時）・`src/type_check/`（型検査）・`src/parser/`

---

## この文書の読み方

| 節 | 中身 | いつ読むか |
|---|---|---|
| §1 | 纏め（項目・決定・コミット・例題） | 何が入ったかを一覧で見るとき |
| §2 | 横断の仕組み（Python 専用の組み込み・「Python のクラスだけ」の判定・属性の上書きの順序） | ⭐ Python の振る舞いを足すとき |
| §3 | 項目ごとの記録（直す前・決定・実装・残した差） | 番号から実装を引くとき |
| §4 | 発見ログ | ⭐ 次に似た作業をする前に読む |

---

## §1 纏め

| # | 内容 | 決定 | コミット | 例題（CPython と同じ出力） |
|---|---|---|---|---|
| 1 | Python のコードの中は格納で写さず参照を共有する・Python のクラスの既定の `==` / `hash` は同一性 | A 案（2026-10-23） | `742b9cb4` | `interop/py_shared_objects.ar` |
| 2 | `del o.a`（属性を未代入に戻す） | 属性は未代入に戻す・名前の `del x` は読み飛ばす（2026-10-23） | `8fd23ef4` | `interop/py_del.ar`・`py_del_error.ar` |
| 3 | `del d[k]`（辞書・リスト・スライス・`__delitem__`） | コレクションの削除は実装する（2026-10-23） | `8fd23ef4` | 同上 |
| 4 | `%s` / `%r` が `__str__` / `__repr__` を呼ぶ | （判断不要） | `16c931eb` | `basics/percent_format.ar`・`interop/py_percent_format.ar` |
| 5 | 静的メソッド・クラスメソッドをインスタンスから呼ぶ | A 案 — Python のクラスだけ（2026-10-25） | `03ebe04f` | `interop/py_methods_via_instance.ar` |
| 6 | クラスメソッドの中の `super()` など | Python 専用の `super()` の組み込み（2026-10-25） | `9aca9472`（調査）・`2e07586b` | `interop/py_super_classmethod.ar`・`py_super_static_error.ar` |
| 9 | 実行時の `KeyError` の `args` をキーそのものに・`OSError` の `errno` / `strerror` / `filename` | 文言と引数の控え（2026-10-24） | `1122903a`・`d069c252` | `interop/py_key_error_args.ar`・`py_os_error_args.ar`・`py_os_error_args_error.ar` |
| 10 | `__setattr__` / `__delattr__`（と `object.__setattr__`） | Python の読み込みでだけ（2026-10-24） | `c5a31eaa` | `interop/py_setattr.ar`・`classes/setattr_hook_error.ar` |
| 11 | `__getattr__`（と `object.__getattribute__`） | 同上（2026-10-24） | `97820e01` | `interop/py_getattr_hook.ar`・`classes/getattr_hook_error.ar` |
| 12 | `__getattribute__` | A 案 — 特殊メソッドの明示の呼び出しも通す（2026-10-26） | `4d8a6396` | `interop/py_getattribute_hook.ar`・`classes/getattribute_hook_error.ar` |

起票は `6525f4ef`（2026-10-22）、master への rebase 後の測り直しは `c89c10da`。

---

## §2 横断の仕組み

### 2.1 Python 専用の内部の組み込み（変換器だけが置く）

Arrow に構文の無い Python の操作は、変換器が `__py_` で始まる内部の組み込みの呼び出しにする。利用者が書く名前ではない
（`builtins.ars` には宣言しない・`tests/mod.rs` の突き合わせも `__` で始まる名前を飛ばす）。

| 組み込み | 変換元 | 本体 | 項目 |
|---|---|---|---|
| `__py_delattr(o, "a")` | `del o.a` | `Interpreter::del_attr_val(.., hook: true)` | #2 |
| `__py_delitem(d, k)` | `del d[k]` | `Interpreter::eval_delitem` | #3 |
| `__py_object_setattr(o, k, v)` | `object.__setattr__(o, k, v)` / 基底が `object` の `super().__setattr__(k, v)` | `attr_assign_without_hook` | #10 |
| `__py_object_delattr(o, k)` | `object.__delattr__(o, k)` / 同 `super()` | `del_attr_val(.., hook: false)` | #10 |
| `__py_object_getattribute(o, k)` | `object.__getattribute__(o, k)` / 同 `super()` | `get_attr_without_hook` | #11 |
| `__py_super(基底, 受け手, "m", ..)` | `super().m(..)` / `super(C, x).m(..)`（基底が Python のクラス） | `Interpreter::py_super_call` | #6 |

- 登録は 3 つの表（`eval/builtins.rs` の `BUILTIN_VALUE_NAMES`・`vm/compiler/diag.rs` の `VM_BUILTIN_NAMES`・
  `type_check/names.rs` の `RUNTIME_BUILTIN_NAMES`）。キーワード引数・`*args` / `**kwargs` を渡すもの（`__py_super`）は
  `VM_BUILTIN_KW_NAMES` と `eval_builtin_evaled_named`（`expand_spread_args`）にも足す
- 型検査の戻り値（`call_check.rs` の `builtin_fn_return`）は `None` を返すものだけ書く（`__py_delattr` など）
- 変換の対応表は `object_method_builtin`（`expressions.rs`・`object.m(..)` と基底が `object` の `super().m(..)` の両方）

### 2.2 「Python のクラスだけ」の判定

Python の振る舞いを Arrow のクラスへ広げないために、3 つの印を使い分ける。

| 印 | 立てる所 | 使う所 |
|---|---|---|
| `ClassValue::is_python`（実行時） | `exec_class_def`（`in_python_module` のとき） | 属性の上書き（`py_attr_hook`）・既定の `==` / `hash`（#1）・インスタンス経由の静的メソッド（#5） |
| `Interpreter::running_python`（実行時） | `vm::run` / `vm::resume_frame`（`Chunk::is_python`）・`exec_module` | 格納で写すか（`copy_for_store`・#1） |
| `TypeRegistry::python_class_names`（型検査） | `import[py]` のモジュールの本体を集める間（`collecting_python`） | `StaticMethodOnInstance` を出さない（#5） |

- 型検査のその他の判定は「上書きを持つクラスか」で見る（`class_has_setattr_hook` / `getattr_hook_type`）。上書きを
  持てるのは Python のクラスだけなので（2.4）、Python のクラスかを別に見る必要が無い

### 2.3 属性の上書きの順序（CPython と同じ・#10〜#12）

```
読み  get_attr_val
        ├─ __getattribute__ があれば それ（無ければ get_attr_without_hook ＝ object.__getattribute__）
        └─ AttributeError（is_attribute_error）で __getattr__ があれば それ
代入  attr_assign_evaled / set_attr_val
        └─ __setattr__ があれば それ（無ければ attr_assign_without_hook ＝ object.__setattr__）
消す  del_attr_val
        └─ __delattr__ があれば それ（無ければ データディスクリプタの __delete__ → フィールドを未代入に）
呼ぶ  call_instance_method_evaled（明示の o.m(..)）
        └─ __getattribute__ があれば それで読んでから呼ぶ（o.__len__() も）
           ⚠ str(o) / len(o) / 演算子 / for が内部で引く特殊メソッドは通さない（CPython も型から直接引く）
```

- 判定は `Interpreter::py_attr_hook(クラス, 名前)`（`is_python` かつメソッドの表にある・基底のものも平坦化済み）
- 明示の呼び出しかどうかは、**ソースの呼び出しだけがメソッドのキャッシュを渡す**ことで見分ける（VM の `CallMethod*`・
  ツリーウォークの `eval_method_call`。内部の呼び出しは `None`）
- フィールド・メソッドのキャッシュは上書きの先（`get_attr_without_hook` の中・通常のメソッド呼び出しの終わり）でしか
  張らないので、上書きを持つクラスには張られず、毎回上書きを通る
- 上書きを呼ぶ自分自身（`__getattribute__` など）は内部の呼び出しで呼ぶ（明示の形にすると `__getattribute__` を読むのに
  `__getattribute__` を呼んで回り続ける）

### 2.4 Arrow のクラスでは定義できない上書き

`fn __setattr__` / `fn __delattr__` / `fn __getattr__` / `fn __getattribute__` は Arrow のクラス・trait ではパースの誤り
（`parser/classes.rs` の `reject_attr_hook`・`fn` の位置を指す）。Arrow のメンバーはクラスの本体で決まる。変換器が作る
Python のクラスはこのパーサを通らない。⚠ `__getattribute__` は以前、Arrow のクラスに書くとただのメソッドだった。

### 2.5 実行時の誤りに値を添える控え（#9）

実行時の誤りは文字列（`"KeyError: 'zz'"`）で運ばれ、例外のインスタンスになるのは捕まえる所・関数の境界
（`make_internal_raised_error`）。そこではキーの値が分からないので、送出の場が `Interpreter::error_args` に
「文言と引数」を控え（`error_with_args` / `key_error` / `key_error_text` / `os_error`）、例外にするとき**文言が完全に
一致すれば**その引数を使う。誤りは文字列のままなので、位置・トレースバックは変わらない。

---

## §3 項目ごとの記録

### #1 参照の共有（`742b9cb4`）

- **直す前**: Arrow は値を格納するたびに写す（`let` の値を共有先から書き換えさせないため）ので、Python のコードの
  別名からの書き換え・登録した物の書き換え・親子の参照が見えなかった。`__eq__` の無いクラスの `==` は中身で比べた
  （`P(1) == P(1)` が真・CPython は偽）。実測 12 項目のうち 9 項目が CPython と違った
- **決定**: A 案 — Python のコードの中では写さない。Python のクラスの既定の `==` / `hash` は同一性。Arrow のソースは
  今のまま写す
- **実装**: 格納の暗黙の写し 13 か所（`vm/run.rs` の `StoreLocalDeepCopy` / `ListAppendLocal` / `StoreCellDeepCopy`・
  `set_methods.rs` の `add`・`method_call.rs` の `append`・`vm_toplevel.rs` の `mut`・`eval/attrs.rs` の 3 か所・
  `eval/subscript.rs`・`exec/vars.rs`・`exec/blocks.rs`・`ops/hash.rs` の `dict_set`（キーと値）・`exec/dispatch.rs`）を
  `Interpreter::copy_for_store` に通し、`running_python` の間は写さない。`ClassValue::is_python` のクラスで
  `__eq__` / `__hash__` が無ければ `values_eq` / `hash_into` が同一性
- **残した差**: Arrow の `let`（`frozen_copy`）・`self` の束縛・ネイティブ呼び出し・`copy()` は写す。Arrow の `let` /
  `mut` に Python のインスタンスを束縛するときも写す（境界は Arrow の規則）
- **確かめ**: 12 項目が CPython と同じ。全例題の出力は変わらなかった。単体テスト `copy_for_store_shares_inside_python_code`

### #2 / #3 `del`（`8fd23ef4`）

- **直す前**: 変換器が `del o.a` / `del d[k]` を変換の誤りにしていた（`del x` は警告して無視）。pandas（tests を除く）で
  名前 16・添字 27・属性 3 か所（`Series._reset_cacher` の `del self._cacher` と `hasattr` の組など）
- **決定**: 属性の `del` はフィールドを未代入に戻す。名前の `del x` は今までどおり読み飛ばす。コレクションの削除は実装する
- **実装**:
  - 変換器 `convert_del_target`（`statements.rs`）: `del o.a` → `__py_delattr(o, "a")`、`del d[k]` → `__py_delitem(d, k)`、
    `del a, b[0]` / `del (a, b)` は左から 1 つずつ
  - `del_attr_val`: `__delattr__`（#10 で Python のクラスだけに）→ データディスクリプタの `__delete__`（`__set__` だけなら
    `AttributeError: __delete__`）→ フィールドを `InstanceData::clear_field` で未代入に戻す（boxed は欄を `None`、raw は
    init のビットマップのビットを落とす。インスタンスの大きさは変わらない）。未代入のフィールドは前から CPython の「無い
    属性」と同じ（読みは `AttributeError`・`hasattr` は偽・`cached_property` は計算し直す・また代入できる）
  - `eval_delitem`（`subscript.rs`）: 辞書（無ければ `KeyError`）・リスト（負の添字・スライス・負のステップ・範囲の外は
    `IndexError: list assignment index out of range`）・`__delitem__`。タプル・文字列・集合などは CPython と同じ `TypeError`
- **残した差**: 不変（`let` / `freeze`）のインスタンスの属性は消せない（`TypeError`）。クラス・モジュールの属性
  （`del C.x` / `del m.x`）は消せない（CPython は消せる）。クラスの本体の `del` は変換の誤り
- **確かめ**: 単体テスト `python_del_becomes_delattr_and_delitem_calls` / `test_py_delattr_resets_field_in_both_layouts`。
  変換の誤りの例だった `py_del_subscript_error.py` を消し、`py_stmt_errors_error.ar` の ⑦ を「変換するようにした」に直した

### #4 `%` の書式（`16c931eb`）

- **直す前**: `"%s|%r" % (T(), T())` が `<T object at 0x..>|"<T object at 0x..>"`（CPython は `t|T()`）。`%r` が Rust の
  書式で引用していた（`"%r" % "ab"` が `"ab"`）
- **実装**: `percent_format`（`str_methods.rs`）が `%s` / `%r` の値を文字列にするやり方を受け取る（`PercentText`）。
  `apply_binop_dyn` が `str()` / `repr()`（`__str__` / `__repr__` を呼ぶ）で答える。`%r` にも精度（`%.2r`）が効く
- **残した差**: インタプリタを回せない経路（ネイティブのコールバックの `apply_binop`）は既定の表示（呼ばない）

### #5 インスタンス経由の静的メソッド・クラスメソッド（`03ebe04f`）

- **直す前**: 変換器は前から `@staticmethod` / `@classmethod` を `static fn` / `class_method fn` に読み替えていたが、Arrow の
  規則（クラス経由だけ）が `self.helper(3)` を `AttributeError` にしていた（pandas の `self._validate(..)` の形）
- **決定**: A 案 — Python のクラスだけインスタンス経由を許す。Arrow のコードから Python のインスタンス経由で呼ぶ形も許す
- **実装**: `call_instance_method_evaled` がクラス経由の呼び出し（`eval_class_method`）へ回す（静的メソッドは `self` を
  渡さず、クラスメソッドは `cls = type(self)`）。値として読む形は、静的メソッドなら束縛しない関数、クラスメソッドなら
  新しい `BoundTarget::ClassMethod`（受け手はクラス）。型検査は `python_class_names` のクラスに `StaticMethodOnInstance`
  を出さない
- **確かめ**: メソッドの中・外からのインスタンス経由・派生クラスの `cls`・値として読む形・関数の形の `staticmethod(f)`・
  読んだ値の同一性（`q.double is q.double`・`q.make == q.make`）が CPython と同じ。Arrow のクラスは静的な誤りのまま

### #6 `super()`（`9aca9472` 調査・`2e07586b`）

- **直す前**（調査で測った 8 形）: CPython と同じなのは「インスタンスメソッドから基底のインスタンスメソッド」だけ。
  クラスメソッドの中の `super()` は `NameError: 'self' is not defined`、`super(Sub, cls).kind()` と「インスタンスメソッド
  から基底のクラスメソッド」は `TypeError: function takes 1 positional argument(s), got too many`。原因は ① 変換器が受け手を
  `self` と決め打ちしていた ② 書き換え先の `Base.m(受け手, ..)` は、`m` がクラスメソッドなら `cls` に `Base` が足されて
  受け手が余分な引数になる（利用者が書いた `Base.m(x)` と区別できない）。pandas はクラスメソッドの中の `super()` が 11 か所
- **決定**: Python 専用の `super()` の組み込みを足し、`super()` の書き換えをすべてそれにする
- **実装**: 変換器が `__py_super(第 1 基底, 受け手, "m", ..)` にし、引数なしの `super()` の受け手を今の関数の最初の仮引数に
  する（`supers.rs` の `SuperReceiverGuard`。メソッドと入れ子の関数の本体で積む）。`py_super_call`（`class_methods.rs`）:
  クラスメソッドは `cls` に受け手（インスタンスならそのクラス）を渡して**基底の関数そのもの**を呼ぶ（名前で引き直すと
  派生のメソッドへ戻って回り続ける）、静的メソッドは受け手を渡さない、受け手がクラスでインスタンスメソッドなら `self` を
  渡さない、ほかは以前の `Base.m(受け手, ..)`（`eval_class_method`・例外の `__init__`・`object` のメソッドへの落ち先）
- **残した差**: 静的メソッド・仮引数の無い関数の中の `super()` は変換の誤り（CPython も実行時の誤り）。探すのは第 1 基底と
  その祖先だけ（多重継承の 2 つ目以降は見ない・以前と同じ）
- **確かめ**: 8 形とキーワード・`*args` / `**kwargs` が CPython と同じ。既存の `super()` の例題の出力は変わらなかった
  （バイトコードは `CALL_BUILTIN __py_super` に変わった）。単体テスト `python_super_becomes_py_super_with_the_first_param`

### #9 実行時の誤りの `args`（`1122903a`・`d069c252`）

- **直す前**: `d["zz"]` の `KeyError` の `repr(e)` が `KeyError("'zz'")`・`e.args` が `("'zz'",)`（CPython は `KeyError('zz')` /
  `('zz',)`）。`open` の失敗は `cannot open 'p': <OS の文言>` で、`errno` / `strerror` / `filename` が無かった
- **決定**: 送出の場に「文言と引数」の控えを置く（2.5）。`OSError` も #9 の一部として同じ仕掛けで
- **実装**:
  - `KeyError`: 7 か所（`d[k]`・`del d[k]`・`d.pop(k)`・`s.remove(x)`・`"{k}".format()`・空の `d.popitem()` / `s.pop()`）が
    `key_error` / `key_error_text` で控える。`"{k}".format()` の文言もキーの `repr` にした
  - `OSError` の系統（別名・派生・Python の派生クラス）に `errno` / `strerror` / `filename` / `filename2` の欄を足した（既定
    `None`・書き換えられる・`OS_ERROR_FIELDS`。⚠ 欄の並びは `make_error_class` と、Python のクラスが継承するときの
    `py_class_field_order` の 2 か所で揃える）。`exc_set_args` が `OSError(errno, strerror[, filename[, winerror[,
    filename2]]])` を CPython の `oserror_init` と同じに読む（`filename` が `None` でなければ `args` は最初の 2 つ・
    `str(e)` は `[Errno 2] x: 'f'`）
  - `open` の失敗は `open_error_kind` → `os_error` で控え、文言を CPython と同じ `[Errno 2] No such file or directory: 'p'` に
    した（errno と文言は C の値。Windows でディレクトリを開くと `PermissionError` の 13）
  - `OSError(2, "..")` は errno に対応する派生を作る（`instantiate_evaled` の `os_error_subclass_for`・表は
    `built_in_types::os_error_subclass_of`。OS ごとに違う番号は分けた。Windows の CPython 3.12 で全番号を作って確かめた）
  - 型検査: `errno: int | None` / `strerror: str | None` / `filename` / `filename2: Any`
- **残した差**: 控えが残る（Rust の中で誤りを捨てた）と、同じ文言の次の誤りが拾う（＝同じ `repr` のキーだけ）。async の
  タスクのスレッドを越える誤りは文言の `args` のまま。4 つ目の `winerror` は持たない（`None` 以外は `TypeError`）。作った後で
  `e.errno` を書き換えても `str(e)` は変わらない（CPython は変わる）。Windows の CPython の 5 つの形の `[WinError None]` は
  真似しない。`open` の 5 種類の外の失敗は `IOError` で errno は OS の番号
- **確かめ**: `KeyError` 13 項目・`OSError` の `open` 3 種類・作り方 12 通り・Python の派生クラスが CPython と同じ。既存の
  例題の出力は変わらなかった（`open` の失敗の文言を出す例題が無かった）。単体テスト
  `test_error_args_are_used_only_for_the_same_message` / `test_os_error_errno_picks_subclass`

### #10 `__setattr__` / `__delattr__`（`c5a31eaa`）

- **直す前**: Python のクラスの `__setattr__` が黙って呼ばれず、`__init__` の `self.x = 1` がそのままフィールドへ入った。
  `object.__setattr__(self, k, v)` は実行時の誤り。pandas は `NDFrame.__setattr__` など 4 か所で定義し、
  `object.__setattr__` を約 30 か所で呼ぶ
- **決定**: Python の読み込みでだけ使える属性の操作の方法にする（`__delattr__` も同じ規則）
- **実装**: 2.3 の「代入」「消す」。`object.__setattr__` / `super().__setattr__` は `__py_object_setattr`。基底が Python の
  クラスで上書きしていなければ `Base.__setattr__(self, ..)` が `object` のものへ落ちる（`eval_class_method`）。
  `object.__setattr__(self, "x", v)` の名前が文字列の定数ならフィールドとして宣言する（`bypass_setattr_name`）。型検査は
  上書きを持つクラスへの代入で宣言していない名前を許す（`attr_is_assign_target`）。`cached_property` が覚える値は
  `__py_object_setattr` で入れる（以前の `setattr` では上書きを通ってしまう・CPython は `__dict__` へ直に入れる）
- **残した差**: 名前が変数の `object.__setattr__(self, name, v)` は、ほかで宣言したフィールドにだけ入る（Arrow の
  インスタンスに `__dict__` は無い）

### #11 `__getattr__`（`97820e01`）

- **直す前**: `__getattr__` が黙って呼ばれず、`getattr(d, "zz", "default")` が既定値・`hasattr` が偽を、誤りを出さずに返した。
  pandas は 13 か所で定義（`NDFrame.__getattr__` は `df.col`）
- **実装**: 2.3 の「読み」の 2 段目。無い名前のメソッドの呼び出し `o.k(..)` は `__getattr__` で読んで呼ぶ。型検査は
  `getattr_hook_type`（`__getattr__` の戻り値の注釈・無ければ `Any`）で宣言していない名前を通す
- **ついでに直した**: `getattr` の既定値・`hasattr` は誤りの文言の頭しか見ておらず、Python の `raise` で送出した
  `AttributeError`（property の getter・`__getattr__` が投げたもの）を受け止めなかった。`is_attribute_error`（文言と、
  `current_exception` の例外の両方を見る）に揃えた

### #12 `__getattribute__`（`4d8a6396`）

- **直す前**: `__getattribute__` がどこからも呼ばれず、Arrow のクラスに書くと普通のメソッドだった。pandas は 1 か所
  （`ArrowStringArrayMixin.__dict__` と `functools.partial` に頼るので、今の変換器ではもともと読めない）
- **決定**: A 案 — Python のクラスだけ CPython と同じに呼ぶ。特殊メソッドを明示して呼ぶ形（`o.__len__()`）も通す
- **実装**: 2.3 の「読み」の 1 段目と「呼ぶ」。型検査は `getattr_hook_type` が `__getattribute__` も見る
- **確かめ**: 上書きが受け取った名前を記録する例題で、明示の読み・呼び出しだけが記録され、`str()` / `len()` / `+` / `for`
  が記録されないことを CPython と比べた

---

## §4 発見ログ

- **実行時の誤りは文字列で運ばれる**。値（`KeyError` のキー・`OSError` の errno）を例外に載せるには、送出の場で控える
  しかなかった（#9）。送出の場で例外のインスタンスを作る形（`mustbe_check` と同じ）は、文字列の誤りに付く「失敗した文の
  位置」（`note_error_site`）が付かなくなり、トレースバックが変わるので採らなかった
- **Python の `raise` は文言にならない**。送出した例外は `RAISE_SENTINEL` と `current_exception` で運ばれるので、
  `e.starts_with("AttributeError")` のような判定は受け止めない（#11 で `getattr` / `hasattr` の既存の取りこぼしが見つかった）。
  例外の種類で分けるときは `is_attribute_error` / `is_stop_iteration` の形にする
- **明示の呼び出しと内部の特殊メソッドの呼び出しは、メソッドのキャッシュの有無で見分けられる**（#12）。ソースの
  呼び出し点（VM の 3 つの op・ツリーウォークの 1 か所）だけがキャッシュを渡す。新しい呼び出し点を足すときはこの約束を
  守ること
- **変換の書き換え先が利用者の書く形と同じだと、実行時に直せない**。`super().m()` → `Base.m(self)` は `Base.m(x)` と
  見分けられなかった（#6）。Python の意味を持つ書き換えは専用の内部の組み込みにする（2.1）
- **組み込みのクラスを Arrow で書くと、Python の上書きを通ってしまうことがある**。`cached_property` が `setattr` で値を
  入れていたので、`__setattr__` を上書きしたクラスでは上書きが呼ばれた（#10 で `__py_object_setattr` に）
- **組み込みの例外の欄の並びは 2 か所にある**（`make_error_class` と、Python のクラスが継承するときの
  `py_class_field_order`）。片方だけ足すと、Python の派生クラスで `e.errno` が `AttributeError` になった（#9 の例題で発見）
- **例題の名前がぶつかる**。`py_getattr` は `getattr()` の例題として既にあった（#11 は `py_getattr_hook` にした）。作る前に
  同じ名前を探す
- **ゲートの揺れ**（どれも走らせ直して確かめた）:
  - 同じ exe で毎回バイトコードの dump が違う例題がある（`async_cs_calls`・`async_native_call`・`import_namespace`・
    `js_proc_values`・`rs_native_arg_check`）。基準の exe 同士で比べて揺れと確かめてから無視する
  - C++ の中継 DLL のビルドが一時的に失敗する（`LNK1104`・一時ファイルが開けない）。そのとき C++ / Rust の例題の出力が
    A/B で違い、`compare_python_impl` が `cpp_raw_pointers` の既知の差を「もう同じ」と言う（Arrow 側も失敗して
    `impl_python` と同じ空の出力になった）。走らせ直すと元に戻る
  - PowerShell で 2 つの exe の出力を `2>&1 | Out-String` で比べると、誤りの表示に呼び出した命令の文字列が入るので
    必ず違う。`cmd /c` でファイルへ書いて比べる
