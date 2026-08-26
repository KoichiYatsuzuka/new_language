# 制御構文

Arrow の制御構文はすべて **文** としても **式** としても使用できます。  
式として使う場合は `->Type` アノテーションを付けて `block_return` / `loop_yield` で値を返します。

---

## if 文

```ar
if condition:
    # 真のとき実行
elif other_condition:
    # 別の条件が真のとき実行
else:
    # いずれも偽のとき実行
```

`Stmt::If { branches: Vec<(Expr, Vec<Stmt>)>, else_body: Option<Vec<Stmt>> }`

**パース**: `if expr ':' block ('elif' expr ':' block)* ('else' ':' block)?`

**実行** (`exec_if_stmt`):
1. 各 `(条件式, ボディ)` を順に評価
2. 最初に `is_truthy()` が `true` になった条件のボディを `exec_scoped_block()` で実行
3. すべて `false` で `else` があれば `else` ボディを実行
4. マッチなし・`else` なし → `ExecResult::Normal` で終了

### 型ガードナロイング

```ar
let x: Option[int] = get_value()

if x is not None:
    # このブロック内で x は int 型として型検査される
    let doubled = x * 2
```

型検査器は `if` ブランチの条件に `is` / `is not` が含まれる場合、  
そのブランチのスコープ内でオペランドの型を絞り込みます。

#### `Result[T, E]` ガード

`Result[T, E]` 型の変数は `is_OK()` / `is_ERR()` メソッドをガード条件として使うことで、  
ブランチ内の変数が内部値の型に絞り込まれます。

```ar
let r: Result[int, str] = divide(10, 2)

if r.is_OK():
    # このブロック内で r は int（Ok の内部値）として扱える
    print(r)

if r.is_ERR():
    # このブロック内で r は str（Err の内部値）として扱える
    print(r)
```

ガード節なしで `Result` 変数に属性アクセス・演算を行うと  
静的型エラー `OperationOnUnion` が報告されます。

---

## match 文

```ar
match value:
    case 1:
        print("one")
    case 2:
        print("two")
    case _:
        print("other")
```

`Stmt::Match { subject: Expr, arms: Vec<MatchArm>, span: Span }`

### case パターン (値比較)

```ar
match code:
    case 200:
        handle_ok()
    case 404:
        handle_not_found()
    case _:      # ワイルドカード — 常にマッチ
        handle_unknown()
```

`MatchPattern::Case(expr)` — `subject == pattern` で比較します。  
`case _:` は `Expr::Ident("_")` として解析され、常に `true` を返します。

### is パターン (型チェック)

```ar
match value:
    is int:
        print("integer:", value)
    is str:
        print("string:", value)
    is MyClass:
        value.method()
```

`MatchPattern::IsType(String)` — `isinstance(subject, TypeName)` 相当の検査です。

> **制約**: 1 つの `match` 文内で `case` パターンと `is` パターンを  
> **混在させることはできません** (パースエラー)。

**実行** (`exec_match_stmt`):
1. サブジェクト式を評価
2. 各アームを上から順に評価
3. `case expr` → `values_eq(subject, pattern)` が `true` であれば選択
4. `is TypeName` → `type_name(subject) == TypeName` が `true` であれば選択
5. 最初にマッチしたアームのボディを `exec_scoped_block()` で実行

---

## for 文

```ar
for item in collection:
    process(item)

for i, val in enumerate(items):   # タプルアンパック
    print(i, val)
```

`Stmt::For { targets: Vec<String>, iter: Expr, body: Vec<Stmt> }`

**タプルアンパック**: `targets` が複数要素の場合、イテレータの各要素が  
タプルであることを期待して各名前にバインドします。

**実行** (`exec_for_stmt`):
1. `iter` 式を評価
2. 値に応じてイテレーション方法を決定:
   - `List` / `FrozenList` → インデックスで順次アクセス
   - `Str` → 各文字
   - `Dict` → キーの一覧
   - `Set` → 要素の一覧
   - `Tuple` → 各要素
   - `Generator` → `next()` を繰り返す
   - `Range` インスタンス → `__iter__` / `__next__` を呼ぶ
3. 各イテレーションで新しいスコープを積んで `targets` にバインド
4. ボディを実行; `ExecResult::Break` で脱出、`ExecResult::Continue` で次へ

### 組み込みイテラブル

```ar
for i in range(10):
    print(i)       # 0, 1, ..., 9

for i in range(1, 5):
    print(i)       # 1, 2, 3, 4

for i in range(0, 10, 2):
    print(i)       # 0, 2, 4, 6, 8
```

---

## while 文

```ar
mut i = 0
while i < 10:
    print(i)
    i += 1
```

`Stmt::While { cond: Expr, body: Vec<Stmt> }`

**実行** (`exec_while_stmt`):
1. `cond` を評価して `is_truthy()` をチェック
2. `true` であればボディを実行; `false` であれば終了
3. `ExecResult::Break` で脱出、`ExecResult::Continue` で次のイテレーションへ

---

## break / continue / pass

```ar
for item in items:
    if item < 0:
        continue      # このイテレーションをスキップ
    if item > 100:
        break         # ループを脱出
    process(item)

while True:
    pass   # 空文 (構文上ボディが必要な箇所)
```

- `break` → `ExecResult::Break` を返してループを脱出
- `continue` → `ExecResult::Continue` を返して次のイテレーションへ
- `pass` → `ExecResult::Normal` (何もしない)

`break`/`continue` はループ外 (ループ深さ 0) で使用すると `SyntaxError`。

### 制御構文式の中での break

```ar
for i in range(10):
    let found = if check(i) ->bool:
        block_return True
    else:
        block_return False
    if found:
        break   # break は if 式を通り抜けてループを脱出する
```

`break` は `block:` / `if` / `match` 式のボディを通り抜けて  
外側の最も近い `for`/`while` ループに到達します。  
(内部では `BREAK_SENTINEL` エラー文字列を伝播させて実現)

---

## block 文

```ar
block:
    let temp = heavy_computation()
    process(temp)
# temp はここでは参照不可
```

`Stmt::Block(Vec<Stmt>)`

独立したスコープを提供します。ブロック内で宣言した変数は外に漏れません。  
**実行**: `exec_scoped_block()` で新しいスコープを積んでボディを実行します。

---

## 制御構文の式としての使い方

`->Type` アノテーションを付けることで制御構文を **式** として使用できます。

### block 式

```ar
let result = block ->int:
    mut acc = 0
    for i in range(10):
        acc += i
    block_return acc
```

### if 式

```ar
let max_val = if a > b ->int:
    block_return a
else:
    block_return b
```

### for 式 と loop_yield

```ar
let evens = for i in range(20) ->list[int]:
    if i % 2 == 0:
        loop_yield i
```

`loop_yield val` — 値をリストに蓄積しつつ実行を継続します。  
ループ終了時に蓄積されたリストを返します。

### while 式

```ar
mut buf = []
let total = while len(buf) < target_len ->int:
    buf.append(read_next())
    if error_flag:
        block_return -1
block_return sum(buf)
```

### match 式

```ar
let label = match score ->str:
    case _ if score >= 90:
        block_return "A"
    case _ if score >= 80:
        block_return "B"
    case _:
        block_return "C"
```

---

## block_return と loop_yield の制約

| 機能 | 有効な場所 | 無効な場所 |
|---|---|---|
| `block_return` | `block:` / `if` / `match` / `for` / `while` 式の中 | `for`/`while` 式の **直下** ボディ |
| `loop_yield` | `for`/`while` 式の中 | `for`/`while` 式の外 |

`for`/`while` 式の直下で `block_return` を使うと  
静的型エラー `BlockReturnInLoopExpr` が発生します。  
その場合は `loop_yield` を使うか、内側に `block:` / `if` 式を挟んでください。

---

## ループ深さの追跡

```rust
thread_local! {
    static LOOP_DEPTH: RefCell<usize> = RefCell::new(0);
}
```

`for`/`while` の実行開始時に `+1`、終了時に `-1` します。  
`break`/`continue` はこの値が 0 のときに `SyntaxError` を返します。
