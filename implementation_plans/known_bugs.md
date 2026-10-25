# 既知の不具合と残タスク

Python のコードを Arrow で動かす作業（[python_builtins_plan.md](python_builtins_plan.md)）の残りと、その作業中に見つけた
不具合をまとめる。

- 起票: 2026-10-22。python_builtins_plan.md の残りのタスク（1-18 / 3-7 / 6-4 / 6-9）をここへ移し、作業中に見つけて
  起票していなかった不具合を足した。移したタスクは python_builtins_plan.md から本書を指す。
- **済んだもの**（#1〜#6・#9〜#12・2026-10-23〜26）の実装の記録は
  [implementation_logs/KNOWN_BUGS_LOG.md](../implementation_logs/KNOWN_BUGS_LOG.md) へ移した。**番号はそのまま**
  （ソースのコメントの「known_bugs.md の #N」は、済んだものならそちらの #N）。
- 残りの #7 / #8 は `import[py-int]`（CPython 経由）の話で、**2026-10-26 に「今回は扱わない」と決定**（2026-10-20 にも
  「CPython との値のやり取りは別の議題」と決定）。
- 測定: Arrow `11362ae`・CPython 3.12.2（release ビルド）。master（`326e112c`）へ rebase した後の `ac5df650`（2026-10-22）で
  測り直し、#7 は `2e07586b`（2026-10-26）でも測り直した。
  ⚠ master の決定（7-27 / 7-30）で、`import[py-int]` の注釈の無い関数の戻り値は `Any` になった。#7 / #8 の形を
  `m.f() === Ellipsis` / `1 + m.f()` と素直に書くと、実行の前に `cannot apply '===' to 'Any' — explicit downcast
  required` の静的な誤りで止まる。実行時の振る舞いは、注釈・スタブで型の付いた値や、タプルから添字で取り出した値
  （`let b = m.boxed()` の `b[0]`）で確かめた。
- 採番は `#1, #2, ..`（フェーズに分けない）。「前提」が「なし」のものは、ほかと独立に着手できる。
- ⚠ 要判断: 直し方に利用者の決定が要るもの。決定したら本文に決定と日付を書く。

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| **7** | CPython の物（`import[py-int]` で受け取った値）どうしの `is`（Arrow の `===`）が `==` を呼んでいる（旧 1-18）。**再測定（2026-10-26・`2e07586b`）**: `__eq__` が常に `True` を返すクラスで、別々の 2 つの `a === b`・`a === None`・`a === 1` がどれも `True`（CPython の `is` はどれも `False`）。同じ物を 2 回受け取った `s1 === s2`・`a === a` は `True`（CPython と同じ）。`__eq__` の無いクラスは、既定の `__eq__` が同一性なので結果は合う（`===` が正しいわけではない）。CPython の物と Arrow の値（CPython から来た `...` と Arrow の `Ellipsis`）は `TypeError: cannot pass an Arrow value of type 'ellipsis' to Python`（CPython は `True`・#8 の ①）。**あわせて見つけた**: 同じ CPython の物を 2 回受け取ると辞書のキーとして引けない（`{p1: "v"}.get(p2)` が `None`・`hash(p1) == hash(p2)` が偽。CPython は `'v'` / 真）。集合の `p2 in {p1}` とリストの `p2 in [p1]` は `==` で探すので引ける。CPython の関数へ渡し返して CPython で `a is b` を判定すると正しい。原因: ① `ops/operators.rs`（`apply_binop`）が片方が CPython の物なら演算子を見る前に `py_binop` / `py_rbinop` へ回し、`py_interop.rs` がそこで `RefEq` を `Eq` と同じ腕で扱う ② `ops/hash.rs` が CPython の物を Arrow の包みの番地でハッシュし、`ops/equality.rs` が包みの同一性（`Arc::ptr_eq`）で比べる（受け取るたびに包みを作り直すので、同じ物でも別のキーになる）。直し方の案: ① `===` は回す前に判定する（両方が CPython の物なら中身の物が同じか、片方だけなら `False`） ② ハッシュと等しさを中身の物で決める（最小は中身の物の番地。CPython の `__hash__` / `__eq__` まで使うかは別の判断・辞書の操作の途中で CPython の誤りが起きうる）。**決定（2026-10-26）: `import[py-int]` 経由なので今回は扱わない** | なし | 小 |
| **8** | ⚠ 要判断: CPython との値のやり取り（`import[py-int]`・2026-10-20 に「別の議題として今回は扱わない」と決定・2026-10-26 にも同じ決定）。6-10〜6-12 の検討で見つけた、境界で CPython と違う振る舞い（どれも実測・rebase 後に再測定）: ① CPython から来た `Ellipsis` / `NotImplemented` は CPython の物の包みのままで、Arrow の `Ellipsis` / `NotImplemented` と対応しない（`x is Ellipsis` が成り立たない。#7 の形では `===` / `==` のどちらも `TypeError: cannot pass an Arrow value of type 'ellipsis' to Python`・`NotImplemented` は `.. type 'NotImplementedType' ..`） ② 左が Arrow の値・右が CPython の物の算術（`1 + V()`）は右の `__radd__` を直接呼ぶ（`py_rbinop`）ので、`NotImplemented` が値として出てくる（CPython は `TypeError: unsupported operand type(s) for +: 'int' and 'V'`）。右に `__radd__` が無いと `ForeignError: AttributeError: 'W' object has no attribute '__radd__'`（CPython は同じく `TypeError`。master の ffi_value_exchange_plan.md 7-24 で Python が投げた例外は `ForeignError` に揃った） ③ Arrow のインスタンス・関数を CPython へ渡せない（`TypeError: cannot pass an Arrow value of type 'Pt' to Python` / `.. type 'function' ..`）。⚠ 以前は型の名前の無い `cannot convert tl value of type 'unknown' to Python` で、**集合**も渡せなかったが、master の ffi_value_exchange_plan.md 2-6 で型の名前が出るようになり、集合は双方向に変換されるようになった（`{3, 1, 2}` を渡すと Python の `set` として届く・rebase 後に確認）。Arrow で作った `object()` は無い（Python のソースからだけ作れる・6-10）ので、渡せないまま残っているのは Python のソースの番兵・Arrow のクラスのインスタンス・呼び戻しの関数。**④ 追加の発見（2026-10-26・#7 の測定中）**: 型検査の型の名前の食い違い。`let xs: list[m.Plain] = [x]` のあと、戻り値の注釈が `-> Plain` の `m.plain()` で受け取った値を `y in xs` と書くと `StaticTypeError: 'in' between 'Plain' and 'm.Plain' can never be true`（誤った静的な誤り。`let x: m.Plain = m.plain()` は通る）。`in` の検査だけが、スタブの素の名前 `Plain` と別名つきの `m.Plain` を別の型と見ている | #7 | 大 |

## 参考: 決定どおりで、起票しない CPython との差

不具合ではなく、決定や Arrow の規則に従った結果。同じものを不具合として起票し直さないための控え（#N は
[KNOWN_BUGS_LOG.md](../implementation_logs/KNOWN_BUGS_LOG.md) の番号。項目ごとの「残した差」はそちらの §3）。

- `type(o)()`（`object()` のクラスを値として呼ぶ）は `TypeError`。`object()` を作れるのは Python のソースの `object()` だけ（6-10 の決定）。
- `object()` は Arrow の `async` に渡すと別の物になる（スレッドへ送る複製は `Rc` を共有できない・6-10）。`Ellipsis` / `NotImplemented` は種類で比べるので同じ物のまま。
- `dict.copy()` は深い写し（CPython は浅い写し）。浅いと `let` の辞書の中のリストを写しの側から書き換えられる（3-6）。
- `del x`（名前）は警告して無視する。消した後に読んでも `NameError` にならない（Arrow はスコープを抜けるときに破棄する・#2 の決定）。
- `del C.x` / `del m.x`（クラス・モジュールの属性）は `TypeError`（CPython は消せる）。Arrow のクラスの本体・モジュールの大域は定義で決まる（#2）。
- 注釈の `object` は静的には `Any`。Arrow の `Any` は使う前に型を確かめる必要があるので、`object` と注釈した値どうしの `===` も静的な誤りになる（6-10）。
- Arrow の `let` / `mut` に Python のインスタンスを束縛すると写す（境界は Arrow の規則・#1）。Arrow のクラスは中身で比べる。
- 属性の操作の上書き（`__setattr__` / `__delattr__` / `__getattr__` / `__getattribute__`）は Python のクラスだけ。Arrow のクラスに書くとパースの誤り（#10〜#12）。
- `str(o)` / `len(o)` / 演算子 / `for` が内部で引く特殊メソッドは `__getattribute__` を通らない（CPython と同じ・#12）。
