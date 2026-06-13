# クラス・trait・new_type・enum

---

## クラス定義

```hv
class Point:
    mut x: float
    mut y: float

    fn distance(self) -> float:
        return sqrt(self.x ** 2 + self.y ** 2)
```

`Stmt::ClassDef { name, template_params, bases, decorators, body }`

**パース**: `['@' decorator]* 'class' ident ['[' template_params ']'] ['(' bases ')'] ':' class_body`

Arrow のクラスは**クラス継承をサポートしません**。  
代わりに trait を使います。

---

## フィールド宣言

```hv
class Config:
    mut timeout: int         # 可変インスタンスフィールド
    let name: str            # 不変インスタンスフィールド
    const DEFAULT_PORT = 8080  # クラス変数 (全インスタンス共有)
    static mut call_count = 0  # 静的可変クラス変数
```

`Stmt::Field { name, kind: FieldKind, type_ann, default, access }`

| フィールド種別 | キーワード | 意味 |
|---|---|---|
| `FieldKind::Mut` | `mut` | 可変インスタンス変数。`__init__` 後も再代入可能 |
| `FieldKind::Let` | `let` | 不変インスタンス変数。`__init__` 内でのみ代入可能 |
| `FieldKind::Const` | `const` | クラス変数。全インスタンスで共有。代入不可 |
| `FieldKind::StaticMut` | `static mut` | 可変クラス変数。全インスタンスで共有 |

**初期値ルール**:
- `const` フィールド → 初期値必須
- `mut`/`let` フィールド → 初期値禁止 (静的型エラー `FieldDefaultNotAllowed`)。  
  初期値が必要な場合は `__init__` で設定するか、デフォルト `__init__` に引数デフォルト値を渡します。

---

## 自動生成 `__init__`

`__init__` を手動定義しなかった場合、すべてのフィールド引数を受け取る  
`__init__` が自動生成されます。

```hv
class Vec3:
    mut x: float
    mut y: float
    mut z: float
    # 自動生成: fn __init__(mut self, let x: float, let y: float, let z: float) -> None:
    #               self.x = x; self.y = y; self.z = z

let v = Vec3(1.0, 2.0, 3.0)
```

**自動 `__init__` の生成ルール**:
1. `mut`/`let` フィールドを宣言順に引数として追加
2. フィールドと同名の引数を `self.field = arg` で初期化
3. trait から継承したフィールドも含まれる

---

## アクセス制御

クラス本体は `public:` / `private:` / `protected:` セクションで区切られます。  
セクション宣言がない場合のデフォルトは `public`。

```hv
class BankAccount:
    public:
        let owner: str
        fn deposit(mut self, let amount: float) -> None:
            self.balance += amount

    private:
        mut balance: float = 0.0
        fn _validate(self) -> bool:
            return self.balance >= 0.0

    protected:
        fn _internal_transfer(mut self, let amount: float) -> None:
            self.balance -= amount
```

| アクセス修飾子 | 参照可能な場所 |
|---|---|
| `public` (デフォルト) | どこからでも |
| `private` | 同じクラスのメソッド内のみ |
| `protected` | 同じクラスまたは継承した trait を実装したクラス |

**実行時の検査**: `Interpreter.current_class` と `field_access`/`method_access` マップを照合。  
違反は `AccessError` を送出します。

---

## trait 定義

```hv
trait Printable:
    fn print(self) -> None: ...   # 抽象メソッド

trait Comparable:
    fn compare(self, let other: Self) -> int: ...   # 抽象メソッド
    
    fn less_than(self, let other: Self) -> bool:  # デフォルト実装
        return self.compare(other) < 0
```

`Stmt::TraitDef { name, template_params, body }`

**特徴**:
- `...` (Ellipsis) ボディが抽象メソッド (`is_abstract: true`)
- デフォルト実装を持つメソッドも定義可能
- 全メソッドに戻り値型アノテーションが必須 (パースエラー)
- 全非仮想メソッドのパラメータに型アノテーションが必須

### trait の実装

```hv
class Temperature(Comparable):
    let value: float

    fn compare(self, let other: Self) -> int:
        if self.value < other.value:
            block_return -1
        elif self.value > other.value:
            block_return 1
        else:
            block_return 0
```

`class ClassName(Trait1, Trait2):` の構文で実装します。  
`bases` に trait 名が列挙されます。

### Self 型

trait メソッド内の `Self` は、そのメソッドを実装したクラスの型を表します。

```hv
trait Builder:
    fn set_name(mut self, let name: str) -> Self: ...
    fn build(self) -> Self: ...
```

---

## テンプレートクラス

```hv
class Stack[T]:
    mut items: list[T]

    fn push(mut self, let item: T) -> None:
        self.items.append(item)

    fn pop(mut self) -> T:
        return self.items.pop()
```

**使用**:

```hv
let int_stack = Stack[int]()
int_stack.push(1)
int_stack.push(2)
let top = int_stack.pop()   # 2
```

**実行** (`instantiate_template_class`):
1. 型引数でテンプレートパラメータを置換した AST を構築
2. 置換済みボディで `ClassDef` を実行して `ClassValue` を生成

---

## `new_type` 宣言

```hv
new_type Celsius: float
new_type UserId: int
new_type AdminId: UserId
```

`Stmt::NewTypeDef { name, original }`

**動作**: `original` の型定義をコピーして `name` という別名で登録します。

```hv
let temp = Celsius(36.5)   # コンストラクタで作成
let id: UserId = UserId(42)

# Celsius と float は構造的に同一だが型名が違うため演算不可
# temp + 1.0   # TypeError: Celsius と float の演算
```

**`Self` 型との組み合わせ**:

```hv
class Meters:
    let value: float

    fn add(self, let other: Self) -> Self:   # Meters + Meters → Meters
        return Self(self.value + other.value)

new_type Kilometers: Meters
# Kilometers.add は Kilometers + Kilometers → Kilometers になる
```

`new_type` 変数への再代入はパースエラー。

---

## `enum` 宣言

```hv
enum Color:
    Red
    Green
    Blue

enum Status:
    Pending = 0
    Running = 1
    Done    = 2
```

`Stmt::EnumDef { name, variants: Vec<(String, Option<Expr>)> }`

**実行**:
1. 各バリアントの値型 `enum_item_Color` (new_type) を登録
2. `Color` クラスを作成し、各バリアントを `const` フィールドとして設定
3. 値は `0` から自動採番 (明示値がない場合)

```hv
let c = Color.Red
match c:
    case Color.Red:
        print("red")
    case Color.Green:
        print("green")
```

---

## インスタンス化の流れ

`instantiate(class_val, evaled_args)`:

1. `class_val.field_defaults` の初期値を評価してフィールドマップを構築
2. `InstanceData { class, fields, immutable: false }` を `Rc<RefCell<...>>` で生成
3. `__init__` メソッドを検索してバインド
4. `exec_fn_evaled` で `__init__` を実行
5. `Value::Instance(...)` を返す

---

## 特殊メソッド一覧

| メソッド名 | 呼ばれる場面 |
|---|---|
| `__init__(mut self, ...)` | インスタンス化時 |
| `__repr__(self)` | `repr(obj)` / デバッグ表示 |
| `__str__(self)` | `str(obj)` / `print(obj)` |
| `__len__(self)` | `len(obj)` |
| `__iter__(self)` | `for x in obj` (gen メソッド) |
| `__next__(self)` | イテレータの次要素取得 |
| `__getitem__(self, let key)` | `obj[key]` |
| `__setitem__(mut self, let key, let val)` | `obj[key] = val` |
| `__add__(self, let other)` | `obj + other` |
| `__cast__[T](let other: T)` | 自動キャスト |
| `__freeze__(mut self)` | `freeze obj` 実行時 |

---

## クラスメソッドの検索順序

メソッドを呼び出すとき:

1. `instance.fields` を検索 (フィールド値に関数が格納されている場合)
2. `instance.class.methods` を検索
3. `instance.class.bases` (継承した trait) を再帰的に検索
4. 見つからなければ `AttributeError`

trait のデフォルト実装も `bases` 経由で検索されます。
