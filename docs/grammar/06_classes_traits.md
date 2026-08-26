# クラス・trait・new_type・enum

---

## クラス定義

```ar
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

### クラスの実行時表現

クラス定義を実行すると `ClassValue` が生成されます:

```rust
pub struct ClassValue {
    pub name:       String,
    pub class_id:   u32,          // 宣言時に alloc_class_id() で発行した一意 ID
    pub is_exception: bool,       // 例外クラス（make_error_class 経由）なら true
    pub field_index: HashMap<String, usize>,  // フィールド名 → Vec インデックス
    pub field_count: usize,       // フィールドスロット総数
    // ... methods, field_defaults, class_vars, static_vars 等
}
```

`class_id` はインスタンスの `InstanceData.class_id` にコピーされ、コンパイル済みコードからの  
型判定・フィールド GEP（オフセット計算）に使用されます。  
詳細なメモリレイアウトは [02_variables.md](02_variables.md) の「インスタンスのメモリ表現」を参照してください。

---

## フィールド宣言

```ar
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

```ar
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

```ar
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

```ar
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

```ar
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

```ar
trait Builder:
    fn set_name(mut self, let name: str) -> Self: ...
    fn build(self) -> Self: ...
```

---

## protocol 定義

`protocol` は**構造的型付け**（ダックタイピング）のための型宣言です。  
`trait` が明示的な継承（名前的型付け）を要求するのに対し、`protocol` は  
必要なメンバーを持っていれば自動的に適合とみなします（Go のインターフェースに相当）。

```ar
protocol Drawable:
    fn draw(self) -> None:
        ...
    fn area(self) -> float:
        ...

protocol Resizable:
    fn resize(self, factor: float) -> None:
        ...
```

`Stmt::ProtocolDef { name, body }`

**パース**: `'protocol' ident ':' class_body`

**制限**:
- `protocol` はインスタンス化できません（静的型エラー `ProtocolInstantiation`）
- `protocol` を継承ベースに指定することはできません
- `protocol` 本体内に `private:` / `protected:` セクションを使えません（すべて `public`）
- メソッドボディは必ず `...` (Ellipsis) にしてください

---

### protocol 適合（コンフォーマンス）

クラスが protocol のすべてのメンバー（メソッド・フィールド）を持っている場合、  
そのクラスは自動的に protocol に適合しているとみなされます。  
明示的な継承宣言は不要です。

```ar
class Circle:
    mut radius: float

    fn __init__(mut self, r: float) -> None:
        self.radius = r

    fn draw(self) -> None:
        print("Drawing circle with radius", self.radius)

    fn area(self) -> float:
        return 3.14159 * self.radius * self.radius

    fn resize(mut self, factor: float) -> None:
        self.radius = self.radius * factor
```

`Circle` は `draw`・`area`・`resize` をすべて持つため、`Drawable` と `Resizable` の両方に適合します。

---

### protocol 型変数への代入

変数宣言時に `: ProtocolName` 型アノテーションを付けると、  
代入時に静的型チェッカーが適合検査を行います。

```ar
mut c = Circle(5.0)
let d: Drawable = c   # 適合検査: Circle が Drawable を満たすか確認
```

適合していない場合は静的型エラー `ProtocolConformanceFailed` が報告されます。

```ar
class Dog:
    let name: str
    fn __init__(self, n: str) -> None: ...
    # print_info メソッドがない

let d = Dog("Rex")
let p: Printable = d  # StaticTypeError: type 'Dog' does not satisfy protocol 'Printable': missing method `print_info`
```

---

### protocol 型パラメータ

関数の引数型に `protocol` 名を指定できます。  
呼び出し時に適合検査が行われます。

```ar
fn render(shape: Drawable) -> None:
    shape.draw()
    print("Area:", shape.area())

fn make_bigger(mut shape: Resizable) -> None:
    shape.resize(2.0)

mut c = Circle(5.0)
let d1: Drawable = c
render(d1)       # OK: d1 は Drawable
make_bigger(c)   # OK: Circle は Resizable に適合
```

**注意**: 呼び出し先でインスタンスを変更したい場合は `mut shape: ProtocolName` と宣言します。

---

### `is Protocol` 型ガード

`is` 演算子で protocol 適合を実行時に検査できます。

```ar
if c is Drawable:
    print("Circle satisfies Drawable")
```

**静的型チェック**: 条件分岐内で変数型を `Protocol(name)` に絞り込みます。  
**実行時**: インスタンスが protocol の全必須メンバー名を持っているか確認します。

---

### trait との違い

| | `trait` | `protocol` |
|---|---|---|
| 型付け方式 | 名前的（継承必須） | 構造的（Duck typing） |
| 適合の宣言 | `class Foo(MyTrait):` が必要 | 不要（メンバーを持てば自動適合） |
| インスタンス化 | 不可 | 不可 |
| デフォルト実装 | 持てる | 持てない（`...` のみ） |
| テンプレート境界 | `[T: MyTrait]` | `[T: MyProtocol]` |
| 継承 | クラスが実装 | なし |

```ar
# trait → 明示的な継承が必要
trait Printable:
    fn print_info(self) -> None: ...

class Cat(Printable):        # (Printable) の宣言が必要
    fn print_info(self) -> None:
        print("Cat")

# protocol → 宣言不要、メンバーを持てば自動適合
protocol Displayable:
    fn display(self) -> None:
        ...

class Dog:                   # (Displayable) の宣言なし
    fn display(self) -> None:
        print("Dog")

let d: Displayable = Dog()   # OK: Dog は display を持つ
```

---

## `Intersection` 型と trait の組み合わせ

`Intersection[T1, T2, ...]` を使うと、複数の trait を**同時に**要求する型制約を表現できます。  
詳細は [08_type_system.md](08_type_system.md) を参照してください。

```ar
trait Flyable:
    fn fly(self) -> str: ...

trait Swimmable:
    fn swim(self) -> str: ...

class Duck(Flyable, Swimmable):
    let name: str
    fn __init__(mut self, n: str) -> None: self.name = n
    fn fly(self) -> str: return self.name + " flies"
    fn swim(self) -> str: return self.name + " swims"

# Flyable かつ Swimmable の両方を要求するパラメータ
fn demo(creature: Intersection[Flyable, Swimmable]) -> None:
    print(creature.fly())    # どちらの trait のメンバーにもアクセス可能
    print(creature.swim())

demo(Duck("Donald"))
```

**継承によって満たされる**: `Duck(Flyable, Swimmable)` と宣言されているため、  
`Duck` インスタンスは `Intersection[Flyable, Swimmable]` として渡せます。  
`class Foo(A, B)` の継承宣言がない場合は型チェックエラーになります。

| 型 | 適合の条件 | アクセス |
|---|---|---|
| `trait T` | 明示的な継承 `class Foo(T):` | メンバーアクセス OK |
| `protocol P` | 必要なメンバーを持つだけで自動適合 | メンバーアクセス OK |
| `Intersection[A, B]` | `A` と `B` の両方の条件を満たす | A・B 双方のメンバーへ直接アクセス OK |

---

## テンプレートクラス

```ar
class Stack[T]:
    mut items: list[T]

    fn push(mut self, let item: T) -> None:
        self.items.append(item)

    fn pop(mut self) -> T:
        return self.items.pop()
```

**使用**:

```ar
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

```ar
new_type Celsius: float
new_type UserId: int
new_type AdminId: UserId
```

`Stmt::NewTypeDef { name, original }`

**動作**: `original` の型定義をコピーして `name` という別名で登録します。

```ar
let temp = Celsius(36.5)   # コンストラクタで作成
let id: UserId = UserId(42)

# Celsius と float は構造的に同一だが型名が違うため演算不可
# temp + 1.0   # TypeError: Celsius と float の演算
```

**`Self` 型との組み合わせ**:

```ar
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

```ar
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

```ar
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
2. 初期 `flags` を計算（`is_exception` → `INST_IS_EXCEPTION`、`new_type_base.is_some()` → `INST_IS_NEW_TYPE`）
3. `InstanceData { class_id, flags, class, fields }` を `Rc<RefCell<...>>` で生成
4. `__init__` メソッドを検索してバインド
5. `exec_fn_evaled` で `__init__` を実行
6. `Value::Instance(...)` を返す

`class_id` は `ClassValue.class_id` から引き継がれます（`alloc_class_id()` で宣言時に発行済み）。

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
