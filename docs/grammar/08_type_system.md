# 型システムと静的型検査

---

## 型アノテーションの書き方

型アノテーションは変数宣言・関数パラメータ・戻り値の後に `: Type` の形で付けます。

```ar
let x: int = 42
mut name: str = "Alice"
fn add(let a: int, let b: int) -> int: ...
```

型アノテーションはオプションですが、関数の戻り値型・パラメータ型がない場合は  
静的型エラーが報告されます。

---

## プリミティブ型

| 型名 | 対応する `Value` | 説明 |
|---|---|---|
| `int` | `Value::Int(i64)` | 64 ビット符号付き整数 |
| `float` | `Value::Float(f64)` | 64 ビット浮動小数点 |
| `str` | `Value::Str(String)` | Unicode 文字列 |
| `bytes` | `Value::Bytes(Rc<[u8]>)` | 不変のバイト列（下の「`bytes`」） |
| `bool` | `Value::Bool(bool)` | 真偽値 (`True` / `False`) |
| `None` | `Value::None` | 値なし |

### `bytes` — バイト列

C の `char*` などを、文字列に変換する前のまま扱うための不変のバイト列です。`str` とは別の型で、
相互の変換は文字コードを書いて明示的に行います（黙って UTF-8 とみなしません）。

```ar
let b: bytes = "日本語".encode("shift_jis")   # str → bytes
let s: str = b.decode("cp932")                 # bytes → str
let raw = bytes([104, 105, 255])               # 0〜255 の int のリストから
print(raw)                                     # b'hi\xff'
print(len(raw), raw[0], raw[0:2])              # 3 104 b'hi'
for x in raw:                                  # 各要素は 0〜255 の int
    print(x)
```

- `len` / 添字（`int`）/ スライス（`bytes`）/ 反復（`int`）/ `==` と順序比較 / `+`（連結）/ `in`（部分列、または 0〜255 の
  `int`）。`dict` のキー・`set` の要素にもなる
- 文字コード: `utf-8`（既定）・`utf-16-le` / `utf-16-be`・`ascii`・`latin-1`・`shift_jis`（`cp932`）・その他の WHATWG の名前
  （`euc-jp` / `gbk` / `big5` …）。変換できないバイト列・文字は `ValueError`（置き換え文字にしない）
- `bytes(n)` は n バイトのゼロ（C の出力バッファを用意するのに使う）。`bytes(str)` は `TypeError`（`s.encode(..)` を使う）。
  リテラルの構文（`b"..."`）はまだありません
- ⚠ 以前の `str.encode()` は文字コードを無視して `list[int]` を返していました

---

## コレクション型

| 型名 | 対応する `Value` | 説明 |
|---|---|---|
| `list` | `Value::List(...)` | 要素型未知のリスト |
| `list[T]` | `Value::List(...)` | 要素型既知のリスト |
| `fixed_list` | `Value::FrozenList` | 不変固定長リスト (freeze 後) |
| `fixed_list[T]` | `Value::FrozenList` | 型付き不変固定長リスト |
| `list_like` | `list` または `fixed_list` | 両方を受け入れる抽象型 |
| `list_like[T]` | — | 型付き抽象リスト |
| `dict` | `Value::Dict(...)` | キー型・値型未知の辞書 |
| `dict[K, V]` | `Value::Dict(...)` | 型付き辞書 |
| `set` | `Value::Set(...)` | 要素型未知のセット |
| `set[T]` | `Value::Set(...)` | 型付きセット |
| `frozenset[T]` | `Value::FrozenSet(...)` | 不変でハッシュできる集合（`frozenset(xs)` で作る）。`set[T]` とは互いに代入できない。演算子の結果は左の種類、`==` と包含の比較（`<=`）は set とも書ける |
| `tuple[T1, T2, ...]` | `Value::Tuple(...)` | 固定長異種タプル |

---

## `Ptr[T]` — 外部のメモリを指す値

外部の関数（C / C++ などのブリッジ）が返すポインタを、中身を展開しないまま持つ型です
（`implementation_plans/ffi_value_exchange_plan.md` D-3）。**コンストラクタは無く**、Arrow から作ることはできません
（外部の関数が返します）。外部の関数との受け渡しの約束（長さ・要素の型名・解放の責任）は
[`docs/ptr_protocol.md`](../ptr_protocol.md) にあります。

```arrow
fn sum_ints(let p: Ptr[int32]) -> int:
    mut total = 0
    for x in p.to_list():        # list[int]: C の int32 を読んで Arrow の int にしたコピー
        total += x
    return total

fn text(let p: Ptr[uint8]) -> str:
    return p.to_bytes().decode("utf-8")   # 文字データだけ bytes にできる

fn as_ints(let p: Ptr[void]) -> list[int]:
    let q = p mustbe Ptr[int32]           # void から戻すときは検査して変換する
    return q.to_list()
```

- **`T`** は C の幅つきの型（`int8` … `int64`・`uint8` … `uint64`・`float32`・`float64`）・構造体・`void`。
  Arrow の `int` / `float` は幅を持たないので書けません（`Ptr[int]` は静的エラー）。要素型を書かない `Ptr` も注釈には
  書けません（`x is Ptr` のような型テストでは書けます）
- **できること**: `==`（同じ `Ptr` のコピーどうしだけが等しい）・外部の関数へ渡し返す・`p.release()`・
  `p.to_list() -> list[T]`・`p.to_bytes() -> bytes`（要素が `int8` / `uint8` / `uint16` のときだけ）。展開した値はコピーで、
  元のポインタとはつながりません
- **できないこと**: アドレスの取得・加算、整数からの生成、中身の書き込み（静的エラー）
- **変換**: どの `Ptr[T]` も `Ptr[void]` として渡せます。逆は `p mustbe Ptr[T]` で、作られたときの要素型と一致するときだけ
  成功します。要素型の分かっている `Ptr` どうし（`Ptr[int32]` と `Ptr[int64]`、`Ptr[Db]` と `Ptr[File]`）は別の型です
- **解放**: 所有しているメモリは、最後の参照がなくなったとき（または `p.release()`）に 1 回だけ解放されます。コピーは同じ
  メモリを共有します。借用しているもの（外部が持ち続けるメモリ）を `release()` すると `ValueError` です
- 書き込み用ポインタの引数（`mut`）にも `let` の `Ptr` を渡せます（Arrow から中身を書き換える手段が無いため）

---

## 特殊型

### `Any` — 動的型エスケープ

```ar
let val: Any = get_unknown()
# val.attr     → 静的型エラー OperationOnAny
# val + 1      → 静的型エラー OperationOnAny
if val is int:
    let n = val   # ここでは int として扱える
```

`Any` 型の値に対して属性アクセス・演算子を適用すると  
静的型エラー `OperationOnAny` が報告されます。  
`is` 型ガードで具体型に絞り込んでから使用してください。

### `Union[T1, T2, ...]` — 合併型

```ar
fn parse(let s: str) -> Union[int, None]:
    ...

let result: Union[int, str] = get_value()
if result is int:
    let n: int = result   # int として扱える
```

Union 型の変数に対して演算子を適用すると  
静的型エラー `OperationOnUnion` が報告されます。

Union の値は、**どの構成型も受かる**型へ渡せます（引数・変数・戻り値）。`Option[int]` は `Union[int, str, None]` へ
渡せますが、`Union[int, str]` は `Option[int]` へ渡せません（`str` が来うる。先に `is int` / `mustbe int` で絞る）。
構成型の順序は問いません。

### `Intersection[T1, T2, ...]` — 交差型

`Intersection[T1, T2, ...]` は、値が **すべての構成型を同時に満たす** ことを表す型です。  
`Union` が「どれか一つ」であるのに対し、`Intersection` は「すべて」の型制約を要求します。

```ar
trait Flyable:
    fn fly(self) -> str: ...

trait Swimmable:
    fn swim(self) -> str: ...

class Duck(Flyable, Swimmable):
    let name: str
    fn __init__(mut self, n: str) -> None: self.name = n
    fn fly(self) -> str: return self.name + " is flying"
    fn swim(self) -> str: return self.name + " is swimming"

# Flyable かつ Swimmable を同時に要求する
fn show_abilities(creature: Intersection[Flyable, Swimmable]) -> None:
    creature.fly()    # Flyable のメンバーに直接アクセス可能
    creature.swim()   # Swimmable のメンバーにも直接アクセス可能

let duck = Duck("Donald")
show_abilities(duck)   # Duck は Flyable と Swimmable を両方継承するので OK
```

**ポイント**:
- 構成型にはクラス名・trait 名・protocol 名を混在して指定できます（2 つ以上必須）
- 変数の型が `Intersection[...]` であれば、すべての構成型のメンバーに**ダウンキャストなし**でアクセスできます
- `Union` 型のメンバーアクセスは静的型エラーになりますが、`Intersection` はなりません
- 関数パラメータを `Intersection[...]` で型付けすると、渡す引数はすべての構成型を継承/適合している必要があります

#### 型ガードと `Intersection`

```ar
let animal: Intersection[Flyable, Swimmable] = duck

# is C でガードして C 型に絞り込める（C がすべての構成型を満たす場合のみ有効）
if animal is Duck:
    print(animal.fly())
```

`is C` のガード型 `C` がいずれかの構成型を満たさない場合、静的型エラー `IntersectionGuardTypeFails` が報告されます。

```ar
class Bird(Flyable, Runnable):   # Swimmable を実装していない
    ...

let x: Intersection[Flyable, Swimmable] = ...
if x is Bird:    # StaticTypeError: Bird は Swimmable を満たさない
    ...
```

`is not` を `Intersection` 型に適用した場合は `Union`/`Option` 型と同様に  
静的型エラー `IsNotOnNonUnion` が報告されます。

#### 部分コンパイル対象外

`Intersection[...]` 型を含む関数（パラメータ・戻り値いずれか）は  
`--compile` によるネイティブコンパイルの対象外となり、警告 `IntersectionSkippedCompile` が出力されます。

#### メンバー衝突の検査

交差型の構成型間で同名メンバーが存在する場合、静的型検査器が衝突を検出します。

| 状況 | 結果 |
|---|---|
| 名前・型・アクセス修飾子がすべて同一 | 警告 `IntersectionMemberDuplicate` |
| 名前は同じだが型やシグネチャが異なる（互換性なし） | エラー `IntersectionMemberConflict` |

---

### `Option[T]` — オプション型

`Option[T]` は `Union[T, None]` の糖衣構文です。

```ar
fn find_user(let id: int) -> Option[User]:
    ...

let user = find_user(42)
if user is not None:
    user.greet()   # user は User 型として扱える
```

### `Result[T, E]` — 結果型

`Result[T, E]` は成功値 `Ok(T)` または失敗値 `Err(E)` を表す特殊な合併型です。  
エラーを例外ではなく戻り値として扱うパターンに使用します。

**コンストラクタ**

| 式 | 説明 |
|---|---|
| `Ok(value)` | 成功値を持つ Result を生成（`value` は型 `T`） |
| `Err(error)` | 失敗値を持つ Result を生成（`error` は型 `E`） |

**型ガードによる絞り込み**

| 条件式 | ブロック内の変数型 |
|---|---|
| `if result.is_OK():` | `result` が `T`（Ok の内部値）に絞り込まれる |
| `if result.is_ERR():` | `result` が `E`（Err の内部値）に絞り込まれる |

```ar
fn divide(a: int, b: int) -> Result[int, str]:
    if b == 0:
        return Err("division by zero")
    return Ok(a / b)

let r1: Result[int, str] = divide(10, 2)
let r2: Result[int, str] = divide(10, 0)

if r1.is_OK():
    print("成功:", r1)   # r1 は int として扱える

if r2.is_ERR():
    print("失敗:", r2)   # r2 は str として扱える
```

**制約**

- `T` と `E` は**異なる型**でなければなりません。同じ型を指定すると静的型エラー `ResultSameTypes` が報告されます。
- ガード節なしで `Result` 型の変数に演算子・属性アクセスを適用すると `OperationOnUnion` が報告されます。
- `is_OK()` / `is_ERR()` は引数なしで呼び出します。

```ar
# エラー例: Ok 型と Err 型が同じ
let bad: Result[int, int] = Ok(1)   # StaticTypeError: ResultSameTypes
```

### `type[T]` — 型値型

```ar
fn create(let cls: type[MyClass]) -> MyClass:
    return cls()
```

クラス自体を引数として渡すときに使用します。

### `function` 型

```ar
fn apply(let f: function[let int]->int, let x: int) -> int:
    return f(x)
```

詳細は [05_functions.md](05_functions.md) を参照。

### `Self` 型

クラス・trait メソッド内でのみ有効な自己参照型。  
そのクラスのインスタンスを表します。

```ar
class Node:
    fn clone(self) -> Self:
        return Self(self.value)
```

---

## `mustbe` — 動的型アサーション

`mustbe` は式の実行時型を検査し、型が一致しなければ `TypeError` を raise します。  
静的型検査では右辺の型情報を完全に保持するため、以降のコード解析もその型として扱われます。

```ar
let x: Any = compute()
let n = x mustbe int     # 実行時に int か確認; 静的型: int
let doubled = n * 2      # n は int なので OK
```

### 構文

```
expr mustbe TypeExpr
```

- `TypeExpr` には `int`, `str`, `list[int]`, `function[T]->R`, `MyClass`, `MyProtocol` など、  
  `Undefined` 以外のすべての型を記述できます。
- `mustbe` は**式**として評価され、右辺の値をそのまま返します（型が一致した場合）。

### 実行時の検査対象

| `TypeExpr` | 実行時チェック |
|---|---|
| プリミティブ型 (`int`, `float`, `str`, `bool`, `None`) | 値の型が一致するか |
| コレクション型 (`list[T]`, `dict[K,V]`, `set[T]`, `tuple[...]`) | **外側のコンテナ型のみ**（要素型は非検査） |
| `function` | 呼び出し可能か（Arrow 関数・`__call__` を持つクラスインスタンス） |
| `function[T]->R` | 呼び出し可能かのみ（シグネチャは非検査） |
| クラス名 | インスタンスのクラス名または継承 trait が一致するか |
| `protocol` 名 | プロトコルが要求する全メンバーが存在するか |

### 静的型の保持

```ar
let xs: Any = get_list()
let typed_xs = xs mustbe list[int]   # 静的型: list[int]
let elem = typed_xs[0]               # 静的型: int (添字型推論)
```

```ar
fn greet() -> int: return 42
let f: Any = greet
let g = f mustbe function[()->int]   # 静的型: ()->int
let r = g()                          # 静的型: int
```

`xs mustbe list` のように要素型を書かなければ、静的型は要素型の無い `list` です。要素型の無いコンテナは、同じ種類の
要素型つきのもの（`[1, 2]` は `list[int]`）と `==` / `!=` で比べられます（要素が合うかは実行してみないと分からないので。
`implementation_plans/ffi_value_exchange_plan.md` 7-1。以前は「決して真にならない」の静的エラーでした）。`set` と `list[int]`
のように種類が違えば静的エラーです。

### 警告

要素型付きコレクション (`list[int]` 等) やシグネチャ付き `function` を `mustbe` に使用すると  
静的型警告が発生します。型情報は静的解析に活用されますが、実行時チェックには反映されません。

```ar
let nums = vals mustbe list[int]
# Warning: `mustbe 'list[int]'` only checks that the value is a `list` at runtime; element type is not verified
```

### 型が一致しない場合

```ar
let s = "hello"
let n = s mustbe int
# TypeError: mustbe assertion failed: expected `int`, got `str`
```

---

## 型ガードナロイング

### `is` による絞り込み

```ar
let x: Union[int, str] = get_value()

if x is int:
    # このブロック内で x は int 型
    let doubled = x * 2
```

### `is not` による絞り込み

```ar
let y: Option[str] = maybe_str()

if y is not None:
    # このブロック内で y は str 型 (None が除外される)
    print(y.upper())
```

`is not` は `Union` / `Optional` 型にのみ適用できます。  
非 Union 型に適用すると静的型エラー `IsNotOnNonUnion`。

---

## 属性の型

`obj.x` の型はメンバーの宣言型です。次の 2 つは宣言型そのものではなく、実行時に起きることに合わせた型で検査します。

### 属性に入れた関数を呼ぶ（`obj.f(x)`）

メソッドの無い名前の呼び出しは、メンバーの型で検査します（[06_classes_traits.md](06_classes_traits.md) の「属性に入れた関数を呼ぶ」）:

| メンバーの型 | 検査 |
|---|---|
| `function[let int]->int` など引数の分かる関数型 | 引数の数・型を検査し、戻り値の型が呼び出しの型 |
| `type[C]`（クラス変数に入ったクラス） | `C` のインスタンス |
| 呼べない型（`int` など） | `'int' is not callable`（値を呼ぶのと同じ誤り） |

### ディスクリプタ

クラス側（`static mut` / `const`）のメンバーの宣言型が `__get__` を持つクラスなら、そのメンバーはディスクリプタです
（[06_classes_traits.md](06_classes_traits.md) の「ディスクリプタ」）。

| 書き方 | 照合する型 |
|---|---|
| `obj.x` / `C.x`（読み） | `__get__` の戻り値の型 |
| `obj.x(..)`（呼び出し） | `__get__` の戻り値の型を呼ぶ |
| `obj.x = v`（データディスクリプタ） | `__set__` の `value` の型（型が書いていなければ検査しない） |
| `obj.x = v`（非データディスクリプタ） | メンバーの宣言型 |
| `C.x = v`（クラスへ代入） | メンバーの宣言型（ディスクリプタのクラス） |

```ar
class F:
    fn __get__(self, obj: Any, owner: Any = None) -> float:
        return 1.5
    fn __set__(self, mut obj: Any, value: float) -> None:
        pass

class T:
    static mut f: F = F()
    const k: F = F()

mut t = T()
let x: float = t.f        # OK（__get__ の戻り値）
t.f = 2.5                 # OK（__set__ の value）
t.k = 3.0                 # OK（const でもデータディスクリプタへの代入は __set__ なので不変の誤りにしない）
t.f = "hot"               # StaticTypeError: field 'f' of class 'T' is declared 'float' but got 'str'
T.f = 2.5                 # StaticTypeError: field 'f' of class 'T' is declared 'F' but got 'float'
                          #（クラスへの代入はメンバーそのものを置き換える）
```

- ⚠ インスタンスのフィールド（`mut f: D`）はディスクリプタとして扱いません（実行時も働かない）。
- ⚠ 組み込みの `property` を置いたメンバーは `Any` です（Python から変換したクラスは `static mut x: Any`）。
- 実装: `TypeChecker::class_descriptor`（`type_check/infer.rs`）。読みは `infer_attr`、呼び出しは `call_check.rs`、
  代入は `stmt/check.rs` の `check_assign` が使う。

---

## 静的型検査のタイミング

```
パース完了
    ↓
TypeChecker::check(&stmts)  ← ここで全 AST を走査
    ↓
StaticTypeError のリストを収集
    ↓
1 件以上あれば全件表示して実行せずに終了
```

静的型エラーは **最初の 1 件で停止せず、全件収集** してから一括報告します。  
これにより 1 回の実行で複数の型エラーを修正できます。

---

## 静的型エラーの種類

| エラー名 | 説明 |
|---|---|
| `IncompatibleComparison` | 型が合わない比較演算 |
| `AssignToImmutable` | 不変変数への代入 |
| `AssignToImmutableField` | 不変フィールドへの代入 |
| `AssignToConst` | `const` なメンバー（クラス・trait の `const`・enum のメンバー・モジュールの `const`）への代入。どの経路でも（`__init__` の中も） |
| `ModifyConst` | `const` の中身（要素・フィールド・書き換えるメソッド）の書き換え。経路のどこに `const` があっても |
| `VariableRedeclaration` | アクセス可能なスコープに既に存在する変数名の再宣言 |
| `CallArgCountMismatch` | 引数の数が合わない |
| `CallArgTypeMismatch` | 引数の型が合わない |
| `MissingParamTypeAnn` | パラメータに型アノテーションなし |
| `MissingReturnTypeAnn` | 戻り値型アノテーションなし |
| `UnknownKeywordArg` | 存在しないキーワード引数 |
| `NoMatchingOverload` | マッチするオーバーロードなし |
| `SelfTypeMismatch` | `self` パラメータの型が不一致 |
| `OperationOnAny` | `Any` 型への演算 |
| `OperationOnUnion` | `Union` 型への演算 |
| `IsNotOnNonUnion` | 非 Union 型への `is not` |
| `CallMutParamWithImmutableArg` | 不変変数を `mut` 引数に渡す |
| `InvalidDecorator` | 不正なデコレータ |
| `TupleUnpackMissingQualifier` | タプルアンパックに `let`/`mut` なし |
| `TupleUnpackArityMismatch` | タプルアンパックの要素数不一致 |
| `PrivateAccessError` | `private` メンバへのアクセス |
| `ProtectedAccessError` | `protected` メンバへのアクセス |
| `StaticMethodOnInstance` | インスタンス経由での `static` メソッド呼び出し |
| `BlockReturnInLoopExpr` | `for`/`while` 式直下での `block_return` |
| `InvalidRaiseType` | `raise` に非インスタンス型を渡す |
| `FieldDefaultNotAllowed` | `mut`/`let` フィールドに初期値を設定 |
| `DirectFreezeCall` | `__freeze__` の直接呼び出し |
| `IntersectionMemberConflict` | 交差型の構成型間でメンバーが衝突 |
| `IntersectionGuardTypeFails` | `is` ガード型が交差型の全構成型を満たさない |
| `ResultSameTypes` | `Result[T, E]` の `T` と `E` が同じ型 |

### 型警告 (TypeWarning)

| 警告名 | 説明 |
|---|---|
| `MustBeElemTypeUnchecked` | `mustbe list[T]` 等で要素型が実行時に検証されない |
| `MustBeFunctionSignatureUnchecked` | `mustbe function[T]->R` 等でシグネチャが実行時に検証されない |

---

## 型推論

型アノテーションが省略された場合、型検査器が式から型を推論します。

```ar
let x = 42           # InferredType::Int
let items = [1, 2]   # InferredType::ListOf(Int)
let d = {"a": 1}     # InferredType::DictOf(Str, Int)
```

型推論は完全ではありません。型が確定できない場合は `InferredType::Unresolved` になり、  
その変数への操作の型検査はスキップされます。

---

## 型チェックの仕組み

### シグネチャ収集 (`collect_fn_sigs`)

型検査の前にトップレベルの関数・クラスシグネチャを先行スキャンします。  
これにより前方参照が可能になります。

```ar
# foo が bar より先に定義されていなくても、bar を呼び出せる
fn foo() -> int:
    return bar()

fn bar() -> int:
    return 42
```

### スコープスタック

型検査器も実行時と同様のスコープスタックを持ちます。  
変数の型情報と可変フラグ (`VarInfo`) をスコープに格納します。

```rust
struct VarInfo {
    ty:      InferredType,
    mutable: bool,
}
```
