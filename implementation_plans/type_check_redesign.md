# 静的型検査の再設計

**状態**: 設計確定（実装未着手）。決定 **D-1〜D-14** すべて確定済み。判断待ちなし
**起票**: 2026-09-12
**前提文書**: [type_binding_enforcement_plan.md](type_binding_enforcement_plan.md)（個別バグ修正キャンペーン 0-1〜A-4 の記録と型義務の棚卸し）
**進捗計**: `scripts/type_obligations.ps1`（型義務 **114 件**・着手時点 **STATIC 34%** → 現在 **64%**）

## 採番の規則

⚠ **3 つの名前空間だけを使う。混ぜないこと。**

| 名前空間 | 形 | 対象 |
|---|---|---|
| **決定** | `D-1` 〜 `D-14` | 仕様上の決定。§1。**単一採番**（旧 `U-x` / `R-x` は統合した） |
| **タスク** | `1.1` / `2.3` のような **フェーズ.連番** | 実装作業。§3。フェーズ番号が依存順を表す |
| **検体** | `B1` / `O9` / `K4` 等 | `scripts/type_obligations/*.ar` のファイル名に焼いてある**別物**。義務の識別子であってタスクではない |

旧採番（`T-x` / `U-x` / `R-x` / `L-x` / `S-x`）からの対応は §6。

---

## 0. なぜ再設計するか（実測）

0-1〜A-4 で 16 件の型検査の穴を塞いだが、**同じ密度で穴が再生する**。原因を測るために
型義務を数え上げた（`src/ast.rs` の `Stmt` 40 / `Expr` 30 バリアントから機械的に導出）。

| 判定 | 件数 | 意味 |
|---|---|---|
| `STATIC` | 39 | 静的検査がある |
| `RUNTIME` | 26 | その実行経路を踏まなければ見逃す |
| **`NONE`** | **49** | **無検査**。黙って不整合な型が通る |

**静的に検査されているのは 34%。** 原因は 3 つで、どれも個別修正では閉じない。

### 原因① `Unresolved` が万能受容体

`type_matches_exact`（`type_check/type_utils.rs`）の冒頭が
`if *arg_ty == InferredType::Unresolved { return true; }`。
**1 箇所の推論漏れが下流の全義務を無効化する。** `NONE` の大半がこれに帰着する。

既知の発生源（すべて実測）:

| 源 | 実害 |
|---|---|
| テンプレート実体化 `Box[str]("s")` | `let x: int = Box[str]("s")` が通る |
| 関数値 `wrong`（`fn` を名前で参照） | `let x: int = wrong` が通る |
| enum メンバー・`.value` | `let s: str = E.A.value` が通る |
| **組み込み関数の戻り値**（`float()`/`len()`/`str()`/`int()`） | `let s: str = float(n)` が通る |
| コレクションリテラルの要素型崩れ（原因②） | 下記 |

### 原因② 推論が「揃わなければ要素型を捨てる」— 捨てた形が何にでも適合する 【✅ タスク 2.7 で解消】

`Expr::List`（`type_check/infer.rs:20-32`）は要素型が一致しないと素の `List` に落とし、
素の `List` は `is_list_like` 規則で `ListOf(任意)` と適合する。**捨てることが「何でも通す」になる。**
実測: `[1, "s"]` が `list[int]` にも `list[str]` にも `list[float]` にも通る。`Set`/`Dict` も同じ。

### 原因③ 検査が AST の地点ごとの手書きで、数え上げる仕組みが無い

`infer()` が「型を返す」と「部分木を歩いて診断を出す」を兼ねているため、
`self.infer(e);` と結果を捨てる書き方が**正当な場合と義務忘れの場合で区別が付かない**
（実測 **19 箇所**が結果を捨てている）。`InferredType` は `#[must_use]` でもないので
コンパイラは何も言わない。0-1〜A-4 で塞いだ 16 件はすべてこの形だった。

### 付随1: 適合判定の独立実装が 4 本ある

| 実装 | 場所 | 用途 |
|---|---|---|
| `type_matches_exact` | `type_check/type_utils.rs` | 静的検査 |
| `value_matches_type_ann` | `interpreter/ops/typecheck.rs:133` | 実行時（注釈文字列） |
| `value_is_type` | `interpreter/ops/typecheck.rs:219` | 実行時（型名） |
| `TypeTag::matches` | `vm/op.rs:45` | VM 高速経路 |

すでに食い違っている（`Optional[` を実行時だけ受ける／`uint` の扱い）。

### 付随2: 「型検査が嘘をつく」箇所が 2 件ある

静的に通したうえで実行時に別の型が居座る。取りこぼしより悪い。

| # | 箇所 | 実測 |
|---|---|---|
| 1 | **`__cast__[T]` を受理して変換しない** | `let x: int = Conv(5)` → `x` に `Conv` が入り `x + 1` が実行時に爆発 |
| 2 | **`and`/`or` の静的結果型が `Bool` なのに実行時は被演算子を返す** | `let r: bool = a and b`（a,b は int）→ `r` に `2` が入る |

---

## 1. 決定事項（D-1 〜 D-14）

### D-1 型検査を 3 種に分ける

煩雑化（度重なる仕様変更でコードが散ること）を防ぐのが目的。

| Kind | 内容 |
|---|---|
| **Kind 1** | 実型が期待型と**同一**、または期待型へ**自明キャスト可能**であること |
| **Kind 2** | 実型が期待型へ**アップキャスト可能**であること（trait / protocol / Intersection が期待型のとき） |
| **Kind 3** | 実型が、**オーバーロードされた演算子が許可する型**に合致する、またはそのクラスへアップキャスト可能であること |

**場所（検査地点）× 型の種類**で Kind を使い分ける（§2 の義務表）。

⚠ **D-5 で自明キャストを廃止するため、Kind 1 は実質「同一のみ」になる。**
「自明キャスト」の枠は `fixed_list → list` のような将来の拡張のために残すが、
**許可する関係を増やすときは必ず変換挿入地点を併せて定める**（D-7 の原則）。

### D-2 型検査は型式解決後に行う

`alias` の展開・`new_type` の同定・テンプレートの具体化・`Intersection`/`Result`/`Union` の
構造化を**先に**済ませ、検査は解決後の型に対して一様に適用する。

⚠ これにより既存バグが閉じる。現在 `resolve_declared_type`
（`type_check/stmt/resolve.rs:184`）は `Intersection` と `Result` の注釈について
**「注釈自身の妥当性」だけ検査して右辺と照合せずに early-return** している
（`let v: Intersection[Alpha, Beta] = OnlyAlpha(..)` と `let r: Result[int,str] = 1.5` が通る。
ARG/RET では捕まるので、原因がこの関数であることの証拠になっている）。→ タスク **1.1**

### D-3 方向はアップキャストのみ（ダウンキャスト不許可）

実行時検査なしのダウンキャストは不健全。現在も `let back: Impl = t`（trait 変数から
具体クラスへ）は静的に弾いている（実測）。この挙動を保つ。

⚠ これにより原因②が閉じる。`list` → `list[int]` は**ダウンキャスト**（情報の増加）なので
禁止される。現在は `(List, ListOf(_)) => true` の**双方向**特例があり、これが
「要素型を捨てると何でも通る」の実装本体になっている。→ タスク **4.1**

### D-4 現在 `Unresolved` になっているものにも 3 分類を適用する

⚠ そのため**先に推論を埋める**（フェーズ 2）。原因①の表にある源はいずれも
「決定できない」のではなく**決定を試みていない**。情報の在処は実測で確認済み:

| 式 | 情報の在処 |
|---|---|
| `Box[str]("s")` | `call_check.rs:203` が `name` と `type_args` を**既に手元に持ち、引数検査に使っている**のに結果型だけ捨てる。`InferredType::GenericInstance`（A-2 で新設済み）がそのまま使える |
| `wrong`（`fn` 名参照） | `infer.rs:71` の `Expr::Ident` が**変数スコープだけ**を引く。レジストリの `fn_sigs` に引数・戻り値が揃っており `InferredType::Function`（既存）も表現力を持つ |
| `Color.Red` / `.value` | `registry/builder.rs:227` は `enum_item_<name>` を名前登録するだけで `value` フィールドを登録しない。`.value` は言語規則として `int`（実行時 `build_enum_classes` が強制） |

### D-5 暗黙キャスト（`int → float`）を廃止する

代わりに `int` の**組み込み float キャスト関数**を使う。

⚠ **`float(n)` は既に存在し `3.0` を返す**（実測）。`n => float` は使えない
（`=>` はインスタンスと `new_type` 専用）ので受け皿は `float()`。

#### 実測（暗黙 `int → float` を一時的に外して計測）

| 項目 | 実測値 |
|---|---|
| 壊れた例題 | **9 件** |
| うち**本キャンペーンで追加**した例題（拡大の実演が目的） | **8 件** |
| **キャンペーン以前から存在**した例題 | **1 件**（`examples/interop/cpp_default_arg_native_call.ar` #56 の既定値 1 行） |
| 現在、昇格を実際に行っている実装地点 | **33 箇所** |
| 昇格の可否を持つ適合判定の独立実装 | **4 本** |

⇒ 廃止により **33 箇所の変換地点が 0 になり**、「受理したなら変換地点がある」という
不変条件が自明に保たれる。`mut`（書き戻し）引数の例外（0-B2 で
`cpp_prim_ptr_int_arg_type_mismatch` を落とした件）も不要になる。
⚠ 実測後にソースは md5 照合して完全復元済み（ゲート同値を確認）。

### D-6 組み込み関数にも戻り値型を持たせて検査する

⚠⚠ **D-5 の前提。** 型検査側に組み込みのシグネチャ表が**存在しない**（実測）:

| 式 | 結果 |
|---|---|
| `let f: float = float(n)` | 通る（ただし `float` だからではなく `Unresolved` だから） |
| **`let s: str = float(n)`** | ⛔ **通る** |
| `let s: str = len([1, 2])` / `let n: int = str(1)` / `let s: str = int("3")` | ⛔ 通る |

`registry/builder.rs` にあるのは `BUILTIN_NEW_TYPES` の 3 件だけ。このまま D-5 を入れると
全ての変換が**無検査の式**を経由し、**穴が閉じずに移動する**。→ タスク **2.4**（4.2 と同時）

### D-7 暗黙キャスト不可なキャストは `=>` を強制する

組み込みで許されていないキャストは**キャスト演算子の使用を強制**する。
⇒ `type_matches_exact` 末尾の `__cast__[T]` による受理をやめる（付随2 の 1 件目）。

⚠ **原則**: 許可する非同一関係には**変換を挿入する地点**が必ず対応する。
対応が無い行は「受理するが変換しない」＝型検査が嘘をつく状態。
⚠ `fixed_list → list` を将来許すなら `Value::FrozenList` → `Value::List` の実変換が必要
（現在は許していない。許されているのは `list_like` へだけ）。さらに `mut list` 引数へ
渡すと凍結リストへの書き戻しになるので禁止。

### D-8 `+=` は加算演算子と代入演算子の**二回**の型検査を経る

`x += v` の義務は「`typeof(x + v)` が `typeof(x)` へ代入可能」。
右辺 `v` を左辺型と照合するだけでは**両方向にずれる**（実測の反例）:

```arrow
class Vec:
    mut n: int
    fn __add__(self, let other: Vec) -> str:   # 戻り値が Vec ではない
        return "oops"
mut v = Vec(1)
v += Vec(2)
print(v)        # oops   ← Vec 変数に str が入る（現状は無検査）
```

⇒ 検査 1（Kind 3: `x + v`）→ 結果型 `R` → 検査 2（Kind 1: `R` → `typeof(x)`）。

### D-9 Kind 3 は「可否」だけでなく**結果型**を返す

- **組み込み演算子**: 要素型を**合成**する
- **クラス**: `__add__` 等の**宣言戻り値型**を使う

⚠ 結果型が粗いと**正しいコードが落ちる**（実測で確認した要件）:

| 結果型の精度 | `list[int] += list[int]` | `list[int] += list[str]` |
|---|---|---|
| 素の `list` を返す | ⛔ 誤って弾く | ✅ 弾く |
| `list[int]` / `list[int,str]` の合成を返す | ✅ 通す | ✅ 弾く |

⚠ 演算子メソッドは **15 種が実在**する:
`__add__ __sub__ __mul__ __mod__ __pow__ __eq__ __ne__ __lt__ __le__ __gt__ __ge__ __neg__ __contains__ __cast__ __call__`。
`__eq__`/`__ne__` が `==`/`!=`、`__contains__` が `in` を担う。

### D-10 `and` / `or` の結果型を**被演算子型の join** にする

付随2 の 2 件目の解消。**実行時の意味論は変えない**（被演算子を返す Python 流を維持）。

現在は型検査が `And`/`Or` を無条件 `Bool` と宣言する（`type_check/binop.rs` の
`infer_binop_result`）のに実行時は被演算子を返すため、**静的に通して実行時に嘘になる**:

```arrow
let a: int = 1
let b: int = 2
let r: bool = a and b    # 静的には通る（And -> Bool と宣言されているため）
print(r)                 # 2   ← bool 変数に int が入る
```

⇒ `bool and bool → bool`、`int and int → int`、`int and str → 両者の join`。
これで `if a and b:`（非 bool）が **D-12 の条件検査で静的に落ちる**。
⚠⚠ **D-12 はこれが前提。** 結果型が `Bool` と宣言されている限り、条件を `bool` 厳密に
しても `if a and b:` は静的に通ってしまう。
⚠ `Any` / `Union` の被演算子は `check_binop` が必ずエラーを報告するので、
そこで `Unresolved` になるのは**報告済みエラーの下流**（塞ぐ対象ではない・旧 2.6 を取り下げた）。

### D-11 空のコレクションリテラルは `list[⊥]`、注釈なしの束縛は `list[Any]`

| 形 | 型 | 根拠 |
|---|---|---|
| `[]` そのもの | **`list[⊥]`** | 要素型が下端。あらゆる `list[T]` へ**アップキャスト可** |
| `mut xs: list[int] = []` | `list[int]` | `list[⊥]` → `list[int]` はアップキャスト ✅ |
| `let xs = []`（注釈なし） | **`list[Any]`** | 既定値。`list[⊥]` → `list[Any]` もアップキャスト ✅ |

⚠⚠ **`list[Any]` を `[]` 自身の型にしてはいけない。方向が逆になる。** 実測:

| 代入 | 結果 |
|---|---|
| `list[int]` → `list[Any]` | ✅ 通る（アップキャスト） |
| `list[Any]` → `list[int]` | ⛔ **弾かれる** |

`Any` は要素型の**上端**なので `list[Any]` は `list[int]` の**スーパータイプ**。
`[] : list[Any]` では D-3 のもとで `mut xs: list[int] = []` が落ちる。

⚠ **`Any` を引数側でも万能にする案は採らない。** `list[Any]` → `list[int]` を通すと
`let xs: list[Any] = [1,"s"]; let ys: list[int] = xs` が通るようになる
（**現在は正しく弾いている** — 実測）。閉じている穴と引き換えになる。
⚠ `⊥` は `set` / `dict` / `tuple` にも要る（`{}` / `()`）。
⚠ `language-differences.md` の「空コレクションは明示的な型付けが必要」は未実装かつ
この決定と矛盾するため注記済み（`let xs = []` は `list[Any]` として許す）。

### D-12 `if` / `while` の条件は `bool` 値のみ許可する

⇒ 義務表の「条件」を **Kind 1（期待型 `bool`）** にする。

| | 真偽性（旧） | **`bool` 厳密（決定）** |
|---|---|---|
| `if 0:` | 書ける（偽） | **静的エラー** → `if n != 0:` |
| `if xs:`（リスト） | 書ける（空なら偽） | **静的エラー** → `if len(xs) > 0:` |
| `if some_instance:` / `if some_func:` | 書ける（**常に真**） | **静的エラー** |

⇒ 実利は「**常に真になる条件の書き間違いを静的に検出できる**」こと（`()` を忘れた
`if some_func:` など）。

**移行コストは例題 1 箇所**（実測）: 条件に裸の識別子を使う箇所は 33 あるが大半は
`flag` / `is_done` / `running` / `found` のような bool 値。非 bool は
**`examples/classes/operator_overload.ar:87` の `if empty:` だけ**
（`examples/typing/variadic.ar` の 2 箇所は `is not None` で bool 式）。

⚠⚠ **`is_truthy` は撤去しない。** 条件から外れても次で必要:

| 用途 | 場所 |
|---|---|
| `not` / `and` / `or` の評価 | `interpreter/ops/operators.rs` |
| **ネイティブ ABI の関数表に export** | `native_api/mod.rs`・`native_api/callbacks.rs`・`cpp_bridge/codegen.rs`・`partial_compiler/rs_loader/codegen.rs`（`is_truthy: extern "C" fn(i64) -> i32`） |

関数表はネイティブ側の C ABI なので**削除・改名すると FFI が壊れる**。
D-12 は「条件位置に静的検査を足す」ことであって `is_truthy` の撤去ではない。

### D-13 `function` 型は**引数は反変・戻り値は共変**

⚠ 引数を共変にすると不健全:

```arrow
fn narrow(let v: int) -> int:
    return v
let f: function[Any]->int = narrow      # 引数が共変ならこれが通る
print(f("s"))                           # ⛔ int 宣言の引数へ str が渡る
```

`function[Any]->int` を期待する側は「**何を渡してもよい**」と約束されているので、
`int` しか受けない関数を入れると破れる。逆向きは安全。

### D-14 整合性検査とは別に「妥当性検査」を置く

⚠⚠ 3 分類はすべて「**2 つの型が適合するか**」を答える。次はどれにも当てはまらないが
実測で漏れている:

| 種別 | 実測した該当 | 現状 |
|---|---|---|
| **名前の存在** | `is NoSuchType`（死んだ腕 ＋ 腕の中のメンバーアクセスが全て無検査になる） | `NONE` |
| | `c.nope`（メンバー存在。「フィールドは全て宣言必須」なのに） | 実行時のみ |
| **個数** | 型引数の個数 `Box[int, int]` | `NONE` |
| | 実引数の個数 | ✅（既存） |
| **定義時の妥当性** | `enum` バリアント値が `int`（**未呼出関数内は無検査**） | `NONE` |
| | `Intersection` メンバーの相互両立 / `Result` の ok ≠ err | ✅（既存） |

⚠⚠ **抽象的な懸念ではない。D-2 で直す既存バグがこの混同の産物である。**
`resolve_declared_type` の `Intersection`/`Result` の早期 return は
**「定義時の妥当性」を検査して満足し、整合性検査へ進まずに return している**。
2 つのカテゴリを同じ関数の同じ分岐で扱ったために、片方をやれば済んだ気になった形。

---

## 2. 義務表（設計の中核）

⚠⚠ **これが無いと再設計しても同じ密度で穴が再生する。** 原因③は「関係の判定」ではなく
「**問いを発していない**」ことだった。3 分類は「問われたときの答え方」を定義するが
「どこで問うか」は定義しない。実測した無検査の多く（`xs.append("s")` / `a[i] = v` /
`yield` / 可変長引数 / `static mut` / `except as`）はこの形。

### 2.1 実装上の強制

1. **役割を 2 関数に割る**
   - `walk(e)` … 部分木を歩いて診断を出す。**戻り値なし**
   - `type_of(e) -> Type` … 型を返す。**`#[must_use]`**
   - `walk` が `type_of` を呼ぶ
   ⇒ 「型を取ったのに使わなかった」19 箇所はコンパイラが指摘する

2. **`walk` を AST バリアントに対して網羅的にする**
   - `match` の `_ => {}` を**禁止**し、全バリアントを明示的に列挙する
   - 各アームに「この地点の義務は (Kind, 期待型の求め方, 変換挿入地点)」または
     **`義務なし` を明示**させる
   ⇒ 新しい構文を足すとコンパイルが止まる（`add-syntax` の手当て表と同じ仕掛け）

⚠ **1 だけでは不十分。** 「期待型を取りに行かなかった」は 1 では捕まらない
（`xs.append("s")` は `walk` も `type_of` も正しく呼べていて、期待型を求める処理が
**存在しない**だけ）。**2 が必須。**

### 2.2 場所 × Kind

| 場所 | Kind | 期待型 | 現状 | タスク |
|---|---|---|---|---|
| `let` / `mut` / `const` / `static` の束縛 | 1 / 2 | 注釈、なければ推論型 | ✅ | — |
| タプル分解束縛 | 1 | 位置ごとの要素型 | ✅ | — |
| `for` のループ変数 | 1 | 反復対象の要素型 | ✅ | — |
| 再代入 `x = v` | 1 / 2 | 束縛時の型 | ✅ | — |
| **複合代入 `x <op>= v`** | **3 → 1** | D-8 の二段 | `NONE`/`RUNTIME` | 5.1 |
| フィールド代入 `o.f = v` | 1 / 2 | 宣言フィールド型 | ✅ | — |
| trait フィールド代入 `o::T.f = v` | 1 / 2 | 同上 | 実行時のみ | 5.2 |
| **`static mut` クラス変数への代入** | 1 / 2 | 宣言型 | `NONE` | 5.3 |
| フィールド既定値 | 1 | 宣言フィールド型 | ✅ | — |
| **添字代入 `a[i] = v`** | 1 | 要素型 | `NONE` | 5.2 |
| 添字読み `a[i]` の添字 | 1 | `int`（`dict` は key 型） | 実行時のみ | 5.2 |
| 実引数 → 仮引数 | 1 / 2 | 仮引数の宣言型 | ✅ | — |
| 既定引数の値 | 1 | 仮引数の宣言型 | ✅ | — |
| **可変長引数の要素** | 1 | 宣言要素型 | `NONE` | 5.2 |
| **組み込みメソッドの引数**（`append` 等） | 1 | 要素型 | `NONE` | 5.2 |
| `return` | 1 / 2 | 宣言戻り値型 | ✅ | — |
| **`yield`** | 1 | 宣言 yield 型 | `NONE` | 5.2 |
| **`block_return` / `loop_yield`** | 1 | `->T` 注釈 | 実行時のみ | 5.2 |
| **二項・単項演算子** | **3** | 演算子が許可する型 | 実行時のみ（`==`/`!=`/`in` は `NONE`） | 4.4 |
| **`match` subject vs `case` パターン** | **1（厳密同一）** | subject の型 | `NONE` | 5.4 |
| **`if` / `while` の条件** | **1** | **`bool`**（D-12） | `NONE` | 5.5 |
| コレクションリテラル vs 注釈 | 1 | 合成した要素型 | `NONE` | 2.7 / 4.1 |
| `cast` `=>` / `mustbe` | 検査ではなく**変換**（D-7） | — | — | 4.3 |

### 2.3 型の種類 × Kind

| 型の種類 | 期待型に置いたときの Kind |
|---|---|
| プリミティブ（`int`/`float`/`str`/`bool`/`uint`/`complex`/`None`/`Undefined`） | 1 |
| コレクション（`list`/`set`/`dict`/`tuple`/`fixed_list`/`list_like`） | 1（要素型へ再帰） |
| 素のクラス（`NamedInstance`） | 1（Arrow はクラス継承を許さないので**完全一致**） |
| `trait` / `protocol` / `Intersection` | **2** |
| `new_type` | 1（基底と**別の型**。両方向とも不可） |
| `alias` | 1（**透過**。D-2 の解決で消える） |
| `enum` / `enum_item` | 1 |
| テンプレート実体（`GenericInstance`） | 1（型引数へ再帰） |
| `function[..]->T` | 1（**引数は反変・戻り値は共変** — D-13） |
| `type[T]`（`TypeVal`/`TypeValOf`） | 1（`type[trait]` は 2） |
| `Union` / `Option` | 1（いずれかのメンバーに適合） |
| `Result[T,E]` | 1 |
| `Any` | 常に可（**上端**） |
| `⊥` | 常に可（**下端**・D-11） |

⚠ **クラス継承が無いことが完全一致判定の根拠**（`parser/classes.rs` が
`cannot inherit from ... (only traits are allowed as bases)` で弾く）。
クラス継承を入れるならこの行を継承チェーン探索へ変える（`FieldCheck` の doc にも同じ注意）。

---

## 3. 実装タスク

⚠ **フェーズ番号が依存順。** 同一フェーズ内のタスクは原則**並行可**（例外は §5 に明記）。
⚠ 各タスクごとにテスト実行・本書の更新・コミットを行う（既存キャンペーンと同じ手順）。
⚠ `scripts/type_obligations.ps1` が進捗計。着手前 **STATIC 34%**（39/114）。

### フェーズ 1 — 独立した既存バグの修正

他に依存しない。先に入れて以降の差分を小さくする。

| # | 内容 | 決定 | 検体 | 規模 |
|---|---|---|---|---|
| ~~**1.1**~~ | ~~`resolve_declared_type` の `Intersection` / `Result` の早期 return に整合性検査を足す~~ → **✅ 完了 2026-09-12** | D-2 | `K4` `K5` → **STATIC** | 下記「1.1 の記録」 |
| ~~**1.2**~~ | ~~`FieldKind::StaticMut` を可変として数える~~ → **✅ 完了 2026-09-12** | — | — | 下記「1.2 の記録」 |
| ~~**1.3**~~ | ~~`protocol` を容器の内側でも一様に扱う~~ → **✅ 完了 2026-09-12** | — | `K11` `K12` → **STATIC** | 下記「1.3 の記録」 |

#### 1.1 の記録 【✅ 完了 2026-09-12】

**修正の本体は早期 return を外すこと。** 妥当性検査（注釈自身が成り立つか）のあと、
**必ず**整合性検査（右辺と適合するか）へ落ちるようにした（D-14 の 2 系統を実装で分離）。

| 層 | 内容 |
|---|---|
| `resolve_declared_type` | `Intersection` / `Result` の早期 return を撤去。`from_ann` 後の共通経路で妥当性検査（`check_intersection_members` / `validate_result_type`）を行い、`return` せず整合性検査へ落とす |
| | ⚠ protocol の早期 return は**残す**。`check_protocol_conformance` が右辺との照合も行うため二重にならない（その旨をコメントに明記） |

**検体**: `K4` `K5` が `NONE` → **`STATIC`**。静的検査の割合 34% → **36%**（41/114）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` / `force_gate.ps1` | 既知の `bench_ab_native.ar` のみ / 0 fall back |
| `compare_python_impl.ps1` | 75/75 identical・stale 0（新規 2 例題を `$knownDiff` へ） |
| `compare_outputs.ps1 -A <HEAD>` | 183/185。差分 2 件はどちらも**意図したもの**（下記） |
| `compare_bytecode.ps1 -A <HEAD>` | 203/204。差分は新規エラー例題のみ（静的エラーなので chunk が作られない） |
| `compare_wasm_frontend.ps1` | **255/255 agreed・INVENTED 0**（`src/type_check/` を触ったので wasm 再ビルド＋VSIX 再生成） |

**意図した出力差分**

| 例題 | 内容 |
|---|---|
| `typing/intersection_result_bind_error.ar` | 新規。塞いだ穴そのもの |
| `classes/intersection_error.ar` | **既存例題にエラーが 1 件増えた**。`let x: Intersection[Flyable, Swimmable] = Bird()` は例題自身が「Bird は Swimmable を実装していない」と書いているのに、以前は型ガードのエラーだけが出ていた。⚠ 例題のコメントを実態に合わせて更新した |

**追加した例題**: `examples/typing/intersection_result_bind{,_error}.ar`

#### 1.2 の記録 【✅ 完了 2026-09-12】

`matches!(kind, FieldKind::Mut)` を `matches!(kind, FieldKind::Mut | FieldKind::StaticMut)` へ。
**同じ式が 2 箇所**（`registry/builder.rs` と `stmt/check.rs`）にあり、片方だけ直すとずれるので
両方を直し、互いを参照するコメントを置いた。

⚠ `static mut` の仕様は「全インスタンスで共有される**可変**セル。インスタンス経由・
クラス名経由どちらでもアクセス・**代入可能**」（`ast.rs` の `FieldKind::StaticMut` の doc）。
**実行時は実装済み**（`attrs.rs` が `inst_class.static_vars` を更新する）で、
静的検査だけが `cannot assign to immutable field` で塞いでいた**層の食い違い**だった。

⚠ 露見しなかったのは、既存例題（`class_trait.ar` の `Registry.entry_count`）が
**クラス名経由でしか代入しておらず**、インスタンス経由を書いた例題が 1 本も無かったため。

⚠ **可変にしただけで型検査は緩んでいない。** `c.total = "s"` は
`field 'total' of class 'Counter' is declared 'int' but got 'str'` で弾かれる
（1.2 の前は「型が違う」ではなく「不変フィールド」という**誤った理由**のエラーも一緒に出ていた）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` / `force_gate.ps1` | 既知の `bench_ab_native.ar` のみ / 0 fall back |
| `compare_python_impl.ps1` | 75/75 identical・stale 0（新規 2 例題を `$knownDiff` へ。py は `static mut` のインスタンス経由アクセス自体が未対応） |
| `compare_outputs.ps1 -A <1.1 前>` | 183/187。差分 4 件は 1.1 の 2 件＋1.2 の新規 2 件のみで、**既存例題は `intersection_error.ar` 以外すべて不変** |
| `compare_wasm_frontend.ps1` | **257/257 agreed・INVENTED 0**（wasm 再ビルド＋VSIX 再生成） |
| `type_obligations.ps1` | 41/114（36%）・退行なし |

**追加した例題**: `examples/classes/static_mut_assign{,_error}.ar`
⚠ 正常系の例題が「インスタンス経由の代入」という**今まで例題が 1 本も無かった形**を埋めている。

#### 1.3 の記録 【✅ 完了 2026-09-12】

⚠⚠ **着手前の診断（「`resolve_protocols` が容器へ再帰しない」）は誤りだった。**
`resolve_protocols` は既に `ListOf` / `SetOf` / `DictOf` / `Union` / `Tuple` /
`GenericInstance` / `Result` へ再帰している。実際の原因は別で、**経路によって 2 通りに
壊れていた**（実測）:

| 経路 | 症状 | 原因 |
|---|---|---|
| フィールド代入・戻り値（`check_expected` 経由） | **非適合クラスが通る**（偽陰性） | `type_matches_exact` の `Protocol` アームが「適合チェックは別途実施するため」というコメント付きで**任意の `NamedInstance` を受理**していた。その「別途」は `check_expected` が**期待型が最上位 `Protocol` のときだけ**行うので、容器の内側では誰も検査していなかった |
| `let` 束縛（`resolve_declared_type` 経由） | **構造的に満たすのに弾かれる**（偽陽性） | この経路は `type_matches` を直接呼び `resolve_protocols` を通らないため、`list[HasN]` の `HasN` が `NamedInstance` のまま**名前**で比較されていた。`Dog` は `HasN` を宣言していないが `mut n: int` を持つので満たすはずだった |

**実装**

| 層 | 内容 |
|---|---|
| `satisfies_protocol`（新設・`stmt/protocol.rs`） | `ty` が protocol を**構造的に満たすか**の**純粋な述語**。`type_matches_exact` は `&self` なので、診断を出す `check_protocol_conformance`（`&mut self`）を呼べない |
| `type_matches_exact` の `Protocol` アーム | 「任意の `NamedInstance` を受理」を `satisfies_protocol` に差し替え |
| `resolve_declared_type` | 照合の直前で `declared` / `rhs_ty` の両方に `resolve_protocols` を適用 |

⚠ 判定規則は `check_protocol_conformance` と**同じに保つこと**（あちらは失敗理由を個別に
報告するため一本化できていない）。畳むのは**フェーズ 3.3**（3 分類の単一定義）の仕事。
doc コメントに相互参照を置いた。

**検体**: `K11` `K12` が `NONE` → **`STATIC`**。静的検査の割合 36% → **38%**（43/114）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` / `force_gate.ps1` | 既知の `bench_ab_native.ar` のみ / 0 fall back |
| `compare_python_impl.ps1` | 75/75 identical・stale 0（新規 2 例題を `$knownDiff` へ） |
| `compare_outputs.ps1 -A <フェーズ1 前>` | 182/189。差分 7 件は 1.1〜1.3 の新規 6 件＋下記 1 件 |
| `compare_wasm_frontend.ps1` | **259/259 agreed・INVENTED 0** |

⚠ 既存例題 `classes/protocol_sites_error.ar` にエラーが 1 件増えた
（`argument 0 of 'render' expects 'protocol Drawable' but got 'Bad'`）。
非適合クラスを protocol 型の仮引数へ渡す地点が、以前は**許容アームのおかげで無検査**
だった。正しい追加診断（引数位置の要約）。

⚠ **偽陽性を出していないことを全経路で確認した**（実測）: 構造的に満たすが protocol を
宣言していないクラスが 引数 / 戻り値 / フィールド / 容器の内側 / `dict` の値側 すべてで通る。

**追加した例題**: `examples/classes/protocol_in_container{,_error}.ar`

### フェーズ 2 — 推論を埋める（3 分類の材料）

⚠ **3 分類は受け取る実型が空だと効かない。** フェーズ 3 以降の前提。

| # | 内容 | 決定 | 検体 | 備考 |
|---|---|---|---|---|
| ~~**2.1**~~ | ~~テンプレート実体化の結果型を `GenericInstance` にする~~ → **✅ 完了 2026-09-12** | D-4 | `K7` `K10` `K13` `K15` → **STATIC** | 下記「2.1 の記録」 |
| ~~**2.2**~~ | ~~関数値の型を `Function` にする~~ → **✅ 完了 2026-09-13**（**4.5 を吸収**） | D-4 / **D-13** | `K8` `K9` `C14` → **STATIC** | 下記「2.2 の記録」 |
| ~~**2.3**~~ | ~~`enum_item_<name>` に `value: int` を登録し、メンバーを `NamedInstance` にする~~ → **✅ 完了 2026-09-13** | D-4 | `N3` `N4` `K14` → **STATIC** | 下記「2.3 の記録」 |
| ~~**2.4**~~ | ~~**組み込み関数のシグネチャ表**~~ → **✅ 完了 2026-09-13**（4.2 と同時） | D-6 | `Z9` `Z11` `Z12` → **STATIC**／`Z10` は見送り | 下記「2.4 / 4.2 の記録」 |
| ~~**2.5**~~ | ~~`and` / `or` の結果型を被演算子型の join にする~~ → **✅ 完了 2026-09-13** | D-10 | `O9` `O10` → **STATIC** | 下記「2.5 の記録」。⚠ **5.5 の前提** |
| ~~**2.6**~~ | ~~`infer_binop_result` の `Any` / `Union` → `Unresolved` の 2 源を塞ぐ~~ → **❌ 取り下げ**（前提が誤り） | — | — | 下記「2.6 を取り下げた理由」 |
| ~~**2.7**~~ | ~~コレクションリテラルの要素型を**合成**する~~ → **✅ 完了 2026-09-13** | D-3 | `L1` `L3` `L9` `L15` `L16` `L18` `L19` → **STATIC**（**7 件**） | 下記「2.7 の記録」 |
| ~~**2.8**~~ | ~~`except ... as name` の束縛型を付ける~~ → **✅ 完了 2026-09-13** | D-4 | `E3` → **STATIC** | 下記「2.8 / 2.9 の記録」 |
| ~~**2.9**~~ | ~~`is` の絞り込みを `match` **式**でも効かせる~~ → **✅ 完了 2026-09-13** | — | `X7` → **STATIC** | 同上 |

#### 2.1 の記録 【✅ 完了 2026-09-12】

| 層 | 内容 |
|---|---|
| `template_call_result_type`（新設・`call_check.rs`） | `Base[T1,T2](args)` の結果型。クラスなら `GenericInstance { name, args }`、テンプレート関数なら宣言戻り値型を `subst_type_params` で置換 |
| `infer_call_inner` の `TemplateInstantiate` アーム | `Unresolved` を捨て返していたのを結果型に差し替え |
| `infer.rs` の `Expr::TemplateInstantiate` | 呼び出さずに値として使う形（`let c = Box[int]`）に `TypeValOf(GenericInstance{..})` を付ける。素のクラス名 `C` が `TypeValOf(NamedInstance("C"))` になるのと揃えた |
| `type_matches_exact` | `GenericInstance{name,..}` → `NamedInstance(name)` を**一方向**で許可（型引数を忘れる＝アップキャスト）。`list[int]` → `list` と同じ扱い |

⚠ 解釈できない型引数・型引数の個数不一致・オーバーロードは `Unresolved` に倒す
（嘘の型を作らない）。⚠ 個数不一致を `Unresolved` にしたことで `K13`（`Box[int,int]`）も
**副産物で `STATIC` になった**（注釈側の `Box[int,int]` が `from_ann` で解釈できず、
右辺の `Box[int]` と食い違うため）。

⚠⚠ **`GenericInstance` → `NamedInstance` を双方向にしないこと。** 逆（`Box` → `Box[int]`）は
情報が増えるダウンキャストで D-3 違反。素の容器の双方向特例（`(List, ListOf(_))` 等）は
タスク 4.1 で撤去する予定なので、ここを真似てはいけない。

⚠ **残した設計上の未決**: `Box[int]` → `Box[Any]` は現在**不可**（`GenericInstance` 同士は
厳密一致）。一方 `list[int]` → `list[Any]` は**可**（容器は共変）。
**ユーザー定義テンプレートの分散が未定義**で容器と挙動が違う。保守的側（不変）に倒してあるが、
決めるならフェーズ 3.3（3 分類の単一定義）で扱う。

**検体**: `K7` `K10` `K13` `K15` が **4 件前進**（`K10` は `RUNTIME` → `STATIC`）。
静的検査の割合 38% → **41%**（47/114）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` / `force_gate.ps1` | 既知の `bench_ab_native.ar` のみ / 0 fall back |
| `compare_python_impl.ps1` | 76/76 identical・stale 0（エラー例題のみ `$knownDiff` へ。正常系は py と一致した） |
| `compare_outputs.ps1 -A <フェーズ1 前>` | 183/191。差分 8 件は新規例題 7 件＋既知の既存 2 件のみ |
| `compare_wasm_frontend.ps1` | **261/261 agreed・INVENTED 0** |

⚠ 既存例題 `typing/generic_type_ann.ar` が一度落ちた（`mut bare: Box = Box[str]("x")`）。
結果型が付いたことで注釈 `Box` と右辺 `Box[str]` が食い違ったため。
**型引数を忘れる方向はアップキャスト**なので `type_matches_exact` に一方向の規則を足して解消。

**追加した例題**: `examples/typing/template_result_type{,_error}.ar`

#### 2.2 の記録 【✅ 完了 2026-09-13・タスク 4.5 を吸収】

⚠⚠ **計画の依存関係が誤っていた。** 当初 `2.2 → 4.5`（関数値に型を付けてから分散規則）と
していたが、**分離できない**。関数値に型を付けた時点で既存の関数型注釈との照合が始まるため、
比較規則が無いまま 2.2 だけ入れると**例題が 13 件壊れた**（実測）。⇒ 4.5 を 2.2 に吸収した。

**原因の特定（実測で 3 段階）**

| # | 症状 | 原因 |
|---|---|---|
| 1 | `'x' is declared 'int'` エラーが出ない | `check_fn_def` が関数名を **`Unresolved` で宣言**していた（`infer.rs` の `Expr::Ident` 側に `fn_sigs` 参照を足しても `lookup` が成功するので届かない） |
| 2 | 可変長引数が `takes 0 argument(s)`・`mut` 引数の判定が変わる（**13 例題**） | 関数名に型が付いたことで **直接呼び出しが `check_call_args` から `check_fn_type_call`（関数**値**用）へ迂回**した。後者は「シグネチャだけ判っている関数値」用で、可変長・既定値・`mut` 引数・オーバーロードを正しく扱えない |
| 3 | `function{let param1:int}->int` vs `function{let y:int}->int`（**5 例題**） | `FnTypeParam` が `PartialEq` を derive しており **`name` も比較に入る**ため、注釈由来と実体由来が**引数名違いだけで不一致**になっていた |

**実装**

| 層 | 内容 |
|---|---|
| `fn_value_type`（新設・`infer.rs`） | `fn_sigs` から `InferredType::Function` を組む。⚠ オーバーロード・テンプレート関数・**可変長パラメータ**は `None`（型が 1 つに決まらない／`FnTypeParam` が可変長を表せない） |
| `check_fn_def` | 関数名を `Unresolved` ではなく `fn_value_type` の結果で宣言 |
| `infer_call_inner` | **直接の関数名呼び出しは `check_call_args` に残す**ガード（`direct_fn_call`）。名前が `fn_sigs` に在るなら `Function` アームへ落とさない |
| `type_matches_exact` | `(Function, Function)` の比較を新設。**引数名は使わない**／**引数は反変・戻り値は共変**（D-13）／期待が素の `function` なら引数を問わない／実引数側のシグネチャ不明は通す |

**分散規則を 4 方向すべて実測で確認した**

| 向き | 期待 | 実測 |
|---|---|---|
| 引数反変（`function[Any]->int` → `function[int]->int`） | 通る | ✅ |
| 引数共変（`function[int]->int` → `function[Any]->int`） | **弾く**（不健全） | ✅ |
| 戻り値共変（`function[]->int` → `function[]->Any`） | 通る | ✅ |
| 戻り値反変（`function[]->Any` → `function[]->int`） | **弾く** | ✅ |

**検体**: `K8` `K9` `C14` が `NONE` → **`STATIC`**。静的検査の割合 41% → **44%**（50/114）。
⚠ `C15`（可変長引数の要素型）は一度 `STATIC` になったが、可変長を `None` に倒したので
`NONE` へ戻した。**嘘のエラーを出さないための意図的な見送り**（タスク 5.2 で別途扱う）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | **FAIL 0 件**（13 件の破壊を解消したことの確認） |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 77/77 identical・stale 0（エラー例題のみ登録。正常系は py と一致） |
| `compare_outputs.ps1 -A <フェーズ1 前>` | 184/193。差分 9 件は新規例題 7 件＋既知の既存 2 件のみ |
| `compare_wasm_frontend.ps1` | **263/263 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/function_value_type{,_error}.ar`

#### 2.3 の記録 【✅ 完了 2026-09-13】

| 層 | 内容 |
|---|---|
| `registry/builder.rs` の `Stmt::EnumDef` | `class_field_details["enum_item_<name>"]` に `value: int` を、`class_field_details["<name>"]` に各バリアント → `NamedInstance("enum_item_<name>")` を登録 |
| `infer_attr` | `class_and_subst` が `None` のとき `TypeValOf(NamedInstance(c))` → クラス `c` のメンバー表を引くフォールバックを追加。**クラス名経由のアクセス**（`Color.Red` / `Counter.total` / `C.K`）が同じ経路に乗る |
| `type_matches_exact` | `enum_item_X` → `X` をアップキャストとして許可（メンバーはその enum に属する） |

⚠ `.value` が `int` であることは**言語規則**で、実行時の `build_enum_classes` が
「enum variant value must be int」で強制している。型は決まっていたのに型検査が
知らなかっただけ。

⚠⚠ **`enum_item_X` → `X` の規則が必要だった。** 2.3 でメンバーに型を付けると
`let m: Color = Color.Green` という**自然な綴り**が落ちる（注釈は `NamedInstance("Color")`、
メンバーは `NamedInstance("enum_item_Color")`）。以前は `Unresolved` ゆえに書けていたので、
これを弾くのは**機能の退行**になる。⚠ 逆（`Color` → `enum_item_Color`）はダウンキャストなので許さない。

⚠ **副産物**: クラス名経由のアクセスに型が付くようになったので、`const` クラス変数
（`Counter.LIMIT`）と `static mut`（`Counter.total`）も `Unresolved` を卒業した。

**検体**: `N3` `N4` `K14` が `NONE` → **`STATIC`**。静的検査の割合 44% → **46%**（53/114）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | FAIL 0 件 |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 77/77 identical・stale 0 |
| `compare_outputs.ps1 -A <フェーズ1 前>` | 185/195。差分 10 件は新規例題 8 件＋既知の既存 2 件のみ |
| `compare_wasm_frontend.ps1` | **265/265 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/enum_member_type{,_error}.ar`

#### 2.5 の記録 【✅ 完了 2026-09-13】

`infer_binop_result` の `And` / `Or` を比較演算子のグループから分離し、
**被演算子型の join**（同型ならその型・異なれば `Union`）を返すようにした。

⚠ **実行時の意味論は変えていない**（案 (a)）。`x = a or default` の定番は書けるまま
（例題で短絡評価・既定値イディオムの両方を実測確認した）。

⚠ `Any` / `Union` の被演算子はこの関数の冒頭で `Unresolved` に落ちるので、
join で `Union` が入れ子になることはない。

**検体**: `O9` `O10` が `NONE` → **`STATIC`**。静的検査の割合 46% → **48%**（55/114）。
⇒ **「型検査が嘘をつく」2 件のうち 1 件目が解消**（残るは `__cast__` = タスク 4.3）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | FAIL 0 件 |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 78/78 identical・stale 0 |
| `compare_outputs.ps1 -A <フェーズ1 前>` | 185/197。差分 12 件は新規例題 10 件＋既知の既存 2 件のみ |
| `compare_wasm_frontend.ps1` | **267/267 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/and_or_result_type{,_error}.ar`

#### 2.6 を取り下げた理由 【❌ 2026-09-13】

⚠⚠ **前提が誤っていた。** 「`infer_binop_result` の `Any` / `Union` → `Unresolved` は
原因①（万能受容体）の発生源」と書いたが、実コードを読むと
**`check_binop` が同じ条件で必ずエラーを報告している**（`OperationOnAny` /
`OperationOnUnion`・全演算子で無条件・`return` 付き）。

⇒ あの `Unresolved` は「**既に報告したエラーの下流**」であって、黙って検査が消える
silent な穴ではない。塞ぐ対象ではないので取り下げる。
⚠ 設計文書の原因①の表から「`infer_binop_result` の `Any` / `Union` 分岐」も削除した。

#### 2.7 の記録 【✅ 完了 2026-09-13・**根本原因②の解消**】

**一度に 7 件前進した最大の一手。** 48% → **54%**（62/114）。

| 層 | 内容 |
|---|---|
| `join_elem_types`（新設・`infer.rs`） | 全要素同型 → その型／混在 → `Union`（重複は畳む）／1 つでも `Unresolved` → `Unresolved` |
| `Expr::List` / `Expr::Set` | 「揃わなければ素の `List`/`Set`」を撤去し `ListOf(join)` / `SetOf(join)` に |
| `Expr::Dict` | キー・値をそれぞれ合成して `DictOf(join_k, join_v)` に |

⚠⚠ **「捨てる」ことは中立ではなかった。** 素の `List` は `is_list_like` 規則で
`ListOf(任意)` と適合するので、要素型を捨てることが「**何でも通す**」になっていた。
同じ `[1, "s"]` が `list[int]` にも `list[str]` にも `list[float]` にも通っていた（実測）。

⚠ `Unresolved` の場合も `ListOf(Unresolved)` という**構造を保つ**形にした。素の `List` と
違い、タスク 4.1 で素の容器の双方向特例を撤去しても壊れない。
⚠ 空リテラルは当面そのまま（D-11 の `⊥` はタスク 4.1 で入れる）。
⚠ `Tuple` は元から位置ごとに型を持つので変更不要。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | **FAIL 0 件**（混在リテラルに依存した既存例題は無かった） |
| `force_gate.ps1` | 0 example(s) still fall back |
| `compare_python_impl.ps1` | 78/78 identical・stale 0 |
| `compare_outputs.ps1 -A <フェーズ1 前>` | 186/199。差分 13 件は新規例題 11 件＋既知の既存 2 件のみ |
| `compare_wasm_frontend.ps1` | **269/269 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/collection_elem_synthesis{,_error}.ar`

#### 2.8 / 2.9 の記録 【✅ 完了 2026-09-13】

##### 2.8 `except ... as e` の束縛型 ＋ 組み込み例外のフィールド型

| 層 | 内容 |
|---|---|
| `Stmt::Try` の handler | 束縛を `Unresolved` ではなく `NamedInstance(exc_type)` で宣言。bare `except:` は型が判らないので `Unresolved` のまま（取りこぼす方へ） |
| `registry/builder.rs` の `with_builtins` | **組み込み例外 19 種**に `message: str` / `code_context: str` / `file: str` / `line: int` / `col: int` を登録 |

⚠⚠ **束縛型だけでは足りなかった。** 束縛に `NamedInstance("ValueError")` を付けても
`e.message` はまだ `Unresolved` だった（実測）。組み込み例外が**名前だけ**登録されていて
`class_field_details` が空だったため。⚠ ユーザー定義例外（`class MyError(Error)`）では
束縛型だけで効いていた（`e.detail` が `int` と判明）ので、原因の切り分けができた。

⚠ 登録内容は実行時の `Error` trait（`interpreter.rs` の `trait_field_order`）と
**揃えること**。片方だけ変えると静的と実行時がずれる。

##### 2.9 `match` 式の絞り込み

`check_match_arms` を抽出し、`match` **文**（`check_match`）と **式**（`Expr::MatchExpr`）の
両方から呼ぶようにした。以前は式側が独自の走査を持ち `IsType` の絞り込みを**していなかった**
ため、文と式で意味論が違った（実測）。⚠ **別々に書くと再びずれる**ので 1 箇所に集約した。

**検体**: `E3` `X7` が `NONE` → **`STATIC`**。静的検査の割合 54% → **56%**（64/114）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` / `force_gate.ps1` | FAIL 0 件 / 0 fall back |
| `compare_python_impl.ps1` | 78/78 identical・stale 0 |
| `compare_outputs.ps1 -A <フェーズ1 前>` | 187/201。差分 14 件は新規例題 12 件＋既知の既存 2 件のみ |
| `compare_wasm_frontend.ps1` | **271/271 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/except_bind_and_match_expr{,_error}.ar`

#### フェーズ 2 の残り

| # | 状態 |
|---|---|
| ~~**2.4**~~（組み込み関数のシグネチャ表） | **✅ 完了 2026-09-13**（4.2 と同時・フェーズ 4 の記録を参照） |

### フェーズ 3 — 検査機構を作る

| # | 内容 | 決定 | 依存 | 備考 |
|---|---|---|---|---|
| ~~**3.1**~~ | ~~`walk` / `type_of` に分離~~ → **✅ 完了 2026-09-13** | — | フェーズ 2 | 下記「3.1 / 3.2 の記録」 |
| ~~**3.2**~~ | ~~`walk` を AST バリアントに対して網羅化~~ → **✅ 完了 2026-09-13**（**一部は既に満たされていた**） | — | 3.1 | 同上 |
| ~~**3.3**~~ | ~~3 分類（Kind 1/2/3）を**単一定義**として実装する~~ → **✅ 完了 2026-09-13** | D-1 | — | 下記「3.3 の記録」。⚠ 4.1 / 4.4 の**後**に実施した |
| ~~**3.4**~~ | ~~妥当性検査を 2 系統目として置く~~ → **✅ 一部完了 2026-09-13** | D-14 | `X6` `N1` `N2` → **STATIC**／`M1` `M2` は残す | 下記「3.4 の記録」 |

#### 3.1 / 3.2 の記録 【✅ 完了 2026-09-13】

##### 測ってみたら 3.2 の半分は既に満たされていた

⚠⚠ 設計時に「`match` の `_ => {}` を禁止する」と書いたが、**実測すると
`infer` の `Expr` match と `check_stmt` の `Stmt` match は既に網羅的**だった
（`_ =>` の 2 件は `Option` の内部 match）。
⇒ **新しい AST バリアントを足せば既にコンパイルが止まる。** 3.2 の前半は達成済みで、
残っていたのは後半「各アームに義務か『義務なし』を明示させる」だった。

##### 3.1 — `#[must_use]` ＋ 2 つのラッパ

| 追加 | 意味 |
|---|---|
| `infer` に `#[must_use]` | 結果を黙って捨てられないようにする |
| `walk(e)` | 「**この地点に型義務は無い**」と宣言して歩く |
| ~~`walk_obligation_pending(e, task)`~~ | 「**義務はあるが未実装**」と宣言して歩く。`task` に実装タスク番号を書く。⇒ **タスク 5.5 で撤去した**（印を付けた地点をすべて実装し終えたため。残すと「義務が無い」と区別できない死んだ足場になる） |

⚠ `walk` 1 つでは不十分。「義務が無い」と「義務を忘れている」を**同じ書き方にすると
再び区別が付かなくなる**ので 2 つに分けた。`walk_obligation_pending` は
`grep` で数え上げられるので、**コード側の記録**として機能する
（外側の網は `scripts/type_obligations.ps1` の 114 件。2 つは独立した網）。

##### 結果 — 19 箇所が「義務なし 6 / 未実装の義務 11」に分離された

`#[must_use]` を付けた時点でコンパイラが 4 箇所を追加で指摘した（`block_return` /
`loop_yield` / `yield` / `if` 式の条件）。**いずれも義務のある地点**で、
手で数えた 19 箇所には入っていたが「歩くだけ」と誤認しかけていた。

| 未実装の義務 | 箇所 | 閉じるタスク |
|---|---|---|
| 条件は `bool` | 4 | 5.5 |
| スライス境界は `int` | 3 | 5.2 |
| `case` パターン型 vs subject | 1 | 5.4 |
| `yield` vs 宣言 yield 型 | 1 | 5.2 |
| `loop_yield` vs `->list[T]` | 1 | 5.2 |
| `block_return` vs `->T` | 1 | 5.2 |

⚠ **挙動は不変**（`compare_outputs` の差分が 2.9 時点と同一・検体も 56% のまま）。
3.1 は「今ある検査を増やす」変更ではなく「**今後の義務忘れを構造的に見えるようにする**」変更。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo build` | **警告 0**（`#[must_use]` の指摘を全て解消） |
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` | FAIL 0 件 |
| `compare_outputs.ps1 -A <フェーズ1 前>` | 187/201（**2.9 時点と同一** ＝ 挙動不変の証拠） |
| `stale_doc_refs.ps1` | OK |
| `compare_wasm_frontend.ps1` | **271/271 agreed・INVENTED 0** |

#### 3.4 の記録 【✅ 一部完了 2026-09-13】

整合性検査（3 分類）と**別系統**の妥当性検査を置いた。D-14 のとおり
「2 つの型が適合するか」ではなく「**名前が在るか・個数が合うか・定義自身が成り立つか**」を見る。

| 追加 | 内容 | 検体 |
|---|---|---|
| `check_guard_type_exists` | 型ガード（`is T`）の型名の存在検査。クラス・protocol・trait・プリミティブを許し、**テンプレート型変数は除外**（正当なので） | `X6` |
| `Stmt::EnumDef` の検査 | バリアント値が `int` であることを**静的に**検査 | `N1` `N2` |
| `TypeErrorKind::UnknownGuardType` / `EnumVariantNotInt` | 妥当性検査用のエラー種別（コメントで整合性検査と別系統であることを明記） | — |
| `TypeRegistry::is_known_trait` | trait 名の判定述語（メンバー 0 個の trait も拾うため 2 つの表を見る） | — |

⚠ `X6` を検査しないと**二重に見逃す**。腕が永久に死ぬうえ、腕の中では対象が存在しない
クラスへ絞り込まれるので**メンバーアクセスが全て無検査**になる（実測）。

⚠ `N1`（最上位の `enum`）は `RUNTIME` → `STATIC` に昇格した。実行時の
`build_enum_classes` の検査も残っているが、そちらは**定義が実行される経路**しか見られない。
既存例題 `typing/enum_in_function_error.ar` のコメントを実態に合わせて更新した。

#### 残した分（M1 / M2 — メンバーの存在検査）

⚠ **`M1`（存在しないフィールドの読み）と `M2`（存在しないメソッドの呼び出し）は
実行時のまま残した。** 静的化には次を全部正しく除外する必要があり、1 つ誤ると
**正しいコードを落とす**:

- メソッドは `class_field_details` ではなく `class_method_sigs` に在る（「フィールドに無い」≠「存在しない」）
- `Any` / `Unresolved` / `Namespace` / `PyNamespace` / `CsObject` 等の**動的メンバー**
- `import[py]` のモジュールメンバー（`editor` ビルドでは本体が空）
- `new_type` ラッパ・`enum_item_*` の内部名

⇒ 義務としては残し、フェーズ 5 で単独タスクとして扱う方が安全。

**検体**: `X6` `N1` `N2` が前進。静的検査の割合 56% → **59%**（67/114）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 772 passed / 0 failed |
| `scan_examples.ps1` / `force_gate.ps1` | FAIL 0 件 / 0 fall back |
| `compare_python_impl.ps1` | 78/78 identical・stale 0 |
| `compare_outputs.ps1 -A <フェーズ1 前>` | 187/204。差分 17 件は新規例題 15 件＋既知の既存 2 件のみ（うち `enum_in_function_error.ar` は実行時→静的の昇格） |
| `compare_wasm_frontend.ps1` | **274/274 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/validity_checks.ar` ／ `validity_checks_error.ar` ／ `validity_checks_enum_error.ar`

### フェーズ 4 — 関係の規則を正す

| # | 内容 | 決定 | 依存 | 備考 |
|---|---|---|---|---|
| ~~**4.1**~~ | ~~アップキャストのみに限定 ＋ `⊥` の導入~~ → **✅ 完了 2026-09-13** | D-3 / D-11 | — | 下記「4.1 の記録」。⚠ 3.3 より先に実施した（§3 の注記） |
| ~~**4.2**~~ | ~~暗黙 `int → float` を廃止~~ → **✅ 完了 2026-09-13**（2.4 と同時） | D-5 | — | 下記「2.4 / 4.2 の記録」 |
| ~~**4.3**~~ | ~~`__cast__[T]` による受理をやめ `=>` を強制する~~ → **✅ 完了 2026-09-13**（**受理地点を絞る**形に変更） | D-7 | 3.3 | 下記「4.3 の記録」。`K6` → **STATIC**。「型検査が嘘をつく」2 件目の解消 |
| ~~**4.4**~~ | ~~演算子を Kind 3 で検査し、結果型を返す~~ → **✅ 完了 2026-09-13** | D-9 | — | `O1` `O2` `Z1` → **STATIC**／`O4` `O5` `O7` `O8` は**仕様**と判明 |
| ~~**4.5**~~ | ~~`function` 型の分散（引数反変・戻り値共変）~~ → **✅ 2.2 に吸収して完了** | D-13 | — | ⚠ **2.2 と分離できなかった**（下記 2.2 の記録） |

#### 3.3 の記録 【✅ 完了 2026-09-13・4.1 / 4.4 の後に実施】

##### 「3 分類の入口」は地点ではなく**期待型の種類**で決まる

実装して分かった構造: **Kind は検査地点が選ぶものではない。** 地点は期待型を渡すだけで、
Kind 1（同一 or 自明キャスト）か Kind 2（アップキャスト）かは**期待型の種類**が決める。
⇒ だから入口は 1 つで足りる。Kind 3 だけは被演算子が 2 つで結果型も返すため別系統。

| Kind | 期待型 | 実体 |
|---|---|---|
| **1** | プリミティブ・コレクション・クラス・`new_type`・`enum`・テンプレート実体・`function` | `type_matches` / `type_matches_exact` |
| **2** | `trait` / `protocol` / `Intersection` | `satisfies_protocol` / `class_implements_trait` / `Intersection` アーム |
| **3** | （演算子） | `check_binop` / `infer_binop_result` / `check_arith`（別系統） |

##### 実測で見つけた「同じ問いの 2 つの実装」

⚠⚠ **`mut` 引数なら自明キャストを許さない**という規則が **2 箇所に別々に書かれていた**
（`check_expected` と `param_type_matches`）。同じ問いに 2 実装がある状態で、
片方だけ直せばずれる。
⚠⚠ さらに **`resolve_protocols` を呼ぶ経路と呼ばない経路が混在**していた。これが
タスク 1.3 のバグの正体（`let` 経路だけが `type_matches` を直接呼んでいた）で、
1.3 では手で 2 行足して直したが**構造は直っていなかった**。

##### 実装

| 追加・変更 | 内容 |
|---|---|
| `Aliasing` enum（新設） | `ByValue` / `WriteBack`。`param_mutable: bool` を引き回していたのを名前付きにした（呼び出し側で `true`/`false` の意味が読めなかった） |
| `types_compatible`（新設） | **整合性検査の唯一の述語**。`resolve_protocols` を**両側**に掛け、`Aliasing` で厳しさを切り替える |
| `check_expected` | 述語のラッパへ（protocol 適合の**報告**だけを担う） |
| `param_type_matches` | 述語へ委譲（重複した規則を削除） |
| `resolve_declared_type` / 可変長引数 / 関数型呼び出し | 述語へ routing |

##### 結果 — 入口の迂回が 0 件になった

| 計測 | 着手前 | 完了後 |
|---|---|---|
| `type_matches` / `type_matches_exact` の**外部**直接呼び出し | **4 グループ**（`let` 束縛・可変長 2 箇所・関数型呼び出し 2 箇所・`param_type_matches`） | **0 件** |
| `types_compatible` の利用者 | — | 9 箇所 |

⇒ **新しい検査地点は述語を呼ぶだけでよく、1.3 と同じ忘れ方ができない。**

##### ⚠ 挙動は不変（文言 1 件のみ変化）

`compare_outputs` の差分が 20 → 19 件になった。消えた 1 件は
`classes/protocol_in_container_error.ar` で、**エラー文言が
`list[protocol HasN]` → `list[HasN]`（利用者が書いた綴り）に戻った**ため。
⚠ **検査結果は完全に不変**（`K11` `K12` `P1` `P2` すべて `STATIC`・構造的適合の偽陽性も無し）。
`protocol` の解決が述語の内側へ移り、エラー報告は外側の未解決な注釈を使うようになった
（利用者の綴りをそのまま出す方が診断として素直）。例題のコメントを更新した。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 773 passed / 0 failed |
| `scan_examples.ps1` / `force_gate.ps1` | FAIL 0 件 / 0 fall back |
| `compare_python_impl.ps1` | 78/78 identical・stale 0 |
| `compare_outputs.ps1 -A <フェーズ1 前>` | 189/208（差分 19 件＝文言 1 件が基準と一致したぶん減） |
| `stale_doc_refs.ps1` | OK（⚠ 4.4 のコメント内 `abab` が識別子と誤検知されたのでバッククォートを外した） |
| `compare_wasm_frontend.ps1` | **278/278 agreed・INVENTED 0** |

#### 2.4 / 4.2 の記録 【✅ 完了 2026-09-13・**同時に入れた**】

##### 2.4 — 名前を占有せずに解いた

⚠ 既存の `builtin_fns` 機構（`range` だけ登録済み）に `len` 等を足す案は
**グローバル名を占有して `let len = ...` が already declared になる**ため前任者が
見送っており、**実際に例題が `len` を変数名に使っている**（実測 3 箇所）。

⇒ 「**型値を呼ぶとその型になる**」という一般規則を入れた。`int` / `float` / `str` / `bool` は
**既に `TypeValOf` として登録済み**なので、新しい名前を 1 つも占有せずに
`float(n) -> float` / `int(x) -> int` / `str(x) -> str` が型付く。
⚠ プリミティブに限る（`TypeValOf(NamedInstance(..))` はクラス・enum・protocol で呼び出しの
意味が違う）。⚠ `len` は名前占有の問題が残るので**見送った**（検体 `Z10` は `NONE` のまま）。

⚠ Arrow の組み込み関数は実測で**小さい**（`len` / `print` / `range` / `repr` / `id` /
`enumerate` / `zip` / `next` / `open` / `close` / `getenv` / `parse_ar` / `flat_*`）。
`abs` / `round` / `sum` / `min` / `max` / `sorted` / `type` / `ord` / `chr` は**存在しない**。
⚠ `range` は実行時にジェネレータを返すので `list[int]` と型付けるのは嘘になる
（`InferredType` にジェネレータが無い）。既存の `builtin_fns` 登録は `for` の要素型を
取るためのもので、そのままにした。

##### 4.2 — 暗黙 `int → float` の廃止

`type_matches` から拡大のアームを外した。⚠ これで `type_matches` と
`type_matches_exact` は**同じ判定になった**が、名前は残した（将来
`fixed_list → list` のような自明キャストを入れるときの置き場所。そのときは
**変換を挿入する地点を併せて定める**こと＝D-7）。
⚠ `Aliasing::WriteBack` の例外（`mut` 引数では拡大を許さない）も**実質不要になった**が、
同じ理由で枠は残してある。

##### ⚠⚠ 廃止して初めて分かったこと — 注釈の誤りが 1 件露見した

`examples/interop/cpp_default_arg_native_call.ar` の
`fn touch(let _unused: float = norm(v345, m)) -> float:` は**注釈が誤っていた**。
C 側の宣言は `int v3_norm(const V3* v, double* out_len)` で**戻り値は `int`**
（長さは `out_len` へ書き戻される）。**暗黙拡大がそれを隠していた。**
⇒ `let _unused: int` に直した。

⇒ 事前の見積り「移行コストは既存例題 1 件の 1 行」は正しかったが、その 1 行は
「書き換えが必要な正しいコード」ではなく「**隠れていた誤り**」だった。

##### 移行した例題（9 件）

| 例題 | 対応 |
|---|---|
| `interop/cpp_default_arg_native_call.ar` | **注釈の誤りを修正**（`float` → `int`） |
| `typing/int_float_widening.ar` | **中身を入れ替えた**。「暗黙拡大の実演」→「明示変換 `float(n)` の実演」 |
| `typing/int_float_widening_error.ar` | 説明を更新（2 つのエラーは廃止の前後で**変わらず**出るので検体としては有効） |
| 残り 7 件（`default_value_type` / `field_type` / `field_type_runtime` / `generic_type_ann` / `return_type` / `template_type_param` / `var_annotation`） | 該当箇所を `float(...)` へ |

**検体**: `Z9` `Z11` `Z12` が `NONE` → **`STATIC`**。静的検査の割合 61% → **64%**（73/114）。

##### ⚠ 副産物 — `$knownDiff` が 5 件 stale になった

暗黙昇格を廃止したことで **impl_python との差分の原因そのものが消えた**
（`int_float_widening` / `return_type` / `var_annotation` / `field_type_runtime` /
`default_value_type`）。⇒ 登録を削除した。
`compare_python_impl` の**検査対象が 78 → 83 件に増え**、すべて一致している。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 773 passed / 0 failed |
| `scan_examples.ps1` / `force_gate.ps1` | FAIL 0 件（**9 件の移行後**）/ 0 fall back |
| `compare_python_impl.ps1` | **83/83 identical**・stale 0（対象が 5 件増えた） |
| `compare_outputs.ps1 -A <フェーズ1 前>` | 189/208（差分 19 件は 3.3 時点と同数） |
| `compare_wasm_frontend.ps1` | **278/278 agreed・INVENTED 0** |

#### 4.1 の記録 【✅ 完了 2026-09-13・**根本原因②の実装本体を撤去**】

⚠ **3.3 より先に実施した。** 3.3（3 分類の単一定義）は挙動不変のリファクタリングで検体が
動かない一方、4.1 は規則そのものを正す。「出来上がった判定を 3.3 で束ねる」方が筋が通るため
順序を 4.1 → 4.4 → 3.3 に変えた（2.2 が 4.5 を吸収したのと同じ判断）。

| 層 | 内容 |
|---|---|
| `InferredType::Never` | **下端型（⊥）** を新設。あらゆる型へアップキャスト可。⚠ 注釈としては書けない（推論の内部表現） |
| 空リテラル | `[]` → `ListOf(Never)` ／ `{}` → `DictOf(Never, Never)` ／ 空 set → `SetOf(Never)` |
| `type_matches_exact` | `Never` はあらゆる型へ適合。**素の容器 → 型引数つき容器の 4 つの規則を撤去**（`List`→`ListOf` / `FixedList`→`FixedListOf` / `Set`→`SetOf` / `Dict`→`DictOf`）。`ListLikeOf` の `is_none_or` も同様に締めた |
| `open_never_to_any`（新設） | **注釈なしの束縛**では `Never` を `Any` へ開く（D-11 / U-6）。`Never` のまま束縛すると「要素を足せない空リスト」になる |

⚠⚠ **`Any` では駄目で `⊥` が必要だったことを実測で確認した。** `Any` は要素型の**上端**なので
`list[Any]` は `list[int]` の**スーパータイプ**。`[] : list[Any]` にすると
`mut xs: list[int] = []` がダウンキャストになって落ちる。

##### 仕様変更の帰結（2 件・どちらも実測で判明）

| 影響 | 内容 | 対応 |
|---|---|---|
| 既存例題 `collections/store_copy_semantics.ar` | `mut lst = []` が `list[Any]` になり `lst[0]` が `Any` → **`Any` への属性アクセスは明示ダウンキャストが必要**（既存の規則）という別のエラーになった | 方針どおり**例題側を直した**（`mut lst: list[list[int]] = []` と注釈を付ける） |
| 既存テスト `untyped_list_matches_list_of_any_ok` | `let xs = []` を `list[int]` の仮引数へ渡すテスト。`list[Any]` → `list[int]` はダウンキャストなので**落ちる** | 仕様に合わせて `untyped_empty_list_to_list_of_int_err` に改め、**注釈版が通る**テスト（`annotated_empty_list_to_list_of_int_ok`）を追加した |

⚠⚠ **U-6（注釈なしは `list[Any]`）には「渡せなくなる」コストがある。**
`let xs = []` は `list[Any]` なので `list[int]` を要求する場所へ渡せない（注釈が必要）。
`list[⊥]` のまま束縛すれば渡せるが「要素を足せない空リスト」になる。決定どおり `Any` を採ったが、
**この帰結は仕様として明記しておく必要がある**（例題 `upcast_only.ar` に書いた）。

⚠ 検体 `L11`（空コレクションの注釈必須）は **`NONE` のままが正しい**。U-6 の決定で
「注釈なしは `list[Any]`」と決めたのでエラーではない。検体に「仕様どおり」と明記した。

##### 検体は動かない（期待どおり）

4.1 は規則の**締め直し**で、これに依存していた検体（`L1` `L15`〜`L19`）は**タスク 2.7 で
既に閉じている**。4.1 の価値は「根本原因②の実装本体を撤去したこと」と
「`⊥` で `= []` を成立させたこと」。静的検査の割合は **59% のまま**。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | **773 passed**（テストを 1 件追加）/ 0 failed |
| `scan_examples.ps1` / `force_gate.ps1` | FAIL 0 件 / 0 fall back |
| `compare_python_impl.ps1` | 78/78 identical・stale 0 |
| `compare_outputs.ps1 -A <フェーズ1 前>` | 188/206。差分 18 件は新規例題 16 件＋既知の既存 2 件のみ |
| `compare_wasm_frontend.ps1` | **276/276 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/upcast_only{,_error}.ar`

#### 4.4 の記録 【✅ 完了 2026-09-13】

##### 可否と結果型を 1 つの表で決めた（D-9）

判定は `infer_binop_result` が `Unresolved` を返すかで行う。**2 つの表を持つとずれる**ので
1 本に寄せた。⚠⚠ **そのため表の取りこぼしが偽陽性に直結する。** 実行時の挙動を
1 組み合わせずつ実測して表を作り直した:

| 演算子 | 以前の誤り | 実測に基づく修正 |
|---|---|---|
| `Add` | list / tuple の連結を知らない | `[1] + [2]` / `(1,) + (2,)` を追加（要素型は join） |
| `Mul` | `str * int` / `list * int` を知らない | 追加（`"a" * 1.5` は不可のまま） |
| `Mod` | `str % 任意`（書式化）を知らない | 追加（`1.5 % 2` は不可） |
| `Div` | **無条件 `Float`** | 数値同士のみに（`"a" / 2` を弾けるようになった） |
| ビット演算 | **無条件 `Int`** | `int` 同士のみに（`1.5 & 2` / `"a" & 1` を弾ける） |
| `FloorDiv` | — | `int` 同士のみ（`1.5 // 2` は実行時も不可） |

⚠ **クラスは素通し**（`__add__` 等で演算を定義できる）。`Unresolved` / `Never` も素通し。

##### 偽陽性 2 件を実測で見つけて直した

| 例題 | 原因 | 対応 |
|---|---|---|
| `collections/collection.ar` | **set 同士の分岐が素の `Set` しか見ていなかった**。タスク 2.7 以降 set リテラルは `SetOf(T)` に推論されるので分岐を外れ、ビット演算の表へ落ちて偽エラーになった | 分岐を `SetOf` にも対応させ、要素型も返すようにした（`\|` `^` は join・`&` `-` は左辺の要素型） |
| `exceptions/try_except.ar` | `let x = "hello" + 42` と**直接書いて実行時 `TypeError` を捕まえる**例題。静的エラーになって書けなくなった | 静的に型が決まらない経路（要素型なしの `list` の要素）から同じ演算に到達させるよう直した。例題の意図（実行時 `TypeError` の捕捉）は保たれている |

##### ⚠⚠ `==` / `!=` / `in` の厳密化は**撤回した**（仕様だった）

一度「`<` は厳密なのに `==` は無検査という非対称」を直す実装を入れたが、
**`examples/basics/equality_numeric_promotion.ar`（B2-b）が仕様として固定していた**:

- 等値は「**値の同一性**（型厳密・辞書のキーとハッシュが使う）」と
  「**式としての比較**（`uint → int → float` の一方向ラティスで昇格してから比べる）」の二層
- **`bool` は昇格ラティスの対象外**（`1 == True` は `False`。Python と違う）
- `1 == "a"` / `1 == None` / `[1] == [1.0]` はいずれも **`False`**（エラーではない）

非対称は**意図された設計**で、「異型に意味のある答えが無い」（順序比較）対
「常に意味のある答えがある」（等値）の区別だった。
⇒ 検体 `O4` `O5` `O8` は「塞ぐべき穴」ではなく**仕様**。検体に明記した（`O7` も同様）。

⚠ **棚卸しの分類を訂正する。** 最初の棚卸しで `O4` / `O5` / `O8` を「比較演算子が非対称＝穴」と
書いたのは誤りだった。既存例題が仕様として固定していることを見落としていた。

##### 結果

**検体**: `O1` `O2` `Z1` が `RUNTIME` → **`STATIC`**。静的検査の割合 59% → **61%**（70/114）。
⚠ 既存例題 `collections/list_concat_repeat_error.ar` も実行時 → 静的へ昇格した（コメントを更新）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo build` | 警告 0 |
| `cargo test --release` | 773 passed / 0 failed |
| `scan_examples.ps1` / `force_gate.ps1` | FAIL 0 件 / 0 fall back |
| `compare_python_impl.ps1` | 78/78 identical・stale 0 |
| `compare_outputs.ps1 -A <フェーズ1 前>` | 188/208。差分 20 件は新規例題 17 件＋既知の既存 3 件のみ |
| `compare_wasm_frontend.ps1` | **278/278 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/arith_operand_check{,_error}.ar`

#### 4.3 の記録 【✅ 完了 2026-09-13・⚠ **起票時の方針を実測で変更した**】

##### ⚠⚠ 「受理をやめる」では例題が壊れた ⇒ 実測してから「地点を絞る」に変えた

起票時の内容は「`__cast__[T]` による受理を**やめて** `=>` を強制する」だった。
実際に `type_matches_exact` から受理を落とすと検体 `K6` は `NONE` → `STATIC` になり
`cargo test` も 773 件通ったが、**`scan_examples` が `examples/typing/other_typing.ar` で落ちた**:

```
argument 0 of 'double' expects 'int' but got 'Wrapper'
```

該当は「`=== cast: auto-cast for let parameters ===`」という節で、`double(w)` が
**正しく `42` を出していた**。つまり「受理したのに変換していない」のではなく、
**実行時が実際に変換している地点**があった。

##### どの地点で実行時が変換するのかを 1 つずつ実測した

フェーズ 1 前のバイナリ（`arrow_p0.exe`）に同じクラスを地点だけ変えて食わせた:

| 地点 | 実行時の挙動 |
|---|---|
| **`let` 仮引数** | `param: 42` — **変換する** |
| 変数束縛 | `var: <W object at …>` — 変換しない |
| フィールド代入 | `Error: value does not match declared type of field …`（A-3 の実行時検査が弾く） |
| 戻り値 | `ret: <W object at …>` — 変換しない |
| `mut` 仮引数 | `Error: unsupported operand types for Mul` — 変換しない |

実装は `src/interpreter/functions/execution.rs` にあり、コメントも一致していた:

> 自動キャスト: `let` パラメータに型アノテーションがあり、渡された値がインスタンスで
> かつ型が異なる場合、`__cast__[TypeName]` メソッドが定義されていれば自動的にキャストする。
> **`mut` パラメータは自動キャストしない。**

⇒ **バグは「受理そのもの」ではなく「地点を問わず受理していたこと」**だった。
D-7 の原則（**受理したなら変換地点がある**）は `let` 仮引数では守られていて、
それ以外の 4 地点で破れていた。

##### `Aliasing` を `Site`（地点）へ拡張した

タスク 3.3 で入れた `Aliasing { ByValue, WriteBack }` は、4.2 で暗黙 `int → float` を
廃止した結果**振る舞いの差が無くなっていた**（両アームが同じ判定に落ちる）。
そこへ「ユーザー定義キャストを許すか」という 2 つ目の軸が来たので、
**同じ場所に 2 軸を持つ 1 つの型**として書き直した:

```rust
pub(super) enum Site {
    LetParam,   // `let` 仮引数への束縛
    MutParam,   // `mut` 仮引数への束縛（書き戻しあり）
    Other,      // 変数束縛・フィールド代入・戻り値・要素型 …
}
```

| 地点 | 自明キャスト | ユーザー定義キャスト（`__cast__`） |
|---|---|---|
| `LetParam` | 許す | **許す** |
| `MutParam` | 許さない（書き戻し） | 許さない |
| `Other` | 許す | **許さない** |

受理の判定は `type_matches_exact`（**地点を知らない**述語）から
`types_compatible`（地点を受け取る唯一の入口・3.3 で作った）へ移した。
⚠ これは 3.3 の効果でもある: **入口が 1 つだったから軸を 1 箇所足すだけで済んだ。**

##### ⚠ `check_expected` の既定は `Other` にした

`check_expected` は仮引数以外（フィールド既定値・再代入・戻り値）からも呼ばれるので、
`param_mutable == false` を `LetParam` と読み替えてはいけない。**`Other` に倒した**。
仮引数の検査は `param_type_matches` が明示的に `Site::LetParam` を渡す。

##### 何が塞がったか

```
class Conv:
    mut n: int
    fn __cast__[int](self) -> int:
        return self.n
let x: int = Conv(5)
print(x)        # 旧: <Conv object at 0x...>   ← int 変数に Conv が居座る
print(x + 1)    # 旧: TypeError: unsupported operand types for `Add`
```

⇒ 静的エラー `'x' is declared 'int' but initialized with 'Conv'` になった。
容器経由の漏れ（`list[int]` に `Conv` が入る）も同時に塞がった。

##### 結果

**検体**: `K6` が `NONE` → **`STATIC`**。静的検査の割合は **65%（74/114）で変わらず**
（4.2 の時点で K6 以外は先に閉じていたため、内訳の `NONE` が 20 → 19 に減った）。

⇒ **「型検査が嘘をつく」2 件が両方とも解消**（1 件目は `and` / `or` ＝ 2.5）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo build --release` | 警告 0 |
| `cargo test --release` | 773 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 fall back |
| `compare_python_impl.ps1` | 84/84 identical・stale 0（新規 `user_cast_site_error` を `$knownDiff` に登録） |
| `compare_outputs.ps1 -A 08bb127` | 209/210。差分は**新規例題 1 件のみ**（旧バイナリは黙って実行し `ここには到達しない` を出す＝この検査が無かった証拠） |
| `stale_doc_refs.ps1` | OK |
| `compare_wasm_frontend.ps1` | **280/280 agreed・INVENTED 0** |
| `make-vsix.ps1` | VSIX 再生成済み |

**追加した例題**: `examples/typing/user_cast_site{,_error}.ar`

### フェーズ 5 — 義務を全地点へ行き渡らせる

| # | 内容 | 決定 | 依存 | 検体 |
|---|---|---|---|---|
| ~~**5.1**~~ | ~~複合代入の二段検査~~ → **✅ 完了 2026-09-14** | D-8 / D-9 | 4.4 | 下記「5.1 の記録」。`B6` `B10` `B11` `F2` が**全て STATIC** |
| ~~**5.2**~~ | ~~未検査の地点を埋める~~ → **✅ 完了 2026-09-14**（5.2a〜5.2d） | D-1 | 3.2 | 検体 12 件が**全て STATIC** |
| ~~5.2a~~ | ~~`block_return` / `loop_yield` / `yield`~~ → **✅ 完了 2026-09-14** | D-1 | 3.2 | 下記「5.2a の記録」。`X1`〜`X4` `C8` が**全て STATIC** |
| ~~5.2b~~ | ~~添字（代入の要素型・index の型・スライス境界）~~ → **✅ 完了 2026-09-14** | D-1 | 3.2 | 下記「5.2b の記録」。`L5`〜`L8` が**全て STATIC** |
| ~~5.2c~~ | ~~組み込みメソッド引数・可変長要素~~ → **✅ 完了 2026-09-14** | D-1 | 3.2 | 下記「5.2c の記録」。`L14` `C15` が**両方 STATIC** |
| ~~5.2d~~ | ~~trait フィールド代入の静的化~~ → **✅ 完了 2026-09-14** | D-1 | 3.2 | 下記「5.2d の記録」。`F3` → **STATIC**。⇒ **5.2 完了** |
| ~~**5.3**~~ | ~~`static mut` クラス変数への代入を検査~~ → **✅ 完了 2026-09-14** | D-1 | 3.2・1.2 | 下記「5.3 の記録」。`F4` → **STATIC** |
| ~~**5.4**~~ | ~~`match` の `case` パターン型を subject と照合~~ → **✅ 完了 2026-09-14** | D-1 | 3.2 | 下記「5.4 の記録」。`X5` → **STATIC** |
| ~~**5.5**~~ | ~~`if` / `while` の条件を `bool` 厳密に~~ → **✅ 完了 2026-09-14** | D-12 | **2.5** | 下記「5.5 の記録」。`K1` `K2` → **STATIC**。移行は予測どおり例題 1 箇所 |
| ~~**5.6**~~ | ~~`new_type` のコンストラクタ引数を検査~~ → **✅ 完了 2026-09-14** | D-1 | 3.2 | 下記「5.6 の記録」。`T3` → **STATIC** |
| ~~**5.7**~~ | ~~オーバーロード解決を実引数型で行う~~ → **✅ 完了 2026-09-14** | D-1 | 3.2 | 下記「5.7 の記録」。`C12` → **STATIC**。⇒ **フェーズ 5 完了** |

#### 5.1 の記録 【✅ 完了 2026-09-14】

##### 複合代入は型検査を**一度も通っていなかった**

起票時の想定より穴が大きかった。地点ごとに理由が違う:

| 地点 | 以前の状態 | 理由 |
|---|---|---|
| `x += v` | `check_binop` を**呼んでいなかった** | 推論（`infer(value)`）と VM の型特化注釈だけを行い、検査へ渡していなかった |
| `o.f += v` | 検査を**丸ごと省いていた** | 0-2 のコメント「複合代入で格納されるのは `v` ではない」は**正しい**が、代わりの検査を置いていなかった |

⇒ `x + "s"` はタスク 4.4 で静的エラーになったのに、**`x += "s"` だけ実行時まで判らない**
状態だった（検体 `B6` / `F2` が `RUNTIME` だったのはこれ）。

##### 右辺を左辺型と直接照合すると**両方向にずれる**（D-8）

実測した反例:

| コード | 右辺を直接照合すると | 正しい判定 |
|---|---|---|
| `mut t: str = "a"; t *= 3` | ⛔ 誤って弾く（右辺は `int`） | ✅ 通す（`str * int` → `str`） |
| `mut fmt: str = "n=%d"; fmt %= 42` | ⛔ 誤って弾く | ✅ 通す（`str % 任意` は書式化） |
| `__add__` が `str` を返すクラスに `v += V(2)` | ✅ **通してしまう**（右辺は `V`） | ⛔ 弾く（結果が `str`） |

⇒ **検査 1**（Kind 3: `x <op> v` が可能か）→ 結果型 `R` → **検査 2**（Kind 1: `R` → `typeof(x)`）。
実装は `check_compound_assign` 1 本で、変数・フィールド両方がここを通る。

##### クラスの結果型は**宣言された戻り値型**を引く（D-9）

`infer_binop_result` は組み込みの表だけを見るので、クラスが左辺だと `Unresolved`＝判定不能を
返し、検査 2 がそこで止まる。`binop_result_type` を足して演算子メソッドの戻り値型を引いた。

⚠⚠ **ダンダー名は推測で書くと実行時とずれる。** `interpreter/ops/operators.rs` の表を読んで
写した: `Div` は `__truediv__`、`FloorDiv` は `__floordiv__`、ビット演算は
`__and__` / `__or__` / `__xor__` / `__lshift__` / `__rshift__`。
演算子名から素直に綴った名前はどれも実在しない。

⚠ オーバーロードが複数あるときは**実引数で選ぶ必要がある**（タスク 5.7）。それまでは
シグネチャが 1 本のときだけ戻り値型を使う（誤った分岐の戻り値で弾かないため）。

##### ⚠ 例題を書いていて見つけた既存の VM 穴（5.1 とは無関係）

`p::Scored.score += 5`（**trait 名で修飾した属性への複合代入**）は
`VmForceError: cannot compile top-level statement 'AttrCompoundAssign' to bytecode` になる。
関数の中でも同じ（`cannot compile function 'bump'`）。**HEAD（5.1 の前）で実測して同じ**なので
5.1 が壊したものではない。型検査は通るので、VM コンパイラ側の未対応。
⚠ 修飾しない `p.score += 5` は通る（例題はそちらを使った）。

##### 結果

**検体**: `B6` `F2` が `RUNTIME` → **`STATIC`**、`B10` `B11` が `NONE` → **`STATIC`**。
静的検査の割合 65% → **68%**（78/114）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo build --release` | 警告 0 |
| `cargo test --release` | 773 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 fall back（281 例題） |
| `compare_python_impl.ps1` | 84/84 identical・stale 0（`str %= int` 未実装と結果型検査なしの 2 件を `$knownDiff` に登録） |
| `compare_outputs.ps1 -A 0e7c986` | 211/212。差分は**新規例題 1 件のみ** |
| `stale_doc_refs.ps1` | OK |
| `compare_wasm_frontend.ps1` | **282/282 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/compound_assign_two_stage{,_error}.ar`

#### 5.2a の記録 【✅ 完了 2026-09-14】

##### 3 つとも「値を囲み構文へ渡す」同じ義務なのに、**別々に無検査**だった

| 構文 | 以前 | 検体 |
|---|---|---|
| `block_return` | 実行時のみ | `X1` `X2` `X4` |
| `loop_yield` | 実行時のみ | `X3` |
| `yield` | **無検査**（実行時も通り抜ける） | `C8` |

⇒ 照合先を 1 本のスタック（`block_expr_expected`）に、検査も 1 本
（`check_block_expr_value`）に寄せた。地点ごとに書き分けると
「`block_return` だけ直して `loop_yield` が漏れる」形になる。

##### ⚠⚠ スタックを 2 本に分けたら例題が**偽エラー**で落ちた

最初は「`loop_yield` は入れ子のブロック式を貫いて外側のループ式へ届く」と考えて
`block_return_expected` と `loop_yield_expected` を**別のスタック**にした。
`scan_examples` が `examples/basics/block_return_typecheck.ar` で落ちて誤りが判った:

```arrow
let f = for i in range(4) ->list[int]:
    let _ = if i == 1 ->int:
        loop_yield "not checked here"   # 内側の注釈は `int` で list[T] ではない
        block_return 0                  # ⇒ loop_yield は検査されない
    else:
        0
    loop_yield i
```

この例題（#35）が「**いちばん内側の注釈が効く**」を仕様として固定していた。
⇒ **1 本のスタック**にして、`loop_yield` は「内側の注釈が `list[T]` のときだけ」
要素型と照合する形に直した。

⚠ **既存例題が仕様を固定していることを、実装前に読んでいなかった。** 4.4 で
`equality_numeric_promotion.ar` に同じ形でつまずいたのと同じ失敗。

##### 積むのは「注釈が付いた式」だけ

- `Stmt::Block` / `Stmt::If` など**文**は積まない ⇒ 中の `block_return` は外側の注釈と
  照合される（例題 7・8 がこの形）。`break` が入れ子の `if`/`match`/`block:` を貫いて
  ループへ届くのと同じ扱い。
- **関数・`gen` の本体は `None` を積む**（`with_fn_body`）。継承すると入れ子 `fn` の
  `block_return` が外側のブロック式の注釈と照合されて偽エラーになる
  （`current_fn_return` / `in_gen_body` を張り替えているのと同じ理由）。

##### `gen` の `->T` を `current_fn_return` へ入れた

`yield` の照合先が無かったのは、`check_gen_def` が `enter_fn` を**呼んでいなかった**から。
`gen` の `->T` は「`yield` 1 回分の型」なので、`return` が使う場所と同じ
`current_fn_return` へ入れて共有した。

##### 結果

**検体**: `X1`〜`X4` が `RUNTIME` → **`STATIC`**、`C8` が `NONE` → **`STATIC`**。
静的検査の割合 68% → **73%**（83/114）。

⚠ 既存例題 `examples/basics/block_return_typecheck_error.ar` が実行時 → 静的へ昇格した
（「静的型検査はこの形を捕まえない」というコメントを更新）。実行時検査は**残してある**
（静的に型が決まらない経路はそちらが受け持つ）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 773 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 fall back（283 例題） |
| `compare_python_impl.ps1` | 84/84 identical・stale 0 |
| `compare_outputs.ps1 -A 3dabedf` | 212/214。差分 2 件は新規例題 1 件＋昇格した既存例題 1 件 |
| `stale_doc_refs.ps1` | OK |
| `compare_wasm_frontend.ps1` | **284/284 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/block_expr_value_static{,_error}.ar`

#### 5.2b の記録 【✅ 完了 2026-09-14】

##### 実測した実行時の規則（先に測ってから書いた）

| 容器 | 添字に許される型 | 外れたとき |
|---|---|---|
| `list` / `fixed_list` / `list_like` / `str` / `tuple` | `int` ・ `Index` ・ スライス | `TypeError` |
| `dict[K, V]` | **`K` と同じ型** | **`KeyError`**（型エラーですらない） |
| `set` | **無し**（添字アクセスできない） | `TypeError` |

⚠⚠ **`bool` と `float` は添字にできない**（`xs[True]` も `xs[1.5]` も `TypeError`）。
真偽値が整数として通る言語の癖で書くと落ちるので、静的に弾く価値がある。

⚠⚠ **`dict` のキーに数値昇格は効かない。** `dict[float, int]` を `int` のキーで引くと
`KeyError`（実測）。等値比較は `uint → int → float` で昇格する（4.4 で確認した B2-b の仕様）のに、
**辞書引きはハッシュなので昇格しない**。⇒ `==` と同じ規則で判定してはいけない。

##### 添字代入は**完全に無検査**だった（検体 `L5`）

実測:

```
mut xs: list[int] = [1,2];      xs[0] = "s"    ⇒ ['s', 2]
mut d: dict[str,int] = {"a":1}; d[1] = 2       ⇒ {'a': 1, 1: 2}
mut d: dict[str,int] = {"a":1}; d["b"] = "s"   ⇒ {'a': 1, 'b': 's'}
```

**実行時エラーにすらならない**ので、値が後で `+` などに流れて初めて壊れる。
⇒ `infer(target)` が返す要素型 / 値型をそのまま期待型に使って照合する
（添字そのものの型は `infer` の中で検査済みなので、容器を二度推論しないで済む）。

##### ⚠⚠ ついでに見つけた**推論のバグ**: スライス添字が要素型を返していた

```arrow
let xs: list[int] = [1,2,3]
let y: int = xs[0:2]        # 旧: 黙って通る。実行時には int 変数に [1, 2] が入る
```

`Expr::Subscript` は添字がスライスかどうかを見ずに `ListOf(T)` → `T` を返していた。
実測（`eval_subscript_slice`）: list→list ／ str→str ／ tuple→tuple ／
クラスは `__getitem__` へ委譲 ／ **それ以外（`fixed_list` を含む）は実行時 `TypeError`**。
⇒ スライス添字は「同じ種類の容器」を返すように直した。
⚠ tuple は**要素数が変わる**ので静的には決められず `Unresolved` に倒した。

##### ⚠ `set` の添字に**嘘の型**を与えていた

`SetOf(T)` の添字に `T` を返していたが、`set` は `'set' object is not subscriptable` で
**必ず**実行時エラーになる。診断（`NotSubscriptable`）を出し、型は `Unresolved` にした。

##### 偽陽性 1 件を `scan_examples` が見つけた

`collection.ar` の**スライス代入** `sl_a[Index(1):Index(3)] = [20, 30]` を
要素型と突き合わせて落ちた。右辺は「要素」ではなく**要素の並び**で、実測では
list / tuple / str のどれでも受け付ける（同じ例題が `(20, 30)` と `"abc"` を渡している）。
⇒ 添字がスライスのときは値の照合を**しない**（別の義務なので 5.2b では扱わない）。

##### 正しい検出 2 件（例題側を直した）

| 例題 | 何を書いていたか | 直し方 |
|---|---|---|
| `store_copy_semantics.ar` | `mut lst2 = [0]`（＝`list[int]`）に `list[int]` を代入 | `list[list[int]]` と注釈（同ファイルの `append` 節が 4.1 で既に同じ直し方をしている） |
| `equality_depth_limit_error.ar` | `mut b = [1, 2]` に `b[0] = b` | `list` と注釈（同ファイルの `x` / `y` と同じ意図） |

⚠ どちらも「**注釈が無い束縛にも推論型で同じ厳格さ**」の帰結で、緩めずに例題を直した。

##### 結果

**検体**: `L5` が `NONE` → **`STATIC`**、`L6` `L7` `L8` が `RUNTIME` → **`STATIC`**。
静的検査の割合 73% → **76%**（87/114）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 773 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 fall back |
| `compare_python_impl.ps1` | 84/84 identical・stale 0 |
| `compare_outputs.ps1 -A bb51540` | 214/216。差分は**新規例題 2 件のみ** |
| `stale_doc_refs.ps1` | OK |
| `compare_wasm_frontend.ps1` | **286/286 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/subscript_static{,_error}.ar`

#### 5.2c の記録 【✅ 完了 2026-09-14】

##### ⚠⚠ 可変長引数は「**全部まちがえると捕まるのに 1 つ混ぜると通る**」形をしていた

実測:

```arrow
fn f(let ...: int) -> int: ...
f(... = "a", "s")     # ⛔ 捕まる（全部 str ＝ list[str] に推論される）
f(... = 1, "s")       # ✅ 通っていた
```

原因は推論側だった。可変長引数の型を「**全要素が同じ型のときだけ `list[T]`、混ざったら素の
`list`**」と決めていて、素の `list` は下流の検査（`Some((_, IT::ListOf(elem_ty)))` で受ける）に
**当たらない**。⇒ **根本原因②「要素型を捨てると何とでも適合する」がここに残っていた。**
コレクションリテラルと同じ合成（`join_elem_types`・タスク 2.7）に寄せて閉じた。

⚠ 「実際のコードほど捕まらない」（混ざるのが普通）ので、検体の `NONE` は
**穴の大きさを過小評価していた**。

##### 組み込みメソッドは**実装側の一覧を読んでから**書いた

対象は「要素を 1 つ受け取るメソッド」だけ:

| レシーバ | メソッド |
|---|---|
| `list` / `fixed_list` / `list_like` | `append` |
| `set` | `add` ・ `discard` ・ `remove` |

⚠⚠ **推測で足すと死んだ枝になる。** 実測すると `insert` / `extend` / `remove` / `index` /
`count`（list）・`get`（dict）は**存在しない**
（`AttributeError: 'list' object has no method 'insert'`）。Python の知識で書くと外す。

⚠ `union` / `intersection` 等は**集合そのもの**を受け取るので対象外
（要素型と突き合わせると偽エラーになる）。

⚠ 弾かなかった場合に何が起きるかは地点で違う（実測）: `list.append` と `set.add` は
**黙って異型を入れる**（`[1, 's']` / `{1, 's'}`）、`set.discard` は黙って何もしない、
`set.remove` は `KeyError`。どれも「要素型が守られない」ことに変わりはない。

##### 番兵値がエラー文言に漏れていた

可変長の検査が効くようになって初めて `argument 18446744073709551615 of 'count_all'` が
表に出た（`param_index: usize::MAX` が可変長を表す番兵）。
`the variadic argument of ...` に直した。⚠ **検査が動いていなかったので誰も気づけなかった**
形のバグで、検体を STATIC にした副産物。

##### 正しい検出 3 件（例題側を直した）

| 例題 | 何を書いていたか | 直し方 |
|---|---|---|
| `store_copy_semantics.ar` | `mut z = [1]` に `z.append(z)`、`mut s = {0}` に `s.add(c)` | `list` / `set[list[int]]` と注釈 |
| `equality_depth_limit_error.ar` | `mut a = [1]` に `a.append(a)` | `list` と注釈 |
| `fixed_list_error.ar` | — | **実行時 → 静的へ昇格**（コメント更新） |

##### 結果

**検体**: `L14` `C15` が `NONE` → **`STATIC`**。静的検査の割合 76% → **78%**（89/114）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 773 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 fall back（285 例題） |
| `compare_python_impl.ps1` | 84/84 identical・stale 0 |
| `compare_outputs.ps1 -A 656c335` | 216/218。差分は新規例題 1 件＋昇格した既存例題 1 件 |
| `stale_doc_refs.ps1` | OK |
| `compare_wasm_frontend.ps1` | **288/288 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/collection_method_args{,_error}.ar`

#### 5.2d の記録 【✅ 完了 2026-09-14・⇒ **5.2 完了**】

##### `o::T.f` は**型を返していなかった**

`Expr::TraitAccess` の推論が `Unresolved` を返していた。`Unresolved` は
「何でも通る」期待型なので、読みの型も判らず、代入は**丸ごと無検査**だった。
止めていたのは A-3 で入れた実行時検査（`store_field`）だけ。
宣言は `trait_field_details` に揃っているので引くようにした。

⚠ メソッド（`o::T.m()`）はここでは解決していない。呼び出しの検査は `infer_call` の
別経路で、そちらを変えると影響範囲が広い。

##### ⚠⚠ 暗黙のコンストラクタが**この経路を通っていた**

`scan_examples` が `trait_conformance.ar` で落ちて分かった:

```
field 'item' of class 'Holder' is declared 'T' but got 'int'
```

例題に `::` は 1 つも書かれていない。**trait のフィールドを持つクラスの `C(...)` が
内部で `self::T.f = arg` の形で代入している**ため、検査がコンストラクタ引数にも
効いていた（カバー範囲としては良い。ただし下の guard が要る）。

##### `mentions_type_param` は**ここでは効かない**

`trait Holder[T]: mut item: T` を `class IntBox(Holder[int])` が実装するとき、
期待型は `T` のままで、具体型引数（`int`）は**基底名 `Holder` だけでは判らない**。
`mentions_type_param` は「**いま見えている**型変数」しか見ず、`IntBox` の中からは
`Holder` の `T` は見えていないので素通りしていた。
⇒ レジストリから trait の**テンプレート引数名**を引いて突き合わせる
`mentions_trait_param` を足した。`trait_conformance.ar` が
「型変数を含む要求は照合を見送る」と明記している方針そのもの。

⚠ 型変数を**含まない**要求（`trait Counted[T]: mut count: int`）はテンプレート trait でも
検査される（例題で固定した）。

##### 結果

**検体**: `F3` が `RUNTIME` → **`STATIC`**。静的検査の割合 78% → **79%**（90/114）。
⇒ **タスク 5.2 の検体 12 件（`L5`〜`L8` `L14` `C8` `C15` `X1`〜`X4` `F3`）が全て STATIC。**

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 773 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 fall back（289 例題） |
| `compare_python_impl.ps1` | 85/85 identical・stale 0 |
| `compare_outputs.ps1 -A 8995433` | 219/220。差分は**新規例題 1 件のみ** |
| `stale_doc_refs.ps1` | OK |
| `compare_wasm_frontend.ps1` | **290/290 agreed・INVENTED 0** |

**追加した例題**: `examples/classes/trait_field_assign_static{,_error}.ar`

#### 5.3 の記録 【✅ 完了 2026-09-14】

##### **同じフィールドなのに経路で扱いが割れていた**

実測:

| 書き方 | 以前 |
|---|---|
| `c.n = "s"`（インスタンス経由） | ⛔ 静的に弾かれる |
| `C.n = "s"`（クラス名経由） | ✅ **黙って通り `s` を出す** |

⚠ 期待型の計算は**合っていた**。`let x: str = C.n` は以前からちゃんと弾かれる
（`infer_attr` が `int` を返している）。穴は**受け手の解決**で、
`class_and_subst` が `NamedInstance` / `GenericInstance` しか知らず、
クラス名そのものの型（`TypeValOf(NamedInstance(C))`）には `None` を返していた。
呼び出し側はそこで `return` していたので、**照合に到達する前に抜けていた**。

⇒ 受け手の解決を `receiver_class` に切り出し、`TypeValOf` を剥がす経路と
「変数として引けないクラス名」の経路を足した。

⚠ `static let` は存在しない（`ParseError: expected 'fn' or 'mut' after 'static'`）ので、
可変性の検査は要らない。

##### 結果

**検体**: `F4` が `NONE` → **`STATIC`**。静的検査の割合 79% → **80%**（91/114）。

⚠ ゲート実行中に `cargo test` が 1 度だけ `772 passed; 1 failed` を出したが、
**続けて 3 回走らせて再現しなかった**（773 passed）。ビルド直後の実行だったので
一時ファイルの競合と見ている。落ちたテスト名は取れていない。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 773 passed / 0 failed（上記のとおり 1 度だけ再現しない失敗あり） |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 fall back（291 例題） |
| `compare_python_impl.ps1` | 86/86 identical・stale 0 |
| `compare_outputs.ps1 -A 1d0ea51` | 221/222。差分は**新規例題 1 件のみ** |
| `stale_doc_refs.ps1` | OK |
| `compare_wasm_frontend.ps1` | **292/292 agreed・INVENTED 0** |

**追加した例題**: `examples/classes/static_mut_class_assign{,_error}.ar`

#### 5.4 の記録 【✅ 完了 2026-09-14】

##### ⚠⚠ これは 4.4 で**撤回した** `==` の厳密化とは別物

一見すると「4.4 で `1 == "a"` を弾くのをやめたのに、`case "s"` は弾くのか」と
矛盾して見えるので、理由を残す:

| | `a == "s"` | `match a: case "s":` |
|---|---|---|
| 結果 | `False` — **意味のある答え** | この腕に**決して入らない** |
| 書いた人の意図 | 「違うなら偽でいい」 | 「この場合を処理したい」 |
| 分類 | **仕様**（B2-b が固定） | **腕が死ぬ**（3.4 の `UnknownGuardType` と同系統） |

⇒ 弾くのは後者だけ。`==` の検査は入れていない。

##### 判定は `==` と**同じ規則**に合わせた（実測してから書いた）

| subject | pattern | 実行時 |
|---|---|---|
| `int` | `1.0` | **一致する**（`uint → int → float` の昇格ラティス） |
| `float` | `1` | **一致する** |
| `int` | `True` | 一致しない（`bool` はラティスの対象外） |
| `str` | `"x"` | 一致する |
| enum | enum メンバー | 一致する |
| `list[int]` | `[1, 2]` | 一致する |

⚠ 「静的検査だけ `==` より厳しい」状態を作ってはいけない。`case 1.0` を弾いていたら
**実行時には一致する腕を殺す**ことになる。

⚠ クラス（`__eq__` を定義できる）・enum・`Union`・`Any`・`Unresolved` は**素通し**。

##### 結果

**検体**: `X5` が `NONE` → **`STATIC`**。静的検査の割合 80% → **81%**（92/114）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 773 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 fall back（293 例題） |
| `compare_python_impl.ps1` | 87/87 identical・stale 0 |
| `compare_outputs.ps1 -A 2a91c2b` | 223/224。差分は**新規例題 1 件のみ** |
| `stale_doc_refs.ps1` | OK |
| `compare_wasm_frontend.ps1` | **294/294 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/match_case_pattern_type{,_error}.ar`

#### 5.5 の記録 【✅ 完了 2026-09-14】

##### 移行コストは**予測どおり例題 1 箇所**だった

起票時に「条件に裸の識別子を使う箇所は 33 あるが、非 bool は
`examples/classes/operator_overload.ar:87` の `if empty:` だけ」と実測していた。
実際に入れて `scan_examples` が落ちたのも**その 1 箇所だけ**。

⚠ `Bag` は `__bool__` を定義しているので「常に真」ではない。それでも D-12 の決定
（「bool 値のみ許可」）に従って `if empty.__bool__():` へ移行した。
**`__bool__` が死ぬわけではない**（`not` / `and` / `or` は今も `eval_truthy` 経由で
`__bool__` を呼ぶ）ので、例題にその実演（`print(not empty)`）を足した。

##### ⚠⚠ 偽陽性を通じて **join の写しが 3 つある**ことが判った

`examples/apps/spider_render.ar`（import が解決しない単体解析）で

```
the condition of 'if' must be 'bool', got 'Union[bool, unknown]'
```

が出た。`Union[bool, unknown]` ＝「**半分だけ判っている**」型で、こんなものを作る
join が 2 箇所あった:

| 場所 | `Unresolved` の扱い |
|---|---|
| `join_elem_types`（コレクションリテラル） | **潰す**（どれかが `Unresolved` なら全体が `Unresolved`）✅ |
| `join2`（連結演算） | 潰さない ⛔ |
| `infer_binop_result` の `And` / `Or` | **`join2` を呼ばず同じ規則を直書き**していた ⛔ |

⇒ `And` / `Or` を `join2` へ寄せ、`join2` に `Unresolved` を潰す規則を入れて 3 つを 1 つにした。
判らない側が混ざったら**全体が判らない**、が 3 箇所で同じになった。

⚠ この偽陽性は `scan_examples` では見つからない（`spider_render.ar` は import 専用モジュールで
単体実行されない）。**`compare_wasm_frontend` が拾った** — エディタ側が
「arrow.exe には無い診断」を出したことで表に出た形。ゲートを全部回す意味がここにあった。

##### 足場の撤去

タスク 3.1 で置いた `walk_obligation_pending`（「義務はあるが未実装」と宣言して歩く
ラッパ）は、**印を付けた地点をすべて実装し終えた**ので撤去した。
残すと「義務が無い」と区別できない死んだ足場になる。`#[must_use] infer` と `walk` は残る。

##### 結果

**検体**: `K1` `K2` が `NONE` → **`STATIC`**。静的検査の割合 81% → **82%**（94/114）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 773 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 fall back（295 例題） |
| `compare_python_impl.ps1` | 88/88 identical・stale 0 |
| `compare_outputs.ps1 -A 8dc4139` | 225/226。差分は**新規例題 1 件のみ** |
| `stale_doc_refs.ps1` | OK |
| `compare_wasm_frontend.ps1` | **296/296 agreed・INVENTED 0**（偽陽性を見つけたのがこれ） |

**追加した例題**: `examples/typing/condition_must_be_bool{,_error}.ar`
**移行した例題**: `examples/classes/operator_overload.ar`（`if empty:` → `if empty.__bool__():`）

#### 5.6 の記録 【✅ 完了 2026-09-14】

##### 照合先が**そもそも無かった**

`new_type Meters: float` に `Meters("s")` が黙って通っていた。`new_type` は
`known_class_names` と `new_type_originals` にしか登録されず、`__init__` の
シグネチャを持たないので、引数を突き合わせる相手が無い。
実行時が見ているのも**個数だけ**（`function takes 1 argument(s), got 2`）なので、
実行時エラーにもならず `<Meters object at 0x…>` が出来ていた。

##### ⚠⚠ 基底が**クラス**のときは検査してはいけない

`scan_examples` が `polymorphism.ar` で落ちて分かった:

```
argument 0 of 'Kilometers' expects 'Meters' but got 'int'
```

`new_type Kilometers: Meters` の `Meters` は**クラス**で、`Kilometers(5)` の `5` は
「`Meters` を包む値」ではなく **`Meters` のコンストラクタ引数**
（レジストリが `class_method_sigs` を引き継いでいる）。
⇒ `new_type` の連鎖を**根まで辿り**、根がプリミティブのときだけ検査する。
`Kg: Meters: float` は `float` を要求し、`Kilometers: Vec`（クラス）は見送る。

##### 結果

**検体**: `T3` が `NONE` → **`STATIC`**。静的検査の割合 82% → **83%**（95/114）。

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 773 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 fall back（297 例題） |
| `compare_python_impl.ps1` | 88/88 identical・stale 0（`new_type` の連鎖が py 未実装なので `$knownDiff` に登録） |
| `compare_outputs.ps1 -A dec83f8` | 227/228。差分は**新規例題 1 件のみ** |
| `stale_doc_refs.ps1` | OK |
| `compare_wasm_frontend.ps1` | **298/298 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/new_type_ctor_arg{,_error}.ar`

#### 5.7 の記録 【✅ 完了 2026-09-14・⇒ **フェーズ 5 完了**】

##### ⚠⚠ オーバーロードを**書いた瞬間に型検査が消えて**いた

絞り込みは**引数の個数だけ**で行われ、個数の合う候補が 2 つ以上あると
`count_matching.len() != 1` で**検査を丸ごと諦めて**いた（メソッド側・自由関数側の両方）。

```arrow
class C:
    fn m(self, let v: int) -> int: ...
    fn m(self, let v: str) -> int: ...
c.m(1.5)        # 旧: 素通り。実測では 1.5 を出していた
```

⇒ **引数が 1 本しかない関数より、オーバーロードした関数の方が緩い**という逆転。
この campaign で何度も出てきた「判らないものは通す」が、ここでは
「**判ろうとしていない**」になっていた。

##### 絞り込みの規則

| 残った候補 | 動作 |
|---|---|
| ちょうど 1 つ | それを使う（以降の引数検査が普通に走る） |
| 0 個 | `NoOverloadForArgTypes` を報告 |
| 2 つ以上 | **絞り込まない**（どれを選んでも嘘になりうる） |

⚠ 安全側に倒す条件（いずれかに当たれば絞り込まない）:
キーワード引数・可変長引数が混ざっている／候補に可変長パラメータがある／
実引数に判定材料の無い型（`Unresolved` / `Any`）がある。

⚠ メソッドと自由関数で**同じ helper を共有**した。別々に書くと片方だけ直す形になる
（3.3 で入口を 1 つにしたのと同じ理由）。

##### 副次効果: タスク 5.1 の保留が解ける見込み

5.1 の `binop_result_type` は「オーバーロードがあると実引数で選ぶ必要がある（タスク 5.7）」
としてシグネチャが 1 本のときだけ戻り値型を使っていた。その絞り込みが手に入ったので、
`__add__` のオーバーロードにも広げられる。⚠ **ただし今回は広げていない**
（複合代入の被演算子は 2 つで、この helper は呼び出し引数の並びを前提にしている）。

##### 結果

**検体**: `C12` が `NONE` → **`STATIC`**。静的検査の割合 83% → **84%**（96/114）。
⇒ **フェーズ 5（5.1〜5.7）完了。**

**ゲート結果**

| ゲート | 結果 |
|---|---|
| `cargo test --release` | 773 passed / 0 failed |
| `scan_examples.ps1` | 既知の `bench_ab_native.ar` TIMEOUT のみ |
| `force_gate.ps1` | 0 fall back（299 例題） |
| `compare_python_impl.ps1` | 88/88 identical・stale 0（py はオーバーロードを型でディスパッチしないので `$knownDiff` に登録） |
| `compare_outputs.ps1 -A c553d68` | 229/230。差分は**新規例題 1 件のみ** |
| `stale_doc_refs.ps1` | OK |
| `compare_wasm_frontend.ps1` | **300/300 agreed・INVENTED 0** |

**追加した例題**: `examples/typing/overload_arg_types{,_error}.ar`

### フェーズ 6 — 実行時との一本化

| # | 内容 | 決定 | 依存 | 備考 |
|---|---|---|---|---|
| **6.1** | 実行時・VM の判定 3 本（`value_matches_type_ann` / `value_is_type` / `TypeTag::matches`）を 3 分類から導出する | D-1 | フェーズ 4 | ⚠ 4 本並立のままだと再設計でずれが 1 本増える。A-3/A-4 で入れた `TypeTag` / `FieldCheck` もここに含む |

---

## 4. 決定 → タスク 対応表

| 決定 | 内容 | 主タスク | 関連タスク |
|---|---|---|---|
| D-1 | 3 分類 | **3.3** | 5.1〜5.7（適用先） |
| D-2 | 型式解決後に検査 | **1.1** | — |
| D-3 | アップキャストのみ | **4.1** | 2.7 |
| D-4 | `Unresolved` にも適用（推論を埋める） | **2.1 / 2.2 / 2.3 / 2.8** | — |
| D-5 | 暗黙 `int → float` 廃止 | **4.2** | 2.4（同時） |
| D-6 | 組み込み関数の戻り値型 | **2.4** | 4.2（同時） |
| D-7 | `=>` 強制（`__cast__` の受理を **`let` 仮引数だけに絞る**） | ~~**4.3**~~ ✅ | — |
| D-8 | `+=` の二段検査 | ~~**5.1**~~ ✅ | 4.4 |
| D-9 | Kind 3 は結果型も返す | ~~**4.4**~~ ✅ | ~~5.1~~ ✅ |
| D-10 | `and`/`or` の結果型を join に | **2.5** | 2.6 |
| D-11 | `list[⊥]` / 注釈なしは `list[Any]` | **4.1**（D-3 と同時） | — |
| D-12 | 条件は `bool` のみ | **5.5** | 2.5（前提） |
| D-13 | `function` は引数反変・戻り値共変 | **4.5** | 2.2 |
| D-14 | 妥当性検査を 2 系統目に | **3.4** | — |

## 5. 依存関係

```
フェーズ1（独立したバグ修正）───────────────────────────┐
                                                        │ 1.2 → 5.3
フェーズ2（推論）──→ フェーズ3（機構）──→ フェーズ4（規則）──→ フェーズ6（実行時）
     │                      │ 3.2                 │ 4.4
     │ 2.5 ─────────────────┼─────────────────────┼──→ 5.5
     │ 2.4 ─────────────────┼─────────────────────┼──→ 4.2（同時）
     │                      └──→ フェーズ5（適用）←┘
```

### 同時に入れなければならない組

| 組 | 理由 |
|---|---|
| **4.1 = D-3 ＋ D-11** | アップキャストのみ化だけだと `mut xs: list[int] = []` が落ちる（`[]` が素の `list` → `list[int]` はダウンキャストになる） |
| **4.2 ＋ 2.4** | 暗黙キャスト廃止だけだと変換が無検査の `float()` を経由して穴が移動する |

### 順序が必須の組

| 順序 | 理由 |
|---|---|
| **2.5 → 5.5** | `and`/`or` の結果型が `Bool` のままだと条件の `bool` 厳密化が機能しない |
| **1.2 → 5.3** | `StaticMut` が不変扱いのままだと検査を足しても代入自体が塞がれている |
| **2.2 → 4.5** | 関数値に型が付かないと分散規則を適用する相手がいない |
| **フェーズ2 → 3.1** | 推論が空のまま分離しても検査の材料がない |

## 6. 旧採番からの対応表

⚠ 本書の旧版と `type_binding_enforcement_plan.md` が使っていた採番。

| 旧 | 新 |
|---|---|
| `D-1`（3 分類＋方向） | D-1 ＋ **D-3**（方向を分離した） |
| `D-2` | D-2 |
| `D-3`（Unresolved へ適用） | D-4 |
| `D-4`（`=>` 強制） | D-7 |
| `D-5`（`+=` 二段） | D-8 |
| `D-6`（結果型） | D-9 |
| `D-7`（`list[⊥]`） | D-11 |
| `D-8`（`and`/`or`） | D-10 |
| `U-1` | D-5 |
| `U-2` | D-12 |
| `U-3` | D-14 |
| `U-4` | **消滅**（D-5 の廃止により合成型の曖昧性が発生しない） |
| `U-5` | D-13 |
| `U-6` | D-11 に統合 |
| `R-1` | タスク 2.1〜2.3 |
| `R-2` | タスク 3.1・3.2 |
| `R-3` | タスク 2.7 |
| `R-4` | タスク 2.9 |
| `R-5` | タスク 6.1 |
| `R-6` | タスク 1.2 |
| `R-7` | タスク 1.3 |
| `R-8` | D-6 ＋ タスク 2.4 |
| `T-0` | タスク 1.1 |
| `T-1` | タスク 2.1〜2.3 |
| `T-2` | タスク 3.1・3.2 |
| `T-3` | タスク 3.3・4.1 |
| `T-4` | タスク 4.1（3.3 と統合） |
| `T-5` | タスク 5.1 |
| `T-6` | タスク 4.2・4.3 |
| `T-6b` | タスク 2.5 |
| `T-7` | タスク 3.4 |
| `T-7b` | タスク 5.5 |
| `T-8` | タスク 6.1 |
| `T-9` | タスク 1.2・1.3・2.9 |
| `L-1`（`__cast__`） | D-7 ／ タスク 4.3 |
| `L-2`（`Intersection`/`Result`） | D-2 ／ タスク 1.1 |
| `L-3`（`Box[T]`/`function` が `Unresolved`） | D-4 ／ タスク 2.1・2.2 |
| `S-1`（`StaticMut`） | タスク 1.2 |

⚠ **検体 ID（`B1` / `O9` / `K4` 等）は変更していない。** ファイル名に焼いてあり、
義務の識別子であってタスクではない（冒頭「採番の規則」）。

## 7. 撤回した過去の評価

| 撤回対象 | 理由 |
|---|---|
| `type_binding_enforcement_plan.md` §7 #6「テンプレート実体化の戻り値型は…型変数の置換が必要（難しい）」 | A-2 で `GenericInstance` と `class_and_subst` ができた時点で機械的な作業になっていた。`call_check.rs:203` は `type_args` を既に持っている |
| 「3 分類では順序問題が残る」 | 地点 × 型の種類 → Kind の割り当てが決まれば順序は消える。残っていた合成型の曖昧性も D-5 で消滅 |
| 「`Kind 2` はダウンキャストなので不健全」 | アップキャストのみに限定（D-3） |
| 「`list[Any]` で空リテラルを表す」 | `Any` は要素型の**上端**なので方向が逆。`list[⊥]` が正しい（D-11） |
