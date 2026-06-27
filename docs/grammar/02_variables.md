# 変数宣言とスコープ

---

## 変数宣言の種類

### `let` — 不変変数

```hv
let x = 42
let name = "Alice"
let pi = 3.14159
```

**パース**: `Token::Let ident [':', type_expr] '=' expr`  
**実行**: 右辺の式を評価して `Var::Immutable(value)` としてスコープに登録。  
再代入は実行時 `TypeError`。型検査は `StaticTypeError::AssignToImmutable` を報告。

### `mut` — 可変変数

```hv
mut count = 0
mut items: list[str] = []
count = count + 1   # 再代入可能
```

**パース**: `Token::Mut ident [':', type_expr] '=' expr`  
**実行**: 右辺の式を **ディープコピー** して `Var::Mutable(value)` として登録。  
ディープコピーにより、リスト等の参照型でも別オブジェクトになります。

### `const` — 定数

```hv
const MAX = 100
const GREETING = "hello"
```

**パース**: `Token::Const ident [':', type_expr] '=' expr`  
**実行**: `let` と同様に `Var::Immutable(value)` として登録。  
意味的に「定数であること」を表明します。

### `static mut` — 静的可変変数

```hv
fn counter() -> int:
    static mut n = 0
    n += 1
    return n
```

**パース**: `Token::Static Token::Mut ident [':', type_expr] '=' expr`  
**実行**: 宣言の Span (ファイル名・行・列) をキーにして `Interpreter.static_cells` に  
`Rc<RefCell<Value>>` のセルを確保。初回呼び出し時のみ初期化されます。  
以降の呼び出しでは同じセルを共有するため、関数を超えて値を保持します。

---

## タプルアンパック宣言

```hv
let x, mut y = (10, 20)
let a, mut b, _ = some_tuple   # _ は残余要素を破棄
```

**パース**: 先頭が `let`/`mut` → カンマが続けばタプルアンパックに切り替わる  
**各ターゲット**:
- `let name` → `TupleTarget::Let(name)` — 不変バインディング
- `mut name` → `TupleTarget::Mut(name)` — 可変バインディング
- `_` → `TupleTarget::Wildcard` — 残余要素の破棄 (末尾のみ)
- 修飾子なし識別子 → 静的型エラー `TupleUnpackMissingQualifier`

**実行**: 右辺を評価してタプル/リストと判断し、各要素を順に対応するターゲットにバインドします。

---

## `freeze` 文

```hv
mut data = [1, 2, 3]
# ... data を構築 ...
freeze data   # data を let (不変) に降格する
```

**パース**: `Token::Freeze ident`  
**実行**:
1. 変数のミュータビリティを `Var::Mutable` → `Var::Immutable` に変更
2. 変数の値が `__freeze__` メソッドを持つ場合は呼び出す  
   (カスタム freeze プロトコル: フラットメモリ展開などに使用)
3. フラット化されたリスト → `Value::FrozenList` に変換

`freeze` は `__freeze__` を直接呼び出す(`inst.__freeze__()`)ことはできません。  
必ず `freeze` キーワードを使用する必要があります (静的型エラー `DirectFreezeCall`)。

---

## 変数の再宣言禁止

`let` / `mut` / `const` / タプルアンパックで宣言する変数の名前が、  
その時点からアクセス可能な **任意のスコープ** に既に存在する場合はエラーです。

```hv
let a = 5
let a = 6   # StaticTypeError / NameError: variable 'a' is already declared
```

```hv
let x = 1
if True:
    let x = 2   # エラー: 外側スコープの x と衝突
```

```hv
let x = 1
fn f() -> None:
    let x = 2   # エラー: 外側スコープの x と衝突
```

**エラー種別**:
- 静的型検査: `TypeErrorKind::VariableRedeclaration { name }` → `StaticTypeError`
- ランタイム: `NameError: variable '<name>' is already declared`

**例外 — `_`（捨て変数）**:  
`_` は何度でも再宣言できます。戻り値を意図的に無視するための慣用パターンです。

```hv
let _ = f.read_letter()
let _ = f.read_letter()   # OK: _ は例外
```

**注意**: 外側スコープの変数を **参照する**（読み取る）ことは引き続き許可されます。  
禁止されるのはあくまで同名の **新規宣言** のみです。

```hv
let x = 5
fn f() -> None:
    print(x)   # OK: 参照は禁止されていない
```

---

## スコープ規則

### レキシカルスコープ

```hv
let outer = 10

fn foo() -> int:
    return outer   # 外側スコープの変数を参照可能
```

- スコープスタック (`Vec<HashMap<String, Var>>`) の末尾から先頭に向けて変数を検索
- インデックス 0 がグローバルスコープ、末尾がローカルスコープ

### ブロックスコープ

制御構文 (`if`/`for`/`while`/`match`/`block`) の中で宣言された変数は  
そのブロックを抜けると破棄されます (Python とは異なります)。

```hv
if condition:
    let temp = "inside"   # この変数はブロック内のみ有効

# ここで temp を参照すると NameError
```

**実装**: 制御構文の実行前後に `push_scope()` / `pop_scope()` を呼びます。

### 関数スコープ

関数呼び出し時:
1. グローバルスコープ以外のスコープを一時退避
2. 新しいローカルスコープを構築して引数をバインド
3. 関数本体を実行
4. スコープを復元

```hv
mut global_x = 0

fn modify() -> None:
    global_x = 1   # グローバルスコープの変数を変更可能
```

---

## クロージャとキャプチャ

関数定義時に外側スコープの変数をキャプチャします。

```hv
fn make_adder(let n: int) -> function->int:
    fn adder(let x: int) -> int:
        return x + n   # n をキャプチャ
    return adder
```

**キャプチャ動作**:
- `let` 変数 → `CapturedVar::Immutable(value)` — 定義時点の値をディープコピーして保持
- `mut` 変数 → `CapturedVar::Mutable(Rc<RefCell<Value>>)` — 外側スコープと同じセルを共有

`mut` キャプチャでは関数内外で値が共有されます:

```hv
mut counter = 0

fn increment() -> None:
    counter += 1   # counter セルを直接変更

increment()   # counter == 1
increment()   # counter == 2
```

> `nonlocal` キーワードは不要です。外側スコープの変数を `mut` で宣言すれば  
> 内側関数から自動的に共有セルとして扱われます。

---

## 変数の実行時表現

```rust
pub(self) enum Var {
    Immutable(Value),         // let / const — 不変
    Mutable(Value),           // mut — 可変 (セル化前)
    Cell(Rc<RefCell<Value>>), // クロージャにキャプチャされた mut 変数
}
```

`Var::Mutable` はクロージャにキャプチャされた時点で `Var::Cell` に昇格します。

---

## `copy()` メソッド

すべてのインスタンスは組み込みの `copy()` メソッドを持ちます。  
`copy()` は呼び出し元インスタンスの **ディープコピー** を新しくメモリ上に確保して返します。

```hv
mut p1 = Point(10, 20)
mut p2 = p1.copy()   # p2 は独立したコピー
p2.x = 99
print(p1.x)   # 10 — p1 は変更されない
```

### デフォルト動作

- すべてのフィールドを再帰的にディープコピーした新しいインスタンスを生成します
- 戻り値は **フリーズなし** (`immutable = false`) の新鮮なインスタンスです。  
  `let` バインドによるフリーズは解除されるため、コピー先での変更が可能です
- メモリ不足の場合は `MemoryError` を raise します

### `__copy__` による挙動のカスタマイズ

クラスに `__copy__(let self) -> ClassName:` メソッドを定義すると、  
`copy()` 呼び出し時にデフォルトのディープコピーの代わりにそのメソッドが使われます。

```hv
class Named:
    mut name: str
    mut tag: int

    fn __init__(mut self, name: str, tag: int) -> None:
        self.name = name
        self.tag = tag

    fn __copy__(let self) -> Named:
        mut c = Named(self.name + "_copy", self.tag + 1)
        return c

mut n1 = Named("hello", 0)
mut n2 = n1.copy()   # __copy__ が呼ばれる
# n2.name == "hello_copy", n2.tag == 1
```

**解決ルール**:
1. クラスに `__copy__` が定義されており、引数なし（`self` のみ）で呼び出せる  
   オーバーロードが存在すれば、それを呼び出す
2. 一致するオーバーロードがなければ、デフォルトのディープコピーにフォールバック

> `__copy__` 内で新しいインスタンスを生成する際は `mut` で宣言してください。  
> `let` で宣言するとインスタンスがフリーズされ、返された後に変更できなくなります。

---

## ミュータビリティと値渡し

| 宣言 | 代入時の動作 | 関数引数の動作 |
|---|---|---|
| `let x = val` | コピーなし (参照のみ) | `let` 引数へ渡す際もコピーなし |
| `mut x = val` | **ディープコピー** | `let` 引数へは `copy()` を適用; `mut` 引数へは参照共有 |

`mut` 変数を別の `mut` 変数に代入すると、**ディープコピー** が作成されます。  
これにより意図しない値の共有を防ぎます。

### `let` パラメータへの引数渡し

インスタンスを `let` パラメータへ渡す際、`copy()` のロジックが適用されます。

- クラスに `__copy__` があれば呼び出される（独立したコピーを返す）
- `__copy__` がなければ `deep_copy_unfrozen`（デフォルトディープコピー）が使われる
- 元のインスタンスが `let` バインドでフリーズされていても、  
  関数内のパラメータは **フリーズなし** の新鮮なコピーとして受け取られる
- `mut` パラメータは引き続き参照共有（コピーなし）

```hv
fn transform(let p: Point) -> None:
    p.x = p.x * 2   # コピー上での変更。呼び出し元の p3 には影響しない

mut p3 = Point(5, 6)
transform(p3)
# p3.x はまだ 5
```

> `let self` パラメータはこのルールの例外です。`let self` は現行のディープコピー  
> を使用します（`__copy__` は呼び出されません）。これにより `__copy__` 内で  
> `let self` を使っても無限再帰が発生しません。
