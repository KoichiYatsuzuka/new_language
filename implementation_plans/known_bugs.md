# 既知の不具合と残タスク

Python のコードを Arrow で動かす作業（[python_builtins_plan.md](python_builtins_plan.md)）の残りと、その作業中に見つけた
不具合をまとめる。

- 起票: 2026-10-22。python_builtins_plan.md の残りのタスク（1-18 / 3-7 / 6-4 / 6-9）をここへ移し、作業中に見つけて
  起票していなかった不具合（#4〜#6・#8）を足した。移したタスクは python_builtins_plan.md から本書を指す。
- 測定: Arrow `11362ae`・CPython 3.12.2（どれも release ビルドで実際に動かして確かめた）。⚠ `11362ae` は master への
  rebase 前の履歴（今のブランチには無い）。
- 再測定: master（`326e112c`）へ rebase した後の `ac5df650`（2026-10-22）で全項目を動かし直した。#1〜#6 は変わらず、
  #7 / #8 は master の外部言語との値の受け渡しの改修（ffi_value_exchange_plan.md）で文言・現状が変わったので直した。
  ⚠ master の決定（7-27 / 7-30）で、`import[py-int]` の注釈の無い関数の戻り値は `Any` になった。#7 / #8 の形を
  `m.f() === Ellipsis` / `1 + m.f()` と素直に書くと、実行の前に `cannot apply '===' to 'Any' — explicit downcast
  required` の静的な誤りで止まる。実行時の振る舞いは、注釈・スタブで型の付いた値や、タプルから添字で取り出した値
  （`let b = m.boxed()` の `b[0]`）で確かめた。
- 採番は `#1, #2, ..`（フェーズに分けない）。「前提」が「なし」のものは、ほかと独立に着手できる。
- ⚠ 要判断: 直し方に利用者の決定が要るもの。決定したら本文に決定と日付を書く。

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| **1** | ⚠ 要判断: Python のクラスのインスタンスの共有（旧 6-9）。Arrow は値を格納するたびに写す（フィールド・辞書のキーと値・`append`・`xs[i] = ..`・集合の `add`・`mut` の宣言・クロージャの取り込み。`let` の値を共有先から書き換えさせないため・`deep_copy_value`）が、CPython は参照を共有する。実測: `__eq__` の無いクラスの `==` は、Arrow は中身で比べ（`E() == E()` / `P(1) == P(1)` が `True`）、CPython は同一性で比べる（`False`）。利用者の決定: **すべてのオブジェクトの既定の `__eq__` をアドレスの比較にする**。⚠ これは写すのをやめる（か、写しても同じ物と分かるようにする）のが前提。写したまま既定の `==` を同一性にすると、`xs.append(p)` の後の `p in xs` が偽になり、辞書のキーにしたインスタンスも引けなくなる。⚠ `object()` / `Ellipsis` / `NotImplemented` は写さず同一性で比べる（6-10〜6-12 で対応済み）。⚠ CPython との境界で list / dict を作り直す件は現状を維持する（2026-10-20 の決定） | なし | 大 |
| **2** | ⚠ 要判断: `del obj.x`（旧 6-4）。ディスクリプタの `__delete__`（property の `fdel`）を呼ぶ入口が無い。Arrow に属性を消す構文が無く、変換器は `del <expr>.attr` を変換の誤りにしている。データディスクリプタの `del` だけを `type(obj).x.__delete__(obj)` に写すか、Arrow に属性の削除（フィールドを未初期化に戻す）を足すか（属性はクラス本体で固定なので「消す」概念が無い・python_builtins_plan.md 4.3 の `delattr`） | なし | 中 |
| **3** | ⚠ 要判断: `del d[k]`（旧 3-7）。Arrow に `del` の構文が無く（`ParseError: unexpected token: del`）、変換器は Python の `del d[k]` を変換の誤りにしている。`del` を文として足すか、変換器が `d.pop(k)` などへ写すか。⚠ #2 と同じく `del` をどう扱うかの判断なので、一緒に決めるとよい | なし | 小 |
| **4** | `%` の書式の `%s` / `%r` がインスタンスの `__str__` / `__repr__` を呼ばない（6-12 の作業中に見つけた）。実測: `"%s\|%r" % (T(), T())`（`__str__` が `t`・`__repr__` が `T()`）が、CPython は `t\|T()`、Arrow は `<T object at 0x..>\|"<T object at 0x..>"`。`str(x)` / f-string / `str.format` は呼ぶ（1-15 / 3-4）ので、`%` だけが外れている。原因は `%` の書式（`str_methods::percent_format`）がインタプリタを持たず、値の既定の表示しか使えないこと。`apply_binop_dyn` の側で `%s` / `%r` に渡す値を先に `str()` / `repr()` にするなどで直せる | なし | 小 |
| **5** | ⚠ 要判断: 静的メソッド・クラスメソッドをインスタンスから呼ぶ形（`self.helper(..)`）が実行時の誤り（6-13 の作業中に見つけた）。実測: Python のクラスの `@staticmethod def helper(x)` を `self.helper(3)` で呼ぶと `AttributeError: static method 'helper' must be called on the class, not an instance`（CPython は `6`）。`@classmethod` も同じ（`self.make()` が `AttributeError`・CPython は `cls` が `type(self)` で呼べる）。デコレータの形・関数の形（6-13）のどちらも同じ。Python ではよく使う形（pandas の `self._validate(..)` など）。原因は Arrow の規則（`classes/method_call.rs` の `call_instance_method_evaled`）。Python のクラス（`FnValue::is_python`）だけ許すか、Arrow の規則そのものを変えるか | なし | 小 |
| **6** | `@classmethod` の中の `super()` が `NameError: 'self' is not defined`（6-2 の作業中に見つけた）。実測: `class Sub(Base)` の `@classmethod def kind(cls): return "Sub<" + super().kind()` を呼ぶと、CPython は `Sub<Base`。変換器は `super().m(..)` を `<第 1 基底>.m(self, ..)` に書き換えるので、`self` の無いクラスメソッドでは名前が無い。2 引数の `super(Sub, cls)` も同じ書き換えで、`Base.kind(cls)` は `cls` を余分な引数として渡してしまう。⚠ 基底のクラスメソッドを、`cls` を派生クラスのまま呼ぶ手段が Arrow に要る（要調査） | なし | 中 |
| **7** | CPython の物（`import[py-int]` で受け取った値）どうしの `is`（Arrow の `===`）が `==` を呼んでいる（旧 1-18）。実測: `__eq__` が常に `True` を返すクラスの別々の 2 つで `a === b` が `True`（CPython の `a is b` は `False`）。CPython の物と Arrow の値（`m.p() === Ellipsis`）は、`False` ではなく `TypeError: cannot pass an Arrow value of type 'ellipsis' to Python`（rebase 後に再測定。以前の文言は `cannot convert tl value of type 'unknown' to Python` で、master の ffi_value_exchange_plan.md 2-6 で型の名前が出るようになった）。原因は `apply_binop` が片方が CPython の物なら演算子を見る前に `py_binop` / `py_rbinop` へ回し、そこが `RefEq` を `Eq` と同じ腕で扱うこと（`py_interop.rs`）。直し方: `===` は回す前に判定する（両方が CPython の物なら中身の物が同じか、片方だけなら `False`）。⚠ 関連: 値の等しさ（`values_eq`）は CPython の物どうしを Arrow の包みの同一性（`Arc::ptr_eq`）で比べている（同じ CPython の物を 2 回受け取ると別の物とされうる・未計測）。⚠ CPython との値のやり取りの議題（#8）に含めるかは利用者の判断（2026-10-20 に「今回は扱わない」と決定） | なし | 小 |
| **8** | ⚠ 要判断: CPython との値のやり取り（`import[py-int]`・2026-10-20 に「別の議題として今回は扱わない」と決定）。6-10〜6-12 の検討で見つけた、境界で CPython と違う振る舞い（どれも実測・rebase 後に再測定）: ① CPython から来た `Ellipsis` / `NotImplemented` は CPython の物の包みのままで、Arrow の `Ellipsis` / `NotImplemented` と対応しない（`x is Ellipsis` が成り立たない。#7 の形では `===` / `==` のどちらも `TypeError: cannot pass an Arrow value of type 'ellipsis' to Python`・`NotImplemented` は `.. type 'NotImplementedType' ..`） ② 左が Arrow の値・右が CPython の物の算術（`1 + V()`）は右の `__radd__` を直接呼ぶ（`py_rbinop`）ので、`NotImplemented` が値として出てくる（CPython は `TypeError: unsupported operand type(s) for +: 'int' and 'V'`）。右に `__radd__` が無いと `ForeignError: AttributeError: 'W' object has no attribute '__radd__'`（CPython は同じく `TypeError`。master の ffi_value_exchange_plan.md 7-24 で Python が投げた例外は `ForeignError` に揃った） ③ Arrow のインスタンス・関数を CPython へ渡せない（`TypeError: cannot pass an Arrow value of type 'Pt' to Python` / `.. type 'function' ..`）。⚠ 以前は型の名前の無い `cannot convert tl value of type 'unknown' to Python` で、**集合**も渡せなかったが、master の ffi_value_exchange_plan.md 2-6 で型の名前が出るようになり、集合は双方向に変換されるようになった（`{3, 1, 2}` を渡すと Python の `set` として届く・rebase 後に確認）。Arrow で作った `object()` は無い（Python のソースからだけ作れる・6-10）ので、渡せないまま残っているのは Python のソースの番兵・Arrow のクラスのインスタンス・呼び戻しの関数 | #7 | 大 |

## 参考: 決定どおりで、起票しない CPython との差

不具合ではなく、決定や Arrow の規則に従った結果。同じものを不具合として起票し直さないための控え。

- `type(o)()`（`object()` のクラスを値として呼ぶ）は `TypeError`。`object()` を作れるのは Python のソースの `object()` だけ（6-10 の決定）。
- `object()` は Arrow の `async` に渡すと別の物になる（スレッドへ送る複製は `Rc` を共有できない・6-10）。`Ellipsis` / `NotImplemented` は種類で比べるので同じ物のまま。
- `dict.copy()` は深い写し（CPython は浅い写し）。浅いと `let` の辞書の中のリストを写しの側から書き換えられる（3-6）。
- 注釈の `object` は静的には `Any`。Arrow の `Any` は使う前に型を確かめる必要があるので、`object` と注釈した値どうしの `===` も静的な誤りになる（6-10）。
