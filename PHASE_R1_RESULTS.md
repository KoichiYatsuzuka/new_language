# Phase R 実装結果 — R1 slot 解決 ／ R0 フレーム隔離 ／ 引数束縛 ／ R3 属性 IC ／ R4 呼び先解決

BYTECODE_VM_PLAN.md の **Phase R**（R0 ランタイムモデル ＋ R1 slot 化 ＋ R3 フィールド IC ＋ R4 呼び先解決）を
実装・計測した記録。比較基準は [bench_baseline.md](bench_baseline.md)（フェーズ0のツリーウォーク実測）。

6つのレバーを実装した（すべて解釈経路の per-access / per-call コスト削減）:
- **R1**: ローカル読み取りの slot 解決（`Expr::LocalRef`）。
- **R0（呼び出し機構）**: `frame_floor` によるスコープ隔離で、**呼び出しごとの Vec 確保・退避・復元を排除**。
- **引数束縛の割り当て削減**: `bind_args` の高速経路（位置引数・完全一致）で中間 Vec を 3 本排除、
  デフォルトなし関数の `evaluated_defaults` 確保も省略（毎コール計 4 本の小 Vec 確保を除去）。
- **R3 属性インラインキャッシュ**: `obj.attr` のインスタンスフィールド解決を `(class_id, slot, access)` で
  キャッシュ。**`field_index` 辞書引き・アクセスキー走査・`format!` 確保をヒット時に全省略**。
- **R4 呼び先解決**: `f(args)` の不変グローバル関数呼び先を global slot でキャッシュ。ヒット時は
  **builtin 判定・名前引き・`name.clone()` を跳ばして直接ディスパッチ**（呼び出しコスト自体を削減）。
- **メソッド呼び出し IC**: `obj.method(args)` を `class_id` でキャッシュ。ヒット時は
  **gen_methods/native/static/class_method 判定と不変性フィルタ（計 ~4 の SipHash 引き）を省略**。
- **§7.4 `Value` サイズ削減**: 72B の `JsProcFn`（String×3）を `Box` 化し、**`size_of::<Value>()` を 72→32B**
  に縮小。フィールド読み・引数受け渡し・`deep_clone`・スタック操作の**全 Value コピーが 2.25x 軽く**なる。

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
| **R4 呼び先解決** | [ast.rs](src/ast.rs), [eval/calls.rs](src/interpreter/eval/calls.rs) | `NativeCallCache` に `SlotCache`（`Cell<u64>`・Send 安全）を追加。`f(args)` が不変グローバル関数（`fn` 定義）と解決されたとき `(slot_epoch, global_slot)` を焼き込む。ヒット時は `eval_builtin_ident_call` の全 builtin 名照合・`get_val` 名前引き・`name.clone()` を跳ばし `scopes[0].slot(idx)` から直接 `exec_fn`。ローカル束縛・オーバーロード・可変束縛は対象外（通常経路）。`freeze` で epoch が進めば自動失効。デバッグビルドで呼び先一致を検証。あわせて slow path の `call_name` を `String`→`&str` 化（毎コールの確保も除去） |
| **メソッド呼び出し IC** | [ast.rs](src/ast.rs), [classes/method_call.rs](src/interpreter/classes/method_call.rs), [eval/calls.rs](src/interpreter/eval/calls.rs) | `NativeCallCache` に 3本目 `AttrCache`（`Cell<u64>`・Send 安全）を追加し、`eval_method_call` に `Option<&cache>` 引数を追加。呼び先が **plain 非 mut-self 単一オーバーロードのインスタンスメソッド**（gen/native/static/class_method でない）と解決されたとき `class_id` を焼き込む。ヒット時は `gen_methods.get`/native 登録引き/`static_method_names.contains`/`class_method_names.contains` と不変性フィルタ Vec 確保を省略。非 mut-self に限定するのでインスタンス可変性に非依存。for ループ内部の `next`/`__iter__` 呼びは `None`（対象外）。デバッグビルドで高速経路の前提（gen/native/static/class_method でない・非 mut-self）を検証 |
| **§7.4 `Value` サイズ削減** | [value/core.rs](src/interpreter/value/core.rs) ほか計6ファイル | `Value::JsProcFn { bridge_key, module_name, fn_name }`（String×3=72B）を `Value::JsProcFn(Box<JsProcData>)`（8B）に変更。`size_of::<Value>()` が **72→32B**、`Var` が 80→40B に縮小。`Box::clone` は中身を複製するので `deep_clone` の「スレッド跨ぎで Rc を共有しない」不変条件も維持（js-proc 値は稀なので複製コストは非支配的）。構築1箇所・マッチ8箇所を更新 |

### なぜ安全に slot 解決できるか（保守的解決）
- `exec_fn_evaled` は呼び出しごとに `scopes.drain(1..)` → `push_scope()` するため、**関数 base スコープは常に実行時 `scopes[1]`**。`up`（深さ）計算が不要で、リゾルバの最大の脆弱点が消える。
- シャドウイング禁止（型検査が保証）＝ base 名は関数内で一意。どこで読んでも同じ base slot。
- 対象は「capture_env が空になる」トップレベル関数のみ（メソッド=Self複雑化・入れ子関数=クロージャは対象外）。base 宣言順が AST から決定的。
- 未対応の宣言的文（import 等）を本体直下に含む関数は解決を丸ごと諦める（bail）。
- 解決できない読み取りは従来の名前引きにフォールバック（正しさ維持）。

## 検証（安定性）
- `cargo test` → **672 passed / 0 failed**（デバッグビルドで **4種の一致 assert**（LocalRef slot / AttrCache slot / R4 呼び先 / method IC 前提）が全テストで発火せず＝キャッシュ結果が実行時解決と完全一致。`frame_floor` 隔離もクロージャ・再帰・async・メソッド・import 全テストで正しく動作。引数束縛の高速経路・空 defaults 許容も arity/kwargs/デフォルト/可変長を含め全テスト緑）。
- 例題回帰 → basics/classes/collections/typing/exceptions/practical_examples の全 non-error 例で新旧の**終了コードも stdout も完全一致**（回帰0）。private/protected を実行時に読む `class_trait.ar` 等も含め出力一致＝IC のアクセス制御も無破壊。相違は skip-list 対象の非決定 async デモ 2 件のみ（タスク中断レースで、ベースラインでも run ごとに揺れる）。既存破損例（built_in / collection / functions / importation）はベースラインと同一。
- `cargo build` 警告 0 / 追加した clippy 警告 0。

## 速度計測（同一マシン・同時刻の A/B、release、各指標 best-of-N）

ベースライン binary（変更前）と最終 binary（R1+R0+引数束縛+R3+R4）を**交互実行**してマシンノイズを相殺。

### 要因分離（bottleneck_bench.ar, N=100万）
| 指標 | base (µs) | 実装後 (µs) | 倍率 |
|---|---|---|---|
| fn call（引数なし） | 0.535 | 0.370 | **1.44x** |
| let→let int（引数1・読み1） | 1.189 | 0.759 | **1.57x** |
| let→let instance | 1.522 | 0.900 | **1.69x** |
| **4-field read** | 1.828 | 1.000 | **1.83x** |
| 4-var declare+lookup（ローカル4読み） | 1.237 | 0.943 | **1.31x** |
| subscript[0] | 1.701 | 1.081 | **1.57x** |

### E2E（bench_field_access.ar / bench_method_call.ar, 各100万コール）
| 指標 | base | 実装後 | 倍率 |
|---|---|---|---|
| concrete class field access（7 field read/call） | 2.861 s | 1.697 s | **1.69x** |
| trait-backed field access（3 field read/call） | 2.467 s | 1.523 s | **1.62x** |
| **method call**（`p.sum()`, メソッド呼び + 2 field read/call） | 1.656 s | 1.049 s | **1.58x** |

> メソッド呼び出し IC の寄与: method call は IC なし（R4 まで）で 1.272s → IC ありで 1.028s（**1.24x** 上乗せ）。
> §7.4 サイズ削減の寄与: 全ワークロードに ~1-7%（fn call/let→let int で r5→r6 各 1.07x）。

### 各レバーの寄与（累積、E2E concrete field access）
| 段階 | E2E 倍率 | 効いた理由 |
|---|---|---|
| R1 読み取り解決 単体 | ~1.02x | ローカル読みは総コストの一部・小スコープの FxHash が既に速い |
| ＋ R0 `frame_floor` 隔離 | ~1.07x | 毎コールの drain/Vec 確保を排除（全コールに効く） |
| ＋ 引数束縛の割り当て削減 | ~1.16x | 中間 Vec 3本＋defaults 1本＝毎コール小 Vec 4本の確保を除去 |
| ＋ R3 属性 IC | ~1.57x | フィールド読みごとの `format!` 確保＋`field_index` 走査＋辞書引き 2本を class_id 比較 1回に置換 |
| **＋ R4 呼び先解決** | **~1.69x** | 呼び出しごとの builtin 名照合＋名前引き＋`name.clone()` を global slot 直参照に置換 |

## 結論

- **解釈経路が全面的に 1.31〜1.83x 高速化**（E2E フィールド/メソッド 1.58〜1.69x、微ベンチ最大 1.83x、全テスト緑、回帰0）。
- 段階ごとの主因:
  - **R0 `frame_floor` ＋ 引数束縛 ＋ R4**: 関数呼び出しの per-call コスト（アロケーション＋名前引き）を削減（呼び出し 1.34x・引数1で 1.50x）。
  - **R3 属性 IC**: フィールドアクセスの per-read オーバーヘッド（`format!`＋走査＋辞書引き）を除去。フィールド支配コードで最大効果（4-field read 1.72x）。
  - **メソッド呼び出し IC**: メソッドディスパッチの補助 SipHash 引き 4本を除去（method call 1.62x、うち IC 分 1.24x）。
  - **R1 読み取り解決**: ローカル読み支配コードで上乗せ（`4-var` 1.27x）。上位レバーの土台。
- BYTECODE_VM_PLAN 投影の **Phase R 1.3〜2x を達成**（フィールド/呼び出し/メソッド支配で ~1.6-1.7x）。
- **まだツリーウォークのまま**の支配項: `Value` clone・命令ディスパッチ・算術・`class.methods.get` 本体の1回引き。

---

# Phase V-A — バイトコード VM スケルトン（デュアルモード）

BYTECODE_VM_PLAN §5 の Phase V の第一段（V-A）。解決済み AST をリーフ関数単位で Chunk に
コンパイルし専用スタックマシンで実行する。非対応構文はコンパイル時に弾いてツリーウォークへ
フォールバックする（D2 デュアルモード）。CLI `--vm=off|auto|force`（既定 auto）。

## 実装（[src/vm/](src/vm/)）
| ファイル | 役割 |
|---|---|
| `op.rs` | オペコード列挙（Const/LoadLocal/StoreLocal/Bin/Un/GetAttr/Jump 系/Return …） |
| `chunk.rs` | `Chunk { code, consts, names, attr_caches, n_locals }` |
| `compiler.rs` | 解決済み AST → Chunk。**トップレベルのリーフ関数**（呼び出し・メソッド・クロージャ・ローカル宣言・for/match/例外・可変長を含まない）だけをコンパイル、他は `None`（フォールバック） |
| `run.rs` | ディスパッチループ。値スタックは Interpreter の**使い回しバッファ**（per-call 確保なし）。int/float 算術・順序比較・public フィールド読み（R3 IC）を**ループ内インライン**、他は既存 `apply_binop_dyn`/`get_attr_val`/`eval_truthy` へ委譲（＝意味論一致） |
| `disasm.rs` | 逆アセンブラ（開発用） |
| `mod.rs` | `VmMode`（Off/Auto/Force）・公開 API |

統合: [functions/execution.rs](src/interpreter/functions/execution.rs) の `exec_fn_evaled` で、フリー関数
（self なし・クロージャなし・非 Python）を初回にコンパイルして `vm_chunks` にキャッシュ、以後 VM 実行。

## 検証
- `cargo test`（既定 `--vm=auto`）→ **672 passed / 0 failed**。VM がコンパイルした関数を全テストで実行し、
  期待結果と一致（＝ツリーウォークと同値）。デバッグビルドで VM GetAttr の slot 一致も検証。
- 例題回帰: 24 の決定的例で `--vm=off` と `--vm=auto` の**終了コード・stdout・エラー出力が完全一致**。
- `cargo build`/clippy 警告 0。

## 速度計測（同一 binary の `--vm=off` vs `--vm=auto`、best-of-8）
| 指標（VM がコンパイルする関数） | vm=off (µs) | vm=auto (µs) | VM 倍率 |
|---|---|---|---|
| let→let int | 0.761 | 0.712 | **1.07x** |
| let→let instance | 0.904 | 0.816 | **1.11x** |
| 4-field read | 1.035 | 0.934 | **1.11x** |
| subscript[0]（引数 use_small） | 1.075 | 1.004 | **1.07x** |
| fn call（`noop` 本体ほぼ空） | 0.369 | 0.388 | 0.95x |
| **E2E field access** | 1.727 s | 1.579 s | **1.09x** |

- **Phase R で高度に最適化されたツリーウォークに対しても VM が 1.07〜1.11x 上回る**（コンパイル対象の関数）。
  本体がほぼ空の `noop` だけは per-call オーバーヘッド（chunk キャッシュ引き・バッファ確保）で微減。
- 効いた要因: (1) 再帰 `eval()` 呼び出しの排除（線形ディスパッチ）、(2) int/float 算術と public フィールド読みの
  ループ内インライン（`apply_binop_dyn`/`get_attr_val` の関数呼び出し回避）、(3) 値スタックの使い回し。
- **累積効果**: E2E field は ツリーウォーク 1.69x（対 baseline）にさらに VM 1.09x で **~1.84x**。

## V-A の限界と次段（V-B〜V-F）
- コンパイル対象は「呼び出しを含まないリーフ関数」に限定。呼び出し・メソッド・ローカル宣言・for/match/
  例外・クロージャは未対応（フォールバック）。多くの実関数はまだツリーウォーク。
- 大きな伸びは V-F（superinstruction・型特化命令の全面化）と V-B/V-C（メソッド・クラス・例外テーブル・
  ブロック式）で、より多くの関数を VM に載せてから出る。V-A は**その土台が動作する実証**。

---

## 次に効くレバー
1. **Phase V-B/V-C** — 呼び出し（CALL op）・メソッド・ローカル宣言・制御フロー式・例外テーブルを VM に追加し、
   対象関数を拡大（V-A の骨格が土台）。
2. **`Value::Str(String)` → `Rc<str>`（§7.4 その2）** — 文字列読みごとのヒープ確保を refcount bump に。
   文字列多用ワークロード専用の効果。波及大につき別途。
3. **メソッド IC の完全化** — `class.methods` を FxHash 化 or slot 索引化し、ヒット時の残り 1 辞書引きも除去。

`LocalRef` / slot 化 `Scope` / `frame_floor` フレームモデル / bind_args 高速経路 / `AttrCache` / R4 呼び先キャッシュ / method IC / リゾルバは上記すべての土台として再利用される。
