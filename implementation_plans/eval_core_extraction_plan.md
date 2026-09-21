# 評価コアの切り出し（メタ関数 0-4 / 0-5 の実体）

[comptime_metafn_design.md](comptime_metafn_design.md) のタスク `0-4`（評価コアの切り出し・A 案）と
`0-5`（`Value` の非 wasm variant を feature 化）は、実測すると**単一コミットに収まらない**ので本書へ分割する。

- 作成: 2026-09-22
- 状態: **#1〜#7 完了（2026-09-22）。評価コアが wasm32 でビルドでき、拡張の crate に載った**
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
| ~~#2~~ | ~~ネイティブ依存を feature で外す~~ **完了（2026-09-22）**。`cargo build --no-default-features --features editor` が**エラー 0** で通る | #1 | 中 |
| ~~#3~~ | ~~FFI モジュールを外す~~ **完了（#2 と同時・2026-09-22）**。`cpp_bridge` は丸ごと落とし、`py_interop` / `cs_dll_runtime` / `native_api` / `eval/native` は**同名スタブへ差し替え** | #2 | 中 |
| ~~#4~~ | ~~`event_loop` / `async_mgr` を外す~~ **不要と判明（2026-09-22）**。どちらもネイティブ crate に依存しておらず（`std` のみ）、評価コアビルドを妨げない。wasm 実行時に動くかは #6 で判断する | #2 | — |
| ~~#5~~ | ~~`exec/modules` の FFI import 経路を外す~~ **完了（2026-09-22）**。併せて `proc_bridge` / `cs_proc_runtime` / `js_proc_runtime` / `msvc_errors` / `partial_compiler::rs_loader` も落とし、**死にコード警告 100 → 22 件**。`import[ar]` と `.arc` の AST 読み取りは残っている | #3 | 中 |
| ~~#6~~ | ~~frontend crate に取り込み wasm32 ビルドを通す~~ **完了（2026-09-22）**。`cargo build --release --target wasm32-unknown-unknown` がエラー 0・警告 0 |  #5 | 中 |
| ~~#7~~ | ~~ゲートを張る~~ **完了（2026-09-22）**。`compare_wasm_frontend` のフロントエンド単体テストが **23 → 482 件**（interpreter のテストが評価コア構成で走るようになった）。これが「評価コアが壊れていないか」の常設の網になる | #6 | 中 |

⚠ **#2 が全体の律速。**

### ⚠⚠ #2 の方針転換（2026-09-22）— 網羅 match は触らない

当初は「`Value` の非 wasm variant 8 種を外し、網羅 `match` 124 箇所を手当てする」計画だったが、
payload 型を調べたところ**ネイティブ crate に触れているのは 2 型だけ**だった
（`PyObjHandle.inner: pyo3::Py<PyAny>` と `NativeLibWrapper(libloading::Library)`）。
`NativeFnRef` は `PathBuf` / `String` / `usize` / `Vec<bool>` など**素のデータが大半**。

⇒ **variant は全ビルドに残し、payload の中身だけ差し替える**（評価コアでは `Infallible` ＝構築不能）。
これで **124 箇所の網羅 `match` に一切触らずに済み**、2 段強制（`language-dev-principles` §2）も保たれる。

### 実際の作業量（実測）

依存を optional 化して `cargo build --no-default-features --features editor` を通すと、
エラーは **23 関数 / 14 ファイル**に収束した。

| ファイル | 対象関数 |
|---|---|
| `exec/modules.rs` | `build_cpp_typed_sig` / `exec_module` / `import_cs_dll` / `load_cpp_module` / `load_cpp_wrapper_dll` / `sig_to_ptr_param_fn` / `try_load_native_module` |
| `classes/method_call.rs` | `call_instance_method_evaled` / `eval_method_call_full` |
| `classes/instantiate.rs` | `instantiate_evaled` |
| `classes/class_methods.rs` | `eval_class_method` |
| `eval/attrs.rs` | `get_attr_val` |
| `eval/builtins.rs` | `eval_builtin_evaled` / `eval_builtin_ident_call` |
| `eval/calls.rs` | `call_value_evaled` |
| `eval/subscript.rs` | `eval_setitem` / `eval_subscript` |
| `exec/control_flow.rs` | `make_for_iterator` |
| `ops/display.rs` | `display` |
| `ops/operators.rs` | `apply_binop` |
| `value/native.rs` | `invoke_typed_abi` |
| `partial_compiler/{mod,rs_loader/mod}.rs` | トップレベル |

### 済んだもの

- `Cargo.toml`: `pyo3` / `libloading` / `rustpython-parser` を **optional** にし、`native` feature で有効化。`default = ["native"]` なので従来のビルドは不変
- `python_converter` / `cpp_bridge` / `native_api` / `cs_dll_runtime` / `py_interop` を `#[cfg(feature = "native")]` でモジュールごと落とす
- `PyObjHandle` / `NativeLibWrapper` の payload を feature 依存にする
- `eval/native.rs` を **`eval/native_stub.rs` へ差し替え**（`parser` の `imports` / `imports_editor` と同じ形）。
  ⚠ **呼び出し側に `#[cfg]` を配らないため**に、面（シグネチャ）は残して実装だけ落とす方針

### 決着（2026-09-22）

エラー推移: 63 →（モジュール落としで一時増）89 → 58 → 31 → 20 → 12 → 6 → 2 → **0**。

**効いたのは「同名スタブへの差し替え」**。`py_interop` / `cs_dll_runtime` / `native_api` / `eval/native`
の 4 モジュールをスタブに差し替えただけで 31 件 → 2 件まで落ちた。
**呼び出し側に `#[cfg]` を 1 つも配っていない**（`Value::PyObject` / `CsObject` /
`NativeFunction` の match アームは全ビルドでそのまま）。

`#[cfg]` を直接置いたのは 3 箇所だけ:
- `exec_import` の FFI アーム（評価コアでは `_` へ落とさず**明示エラー**にする。落とすと
  `exec_module` が普通のモジュールとして探しに行き、理由の分からない「ファイルが無い」になる）
- `ops/display.rs` の `Value::PyObject` 表示（アームは残し、評価コアでは `"<PyObject>"` 固定）
- `exec/modules.rs` のネイティブ払い出し読み込み（評価コアではネイティブ部分を無視して
  通常のモジュールとして続行。`.arc` は AST も持っているので解析は成立する）

⚠ **`#[path]` は宣言元ファイルのディレクトリ基準**。`interpreter.rs` は `src/` にあるので
`#[path = "interpreter/xxx_stub.rs"]` と書く（`eval/mod.rs` 側は `mod.rs` なので相対のままでよい）。実装中に踏んだ。

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

### ⚠ #6 で踏んだもの

- **`#[path]` の無い `mod x;` は取り込み側から解決できない。** `interpreter.rs` の兄弟モジュールが
  すべて `#[path = "interpreter/..."]` を持っているのはこのため。`ffi_boundary` だけ漏れていたので足した
- **frontend crate に `native` / `prof` / `tw_stats` の feature 宣言が要る**（有効にはしない）。
  取り込んだ共有ソースの `#[cfg]` に対する `unexpected_cfgs` 警告を防ぐため。ルート crate が
  `editor` を宣言しているのと同じ理由
- **interpreter の単体テストも frontend crate で走るようになる。** Python 相互運用を実際に動かす
  テスト（`tests/pyobject` ほか 4 本）は評価コアでは必ず落ちるので `#[cfg(feature = "native")]` を付けた
- `regex` を frontend の依存に追加（`str_methods` が使う）。**純 Rust なので wasm32 に載る**

⚠ これは `vm-pitfalls` の「ゲートは `target/release` を見る」「緑だと報告された、ではなく自分で走らせて緑を確かめる」と同じ罠の別形。
