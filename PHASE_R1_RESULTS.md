# Phase R 実装結果 — R1 slot 解決 ／ R0 フレーム隔離 ／ 引数束縛 ／ R3 属性 IC

BYTECODE_VM_PLAN.md の **Phase R**（R0 ランタイムモデル ＋ R1 ローカル slot 化 ＋ R3 フィールド IC）を
実装・計測した記録。比較基準は [bench_baseline.md](bench_baseline.md)（フェーズ0のツリーウォーク実測）。

4つのレバーを実装した（すべて解釈経路の per-access / per-call コスト削減）:
- **R1**: ローカル読み取りの slot 解決（`Expr::LocalRef`）。
- **R0（呼び出し機構）**: `frame_floor` によるスコープ隔離で、**呼び出しごとの Vec 確保・退避・復元を排除**。
- **引数束縛の割り当て削減**: `bind_args` の高速経路（位置引数・完全一致）で中間 Vec を 3 本排除、
  デフォルトなし関数の `evaluated_defaults` 確保も省略（毎コール計 4 本の小 Vec 確保を除去）。
- **R3 属性インラインキャッシュ**: `obj.attr` のインスタンスフィールド解決を `(class_id, slot, access)` で
  キャッシュ。**`field_index` 辞書引き・アクセスキー走査・`format!` 確保をヒット時に全省略**。
  → **フィールド支配ワークロードの最大レバー**（E2E で 1.16x → 1.57x に跳ねた）。

## 実装したもの

| 変更 | ファイル | 内容 |
|---|---|---|
| `Expr::LocalRef { name, slot }` 追加 | [src/ast.rs](src/ast.rs) | リゾルバが付ける解決済みローカル参照。`Ident` は変更せず新バリアント追加（既存83箇所の `Ident` マッチに波及させないため） |
| `Scope` を slot 配列化 | [src/interpreter.rs](src/interpreter.rs) | `HashMap<String,Var>` → `Vec<(String,Var)>` + **遅延ハッシュ索引**（>16 変数のスコープ＝実質グローバルのみ索引構築）。関数/ブロックローカルは宣言=push、未解決名引き=線形走査 |
| 高速読み取り経路 | [src/interpreter/eval/core.rs](src/interpreter/eval/core.rs) | `Expr::LocalRef` → `scopes[frame_floor].slot(i)` を index 1回で読む。デバッグビルドで slot と名前の一致を検証（リゾルバのずれを即露見） |
| リゾルバパス | [src/interpreter/resolver.rs](src/interpreter/resolver.rs) | 型検査後・実行前に **メインプログラム直下の `fn`/`gen`** の base スコープ読み取りを `Ident`→`LocalRef` に書き換え |
| フック | [src/main.rs](src/main.rs), tests/mod.rs | `run_program` とテストヘルパーで `resolve_program` を呼ぶ |
| **`frame_floor` 隔離（R0）** | [interpreter.rs](src/interpreter.rs), [scope.rs](src/interpreter/scope.rs), [functions/execution.rs](src/interpreter/functions/execution.rs) | 呼び出しごとの `scopes.drain(1..).collect()`（Vec 確保）＋退避＋`extend` 復元を廃止。代わりに `frame_floor`（現関数 base の index）を進め、名前引きは `scopes[0]`＋`scopes[frame_floor..]` のみ走査。復元は `truncate` だけ（確保なし）。`capture_env`/`assign_var`/`make_var_immutable`/`try_fill_slot`/async キャプチャも frame_floor 準拠に更新 |
| **引数束縛の割り当て削減** | [functions/args.rs](src/interpreter/functions/args.rs), [functions/execution.rs](src/interpreter/functions/execution.rs) | `bind_args` に高速経路を追加（位置引数のみ・可変長なし・引数数一致）: 中間 Vec（`non_variadic_evaled`/`slots`/`slot_is_mutable`）を確保せず仮引数と評価済み引数を直接 zip。`bind_args`/`bind_args_relaxed` を空 defaults スライス許容に変更し、デフォルトを持つ仮引数がなければ `evaluated_defaults` の確保を省略。出力・エラー意味論は一般経路と同一 |
| **R3 属性 IC** | [ast.rs](src/ast.rs), [eval/attrs.rs](src/interpreter/eval/attrs.rs), [eval/core.rs](src/interpreter/eval/core.rs) | `Expr::Attr` に `AttrCache`（`Cell<u64>` に `class_id`/slot/アクセスレベルをパック）を追加。`eval_attr` は `class_id` 一致で `field_value(slot)` を直接読み、`field_index.get`・アクセスキー走査・`format!("::{attr}")` 確保・`check_member_access` の辞書引きをすべて省略。ミス時は `get_attr_val` が解決してキャッシュを更新（単相 IC・多相点は毎回再解決）。アクセス制御は `check_access_level` でヒット時も**毎回ライブ判定**（`current_class` 依存のため）。デバッグビルドで slot 一致を検証 |

### なぜ安全に slot 解決できるか（保守的解決）
- `exec_fn_evaled` は呼び出しごとに `scopes.drain(1..)` → `push_scope()` するため、**関数 base スコープは常に実行時 `scopes[1]`**。`up`（深さ）計算が不要で、リゾルバの最大の脆弱点が消える。
- シャドウイング禁止（型検査が保証）＝ base 名は関数内で一意。どこで読んでも同じ base slot。
- 対象は「capture_env が空になる」トップレベル関数のみ（メソッド=Self複雑化・入れ子関数=クロージャは対象外）。base 宣言順が AST から決定的。
- 未対応の宣言的文（import 等）を本体直下に含む関数は解決を丸ごと諦める（bail）。
- 解決できない読み取りは従来の名前引きにフォールバック（正しさ維持）。

## 検証（安定性）
- `cargo test` → **672 passed / 0 failed**（デバッグビルドで slot 名前一致 assert ＋ `AttrCache` slot 一致 assert が全テストで発火せず＝解決結果が実行時と完全一致。`frame_floor` 隔離もクロージャ・再帰・async・メソッド・import 全テストで正しく動作。引数束縛の高速経路・空 defaults 許容も arity/kwargs/デフォルト/可変長を含め全テスト緑）。
- 例題回帰 → basics/classes/collections/typing/exceptions の全 non-error 例で新旧の**終了コードも stdout も完全一致**（回帰0）。private/protected を実行時に読む `class_trait.ar` 等も含め出力一致＝IC のアクセス制御も無破壊。相違は skip-list 対象の非決定 async デモ 2 件のみ（タスク中断レースで、ベースラインでも run ごとに揺れる）。既存破損例（built_in / collection / functions / importation）はベースラインと同一。
- `cargo build` 警告 0 / 追加した clippy 警告 0。

## 速度計測（同一マシン・同時刻の A/B、release、各指標 best-of-N）

ベースライン binary（変更前）と最終 binary（R1+R0+引数束縛+R3）を**交互実行**してマシンノイズを相殺。

### 要因分離（bottleneck_bench.ar, N=100万）
| 指標 | base (µs) | 実装後 (µs) | 倍率 |
|---|---|---|---|
| fn call（引数なし） | 0.531 | 0.451 | **1.18x** |
| let→let int（引数1・読み1） | 1.181 | 0.870 | **1.36x** |
| let→let instance | 1.469 | 1.000 | **1.47x** |
| **4-field read** | 1.802 | 1.145 | **1.57x** |
| 4-var declare+lookup（ローカル4読み） | 1.242 | 1.031 | **1.20x** |
| subscript[0] | 1.684 | 1.185 | **1.42x** |

### E2E（bench_field_access.ar, 各100万コール）
| 指標 | base | 実装後 | 倍率 |
|---|---|---|---|
| concrete class field access（7 field read/call） | 2.815 s | 1.797 s | **1.57x** |
| trait-backed field access（3 field read/call） | 2.494 s | 1.610 s | **1.55x** |

### 各レバーの寄与（累積、E2E concrete field access）
| 段階 | E2E 倍率 | 効いた理由 |
|---|---|---|
| R1 読み取り解決 単体 | ~1.02x | ローカル読みは総コストの一部・小スコープの FxHash が既に速い |
| ＋ R0 `frame_floor` 隔離 | ~1.07x | 毎コールの drain/Vec 確保を排除（全コールに効く） |
| ＋ 引数束縛の割り当て削減 | ~1.16x | 中間 Vec 3本＋defaults 1本＝毎コール小 Vec 4本の確保を除去 |
| **＋ R3 属性 IC** | **~1.57x** | フィールド読みごとの `format!` 確保＋`field_index` 走査＋辞書引き 2本を class_id 比較 1回に置換 |

## 結論

- **解釈経路が全面的に 1.18〜1.57x 高速化**（E2E フィールドアクセス 1.55〜1.57x、全テスト緑、回帰0）。
- 段階ごとの主因:
  - **R0 `frame_floor` ＋ 引数束縛**: 全関数呼び出しの per-call アロケーションを削減（呼び出し 1.18x・引数1で 1.36x）。
  - **R3 属性 IC**: フィールドアクセスの per-read オーバーヘッド（`format!`＋走査＋辞書引き）を除去。
    **フィールド支配コードで最大の効果**（4-field read 1.57x・E2E 1.55〜1.57x）。
  - **R1 読み取り解決**: ローカル読み支配コードで上乗せ（`4-var` 1.20x）。上位レバーの土台。
- BYTECODE_VM_PLAN 投影の **Phase R 1.3〜2x に到達**（フィールド/インスタンス支配で ~1.5-1.6x）。
- **まだツリーウォークのまま**の支配項: `Value` clone・ディスパッチ・演算・メソッド呼び出し。

## 次に効くレバー（このスライスが土台）
1. **`Value` clone 削減（§7.4）** — `Value::Str(Rc<str>)` 等。文字列ワークロードに効く。
2. **Phase V バイトコード VM** — ディスパッチ・制御フロー・演算・メソッド呼び出しをまとめて潰す本命。
3. **R4 呼び先解決** — Arrow 関数呼び出しを名前引きから解決済みターゲットへ（`Expr::Call.cache` 拡張）。

`LocalRef` / slot 化 `Scope` / `frame_floor` フレームモデル / bind_args 高速経路 / `AttrCache` / リゾルバは上記すべての土台として再利用される。
