# bug_fix.md — Arrow 本体のバグ（起票）

- 作成: 2026-08-28
- 発見経緯: `python_translation` ブランチでの **Python→Arrow 変換器の実装作業中**に、
  変換器の検証のつもりで書いたコードが落ちて見つかったもの。
  ⚠ **すべて「純 Arrow の `.ar` でも再現する」ことを確認済み**。変換器の問題ではない。
- 検証: 2026-08-28 時点の `python_translation` HEAD（`d3744bb`）をリリースビルドして実測。
  本書の「再現」はすべてそのまま `.ar` に貼って走る最小形。

## 現況（2026-09-02）

**B1 / B2 は修正済み**。両者は結果として**言語仕様の変更**になったので、症状の記述は
削除し、各節に**決まった仕様**を残してある。⚠ 実装の詳細ではなく**規則**として読むこと。
B3〜B7 は未着手（起票のまま）。

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
| ~~**B1**~~ | ~~タプルを dict のキーにすると**黙って消える**~~ | 🔴 サイレント | ✅ **修正済み**（仕様変更あり） |
| ~~**B2**~~ | ~~`list` / `dict` の `==` が**常に false**~~ | 🔴 サイレント | ✅ **修正済み**（仕様変更あり） |
| **B3** | `mut` パラメータが入れ子 `fn` に **None として捕捉される** | 🔴 サイレント | 🟡 見当あり |
| **B4** | `for` のループ変数のスコープが**文脈で 3 通り**違う | 🟠 一部サイレント | ⬜ 未調査 |
| **B5** | `list + list` / `list * int` が未対応 | 🟢 明示エラー | ✅ 特定済み |
| **B6** | モジュール本体から**自モジュールの関数を呼べない** | 🟢 明示エラー | ⬜ 未調査 |
| **B7** | 入れ子 `fn` から `local::args` を参照すると VM 非適格 | 🟢 明示エラー | ✅ 特定済み |

---

## B1. 辞書のキー ✅ 修正済み（`481de14` / `66771a0` / `5bdfd60`）

症状（タプル等をキーにすると黙って消える）は解消。**辞書のキーは仕様として作り直した**ので、
以下は「バグ修正」ではなく**言語仕様の変更**として扱うこと。

### 使えるキー

**ほぼすべての型**が使える。実測で確認済み:
`tuple` / `list` / `set` / インスタンス / 関数 / クラス / `type` /
`uint` / 非整数 `float` / `complex` / `str` / `int` / `bool`。

### 使えないキー（**仕様として禁止**・必ず `TypeError`）

| 禁止 | 理由 |
|---|---|
| `None` / `Undefined` | 「存在しないことを示す値」はキーにしない。⚠ **Python では `None` は hashable** だが Arrow は禁止する |
| `NaN`（**入れ子でも**） | 自分自身と等値にならないので、入れても二度と引けない |
| `__eq__` を定義して `__hash__` を定義しないクラスのインスタンス | 等値の規則とハッシュの規則が食い違い、`__eq__` が「等しい」と言う 2 つが別バケットに落ちて**黙って別キーとして入る**。Python は `__hash__ = None` で unhashable にして防ぐが、Arrow は `__eq__` を `==` やリストの `in` でも使うのでクラス定義自体は禁止せず、**キーにした時点で**弾く |
| 循環した値・極端に深い値 | この先の複製処理でスタックが溢れるため、深さ上限（64）で止める |

⚠ **読み取り・包含検査も同じエラー**にしてある（`KeyError` や `False` ではない）。
「書けないのに読むと『入っていない』と言われる」形は誤診を招くため。

### キーの同一性

- **型厳密**。`d[3]` と `d[3.0]` は**別キー**（旧実装の float → int 正規化は撤去した）。
  ⚠ 式としての `3 == 3.0` は True（B2 の昇格ラティス）だが、**キーの照合はそれとは別の規則**。
  ここが厳密でないとハッシュと食い違う。
- `1` と `True` も別キー（`bool` は数値の昇格ラティスに入らない）。
- **キーは挿入時に複製される**。参照を共有したままだと、あとで中身を書き換えられて
  ハッシュと食い違い、「入れたのに引けない」辞書になる。

### `__hash__` / `__eq__`

- **オーバーライドできる**（`d[k] = v` / `d[k]` / `in` のすべてで効く）。
- 既定は `__hash__` = **構造ハッシュ**（クラス名 + 全フィールドを再帰）、
  `__eq__` = **構造的等値**。何も定義しなくても値として自然に振る舞う。
- ⚠ **必ず対で定義すること**（片方だけは上表のとおりエラー）。

### 実装（要点だけ）

- コンテナは `IndexMap<DictKey, Value>` → **`IndexMap<HKey, Value>`**（`HKey { hash, key }`）。
  ⚠⚠ **Rust の `Hash` / `Eq` トレイト経由では `__hash__` / `__eq__` を呼べない**
  （トレイトのメソッドはインタプリタ文脈を受け取れない）。⇒ `indexmap` の `raw_entry_v1` API を
  使い、ハッシュ値は自分で渡し、等値判定は**クロージャ**で渡す。この API には
  `K: Hash + Eq` の境界が無いので `Cargo.toml` の変更も不要だった。
- 既定ハッシュは [`src/interpreter/ops/hash.rs`](src/interpreter/ops/hash.rs)。
  ⚠ **設計規則はただ 1 つ: `values_eq` と 1:1 に対応させること。**
  `equality.rs` に腕を足したら**必ずここにも足す**。破れると「入れたのに引けない」壊れ方をし、
  しかもエラーにならない。
- ⚠ Python の mod (2⁶¹−1) 環準同型は**不要**。あれは `1 == 1.0 == True` を同じスロットに
  落とすためだけの仕組みで、Arrow は B2 で値の同一性を型厳密にしたので判別子 + ビットで足りる。
- ハッシュシードは**プロセスごとにランダム**（HashDoS 対策）。`IndexMap` が挿入順を保つので
  **観測可能な出力は変わらない**＝ゴールデン系ゲートに影響しない。
- ⚠ **ハッシュは深さで打ち切ってよい**（衝突は正しさを壊さない）。対して `values_eq` は
  打ち切れない（`false` を返すとサイレントな誤答）。この非対称は意図的。
- ⚠ 複製経路（`deep_copy_value` / `Value::deep_clone`）は**ハッシュを取り直す**。
  `deep_clone` は関数等の `Rc` を作り直すのでポインタハッシュが変わり、保存済みハッシュを
  持っていくと複製後の辞書がそのキーで引けなくなる。

---

## B2. `list` / `dict` の `==` ✅ 修正済み（`481de14` / `c1af0b1`）

症状（常に false）は解消。あわせて**等値の規則そのものを二層に分けた**ので、
以下は言語仕様の変更として扱うこと。

### 等値は 2 つある

| | 役割 | 使う場所 |
|---|---|---|
| **値の同一性**（型厳密） | 型が違えば等値にならない | 辞書のキー・ハッシュ、コンテナ内部の再帰比較 |
| **式としての比較** | オペランドを**キャストしてから**比べる | `==` / `!=` / `in` / `not in` / set 演算 |

### 数値の昇格ラティス（式としての比較のみ）

```
uint ──> int ──> float          bool は独立（昇格しない）
```

| 式 | 結果 |
|---|---|
| `1 == 1.0` / `uint(3) == 3` | True |
| `1 == True` | **False**（⚠ Python では True）|
| `1 in [1.0]` | True（`in` も同じ規則）|
| `[1] == [1.0]` | **False** |

⚠ **昇格は比較のオペランドにだけ掛かり、コンテナの中までは再帰しない。**
「比較の場所でキャストする」規則の素直な帰結。

⚠ **静的注釈にはできない。** import モジュール本体には注釈が供給されない
（[resolver.rs](src/interpreter/resolver.rs) の doc・意図的）ので、注釈に依存させると
**モジュール本体だけ `1 == 1.0` が False** になる。⇒ 昇格は実行時の値の型で行う。

⚠ 旧実装は値の同一性の側で int を f64 へ昇格しており、
`9007199254740993 == 9007199254740992.0` が真になる**非可逆な等値**だった。これも解消。

### 比較できる型が増えた

- `list` / `dict` を構造比較（B2 本体）。`fixed_list` も `list` と同じ規則。
- **参照の同一性**で比べる腕を追加: 関数 / ジェネレータ / `Namespace` / `FileObject` /
  `PyObject` / `NativeFunction` / `AsyncManager` / `Signal` / `EventLoop` ほか。
  ⚠ 腕が無く `_ => false` に落ちていたため、**`f == f` すら False** だった。
- ⚠ **クラスの同一性は `class_id`**（`Rc::ptr_eq` ではない）。`ClassValue::deep_clone` が
  クラスを複製するので、async の share-nothing 経路で**スレッドを跨いだ瞬間に `C == C` が
  False になっていた**（実測で再現）。`class_id` は `deep_clone` が引き継ぐので安定する。

### `__eq__` の適用範囲

- `==` に加えて **`in` / `not in` / set の `add` / `discard` / `remove` / 辞書の検索**でも効く。
  ⚠ 以前は `==` でしか呼ばれず、**`a == b` は真なのに `a in [b]` は偽**という食い違いがあった。
- ⚠ `__eq__` のハンドラが同じコンテナに触れうるので、**可変借用を握ったまま等値判定を
  走らせない**こと（`RefCell` が壊れる）。set の `add` と辞書の `set` は「引く」と「書く」を
  借用ごと分けてある。

### 循環参照

- `list` / `dict` の腕を足したことで、**両方が循環している 2 値**の比較が無限再帰しうる。
  ⇒ ①同一性の高速パス（`Rc::ptr_eq`）②深さ上限 → **`RecursionError`**。
- ⚠ Python も同じ構図（`hash()` に循環検出は無く、hashable ⇒ immutable ⇒ 循環を作れない。
  `==` は同一性の高速パス + 再帰上限）。Arrow は可変コンテナが循環しうるので②が要る。
- ⚠⚠ **深さで打ち切って `false` を返してはいけない。** 等しいものを「等しくない」と答える
  ＝サイレントな誤答で、B2 で消したはずのバグがそのまま戻る。打ち切りは必ずエラー。

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

~~1. **B2**~~ / ~~3. **B1**~~ … **完了**（上の各節を参照）。

残り:

1. **B3 + B7**（入れ子 `fn` のキャプチャ）— B3 はサイレントなので優先度が高い。
   ⚠ 起票時の「`capture_env` と `nested_fn_captures` の両方を見る」という見当は
   **外れている可能性がある**。B1 / B2 でも、起票時の原因の見立てより
   **実測での切り分けのほうが速く正確**だった（`__eq__` の適用漏れも
   クラスの `Rc::ptr_eq` も、起票には無い形で見つかった）。まず最小形で切り分けること。
2. **B5**（`list + list` 等）— `operators.rs` への追加だけ。小さく実用度が高い。
3. **B6**（モジュール本体の自己呼び出し）— 影響は大きいが原因未調査。
4. **B4**（`for` のスコープ）— **仕様判断が先**。実装より先に決めることがある。

## 3. 共通の留意点

- ⚠⚠ **どれも例題が 1 本も無かった**ために全ゲートを素通りしていた。
  直したら**必ず `examples/` に追加**すること（`.claude/rules/regulations.md`）。
- ⚠ 解釈側（`eval_*` / `exec_*` / `ops/`）を触るので、「挙動不変」を主張するときは
  `compare_bytecode.ps1` ではなく **`compare_outputs.ps1 -A <直前のコミットのビルド>`** を使う。
  ⚠ **使う前に同一 exe 同士で負の対照**を取る。
- ⚠ `impl_python/` は**触らない**方針（古いため）。差分が出たら
  `compare_python_impl.ps1` の `$knownDiff` に**実測した理由**をつけて登録する。
