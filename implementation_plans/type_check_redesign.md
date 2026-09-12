# 静的型検査の再設計

**状態**: 設計確定（実装未着手）。決定 **D-1〜D-14** すべて確定済み。判断待ちなし
**起票**: 2026-09-12
**前提文書**: [type_binding_enforcement_plan.md](type_binding_enforcement_plan.md)（個別バグ修正キャンペーン 0-1〜A-4 の記録と型義務の棚卸し）
**進捗計**: `scripts/type_obligations.ps1`（型義務 **114 件**・着手時点 **STATIC 34%** → 現在 **56%**）

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
| **2.4** | **組み込み関数のシグネチャ表**（戻り値型・引数型） | D-6 | `Z9`〜`Z12` | ⚠ **4.2 と同時に入れる** |
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
| **2.4**（組み込み関数のシグネチャ表） | **未着手**。⚠ **4.2 と同時に入れる**制約があるのでフェーズ 4 で扱う |

### フェーズ 3 — 検査機構を作る

| # | 内容 | 決定 | 依存 | 備考 |
|---|---|---|---|---|
| ~~**3.1**~~ | ~~`walk` / `type_of` に分離~~ → **✅ 完了 2026-09-13** | — | フェーズ 2 | 下記「3.1 / 3.2 の記録」 |
| ~~**3.2**~~ | ~~`walk` を AST バリアントに対して網羅化~~ → **✅ 完了 2026-09-13**（**一部は既に満たされていた**） | — | 3.1 | 同上 |
| **3.3** | 3 分類（Kind 1/2/3）を**単一定義**として実装する | D-1 | 3.2 | |
| **3.4** | 妥当性検査を 2 系統目として置く（名前の存在・個数・定義時） | D-14 | 3.2 | `X6` `K13` `N2` `M1` `M2` |

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
| `walk_obligation_pending(e, task)` | 「**義務はあるが未実装**」と宣言して歩く。`task` に実装タスク番号を書く |

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

### フェーズ 4 — 関係の規則を正す

| # | 内容 | 決定 | 依存 | 備考 |
|---|---|---|---|---|
| **4.1** | アップキャストのみに限定（`List` ↔ `ListOf` の双方向特例を撤去）＋ `⊥` の導入 | D-3 / D-11 | 3.3 | ⚠⚠ **2 つを同時に入れる**（§5） |
| **4.2** | 暗黙 `int → float` を廃止 | D-5 | 3.3・**2.4** | ⚠ 2.4 と同時。移行は既存例題 1 件＋本キャンペーンの実演例題 8 件の書き換え／撤去 |
| **4.3** | `__cast__[T]` による受理をやめ `=>` を強制する | D-7 | 3.3 | `K6`。「型検査が嘘をつく」1 件目の解消 |
| **4.4** | 演算子を Kind 3 で検査し、結果型を返す | D-9 | 3.3 | `O1`〜`O8` `Z1` |
| ~~**4.5**~~ | ~~`function` 型の分散（引数反変・戻り値共変）~~ → **✅ 2.2 に吸収して完了** | D-13 | — | ⚠ **2.2 と分離できなかった**（下記 2.2 の記録） |

### フェーズ 5 — 義務を全地点へ行き渡らせる

| # | 内容 | 決定 | 依存 | 検体 |
|---|---|---|---|---|
| **5.1** | 複合代入の二段検査 | D-8 / D-9 | 4.4 | `B6` `B10` `B11` `F2` |
| **5.2** | 未検査の地点を埋める（添字代入・添字の型・組み込みメソッド引数・可変長要素・`yield`・`block_return`/`loop_yield`・trait フィールド代入の静的化） | D-1 | 3.2 | `L5` `L6`〜`L8` `L14` `C8` `C15` `X1`〜`X4` `F3` |
| **5.3** | `static mut` クラス変数への代入を検査 | D-1 | 3.2・1.2 | `F4` |
| **5.4** | `match` の `case` パターン型を subject と照合 | D-1 | 3.2 | `X5` |
| **5.5** | `if` / `while` の条件を `bool` 厳密に | D-12 | **2.5** | `K1` `K2`。移行は例題 1 箇所。⚠ `is_truthy` は**撤去しない** |
| **5.6** | `new_type` のコンストラクタ引数を検査 | D-1 | 3.2 | `T3` |
| **5.7** | オーバーロード解決を実引数型で行う | D-1 | 3.2 | `C12` |

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
| D-7 | `=>` 強制（`__cast__` の受理をやめる） | **4.3** | — |
| D-8 | `+=` の二段検査 | **5.1** | 4.4 |
| D-9 | Kind 3 は結果型も返す | **4.4** | 5.1 |
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
