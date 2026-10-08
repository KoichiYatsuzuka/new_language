// types.rs — C type model, struct definitions, and function signatures.
//
// ヘッダから型の情報を読む部分（`cpp_header`）と、実行時に DLL を作る部分（`interpreter/cpp_bridge`）が
// 共有する。⚠ ここには**実行時に依存するもの**（FFI の定数・`RawLayout`）を置かない。そうしたメソッドは
// `interpreter/cpp_bridge/types.rs` の `impl` に置く（拡張の wasm が取り込めるように・
// editor_import_resolution_plan.md 2-2）。

// ── C type model ─────────────────────────────────────────────────────────────

/// tl ↔ ネイティブ境界を越えることができる C の型を表す列挙型。
#[derive(Debug, Clone, PartialEq)]
pub enum CType {
    /// `int`, `short`, `char`, `int32_t`, `uint32_t` → tl `int`
    Int,
    /// `long`, `long long`, `int64_t`, `uint64_t`, `size_t` → tl `int`
    Long,
    /// `float` → tl `float`
    Float,
    /// `double` → tl `float`
    Double,
    /// `bool` → tl `bool`
    Bool,
    /// `void` (return type only) → tl `None`
    Void,
    /// `void*` → tl `int` (opaque pointer stored as raw integer)
    VoidPtr,
    /// `T*` or `const T*` pointer parameter.
    /// `mutable = true` → output param: value written back after the call.
    /// `mutable = false` → input param: read-only, no write-back.
    Ptr { inner: Box<CType>, mutable: bool },
    /// `char*` / `const char*` — tl `str` ↔ null-terminated C string.
    CharPtr,
    /// `const StructName*` or `StructName*` — opaque struct pointer.
    /// Marshaled as `void*` across the ABI; the shim casts to the real type at the call site.
    OpaqueStructPtr { type_name: String, mutable: bool },
    /// Struct/union passed by value (e.g. `VECTOR`, `MATRIX`).
    /// Parameters: shim receives `void*` and dereferences with `*(TypeName*)ptr`.
    /// Return type: shim writes into a per-function static buffer and returns its address.
    /// Maps to tl `int` (opaque handle = pointer to the struct data).
    ByValueStruct { type_name: String },
    /// Function pointer parameter (e.g. `void (*callback)(int)`).
    /// Passed as an opaque `void*`; maps to tl `function`.
    FnPtr,
}

impl CType {
    /// 生成された C++ シム ソースで使用する C 型文字列を返す。
    pub fn c_type_str(&self) -> String {
        match self {
            CType::Int => "int".to_string(),
            CType::Long => "long long".to_string(),
            CType::Float => "float".to_string(),
            CType::Double => "double".to_string(),
            CType::Bool => "int".to_string(),
            CType::Void => "void".to_string(),
            CType::VoidPtr => "void*".to_string(),
            CType::Ptr { inner, mutable } => {
                if *mutable {
                    format!("{}*", inner.c_type_str())
                } else {
                    format!("const {}*", inner.c_type_str())
                }
            }
            CType::CharPtr => "const char*".to_string(),
            // Emit void* for opaque struct pointers; the call-site cast is handled in gen_cpp_shim_source
            CType::OpaqueStructPtr { .. } => "void*".to_string(),
            // By-value structs and fn ptrs: shim receives void* and handles the real type
            CType::ByValueStruct { .. } | CType::FnPtr => "void*".to_string(),
        }
    }

}

// ── Struct definition ─────────────────────────────────────────────────────────

/// `typedef struct { … } Name;` / `struct Name { … };` / `class Name { … };` 形式から
/// 抽出した C/C++ 構造体・クラスの定義。
#[derive(Debug, Clone)]
pub struct CStructDef {
    /// 型名（typedef エイリアスまたは struct/class 名。例: `"VECTOR"`）。
    pub name: String,
    /// 宣言順のフィールド一覧: `(フィールド名, CType)`。
    pub fields: Vec<(String, CType)>,
    /// `fields` が C/C++ 側のレイアウトメンバを**すべて**含むとき `true`。
    /// 配列・ビットフィールド・ネスト構造体・未解決型のフィールドをスキップした場合や、
    /// union・継承付きクラスの場合は `false`（raw レイアウトは付与できない）。
    pub complete: bool,
}

// ── Function signature ───────────────────────────────────────────────────────

/// `.h` ファイルから抽出した C 関数シグネチャ。
#[derive(Debug, Clone)]
pub struct CFnSig {
    /// C 関数名（例: `"CreateWindow"`）。
    pub name: String,
    /// 宣言順のパラメータ一覧: `(パラメータ名, CType)`。
    pub params: Vec<(String, CType)>,
    /// 戻り値の `CType`。
    pub ret: CType,
    /// この関数が属する C++ 名前空間（例: `"DxLib"`）。名前空間がない場合は `None`。
    pub namespace: Option<String>,
    /// 最初のオプション引数のインデックス（DEFAULTPARAM / C++ デフォルト引数を持つもの）。
    /// 呼び出し側は `n_required` 以降の末尾引数を省略できる。省略された引数は
    /// 0（tl の None ハンドル、NULL ポインタまたは 0 にマーシャリングされる）で埋められる。
    pub n_required: usize,
}
