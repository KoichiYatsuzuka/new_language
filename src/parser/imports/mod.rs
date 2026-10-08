// imports/mod.rs — import 文の解析とモジュール読み込みサブシステムのモジュール束ね。
//
// Python 型スタブ抽出・設定(ar_config.json)解析・C 型→Arrow 型変換などの自由ヘルパーを保持し、
// `impl Parser` の import 解析メソッドを役割別サブモジュール(dispatch/cpp/ar_modules/py_modules/
// cs_js_modules)へ分割して宣言する。



/// 解決先が **CPython の標準ライブラリ**の下にあるか（項目 27）。
///
/// `python_lib_dirs()` の**先頭が stdlib・2 番目が purelib（site-packages）**
/// （`python_lib_dirs` のスクリプトがこの順で出す）。stdlib だけを見たいので
/// 先頭のみと突き合わせる。
///
/// ⚠ Python が見つからない環境では `python_lib_dirs()` が空 ⇒ 常に `false`。
///   そのときは stdlib が検索パスにも入らないので、そもそもここに来ない。
pub(crate) fn is_python_stdlib_path(path: &std::path::Path) -> bool {
    crate::import_fs::python_lib_dirs()
        .first()
        .is_some_and(|stdlib| path.starts_with(stdlib))
}

// ─── Python 型スタブ抽出 ───────────────────────────────────────────────────────

// 実体は [`crate::parser::py_stub_extract`]（以前このモジュールが `editor` では丸ごと消えていたので
// 外へ出した・その doc）。サブモジュールは `use super::*` でここを見ているので、名前だけ通しておく。
pub(crate) use crate::parser::py_stub_extract::extract_py_type_stubs;

// ─── C 型スタブ ───────────────────────────────────────────────────────────────

/// C 型を対応する tl 型名に変換する（静的型検査スタブ生成専用 — 実行時の
/// マーシャリングは `NativeFnRef` 解決時に C シグネチャから別途決まる）。
///
/// - `CType::Void` → `"None"`, `CType::Bool` → `"bool"`,
///   `CType::Float` / `CType::Double` → `"float"`, `CType::CharPtr` → `"str"`,
///   整数・`void*` → `"int"`, `CType::FnPtr` → `"function"`。
/// - プリミティブポインタ（`int*` / `double*` 等）は**ポインティ型**で注釈する
///   （`double*` → `"float"`）。実行時の write-back が書き戻す値型と一致させるため。
/// - 構造体ポインタ・by-value 構造体は `"Any"` とする。名義型（構造体名）で縛ると
///   構造互換な別名クラスのシャドウ変換（`MyVec` → `VECTOR*`、SKILL.md P3）と
///   int ハンドル経路の両方を静的に壊すため。可変性検査（`mut`）は型注釈と
///   独立に `Param::mutable` で機能する。
pub(crate) fn ctype_to_tl_str(ct: &crate::cpp_header::CType) -> String {
    use crate::cpp_header::CType;
    match ct {
        CType::Void => "None".to_string(),
        CType::Bool => "bool".to_string(),
        CType::Float | CType::Double => "float".to_string(),
        CType::CharPtr => "str".to_string(),
        CType::Int | CType::Long | CType::VoidPtr => "int".to_string(),
        CType::Ptr { inner, .. } => ctype_to_tl_str(inner),
        CType::OpaqueStructPtr { .. } | CType::ByValueStruct { .. } => "Any".to_string(),
        CType::FnPtr => "function".to_string(),
    }
}


mod dispatch;
mod cpp;
mod ar_modules;
mod py_modules;
// CPython と同じパッケージの扱い（`import a.b` の連鎖・束縛・サブモジュール・`from . import x`）。
mod packages;
mod cs_js_modules;
