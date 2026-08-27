# Phase 5 詳細計画 — TypeChecker 神クラス分割

対象: `src/type_check/`（13ファイル / 約3,750行）、`TypeChecker` 構造体（**18フィールド**）。
挙動は完全不変（純内部再編）。外部 API は `TypeChecker::check(&stmts)` /
`check_with_warnings(&stmts)` の2つのみで、**これらは一切変更しない**。

---

## 0. 事前調査の実測結果（この計画の前提）

計画を立てる前に実際に測った。**引き継ぎ書の前提が2点間違っていたので訂正する。**

| 項目 | 引き継ぎ書の想定 | 実測値 |
|---|---|---|
| `self.<field>` 直接アクセス箇所 | 「数百箇所に波及」 | **94箇所**（type_check 内のみ。外部ゼロ） |
| フィールド数 | 「~15個の HashMap」 | 18フィールド（うちマップ/セット12個） |

**波及が94箇所に収まっている理由**: 状態は既に `scope.rs` の `declare` / `lookup` /
`report_error` 等のアクセサ越しに使われており、生フィールドを触るコードは少ない。
つまり **Phase 5 は当初の想定より遥かに低リスク**。「高churn・高リスク」の評価は下方修正できる。

### 決定的な発見: レジストリ系フィールドは write-once

全フィールドの書き込み箇所を機械的に洗い出した結果:

| 分類 | 書き込み箇所 |
|---|---|
| 宣言レジストリ系 **12フィールド** | **すべて [mod.rs](../src/type_check/mod.rs) の 249–460 行**、すなわち `collect_fn_sigs`（217–484行）**1関数の中だけ** |
| 検査カーソル系 4フィールド | scope.rs / check.rs / infer.rs（検査中に増減） |
| 診断 2フィールド | scope.rs / infer.rs / mod.rs（検査中に append） |

→ **宣言レジストリは「事前パスで一度組み立てたら、検査中は読むだけ」**。
これは設計上の大きな追い風で、後述の「ビルダーで作って凍結する」型が使える。
（`collect_fn_sigs` の中では `errors` / `warnings` への書き込みが1件もない ＝
**収集パスは診断を出さない**ことも確認済み。依存が一方向になる根拠。）

### 補足: 状態分割では解決しない別の問題

`check_stmt`（[stmt/check.rs](../src/type_check/stmt/check.rs#L20)）は **728行・最大ネスト深度14**。
`infer`（[infer.rs](../src/type_check/infer.rs#L9)）は380行、`collect_fn_sigs` は267行。
**フィールドを構造体に束ねてもこの巨大関数は1行も短くならない。**
体感的な「神クラス感」の主因はむしろこちらなので、5B として別建てで扱う（§5）。

---

## 1. 【質問1への回答】個別インスタンスか、統括クラスの子クラスか

### まず前提: Rust に継承はない

「大きな統括的クラスの子クラス」という選択肢は Rust には存在しない。
`TypeChecker` を親クラスにしてサブクラスを生やすことはできないので、
実質的な選択肢は次の3つになる。

| 案 | 形 | 評価 |
|---|---|---|
| **A. 合成（composition）** | `TypeChecker { registry: TypeRegistry, state: CheckState, diags: Diagnostics }`。メソッドは `TypeChecker` に残り、`self.class_bases` → `self.registry.class_bases` になる | **採用（土台）**。機械的・コンパイラが全箇所を検出・段階適用可 |
| **B. 独立インスタンス＋引数渡し** | サブ構造体を個別に持ち、関数が `(reg: &TypeRegistry, diags: &mut Diagnostics, …)` を受け取る | **部分採用**。抽出関数の署名にのみ使う（後述） |
| **C. トレイト分割** | `impl TypeCheckerScope for TypeChecker` のようにトレイトで impl ブロックを分ける | **却下**。状態は1つのままで境界が生まれない。見た目だけの分割 |

### 推奨: A を土台に、レジストリだけ「ビルダー＋凍結」にする

純粋な A（ただの合成）だけでは**借用チェッカ的な利得がほぼない**点に注意が必要。
`self.registry` を借りたまま `self.report_error(…)` を呼ぶと、結局 `&mut self` が
衝突して現状と同じ苦しさが残る（現在の深ネストの一因でもある）。

そこで、§0 で判明した write-once 性を型で固定する:

```rust
// registry/builder.rs — 収集パス専用。insert 系メソッドしか持たない
pub(super) struct TypeRegistryBuilder { /* 12フィールド */ }
impl TypeRegistryBuilder {
    pub(super) fn insert_class(&mut self, …) { … }
    pub(super) fn build(self) -> TypeRegistry { TypeRegistry { … } }   // ← 凍結
}

// registry/mod.rs — 検査パス用。&self のゲッターしか持たない
pub(super) struct TypeRegistry { /* 12フィールド（private） */ }
impl TypeRegistry {
    pub(super) fn class_bases(&self, class: &str) -> &[String] { … }
    pub(super) fn is_known_class(&self, name: &str) -> bool { … }
    // …setter は存在しない
}
```

これで「検査中にレジストリを書き換えてしまう」バグが**型レベルで不可能**になる。
`TypeRegistry` は `TypeChecker` が所有する（`&'r TypeRegistry` を持ち回る案もあるが、
ライフタイム引数が `CheckCtx<'r>` 経由で広範囲に伝播するので**却下**。所有のままでよい）。

そして 5B の関数抽出時に、抽出先の署名を B 形式にする:

```rust
// self を丸ごと渡さない → 借用が競合せず、ネスト平坦化が可能になる
fn check_class_def(reg: &TypeRegistry, st: &mut CheckState, diags: &mut Diagnostics, …)
```

**つまり A と B は排他ではなく、A＝状態の置き場所、B＝抽出関数の署名、という役割分担。**
この組み合わせが「5A を先にやると 5B が楽になる」理由でもある（§4 の順序の根拠）。

---

## 2. 【質問2への回答】サブ構造体間の依存関係

### 結論: サブ構造体どうしの依存は**ゼロ**（相互参照なし）

依存グラフは循環のない放射状（スター型）になる:

```
                    TypeChecker  ← 唯一の統括役（ファサード）
                   /     |      \
                  v      v       v
        TypeRegistry  CheckState  Diagnostics
             (読)       (読書)      (追記)

        ── 3者の間に矢印は1本もない ──
```

| サブ構造体 | 依存先 | 他サブ構造体への依存 |
|---|---|---|
| `Diagnostics` | `StaticTypeError` / `StaticTypeWarning`（errors.rs） | **なし**（完全な葉） |
| `TypeRegistry` | `InferredType` / `FnSig` / `ProtocolInfo`（types.rs）、`FieldKind` / `Accessibility`（ast.rs） | **なし**。収集パスが診断を出さないので `Diagnostics` すら要らない |
| `CheckState` | `InferredType`（`VarInfo` 経由） | **なし** |
| `TypeChecker` | 上記3つすべて | — |

**「AがBを知っている」関係が1つも無いのがこの分割の肝**で、これが成立するのは
横断ロジック（例: 「基底クラスを辿って protected アクセスを検査し、駄目ならエラーを積む」）を
サブ構造体のメソッドにせず、**すべてファサード側（または B 形式の自由関数）に置く**からである。

### やってはいけない設計（依存を作ってしまう例）

- ❌ `TypeRegistry::check_protocol_conformance(&self, diags: &mut Diagnostics)` のように
  レジストリに検査ロジックを持たせる → Registry → Diagnostics の依存が生まれる
- ❌ `CheckState::declare()` の中でエラーを報告する → CheckState → Diagnostics の依存
  （現在 `declare` は `scope_stack` を触るだけで報告しないので、この性質を維持する）
- ✅ 検査ロジックは `fn check_x(reg: &TypeRegistry, st: &mut CheckState, diags: &mut Diagnostics)`

---

## 3. フィールド割り当て表（18 → 3構造体）

`self.<field>` の実測アクセス数つき。移設作業の見積りに使う。

### `TypeRegistry` — 宣言の索引（12フィールド / 49アクセス / 書き込みは収集パスのみ）

| フィールド | 型 | アクセス数 |
|---|---|---|
| `known_protocols` | `HashMap<String, ProtocolInfo>` | 11 |
| `class_method_sigs` | `HashMap<String, HashMap<String, Vec<FnSig>>>` | 8 |
| `known_class_names` | `HashSet<String>` | 6 |
| `class_bases` | `HashMap<String, Vec<String>>` | 5 |
| `fn_sigs` | `HashMap<String, Vec<FnSig>>` | 4 |
| `class_field_details` | `HashMap<String, HashMap<String, (FieldKind, InferredType)>>` | 4 |
| `class_fields` | `HashMap<String, HashMap<String, bool>>` | 2 |
| `trait_method_sigs` | 〃 | 2 |
| `trait_field_details` | 〃 | 2 |
| `class_member_access` | `HashMap<String, HashMap<String, Accessibility>>` | 2 |
| `new_type_originals` | `HashMap<String, String>` | 2 |
| `class_static_methods` | `HashMap<String, HashSet<String>>` | 1 |

> `known_protocols` は現在 `pub(crate)` だが、**crate 外から使われていない**
> （`parser/classes.rs:631` の `self.known_protocols` は Parser 自身の別フィールド）。
> 移設と同時に `pub(super)` へ降格してよい。

### `CheckState` — 検査カーソル（4フィールド / 35アクセス）

| フィールド | 型 | アクセス数 | 備考 |
|---|---|---|---|
| `block_return_forbidden_depth` | `usize` | 20 | infer.rs に save/restore が10対 → §5C でガード化 |
| `current_class_name` | `Option<String>` | 6 | |
| `scope_stack` | `Vec<HashMap<String, VarInfo>>` | 5 | 全アクセスが scope.rs に閉じている |
| `current_fn_name` | `Option<String>` | 4 | |

### `Diagnostics` — 収集された診断（2フィールド / 10アクセス）

| フィールド | 型 | アクセス数 |
|---|---|---|
| `errors` | `Vec<StaticTypeError>` | 5 |
| `warnings` | `Vec<StaticTypeWarning>` | 5 |

> `errors` / `warnings` は現在 `pub`。`check_with_warnings` が最後に取り出すだけなので、
> `Diagnostics::into_parts(self) -> (Vec<_>, Vec<_>)` を生やして private 化できる。

---

## 4. 目標ファイルレイアウト

```
src/type_check/
  mod.rs            753 → 約120行（ファサード: new / check / check_with_warnings のみ）
  registry/
    mod.rs          TypeRegistry（&self ゲッターのみ）
    builder.rs      TypeRegistryBuilder + collect_fn_sigs（mod.rs 217–484 を移設）
  state.rs          CheckState（scope.rs のスコープ操作を吸収）
  diagnostics.rs    Diagnostics（report_error / report_warning / into_parts）
  members.rs        get_type_members / check_intersection_members /
                    check_intersection_guard_type / class_implements_protocol
                    （mod.rs 484–753 を移設）
  scope.rs          検査ロジックのみ残す（check_member_access_static /
                    check_immutable_field_assign / subscript_root_ident）
  infer.rs / binop.rs / call_check.rs / decorator.rs / type_utils.rs / types.rs /
  errors.rs / stmt/    … 変更なし（アクセス経路が変わるだけ）
```

`mod.rs` は 753行 → 約120行。**「神クラス」の実体である mod.rs の肥大が解消される。**

---

## 5. 実施手順

各ステップは**独立してコミット可能**で、それぞれ `cargo test` 672 緑を維持すること。

### 5A. 状態の3分割（機械的・低リスク）【✅ 完了 — 2026-07-21】

実施結果は §8「5A 実施記録」を参照。以下は当初計画（そのまま実施した）。


コンパイラが全アクセス箇所を検出するので、漏れは原理的に起きない。

1. **`diagnostics.rs` 新設**（10箇所）— 最小・最も安全なので**ここから始める**。
   `Diagnostics { errors, warnings }` + `report_error` / `report_warning` / `into_parts`。
   `TypeChecker.errors` → `self.diags.errors`。`check_with_warnings` の返却を `into_parts` に。
   → ここで一度 `cargo test`。3分割の型（パターン）がこの1ステップで確定する。
2. **`state.rs` 新設**（35箇所）— `CheckState { scope_stack, current_fn_name,
   current_class_name, block_return_forbidden_depth }`。`push_scope` / `pop_scope` /
   `declare` / `lookup` を `CheckState` のメソッドへ移す（**診断は出さない**＝依存を作らない）。
   scope.rs に残る `check_member_access_static` 等は `TypeChecker` のメソッドのまま。
3. **`registry/` 新設**（49箇所）— 最大。ただし2段階に割れる:
   - 3a. まず `TypeRegistry` 構造体に12フィールドを移し、`pub(super)` フィールドのまま通す
     （＝ただの合成。この時点でビルドが通ることを確認）
   - 3b. 次に `collect_fn_sigs` を `registry/builder.rs` へ移設して
     `TypeRegistryBuilder` 化し、`TypeRegistry` のフィールドを private ＋ゲッター化
     （＝凍結。**書き込みが収集パスだけに限られる**ことを型で固定）
4. **`members.rs` 新設** — mod.rs 484–753 の4メソッドを移設（純粋な移動、状態変更なし）。

**5A 完了時点の検証**: `cargo test` 672 / `cargo build` 警告0 / `cargo clippy --all-targets` exit 0。
挙動不変なので**エラーメッセージの文言が1文字も変わらないこと**が合格条件
（Phase 3 Item1 と同じ基準）。

### 5B. 巨大関数の平坦化（判断を要する・本命）【✅ 完了 — 2026-07-21】

実施結果は §9「5B 実施記録」を参照。以下は当初計画。

5A を終えてから着手する。理由は §1 の通り、`&mut self` を丸ごと渡す代わりに
`(reg: &TypeRegistry, st: &mut CheckState, diags: &mut Diagnostics)` を渡せるようになり、
**借用の競合なしに関数を切り出せる**ようになるため。順序を逆にすると同じ作業が難しくなる。

| 対象 | 現状 | 方針 |
|---|---|---|
| `check_stmt`（check.rs:20） | **728行・深度14** | `Stmt` バリアント単位で `check_class_def` / `check_fn_def` / `check_assign` … へ分割。dispatch は match だけ残す |
| `infer`（infer.rs:9） | 380行 | 式カテゴリ単位（リテラル / 制御フロー式 / 添字 …）で分割 |
| `collect_fn_sigs`（mod.rs:217） | 267行 | 5A-3b で builder へ移設済み。さらに `collect_class` / `collect_trait` / `collect_protocol` に分割 |
| `check_intersection_members`（mod.rs:562） | 153行 | 5A-4 で members.rs へ移設済み。必要なら分割 |

### 5C. 仕上げ（小粒・任意）【✅ 完了 — 2026-07-21】

実施結果は §10「5C 実施記録」を参照。

- `block_return_forbidden_depth` の save/restore を
  RAII ガードまたは `with_loop_expr(|st| …)` ヘルパに集約 → 復元漏れバグを構造的に排除
- 残 clippy 警告のうち type_check 由来のもの（`collapsible_match` 等）を回収

---

## 6. リスクと非目標

**リスク**
- 唯一の実質リスクは 5B の関数抽出で**条件分岐の順序を変えてしまい、報告されるエラーの
  種類や順序が変わる**こと。→ 対策: type_check のテストは `frontend_tests/type_check_tests/`
  に9ファイル・約1,900行あり、エラー種別を直接アサートしている。5B は**1関数ずつ**切り出し、
  都度 `cargo test` する。
- 5A-3b で `TypeRegistry` のゲッター設計を誤ると呼び出し側が煩雑になる。
  → 対策: 既存のアクセスパターン（`get(class).and_then(|m| m.get(name))` の2段引き）を
  そのままゲッター1本に畳む形にする。

**非目標（今回やらないこと）**
- 型推論アルゴリズムの変更、新しい型規則の追加、エラーメッセージの改善
- `impl_python/` 側の追従（Rust 側の内部再編なので規約の git SHA 同期は非該当）
- `InferredType` / `errors.rs` の再設計（別テーマ。今回は触らない）

---

## 7. 検証コマンド

```
cargo test                                  # 672 passed 期待（各ステップごと）
cargo test type_check                       # 型検査テストのみ高速確認
cargo build                                 # 警告0 を維持
cargo clippy --all-targets                  # exit 0
../scripts/generate-codebase-map.ps1                 # registry/ 等のファイル新設後に必須
```

規約（.claude/rules/regulations.md）: 新文法の追加がないので example 追加は非該当。
VS Code 拡張・Python 実装にも変更が及ばないため VSIX 再生成・git SHA 同期も非該当。

---

## 8. 5A 実施記録（2026-07-21 完了）

計画どおり 5A-1 → 5A-4 の順で実施。**各ステップで `cargo test` 672 緑を確認**。

| ステップ | 内容 | 結果 |
|---|---|---|
| 5A-1 | `diagnostics.rs` 新設。`errors`/`warnings` → `Diagnostics`。直接 push していた8箇所も `report_error`/`report_warning` 経由に統一 | 672 緑 |
| 5A-2 | `state.rs` 新設。scope_stack / current_fn_name / current_class_name / block_return_forbidden_depth → `CheckState` | 672 緑 |
| 5A-3 | `registry/` 新設。12フィールド → `TypeRegistry`（private + `&self` ゲッター13本）。`collect_fn_sigs` を `registry/builder.rs` の `TypeRegistryBuilder::collect` へ移設し**書き込み経路を1ファイルに封じ込め** | 672 緑・**初回コンパイル成功** |
| 5A-4 | `members.rs` 新設。mod.rs の Intersection ヘルパ4関数を移設 | 672 緑 |

### 成果

- **`TypeChecker` のフィールド: 18 → 3**（`state` / `registry` / `diags`）
- **`mod.rs`: 753行 → 121行**（ファサードのみ。神クラスの実体だった肥大が解消）
- ビルド警告 **0件** / clippy **69件**（着手前と同数 = 新規の様式警告を持ち込んでいない）
- 公開 API（`TypeChecker::check` / `check_with_warnings`）は**無変更**。`new()` は
  収集パスを内包する形になったため `pub` → private に降格（外部利用ゼロを確認済み）

### 計画から変えた点（3点）

1. **5A-3 を 3a/3b に分けず一度に実施した。** 当初計画は「まず `pub(super)` フィールドで
   合成 → 次に private + ゲッター化」の2段だったが、事前調査で全49アクセスの読み取り
   パターンが判明していたためゲッター設計を先に確定でき、2段にすると同じ箇所を
   2回書き換えるだけだった。結果は初回コンパイル成功。
2. **`block_return_forbidden_depth` のガード化（5C 予定分）を前倒しした。** 生の深さを
   `pub(super)` で露出させると 5A の目的（カプセル化）が達成できないため、
   `enter_barrier`/`exit_barrier`（障壁）と `enter_loop_expr`/`exit_loop_expr`（ループ）の
   2ペアAPIとして実装。**残る 5C 項目は RAII ガード化のみ**（借用の都合で別途検討）。
3. **`collect_fn_sigs`（267行）は移設と同時に4関数へ分割した**（`collect` /
   `collect_class_methods` / `collect_class_members` / `collect_trait` / `collect_protocol`）。
   5B の対象だったが、移設で全体を書き写す以上、同時にやるのが最小コストだった。

### 調査時の実測ミス（記録）

§0 の「アクセス94箇所」は **`self.<field>` が同一行にある前提の正規表現による集計**で、
`self\n    .class_static_methods` のような**複数行チェーンを取りこぼしていた**
（`class_static_methods` を「1箇所」と数えたが実際は call_check.rs に2つの読み取りがあった）。
桁は合っていたので設計判断に影響はなかったが、**次に同種の集計をするときは
複数行チェーンを考慮すること**。

---

## 9. 5B 実施記録（2026-07-21 完了）

`check_stmt` と `infer` を関数抽出で平坦化。**各抽出ごとに `cargo test` 672 緑を確認**。

### 成果（実測 before → after）

| 関数 | 行数 | 最大ネスト深度 |
|---|---|---|
| `check_stmt`（stmt/check.rs） | **728 → 307** | **14 → 8** |
| `infer`（infer.rs） | **373 → 252** | 8 → 7 |

- ビルド警告 **0件** / `cargo test` **672 passed** / clippy **68件**（5A の 69 から1件減）
- 公開 API・挙動ともに無変更（エラー種別・順序を保存）

### 抽出した関数

**stmt/check.rs**（`check_stmt` は純粋なディスパッチ表になった）:
- 巨大アーム → 専用メソッド: `check_if` / `check_match` / `check_fn_def` / `check_gen_def` / `check_let_tuple`
- `check_if` 内の深いネスト（深度14の主因だった result_guard の7段 `if let`）を
  早期return方式の3つのサブ関数へ分解: `detect_type_guard`（静的）/ `detect_result_guard` /
  `narrow_by_type_guard`
- 重複集約: `Let`/`Const`/`Mut` の3アーム → `check_var_decl`（`mutable` フラグのみ差）、
  `AttrAssign`/`AttrCompoundAssign` の同一2アーム → `check_attr_assign`、
  FnDef の可変長/self/通常パラメータ束縛 → `declare_param`

**infer.rs**（`infer` から最大の3アームを抽出）:
- `infer_attr`（属性アクセス 40行）/ `infer_unaryop`（単項演算 40行）/ `infer_mustbe`（`mustbe` 47行）

### 計画から変えた点・報告事項

1. **状態を部分借用で渡す方式（`(&TypeRegistry, &mut CheckState, &mut Diagnostics)`）は使わなかった。**
   抽出先が `self.infer` / `self.check_stmts` / `self.report_error` 等の self メソッドを
   多数呼ぶため、素直に `&mut self` を取るメソッドに切り出すのが最小コストで、これでも
   借用競合は起きない（self 全体を1回可変借用するだけ）。§1 で想定した部分借用方式は、
   このコードでは呼び出し箇所が多すぎて逆に煩雑になると判明した。
2. **`collect_fn_sigs` / `check_intersection_members` の追加分割は不要と判断。** 前者は
   5A-3 で既に5関数へ分割済み、後者は members.rs で独立関数になっており、それ以上刻む
   価値が薄かった。
3. **⚠️ ツール起因のヒヤリハット（記録）**: infer.rs を編集する直前、Read が返した内容が
   **実ファイルと食い違う偽の内容**（`Expr::IntLit`/`resolve_protocol_type` 等、存在しない
   バリアント名）だった。危うくその偽内容を `old_string` にした破壊的 Edit をしかけたが、
   **編集前に `grep` で当該シンボルの実在を確認したところ「無し」と出て食い違いに気づき**、
   実ファイルを読み直して事なきを得た。教訓: **大きなブロックを差し替える前に、対象シンボルが
   実在するか grep で裏取りする**。とくに同一ファイルを何度も読み書きした後は Read 結果を鵜呑みにしない。

---

## 10. 5C 実施記録（2026-07-21 完了）

`block_return` 深さ操作のガード化と、type_check 由来の clippy 警告回収。**672 緑を維持**。

### block_return 深さ操作のガード化

`enter_barrier`/`exit_barrier`（5対）と `enter_loop_expr`/`exit_loop_expr`（2対）の
生の呼び出しを、**クロージャスコープ方式**の2ヘルパに集約:

```rust
fn with_barrier<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R { … }   // block/if/match 式・関数本体
fn with_loop_expr<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R { … } // for/while 式
```

- 呼び出し側は `self.with_barrier(|c| { c.push_scope(); c.check_stmts(body); c.pop_scope(); })` の形になり、
  enter と exit が1メソッド内で必ず対になるため**復元漏れが構造的に不可能**。
- 検証: `enter_barrier` 等の生呼び出しは scope.rs のヘルパ内2箇所のみに封じ込め済み（他ファイルから消滅）。

**RAII（Drop）ガードを採用しなかった理由**: `f` が `self.check_stmts()` 等で `TypeChecker` 全体を
可変借用するため、`CheckState` を借用し続ける Drop ガードだと借用が衝突する。クロージャ方式なら
`f` に `&mut self` を丸ごと渡せて衝突しない。パニック時の復元保証はないが、型検査はハッピーパスで
巻き戻らず、パニック時は検査全体が中断するので復元は不要。

**副次的整理**: 5式アーム（block/if/for/while/match）末尾に重複していた
`if let Some(t) = return_type { InferredType::from_ann(t)… } else { Unresolved }` を
`ann_or_unresolved(return_type)` ヘルパに集約（5箇所 → 1定義）。

### clippy 警告の回収（type_check 由来 5件）

- `type_utils.rs`: `loop { let Some(..) = .. else { break }; … }` → `while let Some(..) = .. { … }`
- `call_check.rs`（3件）・`decorator.rs`（1件）: 入れ子 `if let` をタプルパターンに畳み込み
  （例: `if let Some((_, ty)) = … { if let Some(x) = ty {` → `if let Some((_, Some(x))) = … {`）

**結果**: clippy 総数 **68 → 63**（type_check 由来はゼロに）。残 63 は他モジュールの様式系
（benches の `irrefutable let...else` 12件・`type_complexity` 等）で、Phase 5 の対象外。

### 計画から変えた点

- 計画は「RAII ガードまたは `with_loop_expr` ヘルパ」と両論併記だったが、上記の借用制約により
  **RAII は不可**と判明。クロージャスコープ方式で確定した。
- `ann_or_unresolved` の集約は計画に無かったが、ガード化で式アームを書き換える際に目に付いた
  明白な重複（5箇所同一）だったため同時に回収した。
