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

**float の表示**（`print` / `str` / `repr`）は CPython の `repr` と同じ最短の往復表現です。10 進の指数が
`-4` 未満か `16` 以上なら指数表記（`1e+20` / `1e-05`）、それ以外は小数表記に `.0` を足します（`3.0`）。
complex も CPython と同じ（`(1+2j)` / `3j`）。

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
f"{name!r:>10}|{x=}|{x:{width}.{prec}f}"
```

置換フィールドは Python と同じ `{式[=][!変換][:書式指定]}` です（python_builtins_plan.md のタスク 3-4）。

| 形 | 意味 |
|---|---|
| `{x}` | `format(x)`（`__format__` の無い値は `str(x)` と同じ） |
| `{x:spec}` | `format(x, "spec")`（書式指定の小言語・[12_builtins.md](12_builtins.md) の「`format`」） |
| `{x!r}` / `{x!s}` / `{x!a}` | `repr(x)` / `str(x)` / `ascii(x)`。書式指定があればその後で `format` |
| `{x=}` / `{x = :.2f}` | 式の文字（と `=` 前後の空白）を前に付ける。書式指定・変換が無ければ `repr`（CPython の自己記述の式） |
| `{x:{w}.{p}f}` | 書式指定の中の置換フィールド（f-string と同じに文字列にして連結する） |
| `{{` / `}}` | `{` / `}` |

- パーサーが文字列の連結（`"x = " + format(x) + ..`）に展開する（`Expr::FStr` は無い）。
- 括弧と文字列の中の `!` / `:` / `=` は式の一部（`{d["a:b"]}` / `{xs[1:2]}` / `{a != b}`）。
- ⚠ 式の後ろに読めない字句が残ると構文の誤り（`f-string: invalid expression`）。以前は `:` / `!` から後ろを黙って捨てていた。

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

**メソッド**（CPython と同じ・python_builtins_plan.md のタスク 3-6）:

| メソッド | 働き | 静的な型 |
|---|---|---|
| `keys()` / `values()` / `items()` | キー・値・`(キー, 値)` の組のリスト（その時点の写し） | `list[K]` / `list[V]` / `list[tuple[K, V]]` |
| `get(k, default=None)` | 値。無ければ `default` | `Option[V]`（既定値が `V` なら `V`） |
| `pop(k[, default])` | 取り除いて値。無ければ `default`、それも無ければ `KeyError` | `V` |
| `setdefault(k, default=None)` | あれば値、無ければ `default` を入れて（入れた後の）値 | `Option[V]` / `V` |
| `update(other=(), **kw)` | 辞書か `(キー, 値)` の組の列から、続けてキーワード引数から入れる | `None` |
| `popitem()` | 最後に入れた `(キー, 値)` を取り除いて返す（空なら `KeyError`） | `tuple[K, V]` |
| `clear()` / `copy()` | 空にする / 別の辞書 | `None` / 同じ辞書型 |

- 書き換えるメソッド（`pop` / `popitem` / `setdefault` / `update` / `clear`）は `let` の辞書には呼べません（静的な誤り）。
- 型つきの辞書（`dict[str, int]`）へ入れる `update` / `setdefault` は `d[k] = v` と同じく型を確かめます。
- ⚠ `copy()` は深い写しです（`mut e = d` と同じ。CPython は浅い写し）。⚠ `del d[k]` はありません（`d.pop(k)` を使う）。

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
- `//` と `%` は **CPython と同じ**: 商は負の無限大の方向へ切り捨て、`%` の結果は割る数と同じ符号
  （`-7 // 2 == -4`・`7 % -2 == -1`・`-7 // -2 == 3`）。float も CPython の `_float_div_mod` と同じ式
  （`0.1 % 0.01` は `3.469446951953614e-18`、`6.0 % -3.0` は `-0.0`）。0 で割ると `ZeroDivisionError`
  （`float floor division by zero` / `float modulo`）

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

list 同士・tuple 同士の大小比較（`<` / `<=` / `>` / `>=`）は CPython と同じ辞書順です（最初に違う要素だけを
比べ、違いが無ければ長さで決める）。list と tuple は比べられません（`TypeError`）。比べられない要素に届いた
ときだけ `TypeError`（`(1, 'a') < (2, 3)` は `True`）。

set / frozenset の大小比較は**包含関係**です（CPython と同じ）: `a <= b` は部分集合、`a < b` は真部分集合、
`>=` / `>` はその逆。set と frozenset は組んで比べられ、`frozenset({1}) == {1}` も `True` です
（python_builtins_plan.md のタスク 3-1）。

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

**評価**: オブジェクトを評価し、値の種類に応じて次の順に探します（`eval/attrs.rs` の `get_attr_val`）:
- `Instance`（CPython の `object.__getattribute__` と同じ順）:
  1. クラス側（`static mut` / `const`）の**データディスクリプタ**（`__get__` と `__set__` / `__delete__` を持つ値）→ `__get__(obj, C)`
  2. フィールド
  3. クラス側の**非データディスクリプタ**（`__get__` だけ）→ `__get__(obj, C)`
  4. クラス側の値（`const` / `static mut`）そのもの
  5. メソッド（**受け手を束縛した関数**・[06_classes_traits.md](06_classes_traits.md) の「メソッドを値として読む」）・`gen` メソッド
- `Class`:
  1. `__name__` / `__qualname__`（クラスの名前）
  2. `const` / `static mut`（ディスクリプタなら `__get__(None, C)`。`property` は自分自身を返す）
  3. メソッド（束縛しない関数）
  4. `name`（クラスの名前。⚠ 同じ名前のメンバーがあればメンバーが先）
- `Namespace` → `members` マップを検索
- `Str`/`List`/`Dict`/`Set`/... → 組み込みメソッドのバインド済み関数を返す

ディスクリプタは [06_classes_traits.md](06_classes_traits.md) の「ディスクリプタ」、組み込みの `property` は同じファイルの
「組み込みの `property`」を参照。

**代入** `obj.attr = v`（`attr_assign_evaled`）: クラス側のデータディスクリプタなら `__set__(obj, v)`、`static mut` なら
共有セルを書き換え、それ以外はフィールドへ格納します（`const` への代入は `TypeError`）。`C.attr = v` は `static mut` の
値を置き換えます（ディスクリプタの `__set__` は呼びません）。

**呼び出し** `obj.m(args)`: メソッドを探し、無ければ属性として読んでその値を呼びます（フィールドに入れた関数・
`__get__` が返す関数。[06_classes_traits.md](06_classes_traits.md) の「メソッドの検索順序」）。

**アクセス制御**: `current_class` と `field_access`/`method_access` を照合して  
`private`/`protected` 違反があれば `AccessError` を送出します。ディスクリプタを通る読み書きも同じです。

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

**文字列の単位**: 文字列の添字・スライス・反復・`len(s)`・`find` / `rfind` / `index` / `rindex` / `count` の位置はどれも**文字**単位です
（負の位置は末尾から数える・Python と同じ）。UTF-8 のバイト数が要るときは `len(s.encode("utf-8"))`
（`implementation_plans/ffi_value_exchange_plan.md` 7-2。以前は `len` と `find` などがバイト単位で、添字と食い違っていました）。

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

`block:` を**文**として書いた場合（値を受けない）、中の `return` は普通の文と同じく関数から抜けます
（囲む `try` の `finally` も走る）。⚠ ブロック**式**の中の `return`（式の途中から関数を抜ける形）は作れません。
⚠ 2026-10-05 までは、関数の中の `block:` 文に `return` があるだけで関数ごと `VmForceError` でした
（Python の `with ...: return ..` の変換先も `block:` なので同じ理由で読めなかった）。

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
