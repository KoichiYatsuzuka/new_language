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

## 🚩 次スレッドへの引き継ぎ（2026-08-13・**ここだけ読めば再開できる**）

### 現在地
**#16（AST 型解決層）＝本系列の主目的は完了**。三経路（ツリーウォーク／VM／ネイティブ）が同一の型解決注釈を消費する。
続けて #18・#11 R2-a/R2-a′/R2-b・#14 の一部・#15b・#15c・#15・#15d・#20・#15e・#21-a・#22-a・#22-b・#22-c・#22-d・#21-b・#12・#2b・#2a・#1・**#10-a/#10-b** まで完了。
git の HEAD は `56b8e66 "#10 (partially)"`（#10-a/#10-b までコミット済）。**#27・#27-a・#26 は未コミット**。

**→ #3（強制バイトコード）の前提は #10 だけではないと実測で判明**（下記 #10-a）。
最上位（#10-c/#10-d）と**関数の VM コンパイル失敗**（#27）の両方が要る。#27 は **49 → 29** まで進行。

直近の要点だけ先に:
- **#27 で 20 件解消**（`pass`／`break_point`／`undefined`／`obj::Trait.attr`／メソッド本体の `Self`）。
  ⚠ **`break_point` は `vm_debug_pause` を通すこと**。`exec_breakpoint` を直接呼ぶと
  REPL からそのフレームのローカルが見えない（`dbg_vars.ar` が off/auto 不一致で検出）。
- **#27-a/#26 完了。レシーバ判定を「形＋出自」の 2 段にした**（最上位ツリーウォーク **120x 減**・
  `bench_method_call` 1.45x）。`NamedInstance` は **Arrow クラスと外部言語スタブを区別しない**ので、
  型検査に `arrow_class_names`（外部 import 本体の `ClassDef` を除外）を持たせて出自を見る。
  ⚠ **健全化でカバレッジが 2 件減った**。従来 unsound にコンパイルしていた分で、これは正しい変化。
  ⚠ **当初の #27-a 定義（16 レシーバの畳み込み・644 行）は #26 には過剰**だった。
  残 12 件の非 Arrow レシーバ用に**畳み込み自体は #3 の前提として残る**。
- **⚠⚠ opcode を 1 つ足すごとに VM 支配ベンチが ~1〜1.5% 落ちる**（#27/#27-a で確定）。
  `exec_op` が `#[inline(always)]` なので**アームを足すとディスパッチループ全体が太る**。
  **どの op かではなく何個足したかで決まる**（実行されないダミー 4 op で同じ差が再現）。
  **ノイズ床のプローブは変更と同規模でなければ無意味**（#10-b は小関数 1 本で測り誤結論した）。
  → **opcode 追加の可否を E2E ベンチの数 % で判断しない**。判断材料は「`--vm=off` でも同じ差が出るか」
  と「同規模プローブとの比較」。**#3 に向けては `Op::Rare` への畳み込みを検討**（実装ログ）。
- **#10-a で「#3 の前提は #10 のみ」が誤りと判明**。`AR_TW_STATS` の実測でツリーウォークの負荷は
  **実質すべて最上位**（1123 万回中 99.8%）だが、**fn の VM コンパイル失敗も 49 件**残っていた。
  ⚠ **保留理由は経年で腐る**。#10 の保留判断（2026-07-27）は 2 点が実測で覆った。
- **#10-b で最上位ループを Chunk 化**（最上位ツリーウォーク **3.09x 減**・E2E 1.11〜1.28x）。
  欠けていた基本要素は記録どおり **`StoreGlobal` op ただ 1 つ**だった（読み側は #21-b で既に揃っていた）。
  ⚠ **性能の罠を 4 件踏んだ**（索引キャッシュ／診断フック 11%／`exec_op` の展開／関数内ループの巻き添え）。
  詳細は実装ログ。**いずれも実測でしか見つからない**ので、この周辺を触るときは必ず A/B すること。
- **#1 完了。デバッグ中も VM を使う**（`run_stepping` が文境界で停止判定）。既存バグを 1 件発見・修正。
  ⚠ **`exec_op` は `#[inline(always)]` が必須**（呼び出し元が 2 つになると `#[inline]` では展開されず
  3〜5% 退行。付けたら元より 1.06〜1.32x 速くなった＝元から展開されていなかった）。
  ⚠ デバッガ／`vm_eligible`／`run.rs` に触ったら [compare_debug_modes.ps1](compare_debug_modes.ps1) を必ず回すこと。
- **#2a/#2b**: peephole は総命令の 0.31%（E2E は分岐支配で +4.4%）。型特化の穴は op ではなく
  **文の側**にあった（`x += e` に `binop_kind` 未付与・1.9x）。
  ⚠ **コード索引を持つ op は飛び先だけではない**（`ForIter` の exit・`SetupTry` の handler）。
  追加時は `code_target_mut` へ（唯一の窓口・テストで固定済み）。
  ⚠ **`Expr` に注釈を付けたら同じ演算の `Stmt` 側も見ること**（複合代入・属性複合代入）。
- **#15b / #15-3 は消費者 0 件**（実測）。速度目的で再訪する価値は無い。
  ⚠ `Value::Str` を複製する新コードでは **`deep_clone` だけは独立バッファ必須**（async の share-nothing）。
- **#22 系列**: 同じ判断をする 2 実装は**片方を他方への委譲にして畳む**（22-b/22-c）。
  畳めないもの（コンパイル時 vs 実行時）は**不変条件をテストで固定する**（22-d）。
  ⚠ **`*_evaled` 版とずれた実装を作らない** — 実バグ **4 回**（#22-a/-b/-c・#10-b′ の `CsObject`）。
- **#15e/#15d の原則**: **注釈（`res`・型）は最適化ヒントであって意味論の根拠ではない**。
  破ると「同じコードが書かれた場所で挙動を変える」（実バグ 4 件: `mut→let` のコピー漏れ・
  FFI OutPtr の書き戻し漏れ・組み込み振り分け・**#26 の `NamedInstance`≠Arrow クラス**）。
  ⚠ `Resolution::Unresolved` は**「シャドウが無い」を意味しない**（最上位・テンプレート本体・
  合成 AST は常に `Unresolved`）。健全性の判定は**実際の束縛／出自**を見ること。

### 統一の到達度（この系列の本来の目的）
```
                  ツリーウォーク   VM      ネイティブ
型の解決               ✅          ✅        ✅   ← #16 c-3
ローカルの解決         ✅          ✅        ✅   ← #11 R2-a / R2-a′
グローバルの解決       ✅          ✅        —    ← #11 R2-b（ネイティブは参照自体がほぼ無く保留）
```

### 次にやる候補（推奨順）
下の「残り — 依存関係つき一覧」から、前提が空欄のものを推奨順に:

1. **#10-c 最上位の残りを Chunk 化** — #3 の前提。`DeclareGlobal` op を足して
   `let`/`Assign`/`if`/`try`/式文を覆う。#10-b の `StoreGlobal` と同じ形で足せる。
   最上位に残るツリーウォークは **30,123 件**で、うち `Let` 14,412・`LoopYield` 14,018 が
   **`for`/`while` 式**（`mut xs = for i in ...: loop_yield ...`）＝ここが次の山。
2. **#27-b レシーバ dispatcher の畳み込み** — #3 の前提。残 14 アーム・644 行。
   #27 の残り 12 件（非 Arrow レシーバ）もこれで片付く。
3. **#27 の残り** — 入れ子 `fn`（クロージャ）8 が最大。可変長 2・`static` 2 は小粒。

> **⚠ 速度の残余は「最上位」に寄っている**（2026-08-13 更新）。#12（2.61x）・#2b（1.6x）・#2a（+4%）・
> #1-x（1.06〜1.32x）に加え **#10-b で 1.11〜1.28x**・**#26 で method_call 1.45x**。残るのは #10-c。
> 新しく速度課題を立てるなら**まず計測して支配項を出すこと**。
> ⚠ #1-x の教訓: **`#[inline]` は効いているとは限らない**。巨大関数は LLVM が却下する。
> ホットループの関数はインライン展開されているかを疑うこと（実測 1.3x が眠っていた）。

> **⚠ #12b / #2c は保留**（2026-08-12・循環依存。2c の目的は VM 経路が既にフラットなので達成済み、
> 速度理由も #12 で消えた）。再訪条件と根拠は実装ログ。

> **⚠ #19 / #17-a / #17-b（外部接続系）は優先度を下げる。**
> いつでも着手でき、かつ**本系列（実行方式の統一・高速化）とは独立**した外部言語境界の話題。
> 実行系のリファクタリングが一段落してから、別レーンとしてまとめて扱う。

### 作業の進め方（この系列で有効だった型）
- **推測せず先に計測する**。#12 が決定的な例: 計画書は支配項を「フレーム構築 ~630ns/call」としていたが、
  実測すると 0.360 µs で、しかも真因は**フレームとは無関係な per-call のヒープ確保 2 件**
  （成功時の `build_caller_frame` と関数名の `to_string`）。
  **設計変更ゼロ・数行の修正で 2.61x** を得た（`Vec<Rc<Frame>>` 化は不要だった）。
  本系列では見積もりが何度も外れた
  （c-2「効果あり」→実質デッドコード／c-3「効果小」→ 1.73x／(b)(i)「IC 削減」→ 真因は `Value` clone）。
  診断フックを足して数字を見てから設計を決めること。
- **検証は 4 点セット**: `cargo build`（**警告 0**）・`cargo test`（**705 緑**）・
  [compare_vm_modes.ps1](compare_vm_modes.ps1)（off/auto byte-identical）・
  [scan_examples.ps1](scan_examples.ps1)（例題 **FAIL 0**）。
  **デバッガ／`vm_eligible` に触るときは追加で [compare_debug_modes.ps1](compare_debug_modes.ps1)**
  （ステッピングの off/auto 一致。4 点セットのどれもこの経路を覆えない）。
  **codegen を触るときは追加で [dump_native_ir.ps1](dump_native_ir.ps1) の IR byte-identical を必ず確認**
  （1 箇所の取りこぼしで関数が非適格になり IR が変わるので、最強の検査になる）。
  `cargo clippy` は **HEAD 時点で既存警告が 62 件ある**（サマリ行を除いた実件数）。総数で判断せず**増分 0** を確認すること
  （既存分は `ast.rs` の `IfExpr` 命名・`cpp_bridge` の prefix 剥がし等で、本系列とは無関係）。
- 大きな変更の前後で **A/B 実測**する（同一ビルドで emit のみを切り替える）。
- 速度効果が大きくなくても、コードやロジックの簡素化が見込めるのであれば、それをメリットとして認識する。
- 全てのタスクは番号で管理する。タスク名だけを提示しない。番号付けされていないタスクは新たなタスクとして昇格を提案する。
- **高リスク低リターンと判断したらスキップして保留にし、理由を記録する**（勝手に大改造しない）。
  保留の判定基準としてこの系列で実際に効いたのは「**消費者が居るか**」— #11 R2-c（`CB_GET_GLOBAL` 3 箇所）・
  #14（照合対象の辺が 0）・#15b（型を落とす識別子読みが 0 件）はいずれもこれで保留になった。リターンとは実行速度効果だけでなく、コードやロジックの簡潔さへの貢献を含む。

### 検証・計測スクリプト（リポジトリ直下）
| スクリプト | 用途 |
|---|---|
| [scan_examples.ps1](scan_examples.ps1) | 全例題をタイムアウト付きで実行し、失敗のみ理由付きで列挙 |
| [compare_vm_modes.ps1](compare_vm_modes.ps1) | `--vm=off` / `--vm=auto` の **stdout + stderr** byte-identical 検証（ヒープアドレスは正規化）。`_error` 例題も対象（#20）。退避は `-SkipErrorExamples` |
| [dump_native_ir.ps1](dump_native_ir.ps1) | 代表 6 モジュールの生成 LLVM IR を保存（`.arc`/`.ars` は退避・復元） |
| [annot_diff.ps1](annot_diff.ps1) | 注釈の充填状況・binop 特化の内訳・`Unresolved` の発生源を出力 |
| [annot_unresolved.ps1](annot_unresolved.ps1) | 全例題を `AR_ANNOT_DIFF=1` で実行し `Unresolved` の発生源を式種別ごとに集計 |
| [ab_bench.ps1](ab_bench.ps1) | 2 つの `arrow.exe` を**交互実行**して経過時間を比較（`-A head.exe -B new.exe`）。#2b で新設 |
| [compare_debug_modes.ps1](compare_debug_modes.ps1) | **対話デバッガのステッピング**が off/auto で byte-identical か検証（`examples/debugger/<name>.ar` ＋ `<name>.in`）。#1 で新設 — `compare_vm_modes.ps1` は stdin を与えないのでこの経路を覆えない |
| [ab_bench_vm.ps1](ab_bench_vm.ps1) | `ab_bench.ps1` の **`--vm=<mode>` 付き**版。「退行が VM 経路由来か」を切り分ける唯一の手段（#10-b で新設）。⚠ 交互実行必須 |
| [tw_stats.ps1](tw_stats.ps1) | **ツリーウォークが実際に実行している文**を全例題で集計（`AR_TW_STATS`）。feature `tw_stats` 付きビルドを自動で行う。#10-a で新設 |
| [tw_stats_files.ps1](tw_stats_files.ps1) | 同上の例題別内訳（集中度・失敗例題の特定） |
| [run_examples.ps1](run_examples.ps1) | 既存の素朴な例題ランナー（タイムアウトなし） |
| [bench.ps1](bench.ps1) | ベンチ一式 |

### 診断フック（環境変数）
| 変数 | 効果 |
|---|---|
| `AR_DUMP_LL=<path>` | `--compile` 時に生成 LLVM IR を保存 |
| `AR_ANNOT_DIFF=1` | 注釈の充填状況・binop 特化の内訳・`Unresolved` の発生源・slot 索引読みの件数・`AnnotIdent`（識別子読みの型落ち内訳・#15b） |
| `AR_VM_DUMP=1` | VM の生成バイトコードを逆アセンブルして stderr へ |
| `AR_TW_STATS=1` | ツリーウォークの実行内訳（文種別×最上位/関数内）・VM コンパイル成否・bail 地点。**`cargo build --features tw_stats` が要る**（既定ビルドではコードごと消える。env 判定だけにすると `exec()` 1 文ごとの atomic 読みで 11% 退行する） |

### 落とし穴（既知）
- **PowerShell 5.1 は BOM 無し `.ps1` を ANSI として読む**。日本語コメント入りスクリプトは
  UTF-8 **BOM 付き**で保存しないと構文エラーになる。`Set-Content` での Rust ソース書き換えも文字化けの原因
  （`.NET` の `File.WriteAllText` + UTF8Encoding(false) を使う）。
- **native exe の stderr を `2>&1` で受けると PS5.1 が ErrorRecord 化**して exit 0 でも失敗扱いになる。
- `Start-Process -PassThru` の `ExitCode` は当てにならない。`System.Diagnostics.Process` を直接使う。
- **Rust ソースの一括書き換えは Python で `encoding='utf-8'` を明示**して読み書きする（`Set-Content` は文字化けする）。
  ただし**そのスクリプトから日本語を `print` すると コンソールの cp932 で `UnicodeEncodeError` になり、
  書き込み前に落ちて変更が丸ごと消える**。進捗表示は ASCII か件数だけにすること。
- **A/B は当該変更だけを切り替えて取ること**（#21-b）。前回測定から時間が空いた値と比べると
  他の変更やマシン変動を誤って帰属する。実際 `bench_field_access` の退行を 21-b のせいと誤認し、
  `resolve_toplevel` の呼び出しを外して測り直して初めて無関係と判った。
- **HEAD のバイナリが要る A/B・IR 比較では `git stash push -- src/` で退避**してビルドする。
  実行前に必ず `git status` を確認し、`src/` をスクラッチパッドへコピーしておく（過去に未コミット変更の破棄事故あり）。
- **診断フックを実行経路に足すときは feature で消せる形にする**（#10-a）。`exec()` は文ごとに呼ばれるので
  `OnceLock` の atomic 読み 1 回でも **11% 退行**した。`cfg!(feature=..)` を先に見て定数 false にする。
- **`exec_op` は `#[inline(always)]`。op のアームに重い本体を書かない**（#10-b）。その op を使わない Chunk の
  ホットループまで 4〜6% 遅くなる。逆に**全部外へ出すと**その op を使う側が 7〜10% 損する。
  → **IC はヒット経路だけインライン・ミス経路は `#[inline(never)]`**。
- **退行を疑ったら「そもそも触っているか」を先に見る**。`AR_TW_STATS` で top-level chunk 0 件、
  または **`--vm=off` でも同じ差が出る**なら VM 経路とは無関係（#10-b で 3 件、#27 で全件を誤帰属しかけた）。
- **⚠ ノイズ床のプローブは「変更と同規模」で測る**（#27）。opcode を足す規模の摂動は、
  **1 命令も実行しなくても** `partial_call_overhead` を 0.884x・`bench_branch` を 0.944x 動かす。
  小関数 1 本のプローブ（#10-b・0.993x）は規模不足で「配置に鈍感」と誤結論させた。
- **A/B を自前で測るときは必ず交互実行**（A,B,A,B…）。A を N 回 → B を N 回だとサーマルドリフトで
  後者が 10% 級に不利になり、実在しない退行が見える（`ab_bench.ps1` は交互実行している）。
- **`compare_vm_modes.ps1` / `scan_examples.ps1` は `target
elease` を見る**。
  `cargo build`（debug）だけして「直った」と判断しないこと（#10-b で 1 度踏んだ）。
- **`$ErrorActionPreference='Stop'` のスクリプトから `cargo` を呼ぶと、進捗の stderr で終了エラーになる**。
  その呼び出しの間だけ `Continue` に落とす（`tw_stats.ps1` 参照）。
- **`Rc` を含む値をスレッドへ送る経路（async）では `deep_clone` が独立バッファを作ること**（#15）。
  参照カウントが非アトミックなので共有したまま送ると壊れる。回帰検知は
  [async_string_share.ar](examples/async/async_string_share.ar)（**接触回数を減らすと検知力を失う**）。

---

## 実装状況（2026-08-11 時点）

各段の実装詳細・実測は [PHASE_R1_RESULTS.md](PHASE_R1_RESULTS.md)。off/auto の byte-identical を常時維持。

### 完了 ✅

各項目は**事実と手法のみ**。実測値・判断の経緯は [IMPLEMENTATION_LOG.md](IMPLEMENTATION_LOG.md)。

| # | 完了項目 | 手法 |
|---|---|---|
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
| 1-a | デバッガの自動検証を新設 | `compare_debug_modes.ps1` ＋ `examples/debugger/<name>.ar`+`.in` の **5 シナリオ**。off/auto のステッピング transcript を比較。**負の対照（`dbg_active()` ゲート除去）で検知力を確認**。これで #1-c の既存バグが可視化された |
| 1-b | デバッグ中の VM 無効化を最小化 | `should_pause_at` を読み「**停止し得るのは StepInto だけ**」と確定し `dbg_blocks_vm()` へ置換。step-over が跨ぐ重い呼び出しで **1.97x**・通常経路のコストはゼロ |
| 1 | V-E 本体（VM 内の文単位ブレーク） | Chunk に**文境界行テーブル**（`stmt_spans`・code と 1:1）／停止判定つき専用ループ `run_stepping`（**通常ループには何も足さない**・入口と `Flow::NextAfterCall` で入る）／停止フレームのローカルを `local_names` から一時スコープへ（`local_names` の**最初の消費者**）。**既存バグ 1 件を修正**（VM フレームへの step-out が停止しなかった）。**ツリーウォークへのデバッグ用フォールバックを撤去** |
| 10-a | 最上位ツリーウォークの実測 | 診断フック `AR_TW_STATS`（feature `tw_stats`）＋ [tw_stats.ps1](tw_stats.ps1)。文種別×最上位/関数内、VM コンパイル成否、bail 地点（未帰属 catch-all つき）。**#10 の保留理由 2 点と「#3 の前提は #10 のみ」を実測で否定** |
| 27-a/26 | レシーバ判定を**「形＋出自」の 2 段**に | 型検査に `arrow_class_names`（外部言語 import 本体の `ClassDef` を除外）を持たせ注釈経由で VM へ。slot を持たないレシーバ（グローバル/属性/呼び出し結果）も注釈から判定。最上位ツリーウォーク **120x 減**・`bench_method_call` **1.45x**・unsound だった 2 件を正しく弾いた |
| 27 の一部 | fn の VM コンパイル失敗 **49 → 29** | `pass`(3)／`break_point`(5, `Op::BreakPoint`＋`vm_debug_pause` 委譲)／`undefined`(1)／`obj::Trait.attr`(5, `Get/SetTraitAttr`＋`trait_*_evaled` へ委譲)／メソッド本体の `Self`(4, `Op::LoadSelfClass`)。bail 計上を fn/最上位に分離し未帰属を 6→2 に |
| 10-b | 最上位ループの Chunk 化 | `Op::StoreGlobal`（IC つき・`try_fill_slot` へ委譲）＋ `compile_toplevel_stmt`（`while`/`for` 文限定）。書き込み先判定は `resolver::toplevel_visible_globals` を共有。最上位ツリーウォーク **3.09x 減**・E2E **1.11〜1.28x** |
| 1-x | `exec_op` の `#[inline(always)]` | #1 で呼び出し元が 2 つになり `#[inline]` では展開されず 3〜5% 退行 → 明示指定。**元から展開されていなかった**ため通常経路が **1.06〜1.32x**（bench_arith 1.29x・bench_branch 1.31x） |

### 残り — 依存関係つき一覧

依存の凡例: 「前提」が空欄＝**今すぐ着手できる**。`←X` は X が終わるまで着手できない。

| # | タスク | 手法 | 前提（依存） | 状態 |
|---|---|---|---|---|
| 23 | 評価済み引数を名前付き struct 化 | 3 つ組 `(Option<String>, Value, bool)` に `source_name` を足す（27 箇所）。**`NativeFunction` を C 軸へ寄せる前提** | — | 保留（高コスト・速度効果なし） |
| 12b | R0-A 明示フレームスタック | `Rc<Frame>` のスタック化 | — | **保留**（速度理由は #12 で消滅・依存は循環・A 自体に borrow コスト。詳細は実装ログ） |
| 15-3 | 文字列インターン（§7.4-3） | 属性名・メソッド名を `Rc<str>` + ポインタ比較 | — | 保留（**消費者 0 件と実測**・R3 IC が名前引きを既に潰済） |
| 2c | V-F R0-A エスケープ解析 | 非エスケープフレームのフラット確保 | **← 12b** | **保留**（12b が作るコストを取り消すだけのタスク。VM 経路は既にフラット） |
| 10-c | 最上位の**残り**を Chunk 化 | ループ以外の最上位文（`let`/`Assign`/`if`/`try`/式文…）。`DeclareGlobal` op が要る。定義文（fn/class/import）は #10-d | — | 未着手（#3 の前提の片方） |
| 10-d | 定義文のオペコード化・import モジュール本体 | `exec_module` の本体も Chunk 化。定義文は既存 exec への委譲 op で足りるか要検討 | **← 10-c** | 未着手（#3 の前提の片方） |
| 27-b | `eval_method_call_evaled` を CallArg 版と同じ 16 レシーバへ | 残 **14 アーム・644 行**（`Instance`/`PyObject` は済）。`exec_signal_method` 等の `&[CallArg]` ヘルパに評価済み版が要る。**#27 の残り 12 件（非 Arrow レシーバ）はこれ待ち** | — | 未着手（**#3 の前提**） |
| 27 | fn の VM コンパイル失敗の解消（残 **29**） | 内訳: **入れ子 `fn`（クロージャ）8**・**非 Arrow レシーバ 12（← 27-b）**・可変長 2・`static` 2・for ターゲットのシャドウ 1・未帰属 2 | 一部 **← 27-b** | 進行中（**#3 の前提**・49→29 まで完了） |
| 24 | peephole パターンの追加 | `peephole.rs` へ足す（到達不能コード除去・`Const;Pop` 消去等）。**#2a の実測では JUMP 除去だけで総命令の 0.31%**なので、追加は「効く形を実測してから」 | — | 未着手（低優先・効果は要実測） |
| 3 | 強制バイトコード（D2） | フォールバック撤去＋TLS4本/センチネル2種の実削除 | **← 10-d ＋ 27**（#1・#10-b は完了） | ブロック中（本系列の最終段） |
| 11 R2-c | グローバル記憶域の index 配列化 | ネイティブの index 参照 | **← 消費者の出現（14 と同時に再評価）** | ブロック中（消費者不在） |
| 14 | §6 モジュール動的リンク | ディスクリプタシンボル＋ABI ハッシュ照合 | **← モジュール間ネイティブ直リンクの導入**（未実装・未計画） | ブロック中 |

```
モジュール間ネイティブ直リンク（未計画）→ 14 → 11 R2-c
（12b → 2c は循環依存のため両方保留）
10-c → 10-d ┐
27 ─────────┴→ 3    ← #3 の前提は「最上位の全カバー」と「関数の失敗 0」の両方（#10-a の実測）
27-b → 27 の残り 12 件（非 Arrow レシーバ）
（#22 系列・#2 系列・#1・#10-a/#10-b は完了）
```

### 別レーン — 外部接続系（**優先度低**・本系列と独立）

実行方式の統一・高速化とは**交差しない**（型検査とスタブ側の話題で、実行時ディスパッチに触れない）。
いつでも着手でき、いつ着手しても他タスクをブロックしない。実行系が一段落してからまとめて扱う。

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
| D2 | **解釈実行は強制バイトコード**（最終状態）。移行中のみデュアルモード | §3.2 |
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

### 2.4 クロージャは定義時に名前でキャプチャ — `capture_env` 【❌ クロージャは現状 bail】
[blocks.rs:39](src/interpreter/exec/blocks.rs#L39) が本体の自由変数を走査、外側から拾って `HashMap<String, CapturedVar>` を作る。
可変変数は `Var::Mutable` → `Var::Cell(Rc<RefCell<Value>>)` に**その場で昇格**して共有。
→ **バイトコード化と相性が良い**。自由変数解析をコンパイル時に前倒しすれば R0-A のフレーム参照になる。`Rc<RefCell>` 表現は流用。

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

### 2.8 その他（影響小・意味論変更なし）【🔶 FFI/overload は実行時委譲・import VM は残 #10】
- **FFI 経路**（DLL / cpp_bridge / cs-dll / cs-proc / js-proc / PyO3）は全て `eval_call` の先。`CALL_NATIVE` 系に集約されるだけ。
- **オーバーロード解決**は評価済み引数への実行時ディスパッチ（`dispatch_overload`）。当面そのまま実行時。
- **import は実行時にモジュールを実行**して名前空間を作る。モジュール単位 Chunk、初回 import 時コンパイル。

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

## 4. Phase R — AST 解決層 ＋ フレーム/slot ランタイム（**共有基盤・本命**）　【✅ R1/R3/R4/codegen 消費 完了・R2-c/R0-A は残 #11/#12】

「AST 展開時に、静的に決まる参照を slot / オフセット / 解決済みターゲットへ落とし、決まらない所は `Dynamic` 印」。
解釈経路・ネイティブ codegen の**両方**がこの解決結果を消費する。

### 4.1 成果物の表現
AST を破壊的に書き換えず、**ノードに解決結果を持たせる**（既存 `SlotCache`（[ast.rs:73](src/ast.rs#L73)）と同じ
「ノード埋め込み `Cell` / 付随フィールド」方式。§9-1 で最終確認）。決まらなければ `Dynamic` を保持し実行時に従来経路。

### 4.2 R0 ランタイムモデルの導入（解釈器ストレージ改修）— **Phase R の主作業**
Phase R で解釈経路の速度が上がるには、**解釈器の変数ストレージを `scopes: Vec<HashMap>` から §3.4 の
フレーム/slot モデルへ改修する必要がある**（注釈を付けるだけでは不十分）。この改修が Phase R の実質的な主作業。
- フレームスタック（`Vec<Rc<Frame>>` 相当）を `Interpreter` に導入。`Frame` はサイズ既知のローカル領域。
- `get_var`/`declare_var`/`assign_var`（[scope.rs](src/interpreter/scope.rs)）を slot 索引アクセスに置換。
- ネイティブ codegen はストレージ改修不要（自前 alloca 保持）。**解決注釈だけを消費**。
- この R0 ストレージは **Phase V のバイトコード VM がそのまま再利用**する（フレーム/slot は共通）。
- 現状: `frame_floor` によるスコープ隔離＋ `Scope` の slot 配列化まで実装。**明示 `Rc<Frame>` スタック（#12b）は保留**。
- ⚠ **この節は Phase 0 の測定に基づく。現在の支配項を指していない**（#12 の実測: 呼び出しコストの真因は
  フレーム構築ではなく per-call のヒープ確保 2 件だった）。ストレージ改修を再提案するときは必ず測り直すこと。
- ⚠ **VM は結局この R0 ストレージを再利用していない**。VM 適格関数のローカルは共有 flat buf（`vm_stack`）で、
  `scopes` を一切参照しない。上の「Phase V がそのまま再利用する」は実装と食い違っている。

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

### 4.5 検証 + Phase V ゲート
- **検証**: `cargo test` 672 緑 + `run_examples.ps1` 回帰 + `bench.ps1` 再測定（**R1/R3 で名前引きコストが消えるはず**）
  + `--compile examples/interop/test_modules/physics.ar` が従来と数値一致。
- **中断可能性**: R1〜R4 は各々独立コミット・単体で価値あり。R1（ローカル slot）だけでも支配項の一角に効く。
- **ゲート**（当時の値。現在は #12 で 0.138µs）: Phase R 完了時に `bench.ps1` を再測定。呼び出し機構(0.53µs)・ノードディスパッチがなお支配的なら Phase V の
  効果（§7 の投影）が裏付けられる。強制バイトコード（D2）が最終目標なので Phase V は実施前提だが、この測定で設計を確認する。

---

## 5. Phase V — バイトコード VM（解釈経路の最終実行形）　【✅ V-A〜V-F・強制バイトコードは残 #3】

入力は Phase R で解決済みの AST。バイトコード生成は解決ロジックを持たず軽い（起動バジェットは §7.3）。

### 5.1 モジュール構成（`src/vm/`）
> **実装差分**: compiler/ サブ分割（stmt/expr/control）・frame.rs は未分離。単一 `compiler.rs` ＋ 共有バッファ方式
> （→ [IMPLEMENTATION_LOG.md](IMPLEMENTATION_LOG.md) の「実装メモ」節）。

`src/partial_compiler/` の構成に倣う:
```
src/vm/
  mod.rs          公開 API: compile_fn / compile_debug / run
  op.rs           Op 列挙型（オペコード定義）
  chunk.rs        Chunk { code, consts, names, attr_caches, spans(行テーブル), local_names(デバッグ名テーブル), n_locals }
  compiler.rs     Compiler 本体・関数単位のコンパイル（Phase R の解決注釈を読む）／ compile_debug（デバッガ）
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
（VM 経路では達成済み。実削除はデュアルモード撤去＝強制バイトコード D2 時 #3。）

### 5.4 段階（V-A 〜 V-F, 各段で 672 緑 + `--vm=force` で穴可視化 + ベンチ再測定）
- **V-A〜V-D** 【✅】: 骨格（op/chunk/run/disasm）→ 算術・slot・制御フロー・呼び出し → クラス/メソッド/属性（R3 の IC）
  → 例外・match・ブロック式（TLS4本+センチネル2種を**VM 経路で不使用化**・実削除は D2）→ for・組み込み・Chunk キャッシュ。
  ※クロージャは残。最上位/import は #10-b で一部、残りは #10-c/#10-d。
- **V-E** 【✅ #1 で完了】: デバッガ統合（トレースバック・デバッグ名テーブル・REPL バイトコード実行）＋
  **文境界行テーブル（`stmt_spans`）と VM 内ステップ実行**。ツリーウォークへのデバッグ用フォールバックは撤去済み。
- **V-F** 【✅ #2a/#2b】: 最適化（peephole=`vm/peephole.rs`・superinstruction・単型算術命令）。
  ※ **R0-A エスケープ解析（#2c）は保留**（#12b が作るコストを取り消すだけのタスクで、VM 経路は既にフラット）。
- **完了時** 【❌ #3】: デュアルモードのフォールバックを撤去し**強制バイトコード**へ（D2）。

---

## 10. 検証コマンド / 規約

```
cargo test                          # 672 passed を維持（各ステップ/各段ごと）
cargo build                         # 警告0 を維持
cargo clippy --all-targets          # exit 0
./run_examples.ps1                  # 例題スイートの回帰確認
./bench.ps1                         # Phase R の各ステップ / Phase V の各段で再測定（フェーズ0基準 = bench_baseline.md）
cargo run -- --compile examples/interop/test_modules/physics.ar  # Phase R: native 経路の数値一致確認
cargo run -- --vm=force <file.ar>   # Phase V 移行中: 未対応構文を可視化（フォールバック禁止）
./generate-codebase-map.ps1         # src/vm/ 等の新設後に必須
```

規約（`.claude/rules/regulations.md`）:
- 新文法の追加はないため example / `_error` example 追加は非該当。
- VS Code 拡張・Python 実装に変更が及ばないため VSIX 再生成・git SHA 同期は非該当。
- 同じスクリプトを繰り返し実行する場合は .ps1 化（`bench.ps1` は作成済み）。

### 参照資料
- [IMPLEMENTATION_LOG.md](IMPLEMENTATION_LOG.md) — **実装の詳細・実測値・判断の根拠・保留タスクの調査記録**（本文書の切り分け先）。
- [PHASE_R1_RESULTS.md](PHASE_R1_RESULTS.md) — Phase R / Phase V-A〜V-E の実装詳細と実測（進捗の一次資料）。
- [bench_baseline.md](bench_baseline.md) — フェーズ0の全実測値と支配項の切り分け（各ステップの比較基準）。
- [.claude/skills/c-abi-interop](.claude/skills/c-abi-interop/SKILL.md) — オフセットアクセス記憶域（`InstanceData` raw ブロック）の設計仕様。
- `codebase-map` スキル — `src/` のディレクトリ別役割 + ファイル別行数。

<br>

═══════════════════════════════════════════════════════════════════════════════
## 以降は「参照・根拠」（決定済み・履歴）— 実装再開には**読まなくてよい**
§3 アーキテクチャ決定 / §1 背景実測 / §7 速度投影 / §8 非目標 / §9 未決事項。
番号は初版のまま（本文中の相互参照 §3.4 等を保つため物理位置のみ末尾へ移動）。
═══════════════════════════════════════════════════════════════════════════════

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
  `--vm=off` / `--vm=auto`（既定）/ `--vm=force`（未対応で失敗させ穴を可視化）の CLI フラグ。
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

1. **解決注釈の持ち方**: AST 埋め込み（`Cell`/付随フィールド, `SlotCache` 流儀）か 別テーブル（node-id キー）か。
   前者はノード局所で速いが AST 型が肥大、後者は AST を汚さないが間接引き。→ **前者推奨**（既存流儀）。
2. **R3 の型チェッカ連携**: 呼び出し点でオブジェクトの具象クラスを渡せる API が `type_check/` にあるか。着手時に推論結果の受け渡し方を確認。
3. **R0-A フレームの内部表現**: `Rc<RefCell<Vec<Value>>>` か `Rc<Frame>`（`Frame` に inline 配列 + 借用管理）か。RefCell borrow の
   パニック表面とコストを見て決定。まず素直な `Rc<RefCell<...>>` で正しさ優先、V-F で最適化。
4. **ブロック跨ぎ同名の slot 再利用**（B）: ブロックを抜けた後の同名変数の slot 寿命解析の正確な規則。Arrow のブロックスコープ意味論を
   リゾルバ構築時に確認（可視名再宣言禁止＝シャドウなしは追い風）。※現状は「既出名はスキップ＝slot 再利用」で実装済み。
5. **循環 import のリンク**（§6）: A⇄B の相互参照は「全シンボル宣言（index 採番）→本体解決」の2フェーズで解く。
