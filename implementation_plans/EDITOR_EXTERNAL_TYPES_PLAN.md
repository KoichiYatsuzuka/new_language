# EDITOR_EXTERNAL_TYPES_PLAN.md — エディタで外部由来（C++ / C# / py）の型を認識させる（2026-09-19 起票）

**きっかけ**: 「実コードで外部由来（C++, C# など）のライブラリの型が認識されない」。
調査の結果、**不具合はエディタ（VS Code 拡張）側に限定**され、`arrow.exe` の型検査は正常だった。
原因は独立した 2 つ（§1-b の索引欠落と §1-c の設計上の帰結）で、本書はその是正計画（#1〜#7）を扱う。

> ## ✍️ 記載規約（`implementation_logs/FUTURE_FEATURE.md` に準拠）
>
> 1. **1 タスク = 意義 / 前提 / 手法 / 留意点 / 検証 / 参照** を必ず書く。
> 2. **外部にある情報は複製しない。** パスと「そこに何があるか」の 1 行だけを書く。
> 3. 着手したら末尾の状態表を更新する。実測値・経緯は実装ログへ、ここにはリンクだけ。

## 外部参照（ここには複製しない）

| 参照先 | そこにあるもの |
|---|---|
| skill `vscode-extension-dev` | 拡張の機能追加手順と **VSIX 梱包（`make-vsix.ps1`）** |
| skill `vscode-debug-runner` | `run_debug.js` / `stress.js` の使い方と再ビルド範囲 |
| skill `importation` | `import[lang]` の全タグ仕様と `src/parser/imports/` の実装 |
| skill `partial-compile` | `--compile` の全体像と `.arc`/`.ars` 形式 |
| skill `type-checking` | `InferredType`・推論規則・全 `TypeErrorKind` |
| skill `language-dev-principles` | ゲートの選び方、「挙動不変」を主張する前にやること、計画文書の規約 |
| [`src/parser/imports_editor.rs`](../src/parser/imports_editor.rs) 冒頭 doc | **なぜエディタ版は import 先を読まないのか**とその代償 |
| [`src/parser/editor_index.rs`](../src/parser/editor_index.rs) 冒頭 doc | AST を変えずに副次テーブルを持つ設計判断 |
| [`crates/arrow-frontend/src/analyze.rs`](../crates/arrow-frontend/src/analyze.rs) 冒頭 doc | 解析 JSON の全キーと供給元（**ここは判断をしない層**） |
| [`crates/arrow-frontend/src/wasm.rs`](../crates/arrow-frontend/src/wasm.rs) 冒頭 doc | wasm ABI（`wasm-bindgen` を使わない理由と呼び出し手順） |
| [`implementation_logs/FUTURE_FEATURE.md`](../implementation_logs/FUTURE_FEATURE.md) #17-b / #19 | `.ars` スタブ整備の既存レーン。**本計画はその供給先をエディタへ広げるもの** |

---

## 1. 調査で確定した事実

着手時に再調査しなくて済むよう、**根拠となる位置**だけを残す。

### 1-a. CLI 側は正常。不具合はエディタ限定

`d:\repository\TeX_editor\main.ar` と同じ import 構成でプローブを走らせると、
`arrow.exe` は外部由来の戻り値型を正しく持っている:

```
import[cpp-dll] SakuraCore.SakuraEdit as sakura → let n: str = sakura.get_line_count(0)
  → StaticTypeError  'n' is declared 'str' but initialized with 'int'
import[cs-proc] WpfShell as wpf → let a: str = wpf.HostWindow.get_pane_hwnd("main")
  → StaticTypeError  'a' is declared 'str' but initialized with 'int'
```

同じファイルを拡張の解析器（`run_debug.js`）に食わせると `let ed`（型なし）・`json.` 補完 0 件。
⇒ **直す対象は `src/type_check/` ではなく、エディタ経路への型情報の供給**である。

### 1-b. cpp-dll / cpp-lib の別名だけがエディタ索引に載らない（純粋なバグ）

[`src/parser/imports_editor.rs:140`](../src/parser/imports_editor.rs#L140) `parse_cpp_import_syntax` にだけ
`#[cfg(feature = "editor")]` の `note_def_at` / `note_signature` が無い
（同ファイルの `parse_import_stmt:29` / `parse_from_import_stmt:86` にはある）。

結果、cpp 系の別名だけが他タグと違う欠落の仕方をする:

| import タグ | 索引に載るか | 型が付くか |
|---|---|---|
| `cs-dll` / `cs-proc` / `py-int` / `rs` / `ar` | **載る**（hover・定義ジャンプ・補完・module 着色が効く） | 付かない（§1-c） |
| `cpp-dll` / `cpp-lib` | **載らない**（hover なし・アウトライン不在・スコープ補完に出ない・着色されない） | 付かない |

リポジトリ内の再現: [`examples/interop/cpp_struct_ptr.ar`](../examples/interop/cpp_struct_ptr.ar) の別名 `vm` が
アウトラインにもスコープ補完にも出ない。**#1 はこれ 1 点を直す。**

### 1-c. 外部の型がエディタへ届く経路が存在しない（設計上の帰結）

`editor` feature（＝拡張の wasm ビルド）は [`src/parser/imports/`](../src/parser/imports/) を
`imports_editor.rs` に差し替え、import 文を**構文だけ解釈して `body: vec![]`** を返す。
fs・プロセス・DLL に触れないため（wasm32 に載せる制約 ＋ 1 打鍵ごとの解析で実測 7.7 秒）。

その空 body が型検査に届くと、
[`src/type_check/stmt/check.rs:571`](../src/type_check/stmt/check.rs#L571) の `editor_stub_body(body)` 分岐で
別名に `InferredType::Unresolved` が束縛される。`Unresolved` は属性アクセスの match で `_ => {}` に落ちるので、
**エラーは出ないが型も付かない**。これは偽陽性を出さないための意図的な倒し方であり、
C++/C# に限らず `py-int` も `.ar` も同じ扱いになる。

⇒ **#2〜#4 は「fs に触れない」を保ったまま、ホスト（Node）が読んだスタブを wasm へ渡す経路を作る。**

### 1-d. `.` 補完は名前空間を受け手にできない（body を埋めても直らない）

[`crates/arrow-frontend/src/analyze.rs:173`](../crates/arrow-frontend/src/analyze.rs#L173) `collect_members` は
`ClassDef` / `TraitDef` / `ProtocolDef` / `EnumDef` だけを拾い、**`Stmt::Import` を見ていない**（末尾 `_ => {}`）。
TS 側の [`wasm_providers.ts:623`](../vscode-extension/src/wasm_providers.ts#L623) `known()` も
`analysis.members[型名]` しか引かない。

⇒ body を埋めるだけでは hover / inlay に型は出るが `sakura.` の補完は空のまま。**#5 が要る。**
⚠ これは外部言語に限らない既存の穴で、`.ar` モジュールの `mod.` 補完も同様に空。

### 1-e. 素材はすでに全部ある

| 要るもの | 既存の供給元 |
|---|---|
| `Vec<Stmt>` → `.ars` テキスト | [`src/partial_compiler/stub_gen.rs:9`](../src/partial_compiler/stub_gen.rs#L9) `generate_stub`（**AST 汎用**。`--compile` が使用） |
| .NET メタデータ → `.ars` | [`src/parser/cs_assembly/stub_gen.rs:19`](../src/parser/cs_assembly/stub_gen.rs#L19) `render_cs_ars_text`（`--compile-cs` が使用） |
| C ヘッダ → `Stmt` スタブ | [`src/parser/imports/cpp.rs:19`](../src/parser/imports/cpp.rs#L19) `parse_cpp_import`（**パース時はヘッダ読みのみ。シムのビルドは実行時**） |
| `.ars` テキストを解析して取り込む前例 | [`wasm_providers.ts:150`](../vscode-extension/src/wasm_providers.ts#L150) `loadPrelude`（`builtins.ars`） |

⚠ `loadPrelude` は**グローバル 1 枚**（`scope === 0` の宣言を組み込みとして混ぜる）。
名前空間ごとに分ける造りではないので、**そのままでは `sakura.foo()` に使えない**。

⚠ `generate_stub` は**docstring を落とす**（`src/partial_compiler/stub_gen.rs` に
`Expr::Str` の分岐が無い）。`render_cs_ars_text` が XML doc を `"""…"""` として
埋めているのと非対称なので、C# を通すときに hover のドキュメントが消える。→ D-5。

### 1-f. py の同梱スタブは、エディタから**届く位置に無い**だけ

[`src/py_stubs.rs`](../src/py_stubs.rs)（#19、コミット `3759dba`）は `time` / `math` の `.pyi` を
`include_str!` で**バイナリに埋め込んで**いる。fs にも syscall にも触れない形なので、
**wasm でもそのまま使える**。

にもかかわらずエディタには届いていない。理由は 2 つ:

1. [`crates/arrow-frontend/src/lib.rs`](../crates/arrow-frontend/src/lib.rs) の `#[path]` 一覧に
   `py_stubs` が**無い**（`ast` / `token` / `decl_names` / `expr_walk` / `stmt_walk` /
   `lexer` / `parser` / `type_check` の 8 本だけ）。
2. 引き手が [`src/parser/imports/py_modules.rs:89`](../src/parser/imports/py_modules.rs#L89) にあり、
   これは**エディタビルドでは差し替えられて消える**モジュール。

⚠ `py_stubs.rs` の冒頭 doc は「置き場所が `src/` 直下なのは
`crates/arrow-frontend`（VS Code 拡張の wasm）が取り込むため」と**すでに書いている**。
配線だけが残っている状態。⇒ **#7 は #2〜#4 の機構を一切使わずに py を一部救う。**

⚠⚠ **`.pyi` は Python 構文なので Arrow パーサでは読めない**（起票時の #7 手法はここを
外していた）。CLI は `python_converter`（rustpython）で変換しているが、それを
`crates/arrow-frontend` に足すのは禁じ手（同 `Cargo.toml` の「ネイティブ依存を足さないこと」）。
使えるのは**行ベースの近似抽出器**のほうで、こちらは `rustpython` に依存しない。
ただし実体が `parser/imports/mod.rs` の中にあり、**そこは editor ビルドで丸ごと消える**。
⇒ 抽出器を共有モジュールへ出すのが #7 の実際の手法。

### 1-g. 既存のゲートがこの欠落を構造的に見逃す

- `compare_wasm_frontend.ps1` は**診断の一致**だけを見る。`Unresolved` も未登録の別名も診断を 1 件も出さないので、両者「エラー 0 件」で一致し緑になる。
- `stress.js` の hover/def misses は `provideDocumentSymbols` が返したシンボルを母集団にする。索引に載っていない別名は**probe されず** miss に数えられない。

⇒ **#6 で「索引に載るべき名前が載っているか」を見る網を足さないと、#1 は再発しても気づけない。**

### 1-h. 死んでいる設定がある

`vscode-extension/package.json` の `arrow.pythonLibraryPaths` は宣言だけで、
`vscode-extension/src/` のどこからも読まれていない（grep 0 件）。#4 で吸収するか削除する。

---

## 2. 設計判断（先に決めておくこと）

着手順を変えても揺らがないよう、**5 点だけ先に固定する**。

### D-1. スタブ由来の body でも `registry_incomplete` は `true` のままにする

[`src/type_check/mod.rs:222`](../src/type_check/mod.rs#L222) `has_unloaded_import` が false になると
[`src/type_check/stmt/resolve.rs:343`](../src/type_check/stmt/resolve.rs#L343) の
`check_ann_names_exist` が有効になり、**エディタが新しくエラーを出し始める**。
スタブは古くなりうる（§1-e の cs `.ars` は生成物）ので、
`compare_wasm_frontend.ps1` の不変条件「**wasm は少なく報告してよいが、多く報告してはならない**」を破る。

⇒ **本計画の変更は加算のみ**とする：型は付くようになるが、診断は 1 件も増えない。
そのために「読み込めた body」と「スタブ由来の body」を型検査が区別できる印が要る（#2 の手法）。

### D-2. 解決規則を TypeScript に置かない

DLL / ヘッダの探索順は `source_dir` → `root_dir` → `ar_config.json` の `csharp.lib_paths` …と
言語側の規則で決まっている（skill `importation`）。これを TS で再実装すると
「拡張だけ解釈がずれる」という、`analysis.ts` を捨てたときの問題がそのまま戻る。

⇒ **解決と生成は `arrow.exe` が行い（#3）、拡張はその出力（マニフェスト）を読むだけ**（#4）。

### D-3. 受け渡しの形は `.ars` テキスト（AST の別表現を作らない）

`.ars` は valid な Arrow なので、wasm 側は**同じ Parser で読み直せる**。
JSON 化した AST を第 2 の表現として持つと、AST に variant を足したとき
両方を直す必要が出て、`language-dev-principles` §2 の「同じ木を歩く walker は必ずずれる」に当たる。

### D-4. wasm は fs に触れないまま。スタブはホストが渡す

`ar_analyze` は現在ステートレス（ソース 1 本 → JSON 1 本）。
スタブ表という**状態**を持たせるので、ホスト側のキャッシュ無効化と対にして設計する（#2 の留意点）。

### D-5. 既に `.ars` を吐ける言語では、既存の生成器を優先する

`.ars` の作り手が 2 つある（§1-e）:

| 生成器 | 入力 | docstring |
|---|---|---|
| `cs_assembly::render_cs_ars_text` | .NET メタデータ ＋ XML doc | **埋める** |
| `partial_compiler::stub_gen::generate_stub` | 任意の `&[Stmt]` | **落とす** |

#3 が全タグを `generate_stub` で通すと、**C# だけ hover のドキュメントが消える**
（`--compile-cs` が出す `.ars` より劣化する）。py の `.pyi` も docstring を持つので同じ。

⇒ **言語ごとに最良の生成器を選ぶ**：C# は `render_cs_ars_text`、
それ以外は `generate_stub`。`--compile-cs` が出した `.ars` がすでにディスクにあるなら
**それを再生成せずそのまま使う**（マニフェストにパスを書くだけ）。
⚠ 将来 `generate_stub` に docstring を足して 1 本化するのは**別タスク**。
`--compile` の `.ars` 形式が変わるので、既存の読み手への波及を測ってからでないと畳めない。

---

## #1 — cpp import の別名をエディタ索引に登録する

- **意義**: §1-b の欠落を直す唯一のタスク。他の全タスクと独立に着手・出荷でき、
  cpp 系の別名が他タグと同じ土俵（hover・定義ジャンプ・スコープ補完・module 着色）に乗る。
  ⚠ これで**型は付かない**（型は #2〜#4）。
- **前提**: なし。
- **手法**: `imports_editor.rs` の `parse_cpp_import_syntax` に、`parse_import_stmt` と同じ体裁で
  `note_def_at` ＋ `note_signature` を足す。束縛名は `as` があればその別名、無ければ末尾セグメント。
  ⚠ **位置を取る順番が効く**：`prev_pos()` は「最後に読んだ識別子」なので、
  `as` を読む**前**にモジュール名の位置を控えておく（`parse_import_stmt` の `module_pos` と同じ理由）。
- **留意点**:
  - ⚠ 通常ビルド側（`imports/cpp.rs`）には索引が無い（`editor` feature 専用）ので、
    **直す場所は `imports_editor.rs` だけ**。両者を「揃える」対象ではない。
  - ⚠ 受理する構文は 1 文字も変えない。`imports_editor.rs` の doc が要求する不変条件
    （両者が同じ構文を受理する）に触る変更ではないことを明記してコミットする。
- **検証**: `run_debug.js examples/interop/cpp_struct_ptr.ar` で別名 `vm` が
  アウトライン・スコープ補完・hover に出ること。`./scripts/compare_wasm_frontend.ps1` が緑のまま
  （診断は 1 件も変わらないはず）。
- **参照**: [`src/parser/editor_hooks.rs`](../src/parser/editor_hooks.rs)（フックの書き方）／skill `vscode-debug-runner`。

---

## #2 — wasm にスタブ供給 ABI を足し、`imports_editor` がそれを見る

- **意義**: §1-c の経路を作る本体。これが入ると `let ed = sakura.create(host)` に型が付く
  （hover・inlay・`exprTypes` は body から自動で follow する）。
- **前提**: なし（#3 / #4 が無くても、テストからは直接叩ける）。
- **手法** ✅ **実装済（2026-09-19）**:
  1. `src/parser/stub_registry.rs`（`editor` 専用）にスタブ表を置いた。`thread_local!` の
     `HashMap<String, String>`。鍵の定義は `stub_key(lang, module)` **1 箇所だけ**で、
     解析中のファイル位置にも DLL / ヘッダの置き場所にも依存しない（D-2 の受け皿）。
  2. `crates/arrow-frontend/src/wasm.rs` に `ar_set_stub` / `ar_clear_stubs` /
     `ar_stub_count` を足した。既存の alloc/free の渡し方のままで、`wasm-bindgen` は不使用。
  3. `imports_editor.rs` に `editor_import_body(lang, module)` を置き、3 つの入口
     （`parse_import_stmt` / `parse_from_import_stmt` / `parse_cpp_import_syntax`）が
     すべてそこを通るようにした。優先順は **ホスト由来 → 同梱 py → 空**で、CLI と逆にならない。
     スタブ本文は**同じ Arrow パーサ**で読む（D-3）。`node_counter` は親と共有する。
  4. **D-1 の印は「editor では import が 1 つでもあれば不完全」**という形にした
     （`has_unloaded_import` に `#[cfg(feature = "editor")]` のアームを足す）。
     AST にもスタブ表にも印を持たせずに済み、しかも**正確**: editor の body は
     「空」か「スタブ由来」のどちらかで、実読み込みは決して起きない。
     import が無いファイルは従来どおり `false` なので検査範囲は狭まらない。
     ⚠ これは #7 が入れてしまった潜在的なずれの修正でもある（`import[py-int] math`
     だけのファイルで body が非空になり、editor だけ `check_ann_names_exist` が
     動き出していた）。
  5. 循環（スタブが自分自身を import する形）は `with_stub` の展開中セットで止める。
- **留意点**:
  - ⚠⚠ **`ar_analyze` がステートフルになる。** TS 側の解析キャッシュは
    `document.version` だけをキーにしている（`wasm_providers.ts` の `getAnalysis`）。
    スタブを入れ替えたら**キャッシュを捨てる**こと。忘れると「スタブを更新したのに古い型が出続ける」になる。
  - ⚠ スタブの解析に失敗しても**握り潰して空 body に倒す**。壊れた `.ars` でエディタが死ぬのは、
    直そうとしている問題より悪い。
  - ⚠ 入れ子 Parser を回すので**再入**に注意（スタブの中の import は解析しない＝空 body のまま）。
  - ⚠ `language-dev-principles` §1「注釈は最適化ヒント」と同じ性質：
    **スタブが無くても正しく動く**こと。スタブは「あれば型が付く」だけ。
- **検証** ✅: `crates/arrow-frontend/tests/stubs.rs`（5 件・全通過）— スタブ無しで型が
  付かないこと／積むと戻り値型が届くこと／**積んでも診断が増えないこと**（D-1）／
  自己参照スタブが停止すること／壊れた `.ars` が空 body に倒れること。
  `compare_wasm_frontend` 368/368 一致・INVENTED 0・**wasm fewer 0**／`stress.js` threw 0・
  misses 0／`scan_examples`／`force_gate` fall back 0／`compare_python_impl` 101/101／
  `stale_doc_refs` OK。
  ⚠ `crates/arrow-frontend` の `type_refs::skipped_generic_arguments` は**本変更と無関係に
  HEAD で落ちている**（`type Box does not take type arguments`）。stash して確認済み。
- **参照**: [`crates/arrow-frontend/src/wasm.rs`](../crates/arrow-frontend/src/wasm.rs)（ABI の作法）／
  [`src/parser/imports_editor.rs`](../src/parser/imports_editor.rs)（不変条件）。

---

## #3 — `arrow.exe --emit-stubs <file.ar>` を足す

- **意義**: D-2 の受け皿。**言語側の解決規則を 1 箇所に保ったまま**、拡張が読める形でスタブを吐く。
  cs-dll の `.ars`（`--compile-cs`）と `.ar` の `.ars`（`--compile`）に続く 3 本目で、既存の素材で足りる（§1-e）。
- **前提**: なし。
- **手法** ✅ **実装済（2026-09-20）**:
  1. `--emit-stubs <file.ar>` を追加（`--compile` と同じ体裁）。パースだけで実行も型検査もしない。
  2. 契約（鍵の形・置き場・マニフェスト）を `src/stub_manifest.rs` に置き、
     **`--emit-stubs` と `stub_registry` が同じ定義を使う**（`crates/arrow-frontend` も `#[path]` で取り込む）。
     出力は `<source_dir>/.arrow-stubs/`。
  3. 書き出す前に `arrow_writable()` で **Arrow として書ける形**に整える。外部言語のスタブ AST は
     Arrow の規則を満たしていないため（下の「実測で出た 4 件」）。
  4. ⚠ **body が非空でもテキストが空になることがある**（`generate_stub` が `Stmt::Let` を落とす＝py の形）。
     空の `.ars` は「読めたがメンバ 0」と誤読されるので**書かずにマニフェストからも落とす**。
     件数も `body.len()` ではなく**生成後のテキスト**から数える。

  **実測で出た 4 件**（どれも「生成した `.ars` が読み返せない」形で、`.ars` を再パースする
  経路が無かったため誰も踏んでいなかった）:

  | 症状 | 原因 | 直し方 |
  |---|---|---|
  | `expected :, got ->` | `class X->X:` を書いていた。`->X` は**旧・正規表現版拡張**にコンストラクタの戻り値型を伝える印で Arrow の構文ではない | 両生成器（`partial_compiler` / `cs_assembly`）から落とした。現行拡張に読み手は 1 つも無い |
  | `only traits are allowed as bases` | C# はインタフェースをクラス基底に並べるが Arrow は trait しか許さない | `--emit-stubs` 側で**同じスタブが trait 宣言していない基底を落とす**（読み手はどのみち辿れない） |
  | `expected :, got .` | .NET の**他アセンブリ参照型**だけが完全修飾名（`System.Windows.Controls.Orientation`） | `--emit-stubs` 側で最終セグメントへ平坦化。どちらの綴りでも不透明な名前なので意味は落ちない |
  | `'WpfApp.show' takes 0 argument(s) but 1 were given` | `generate_stub` が **`static` を落として**いた。`self` を取らないメソッドがインスタンスメソッドとして読み直され、先頭仮引数が `self` の席に吸収されて**引数の数が 1 つずれる** | `fn_stub` に `is_static` を通し `static fn` を書く |

  ⚠ 下 2 件は `--emit-stubs` 側だけで直した（`cs_assembly` の型名は CLI の型検査が見ているため）。
  上 2 件は生成器そのものの誤りなので `--compile` / `--compile-cs` の出力も直っている。
- **留意点**:
  - ⚠ **cpp はヘッダを読むだけ**（シムの C++ ビルドは実行時なので走らない）。
    **py はサブプロセスを起動する**ので、このコマンドは秒オーダーになりうる。
    ⇒ 「打鍵ごと」ではなく「明示的な契機」でしか呼ばれない設計にする（#4）。
  - ⚠ **キャッシュの鮮度**。`import[cs-dll]` の `.ars` で既知の問題（FUTURE_FEATURE #17-b の留意点、
    および cpp ブリッジの `ar_<mod>.dll` が絶対パスを埋め込む件）と同じ罠。
    マニフェストに**入力側（DLL / ヘッダ）の mtime とサイズ**を書き、古ければ使わない。
  - ⚠ 見つからない DLL / ヘッダは CLI 側がすでに警告して空 body に倒す。
    **その場合はスタブを書かない**（空の `.ars` を置くと「読めた」と誤認する）。
  - ⚠ `generate_stub` は `--compile` の出力形式でもある。**ここで形式を変えない**
    （変えると `.arc`/`.ars` の既存の読み手に波及する）。
- **検証** ✅: `d:epository\TeX_editor\main.ar`（cpp-dll / cs-dll / cs-proc / py-int / `.ar` を
  すべて含む実コード）で e2e 確認 — 生成した 4 件が**全て構文として通り**、
  それをレジストリに積むと `let ed`（cpp-dll）と `let host`（cs-proc の static メソッド）が
  ともに `int` に解決し、**診断は元の 2 件（既存の `mustbe` 警告）のまま増えない**（D-1）。
  `scan_examples`／`force_gate` fall back 0／`compare_python_impl` 101/101／
  `compare_wasm_frontend` 368/368・INVENTED 0／`stale_doc_refs` OK／codebase-map 再生成。
  **直前コミットからビルドした基準バイナリとの実 A/B**: `compare_import_paths` 13/13 同一、
  `compare_outputs` 298/298 同一（AST のフィールド改名が実行時に影響していないことの確認）。
  ⚠ 不動点テストは**未実装**。ルートの `cargo test` が HEAD 側の破損でビルドできず、
  `crates/arrow-frontend` からは `partial_compiler` に手が届かないため。
- **参照**: skill `partial-compile`（`.ars` 形式）／skill `importation`（探索順）／
  [`src/partial_compiler/module_compiler.rs`](../src/partial_compiler/module_compiler.rs)（`.ars` の書き出し前例）。

---

## #4 — 拡張ホストがスタブを読み、wasm へ流し込む

- **意義**: #2 と #3 をつないで、**実際にエディタ上で型が出る**ようにする最後の 1 本。
- **前提**: **#2 ＋ #3**。
- **手法**:
  1. `.ar` を開いた／保存したときに、そのファイルのディレクトリからマニフェストを探して読む。
  2. マニフェストの各エントリの `.ars` を読み、`ar_set_stub(key, text)` で wasm に渡す。
     渡したら**解析キャッシュを捨てる**（#2 の留意点）。
  3. マニフェストが無い／古いときの再生成契機を決める。
     **推奨**: 明示コマンド `Arrow: Refresh External Stubs` を 1 本足し、
     自動再生成は**しない**（py の起動コストと、任意のプロセス起動を暗黙にやらないため）。
     マニフェストが無いときは現状どおり（型が付かないだけ）。
  4. `arrow.exe` の場所は設定で指定させる（`arrow.executablePath`）。未設定なら機能を黙って無効化する。
  5. §1-h の死んだ設定 `arrow.pythonLibraryPaths` を、この機構に吸収するか削除する。
- **留意点**:
  - ⚠ **同名別ファイル**。`main.ar` と別ディレクトリの `main.ar` でスタブ表が混ざらないよう、
    スタブ表は**ドキュメント単位で入れ替える**（`ar_clear_stubs` → 必要分を `ar_set_stub`）。
  - ⚠ **`run_debug.js` / `stress.js` も同じ経路を通す**こと。ここだけ素通りすると
    「VS Code では出るが harness では出ない」が起きて、以後の調査が全部狂う。
  - ⚠ 拡張を触ったので **VSIX の再梱包が要る**（`make-vsix.ps1`。リポジトリ規約）。
  - ⚠ 外部プロセス起動を拡張に入れるのは**初**。既定 off・明示コマンドのみ、を崩さない。
- **検証**: `d:\repository\TeX_editor\main.ar` で
  `let ed` に `int` が出る・`sakura.` の補完が出る・`wpf.HostWindow.` が引ける、を目視 ＋ `run_debug.js`。
  `./scripts/compare_wasm_frontend.ps1`（スタブを置いた状態でも**診断が増えない**こと ＝ D-1 の実地確認）。
- **参照**: skill `vscode-extension-dev`（設定・コマンド・VSIX）／skill `vscode-debug-runner`。

---

## #5 — 名前空間を `.` 補完の受け手にする

- **意義**: §1-d の穴。#2 まで入れても `sakura.` / `json.` / `mod.` の補完は空のまま。
  **外部言語に限らず `.ar` モジュールにも効く**ので、単独でも価値がある。
- **前提**: 実益を出すには **#2**（body が空だと出すものが無い）。コード自体は独立に書ける。
- **手法**:
  1. `analyze.rs` の `collect_members` に `Stmt::Import` / `Stmt::FromImport` を足し、
     **別名をキーにしたメンバ表**を `members` に載せる（クラス名と衝突しない鍵の付け方を決める）。
  2. TS 側 `receiverTypeAt` の `known()` が、受け手名をその表でも引けるようにする。
  3. `signature help` / `hover` も同じ表を引く（`membersOf` 経由なので 1 箇所で済む）。
- **留意点**:
  - ⚠ `collect_members` は末尾 `_ => {}` の walker。`language-dev-principles` §2 のとおり
    **variant を足しても何も強制しない**。ここを触るなら、
    「**モジュール本体を持つ文**」の列挙を 1 箇所に寄せて exhaustive にできないか先に検討する。
  - ⚠ `members` はクラス／トレイト名の平坦な辞書。**別名がクラス名と衝突しうる**
    （`import[...] Foo as Foo` と `class Foo`）。衝突時にどちらを勝たせるか決めてコメントに書く。
  - ⚠ PyNamespace（未知メンバが `Any`）と Namespace（未知メンバが `Unresolved`）で
    補完の出し方を変えない。補完は「知っている名前を出す」だけで、
    **知らない名前について何も主張しない**。
- **検証**: `run_debug.js` で `sakura.` / `json.` / `.ar` モジュールの `.` 補完が出ること。
  `stress.js` の `threw` が 0、`hover misses` / `def misses` が 0 のままであること。
- **参照**: [`crates/arrow-frontend/src/analyze.rs`](../crates/arrow-frontend/src/analyze.rs)（`members` の作り）／
  [`vscode-extension/src/wasm_providers.ts`](../vscode-extension/src/wasm_providers.ts)（`receiverTypeAt` / `membersOf`）。

---

## #6 — 「索引に載るべき名前が載っているか」の網を足す

- **意義**: §1-g のとおり、#1 の欠落は `imports_editor.rs` の新設（2026-08-28, `93eca1d`）から
  **全ゲート緑のまま残り続けた**。同じ形の再発（import タグを足したときに索引フックだけ忘れる）を止める。
- **前提**: **#1**（直した状態を基準にする）。
- **手法**:
  1. `stress.js` に「**`import` 文が束縛する名前は必ずシンボル表にある**」という検査を足す。
     母集団をシンボル表から取っている限りこの欠落は見えないので、
     **ソース側（トークン列／import 文）から期待値を作って突き合わせる**のが要点。
  2. `import[lang]` のタグ一覧に対して、**タグごとに 1 本**の最小例題で
     「別名が索引に載る」ことを固定する。
- **留意点**:
  - ⚠ `language-dev-principles` §2 の「強制には守れない範囲がある」そのもの。
    **列挙し忘れは人間が読むしかない**ので、期待値は**ソースから機械生成**して突き合わせる。
  - ⚠ 「登録を外したら落ちるか」を 1 件ずつ実測する。落ちないならテストが効いていない。
  - ⚠ `examples/archived/` と `examples/practical_examples/` は 9 カテゴリのゲート対象外
    （CLAUDE.md の警告）。母集団をどちらにするか明示して書く。
- **検証**: #1 を意図的に revert して**落ちること**を確認する。
- **参照**: skill `vscode-debug-runner`（`stress.js` のカウンタの意味）／skill `language-dev-principles` §2・§3。

---

## #7 — 同梱 py スタブ（`time` / `math`）をエディタへ届かせる

- **意義**: §1-f のとおり、**スタブ本体はもう存在し、wasm でも使える形（`include_str!`）で
  埋め込まれている**。届いていないのは配線だけ。
  ⇒ **#2〜#4 の機構を一切使わずに** py の一部で型が付く、この計画で最も安い 1 件。
  `let s: str = math.sqrt(2.0)` が CLI では静的エラー・エディタでは無反応、というずれが消える。
- **前提**: なし。#1 と同様、単独で着手・出荷できる。
- **手法** ✅ **実装済（2026-09-19）**:
  1. 行ベース抽出器を `parser/imports/mod.rs` から `parser/py_stub_extract.rs` へ出し、
     **両ビルドから見える**ようにした（`imports/` は editor で丸ごと差し替わるため）。
     `imports/mod.rs` は名前だけ再エクスポートするので、CLI 側の呼び出しは無変更。
  2. `crates/arrow-frontend/src/lib.rs` の `#[path]` 一覧に `py_stubs` を足した。
  3. `imports_editor.rs` に `bundled_py_stub_body(lang, module)` を足し、`py` / `py-int` の
     とき同梱スタブを抽出器にかけて body にした。`parse_import_stmt` と
     `parse_from_import_stmt` の両方から呼ぶ。
  4. ⚠ **抽出器のバグを 1 件直した**（これが無いと型が付かない）。`.pyi` の標準形
     `def f(x: float) -> float: ...` で、戻り値注釈を行末の `:` だけで切っていたため
     `"float: ..."` になり、`py_type_to_arrow` の catch-all で `Any` に落ちていた。
     ⇒ **深さ 0 の `:` まで**を型として切り出す形に変更（`-> dict[str, int]: ...` 対応）。
     CLI の `.py` 経路も同じ関数を通るので、**そちらも直る**（型が増える方向のみ）。
  5. #2 が入るまでは、これが唯一のスタブ供給源になる。
     **#2 を入れるときは、この経路をスタブ表の一段として吸収する**（供給源を 2 本に増やさない）。
- **留意点**:
  - ⚠⚠ **利用者のファイルが常に勝つ**という優先順（`py_modules.rs:85` のコメント）を壊さない。
    CLI は「検索ディレクトリを全部見て空振りしたときだけ」同梱スタブを引く。
    エディタは検索ができないので**同梱スタブしか引けない**が、#2 でホストからスタブが来たら
    **そちらを優先する**（CLI と順序が逆にならないようにする）。
  - **D-1 の印は #7 では不要**と判断した（起票時は「必要」と書いていた）。エディタの body は
    **CLI の body の部分集合にしかならない**（CLI = `python_converter` ＋ 抽出器の補完、
    エディタ = 抽出器のみ）。`has_unloaded_import` が false に倒れる条件も
    「全 import の body が非空」で、エディタは CLI より空 body が多いので**先に倒れることはない**。
    ⇒ ホストから任意のスタブが来る **#2 で初めて印が要る**。
  - ⚠ 埋まるのは **`time` / `math` の 2 つだけ**。他の py モジュールは従来どおり型が付かない。
    「py が直った」と言わないこと。
  - ⚠ 対象はトップレベル 1 要素のモジュールのみ（`builtin_stub` の doc）。`os.path` は入らない。
- **検証** ✅: `run_debug.js` で `math.sqrt(2.0)` / `time.time()` が `float`（変更前は型なし）。
  `compare_wasm_frontend`（368/368 一致・INVENTED 0）／`stress.js`（threw 0・misses 0）／
  `scan_examples`／`force_gate`（fall back 0）／`compare_python_impl`（101/101 一致）／
  `stale_doc_refs`（OK）。⚠ `cargo test` は**この変更と無関係な HEAD 側の破損**で
  ビルドできず未実行（`src/vm/compiler/mod.rs:430` が `Op::DictMerge` /
  `SeqExtend` / `SeqFinish` を網羅していない）。
- **参照**: [`src/py_stubs.rs`](../src/py_stubs.rs) 冒頭 doc（なぜ `include_str!` か・なぜ `src/` 直下か）／
  [`implementation_logs/FUTURE_FEATURE.md`](../implementation_logs/FUTURE_FEATURE.md) #19（スタブを増やす側のレーン）。

---

## 状態表

| # | タスク | 前提 | 状態 |
|---|---|---|---|
| #1 | cpp import の別名をエディタ索引に登録 | — | ✅ **実装済（2026-09-19）** |
| #2 | wasm にスタブ供給 ABI ＋ `imports_editor` が引く | — | ✅ **実装済（2026-09-19）** |
| #3 | `arrow.exe --emit-stubs` | — | ✅ **実装済（2026-09-20）** |
| #4 | 拡張ホストがスタブを読み wasm へ流す | #2 ＋ #3 | 未着手 |
| #5 | 名前空間を `.` 補完の受け手にする | （実益は #2） | 未着手 |
| #6 | 索引欠落を検出する網 | #1 | 未着手 |
| #7 | 同梱 py スタブ（`time` / `math`）をエディタへ届かせる | — | ✅ **実装済（2026-09-19）** |

**#1 / #2 / #3 / #7 は完了。次は #4（前提 #2 ＋ #3 とも充足済み）。**

### タグ別に何が効くようになるか

⚠ **#2〜#5 はタグ非依存**（import 文の形しか見ない）。cpp に見えるのは #1 だけ、
py に見えるのは #7 だけで、**本体の経路は C# も Python も同じ 1 本**。

| import タグ | 索引に載る | 型が付く | `.` 補完 |
|---|---|---|---|
| `cpp-dll` / `cpp-lib` | **#1**（現状は載らない） | #2〜#4 | #5 |
| `cs-dll` / `cs-proc` | すでに載っている | #2〜#4（`.ars` は D-5 で既存生成器を使う） | #5 |
| `py-int` / `py` | すでに載っている | **#7**（`time` / `math` のみ・単独で効く）＋ #2〜#4（残り） | #5 |
| `rs` / `js-proc` / `.ar` | すでに載っている | #2〜#4 | #5 |

### 推奨する着手順

1. ~~**#1 ＋ #7**~~ ✅ 完了（2026-09-19）。どちらも単独で出荷でき、症状の一部が即なくなった。
2. **#6** — #1 を固定してから土台を触る（#2 以降は同じ層を触るため）。
3. **#3 → #2 → #4** — #3 の出力形式が #2 のキーの形を決めるので、生成側を先に固定する。
   #2 のときに #7 の経路をスタブ表の一段として吸収する（供給源を 2 本に増やさない）。
4. **#5** — 最後。#2 が入るまで出すものが無い。

### やらないと決めたこと

- **`loadPrelude` を名前空間対応に拡張する**（§1-e）。
  prelude はグローバル 1 枚という前提で `visibleSymbols` に連結されており、
  ここに名前空間を持ち込むと組み込み名の解決規則まで揺れる。**別の表**（#5）で持つ。
- **拡張側で DLL / ヘッダを直接読む。** D-2 のとおり、解決規則が TS に漏れる。
- **`registry_incomplete` をスタブありで false にする。** D-1 のとおり、
  ゲートの不変条件（wasm は多く報告しない）を破る。将来やるなら**独立のタスクとして起票**し、
  スタブの鮮度保証と対で設計すること。
