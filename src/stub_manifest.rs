// stub_manifest.rs — エディタ向け型スタブの**契約**（鍵の形・置き場・マニフェスト）。
//
// # なぜ crate 直下なのか
//
// この契約は 3 者が共有する:
//   - `arrow.exe --emit-stubs`（生成側。`src/main.rs`）
//   - `parser::stub_registry`（消費側。`editor` feature・wasm に載る）
//   - VS Code 拡張（読み手。マニフェストの `key` をそのまま `ar_set_stub` へ渡す）
// 置き場所が `src/` 直下なのは `crates/arrow-frontend` が `#[path]` で取り込むため
// （`py_stubs.rs` と同じ理由）。
//
// # ⚠ ホストに鍵を組み立てさせない
//
// DLL / ヘッダの探索順は言語側の規則（`source_dir` → `root_dir` → `ar_config.json`）で、
// それを TypeScript に再実装させると「拡張だけ解釈がずれる」という、`analysis.ts` を
// 捨てたときの問題がそのまま戻る。⇒ **鍵は [`stub_key`] だけが作り**、拡張は
// マニフェストに書かれた文字列をそのまま運ぶ。
//
// # ⚠ 鮮度は**利用者の明示操作**で保つ（現状の割り切り）
//
// マニフェストは `generatedAt` しか持たない。「この DLL は更新されたからスタブが古い」
// という判定は**できない**: 解決した DLL / アセンブリのパスが AST に残らないため
// （cpp だけは `module` がヘッダパスになるが、cs / py は残らない）。
// ⇒ 再生成は拡張の明示コマンドのみで、自動では走らせない。
// 個々の入力の mtime まで見るには、各ローダに「解決したパス」を報告させる必要がある。

use serde_json::json;

/// 生成物を置くディレクトリ名（ソースと同じ階層に作る）。`.gitignore` 向けに 1 本化してある。
pub const STUB_DIR: &str = ".arrow-stubs";

/// マニフェストのファイル名。
pub const MANIFEST_NAME: &str = "manifest.json";

/// スタブ表の鍵。**これが唯一の定義**。
///
/// ⚠ `lang` を含めること。`import[cs-dll] Foo` と `import[cpp-dll] Foo` は別物で、
/// 別のスタブを持つ。
pub fn stub_key(lang: &str, module: &[String]) -> String {
    format!("{lang}:{}", module.join("."))
}

/// 鍵から `.ars` のファイル名を作る。パス区切りや `:` を含まない形に落とす。
///
/// ⚠ 可逆である必要はない（引き当てはマニフェスト経由）。**衝突しない**ことだけが要件。
/// `:` はモジュールパスに現れないので、`__` への置換で 1 対 1 が保たれる。
pub fn stub_file_name(key: &str) -> String {
    format!("{}.ars", key.replace(':', "__"))
}

/// マニフェスト 1 件分。
pub struct StubEntry {
    pub key: String,
    pub lang: String,
    pub module: String,
    /// `STUB_DIR` からの相対ファイル名。
    pub file: String,
    /// スタブに載った最上位宣言の数（生成が空振りしていないかの目安）。
    pub decls: usize,
}

/// マニフェストの JSON テキストを組み立てる。
///
/// ⚠ 形式を変えたら拡張側（`vscode-extension/src/`）の読み手も直すこと。
/// 変換を挟まない素直な形にしてあるのは、そこを「判断をしない層」に保つため。
pub fn render_manifest(entries: &[StubEntry], generated_at: &str) -> String {
    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            json!({
                "key": e.key,
                "lang": e.lang,
                "module": e.module,
                "file": e.file,
                "decls": e.decls,
            })
        })
        .collect();
    serde_json::to_string_pretty(&json!({
        "version": 1,
        "generatedAt": generated_at,
        "stubs": items,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}
