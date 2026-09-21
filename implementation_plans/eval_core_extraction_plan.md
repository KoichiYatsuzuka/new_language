# 評価コアの切り出し（メタ関数 0-4 / 0-5 の実体）

[comptime_metafn_design.md](comptime_metafn_design.md) のタスク `0-4`（評価コアの切り出し・A 案）と
`0-5`（`Value` の非 wasm variant を feature 化）は、実測すると**単一コミットに収まらない**ので本書へ分割する。

- 作成: 2026-09-22
- 状態: **#1 完了（2026-09-22）・#2 以降未着手**
- 採番は `#1, #2, ...`（フェーズに分ける必要がないため。`.claude/rules/regulations.md`）
- 位置はシンボル名で指す（行番号は書かない）

---

## 0. なぜ必要か

D4（展開時評価器は既存 VM に機能を積む）と D5（拡張はメタ関数展開後のコードを型検査する）を
両立させるには、**VM と評価コアが wasm フロントエンドでも動く**必要がある。
そうしないと拡張とコンパイラの型認識が食い違い、`compare_wasm_frontend.ps1` が守っている
不変条件が壊れる。

## 1. 実測（2026-09-22）

### VM は Interpreter を丸ごと要求する

```rust
pub fn run(interp: &mut Interpreter, chunk: &Chunk, ...)
```

VM から `interp.<name>` の呼び出しは **68 種**。うち **32 種が `vm_` 接頭辞**＝ VM のために
Interpreter 側に置かれた支援コードで、実質すでにコアの一部。
⇒ **VM だけを切り出すことは不可能。**

### 非コアは全体の約 4 割

| 分類 | 行数 | コアか |
|---|---|---|
| `tests` | 6,880 | 非コア（`#[cfg(test)]`） |
| `cpp_bridge` | 3,338 | 非コア（FFI） |
| `native_api` | 1,431 | 非コア（FFI） |
| `py_interop` / `cs_dll` / `cs_proc` / `js_proc` | 1,144 | 非コア |
| `event_loop` / `async` | 1,297 | 非コア |
| `exec/modules` | 962 | 一部必要（D32: import 越しの `const`） |
| `templates` | 1,033 | D36 で単相化に置換 |
| **小計（非コア）** | **約 14,000** | |
| **`src/interpreter` 合計** | **35,065** | |
| `src/vm` | 8,412 | コア |
| frontend crate 現状（lexer + parser + type_check） | 20,545 | |

⇒ コアは概算 **21,000（interpreter）＋ 8,400（vm）≒ 29,000 行**。

### 本当の難所は `Value` の非 wasm variant

| variant | match しているファイル数 |
|---|---|
| `PyObject` | 17 |
| `NativeFunction` | 12 |
| `CsObject` | 12 |
| `AsyncManager` | 10 |
| `Signal` | 10 |
| `JsProcFn` | 7 |
| `EventLoop` | 7 |
| `FileObject` | 6 |

計 **124 箇所**。網羅 `match` の全アームに `#[cfg]` が要る。
⚠ プロジェクトは網羅性を drift 制御の武器にしている（`language-dev-principles` §2）ので、
ここを崩さないことが最大の制約。

### 非コアモジュールへの参照元

`native_api` 7 ファイル / `py_interop` 8 / `cs_dll_runtime` 5 / `event_loop` 7 / `async_mgr` 7 /
`cpp_bridge` 1（`tests` を除く）。

## 2. 方針

**新しい feature を作らず、既存の `editor` feature を拡張する。**

`editor` は既に「fs に触れない import 解析へ差し替える」「`parser/cs_assembly` を外す」という
**同種の切り分け**を行っており（`src/lexer` 8 箇所 / `src/parser` 58 箇所 / `src/type_check` 4 箇所）、
スイッチを 2 つに増やす理由がない。

⇒ `editor` を「**エディタ解析ビルド = FFI も async も持たない評価コア**」の意味に広げる。

## 3. タスク

| # | 内容 | 前提 | 規模 |
|---|---|---|---|
| ~~#1~~ | ~~`editor` feature を評価コア用に配線する~~ **完了（2026-09-22）**。ルート crate が `--features editor` で**ビルドできるようになった**（`main.rs` の `--compile-cs` が `parser::cs_assembly` を参照していた 1 箇所を `#[cfg]` で落とした）。⇒ **#2 以降を wasm を経由せず `cargo build --features editor` で検証できる**（速い反復路の確保がこのタスクの実体） | — | 小 |
| **#2** | `Value` の非 wasm variant 8 種を `#[cfg(not(feature = "editor"))]` で外し、網羅 `match` 124 箇所を手当てする | #1 | **大** |
| **#3** | FFI モジュール（`cpp_bridge` / `native_api` / `py_interop` / `cs_dll_runtime` / `cs_proc` / `js_proc`）を外す | #2 | 中 |
| **#4** | `event_loop` / `async_mgr` を外す | #2 | 中 |
| **#5** | `exec/modules` の FFI import 経路を外す（`import[ar]` と `const` 読み取りは残す・D32） | #3, #4 | 中 |
| **#6** | frontend crate に `src/interpreter` と `src/vm` を `#[path]` で取り込み、**wasm32 ビルドを通す** | #5 | 中 |
| **#7** | ゲートを張る（`compare_wasm_frontend` / `test_build_gate` / `scan_examples` / `force_gate` / `compare_outputs`） | #6 | 中 |

⚠ **#2 が全体の律速。** ここを飛ばして #3 以降はできない（variant が残っているとモジュールを外せない）。

## 4. ⚠ 注意

- **`cargo test`（debug）も通すこと。** release ゲートは `cfg(debug_assertions)` の強制点を見ない
  （2026-09-20 に各所へ記録された注意）。⇒ 毎回 `test_build_gate.ps1` を前段に置く。
- **frontend crate にネイティブ依存を足さない。** `Cargo.toml` に明記された制約。
  `pyo3` / `libloading` / `rustpython-parser` が入った瞬間に wasm32 ビルドが壊れる。
- **`[profile.release]` を共有しない。** frontend crate は workspace の `exclude` に置かれており、
  ルートの profile（速度ベンチの基準）に影響させないため。
- **網羅 `match` を `_ => {}` で潰さない。** variant を足したときに止まらなくなる
  （`language-dev-principles` §2 の 2 段強制）。`#[cfg]` で**アームごと**消す。
- 並行セッションが同じ領域（型検査・テンプレート）を触っている。**着手前に `git log` を確認する。**

### ⚠⚠ `--features editor` のビルドは `target/release/arrow.exe` を上書きする（#1 で実際に踏んだ）

`cargo build --release --features editor` は**同じ profile・同じ target ディレクトリ**なので、
ゲートが見る `target/release/arrow.exe` が**エディタ版に差し替わる**。
エディタ版は import が fs に触れないので、`scan_examples` が **import 系 50 本以上を一斉に FAIL** させる。

⇒ **検証後は必ず `cargo build --release`（feature なし）に戻してからゲートを走らせる。**
⇒ 迷ったら `ls -la target/release/arrow.exe` で時刻を見る。サイズも違う（feature なし 11.3MB / editor 版 6.9MB）。

⚠ これは `vm-pitfalls` の「ゲートは `target/release` を見る」「緑だと報告された、ではなく自分で走らせて緑を確かめる」と同じ罠の別形。
