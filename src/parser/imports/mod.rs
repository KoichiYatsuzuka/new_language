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
/// C の型が型スタブのどこに現れるか（4-3 でポインタの型が位置で変わるようになった）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CPos {
    Param,
    Return,
    Field,
}

/// C の型を型スタブの注釈にする。
///
/// 生のポインタの既定の規則（ffi_value_exchange_plan.md 4-3・D-7・D-8・D-13）:
/// - `char*` / `wchar_t*` → `Option[bytes]`（戻り値は NUL 終端の借用をコピーした `bytes`、`wchar_t` は UTF-16LE。引数の
///   `const char*` は `bytes` だけ受ける。`str` は静的エラーにして文字コードを書かせる）
/// - ほかのポインタの戻り値・`void*` の引数・レイアウトの分からない構造体ポインタの引数 → `Option[Ptr[T]]`（NULL は `None`）
/// - 数値の書き込みポインタの引数は指す先の型（書き戻し・D-14）、正確なレイアウトの構造体ポインタの引数は `Any`
///   （インスタンスを渡す・形の合う別のクラスも渡せる・c-abi-interop P6）
/// ⚠ 以前は `char*` を `str`、`void*` を `int` と注釈していた。
pub(crate) fn ctype_to_tl_str(
    ct: &crate::cpp_header::CType,
    pos: CPos,
    structs: &[crate::cpp_header::CStructDef],
) -> String {
    use crate::cpp_header::CType;
    let exact = |name: &str| structs.iter().any(|d| d.name == name && d.has_exact_layout());
    let ptr_of = |ct: &CType| format!("Option[Ptr[{}]]", ct.ptr_elem_name().unwrap_or_else(|| "void".to_string()));
    match ct {
        CType::Void => "None".to_string(),
        CType::Bool => "bool".to_string(),
        CType::Float | CType::Double => "float".to_string(),
        CType::CharPtr { .. } => "Option[bytes]".to_string(),
        CType::Int { .. } => "int".to_string(),
        CType::VoidPtr => ptr_of(ct),
        // 数値の書き込みポインタの引数は、書き戻す値の型（D-14）
        CType::Ptr { inner, .. } if pos == CPos::Param => ctype_to_tl_str(inner, pos, structs),
        CType::Ptr { .. } => ptr_of(ct),
        CType::OpaqueStructPtr { type_name, .. } if pos == CPos::Param && exact(type_name) => "Any".to_string(),
        CType::OpaqueStructPtr { .. } => ptr_of(ct),
        CType::ByValueStruct { .. } => "Any".to_string(),
        CType::FnPtr => "function".to_string(),
        // `ArPtrProtocol`（4-2・4-4）: 受け取りは `Option[Ptr[void]]`（要素の型は実行時に届くので、`p mustbe Ptr[T]` で
        // 照合して戻す・D-10・D-12）。引数はどの形でも `Ptr` か `None`（どの `Ptr` も渡せる）。
        // ⚠ `list` から組み立てて渡すのは 4-7（そのとき引数の型を広げる）
        CType::PtrProtocol { .. } => "Option[Ptr[void]]".to_string(),
    }
}


mod dispatch;
mod cpp;
mod ar_modules;
mod py_modules;
// CPython と同じパッケージの扱い（`import a.b` の連鎖・束縛・サブモジュール・`from . import x`）。
mod packages;
mod cs_js_modules;
