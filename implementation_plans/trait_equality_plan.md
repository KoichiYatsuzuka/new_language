# trait 型の値とクラスの値の等値比較が誤ってエラーになる

trait 型で注釈した値と、その trait を実装するクラスの値を `==` / `!=` / `in` で比べると、
「can never be true」の静的エラーになる。実行時には真になりうる比較なので、これは誤検出である。

- 起票: 2026-10-07（[enum_member_type_plan.md](enum_member_type_plan.md) の調査中に見つけた。症状は同じだが原因が違うので分けた）
- 調査: Arrow `2045111`

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

⚠ 既存の `types_compatible` を両方向に呼ぶのが手軽だが、決定 D-15 が**意図して弾いている比較**
（`x == True`（`x: int`）・`1 == None` など）を通してしまわないか、例題で確かめてから採ること。
通してしまうなら、部分型の関係だけを判定する専用の条件を書く。
⚠ 型引数つきの trait（`Shape[T]`）を実装するクラスとの比較も確かめる。

## 3. タスク

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| **#1** | `check_equality` に部分型の規則を足す。例題を足す。成功例: trait 型と実装クラスの `==` / `!=` / `in`・逆向き・trait の継承をたどる場合。エラー例（`_error`）: 同じ trait を実装する別のクラス同士。`binop.rs` の doc の規則一覧（1〜4）も更新する | なし | 小 |
| **#2** | `type-checking` スキルの等値の節を更新する。型検査が変わるので VSIX を作り直す | #1 | 小 |

**ゲート**: `scan_examples` / `force_gate` / `compare_python_impl` / `type_obligations` / `compare_wasm_frontend`

⚠ enum のメンバーの比較は [enum_member_type_plan.md](enum_member_type_plan.md) でメンバーの型を enum 型にすれば
同型の比較になるので、本書の規則に頼らない。2 つの計画は独立に進められる。
