# 実行方式改善計画 — AST 解決層（Phase R）＋ バイトコード VM（Phase V）

Arrow（LLVM IR ターゲットのスクリプト言語, Rust 実装 `src/`）の**非ネイティブ実行経路**を高速化する計画。
現状の「AST ツリーウォーク」を、`AST → 解決層 → バイトコード VM` に置き換える。

> **この文書は単独で実装再開できることを目的とする**（`REFACTORING_HANDOFF.md` / `PHASE5_PLAN.md` と同じ引き継ぎ文書）。
>
> **読み方（トークン節約）**: まず冒頭の **🚩 次スレッドへの引き継ぎ**（現在地・次の候補・検証手順・落とし穴）。
> それだけで再開できる。背景が要るときのみ「実装状況」→ §0 → §2 → §4/§5 → §10。
> ページ後半の `═══ 参照・根拠 ═══` 以降（§6 動的リンク仕様・§3 アーキテクチャ決定・§1 背景実測・§7 投影・
> §8 非目標・§9 未決）は**決定済みの根拠／履歴**であり、通常は**読まなくてよい**。
> 進捗の一次資料は [PHASE_R1_RESULTS.md](PHASE_R1_RESULTS.md)。

> ## ✍️ この文書の記載規約（**更新時は必ず守る**）
>
> 1. **実装した内容の詳細をここに書かない。** 書くのは「**実装したという事実**」と「**その手法**」だけを
>    **一文以内（できれば語句）**で。表の 1 行に収まる粒度にする。
> 2. **詳細は切り分け先へ**: 実装の経緯・実測値・判断の根拠・調査結果は
>    [IMPLEMENTATION_LOG.md](IMPLEMENTATION_LOG.md) に追記する。この文書からは参照しない（リンクのみ）。
> 3. **必読部（先頭〜`═══ 参照・根拠 ═══` の手前）は 500 行以内**に保つ。可能な限り短くする。
>    超えそうなら、確定済み・履歴・保留タスクの仕様を区切り線の下か実装ログへ移す。
> 4. **残タスクには依存関係を必ず付記する**（「前提」が空欄＝今すぐ着手できる、を一目で分かるように）。

---

## 🚩 次スレッドへの引き継ぎ（2026-08-21・**ここだけ読めば再開できる**）

### 現在地
🎉 **本系列の主目的は完了した**。Phase R（AST 解決層）・Phase V（バイトコード VM）に加え、
**解釈実行はバイトコード VM 一本**になった（#33 でツリーウォークの制御フロー・TLS・
センチネル・`--vm` フラグを削除。**src 実質 -762 行**）。ツリーウォークが実行するのは
**定義文だけ**（設計どおり・#10-d／#55 で `eval()` もほぼ 0 と実測）。

**exec の中の速度は打ち止め**（#24/#46 を実測して却下）。残る速度は **#50 の分布から起票した
69（`interp_init`）・70（最上位ループ）＝ どちらも「exec の外／載り方」の話**。
保守性は**第 2 弾（#58〜#68）を 2026-08-21 に起票**し、**#68・#71（実バグ）・#66・#58・#59・#63・#62・#61・#69・#64・#60 が完了**。
直近の到達点は 1 行ずつ（詳細は
[IMPLEMENTATION_LOG.md](IMPLEMENTATION_LOG.md) の同番号）:

| # | 到達点（**事実と手法のみ**） |
|---|---|
| 47 | master との**端点 A/B を実行モード別に実測**（[ab_bench_modes.ps1](ab_bench_modes.ps1) 新設）。解釈 **3.97x** ／ native の境界・コールバック **1.60x** ／ 純ネイティブ **1.00x**（＝負の対照）／ C DLL は見かけ 2.18x でも **FFI 自体は 1.15〜1.23x** |
| 48 | #47 で出た実バグを修正 — native の `mut` ポインタ書き戻しを**コンパイル時に決めて副表（node_id キー）へ**。副産物で cdll 呼び出しが **1.13〜1.16x** |
| 49 | [debug_session.ps1](debug_session.ps1) の stdin BOM 混入を解消（**golden 無変更で 5/5 identical**） |
| 50 | **段別＋op 別の実行時間分布**を実測（[prof_dist.ps1](prof_dist.ps1)・`--features prof` 新設） |
| 57 | [stale_doc_refs.ps1](stale_doc_refs.ps1) の**再発 14 件を 0 に**（改名の取り残し／マーカー語／`$whitelist`）。**負の対照で検知力も確認** |
| 68 | 関数本体の `enum` が `VmForceError` だった実バグを修正 — `build_enum_classes` で**組み立てと記憶域を分離**し `Op::EnumDef` が `Name` を slot へ。副産物で **#59 のドリフトが実害として発火**（`collect_declared_names` に `EnumDef` を追加）。[compare_bytecode.ps1](compare_bytecode.ps1) を新設 |
| 60 | `parse_struct_bodies` を早期 continue へ反転（**103→60 行・ネスト 11→3**）＋ `scan_scope`（**82→49 行・ネスト 7→3**）。⚠⚠ **起票時の行数 307/405 は誤り**（診断が**文字リテラルを潰さず**ブレース平衡を崩していた）。実ヘッダ 6 本の全パース結果が **byte-identical**。⚠ 実ヘッダ検査が「VECTOR が居る」だけの空検査だったので件数・レイアウトを固定 |
| 64 | `gen_expr_inner` の `Expr::Call` を切り出し（**558→414 行・ネスト 9→6**）。⚠⚠ **統合はしない**と判断 — `gen_call` とは「2 実装」ではなく **float 直返し ↔ ハンドル＋アリーナの 2 つの ABI**。重複していたのは**判断 4 種・計 11 箇所**でそこだけ畳んだ。**IR 6/6 byte-identical** |
| 71 | **`import[py]` の関数が 1 つも呼べなかった実バグを修正**（`vm_eligible` の `!is_python` が #33 で `VmForceError` に化けていた）。本体は `python_converter` が作る**普通の Arrow AST**なので載る。⚠ `try_fast_bind` も塞ぐ（copy 規則が 2 経路でずれる）。**CPython と 1 行ずつ突き合わせ**。差分は bytecode 109/110・outputs 92/93 の**修正 1 件だけ** |
| 69 | `ar_config.json` の祖先ウォークを**遅延化**（`OnceCell` ＋ 起点だけ記憶）。`interp_init` **0.378→0.172 ms**・`ar_config_setup` **0.19〜0.21→0.000 ms**（repo 外の深い階層でも同じ）。⚠ **first-touch 仮説は棄却**。⚠⚠ **踏む例題が 0 本だった**ので 2 本新設（読み取りの実装は**2 つある**）。副産物で **`--features prof` が #68 以来壊れていた**のを修正 |
| 61 | `run_program` の設定探索を `load_python_search_paths` へ（**195→173 行・ネスト 9→5**）。⚠ 挿入位置を誤って `run_program` の doc を orphan 化し clippy +2 → 置き直して復帰 |
| 62 | `compile_stmt` **817→371 行**（最大アーム 97→34）。代入族を [stmt_assign.rs](src/vm/compiler/stmt_assign.rs) へ分離し、`AttrCompoundAssign` の重複アームを統合。**bytecode 108/108 byte-identical**。⚠⚠ **偽の 6.5% 退行**を出したが、**`mod` 宣言の順序を逆にしただけのプローブが同じ 0.929x を再現**して棄却 |
| 63 | `eval_method_call_full` の委譲漏れ 4 型を `*_methods.rs` へ（**1 レシーバ = 1 ファイル**）。**530→205 行**・最大ネスト **10→6**。⚠⚠ **bytecode は自明に一致する**ので証拠にならず、[compare_outputs.ps1](compare_outputs.ps1)（全例題の stdout/stderr/exit）を新設 |
| 59 | 「**この文はどの名前を束縛するか**」を [decl_names.rs](src/decl_names.rs) の 1 箇所へ（`each_declared_name` ＋ `DeclOrigin`）。**exhaustive match 2 段で「足したら壊れる」形に**し、walker 4 本を委譲（4 本は理由を確かめて残置）。⚠ **負の対照で検知力を確認**（`DeclOrigin` +1 → 消費者 4 本ちょうど停止／`Stmt` +1 → `decl_names` が停止） |
| 58 | `exec` の `Stmt::Import` アーム **210 行 → 8 行の委譲**（`exec_import` ＋ 補助 7 本を `exec/modules.rs` へ）。`exec` **394→192 行**・最大ネスト **11→5**。逐語重複 2 件を畳み、探索順が違う 2 本は**畳まない理由を明記**。⚠ **cs-proc を見る差分ゲートが 0 個**だったので [compare_import_paths.ps1](compare_import_paths.ps1) を新設 |
| 66 | `Compiler` の `Chunk` 複製 17 フィールドを廃止 — **`Compiler` が `Chunk` を直接組み立てる**（`chunk: Chunk`）。フィールド **37→21**・`into_chunk` **21→6 行**・`Chunk` にフィールドを足すとき直す箇所 **4→1**。[compare_bytecode.ps1](compare_bytecode.ps1) **108/108 byte-identical** |
| 51〜56 | 保守性レーン全 6 件完了（陳腐化コメント一掃＋[stale_doc_refs.ps1](stale_doc_refs.ps1) 新設／`CompileMode` 導入／`compiler.rs` を 10 モジュールへ分割／typed ABI 呼び出しの 1 本化／`eval()` の生死判定／**`parse_ar` の復活**） |

⚠⚠ **#50 で狙い所が変わった。** exec は 3.97x でも、例題 1 本の中央値では
**exec は全体の 14%**（0.46ms / 3.40ms）しかなく、**exec を無限に速くしても端点は 1.59x**。
残りはプロセス費用 1.5ms・`interp_init` 0.32・`type_check` 0.18・`parse` 0.17
＝ **master と同じまま残っている段**。⇒ **次に速度をやるなら「exec 以外」を測ってから決める**。

⚠ **同じコードでも書く場所で速さが違う**（#47）: 最上位は fn 内より改善が薄い
（baseline 1.62x ↔ 5.90x）＝ **VM に載っている ≠ 同じ速さ**。

> **✅ 全ゲート緑（2026-08-21 に自分で走らせて確認）**
> `cargo test` **742** ／ `cargo build` **警告 0** ／ `cargo clippy` **51 件（増分 0）** ／
> [scan_examples.ps1](scan_examples.ps1) **FAIL 0** ／ [force_gate.ps1](force_gate.ps1) **0 件・153 例題** ／
> [compare_python_impl.ps1](compare_python_impl.ps1) **51/51** ／ [repl_session.ps1](repl_session.ps1) **identical** ／
> [debug_session.ps1](debug_session.ps1) **5 identical** ／ [stale_doc_refs.ps1](stale_doc_refs.ps1) **0 件** ／
> `tw_stats` は `in_fn` 0・`tw_control_flow` 0・bail 0。
> ⚠ **`cargo clippy` と `cargo clippy --all-targets` は別の数**（51 ↔ 64。差は `benches/`）。
> **基準値を書くときは必ずコマンドごと書く**（§10 の指示と 52 件が食い違っていた）。
> ⚠⚠ **この「全ゲート緑」の状態で #68 の実バグが生きていた**（`enum` in fn）。
> **緑は「例題が書いている形については緑」という意味**でしかない。

⚠⚠ **ゲートを作っただけでは陳腐化は止まらない**（#51 → #57・内訳は実装ログ）。
#51 が 61→0 にした [stale_doc_refs.ps1](stale_doc_refs.ps1) は**直後の 4 タスクで 14 件に戻った**
（#52/#53/#56 が**走らせなかった**）。⇒ **改名・削除をしたら必ず走らせる**（5 点セットに明記）。
⚠ **マーカー語は行単位で効く** — 足すと**同じ行の他の識別子も検査から外れる**（黙らせない）。

**⚠⚠ #33 の前提は 2 度崩れた**（この系列で最も再利用価値のある教訓）。
1 度目: 「TLS は `--vm=off` のためだけに生きている」→ **`Default` が `Off`** なので REPL と
単体テストも踏んでいた（→ #36）。加えて `--vm=on` で動かない正しいプログラムが 5 種あった
（→ #34/#35/#37/#39/#40）。
2 度目: それらを全部潰した後でも生きていた — **定義文脈の式**（→ #41）と
**import モジュール本体**（→ #42）。
⇒ **`force_gate` 0 件・`tw_control_flow` 0 は毎回「例題がその形を書いているか」に依存していた。**
⇒ 潰すたびに**その形の例題を新設**した（これが 3 度目を防いだ）。
⚠ **3 度目は #56 で判明した** — `is_builtin_callee` の bail が #33 以来 `parse_ar` を
**完全に殺していた**（`VmForceError` で停止）。`force_gate` も例題も気づかず、
**#55 の計測で偶然出た**（`parse_ar` を使う例題が 1 本も無かったため）。
⇒ **「bail＝ツリーウォークへ落とす」という前提が #33 で消えたのに、表と doc がそのまま残っていた**
＝ 前提が変わったとき、**それに依存していた側を機械的に洗い出す手段が無い**のが根本。
⚠⚠ **5 度目は #71**（修正済み）— **`import[py]` の関数が 1 つも呼べなかった**
（`vm_eligible = !is_python` が「ツリーウォークで実行」から「**死ぬ**」に化けていた）。
⚠⚠ **4 度目は #68**（修正済み）— 関数本体の `enum`。**またしても「例題が 1 本も無かった」**。
⇒ **`force_gate` 0 件・全例題 緑は「0 である」ことの証明ではない**。**未カバーの構文を
例題側から数える手段がまだ無い**（`Stmt` の全 variant × 文脈のマトリクスが存在しない）。

### 設計上の教訓（**再利用する知識**。各タスクの経緯は [IMPLEMENTATION_LOG.md](IMPLEMENTATION_LOG.md)）

**計測と判断**
- **推測せず先に計測する**。本系列で見積もりは何度も外れた（一覧は実装ログ末尾の表）。
  直近では **#27-d の「クロージャ＝フレーム表現の変更が必須」が言い過ぎ**（半分は可変キャプチャ無し）、
  **#32 の「async 本体は 17 文だけ＝些細」が誤り**（本体のループは全反復ツリーウォークで 3.77x 遅い）、
  **#3 の「TLS 4 本を消す」の前提が古い**（2 つは VM が使用中）だった。
- **⚠⚠ この規模の変更は VM 支配ベンチを ±5% 揺らす**（#28 は却下。稀な op 7 個を 1 アームに畳んでも
  **何も回復せず** 1 件は悪化 ⇒ 効くのは**アーム数ではなくコード配置**）。**数 % で良し悪しを決めない**。
  判断材料は「**同じ 2 バイナリで測り直しても同じ差が出るか**」と「**変更と同規模**のプローブとの
  比較」の 2 つだけ。⚠⚠ **「再現したから実差」は誤り**（#62）— `flat_bench.ar` は **`mod` 宣言の
  順序を逆にしただけ**で **0.929x** 動く（既定 `codegen-units=16`。潰す順序は skill `vm-pitfalls` §1）（⚠ `--vm=off` での切り分けは #33 で失われた）。
- **`force_gate` は例題ごとに最初の 1 件で止まる**（1 例題 = 1 原因ではない）。潰すたび測り直す。
  文言だけで bail と `vm_ineligible` は区別できないので `AR_TW_STATS` の両表を突き合わせる。
- **⚠ `--vm=off` は #33 で削除した**（`compare_vm_modes.ps1` / `ab_bench_vm.ps1` も同時に削除）。
  差分検査は [compare_python_impl.ps1](compare_python_impl.ps1)（参照実装との突き合わせ）が担う。
  **bail する形はツリーウォークが正しいとは限らない**（`for-target-shadow` で実バグ。
  基準は `python -m impl_python` の出力）。

**健全性の原則**
- **注釈（`res`・型）は最適化ヒントであって意味論の根拠ではない**（#15e）。破ると
  「同じコードが書かれた場所で挙動を変える」（**実バグ 5 件**）。
  ⚠ `Resolution::Unresolved` は「シャドウが無い」を意味しない。判定は**実際の束縛／出自**を見る。
- **同じ判断をする 2 実装は片方を委譲にして畳む**（#22 系列）。畳めないものは**不変条件をテストで固定**。
  ⚠ **`*_evaled` 版とずれた実装を作らない** — 実バグ 4 回。
- **⚠⚠ 採番はリゾルバと同順・同数**。`push_base` される名前は `slots` に入れなくても
  **必ず 1 slot 消費する**（`static` で飛ばして `LoadLocal` が範囲外を読んだ）。
- **同じ木を歩く walker が 2 つあるとずれる**。`compile_stmt` に文種別を足したら
  **必ず採番側（`collect_nested_decls`）も見る**（#27-c で 2 回踏んだ）。
  ⇒ **#59 で「宣言される名前」だけは強制化**（[decl_names.rs](src/decl_names.rs) の exhaustive 2 段で
  **variant を足すとコンパイルが止まる**）。⚠ **`_ => {}` で黙らせない**・**順序に意味がある束縛
  （`for` ターゲット・`except as` 別名）は載せない**（採番がずれる）。
- **⚠⚠ op を足したら `cargo build --features prof` と `--features tw_stats` も通す**（#69）。
  `op_prof.rs` は「ずれたら壊れる」設計だが**既定ビルドでは消える**ので #68 以来壊れていた。
- **コード索引を持つ op を足したら `peephole::code_target_mut` に登録する**
  （`ForIter` の exit・`SetupTry` の handler・`StaticInit` の after）。忘れても**テストは通ってしまう**。
- **コンパイラの「この形のときだけ載せる」条件は理由を確かめてから外す**（2 実装の差／最適化の前提／
  本当の非対応の 3 通りがあり、外し方が違う）。
- **⚠ VM は「解決情報が揃っている」前提**（#3/#36）。`resolve_program` ＋ `check_and_annotate` ＋
  `set_toplevel_globals` を供給しない入口では、正しいコードでも `VmForceError` になる。
  ⇒ **入口ごとに配線する責任がある**（`run_program`・REPL・テストヘルパー・モジュール本体）。
  ⚠ 例題は全部 `run_program` を通るので**例題だけ見ていても入口の穴に気づけない**。
- **⚠ worker スレッドは `Interpreter::new()`**（#32）。**`--vm` も型注釈も引き継がない**ので、
  渡し忘れるとゲートに穴が開く（実際に開いていた）。

**VM の作り（触るとき見る所）**
- **`exec_op` は `#[inline(always)]`。op のアームに重い本体を書かない**（#10-b）。
  IC は**ヒット経路だけインライン・ミス経路は `#[inline(never)]`**。
- **`LoadName` は関数本体で使えないが `LoadGlobal` なら使える**（`scopes[0]` だけを見る）。
  根拠は「**`slots` に無い＝この関数のローカルではない**」がコンパイル時に確定していること。
- **可変な共有は slot では表せない**。`Rc<RefCell<Value>>` を**セル表**（`LoadCell`/`StoreCell`）に置く。
  `static mut` だけは `Interpreter::static_cells`（span がキー）を直読みするのでセル表を使わない。
- **診断フックは feature で消せる形にする**（`exec()` は文ごとに呼ばれるので atomic 読み 1 回で 11% 退行）。

### 次にやる候補
> **#50 の実測で狙い所が変わった**（「現在地」の再掲。exec の中は #24/#46 で打ち止め）:
> ① **exec 以外が中央値で 86%**（プロセス費用・`interp_init`・`type_check`・`parse`）。
>    ここは手つかずで、**短命スクリプトの体感を動かせるのはこちらだけ**。
> ② exec の中を続けるなら **呼び出し系（37〜38%）一択**。`Call` 単体で 19.9%、
>    1 呼び出し ≈ 275ns（うち `Call` op 自体が ≈ 213ns。int 加算 ≈ 11〜13ns の **20 倍**）。
>    `Jump`+`JumpIfFalse` は合わせて **4.1%** しかない（安い命令を狙うな・#46）。

1. **速度（#69 / #70）— 調査済み・前提なし**。69 は `interp_init` の内訳を実測済み
   （ウォーク 50〜75%）で **61 と同じ場所＝同時にやる**。70 は原因確定済み（`as_local` 前提）。
2. **保守性レーン — 残り 2 件（#65・#67）＋ #72**、全件が前提なし。
   次は **72**（`search_paths` の読み取り 2 実装）／**67**（`Interpreter` の部分クラスタ化）。
3. 別レーン（#19 / #17-a / #17-b）— 外部接続系。いつ着手しても他をブロックしない。
4. ブロック中（#14 → #11 R2-c）— 前提の「モジュール間ネイティブ直リンク」が未計画。

> **⚠ 枯れたのは「exec の中」だけ**（#12 2.61x・#2b 1.6x・#2a・#1-x・#10-b・#26 で取り切り、
> #24/#46 は実測して却下）。**exec の外（#69）と載り方（#70）は手つかず**。
> ⚠ **カバレッジを広げると速度も付いてくる**ことがある（#32 の async 本体は 3.77x）。
> 新しく速度課題を立てるなら**まず計測して支配項を出すこと**。
> ⚠ #1-x の教訓: **`#[inline]` は効いているとは限らない**（巨大関数は LLVM が却下する）。
> **⚠ #12b / #2c は保留**（循環依存・速度理由も #12 で消えた）。
> **⚠ #19 / #17-a / #17-b（外部接続系）は優先度低**（本系列と独立・別レーンで扱う）。

### 作業の進め方（この系列で有効だった型）
- **推測せず先に計測する**。見積もりは本系列で何度も外れた（一覧は実装ログ末尾の表）。
  ⚠ **命令数は「当たりを付ける」用で「速度の予測」には使えない**（#46）。
- **検証は 5 点セット**: `cargo build`（**警告 0**）・`cargo test`（**742 緑**）・
  [compare_python_impl.ps1](compare_python_impl.ps1)（**51/51**）・[scan_examples.ps1](scan_examples.ps1)（**FAIL 0**）・
  [force_gate.ps1](force_gate.ps1)（`VmForceError` **0 件・153 例題**）。⚠ **release バイナリを見る**。
  デバッガに触るなら [debug_session.ps1](debug_session.ps1)、REPL なら [repl_session.ps1](repl_session.ps1)、
  codegen なら [dump_native_ir.ps1](dump_native_ir.ps1) の IR byte-identical（最強の検査）。
  ⚠ **VM の適格範囲に触るなら [tw_stats.ps1](tw_stats.ps1) も**（ツリーウォークが定義文だけかを確認）。
  ⚠⚠ **「挙動不変」の主張には [compare_bytecode.ps1](compare_bytecode.ps1)**（#68 で .ps1 化・
  **使う前に同一 exe 同士で負の対照**を取る。#68 は 108/108 を確認してから A/B を読んだ）。
  ⚠⚠ **識別子を改名・削除したら [stale_doc_refs.ps1](stale_doc_refs.ps1) を必ず走らせる**
  （#52/#53/#56 が走らせず 14 件入れ、#57 で戻した。**ゲートは作るだけでは守られない**）。
  clippy は **`cargo clippy` で 51 件**・**`cargo clippy --all-targets` で 64 件**（差は `benches/`）。
  ⚠ **基準値はコマンドごと書く**。総数でなく**増分 0** を確認すること。
- 大きな変更の前後で **A/B 実測**する（同一ビルドで emit のみを切り替える）。
  ⚠ **op を足す規模の変更は「op を足しただけ」のプローブと 3 本で切り分ける**（#27/#46）。
- 速度効果が小さくても、コード・ロジックの簡素化が見込めるならメリットとして認識する。
  ⚠ **逆に「速度も出ず複雑になる」なら実装済みでも捨てる**（#28/#46 は revert した）。
- 全てのタスクは番号で管理する。番号付けされていないものは新タスクとして昇格を提案する。
- **高リスク低リターンと判断したらスキップして保留にし、理由を記録する**（勝手に大改造しない）。
  判定基準として効いたのは「**消費者が居るか**」（#11 R2-c・#14・#15b はこれで保留）。
  ⚠ **タスクの定義が目的に対して過剰なら定義を見直す**（#27-a は 644 行→50 行）。

### 検証・計測スクリプト（リポジトリ直下）
| スクリプト | 用途 |
|---|---|
| [scan_examples.ps1](scan_examples.ps1) | 全例題をタイムアウト付きで実行し、失敗のみ理由付きで列挙 |
| [dump_native_ir.ps1](dump_native_ir.ps1) | 代表 6 モジュールの生成 LLVM IR を保存（`.arc`/`.ars` は退避・復元） |
| [annot_diff.ps1](annot_diff.ps1) / [annot_unresolved.ps1](annot_unresolved.ps1) | 注釈の充填状況・binop 特化の内訳・`Unresolved` の発生源／その全例題集計（式種別ごと） |
| [ab_bench.ps1](ab_bench.ps1) | 2 つの `arrow.exe` を**交互実行**して経過時間を比較（`-A head.exe -B new.exe`）。#2b で新設・**#38 で非同期読み化**。異常終了／タイムアウト（`-TimeoutSec` 既定 180）／不在パスは**値を出さず理由を表示**する（黙って速い値を出さない）。⚠ `powershell -File` 経由だと `-Scripts a,b,c` が 1 要素に潰れるので `-Command` で呼ぶ |
| [ab_bench_modes.ps1](ab_bench_modes.ps1) | **実行モード別**の A/B 計測（#47 で新設）。非コンパイル／コンパイル済み native／C DLL の 3 経路を交互実行し、スクリプトが出す `METRIC <name> <secs>` を解析する（プロセス経過時間ではないので起動・DLL ロード・setup を計測から外せる）。`CHECKSUM` を A/B で突き合わせ、食い違えば値を出さず警告。⚠ native モードは**測る側のバイナリで `--compile` し直す**（.arc の形式がブランチ間で違う） |
| [repl_session.ps1](repl_session.ps1) | **対話 REPL の回帰検知**（#36 で新設）。`examples/repl/repl_session.{in,out}` の golden 比較。⚠ `compare_vm_modes` は stdin を与えず、`compare_debug_modes` はデバッガ REPL（別物）を見るので、**対話 REPL はこれだけが検査している**。更新は `-Update`（差分は必ず目で見る） |
| [debug_session.ps1](debug_session.ps1) | **対話デバッガのステッピングの回帰検知**（#1 で新設・#33 で golden 化）。`examples/debugger/<name>.{ar,in,out}` の期待値比較。⚠ 他のどのゲートも stdin を与えないので、**ステッピングはこれだけが検査している**。更新は `-Update`（**#44 以降は書き換える行を必ず表示する**＝黙って上書きできない）。⚠ **このゲートは 2 度、黙って赤くなった**: ①`6bf039c`〜`7aea0e5`（golden が BOM 修正前のまま・#44 で録り直し）②#44〜#49（**環境側**＝コンソールのコードページで `Process.Start` が stdin に BOM を書いていた。**src も golden も無関係**なので遡っても原因が出ない／**別のマシンでは緑のまま通る**）。#49 で解消 |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **参照実装（`impl_python`）との stdout 差分検査**（#31 で新設）。「**両実装が同じ間違いをする形**」以外を覆う唯一の網で、`compare_vm_modes` を失った後の代替。既知差分は理由つきで `$knownDiff` に列挙（`-ShowSkipped`）。⚠ `impl_python` は 100 コミット前に同期 |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **コメント内の `` `識別子` `` が src に実在するか**の検査（#51 で新設）。#33 で消した関数を指す記述が 61 箇所あり、うち 2 件は**指示が真逆に矛盾**していた（コンパイラは何も言わない）。⚠ 履歴として正しい言及は落とす — 同じ行に「削除／廃止／撤去／以前／旧／移設／だった／していた」があれば履歴扱い。⇒ **消えたものに言及するときはマーカー語を書く**。外部成果物（`.ps1` 名・METRIC 名）は `$whitelist`。`-All` で除外分も表示。⚠⚠ **#51 で 0 にした直後、#52/#53/#56 が走らせずに 14 件へ戻した**（#57 で再び 0）＝ **改名・削除をしたら走らせること**。⚠ マーカー語は**行単位**なので、足すと同じ行の他の識別子も検査から外れる |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **2 つの `arrow.exe` のバイトコードが同一か**を全例題で突き合わせる（#52 の手順を #68 で .ps1 化）。「挙動不変」を exit code より強く裏付ける唯一の手段で、#62/#63/#66 が要る。⚠ **async は対象外**（同一バイナリでも dump が揺れる）。⚠ **使う前に同一 exe 同士で負の対照を取る**。⚠ 生成する .ps1 のパス区切りはスラッシュにすること（バックスラッシュがエスケープに化けて壊した実績あり） |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **import 系例題の stdout/stderr/exit が 2 バイナリで同一か**（#58 で新設）。cs-dll・**cs-proc**・js-proc・cpp-dll/lib の 10 例題。⚠⚠ **子の出力はパイプで受けず `Start-Process -RedirectStandard*` でファイルへ落とす** — `import[js-proc]` の **node ブリッジが孫として生き残ってパイプを握る**ので `ReadToEndAsync` でも返らない（#38 のデッドロックとは別原因・skill `vm-pitfalls` §4）。⚠ `cs_proc_app.ar` は **`import[cs-proc]` を踏む唯一の非 GUI 例題**で、#58 以前は**どの差分ゲートにも入っていなかった** |
| [compare_outputs.ps1](compare_outputs.ps1) | **全例題の stdout/stderr/exit が 2 バイナリで同一か**（#63 で新設・91 例題）。⚠⚠ **解釈側（`eval_*`/`exec_*`）だけを触った変更は [compare_bytecode.ps1](compare_bytecode.ps1) が自明に一致してしまう**ので、挙動不変はこちらで主張する。⚠ `bench` 分類は**経過時間そのものが出力**なので丸ごと対象外（`interop/bench_ab_cdll.ar` は名指しで除外）。⚠ アドレス（`0x…`・`id()`）は**除外せず正規化**する — `collection.ar` を落とすと set メソッドを見る網が消える |
| [force_gate.ps1](force_gate.ps1) | **VM に載らない構文の回帰検知**（#25。#33 完了後は「既定の挙動が全例題で通るか」の検査）。全例題を実行し `VmForceError` を列挙。⚠ **止めて判定する**用途で件数は `tw_stats.ps1` で見る。GUI 例題は**タイムアウト後に窓を閉じて**完走させる（#29） |
| [prof_dist.ps1](prof_dist.ps1) | **非コンパイル実行の実行時間分布**（#50 で新設・要 `--features prof`）。`-Mode phases` で段別（startup/lex/parse/type_check/resolve/interp_init/exec/teardown）、`-Mode ops` で **exec 中の op 別滞在時間**（統計サンプリング）。⚠ **1 回目はファイルのコールドリードを踏む**ので既定で 2 パス走らせて 2 パス目だけ採る。⚠ `-Mode ops` はサンプラーが 1 コアをスピンするので**プロセス wall が伸びる**（wall を見るときは `-Mode phases`） |
| [tw_stats.ps1](tw_stats.ps1) / [tw_stats_files.ps1](tw_stats_files.ps1) | **ツリーウォークが実際に実行している文**を全例題で集計（`AR_TW_STATS`）／その例題別内訳。feature 付きビルドを自動で行う |
| [run_examples.ps1](run_examples.ps1) / [bench.ps1](bench.ps1) | 素朴な例題ランナー（タイムアウトなし）／ベンチ一式 |

### 診断フック（環境変数）
| 変数 | 効果 |
|---|---|
| `AR_DUMP_LL=<path>` | `--compile` 時に生成 LLVM IR を保存 |
| `AR_ANNOT_DIFF=1` | 注釈の充填状況・binop 特化の内訳・`Unresolved` の発生源・slot 索引読みの件数・`AnnotIdent`（識別子読みの型落ち内訳・#15b） |
| `AR_VM_DUMP=1` | VM の生成バイトコードを逆アセンブルして stderr へ |
| `AR_PROF=1` / `AR_PROF=ops` | 実行時間分布（#50）。段別のみ／段別＋**exec 中の op 別サンプリング**（`AR_PROF_US` で間隔µs・既定 20、`AR_PROF_CSV` で CSV 出力）。**`cargo build --release --features prof` が要る**（既定ビルドではコードごと消える） |
| `AR_TW_STATS=1` | ツリーウォークの実行内訳（文種別×最上位/関数内）・VM コンパイル成否・bail 地点・**`tw_eval`（`eval()` の回数と AST 引数版入口 5 つの通過数・#55。通常実行ではほぼ 0）**・**`tw_control_flow`（TLS/センチネルを使う経路に入った回数・#3。通常実行では 0）**。**`cargo build --features tw_stats` が要る**（既定ビルドではコードごと消える。env 判定だけにすると `exec()` 1 文ごとの atomic 読みで 11% 退行する） |

### 落とし穴（既知）

> **本文は skill [vm-pitfalls](.claude/skills/vm-pitfalls/SKILL.md) へ移した**（規約 3。#49 時点の全 30 項）。
> 計測・A/B（§1）／VM・コンパイラ（§2）／検査網とゲートの信用（§3）／
> PowerShell・子プロセス・エンコーディング（§4）の 4 節。**各項は実際に踏んだ失敗**で、
> 大半は**テストもゲートも緑のまま間違っていた**もの。
> ⇒ ベンチを取る前・opcode を足す前・「緑だから大丈夫」と言う前・子プロセスを
> 起こすスクリプトを書く前に、対応する節を読むこと（skill 名: `vm-pitfalls`）。

---

## 実装状況

各段の実装詳細・実測は [PHASE_R1_RESULTS.md](PHASE_R1_RESULTS.md)。
⚠ **不変条件の検査は #33 で入れ替わった**: `--vm=off` との byte-identical 比較は成立しなくなり、
[compare_python_impl.ps1](compare_python_impl.ps1)（参照実装との突き合わせ）＋
[repl_session.ps1](repl_session.ps1) ＋ [debug_session.ps1](debug_session.ps1) が担う。

**完了項目の一覧は区切り線の下へ移した**（規約 3）。経緯は [IMPLEMENTATION_LOG.md](IMPLEMENTATION_LOG.md)。

### 残り — 依存関係つき一覧
依存の凡例: 「前提」が空欄＝**今すぐ着手できる**。`←X` は X が終わるまで着手できない。

| # | タスク | 手法 | 前提（依存） | 状態 |
|---|---|---|---|---|
| **70** | **最上位ループが型特化・融合命令に載らない**（速度・#59 とは無関係） | 帰納変数の slot 昇格 か グローバル版融合 op | — | **未着手・調査済み**（fn 内比 **2.6x**・原因は `try_emit_bin_fused` の `as_local` 前提と確定） |
| 72 | `python.search_paths` の読み取りが **2 実装**（#69 で発見） | パーサ側（自前の文字列走査・source_dir と root_dir だけ）を `load_python_search_paths` へ委譲 | — | 未着手（⚠ **探索範囲が違う**ので畳むと挙動が変わる。まず差を測る） |
| 65 | `eval_str_method` 48 アーム | カテゴリ別サブ関数へ | — | 未着手・**優先度低**（平坦な表・得るのは行数だけ） |
| 67 | `Interpreter` 31 フィールドの部分クラスタ化 | イベントループ 4 本・デバッガ 2 本のみ畳む | — | 未着手・**部分適用のみ**（全面分解は保留） |
| 11 R2-c | グローバル記憶域の index 配列化 | ネイティブの index 参照 | **← 消費者の出現（14 と同時に再評価）** | ブロック中（消費者不在） |
| 14 | §6 モジュール動的リンク | ディスクリプタシンボル＋ABI ハッシュ照合 | **← モジュール間ネイティブ直リンクの導入**（未実装・未計画） | ブロック中 |

> **69・70 は #50 の分布から起票した速度タスク**（根拠は [IMPLEMENTATION_LOG.md](IMPLEMENTATION_LOG.md) の同番号）。
> **58〜68 は 2026-08-21 の全 src 機械診断で起票**（保守性レーン第 2 弾。内訳・実測・対象外の根拠は
> [IMPLEMENTATION_LOG.md](IMPLEMENTATION_LOG.md) #58〜#68）。**残る 8 件は全件が前提なし**。
> **58〜64・66・68・69・71 は完了**。残りは **72 → 67 → 70**（65 は最後）。
> ⚠⚠ **`vm/run.rs:exec_op`（836 行）は対象外**。86 アームの平坦な表で**最大アームは 42 行**であり、
> 行数だけを見て割ると **#10-b（`#[inline(always)]` のアームに重い本体を書くと全体が遅くなる）を再演する**。
> 同じ理由で `ast.rs`(1124)・`parser/exprs.rs`(940)・`Value`/`Stmt`/`Expr` の広い参照も**起票しない**。

> **保守性レーンは速度目的ではない**ので A/B は**退行が無いことの確認**に使う。
> ⚠ **51 で「コメントだけ」の変更がゲートを 2 件動かした**（`#[allow(dead_code)]` が
> unreachable 警告を食っていた）＝ **属性が警告を隠していないか疑うこと**。
> ⚠⚠ **「挙動不変」は [compare_bytecode.ps1](compare_bytecode.ps1) で確かめる**（exit code では弱い）。
> **async 例題は同一バイナリでも dump が揺れる**ので対象外（差分を見たらまず同一 exe で再現を見る）。
> 第 1 弾 51〜57 の経緯・23 の 54 への吸収は [IMPLEMENTATION_LOG.md](IMPLEMENTATION_LOG.md)。

```
27/27-c/27-d/32/29/3 は完了（当時 `force_gate` 0・128 例題・フォールバック撤去済み）
    ※例題数はその後 **151** まで増えた（2026-08-20）。**現在の値は必ずゲートを走らせて確認すること**

🎉 VM 一本化レーンは全完了（2026-08-18）
    34（break/continue 貫通）／35（block_return・loop_yield 検査）／36（REPL・テストの移行）
    ／37（finally 跨ぎ）／39（関数からのグローバル代入）／40（finally 本体）／41（定義文脈の式）
    ／42（モジュール本体）……すべて完了 ＝ **VM に載らない正しいプログラムを 0 にした**
    31（参照実装との差分検査）／43（実行時型検査の高速判定）……完了
    33（ツリーウォーク制御フロー・TLS・センチネル・`--vm` の削除）……**完了**（src 実質 -762 行）

    38（A/B のデッドロック解消）／30（クロージャ Chunk の実体跨ぎ再利用・1.456x）／
    44（デバッガ golden 録り直し）／45（`FnValue.body` を `Rc<[Stmt]>` 化）……いずれも完了
    24（peephole パターン追加）／46（ループ反転）……**却下** ＝ 実測で効果 0%・1.003x
        （⚠ **命令数 -20% でも速くならない** ＝ 命令には値段の差がある）
    47（端点 A/B）……完了 ＝ 解釈 3.97x / native 境界 1.60x ／ 48（native 書き戻しの実バグ）……**完了**
    49（`debug_session.ps1` の stdin BOM 混入）……**完了** ＝ golden 無変更で 5/5 identical
        （原因は `Process.Start` が `[Console]::InputEncoding` の preamble を子の stdin へ書くこと）
    50（実行時間分布の実測）……**完了** ＝ **exec は全体の 14%（中央値）＝端点 1.59x**（exec 内は呼び出し系 37〜38%）
    51（doc/属性の迷子と陳腐化コメント）……**完了** ＝ 実バグ 1 件（orphan doc に属性が同行）＋
        存在しない識別子への参照 61→0（[stale_doc_refs.ps1](stale_doc_refs.ps1) 新設で再発防止）
    52（`CompileMode` ＋ `base`/`finish`）……**完了** ＝ 3 bool を enum 1 本へ（compiler.rs -91 行）
    53（`vm/compiler.rs` のサブ分割）……**完了** ＝ 4,331 行を 10 モジュールへ。
        **1 行も書き換えない機械的な移動**で**バイトコード 202 例題すべて byte-identical**
    55（ツリーウォークの式評価経路の生死判定）……**完了** ＝ `AR_TW_STATS` に `tw_eval` を追加。
        **通常実行では `eval()` がほぼ 0**（130 例題中 129 本で 0）／**AST 引数版の native 2 経路は
        FFI 例題を含め全 0**／REPL とデバッガでのみ生存。⚠ **副産物で実バグ 1 件（→ 56）**
    56（`is_builtin_callee` の bail が実バグ化）……**完了** ＝ **`parse_ar` を復活**（#33 以来
        `VmForceError` で完全に死んでいた）。`tuple`/`list`/`type`/`byte` のエラー文言も
        `NameError` へ戻した。`is_builtin_callee` は**削除**。例題 3 本を新設
    57（`stale_doc_refs` の再発 14 件）……**完了** ＝ #51 が 0 にしたゲートが**直後 4 タスクで 14 件に戻っていた**
        （実害は改名の取り残し 4 件のみ／残り 10 件はマーカー語・whitelist の使い忘れ）
    54（native typed ABI 呼び出しの 1 本化）……**完了** ＝ 3 箇所に手書きされていた
        typed ABI 呼び出しを `invoke_typed_abi` ＋ `decode_typed_ret` へ集約（#48 の実バグの温床）。
        ⚠ **#55 の「通常実行から到達不能」は誤りだった** — デフォルト引数の式はツリーウォークで
        評価されるので AST 引数版 2 経路にも通常実行で到達する。負の対照を例題化

    58〜67（保守性レーン第 2 弾）……**58〜64・66 完了・残り 2 件（65・67）は起票のみ**
        ⚠ **`exec_op` 836 行は対象外**（平坦な表・割ると #10-b の再演）
    58（`Stmt::Import` アーム 210 行）……**完了** ＝ `exec_import` ＋ 補助 7 本へ切り出し（`exec` 394→192 行）
    62（`compile_stmt` 817→371 行）……**完了**。⚠⚠ **偽の 6.5% 退行**をプローブで棄却
    61（設定探索の切り出し）／69（遅延化）……**完了** ＝ `interp_init` **0.378→0.172 ms**
    71（`import[py]` の関数が呼べない）……**完了**（実質 2 行。**#33 の綻びの 5 度目**）
    64（`Expr::Call` の二重化）……**完了** ＝ 558→414 行。⚠⚠ **統合はしない**（2 つの ABI）。**IR 6/6 一致**
    60（cpp ヘッダパーサのネスト）……**完了** ＝ ネスト **11→3** / **7→3**。実ヘッダ全ダンプが byte-identical
    63（委譲漏れ 4 型）……**完了** ＝ `*_methods.rs` へ（530→205 行）。⚠⚠ **bytecode が自明に一致する変更**
    59（walker 8 本のドリフト）……**完了** ＝ 束縛判断を [decl_names.rs](src/decl_names.rs) へ集約。
        ⚠ **買ったのは行数ではなく強制**（src は +51 行）。⚠ 起票時の「委譲」案は再帰範囲が違って不成立
    66（`Compiler` の `Chunk` 複製 17 フィールド）……**完了** ＝ `Compiler` が `Chunk` を直接組み立てる。
        ⚠ 起票時の `ChunkBuilder` 案は**逐語 move が引っ越すだけ**なので採らなかった
    68（関数本体の `enum` が `VmForceError`）……**完了** ＝ 組み立て（`build_enum_classes`）と
        記憶域を分離し `Op::EnumDef` で slot へ。**副産物で #59 のドリフトが実害化**（→ 部分修正）

残り: **第 2 弾は 58〜64・66・68 完了**／速度は **69 完了**・70／**71 完了・72 は未着手**／別レーン／ブロック中 14・11 R2-c
      ⇒ **今すぐ着手できるのは 65・67・72 の 3 件・70 と別レーン 3 件**
保留: モジュール間ネイティブ直リンク（未計画）→ 14 → 11 R2-c ／ 12b → 2c は循環依存で両方保留
      23（評価済み引数の struct 化）は **54 に吸収**（保留理由が #48 で失効した）
```

### 別レーン — 外部接続系（**優先度低**・本系列と独立）

実行方式の統一・高速化とは**交差しない**（型検査とスタブ側の話題で、実行時ディスパッチに触れない）。
いつでも着手でき、いつ着手しても他タスクをブロックしない。
⇒ 実行系は一段落した（2026-08-19）。**今すぐ着手できるのはこの 3 件だけ**（2026-08-20 時点）。

| # | タスク | 手法 | 前提（依存） | 状態 |
|---|---|---|---|---|
| 19 | py 組み込みスタブ整備（`time`/`math`） | 同梱 `.pyi` ＋ `python_search_dirs()` に置き場を追加 | — | 未着手・優先度低 |
| 17-b | JS スタブの型付け | 既定 `Any`・`.d.ts` から `.ars` 生成 | — | 未着手・優先度低 |
| 17-a | C/C++ `void*` の専用型 | 不透明ハンドル型を導入し int との相互代入を静的禁止 | — | 未着手・優先度低 |

## 0. この文書の使い方 / 現在の決定状態

### 0.1 全体像（2フェーズ）

```
                        ┌─ 解釈経路: 注釈付き AST を フレーム/slot でツリーウォーク（Phase R 中間状態）
source → lexer → parser ┤                              ↓ その上に
        → type_check     │                        バイトコード VM を載せる（Phase V, 強制バイトコード）
        → [Phase R: 解決層]┤
                        └─ ネイティブ経路: 注釈付き AST → LLVM（既存 codegen を簡素化・拡張）
```

- **Phase R**（解決層）は `src/interpreter/`（解釈）と `src/partial_compiler/`（ネイティブ）の**両方**が消費する共有基盤。
- **Phase V**（バイトコード VM）は解釈経路の**最終実行形**。解釈実行は最終的に**強制バイトコード**（ツリーウォークは通常実行から除去、ただし `eval()`/`exec()` は §2.1/§2.3 のツール用に残置）。

### 0.2 決定済み事項（確定・再litigateしない）

| # | 決定 | 根拠 |
|---|---|---|
| D1 | **共有 IR は AST 解決（X 案）**。バイトコード→機械語（Y 案）は**不採用** | §3.1 |
| D2 | **解釈実行は強制バイトコード**（#3 で達成 → **#33 でツリーウォークごと削除**。`--vm` も無い） | §3.2 |
| D3 | **スタックマシン**（レジスタマシンではない） | §3.3 |
| D4 | **変数解決モデル R0**: A=参照カウントフレームのスタック / B=関数内固定 slot / C=動的名はほぼ静的解決 | §3.4 |
| D5 | **async は share-nothing 維持**。deepcopy 削減は COW/不変 Arc 共有/move、共有可変は明示プリミティブ | §3.5 |
| D6 | **モジュール動的リンク = ディスクリプタシンボル + モジュール単位 ABI ハッシュ照合** | §3.6 / §6 |
| D7 | フェーズ0（計測）完了・ゲート通過。支配項＝呼び出し機構・引数束縛・ローカル名前引き | §1.5 |
| D8 | Phase 5（TypeChecker 分割）完了・main マージ済み。着手障害なし | — |

### 0.3 実装順序

1. **Phase R**（§4）— R0 ランタイムモデル導入 → R1〜R4 解決ステップ。各ステップ独立コミット可、`cargo test` 緑維持。
2. `bench.ps1` 再測定（§4.5 ゲート）— 呼び出し機構がなお支配的かを確認（Phase V の設計裏付け）。
3. **Phase V**（§5）— V-A〜V-F。バイトコード VM を R0 のフレーム/slot 上に載せる。
4. **モジュールリンク**（§6）— ネイティブ経路（`.arc`）の安全動的リンク。Phase V と並行可。

### 0.4 未決事項（§9 に集約）

着手時に確認すべき小さな設計判断のみ。方針転換級の未決はない。

---

## 2. 設計制約 — naïve なバイトコード化を阻む既存機能（**着手前に必読**）

「AST を捨ててバイトコードにする」を**不可能にする**要素。把握していないと途中で詰まる。
（✅=対応済 / 🔶=部分 / ❌=未着手 を各項目に付記）

### 2.1 AST は実行時の第一級の値 — `ast_value.rs` 【✅ AST 保持】
[ast_value.rs](src/interpreter/ast_value.rs) は `parse_ar()` 組み込みの実装で、**AST ノードを `Value::Namespace` ツリーに変換**して
Arrow へ渡す（`__type__` 等）。`python_converter` / converter.ar が依存。
→ **AST は破棄不可。バイトコードは「置き換え」でなく「追加の実行表現」**。Chunk は AST から生成、AST は保持し続ける。

### 2.2 テンプレートは実行時 AST 置換 — `templates.rs` 【✅ #7 で `(テンプレート,型引数)` キーの実体化メモ化・関数/ジェネレータ。クラスは副作用のため対象外】
[templates.rs](src/interpreter/templates.rs) の `subst_stmt`/`subst_expr` が、呼び出し時に型変数を具体型へ置換した
**新 AST を clone-walk で生成**してから実行。
→ **テンプレートは実体化時オンデマンドコンパイル**。`(テンプレート名, 型引数リスト)` をキーに Chunk キャッシュ。解決も実体化ごと。

### 2.3 デバッガが文単位 Span に依存 — `debugger.rs` 【✅ 行テーブル(トレースバック)・デバッグ名テーブル・REPL バイトコード実行】
`exec()` 冒頭で `should_pause_at(stmt)` を Span で判定（[dispatch.rs:19-23](src/interpreter/exec/dispatch.rs#L19)）。
デバッガ REPL は**生スコープに対して式を評価**し `let dbg::name = …` で一時変数宣言。
→ **必須成果物**: ① バイトコードオフセット→Span の**行テーブル**、② slot→変数名の**デバッグ名テーブル**。
   **後付け不可。Chunk の初期設計に入れる**。（V-E で ①②とも実装・REPL は `LoadName`/`compile_debug` で名前引きバイトコード実行。
   **VM 関数内のステートメント単位ブレークも #1 で完了**（`stmt_spans` 行テーブル＋`run_stepping`）。）

### 2.4 クロージャは定義時に名前でキャプチャ — `capture_env` 【✅ #27-d で VM 化・#30/#45 で実体ごとの費用も除去】
[blocks.rs:39](src/interpreter/exec/blocks.rs#L39) が本体の自由変数を走査、外側から拾って `HashMap<String, CapturedVar>` を作る。
可変変数は `Var::Mutable` → `Var::Cell(Rc<RefCell<Value>>)` に**その場で昇格**して共有。
→ **バイトコード化と相性が良い**。自由変数解析をコンパイル時に前倒しすれば R0-A のフレーム参照になる。`Rc<RefCell>` 表現は流用。
（#27-d で実装: 不変キャプチャは**末尾 slot**・可変キャプチャは**セル表**（`LoadCell`/`StoreCell`）。
さらに **#30** で Chunk を**定義サイト単位**（`ChunkFnDef::compiled`）にして実体ごとの再コンパイルを、
**#45** で `FnValue.body` を `Rc<[Stmt]>` にして実体ごとの AST 複製を消した。
⚠ どちらも「実体間で共有してよい」根拠は **slot 採番が `sort()` 済み ＋ 束縛が名前引き**の 2 点。
⚠ `deep_clone`（スレッド送出）では **`Rc` を共有してはいけない**（#15/#45・不変条件テスト 3 本で固定）。）

### 2.5 名前が実行時に計算される代入 【✅ 通常は静的解決・デバッガは名前引き】
- `local::{name}`（可変長引数）— [eval/core.rs:34](src/interpreter/eval/core.rs#L34)。ただし `name` は AST 内リテラル＝静的解決可（§3.4-C）
- `Self` / `kwargs` — メソッド/Python 呼び出しで動的に `declare_var`。ただし名前は既知＝slot/型index に解決可
- `import` バインド名 — パース時に一意確定＝静的解決可

→ 大半は静的解決可能（§3.4-C）。**真に動的なのは実行後パースされるコード（デバッガ/対話 REPL）のみ**。そこだけ動的名エスケープハッチ（`LoadName`/`DeclareName`）。

### 2.6 `freeze` が実行時に可変性を変える 【✅ 保守的に扱う】
`make_var_immutable`（[scope.rs:83](src/interpreter/scope.rs#L83)）が `Mutable`→`Immutable` に降格。
→ **「不変だから定数畳み込み」を仮定してはいけない**。freeze 対象になりうる変数は保守的に扱う。

### 2.7 ジェネレータが eager（先行収集）【✅ #8 で本体を VM 化（`Yield` op・eager 収集維持）。lazy 化は §8 非目標のまま】
`exec_generator`（[functions/execution.rs:292](src/interpreter/functions/execution.rs#L292)）は**本体を最後まで実行し全 yield 値を Vec 収集**
してから `Value::Generator` を返す。遅延評価ではない。
→ **Phase V では eager のまま移植**。lazy 化（frame 中断・再開＝新機能）は §8 非目標。

### 2.8 その他（影響小・意味論変更なし）【🔶 FFI/overload は実行時委譲・import モジュール本体は ✅ #42】
- **FFI 経路**（DLL / cpp_bridge / cs-dll / cs-proc / js-proc / PyO3）は全て `eval_call` の先。`CALL_NATIVE` 系に集約されるだけ。
- **オーバーロード解決**は評価済み引数への実行時ディスパッチ（`dispatch_overload`）。当面そのまま実行時。
- **import は実行時にモジュールを実行**して名前空間を作る。モジュール単位 Chunk、初回 import 時コンパイル。
  （**#42 で VM 経路へ**。足りなかったのは**名前ベースの代入**だけで、`Op::StoreName` を 1 個足して解決した。
  宣言（`DeclareGlobal`）と読み（`LoadName`）は元からスコープチェーンを見ていた。
  残るツリーウォークは**定義文だけ**＝設計どおり・#10-d。）

### 2.9 オフセットアクセスの記憶域は既に存在する（重要な追い風）【✅ R3 IC が field_value offset を利用】
インスタンス先頭ポインタからのオフセットアクセスは、**コンパイルと無関係に解釈実行でも既に全インスタンスの実行時表現**:
- `InstanceData { raw: Box<[u64]>, class, boxed_fields }`（[value/instance.rs:129](src/interpreter/value/instance.rs#L129)）。
  slot 0=`[class_id][flags]`、適格クラス（全プリミティブ・trait 継承なし・≤24 フィールド）は C-ABI レイアウトで raw に格納。
- `field_value(idx)` が `byte_offset` で**直接オフセット読み**（[value/instance.rs:213](src/interpreter/value/instance.rs#L213)）。
- ただし **名前→idx の解決だけが毎回辞書引き**（`get_attr_val` → `field_index.get(attr)`, [eval/attrs.rs:70](src/interpreter/eval/attrs.rs#L70)）。
  `Expr::Attr` は解決キャッシュを持たない（[ast.rs:334](src/ast.rs#L334)）。

→ **記憶域は整地済み。Phase R の R3 は「名前→idx の辞書引き」だけを潰せばよい**（詳細 §4.3）。
   設計思想は c-abi-interop（[.claude/skills/c-abi-interop](.claude/skills/c-abi-interop/SKILL.md)）が**ネイティブ境界**に既に採る
   「AST 作成時に解決済みなら直接オフセット、できねば辞書」を、**解釈経路そのものに広げる**もの。

---

## 4 / 5. Phase R（AST 解決層）・Phase V（バイトコード VM）

**どちらも完了済み**。設計と各段の内訳は**区切り線の下**へ移した（規約 3）。
R1〜R4 の解決ステップ・V-A〜V-F の段階・`src/vm/` の構成はそこを見ること。
進捗の一次資料は [PHASE_R1_RESULTS.md](PHASE_R1_RESULTS.md)。

## 10. 検証コマンド / 規約

```
cargo test                          # 742 passed を維持（各ステップ/各段ごと）
cargo build                         # 警告0 を維持
cargo clippy                        # 既存 51 件・増分 0（--all-targets だと 64 件。基準値はコマンドごと書く）
./scan_examples.ps1                 # 例題スイートの回帰確認（FAIL 0）
./compare_python_impl.ps1           # 参照実装との stdout 差分（51/51）— off/on 比較の代替
./repl_session.ps1                  # 対話 REPL の golden（identical）
./debug_session.ps1                 # 対話デバッガのステッピングの golden（5 identical）
./stale_doc_refs.ps1                # コメント内の識別子が src に実在するか（0 件。⚠ 改名・削除の直後は必ず）
./compare_bytecode.ps1 -A <head.exe> # 2 バイナリのバイトコードが同一か（コンパイラを触ったとき）
./compare_outputs.ps1 -A <head.exe>  # 全例題の stdout/stderr/exit が同一か（⚠ 解釈側を触ったときはこちら）
./dump_native_ir.ps1 -OutDir <dir>   # 生成 LLVM IR（⚠ **ネイティブ codegen を触ったときはこれが主検査**・#64）
./prof_dist.ps1                     # 実行時間の段別/op 別分布（要 --features prof）
./bench.ps1                         # Phase R の各ステップ / Phase V の各段で再測定（フェーズ0基準 = bench_baseline.md）
cargo run -- --compile examples/interop/test_modules/physics.ar  # Phase R: native 経路の数値一致確認
cargo run -- <file.ar>              # フォールバックは無い。載らなければ VmForceError で停止（#25/#33）
./force_gate.ps1                    # 全例題で上記を回す「VM に載らない形が無い」ゲート（0 件・153 例題）
./tw_stats.ps1                      # 未対応箇所の件数・内訳（要 --features tw_stats）
./generate-codebase-map.ps1         # src/vm/ 等の新設後に必須
```

規約（`.claude/rules/regulations.md`）: 新文法の追加は非該当（ただし **VM の新しい適格範囲ごとに
例題を足す**のは #36/#41/#42 の教訓で必須）／VS Code 拡張・Python 実装は非該当（VSIX・SHA 同期不要）／
同じスクリプトの反復実行は .ps1 化する。

### 参照資料
- [IMPLEMENTATION_LOG.md](IMPLEMENTATION_LOG.md) — **実装の詳細・実測値・判断の根拠・保留タスクの調査記録**（本文書の切り分け先）。
- [PHASE_R1_RESULTS.md](PHASE_R1_RESULTS.md) — Phase R / Phase V-A〜V-E の実装詳細と実測（進捗の一次資料）。
- [bench_baseline.md](bench_baseline.md) — フェーズ0の全実測値と支配項の切り分け（各ステップの比較基準）。
- `c-abi-interop` スキル（設計仕様）／`codebase-map` スキル（`src/` の役割 + ファイル別行数）。

═══════════════════════════════════════════════════════════════════════════════
## 以降は「参照・根拠」（決定済み・履歴）— 実装再開には**読まなくてよい**
§3 アーキテクチャ決定 / §1 背景実測 / §7 速度投影 / §8 非目標 / §9 未決事項。
番号は初版のまま（本文中の相互参照 §3.4 等を保つため物理位置のみ末尾へ移動）。
═══════════════════════════════════════════════════════════════════════════════

## 4. Phase R — AST 解決層 ＋ フレーム/slot ランタイム（**共有基盤・本命**）　【✅ R1/R3/R4/codegen 消費 完了。R2-c は**ブロック中**（消費者不在）・R0-A(#12b) は**保留**】

「AST 展開時に、静的に決まる参照を slot / オフセット / 解決済みターゲットへ落とし、決まらない所は `Dynamic` 印」。
解釈経路・ネイティブ codegen の**両方**がこの解決結果を消費する。

### 4.1 成果物の表現
AST を破壊的に書き換えず、**ノードに解決結果を持たせる**（既存 `SlotCache`（[ast.rs:73](src/ast.rs#L73)）と同じ
「ノード埋め込み `Cell` / 付随フィールド」方式。§9-1 で最終確認）。決まらなければ `Dynamic` を保持し実行時に従来経路。

### 4.2 R0 ランタイムモデル（解釈器ストレージ改修）— **当初の想定は外れた**
当初は「`scopes: Vec<HashMap>` をフレーム/slot モデルへ改修するのが Phase R の主作業」としていた。
実装は `frame_floor` によるスコープ隔離＋ `Scope` の slot 配列化まで。**明示 `Rc<Frame>` スタック（#12b）は保留**。
- ⚠ **現在の支配項を指していない**（#12 の実測: 呼び出しコストの真因はフレーム構築ではなく
  per-call のヒープ確保 2 件）。ストレージ改修を再提案するなら必ず測り直すこと。
- ⚠ **VM はこの R0 ストレージを再利用していない**。VM 適格関数のローカルは共有 flat buf（`vm_stack`）で
  `scopes` を一切参照しない。

### 4.3 解決ステップ R1〜R4（各々独立コミット可）

| ステップ | 内容 | 消す辞書引き |
|---|---|---|
| **R1. ローカル/引数の slot 化** 【✅】 | 関数本体の変数を宣言順に slot 番号付け（B: フレーム内固定 slot）。`Expr::Ident` に `Resolved::Local{frame_level, slot}` を付与。決まらなければ `Dynamic`（§2.5） | scope HashMap 引き（~0.09µs/access） |
| **R2. グローバルの slot 化** 【保留 #11・§6 と連動】 | 既存 `SlotCache` を「実行時遅延」から「AST 展開時解決」へ前倒し。各 .ar ファイルは固有のグローバル配列を持ち index アクセス（§6 のモジュールモデルと接続） | epoch 検証つき実行時キャッシュ |
| **R3. フィールドのオフセット化** 【✅】 | 呼び出し点でオブジェクトの具象クラスが型チェッカから判れば `Expr::Attr` に `(class_id, idx)` を焼く（記憶域は §2.9 の通り既存）。判らなければ **多相 IC**: `InstanceData.class_id`（[value/core.rs:19](src/interpreter/value/core.rs#L19) `alloc_class_id`）で「前回と同じ class_id ならオフセット再利用、違えば `field_index` 引き直してキャッシュ更新」 | `field_index.get(attr)`（[eval/attrs.rs:70](src/interpreter/eval/attrs.rs#L70)） |
| **R4. 呼び先の解決** 【✅】 | Arrow 関数呼び出しを名前引きから解決済みターゲット（グローバル関数 index / 関数ポインタ）へ。`Expr::Call.cache`（[ast.rs:356](src/ast.rs#L356)）を Arrow 関数にも拡張。関数オブジェクトを変数に代入した場合は slot 内 Value の CALL ディスパッチ（名前引きなし） | 呼び先名前引き |

- **protocol 引数・template**: 原型 AST は `templates.rs` が保持（§2.2）。解決可能な呼び出し点は固定オフセットに焼き（monomorphize 相当）、
  真に多相な protocol 引数は R3 の多相 IC に倒す（＝「できねば辞書アクセス」の実体）。

### 4.4 ネイティブ codegen 側の消費 【✅ 完了（#16 段階 c-3）】
`llvm_codegen` の自前再導出（`locals`/`param_classes`/`field_ty`）を Phase R の解決結果に置換し、**codegen を簡素化 + 適用範囲拡大**
（今まで解決できず native 非適格だったケースが解決情報で救える）。

### 4.5 検証 + Phase V ゲート 【通過済み】
Phase R 完了時のゲート（呼び出し機構がなお支配的か）は通過し、Phase V は実施済み。
当時 0.53µs だった呼び出しオーバーヘッドは #12 で **0.138µs**。

---

## 5. Phase V — バイトコード VM（解釈経路の最終実行形）　【✅ V-A〜V-F ＋ 強制バイトコード(#3) ＋ ツリーウォーク削除(#33) まで完了】

入力は Phase R で解決済みの AST。バイトコード生成は解決ロジックを持たず軽い（起動バジェットは §7.3）。

### 5.1 モジュール構成（`src/vm/`）
> **実装差分**: ~~compiler/ サブ分割~~ は **#53 で実施済み**（下記）。`frame.rs` は**未分離のまま**
> （`run.rs` の 7 引数を束ねる話。VM 経路は `vm_stack` のフラット buf で足りているので消費者不在）。

`src/partial_compiler/` の構成に倣う:
```
src/vm/
  mod.rs          公開 API: compile_* / run
  op.rs           Op 列挙型（オペコード定義）
  chunk.rs        Chunk { code, consts, names, attr_caches, spans(行テーブル), local_names(デバッグ名テーブル), n_locals }
  compiler/       解決済み AST → Chunk（#53 で分割・全ファイル 1000 行未満）
    mod.rs          型（Compiler / CompileMode / ChunkMeta / StoreTarget / LoopCtx / BlockCtx）＋ base/finish
    entry.rs        公開入口 6 つとその _inner
    diag.rs         bail 診断フック・VM が扱える組み込み名の表
    decls.rs        slot 採番と AST 走査（⚠ リゾルバと同順・同数）
    emit.rs         命令発行のプリミティブ・書き込み先の決定・型特化の判定
    calls.rs        呼び出し（引数・FFI 情報・書き戻し先・async 投入）
    control.rs      try/finally/match 文と脱出時の巻き戻し
    stmt.rs         compile_stmt
    expr.rs         compile_expr
    block_expr.rs   ブロック式 5 種
  run.rs          ディスパッチループ本体（exec_op + ハンドラスタック）
  disasm.rs       逆アセンブラ（開発に必須・後回し厳禁）
```
- **`Value` は変更しない**（§7 の Value 表現改善は別テーマ）。値スタックは `Vec<Value>`（共有バッファ `vm_stack`）。
- **行テーブル・デバッグ名テーブルを Chunk の初期設計に入れる**（§2.3, 後付け不可）。

### 5.2 オペコード
**素案は役目を終えた。現物は [src/vm/op.rs](src/vm/op.rs) の `enum Op`**（doc コメントが仕様）。
素案との差分: 例外は静的テーブルではなく実行時ハンドラスタック（`SETUP_TRY`/`POP_TRY`）。
純粋組み込みは `CALL_BUILTIN`。デバッガは `LOAD_NAME`/`DECLARE_NAME`。`STORE_GLOBAL` は #10-b で追加。

### 5.3 制御フローのジャンプ化（TLS/センチネル除去がこのフェーズの成果物）
`block_return`/`loop_yield`/`break`/`continue` は**「どのブロックまで抜けるか」をコンパイル時に決定できる**ので、
`ExecResult` 伝播と `LOOP_DEPTH`/`BLOCK_YIELDS` スレッドローカル（§1.4）が**丸ごと消える**。
Arrow 特有の「`break` が入れ子の if/match/block を貫通して外側ループへ届く」規則も、コンパイル時のジャンプ先計算で自然に表現でき、
**実行時センチネル（`RAISE_SENTINEL`/`BREAK_SENTINEL`）が不要**になる。例外はフレームアンワインド + 例外テーブルで表現。
（**#33 で実削除まで完了** — TLS 3 本とセンチネル 2 種、`ExecResult` の 4 バリアント、
ツリーウォークの制御フロー実装をすべて削除した。`GENERATOR_YIELDS`（#8）と `RAISE_SENTINEL`（V-C）は
**VM が使うので残置**。詳細は実装ログ。）

### 5.4 段階（V-A 〜 V-F, 各段で テスト緑 + 穴の可視化 + ベンチ再測定）
> ⚠ 当初は各段の穴を `--vm=force` で可視化する想定だったが、長らく **`Force` は `Auto` と同一の
> no-op** だった。#25 でゲート化済み（[force_gate.ps1](force_gate.ps1)）。件数の可視化は `AR_TW_STATS`。
- **V-A〜V-D** 【✅】: 骨格（op/chunk/run/disasm）→ 算術・slot・制御フロー・呼び出し → クラス/メソッド/属性（R3 の IC）
  → 例外・match・ブロック式（TLS/センチネルを**VM 経路で不使用化**・実削除は #33）→ for・組み込み・Chunk キャッシュ。
  ※当時はクロージャ・最上位・import が残っていた（**その後すべて解消**:
  クロージャ=#27-d/#30/#45・最上位=#10-b/#10-c/#27-c・定義文脈の式=#41・モジュール本体=#42）。
- **V-E** 【✅ #1 で完了】: デバッガ統合（トレースバック・デバッグ名テーブル・REPL バイトコード実行）＋
  **文境界行テーブル（`stmt_spans`）と VM 内ステップ実行**。ツリーウォークへのデバッグ用フォールバックは撤去済み。
- **V-F** 【✅ #2a/#2b】: 最適化（peephole=`vm/peephole.rs`・superinstruction・単型算術命令）。
  ※ **R0-A エスケープ解析（#2c）は保留**（#12b が作るコストを取り消すだけのタスクで、VM 経路は既にフラット）。
- **完了時** 【✅ #3 → #33】: フォールバックを撤去し（#3）、さらに **`VmMode`・`--vm`・
  ツリーウォークの制御フロー・TLS・センチネルをすべて削除**した（#33・src 実質 **-762 行**）。
  ⚠ 入口（`run_program`・REPL・テスト・モジュール本体）は**解決情報を供給する責任**を持つ（#36/#42）。

---

## 保留・却下タスク（仕様と理由）

「残り」表からは外してある（**着手できないもの／着手しないと決めたもの**）。
再開するときは前提が変わったかを先に確かめること。

| # | タスク | 手法 | 前提（依存） | 状態 |
|---|---|---|---|---|
| 23 | 評価済み引数を名前付き struct 化 | 3 つ組 `(Option<String>, Value, bool)` に `source_name` を足す（現 **46 の型位置 / 16 ファイル**）。**`NativeFunction` を C 軸へ寄せる前提** | — | **→ #54 に吸収**（保留理由「速度効果なし」は #48 以前の評価。目的を「2 実装の畳み込み」へ再定義） |
| 12b | R0-A 明示フレームスタック | `Rc<Frame>` のスタック化 | — | **保留**（速度理由は #12 で消滅・依存は循環・A 自体に borrow コスト。詳細は実装ログ） |
| 15-3 | 文字列インターン（§7.4-3） | 属性名・メソッド名を `Rc<str>` + ポインタ比較 | — | 保留（**消費者 0 件と実測**・R3 IC が名前引きを既に潰済） |
| 2c | V-F R0-A エスケープ解析 | 非エスケープフレームのフラット確保 | **← 12b** | **保留**（12b が作るコストを取り消すだけのタスク。VM 経路は既にフラット） |
| 10-d | 定義文のオペコード化・import モジュール本体 | — | — | **保留**（計測で**両半分とも #3 に寄与しない**と判明: モジュール本体は 20 文・定義文は制御フローも TLS も持たない。詳細は実装ログ） |
| 28 | `Op::Rare` への畳み込み | 稀な 7 op を 1 アームに集約 | — | **却下**（実装して A/B した結果**何も回復せず** 1 件は悪化。**前提だった「op 1 個あたり ~1〜1.5%」というモデル自体が誤り**だった。詳細は実装ログ） |
| 24 | peephole パターンの追加 | 到達不能コード除去・`Const;Pop` 消去 | — | **却下**（実測で**実行時効果 0%**: 到達不能コードは原理的に速度へ効かず、`Const;Pop` は全例題 16838 命令中 **4 件**。詳細は実装ログ） |
| 46 | ループ反転（後方 JUMP の除去） | `L: cond; JUMP_IF_FALSE exit; body; JUMP L` を `JUMP L; body; L: cond; JUMP_IF_TRUE body` へ。**実行命令の 10〜12%** が無条件 `Jump`（`bench_arith` は `Jump` 18,000,000 : `JumpIfFalse` 18,000,006 ＝ **1:1**）。最内ループが **5 命令→4 命令**になる。⚠ **`JumpIfTrue` op の新設が要る**（#27: op を足すだけでベンチが 0.88〜0.94x 動く）。⚠ `code_target_mut` への登録を忘れない（#27-d で踏んだ） | — | **却下**（実装・テスト全緑まで行って A/B。**最大効果ケース（空ループ・命令数 -20%）でも 1.003x**＝無条件 `Jump` はこの VM ではほぼ無料。外れた前提は「**実行命令の割合 ≒ 時間の割合**」。詳細は実装ログ） |

### 統一の到達度
**型の解決**（#16 c-3）・**ローカルの解決**（#11 R2-a/R2-a′）は三経路とも到達済み。
**グローバルの解決**（#11 R2-b）はツリーウォーク／VM が共有（ネイティブは参照が無く保留）。

## 完了項目の一覧（履歴）

各項目は**事実と手法のみ**。実測値・判断の経緯は [IMPLEMENTATION_LOG.md](IMPLEMENTATION_LOG.md)。

### 経緯のメモ（必読部から移設・規約 3）

> **#31 完了 / #33 は部分完了**（2026-08-18）。`--vm` とツリーウォークの
> **関数本体・ジェネレータ・async** 経路は削除できた（src 実質 **-284 行**）。
> ⚠ **制御フロー本体は削除できなかった** — クラスのフィールド既定値のような
> **定義文脈の式**から生きていると実測した（→ #41）。
> ⚠ ここでも **`tw_control_flow` 0 は例題依存**だった（#36 と同じ構図）。しかも
> `if`/`match` 式には**計測フックが無く過小報告**していた（追加済み）。

> **#34 / #35 / #36 / #37 / #39 / #40 は完了**（2026-08-17）。**`--vm=off` でしか走らない
> 言語機能も入口も 0 になった**（133 形の総当たりで off/on 不一致 0・テスト 734 件が VM 経路）。
> ⚠ どれも「VM に載せる」だけでは終わらず、
> **ツリーウォーク側のバグと VM 側の意味論差**が芋づるで出た（詳細は実装ログ）:
> `continue` の貫通が無い 2 件（SyntaxError 化・**黙って握り潰し**）／**`try` のハンドラが残る**
> （`has_escape` は**文しか歩かない**）／**`block:` 文が `loop_yield` を吸い込む**／
> **`loop_yield` を脱出扱いしていた**（跳ばないのに丸ごと bail）。
> ⇒ *bail する形はツリーウォークが正しいとは限らない* の再演が続いている。
> **見つけ方は毎回同じ**: 形を総当たりして**参照実装（`impl_python`）**と突き合わせる
> （`--vm=off` との比較は #33 で失われた。[compare_python_impl.ps1](compare_python_impl.ps1) が代替）。

| # | 完了項目 | 手法 |
|---|---|---|
| 27 | fn の VM コンパイル失敗の解消 | 未解決 Ident=`LoadGlobal`／属性複合代入=融合を使わない 2 回評価経路／for シャドウ=本体の間だけ slot 差し替え／可変長=`local::args` を末尾 slot へ／`static`=`static_cells` 直読み／可変キャプチャ=セル表（**完了**（`vm_bail_fn` 49→**0**）） |
| 27-d | クロージャ本体の VM 化 | 段階 1（不変キャプチャ→末尾 slot・`captured_slots`）／段階 2a（`static`→`static_cells` 直読み）／段階 2b（可変キャプチャ→**slot と並行するセル表**・`LoadCell`/`StoreCell`）。⚠ クロージャ実体ごとに `FnValue` が別物で **Chunk を使い回せない**（→ #30）（**完了**（`vm_ineligible` 20→**0**）） |
| 27-c | 最上位 Chunk 化の bail 解消 | flat リスト組み込みの 1 実装化／`StoreLocalFromIdent`／`try/except/finally` は `try/except` を `try/finally` で包む／ブロック式内 `fn` の採番／一般の呼び先式／`CallBuiltinKw`・`CallMethodKw`（**完了**（`vm_bail_toplevel` 175→**0**・`force_gate` 36→**4**）） |
| 29 | `force_gate` の未判定を無くす | タイムアウトでも stderr を読む／`-Timeout` 45 秒／**kill の前に窓を閉じる**（繰り返し送る）（**完了**（未判定 5→**0**・128 例題すべて完走）） |
| 32 | async ブロック本体の VM 化 | `compile_async_body`（`compile_block_expr` ＋ `Return`）で Chunk 化。捕捉環境は `captured_slots`。`vm_mode` を worker へ伝搬し `Force` を効かせた（**完了**（**3.77x**・最上位ツリーウォークが定義文だけになった）） |
| 3 | 強制バイトコード（D2） | `VmMode` を `Off`/`On` へ畳み、`On` は載らなければ `VmForceError` で停止。⚠ **`On` にするのは `run_program` だけ**（REPL/テストは解決情報を持たないので壊れる）（**完了**（フォールバック撤去。**TLS/センチネル削除は #33 へ分離**）） |
| **34** | **制御フロー式を貫通する `break`/`continue` の VM コンパイル** | 跳ぶ前に「途中の式が積んだオペランド」を `Pop` する（深さ＝`stmt_base`）。深さは `BinOp` 左右・`UnaryOp` にだけ伝播し、**不明なら bail**（既定 `None` が安全側）。判定を `Stmt::Break` へ一本化し `block_body_bails` の 2 つ目の walker を撤去。囲むループが無い場合は bail せず `Op::Fail`（ツリーウォークと同文言）（**完了**（**新オペコード 1・既存 Chunk 不変**。ツリーウォークの `continue` バグ 2 件も修正）） |
| **35** | **`block_return`/`loop_yield` の実行時検査を VM へ** | `BlockCtx` に `->T` を持たせ `Op::CheckBlockReturn`/`CheckLoopYield` を発行（判定は `check_block_return_type`/`check_loop_yield_type` の**1 実装へ委譲**）。for/while 式の外の `loop_yield` は bail せず `Op::Fail`。⚠ `block:` **文**は TLS へ push しないので**外側の注釈を継承**し、**`loop_yield` には透過**（蓄積先を持たせない）（**完了**（新オペコード 2。`block:` 文の yield 透過バグも修正）） |
| **39** | **関数本体からのグローバル代入の VM コンパイル** | `store_target` の `toplevel_globals` 門を撤去（`cells`/`statics`/`slots` を全部外れた名前＝ローカルでもキャプチャでもない＝`scopes[0]`。読み側 `LoadGlobal` と同じ根拠）。⚠ **`vm_assign_global` を `assign_var` 委譲から `scopes[0]` 限定へ変更**（VM 関数は `scopes` を押さないので、委譲すると**呼び出し元のローカル**を走査してしまう）。デバッガ REPL（`debug_mode`）だけは名前引きが要るので bail 継続（**完了**（新オペコード 0。潜在的な健全性の穴も解消）） |
| **37** | **`try/finally` を跨ぐ `break`/`continue`/`return`/`block_return` の VM コンパイル** | `try_depth`/`finally_guard` を **`try_stack: Vec<Option<Vec<Stmt>>>`**（各 try の finally 本体）へ置換し、`emit_unwind_tries(keep, pop_except)` が**脱出経路へ finally 本体を複製**する（内側から）。バリアは `LoopCtx.try_len`／`BlockCtx.try_len`／`return` は 0。⚠ **`loop_yield` は跳ばないので `has_escape` から外した**（誤検知で丸ごと bail していた）（**完了**（新オペコード 0。`try/except` を跨ぐ脱出の bail も解消）） |
| **40** | **`finally` 本体そのものからの脱出の VM コンパイル** | `compile_finally_copy(fin, extra)` が**複製が載っているオペランド数**だけ `stmt_base` を持ち上げる（例外路 `[exc]`＝1・`return` 路の戻り値＝1）。跳ぶときにその分まで捨てるので **Python と同じ「保留中の動作を破棄する」**意味論になる。`block_return` は `BlockCtx.entry_depth` との差分を `Pop`。複製の増殖は `MAX_FINALLY_NEST` で頭打ち（**完了**（新オペコード 0。`has_escape` walker を**全廃**）） |
| **36** | **`Interpreter::new()` 消費者の VM 経路移行** | テストは `prepare()` に一本化（`resolve_program`＋`check_and_annotate`＋`set_toplevel_globals`＋`On`）／REPL はブロックごとに同じ配線（**globals は積み増し**・注釈は差し替え）。⚠ **`toplevel_vm_candidate` の `!toplevel_globals.is_empty()` を撤去**（最上位に宣言の無いプログラムが最上位丸ごとツリーウォークだった＝ゲートの穴）。⚠ **最上位 Chunk キャッシュは `Stmt` のアドレスをキーにするので AST を捨ててはいけない**（REPL がブロックごとに捨てて別文の Chunk を実行していた）（**完了**（734 テスト全件が VM 経路。[repl_session.ps1](repl_session.ps1) を新設）） |
| — | Phase R（R0/R1/R3/R4） | フレーム隔離・ローカル slot 化・属性 IC・呼び先解決・`Value` 32B 化 |
| — | Phase V-A〜V-E | VM 骨格・制御フロー・メソッド・例外ハンドラスタック・ブロック式・トレースバック・デバッガ REPL |
| 4〜9 | VM カバレッジ拡大 | メソッド高速バインド／添字・コレクション／組み込み拡張／テンプレート Chunk メモ化／ジェネレータ VM 化／関数内 async |
| 16 | **AST 型解決層**（本系列の主目的） | 型検査の結果を node-id 索引の注釈テーブルへ永続化し三経路が消費 |
| 13 | R4 ネイティブ codegen の消費（16 c-3 に包含） | 自前型導出を注釈の消費へ置換 |
| 18 | 順序比較の食い違い解消 | 実行時 `apply_binop` を型検査の仕様に合わせて 8 アーム追加 |
| 11 | R2-a / R2-a′ / R2-b | ネイティブが解決済み AST を消費・グローバル参照ノードの共有 |
| 14 の一部 | `.arc` 陳腐化検査 | 埋め込みソースと隣の `.ar` の照合 |
| 15b | `Ident` の AST 表現再設計 | node-id 付与＋型検査が参照サイトごとの型を焼く（**消費者 0 件と実測**・配線せず） |
| 15c | 識別子 3 変種の統合 | `Ident`/`LocalRef`/`GlobalRef` → `Expr::Ident { name, node_id, res: Resolution }` |
| 15 | `Value::Str` → `Rc<str>`（§7.4-1） | `Value`/`DictKey`/`Expr` の Str を `Rc<str>` 化・`Value::str()` へ集約（§7.4-3 インターンは消費者 0 件で保留） |
| 15d | 組み込み振り分け・traceback 名の是正 | **`res` ではなく実際の束縛**を見る（`builtin_is_shadowed`）。ツリーウォーク／VM 両方 |
| 20 | off/auto 比較に stderr と `_error` 例題を追加 | 45→**68 例題**・stderr 発火 4→**27 件**。負の対照で #15d-1/-2 の検知を確認 |
| 15e | 実行経路の `res` ゲート一掃 | 「`res` は記憶域のヒントで意味論の根拠ではない」で 18 サイトを分類し 13 サイトから条件除去。**実バグ 2 件**（`mut→let` コピー漏れ・FFI OutPtr 書き戻し漏れ）を修正 |
| 21-a | 最上位 `Unresolved` の調査・判断 | 三点比較で「`Global` 読みは `Local` 読みと同速」を実測（遅さの主因はグローバル**代入**だった）。**21-b を実装する判断** |
| 22-a | 呼び出しディスパッチの分類 | A(同定)/B(正規化)/C(実行方式) に仕分け。**C は 4 分岐で閉じる**と確認。C の 3 重実装から **off/auto 不一致 1 件を発見し修正**（`JsProcFn` 欠落） |
| 22-b | C 軸の統合 | `eval_call` の 11 アームを `call_value_evaled` への委譲へ置換。`Op::Call` に `node_id` を追加し **VM 経路の FFI 境界検査欠落**を解消。式が要る 3 呼び先のみ例外として残す |
| 22-c | C 軸の統合を完了 | `Namespace` アームを委譲・`Type` を移動・`instantiate` を 1 実装へ。**残る例外は `NativeFunction` のみ**（write-back に引数の変数名が要る） |
| 22-d | A 軸の整理 | 組み込み振り分けから冗長な `res` を除去。畳めない重複（`is_vm_builtin` ↔ `eval_builtin_evaled`）を **`VM_BUILTIN_NAMES` + 不変条件テスト**で固定 |
| 21-b | リゾルバを最上位文列へ | `resolve_toplevel` を追加し最上位の識別子へ `Resolution::Global` を付与（**1.19x**）。覆う名前は `collect_toplevel_shadowing` で除外 |
| 12 | 呼び出し機構の高速化 | `build_caller_frame` の遅延化＋関数名バッファのプール化で **per-call のヒープ確保 2 件を除去**。呼び出しオーバーヘッド **0.360→0.138 µs（2.61x）**・E2E 1.36x。**フレームスタック改修は不要だった** |
| 2b | V-F 単型算術命令 | 型検査が `Stmt::CompoundAssign` にも `binop_kind` を焼き、VM が `Expr::BinOp` と同じ融合＋特化経路へ委譲（**新 op ゼロ**）。`x += e` **1.9x**・カウントループ **1.60x**・E2E 1.00〜1.33x。命令列の再構成は #2a へ申し送り |
| 2a | V-F peephole | `src/vm/peephole.rs` を新設（Jump 連鎖畳み込み＋次命令 Jump 除去＋**コード索引の一括再マップ**）。併せて `obj.x += e` を 7→最短 5 命令へ（`GetAttrLocal` 化＋`Swap` 除去）。属性複合代入は変数版と**同速に到達**。peephole 単体は JUMP の 14.3% 除去だが総命令の 0.31%・E2E 分岐支配で **+4.4%**・他はほぼ 0 |
| 1-a | デバッガの自動検証を新設 | `examples/debugger/<name>.{ar,in}` の **5 シナリオ**（ステッピング transcript の比較）。**負の対照で検知力を確認**。⚠ #33 で golden 方式（`debug_session.ps1`）へ移行 |
| 1-b | デバッグ中の VM 無効化を最小化 | `should_pause_at` を読み「**停止し得るのは StepInto だけ**」と確定し `dbg_blocks_vm()` へ置換。step-over が跨ぐ重い呼び出しで **1.97x**・通常経路のコストはゼロ |
| 1 | V-E 本体（VM 内の文単位ブレーク） | 文境界行テーブル `stmt_spans` ＋停止判定つき専用ループ `run_stepping`（**通常ループには何も足さない**）／停止フレームのローカルを `local_names` から一時スコープへ。**既存バグ 1 件を修正**・ツリーウォークへのデバッグ用フォールバックを撤去 |
| 10-a | 最上位ツリーウォークの実測 | 診断フック `AR_TW_STATS`（feature `tw_stats`）＋ [tw_stats.ps1](tw_stats.ps1)。文種別×最上位/関数内、VM コンパイル成否、bail 地点（未帰属 catch-all つき）。**#10 の保留理由 2 点と「#3 の前提は #10 のみ」を実測で否定** |
| 27 の一部 | クロージャ: **外側関数**を VM 化 | `Op::MakeFn` ＋ `Chunk.fn_defs`。`nested_fn_captures` が「自由変数 ∩ 外側 slot」を求め**全て不変のときだけ**載せる（値を複製して `CapturedVar::Immutable`）。オーバーロード合成は `merge_fn_overload` に集約。`decl-prepass:FnDef` 8→**0**・`fn_FAILED` 17→**11**・`in_fn` 127→**111** |
| 27-c の一部 | 最上位 bail **175 → 126**（制御フロー文の bail 18→**10**） | リゾルバ 4 件（`enum`/`new_type` を globals へ・`AsyncAssign` の `target` は束縛でない・虚数リテラル・**VM へ渡す集合をシャドウ減算なしに**）＋ `block:` 文＋`for k, v in ...`（`Op::UnpackTuple` 1 個）＋ `collect_nested_decls` の `Stmt::Block` 漏れ。**計測の穴 3 件**も修正（意図的スキップ 336 件が失敗に化けていた／未帰属 46→3／bail を `<文種別>/<理由>` で切れるように） |
| 10-c2 | 最上位の残り文を Chunk 化 | 受理判定を**許可リスト→定義文の除外リスト**へ反転（式文・`if`/`try`/`match`・代入・属性代入…）。`exec()` の VM 試行を 5 アーム→**入口 1 箇所**へ集約。最上位 Chunk 499→**1,368**・ツリーウォーク **663**（うち 56% は定義文）。**計測フックの過大計上を修正** |
| 27-b | メソッド dispatcher の 1 本化 | `eval_method_call` を「引数評価＋委譲」だけにし 16 レシーバを評価済み版へ集約。`Instance` アーム 144 行と旧 evaled 版 85 行を委譲に畳む。`Op::CallMethod*` に `node_id` を足して FFI 戻り値検査を VM へ。`fn_FAILED` 29→**17**・`toplevel_FAILED` 163→**65**・実質 -230 行 |
| **31** | **参照実装（`impl_python`）との差分検査** | [compare_python_impl.ps1](compare_python_impl.ps1) を新設。**stdout だけ**を比べ、Rust は**既定モード**で走らせる。既知差分は**理由つきの `$knownDiff` に明示列挙**し、載っていない例題は既定で検査対象＝**新しい例題は自動的に検査される**。一致するようになった項目は STALE として報告。⚠ **`impl_python` は 100 コミット前（`33ef765`）に同期**なので既知差分が 36 件ある（**完了**（**46 検査・46 一致**／既知差分 36。負の対照 2 種で検知力確認）） |
| **41** | **定義文脈の式の VM 化** | `compile_definition_expr`（式 ＋ `Return` の Chunk）と `eval_definition_expr` を新設し、クラスのフィールド既定値・クラス変数・`enum` 値・デコレータの **7 箇所**を VM 経路へ。⚠ **自由な識別子は `LoadName`**（新フラグ `name_lookup`）— 定義文の実行位置は最上位とは限らない（モジュール本体の中など）ので `scopes[0]` 限定の `LoadGlobal` ではツリーウォークと答えが変わる（**完了**（新オペコード 0。[definition_context_expr.ar](examples/classes/definition_context_expr.ar) の `tw_control_flow` 6→**0**）） |
| **42** | **import モジュール本体の VM 化** | `Op::StoreName`（`assign_var` へ委譲＝チェーン探索）を 1 個追加し、`module_mode` / `compile_module_stmt` / `try_run_module_stmt` で `exec_module` の 2 つの実行ループを VM 経路へ。⚠ 宣言（`DeclareGlobal`→`declare_var`→`scopes.last_mut()`）と読み（`LoadName`）は**元からチェーンを見る**ので変更不要で、足りないのは**名前ベースの代入**だけだった（**完了**（[module_toplevel_flow.ar](examples/interop/module_toplevel_flow.ar) の `module_body` 18→**2（定義文のみ）**・`tw_control_flow` **0**）） |
| **43** | **実行時型検査の高速判定**（`loop_yield`/`block_return`） | `TypeTag`（**アノテーション文字列から**コンパイル時に決める種別）を op に載せ、実行時は**列挙比較 1 回**で済ませる。外れたときだけ一般判定（`check_*_type`）へ落とすので**エラー文言は 1 実装のまま**。`Any` は自明に真なので **op 自体を出さない**。⚠ **型推論の結果は使わない**（#15e の「注釈は意味論の根拠ではない」を破らないため）（**完了**（`loop_yield` 支配ベンチ **1.177x**＝検査を消した上限の **104%** を回収。`bench_block_expr` は 0.936→**1.016x**）） |
| **33** | **ツリーウォーク制御フローの削除**（本系列の主目的） | 4 層で削除（**src 実質 -762 行**）: A=`eval/control_expr.rs` 全体＋`eval()` の 5 アーム ／ B=`exec/control_flow.rs`（227→55・`make_for_iterator` だけ残す）＋`exec()` の 10 アーム ／ C=**TLS 3 本＋センチネル 2 種**＋`exec_loop_yield`／ D=`ExecResult` の 4 バリアント＋ツリーウォークの `try`＋`exec_block`/`exec_scoped_block`。⚠ `GENERATOR_YIELDS`（#8）と `RAISE_SENTINEL`（V-C）は **VM が使うので残す**。⚠ `eval()`/`exec()` のアームは**削除せず明示的なエラー**に畳んだ（到達したら黙って動かず落とす）（**完了**（着手前に `tw_stats` を全例題で再測定して 0 を確認）） |
| **38** | **A/B 計測スクリプトのデッドロック解消** | [ab_bench.ps1](ab_bench.ps1) の `Measure-Run` を**非同期読み**（`ReadToEndAsync`）に揃え、併せて**異常終了・タイムアウト・不在パスを黙殺しない**ようにした（`-TimeoutSec` 既定 180・失敗は min から除外して `!!`／`WARN:` で再掲）（**完了**（負の対照＝`AR_VM_DUMP=1` で修正前ハング→修正後完走。詳細は実装ログ）） |
| **30** | **クロージャ Chunk の実体跨ぎ再利用＋計測** | `get_or_compile_chunk` は `FnValue` ごとにキャッシュするので、**クロージャは実体ごとに再コンパイル**する。#27-d 段階 1 で初めて実際に走るようになった（それまでクロージャは VM 非対象）。⚠ **クロージャのベンチが 1 本も無い**ので、まず計測手段を作ってから判断する（未着手（**新たに生じた性能リスク**・唯一残る）） |
| **44** | **デバッガ golden の録り直し** | [debug_session.ps1](debug_session.ps1) が **#33 partial（`6bf039c`）からずっと FAILED**。同じコミットが「stdin を BOM 無しで書く」修正と「修正前の出力の golden」を**同時に**入れたため、golden に BOM 由来の**二重プロンプトと偽エラー**が焼き付いている。⚠ **`-Update` で黙って上書きしない**（差分の中身＝偽の 1 行が消えることを確認してから）。⚠ ステッピングを見ているゲートはこれだけなので**現状は無検査**（**完了**（golden を録り直して 5 identical。負の対照 3 種で検知力確認・`-Update` は差分表示するようにした）） |
| **45** | **`FnValue.body` の共有**（`Rc<[Stmt]>` 化） | #30 の実測で、クロージャ生成そのものに **1.05µs/実体**残っている。大半は `FnValue` が `body: Vec<Stmt>` を**実体ごとにディープクローン**する費用。`Rc` 共有にすれば消えるが `body` の消費者が多い。⚠ **#30 とは別軸**（#30 はコンパイル、これは AST 複製）（**完了**（本体サイズ依存を O(n)→O(1) 化。1 文 1.11x／10 文 4.07x／40 文 13.1x・`bench_closure` C-1 で 3.26x。⚠ `body.clone()` が「複製」から「参照カウント加算」へ黙って変わるので不変条件テスト 3 本で固定）） |
| 10-c | 最上位の宣言文を Chunk 化 | `Op::DeclareGlobal(name, DeclKind)`（**4 種を 1 op のオペランドに畳む**）＋ `compile_toplevel_stmt` が `let`/`mut`/`const` を受理。最上位ツリーウォーク **30,123 → 2,071**（#10 着手前から **5,425x 減**）。E2E は退行なし |
| 27-a/26 | レシーバ判定を**「形＋出自」の 2 段**に | 型検査に `arrow_class_names`（外部言語 import 本体の `ClassDef` を除外）を持たせ注釈経由で VM へ。slot を持たないレシーバ（グローバル/属性/呼び出し結果）も注釈から判定。最上位ツリーウォーク **120x 減**・`bench_method_call` **1.45x**・unsound だった 2 件を正しく弾いた |
| 27 の一部 | fn の VM コンパイル失敗 **49 → 29** | `pass`(3)／`break_point`(5, `Op::BreakPoint`＋`vm_debug_pause` 委譲)／`undefined`(1)／`obj::Trait.attr`(5, `Get/SetTraitAttr`＋`trait_*_evaled` へ委譲)／メソッド本体の `Self`(4, `Op::LoadSelfClass`)。bail 計上を fn/最上位に分離し未帰属を 6→2 に |
| 10-b | 最上位ループの Chunk 化 | `Op::StoreGlobal`（IC つき・`try_fill_slot` へ委譲）＋ `compile_toplevel_stmt`（`while`/`for` 文限定）。書き込み先判定は `resolver::toplevel_visible_globals` を共有。最上位ツリーウォーク **3.09x 減**・E2E **1.11〜1.28x** |
| 1-x | `exec_op` の `#[inline(always)]` | #1 で呼び出し元が 2 つになり `#[inline]` では展開されず 3〜5% 退行 → 明示指定。**元から展開されていなかった**ため通常経路が **1.06〜1.32x**（bench_arith 1.29x・bench_branch 1.31x） |

## 6. モジュール動的リンク仕様（D6 詳細）　【❌ 未実装 #14・前提が未実装のためブロック中】


ネイティブ経路（`.arc`）の安全な動的リンク。現状は `try_load_native_module` が **関数ごとに `GetProcAddress`**
（`{fn_name}_tl` シンボル）を引く。これを**モジュールにつき1回**へ改める。`.arc` フォーマットは
`partial_compiler/module_compiler.rs`（`write_tlc_native`, v1）を拡張。

### 6.1 各モジュール DLL がエクスポートするもの
- **単一ディスクリプタシンボル**（例 `__ar_module_descriptor`）— `GetProcAddress` はこれ1回だけ。ディスクリプタは:
  1. **エクスポート表**: グローバル変数/関数/型それぞれの `index → { name, kind, 型 or シグネチャ, 実体ポインタ or グローバル slot }`
  2. **モジュール ABI ハッシュ**: エクスポート表の（名前 + index + シグネチャ + 型レイアウト）の内容ハッシュ（u64/u128）
- 型も同様に**型エクスポート表**（型 index → { 名前, レイアウト/フィールド, kind }）+ 型 ABI ハッシュ。

### 6.2 各 import 側（`.arc`）ヘッダが保持するもの
- 自身のエクスポート表 + 自身の ABI ハッシュ。
- **呼び出す外部モジュールごと**に: モジュール識別子 + **コンパイル時に見た相手の ABI ハッシュ** + 焼いた
  「名前 → 期待 index」対応（照合・再解決用）。
- 呼び出す外部の型についても同様のリスト。

### 6.3 ロード時のリンク手順
```
for each import 辺 (自分 → B):
    desc_B = GetProcAddress(B.dll, "__ar_module_descriptor")   # モジュールにつき1回
    if desc_B.abi_hash == 自ヘッダに焼いた B の abi_hash:        # ハッシュ1個の比較 = O(1)
        安全確定。焼いた index をそのまま使用（B のグローバル/関数/型配列へ index アクセス）
    else:
        フォールバック:
          - 名前で再解決（B のエクスポート表の name→index）+ シグネチャ/型レイアウト照合
          - 一致すれば relink（新 index に差し替え）、不一致なら明示エラー（何が変わったか diff 報告）
```
- **関数オブジェクトを変数へ代入**した場合は index 直解決の限りではないが、**関数自体を関数ポインタで管理（変数として）**
  すれば実質的に名前参照は消える（呼び出しは slot/Value の CALL ディスパッチ）。

### 6.4 コスト概算（一度きりのロード時）
支配項は Arrow の表照合ではなく **OS の `GetProcAddress`**:

| 規模 | 素朴（関数ごと GetProcAddress + 文字列比較） | 本方式（ディスクリプタ1回 + ABI ハッシュ照合） |
|---|---|---|
| 小（数モジュール, ~50 sym） | ~0.1ms | **~10µs** |
| 中（~500 sym, 数十モジュール） | ~1ms | **~50µs** |
| 大（~5000 sym） | ~10ms | **<1ms** |

→ 安全検査つきでもリンクは実用上ほぼ無視できる。バイトコード生成コスト（§7.3）と同オーダー。

---


## 3. アーキテクチャ決定（確定事項と根拠）

### 3.1 【D1】共有 IR は AST 解決（X 案）。バイトコード→機械語（Y 案）は不採用

**問題**: 解釈経路とネイティブ経路の**両方**が共有する解決結果をどこに置くか。
- **X**: AST に解決表現を注釈（slot/オフセット/解決済みターゲット）。両経路が同じ注釈付き AST を消費。
- **Y**: バイトコードを共有 IR にし、ネイティブ codegen もバイトコードを消費して機械語へ。

**決定的事実**: ネイティブ codegen（`llvm_codegen`）は**既に AST を直接消費し、解決を自前で行っている**:

| codegen が既にやっている解決 | コード |
|---|---|
| ローカル名 → alloca レジスタ | `GenCtx.locals: HashMap<String,(reg,Ty)>` [context.rs:22](src/partial_compiler/llvm_codegen/context.rs#L22) |
| 引数名 → クラス | `param_classes` [context.rs:33](src/partial_compiler/llvm_codegen/context.rs#L33) |
| フィールド → 型付きオフセット(GEP) | `field_ty()` [context.rs:48](src/partial_compiler/llvm_codegen/context.rs#L48) |
| 二項演算を再帰でレジスタへ | `Expr::BinOp{left,right}` 依存（**木構造前提**） [expr.rs](src/partial_compiler/llvm_codegen/expr.rs) |

**X が軽い理由**:
1. codegen は既に AST 消費・解決済み。X は**表現を変えず**事前解決注釈を読むだけ（むしろ簡素化）。Y は入力を総取り替えし動くコードを捨てる。
2. Y はスタックバイトコード→機械語で**デスタック**（push/pop から SSA 復元）が要る。木構造が持つ情報を捨てて拾い直す方向で本質的に重い。
3. X の解決パスは、いまバラバラな4機構（`SlotCache`/`field_index`/`NativeCallCache`/codegen の `locals`）を「AST 展開時に一度」統合したもの。

→ **X 採用。ネイティブ経路は Phase R の解決付き AST を入力に保つ**（Y=バイトコード→機械語は §8 非目標）。

### 3.2 【D2】解釈実行は強制バイトコード（最終状態）。移行中のみデュアルモード

- **最終状態**: 通常のプログラム実行は**常にバイトコード VM**を通る。関数単位のツリーウォークフォールバックは**除去**。
- **`eval()`/`exec()` は残置**（§2.1 `parse_ar` / §2.3 デバッガ REPL / 対話 REPL のツール経路が使う）。「ツリーウォーク廃止」は
  **通常実行経路からの除去**の意味であり、`eval()` 関数自体を消すことではない。
- **移行中（Phase V 実装期間）はデュアルモード**が生命線: 未対応構文はツリーウォークにフォールバックし、`cargo test` を常時緑に保つ。
  ```
  関数を初めて呼ぶとき:
    compile(fn) が Ok(chunk)          → VM 実行、chunk キャッシュ
    compile(fn) が Err(Unsupported)   → 従来 exec_fn_evaled にフォールバック（移行中のみ）
  ```
  `--vm=off` / `--vm=auto`（既定）/ `--vm=force`（フォールバック禁止・#25 でゲート化）の CLI フラグ。
  V-A〜V-F 完了時に全構文対応が済んだらフォールバックを撤去し強制バイトコードに切替。

### 3.3 【D3】スタックマシン

現在の `eval()` が既にスタック的評価構造なので、移植が「式ごとに push/pop へ書き下す」機械作業になる。レジスタ VM の
上積み（命令数減）は IC や §7 より優先度が低い。将来 superinstruction で一部回収。

### 3.4 【D4】変数解決モデル R0（A / B / C）

Phase R/V の**ランタイム表現の土台**。解釈のフレーム/slot ストレージと、バイトコード VM の両方がこのモデルを使う。

**A. 参照カウント付きフレームのスタック** — フラット1本スタックではなく、**フレーム(活性化レコード)のスタック**。
- 関数に入るたび**フレーム(サイズ既知のローカル領域)を生成**しフレームスタックに push。フレームは `Rc` 管理。
  関数を抜けるとフレームスタックから pop、参照カウント 0 で破棄。
- 関数は「現スコープから見えるフレーム群 + グローバル」への参照を持ち、**(フレーム階層, index)** でローカル/クロージャ変数を解決
  （ハッシュなし・非局所は Rc を1段たどる）。
- **クロージャ**: フレームスタックから pop されても、クロージャが掴むフレームは参照カウントが 0 にならず**生き続ける**。
  → フラット案の「open/closed upvalue 開閉処理」「slot に直値/セル混在」が**不要**になり、クロージャが Rc 寿命管理で自動成立。
  **コード単純化がこのモデルの主目的**。
- **トレードオフ（実測ベース）**: 呼び出しごとにフレームをヒープ確保するため、呼び出しパスの伸びはフラット案 ~4x に対し
  **~2x（0.53µs→~0.25–0.35µs）**。ローカルアクセスの ~30x は不変。
- **後段最適化（V-F）**: `capture_env` の自由変数解析で「クロージャに捕捉されない=エスケープしないフレーム」を判定し、
  **非エスケープフレームだけフラット確保**に落として呼び出し速度を回収。まず一様ヒープフレームで正しさ・単純さを取る。
- 単スレッド `Rc` で可（async は D5 の share-nothing で per-thread フレーム）。

**B. 関数内は固定 slot** — 関数内の if/while/for/block は**実行時に push/pop せず**、全ローカルにフレーム内固定 slot を割り当てる
（フレームサイズ = 同時生存ローカルの最大数、コンパイル時算出）。ブロックスコープはコンパイル時にのみ強制。
→ **関数内の実行時スコープ操作が消える**。Arrow は可視名の再宣言を禁止
（[dispatch.rs:31](src/interpreter/exec/dispatch.rs#L31) 付近の "already declared" 検査）＝**シャドウイングなし**なので slot 割り当てが素直。
ブロックを跨ぐ同名の slot 再利用/寿命解析のみ要実装。

**C. 動的名はほぼ静的解決可** — `self`/可変長 `local::`/`kwargs`/import バインド名/`Self`/`static` は
**識別子が AST 内リテラルのため slot/型index へ静的解決可能**。「通常実行の変数アクセスから実行時ハッシュは消える」。
**残余は実行後にパースされるコードのみ**:
- **デバッガ REPL**: 生フレームへの任意式 → **slot→名前 デバッグメタデータ**で解決（ホットパス外, §2.3）。
- **対話 REPL**: 各行を逐次コンパイル → **リゾルバがインクリメンタルに global slot を採番**（実行時ハッシュ不要）。
- テンプレートは実体化ごとに解決（名前は既知＝静的解決の範疇, §2.2）。
- 補足: 属性/メソッドの**多相**ディスパッチは slot ではなく `class_id` インラインキャッシュ（§4.3 R3）。動的名とは別機構。

### 3.5 【D5】async は share-nothing 維持 【✅ #9 で関数内 async を VM 化（capture を frame から再構成し `capture_env` 再利用）。share-nothing 不変】
現行 async（`mng <- async->T: body`）は**投入時 deep-clone = 共有可変状態なし**。これを維持。
- 過剰 deepcopy 削減: **読み取りのみキャプチャは不変 Arc 共有** / **書込み要は COW** / **投入後に main が使わない変数は move**。
- 「async が main の mut 変数を編集」したい場合は**明示的 Mutex 系プリミティブ**（CLAUDE.md 記載の将来機能・別テーマ）。
- **フレーム暗黙共有による mut 編集は不採用**（`Arc<Mutex>` 化 → ロック → データ競合 → R0-A の `Rc` 前提が壊れる）。

### 3.6 【D6】モジュール動的リンク = ディスクリプタシンボル + ABI ハッシュ
ネイティブ経路（`.arc`）の安全な動的リンク方式。**詳細仕様は §6**。要点:
- 各モジュール DLL は**単一のディスクリプタシンボル**をエクスポート（`GetProcAddress` はモジュールにつき1回）。
  ディスクリプタはエクスポート表（グローバル/関数/型の index↔名前↔シグネチャ↔ポインタ）+ **モジュール ABI ハッシュ**を指す。
- import 側は「コンパイル時に見た相手モジュールの ABI ハッシュ + 焼いた index」を自ヘッダに保持。
- ロード時: import 辺ごとにディスクリプタを1回引き、**ABI ハッシュ1個を比較**（O(モジュール数)）。一致=安全確定、
  不一致=名前で再解決するか明示エラー。
- **支配項は OS の `GetProcAddress`（~1–5µs/sym）**なので「関数ごとに引かない」が肝。概算リンク時間: 中規模 ~50µs / 大規模 <1ms。

---

## 1. 背景 — 現在の実行モデルと実測ベースライン（実コード確認済み）

### 1.1 実行パイプライン

```
source → lexer → parser → type_check → Interpreter::exec(&Stmt) / eval(&Expr) を再帰呼び出し
```

- `exec()` = [exec/dispatch.rs:16](src/interpreter/exec/dispatch.rs#L16) の巨大 match。`Stmt` バリアントごとに専用メソッドへ委譲。
- `eval()` = [eval/core.rs:16](src/interpreter/eval/core.rs#L16) の match。同様。
- 関数呼び出しは **Rust の再帰**。`exec_fn_evaled`（[functions/execution.rs:31](src/interpreter/functions/execution.rs#L31)）が
  スコープ退避 → 新スコープ構築 → `exec_block` → 復元。

### 1.2 変数アクセスのコスト構造

```rust
type ScopeMap = HashMap<String, Var, FxBuildHasher>;   // interpreter.rs:187
scopes: Vec<ScopeMap>                                   // 0=グローバル, 末尾=最内
```

- `get_var` は**末尾→先頭へ線形にスコープを遡り、各段で文字列ハッシュ**（[scope.rs:30](src/interpreter/scope.rs#L30)）。
- `get_val` は `Var::get_value()` = **`v.clone()`**（[interpreter.rs:219](src/interpreter.rs#L219)）。
  → **変数を1回読むたびに `Value` が1つクローンされる**。`Value::Str(String)` なら毎回ヒープ確保。

### 1.3 既存の場当たり最適化（Phase R/V で不要になる = 置換・撤去対象）

| 既存最適化 | 場所 | 目的 | 置換後 |
|---|---|---|---|
| `FxHasher` | interpreter.rs:150 | 変数名ハッシュ高速化 | **撤去**（名前引きが消える） |
| `SlotCache`（epoch 付き） | [ast.rs:73](src/ast.rs#L73) | グローバル代入のスコープ検索回避 | **撤去**（グローバル索引化, R2） |
| `global_slot_cells` / `slot_epoch` | interpreter.rs:268 | 同上 + `freeze` 時の一括無効化 | **撤去** |
| `Expr::Call { cache }` | [ast.rs:356](src/ast.rs#L356) | 呼び先解決キャッシュ | R4 の解決 + 多相 IC へ発展 |

> `SlotCache` を AST に埋め込んでいる時点で、既に「AST を可変な実行表現として使う」方向にある。Phase R/V はその延長線上。

### 1.4 制御フローの3系統（Phase V で構造的に消える）

1. `ExecResult` enum の戻り値伝播 — `Normal`/`Break`/`Continue`/`Return`/`BlockReturn`/`BlockYield`/`Raise`
2. **スレッドローカル 4本**（[interpreter.rs:117-136](src/interpreter.rs#L117)）
   - `LOOP_DEPTH`（break/continue 妥当性）/ `GENERATOR_YIELDS`（yield 収集）/
     `BLOCK_YIELDS`（loop_yield 収集）/ `BLOCK_RETURN_EXPECTED_TYPE`（block_return 実行時型検査; 式ごと push/pop）
3. **文字列センチネル** — `RAISE_SENTINEL` / `BREAK_SENTINEL` を `Result<Value,String>` の `Err` に載せ、
   実体は `self.current_exception` に置く（`eval()` が `RaisedError` を返せない回避策）

→ Phase V ではジャンプ命令 + 例外テーブルに解消。**速度以前に設計上の大整理**。（VM 経路では不使用化済み・実削除は D2。）

### 1.5 フェーズ0 実測（支配項の切り分け・ゲート）【完了 2026-07-21, 詳細 [bench_baseline.md](bench_baseline.md)】

`examples/bench/bottleneck_bench.ar`（要因分離, N=100万）+ `bench_field_access.ar`（E2E）, release, 各3回・安定。

| 要因 | コスト | 誰が潰すか |
|---|---|---|
| **関数呼び出し機構**（scope HashMap 構築 + ExecResult 伝播） | **0.53 µs/call** | Phase V（フレーム）+ R0 |
| **引数束縛**（1個: eval + HashMap insert + clone） | **~0.7 µs/arg** | Phase V + R |
| **ローカル変数アクセス**（HashMap 引き・**SlotCache 無し**） | **~0.09 µs/access** | Phase R（slot 索引） |
| AST ノードディスパッチ（enum match + `Box<Expr>` 追跡） | baseline 0.13µs に内包 | Phase V（線形走査） |
| Value deep_clone（複合値） | ~0.13 µs | §7 のみ（バイトコードでは不変） |
| フィールド読み | ~0.15 µs | Phase R（offset/IC） |

- **決定的な発見**: グローバルは `SlotCache` で索引化済みだが、**関数内ローカルは毎回 HashMap 引き**。slot 化の伸びしろ最大。
- **§7（`Value::Str(Rc<str>)` 等）は数値コードでは支配項でない**（deep_clone ~0.13µs）。文字列多用時のみ効く二次テーマ。
  ※ 本ベンチは int/float 中心で String クローンを踏んでいない点に留意。

---

## 7. 速度・コスト投影

### 7.1 定性的効果
| 項目 | 効果 |
|---|---|
| 変数アクセス | ハッシュ + スコープ遡り → **配列インデックス1回** |
| 制御フロー | ExecResult 伝播 + TLS borrow → **ジャンプ1命令** |
| 深い再帰 | Rust スタック依存 → **明示フレームスタック**（スタックオーバーフローが消える） |
| コードの整理 | センチネル文字列2種・スレッドローカル4本が**構造的に不要になる** |
| 将来性 | lazy generator、末尾呼び出し、プロファイル駆動最適化への**足場** |

### 7.2 定量投影（フェーズ0実測からの積み上げ・実測ではない）
効くのは**ディスパッチと名前解決**のみ。**Value 操作・FFI・実演算には効かない**。

| 成分 | ツリーウォーク(実測) | バイトコード+R(投影) | 倍率 | 担当 |
|---|---|---|---|---|
| ローカル変数読み | 0.09 µs (HashMap) | ~0.003 µs (`stack[base+slot]`) | ~30x | Phase R |
| フィールド読み | 0.15 µs (HashMap+offset) | ~0.03 µs (offset) | ~5x | Phase R |
| ノードディスパッチ | baseline 0.13µs に内包 | 線形走査 | ~2–3x | Phase V |
| 関数呼び出し機構 | 0.53 µs | ~0.25 µs（R0-A ヒープフレーム; フラットなら ~0.12） | ~2x（フラット ~4x） | Phase V + R0 |
| 引数束縛 | ~0.7 µs/arg | ~0.15 µs/arg | ~4–5x | Phase V+R |
| 制御フロー | ExecResult+TLS | ジャンプ | ~2–5x | Phase V |
| **Value clone/コピー** | 0.13 µs | 0.13 µs | **1x** | §7.4 のみ |
| **FFI/ネイティブ** | 固定 | 固定 | **1x** | `--compile` |

- **実測ベンチ適用**: 空 while 0.13→~0.04µs(~3x) / 引数なし呼び出し 0.53→~0.20–0.25µs(R0-A) / E2E `kinetic()` 3.13→~1.0–1.2µs(~3x)。
- **ワークロード別**: 呼び出し/ローカル/制御フロー支配 = **3〜5x**（R0-A のフレーム確保ぶん上限やや低下）/ 典型混在 = **2〜3x** /
  clone・コレクション・文字列支配 = **1.3〜2x** / FFI 支配 = **~1x**。
- **Phase R と V の切り分け**: Phase R 単独（注釈付きツリーウォーク + R0 ストレージ）で **~1.3–2x**、Phase V 上乗せで **さらに ~1.5–2.5x**。
- **上限を抑える要因**: `Value` は約80バイト（`JsProcFn` が String×3=72B 内包）。スタック push/move/clone で毎回 ~80B コピー。
  §7.4 の一部（大 variant の Box 化）を並行すると呼び出し/引数束縛の高速化上限が上がる。（§7.4-2 は実施済み・72→32B。）

### 7.3 起動コストのバジェット（強制バイトコード＝全プログラムが生成を通る）
`scratchpad/gen_*.ar`（規模違いのパース支配プログラム）で実測代理:
- **パース+型検査 = ~2.9 µs/行**（250/1000/4000 関数で線形・一定）。
- **バイトコード生成 = 0.3〜1.0 µs/行**（AST 1walk + emit。パース/型検査より単純ゆえその一部が上界）。

| 規模 | 一括生成コスト | 判断 |
|---|---|---|
| 100 行 | 0.03〜0.1 ms | プロセス起動 ~8.5ms に埋もれる＝無視可 |
| 700 行（spider 全体） | 0.2〜0.7 ms | 体感不能 |
| 5,000 行 | 1.5〜5 ms | 許容 |
| 24,000 行 | 7〜24 ms | この規模はパース+型検査で既に ~70ms |

**設計指針**: 強制でも**関数単位の遅延生成（初回呼び出し時コンパイル + キャッシュ）**を採る。デッドコードの生成費用を払わず起動が
規模フラット。ホット関数は初回 ~数µs のみ（呼び出し1回 0.53µs＝生成 ~3µs は ~6回で回収）。モジュール本体は一括生成でよい。

### 7.4 併走を検討すべき別テーマ（本計画と独立）
1. **`Value` クローンコスト削減** — `Value::Str(String)` → `Value::Str(Rc<str>)`。変数を読むたび String ヒープ確保（§1.2）。【✅ #15】
2. **`Value` サイズ削減** — `JsProcFn`（String×3=72B）等の大 variant を `Box` 化して `size_of::<Value>()` を縮小。スタック操作が軽くなる。【✅ 72→32B】
3. **文字列インターン** — 属性名・メソッド名を `Rc<str>` + ポインタ比較。【保留 #15-3・消費者 0 件と実測】

---

## 8. 非目標（今回やらないこと）

- **ジェネレータの lazy 化**（意味論変更・新機能。別提案として切り出す, §2.7）
- **`Value` 表現の変更**（§7.4 として独立管理。Phase R/V は `Value` を変更しない）
- **`eval()` / `exec()` の削除**（`parse_ar` / デバッガ REPL / 対話 REPL が使い続ける。D2 の「ツリーウォーク廃止」は通常実行経路からの除去の意）
- **バイトコード → 機械語（§3.1 Y 案）**。ネイティブ経路は Phase R の解決付き AST を入力に保つ
- **async の share-mutable 化**（D5）。既定は share-nothing 維持、共有可変は明示プリミティブ、フレーム暗黙共有は不採用
- **`impl_python/` の追従**（Rust 側のみ。規約の git SHA 同期は非該当）
- **JIT**（バイトコード VM 安定後の話）

---

## 9. 残る未決事項（着手時に確認・小さな判断のみ）

> ⚠ **1 / 2 / 4 は #51 で決着させた**（実装が先に答えを出していたのに、ここが未決のまま残っていた
> ＝ 文書側の陳腐化。src のコメントで起きたのと同じ現象）。**残る未決は 3 と 5 だけ**で、
> どちらも保留中の #12b / #14 に従属する。

1. **解決注釈の持ち方** 【✅ 決着（#51）— 実際は**両方採用**している】
   - **AST 埋め込み**（`Cell`/付随フィールド）: `SlotCache`（[ast.rs:85](src/ast.rs#L85)）・
     `AttrCache`（[ast.rs:165](src/ast.rs#L165)）・`Expr::Call.cache`。
   - **node-id 副表**: `AstAnnotations`（型・検査指示）・`Chunk::ffi_call_info`（#27-b）・
     `Chunk::wb_targets`（#48）。
   - ⇒ **規則**: *ノード局所で毎回引く情報は AST 埋め込み・Chunk 生成時に確定して実行時は読むだけの
     情報は node-id 副表*。⚠ 「前者推奨」だけを読んで #48 の副表方式と分岐させないこと。
2. **R3 の型チェッカ連携** 【✅ 決着 — #16 段階 c-3 で完了（§4.4）】
3. **R0-A フレームの内部表現**: `Rc<RefCell<Vec<Value>>>` か `Rc<Frame>`（`Frame` に inline 配列 + 借用管理）か。RefCell borrow の
   パニック表面とコストを見て決定。まず素直な `Rc<RefCell<...>>` で正しさ優先、V-F で最適化。
   ⚠ **← #12b（保留）に従属**。VM 経路は `vm_stack` のフラット buf で既に解決済みなので、消費者がいない。
4. **ブロック跨ぎ同名の slot 再利用**（B） 【✅ 決着 — 「既出名はスキップ＝slot 再利用」で実装済み】
   ⚠ 不変条件は **「採番はリゾルバと同順・同数」**（`push_base` される名前は `slots` に入れなくても
   必ず 1 slot 消費する）。破ると `LoadLocal` が範囲外を読む。
5. **循環 import のリンク**（§6）: A⇄B の相互参照は「全シンボル宣言（index 採番）→本体解決」の2フェーズで解く。
   ⚠ **← #14（ブロック中）に従属**。
