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
    pub is_exception: bool,       // 組み込みの例外クラス（make_error_class 経由）なら true
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
| `FieldKind::Const` | `const` | クラス変数。全インスタンスで共有。代入不可（静的エラー `AssignToConst`） |
| `FieldKind::StaticMut` | `static mut` | 可変クラス変数。全インスタンスで共有 |

**`const` は値ごと不変**（クラス・trait・enum のメンバー・モジュールの `const` のどれでも同じ）。
書き換えられるかは**型ではなく属性**（`const` / `let` / `mut`）で決まる
（import したモジュールのグローバル変数は `09_imports.md` の「グローバル変数の型と属性」）:

- **代入**: クラス名経由（`Counter.LIMIT = 5`）・インスタンス経由（`c.LIMIT = 5`）・
  `__init__` の中の `self.LIMIT = 5`・複合代入・trait の `const`・モジュールの `const`（`m.K = 5`）の
  どれでも静的エラー `cannot assign to const 'Counter.LIMIT'`（`AssignToConst`）。enum のメンバーも
  暗黙に `const` で、同じ規則で弾かれる（`Color.Red = ..`）
- **中身の書き換え**: `const` の要素・フィールド・書き換えるメソッド（`Counter.NAMES[0] = ..` /
  `c.NAMES.append(..)` / `c.ORIGIN.x = ..` / メソッドの中の `self.NAMES.append(..)` / `m.NAMES[0] = ..`）も
  静的エラー `cannot modify the contents of const 'Counter.NAMES'`（`ModifyConst`）。経路のどこに
  `const` があっても同じで、根の変数が `mut` でも書き換えられない。`mut` の仮引数にも渡せない
- 読む・写しを作る（`mut copy = Counter.NAMES` の後の `copy.append(..)`）・`let` の仮引数に渡すのは通る
- `static mut` はどの経路でも代入でき、中身も書き換えられる（`Counter.items.append(..)`）

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
1. メンバーのクラスを作る。名前は enum と同じ `Color` で、`value: int` フィールドを 1 つ持つ（`build_enum_classes`）
2. `Color` クラスを作成し、各バリアント（メンバーのクラスのインスタンス）を `const` フィールドとして設定
3. 値は `0` から自動採番 (明示値がない場合)

**型**: メンバー（`Color.Red`）の型は **enum 型 `Color` そのもの**。`let c: Color = Color.Red`・
`c == Color.Red`・`c is Color`・`c mustbe Color`・`Color` 型の引数やフィールドがそのまま書ける。

- メンバーは型の値（`Color.Red`）からだけ、`value` はメンバー（`Color` 型の値）からだけ引ける。
  `c.Red` と `Color.value` は静的エラー
- 別の enum のメンバーとは比較も代入もできない（`Color.Red == Shape.Square` は静的エラー）
- メンバーは**値**で、変数ではない。メンバーは**暗黙に `const`** で、メンバーへの代入
  （`Color.Red = Color.Green` / `Color.Red += 1`）はクラスの `const` と同じ `AssignToConst`、
  メンバーの中身の書き換え（`Color.Red.value = 5`）は `ModifyConst`。変数に入れた写しの
  `m.value = 5` も静的エラー（`value` は不変のフィールド。実行時も不変）。
  メンバーを入れた変数の付け替え（`mut c = Color.Red` の後の `c = Color.Green`）は通る
- ⚠ 以前の内部名 `enum_item_Color` は廃止した（書くと静的エラー。
  経緯は `implementation_plans/enum_member_type_plan.md`）

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
4. 組み込みの例外の階層のクラスなら、作るときの位置引数を `args` に、CPython の `str(e)` を `message` に入れる
   （[07_exceptions.md](07_exceptions.md) の「組み込みの例外を作る・表示する」）
5. `__init__` メソッドを検索してバインド
6. `exec_fn_evaled` で `__init__` を実行
7. `Value::Instance(...)` を返す

`class_id` は `ClassValue.class_id` から引き継がれます（`alloc_class_id()` で宣言時に発行済み）。

---

## メソッドを値として読む（束縛メソッド）

`obj.method` を呼ばずに値として読むと、**受け手（`self`）を束縛した関数**になります（CPython の束縛メソッドと同じ）。

```ar
mut c = Counter(1)
let add = c.add                      # 受け手 c を束縛した関数
print(add(10))                       # c.add(10) と同じ
print(list(map(c.add, [1, 2])))      # コールバックにも渡せる（sorted(key=obj.m) / シグナルの on も）
print(c.get == c.get)                # True（同じ受け手の同じメソッドは等しい）
```

- 呼ぶと `obj.method(..)` と**同じ実体が同じ判定で**走ります（オーバーロードの選択・ネイティブのメソッドも）。
- **受け手の不変は呼ぶ時点で**見ます: `let c` / `freeze` した受け手の `mut self` のメソッドは、束縛はできますが
  呼ぶと `TypeError: cannot call mutable method '..' on immutable instance of '..'`（直接呼ぶのと同じ）。
  束縛した関数の値が不変（`let add = ..`）であることと、それが受け手を書き換えることは別の話です。
- アクセス制御は**読んだ時点で**見ます（クラスの中で `self._key` を取り出してコールバックに渡せば、外で呼ばれても通る）。
- 表示は `<bound method Counter.add of <Counter object at 0x..>>`。シグナルの `off` も同じ照合で外れます。
- クラスから読む `C.method` は従来どおり束縛しない関数です（`C.method(obj, ..)` で呼ぶ）。
- `gen` メソッドも読めます（呼ぶとジェネレータ）。組み込みの値のメソッドも読めます（`push = out.append` /
  `"s".upper` / `d.items` / `seen.add`。同じ値を書き換えます）。読める名前は `type_check/names.rs` の
  `BUILTIN_VALUE_METHODS`（`complex` の `real` などは読めません）。表示は `<built-in method append of list object at 0x..>`。
- **型**: `c.add` は `self` を除いたシグネチャの関数型（`function{let k: int}->int`）で、引数・戻り値を静的に検査します
  （多重定義・可変長のメソッドは型引数なしの `function`）。⚠ 異なる関数型どうしの `==` は静的な誤りです。
- **不変の受け手**: 型検査は「不変の受け手を書き換える」印を関数型に付け、**呼んだ行**を
  `cannot call 'add' on 'c' — it is immutable`（`c.add(1)` と同じ誤り）にします。束縛した行は誤りにしません。
  型検査が追えない渡し方（関数の引数に渡して中で呼ぶ）でも、インスタンスは実行時に止まります。⚠ list / dict / set は
  実行時の不変の印を持たないので、そこは止まりません。
- 実装: `FnValue::bound`（`BoundMethod { receiver, target }`・`target` は `Methods` / `Gen` / `Builtin`）。呼ぶと
  `exec_fn_evaled` の入口で `call_bound_method` へ回ります（`Value` に種類は足していません）。

---

## 属性に入れた関数を呼ぶ

メソッドの無い名前の `obj.f(x)` は、**属性を読んでその値を呼びます**（CPython と同じ）。フィールド・`const`・
`static mut` に入れた関数（コールバック）をそのまま呼べます。

```ar
fn double(let x: int) -> int:
    return x * 2

class Holder:
    let f: function[let int]->int
    fn run(self, let x: int) -> int:
        return self.f(x)              # フィールドの関数を呼ぶ

let h = Holder(double)
print(h.run(21))                      # 42
print(h.f(4))                         # 8
```

- 同じ名前のメソッドがあればメソッドが先です（下の「メソッドの検索順序」）。
- 読みは `obj.f` と同じ経路なので、アクセス制御・ディスクリプタ（次節）も同じに働きます。
- **型**: フィールドの関数型で引数・戻り値を検査します。呼べない値（`let n: int` の `b.n(1)`）は
  `'int' is not callable`（値を呼ぶのと同じ誤り）。

---

## ディスクリプタ（`__get__` / `__set__`）

クラス側の値（`static mut` / `const`）が `__get__` を持つインスタンスなら、その属性は**ディスクリプタ**として働きます
（CPython のディスクリプタのプロトコル。Python の `property` もこの仕組み）。

```ar
class Fahrenheit:
    fn __get__(self, mut obj: Thermo, owner: Any = None) -> float:
        return obj.celsius * 9.0 / 5.0 + 32.0
    fn __set__(self, mut obj: Thermo, value: float) -> None:
        obj.celsius = (value - 32.0) * 5.0 / 9.0

class Thermo:
    mut celsius: float
    static mut fahrenheit: Fahrenheit = Fahrenheit()
    fn __init__(mut self, c: float) -> None:
        self.celsius = c

mut t = Thermo(100.0)
print(t.fahrenheit)       # 212.0（__get__(t, Thermo)）
t.fahrenheit = 32.0       # __set__(t, 32.0)
print(t.celsius)          # 0.0
```

| 書き方 | 働き |
|---|---|
| `obj.x`（読み） | ① **データディスクリプタ**（`__set__` か `__delete__` も持つ）なら `__get__(obj, C)` ② フィールド ③ 非データディスクリプタ（`__get__` だけ）なら `__get__(obj, C)` ④ クラス側の値そのもの ⑤ メソッド（束縛メソッド） |
| `obj.x = v` | データディスクリプタなら `__set__(obj, v)`（`__set__` が無ければ `AttributeError: __set__`）。それ以外は従来どおり |
| `C.x`（クラスから読む） | ディスクリプタなら `__get__(None, C)` |
| `C.x = d`（クラスへ代入） | メンバーそのものを置き換える（`__set__` は呼ばない） |
| `obj.x(..)` | 上の読みで得た値を呼ぶ（前節） |

### データディスクリプタと非データディスクリプタ

| 種類 | 持つメソッド | インスタンスから読む | インスタンス経由で代入する |
|---|---|---|---|
| データディスクリプタ | `__get__` と、`__set__` か `__delete__` | **フィールドより先に** `__get__(obj, C)` | `__set__(obj, v)`。`__set__` が無ければ `AttributeError: __set__` |
| 非データディスクリプタ | `__get__` だけ | 同じ名前のフィールドが無ければ `__get__(obj, C)` | ディスクリプタを通らない（`static mut` なら共有セルを書き換える） |

非データディスクリプタは「読むたびに計算する値」に使えます。`const` に置いてもかまいません:

```ar
class Greeting:
    fn __get__(self, mut obj: Person, owner: Any = None) -> str:
        return f"Hello, {obj.name}"

class Person:
    let name: str
    const greeting: Greeting = Greeting()
    fn __init__(mut self, name: str) -> None:
        self.name = name

let p = Person("Ada")
print(p.greeting)             # Hello, Ada
print(p.greeting.upper())     # HELLO, ADA（読んだ値は str）
```

- ⚠ Arrow のクラスではフィールドとクラス側の値に同じ名前を付けられないので、「フィールドより先か後か」の違いが
  出るのは Python から変換したクラス（`__init__` が `self.x = ..` と書くクラス）だけです。

### クラスから読む・関数を返す

クラスから読むと `__get__` の `obj` は `None` です（CPython と同じ）。`__get__` が関数を返せば、その場で呼べます
（「属性に入れた関数を呼ぶ」と同じ経路）:

```ar
class Where:
    fn __get__(self, obj: Any, owner: Any = None) -> str:
        if obj is None:
            return "through the class"
        return "through an instance"

class Spot:
    static mut where: Where = Where()

print(Spot.where)             # through the class
print(Spot().where)           # through an instance

fn double(x: int) -> int:
    return x * 2

class Op:
    fn __get__(self, obj: Any, owner: Any = None) -> function[let int]->int:
        return double

class Calc:
    static mut op: Op = Op()

print(Calc().op(21))          # 42
```

### 型と注意

- **型**: 読んだ値は `__get__` の戻り値の型、インスタンス経由の代入は `__set__` の `value` の型、クラス経由の代入は
  メンバーの宣言型（ディスクリプタのクラス）と照合します（[08_type_system.md](08_type_system.md) の「属性の型」）。
  `const` に置いたデータディスクリプタへの代入は `__set__` を呼ぶので不変の誤りにはなりません。
  ```ar
  t.fahrenheit = "hot"      # StaticTypeError: field 'fahrenheit' of class 'Thermo' is declared 'float' but got 'str'
  ```
- 複合代入 `t.fahrenheit += 18.0` は `__get__` で読んで `__set__` で書きます。
- ⚠ `obj` を書き換える `__set__` / `__get__` は仮引数を `mut obj` にしてください（`let` の仮引数は `mut` の実引数を写すので、
  書き換えが元に届かない呼び方があります）。
- ⚠ `__get__` が `self` を返すとき（クラスから読んだら自分自身を返す形）は `mut self` にしてください。`self` のままだと
  `return self` は写しを返します（Arrow の値の規則）。組み込みの `property` はそうしています。
- ⚠ インスタンスのフィールド（`mut f: D`）に入れた値はディスクリプタとして働きません（CPython と同じ）。
- ⚠ `del obj.x`（`__delete__`）は Arrow に属性を消す構文が無いので使えません。
- 実装: `eval/attrs.rs` の `get_attr_val` / `attr_assign_evaled` / `set_attr_val`（3 つの入口だけ）。型検査は
  `TypeChecker::class_descriptor`。例題は `examples/classes/descriptor.ar`（CPython と同じ出力）・`descriptor_error.ar`。

---

## 組み込みの `property`

CPython 3.12 の `property` と同じ振る舞いのクラスです（データディスクリプタ）。Python の `@property` / `@x.setter` は
変換器がこれで組み立てます（[09_imports.md](09_imports.md) の「クラスの属性・property」）。

| 書き方 | 働き |
|---|---|
| `property(fget=None, fset=None, fdel=None, doc=None)` | 作る。欄 `fget` / `fset` / `fdel` / `__doc__` で読める |
| `p.getter(f)` / `p.setter(f)` / `p.deleter(f)` | その関数だけを差し替えた**新しい** property を返す（名前も写す） |
| `p.__set_name__(owner, name)` | 誤りの文言に使う名前を入れる（CPython ではクラスを作るときに呼ばれる） |
| `obj.x` / `obj.x = v` | `fget(obj)` / `fset(obj, v)`。無ければ CPython と同じ `AttributeError` |
| `C.x` | property そのもの（`C.x is C.x` は真・`type(C.x).__name__` は `property`） |

誤りの文言は CPython と同じです:

```text
AttributeError: property 'value' of 'ReadOnly' object has no setter
AttributeError: property of 'ReadOnly' object has no getter      # 名前を入れていない property
```

- `fget` などには束縛しない関数（`C.get_x`。呼ぶと `self` に `obj` が入る）も普通の関数も渡せます。
- ⚠ `property(..)` の値そのものの型は `Any` です（`property[T]` のような型は付けていません）。Arrow のクラスで型を付けたいときは、
  型を付けたディスクリプタのクラス（上の節）を書いてください。Python のクラスの `@property` は、getter の戻り値の注釈
  （`def x(self) -> float`）が Arrow から読んだ値の型になります（[09_imports.md](09_imports.md)・タスク 6-8）。
- ⚠ Arrow で書いたクラスで（`eval/py_builtins.rs` の `PY_PROPERTY_SRC`）、名前 `property` を初めて引いたときに組み立てます。
  CPython 3.12 の property は `__name__` を持たないので、名前は外から読めない欄に置いています。

---

## 特殊メソッド一覧

| メソッド名 | 呼ばれる場面 |
|---|---|
| `__init__(mut self, ...)` | インスタンス化時 |
| `__repr__(self)` | `repr(obj)` / コンテナの中の表示（`print([obj])` / `str((obj, 1))`）/ `__str__` が無いときの `str(obj)` / `print(obj)` |
| `__str__(self)` | `str(obj)` / `print(obj)` / `f"{obj}"`（コンテナの中では使われない。CPython と同じ） |
| `__len__(self)` | `len(obj)` |
| `__iter__(self)` | `for x in obj` (gen メソッド) |
| `__next__(self)` | イテレータの次要素取得 |
| `__getitem__(self, let key)` | `obj[key]` |
| `__setitem__(mut self, let key, let val)` | `obj[key] = val` |
| `__add__(self, let other)` | `obj + other` |
| `__cast__[T](let other: T)` | 自動キャスト |
| `__freeze__(mut self)` | `freeze obj` 実行時 |
| `__get__(self, mut obj, owner)` | クラス側に置いた値を読んだとき（ディスクリプタ・上の節。クラスから読むと `obj` は `None`） |
| `__set__(self, mut obj, value)` | インスタンス経由で代入したとき（データディスクリプタ） |
| `__delete__(self, mut obj)` | 持っているとデータディスクリプタになる（⚠ `del obj.x` の構文が無いので呼ばれるのは明示の呼び出しだけ） |
| `__set_name__(mut self, owner, name)` | Python のクラス本体の `property` を組み立てるとき（変換器が呼ぶ） |

---

## メソッドの検索順序

`obj.m(..)` を呼び出すとき:

1. クラスのメソッド表（`methods`。trait のデフォルト実装は定義時に平坦化済み）を検索する
2. 無ければ、クラス変数に入ったクラスを呼ぶ形（`Outer.Inner()`）
3. それも無く、フィールド・`const`・`static mut` にその名前があれば、**属性として読んで（ディスクリプタを含む）
   その値を呼ぶ**（「属性に入れた関数を呼ぶ」）
4. 見つからなければ `AttributeError: '..' object has no method '..'`
