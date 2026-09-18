# `examples/archived/` — ⚠ ゲート対象外（ungated）

このディレクトリの例題は **`scripts/` の回帰ゲートで動作を検証していない**。
過去に役目を終えた例題の置き場であり、**現在の言語仕様に追随していない**。

⚠⚠ **ここのファイルを「動く例」として読まないこと。** 実測（2026-09-18）で
**72 件中 53 件が失敗する**（成功するのは 19 件）。

## どのゲートが見ていて、どのゲートが見ていないか

| ゲート | ここを見るか | 何を検査するか |
|---|---|---|
| `scan_examples.ps1` | ❌ | 例題が失敗しないか |
| `compare_outputs.ps1` | ❌ | 2 バイナリの stdout/exit が同一か |
| `compare_bytecode.ps1` | ❌ | 2 バイナリのバイトコードが同一か |
| `compare_python_impl.ps1` | ❌ | 参照実装 `impl_python` との差分 |
| `force_gate.ps1` | ✅ | VM に載らない構文が無いか |
| `compare_wasm_frontend.ps1` | ✅ | 拡張の wasm が `arrow.exe` と同じ診断を出すか |

上の 4 つは `$categoryDirs`（`basics` / `collections` / `classes` / `typing` /
`exceptions` / `async` / `bench` / `apps` / `interop`）だけを走査する。
下の 2 つは `examples/` を `-Recurse` で舐めるのでここも含むが、
**どちらも「例題が成功するか」は見ていない**（前者はバイトコード化の可否、
後者は 2 実装の診断の一致）。⇒ **壊れていても素通りする。**

## 失敗 53 件の内訳（実測 2026-09-18）

| 原因 | 件数 | 例 |
|---|---|---|
| `StaticTypeError`（型検査） | 18 | `id.ar`（未宣言フィールドへの代入）・`decorator.ar` |
| **UTF-8 BOM** をレキサが弾く | 13 | `bench_hard.ar` `compile_types.ar` `point_ops.ar` ほか |
| 外部ファイル・モジュール不足 | 10 | `py_import.ar`（`py_calculator` が無い）・`point_class.ar`（`point.h` が無い） |
| クラス継承（現在は trait のみ基底に許可） | 3 | `class_demo.ar` `class_syntax.ar` `overload_success.ar` |
| その他 | 8 | `file_io.ar`（一時ファイルのパス）ほか |

## ⚠⚠ 計測の母集団に混ぜないこと

タスク 7.5（メンバー存在検査）の偽陽性計測で、**ここの壊れた例題を「偽陽性」として
数えかけた**。新しい検査の影響を測るときは、母集団を
`$categoryDirs` の 9 カテゴリに限ること。

## 直す場合

このディレクトリの例題を現役に戻すなら、**直したうえで 9 カテゴリのどれかへ移す**こと。
`archived/` に置いたまま直しても、上表のとおり誰も検証し続けてくれない。
