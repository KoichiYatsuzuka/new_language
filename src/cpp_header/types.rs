// types.rs — C type model, struct definitions, and function signatures.
//
// ヘッダから型の情報を読む部分（`cpp_header`）と、実行時に DLL を作る部分（`interpreter/cpp_bridge`）が
// 共有する。⚠ ここには**実行時に依存するもの**（FFI の定数・`RawLayout`）を置かない。そうしたメソッドは
// `interpreter/cpp_bridge/types.rs` の `impl` に置く（拡張の wasm が取り込めるように・
// editor_import_resolution_plan.md 2-2）。

// ── C type model ─────────────────────────────────────────────────────────────

/// C の整数の幅（ffi_value_exchange_plan.md 1-6）。
///
/// `CLong` と `WChar` は環境で幅が変わるので、ヘッダの段階では名前のまま持ち、実行時の中継コードの生成で
/// 決める（`interpreter/cpp_bridge/types.rs`）: Windows（LLP64）は `long` = 4 バイト・`wchar_t` = 2 バイト（符号なし）、
/// それ以外（LP64）は `long` = 8 バイト・`wchar_t` = 4 バイト（符号付き）。
/// ⚠ 以前は `char` / `short` / `int` / 符号の有無をすべて符号付き 4 バイトに、`long` / `long long` / `size_t` を
///   8 バイトにまとめていて、`unsigned` の 2^31 以上が負になる・Windows の `long` の戻り値を 8 バイトとして読む・
///   `short*` へ 4 バイト書く、といったことが起きていた。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntWidth {
    W8,
    W16,
    W32,
    W64,
    /// C の `long`
    CLong,
    /// C の `wchar_t`（ポインタでないもの）
    WChar,
}

/// tl ↔ ネイティブ境界を越えることができる C の型を表す列挙型。
#[derive(Debug, Clone, PartialEq)]
pub enum CType {
    /// 整数（`char` / `short` / `int` / `long` / `long long` / `intN_t` / `size_t` …）→ tl `int`。
    /// 幅と符号を持つ（1-6）。
    Int { width: IntWidth, signed: bool },
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
    /// `char*` / `const char*` / `wchar_t*` — a NUL-terminated C string.
    /// `wide` = `wchar_t*`, `mutable` = no `const` (an output buffer: the C side writes into it).
    /// Only `const char*` parameters cross today (Arrow `str` → UTF-8 copy for the call);
    /// the other shapes are unsupported (`CFnSig::unsupported_reason`, ffi_value_exchange_plan.md 1-2).
    CharPtr { wide: bool, mutable: bool },
    /// `const StructName*` or `StructName*` — opaque struct pointer.
    /// Marshaled as `void*` across the ABI; the shim casts to the real type at the call site.
    OpaqueStructPtr { type_name: String, mutable: bool },
    /// Struct/union passed by value (e.g. `VECTOR`, `MATRIX`).
    /// Passed and returned **by value** everywhere: the Rust wrapper uses a `#[repr(C)]` mirror
    /// `_Struct_{name}`, and the cpp-lib shim keeps the real type (ffi_value_exchange_plan.md 1-3).
    /// Only structs whose exact layout is known (`CStructDef::raw_layout`) can cross; a function that
    /// passes any other struct by value is not generated. The type-check stub annotates it as `Any`.
    ByValueStruct { type_name: String },
    /// Function pointer parameter (e.g. `void (*callback)(int)`).
    /// Passed as an opaque `void*`; maps to tl `function`.
    FnPtr,
    /// `ArPtrProtocol`（`bridge/ptr_protocol/ar_ptr_protocol.h`・ffi_value_exchange_plan.md 4-1・4-2）。配列・文字データ・
    /// 所有権のあるメモリを、長さ・要素の型名・解放の責任と一緒に受け渡す。
    /// `by_pointer` = `ArPtrProtocol*`、`mutable` = ポインタに `const` が無い（C の側で解放して `address` を 0 に書き戻せる）。
    /// ⚠ ヘッダ解析は `#include` を辿らないので、**名前で**見分ける（`parse_c_type_str`）。
    /// ⚠ 受け渡しは 4-4（`ArPtrProtocol*` の引数）・4-5（戻り値）・4-7（Arrow → C の引数）で入る。それまでは
    ///   `CFnSig::unsupported_reason` が止める（中継コードの生成にも型スタブにも来ない）。
    PtrProtocol { by_pointer: bool, mutable: bool },
}

impl CType {
    /// ポインタの指す先の要素の型名（`PtrProtocol` と同じ名前・D-12）。`Ptr[T]` の `T` に使う（4-3）。
    /// `void*` → `void`、構造体ポインタ → 構造体名、数値のポインタ → `int32` など、`char*` → `uint8`、`wchar_t*` → `uint16`。
    /// ポインタでなければ `None`。
    pub fn ptr_elem_name(&self) -> Option<String> {
        match self {
            CType::VoidPtr => Some("void".to_string()),
            CType::OpaqueStructPtr { type_name, .. } => Some(type_name.clone()),
            CType::Ptr { inner, .. } => inner.c_elem_name(),
            CType::CharPtr { wide: false, .. } => Some("uint8".to_string()),
            CType::CharPtr { wide: true, .. } => Some(if cfg!(windows) { "uint16" } else { "int32" }.to_string()),
            _ => None,
        }
    }

    /// 数値の C の幅つきの名前（`int8` … `uint64`・`float32` / `float64`）。C の `bool` は 1 バイトなので `uint8`。
    /// ⚠ `long` と `wchar_t` の幅は環境で決まる（Windows は 4 バイト・2 バイト）。
    pub fn c_elem_name(&self) -> Option<String> {
        Some(match self {
            CType::Int { width, signed } => {
                let bits = match width {
                    IntWidth::W8 => 8,
                    IntWidth::W16 => 16,
                    IntWidth::W32 => 32,
                    IntWidth::W64 => 64,
                    IntWidth::CLong => if cfg!(windows) { 32 } else { 64 },
                    IntWidth::WChar => if cfg!(windows) { 16 } else { 32 },
                };
                format!("{}int{bits}", if *signed { "" } else { "u" })
            }
            CType::Float => "float32".to_string(),
            CType::Double => "float64".to_string(),
            CType::Bool => "uint8".to_string(),
            _ => return None,
        })
    }

    /// 生成された C++ シム ソースで使用する C 型文字列を返す。
    pub fn c_type_str(&self) -> String {
        match self {
            // シムは実際の幅と符号の型で受け渡す（中継コード側の幅と一致させる・1-6）
            CType::Int { width, signed } => {
                let base = match width {
                    IntWidth::W8 => "char",
                    IntWidth::W16 => "short",
                    IntWidth::W32 => "int",
                    IntWidth::W64 => "long long",
                    IntWidth::CLong => "long",
                    IntWidth::WChar => return "wchar_t".to_string(),
                };
                if *signed { format!("signed {base}") } else { format!("unsigned {base}") }
            }
            CType::Float => "float".to_string(),
            CType::Double => "double".to_string(),
            // C++ の bool（1 バイト）。中継コードも 1 バイトで受け渡す（1-6）
            CType::Bool => "bool".to_string(),
            CType::Void => "void".to_string(),
            CType::VoidPtr => "void*".to_string(),
            CType::Ptr { inner, mutable } => {
                if *mutable {
                    format!("{}*", inner.c_type_str())
                } else {
                    format!("const {}*", inner.c_type_str())
                }
            }
            CType::CharPtr { wide, mutable } => {
                let ch = if *wide { "wchar_t" } else { "char" };
                if *mutable { format!("{ch}*") } else { format!("const {ch}*") }
            }
            // Emit void* for opaque struct pointers; the call-site cast is handled in gen_cpp_shim_source
            CType::OpaqueStructPtr { .. } => "void*".to_string(),
            // By-value structs keep their real type: the shim passes and returns them by value, so its
            // signature has the same calling convention as the real function (ffi_value_exchange_plan.md 1-3).
            CType::ByValueStruct { type_name } => type_name.clone(),
            // Function pointers: the shim receives an opaque void*
            CType::FnPtr => "void*".to_string(),
            CType::PtrProtocol { by_pointer: false, .. } => "ArPtrProtocol".to_string(),
            CType::PtrProtocol { by_pointer: true, mutable: true } => "ArPtrProtocol*".to_string(),
            CType::PtrProtocol { by_pointer: true, mutable: false } => "const ArPtrProtocol*".to_string(),
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

// ── What can cross the boundary today (ffi_value_exchange_plan.md 1-2) ──────

/// 型スタブで「この関数は今は使えない」を表すデコレータ（`Expr::Str`）の接頭辞。続きが理由。
/// パーサ（`parser/imports/cpp.rs`）が付け、型検査（`TypeErrorKind::FfiUnsupported`）が読む。
/// ⚠ C/C++ のスタブは実行時には実行されないので、この文字列のデコレータが評価されることはない。
pub const FFI_UNSUPPORTED_MARK: &str = "ffi-unsupported: ";

impl CStructDef {
    /// この構造体を値で受け渡せない理由（受け渡せるなら `None`・4-6）。`ret` は戻り値か。
    ///
    /// フィールドは数値・`bool`・ポインタ（D-7: `char*` は `bytes`、ほかは `Ptr`）・`ArPtrProtocol`（値）。不完全な
    /// レイアウト（配列・ビットフィールドを読み飛ばした）・関数ポインタ・入れ子の構造体は渡せない。引数では、C が書き込む
    /// かもしれない `char*` と `wchar_t*` のフィールドも渡せない（呼び出しの間だけのコピーを渡すので書き込みが失われる・
    /// 出力バッファは 4-7）。⚠ ヘッダが構造体と宣言していない名前へのポインタのフィールドも渡せない。
    pub fn by_value_reason(&self, structs: &[CStructDef], ret: bool) -> Option<String> {
        if !self.complete || self.fields.is_empty() {
            return Some("its exact C layout is not known".to_string());
        }
        let known = |name: &str| structs.iter().any(|d| d.name == name);
        for (fname, ft) in &self.fields {
            let ok = match ft {
                CType::Int { .. } | CType::Float | CType::Double | CType::Bool => true,
                CType::CharPtr { wide: false, mutable: false } => true,
                CType::CharPtr { .. } => ret,
                CType::VoidPtr | CType::Ptr { .. } => true,
                CType::OpaqueStructPtr { type_name, .. } => known(type_name),
                CType::PtrProtocol { by_pointer: false, .. } => true,
                _ => false,
            };
            if !ok {
                return Some(format!("its field `{fname}` cannot be passed in a struct by value"));
            }
        }
        None
    }

    /// C と**同じレイアウトを正確に作れる**か: 完全（`complete`）で、全フィールドが整数（幅と符号つき・1-6）/
    /// float / double。値渡し・値返し・構造体ポインタの受け渡しはこれが真のものだけ（`raw_layout` もこれで判定する）。
    /// `long` / `wchar_t` の幅は環境で決まる（実行時に決める）。`bool` と ポインタのフィールドは含めない。
    pub fn has_exact_layout(&self) -> bool {
        self.complete
            && !self.fields.is_empty()
            && self
                .fields
                .iter()
                .all(|(_, ct)| matches!(ct, CType::Int { .. } | CType::Float | CType::Double))
    }
}

impl CFnSig {
    /// この関数を**今の Arrow から正しく呼べない**なら、その理由（呼べるなら `None`）。
    ///
    /// 型スタブの生成（パーサ）と中継 DLL の生成（実行時）の両方がこれを使う。両者で判定が
    /// ずれないよう、ここ 1 か所に置く（規約「掬いきれなかった選択肢はエラーにする」・
    /// ffi_value_exchange_plan.md 1-2）。以前はこうした関数も生成され、戻り値が黙って `None` になる・
    /// Arrow 内部のハンドル番号をポインタとして渡す（落ちる）・書き込みが失われる、といった形で壊れていた。
    /// 対応は計画書のフェーズ 3〜5 で順に入る（文字列・配列・`Ptr[T]`）。
    pub fn unsupported_reason(&self, structs: &[CStructDef]) -> Option<String> {
        let exact = |name: &str| structs.iter().any(|d| d.name == name && d.has_exact_layout());
        // ヘッダが構造体と宣言した名前か（本体の無い前方宣言を含む・4-3）。⚠ 解決できない typedef（`TCHAR` など）も
        // 構造体ポインタに見えるので、宣言の無い名前は `Ptr` にしない（`const TCHAR*` は実は C の文字列）。
        let known = |name: &str| structs.iter().any(|d| d.name == name);
        // 値で受け渡す構造体の理由（4-6）。定義の無い名前は「レイアウトが分からない」
        let struct_reason = |name: &str, ret: bool| match structs.iter().find(|d| d.name == name) {
            Some(d) => d.by_value_reason(structs, ret),
            None => Some("its exact C layout is not known".to_string()),
        };
        // ⚠ ポインタの戻り値は 4-3 で受け取れるようになった（`char*` / `wchar_t*` → `bytes`、ほか → 借用の `Ptr[T]`・
        //   NULL → `None`・D-7・D-8）。以前はどれもここで使えない関数にしていた（1-2）。
        let ret = match &self.ret {
            CType::FnPtr => Some("it returns a function pointer".to_string()),
            CType::OpaqueStructPtr { type_name, .. } if !known(type_name) => Some(format!(
                "it returns a pointer to `{type_name}`, which the header does not declare as a struct (an unresolved typedef?)"
            )),
            // ⚠ `ArPtrProtocol` の値返しは 4-5 から `Option[Ptr[T]]` で受け取る。ポインタを返すものは誰が持つか決められない
            CType::PtrProtocol { by_pointer: true, .. } => Some(
                "it returns a pointer to `ArPtrProtocol` (return the struct itself by value)".to_string(),
            ),
            // 4-6: ポインタのフィールドを持つ構造体も D-7 の規則で受け取れる（`by_value_reason`）
            CType::ByValueStruct { type_name } => struct_reason(type_name, true).map(|why| {
                format!("it returns struct `{type_name}` by value, and {why}")
            }),
            _ => None,
        };
        if ret.is_some() {
            return ret;
        }
        for (i, (name, ct)) in self.params.iter().enumerate() {
            let p = if name.is_empty() { format!("#{i}") } else { format!("`{name}`") };
            let why = match ct {
                CType::CharPtr { wide: true, .. } => Some(format!("parameter {p} is a wide string (`wchar_t*`)")),
                CType::CharPtr { mutable: true, .. } => Some(format!(
                    "parameter {p} is `char*`, an output buffer (what the C side writes would be lost)"
                )),
                // ⚠ `void*`・レイアウトの分からない構造体ポインタは 4-3 から `Ptr` / `None` を受ける（以前は使えなかった）
                CType::FnPtr => Some(format!("parameter {p} is a function pointer (callback)")),
                CType::OpaqueStructPtr { type_name, .. } if !known(type_name) => Some(format!(
                    "parameter {p} points to `{type_name}`, which the header does not declare as a struct (an unresolved typedef?)"
                )),
                // ⚠ `ArPtrProtocol` の引数は 4-4 から `Ptr` / `None` を受ける（`list` から組み立てるのは 4-7）
                CType::Ptr { mutable: false, .. } => Some(format!(
                    "parameter {p} is a `const` pointer to a number, which may be a single value or an array"
                )),
                CType::ByValueStruct { type_name } => struct_reason(type_name, false).map(|why| {
                    format!("parameter {p} passes struct `{type_name}` by value, and {why}")
                }),
                _ => None,
            };
            if why.is_some() {
                return why;
            }
        }
        // 構造体ポインタは typed 経路でしか受け渡せない（ハンドル経路は Arrow 内部のハンドル番号をポインタとして
        // 渡してしまう・c-abi-interop P7 の残課題）。typed 経路に載らない型と混ざっている関数は呼べない。
        // ⚠ レイアウトの分からない構造体ポインタは `Ptr` としてハンドル経路で渡せる（4-3）ので、この規則の対象は
        //   インスタンスを渡す（正確なレイアウトの）構造体ポインタだけ。
        if self.params.iter().any(|(_, t)| matches!(t, CType::OpaqueStructPtr { type_name, .. } if exact(type_name)))
            && !self.typed_abi_eligible(structs)
        {
            let other = if typed_abi_ok(&self.ret, structs, false) {
                self.params.iter().map(|(_, t)| t).find(|t| !typed_abi_ok(t, structs, true))
            } else {
                Some(&self.ret)
            };
            let what = match other {
                Some(CType::Bool) => "`bool`",
                Some(CType::CharPtr { .. }) => "a C string",
                Some(CType::ByValueStruct { .. }) => "a struct returned by value",
                _ => "another parameter type",
            };
            return Some(format!(
                "it passes a struct pointer together with {what}, which the struct-pointer call path cannot carry yet"
            ));
        }
        None
    }

    /// typed ABI（中継 DLL の `{name}_typed`・`AbiTy`）で呼べるか。戻り値は void / 整数 / 浮動小数、
    /// 引数は整数 / 浮動小数 / 数値への書き込みポインタ / 正確なレイアウトを持つ構造体（ポインタ・値渡し）。
    /// 中継 DLL の生成（`cpp_typed_eligible`）と実行時のシグネチャ（`build_cpp_typed_sig`）はこれと一致させる。
    pub fn typed_abi_eligible(&self, structs: &[CStructDef]) -> bool {
        typed_abi_ok(&self.ret, structs, false) && self.params.iter().all(|(_, t)| typed_abi_ok(t, structs, true))
    }
}

/// `typed_abi_eligible` の 1 つの型の判定。`param` は引数か（戻り値は構造体・ポインタを持てない）。
fn typed_abi_ok(t: &CType, structs: &[CStructDef], param: bool) -> bool {
    let exact = |name: &str| structs.iter().any(|d| d.name == name && d.has_exact_layout());
    let known = |name: &str| structs.iter().any(|d| d.name == name);
    // ⚠ `exec::modules::build_cpp_typed_sig` と同じ対応にすること（中継 DLL の `_typed` とインタプリタのシグネチャが一致する）
    match t {
        CType::Int { .. } | CType::Float | CType::Double => true,
        CType::Void => !param,
        CType::Ptr { inner, mutable: true } if param => {
            matches!(**inner, CType::Int { .. } | CType::Float | CType::Double)
        }
        CType::ByValueStruct { type_name } if param => exact(type_name),
        // 4-4: 構造体ポインタはインスタンス（正確なレイアウト）か `Ptr`（構造体と宣言された名前）。戻り値は借用の `Ptr`
        CType::OpaqueStructPtr { type_name, .. } => known(type_name),
        CType::Bool | CType::VoidPtr => true,
        CType::CharPtr { wide: false, mutable: false } if param => true,
        CType::CharPtr { .. } => !param,
        CType::Ptr { .. } => !param,
        CType::PtrProtocol { by_pointer, .. } => param || !*by_pointer,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(name: &str, fields: Vec<CType>, complete: bool) -> CStructDef {
        CStructDef {
            name: name.to_string(),
            fields: fields.into_iter().enumerate().map(|(i, t)| (format!("f{i}"), t)).collect(),
            complete,
        }
    }
    fn structs() -> Vec<CStructDef> {
        vec![
            st("Rec", vec![I32, CType::Double], true),
            st("Named", vec![I32, CType::CharPtr { wide: false, mutable: false }], true),
            st("Partial", vec![I32; 2], false),
            st("WithCallback", vec![I32, CType::FnPtr], true),
            st("OutBuf", vec![I32, CType::CharPtr { wide: false, mutable: true }], true),
        ]
    }
    fn sig(params: Vec<CType>, ret: CType) -> CFnSig {
        CFnSig {
            name: "f".to_string(),
            params: params.into_iter().enumerate().map(|(i, t)| (format!("p{i}"), t)).collect(),
            ret,
            namespace: None,
            n_required: 0,
        }
    }
    fn st_ptr(n: &str) -> CType { CType::OpaqueStructPtr { type_name: n.to_string(), mutable: false } }
    fn by_val(n: &str) -> CType { CType::ByValueStruct { type_name: n.to_string() } }
    const CSTR: CType = CType::CharPtr { wide: false, mutable: false };
    const I32: CType = CType::Int { width: IntWidth::W32, signed: true };
    const I64: CType = CType::Int { width: IntWidth::W64, signed: true };
    const U8: CType = CType::Int { width: IntWidth::W8, signed: false };

    /// 今の Arrow から正しく呼べるもの（理由なし）
    #[test]
    fn supported_signatures() {
        let s = structs();
        for f in [
            sig(vec![I32, CType::Double], I64),
            sig(vec![CSTR], I32),
            sig(vec![by_val("Rec")], by_val("Rec")),
            sig(vec![st_ptr("Rec"), CType::Ptr { inner: Box::new(CType::Double), mutable: true }], I32),
            sig(vec![CType::Bool], CType::Void),
            // 4-3: ポインタの戻り値（`char*` / `wchar_t*` → bytes、ほか → Ptr[T]）と、`void*`・構造体と宣言された名前への
            // ポインタ（レイアウトが分からなくても `Ptr[T]` で渡せる）の引数
            sig(vec![], CSTR),
            sig(vec![], CType::CharPtr { wide: true, mutable: false }),
            sig(vec![], CType::Ptr { inner: Box::new(I32), mutable: true }),
            sig(vec![], CType::VoidPtr),
            sig(vec![], st_ptr("Rec")),
            sig(vec![], st_ptr("Partial")),
            sig(vec![CType::VoidPtr], CType::Void),
            sig(vec![st_ptr("Named")], CType::Void),
            sig(vec![st_ptr("Partial"), CSTR], I32),
            // 4-4: 正確なレイアウトの構造体ポインタと C の文字列・bool が混ざっても型つき経路で渡せる
            sig(vec![st_ptr("Rec"), CSTR], CType::Void),
            sig(vec![st_ptr("Rec")], CType::Bool),
            sig(vec![CType::PtrProtocol { by_pointer: true, mutable: true }], I32),
            sig(vec![CType::PtrProtocol { by_pointer: false, mutable: false }, st_ptr("Rec")], CSTR),
            // 4-5: `ArPtrProtocol` の値返し
            sig(vec![I32], CType::PtrProtocol { by_pointer: false, mutable: false }),
            // 4-6: ポインタのフィールドを持つ構造体の値渡し・値返し（D-7 の規則）。`char*` のフィールドは戻り値なら受け取れる
            sig(vec![by_val("Named")], CType::Void),
            sig(vec![], by_val("Named")),
            sig(vec![], by_val("OutBuf")),
        ] {
            assert_eq!(f.unsupported_reason(&s), None, "{f:?}");
        }
    }

    /// ポインタの指す先の要素の型名（D-12・4-3）
    #[test]
    fn pointer_element_names() {
        assert_eq!(CType::VoidPtr.ptr_elem_name().as_deref(), Some("void"));
        assert_eq!(st_ptr("Rec").ptr_elem_name().as_deref(), Some("Rec"));
        assert_eq!(CSTR.ptr_elem_name().as_deref(), Some("uint8"));
        let p = |t: CType| CType::Ptr { inner: Box::new(t), mutable: false }.ptr_elem_name();
        assert_eq!(p(I32).as_deref(), Some("int32"));
        assert_eq!(p(U8).as_deref(), Some("uint8"));
        assert_eq!(p(I64).as_deref(), Some("int64"));
        assert_eq!(p(CType::Float).as_deref(), Some("float32"));
        assert_eq!(p(CType::Double).as_deref(), Some("float64"));
        assert_eq!(I32.ptr_elem_name(), None);
    }

    /// 呼べないもの: 理由に何が問題かが入る（1-2）
    #[test]
    fn unsupported_signatures() {
        let s = structs();
        let cases: Vec<(CFnSig, &str)> = vec![
            (sig(vec![], CType::FnPtr), "function pointer"),
            // 構造体と宣言されていない名前（解決できない typedef など）へのポインタ（4-3）
            (sig(vec![], st_ptr("Unknown")), "does not declare as a struct"),
            (sig(vec![], by_val("Partial")), "struct `Partial` by value"),
            (sig(vec![CType::CharPtr { wide: false, mutable: true }], CType::Void), "output buffer"),
            (sig(vec![CType::CharPtr { wide: true, mutable: false }], CType::Void), "wide string"),
            (sig(vec![CType::FnPtr], CType::Void), "callback"),
            (sig(vec![CType::Ptr { inner: Box::new(I32), mutable: false }], CType::Void), "single value or an array"),
            (sig(vec![st_ptr("Unknown")], CType::Void), "does not declare as a struct"),
            // 構造体に関数ポインタのフィールド（4-6 でも渡せない）
            (sig(vec![by_val("WithCallback")], CType::Void), "field `f1` cannot be passed in a struct by value"),
            // 引数では、C が書き込むかもしれない `char*` のフィールドは渡せない（戻り値なら受け取れる）
            (sig(vec![by_val("OutBuf")], CType::Void), "field `f1` cannot be passed in a struct by value"),
            // 型つき経路に載らない形（値返しの構造体）と、インスタンスを渡す構造体ポインタは混ぜられない
            (sig(vec![st_ptr("Rec")], by_val("Rec")), "struct pointer together with a struct returned by value"),
            // `ArPtrProtocol*` を返す関数は誰が持つか決められない（値で返す）
            (sig(vec![], CType::PtrProtocol { by_pointer: true, mutable: false }), "return the struct itself by value"),
        ];
        for (f, want) in cases {
            let got = f.unsupported_reason(&s);
            assert!(got.as_deref().is_some_and(|r| r.contains(want)), "{f:?} -> {got:?} (want {want:?})");
        }
    }

    /// 正確なレイアウト: 完全で int / float / double だけ
    #[test]
    fn exact_layout() {
        let s = structs();
        assert!(s[0].has_exact_layout());
        assert!(!s[1].has_exact_layout()); // ポインタのフィールド
        assert!(!s[2].has_exact_layout()); // 不完全（配列などのフィールドを読み飛ばした）
        assert!(st("Rgba", vec![U8; 4], true).has_exact_layout()); // 4 バイト未満の整数も幅が分かる（1-6）
        assert!(!st("Empty", vec![], true).has_exact_layout());
        assert!(!st("L", vec![CType::Bool], true).has_exact_layout());
    }
}
