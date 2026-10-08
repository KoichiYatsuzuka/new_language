# 拡張でも import を CLI と同じに解決する・未解決の import をエラーにする

VS Code 拡張（`vscode-extension/`・解析器は wasm の `crates/arrow-frontend`）は、CLI と同じ字句解析器・
パーサ・型検査器を使っているのに、**import した先の中身が分からない**。`import g` のモジュールで宣言された
関数・クラス・グローバル変数などが、拡張では名前空間のメンバーとして認識されない。また CLI・拡張とも、
import 先が見つからない・読めないときに**黙って型情報を落とす**経路がある。

本書は、拡張でも CLI と同じ import の処理を使い（違うのはファイルの読み方だけ）、未解決の import を
エラーにするための計画書である。

- 起票: 2026-10-08（`enum_member_type_plan.md` 6-3 の後、利用者の指摘から）
- 調査: Arrow `25ca6ad`
- 状態: フェーズ 1 完了（§5）

## 0. 何が起きているか

### 0.1 拡張では import 先の中身が届かない

```
import g            # g.ar に const LIMIT: int = 1 / fn f() -> int: ... がある
g.LIMIT = 5         # CLI: cannot assign to const 'g.LIMIT'   拡張: 何も出ない
let s: str = g.f()  # CLI: 型の不一致                         拡張: 何も出ない
```

- 拡張では、import 先は手作業で作るスタブ（`arrow --emit-stubs` → `.arrow-stubs/`・コマンド
  `Arrow: Refresh External Stubs`）が無い限り**空**として扱われる。スタブは自動では作られない。
- 6-3（`enum_member_type_plan.md`）でスタブにグローバル変数を書くようにしたが、手作業が要る限り
  エディタの拡張としての役割を果たしていない（利用者の指摘）。

### 0.2 未解決の import が黙って型なしになる（CLI）

| import の種類 | 見つからない・読めないとき（`25ca6ad`） |
|---|---|
| `.ar` / `.arc` | エラー（`cannot find module`） |
| `rs` | エラー |
| `py`（変換） | 見つからなければエラー。同梱スタブの変換に失敗したら警告を出して行単位の抽出に落とす |
| `py-int` | `.pyi` / `.py` も同梱スタブも無ければ**黙って型なし**（`py_modules.rs` の `load_python_interface_body`） |
| `cpp-dll` / `cpp-lib` | ヘッダが読めなければ**黙って型なし**（`cpp.rs` の `std::fs::read(..).ok()`） |
| `cs-dll` / `cs-proc` | DLL が無い・読めなければ警告を出して型なし（`cs_js_modules.rs` の `load_cs_module`） |
| `js-proc` | `.ars` が無い・構文解析に失敗したら**黙って型なし**（`load_js_module`） |
| 拡張（全言語） | スタブが無ければ**黙って型なし**（`imports_editor.rs` の `editor_import_body`） |

## 1. 原因

### 1.1 拡張だけ import の処理が差し替えられている

- `src/parser/mod.rs:38-42` で、拡張のビルドでは `parser/imports/`（CLI: import 先を探して読み、構文解析して
  `Stmt::Import::body` に持たせる）が `parser/imports_editor.rs`（構文だけを読む・body はスタブか空）に
  差し替わる。`CLAUDE.md` の「唯一の例外」。
- 型検査器にも、body が空であることを前提にした拡張だけの分岐がある:
  - `type_check/stmt/check.rs:24`（`editor_stub_body`）: body が空なら束縛を `Unresolved` に
  - `type_check/stmt/check.rs:36`（`closed_module`）: 拡張ではメンバーが確定したとみなさない（無いメンバーを誤りにしない）
  - `type_check/mod.rs:310`（`has_unloaded_import`）: 拡張では import が 1 つでもあればレジストリが不完全とみなし、
    型名の存在検査などを止める
- ⇒ 型検査のロジックは同じでも、**入力（import の body）が違う**ので挙動が違う。

### 1.2 差し替えの理由はどちらも成り立たない

`imports_editor.rs` / `stubs.ts` が挙げていた理由:

1. **wasm はファイルを読めない** → ホスト（拡張の TypeScript 側）が読んで渡せばよい（wasm から同期的に
   呼べる関数を用意する）。
2. **import の処理が重く、打鍵ごとに走らせられない**（`importation.ar` で 7.7 秒） → 一度読み込んだモジュールを
   保持すればよい。打鍵ごとに解析し直すのは編集中のファイルだけで、import 先を読み直すのはそのファイルが
   変わったときだけ（一般的なコード解析器と同じ）。

さらに、型の情報はどの言語も**ファイルを読むだけ**で得られる（`arrow.exe` も外部プロセスも要らない）:

| import の種類 | 型の情報の出所 | 必要なもの |
|---|---|---|
| `.ar` / `.arc` | ソース・`.ars` の構文解析 | テキスト |
| `cpp-dll` / `cpp-lib` | `.h` を Rust のヘッダ解析器で読む（中継 DLL のビルドは実行時だけ） | テキスト |
| `cs-dll` / `cs-proc` | DLL の .NET メタデータを Rust で読む（`cs_assembly::load_cs_assembly`） | バイナリ |
| `js-proc` | `.ars` | テキスト |
| `py` / `py-int` | `.py` / `.pyi` を Rust の変換器で読む | テキスト |
| `rs` | crate のソースから `pub fn` / `pub struct` を拾う（`scan_all_sigs`）。import 時の `cargo build` は実行時の DLL のためで、型には使わない | テキスト |

外部プロセスは 1 か所だけ: Python の標準ライブラリ・site-packages の場所を `python -c "import sysconfig..."` で
調べている（`parser/imports/mod.rs` の `python_lib_dirs`）。一度取れば変わらない。

## 2. 決定

- **D-1**: 拡張でも CLI と同じ import の処理（`parser/imports/`）を使う。違うのは**ファイルの読み方だけ**
  （CLI は `std::fs`、拡張はホストの関数）。探索の規則は Rust の 1 か所に置いたまま（TypeScript に
  解析ロジックを持たせない方針は変えない）。`imports_editor.rs` と手作業のスタブの仕組み
  （`stub_registry` / `--emit-stubs` / `stubs.ts` / コマンド）は撤去する。
- **D-2**: 読み込んだモジュールは保持し、**変わったときだけ**読み直す（ホストがファイルの変更を知らせる）。
- **D-3**: 型の情報に関係しない処理は拡張ではしない（`rs` の `cargo build`）。Python の標準ライブラリ・
  site-packages の場所は、ホストが一度だけ調べて渡す（または設定）。
- **D-4**: **未解決の import はエラー**（CLI も拡張も・利用者の指示）。黙って型情報を落とさない。
  0.2 の表の「黙って型なし」「警告を出して型なし」をすべてエラーにし、型の出所の置き方を案内する
  （例: `py-int` で `.pyi` / `.py` が無いモジュールは `.pyi` を置く、`js-proc` は `.ars` を置く）。
  - ⚠ 実行時には解決できても型の出所が無いもの（`py-int` の C 拡張モジュールなど）もエラーになる。

## 3. 実装予定

- フェーズ 1: 未解決の import をエラーにする（CLI・独立して着手できる）
- フェーズ 2: ファイルの読み方の抽象化と、wasm に載らない依存の切り出し
- フェーズ 3: 拡張で Arrow のモジュールを CLI と同じに読む（保持・読み直しを含む）
- フェーズ 4: 拡張で外部言語の import を CLI と同じに読む
- フェーズ 5: スタブの仕組み・拡張だけの分岐の撤去と文書

| # | 内容 | 前提 | 重さ |
|---|---|---|---|
| **1-1** | 0.2 の「黙って型なし」「警告を出して型なし」の経路で、例題（9 カテゴリと `interop`）がどれだけ当たるかを測る（エラーにしたときに落ちる例題の一覧） | なし | 小 |
| **1-2** | 未解決の import をエラーにする（`py-int` / `cpp` / `cs` / `js-proc` / `py` の同梱スタブの変換失敗）。エラー文で型の出所の置き方を案内する。落ちる例題は型の出所を足すか、エラー例題（`_error`）に回す | 1-1 | 中 |
| **2-1** | import の処理がファイルに触る所（存在・ディレクトリか・テキスト・バイナリ・`ar_config.json`）を 1 つの抽象（例: `ImportFs`）の後ろへ集める。CLI は `std::fs` の実装 | なし | 中 |
| **2-2** | wasm に載らない依存を切り出す: C/C++ のヘッダ解析（`interpreter/cpp_bridge` の解析部）・Rust crate の読み込み（`partial_compiler/rs_loader` の `scan_all_sigs` など）を、ファイルを解析するだけの部分と実行時の部分に分け、前者を拡張の wasm から使える場所へ移す | 2-1 | 中 |
| **3-1** | wasm からホストへファイルの読み込みを頼む関数（同期）と、解析するドキュメントのパスを渡す入口を足す。ホスト（`frontend.ts`）に実装する | 2-1 | 中 |
| **3-2** | 拡張のビルドで `.ar` / `.arc` の import に `parser/imports/` を使う。読み込んだモジュールを保持し（パスと更新日時）、ホストがファイルの変更を知らせたものだけ読み直す | 3-1 | 中 |
| **3-3** | 中身が届いた import については型検査器の拡張だけの分岐（1.1 の 3 つ）を外す。`compare_wasm_frontend.ps1` が import を含む例題でも CLI と同じ診断になることを確かめ、「wasm は少なくてよい」の例外を外す | 3-2 | 小 |
| **4-1** | 拡張で外部言語（`py` / `py-int` / `cpp` / `cs` / `js-proc` / `rs`）も `parser/imports/` で読む。`rs` は型の情報だけ（`cargo build` しない）。Python の場所はホストから | 2-2・3-2 | 中 |
| **5-1** | スタブの仕組みを撤去（`imports_editor.rs`・`stub_registry`・`--emit-stubs`・`stub_manifest`・`stubs.ts`・コマンド・設定、`enum_member_type_plan.md` 6-3 の `generate_editor_stub`）。`CLAUDE.md` の「唯一の例外」・スキル（`vscode-extension-dev` / `importation` / `codebase-map`）・VSIX を更新 | 3-3・4-1 | 中 |

## 4. ゲート

| タスク | 走らせるもの |
|---|---|
| 全タスク | `scan_examples` / `force_gate` / `compare_python_impl`（`test_build_gate` は前段で自動） |
| 1-2 | `compare_import_paths -A`・`compare_outputs -A`（import の振る舞いを変えるので） |
| 2-1・2-2 | `compare_import_paths -A`・`compare_outputs -A`（挙動不変の主張） |
| 3-*・4-1 | `compare_wasm_frontend`（3-3 以降は import を含む例題も一致すること）・デバッグランナーで import を含む例題 |
| 5-1 | `stale_doc_refs`・`generate-codebase-map`・VSIX |

## 5. 実施記録

### 1-1 測定（未解決の import をエラーにしたときに落ちるもの）

エラーにする実装を入れて全例題を走らせた（`25ca6ad` との比較）:

| 落ちたもの | 原因 | 対応 |
|---|---|---|
| `event_external_handler.ar` / `ffi_boundary_check*.ar`（3 本）/ `relative_import_langs.ar` | `import[py-int] pkg.mod` の `pkg` が `__init__` の無いディレクトリ（名前空間パッケージ）。判定がディレクトリを見ていなかった（**実装の誤り**） | ディレクトリがあれば解決済み（空のパッケージ）とする |
| `js_proc_test.ar` / `js_proc_async_test.ar` | `import[js-proc] path`（Node.js 組み込み）に `.ars` が無い。`out_debug.analysis` は拡張から削除済みの JS で、**以前から実行時に `AttributeError`** で失敗していた | `path.ars` と、代わりの `js_text.js` / `js_text.ars` を置いた（2 本とも最後まで動くようになった） |
| `cargo test` の py-int のテスト 8 件 | テストの補助関数（`run_py_get`）が、Python の検索先を構文解析の後で実行時にだけ足していた | 構文解析の起点を `examples/interop/test_modules` にする（`prepare_at`） |

### 1-2 実装

- `py-int`: 型の出所（`.pyi` / `.py` / 同梱スタブ / 名前空間パッケージのディレクトリ）が無ければエラー。
  同梱スタブの変換失敗もエラー（以前は警告を出して行単位の抽出へ落ちていた）。⚠ 利用者の `.pyi` の
  変換失敗は従来どおり行単位の抽出で補う（見つかっているので「未解決」ではない）
- `cpp-*`: ヘッダが読めなければエラー
- `cs-*`: DLL が無い・メタデータが読めなければエラー
- `js-proc`: `.ars` が無い・読めない・構文解析できなければエラー
- エラー例題: `examples/interop/unresolved_{js_proc,py_int,cs_dll,cpp_header}_error.ar`
- ⚠ **`compare_wasm_frontend` はこの差を見ていない**。拡張はまだ import 先を読まない（エラーを出さない）が、
  拡張側の診断が空のファイルは CLI を走らせずに「一致」と数える作りなので、4 本とも「一致」になる。
  フェーズ 3・4 で拡張が import 先を読むようになれば、同じエラーが出る
