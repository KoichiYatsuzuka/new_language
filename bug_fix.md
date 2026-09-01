# bug_fix.md — Arrow 本体のバグ（起票）

- 作成: 2026-08-28
- 発見経緯: `python_translation` ブランチでの **Python→Arrow 変換器の実装作業中**に、
  変換器の検証のつもりで書いたコードが落ちて見つかったもの。
  ⚠ **すべて「純 Arrow の `.ar` でも再現する」ことを確認済み**。変換器の問題ではない。
- 検証: 2026-08-28 時点の `python_translation` HEAD（`d3744bb`）をリリースビルドして実測。
  本書の「再現」はすべてそのまま `.ar` に貼って走る最小形。

---

## 0. なぜ見つかっていなかったか

⚠ **どれも例題が 1 本も書いていない形**だった。既存のゲート
（`scan_examples` / `compare_outputs` / `compare_python_impl`）は
**例題が踏まない挙動を映さない**（[FUTURE_FEATURE.md](implementation_logs/FUTURE_FEATURE.md) §5(a)
の `NESTED-GAP` と同じ構図）。

⇒ 修正するときは**必ず例題も足す**こと。直しただけでは次も同じように壊れる。

---

## 1. 優先度

**「黙って違う答えが出る」ものを先に**。実行が止まるものは気付けるので後回しでよい。

| # | 症状 | 失敗の形 | 原因の判明度 |
|---|---|---|---|
| **B1** | タプルを dict のキーにすると**黙って消える** | 🔴 サイレント | ✅ 特定済み |
| **B2** | `list` / `dict` の `==` が**常に false** | 🔴 サイレント | ✅ 特定済み |
| **B3** | `mut` パラメータが入れ子 `fn` に **None として捕捉される** | 🔴 サイレント | 🟡 見当あり |
| **B4** | `for` のループ変数のスコープが**文脈で 3 通り**違う | 🟠 一部サイレント | ⬜ 未調査 |
| **B5** | `list + list` / `list * int` が未対応 | 🟢 明示エラー | ✅ 特定済み |
| **B6** | モジュール本体から**自モジュールの関数を呼べない** | 🟢 明示エラー | ⬜ 未調査 |
| **B7** | 入れ子 `fn` から `local::args` を参照すると VM 非適格 | 🟢 明示エラー | ✅ 特定済み |

---

## B1. タプルを dict のキーにすると黙って消える 🔴

### 再現

```
mut d = {(1, 2): "x"}
print(d)        # {}   ← 空になる
print(len(d))   # 0

mut e = {}
e[(1, 2)] = "y"
print(len(e))   # 0    ← 代入も入らない
```

`str` / `int` / `bool` / `None` のキーは正常（4 件入ることを確認済み）。

### 原因（特定済み）

[`src/interpreter/value/collections.rs`](src/interpreter/value/collections.rs):

- `DictKey` 列挙が **`Int` / `Str` / `Bool` / `None` の 4 種しか無い**。
- `DictData::set` は `DictKey::from_value(&key)` が `None` を返すと**何もしない**:

```rust
pub fn set(&mut self, key: Value, value: Value) {
    if let Some(k) = DictKey::from_value(&key) {
        self.map.insert(k, value);
    }
    // unhashable key (e.g. instance) silently ignored — same as before
}
```

⚠ コメントは「unhashable なキーは黙って無視」と**意図的な設計として書かれている**が、
**タプルは Python では hashable**（辞書のキーとして最も普通に使われる形の一つ）。

### 影響

- Python の `d[(x, y)] = v`（座標をキーにする等）が**黙って何も起きない**。
- **辞書内包の前提**でもある（[FUTURE_FEATURE.md](implementation_logs/FUTURE_FEATURE.md) §4(4-a)）。
  「ペアのリストから dict を作る」経路はここを通るので、**B1 を先に直す**こと。

### 修正方針

1. `DictKey` に `Tuple(Vec<DictKey>)` を足し、`from_value` で再帰的に変換する
   （要素がすべて hashable なら `Some`）。
2. **本当に unhashable なキー（`Instance` / `List` / `Dict` 等）は黙って捨てず `TypeError`**
   にする。Python も `TypeError: unhashable type: 'list'` を出す。
   ⚠ ここを黙って捨て続けるかぎり、同じ形のバグがまた出る。

### 留意点

- ⚠ `DictKey` は `IndexMap` のキーなので `Hash` + `Eq` が要る。`Value` をそのまま入れられない
  理由（`f64` など）がこの設計の出発点。**タプルだけを足すのは筋が良い**が、
  「どの型を hashable とするか」を**仕様として決めてから**足すこと。
- ⚠ `all_keys()` がキーを `Value` へ戻す経路もあるので、そこも同時に対応する。

---

## B2. `list` / `dict` の `==` が常に false 🔴

### 再現

```
print([1, 2] == [1, 2])     # False   ← Python は True
print({"a": 1} == {"a": 1}) # False   ← Python は True

mut a = [1, 2]
print(a == a)               # False   ← **同一オブジェクトでも false**
print(a != a)               # True

print((1, 2) == (1, 2))     # True    ← タプルは正しい
print({1, 2} == {1, 2})     # True    ← セットも正しい
print("ab" == "ab")         # True
```

### 原因（特定済み）

[`src/interpreter/ops/equality.rs`](src/interpreter/ops/equality.rs) の `values_eq` に
**`Value::List` と `Value::Dict` のアームが無い**（`Tuple` と `Set` にはある）。
末尾の `_ => false` に落ちるので**常に false**。

### 影響

- `if xs == ys:` が常に偽になる。**エラーにならないので気付けない**。
- ⚠ `a == a` すら false なので、「参照比較になっている」のではなく**純粋な抜け**。

### 修正方針

`Tuple` / `Set` のアームと同じ形で `List` / `Dict` を足す:

- `List`: 長さが同じ ＋ 各要素を `values_eq` で再帰比較。
- `Dict`: 要素数が同じ ＋ 各キーの値を `values_eq` で再帰比較。

### 留意点

- ⚠ **`values_ref_eq`（`===`）は別物**。あちらは `Rc::ptr_eq` で正しく実装されている
  （`equality.rs:91`）。混ぜないこと。
- ⚠ 再帰なので**循環参照**でスタックが溢れうる。既存の `Tuple` / `Set` / `Instance` も同じ形なので、
  ここだけ気にするか全体で決めるかを判断する。
- ⚠ `!=` は `==` の否定として実装されているらしい（`a != a` が true になった）。連動を確認すること。

---

## B3. `mut` パラメータが入れ子 `fn` に None として捕捉される 🔴

### 再現

```
fn make_adder(mut n: int) -> function:
    fn add(mut x: int) -> int:
        return x + n
    return add

mut k = 5
let a = make_adder(k)
mut z = 3
print(a(z))
# TypeError: unsupported operand types for `Add`: int and NoneType
```

**`let` パラメータなら正しく動く**（対照）:

```
fn make_adder(let n: int) -> function:
    fn add(let x: int) -> int:
        return x + n
    return add
print(make_adder(5)(3))   # 8
```

### 影響

- **クロージャで包む形の関数がまるごと壊れる**。
- ⚠ Python→Arrow 変換器は**全パラメータを `mutable: true` にする**（Python に不変引数が無いため）ので、
  **Python で最も普通の「クロージャで包むデコレータ」が動かない**
  （[python_converter_coverage.md](python_converter_coverage.md) 項目 20 参照）。

### 原因（見当）

「同じ自由変数を 2 箇所が別々に解決している」形に見える。片方が古い値（None）を掴む。

- ツリーウォーク側: `capture_env`（[`src/interpreter/exec/blocks.rs`](src/interpreter/exec/blocks.rs)）
  の可変パス。`self.scopes[scope_idx]` から値を取って `Var::Cell` へ昇格する。
- VM 側: `nested_fn_captures`（[`src/vm/compiler/calls.rs`](src/vm/compiler/calls.rs)）。
  ⚠ ここには「**可変ローカルのキャプチャ。セル化は `nested_fn_free_names` の事前解析が担うので、
  ここへ来るのは解析漏れ（保守的に諦める）**」というコメントと `return None` がある。

⇒ **パラメータが「事前解析の漏れ」になっている**疑いが濃い。
実測の症状が `VmForceError` ではなく **None** なので、
**スコープ側に残っている古いエントリ（None）を掴んでいる**＝
skill `language-dev-principles` のいう「4 つの storage kind」の取り違えの形。

### 留意点

- ⚠⚠ `calls.rs` のコメントが明言しているとおり、**`capture_env` と `nested_fn_captures` は
  「自由変数の定義」を共有している**。**片方だけ直すと閉包変数が黙って消える**。必ず両方見ること。
- ⚠ 直したら例題を足すこと。現状**入れ子 `fn` の中で書かれていない構文が 10 件ある**
  （[FUTURE_FEATURE.md](implementation_logs/FUTURE_FEATURE.md) §5(a)）ので、この系統は今後も出る。

---

## B4. `for` のループ変数のスコープが文脈で 3 通り違う 🟠

### 再現

```
# (a) モジュール直下・外側に同名の束縛あり → ループが**隠す**だけで外側は変わらない
mut i = -1
for i in [1, 2, 3]:
    print(i)          # 1 2 3
print(i)              # -1        ← Python は 3

# (b) モジュール直下・外側に束縛なし → ループ後は**見えない**
for i in [1, 2, 3]:
    print(i)          # 1 2 3
print(i)              # NameError: 'i' is not defined   ← Python は 3

# (c) 関数の中・外側に束縛なし → ループ後も**見える**
fn f() -> int:
    for i in [1, 2, 3]:
        print(i)
    return i          # 3         ← Python と一致
print(f())
```

### 影響

- (a) が**サイレントに違う答え**を出す。(b) と (c) は同じ書き方なのに**片方だけ落ちる**。
- Python→Arrow 変換器の項目 2（再代入の巻き上げ）の残差もこれ
  （[python_converter_coverage.md](python_converter_coverage.md) 項目 2 の「残る意味差」）。
  変換器側では埋められない。

### 修正方針（要判断）

**まず「Arrow の `for` のループ変数はループ後も見えるのか」を仕様として決める**こと。
3 通りに割れているのは実装の偶然で、仕様が決まっていないことの現れ。

- Python に寄せる（ループ後も見える・外側の同名変数を書き換える）なら、
  `for` は**新しいスコープを作らず現スコープに束縛**する形になる。
- Arrow 独自に「ループ変数はループ内だけ」とするなら、**(c) を (b) に揃える**。

### 留意点

- ⚠ どちらに倒しても**既存の例題の挙動が変わりうる**。`compare_outputs.ps1` で差分を取ってから決めること。
- ⚠ 内包表記（`ast::build_list_comprehension`）は `for` 式 + `loop_yield` に脱糖するので、
  ここを触ると内包表記にも波及する。

---

## B5. `list + list` / `list * int` が未対応 🟢

### 再現

```
print([1, 2] + [3])   # TypeError: unsupported operand types for `Add`: list and list
print([1] * 2)        # TypeError: unsupported operand types for `Mul`: list and int
print((1, 2) + (3,))  # TypeError: unsupported operand types for `Add`: tuple and tuple

print("ab" * 2)       # abab   ← str の繰り返しは動く
```

`list.extend` も無い（`AttributeError: 'list' object has no method 'extend'`）ので、
**リストの連結手段が `append` のループしかない**。

### 原因（特定済み）

[`src/interpreter/ops/operators.rs`](src/interpreter/ops/operators.rs) の `Add` / `Mul` に
`Int` / `Float` / `Str` / `UInt` / `Complex` のアームはあるが、
**`List` / `Tuple` のアームが無い**。

### 影響

- Python コードの `xs + ys` / `xs += [v]` / `[0] * n` が**軒並み落ちる**。
  明示エラーなので気付けるが、**実在の Python モジュールを読むときの当たり所が多い**。

### 修正方針

`operators.rs` に以下を足す:

- `(Add, List, List)` → 連結した新しいリスト
- `(Add, Tuple, Tuple)` → 連結した新しいタプル
- `(Mul, List, Int)` / `(Mul, Int, List)` → 繰り返し（`Str` の既存実装と同じ形）
- `(Mul, Tuple, Int)` / `(Mul, Int, Tuple)` → 同上

### 留意点

- ⚠ **新しいリストを作る**こと（左辺を破壊しない）。`xs += ys` は複合代入で
  `xs = xs + ys` に落ちるので、破壊的にすると別名に波及する。
- ⚠ `Mul` の負数・0 は Python では空リスト。合わせること。

---

## B6. モジュール本体から自モジュールの関数を呼べない 🟢

### 再現

`lib.ar`:

```
fn hello(let name: str) -> str:
    return "hi " + name

let G = hello          # ← **参照は通る**
let MSG = hello("bob") # ← **呼び出しが落ちる**
```

`main.ar`:

```
import[ar] lib as L
print(L.MSG)
# NameError: 'hello' is not defined
```

⚠ **参照（`G = hello`）は通り、呼び出し（`hello("bob")`）だけが落ちる**のがこのバグの形。

### 影響

- **モジュール本体の初期化コードが全滅**する（テーブルを組む・定数を計算する等）。
- Python→Arrow でも同じ（`MSG = hello("bob")` を含む `.py` が読めない）。
- 変換器の項目 27（Python モジュール内 import の再帰ロード）の前提にも関わる。

### 調査の入口

- 参照と呼び出しで解決経路が違う。VM の呼び出し命令が
  **モジュール本体を実行しているスコープを見ていない**疑い。
- モジュール本体の実行は `exec_module`（[`src/interpreter/exec/modules.rs`](src/interpreter/exec/modules.rs)）。

---

## B7. 入れ子 `fn` から `local::args` を参照すると VM 非適格 🟢

### 再現

```
fn outer(let ...: int) -> int:
    fn inner() -> int:
        return len(local::args)
    return inner()
print(outer(... = 1, 2, 3))
# VmForceError: cannot compile function 'inner' to bytecode
```

### 原因（特定済み）

[`src/vm/compiler/expr.rs`](src/vm/compiler/expr.rs) の `Expr::LocalVar` アームは
**自分の `slots` に `local::args` が無ければ `bail_expr("localvar-unbound")`** する。
入れ子 `fn` は可変長パラメータを持たないので slot が無く、
外側からのキャプチャ対象にもなっていない（`nested_fn_captures` は
`collect_referenced_names` の結果を `self.slots` で引くが、`local::args` は拾われない）。

### 影響

- `def outer(*xs): def inner(): return len(xs)` の形が動かない。
- Python→Arrow の項目 6（`*args`）で実際に踏んだ
  （[python_converter_coverage.md](python_converter_coverage.md) 項目 7 参照）。

### 修正方針

`nested_fn_captures` / `nested_fn_free_names` が **`local::args` もキャプチャ対象として扱う**ようにする
（名前が `local::` 接頭辞つきである点に注意）。

### 留意点

- ⚠ B3 と**同じ関数**を触る。合わせて直すなら一緒に、別々にやるなら
  「`capture_env` と `nested_fn_captures` の自由変数の定義を揃える」ことを両方で守ること。

---

## 2. 着手の順序（提案）

1. **B2**（`list` / `dict` の `==`）— 原因が特定済みで**修正が最も小さく**、サイレントな誤答を消せる。
2. **B5**（`list + list` 等）— 同じく `operators.rs` の追加だけ。B2 と一緒に片付くと実用度が跳ねる。
3. **B1**（タプルの dict キー）— サイレントな**データ消失**。辞書内包の前提でもある。
   「どの型を hashable とするか」の仕様決めが要る。
4. **B3 + B7**（入れ子 `fn` のキャプチャ）— 同じ関数群。B3 はサイレントなので優先度が高いが、
   **`capture_env` と `nested_fn_captures` の両方**を見る必要があり調査量が読めない。
5. **B6**（モジュール本体の自己呼び出し）— 影響は大きいが原因未調査。
6. **B4**（`for` のスコープ）— **仕様判断が先**。実装より先に決めることがある。

## 3. 共通の留意点

- ⚠⚠ **どれも例題が 1 本も無かった**ために全ゲートを素通りしていた。
  直したら**必ず `examples/` に追加**すること（`.claude/rules/regulations.md`）。
- ⚠ 解釈側（`eval_*` / `exec_*` / `ops/`）を触るので、「挙動不変」を主張するときは
  `compare_bytecode.ps1` ではなく **`compare_outputs.ps1 -A <直前のコミットのビルド>`** を使う。
  ⚠ **使う前に同一 exe 同士で負の対照**を取る。
- ⚠ `impl_python/` は**触らない**方針（古いため）。差分が出たら
  `compare_python_impl.ps1` の `$knownDiff` に**実測した理由**をつけて登録する。
