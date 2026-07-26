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
| `compiler.rs` | 解決済み AST → Chunk。トップレベル関数を対象に、算術・フィールド読み・制御フロー・**ローカル宣言（let/mut/const）**・**関数呼び出し**を対応。メソッド呼び(Attr func)・for/match/例外・クロージャ・可変長・keyword 引数は `None`（フォールバック） |
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

### ローカル宣言（let/mut/const）対応
V-A に続き、ローカル宣言を VM に追加。exec_let / exec の const・mut と**完全に同一のセマンティクス**を
4種の store op で表現する（すべて既存 `deep_copy_value`/`apply_freeze_to_value` へ委譲＝結果一致）:
- **const** → `StoreLocal`（copy/freeze なし・全型）
- **mut** → `StoreLocalDeepCopy`（常に deep_copy・全型）
- **let（不変ソース / リテラル）** → `StoreLocal`（そのまま）
- **let（可変ソース）** → `StoreLocalCopyFreeze`（deep_copy + freeze）
- **let（非識別子式）** → `StoreLocalFreezeInstance`（Instance のときのみ deep_copy + freeze）

slot 採番はリゾルバと同順（パラメータ→トップレベル宣言）で `LocalRef` と一致。LetTuple/Static/入れ子定義など
slot をずらす形は丸ごとフォールバック。これでローカル変数を持つ数値関数が VM に載る。

## 速度計測（同一 binary の `--vm=off` vs `--vm=auto`、best-of-6〜8）
| 指標（VM がコンパイルする関数） | vm=off (µs) | vm=auto (µs) | VM 倍率 |
|---|---|---|---|
| **4-var declare+lookup**（`scope_lookup`: ローカル4宣言+読み） | 1.127 | 0.677 | **1.66x** |
| let→let instance | 0.904 | 0.816 | **1.11x** |
| 4-field read | 1.035 | 0.891 | **1.11x** |
| let→let int | 0.761 | 0.682 | **1.08x** |
| subscript[0]（引数 use_small） | 1.075 | 1.004 | **1.07x** |
| fn call（`noop` 本体ほぼ空） | 0.369 | 0.388 | 0.95x |
| **E2E field access** | 1.727 s | 1.579 s | **1.09x** |

- ローカル宣言対応で `scope_lookup`（let 4本 + 算術）が VM に載り **1.66x**（プリミティブ局所変数＋型特化算術＝VM の得意領域）。
  ツリーウォークの 4× exec_let（宣言チェック）+ 再帰 eval を線形バイトコードが置き換える。

### 関数呼び出し（CALL）対応
`func(args)` を VM に追加し、**非リーフ関数**（他の関数を呼ぶ関数）も VM に載るようにした。
- 呼び先: グローバル Ident → `LoadGlobal`（`scopes[0]` 解決＝呼び出し元スコープを跨がない）、ローカル関数値 → `LoadLocal`。
  純粋 builtin（print/len/range 等 15 個）と型コンストラクタ（int/str 等）は**コンパイル時に blocklist で弾く**（フォールバック）。
- 引数: 各引数の `is_mutable` を**コンパイル時に算出**（`eval_call_args` と同じ判定: LocalRef→slot 可変性、他 true）して
  `Op::Call(argc, mut_mask)` に載せる。ランタイムは正しいフラグ付きの評価済み引数で `call_value_evaled` へディスパッチ。
  → let 引数 → let パラメータの `===`（参照等値）まで含めツリーウォークと一致。
- 例外は `?` で伝播し、`exec_fn_evaled` の VM 経路が呼び出し元フレームを付加（トレースバック一致）。

**計測（非リーフベンチ: `compute`= helper 2回 + let 2本 + 算術）**: `--vm=off` 2.89 → `--vm=auto` 2.22 µs/iter = **1.30x**
（呼び先 `helper` も VM コンパイルされ両段バイトコード実行）。totals 一致・例外伝播一致・関数呼び中心の 23 例で
`off`/`auto` 出力完全一致。

### メソッド呼び出し（CallMethod）対応
`obj.method(args)` を VM に追加。`obj` が**型注釈でインスタンスと保証できる LocalRef** のときだけコンパイルする
（`is_user_instance_type`: 組み込み型・ジェネリック・Optional/union は除外。型検査が Instance を担保）。
- ランタイムは `call_instance_method_evaled`（`eval_method_call` の Instance アームと同一ディスパッチ:
  copy / gen（`exec_generator_evaled`）/ native / static・class 判定 / 不変性フィルタ / オーバーロード）を
  評価済み引数で実行。**method IC（class_id キャッシュ）も VM 内で使う**（chunk の `attr_caches` を流用）。
- 引数の `is_mutable` は CALL と同じくコンパイル時算出。gen メソッドのため `exec_generator` を
  `exec_generator_evaled` に薄くリファクタ。
- **計測（`work`= p.sum() 2回 + 算術）**: off 2.55 → auto 2.47 µs/iter = **1.03x**（メソッド本体はまだツリーウォーク
  ＝self 付き関数は未コンパイルなので上げ幅は小。呼び出し側 `work` の本体が VM 化）。OOP 例で off/auto 出力一致・
  全 672 テスト緑。メソッド本体自体の VM 化（self 対応）は次段。

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

# Phase V-B — メソッド本体の VM 化（self 対応）＋ 属性書き込み（SET_ATTR）

BYTECODE_VM_PLAN §5.4 の V-B。**メソッド本体そのものを VM にコンパイル**し、`self` フィールド読み・
`self` 変異・`self.method()` 間接呼び出しをバイトコードで実行する。V-A では呼び出し**側**の本体だけが
VM 化され、メソッド本体（`self` 付き関数）はツリーウォークだった（1.03x）ため、その最後の穴を塞ぐ。

## 設計の要点 — リゾルバを変えずに `Self` スロット問題を回避
メソッド本体をリゾルバで解決（`Ident`→`LocalRef`）しようとすると、ツリーウォークの base スコープに
`self` / params の後で宣言される `Self`（レシーバのクラス）が **base slot を1つ占める**ため、リゾルバの
slot 採番（`self`, params, body-decls…）と実行時レイアウト（`self`, params, **Self**, body-decls…）が
ずれる（`Self` が実行時に self_val の型で条件宣言されるため、コンパイル時に確定できない）。

→ **リゾルバはトップレベル関数のみのまま据え置き**、メソッド本体は**コンパイラの `Ident`→slot 機構**で
直接コンパイルする。コンパイラは `slots`（params + body 直下宣言）を自前で持ち、`Expr::Ident` を
`LoadLocal(slot)` に落とせる。`Self` はコンパイラ `slots` に無い（params/宣言ではない）ので、`Self`
参照は自動的に bail（フォールバック）。**VM モデルには `Self` slot が存在しない**ので不整合が起きない。
ツリーウォーク経路は未解決（名前引き）のまま＝正しさ不変。

## 実装
| 変更 | ファイル | 内容 |
|---|---|---|
| VM 経路をメソッドに開く | [functions/execution.rs](src/interpreter/functions/execution.rs) | `exec_fn_evaled` の VM ガードを `self_val.is_none()` から **`None` または `Some(Instance)`** に拡大（クラスメソッド等の非 Instance レシーバは除外）。`self` は `bind_args` で slot 0・compiler slot 0 に一致。実行前に **`current_class` を張り**（アクセス制御・`Self` 依存ディスパッチ）実行後に復元 |
| `self` をレシーバ判定 | [vm/compiler.rs](src/vm/compiler.rs) | `self_slot`（`self` パラメータの slot）を記録。`object_is_instance` が「`self` slot」または「ユーザークラス型注釈の LocalRef/Ident」を Instance と判定。`self.method()` / `self.field = …` のコンパイルを許可 |
| 呼び先 Ident の slot 優先 | [vm/compiler.rs](src/vm/compiler.rs) | 未解決メソッド本体では呼び先が `Ident` のまま来る。call 分岐を **slots 優先**（ローカル/param が関数値を保持）→ builtin/`Self` は bail → それ以外 `LoadGlobal` に修正（`Self(...)` コンストラクタ呼びが誤って `LoadGlobal` されるバグを解消） |
| `SetAttr` op | [vm/op.rs](src/vm/op.rs), [vm/run.rs](src/vm/run.rs) | `[obj, value]` を pop し `attr_assign_evaled(obj, name, value)` で代入。コンパイラは `self`/instance 受け手の side-effect-free 対象にのみ発行 |
| `Swap` op | [vm/op.rs](src/vm/op.rs), [vm/run.rs](src/vm/run.rs) | 複合属性代入で **rhs を先に評価**（ツリーウォークの評価順）しつつ演算オペランド順を保つためスタックトップ2つを入れ替える |
| `attr_assign_evaled` | [eval/attrs.rs](src/interpreter/eval/attrs.rs) | `attr_assign` の `Value::Instance` アーム（class-var 検査・static mut・アクセス制御・field_index・可変性・`INST_IMMUTABLE`＋`slot_initialized`・`store_field` 型検査）と**同一セマンティクス**を評価済み値で実行 |
| `AttrAssign`/`AttrCompoundAssign` | [vm/compiler.rs](src/vm/compiler.rs) | `self.x = v` → `[obj, value, SetAttr]`。`self.x op= v` → `[obj, value, obj, GetAttr, Swap, Bin, SetAttr]`（value 先行評価＝ツリーウォーク一致） |

## 検証
- `cargo test`（既定 `--vm=auto`）→ **672 passed / 0 failed**。全 OOP テスト（メソッド・アクセス制御・
  演算子オーバーロード・`__init__`・`Self(...)` コンストラクタ・NewType）が**メソッド本体を VM 実行**して
  期待結果と一致。デバッグビルドの GetAttr slot assert も発火せず。
  - 初回 2 件失敗（`Self(...)` を `LoadGlobal("Self")` にコンパイル → NameError）→ call 分岐の slots 優先化で解消。
- 例題回帰: classes / exceptions / basics / typing / collections / practical の決定的例で `--vm=off` と
  `--vm=auto` の**出力・終了コードが完全一致**（35+ 例、差分 0）。private/protected を実行時に読む例も一致
  ＝VM メソッド経路のアクセス制御（`current_class`）も無破壊。
- `cargo build` / 追加 clippy 警告 0。

## 速度計測（同一 binary の `--vm=off` vs `--vm=auto`、best-of-3、release）
| ベンチ | 内容 | off | auto | VM 倍率 |
|---|---|---|---|---|
| **method_hot** | オブジェクト固定・`v.norm_sq()`（→`self.dot(self)`：self 経由メソッド呼び + フィールド読み）を 400万回 | 11.37s | 10.02s | **1.13x** |
| **method_body** | 毎回 `Vec3()` 生成 + `scale`(SET_ATTR) + `bump`(複合代入) + `norm_sq` を 80万回 | 8.12s | 6.66s | **1.22x** |

- **V-A（1.03x）→ V-B で 1.13〜1.22x**。差分はメソッド本体の VM 化（GET_ATTR/SET_ATTR/算術のループ内インライン）。
- **変異多め（SET_ATTR + 複合代入）ワークロードで効果大**（1.22x）: V-A では `self.x = …` を含むメソッドは
  丸ごとフォールバックしていたが、V-B で本体が VM 化。
- 上げ幅が中程度なのは、メソッド**呼び出し機構**（`call_instance_method_evaled` の bind_args・copy 意味論・
  `current_class` 設定・バッファ確保）がまだツリーウォークのままで、小さいメソッド本体ではそこが支配的なため。
  これは §7.2 の投影どおり（メソッド支配＝呼び出し機構ボトルネック）で、V-F の superinstruction と
  呼び出しオーバーヘッド削減が次の伸びしろ。

## V-B の到達点
- **クラス/メソッドを含む実プログラムがバイトコード実行される**: メソッド本体（`self` フィールド読み・
  変異・`self.method()`）・`Self(...)` 以外のフリー関数呼び・ローカル宣言・制御フローが VM に載る。
- 未対応（フォールバック）: `Self` 参照（コンストラクタ/静的呼び）・クラスメソッド/静的メソッド本体・
  for/match/ブロック式・例外・クロージャ・可変長・添字・コレクションリテラル。

---

# Phase V-C（第1増分）— 制御フローのジャンプ化 ＋ ネスト局所変数の平坦 slot 化 ＋ match 文

BYTECODE_VM_PLAN §5.3/§5.4 の V-C。制御フローを実行時センチネル／スレッドローカルから
**コンパイル時ジャンプ**へ移す第一歩。本増分では **break/continue のジャンプ化**、
**ネストしたブロック内ローカル宣言の平坦 slot 割り当て（R0-B の全ローカル slot 化）**、**match 文**を実装。
例外テーブル（try/except/finally・raise）とブロック式（block_return/loop_yield）は次増分（下記「残り」）。

## 実装
| 変更 | ファイル | 内容 |
|---|---|---|
| **break/continue のジャンプ化** | [vm/compiler.rs](src/vm/compiler.rs) | Compiler に `loops: Vec<LoopCtx>`（continue 先＝while 条件先頭 / break 命令位置）を追加。`while` 進入時に push、`break`→末尾へバックパッチする `Jump`、`continue`→条件先頭への `Jump`。**Arrow の「break/continue が入れ子の if/match を貫通して外側ループへ届く」規則は絶対ジャンプで自然に成立**（スタックは文境界で平衡）。ループ外の break/continue は bail。**`LOOP_DEPTH` スレッドローカルは VM 経路では不要に** |
| **ネスト局所の平坦 slot 化（R0-B）** | [vm/compiler.rs](src/vm/compiler.rs) | `collect_nested_decls` を追加し、if/while/match のボディ内 `let`/`const`/`mut` にもフレーム内固定 slot を割り当て（再帰）。**トップレベル decl はリゾルバと同順で先に採番**し、ネスト decl はその上に積む（リゾルバはネスト名を解決しない＝Ident のまま・衝突しない）。シャドウ禁止＝同名は非同時生存なので slot 再利用は健全。**これまで「if/while 内で `let` する関数」は丸ごとフォールバックしていた** のが VM に載る（適用範囲が大幅拡大） |
| **match 文** | [vm/compiler.rs](src/vm/compiler.rs), [vm/op.rs](src/vm/op.rs), [vm/run.rs](src/vm/run.rs) | サブジェクトを temp slot に一度だけ評価し（`alloc_temp`/`free_temp` のスタック規律割り当て）、各アームを順に照合。`case v`→`LoadLocal(temp); <pat>; Bin(Eq); JumpIfFalse(next)`、`is T`→新 `IsType(name_idx)` op（`value_is_type` 委譲）。ワイルドカード `case _` は無条件。`exec_match_stmt` と同一意味論（`apply_binop_dyn(Eq)` 委譲・最初のマッチのみ・非マッチは fall-through） |
| temp slot 割り当て | [vm/compiler.rs](src/vm/compiler.rs) | `named_locals`/`temps_in_use` を追加。名前付き slot の上にスタック規律で temp を確保し、`n_locals`（フレーム総 slot 数）を高水位で拡張 |

## 検証
- `cargo test`（既定 `--vm=auto`）→ **672 passed / 0 failed**。break/continue・ネスト局所・match を
  含む関数を VM 実行して期待結果と一致（デバッグ assert も無発火）。
- 例題回帰: classes/exceptions/basics/typing/collections/practical の決定的例 **27 件** ＋ `_error` 例
  **18 件** で `--vm=off` と `--vm=auto` の出力・終了コードが一致。
  - **既知の非一致 1 件**（`exceptions/runtime_error.ar`）: 未捕捉例外のトレースバックで VM 経路が
    行番号・コンテキストを欠く（`File "", in compute`）。**これは V-A/V-B からの既存挙動**（stash で確認済み）で、
    VM の**行テーブル未実装**（§2.3・V-E の課題）に起因。V-C の回帰ではない。
- `cargo build` / 追加 clippy 警告 0。

## 速度計測（`--vm=off` vs `--vm=auto`、best-of-3、release）
| ベンチ | 内容 | off | auto | VM 倍率 |
|---|---|---|---|---|
| **control_flow** | `collatz_steps`（while + ネスト if/else 内 `let` + break）＋ `bucket`（match 4分岐）を 30万回 | 15.64s | 8.19s | **1.91x** |

- **1.91x**。これらの関数は **V-B まで丸ごとフォールバック**（ネスト内 `let`・match で bail）していたため
  off/auto ともツリーウォークだった。V-C で VM に載り、ほぼ 2倍。制御フロー支配コードは VM の得意領域
  （ジャンプ1命令・型特化算術・slot 直読み）で §7.2 投影（制御フロー 2〜5x）とも整合。

## V-C 第1増分の到達点と残り
- **到達**: ループ（break/continue 含む）・ネストした if/while/match とその中のローカル宣言を持つ関数が VM に載る。
  `LOOP_DEPTH` スレッドローカルは VM 経路で不要化。
- **残り（次増分）**:
  - **例外テーブル**（try/except/finally・raise）— VM ディスパッチループを Err 捕捉可能に再構成し
    ハンドラスタック（SETUP_TRY/POP_TRY）を導入。`RAISE_SENTINEL`／`current_exception`・finally・
    トレースバックフレーム蓄積をツリーウォークと byte-identical にするのが要（慎重に別増分で実施）。
  - **ブロック式**（`block:`/if/while 式 + `block_return`/`loop_yield`）— 値を産む式形。
    `BLOCK_YIELDS`/`BLOCK_RETURN_EXPECTED_TYPE` スレッドローカルの除去はここで。
  - スレッドローカル4本＋センチネル2種の**実削除**は、上記完了＋強制バイトコード（D2）時。

---

# Phase V-D — for ループ（GET_ITER/FOR_ITER）＋ 組み込み呼び出し（print/range/len）＋ Chunk キャッシュの健全化

BYTECODE_VM_PLAN §5.4 の V-D。for ループを VM に載せ、あわせて **for/print を含む関数を VM 化できるよう
共通組み込み（print/range/len）を VM 呼び出し可能に**した。実装中に **Chunk キャッシュのポインタ再利用
バグ（V-A からの潜在）を発見・修正**（テンプレートで顕在化）。

## 実装
| 変更 | ファイル | 内容 |
|---|---|---|
| **for ループ** | [vm/op.rs](src/vm/op.rs), [vm/run.rs](src/vm/run.rs), [vm/compiler.rs](src/vm/compiler.rs) | `GetIter`（iterable→イテレータ）＋ `ForIter(iter_slot, target_slot, exit_ip)`（`.next()` 呼び・`EndOfIteration` で exit へ・要素は target へ束縛）。イテレータは temp slot に保持。ループ変数は `collect_nested_decls` で平坦 slot 割り当て（可変）。break/continue は既存 LoopCtx で自然対応（continue→ForIter へ戻る／break→exit）。単一ターゲットのみ（タプルアンパックは bail） |
| イテレータ変換の共有 | [exec/control_flow.rs](src/interpreter/exec/control_flow.rs) | `exec_for_stmt` から `make_for_iterator`（List/FrozenList/Str/Set/Tuple/Generator/Instance(`__iter__`)/PyObject → イテレータ）を抽出し、ツリーウォークと VM `GetIter` で共有（意味論一致） |
| **Generator 高速パス** | [vm/run.rs](src/vm/run.rs) | `ForIter` は iterator が `Value::Generator`（range/list/str/…/`gen __iter__` の実体）なら index を**直接前進**（`eval_method_call` のディスパッチを丸ごと回避）。カスタム Instance イテレータのみ `.next()` フォールバック。**この高速パスが for の速度差の主因** |
| **共通組み込みの VM 呼び出し** | [eval/builtins.rs](src/interpreter/eval/builtins.rs), [vm/op.rs](src/vm/op.rs), [vm/run.rs](src/vm/run.rs), [vm/compiler.rs](src/vm/compiler.rs) | `print`/`range`/`len` を評価済み引数で呼ぶ `eval_builtin_evaled` を追加（`eval_builtin_ident_call` の対応アームと同一意味論）＋ `CallBuiltin(name_idx, argc)` op。コンパイラは `is_vm_builtin` かつローカル未シャドウのとき発行。**これまで `print` や `range` を含む関数（＝多数）が丸ごとフォールバックしていた** のを解消 |
| **Chunk キャッシュの健全化（バグ修正）** | [interpreter.rs](src/interpreter.rs), [functions/execution.rs](src/interpreter/functions/execution.rs) | `vm_chunks` の値を `(Weak<FnValue>, Option<Rc<Chunk>>)` に変更。ヒット時に `Weak::upgrade()` が失敗したら「そのアドレスが別 fn_val に再利用された」と判定して**再コンパイル**する。**テンプレート実体化（`instantiate_template*`）は呼び出しごとに一時 `Rc<FnValue>` を生成・破棄するため、`Rc::as_ptr` キーが再利用されて古い Chunk を誤用する潜在バグ（V-A 由来）を修正**。リークなし |

### 発見したバグ（重要）
`polymorphism.ar` で `--vm=off`/`auto` が相違（`AttributeError: 'NoneType' ... to_str`）。原因は
**テンプレート実体化が毎回一時的な `Rc<FnValue>` を作って捨てる**ため、解放アドレスが後続の別関数
（別テンプレート実体化やクラスメソッド）に再利用され、`Rc::as_ptr` キーの `vm_chunks` が**別関数の
Chunk を返して誤実行**していた。V-A から潜在していたが、V-D で対象関数が増え顕在化。`Weak` 検証で修正。

## 検証
- `cargo test`（`--vm=auto`）→ **672 passed / 0 failed**。
- 例題回帰: basics/collections/classes/typing/exceptions/practical/control_flow/functions の決定的例
  **43 件** ＋ `_error` 例 **18 件** で `--vm=off`/`--vm=auto` 一致。for（range/list/str/`gen __iter__`・
  ネスト・break/continue・early return）・print・テンプレート（`polymorphism.ar` 修正確認）を含む。
  - **既知の非一致 1 件**は `runtime_error.ar`（未捕捉例外トレースバックの行番号欠落＝VM 行テーブル未実装, V-E）。V-A 由来で V-D の回帰ではない。
- `cargo build` / 追加 clippy 警告 0。

## 速度計測（`--vm=off` vs `--vm=auto`、best-of-3〜4、release）
| ベンチ | 内容 | off | auto | VM 倍率 |
|---|---|---|---|---|
| **for** | `range` + ネスト for + break/continue + print を回す | 0.635s | 0.294s | **2.16x** |
| for-over-list | list パラメータを for 反復（`total += x*2-1`）600万回 | 2.054s | 0.915s | **2.24x** |
| **control_flow**（再測定） | V-C の bench。print/main も VM 化され更に伸びた | 11.14s | 4.58s | **2.43x** |

- **for ループは ~2.2x**。Generator 高速パス（index 直進）＋ ループ制御のジャンプ化＋型特化算術が効く。
- 当初 `range`/`print` が blocklist で bail し 0.99x だったが、`CallBuiltin` で解消 → 2.16x。
- **組み込みの VM 化で `main`/多くの実関数が VM に載る**ようになり、既存 bench も更に伸びた（control_flow 1.91→2.43x）。

## V-D の到達点と残り
- **到達**: for ループ（range/list/str/set/tuple/カスタム `__iter__`・break/continue・ネスト）と、
  print/range/len を含む関数が VM に載る。Chunk キャッシュがテンプレートでも健全。
- **残り（V-D 続き・別テーマ）**: テンプレート実体化の Chunk メモ化（現状は毎回再コンパイル、§2.2）・
  ジェネレータ本体の VM 化・async・import モジュール Chunk は未着手（大半は §5.4 V-D の別項で独立）。

---

# Phase V-C（第2増分）— 例外テーブル（try/except/finally・raise）

BYTECODE_VM_PLAN §5.3/§5.4 の V-C の本丸。**例外処理を VM のハンドラスタックで実装**し、
try/except・try/finally・raise・bare raise（再送出）を VM で実行できるようにした。
これに伴い **VM ディスパッチループを「1 命令実行 → 制御フロー enum を返す」形に再構成**し、
命令が返す `Err` をループ側で捕捉してハンドラへ回せるようにした。

## 実装
| 変更 | ファイル | 内容 |
|---|---|---|
| **ディスパッチループの再構成** | [vm/run.rs](src/vm/run.rs) | `run` の巨大 match を `exec_op(...) -> Result<Flow, String>`（`Flow = Next/Jump/Return`）へ分離。`run` は `exec_op` を呼び、`Ok(Flow)` は ip 制御、**`Err(e)` は VM のハンドラスタックへ回す**。既存 op のロジックは不変（`?` はそのまま `exec_op` から伝播）。 |
| **ハンドラスタック** | [vm/run.rs](src/vm/run.rs) | `run` が `Vec<Handler{ handler_ip, stack_len }>` を持つ。`Err` 時に最内ハンドラを pop → オペランドを try 進入時の深さへ巻き戻し → 例外値を push → landing pad へジャンプ。ハンドラ無し/変換不可なら伝播。ネストした関数呼び出しはそれぞれ独立の run/ハンドラスタックを持ち、callee の未捕捉例外は caller の Call op（`?`）→ caller のハンドラへ届く |
| **例外 op** | [vm/op.rs](src/vm/op.rs), [vm/run.rs](src/vm/run.rs) | `SetupTry(handler_ip)`/`PopTry`（ハンドラ push/pop）・`Raise(span_idx)`（`raise expr`）・`Reraise`（bare raise・no-match 再送出）・`Dup`（例外値を型照合で残す）・`ExcMatch(name_idx)`（`exc_matches` 委譲） |
| **例外ヘルパ（interpreter 側）** | [exec/exceptions_async.rs](src/interpreter/exec/exceptions_async.rs) | `vm_raise`（`exec_raise` と同一: span フィールド書込み＋フレーム＋current_exception 設定）・`vm_reraise`・`vm_take_raised`（RAISE_SENTINEL or 内部エラー→RaisedError、`exec_try` と同じ変換）・`vm_exc_matches`。`current_exception`/`RAISE_SENTINEL` は interpreter private なのでここに集約 |
| **Chunk に span 表** | [vm/chunk.rs](src/vm/chunk.rs) | `spans: Vec<Span>`（`Raise` が例外に file/line/col を焼くため） |
| **try/except・try/finally コンパイル** | [vm/compiler.rs](src/vm/compiler.rs) | `compile_try_except`（SetupTry→body→PopTry→正常 Jump／landing pad で各 except 節を `Dup;ExcMatch;JumpIfFalse` で照合・別名 slot 束縛・no-match は `Pop;Reraise`）。`compile_try_finally`（正常経路・例外経路の両方で finally を走らせ、例外経路は `Reraise` で再伝播）。`try/except/finally` 併用は bail |
| **脱出制御の bail 判定** | [vm/compiler.rs](src/vm/compiler.rs) | `has_escape`: try/handler 本体に「try を飛び越える」`break`/`continue`（本体内の while/for に囲まれない）・`block_return`/`loop_yield`（finally では `return` も）があれば bail（ハンドラ残り・finally スキップを防ぐ）。`return` は try/except（finally なし）では run から即復帰しハンドラ破棄されるので許容 |
| **別名 slot の事前採番** | [vm/compiler.rs](src/vm/compiler.rs) | `collect_nested_decls` を `Try` に対応（body/handler/finally へ再帰＋`except E as e` の別名を不変 slot として採番） |

## 検証
- `cargo test`（`--vm=auto`）→ **672 passed / 0 failed**。
- 手動 A/B（`--vm=off` vs `--vm=auto`）で以下が**完全一致**:
  try/except（ZeroDivisionError 捕捉）・`raise ValueError`＋`except ... as e`＋`e.message`・複数 except 節の
  順次照合・try/finally（正常経路 finally 実行）・ネスト try＋bare `raise` 再送出→外側捕捉・内部型エラーの捕捉。
  finally-on-exception（finally 実行後に外側が捕捉）・未捕捉例外の伝播も**結果一致**。
- 例題回帰: 決定的例 43 件で off/auto 一致（唯一の差は既知の `runtime_error.ar` トレースバック行番号）。
- `cargo build` / 追加 clippy 警告 0。

## 到達点と既知の制限
- **到達**: try/except・try/finally・raise・bare raise（再送出）を VM で実行。`RAISE_SENTINEL` センチネルは
  interpreter 境界の規約として残るが、**VM 内の例外制御はハンドラスタックのジャンプ**で行う（`LOOP_DEPTH` は
  既に不要化済み）。
- **未捕捉例外のトレースバック行番号**は依然として欠落（`File "", in <fn>`）。VM が op→span の**行テーブル**を
  持たないため（§2.3・V-E 課題）。**捕捉される例外（通常の try/except）では差は出ない**（トレースバック非表示）。
  V-A 由来の既存制限で、例外コンパイルにより顕在化する関数が増えた。
- **bail（ツリーウォークへ）**: `try/except/finally` 併用・try を飛び越える break/continue/block_return/
  loop_yield を含む try・finally 内 return。

- **bail（ツリーウォークへ）**: `try/except/finally` 併用・try を飛び越える break/continue/block_return/
  loop_yield を含む try・finally 内 return。

---

# Phase V-C（第3増分）— ブロック式（block:/if/while/for/match 式 ＋ block_return/loop_yield）

BYTECODE_VM_PLAN §5.4 の V-C 最後のピース。**値を産む制御構文（ブロック式）5 形**を VM にコンパイルし、
`block_return`（単一値で脱出）・`loop_yield`（for/while 式でリスト蓄積）を実装した。これで V-C が完了。

## 実装
| 変更 | ファイル | 内容 |
|---|---|---|
| **ブロック式コンテキスト** | [vm/compiler.rs](src/vm/compiler.rs) | `BlockCtx{ result_slot, end_jumps, yield_slot }` のスタック。`block_return` は最内 BlockCtx の `result_slot` に格納して出口へ跳ぶ。`loop_yield` は最内の「yield 先を持つ」BlockCtx（block:/for/while 式）の蓄積リストへ追加。**if/match 式は yield 透過**（`yield_slot=None`）＝外側の for/while/block へ届く（`eval_capture_block_return` の透過性に一致） |
| **5 つの式形** | [vm/compiler.rs](src/vm/compiler.rs) | `Expr::Block`（block_return 値／loop_yield リスト／None）・`IfExpr`・`MatchExpr`（分岐/アームの block_return 値・既定 None）・`ForExpr`・`WhileExpr`（loop_yield 蓄積・break で蓄積リスト・block_return で単一値・二出口）。for/while 式は LoopCtx（break→NORMAL_END/continue→先頭）と BlockCtx（block_return→BR_END）を併用 |
| **loop_yield 用 op** | [vm/op.rs](src/vm/op.rs), [vm/run.rs](src/vm/run.rs) | `BuildEmptyList`（蓄積リスト初期化）・`ListAppendLocal(slot)`（`loop_yield` 追加）・`ListOrNone`（蓄積が空なら None・非空ならリスト＝`eval` の `yields.is_empty()` 分岐に一致） |
| **式内宣言の slot 採番** | [vm/compiler.rs](src/vm/compiler.rs) | `collect_nested_decls` に**式ウォーカ `collect_expr_decls`** を追加。`let x = block: let a=…` のような**式の中のブロック式本体の宣言**へ再帰的に slot を割り当てる（全部分式を辿り入れ子ブロック式を漏れなく採番）。`add_decl` を自由関数へ抽出 |
| **脱出 bail 判定** | [vm/compiler.rs](src/vm/compiler.rs) | `block_body_bails`: ブロック式本体の `return`（常に不可）・非ループ式（block:/if/match）の脱出 break/continue を検出して bail。for/while 式は自身が最内ループなので直下 break/continue は許容（LoopCtx が処理） |

## 検証
- `cargo test`（`--vm=auto`）→ **672 passed / 0 failed**。
- 手動 A/B（`--vm=off` vs `--vm=auto`）で**完全一致**:
  `block:`＋block_return・block:＋if/elif/else 分岐 block_return（FizzBuzz）・if 式・match 式・
  for 式＋loop_yield（`[0,1,4,9,16]`）・while 式＋loop_yield・block_return なし block:（None）・
  **条件付き loop_yield**（if 内 yield）・**break で蓄積リスト返却**・**入れ子ブロック式**・
  **for 式内のネスト for 文**。型検査が弾く不正形（for 式直下の block_return 等）も両モードで同一エラー。
- 例題回帰: 決定的例 43 件で off/auto 一致（唯一の差は既知の `runtime_error.ar` トレースバック行番号）。
- `cargo build` / 追加 clippy 警告 0。

## 速度（`--vm=off` vs `--vm=auto`、best-of-3、release）
| ベンチ | 内容 | off | auto | VM 倍率 |
|---|---|---|---|---|
| **block_expr** | `classify`（block:+if 式 block_return）＋ `sum_squares`（for 式 loop_yield）を 20万回 | 2.90s | 1.06s | **2.72x** |

- **2.72x**。これらの関数は第2増分まで丸ごとフォールバックしていた。block_return/loop_yield のジャンプ化＋
  蓄積リストのインライン＋型特化算術で VM の得意領域。

## V-C 完了
- **到達**: 制御フロー（break/continue/match/例外/ブロック式）がすべて VM のジャンプ／ハンドラスタックで実行される。
  `LOOP_DEPTH` は VM 経路で不要化、`BLOCK_YIELDS`/`BLOCK_RETURN_EXPECTED_TYPE` は VM コンパイルされたブロック式では
  不使用（蓄積は `ListAppendLocal`、block_return は slot＋ジャンプ）。**スレッドローカル4本＋センチネル2種の
  “実削除”は強制バイトコード（D2）時**（デュアルモード中はツリーウォークが使い続けるため保持）。
- **bail（ツリーウォーク）**: 式内 return・非ループブロック式の脱出 break/continue・（既出の）try/except/finally 併用等。
- **トレースバック行番号**: V-E（下記）で解消済み。

---

# Phase V-E（部分）— VM 行テーブル（トレースバック行番号）＋ デバッグ名テーブル

BYTECODE_VM_PLAN §5.4 V-E の実利部分。**VM 関数のトレースバックの行番号欠落を解消**し、
`--vm=off` と `--vm=auto` の**全決定的例が byte-identical** になった（例外・ブロック式のコンパイルで
VM 化される関数が増え、未捕捉例外の degraded トレースバックが唯一の差として残っていた）。

## 実装
| 変更 | ファイル | 内容 |
|---|---|---|
| **呼び出し位置 span の伝搬** | [vm/op.rs](src/vm/op.rs), [vm/run.rs](src/vm/run.rs), [vm/compiler.rs](src/vm/compiler.rs) | `Op::Call(argc, mut_mask, **name_idx, span_idx**)` に呼び出し元名（`names`）と位置（`spans`）を追加。`Expr::Call.span` をコンパイル時に記録し、実行時に `call_value_evaled` へ渡す |
| **`call_value_evaled` の署名拡張** | [eval/calls.rs](src/interpreter/eval/calls.rs) | `fn_name: &str`・`call_span: Option<Span>` を受け取り `exec_fn_evaled` へ渡す（従来は `"<fn>"`/`None` 固定で degraded だった）。ネイティブ経路 `call_value_with_args` は `"<fn>"`/`None` 維持 |
| **`call_instance_method_evaled` の署名拡張** | [classes/method_call.rs](src/interpreter/classes/method_call.rs) | `call_span` を受け取れるように（内部 `exec_fn_evaled` へ伝搬）。**ただし VM は `None` を渡す** — ツリーウォークのメソッド呼び出し（`eval_method_call`）が call_span を渡さず degraded なので、**それに一致**させる（byte-identical 優先） |
| **デバッグ名テーブル** | [vm/chunk.rs](src/vm/chunk.rs), [vm/compiler.rs](src/vm/compiler.rs) | `Chunk.local_names: Vec<String>`（slot→変数名）を追加。デバッガ VM 統合（将来）用メタデータ。現状は保持のみ |

### 設計判断: メソッド呼び出しは “あえて degraded に一致”
関数呼び出しはツリーウォークが `call_span` を渡す（フレームに行番号あり）ので VM も渡す。
一方**メソッド呼び出しはツリーウォークが `call_span` を渡さない**（`eval_method_call` の設計）ため、
VM が span を渡すと**ツリーウォークより詳細なトレースバック**になり `off`≠`auto` になる。
byte-identical を優先し、VM のメソッド呼び出しも `call_span=None`（degraded）に合わせた
（ツリーウォーク自体の改善は別テーマ）。

## 検証
- `cargo test`（`--vm=auto`）→ **672 passed / 0 failed**。
- 例題回帰: **決定的例 44 件 ＋ `_error` 例 19 件 = 全 63 例で `--vm=off`/`--vm=auto` が完全一致**
  （V-E 前に唯一残っていた `runtime_error.ar` のトレースバック行番号差が解消）。
- 深いトレースバック（関数チェーン `run→use_widget→area`・ゼロ除算）でも関数フレームの行番号・
  コンテキスト・メソッドフレームの degraded 表示までツリーウォークと一致。
- `cargo build` / 追加 clippy 警告 0。

## V-E の到達点と残り
- **到達**: VM 関数の**未捕捉例外トレースバックがツリーウォークと byte-identical**。呼び出し位置の
  行番号・コンテキスト行が復元される。デバッグ名テーブルを Chunk に保持。
- **残り（V-E 本体）**: op→Span の**汎用行テーブル**（デバッガのステートメント単位ブレークポイント用）。
  トレースバック用途は本増分で満たされたので、残りはステップ実行の精密化時に実施。

---

# Phase V-E（続き）— デバッガ REPL のバイトコード実行（停止スコープ視点）

停止スコープの**生変数を名前で参照**しながら、デバッガ REPL 入力をバイトコードにコンパイルして
実行する経路を追加した（§2.5/§3.4-C の「動的名エスケープハッチ」を VM で実現）。

## 実装
| 変更 | ファイル | 内容 |
|---|---|---|
| **名前引き op** | [vm/op.rs](src/vm/op.rs), [vm/run.rs](src/vm/run.rs) | `LoadName(name_idx)`（`get_val` で停止スコープから名前解決）・`DeclareName(name_idx)`（`let dbg::x` を停止スコープへ宣言・`let` 意味論で Instance は deep_copy+freeze） |
| **VM 側ヘルパ** | [interpreter/scope.rs](src/interpreter/scope.rs) | `vm_load_name`（`get_val` 委譲）・`vm_declare_debug`。`get_val`/`declare_var` は module private なので pub(crate) 経由 |
| **デバッグモードコンパイラ** | [vm/compiler.rs](src/vm/compiler.rs) | `Compiler.debug_mode` を追加。true のとき `Expr::Ident`→`LoadName`、関数呼び先も名前引き。`compile_debug(stmt)`: 式文（値を `Return`）・`let/const dbg::name`（`DeclareName`）を対応。メソッド呼び出し・添字・制御フロー等は `None` |
| **REPL 統合** | [interpreter/debugger.rs](src/interpreter/debugger.rs) | `exec_debug_input` が `compile_debug` を試し、`run_debug_chunk`（共有バッファ・`base` からローカル確保）で VM 実行。**式の値を表示**（従来は Return のみ表示だったのを改善し `access: dbg::x` が機能）。コンパイル不能な入力はツリーウォーク（`eval`/`exec`）へフォールバック |

## 検証（対話デバッガに stdin パイプ）
- **停止スコープ視点**: 関数フレーム内で停止し、局所変数（`total`・引数 `p`/`nums`）を名前で参照して
  バイトコード評価（`frame_floor` 準拠の名前解決）。
- `x + y`→30・`p.x`→3・`dbg::t`→20 等、**式がバイトコード実行され値が表示**される。
- `let dbg::m = p.x + total` 宣言 → 後続 `dbg::m`→10 で参照。関数呼び出し `add(x, y)`→30 もバイトコード実行。
- **フォールバック**: メソッド呼び出し `p.sum()`→7・添字 `nums[1]`→200 はツリーウォークで正しく評価。
- `q`（resume）後、停止プログラムが正しい状態で継続（`r=107`）。
- `cargo test` → **672 passed / 0 failed**、通常実行の回帰なし（debug 経路は隔離）、clippy 0・build 警告 0。

## 到達点
- デバッガ REPL が**停止フレームの生変数を名前で参照しつつバイトコード実行**。式・`let dbg::`・
  関数呼び出しは VM、メソッド/添字/制御フローはツリーウォークへフォールバック。**§2.3 が求める
  slot→名前デバッグメタデータ（`Chunk.local_names`）と名前引きエスケープハッチが揃った**。
  （タスク #5 後は添字・コレクションリテラルもデバッガ REPL でバイトコード実行される。）

---

# タスク #5 — 添字 `obj[i]`・コレクションリテラル（list/tuple/set/dict）の VM 化

`obj[i]` の読み書きと list/tuple/set/dict リテラルを VM にコンパイルする。実プログラムで頻出のため
VM 化率が上がり、デバッガ REPL のフォールバックも減る。

## 実装
| 変更 | ファイル | 内容 |
|---|---|---|
| **op** | [vm/op.rs](src/vm/op.rs), [vm/run.rs](src/vm/run.rs) | `Subscript`（pop key/obj → `eval_subscript`）・`SetIndex`（pop value/key/obj → `eval_setitem`）・`BuildList/BuildTuple/BuildSet/BuildDict(N)`（末尾 N（辞書は 2N）要素を pop して構築） |
| **構築ヘルパ** | [eval/core.rs](src/interpreter/eval/core.rs) | `vm_build_list/tuple/set/dict`（`Expr::List/Tuple/Set/Dict` と同一意味論・tuple は要素型名収集・set は `set_insert` 重複排除・dict は `[k0,v0,..]` フラット列） |
| **コンパイラ** | [vm/compiler.rs](src/vm/compiler.rs) | `Expr::Subscript/List/Tuple/Set/Dict` を対応。`obj[i] = value` は `Stmt::AttrAssign` の Subscript 分岐で対応（**value を temp に先評価**しツリーウォークの評価順に一致） |
| **シャドウ検出 bail** | [vm/compiler.rs](src/vm/compiler.rs) | `has_for_target_shadow`: `for` 変数が param/非 for 宣言をシャドウする関数は bail（下記） |

## `for` 変数シャドウの扱い（重要な健全性判断）
Arrow の `for` 変数は**ブロックスコープ**（ループ後に外側の同名変数へ戻る）だが、flat-slot VM は同名 slot を
再利用するため、`mut i = 0; for i in ...: nums[i] = i*i` のような**シャドウ**でツリーウォークと挙動が食い違う
（ツリーウォークはリゾルバが外側 slot を読み `nums[i]` が古い `i` を使う；VM は slot 再利用でループ値を使う）。
→ タスク #5 で添字代入がコンパイル可能になり、この既存の不整合が顕在化。**`for` 変数が param/非 for 宣言と
名前衝突する関数は丸ごと bail**（`has_for_target_shadow` で検出）してツリーウォークへ委譲し byte-identical を維持。
`for j`（新規名）や兄弟 `for i; for i`（外側シャドウなし）は対象外＝コンパイルされる。

## 検証
- `cargo test` → **672 passed / 0 failed**。
- 例題回帰: **決定的例 44 件 ＋ `_error` 例 19 件 = 全 63 例で off/auto 完全一致**。
  添字読み書き・多次元（`grid[1][0]=100`）・dict 更新・set 重複排除・for 式内 loop_yield・シャドウ bail を含む。
- デバッガ REPL でも `nums[1]`→200・`[total, total*2]`→`[7,14]` がバイトコード実行される。
- `cargo build` / 追加 clippy 警告 0。

## 速度（`--vm=off` vs `--vm=auto`、best-of-3、release）
| ベンチ | 内容 | off | auto | VM 倍率 |
|---|---|---|---|---|
| **collections** | `dot`（list 添字読み）＋ `build_pairs`（tuple リテラル + 添字）を 100万回 | 12.80s | 8.51s | **1.50x** |

---

# タスク #4 — メソッド呼び出し機構の軽量化（高速バインド経路）

V-B のメソッド呼び出しは `exec_fn_evaled` の一般経路（`bind_args` で Vec 確保・パラメータ名 clone・
`fn_val.params.clone()`・copy/cast の複数パス）を毎回通り、小さいメソッド本体では per-call オーバーヘッドが
支配的だった（method_hot 1.13x 止まり）。**単純シグネチャの VM 呼び出しに高速バインド経路**を追加。

## 実装（[functions/execution.rs](src/interpreter/functions/execution.rs)）
| ヘルパ | 内容 |
|---|---|
| `get_or_compile_chunk` | Chunk 取得/コンパイルを抽出（`exec_fn_evaled` の先頭で1回。fast/general 両経路で共有） |
| `try_fast_bind` | **単純シグネチャ**（可変長・デフォルト・キーワード引数なし／実引数数一致／キャスト不要）なら `bind_args` を介さず引数を**直接バッファへ束縛**。self 非 mut → deep_copy、let パラメータ + mut 引数 → copy_value（**コピー意味論は bind_args + copy ループと完全一致**）。外れたら `Ok(None)` で一般経路へ |
| `run_vm_method` | バインド済みバッファで `run` を実行し `current_class` 設定・例外フレーム組み立てを共通化（fast/general 共有） |
| 一般経路の軽量化 | cast パスの `fn_val.params.clone()` を借用に変更（毎コールの Param Vec clone を除去） |

## 健全性判断: self deep_copy は省略しない
「self を変異しない本体（SetAttr/Call/CallMethod なし）なら deep_copy を省いて Rc 共有」を検討したが、
**deep_copy は変異だけでなく“self（や self の可変フィールド）を戻り値としてエイリアスさせない”役割も持つ**
（例 `fn get(self)->list: return self.x`）。escape 解析が要り健全に省けないため**不採用**（self は従来どおり deep_copy）。

## 検証
- `cargo test` → **672 passed / 0 failed**。
- 例題回帰: **全 63 例で off/auto 完全一致**。デフォルト引数・`__cast__` 自動キャスト・キーワード引数・
  mut パラメータ・オーバーロードが fast-bind を正しく bail して一般経路で処理されることを確認。
- `cargo build` / 追加 clippy 警告 0。

## 速度（`--vm=off` vs `--vm=auto`、best-of-3〜4、release）
| ベンチ | V-B | 本タスク後 |
|---|---|---|
| **method_hot**（`v.norm_sq()`→`self.dot(self)`） | 1.13x | **1.61x** |
| **method_body**（`Vec3()` 生成 + scale/bump/norm_sq） | 1.22x | **1.64x** |

- bind_args の Vec 確保・パラメータ名 String clone・`params.clone()`・copy/cast の各パスを飛ばしたことで
  小さいメソッド本体の per-call オーバーヘッドが縮小。self deep_copy（意味論上必須）は残る。

---

# タスク #6 — その他組み込み（enumerate/zip/next/repr/id/getenv ＋ 型コンストラクタ）の VM 化

V-D では `print`/`range`/`len` のみが `CallBuiltin` で VM 実行され、それ以外の組み込み・型コンストラクタを
含む関数は丸ごとフォールバックしていた。本タスクで **純粋組み込み6種**と**登録済み型コンストラクタ**を
VM 経路に載せ、対象関数を大幅に拡大した。

## 実装
| 変更 | ファイル | 内容 |
|---|---|---|
| **純粋組み込みの拡張** | [eval/builtins.rs](src/interpreter/eval/builtins.rs) | `eval_builtin_evaled` に `next`/`repr`/`id`/`enumerate`/`zip`/`getenv` を追加（`eval_builtin_ident_call` の対応アームと**同一意味論**）。`enumerate`/`zip` は**コアを共有ヘルパ `enumerate_core`/`zip_core` に抽出**し、CallArg 版（ツリーウォーク）と評価済み版（VM）が同一実装を呼ぶ形にして意味論の分岐を封じた |
| **is_vm_builtin 拡張** | [vm/compiler.rs](src/vm/compiler.rs) | `CallBuiltin` を発行する純粋組み込み集合を `print`/`range`/`len` から6種追加。キーワード/可変長引数は `compile_call_args` が bail するので、位置引数の形だけが `CallBuiltin` になる |
| **型コンストラクタは LoadGlobal+Call に開放** | [vm/compiler.rs](src/vm/compiler.rs) | `is_builtin_callee`（bail 集合）から**登録済み型コンストラクタ**（int/uint/str/float/complex/bool/dict/set/function/slice）を除外。これらは通常のグローバル呼び出し（`LoadGlobal`+`Call`）に流れ、`call_value_evaled` の `Value::Type` アーム＝`call_type_by_name_evaled` へ委譲される（ツリーウォークの `eval_type_constructor_call` と同一経路）。**`CallBuiltin` を使わない**理由: ユーザーが同名をグローバル shadow した場合も `LoadGlobal` が実バインディングを拾うので健全（`CallBuiltin` だと組み込みが常に勝ってしまう） |

## 健全性判断: 型コンストラクタは `CallBuiltin` にしない
当初 int/str 等を `is_vm_builtin` に入れて `CallBuiltin`→`call_type_by_name_evaled` へ委譲したが、
**`list`/`tuple`/`type`/`byte` は `Value::Type` グローバルとして未登録**（ツリーウォークでは `list("abc")` が
`NameError`）なのに VM だけ成功して**分岐**が生じた。修正: 型コンストラクタは `LoadGlobal`+`Call` に流す。
- 登録済み（int/str/…）→ `LoadGlobal` が `Value::Type` を解決 → `call_type_by_name_evaled`（ツリーウォーク一致）。
- 未登録（list/tuple/type/byte）→ `is_builtin_callee` に残して bail。`LoadGlobal` の欠落名は `NameError: '{name}' is not defined`
  でツリーウォークと同一だが、確実性のため bail してツリーウォークに委ねる。
- ローカル shadow（`let str = …`）はコンパイラの `slots` 判定が `LoadLocal` を優先＝ツリーウォーク一致。

## 検証
- `cargo test`（`--vm=auto`）→ **672 passed / 0 failed**。
- 手動 A/B（`--vm=off` vs `--vm=auto`）で**完全一致**: enumerate/zip の for 反復・`str(42)`/`int("7")`/`float("3.5")`/
  `bool(0)`/`complex(2,3)` 型コンストラクタ・`repr`・`set([...])` 重複排除・`next(custom_iter)`・`getenv` フォールバック・
  **エラーパス一致**（`int("xyz")`→ValueError・`list("abc")`→NameError がトレースバック含め両モード同一）。
- 例題回帰: basics/collections/typing/exceptions の決定的例 **35 件**＋組み込み利用例で off/auto 一致。`--vm=force` で
  対象関数が実際に VM コンパイルされる（bail していない）ことを確認。
- `cargo build` / 追加 clippy 警告 0。

## 速度（`--vm=off` vs `--vm=auto`、best-of-2、release）
| ベンチ | 内容 | off | auto | VM 倍率 |
|---|---|---|---|---|
| **builtins**（enumerate+zip 反復の `work` を 40万回） | for enumerate/zip + 算術 | 5.79s | 3.88s | **1.49x** |

- **1.49x**。enumerate/zip を含む `work` は V-D まで丸ごとフォールバックしていたのが VM に載り、for の Generator 高速パス＋
  型特化算術＋ループ制御ジャンプ化が効く。

---

## 次に効くレバー
1. ~~その他組み込み（enumerate/zip/str/int 等）の VM 化~~ 【✅ 完了】（純粋6種＋型コンストラクタを LoadGlobal+Call/CallBuiltin で対象化, 1.49x）。
3. **V-F 最適化** — peephole・superinstruction・単型算術命令・R0-A エスケープ解析。
4. **`Value::Str(String)` → `Rc<str>`（§7.4 その2）** — 文字列読みごとのヒープ確保を refcount bump に。
5. **強制バイトコード（D2）への移行** — 全構文カバー後にツリーウォークのフォールバックを撤去し
   スレッドローカル4本＋センチネル2種を実削除。

`LocalRef` / slot 化 `Scope` / `frame_floor` / bind_args 高速経路 / `AttrCache` / R4 呼び先 / method IC / リゾルバ / VM 骨格（op/chunk/run）/ break-continue ジャンプ / 平坦 slot / match / for（GetIter/ForIter）/ CallBuiltin / Weak 検証 Chunk キャッシュ / 例外ハンドラスタック / ブロック式 BlockCtx / 呼び出し位置 span は上記すべての土台として再利用される。
