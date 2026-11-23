# ゲートの実行時間を縮める（ゲート専用のテストを `gate/` に作り、実行を 1 回にまとめる）

言語を変えるたびに走らせるゲート（[scripts/](../scripts/)）は、**同じ exe で同じ例題を同じ引数で何度も実行している**。
「毎回」の 3 本（`scan_examples` / `force_gate` / `compare_python_impl`）だけで同じ実行が 3 回あり、A/B 系・wasm の
突き合わせにも同じ形の重複がある。

本書は、**ゲートが実行するテストを利用者向けの例題と分けて専用のフォルダ `gate/` に独立に作り**、それを **1 回ずつ実行して
結果を保存する共通ランナー**で走らせ、各ゲートを「保存した結果を読んで判定するだけ」にする計画書である。あわせて、
禁止の形のテストは**期待した誤りで止まること**まで機械で照らす（今の例題の `_error` は、期待をコメントに書いているが
どのゲートも照らしていない）。

- 起票: 2026-11-15（利用者の依頼: ゲートの実行が長く作業を止めている。1 つのプログラムで複数の検証ができないか）
- 調査: Arrow `61beb9d9`（0.6.0・release ビルド・Ryzen 5 5600X 6 コア 12 スレッド / Windows 11）
- 状態: **全フェーズ完了**（2026-11-24・フェーズ 1〜8）。「毎回」のゲートは 573 秒 → 約 30 秒（5 節）。残した事項:
  wasm の診断を `gate/` の期待と種類まで照らすこと（7-3・拡張の解析に種類が無い）、2 つの exe が別々の cpp-lib の
  キャッシュを持つ場合の入れ替えを走らせて確かめること（6-4・この環境では MSVC のパスが合わず作り直せない）
- 改訂:
  - 2026-11-15 利用者の決定を反映した。`impl_python` との突き合わせは凍結（D-9・U-1）。ゲートが実行するコードは
    `gate/` に専用に書き下ろす。例題のコードを流用してつなぎ合わせてよい。GUI は入れず、ベンチと対話は入れる
    （D-10・U-2）。cpp-lib の中継 DLL のキャッシュは今の仕組みのまま（D-11・U-3）。これに合わせてフェーズを組み直した。
  - 2026-11-22 利用者の決定を反映した。`examples/` は利用者向けの見本で、テストは `gate/` に独立に作る（D-13・U-4）。
    禁止の形のテストを作り、正しく誤りになることを照らす（D-14・U-5 の誤りの側）。照らすために arrow.exe に機械向けの
    診断の出力を足すタスク（3-1）と、誤りの種類の網（5-2）を足した。正常系の示し方を U-5 に、切り替えた後の
    `examples/` の扱いを U-6 に残した。
  - 2026-11-22 利用者の決定を反映した。正常系はテストのコードの中で結果を確かめ、違えば確かめに落ちたと分かる誤りで
    止める。正しく誤りになる形は捕まえて正常に終わる（D-15・U-5）。切り替えた後も `examples/` を確かめる（D-16・U-6）。
    リリースの手順で `examples/` を確かめるタスク（5-4）を足し、5-4〜5-7 を 5-5〜5-8 に送った。
- 関連: [CLAUDE.md](../CLAUDE.md) のゲートの表、skill `vm-pitfalls`（§4 子プロセスの罠）、skill `language-dev-principles`
  （どの変更でどのゲートを走らせるか）、[BYTECODE_VM_PLAN.md](../implementation_logs/BYTECODE_VM_PLAN.md) のスクリプトの表
- 範囲外: ゲートの実行中に窓が出る件（利用者の指示で実行時間の後に扱う。分かっていることだけ 6 節に書く）

## 0. 何が起きているか

### 0.1 計測（release exe・同じセッション）

| 対象 | 実測 |
|---|---:|
| `scan_examples` と同じ 434 例題を逐次実行 | 156 秒 |
| 　うち `bench/bench_ab_native.ar`（毎回 45 秒のタイムアウト） | 45 秒 |
| 　うち `bench`（23 件） | 82 秒 |
| 　うち `interop`（188 件） | 58 秒 |
| 　うち残り 6 カテゴリ（223 件） | 16 秒 |
| 同じ 434 例題を 6 並列 | 46 秒（タイムアウトの 1 件が下限） |
| 同じ 434 例題を `AR_CHECK_ONLY=1`（静的検査まで） | 27 秒 |
| `arrow.exe` の起動 1 回（小さい例題） | 約 24 ms |
| `python -m impl_python` の起動 1 回（小さい例題） | 約 170 ms |
| wasm の `dump_diags.js` 1 ファイル（毎回 `Code.exe` を起動） | 約 102 ms（831 ファイルで約 85 秒） |
| `crates/arrow-frontend` の `cargo test`（ビルド済み） | 6 秒 |
| `vscode-extension` の tsc | 3.5 秒 |

- 例題の中央値は 59 ms。1 秒を超えるのは 35 件、5 秒を超えるのは 2 件。**時間は少数の重い例題と、回数の多さで決まる。**
- ⚠ `force_gate` / `compare_python_impl` / `compare_wasm_frontend` の**全体の時間は測っていない**（GUI の例題が窓を出す
  ため）。下の重複はスクリプトの中身から読んだもの。全体の時間はタスク 1-1 で測る。
- ⚠ 調査したセッションでは `cargo test --no-run` が pyo3 のビルドスクリプトで失敗した（存在しない
  `C:\Users\…\Python312\python.exe` を見に行く。実際の python は `H:\` 側）。`test_build_gate` の時間は測れていない。
  利用者の環境で通っているなら調査側のシェルだけの問題。

### 0.2 重複

| # | 重複 | 根拠 |
|---|---|---|
| ① | `scan_examples` の実行は `force_gate` に丸ごと含まれる | `force_gate` は `--vm=force` を渡していない（#33 以降、既定の実行が force と同じ）。違いは作業ディレクトリ（`scan` はリポジトリ直下・`force` は例題のディレクトリ）と対象（`force` は `_error`・GUI・`practical_examples` も実行する） |
| ② | `compare_python_impl` の Rust 側は同じ実行の 3 回目 | 同じ exe・同じ引数・作業ディレクトリは `scan` と同じ。取っているのは stdout だけ |
| ③ | `impl_python` 側は毎回同じ結果 | `impl_python/` は更新を止めている。例題と `impl_python/` が変わらなければ出力は変わらない |
| ④ | `compare_outputs` と `compare_bytecode` は例題ごとに A・B を別々に起動する（計 4 回） | `AR_VM_DUMP=1` のダンプは stderr に出る。1 回の実行で stdout / exit / stderr / ダンプが全部取れる |
| ⑤ | A/B 系の基準側（`-A`）は毎回同じ結果 | 基準の exe は 1 つのタスクの間は変わらない |
| ⑥ | `compare_wasm_frontend` は 1 ファイルごとに `Code.exe` を起動して wasm を読み直す | [dump_diags.js](../crates/arrow-frontend/dump_diags.js) は 1 ファイルを解析して終わる |
| ⑦ | `compare_wasm_frontend` の exe 側・`annot_unresolved` は、どちらも全例題を舐める | 環境変数が違うだけ（`AR_CHECK_ONLY` / `AR_ANNOT_DIFF`）。⚠ 起票のときは `type_obligations` もここに入れていたが、あちらは例題ではなく自前の検体を実行する（1-4） |

### 0.3 そのほかに見えたもの

- **`bench/bench_ab_native.ar` が毎回タイムアウトする。** `scan_examples` で 45 秒、`force_gate` で 70 秒（窓を閉じる猶予
  25 秒を含む）、`compare_bytecode` で 90 秒（A・B）。→ 1-2 で原因を特定して直した（205 秒 → 2.1 秒）。
- **そのまま並列にすると壊れる。** 6 並列では失敗が 2 件から 13 件に増えた。新しく落ちたのは `cpp_*` 8 件・`rs_*` 2 件・
  `js_proc_values` の 11 件。→ 2-1 で原因（一時ディレクトリの固定の名前の取り合い）を特定して直した。
- 逐次でも `interop/event_external_handler.ar` が exit 1 で終わる（`scan_examples` が今も `FAIL` と出しているはず）。
  本計画とは別の問題。
- **禁止の形（`_error`）が正しく誤りになるかは、どのゲートも見ていない。** 例題の `_error`（9 カテゴリで約 290 件）は
  期待する誤りをコメントに書いている（「期待されるエラー」「Expected error」「⛔」）。しかし `scan_examples` は `_error` を
  対象から外し、`force_gate` は `_error` も実行するが `VmForceError` の有無しか見ない。⇒ **違う理由で落ちても、
  誤りにならなくなっても（exit 0）気づかない。**
- **誤りの出力は段階ごとに形が違い、種類が出ない・位置が無いものがある**（例題の誤りの例題で確かめた）。

  | 段階 | stderr の形 | 位置 | 種類 |
  |---|---|---|---|
  | 静的（型検査） | 表（File / Line:Col / Error Type / Message）。**全部の誤りを 1 回で出す**（`block_scope_error` で 6 件）。文面は端末の幅（端末が無いときは 120 桁）で折り返す | あり | 欄は常に `StaticTypeError`。`TypeErrorKind`（約 85 種）は出ない |
  | 構文 | `ParseError: <文> at <file>:<line>:<col>`。最初の 1 件で止まる | あり | 無し |
  | 展開 | `MetaError: <文>`（`template_recursion_error`）。`while expanding … at …` が付くことがある | 無いものがある | `MetaError` / `TemplateError` |
  | 実行時（捕まえない例外） | `<file>:<line>:<col>: TypeError: <文>`（`freeze_like_let_error`）／`TypeError: <文>` だけ（`exception_args_error`） | **無いものがある** | 例外のクラスの名前 |

  ⇒ 表の文面を読んで照らすのは壊れやすい（`compare_wasm_frontend` も位置だけを鍵にしている）。種類で照らすには
  機械向けの出力が要る（3-1）。位置の無い実行時の誤りは別の不具合の候補（本計画では直さない・6 節）。

## 1. 決定事項

- **D-1 今の判定は弱めない。** 今のゲートの正規化・判定の規則（exit・`VmForceError`・A/B の正規化）は引き継ぐ。
  共通ランナーは、対象を切り替える前に**同じ `examples/` で古いスクリプトと同じ判定を出すこと**を確かめる（4 節）。
  `gate/` では D-14 の照合と D-15 の確かめが加わり、検査は強くなる。対象を切り替えるときは、`gate/` が `examples/` と
  同じ範囲を覆うことを別に確かめる（D-12）。
- **D-2 既存のスクリプトは入口として残す。** `./scripts/scan_examples.ps1` などの名前・引数・出力の最後の行
  （`SCAN-DONE` / `FORCE-GATE: …`）は変えない。中身が「共通ランナーを呼んで、保存した結果を読む」になる。
  CLAUDE.md の表・skill の手順はそのまま使える。
- **D-3 実行と判定を分ける。** 共通ランナーはファイルを実行して exit / stdout / stderr / 機械向けの診断（3-1）/
  経過時間 / タイムアウトの有無を保存するだけで、合否を知らない。判定は各ゲートが保存した結果から出す。
- **D-4 保存先は `target/gate_results/`。** git の管理外で、`cargo clean` で消える。exe ごとのディレクトリに分け、
  名前は exe の内容のハッシュにする（パスや更新時刻にしない。同じパスに作り直した exe を取り違えないため）。
- **D-5 子の出力はパイプで受けずファイルへ落とす。** js-proc / cs-proc の孫プロセスがパイプを握って返らなくなる
  （`vm-pitfalls` §4・`compare_python_impl` が 7 時間止まった件）。共通ランナーは全部ファイルへ落とす。
- **D-6 保存した結果を使ったことは必ず表示する。** 何件を実行し何件を保存から読んだかを毎回出す。保存を無視して
  全部実行し直す引数（`-Fresh`）を全ゲートに付ける。「緑でないのに緑」を保存の取り違えで起こさないため。
- **D-7 並列にするのは取り合いが無いと確かめたファイルだけ。** 逐次に残す規則は理由つきで持つ（2-1）。速度を測る
  スクリプト（`ab_bench` / `speed_modes` など）は対象外（並列にすると数字が変わる）。
- **D-8 ランナーは PowerShell 5.1 のスクリプトのまま。** 既存のゲートと同じ書き方（`System.Diagnostics.Process`・
  .NET の API でファイルを読む）にそろえる。別の言語の道具を足さない（arrow.exe 側の出力の追加・3-1 は Rust）。
- **D-9 `impl_python` との突き合わせ（`compare_python_impl`）は凍結する**（利用者の決定・2026-11-15）。`impl_python/` は
  更新を止めているので、Python 実装を再開するまで「毎回」のゲートから外す。スクリプトと `$knownDiff` は消さずに残し、
  再開するときに方針を決め直す。
  - ⚠ CLAUDE.md はこのゲートを「意味論を守る唯一の網」としている。凍結の間、Rust と独立した実装との比較は無くなる。
    代わりに `gate/` の正常系が自分で結果を確かめ（D-15）、禁止の形は D-14 で照らす。
- **D-10 ゲートが実行するテストは、リポジトリ直下の `gate/` に専用に書き下ろす**（利用者の決定・2026-11-15）。
  - `examples/` のコードを流用し、つなぎ合わせて書いてよい（流用したコードも `gate/` に写して持つ・D-13）。
  - **GUI は入れない。** GUI は C++ / C# のライブラリの機能の 1 つなので、C++ / C# との受け渡しを確かめていれば足りる。
    ⇒ 窓を出すコード・除外の一覧（`spider_*` / `cs_form_app` / `math_render` など）・`force_gate` の「窓を閉じて終わらせる」
    処理は `gate/` には要らなくなる。
  - **ベンチと対話（REPL・デバッガ）は入れる。**
  - 作業ディレクトリは実行するファイルのディレクトリに統一する（もとの U-2 の論点）。
- **D-11 cpp-lib の中継 DLL のキャッシュは今の仕組み（`cpp_cache_isolation.ps1`）のまま使う**（もとの U-3）。
  - C++ のライブラリそのものは Arrow に依らない。exe に依るのは、Arrow がヘッダから**生成する**中継 DLL とシム
    （`ar_{stem}.dll` / `ar_{stem}_shim.*`）で、印（`ar_{stem}.syms`）は生成するコードのハッシュ。
  - 生成するコードが 2 つの exe で同じなら（FFI の橋渡しを触っていない変更の大半）キャッシュは同じで、入れ替えは
    起きない。違うときだけ今の仕組みが exe ごとに分ける。基準側の結果を保存しても、この動きは変えなくてよい。
  - ⚠ 今の仕組みは `examples/` の下のキャッシュを探す。`gate/` の下も探すように直す（6-2）。→ 直した（6-2・`-Under`）。
- **D-12 `gate/` へ切り替える条件は、覆う範囲が `examples/` を下回らないこと。**
  - 構文: `syntax_cov` を `examples/` と `gate/` の両方で取り、`gate/` の側に新しい穴（`UNCOVERED`・文脈の表の `.`・
    `NESTED-GAP`）が無いこと（`examples/` の側の値は 1-4）。
  - ⚠ 型義務（`type_obligations`）は `examples/` を見ない（自前の検体 `scripts/type_obligations/*.ar` 149 件を実行する・
    1-4 で確かめた）ので、切り替えの影響を受けない。今のまま「型検査を触ったとき」のゲートとして残す。
  - 禁止の形: `examples/` の誤りの例題が出す誤りの段階と種類（3-1 の出力で取る）が、`gate/` の期待（D-14）に
    全部あること。
- **D-13 `examples/` は利用者向けの見本で、ゲートのテストは `gate/` に独立に作る**（利用者の決定・2026-11-22・
  もとの U-4）。
  - 切り替えた後（フェーズ 5）、ゲートは `gate/` だけを見る。`gate/` のテストは `examples/` のファイルを import・参照
    しない（例題を見本として書き直しても、ゲートが変わらないように）。
  - 新しい文法・新しい禁止を足すときは、`examples/` の見本（今の規則）に加えて `gate/` のテスト（正常系と禁止の形）も
    書く。`regulations.md` に足す（2-2）。
- **D-14 禁止の形は「期待した誤りで止まること」まで照らす**（利用者の決定・2026-11-22）。
  - `examples/` の `_error` と同じく、禁止されていることを書いたテストを作る。照らすのは「誤りが出たか」ではなく、
    **期待した場所で期待した段階・種類の誤りが出て、ほかの誤りが出ないこと**。
  - 期待はその行の末尾のコメントに書く。書式の細部は 2-1 で決める（形の例:
    `let a: int = "s"   # expect: static <TypeErrorKind の名前>`、文面の断片を足すときは `"declared 'int'"`）。
    位置を持たない止まり方（0.3 の表）は、ファイルの先頭に書く期待で照らす。
  - 判定（3-4）:
    - 出た誤りの集まりと期待の集まりが一致すること（行・段階・種類で照らす。文面の断片を書いたときはそれも）。
      **足りない＝禁止が通ってしまった、余分＝別の理由で落ちた。どちらも不合格。**
    - 期待が 1 つでもあるファイルは exit が 0 なら不合格。期待の無いファイルは誤りが 1 つでも出たら不合格。
    - 警告（`StaticTypeWarning`）も同じ書き方で照らせるようにする（期待していない警告を不合格にするかは 2-1）。
  - 1 ファイルに入れる数:
    - **静的な誤りは 1 回の実行で全部出るので、主題ごとに 1 ファイルにまとめる**（例題では多くが 1 件 1 ファイル）。
      各々を別の関数に入れて互いに影響しないようにする。推論が `Unknown` に落ちて後ろの誤りが消える・誤りが連鎖する
      形は、集まりの一致で見つかる。
    - 構文の誤り・展開の誤り・捕まえない実行時の誤りはそこで止まるので 1 件 1 ファイル。
    - 捕まえられる実行時の誤り（例外）は、正常系のテストの中で `try` / `except` で例外のクラスと文面を確かめる
      （多数を 1 ファイルに・D-15）。捕まえない形（止まり方・exit・出力の形）は段階ごとに数件にする。
- **D-15 正常系はテストのコードの中で結果を確かめる**（利用者の決定・2026-11-22・もとの U-5）。
  - 結果が正しいかをテスト自身が確かめ、**違えば「確かめに落ちた」と分かる誤りで止める**。`gate/` に確かめ用の
    小さなモジュールを置き（名前は 2-1。例: `check_eq(実際, 期待, "説明")`）、合わなければ専用の例外（例:
    `GateCheckFailed`）を説明・期待・実際の値つきで `raise` する。判定（3-3）は exit が 0 でないファイルのうち、
    この例外で止まったもの（3-1 の種類で分かる）を「確かめ落ち」、ほかを「異常終了」と分けて表示する。
  - **正しく誤りになる形（捕まえられる例外）は、捕まえて正常に終わる。** 黙って通らないように次を守る（形は 2-1 で
    決め、2-3 で書けることを確かめる）:
    - 誤りが**出なかったら**確かめ落ちにする。そのための `raise` は `try` の外に置く（`try` の中に置くと、
      `except Exception` などで自分の `raise` を捕まえて通ってしまう）。
    - 捕まえるのは期待する例外のクラスだけにする（`except Exception` で別の誤りを捕まえて通さない）。文面の断片を
      照らすときは `str(e)` で確かめる。
  - 表示の形（`str` / `repr` / f-string の書式など）も、文字列を作って期待する文字列と比べる形でコードの中で確かめる
    （stdout を照らすファイルは作らない）。コードの中で確かめられないもの（REPL とデバッガの対話・捕まえない誤りの
    出力）だけを、今の golden（4-9）と D-14 で照らす。⚠ `print` の引数（`sep` / `end` など）は `str` に還元できないので、
    確かめ方（ファイルへ書いて読み戻すなど）を 2-1 で決める（ここは利用者の決定から私が導いた部分）。
  - **確かめ用のモジュールそのものを確かめる。** わざと合わない値を渡すテストを D-14 の期待つき（`GateCheckFailed` で
    止まること）で置く。`==` が常に真になるような壊れ方をしたとき、全部が黙って通るのを防ぐため（4 節の正の対照）。
  - ⚠ 確かめ用のモジュールが壊れると全部が落ちる（黙って通るのではなく大きく落ちるので許容できる）。
- **D-16 切り替えた後も `examples/` を確かめる**（利用者の決定・2026-11-22・もとの U-6）。
  - `examples/` はゲートの対象から外れる（D-13）が、見本が壊れたまま利用者に届かないように確かめは残す。
  - 時期は**リリースの手順**（`scripts/release.ps1`。今はどのゲートも走らせていない）で 1 回にし、「毎回」には入れない
    （5-4）。対象のフォルダを選べる `scan_examples`（3-3）を `examples/` に走らせ、正常の例題は exit 0、`_error` の
    例題は exit が 0 でないことを見る（期待との照合はしない・6 節）。落ちたらリリースを止める。
    ⚠ 時期（リリースのときだけ）と、`_error` の exit を見る点は、推奨の案に沿って私が決めた部分。
  - いつでも手で走らせられる（`./scripts/scan_examples.ps1 -Root examples` のような形。引数の名前は 3-3）。

## 2. 未決事項

- **U-1（解決・2026-11-15）** `impl_python` 側の保存のキー → 突き合わせそのものを凍結したので不要（D-9）。
- **U-2（解決・2026-11-15）** 作業ディレクトリの不一致 → `gate/` を作り、そこで 1 つに決める（D-10）。
- **U-3（解決・2026-11-15）** A/B 系の基準側を保存するときの中継 DLL のキャッシュ → 今の仕組みのまま（D-11）。
- **U-4（解決・2026-11-22）** `gate/` と `examples/` の関係 → `examples/` は利用者向けの見本、テストは独立に作る（D-13）。
- **U-5（解決・2026-11-22）** 正常系が「正しい結果」をどう示すか → テストのコードの中で確かめ、違えば確かめに落ちたと
  分かる誤りで止める。正しく誤りになる形は捕まえて正常に終わる（D-15）。今の `scan_examples` は exit が 0 かどうか
  しか見ておらず、出力の正しさを見ていたのは凍結する `compare_python_impl` と A/B 系（「変わっていない」は言えるが
  「正しい」は言えない）だけだったので、その代わりになる。
- **U-6（解決・2026-11-22）** 切り替えた後に `examples/` を確かめるか → 確かめる（D-16）。

## 3. タスク

番号はフェーズごと。前のフェーズが後のフェーズの前提（例外は各タスクに書く）。フェーズ 6・7 はフェーズ 5 の後なら
互いに独立。

### フェーズ 1: 計測と前提の確認

- **1-1（済・2026-11-23）** 今のゲートの全体の時間を測って 5 節の表に書く（`test_build_gate` / `scan_examples` / `force_gate` /
  `compare_wasm_frontend`、A/B 系は同一 exe どうしで）。GUI の例題が窓を出すので、走らせる時刻を利用者に知らせてから。
  以後の各フェーズはこの数字と比べる。
  - 方法: 利用者の指示（2026-11-23）で走らせた。`scan_examples` / `force_gate` はフェーズ 3 より前の版（`a7b8481b`）を
    git から取り出して一時的に置き、ほかは今の版（その後に変わったのは `compare_python_impl` の先頭の注記だけ）。exe は
    全部今の exe（`c788c93c…`）、A/B 系の基準はその写し。1 本ずつ逐次に走らせた。
  - 結果（秒）:

    | スクリプト | 時間 | 結果 |
    |---|---:|---|
    | `test_build_gate` | 0.4 | ok |
    | `scan_examples`（古い版） | 80.3 | `FAIL` 1 件（`event_external_handler`・この環境に `libm` のクレートが無い） |
    | `force_gate`（古い版・GUI を含む） | 175.0 | 831 完走（1 件は窓を閉じて終了）・`FORCE-GATE: 0` |
    | `compare_python_impl` | 317.5 | 243 件一致・既知の差分 449 件（`STALE` 1 件） |
    | **当時の「毎回」の合計** | **573.2** | （上の 4 本） |
    | `compare_wasm_frontend` | 196.8 | 831 件一致 |
    | `compare_outputs -A`（同じ exe） | 135.0 | 694 / 694 一致 |
    | `compare_bytecode -A`（同じ exe） | 218.1 | **713 / 714**（`interop/async_cs_calls` が同じ exe どうしで違った。行数は同じ 313 行。フェーズ 6 の負の対照で扱う） |
    | `compare_import_paths -A`（同じ exe） | 5.0 | 13 / 13 一致 |
    | 8 本の合計 | 1,128.1 | |

  - ⚠ `scan_examples` の 80 秒は、1-2 の `.arc` の改行の修正（`bench_ab_native` 205 秒 → 2 秒）と 2-1 の一時ディレクトリの
    修正が入った今の exe での値。計画を立てたときの 156 秒より短い。
- **1-2（済・2026-11-22）** `bench/bench_ab_native.ar` がタイムアウトする原因を調べる（古い `.arc`・ハング・単に長い）。
  `gate/` へ持っていくベンチに同じ問題を持ち込まないため。
  - 原因: ハングではなく、**`.arc` を使わずにソースを解釈していた**。`.arc` は LF のソースを埋め込んで追跡されているが、
    `core.autocrlf=true` の作業ツリーは `.ar` を CRLF で書き出すので、陳腐化の検査（`src/parser/imports/ar_modules.rs`）が
    毎回「古い」と判定してソースに落ちていた。解釈では `sum_fixed`（4000 × 20 万要素）だけで 199 秒かかる。
  - 直した: 改行（CRLF / LF）だけの違いは同じソースとみなす（`same_source_ignoring_newlines`・単体テスト付き）。
    `bench_ab_native` は 205 秒 → 2.1 秒、CHECKSUM は同じ。
  - ⚠ 同じ理由で、9 カテゴリの `.arc` 36 個（`interop/test_modules/native_*_module.arc` など）が使われず、**ネイティブの
    経路を確かめるはずの例題がソースの解釈で走っていた**（この作業ツリーでは）。直す前の exe との `compare_outputs` で 30 件が差分になり、30 件とも
    exit と stdout は同じ。差は stderr だけ（陳腐化の警告が消え、ネイティブ経路の `NativeMethod: …` の表示が出る。
    `native_field_read_error` はネイティブの側でトレースバックが付かず例外の行だけになる。例題の期待は「両モードで同じ
    `AttributeError`」で、それは同じ）。
  - `gate/` への教訓: ネイティブの経路を確かめるテストは、**本当にネイティブで走ったこと**も確かめる（陳腐化の警告が
    出たら不合格にする、など。2-1 で決める）。
- **1-3（済・2026-11-22）** `AR_VM_DUMP=1` を付けても exit / stdout と、stderr からダンプの行を除いた残りが変わらないかを
  全例題で確かめる（重複 ④ の前提）。同じ exe で 2 回実行して結果が同じにならない例題の一覧も取る（`async`・アドレスの
  表示・経過時間）。`gate/` のテストを書くときの注意になる。
  - 方法: 9 カテゴリの 733 件（GUI・対話の 7 件を除く）を同じ exe（`45d8a0d0`）で 4 回ずつ実行（捨て・普通・普通・
    `AR_VM_DUMP=1`）。作業ディレクトリはリポジトリ直下、正規化は `compare_outputs` と同じ（アドレス）。ダンプは
    `== chunk … ==` の行・続く命令の行・終わりの空行 1 つを除く。
  - **ダンプを付けて変わったもの: 0 件。** ⇒ 6-1 で `compare_outputs` と `compare_bytecode` の実行を 1 回にまとめてよい。
  - 2 回で結果が揃わなかったもの: 18 件。全部が経過時間を出すもの（`bench/` の 16 件・`interop/bench_ab_cdll`・
    `async/async_bench`）。それ以外の `async` 12 件は揃った。⇒ `gate/` のベンチは経過時間を出さない（README の 7 節）。
  - ⚠ 子の stderr を落としたファイルを、子が終わった後に読めないことがあった（`interop/async_cs_calls`）。cs-proc の
    孫プロセスがファイルを握ったまま残るため。**共通ランナー（3-2）はファイルを共有モード（`FileShare.ReadWrite`）で
    読む**（パイプの罠・D-5 と同じ根）。
- **1-4（済・2026-11-22）** 切り替えの基準のうち、今取れるものを取る（D-12）: `examples/` での `syntax_cov` の結果。
  禁止の形の基準は 3-1 の後に取る（3-1 の確かめに含める）。
  - `syntax_cov`（既定の `-Exclude '\\archived\\'`・`45d8a0d0`）: 797 ファイルを解析（構文の誤りで測れないもの 34）。
    `Stmt` 42 / 43（`DebugLet` が無い）、`Expr` 31 / 32（`DebugVar` が無い）。`NESTED-GAP` 10 件（`AttrAssign` / `Pass` /
    `Freeze` / `GenDef` / `MetaFnDef` / `AsyncAssign` / `BreakPoint` / `Undefined` / `TraitAccess` / `Cast`）。
    9 カテゴリだけにしても（`practical_examples` と直下を除く）787 ファイル・同じ穴。⇒ `gate/` はこれより穴を
    増やさない（`DebugLet` / `DebugVar` と `NESTED-GAP` は `gate/` で埋めてもよい）。
  - 文脈の表（`top` / `fn` / `nested_fn` / `type` / `module` / `async` の `O` / `.`）は 5-1 で `examples/` を測り直して比べる
    （`examples/` は残るので、ここに写しを持たない）。
  - `type_obligations`: 149 件中 `STATIC` 147・`NONE` 2、退行なし。⚠ `examples/` を見ないと分かったので D-12 から外した。

### フェーズ 2: `gate/` の形を決める

- **2-1（済・2026-11-22）** `gate/` の構成と規則を決めて `gate/README.md` に書く:
  - ディレクトリの分け方（言語の機能・外部言語ごと・`bench`・`repl`・`debugger`）とファイルの名前。
  - 期待の書式（D-14）: 段階と種類の書き方、文面の断片、位置を持たない止まり方の書き方、同じ行に 2 つ以上の誤りが
    出るとき、警告の扱い。
  - 正常系の確かめ方（D-15）: 確かめ用のモジュールの名前・置き場所・関数、確かめ落ちの例外の名前と文面、捕まえる形の
    書き方（誤りが出なかったときの `raise` を `try` の外に置く形）、`print` の引数の確かめ方。
  - 逐次にしか走らせられないテストの印（cpp-lib / `import[rs]` / js-proc / cs-proc）。0.3 の 6 並列で落ちた 11 件の
    原因をここで特定し、規則はできるだけ書いた内容（`import[..]` の種類）から決める（新しいテストが自動で正しい側に
    入るように）。
  - ベンチの繰り返しの回数（合否のためだけに走らせる回数）。
  - 結果（[gate/README.md](../gate/README.md)）:
    - 確かめ用のモジュールは `gate/lib/check.ar`（`check_eq` / `check` / `fail` と例外 `GateCheckFailed`）。テンプレートは
      型引数を明示しないと呼べない（`check_eq[int](..)`）ので、引数は `Any` にした（`==` と `repr` だけを使う）。
      テストからは相対 import（`from ..lib.check import ..`）で読む（作業ディレクトリに依らず読めることを確かめた）。
    - 期待の書式: `# expect: <段階> <種類> ["断片"]`（段階は `parse` / `expand` / `static` / `warning` / `runtime`）、位置の
      無い誤りは先頭の `# expect-file:`。実行時の誤りの行は「テストのファイル自身の、いちばん内側のフレーム」。
    - **逐次に回す規則は要らなかった。** 6 並列で落ちた 8 件（0.3 では 11 件と書いたが、cpp-lib の中継 DLL のキャッシュを
      先に温めると 8 件）は、cpp-dll の中継のビルド・読み込む DLL・`.arc` から取り出す DLL・`import[rs]` の cargo の
      プロジェクトが**一時ディレクトリの固定の名前**を使い、同時に走る arrow.exe どうしが取り合っていたため（`LNK1104`・
      `couldn't read _tl_cpp_bridge.rs`・`manifest path … does not exist`）。プロセスごとの作業ディレクトリに直した
      （`partial_compiler::private_temp_dir`・古いものは次に作るときに消す）。直した後、9 カテゴリの 733 件で逐次と 6 並列の
      exit が全件一致した（逐次 168 秒・6 並列 36 秒）。⚠ 逐次の回でも、同じ時刻に別のセッションが同じ一時ファイルを
      使って `cpp_list_to_c` が落ちていた（同じ原因）。
    - ⚠ 2026-11-24 にリモートの master（copy_alloc_speed_plan.md 7-8 を含む）へ載せ直した。7-8 は**読み込む DLL**
      （`.arc` から取り出す DLL・cpp-dll の中継 DLL）を中身のハッシュの名前で置き、プロセスをまたいで使い回す
      （`materialize_native_lib`・書き直すと読み込みのたびにウイルス対策の検査を受けるため）。取り合いも同じく解く。
      ⇒ 読み込む DLL は 7-8 の形を採り、`private_temp_dir` はビルドの中間（cpp-dll の中継のビルド・`import[rs]` の
      cargo のプロジェクト・`--compile` の clang）にだけ使う。
    - 残る取り合い: 同じ cpp-lib の中継 DLL（ヘッダの隣の 1 つのファイル）を同時に作り直すこと。ランナーは並列の前に
      cpp-lib のテストを 1 本ずつ走らせて温める（3-5）。
    - 名前: `examples/` には名前が `_error` で終わるのに正常に終わる例題が 5 件ある（`eof_error` など。主題が例外）。
      `gate/` では `_error.ar` を禁止の形だけに使い、判定（3-4）は名前と期待の有無の食い違いも不合格にする。
    - cpp-lib の中継 DLL は `examples/` と同じくコミットする（作り直しに MSVC が要り、`ar_config.json` の `cpp.msvc` が
      環境によっては無いパスを指す・この環境でも C: 側を指していて作り直せなかった）。
    - ⚠ stderr をファイルに落としても色の符号（ANSI）が付き、`NO_COLOR` も効かない。3-1 の出力が要る理由の 1 つ。
- **2-2（済・2026-11-22）** 規則を文書に書く: `regulations.md` に「新しい文法・新しい禁止には `gate/` のテスト（正常系と禁止の形）も
  書く」を足す（D-13）。CLAUDE.md に `gate/` と `examples/` の役割の違いを書く。
- **2-3（済・2026-11-22）** 確かめ用のモジュールを作り、各主題で 1 本ずつ試しに書き（正常系・静的な誤りをまとめたファイル・
  構文の誤り・捕まえない実行時の誤り・捕まえる実行時の誤り・確かめ用のモジュールそのものの確かめ・cpp-lib・ベンチ・REPL）、
  2-1 の規則で足りるかを確かめる。照らす仕組み（3-4）はまだ無いので、ここでは書けるか・読めるかだけを見る。
  - 書いたもの: `gate/check/`（`check_self` と、確かめ落ちになるべき 3 本）・`gate/lang/variables/immutability_error`
    （静的な誤り 5 件をまとめたもの）・`let_without_value_parse_error`・`gate/lang/operators/division_by_zero_error`・
    `gate/ffi/cpp/cpp_struct_ptr`（cpp-lib・中継 DLL を含む）・`gate/bench/int_loop`（0.1 秒）・
    `gate/repl/state_across_blocks.in` / `.out`。どれも意図どおりの止まり方・終わり方をした。
  - 分かったこと（3 節以降の前提）:
    - **種類の名前は今の出力からは分からない**（`immutability_error` の `K = 4` が `AssignToConst` か `AssignToImmutable` か
      は表の文面だけでは決まらない）。期待は 3-1 の後に 3-4 で確かめて直す。
    - `try` の中で出た `IndexError` を捕まえずに止まると、位置が 1 つも出ない（`Traceback` の見出しだけ・0.3 の表）。
      `check_wrong_exception_error` は `# expect-file:` で書いた。
    - **REPL は静的な誤りを表示せずに実行を続ける**（`src/repl.rs` の `run_block` が型検査の誤りを捨てる。ブロックを
      1 つずつ検査すると前のブロックの名前を知らないためと見ている）。`let b: int = "s"` が通り、`b` は `s` を返す。
      本計画では直さない（`gate/repl/` には静的な誤りを書かない）。直すかは別に決める。
    - cpp-lib の中継 DLL を初めて作るには、この環境の MSVC を指す一時的な `ar_config.json` が要った（README 4 節）。

### フェーズ 3: 診断の出力と共通ランナー

- **3-1（済・2026-11-22）** arrow.exe に機械向けの診断の出力を足す（D-14・0.3 の表）。
  - 結果: `AR_DIAG_OUT=<ファイル>`（`src/diag_out.rs`）。立っていれば起動時にファイルを空で作り、誤りごとに 1 行の JSON
    （`stage` / `kind` / `file` / `line` / `col` / `message` / `frames`）を書く。種類の名前は `TypeErrorKind::name` /
    `TypeWarningKind::name`（網羅の `match`・85 種と 6 種）。例外の行は `Interpreter::exception_line` に切り出し、
    トレースバックと同じ 1 箇所から取る。
  - 確かめ（9 カテゴリの 733 件・6 並列・環境変数あり / なし）: stdout / stderr / exit が変わったもの 0 件（経過時間を
    出す 18 件を除く）。**exit が 0 でないのに診断が 1 件も無いもの 0 件**（止まる経路を全部覆った）。exit が 0 なのに
    誤りの診断があるもの 0 件。静的な誤り 426 件の行・桁は全件が今の表と一致。単体テスト 5 件。
  - `examples/` の基準（D-12・5-1 で測り直して比べる）: 段階ごとに parse 31・expand 27・static 426・warning 6・runtime 63 件。
    static の種類は 85 種のうち 68 種が出た。runtime の種類は 16 種（`TypeError` 27・`ValueError` 10・`FfiTypeError` 6 …）。
  - ⚠⚠ **捕まえない実行時の誤りの大半に位置が無い**（63 件中 57 件。stderr も `TypeError: …` の 1 行だけ）。0.3 では
    「無いものがある」と書いたが、実際は大半。expand も 27 件すべて位置が無い。parse は全件に位置がある（種類は無い）。
    ⇒ `gate/` の実行時の禁止の形は、多くを `# expect-file:` で書くことになり、行で照らせない。位置を付けるのは本計画の
    範囲外（6 節）だが、別の課題として起票する価値がある（照らす精度がそこで決まる）。
  - 環境変数（名前は実装時に決める。例: `AR_DIAG_OUT=<ファイル>`）が立っているときだけ、**そのファイルへ** 1 行に
    1 件の JSON を書く。stdout / stderr は変えない（A/B 系・今のゲートの出力が変わらないように）。
  - 欄: 段階（構文・展開・テンプレート・静的・実行時・警告）、種類（静的は `TypeErrorKind` の名前・実行時は例外の
    クラスの名前）、ファイル、行、桁、文面（色の符号なし・折り返しなし）。位置が無いものは行・桁を空にする
    （黙って 0 にしない）。
  - 種類の名前は `TypeErrorKind` の網羅 `match` で返す（`_ =>` で既定の名前に落とさない。新しい種類を足すと
    コンパイルが止まるように）。
  - 確かめ: `cargo test` に単体テスト。`examples/` の誤りの例題（約 290 件）に走らせ、静的な誤りの行・桁が今の表と
    一致すること（`compare_wasm_frontend` の鍵と同じ）。ここで `examples/` の禁止の形の基準（D-12）を保存する。
  - ⚠ wasm のフロントエンドも同じ型検査を使うので、種類の名前を wasm の診断にも載せられるかを見る（7-3 で使う）。
- **3-2（済・2026-11-22）** 共通ランナー（`scripts/gate_run.ps1`・dot-source して使う関数の形）を作る。入力は exe・ファイルの一覧・
  環境変数・制限時間。ファイルごとに exit / stdout / stderr / 3-1 の診断 / 経過時間 / タイムアウトの有無を
  `target/gate_results/<exe のハッシュ>/` に保存する。保存済みなら実行しない（`-Fresh` で実行し直す）。全部に
  `CreateNoWindow` を付ける。タイムアウトしたら子プロセスごと止める（`taskkill /T`）。この段階では逐次。
  - 結果: `Invoke-GateRun -Exe -Root -Files [-WorkDir file|<dir>] [-ExtraEnv] [-TimeoutSec] [-Jobs] [-Fresh]`。読むのは
    `Read-GateText`（共有モード）・`Read-GateDiag`（3-1 の JSON。ファイルが無ければ `$null`）。
  - ⚠ 保存の鍵は exe の中身だけでは足りない（テストや import 先を直した後に古い結果を読む）。exe の中身のハッシュの下に、
    対象のディレクトリの全ファイルの（相対パス・大きさ・更新時刻）・実行するファイルの一覧・環境変数・作業ディレクトリ・
    制限時間のハッシュで分けた。`gate/` で、新しく実行 → 保存を読む → ファイルの更新時刻を変えると実行し直す、を確かめた。
    タイムアウト（`taskkill /T /F`）で子が残らないことも確かめた。index は全部そろったときだけ書く。
  - `-Jobs` の入れ物は作ったが、既定は 1（並列は 3-5 で確かめてから）。
- **3-3（済・2026-11-22）** `scan_examples` と `force_gate` の判定を、保存した結果から出す形で書く。対象のフォルダは引数で選べるように
  する。**まず `examples/` を対象にして、古いスクリプトと同じ判定が出ることを確かめる**（4 節）。
  - 結果: 2 本とも `gate_run.ps1` で実行し、`gate_judge.ps1` の判定（`Write-GateScanReport` / `Get-GateForceHits`）で出す。
    名前・引数・最後の行は同じ（D-2）。足した引数: `-Root <dir>`（`gate/` など）・`-Fresh`・`-Jobs`、`force_gate` に
    `-Exclude <正規表現>`。`examples/` を対象にするときの対象・作業ディレクトリ（`scan` はリポジトリ直下・`force` は
    例題のディレクトリ）・猶予は今までと同じ。窓を閉じる猶予はランナーの `-GraceSec`。
  - 確かめ（同じ exe・古い版はコミット済みの写し）: `scan` は出力の行（`FAIL` 2 行・`SCAN-DONE`）が一致。`force` は窓を出す
    例題と `practical_examples` を除いた 811 件で一致（`811 完走 / 0 サンプル判定`・`FORCE-GATE: 0`）。
    窓を出す例題を含めた比較（2026-11-23・1-1 と一緒に走らせた）: 古い版も新しい版（`-Root examples`）も
    `831 完走（うち 1 件はタイムアウト後に窓を閉じて終了） / 0 サンプル判定`・`FORCE-GATE: 0`・窓を閉じて終わった
    例題は同じ `interop/cs_form_app` で、判定は一致した。時間は古い版 175.0 秒（逐次）、新しい版 73.1 秒（6 並列）。
  - ⚠ 最初の比較で `examples/debugger/` の 6 件が新しい版だけ時間切れになった。`CreateNoWindow` の子は隠れた
    コンソールから標準入力を読むので、デバッガの例題が入力を待ち続けた（古い版は親の標準入力＝EOF を引き継いでいた）。
    ランナーで標準入力を `NUL` にした。
  - ⚠ 逐次では少し遅くなった（`force` 125 → 161 秒、`scan` 96 → 98 秒）。出力をファイルへ落とすために 1 件ごとに
    `cmd.exe` を挟む分（1 件あたり約 45 ms）。並列（3-5）で取り返す。
- **3-4（済・2026-11-22）** 期待との照合（D-14）を判定に足す（`scripts/gate_expect.ps1`）。期待をファイルから読み、3-1 の診断と照らす。
  正の対照を 4 節のとおり取る。2-3 で試しに書いたテストで動くことを確かめる。
  - 結果: 照合は `scripts/gate_judge.ps1` の `Test-GateResult`（`Read-GateExpectations` / `Get-GateDiagLine`）、入口は
    `scripts/gate_expect.ps1`（最後の行 `GATE-EXPECT: <合格> passed, <不合格> failed`・不合格があれば exit 1）。正常系も同じ
    判定に入れた（exit 0 で診断 0 件。`GateCheckFailed` で止まれば `CHECK-FAILED`、ほかは `FAILED`）。
  - 期待の書式は厳しく読む（種類の後に置けるのは `"断片"` だけ。読めなければ不合格）。誤りが別のファイル（import 先）で
    出たら `expect-file` とだけ照らす。実行時の誤りはテストのファイル自身のいちばん内側のフレームの行。
  - `gate/` の 9 本: 全部合格。2-3 の `immutability_error` の期待 1 つ（`AssignToConst` → `AssignToImmutable`）は 3-1 の
    出力で直した。
  - 正の対照（13 件・全部が期待した理由で不合格）: 誤りが出ない（exit 0）・期待より少ない・余分・段階違い・種類違い・
    行違い・断片違い・期待の無いテストで誤り・`_error` なのに期待が無い・期待があるのに `_error` でない・読めない期待・
    確かめ落ち（`CHECK-FAILED`）・実行時の行（呼び出し側の行に書いた期待は、いちばん内側のフレームと合わない）。
    合格すべき 2 本（正常系・断片つきの静的な誤り）は合格。
  - ⚠ PowerShell は空の配列を `return` すると `$null` に崩す。`Read-GateDiag` は `,@(..)` で返し、「診断のファイルが
    空」と「ファイルが無い（arrow.exe が書く前に止まった）」を区別する。
- **3-5（済・2026-11-22）** 並列の実行を入れる（既定の数は論理コアの半分。`-Jobs 1` で逐次）。逐次の印（2-1）が付いたファイルは逐次。
  並列と逐次で保存される結果が同じことを、5 回繰り返して確かめる。
  - 結果: `Invoke-GateRun -Jobs 0`（既定）で論理コアの半分（この機械では 6）。逐次の印は要らなかった（2-1）が、
    `import[cpp-lib]` を書いたファイルだけは先に 1 本ずつ走らせる（中継 DLL のキャッシュを温める・同時に作り直すと
    取り合う）。`scan_examples` / `force_gate` / `gate_expect` の `-Jobs` の既定も 0 にした。
  - 確かめ: 窓を出す例題を除いた `force_gate` の対象 811 件で、逐次 1 回（159 秒）と並列 5 回（各 約 40 秒）の
    exit / stdout / stderr を比べた。5 回とも差は `bench/speed_modes/` の 3 件だけで、中身は経過時間の数字（1-3 の 18 件と
    同じ種類。1-3 は 9 カテゴリの直下だけを見ていたので漏れていた）。それ以外は全件一致。

### フェーズ 4: `gate/` のテストを書く

1 タスクごとにコミットする。各タスクの終わりに、その範囲で D-12 の基準を下回らないこと（構文・型義務・禁止の段階と
種類）と、全部が 3-4 の照合を通ることを見る。

- **4-1（済・2026-11-23）** 言語の中核（`examples/` の `basics` / `collections` / `classes` / `typing` / `exceptions` に当たる範囲）の正常系。
  捕まえられる実行時の誤りもここで確かめる（D-14）。
  - 方法: 5 カテゴリの正常の例題 205 件を `gate/lang/<主題>/` に写した。`print(..)` を `emit([str(..), ..], sep=.., end=..)` に
    置き換え、写したときの stdout を末尾の `check_output([..])` の期待にした（`gate/lib/check.ar`・README の 2.2）。
    import とモジュールを主題にする 11 件は 4-7、`debug_demo` は 4-9 に回した。名前が `_errors` で終わる 3 件
    （`functions_errors` / `other_typing_errors` / `try_except_errors`）は落ちる例題なので 4-2 に回す。
  - 写すときに直したもの:
    - 例題のパス（`examples/basics/_tmp_*.txt` など・作業ディレクトリが違う）: 7 件。
    - 実行ごとに変わる値（`slice` の中のアドレス・`id` の値）: 2 件。
    - 写しの見出しでずれた行番号（`raise_span_fields`）: 1 件。
    - 展開時の `print`（`emit` は展開時に呼べない）: 1 件。
    - 警告が出る 3 件: 警告を期待として書き、`_warning.ar` にした。判定に「警告だけを期待するテストは exit 0 で
      最後まで走る」規則を足した（README の 3.4。正の対照 6 件で不合格になることを見た）。
  - 期待の正しさの確かめ:
    - 例題の行末のコメント（`# 15`）と、その行が 1 回だけ出した行を突き合わせた。1,244 件のうち説明文 12 件を除いた
      1,232 件で合った。表記の違いでない食い違いは 1 件で、例題のコメントが古かった（`classes/polymorphism.ar`）。
    - `impl_python` の stdout とは 202 件中 95 件だけが一致した（`impl_python` が止まっているため）。根拠にはしていない。
  - 結果:
    - `gate/` 全体の 211 件が `gate_expect` に合格した。`-Fresh` で 2 回走らせても同じで、2 回目は保存した結果を使った。
    - `force_gate -Root gate/lang` で `VmForceError` は 0 件。
    - `syntax_cov`（`-Root` を足した）では、例題の同じ範囲と同じ穴だった。`gate/` に無いのは `BreakPoint` だけで、
      これは `debug_demo`（4-9）にしか無い。
  - 共通ランナーも直した: テストが書く `_tmp_*` をツリーの指紋から外した（入れると毎回指紋が変わり、保存した結果が
    使われない）。`.gitignore` に `gate/**/_tmp_*.txt` を足した。
  - ⚠ 見つけた別件: `id(x).value > 0`（`uint` と `int` の比較）が静的には通り、実行時に位置の無い
    `TypeError: unsupported operand types for Gt: uint and int` になる。`builtins/built_in.ar` では文字列にして比べた。
- **4-2（済・2026-11-23）** 同じ範囲の禁止の形。静的な誤りは主題ごとにまとめ、構文・展開・捕まえない実行時の誤りは 1 件 1 ファイル（D-14）。
  - 方法: 5 カテゴリの禁止の例題 193 件（`_error` 190・`_errors` 3。import が主題の 9 件は 4-7）を `gate/lang/<主題>/` に
    写した。写したファイルを走らせて出た診断（3-1）から行末の `# expect:` を書いた。位置の無いものは `# expect-file:` にした。
    構文・実行時・展開の誤りには文面の断片も付けた。
  - ⚠ **D-14 の「静的な誤りは主題ごとに 1 ファイルにまとめる」はしなかった（1 件 1 ファイルのまま）。** まとめる目的は
    プロセスの数を減らすことだったが、実測で `gate/` 全体の 405 件が 5.5〜6.9 秒（6 並列）と小さく、まとめても縮むのは
    2 秒ほど。一方、まとめるとトップレベルにしか書けない宣言（`class` など）を関数に入れられず、推論の連鎖が変わって、
    例題と同じ禁止を確かめているとは言えなくなる。集まりの一致で照らす点（D-14 の判定）は変わらない。
  - 期待が例題の意図どおりかの見直し:
    - 診断の行の近くにコメントの無い 90 件は、その行を読んで禁止の形だと確かめた。
    - 例題が禁止の印（`⛔`）を付けた行のうち、期待の付かない行を全部見た。位置の無い実行時の誤りと、文の頭の行
      （`try:` / `match` / `enum`）に報告される静的な誤りを除くと、食い違いは `typing/builtin_args_error` の 2 行だけ
      だった。**例題が古かった**: `open(path)`（今は open_mode を省ける）と `print(1, sep=" ")`（`sep=` を受けるように
      なった）は今は誤りにならない。写しからは外した（例題の側は直していない）。
    - 例題の見出しに並ぶ「期待されるエラー」の数と比べた。違うのは上の 1 件と、「静的に止まらなければ実行時に
      `TypeError`」と書いてあるだけの 4 件。
    - 例題のパスを開いて `FileNotFoundError` で止まっていた（例題の意図と違う理由）3 件は、パスを直して期待を付け直した。
      直した後は意図どおり `EOFError` / `ValueError`（閉じたファイル）になった。
    - `exceptions/eof_error` は正常に終わる例題なので、正常系 `exceptions/eof_exception.ar` として写した（README 1 節）。
  - 結果:
    - `gate/` 全体の 405 件が、`-Fresh` で 2 回とも合格した。
    - D-12 の禁止の形の基準を満たした。例題（同じ 184 件）の誤りの段階と種類 73 通りが全部 `gate/` の期待にある
      （`gate/` は 76 通り）。
  - ⚠ 写した見出しの注記に「`# expect:`」と書いたら、照合がそれを期待として読み、読めない期待として不合格にした
    （厳しく読む規則が働いた）。注記の言い回しを変えた。
- **4-3（済・2026-11-23）** `async`。同じ exe で 2 回実行して結果が変わらない書き方にする（1-3 の一覧）。
  - `examples/async/` の 12 件のうち、ベンチの `async_bench` を 4-8 に回し、11 件を `gate/async/` に写した（正常系 11・
    禁止の形 1。`async_py_objects` の Python のモジュールは `async_py_objects_mods/` に写した）。
  - 直したもの:
    - `async_demo`: `error_list` の誤りの文がトレースバック（実行したファイルの絶対パスと行番号）を含むので、最初と
      最後の行だけを照らす形にした。
    - `async_thread_bound`: タスクの中で呼ばれるハンドラの出力は `emit` では呼び出し元に届かない（タスクは `mut` の
      束縛し直しを伝えない）ので `print` に戻した。呼ばれたことは `handler_count` が 1 → 0 になることで確かめている
      （README 2.2 に注意を足した）。
  - 結果: `gate/async` の 12 件が `-Fresh` で 5 回とも合格した。例題のコメントと 1 回だけ出る行の突き合わせは
    14 行で合った（説明文 1 を含む）。
- **4-4（済・2026-11-23）** 外部言語: C / C++（cpp-dll / cpp-lib）。GUI のライブラリ（DxLib）は使わない。
- **4-5（済・2026-11-23）** 外部言語: C#（cs-dll / cs-proc）と JS（js-proc）。フォームは使わない。
- **4-6（済・2026-11-23）** 外部言語: Python（`import[py]`）と Rust（`import[rs]`）。
  - 4-4〜4-6 はまとめて写した（補助を共有するため）。コミットは言語ごとに分けた。
  - 置き場所: `examples/interop/` の補助（`test_modules/` / `py_libs/` / `rs_crates/` / `cs_*/` / `ar_config.json` …・
    追跡されているものだけ）を**同じ名前で** `gate/ffi/` に写し、テストは `gate/ffi/` の直下に置いた。名前を変えると
    import を書き換えることになり、Python のモジュールの名前や型の名前が変わって出力も変わるため。
    ランナーは `ffi/` のサブディレクトリの下を実行しない（`Get-GateTestFiles`・README 1 節）。
    もとの試しの `gate/ffi/cpp/cpp_struct_ptr.ar`（2-3）は、写した `ffi/cpp_struct_ptr.ar` に置き換えた。
  - 対象: `examples/interop/` の 283 件のうち 277 件（C / C++ 36・C# / JS 25・Python / Rust 170・ネイティブの経路と
    モジュール 46。最後の 46 件は 4-7 の範囲）。外したもの: 窓を出す 3 件（`cs_form_app` / `cs_proc_app` /
    `math_render`）と補助（`cs_form_test` / `math_output`）、ベンチの `bench_ab_cdll`（4-8）、この環境で例題そのものが
    落ちる 2 件（`event_external_handler` は `import[rs] libm` のクレートが無い・`importation` は exit 1）。
  - 直したもの:
    - 例題のパス（`examples/interop/_tmp_*` など）: テスト 6 件と Python の補助 1 件（`test_modules/py_os_error_args.py`）。
    - 絶対パスの出力（`js_proc_async_test` の `path.resolve`）: 末尾だけを照らす。
    - 警告の 2 件: `_warning.ar` にした。
    - モジュール `native_method_call_module.ar` を写しの形に書き換えて `.arc` が陳腐化した（README 5 節の検査で
      止まった）。もとのまま写し直した。
    - **外部言語の側の `print`**（27 件）: Python のモジュールやネイティブのコードの `print` は stdout へ直接出て
      `emit` に入らない。`emit` の出力に印を付けて走らせ、Arrow の側の行と外部言語の側の行に分けた。Python の側の
      行は `gate/lib/pycap.ar`（CPython の `sys.stdout` を `io.StringIO` に差し替える）で貯めて `check_text` で照らす
      （26 件・計 300 行余り。README 2.3）。ネイティブのコードの 1 行（`native_builtins`）だけは照らせない。
      Python の側の 1 行を変えた写しで `GateCheckFailed` になることを見た。
    - `py_int_foreign_error` は正常に終わる例題なので、正常系 `py_int_foreign_exception.ar` として写した。
  - cpp-lib の中継 DLL は、写した場所でも印（`.syms`）が一致して作り直されなかった（MSVC は要らなかった）。
  - 結果:
    - `gate/` 全体の 693 件が `-Fresh` で 3 回とも合格した（6 並列で 22〜24 秒）。`force_gate -Root gate` で
      `VmForceError` は 0 件。
    - 禁止の形 84 件は、例題（作業ディレクトリはリポジトリ直下）と写し（`gate/ffi`）で、ファイルごとの誤りの段階と
      種類の集まりが全部同じだった（4-2 の 193 件も同じ方法で全部同じ）。
  - ⚠ 全体を走らせたとき `async/async_demo` が 1 回揺れた（`raise_immediately=True` で 3 つ目のタスクが終わるか
    中断されるかはスレッドの順で決まる）。決まる部分だけを照らす形に直した（4-3 の 5 回では揺れなかった。負荷で出る）。
- **4-7（済・2026-11-23）** import とモジュール（パッケージ・相対 import・再エクスポート・`.arc`）。`compare_import_paths` の 13 例題に
  当たる範囲を含める。
  - `gate/modules/`: `basics` / `typing` / `exceptions` の import が主題の 19 件（正常系 10・禁止の形 9）と、
    補助のディレクトリ 6 つ（`namespace_modules/` など）・モジュール 2 つ（`let_arg_sharing_probe` / `_module`）。
    `ffi/` と同じく、テストは直下だけで、補助は `examples/` と同じ名前で写した（名前は import のパスと型の名前
    `namespace_modules.tags.Tag` に出る）。ランナーの規則を `modules/` にも当てた。
  - `.arc`（ネイティブの経路）とモジュールの 46 件は、補助（`test_modules/`）を共有するので `gate/ffi/` に置いた
    （`native_*` / `cross_module_*` / `module_*` / `stale_arc_check` / `typed_abi` など）。
  - `compare_import_paths` の 13 件: 窓を出す `cs_proc_app` を除く 12 件が `gate/ffi/` にある。
    サブディレクトリから祖先の `ar_config.json` を読む `py_subdir/import_py_ancestor_config.ar` のために、名前が `_tests` で
    終わるディレクトリは `ffi/` / `modules/` の下でも実行する例外を足し、`ffi/py_subdir_tests/` に置いた。
  - モジュールの側の `print`: `let_arg_sharing_probe` は写したモジュールの側で `emit` に置き換えた。
    `meta_modules/greeting_macros` はメタ関数が置くコードの中の `print` なので置き換えず、照らさない（4 行・README 2.3）。
  - 禁止の形 9 件は、例題とファイルごとの誤りの段階と種類の集まりが同じだった。
  - ⚠ 期待の文面の断片に**絶対パス**が入っていた 2 件（`relative_import_error` / `ffi/unresolved_cpp_header_error`）を、
    パスの手前で切った。worktree や別の環境では合わなくなるため。照らす出力の期待にも絶対パスが無いことを確かめた。
  - 結果: `gate/` 全体の 715 件が `-Fresh` で 2 回とも合格した（6 並列で 23〜28 秒）。
  - 置き場所に依らないことの確かめ: worktree（一時ディレクトリ）に取り出し、同じ exe で照らした。初めは 2 件落ちた:
    - `lang/variables/variable.ar`: worktree は `core.autocrlf=true` で `.ar` を CRLF で取り出し、三重引用符の文字列の
      中の CRLF が残って `'triple\r'` になった。⇒ `.gitattributes` で `gate/**/*.ar` を LF で取り出すようにした。
      ⚠ 言語の側の別件: Arrow の字句解析は三重引用符の中の CRLF を残す（CPython は `\n` にそろえる）。
    - `lang/builtins/built_in.ar`: 前の実行が残した一時ファイルの有無で出力の行が変わった。残り物を消す分岐では
      何も出さず、作ったファイルを消すようにした。
    - 直した後、worktree で 2 回（何も残っていない状態と、前の実行の後）とも 715 件が合格した。
- **4-8（済・2026-11-23）** ベンチ。合否のゲートとして走らせる回数にする（今は 23 件で 82 秒。計測は `ab_bench` / `speed_modes` の担当）。
  - `examples/bench/` の 23 件と `async/async_bench` を `gate/bench/` に、`interop/bench_ab_cdll` を `gate/ffi/` に写した。
    `.arc` と対になるモジュール 3 つは書き換えずに写した。`speed_modes/`（`speed_modes.ps1` が測るもの）は写していない。
  - 回数: 大文字の名前の最上位の定数（100 以上）と 1 万以上の整数のリテラルを 100 で割った。時計: `import[py] time` を
    `gate/lib/fake_time.ar`（呼ぶたびに 1 ミリ秒進む）に替えた。経過時間から出す値も決まった値になるので、出力を
    `emit` で貯めて照らす。
  - 結果: どれも 1 本 0.25 秒以内（もとは 23 件で 82 秒）。同じ exe で 2 回走らせて出力が揃った。`gate/` 全体の
    743 件が `-Fresh` で 3 回とも合格した。
  - ⚠ 期待の値は写したときの exe の出力。ベンチの多くはループの結果を捨てているので、照らせるのは「最後まで走る・
    出力の形・`CHECKSUM` などの値」まで。別の実行の経路で突き合わせようとしたが、`--vm=off` は「ツリーウォークの
    経路は取り除いた」として無視されるので、できなかった。
- **4-9（済・2026-11-23）** 対話: REPL とデバッガの golden（`repl_session` / `debug_session` が読むもの）。
  - `examples/repl/repl_session.in/.out` を `gate/repl/` に、`examples/debugger/` の 6 組（`.ar` / `.in` / `.out`）を
    `gate/debugger/` に写した。デバッガの golden の中のパス（`<REPO>\examples\debugger\…`）は `gate\debugger` に直した。
  - `repl_session.ps1` はディレクトリの中の `.in` / `.out` の対を全部照らすようにした（以前は 1 本だけ）。2-3 の試しの
    `state_across_blocks` も照らされる。`debug_session.ps1` と合わせて、既定の読み先を `gate/` にし、`-Dir` で
    `examples/` も指せるようにした。
  - 共通ランナーは `gate/debugger/` を実行しない（`debug_session` が `.in` を流して照らすもの）。
  - 結果: `gate/` で REPL 2 件・デバッガ 6 件が一致した。`-Dir examples/…` でも同じく一致。期待を 1 行変えた REPL の
    写しで `DIFFERING`（exit 1）になることを見た。
  - フェーズ 4 の終わりの構文の網羅（`syntax_cov -Root gate`）: `Stmt` 42 / 43・`Expr` 31 / 32 で、無いのは例題と同じ
    `DebugLet` / `DebugVar`。`NESTED-GAP` は例題の 10 件に `Const` が足されて 11 件（`gate/` には関数の中の `const` が
    あるが入れ子の関数の中には無い。例題には関数の中の `const` も無いので穴として出なかった）。5-1 で埋める。

各外部言語のタスク（4-4〜4-6）も、正常系と禁止の形（境界での型の誤り・`FfiTypeError` など）の両方を書く。

### フェーズ 5: ゲートを `gate/` へ切り替える

- **5-1（済・2026-11-23）** D-12 の条件を全体で確かめる（`gate/` と `examples/` の覆う範囲の比較）。足りないところを書き足す。
  - 構文: `syntax_cov -Pairs` を `examples/`（既定の範囲・`archived` だけを除く 797 件）と `gate/` で取り、文脈の表の
    マス・親 > 子の組・`UNCOVERED`・`NESTED-GAP` を比べた（比べる道具は文脈の表と組の表を読み、例題にあって
    `gate/` に無いものを出す）。初めは次が足りなかった:
    - マス 2 つ: 最上位の `BreakPoint`（`basics/debug_demo`・デバッガの golden として `gate/debugger/debug_demo` を
      足した）、関数の中の `Slice`。
    - `NESTED-GAP` の `Const`（入れ子の関数の中の `const`）。
    - 組 9 つ（`Call>Block` / `Const>Float` / `Expr>BinOp` / `If>Attr` / `If>AttrAssign` / `If>While` / `Static>Call` /
      `Subscript>Cast` / `While>UnaryOp`）。出どころは 9 カテゴリの外: `examples/alias.ar`（`examples/` の直下）、GUI の
      `apps/spider_*` / `interop/cs_form_app`、`bench/speed_modes/`、`practical_examples/`。
    - ⇒ `examples/` の直下の `alias.ar` / `alias_error.ar` を `gate/lang/alias/` に写し（4-1 / 4-2 で漏れていた）、
      残りは `gate/lang/syntax/context_forms.ar` に 1 回ずつ書いた（結果は `check_eq` で確かめる）。
    - 直した後: 例題のマス 295 個・組 575 個が全部 `gate/` にある（`gate/` はマス 298・組 579）。`UNCOVERED` も
      `NESTED-GAP` も例題より増えていない（`DebugLet` / `DebugVar` が無いのは例題と同じ）。
  - 禁止の形: 例題の `_error` / `_errors` 291 件（`archived` / `practical_examples` を除く全部）が出す誤りの段階と種類
    88 通りが、全部 `gate/` の期待にある（`gate/` は 91 通り）。
  - ⚠ 見つけた別件: デバッガで最上位の行を 1 行進めると、同じ行で 2 回止まる（`debug_demo` で 8 → 10 → 10 → 11 → 11）。
    golden は今の動きのまま記録した。
  - 結果: `gate/` の 746 件が合格。REPL 2 件・デバッガ 7 件が一致。
- **5-2（済・2026-11-23）** 誤りの種類の網を足す: `TypeErrorKind` の全種類（約 85）に、期待を持つテストが `gate/` に 1 件以上あること。
  届かない種類は理由つきの一覧に載せる（黙って外さない）。足りなければ exit 1（`type_obligations` と同じ形の網）。
  - `scripts/gate_kinds.ps1`: 種類の一覧は `src/type_check/errors.rs` の `name()`（3-1 で作った網羅の match）から読み、
    `gate/` の `# expect: static|warning <種類>` と照らす。足りない種類は `MISSING`、届かない一覧に載っているのに
    期待がある・もう無い種類が載っているものは `STALE`、存在しない種類を期待に書いたものは `UNKNOWN`。どれかがあれば exit 1。
  - 初めは 91 種類（`TypeErrorKind` 85・`TypeWarningKind` 6）のうち 20 種類に期待が無かった。16 種類は
    `gate/lang/kinds/` に 1 件 1 ファイルで足した（単体テスト `src/frontend_tests/` の入力と、作る場所の条件から書いた）。
  - 届かない 4 種類（理由はスクリプトの中の一覧）:
    - `ProtocolInheritance` / `ProtocolSkippedCompile`: 宣言と表示だけがあり、作る場所が src に無い。
    - `CodeEscapesMetafunction`: 型検査より前に展開が `MetaError` で止める（書いて確かめた）。
    - `UnexpandedMemberDecorator`: 展開が漏れたときだけの守り。
  - 結果: `GATE-KINDS: 91 kinds, 87 covered, 4 unreachable, 0 missing`。正の対照: 範囲を `gate/lang/kinds` に狭めると
    71 種類が `MISSING` で exit 1、存在しない種類の期待は `UNKNOWN` で exit 1。
- **5-3（済・2026-11-23）** `scan_examples` / `force_gate` の対象を `gate/` にし、期待との照合（3-4）を加えて、まとめて走らせる入口
  （`scripts/gates_every_time.ps1`）を作る。`test_build_gate` → 実行 1 回 → 判定。1 つでも落ちたら exit 1。
  `examples/` は「毎回」の対象に入れない（D-16）。
  - `gate_judge.ps1` に `Invoke-GateSuite` を足した。`gate/` を決まった引数（全部のテスト・名前の順・作業ディレクトリは
    ファイルのディレクトリ・60 秒・猶予 0）で走らせる。`gate_expect` / `scan_examples` / `force_gate` /
    `gates_every_time` がどれもこれを呼ぶので保存のキーが揃い、1 回走らせれば残りは読むだけになる（4 本続けて走らせ、
    2 本目からは `reused 762` になることを見た）。
  - `scan_examples` / `force_gate` の既定の対象を `gate/` にした。`-Root examples` で今までの例題の形（作業ディレクトリ・
    9 カテゴリ / 再帰・時間切れ・猶予もそのまま）。どちらも落ちたものがあれば exit 1 を返すようにした（以前は一覧を
    出すだけで exit 0）。
  - `gates_every_time.ps1`: `test_build_gate` → `gate/` を 1 回 → 期待との照合・`VmForceError`・`gate_kinds`。最後の行は
    `GATES-EVERY-TIME: OK` / `FAILED (…)`。段ごとの時間も出す。
  - 結果: `-Fresh` で 33.2 秒（`test_build_gate` 0.7・実行 29.3・判定 3.2）、保存した結果を読むときは 3.4 秒。
    正の対照: わざと落ちるテストを置くと `FAILED (gate_expect 1)` で exit 1、消すと OK に戻った。
  - `scan_examples -Root examples` は、この環境で例題そのものが落ちる `event_external_handler`（`import[rs] libm`）の
    1 件で exit 1 になる（5-4 で扱う）。
- **5-4（済・2026-11-23）** リリースの手順（`scripts/release.ps1`）に `examples/` の確かめを足す（D-16）。正常の例題は exit 0、`_error` は
  exit が 0 でないこと。落ちたらリリースを止める。skill `release` に手順を書く。正の対照として、わざと壊した例題で
  止まることを見る（リリースそのものは走らせず、確かめの部分だけを呼ぶ）。
  - `scripts/check_examples.ps1`: 9 カテゴリの直下（走らせない例題は `scan_examples` と同じ一覧。`gate_judge.ps1` の
    `$GateExampleSkip` / `$GateExampleCategories` に移して共有した）を 1 回走らせる。正常の例題は exit 0、名前の末尾が
    `_error` / `_errors` の例題は exit が 0 でないこと、どれも時間切れにならないこと。名前が `_error` で終わるのに正常に
    終わる 2 件（`eof_error` / `py_int_foreign_error`）は理由つきの一覧で正常の側に入れた。この環境でだけ落ちるものは
    `-Allow <名前>` で許す（書き間違えた名前は止める）。
    - ⚠ `scan_examples -Root examples` は名前の**どこかに** `_error` を含むものを全部除くので、`foreign_error_catch` /
      `py_key_error_args` / `py_os_error_args` など正常に終わる例題の exit 0 を、今まで誰も確かめていなかった。
      `check_examples` はこれらを正常の側で確かめる（今は全部 exit 0）。
  - `release.ps1` の 0' 段: 前提の確認の後、**版を上げる前**に `cargo build --release` と `check_examples.ps1` を走らせる
    （ビルドの後に置くと、落ちたときに版だけ上がった状態が残り、再実行が「版は上がっているのに VSIX がコミットされて
    いない」で止まるため）。`-AllowExampleFailures` を `-Allow` へ渡す。skill `release` に段と落とし穴を書いた。
  - 結果: 725 件のうち、この環境では `interop/event_external_handler`（`import[rs] libm` のクレートが無い）だけが落ちる。
    `-Allow event_external_handler` で `724 ok, 0 failed, 1 allowed`。正の対照: 落ちる正常の例題・exit 0 で終わる
    `_error` の例題を一時的に足すと 2 件とも `FAILED`、書き間違えた `-Allow` の名前も止まって exit 1。`release.ps1` は
    ASCII だけのまま構文が読めることを確かめた（リリースそのものは走らせていない）。
- **5-5（済・2026-11-23）** `compare_python_impl` を凍結する（D-9）。スクリプトの先頭に凍結の理由と日付を書く。
  - 先頭に、凍結の日付・理由（`impl_python` の更新を止めている・4-1 の実測で 202 件中 95 件しか一致しない）・代わりの網
    （`gate/` の D-15 / D-14・入口は `gates_every_time.ps1`）・残す理由（再開するときに方針を決め直す）を書いた。
    CLAUDE.md の表の書き換えは 5-7。
- **5-6（済・2026-11-23）** 全例題を再帰で舐めるゲート（`syntax_cov` / `tw_stats` / `compare_tm_grammar`）の対象を
  `gate/` にする（`compare_wasm_frontend` はフェーズ 7）。
  - 3 本とも既定の対象を `gate/` にし、`-Root examples` で今までの範囲を指せるようにした。
    - `syntax_cov`: 除くものの既定は `gate/` なら `lib/` と `*_mods/`、`examples/` なら `archived/`。
    - `tw_stats`: `gate/` のときは共通ランナーと同じテストの一覧（`Get-GateTestFiles`）。`debugger/` の `.ar` は
      入力を待つ（標準入力を渡さない）ので外れる。
    - `compare_tm_grammar`: `gate/` の `.ar` を全部（補助を含む）と `vscode-extension/test_fixtures/`。
  - 結果: `syntax_cov` 801 件（`Stmt` 42 / 43・`Expr` 31 / 32・`NESTED-GAP` 10 件で例題と同じ）、`tw_stats` 762 件が
    全部走り、ツリーウォークに落ちるのは定義文だけ、`compare_tm_grammar` 838 件で `SPILL` 0・キーワードの一覧が一致。
- **5-7（済・2026-11-23）** 文書を直す: CLAUDE.md のゲートの表と注意書き（「例題ゲートが見ていない場所」の節を含む）、`regulations.md`、
  skill `language-dev-principles` / `vm-pitfalls` / `add-syntax`、`BYTECODE_VM_PLAN.md` のスクリプトの表。
  `generate-codebase-map.ps1` を走らせる。
  - CLAUDE.md: 表の先頭に `gates_every_time.ps1` を置き、`gate_expect` / `gate_kinds` / `check_examples` の行を足した。
    `compare_python_impl` は「凍結中」、`scan_examples` / `force_gate` / `syntax_cov` / `tw_stats` / `compare_tm_grammar` /
    `repl_session` / `debug_session` は対象を `gate/` に書き換えた。「移している途中」と「例題ゲートが見ていない場所」の
    2 つの注意書きは、「ゲートが見るのは `gate/`」の 1 つにまとめた（まだ `examples/` を見る A/B 系と
    `compare_wasm_frontend` も書いた）。
  - `regulations.md`: 「毎回」は `gate/` だけ・例題はリリースのとき・種類を足したら `gate/` のテストも、を足した。
  - skill: `language-dev-principles` の「触った場所 → 見るゲート」の表に毎回の入口・種類の網・例題の行を足し、
    「意味論そのもの」を `gate/` に書き換えた。`add-syntax` に `gate/` のテストを書く手順（2b）と確かめ方を足した。
    `vm-pitfalls` の「非再帰のグロブ」の項に、`gate/` の対象の決まりが 1 か所（`Get-GateTestFiles`）であることを足した。
  - `BYTECODE_VM_PLAN.md` のスクリプトの表: 新しい 5 本（入口・ランナーと判定・照合・種類の網・例題の確かめ）の行を
    足し、既存の 6 本の対象を書き換えた（当時の計測の記録は書き換えていない）。
  - `generate-codebase-map.ps1` を走らせた。`stale_doc_refs` は OK。
- **5-8（済・2026-11-23）** 切り替えた後の時間を測り、1-1 と比べて 5 節の表を置き換える。
  - 切り替えた後の「毎回」は `gates_every_time -Fresh` で約 30 秒。1-1 の当時の「毎回」の 4 本の合計 573.2 秒に対して
    約 1/19。保存した結果を読むだけなら 3.4 秒。A/B 系と wasm（1-1 で 555 秒）はフェーズ 6・7 の対象。

### フェーズ 6: A/B 系

- **6-1（済・2026-11-24）** `compare_outputs` と `compare_bytecode` の対象を `gate/` にし、実行を 1 回にする（`AR_VM_DUMP=1`。1-3 で
  変わらないと確かめた場合）。
  - 共通部分を `scripts/gate_ab.ps1` に置いた（`Invoke-GateAbSides` / `Compare-GateAbResults` / 1 件の文字列の作り方 2 つ）。
    両方のスクリプトは同じ実行の結果を読み、比べ方だけが違う。
  - **「毎回」のゲートの実行にもダンプを付けた**（`Invoke-GateSuite`）。⇒ A/B の新しい側（`target/release`）は
    `gates_every_time` が残した結果をそのまま読む。付ける前に測った: `gate/` の 762 件を `-Fresh` で、付けないと 33.0 / 26.1 秒・
    付けると 27.9 / 26.1 秒（2 回ずつ・差はノイズ）。stderr は計 0.6 MB → 5.2 MB。
    stderr を読む判定（scan の FAIL 行の末尾 3 行）はダンプを除いてから読む（`Split-GateVmDump`・gate_run.ps1）。
  - ダンプの切り出し: 1 つの塊は `== chunk … ==` の行・命令の行・終わりの空行 1 つで、1 回の `eprintln!` で出る
    （`src/vm/compiler/entry.rs`）。stderr の lock があるので別のスレッドの出力が塊の途中に入らない。
  - `compare_outputs` が比べるもの: exit・stdout・ダンプを除いた stderr・**機械向けの診断（3-1）**（以前は無かった）。
    アドレスの正規化は前と同じ。`compare_bytecode` は塊だけ（並びは見ない・6-4）。
  - 以前の除外（`bench` 全部・`async` 全部・外部プロセス・GUI）は無くなった: `gate/` のベンチは偽の時計、async は揃う
    書き方、外部プロセスは共通ランナーがファイルへ落とす、GUI は `gate/` に無い（D-10）。
  - `examples/` を A/B する形は残していない（D-13。`gate/` が `examples/` の範囲を覆う・D-12）。
- **6-2（済・2026-11-24）** 基準側（`-A`）の結果を exe のハッシュで保存する。2 回目からは新しい exe の実行だけになる。中継 DLL の
  キャッシュは D-11 のとおり（`gate/` の下のキャッシュも扱うように直す）。
  - 保存は共通ランナーのもの（`target/gate_results/<exe のハッシュ>/<鍵>`）をそのまま使う。保存があるかを走らせずに見る
    `Test-GateRunSaved`（鍵の計算を `Get-GateRunPlan` に切り出した）を足した。
  - 中継 DLL のキャッシュ: **走らせる exe だけ**を温め（保存を読む側は温めない）、exe ごとにまとめて走らせ、終わったら
    始める前に戻す。`cpp_cache_isolation.ps1` に `-Under`（既定 `gate`）と `-WorkDir file` を足した。
  - 保存した結果が**冷えたキャッシュで取られていた**（`CppBridge:` の行のうち `loading … (permanent)` でないものがある）
    ときは、その exe を温めて走らせ直す（作り直しの表示が偽の差分になるため）。温めた後も作り直したら `WARNING` を出す。
  - ⚠ 中継 DLL のキャッシュのファイルを**ディレクトリの指紋から外した**（gate_run.ps1）。ヘッダ（指紋に入る）と exe
    （鍵に入る）から作られるもので、入れると入れ替えや作り直しのたびに鍵が変わり、基準の exe の保存が使われなくなる。
  - 時間（`gate/` の 762 件・6 並列）: 基準も新しい側も保存が無い初回は約 35 秒（基準の実行 26 秒＋温め・判定）、
    基準の保存がある 2 回目は `compare_bytecode` が 6.5 秒。1-1 の `compare_outputs` 135.0 秒・`compare_bytecode` 218.1 秒に対し、
    続けて走らせても実行は exe ごとに 1 回。
  - ⚠ release の exe は**同じソースから作り直してもハッシュが変わる**（6-4 で `c788c93c…` → `c02330b9…`）。保存が使われない
    だけで、取り違えは起きない。
- **6-3（済・2026-11-24）** `compare_import_paths` を同じ仕組みに載せる（対象は 4-7 のテスト）。
  - 対象は `gate/ffi/` の 13 件。窓を出す `cs_proc_app`（cs-proc を踏む唯一の非 GUI の例題とされていた）の代わりに
    `cs_proc_values` を置いた。一覧のファイルが無ければ `MISSING` で exit 1（黙って減らさない）。
  - 一覧が違うので全件とは別に 1 回走らせる（13 件で 1.4 秒）。初回 3.5〜4.7 秒・保存があれば 1.1 秒（1-1 は 5.0 秒）。
- **6-4（済・2026-11-24）** 同一 exe どうしの負の対照（差分 0）と、わざと出力を変えた exe での正の対照（差分が出る）を取る。
  - 負の対照の作り: `-A` と `-B` が同じ exe（ハッシュが同じ）なら、B を別の保存先（`target/gate_results_negctl`）に毎回
    走らせ直し、**独立した 2 回**を比べる。同じ保存を 2 回読んで「一致」と言わないため。
  - 負の対照の結果:
    - `compare_outputs` 762 / 762。
    - `compare_bytecode` は**初め 756 / 762** だった。違ったのは async を使う 6 件（`async/async_closure_share` /
      `async_demo` / `async_string_share`・`bench/async_bench`・`ffi/async_cs_calls` / `async_native_call`）で、行の数も
      行の集まりも同じで**塊の出る順だけ**が違った（worker スレッドがタスクをそれぞれコンパイルする）。1-1 の
      `examples/interop/async_cs_calls`（713 / 714）も同じ原因。⇒ 塊を並べ替えて比べるようにした。並びはコンパイルの順
      ＝実行の順で、実行の順が変われば出力が変わるので `compare_outputs` が見る。直した後は 2 回とも 762 / 762。
    - `compare_import_paths` 13 / 13。
    - 同じソースから 2 回作った exe（ハッシュが違う）どうし: 762 / 762（出力・バイトコード）、13 / 13。
  - 正の対照: 2 か所を変えた exe を作った（int の `%` のゼロ除算の文言に `POSCTL` を足す・逆アセンブルの `IBIN_LC` の表示を
    変える）。`compare_outputs` は文言を照らす 1 件（`bench/numeric_ops_equivalence`）だけを、`compare_bytecode` は
    `IBIN_LC` を含む 449 件を差に出し、どちらも exit 1。この exe の結果で scan の FAIL 行がダンプではなくトレースバックの
    末尾を出すことも見た。
  - 中継 DLL のキャッシュの印（`cache_stamp`）を変えた exe: 新しい側が温めでも作り直そうとして `WARNING` を出し、
    cpp-lib の 9 件（`compare_import_paths` では 3 件）を差に出した。終わった後、`gate/` のキャッシュ 10 ファイルは中身も
    更新時刻も始める前と同じだった。⚠ ただし、この環境では作り直しそのものができない（ルートの `ar_config.json` の
    `cpp.msvc` が無いパス `C:/…` を指す。MSVC は `H:` にある）ので、**2 つの exe が別々のキャッシュを持つ**場合の入れ替え
    （`the two exes get their own caches`）は走らせて確かめられていない。

### フェーズ 7: wasm の突き合わせ

- **7-1（済・2026-11-24）** `dump_diags.js` がファイルの一覧を受け取り、1 プロセスで全部を解析して 1 行に 1 ファイルの JSON を出す形に
  する。ファイルごとに `setHostCwd` を呼ぶ。拡張も wasm を読んだまま複数のファイルを解析するので、実際の使われ方に近づく。
  - `node dump_diags.js <wasm> --list <files.txt> --out <out.jsonl>`。出力はファイルへ（パイプで受けない・D-5）。解析が
    例外を投げたファイルは `{"file", "error"}` の行にして、捨てない。1 ファイルの形（stdout に JSON）も残した。
  - `compare_wasm_frontend.ps1` はこれを 1 回だけ起動する。`-PerFile` で以前の 1 ファイル 1 プロセスに戻せる。
  - wasm の側の時間: `gate/` の 836 件で 12.7〜14.7 秒、`examples/` の 831 件で 22.3 秒（1 プロセス）。
- **7-2（済・2026-11-24）** exe 側（`AR_CHECK_ONLY=1`）を共通ランナーで実行する。対象を `gate/` にする。
  - 対象は `gate/` の下の全 `.ar`（テストが import するモジュールと `lib/` も。拡張はそれらも開く。以前の `examples/` も
    import 先を含めていた）。`-Root examples` で以前の集まり（`archived/` を除いて再帰）。
  - exe の結果は共通ランナーが exe のハッシュで保存する（6 並列・`gate/` で 25.8 秒。2 回目は読むだけ）。
  - 比べ方（行:桁の鍵・ParseError の文面・import の誤り）は変えていない。
  - ⚠ 変えた判定: wasm の解析が失敗したファイルと exe が時間切れになったファイルを、以前は `skipped` と数えて通していた。
    今は `not compared` として exit 1。
  - 時間: 1-1 は 196.8 秒。今は `-Fresh` 相当の初回が 60〜83 秒（frontend の単体テストと tsc を含む）、exe の保存がある
    2 回目は 24.4 秒。
- **7-3（済・2026-11-24）** 1 ファイルずつ起動する今の形と、診断が全ファイルで同じことを確かめる。前のファイルの状態が残って結果が
  変わるファイルがあれば、それは拡張の不具合なので別に報告する。あわせて、wasm の診断を exe との一致に加えて
  `gate/` の期待（D-14）とも照らす（3-1 で種類の名前を wasm に載せられた場合）。
  - 1 プロセスで全部を解析した JSON と、1 ファイル 1 プロセスの JSON を、ファイルごとに丸ごと比べた（判定ではなく生の
    出力）: `gate/` 836 / 836、`examples/` 831 / 831 が同じ。⇒ 前のファイルの状態が残って結果が変わるファイルは無かった。
  - ゲートの判定: `gate/` 836 件・`examples/` 831 件とも全件一致（1-1 の `examples/` も 831 件一致）。
  - 正の対照: 一覧の形でだけ各ファイルの最初の診断を捨てる `dump_diags.js` にして `gate/` で走らせ、`MISSED` 208・
    `parse mismatch` 8 で exit 1 になった（戻した）。
  - ⚠ wasm の診断を `gate/` の期待と照らすことは**していない**。wasm の診断には種類の名前が無い（`source` は
    `StaticTypeError` などの大分類だけ）。行と桁は exe と一致することをこのゲートが確かめ、exe の診断は `gate_expect` が
    期待と照らすので、行と桁については間接に照らされている。種類まで照らすには、拡張の解析（`crates/arrow-frontend`）が
    返す診断に種類を足す必要があり、本計画の外とする。
  - ⚠ ゲートを書くときの罠: `Code.exe` は GUI のサブシステムの exe なので、PowerShell の `&` は終わるのを**待たない**
    （確かめのスクリプトで、出力ファイルが無いまま先へ進んだ）。`compare_wasm_frontend.ps1` は `Process.WaitForExit` で待つ。

### フェーズ 8: 候補（フェーズ 7 の後に測って判断）

- **8-1（済・2026-11-24）** `annot_unresolved` を共通ランナーに載せる（重複 ⑦）。`type_obligations` の検体（149 件・1 件 1 ファイル）を
  共通ランナーで走らせるかもここで測って決める。
  - 先に測った（同じ exe・逐次で 1 本ずつ）: `annot_unresolved`（`examples/`）155.6 秒、`type_obligations` 4.5 秒、
    `syntax_cov`（`gate/`）70.0 秒、`tw_stats`（`gate/`）119.2 秒、`compare_tm_grammar` 21.2 秒。
  - `annot_unresolved`: 共通ランナーに載せた（並列・exe のハッシュで保存）。`examples/` で 155.6 → 50.2 秒、出力は
    全く同じ（814 件・時間切れ 1 件も同じ）。保存を読むだけなら約 1 秒。
    - ⚠ **既定の対象は `examples/` のままにした**。`gate/`（`-Root gate`）で測ると Unresolved が 1475 件（`examples/` は
      145 件）になり、その大半は `gate/lib/check.ar` のもの: ほぼ全部のテストが import し、それだけで 1 回に Unresolved 3・
      型の混ざった二項演算 19 を数える。これは合否のゲートではなく「普通のコードでどこから Unresolved が出るか」の計測
      なので、母集団を `gate/` にすると答えが変わる。
    - ⚠ `AR_ANNOT_DIFF=1` を「毎回」の実行（`Invoke-GateSuite`）には付けない。その行が `compare_outputs` の比べる stderr
      に入り、内部の数の変化が挙動の差に見えるため（`AR_VM_DUMP` はダンプを切り出して除けるので付けた・6-1）。
  - `type_obligations`: **載せない**。149 件で 4.5 秒しかかからない。
  - 載せる候補に無かったが、測って遅かった 2 本も載せた（どちらも `--features tw_stats` の debug ビルドを使う）:
    - `syntax_cov`: 70.0 → 25.8 秒。まずファイルの dir で全部を走らせ、測れなかったものだけをリポジトリ直下で走らせ直す
      （以前の「作業ディレクトリを 2 通り試す」と同じ）。表・件数は全部同じ。違ったのは測れなかった理由の 2 行の
      ASCII でない文字だけで、以前は stderr を `Get-Content`（ANSI）で読んで化けていた（今は UTF-8 で読む）。
    - `tw_stats`: 119.2 → 36.9 秒。出力は全く同じ。
- **8-2（済・2026-11-24）** `tw_stats` / `syntax_cov` は `--features tw_stats` の別のビルドなので実行はまとめられない。ビルドを 2 本で
  共有しているかだけ確かめる。
  - 共有している。どちらも `cargo build --features tw_stats`（`target/debug`）で、1 回目 16.9 秒・2 回目 0.2 秒。
    `test_build_gate`（debug の `cargo test --no-run`）の直後も 0.2 秒で、互いに追い出さない。直すことは無い。
- **8-3（済・2026-11-24）** `compare_tm_grammar` も 1 ファイルごとに `Code.exe` を起動していないか確かめ、していれば 7-1 と同じ形にする。
  - していない。ファイルの一覧（`--list`）を渡して `Code.exe` を 1 回だけ起動している（838 件で 21.2 秒・tsc を含む）。
    直すことは無い。

## 4. 検証の方針

ゲートそのものを書き換えるので、**新しいゲートが古いゲートより弱くならないこと**を、置き換えるたびに確かめる。

- **共通ランナー（3-3）**: 対象を `examples/` にして、同じ exe で古いスクリプトと新しい実装の出力（`FAIL` /
  `TIMEOUT` の行・`VmForceError` の一覧・件数）を比べる。
- **診断の出力（3-1）**: 静的な誤りの行・桁が今の表と全件で一致すること。stdout / stderr が環境変数の有無で変わらないこと。
- **`gate/` のテスト（フェーズ 4・5-1）**: 覆う範囲が `examples/` を下回らないこと（D-12）。
- **正の対照**: 落ちるはずの状態を作って、新しいゲートが落ちることを見る。
  - `scan`: 例外で終わるファイル・確かめ用のモジュールで `raise` するファイルを一時的に足す。「確かめ落ち」と
    「異常終了」が分けて表示されること（D-15）。
  - 確かめ用のモジュール（D-15）: 合わない値・期待した例外が出ない・別の例外が出る、のそれぞれで確かめ落ちになること。
    これは `gate/` に D-14 の期待つきのテストとして常に置く。
  - `force`: stderr に `VmForceError` を出す exe（または保存した結果を書き換える）。
  - 期待との照合（3-4）: 次のそれぞれが不合格になること — 誤りが出ない（exit 0）・期待より誤りが少ない・期待に無い
    誤りが余分に出る・段階が違う・種類が違う・行が違う・文面の断片が合わない・期待の無いファイルで誤りが出る。
  - 保存: exe を作り直したら保存を使わないこと。
- **負の対照**: `-Fresh` で全部実行した結果と、保存から読んだ結果が同じ。
- 並列（3-5）は 5 回繰り返して判定が変わらないこと。
- ゲートは `target/release` を見る。自分で走らせて緑を確かめる（CLAUDE.md）。

## 5. 見込み（推測。1-1 と各フェーズの実測で置き換える）

| | 今（フェーズ 3 より前） | フェーズ 5 の後（推測） | フェーズ 5 の後（実測・2026-11-23） |
|---|---:|---:|---:|
| 「毎回」のテストの実行回数（Rust 側） | 3 回 | 1 回 | 1 回（`gate/` の 762 件・6 並列） |
| 「毎回」の `impl_python` の実行 | 約 700 件 × 170 ms 以上 | 0（凍結・D-9） | 0 |
| 「毎回」のゲートの合計 | **573.2 秒**（1-1・今の exe で。`test_build_gate` 0.4・`scan` 80.3・`force` 175.0・`compare_python_impl` 317.5） | 1 分前後 | **約 30 秒**（`gates_every_time -Fresh`・3 回で 29.1〜30.6 秒。`test_build_gate` 0.3・実行 26〜27・判定 3）。保存した結果を読むだけなら 3.4 秒 |
| 触ったときに走らせる A/B 系と wasm | 555 秒（1-1・`compare_wasm_frontend` 196.8・`compare_outputs` 135.0・`compare_bytecode` 218.1・`compare_import_paths` 5.0） | — | A/B 系 3 本（フェーズ 6 の後）: 基準の exe の初回は `compare_outputs` 約 35 秒、その後の `compare_bytecode` 6.5 秒・`compare_import_paths` 約 4 秒（新しい側は「毎回」のゲートの保存を読む）。`compare_wasm_frontend`（フェーズ 7 の後・`gate/` の 836 件）: 初回 60〜83 秒、exe の保存があれば 24.4 秒 |

- ⚠ 「今」の列は、計画を立てた後に直した exe（1-2 の `.arc`・2-1 の一時ディレクトリ）で測った値。計画を立てたときの exe なら
  `scan` は 156 秒だった。スクリプトはフェーズ 3 より前の版。

- つなぎ合わせでプロセスの数が減る分（例題の中央値 59 ms のうち起動が約 24 ms）、GUI・タイムアウトが無くなる分、
  並列の分が効く。禁止の形も、静的な誤り（例題の `_error` の多く）を主題ごとにまとめる分だけ減る（D-14）。
  下限は逐次に残す外部言語のテスト（cpp / rs / js / cs）とベンチで決まる。
- 照合（3-4）は保存した結果を読むだけなので、実行の時間はほぼ増えない。
- A/B 系は 4 回の実行が 1 回になる（基準側は最初の 1 回だけ）。wasm は 85 秒の大半が消える見込み。
- ⚠ フェーズ 4（テストの書き下ろし）がこの計画でいちばん大きい作業になる（今の例題は 9 カテゴリで約 720 件、
  うち `_error` が約 290 件）。

## 6. 本計画でやらないこと

- **窓が出る件を単独で扱うこと。** `gate/` に GUI を入れず（D-10）、共通ランナーが全部に `CreateNoWindow` を付ける
  （3-2）ので、切り替えた後は窓が出なくなる見込み。切り替えるまでの間の今のスクリプトは直さない（利用者が先に
  直したい場合は別に扱う）。
- **覆う範囲を減らすこと。** `gate/` は `examples/` と同じ範囲を覆う（D-12）。減るのは GUI（D-10）と `impl_python`
  との突き合わせ（D-9）、ベンチの繰り返しの回数（4-8）だけ。
- **`examples/` のコメントの期待を照らせる形に書き直すこと。** `examples/` は利用者向けの見本（D-13）。照らすのは
  `gate/` のテストだけ。
- **位置の無い実行時の誤り・展開の誤りに位置を付けること**（0.3）。3-1 の出力では行・桁を空にして、そのまま見せる。
  直すなら別の計画で。
- **`impl_python/` の変更と、`impl_python` との突き合わせの再開。** 更新を止めている（D-9）。
- **速度を測るスクリプト**（`ab_bench` / `speed_modes` / `prof_dist` など）の変更。
- **`cargo test` そのものを速くすること。**
