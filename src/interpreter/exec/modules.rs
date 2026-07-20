// exec/modules.rs — モジュール読み込みの実行: import 文、ネイティブ/C++/C# ブリッジ DLL のロードと型シグネチャ構築。

use {
    std::collections::HashMap, std::path::PathBuf,
    std::rc::Rc, std::sync::Arc,
    crate::ast::Stmt,
    crate::interpreter::{
        ExecResult,
        Interpreter, ModuleState, NamespaceData, NativeFnRef, NativeLibWrapper, Value, Var,
    },
};
use super::*;

impl Interpreter {
    /// モジュールの body を孤立スコープで実行し、`Value::Namespace` を返す。
    /// キャッシュを使用し、循環 import はエラーにする。
    pub(crate) fn exec_module(
        &mut self,
        lang: &str,
        module: &[String],
        body: &[Stmt],
    ) -> Result<Rc<NamespaceData>, String> {
        let cache_key = (lang.to_string(), PathBuf::from(module.join("/")));

        match self.module_cache.get(&cache_key).cloned() {
            Some(ModuleState::Loading) => {
                return Err(format!(
                    "RuntimeError: circular import detected: '{}'",
                    module.join(".")
                ));
            }
            Some(ModuleState::Loaded(ns)) => return Ok(ns),
            None => {}
        }

        self.module_cache
            .insert(cache_key.clone(), ModuleState::Loading);

        if lang == "py-int" {
            let search_dirs = self.python_search_dirs.clone();
            let ns = crate::interpreter::py_interop::load_py_int_module(module, &search_dirs)?;
            self.module_cache
                .insert(cache_key, ModuleState::Loaded(ns.clone()));
            return Ok(ns);
        }

        // tl-auto / tlc / rs: native payload in cache → try to load natively
        if lang == "tl-auto" || lang == "ar-auto" || lang == "tlc" || lang == "arc" || lang == "rs" {
            let module_name = module.join(".");
            // .arc files store only the stem as their module name; fall back to last segment.
            let native_data = crate::partial_compiler::take_native_bytes(&module_name)
                .or_else(|| {
                    let stem = module.last().map(|s| s.as_str()).unwrap_or("");
                    if stem != module_name { crate::partial_compiler::take_native_bytes(stem) } else { None }
                });
            if let Some((_exports, payload)) = native_data
            {
                use crate::partial_compiler::NativePayload;
                // ── DLL path (v1) ─────────────────────────────────────────────
                let NativePayload::Dll(dll_bytes) = payload;
                let ext = crate::partial_compiler::native_lib_ext();
                let stem = module.last().cloned().unwrap_or_default();
                let tmp_path = std::env::temp_dir().join(format!("{stem}_tl.{ext}"));
                match std::fs::write(&tmp_path, &dll_bytes) {
                    Ok(()) => match self.try_load_native_module(module, body, &tmp_path) {
                        Ok(ns) => {
                            self.module_cache.insert(cache_key, ModuleState::Loaded(ns.clone()));
                            return Ok(ns);
                        }
                        Err(e) => eprintln!("NativeLoad(DLL): {e}"),
                    },
                    Err(e) => eprintln!("NativeLoad(DLL): cannot write temp DLL: {e}"),
                }
            }
        }

        let prev_in_python = self.in_python_module;
        if lang == "py" {
            self.in_python_module = true;
        }
        self.push_scope();
        for stmt in body {
            match self.exec(stmt)? {
                ExecResult::Normal => {}
                ExecResult::Raise(_) => {
                    self.pop_scope();
                    return Err(format!(
                        "RuntimeError: exception during module initialization: {}",
                        module.join(".")
                    ));
                }
                _ => {}
            }
        }
        let members: HashMap<String, Value> = self
            .scopes
            .last()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.get_value()))
            .collect();
        self.pop_scope();
        self.in_python_module = prev_in_python;

        // Python モジュールのメソッドが同モジュール内の他の関数を呼び出せるように
        // モジュールメンバをグローバルスコープに登録する（既存エントリは上書きしない）。
        for (name, value) in &members {
            self.scopes[0]
                .entry(name.clone())
                .or_insert_with(|| Var::new(value.clone(), false));
        }

        let ns = Rc::new(NamespaceData {
            name: module.join("."),
            members,
        });
        self.module_cache
            .insert(cache_key, ModuleState::Loaded(ns.clone()));
        Ok(ns)
    }

    /// 関数シグネチャから typed ABI のシグネチャを構築する。
    /// 全パラメータが `let` かつ int/float 注釈、戻り値も int/float のときのみ Some。
    /// codegen 側の typed 候補条件（llvm_codegen.rs）と一致させること。
    pub(crate) fn build_typed_sig(
        params: &[crate::ast::Param],
        return_type: Option<&str>,
    ) -> Option<crate::interpreter::value::TypedSig> {
        use crate::interpreter::value::{AbiTy, TypedSig};
        let ret = match return_type {
            Some("int") => AbiTy::I64,
            Some("float") => AbiTy::F64,
            _ => return None,
        };
        let mut ptys = Vec::with_capacity(params.len());
        for p in params {
            if p.mutable || p.default.is_some() {
                return None;
            }
            match p.type_ann.as_deref() {
                Some("int") => ptys.push(AbiTy::I64),
                Some("float") => ptys.push(AbiTy::F64),
                _ => return None,
            }
        }
        Some(TypedSig { params: ptys, ret })
    }

    /// C 関数シグネチャ（cpp ブリッジ）から typed ABI のシグネチャを構築する。
    ///
    /// パラメータは int/long/float/double、または raw レイアウトが既知の C 構造体への
    /// ポインタ（`OpaqueStructPtr`）・by-value 構造体（`ByValueStruct`）のいずれかであること。
    /// 戻り値は void/int/long/float/double のみ（構造体戻り値は非対応）。
    /// codegen 側の `cpp_typed_eligible`（cpp_bridge/codegen.rs）と条件を一致させること。
    pub(crate) fn build_cpp_typed_sig(
        sig: &crate::interpreter::cpp_bridge::CFnSig,
        raw_layouts: &HashMap<String, Arc<crate::interpreter::value::RawLayout>>,
    ) -> Option<crate::interpreter::value::TypedSig> {
        use crate::interpreter::cpp_bridge::CType;
        use crate::interpreter::value::{AbiTy, TypedSig};
        let ret = match sig.ret {
            CType::Void => AbiTy::Void,
            CType::Int | CType::Long => AbiTy::I64,
            CType::Float | CType::Double => AbiTy::F64,
            _ => return None,
        };
        let mut ptys = Vec::with_capacity(sig.params.len());
        for (_, ct) in &sig.params {
            match ct {
                CType::Int | CType::Long => ptys.push(AbiTy::I64),
                CType::Float | CType::Double => ptys.push(AbiTy::F64),
                CType::OpaqueStructPtr { type_name, mutable } => {
                    let layout = raw_layouts.get(type_name)?.clone();
                    ptys.push(AbiTy::Ptr { mutable: *mutable, by_value: false, layout });
                }
                CType::ByValueStruct { type_name } => {
                    let layout = raw_layouts.get(type_name)?.clone();
                    ptys.push(AbiTy::Ptr { mutable: false, by_value: true, layout });
                }
                // プリミティブ書き込みポインタ（`int*` / `double*`）: OutPtr スロット。
                // 構造体ポインタと混在する関数（例: v3_norm(const V3*, double*)）を
                // typed 経路の対象にする — ハンドル経路は構造体ポインタ引数を扱えない。
                // 幅は rust_extern_type と同じ規約（Int→i32, Long→i64 — LLP64 既知課題）。
                CType::Ptr { inner, mutable: true } => {
                    use crate::interpreter::value::RawWidth;
                    let width = match **inner {
                        CType::Int => RawWidth::I32,
                        CType::Long => RawWidth::I64,
                        CType::Float => RawWidth::F32,
                        CType::Double => RawWidth::F64,
                        _ => return None,
                    };
                    ptys.push(AbiTy::OutPtr { width });
                }
                _ => return None,
            }
        }
        Some(TypedSig { params: ptys, ret })
    }

    /// ネイティブ共有ライブラリをロードして、そのモジュールの `Namespace` を構築する。
    pub(crate) fn try_load_native_module(
        &mut self,
        module: &[String],
        body: &[Stmt],
        lib_path: &std::path::Path,
    ) -> Result<Rc<NamespaceData>, String> {
        let lib = unsafe { libloading::Library::new(lib_path) }
            .map_err(|e| format!("libloading: {e}"))?;

        let lib_path_buf = lib_path.to_path_buf();

        self.push_scope();
        for stmt in body {
            match self.exec(stmt)? {
                ExecResult::Normal => {}
                ExecResult::Raise(_raised) => {
                    self.pop_scope();
                    return Err(format!(
                        "RuntimeError: exception during native module init: {}",
                        module.join(".")
                    ));
                }
                _ => {}
            }
        }
        let mut members: HashMap<String, Value> = self
            .scopes
            .last()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.get_value()))
            .collect();
        self.pop_scope();

        for stmt in body {
            match stmt {
                Stmt::FnDef { name, params, return_type, .. } => {
                    let symbol_name = format!("{name}_tl\0");
                    if let Ok(func) = unsafe {
                        lib.get::<unsafe extern "C" fn(*const i64, i32) -> i64>(symbol_name.as_bytes())
                    } {
                        let initial_ptr = *func as usize;
                        // typed エントリ ({name}_typed): 統一 typed ABI。
                        // シグネチャが全プリミティブの場合のみシンボルを探す。
                        let typed_sig = Self::build_typed_sig(params, return_type.as_deref());
                        let typed_ptr = if typed_sig.is_some() {
                            let typed_symbol = format!("{name}_typed\0");
                            match unsafe {
                                lib.get::<unsafe extern "C" fn(
                                    *const u64,
                                    *mut u64,
                                    *mut crate::interpreter::native_api::ErrSlot,
                                ) -> u32>(typed_symbol.as_bytes())
                            } {
                                Ok(f) => *f as usize,
                                Err(_) => 0,
                            }
                        } else {
                            0
                        };
                        let fn_ref = Arc::new(NativeFnRef {
                            lib_path: lib_path_buf.clone(),
                            fn_name: name.clone(),
                            n_params: params.len(),
                            min_params: params.len(),
                            param_mutabilities: params.iter().map(|p| p.mutable).collect(),
                            ptr_params: vec![crate::interpreter::PtrParam::None; params.len()],
                            cached_fn_ptr: std::sync::atomic::AtomicUsize::new(initial_ptr),
                            typed_fn_ptr: std::sync::atomic::AtomicUsize::new(typed_ptr),
                            typed_sig: if typed_ptr != 0 { typed_sig } else { None },
                        });
                        members.insert(name.clone(), Value::NativeFunction(fn_ref));
                    }
                }
                Stmt::GenDef { name, params, .. } => {
                    let symbol_name = format!("{name}_tl\0");
                    if let Ok(func) = unsafe {
                        lib.get::<unsafe extern "C" fn(*const i64, i32) -> i64>(symbol_name.as_bytes())
                    } {
                        let initial_ptr = *func as usize;
                        let fn_ref = Arc::new(NativeFnRef {
                            lib_path: lib_path_buf.clone(),
                            fn_name: name.clone(),
                            n_params: params.len(),
                            min_params: params.len(),
                            param_mutabilities: params.iter().map(|p| p.mutable).collect(),
                            ptr_params: vec![crate::interpreter::PtrParam::None; params.len()],
                            cached_fn_ptr: std::sync::atomic::AtomicUsize::new(initial_ptr),
                            typed_fn_ptr: std::sync::atomic::AtomicUsize::new(0),
                            typed_sig: None,
                        });
                        members.insert(name.clone(), Value::NativeFunction(fn_ref));
                    }
                }
                Stmt::ClassDef { name: class_name, body: class_body, .. } => {
                    for method_stmt in class_body {
                        let (mname, params) = match method_stmt {
                            Stmt::FnDef { name, params, .. } => (name, params),
                            Stmt::GenDef { name, params, .. } => (name, params),
                            _ => continue,
                        };
                        let symbol = crate::partial_compiler::llvm_codegen::method_symbol(class_name, mname);
                        let symbol_name = format!("{symbol}_tl\0");
                        if let Ok(func) = unsafe {
                            lib.get::<unsafe extern "C" fn(*const i64, i32) -> i64>(symbol_name.as_bytes())
                        } {
                            let fn_ptr = *func as usize;
                            crate::interpreter::native_api::register_native_method(class_name, mname, fn_ptr);
                            eprintln!("NativeMethod: {class_name}.{mname} ({} param(s)) → native", params.len());
                        }
                    }
                }
                _ => {}
            }
        }

        for (name, value) in &members {
            self.scopes[0]
                .entry(name.clone())
                .or_insert_with(|| Var::new(value.clone(), false));
        }

        {
            let cb_ptr = crate::interpreter::native_api::get_callbacks();
            let init_result = unsafe {
                lib.get::<unsafe extern "C" fn(*const crate::interpreter::native_api::ArCallbacks)>(b"ar_init\0")
            }.or_else(|_| unsafe {
                // backward compat: DLLs compiled before rename still export hv_init
                lib.get::<unsafe extern "C" fn(*const crate::interpreter::native_api::ArCallbacks)>(b"hv_init\0")
            });
            if let Ok(ar_init) = init_result {
                unsafe { ar_init(cb_ptr) };
            }
        }

        self.native_libs.insert(lib_path_buf, NativeLibWrapper(lib));

        let ns = Rc::new(NamespaceData {
            name: module.join("."),
            members,
        });
        Ok(ns)
    }

    // ---------------------------------------------------------------------------
    // C++ bridge module loading
    // ---------------------------------------------------------------------------

    /// C++ ライブラリ（`cpp-lib`）または DLL（`cpp-dll`）を tl モジュールとしてロードする。
    /// ヘッダーをパースして関数シグネチャを収集し、ラッパー DLL を構築・ロードして名前空間を返す。
    pub(crate) fn load_cpp_module(
        &mut self,
        lang: &str,
        header_path_str: &str,
        _with_file: Option<&str>,
    ) -> Result<Rc<NamespaceData>, String> {
        let header_path = std::path::Path::new(header_path_str);
        let header_dir = header_path.parent().unwrap_or(std::path::Path::new("."));

        // Parse the header to extract function signatures.
        // Read as raw bytes then convert lossily: non-UTF-8 bytes (e.g. Shift-JIS
        // in Japanese comments) become U+FFFD replacement chars, which strip_comments
        // discards along with the surrounding comment text.
        let raw = std::fs::read(header_path)
            .map_err(|e| format!("CppImport: cannot read header '{header_path_str}': {e}"))?;
        let raw_str = String::from_utf8_lossy(&raw);
        // Load config before parsing so custom_type_map is available for all parse_header calls.
        let config = crate::interpreter::cpp_bridge::load_cpp_config(header_dir);
        let typedefs = crate::interpreter::cpp_bridge::load_system_typedefs(
            &config.system_headers,
            &config.precompile_macros,
        );
        let (mut sigs, mut struct_defs) =
            crate::interpreter::cpp_bridge::parse_header_full(&raw_str, &config.custom_type_map, &typedefs);

        match lang {
            "cpp-lib" => {
                // Build tl_{stem}.dll next to the header (permanent cache).

                // When precompile_macros are set, the main header may conditionally
                // include other headers (e.g. WINDOWS_DESKTOP_OS → DxFunctionWin.h).
                // Scan for local #include directives and parse those headers too so
                // their function signatures are available in the tl namespace.
                if !config.precompile_macros.is_empty() {
                    let included =
                        crate::interpreter::cpp_bridge::collect_included_headers(&raw_str, header_dir);
                    let mut known_names: std::collections::HashSet<String> =
                        sigs.iter().map(|s| s.name.clone()).collect();
                    let mut known_structs: std::collections::HashSet<String> =
                        struct_defs.iter().map(|d| d.name.clone()).collect();
                    for inc_path in &included {
                        if let Ok(inc_raw) = std::fs::read(inc_path) {
                            let inc_str = String::from_utf8_lossy(&inc_raw);
                            let (inc_sigs, inc_structs) =
                                crate::interpreter::cpp_bridge::parse_header_full(&inc_str, &config.custom_type_map, &typedefs);
                            let new_count = inc_sigs
                                .iter()
                                .filter(|s| !known_names.contains(&s.name))
                                .count();
                            if new_count > 0 {
                                eprintln!(
                                    "CppImport: {} additional function(s) from '{}'",
                                    new_count,
                                    inc_path.display()
                                );
                            }
                            for s in inc_sigs {
                                if known_names.insert(s.name.clone()) {
                                    sigs.push(s);
                                }
                            }
                            for d in inc_structs {
                                if known_structs.insert(d.name.clone()) {
                                    struct_defs.push(d);
                                }
                            }
                        }
                    }
                }

                if sigs.is_empty() {
                    eprintln!("CppImport: no supported functions found in '{header_path_str}'");
                }
                eprintln!("CppImport[{lang}]: {} function(s) total", sigs.len());

                let (dll_path, effective_sigs) =
                    crate::interpreter::cpp_bridge::compile_tl_dll(header_path, &sigs, &struct_defs, &config)?;
                self.load_cpp_wrapper_dll(&dll_path, &effective_sigs, &struct_defs, header_path_str)
            }
            "cpp-dll" => {
                // Find the DLL by stem next to the header and wrap it dynamically.
                let stem = header_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("lib");
                let dll_path = header_dir.join(format!("{stem}.dll"));
                let dll_str = dll_path.to_string_lossy().into_owned();
                let rust_src = crate::interpreter::cpp_bridge::gen_dll_wrapper(&dll_str, &sigs, &struct_defs);
                let dll_bytes = crate::interpreter::cpp_bridge::compile_wrapper(&rust_src, &[])?;
                let ext = crate::partial_compiler::native_lib_ext();
                let tmp_path = std::env::temp_dir()
                    .join(format!("_tl_cpp_{:x}.{ext}", simple_hash(header_path_str)));
                std::fs::write(&tmp_path, &dll_bytes)
                    .map_err(|e| format!("CppImport: cannot write wrapper DLL: {e}"))?;
                self.load_cpp_wrapper_dll(&tmp_path, &sigs, &struct_defs, header_path_str)
            }
            _ => unreachable!(),
        }
    }

    /// C パラメータ型を tl の `PtrParam` 種別にマッピングする。
    ///
    /// `OpaqueStructPtr`（`VECTOR*` 等）も通常のポインタと同じ規則で分類する。
    /// これにより `call_native_function` の write-back パスに入り、typed ABI の
    /// Ptr 高速経路（`resolve_typed_ptr_arg`）がまず試みられる。typed 経路に
    /// 乗らない場合のみ従来のハンドルベース書き戻しにフォールバックする。
    pub(crate) fn sig_to_ptr_param_fn(ct: &crate::interpreter::cpp_bridge::CType) -> crate::interpreter::PtrParam {
        use crate::interpreter::cpp_bridge::CType;
        use crate::interpreter::PtrParam;
        match ct {
            CType::Ptr { mutable: true, .. } | CType::OpaqueStructPtr { mutable: true, .. } => {
                PtrParam::MutPtr
            }
            CType::Ptr { mutable: false, .. }
            | CType::OpaqueStructPtr { mutable: false, .. }
            | CType::CharPtr => PtrParam::ConstPtr,
            _ => PtrParam::None,
        }
    }

    /// コンパイル済み C++ ラッパー DLL をロードして名前空間を構築する。
    /// `ar_init_bridge`（あれば）でコールバックテーブルを初期化し、各関数を `NativeFunction` として登録する。
    pub(crate) fn load_cpp_wrapper_dll(
        &mut self,
        lib_path: &std::path::Path,
        sigs: &[crate::interpreter::cpp_bridge::CFnSig],
        struct_defs: &[crate::interpreter::cpp_bridge::CStructDef],
        module_name: &str,
    ) -> Result<Rc<NamespaceData>, String> {
        let lib = unsafe { libloading::Library::new(lib_path) }
            .map_err(|e| format!("CppImport: cannot load wrapper DLL: {e}"))?;

        let lib_path_buf = lib_path.to_path_buf();

        // Initialise: prefer ar_init_bridge (cpp-dll), fall back to ar_init / hv_init (compat)
        let cb_ptr = crate::interpreter::native_api::get_callbacks();
        let bridge_init = unsafe {
            lib.get::<unsafe extern "C" fn(*const crate::interpreter::native_api::ArCallbacks)>(
                b"ar_init_bridge\0",
            )
        }.or_else(|_| unsafe {
            lib.get::<unsafe extern "C" fn(*const crate::interpreter::native_api::ArCallbacks)>(b"hv_init_bridge\0")
        });
        if let Ok(f) = bridge_init {
            unsafe { f(cb_ptr) };
        } else if let Ok(f) = unsafe {
            lib.get::<unsafe extern "C" fn(*const crate::interpreter::native_api::ArCallbacks)>(b"ar_init\0")
        }.or_else(|_| unsafe {
            lib.get::<unsafe extern "C" fn(*const crate::interpreter::native_api::ArCallbacks)>(b"hv_init\0")
        }) {
            unsafe { f(cb_ptr) };
        }

        let mut members: HashMap<String, Value> = HashMap::new();

        // C 構造体名 → raw レイアウトの共有マップ。typed ABI シグネチャ（`AbiTy::Ptr`）は
        // `NativeFnRef` 越しに `Send + Sync` が要求されるため `Arc` で保持する。
        // クラス側の `raw_layout`（`Rc`）はクラス生成時に個別に複製する（構築は起動時 1 回のみ）。
        let raw_layouts: HashMap<String, Arc<crate::interpreter::value::RawLayout>> = struct_defs
            .iter()
            .filter_map(|d| d.raw_layout().map(|l| (d.name.clone(), Arc::new(l))))
            .collect();

        // Build tl class values for each C struct so that tl code can construct
        // and access struct instances, and native code can call get_global/call_fn.
        for sdef in struct_defs {
            use crate::interpreter::{ClassValue, FnValue};
            use crate::ast::Param;
            use crate::token::Span;

            let mut field_mutability: HashMap<String, bool> = HashMap::new();
            let mut field_index: HashMap<String, usize> = HashMap::new();
            let mut field_mutability_vec: Vec<bool> = Vec::new();
            let mut init_params: Vec<Param> = vec![Param {
                name: "self".to_string(),
                mutable: true,
                type_ann: None,
                default: None,
                variadic: false,
            }];
            for (i, (fname, _)) in sdef.fields.iter().enumerate() {
                field_mutability.insert(fname.clone(), true);
                field_index.insert(fname.clone(), i);
                field_mutability_vec.push(true);
                init_params.push(Param {
                    name: fname.clone(),
                    mutable: false,
                    type_ann: None,
                    default: None,
                    variadic: false,
                });
            }
            let field_count = sdef.fields.len();

            // __init__ body: `self.field = field` for each field
            let init_body: Vec<crate::ast::Stmt> = sdef
                .fields
                .iter()
                .map(|(fname, _)| crate::ast::Stmt::AttrAssign {
                    target: crate::ast::Expr::Attr {
                        object: Box::new(crate::ast::Expr::Ident("self".to_string())),
                        attr: fname.clone(),
                        span: Span::unknown(),
                    },
                    value: crate::ast::Expr::Ident(fname.clone()),
                })
                .collect();

            let init_fn = Rc::new(FnValue {
                name: "__init__".to_string(),
                params: init_params,
                body: init_body,
                is_python: false,
                captured_env: HashMap::new(),
            return_type: None,
            });

            let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
            methods.insert("__init__".to_string(), vec![init_fn]);

            // C/C++ 構造体のレイアウトが完全に判明している場合、インスタンスを
            // C ABI 準拠の raw ブロックで生成する（フィールド幅・オフセットが C と一致
            // するため、`raw.as_ptr()+8` をそのまま構造体ポインタとして渡せる）。
            // `ClassValue.raw_layout` は `Rc`（スレッド境界を越えない）なので複製する。
            let raw_layout = raw_layouts.get(&sdef.name).map(|l| Rc::new((**l).clone()));
            let cls = Rc::new(ClassValue {
                name: sdef.name.clone(),
                class_id: crate::interpreter::value::alloc_class_id(),
                bases: vec![],
                methods,
                gen_methods: HashMap::new(),
                class_vars: HashMap::new(),
                field_defaults: vec![],
                field_mutability,
                field_index,
                field_count,
                field_mutability_vec,
                field_access: HashMap::new(),
                method_access: HashMap::new(),
                static_method_names: std::collections::HashSet::new(),
                class_method_names: std::collections::HashSet::new(),
                static_vars: HashMap::new(),
                new_type_base: None,
                is_exception: false,
                raw_layout,
            });

            members.insert(sdef.name.clone(), Value::Class(cls));
        }

        for sig in sigs {
            let symbol = format!("{}_tl\0", sig.name);
            let has_sym = unsafe {
                lib.get::<unsafe extern "C" fn(*const i64, i32) -> i64>(symbol.as_bytes())
                    .is_ok()
            };
            if has_sym {
                let ptr_params: Vec<crate::interpreter::PtrParam> = sig
                    .params
                    .iter()
                    .map(|(_, ct)| Self::sig_to_ptr_param_fn(ct))
                    .collect();
                // typed ABI: 全プリミティブ + raw レイアウト既知の構造体シグネチャなら
                // {name}_typed を解決する
                let typed_sig = Self::build_cpp_typed_sig(sig, &raw_layouts);
                let typed_ptr = if typed_sig.is_some() {
                    let tsym = format!("{}_typed\0", sig.name);
                    match unsafe {
                        lib.get::<unsafe extern "C" fn(
                            *const u64,
                            *mut u64,
                            *mut crate::interpreter::native_api::ErrSlot,
                        ) -> u32>(tsym.as_bytes())
                    } {
                        Ok(f) => *f as usize,
                        Err(_) => 0,
                    }
                } else {
                    0
                };
                let fn_ref = Arc::new(NativeFnRef {
                    lib_path: lib_path_buf.clone(),
                    fn_name: sig.name.clone(),
                    n_params: sig.params.len(),
                    min_params: sig.n_required,
                    param_mutabilities: vec![false; sig.params.len()],
                    ptr_params,
                    cached_fn_ptr: std::sync::atomic::AtomicUsize::new(0),
                    typed_fn_ptr: std::sync::atomic::AtomicUsize::new(typed_ptr),
                    typed_sig: if typed_ptr != 0 { typed_sig } else { None },
                });
                members.insert(sig.name.clone(), Value::NativeFunction(fn_ref));
            }
        }

        // Register into global scope so module-level calls and get_global() from
        // native code resolve. Struct classes are registered so native wrappers can
        // call get_global("VECTOR") then call_fn to construct instances.
        for (name, value) in &members {
            self.scopes[0]
                .entry(name.clone())
                .or_insert_with(|| Var::new(value.clone(), false));
        }

        self.native_libs.insert(lib_path_buf, NativeLibWrapper(lib));

        Ok(Rc::new(NamespaceData {
            name: module_name.to_string(),
            members,
        }))
    }

    // ---------------------------------------------------------------------------
    // Block execution helpers
    // ---------------------------------------------------------------------------

}
