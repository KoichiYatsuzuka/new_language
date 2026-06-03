# 関数とジェネレータ

---

## 関数定義 (`fn`)

```hv
fn greet(let name: str) -> str:
    return "Hello, " + name

fn add(let a: int, let b: int) -> int:
    return a + b
```

`Stmt::FnDef { name, template_params, params, return_type, body, is_abstract, is_static, is_class_method, decorators, access }`

**パース**: `'fn' ident ['[' template_params ']'] '(' params ')' '->' type_expr ':' block`

### パラメータ

| 形式 | 意味 |
|---|---|
| `let name: Type` | 不変パラメータ (呼び出し元の値はコピーされない) |
| `mut name: Type` | 可変パラメータ (呼び出し元の `mut` 変数を参照共有) |
| `name: Type` | 修飾子なし (静的型エラー `MissingParamTypeAnn` を記録するが実行は可能) |
| `self` | メソッドの第1引数 (型アノテーション不要) |
| `mut self` | 可変レシーバ (インスタンスフィールドを変更するメソッド) |

```hv
fn scale(mut self, let factor: float) -> None:
    self.x *= factor
    self.y *= factor
```

### デフォルト値

```hv
fn connect(let host: str, let port: int = 8080) -> None:
    ...
```

デフォルト値を持つパラメータの**後ろ**にデフォルト値なしのパラメータを置くとパースエラー。

### 戻り値型アノテーション

戻り値型アノテーション (`-> Type`) が**ない**場合、静的型検査が  
`MissingReturnTypeAnn` エラーを報告します。  
`-> None` を明示することを推奨します。

---

## ジェネレータ定義 (`gen`)

```hv
gen count_up(let start: int, let end: int) -> int:
    mut i = start
    while i < end:
        yield i
        i += 1
```

`Stmt::GenDef { name, template_params, params, yield_type, body, access }`

`yield expr` でジェネレータから値を産出します。  
呼び出すと `Value::Generator` が返ります。  
`for` ループや `next()` で値を取り出します。

```hv
for val in count_up(0, 5):
    print(val)   # 0, 1, 2, 3, 4
```

**実装注意**: 現在の実装では `exec_generator` がジェネレータ本体を **一括実行** して  
すべての `yield` 値を収集してから `GeneratorState { values, index }` に格納します。  
本物の遅延評価 (コルーチン) とは異なります。

**`gen` 内の制約**:
- `return expr` は禁止 (パースエラー)
- `mut` パラメータは禁止 (パースエラー)

---

## オーバーロード

同じスコープに同名の関数を複数定義できます。

```hv
fn process(let x: int) -> str:
    return "int: " + str(x)

fn process(let x: str) -> str:
    return "str: " + x

fn process(let x: int, let y: int) -> str:
    return "pair: " + str(x) + ", " + str(y)
```

**評価**: `Value::OverloadedFn(Vec<Rc<FnValue>>)` として格納されます。  
呼び出し時は `dispatch_overload` が引数の数と型を照合して最初にマッチした候補を選択します。

**マッチング順序**: 候補リストの登録順 (定義順) で評価され、  
最初にマッチしたオーバーロードが実行されます。

**静的型検査**: オーバーロードが存在する関数呼び出しでは  
`NoMatchingOverload` エラーが引数数不一致の場合に報告されます。

---

## テンプレート (ジェネリクス)

```hv
fn identity[T](let x: T) -> T:
    return x

fn first[T: Printable](let items: list[T]) -> T:
    items[0].print()
    return items[0]
```

`template_params: Vec<TemplateParam>` — 各パラメータは `name: constraints`  
複数の制約は `and` で結合: `[T: Printable and Comparable]`

**呼び出し**:

```hv
identity[int](42)
first[Vector](vec_list)
```

**実行** (`instantiate_template`):
1. テンプレートパラメータと型引数を照合して制約を検証
2. AST 内の型変数名を具体型名に置換 (`subst_*` ファミリの関数)
3. 置換済み `FnValue` を構築して実行

---

## 関数値とファーストクラス関数

```hv
fn square(let x: int) -> int:
    return x * x

let f = square          # Value::Function を変数にバインド
let result = f(5)       # 関数値を呼び出す

fn apply(let func: function[let int]->int, let x: int) -> int:
    return func(x)

apply(square, 10)
```

### 関数型アノテーション

| 記法 | 意味 |
|---|---|
| `function` | 型引数なし (任意のシグネチャ) |
| `function[let T]->R` | 型引数 (位置引数のみ) |
| `function{let name: T}->R` | 名前付き型引数 |
| `function[mut T]->R` | 可変引数 |

---

## クロージャとキャプチャ

```hv
fn make_multiplier(let n: int) -> function[let int]->int:
    fn multiply(let x: int) -> int:
        return x * n   # n をキャプチャ
    return multiply
```

クロージャの詳細は [02_variables.md](02_variables.md) の「クロージャとキャプチャ」を参照。

---

## デコレータ

```hv
@logger
fn compute(let x: int) -> int:
    return x * x

@memoize
@validate_input
fn process(let data: list[int]) -> list[int]:
    ...
```

デコレータは **上から下へ** パースされ、**下から上へ** (内側から) 適用されます。

**実行** (`exec_fn_def` 内):
```
fn_value = FnValue(...)
for decorator in reversed(decorators):
    dec_val = eval(decorator)
    fn_value = apply_value_call(dec_val, fn_value)
```

**静的型検査**: デコレータが `function -> function` または `type -> type` でなければ  
`InvalidDecorator` エラーを報告します。

---

## 自動キャスト (`let` パラメータ)

`let` パラメータに型アノテーションがあり、渡された値のクラスが異なる場合、  
`__cast__[TypeName]` テンプレートメソッドが定義されていれば自動的に呼び出されます。

```hv
class Point:
    let x: float
    let y: float

    fn __cast__[Vec2D](let v: Vec2D) -> Self:
        return Self(v.x, v.y)

fn distance(let p: Point) -> float:
    return sqrt(p.x ** 2 + p.y ** 2)

let v = Vec2D(3.0, 4.0)
distance(v)   # 自動的に Point にキャスト
```

`mut` パラメータは自動キャストされません。

---

## 関数実行のフロー

`exec_fn_evaled` の処理:

1. デフォルト値を事前評価
2. `bind_args` で引数を仮引数にバインド (位置・キーワード・デフォルト)
3. `let` パラメータへの自動キャスト
4. グローバル以外のスコープを退避して新しいローカルスコープを構築
5. キャプチャ変数をスコープに展開
6. `self_val` があれば `self` をバインド
7. 関数本体を実行 (`exec_block`)
8. スコープを復元、`call_stack` を pop
9. 例外が伝播中であればトレースバックフレームを追加
10. `ExecResult::Return(v)` の場合は値 `v` を返す。なければ `Value::None`

---

## `static` メソッドと `class_method`

```hv
class MathHelper:
    static fn square(let x: int) -> int:
        return x * x

    class_method fn create(cls: type[Self]) -> Self:
        return cls()
```

- `static fn` — `self` を受け取らないクラス関連の関数。クラス名で呼び出します。
- `class_method fn` — 第1引数が `cls: type[Self]` (クラス自体)。  
  サブクラス対応のコンストラクタに使用します。

インスタンス経由で `static` メソッドを呼び出すと静的型エラー `StaticMethodOnInstance`。
