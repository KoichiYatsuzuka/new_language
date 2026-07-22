# Phase R / R0+R1 実装結果 — フレーム floor 隔離 ＋ ローカル読み取りの slot 解決

BYTECODE_VM_PLAN.md の **Phase R**（R0 ランタイムモデル ＋ R1 ローカル slot 化）の第一スライスを
実装・計測した記録。比較基準は [bench_baseline.md](bench_baseline.md)（フェーズ0のツリーウォーク実測）。

2つのレバーを実装した:
- **R1**: ローカル読み取りの slot 解決（`Expr::LocalRef`）。
- **R0（呼び出し機構）**: `frame_floor` によるスコープ隔離で、**呼び出しごとの Vec 確保・退避・復元を排除**。
  → こちらが全ワークロードに効く主因（下記 A/B 参照）。

## 実装したもの

| 変更 | ファイル | 内容 |
|---|---|---|
| `Expr::LocalRef { name, slot }` 追加 | [src/ast.rs](src/ast.rs) | リゾルバが付ける解決済みローカル参照。`Ident` は変更せず新バリアント追加（既存83箇所の `Ident` マッチに波及させないため） |
| `Scope` を slot 配列化 | [src/interpreter.rs](src/interpreter.rs) | `HashMap<String,Var>` → `Vec<(String,Var)>` + **遅延ハッシュ索引**（>16 変数のスコープ＝実質グローバルのみ索引構築）。関数/ブロックローカルは宣言=push、未解決名引き=線形走査 |
| 高速読み取り経路 | [src/interpreter/eval/core.rs](src/interpreter/eval/core.rs) | `Expr::LocalRef` → `scopes[frame_floor].slot(i)` を index 1回で読む。デバッグビルドで slot と名前の一致を検証（リゾルバのずれを即露見） |
| リゾルバパス | [src/interpreter/resolver.rs](src/interpreter/resolver.rs) | 型検査後・実行前に **メインプログラム直下の `fn`/`gen`** の base スコープ読み取りを `Ident`→`LocalRef` に書き換え |
| フック | [src/main.rs](src/main.rs), tests/mod.rs | `run_program` とテストヘルパーで `resolve_program` を呼ぶ |
| **`frame_floor` 隔離（R0）** | [interpreter.rs](src/interpreter.rs), [scope.rs](src/interpreter/scope.rs), [functions/execution.rs](src/interpreter/functions/execution.rs) | 呼び出しごとの `scopes.drain(1..).collect()`（Vec 確保）＋退避＋`extend` 復元を廃止。代わりに `frame_floor`（現関数 base の index）を進め、名前引きは `scopes[0]`＋`scopes[frame_floor..]` のみ走査。復元は `truncate` だけ（確保なし）。`capture_env`/`assign_var`/`make_var_immutable`/`try_fill_slot`/async キャプチャも frame_floor 準拠に更新 |

### なぜ安全に slot 解決できるか（保守的解決）
- `exec_fn_evaled` は呼び出しごとに `scopes.drain(1..)` → `push_scope()` するため、**関数 base スコープは常に実行時 `scopes[1]`**。`up`（深さ）計算が不要で、リゾルバの最大の脆弱点が消える。
- シャドウイング禁止（型検査が保証）＝ base 名は関数内で一意。どこで読んでも同じ base slot。
- 対象は「capture_env が空になる」トップレベル関数のみ（メソッド=Self複雑化・入れ子関数=クロージャは対象外）。base 宣言順が AST から決定的。
- 未対応の宣言的文（import 等）を本体直下に含む関数は解決を丸ごと諦める（bail）。
- 解決できない読み取りは従来の名前引きにフォールバック（正しさ維持）。

## 検証（安定性）
- `cargo test` → **672 passed / 0 failed**（デバッグビルドで slot 名前一致 assert が全テストで発火せず＝解決順が実行時と完全一致。`frame_floor` 隔離もクロージャ・再帰・async・メソッド・import 全テストで正しく動作）。
- 例題回帰 → basics/classes/collections/async/typing/exceptions 全 non-error 例で floor 版とベースライン版の終了コードが一致（回帰0）。既存破損例（built_in / collection / functions / importation）はベースラインと同一。
- `cargo build` 警告 0 / 追加した clippy 警告 0。

## 速度計測（同一マシン・同時刻の A/B、release、各指標 best-of-10）

ベースライン binary（変更前）と R0+R1 binary を**交互実行**してマシンノイズを相殺。

### 要因分離（bottleneck_bench.ar, N=100万）
| 指標 | base (µs) | R0+R1 (µs) | 倍率 |
|---|---|---|---|
| fn call（引数なし） | 0.528 | 0.473 | **1.12x** |
| let→let int（引数1・読み1） | 1.175 | 1.094 | **1.07x** |
| let→let instance | 1.480 | 1.385 | **1.07x** |
| 4-field read | 1.802 | 1.696 | **1.06x** |
| **4-var declare+lookup**（ローカル4読み） | 1.251 | **1.012** | **1.24x** |
| subscript[0] | 1.688 | 1.583 | **1.07x** |

### E2E（bench_field_access.ar, 各100万コール）
| 指標 | base | R0+R1 | 倍率 |
|---|---|---|---|
| concrete class field access | 2.798 s | 2.612 s | **1.07x** |
| trait-backed field access | 2.467 s | 2.367 s | **1.04x** |

> 参考: R1（LocalRef 読み取り解決）**単体**では E2E ~1-2%・ほとんどの行が ~1.00x だった。
> `frame_floor` 隔離（呼び出しごとの Vec 確保排除）を足して初めて**全ワークロードが 1.04〜1.24x** に伸びた。

## 結論

- **全関数呼び出しワークロードが安定して速くなった**（E2E 1.04〜1.07x、微ベンチ 1.06〜1.24x、全テスト緑、回帰0）。
- 効いたのは主に **R0 の呼び出し機構軽量化**（毎コールの `drain(1..).collect()` Vec 確保＋退避＋復元の排除）。
  R1 の読み取り解決はローカル読み支配のコードで上乗せ（`4-var` で 1.24x）。
- なぜ「劇的」でないか（正直な評価）:
  1. 引数束縛（~0.7µs/arg）・フィールド読み（~0.5µs/field）・`Value` clone は**まだツリーウォークのまま**。
  2. 小さいスコープでは FxHash 名前引きが既に速く、slot 化の理論値ほど wall-clock に乗らない。
- BYTECODE_VM_PLAN 投影の Phase R ~1.3-2x に近づくには、次のレバーが必要。

## 次に効くレバー（このスライスが土台）
1. **引数の slot 直束縛** — bindings Vec + 名前 insert を経由せず base slot に直書き（引数束縛 ~0.7µs/arg を削る）。
2. **`Value` clone 削減（§7.4）** — `Value::Str(Rc<str>)` 等。文字列ワークロードに効く。
3. **Phase V バイトコード VM** — ディスパッチ・制御フロー・引数束縛をまとめて潰す本命。

`LocalRef` / slot 化 `Scope` / `frame_floor` フレームモデル / リゾルバは上記すべての土台として再利用される。
