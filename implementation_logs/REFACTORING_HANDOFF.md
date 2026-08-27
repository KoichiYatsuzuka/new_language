# リファクタリング引き継ぎ (Rust側 src/)

Arrow(LLVM IR ターゲットのスクリプト言語)Rust実装のリファクタリング進捗ハンドオフ。
**このファイルだけ読めば次スレッドで再開できる**ことを目的とする。

## 現況サマリ
- 対象: `src/`(Rust実装、~55,700行)。`impl_python/` は未変更。
- ブランチ: リファクタ用の別ブランチでコミット管理中(ユーザー運用)。作業ツリーに Phase 0–3 の変更あり。
- テスト: **`cargo test` → 672 passed / 0 failed**(Phase 0–3 通して緑を維持)。
- **`cargo build`(既定=非llvm)は通る。`cargo build --features llvm` は通らない**(MSVC用 LLVM 開発SDK未導入。詳細は「LLVM環境の結論」参照)。
- 各フェーズ完了報告の末尾に**残タスク一覧を必ず表示する**こと(ユーザー要望・memory化済み)。

## LLVM環境の結論(Phase 4 の前提)
- ネイティブcodegenは2実装が併存: `llvm_codegen/`(テキストIR→外部`clang.exe`→DLL、非cfg・既定)と `inkwell_codegen/`(libLLVM静的リンク・JIT、`#[cfg(feature="llvm")]`)。
- inkwell を使うには **MSVCでビルドされた LLVM dev libs(`.lib`+`llvm-config.exe`)** が必要。winget版(clangのみ)・MSYS2版(mingw/GNU ABI)はいずれも**MSVCツールチェーンと非互換で不可**(実ビルドで確認済: cl.exe が mingw ヘッダを解釈できず失敗)。入手はMSVCソースビルド/vcpkgのみ。
- **決定した方針 = A′: `llvm_codegen`(clang経路)に統一し、`inkwell_codegen` を削除する。** clang.exe は導入済み(`C:\Program Files\LLVM\bin\clang.exe`、`ar_config.json` の `llvm.path` 経由で発見)、A′は現環境で検証しながら完遂できる。
- 補足: `ar_config.json` の `llvm` セクション(path)は **A′でも clang 発見に使うので残す**。`.cargo/config.toml` の `LLVM_SYS_*` は A′では不要になる(`PYO3_PYTHON` は残す)。

---

## Phase 0 — 地ならし(完了)
1. [frontend_tests/lexer_tests.rs](../src/frontend_tests/lexer_tests.rs) の float リテラルテストに `#[allow(clippy::approx_constant)]`。→ `cargo clippy --all-targets` が exit 101→0。
2. `cargo clippy --fix`(bin+tests)で機械修正99+件(`map_or`簡約・`needless_return`・`redundant_closure`・`manual_strip`・テストの未使用 `crate::interpreter::*` 20件 等)。bin警告 175→65。
3. 確定デッドコード**12件**削除:
   - `async_queue` field / `unsubscribe_by_name` — event_loop.rs
   - `release_handle` / `path` field — cs_dll_runtime.rs
   - `path` field — cs_proc_runtime.rs / `bridge_script` field — js_proc_runtime.rs
   - `find_cs_bridge_dll` — exec/modules.rs
   - `push_value_writeback` / `clear_native_methods` — native_api/mod.rs
   - `INST_FIELD_INIT_MASK` / `is_float` / `class_id`(メソッド) — value/instance.rs

## Phase 1 — 抑制解除で依存可視化(完了)
- **Part1a**: `ast.rs` + `type_check/*`(13ファイル)の `#![allow(dead_code)]` 撤去。
- **Part1b**: 炙り出した4件:
  - 削除: `c_abi_byte_width`([ast.rs](../src/ast.rs))、`MemberKind::Method.access` field([type_check/mod.rs](../src/type_check/mod.rs))+構築3箇所。
  - **reserved保持**(バリアント単位 `#[allow(dead_code)]`+`TODO(reserved)`): `ProtocolInheritance`(TypeErrorKind)、`ProtocolSkippedCompile`(TypeWarningKind)@ [errors.rs](../src/type_check/errors.rs)。メッセージ定義済みだが未発火の意図的な将来診断。
- **Part2**: `type_check/stmt/{check,resolve,protocol}.rs` の `#[allow(unused_imports)]` 撤去 + `cargo fix` で import narrowing(コピペkitchen-sink importを実使用に圧縮)。

## Phase 2 — 重複集約(完了)
- **Item3**: [module_compiler.rs](../src/partial_compiler/module_compiler.rs) の `write_tlc_v1`/`v2` 完全同一ロジックを `write_tlc_native(…, version)` に統合。`write_len_prefixed`/`read_len_prefixed`/`read_string` ヘルパで手書きバイト直列化の重複を集約。
- **Item1**: 新規 [proc_bridge.rs](../src/interpreter/proc_bridge.rs) に `PipeConn` を新設し、cs_proc/js_proc に二重実装だったパイプ機構(名前付きパイプFFI `open_pipe_client`・READYハンドシェイク・`send_recv`・`PIPE_COUNTER`・`Drop`)を一本化。両ランタイムは自分の spawn引数・op・値エンコードのみ保持。ドリフトしていたリトライ定数(5/100ms vs 8/150ms)を **8/150ms に統一**。`interpreter.rs` に `mod proc_bridge` 登録。
- **Item2/4 はドロップ**(調査の結果): Value⇄JSON統合=cs/jsで別コーデックのため不適 / cs_dll `call_variadic`→native_api方式=C#側が固定シグネチャ位置引数で不可。
- **ドキュメント反映済**: codebase-map再生成、importation SKILL(`JsBridge`節→`PipeConn`)、partial-compile SKILL(`write_tlc`記述)、interpreter-internals SKILL(`INST_FIELD_INIT_MASK`行削除)。
- 検証: `cargo run -- examples/interop/js_proc_test.ar` を実Nodeブリッジで実行し全出力正常(end-to-end)。

## Phase 3 — ネスト/ボイラープレート(Item1のみ完了)
- **Item1**: [method_call.rs](../src/interpreter/classes/method_call.rs) に3ヘルパ導入し `eval_method_call` の反復検証を集約:
  - `expect_no_args(args, type, method)`(14箇所)/ `eval_one_arg(args, type, method)`(9箇所)/ `set_other_items(other, method)`(6箇所)。
  - 対象: List / FrozenList / Complex / Dict / Set(13メソッド)/ Generator / AsyncManager。**エラーメッセージは完全に元と同一**。
- **Item2(TypeChecker神クラス分割)→ Phase 5 に昇格。**

---

## Phase 0–3 の取りこぼし(→ **Phase 4 で全て処理済**。記録として残す)
1. **Phase 0 延期の確定デッドコード(未削除)**: `FnExport.class_name`×2 / タプル `.0` 未読×2([value/native.rs](../src/interpreter/value/native.rs), [parser/cs_assembly/mod.rs](../src/parser/cs_assembly/mod.rs)) / `Registry` variant([rs_loader/mod.rs](../src/partial_compiler/rs_loader/mod.rs))。
   → `class_name`は当初 `--features llvm` 検証待ちで延期したが、**A′(inkwell削除)後は非cfgコードのみになり削除・検証可能**。`Registry` は [loader.rs:266](../src/partial_compiler/rs_loader/loader.rs#L266) で match されている中途配線機能(削除は要方針判断)。**Phase 4 で一括処理推奨。**
2. **crate全体(~125ファイル)の `#[allow(unused_imports)]` narrowing(未実施)**: Phase 1 Part2 は type_check の3ファイルのみ実施。残りは「非llvm一括fixが llvm-only import を誤削除する」懸念で延期していたが、**A′で inkwell を削除すればその懸念は消える**(他cfg=windows等は残る)。Phase 4 後にまとめて `cargo fix` 可能。
3. **残 clippy 警告 ~65件(未対応)**: 自動修正不可の様式系(`collapsible_match`・`type_complexity`(複雑型)・`doc_lazy_continuation`・`needless_lifetimes` 等)。各フェーズで順次。
4. **Phase 3 Item1 の横展開余地**: `method_call.rs` のみ適用。`classes/object_methods.rs` にも同種の引数検証がある可能性(未確認)。`classes/string_methods.rs` は既に `arg_str!`/`arg_opt_str!` マクロ化済み。
5. `cs_dll_runtime.rs` は直接FFI機構のため proc_bridge に統合していない(意図的)。

---

## Phase 4 — LLVM統一(A′: clang経路に一本化・inkwell削除)【完了】
1. **inkwell バックエンド削除**: `src/partial_compiler/inkwell_codegen/`(5ファイル)をディレクトリごと削除、`partial_compiler/mod.rs` の cfg mod 宣言も削除。
2. **フィーチャ/依存の除去**: Cargo.toml の `inkwell` 依存・`[features]` セクションごと削除。`.cargo/config.toml` の `LLVM_SYS_*` 削除(`PYO3_PYTHON` は残置)。**`build.rs` は `CARGO_FEATURE_LLVM` 専用スクリプトだったのでファイルごと削除**。
3. **`#[cfg(feature="llvm")]` ゲート全廃**: `compile_native` は llvm_codegen(clang)一本。`exec/modules.rs` の JIT 分岐・`load_jit_module`・`Interpreter::jit_handles` を削除。
4. **NativePayload / .arc v2 整理**: `Bitcode` variant・`VERSION_V2`・v2 write/parse を削除。`NativePayload` は `Dll` のみの単一 variant(将来の拡張点として enum のまま残置)。`parse_tlc` は v1 が上限。
5. **JIT ポインタ経路の単純化**: `NativeFnRef.raw_fn_ptr` **フィールドごと削除**(常に0だった)。eval/native.rs×2・native_api/callbacks.rs の二分岐を DLL(libloading + `cached_fn_ptr`)経路のみに簡約。
6. **Phase 0/1 延期デッドコードの回収**:
   - 削除: `FnExport.class_name`(構築8箇所)、`FlatListInfo.class_name`、`PropertyRole::EventAdder` のペイロード(常に `String::new()` のダミーだった → unit variant 化)。
   - **`PtrArgCleanup::KeepAlive(Vec<u8>)` は削除しなかった**: 「読まれない」が C 呼び出し中ポインタの参照先として**生存させるための RAII フィールド**で、消すと dangling pointer になる。`#[allow(dead_code)]` + 理由コメントを付与。※ 旧ハンドオフの「確定デッドコード」判定は誤りだった。
   - `CrateSource::Registry` は **reserved 保持**(`#[allow(dead_code)]` + `TODO(reserved)`)。`prepare_wrapper` 側の生成は実装済みで `find_config` が未配線なだけ、という状況をコメント化(Phase 1 の `ProtocolInheritance` と同じ扱い)。
7. **横断クリーンアップ(取りこぼし#2)完了**: src/ 全体から `#[allow(unused_imports)]` を 69ファイル・128箇所除去 → `cargo fix --all-targets` で kitchen-sink import を narrowing(**83ファイル / -2,636行**)。
   - 例外: [type_check/mod.rs](../src/type_check/mod.rs) の `pub use types::{FnTypeParam, ...}` / `pub use errors::{..., TypeErrorKind, TypeWarningKind}` は **allow を戻した**。bin からは未使用だが `frontend_tests` が使う公開APIで、cargo fix が消すとテストが `E0432` で壊れる(実際に一度壊れた)。
8. **既存 rustc 警告7件(不要 `unsafe` ブロック)も解消** → `cargo build` 警告 **0件**。

**保持したもの**: DLLロード機構(`libloading` / `NativePayload::Dll` / `try_load_native_module`)、`llvm_codegen/`(唯一のバックエンド)、`ar_config.json` の `llvm.path`(clang 発見用)。

**検証結果**:
- `cargo build` → 警告0 / `cargo test` → **672 passed, 0 failed**。
- `cargo run --release -- --compile examples/interop/test_modules/physics.ar` → 7関数の DLL を v1 `.arc` に埋め込み成功 → import 実行で `NativeMethod: Body.* → native` を確認し、**native 経路と `import[ar]` 解釈実行の数値が完全一致**(end-to-end)。※ `examples/interop/test_modules/physics.arc` はこの再生成で更新済み。
- ドキュメント反映済: partial-compile / importation / interpreter-internals / codebase-map SKILL、codebase-map 再生成。

> ⚠️ **PowerShell 5.1 の落とし穴(この作業で実際に踏んだ)**: `Get-Content`/`Set-Content` は既定で ANSI(cp932)。UTF-8 の .rs を読み書きすると**日本語コメントが全滅する**。一括書き換えは `[System.IO.File]::ReadAllLines/WriteAllLines` + `UTF8Encoding($false)` を使うこと。srcを事前バックアップしていたので復旧できた。

## Phase 5 — TypeChecker 神クラス分割【✅ 全完了 5A/5B/5C】
→ **詳細計画と 5A/5B/5C 実施記録は [PHASE5_PLAN.md](PHASE5_PLAN.md)。**

**5C(仕上げ) 完了(2026-07-21)**: `cargo test` 672 緑・ビルド警告0・clippy **63**(5B の 68 から5件減)。
- `block_return` 深さの enter/exit 生呼び出し(barrier 5対 + loop 2対)を `with_barrier`/`with_loop_expr` の**クロージャスコープ方式**ヘルパに集約 → 復元漏れが構造的に不可能に(生呼び出しは scope.rs の2ヘルパ内のみに封じ込め)。RAII は `check_stmts` が self 全体を借用するため不可と判明。
- 式アーム末尾の重複(`return_type` → 型)を `ann_or_unresolved` に集約(5箇所→1)。
- type_check 由来の clippy 警告5件(while-let 化・入れ子 if-let 畳み込み)を全回収。

**5B(巨大関数の平坦化) 完了(2026-07-21)**: `cargo test` 672 緑・ビルド警告0・clippy 68 を維持。挙動・公開API無変更。
- `check_stmt`(stmt/check.rs): **728行・深度14 → 307行・深度8**。巨大アームを `check_if`/`check_match`/`check_fn_def`/`check_gen_def`/`check_let_tuple` へ抽出。深度14の主因だった result_guard の7段ネストを早期return方式の `detect_type_guard`/`detect_result_guard`/`narrow_by_type_guard` に分解。重複3アーム(Let/Const/Mut)を `check_var_decl` に集約。
- `infer`(infer.rs): **373行 → 252行**。`infer_attr`/`infer_unaryop`/`infer_mustbe` を抽出。
- ⚠️ **ツール起因のヒヤリハット**: infer.rs 編集直前に Read が実ファイルと食い違う偽内容(存在しないバリアント名)を返した。破壊的 Edit の直前に grep でシンボル実在を裏取りして気づき回避。詳細は PHASE5_PLAN §9。

**5A(状態の3分割) 完了(2026-07-21)**: `TypeChecker` のフィールド **18 → 3**(`state` / `registry` / `diags`)、[type_check/mod.rs](../src/type_check/mod.rs) **753行 → 121行**。`cargo test` 672 緑・ビルド警告0・clippy 69(着手前と同数)を維持。公開API無変更。
- 新設: [diagnostics.rs](../src/type_check/diagnostics.rs) / [state.rs](../src/type_check/state.rs) / [registry/](../src/type_check/registry/)(mod.rs + builder.rs) / [members.rs](../src/type_check/members.rs)。
- レジストリへの**書き込みは `registry/builder.rs` だけ**に封じ込め済み(`build()` 後は `&self` ゲッターのみ)。検査中の誤書き換えが型レベルで不可能になった。
- 5C 予定だった `block_return_forbidden_depth` のAPI化は前倒しで実施済み(`enter_barrier`/`exit_barrier`・`enter_loop_expr`/`exit_loop_expr`)。**5C の残りは RAII ガード化のみ**。

**Phase 5 は 5A/5B/5C すべて完了。** TypeChecker 神クラス分割は当初計画の全項目を達成:
フィールド 18→3、mod.rs 753→121行、check_stmt 728→307行(深度14→8)、infer 373→252行、
block_return 深さのガード化、clippy 69→63。`cargo test` 672 緑・ビルド警告0 を全工程で維持。

要点のみ:
- `TypeChecker`(**18フィールド**)を `TypeRegistry`(12) / `CheckState`(4) / `Diagnostics`(2) の3構造体へ**合成**で分割。Rust に継承はないので「子クラス」は不可、サブ構造体**相互の依存はゼロ**(スター型DAG)。
- **リスク評価を下方修正**: 実測で `self.<field>` 直接アクセスは **94箇所のみ**(旧想定「数百箇所」は誤り)。状態は既にアクセサ越しに使われており、外部からのフィールド参照はゼロ。
- **決定的な発見**: 宣言レジストリ系12フィールドの書き込みは **`collect_fn_sigs`(mod.rs:217–484)の1関数に完全に閉じている** = 検査中は read-only。ビルダーで組み立てて凍結する型にできる。
- **状態分割では `check_stmt`(728行・深度14)は1行も短くならない**。神クラス感の主因はこちらなので 5B として別建て。順序は **5A(状態分割) → 5B(関数抽出)** 必須(5A により抽出関数が `&mut self` を取らずに済み、借用競合が消えるため)。

---

## Phase 5 完了後に残るタスク
1. **残 clippy 警告 69件**(全て様式系・自動修正不可): `collapsible_match` / `type_complexity` / `doc_lazy_continuation` / `needless_lifetimes` / benches の `irrefutable let...else` 12件 等。
2. **Phase 3 Item1 の横展開余地**: `classes/object_methods.rs` に `expect_no_args`/`eval_one_arg` 相当の重複がある可能性(未確認)。
3. `cs_dll_runtime.rs` は直接FFI機構のため `proc_bridge` に統合していない(意図的・対応不要)。
4. `impl_python/` は Phase 0–4 を通して未着手(Rust側のみのリファクタのため、規約の git SHA 同期は該当せず)。

---

## 検証コマンド早見
```
cargo test                         # 672 passed 期待
cargo clippy --all-targets         # exit 0 期待(警告は残 69 の様式系)
cargo build                        # 唯一の構成(警告0を維持すること)
cargo run -- --compile <file.ar>   # .arc/.ars 生成(native codegen 検証)
../scripts/generate-codebase-map.ps1        # ファイル新規/移動/削除後に必須
```
規約(.claude/rules/regulations.md): 新文法追加時は example と `_error` example を追加 / VS拡張更新時は VSIX 再生成 / Python実装更新時は git SHA同期(今回いずれも該当せず)。
