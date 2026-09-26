# bug_reports.md — Arrow 本体の未修正バグ（起票）

- 作成: 2026-09-26
- 発見経緯: テンプレートの型検査（メタ関数の設計書 2-13〜2-15）と、`import` の名前空間の分離を
  進める途中で見つけたもの。
- 検証: 2026-09-26 時点の `master` をリリースビルドして実測。各節の「再現」は、そのまま `.ar` に
  貼って走る最小形（モジュールを使うものはファイル名を添えてある）。「現状」はその出力。
- 採番: 互いに依存しないので `#1` から順に振る（`bug_fix.md` の `B1`〜`B13` とは別系統）。

⚠ 同じ調査で見つけて**直したもの**（ここには載せない）:
`import m` がモジュールの名前を呼び出し側の大域へ流し込んでいた（名前空間の侵食）／
モジュールの最上位で呼んだメソッドがそのモジュールの関数を引けなかった／
非同期タスクの中で呼んだ関数が自分の大域を引けなかった。

| # | 題 | 静的 | 実行時 | 重さ |
|---|---|---|---|---|
| #1 | private メソッドを外から呼べる | 通る | 通る | 高 |
| #2 | 型注釈が守られない（ジェネレータの要素） | 通る | 通る | 高 |
| #3 | 型検査がクラスを名前だけで引く（モジュールの同名クラスと混ざる） | 偽のエラー | — | 中 |
| #4 | 具体化を値として使えない（静的メソッド・`static mut`） | — | 落ちる | 中 |
| #5 | `x is Stack[int]` が構文エラー | 構文エラー | — | 低 |
| #6 | モジュールの `mut` 変数の変更が `m.x` に映らない | — | 古い値 | 中 |
| #7 | 未定義の名前を静的に検査しない | 通る | `NameError` | 中 |
| #8 | 演算子オーバーロードの被演算子の型を静的に検査しない | 通る | 落ちる | 中 |
| #9 | `int` などの属性アクセスを静的に検査しない | 通る | 落ちる | 中 |
| #10 | `freeze` した変数への `mut self` メソッド呼び出しを静的に検査しない | 通る | `TypeError` | 低 |
| #11 | 関数の中のクラス定義・`import` が内部エラー（`VmForceError`） | — | 落ちる | 中 |
| #12 | 誤りの位置が出ない（静的エラー・ParseError・traceback） | — | — | 中 |

---

## #1 private メソッドを外から呼べる

再現:
```
class Account:
    mut balance: int

    fn deposit(mut self, let n: int) -> None:
        self.audit(n)
        self.balance += n

    private:
    fn audit(self, let n: int) -> None:
        print("audit", n)

mut a = Account(0)
a.audit(5)
```
現状: `audit 5` と出力して完走する（静的にも実行時にも止まらない）。
期待: クラスの外からの `a.audit(5)` は、静的エラー（少なくとも実行時エラー）。

⚠ private の**フィールド**は静的に止まる（同じクラスの `private:` の下に `mut secret: int` を置いて
`a.secret` と書くと `'secret' is private and cannot be accessed outside 'Account'`）。メソッドだけ抜けている。

## #2 型注釈が守られない（ジェネレータの要素）

再現:
```
gen each(let items: list[int]) -> int:
    for x in items:
        yield x

for s in each([1, 2]):
    let z: str = s
    print(z)
```
現状: `1` と `2` を出力して完走する。`str` と注釈した `z` に `int` が入っている。
期待: `let z: str = s` は静的エラー（`s` は `int`）。

⚠ 原因の見当: ジェネレータの呼び出し結果の型が `for` の変数へ伝わらず、`s` の型が分からないまま
（検査されない）。さらに実行時も注釈を確かめていない。

## #3 型検査がクラスを名前だけで引く（モジュールの同名クラスと混ざる）

モジュール `tags.ar`:
```
class Tag:
    mut v: int
    fn who(self) -> int:
        return 1
```
メイン:
```
import tags as t

class Tag:
    mut v: int
    fn who(self) -> str:
        return "main"

let a: str = Tag(1).who()
let b: int = t.Tag(1).who()
print(a, b)
```
現状: `'b' is declared 'int' but initialized with 'str'` という**偽の**静的エラーで止まる。
期待: 通って `main 1` を出す（実行時は正しく区別している）。

⚠ 原因: 型検査のレジストリがクラス・関数を**素の名前だけ**で集めている
（`import` の本体の宣言も同じ表に入る）。実行時の名前空間の分離（2026-09-26）の静的な側の対応物。
テンプレートの具体化（`t.Box[int]` とメインの `Box[int]`）も同じ理由で混ざる。

## #4 具体化を値として使えない（静的メソッド・`static mut`）

再現 1:
```
class Stack[T]:
    mut items: list[T]
    static fn empty() -> Stack[T]:
        return Stack[T]([])

let s = Stack[int].empty()
```
現状: 実行時に `TypeError: 'template' object is not subscriptable`。

再現 2:
```
class Counter[T]:
    static mut n: int = 0
    mut v: T
    fn bump(self) -> None:
        Counter[T].n += 1

let a = Counter[int](1)
a.bump()
```
現状: 実行時に `NameError: 'T' is not defined`。
期待: どちらも動く（`Stack[int]` は単相化したクラスそのもの）。

⚠ 原因: パーサは `]` の直後が `(` のときだけ具体化として読み、それ以外は添字（`Stack` の `int` 番目）
として読む。実行時の具体化の廃止（D36）と合わせて、値としての具体化を単相化したクラスに結び付けるのが筋。

## #5 `x is Stack[int]` が構文エラー

再現:
```
class Stack[T]:
    mut items: list[T]

let s = Stack[int]([])
if s is Stack[int]:
    print("yes")
```
現状: ``ParseError: expected `:`, got `[` ``。
期待: 型の判定として読む（`is Stack` は書ける）。

## #6 モジュールの `mut` 変数の変更が `m.x` に映らない

モジュール `counter.ar`:
```
mut count = 0

fn bump() -> None:
    count += 1
```
メイン:
```
import counter as c
c.bump()
c.bump()
print(c.count)
```
現状: `0` を出力する。
期待: `2`。

⚠ 原因: 名前空間（`c`）は import が終わった時点のメンバーの**写し**。モジュールの関数はモジュールの
大域を書き換える（2026-09-26 から）が、写しは更新されない。名前空間の読みをモジュールの大域へ
向けるのが筋。

## #7 未定義の名前を静的に検査しない

再現:
```
print(nonexistent(2))
```
現状: 実行時の `NameError: 'nonexistent' is not defined`（静的には通る）。
期待: 静的エラー。

⚠ 名前空間の分離の後は、`import m` だけで修飾なしのモジュールの名前（`helper(2)`）を書いた場合も
これと同じ扱い（実行時の `NameError`）になる。

## #8 演算子オーバーロードの被演算子の型を静的に検査しない

再現:
```
class Money:
    mut cents: int

    fn __add__(self, let other: Money) -> Money:
        return Money(self.cents + other.cents)

let m = Money(100) + 5
print(m.cents)
```
現状: 実行時に `AttributeError: 'int' object has no attribute 'cents'`（`__add__` の中で落ちる）。
期待: `Money(100) + 5` が静的エラー（`other` は `Money`）。

## #9 `int` などの属性アクセスを静的に検査しない

再現:
```
fn label(let x: int) -> str:
    return x.name

print(label(1))
```
現状: 実行時エラー（`label` の中で落ちる）。
期待: `x.name` が静的エラー（`int` に `name` は無い）。

⚠ テンプレートの具体化（`fn show[T](let x: T) -> str: return x.name` を `show[int]` で具体化）も
同じ理由で静的に止まらない。

## #10 `freeze` した変数への `mut self` メソッド呼び出しを静的に検査しない

再現:
```
class Box:
    mut v: int
    fn set(mut self, let x: int) -> None:
        self.v = x

mut b = Box(1)
freeze b
b.set(2)
```
現状: 実行時に `TypeError: cannot call mutable method 'set' on immutable instance of 'Box'`。
期待: 静的エラー（`freeze` の後は書き込みが静的エラーになる規則・`.claude/rules/language-differences.md`。
組み込みの変更メソッド `x.append(..)` は静的に止まる）。

## #11 関数の中のクラス定義・`import` が内部エラー（`VmForceError`）

再現:
```
fn f() -> int:
    class Local:
        mut v: int
    return Local(1).v

print(f())
```
現状: `VmForceError: cannot compile function 'f' to bytecode`。関数の中の `import` も同じ。
期待: 動く、または「関数の中では書けない」という静的エラー（今の文面は内部の事情しか言っていない）。

## #12 誤りの位置が出ない

- 静的エラーの多くが位置を持たない（表の `File` が `<unknown>`、`Line:Col` が `-`）。
  例: #3 の偽のエラー、テンプレートのメソッド引数の型違い。
- ParseError に行・列が無い。再現: `let x = 1 +` → ``ParseError: unexpected token: `NEWLINE` ``。
- traceback の内側のフレームのファイル名が空（`File "", in label`）。
