# `examples/practical_examples/` — ⚠ ゲート対象外（ungated）

外部ライブラリを実際に叩く「実用デモ」の置き場。**回帰ゲートで動作を検証していない。**

⚠ ここが ungated なのには理由がある。実行に **この repo の外の環境**が要る:

- `DxLib/` … DxLib の DLL と **GUI ウィンドウ**（CI でも headless でも完走しない）
- `pd_numpy_pyplot/` … Python の `pandas` / `numpy` / `matplotlib`

⇒ 「環境が揃っていないから落ちた」と「コードが壊れているから落ちた」を
ゲートが区別できないので、`scan_examples.ps1` の対象から外してある。

## どのゲートが見ていて、どのゲートが見ていないか

| ゲート | ここを見るか | 何を検査するか |
|---|---|---|
| `scan_examples.ps1` | ❌ | 例題が失敗しないか |
| `compare_outputs.ps1` | ❌ | 2 バイナリの stdout/exit が同一か |
| `compare_bytecode.ps1` | ❌ | 2 バイナリのバイトコードが同一か |
| `compare_python_impl.ps1` | ❌ | 参照実装 `impl_python` との差分 |
| `force_gate.ps1` | ✅ | VM に載らない構文が無いか |
| `compare_wasm_frontend.ps1` | ✅ | 拡張の wasm が `arrow.exe` と同じ診断を出すか |

⚠⚠ 下の 2 つは走査対象に含むが、**どちらも「例題が成功するか」は見ていない**
（前者はバイトコード化の可否、後者は 2 実装の診断の一致）。⇒ **壊れていても素通りする。**

## 現状（実測 2026-10-16・ffi_value_exchange_plan.md 7-13）

| ファイル | 状態 |
|---|---|
| `DxLib/ant_render.ar` | ✅ 成功（`.arc` を `--compile` で作り直した） |
| `DxLib/dxlib_window.ar` ・ `dxlib_string.ar` | ✅ 成功（10 秒・5 秒で自分で閉じる。`SetAlwaysRunFlag(1)` でウィンドウが前面でなくても進む） |
| `DxLib/langtons_ant.ar` ・ `langtons_ant_profile.ar` ・ `life_game.ar` | ⏱ GUI ウィンドウで待つ（閉じるまで動く。タイムアウト＝想定どおり） |
| `DxLib/matrix_progress.ar` | ⛔ **静的エラー**: py-int のクラスのインスタンスが `Any` になり属性を使えない（⇒ 7-27） |
| `pd_numpy_pyplot/data_analysis.ar` | ✅ 成功（グラフを `data/sales_analysis.png` へ書く） |

**DxLib の SDK の置き場所**: `examples/DxLib/`（`DxLib.h` と `.lib`・git の対象外）。例題は
`import[cpp-lib] ...DxLib.DxLib`（2 つ上のディレクトリ）で読む。`TCHAR` などの Windows の型は `ar_config.json` の
`cpp.system_headers`（Windows SDK の `Windows.h`）から読むので、インストールされた SDK の版に合わせること。

⚠ 2026-09-18 〜 10-15 の失敗の原因（7-13 で直した）: ヘッダの置き場所（CPython と同じ探索になってエントリの
ディレクトリの `DxLib/` を探していた）、存在しない SDK の版、`DEFAULTPARAM` の既定値を読まない（`ClearDrawScreen()` が
引数の数の静的エラー）、`RECT` などのシステムヘッダの構造体を知らない、`const TCHAR*` の `const` が typedef で外れる、
`1000 / FPS` が `float`（`WaitTimer` は `int`）、`data_analysis.ar` の CSV のパス。

## ここを直したい場合

型検査を厳しくしたときにここが落ちるのは**想定内**（ゲートに映らないので気づきにくい）。

⚠ 直したあともゲートは見てくれない。**変更のたびに手で走らせること。**
