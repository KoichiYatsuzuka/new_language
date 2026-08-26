# 式 (Expressions)

式は評価されると `Value` を返す構文要素です。  
パーサーは優先順位の低い演算子から高い演算子へと再帰的に解析します。

---

## 演算子の優先順位 (低→高)

| 優先順位 | 演算子・形式 | 結合性 |
|---|---|---|
| 1 | `or` | 左 |
| 2 | `and` | 左 |
| 3 | `not` | 右 (単項) |
| 4 | `==` `!=` `<` `>` `<=` `>=` `in` `not in` `is` `is not` | 非結合 |
| 5 | `\|` | 左 |
| 6 | `^` | 左 |
| 7 | `&` | 左 |
| 8 | `<<` `>>` | 左 |
| 9 | `+` `-` | 左 |
| 10 | `*` `/` `//` `%` `@` | 左 |
| 11 | `-x` `~x` (単項) | 右 |
| 12 | `**` | 右 |
| 13 | `=>` (キャスト) | 右 |
| 14 | `.attr` `[idx]` `(args)` | 左 (後置) |

---

## リテラル

```ar
42          # Expr::Int(42)
3.14        # Expr::Float(3.14)
"hello"     # Expr::Str("hello")
True        # Expr::Bool(true)
False       # Expr::Bool(false)
None        # Expr::None
```

**評価**: リテラルは対応する `Value` を直接返します。

---

## 識別子 (変数参照)

```ar
x
name
```

`Expr::Ident(String)` として AST に記録されます。  
**評価**: スコープスタックを末尾から先頭に向けて検索。見つからなければ `NameError`。

---

## f-string

```ar
f"x = {x}, y = {x + y:.2f}"
```

- `Expr::FStr` ではなく、パーサーがコンパイル時に `str.format()` 相当の式に展開
- `{expr}` 部分は式として再解析される
- `{{` / `}}` → エスケープされた `{` / `}`

**評価**: 各 `FStrPart::Lit(s)` は文字列そのまま、`FStrPart::Expr(src)` は  
ソーステキストを再パースして評価し `display()` で文字列化して連結。

---

## コレクションリテラル

### リスト

```ar
[1, 2, 3]                    # Expr::List([...])
let empty: list[int] = []    # 空リストは型アノテーション必須
```

**評価**: 各要素を左から順に評価して `Value::List(Rc<RefCell<Vec<Value>>>)` を生成。

### タプル

```ar
(1, "hello", True)           # Expr::Tuple([...])
(42,)                        # 末尾カンマで 1 要素タプル
```

**評価**: 各要素を評価して `Value::Tuple(Rc<TupleData>)` を生成。  
`(expr)` はグループ式 (タプルではない)。

### 辞書

```ar
{"key": value, x: y + 1}    # Expr::Dict([...])
```

**評価**: キー・値のペアを左から順に評価して `Value::Dict(Rc<RefCell<DictData>>)` を生成。

### セット

```ar
{1, 2, 3}                   # Expr::Set([...])
```

辞書と同様の構文ですが、パーサーが先読みで `{val, ...}` と `{key: val, ...}` を区別します。

---

## 算術演算子

```ar
a + b    # 加算 (str 連結にも使用)
a - b    # 減算
a * b    # 乗算
a / b    # 除算 (常に float)
a // b   # 整数除算 (floor)
a % b    # 剰余
a ** b   # べき乗 (右結合)
```

**評価** (`apply_binop`):
- `int + int` → `Int`、`float + float` → `Float`、`int + float` → `Float`
- `str + str` → `Str` (連結)
- `list + list` → 連結した新しいリスト
- 型が一致しない場合は `TypeError`

### 文字列乗算

```ar
"ab" * 3   # → "ababab"
[1] * 3    # → [1, 1, 1]
```

### 除算の注意

- `/` は常に `float` を返します
- `//` は `int // int` → `int`、それ以外 → `float`

---

## 比較演算子

```ar
a == b    # 等値 (values_eq)
a != b    # 非等値
a < b
a > b
a <= b
a >= b
a in b    # b が a を含む (リスト・辞書・文字列)
a not in b
```

**評価**: `Bool` を返します。  
`values_eq` は型を考慮して比較 (`Int(1) == Float(1.0)` は `true`)。

---

## 論理演算子

```ar
a and b   # 短絡評価: a が falsy なら a を返す、そうでなければ b を返す
a or b    # 短絡評価: a が truthy なら a を返す、そうでなければ b を返す
not a     # 論理否定: Bool を返す
```

**評価**: `is_truthy()` で真偽判定。Python と同じ短絡評価セマンティクス。

---

## ビット演算子

```ar
a & b    # AND
a | b    # OR
a ^ b    # XOR
~a       # NOT (単項)
a << n   # 左シフト
a >> n   # 右シフト
```

`int` 型にのみ適用可能。

---

## 型ガード (`is` / `is not`)

```ar
if x is int:
    # このブロック内で x は int として扱われる
    let val = x + 1

if y is not None:
    # y は None でないことが保証される
    y.do_something()
```

`Expr::IsType { expr, negated, type_name, span }` として AST に記録されます。  

**評価**: ランタイムで `type_name(val)` を確認して `Bool` を返します。  
**型検査**: `if` ブランチ内でオペランドの型を絞り込みます (型ガードナロイング)。

`is not` は `Union` / `Optional` 型の変数にのみ使用できます。  
非 Union 型に使うと静的型エラー `IsNotOnNonUnion` が発生します。

---

## キャスト演算子 (`=>`)

```ar
let v: MyType = raw_value => MyType
```

`Expr::Cast { object, type_name, span }` として AST に記録されます。

**評価**:
1. `type_name` が `new_type` で宣言された型 → コンストラクタ呼び出し
2. オブジェクトが `__cast__[type_name]` テンプレートメソッドを持つ → 呼び出し
3. それ以外 → `TypeError`

---

## 関数呼び出し

```ar
func(a, b, c)           # 位置引数
func(x=1, y=2)          # キーワード引数
func(a, keyword=b)      # 混在
template_fn[T](args)    # テンプレート型引数付き呼び出し
```

`Expr::Call { func, args, span }` として記録されます。

**引数の種類**:
- `CallArg::Positional(expr)` — 位置引数
- `CallArg::Keyword { name, value }` — キーワード引数

**評価** (`eval_call_args` → `bind_args`):
1. `func` 式を評価して呼び出し対象を決定
2. 各引数を評価
3. 引数を仮引数にバインド (デフォルト値の評価・自動キャスト含む)
4. 適切な実行ルーティン (`exec_fn_evaled` / `eval_method_call` 等) に委譲

---

## 属性アクセス

```ar
obj.name
self.field
module.function
```

`Expr::Attr { object, attr, span }` として記録されます。

**評価**: オブジェクトを評価し、値の種類に応じてフィールドまたはメソッドを返します:
- `Instance` → `fields` マップを検索、なければ `class.methods` を検索
- `Class` → `class_vars` (const) を検索
- `Namespace` → `members` マップを検索
- `Str`/`List`/`Dict`/`Set`/... → 組み込みメソッドのバインド済み関数を返す

**アクセス制御**: `current_class` と `field_access`/`method_access` を照合して  
`private`/`protected` 違反があれば `AccessError` を送出します。

---

## トレイト修飾アクセス (`::`)

```ar
obj::Trait.method(args)
```

`Expr::TraitAccess { object, trait_name, attr }` として記録されます。  
特定のトレイト実装のメソッドを明示的に呼び出します。

---

## 添字アクセスとスライス

```ar
items[0]          # Expr::Subscript
items[-1]         # 負インデックス (末尾から)
items[1:3]        # Expr::Slice (begin=1, end=3, step=None)
items[::2]        # Expr::Slice (step=2)
items[1:10:2]     # Expr::Slice (begin=1, end=10, step=2)
d["key"]          # 辞書アクセス
```

**評価**:
- `List`/`FrozenList` → インデックス (Python 互換負インデックス)
- `Str` → 1 文字の `Str` を返す
- `Dict` → キー検索
- スライス → 要素のリストを返す (ステップ対応)

---

## テンプレート型引数適用

```ar
Container[int]       # Expr::TemplateInstantiate
Template[int](args)  # Call の func に TemplateInstantiate を使用
```

`Expr::TemplateInstantiate { base, type_args }` は単独では評価できません。  
必ず `Call` の `func` として使用します。

---

## ブロック式

```ar
let result = block ->int:
    mut x = 0
    for i in range(10):
        x += i
    block_return x
```

`Expr::Block { stmts, return_type }` として記録されます。  
`block_return val` でブロック式を即座に終了して値を返します。  
`block_return` なしの場合は `None` を返します。

---

## if 式

```ar
let label = if score >= 90 ->str:
    block_return "A"
elif score >= 80:
    block_return "B"
else:
    block_return "C"
```

`Expr::IfExpr { branches, else_body, return_type }` として記録されます。

---

## for 式

```ar
let squares = for i in range(5) ->list[int]:
    loop_yield i * i
```

`Expr::ForExpr { target, iter, body, return_type }` として記録されます。  
`loop_yield val` で値をリストに蓄積します。  
`block_return val` で即座に単一値を返します。

---

## while 式

```ar
let found = while condition ->str:
    let item = next()
    if item.matches(target):
        block_return item
```

`Expr::WhileExpr { cond, body, return_type }` として記録されます。

---

## match 式

```ar
let msg = match code ->str:
    case 200:
        block_return "OK"
    case 404:
        block_return "Not Found"
    case _:
        block_return "Unknown"
```

`Expr::MatchExpr { subject, arms, return_type }` として記録されます。
