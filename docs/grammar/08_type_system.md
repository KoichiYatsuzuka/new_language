# 型システムと静的型検査

---

## 型アノテーションの書き方

型アノテーションは変数宣言・関数パラメータ・戻り値の後に `: Type` の形で付けます。

```hv
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
| `bool` | `Value::Bool(bool)` | 真偽値 (`True` / `False`) |
| `None` | `Value::None` | 値なし |

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
| `tuple[T1, T2, ...]` | `Value::Tuple(...)` | 固定長異種タプル |

---

## 特殊型

### `Any` — 動的型エスケープ

```hv
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

```hv
fn parse(let s: str) -> Union[int, None]:
    ...

let result: Union[int, str] = get_value()
if result is int:
    let n: int = result   # int として扱える
```

Union 型の変数に対して演算子を適用すると  
静的型エラー `OperationOnUnion` が報告されます。

### `Option[T]` — オプション型

`Option[T]` は `Union[T, None]` の糖衣構文です。

```hv
fn find_user(let id: int) -> Option[User]:
    ...

let user = find_user(42)
if user is not None:
    user.greet()   # user は User 型として扱える
```

### `type[T]` — 型値型

```hv
fn create(let cls: type[MyClass]) -> MyClass:
    return cls()
```

クラス自体を引数として渡すときに使用します。

### `function` 型

```hv
fn apply(let f: function[let int]->int, let x: int) -> int:
    return f(x)
```

詳細は [05_functions.md](05_functions.md) を参照。

### `Self` 型

クラス・trait メソッド内でのみ有効な自己参照型。  
そのクラスのインスタンスを表します。

```hv
class Node:
    fn clone(self) -> Self:
        return Self(self.value)
```

---

## 型ガードナロイング

### `is` による絞り込み

```hv
let x: Union[int, str] = get_value()

if x is int:
    # このブロック内で x は int 型
    let doubled = x * 2
```

### `is not` による絞り込み

```hv
let y: Option[str] = maybe_str()

if y is not None:
    # このブロック内で y は str 型 (None が除外される)
    print(y.upper())
```

`is not` は `Union` / `Optional` 型にのみ適用できます。  
非 Union 型に適用すると静的型エラー `IsNotOnNonUnion`。

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

---

## 型推論

型アノテーションが省略された場合、型検査器が式から型を推論します。

```hv
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

```hv
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
