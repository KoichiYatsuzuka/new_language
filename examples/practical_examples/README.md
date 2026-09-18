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

## 現状（実測 2026-09-18）

| ファイル | 状態 |
|---|---|
| `DxLib/ant_render.ar` | ✅ 成功 |
| `DxLib/langtons_ant.ar` ・ `langtons_ant_profile.ar` | ⏱ GUI ウィンドウで待つ（タイムアウト＝想定どおり） |
| `DxLib/dxlib_window.ar` ・ `dxlib_string.ar` ・ `life_game.ar` | ⛔ **静的エラー**: `'ClearDrawScreen' takes 1 argument(s) but 0 were given` |
| `DxLib/matrix_progress.ar` | ⛔ **静的エラー**: `cannot apply attribute access to …` |
| `pd_numpy_pyplot/data_analysis.ar` | ⛔ **静的エラー**: `parameter 'backend' of 'use' expects a mutable argument` |

⚠⚠ **失敗 6 件は「環境不足」ではなく本物の静的型エラー**（実測で確認した）。
DxLib の 3 件はスタブのシグネチャ不一致で、**プログラムが走る前に落ちている**。

## ここを直したい場合

型検査を厳しくしたときにここが落ちるのは**想定内**（ゲートに映らないので気づきにくい）。
直すなら:

1. DxLib の 3 件 … `import[cpp-dll]` のスタブ生成側（`ClearDrawScreen` の引数）を見る
2. `data_analysis.ar` … `mpl.use("Agg")` の `mut` 実引数の扱い

⚠ 直したあともゲートは見てくれない。**変更のたびに手で走らせること。**
