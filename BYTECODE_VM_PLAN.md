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

## 🚩 次スレッドへの引き継ぎ（2026-08-17・**ここだけ読めば再開できる**）

### 現在地
**Phase R（AST 解決層）と Phase V（バイトコード VM）は完了**。三経路（ツリーウォーク／VM／
ネイティブ）が同一の型解決注釈を消費し、**解釈実行は強制バイトコード**（D2）になった。
完了: #16・#18・#11 R2-a/R2-a′/R2-b・#14 の一部・#15 系列・#20・#21・#22 系列・#12・#2a/#2b・
#1・#10-a/-b/-c/-c2・#25・#26・**#27 系列（-a/-b/-c/-d 全段）**・**#32**・**#29**・**#3**。

**🎉 本系列の主目的は達成した**（2026-08-17）。`force_gate` **0 件・128 例題すべて完走**、
**制御フローを持つツリーウォークは 1 文も残っていない**（`vm_bail_fn`/`vm_bail_toplevel`/
`vm_ineligible`/`in_fn` すべて 0・`tw_control_flow` も 0）。最上位の 348 件と `module_body` の
20 件は**全て定義文**（設計上インタプリタが実行する・#10-d）。
**#3 のフォールバック撤去も完了**（`VmMode` は `Off`/`On` の 2 値・`On` は載らなければ停止）。

**→ 残るは `--vm=off` 廃止レーン（#34/#35/#36 → #33）と、独立レーンの #30/#31。**

**⚠ #33 の前提は誤りだった**（2026-08-17 実測）。「TLS 4 本は `--vm=off` のためだけに生きている」
としていたが、**`VmMode::Default` は `Off`** なので `Interpreter::new()` を直接使う
**REPL（1 箇所）と単体テスト（14 箇所）も同じコードを踏む**。さらに **`--vm=on` では実行できない
正しいプログラムが実在する**（制御フロー式を貫通する `break`・#34／`block_return` の実行時検査・#35）。
**`force_gate` 0 件は「128 例題で 0」であって「言語全体で 0」ではない**。詳細は実装ログ。

### 設計上の教訓（**再利用する知識**。各タスクの経緯は [IMPLEMENTATION_LOG.md](IMPLEMENTATION_LOG.md)）

**計測と判断**
- **推測せず先に計測する**。本系列で見積もりは何度も外れた（一覧は実装ログ末尾の表）。
  直近では **#27-d の「クロージャ＝フレーム表現の変更が必須」が言い過ぎ**（半分は可変キャプチャ無し）、
  **#32 の「async 本体は 17 文だけ＝些細」が誤り**（本体のループは全反復ツリーウォークで 3.77x 遅い）、
  **#3 の「TLS 4 本を消す」の前提が古い**（2 つは VM が使用中）だった。
- **⚠⚠ この規模の変更は VM 支配ベンチを ±5% 揺らす**（#28 は却下。稀な op 7 個を 1 アームに畳んでも
  **何も回復せず** 1 件は悪化 ⇒ 効くのは**アーム数ではなくコード配置**）。**数 % で良し悪しを決めない**。
  判断材料は「**`--vm=off` でも同じ差が出るか**」と「**変更と同規模**のプローブとの比較」の 2 つだけ。
- **`force_gate` は例題ごとに最初の 1 件で止まる**（1 例題 = 1 原因ではない）。潰すたび測り直す。
  文言だけで bail と `vm_ineligible` は区別できないので `AR_TW_STATS` の両表を突き合わせる。
- **`compare_vm_modes.ps1` は「両モードともツリーウォークに落ちる形」を検知できない**。
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
- **コード索引を持つ op を足したら `peephole::code_target_mut` に登録する**
  （`ForIter` の exit・`SetupTry` の handler・`StaticInit` の after）。忘れても**テストは通ってしまう**。
- **コンパイラの「この形のときだけ載せる」条件は理由を確かめてから外す**（2 実装の差／最適化の前提／
  本当の非対応の 3 通りがあり、外し方が違う）。
- **⚠ 「強制」を全体既定にしてはいけない**（#3）。`VmMode::On` は解決情報が揃っている前提なので、
  `Interpreter::new()` を直接使う **REPL・単体テスト・組み込み**では正しいコードでも落ちる。
  ⇒ **`Default` は `Off`、`On` にするのは `run_program` だけ**。例題は全部 `run_program` を通るので
  **例題だけ見ていても気づけない**。
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

### 統一の到達度
**型の解決**（#16 c-3）・**ローカルの解決**（#11 R2-a/R2-a′）は三経路とも到達済み。
**グローバルの解決**（#11 R2-b）はツリーウォーク／VM が共有（ネイティブは参照が無く保留）。
### 次にやる候補
1. **#39 / #40** — `--vm=off` でしか走らない**言語機能**の残り（関数からのグローバル代入・finally 本体からの脱出）。着手可。
2. **#36** — `--vm=off` でしか走らない**入口**（REPL・単体テスト）の VM 移行。着手可（並行可）。
3. **#33** — 上 3 つが終わってはじめて成立する（`--vm=off` の削除とツリーウォーク制御フローの実削除）。
4. **#38 → #30 / #31** — 独立レーン。いつでも着手できる。

> **#34 / #35 / #37 は完了**（2026-08-17）。⚠ どれも「VM に載せる」だけでは終わらず、
> **ツリーウォーク側のバグと VM 側の意味論差**が芋づるで出た（詳細は実装ログ）:
> `continue` の貫通が無い 2 件（SyntaxError 化・**黙って握り潰し**）／**`try` のハンドラが残る**
> （`has_escape` は**文しか歩かない**）／**`block:` 文が `loop_yield` を吸い込む**／
> **`loop_yield` を脱出扱いしていた**（跳ばないのに丸ごと bail）。
> ⇒ *bail する形はツリーウォークが正しいとは限らない* の再演が続いている。
> ⚠ **`--vm=on` で動かない正しいプログラムはまだ残っている**（#39 / #40）。
> **見つけ方は毎回同じ**: 形を総当たりして `--vm=off` / `--vm=on` / `impl_python` を突き合わせる。

> **⚠ 速度目的の残タスクは枯れた**（#12 2.61x・#2b 1.6x・#2a・#1-x・#10-b・#26 で取り切った）。
> ただし**カバレッジを広げると速度も付いてくる**ことがある（#32 の async 本体は 3.77x）。
> 新しく速度課題を立てるなら**まず計測して支配項を出すこと**。
> ⚠ #1-x の教訓: **`#[inline]` は効いているとは限らない**（巨大関数は LLVM が却下する）。
> **⚠ #12b / #2c は保留**（循環依存・速度理由も #12 で消えた）。
> **⚠ #19 / #17-a / #17-b（外部接続系）は優先度低**（本系列と独立・別レーンで扱う）。

### 作業の進め方（この系列で有効だった型）
- **推測せず先に計測する**。見積もりは本系列で何度も外れた（一覧は実装ログ末尾の表）。
  診断フックを足して数字を見てから設計を決めること。
- **検証は 4 点セット**: `cargo build`（**警告 0**）・`cargo test`（**706 緑**）・
  [compare_vm_modes.ps1](compare_vm_modes.ps1)（off/on byte-identical）・
  [scan_examples.ps1](scan_examples.ps1)（例題 **FAIL 0**）。⚠ **release バイナリを見る**。
  デバッガ／`vm_eligible` に触るなら追加で [compare_debug_modes.ps1](compare_debug_modes.ps1)、
  codegen なら [dump_native_ir.ps1](dump_native_ir.ps1) の IR byte-identical（最強の検査）。
  `cargo clippy` は既存警告 **62 件**（サマリ行除く）。総数でなく**増分 0** を確認すること。
- 大きな変更の前後で **A/B 実測**する（同一ビルドで emit のみを切り替える）。
- 速度効果が小さくても、コード・ロジックの簡素化が見込めるならメリットとして認識する。
- 全てのタスクは番号で管理する。番号付けされていないものは新タスクとして昇格を提案する。
- **高リスク低リターンと判断したらスキップして保留にし、理由を記録する**（勝手に大改造しない）。
  判定基準として効いたのは「**消費者が居るか**」（#11 R2-c・#14・#15b はこれで保留）。
  ⚠ **タスクの定義が目的に対して過剰なら定義を見直す**（#27-a は 644 行→50 行／#27-c の
  `try/except/finally` は入れ子で足りた／**#27-d の「フレーム表現の変更が必須」は可変キャプチャだけ**）。

### 検証・計測スクリプト（リポジトリ直下）
| スクリプト | 用途 |
|---|---|
| [scan_examples.ps1](scan_examples.ps1) | 全例題をタイムアウト付きで実行し、失敗のみ理由付きで列挙 |
| [compare_vm_modes.ps1](compare_vm_modes.ps1) | `--vm=off` / `--vm=on`（`auto` は別名）の **stdout + stderr** byte-identical 検証（ヒープアドレスは正規化）。`_error` 例題も対象（#20）。退避は `-SkipErrorExamples` |
| [dump_native_ir.ps1](dump_native_ir.ps1) | 代表 6 モジュールの生成 LLVM IR を保存（`.arc`/`.ars` は退避・復元） |
| [annot_diff.ps1](annot_diff.ps1) / [annot_unresolved.ps1](annot_unresolved.ps1) | 注釈の充填状況・binop 特化の内訳・`Unresolved` の発生源／その全例題集計（式種別ごと） |
| [ab_bench.ps1](ab_bench.ps1) | 2 つの `arrow.exe` を**交互実行**して経過時間を比較（`-A head.exe -B new.exe`）。#2b で新設 |
| [compare_debug_modes.ps1](compare_debug_modes.ps1) | **対話デバッガのステッピング**が off/on で byte-identical か検証（`examples/debugger/<name>.ar` ＋ `<name>.in`）。#1 で新設 — `compare_vm_modes.ps1` は stdin を与えないのでこの経路を覆えない |
| [ab_bench_vm.ps1](ab_bench_vm.ps1) | `ab_bench.ps1` の **`--vm=<mode>` 付き**版。「退行が VM 経路由来か」を切り分ける唯一の手段（#10-b で新設）。⚠ 交互実行必須 |
| [force_gate.ps1](force_gate.ps1) | **強制バイトコードの回帰検知**（#25。#3 完了後は「既定の挙動が全例題で通るか」の検査）。全例題を `--vm=force`（=`on`）で実行し `VmForceError` を列挙。⚠ **止めて判定する**用途で件数は `tw_stats.ps1` で見る。GUI 例題は**タイムアウト後に窓を閉じて**完走させる（#29） |
| [tw_stats.ps1](tw_stats.ps1) / [tw_stats_files.ps1](tw_stats_files.ps1) | **ツリーウォークが実際に実行している文**を全例題で集計（`AR_TW_STATS`）／その例題別内訳。feature 付きビルドを自動で行う |
| [run_examples.ps1](run_examples.ps1) / [bench.ps1](bench.ps1) | 素朴な例題ランナー（タイムアウトなし）／ベンチ一式 |

### 診断フック（環境変数）
| 変数 | 効果 |
|---|---|
| `AR_DUMP_LL=<path>` | `--compile` 時に生成 LLVM IR を保存 |
| `AR_ANNOT_DIFF=1` | 注釈の充填状況・binop 特化の内訳・`Unresolved` の発生源・slot 索引読みの件数・`AnnotIdent`（識別子読みの型落ち内訳・#15b） |
| `AR_VM_DUMP=1` | VM の生成バイトコードを逆アセンブルして stderr へ |
| `AR_TW_STATS=1` | ツリーウォークの実行内訳（文種別×最上位/関数内）・VM コンパイル成否・bail 地点・**`tw_control_flow`（TLS/センチネルを使う経路に入った回数・#3。通常実行では 0）**。**`cargo build --features tw_stats` が要る**（既定ビルドではコードごと消える。env 判定だけにすると `exec()` 1 文ごとの atomic 読みで 11% 退行する） |

### 落とし穴（既知）
- **PowerShell 5.1 は BOM 無し `.ps1` を ANSI として読む**（日本語コメント入りは UTF-8 **BOM 付き**必須）。
  **Rust ソースの一括書き換えは Python で `encoding='utf-8'` を明示**（`Set-Content` は文字化けする）。
  ⚠ **そのスクリプトから日本語を `print` すると cp932 の `UnicodeEncodeError` で書き込み前に落ち、
  変更が丸ごと消える**。進捗表示は ASCII か件数だけにすること。
- **native exe の stderr を `2>&1` で受けると PS5.1 が ErrorRecord 化**して exit 0 でも失敗扱いになる。`Start-Process -PassThru` の `ExitCode` も当てにならない（`System.Diagnostics.Process` を直接使う）。
- **⚠ `ReadToEnd()` を stdout→stderr の順に逐次呼ぶと子とデッドロックする**（#34 で 1 時間停止・#38）。
  子が stderr のパイプを埋めると書き込みでブロックし、親は stdout を待ち続ける。
  **必ず `ReadToEndAsync()` で同時に読む**（[scan_examples.ps1](scan_examples.ps1) が手本）。
  症状は「**CPU 時間が伸びないまま生き続ける**」。⚠ 現に [ab_bench.ps1](ab_bench.ps1) /
  [ab_bench_vm.ps1](ab_bench_vm.ps1) がこの形（#38 で直すまで A/B は別手段で取ること）。
- **A/B は当該変更だけを切り替えて取ること**（#21-b）。前回測定から時間が空いた値と比べると他の変更やマシン変動を誤って帰属する（`bench_field_access` の退行を 21-b のせいと誤認した実例あり）。
- **HEAD のバイナリが要る A/B・IR 比較では `git stash push -- src/` で退避**してビルドする。
  実行前に必ず `git status` を確認し、`src/` をスクラッチパッドへコピーしておく（過去に未コミット変更の破棄事故あり）。
- **診断フックを実行経路に足すときは feature で消せる形にする**（#10-a）。`exec()` は文ごとに呼ばれるので `OnceLock` の atomic 読み 1 回でも **11% 退行**した（`cfg!(feature=..)` を先に見て定数 false にする）。
- **`exec_op` は `#[inline(always)]`。op のアームに重い本体を書かない**（#10-b）。その op を使わない Chunk の
  ホットループまで 4〜6% 遅くなる。逆に**全部外へ出すと**その op を使う側が 7〜10% 損する。
  → **IC はヒット経路だけインライン・ミス経路は `#[inline(never)]`**。
- **`compare_vm_modes.ps1` は「両モードともツリーウォークに落ちる形」を検知できない**（#27）。
  **bail する形はツリーウォークが正しいとは限らない**（`for-target-shadow` で実バグ。
  基準は `python -m impl_python` の出力）。
- **退行を疑ったら「そもそも触っているか」を先に見る**。**`--vm=off` でも同じ差が出る**なら VM 経路とは無関係。
- **⚠ ノイズ床のプローブは「変更と同規模」で測る**（#27）。opcode を足す規模の摂動は
  **1 命令も実行しなくても** ベンチを 0.88〜0.94x 動かす（小関数 1 本のプローブは規模不足）。
- **A/B は必ず交互実行**（A,B,A,B…）。A を N 回 → B を N 回だとサーマルドリフトで実在しない退行が見える。
- **⚠ コード索引を持つ op を足したら `peephole::code_target_mut` に登録する**（#27-d で
  `StaticInit` の飛び先を忘れ、**テストも例題も通ってしまった**＝たまたま除去対象が無かっただけ）。
- **`compare_vm_modes.ps1` / `scan_examples.ps1` は `target/release` を見る**（`cargo build` の debug だけ見て「直った」と判断しない）。**`$ErrorActionPreference='Stop'` から `cargo` を呼ぶと進捗の stderr で終了エラーになる**ので、その呼び出しの間だけ `Continue` に落とす（`tw_stats.ps1` 参照）。
- **`Rc` を含む値をスレッドへ送る経路（async）では `deep_clone` が独立バッファを作ること**（#15）。
  回帰検知は [async_string_share.ar](examples/async/async_string_share.ar)（**接触回数を減らすと検知力を失う**）。
- **⚠ worker スレッドは `Interpreter::new()` を作る**（#32）。**`--vm` も型注釈も引き継がない**ので、
  親の設定を渡し忘れると `--vm=off/force` が効かず**ゲートに穴が開く**（実際に開いていた）。
- **⚠ `force_gate` 0 件は「128 例題で 0」であって「言語全体で 0」ではない**（#34/#35 で判明）。
  **例題が無い言語機能はゲートに映らない**。単体テストだけが押さえている形が現に 11 件あり、
  そのうち 5 件は `--vm=on` が `VmForceError` で止まっていた。**新しい形を疑ったら例題を先に書く**。
- **⚠ `--vm=on` が bail する形は「エラー報告」も食い違う**（#34）。実行時エラーになると分かっている
  文（囲むループの無い `break`）を bail すると、`--vm=off` は `SyntaxError`・`--vm=on` は
  `VmForceError` になる。**必ず失敗する文は bail せず `Op::Fail` で同じ文言を出す**。
- **⚠ 脱出制御を足すときは「オペランドスタック」と「ハンドラスタック」の両方を戻す**（#34）。
  `has_escape` は**文しか歩かない**のでブロック式の中の `break` を見落とす。`try` から跳ぶと
  `PopTry` を通らず**ハンドラが残り、ループを抜けた後の無関係な例外を横取りする**。
  検知には「**跳んだ後に別の例外を投げる**」例題が要る（跳ぶだけの例題では素通りする）。

---

## 実装状況

各段の実装詳細・実測は [PHASE_R1_RESULTS.md](PHASE_R1_RESULTS.md)。off/on の byte-identical を常時維持。

**完了項目の一覧は区切り線の下へ移した**（規約 3）。経緯は [IMPLEMENTATION_LOG.md](IMPLEMENTATION_LOG.md)。

### 残り — 依存関係つき一覧
依存の凡例: 「前提」が空欄＝**今すぐ着手できる**。`←X` は X が終わるまで着手できない。

| # | タスク | 手法 | 前提（依存） | 状態 |
|---|---|---|---|---|
| 23 | 評価済み引数を名前付き struct 化 | 3 つ組 `(Option<String>, Value, bool)` に `source_name` を足す（27 箇所）。**`NativeFunction` を C 軸へ寄せる前提** | — | 保留（高コスト・速度効果なし） |
| 12b | R0-A 明示フレームスタック | `Rc<Frame>` のスタック化 | — | **保留**（速度理由は #12 で消滅・依存は循環・A 自体に borrow コスト。詳細は実装ログ） |
| 15-3 | 文字列インターン（§7.4-3） | 属性名・メソッド名を `Rc<str>` + ポインタ比較 | — | 保留（**消費者 0 件と実測**・R3 IC が名前引きを既に潰済） |
| 2c | V-F R0-A エスケープ解析 | 非エスケープフレームのフラット確保 | **← 12b** | **保留**（12b が作るコストを取り消すだけのタスク。VM 経路は既にフラット） |
| 10-d | 定義文のオペコード化・import モジュール本体 | — | — | **保留**（計測で**両半分とも #3 に寄与しない**と判明: モジュール本体は 20 文・定義文は制御フローも TLS も持たない。詳細は実装ログ） |
| 27 | fn の VM コンパイル失敗の解消 | 未解決 Ident=`LoadGlobal`／属性複合代入=融合を使わない 2 回評価経路／for シャドウ=本体の間だけ slot 差し替え／可変長=`local::args` を末尾 slot へ／`static`=`static_cells` 直読み／可変キャプチャ=セル表 | — | **完了**（`vm_bail_fn` 49→**0**） |
| 27-d | クロージャ本体の VM 化 | 段階 1（不変キャプチャ→末尾 slot・`captured_slots`）／段階 2a（`static`→`static_cells` 直読み）／段階 2b（可変キャプチャ→**slot と並行するセル表**・`LoadCell`/`StoreCell`）。⚠ クロージャ実体ごとに `FnValue` が別物で **Chunk を使い回せない**（→ #30） | — | **完了**（`vm_ineligible` 20→**0**） |
| 28 | `Op::Rare` への畳み込み | 稀な 7 op を 1 アームに集約 | — | **却下**（実装して A/B した結果**何も回復せず** 1 件は悪化。**前提だった「op 1 個あたり ~1〜1.5%」というモデル自体が誤り**だった。詳細は実装ログ） |
| 27-c | 最上位 Chunk 化の bail 解消 | flat リスト組み込みの 1 実装化／`StoreLocalFromIdent`／`try/except/finally` は `try/except` を `try/finally` で包む／ブロック式内 `fn` の採番／一般の呼び先式／`CallBuiltinKw`・`CallMethodKw` | — | **完了**（`vm_bail_toplevel` 175→**0**・`force_gate` 36→**4**） |
| 24 | peephole パターンの追加 | `peephole.rs` へ足す（到達不能コード除去・`Const;Pop` 消去等）。**#2a の実測では JUMP 除去だけで総命令の 0.31%**なので、追加は「効く形を実測してから」 | — | 未着手（低優先・効果は要実測） |
| 29 | `force_gate` の未判定を無くす | タイムアウトでも stderr を読む／`-Timeout` 45 秒／**kill の前に窓を閉じる**（繰り返し送る） | — | **完了**（未判定 5→**0**・128 例題すべて完走） |
| **38** | **A/B 計測スクリプトのデッドロック解消** | [ab_bench.ps1](ab_bench.ps1) / [ab_bench_vm.ps1](ab_bench_vm.ps1) の `Measure-Run` が `StandardOutput.ReadToEnd()` → `StandardError.ReadToEnd()` を**逐次**呼ぶので、子が stderr のパイプを埋めると相互ブロックする（#34 で **1 時間停止**）。[scan_examples.ps1](scan_examples.ps1) と同じ**非同期読み**（`ReadToEndAsync`）に揃える。⚠ 症状は「CPU 時間が伸びないまま生き続ける」 | — | 未着手（**計測手段の不具合**・#30 の前に潰す） |
| **30** | **クロージャ Chunk の実体跨ぎ再利用＋計測** | `get_or_compile_chunk` は `FnValue` ごとにキャッシュするので、**クロージャは実体ごとに再コンパイル**する。#27-d 段階 1 で初めて実際に走るようになった（それまでクロージャは VM 非対象）。⚠ **クロージャのベンチが 1 本も無い**ので、まず計測手段を作ってから判断する | **← 38**（A/B が止まるので先に直す） | 未着手（**新たに生じた性能リスク**） |
| **31** | **`--vm=off` × `impl_python` の差分検査** | `compare_vm_modes` は**両モードともツリーウォークに落ちる形**を構造的に検知できない（#27 の `for-target-shadow` を取り逃していた）。参照実装との突き合わせスクリプトを作る。⚠ `impl_python` の対応範囲が狭ければ対象例題を絞る。**実現可能性は実測済み**（72 例題中 stdout 一致 **39**・不一致 30・内部クラッシュ 3 ⇒ **39 本に絞れば成立する**・詳細は実装ログ） | — | 未着手（**実バグ 1 件を取り逃した穴**・#33 の代替網） |
| 32 | async ブロック本体の VM 化 | `compile_async_body`（`compile_block_expr` ＋ `Return`）で Chunk 化。捕捉環境は `captured_slots`。`vm_mode` を worker へ伝搬し `Force` を効かせた | — | **完了**（**3.77x**・最上位ツリーウォークが定義文だけになった） |
| 3 | 強制バイトコード（D2） | `VmMode` を `Off`/`On` へ畳み、`On` は載らなければ `VmForceError` で停止。⚠ **`On` にするのは `run_program` だけ**（REPL/テストは解決情報を持たないので壊れる） | — | **完了**（フォールバック撤去。**TLS/センチネル削除は #33 へ分離**） |
| **34** | **制御フロー式を貫通する `break`/`continue` の VM コンパイル** | 跳ぶ前に「途中の式が積んだオペランド」を `Pop` する（深さ＝`stmt_base`）。深さは `BinOp` 左右・`UnaryOp` にだけ伝播し、**不明なら bail**（既定 `None` が安全側）。判定を `Stmt::Break` へ一本化し `block_body_bails` の 2 つ目の walker を撤去。囲むループが無い場合は bail せず `Op::Fail`（ツリーウォークと同文言） | — | **完了**（**新オペコード 1・既存 Chunk 不変**。ツリーウォークの `continue` バグ 2 件も修正） |
| **35** | **`block_return`/`loop_yield` の実行時検査を VM へ** | `BlockCtx` に `->T` を持たせ `Op::CheckBlockReturn`/`CheckLoopYield` を発行（判定は `check_block_return_type`/`check_loop_yield_type` の**1 実装へ委譲**）。for/while 式の外の `loop_yield` は bail せず `Op::Fail`。⚠ `block:` **文**は TLS へ push しないので**外側の注釈を継承**し、**`loop_yield` には透過**（蓄積先を持たせない） | — | **完了**（新オペコード 2。`block:` 文の yield 透過バグも修正） |
| **39** | **関数本体からのグローバル代入の VM コンパイル** | `mut g = 0` / `fn bump(): g += 1` が `VmForceError`（`--vm=off` と `impl_python` は動く）＝**既定 `--vm=on` で動かない正しいプログラム**。⚠ **HEAD からの既存ギャップ**で #34/#35 とは独立（#35 の計測中に発見）。`StoreGlobal` は最上位モード限定なので、関数本体でも書き込み先を `scopes[0]` と断定できる条件を用意する必要がある | — | 未着手（#34/#35 と同じクラス・#33 の前提） |
| **37** | **`try/finally` を跨ぐ `break`/`continue`/`return`/`block_return` の VM コンパイル** | `try_depth`/`finally_guard` を **`try_stack: Vec<Option<Vec<Stmt>>>`**（各 try の finally 本体）へ置換し、`emit_unwind_tries(keep, pop_except)` が**脱出経路へ finally 本体を複製**する（内側から）。バリアは `LoopCtx.try_len`／`BlockCtx.try_len`／`return` は 0。⚠ **`loop_yield` は跳ばないので `has_escape` から外した**（誤検知で丸ごと bail していた） | — | **完了**（新オペコード 0。`try/except` を跨ぐ脱出の bail も解消） |
| **40** | **`finally` 本体そのものからの脱出の VM コンパイル** | `finally:` の**中**の `break`/`return` が bail（`--vm=off` は動く）＝**既定 `--vm=on` で動かない正しいプログラム**。⚠ finally は正常路・例外路・各脱出路に複製されており**コピーごとにスタックの形が違う**（例外路は `[exc]`、return 路は戻り値が載る）。#37 は `in_finally` で「通さない」ことを明示しただけ | — | 未着手（#33 の前提・稀な構文） |
| **36** | **`Interpreter::new()` 消費者の VM 経路移行** | `VmMode::Default` が `Off` なので **REPL（[repl.rs:30](src/repl.rs#L30)）と単体テスト 14 箇所**が `--vm=off` と同じ経路を踏む。入口で `check_and_annotate` ＋ `resolve_program` ＋ `set_toplevel_globals` を供給して `On` にする。実測: テストヘルパー 3 本を `On` にすると **676 passed / 30 failed**（うち 19 は注釈欠落＝この配線で解消・11 は #34/#35 の実体） | — | 未着手 |
| **33** | **`--vm=off` の削除とツリーウォーク制御フローの実削除** | ⚠ 前提は **34（済）/35/36/37**。`--vm` フラグ・`VmMode::Off`（8 箇所）・`BLOCK_YIELDS`/`LOOP_DEPTH`/`BLOCK_RETURN_EXPECTED_TYPE`/`BREAK_SENTINEL`/`CONTINUE_SENTINEL`（#34 で追加）と制御フロー実装を削除（**実測 ≈700 行**）。`GENERATOR_YIELDS`（#8）と `RAISE_SENTINEL`（V-C）は **VM が使うので対象外**。⚠ **[compare_vm_modes.ps1](compare_vm_modes.ps1)（実バグ 4 件を検出した唯一の差分網）と `ab_bench_vm.ps1` と「`--vm=off` でも同じ差が出るか」という切り分け基準を同時に失う**ので、先に #31 で代替網を用意すること | **← 34, 35, 36**（＋代替網として **31** を推奨） | **未着手（前提が未完）** |
| 11 R2-c | グローバル記憶域の index 配列化 | ネイティブの index 参照 | **← 消費者の出現（14 と同時に再評価）** | ブロック中（消費者不在） |
| 14 | §6 モジュール動的リンク | ディスクリプタシンボル＋ABI ハッシュ照合 | **← モジュール間ネイティブ直リンクの導入**（未実装・未計画） | ブロック中 |

```
27/27-c/27-d/32/29/3 は完了（`force_gate` 0・128 例題完走・フォールバック撤去済み）

--vm=off 廃止レーン（34/35/37 は完了。残り 3 本 → 最後に 33）
    34（break/continue 貫通）／35（block_return 検査）／37（finally 跨ぎ）……完了
    36（REPL・単体テストの VM 移行）      ┐
    39（関数からのグローバル代入の VM 化）├→ 33（--vm=off とツリーウォーク制御フローの削除）
    40（finally 本体からの脱出の VM 化）  ┘   ↑ 31 を先に済ませて代替網を作るのが望ましい

38（A/B スクリプトのデッドロック解消）→ 30（クロージャ Chunk 再利用の計測）
31（参照実装との差分検査）は独立していつでも可
モジュール間ネイティブ直リンク（未計画）→ 14 → 11 R2-c ／ 12b → 2c は循環依存で両方保留
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
| D2 | **解釈実行は強制バイトコード**（**#3 で達成**。`--vm=off` は検証用の出口として残置） | §3.2 |
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
（VM 経路では達成済み。**実削除は #33 へ分離** — 4 つは `--vm=off` のためだけに生きており、
`GENERATOR_YIELDS`/`RAISE_SENTINEL` は VM が使うので対象外。詳細は #33 と実装ログ。）

### 5.4 段階（V-A 〜 V-F, 各段で テスト緑 + 穴の可視化 + ベンチ再測定）
> ⚠ 当初は各段の穴を `--vm=force` で可視化する想定だったが、長らく **`Force` は `Auto` と同一の
> no-op** だった。#25 でゲート化済み（[force_gate.ps1](force_gate.ps1)）。件数の可視化は `AR_TW_STATS`。
- **V-A〜V-D** 【✅】: 骨格（op/chunk/run/disasm）→ 算術・slot・制御フロー・呼び出し → クラス/メソッド/属性（R3 の IC）
  → 例外・match・ブロック式（TLS/センチネルを**VM 経路で不使用化**・実削除は #33）→ for・組み込み・Chunk キャッシュ。
  ※クロージャは残。最上位/import は #10-b で一部、残りは #10-c/#10-d。
- **V-E** 【✅ #1 で完了】: デバッガ統合（トレースバック・デバッグ名テーブル・REPL バイトコード実行）＋
  **文境界行テーブル（`stmt_spans`）と VM 内ステップ実行**。ツリーウォークへのデバッグ用フォールバックは撤去済み。
- **V-F** 【✅ #2a/#2b】: 最適化（peephole=`vm/peephole.rs`・superinstruction・単型算術命令）。
  ※ **R0-A エスケープ解析（#2c）は保留**（#12b が作るコストを取り消すだけのタスクで、VM 経路は既にフラット）。
- **完了時** 【✅ #3】: デュアルモードのフォールバックを撤去し**強制バイトコード**へ（D2）。
  `VmMode` は `Off`/`On` の 2 値。⚠ **`On` にするのは `run_program` だけ**（REPL・単体テスト・
  組み込みは解決情報を持たないので落ちる）。**TLS/センチネルの実削除は #33**（要判断）。

---

## 10. 検証コマンド / 規約

```
cargo test                          # 672 passed を維持（各ステップ/各段ごと）
cargo build                         # 警告0 を維持
cargo clippy --all-targets          # exit 0
./run_examples.ps1                  # 例題スイートの回帰確認
./bench.ps1                         # Phase R の各ステップ / Phase V の各段で再測定（フェーズ0基準 = bench_baseline.md）
cargo run -- --compile examples/interop/test_modules/physics.ar  # Phase R: native 経路の数値一致確認
cargo run -- --vm=force <file.ar>   # フォールバック禁止。載らなければ VmForceError で停止（#25）
./force_gate.ps1                    # 全例題で上記を回す「#3 へ進めるか」のゲート
./tw_stats.ps1                      # 未対応箇所の件数・内訳（要 --features tw_stats）
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
- [.claude/skills/c-abi-interop](.claude/skills/c-abi-interop/SKILL.md) — オフセットアクセス記憶域の設計仕様／
  `codebase-map` スキル — `src/` のディレクトリ別役割 + ファイル別行数。

═══════════════════════════════════════════════════════════════════════════════
## 以降は「参照・根拠」（決定済み・履歴）— 実装再開には**読まなくてよい**
§3 アーキテクチャ決定 / §1 背景実測 / §7 速度投影 / §8 非目標 / §9 未決事項。
番号は初版のまま（本文中の相互参照 §3.4 等を保つため物理位置のみ末尾へ移動）。
═══════════════════════════════════════════════════════════════════════════════

## 完了項目の一覧（履歴）

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
| 1-a | デバッガの自動検証を新設 | `compare_debug_modes.ps1` ＋ `examples/debugger/<name>.{ar,in}` の **5 シナリオ**（off/auto のステッピング transcript 比較）。**負の対照で検知力を確認** |
| 1-b | デバッグ中の VM 無効化を最小化 | `should_pause_at` を読み「**停止し得るのは StepInto だけ**」と確定し `dbg_blocks_vm()` へ置換。step-over が跨ぐ重い呼び出しで **1.97x**・通常経路のコストはゼロ |
| 1 | V-E 本体（VM 内の文単位ブレーク） | 文境界行テーブル `stmt_spans` ＋停止判定つき専用ループ `run_stepping`（**通常ループには何も足さない**）／停止フレームのローカルを `local_names` から一時スコープへ。**既存バグ 1 件を修正**・ツリーウォークへのデバッグ用フォールバックを撤去 |
| 10-a | 最上位ツリーウォークの実測 | 診断フック `AR_TW_STATS`（feature `tw_stats`）＋ [tw_stats.ps1](tw_stats.ps1)。文種別×最上位/関数内、VM コンパイル成否、bail 地点（未帰属 catch-all つき）。**#10 の保留理由 2 点と「#3 の前提は #10 のみ」を実測で否定** |
| 27 の一部 | クロージャ: **外側関数**を VM 化 | `Op::MakeFn` ＋ `Chunk.fn_defs`。`nested_fn_captures` が「自由変数 ∩ 外側 slot」を求め**全て不変のときだけ**載せる（値を複製して `CapturedVar::Immutable`）。オーバーロード合成は `merge_fn_overload` に集約。`decl-prepass:FnDef` 8→**0**・`fn_FAILED` 17→**11**・`in_fn` 127→**111** |
| 27-c の一部 | 最上位 bail **175 → 126**（制御フロー文の bail 18→**10**） | リゾルバ 4 件（`enum`/`new_type` を globals へ・`AsyncAssign` の `target` は束縛でない・虚数リテラル・**VM へ渡す集合をシャドウ減算なしに**）＋ `block:` 文＋`for k, v in ...`（`Op::UnpackTuple` 1 個）＋ `collect_nested_decls` の `Stmt::Block` 漏れ。**計測の穴 3 件**も修正（意図的スキップ 336 件が失敗に化けていた／未帰属 46→3／bail を `<文種別>/<理由>` で切れるように） |
| 10-c2 | 最上位の残り文を Chunk 化 | 受理判定を**許可リスト→定義文の除外リスト**へ反転（式文・`if`/`try`/`match`・代入・属性代入…）。`exec()` の VM 試行を 5 アーム→**入口 1 箇所**へ集約。最上位 Chunk 499→**1,368**・ツリーウォーク **663**（うち 56% は定義文）。**計測フックの過大計上を修正** |
| 27-b | メソッド dispatcher の 1 本化 | `eval_method_call` を「引数評価＋委譲」だけにし 16 レシーバを評価済み版へ集約。`Instance` アーム 144 行と旧 evaled 版 85 行を委譲に畳む。`Op::CallMethod*` に `node_id` を足して FFI 戻り値検査を VM へ。`fn_FAILED` 29→**17**・`toplevel_FAILED` 163→**65**・実質 -230 行 |
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

1. **解決注釈の持ち方**: AST 埋め込み（`Cell`/付随フィールド, `SlotCache` 流儀）か 別テーブル（node-id キー）か。
   前者はノード局所で速いが AST 型が肥大、後者は AST を汚さないが間接引き。→ **前者推奨**（既存流儀）。
2. **R3 の型チェッカ連携**: 呼び出し点でオブジェクトの具象クラスを渡せる API が `type_check/` にあるか。着手時に推論結果の受け渡し方を確認。
3. **R0-A フレームの内部表現**: `Rc<RefCell<Vec<Value>>>` か `Rc<Frame>`（`Frame` に inline 配列 + 借用管理）か。RefCell borrow の
   パニック表面とコストを見て決定。まず素直な `Rc<RefCell<...>>` で正しさ優先、V-F で最適化。
4. **ブロック跨ぎ同名の slot 再利用**（B）: ブロックを抜けた後の同名変数の slot 寿命解析の正確な規則。Arrow のブロックスコープ意味論を
   リゾルバ構築時に確認（可視名再宣言禁止＝シャドウなしは追い風）。※現状は「既出名はスキップ＝slot 再利用」で実装済み。
5. **循環 import のリンク**（§6）: A⇄B の相互参照は「全シンボル宣言（index 採番）→本体解決」の2フェーズで解く。
