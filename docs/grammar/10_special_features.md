# 特殊機能

---

## block_return

`block:` / `if` / `match` / `for` / `while` の各式から値を返します。

```hv
let result = block ->int:
    mut x = 0
    for i in range(5):
        x += i
    block_return x   # 10 を返してブロックを終了

let label = if score >= 90 ->str:
    block_return "A"
elif score >= 80:
    block_return "B"
else:
    block_return "C"
```

`Stmt::BlockReturn(expr, span)`

**実行**:
1. `expr` を評価して値 `v` を取得
2. スレッドローカル `BLOCK_RETURN_EXPECTED_TYPE` で型アノテーションと照合 (型注釈あり時)
3. `ExecResult::BlockReturn(v)` を返してブロック式の実行を終了

`block_return` は `for`/`while` 式の**直下ボディ**では使えません。  
その場合は静的型エラー `BlockReturnInLoopExpr`。内側に `block:` / `if` を挟んでください。

---

## loop_yield

`for`/`while` 式の中で値を蓄積してリストを構築します。

```hv
let squares = for i in range(10) ->list[int]:
    loop_yield i * i

let filtered = for x in data ->list[float]:
    if x > 0.0:
        loop_yield x
```

`Stmt::LoopYield(expr)`

**実行**:
1. スレッドローカル `BLOCK_YIELDS` に値を追加
2. `ExecResult::BlockYield(v)` を返して実行を**継続** (ループは続く)
3. `for`/`while` 式のループが終了したとき、`BLOCK_YIELDS` に蓄積されたリストを返す

`loop_yield` が `for`/`while` 式の外で使われた場合は `RuntimeError`。

---

## yield (ジェネレータ)

ジェネレータ関数 (`gen`) の中で値を産出します。

```hv
gen fibonacci() -> int:
    mut a, mut b = 0, 1
    while True:
        yield a
        let temp = a
        a = b
        b = temp + b
```

`Stmt::Yield(expr)`

**実行**:
1. スレッドローカル `GENERATOR_YIELDS` に値を追加
2. `ExecResult::Normal` を返してジェネレータ本体の実行を**継続**
3. ジェネレータ本体の実行が終了したとき、全 yield 値を `GeneratorState` に格納

現在の実装では本物の遅延評価ではなく、呼び出し時に**全値を一括収集**します。

---

## 非同期タスク (`<-`)

`AsyncManager` インスタンスにタスクを送信します。

```hv
let mng = AsyncManager(num_thread=4)

mng <- async ->int:
    let result = heavy_computation()
    block_return result

mng <- async ->str:
    let data = fetch_data()
    block_return data
```

`Stmt::AsyncAssign { target, return_type, stmts }`

**パース**: `ident '<-' 'async' ['->' type_expr] ':' block`

**実行** (`exec_async_assign`):
1. 現在のスコープ変数をキャプチャ:
   - `mut` 変数 → `Rc<RefCell<Value>>` セルで共有 (スレッド境界は `SendableEnv` でラップ)
   - `let` 変数 → ディープクローン
2. `std::thread::spawn` で新しいスレッドを起動
3. スレッド内で Arrow インタープリタの別インスタンスを生成してタスクを実行

**結果の収集**:

```hv
mng.poll_completed()   # 完了したタスクを収集 (非ブロッキング)
mng.wait_all()         # 全タスクの完了を待つ (ブロッキング)
let results = mng.results
```

**注意**: `PyObject` をキャプチャすると Python の GIL により並列化されません。

---

## デバッガ (`break_point`)

実行を一時停止してデバッグ REPL を起動します。

```hv
mut x = compute()
break_point   # ここで一時停止
process(x)
```

`Stmt::BreakPoint { span }`

**REPL コマンド**:

| コマンド | 動作 |
|---|---|
| Enter (空行) | ステップオーバー (同深さの次の文へ) |
| `e` | ステップイン (関数の中へ) |
| `o` | ステップアウト (現在の関数から抜けるまで実行) |
| `q` | 実行再開 (ブレークポイントを解除) |
| 式/文 | REPL 内でコードを実行 |
| `let dbg::name = expr` | デバッガ用一時変数を宣言 |

---

## `Expr::DebugVar` と `Stmt::DebugLet`

デバッガ REPL 内での専用変数:

```hv
# REPL 内で:
let dbg::temp = x * 2   # デバッガ一時変数を宣言
dbg::temp               # 参照
```

`Stmt::DebugLet(name, expr)` — デバッガ変数を `Interpreter.dbg_vars` に格納  
`Expr::DebugVar(name)` — `dbg_vars` から値を取得  
再開 (`q`) 時に全デバッガ変数はクリアされます。

---

## 数学文字列

LaTeX 風の記法で数学記号を Unicode として扱えます。

```hv
let phi = m"\phi"         # "φ"
let eq  = m"E = mc^2"     # "E = mc²"
let vec = m"\vec{v}_{n}"  # "v⃗ₙ"
let sum = $\sum_{i=0}^{n}$  # "∑ᵢ₌₀ⁿ" (短縮記法)
```

`m"..."` / `$...$` はレキサー段階で変換されます。  
LaTeX コマンド→ Unicode 変換は `src/lexer/math.rs` の `render_math_str` が担当。

---

## f-string 式補間

```hv
let name = "world"
let n = 42
print(f"Hello, {name}! The answer is {n}.")
print(f"Pi ≈ {3.14159:.4f}")   # 書式指定
```

**書式指定子** (`: format_spec`):

| 指定子 | 例 | 説明 |
|---|---|---|
| `.Nf` | `{x:.2f}` | 小数点 N 桁の浮動小数点 |
| `Nd` | `{n:5d}` | 幅 N の整数 (右揃え) |
| `<N` | `{s:<10}` | 幅 N の左揃え |
| `>N` | `{s:>10}` | 幅 N の右揃え |
| `0N` | `{n:05}` | 0 パディング |

---

## `@` 演算子 (行列積)

```hv
let c = a @ b   # 行列積 (NumPy 互換)
a @= b          # インプレース行列積
```

クラスに `__matmul__(self, let other)` メソッドを実装すると  
`@` 演算子が使えます。

---

## `:=` 演算子 (セイウチ演算子)

将来的なサポートのために予約されていますが現在未実装。

---

## `::` 演算子 (トレイト修飾アクセス)

```hv
obj::TraitName.method(args)
```

複数の trait を実装していてメソッド名が衝突する場合に使います。

---

## `assert` 文

```hv
assert x > 0
assert len(items) > 0, "items must not be empty"
```

条件が `False` のとき `AssertionError` を送出します。
