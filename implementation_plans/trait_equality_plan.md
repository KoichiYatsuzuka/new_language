# trait 型の値とクラスの値の等値比較が誤ってエラーになる

trait 型で注釈した値と、その trait を実装するクラスの値を `==` / `!=` / `in` で比べると、
「can never be true」の静的エラーになる。実行時には真になりうる比較なので、これは誤検出である。

- 起票: 2026-10-07（[enum_member_type_plan.md](../implementation_logs/enum_member_type_plan.md) の調査中に見つけた。症状は同じだが原因が違うので分けた）
- 調査: Arrow `2045111`。2026-11-25 に `8e25ece6`（0.9.1）で再現を確かめ、周辺の形を測った（4 節）
- 状態: **#1・#2・#3 は済**（2026-11-25）。**#4 は要判断**（5 節）

## 0. 何が起きるか

```
trait Shape:
    fn area(self) -> int:
        ...

class Sq(Shape):
    let s: int
    fn area(self) -> int:
        return self.s * self.s

class Circ(Shape):
    let r: int
    fn area(self) -> int:
        return self.r

let a: Shape = Sq(2)
let b = Sq(2)
let c = Circ(1)
print(a != b)      # StaticTypeError: '!=' between 'Shape' and 'Sq' can never be true
print(a in [b])    # StaticTypeError: 'in' between 'Shape' and 'Sq' can never be true
print(b == a)      # StaticTypeError: '==' between 'Sq' and 'Shape' can never be true
print(b == c)      # StaticTypeError: '==' between 'Sq' and 'Circ' can never be true
```

| 比較 | 型検査 | 実行時（Python 実装で確認） | 判定 |
|---|---|---|---|
| `a != b`（`Shape` と `Sq`） | ❌ | `False`（＝等しい） | **誤検出** |
| `a in [b]` | ❌ | `True` | **誤検出** |
| `b == a`（逆向き） | ❌ | `True` | **誤検出** |
| `b == c`（同じ trait を実装する別のクラス同士） | ❌ | 常に `False` | 正しい |

## 1. 原因

等値の検査 `check_equality`（`src/type_check/binop.rs:120-162`、タスク 7.6・決定 D-15）は、
次のどれかに当たらなければエラーにする。

1. どちらかが不透明（`Unresolved` / `Protocol` / `Union` …）
2. 同型
3. 数値族（`int` / `float` / `complex`）
4. どちらかのクラスの `__eq__` が相手を受ける

**部分型の関係を見ていない。** `Shape` 型の値は実行時に `Sq` のインスタンスでありうるので、
`Shape` と `Sq` の比較が「決して真にならない」とは言えない。

## 2. 修正の方向

`check_equality` に「**片方がもう片方の部分型なら通す**」規則を足す
（クラス → それが実装する trait、trait → それが継承する trait。推移的にたどる）。
互いに部分型の関係に無い別のクラス同士（上の `b == c`）は、今までどおり弾く。

⚠ trait の継承（`trait Sub(Shape)`）は書けない（構文解析の誤り）ので、「trait が継承する trait」の形は無い（2026-11-25）。

⚠ 既存の `types_compatible` を両方向に呼ぶのが手軽だが、決定 D-15 が**意図して弾いている比較**
（`x == True`（`x: int`）・`1 == None` など）を通してしまわないか、例題で確かめてから採ること。
通してしまうなら、部分型の関係だけを判定する専用の条件を書く。
⚠ 型引数つきの trait（`Shape[T]`）を実装するクラスとの比較も確かめる。

## 3. タスク

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| ~~#1~~ | ✅ **済**（2026-11-25）`check_equality` に「同じインスタンスが両方の型になりうるなら通す」規則（`instances_may_coincide`）を足し、容器の要素（`containers_may_equal`）からも使う。protocol の名前は先に `Protocol` へ寄せる。trait が宣言した `__eq__` を見る（`eq_overload_accepts`）。テストは `gate/lang/typing/trait_equality{,_error}.ar`・`gate/ffi/py_class_equality{,_error}.ar`、例題は `examples/typing/trait_equality{,_error}.ar` | なし | 小 |
| ~~#2~~ | ✅ **済**（2026-11-25）`type-checking` スキルに等値の節を足した。VSIX を作り直した | #1 | 小 |
| ~~#3~~ | ✅ **済**（2026-11-25・#1 の調査で見つけた）テンプレートの本体の型変数（`x: T` の `x == 1`）を `T` という名のクラスとして比べていた。`types_compatible` と同じく型変数を含む比較は見ず、具体化した本体で検査する | なし | 小 |
| **#4** | ⚠ **要判断**: trait 型の値と、trait を実装する**一部のクラスの `__eq__` だけ**が受ける型の比較（5 節） | #1 | 小〜中 |

**ゲート**: `gates_every_time` / `type_obligations` / `compare_wasm_frontend`（`compare_python_impl` は凍結中）

⚠ enum のメンバーの比較は [enum_member_type_plan.md](../implementation_logs/enum_member_type_plan.md) でメンバーの型を enum 型に
したので同型の比較になり、本書の規則に頼らない。

## 4. 測定と直した後（2026-11-25）

どの形も直す前は「can never be true」だった。

| 形 | 実行時 | 直した後 |
|---|---|---|
| trait 型と実装クラス（`==` / `!=` / `in` / `not in`・両向き・`dict` のキー・絞った `Option[Shape]`・別モジュールの trait） | 真になりうる | 通る |
| 型引数つきの trait（`Box[int]` と `IntBox`） | 真になりうる | 通る（`Box[int]` と `StrBox` は弾く） |
| 別の trait どうし（`Shape` と `Other`） | 両方を実装するクラスなら真 | 通る |
| 例外の階層（`Exception` と `ValueError`・`LookupError` と `KeyError`・`Error` と実装クラス） | 同じ物なら真 | 通る |
| Python のクラスの階層（`z.Animal` と `z.Puppy`） | 同じ物なら真 | 通る（互いに派生でなければ弾く） |
| 容器の要素（`list[Shape] == list[Sq]`・tuple・dict） | 真になりうる | 通る |
| protocol 型の**仮引数**（`let h: HasArea` は前から通る） | 真になりうる | 通る |
| trait が宣言した `__eq__` が相手を受ける（`z: Num` の `z == 1`） | 真になりうる | 通る |
| テンプレートの型変数（`x: T` の `x == 1`） | 具体化による | 具体化した本体で検査する |
| 同じ trait を実装する別のクラス・trait とそれを実装しないクラス・`Shape == 1`・`Shape == None`・`x == True`・`1 == None` | 偽（`Shape == 1` は 5 節） | 弾く（変えない） |

## 5. #4（要判断）: 実装クラスの `__eq__` だけが受ける型

```
trait Shape:
    fn a(self) -> int:
        ...
class Ev(Shape):
    let v: int
    fn a(self) -> int:
        return 1
    fn __eq__(self, o: int) -> bool:      # trait には無い宣言
        return self.v == o

let z: Shape = Ev(1)
print(z == 1)        # StaticTypeError: '==' between 'Shape' and 'int' can never be true
if z is Ev:
    print(z == 1)    # True（絞れば通る）
```

trait の宣言には `__eq__(int)` が無いので静的には弾くが、実行時は `Ev.__eq__` が呼ばれて真になる。

| 案 | 内容 | 利点 | 欠点 |
|---|---|---|---|
| **1（推奨）** | 今のまま。trait 型の値に使えるのは trait が宣言したものだけ、という規則（メソッド呼び出し `z.only_in_ev()` が誤りなのと同じ）と読む | 検査がモジュールの中で閉じる。`Shape == 1`・`Shape == None`・`Shape == Plain` の書き間違いを捕まえ続ける | 「can never be true」の文面は、この形では正確でない |
| 2 | trait を protocol と同じ不透明にする | 誤検出が無くなる。`HasArea == 1` が通る protocol と揃う | `Shape == 1` などの書き間違いを捕まえなくなる（`trait_equality_error.ar` の 6 行） |
| 3 | 型の表にある実装クラスを全部たどり、相手を受ける `__eq__` があれば通す | 今のプログラムについては正確 | 別のモジュールに実装クラスを足すだけで、ここの誤りの有無が変わる。単独で検査したモジュールと、import した側の検査で結果が食い違いうる |

案 1 のときの書き方（どれも測った）:

- 比べる前に絞る（`if z is Ev:`）。
- trait に `__eq__(self, o: int) -> bool` を宣言する（#1 で効くようになった）。⚠ 実装クラスの `__eq__` は trait と同じ型で
  なければならない。⚠ trait に既定の本体を書くと、`__eq__` を書かないクラス（`Sq`）もそれを使うので、`s == Sq(1)` が中身の
  比較でなくなる（既定の本体が `False` か `NotImplemented` を返すと、どちらも `False` になった）。

該当するコード（trait を実装し、自分と別の型を受ける `__eq__` を持つクラス）は、`examples/` と `gate/` に無い
（#1 で足した `trait_equality.ar` の `Num` は trait 側の宣言）。
