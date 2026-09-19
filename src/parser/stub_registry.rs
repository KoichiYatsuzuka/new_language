// parser/stub_registry.rs — `editor` feature 専用: ホストが渡した型スタブ（`.ars` テキスト）の表。
//
// # なぜ必要か
//
// エディタ版の import 解析（[`super::imports_editor`]）は fs・プロセス・DLL に一切触れない。
// wasm32 に載せるための制約であり、1 打鍵ごとの解析で DLL をロードするわけにもいかない。
// その代償として **import 先のメンバ型が一切分からない**（`imports_editor.rs` 冒頭 doc）。
//
// 解決は「wasm に fs を与える」ではなく「**fs を持っているホスト（Node）に読ませて渡す**」。
// ここはその受け皿で、wasm 側は最後まで fs を知らない。
//
// # 鍵は import 文だけから作る
//
// [`stub_key`] が唯一の定義。`(lang, モジュールパス)` から作るので、
// **解析中のファイルの位置にも、DLL / ヘッダの実際の置き場所にも依存しない**。
// 探索順（`source_dir` → `root_dir` → `ar_config.json`）は言語側の規則であって、
// それをホストや TypeScript に再実装させないための形（計画 D-2）。
// ⇒ ホストは `arrow.exe` が出したマニフェストの鍵をそのまま渡すだけでよい。
//
// # ⚠ 状態を持つので、ホストは入れ替えと解析を対にすること
//
// `ar_analyze` はもともとステートレス（ソース 1 本 → JSON 1 本）だった。ここが唯一の
// 例外なので、**ドキュメントを切り替えたら [`clear_stubs`] してから入れ直す**。
// 拡張側の解析キャッシュ（`document.version` が鍵）も同時に捨てないと、
// 「スタブを更新したのに古い型が出続ける」になる。
//
// ⚠ スタブが無いことは異常ではない。引けなければ従来どおり空 body に倒れ、
//   型が付かないだけで**間違った型は付かない**。

use std::{cell::RefCell, collections::{HashMap, HashSet}};

thread_local! {
    /// 鍵 → `.ars` テキスト。ホストが `ar_set_stub` で積む。
    static STUBS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    /// 展開中の鍵。スタブが自分自身を import する形で**無限再帰しない**ようにする。
    static EXPANDING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// スタブ表の鍵。**これが唯一の定義**で、ホストもここが出した文字列をそのまま使う。
///
/// ⚠ `lang` を含めること。`import[cs-dll] Foo` と `import[cpp-dll] Foo` は
/// 別のモジュールで、別のスタブを持つ。
pub(crate) fn stub_key(lang: &str, module: &[String]) -> String {
    format!("{lang}:{}", module.join("."))
}

/// スタブを 1 件登録する（同じ鍵があれば置き換える）。
pub fn set_stub(key: String, source: String) {
    STUBS.with(|s| s.borrow_mut().insert(key, source));
}

/// スタブ表を空にする。**ドキュメントを切り替えたら必ず呼ぶ**（冒頭 doc）。
pub fn clear_stubs() {
    STUBS.with(|s| s.borrow_mut().clear());
}

/// 登録件数（ホストの配線を確かめる用）。
pub fn stub_count() -> usize {
    STUBS.with(|s| s.borrow().len())
}

/// 鍵に対応するスタブ本文を、**再帰を避けつつ** `f` に渡す。
///
/// 戻り値が `None` になるのは 2 通り: スタブが無い／既に展開中（＝循環）。
/// どちらも呼び出し側は「空 body」に倒せばよい。
pub(crate) fn with_stub<T>(key: &str, f: impl FnOnce(&str) -> T) -> Option<T> {
    let source = STUBS.with(|s| s.borrow().get(key).cloned())?;
    // 循環検出。`insert` が false を返したら既に展開中。
    if !EXPANDING.with(|e| e.borrow_mut().insert(key.to_string())) {
        return None;
    }
    let out = f(&source);
    EXPANDING.with(|e| e.borrow_mut().remove(key));
    Some(out)
}
