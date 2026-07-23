# Phase R / R0+R1 実装結果 — フレーム floor 隔離 ＋ ローカル読み取りの slot 解決

BYTECODE_VM_PLAN.md の **Phase R**（R0 ランタイムモデル ＋ R1 ローカル slot 化）の第一スライスを
実装・計測した記録。比較基準は [bench_baseline.md](bench_baseline.md)（フェーズ0のツリーウォーク実測）。

3つのレバーを実装した:
- **R1**: ローカル読み取りの slot 解決（`Expr::LocalRef`）。
- **R0（呼び出し機構）**: `frame_floor` によるスコープ隔離で、**呼び出しごとの Vec 確保・退避・復元を排除**。
- **引数束縛の割り当て削減**: `bind_args` の高速経路（位置引数・完全一致）で中間 Vec を 3 本排除、
  デフォルトなし関数の `evaluated_defaults` 確保も省略（毎コール計 4 本の小 Vec 確保を除去）。
  → R0 とこの引数束縛が全ワークロードに効く主因（下記 A/B 参照）。

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

### なぜ安全に slot 解決できるか（保守的解決）
- `exec_fn_evaled` は呼び出しごとに `scopes.drain(1..)` → `push_scope()` するため、**関数 base スコープは常に実行時 `scopes[1]`**。`up`（深さ）計算が不要で、リゾルバの最大の脆弱点が消える。
- シャドウイング禁止（型検査が保証）＝ base 名は関数内で一意。どこで読んでも同じ base slot。
- 対象は「capture_env が空になる」トップレベル関数のみ（メソッド=Self複雑化・入れ子関数=クロージャは対象外）。base 宣言順が AST から決定的。
- 未対応の宣言的文（import 等）を本体直下に含む関数は解決を丸ごと諦める（bail）。
- 解決できない読み取りは従来の名前引きにフォールバック（正しさ維持）。

## 検証（安定性）
- `cargo test` → **672 passed / 0 failed**（デバッグビルドで slot 名前一致 assert が全テストで発火せず＝解決順が実行時と完全一致。`frame_floor` 隔離もクロージャ・再帰・async・メソッド・import 全テストで正しく動作。引数束縛の高速経路・空 defaults 許容も arity/kwargs/デフォルト/可変長を含め全テスト緑）。
- 例題回帰 → basics/classes/collections/typing/exceptions の全 non-error 例で新旧の**終了コードも stdout も完全一致**（回帰0）。相違は skip-list 対象の非決定 async デモ 2 件のみ（タスク中断レースで、ベースラインでも run ごとに揺れる）。既存破損例（built_in / collection / functions / importation）はベースラインと同一。
- `cargo build` 警告 0 / 追加した clippy 警告 0。

## 速度計測（同一マシン・同時刻の A/B、release、各指標 best-of-10）

ベースライン binary（変更前）と R0+R1+引数束縛 binary を**交互実行**してマシンノイズを相殺。

### 要因分離（bottleneck_bench.ar, N=100万）
| 指標 | base (µs) | 実装後 (µs) | 倍率 |
|---|---|---|---|
| fn call（引数なし） | 0.526 | 0.474 | **1.11x** |
| let→let int（引数1・読み1） | 1.171 | 0.860 | **1.36x** |
| let→let instance | 1.479 | 1.183 | **1.25x** |
| 4-field read | 1.785 | 1.473 | **1.21x** |
| **4-var declare+lookup**（ローカル4読み） | 1.213 | 1.028 | **1.18x** |
| subscript[0] | 1.681 | 1.386 | **1.21x** |

### E2E（bench_field_access.ar, 各100万コール）
| 指標 | base | 実装後 | 倍率 |
|---|---|---|---|
| concrete class field access | 2.804 s | 2.416 s | **1.16x** |
| trait-backed field access | 2.518 s | 2.135 s | **1.18x** |

### 各レバーの寄与（累積、E2E concrete）
| 段階 | E2E 倍率 | 備考 |
|---|---|---|
| R1 読み取り解決 単体 | ~1.02x | ローカル読みは総コストの一部・小スコープの FxHash が既に速い |
| ＋ R0 `frame_floor` 隔離 | ~1.07x | 毎コールの drain/Vec 確保を排除（全コールに効く） |
| ＋ 引数束縛の割り当て削減 | **~1.16x** | 中間 Vec 3本＋defaults 1本＝毎コール小 Vec 4本の確保を除去 |

## 結論

- **全関数呼び出しワークロードが安定して 1.11〜1.36x 高速化**（E2E 1.16〜1.18x、全テスト緑、回帰0）。
- 効いたのは主に **呼び出し機構と引数束縛の per-call アロケーション削減**（frame_floor ＋ bind_args 高速経路）。
  R1 の読み取り解決はローカル読み支配コードで上乗せする土台。
- **まだツリーウォークのまま**の支配項: フィールド読み（~0.5µs/field）・`Value` clone・ディスパッチ。
  ここは Phase V（バイトコード）と §7.4（Value 表現）で潰す領域。
- 微ベンチで最大 **1.36x**（引数1・読み1）に達し、BYTECODE_VM_PLAN 投影の Phase R 下限（~1.3x）に到達。

## 次に効くレバー（このスライスが土台）
1. **`Value` clone 削減（§7.4）** — `Value::Str(Rc<str>)` 等。文字列ワークロードに効く。
2. **Phase V バイトコード VM** — ディスパッチ・制御フロー・演算をまとめて潰す本命。
3. **フィールドアクセスの IC / オフセット化（R3）** — `get_attr_val` の辞書引きを class_id インラインキャッシュへ。

`LocalRef` / slot 化 `Scope` / `frame_floor` フレームモデル / bind_args 高速経路 / リゾルバは上記すべての土台として再利用される。
