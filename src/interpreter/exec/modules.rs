// exec/modules.rs — モジュール読み込みの実行: import 文、ネイティブ/C++/C# ブリッジ DLL のロードと型シグネチャ構築。

use crate::ast::Resolution;
use {
    std::collections::HashMap, std::path::{Path, PathBuf},
    std::rc::Rc, std::sync::Arc,
    crate::ast::{ImportBind, ImportOrigin, Stmt},
    crate::interpreter::{
        ExecResult,
        Interpreter, ModuleState, NamespaceData, NativeFnRef, NativeLibWrapper, Value, Var,
    },
};
use super::*;

impl Interpreter {
    /// `import[lang] a.b as c` の実行（#58 で `exec` から切り出し）。
    ///
    /// ⚠ **言語ごとに違うのは「名前空間の作り方」だけ**で、束縛（`declare_var`）は共通。
    /// #58 以前は cs-dll / cs-proc / js-proc の 3 経路が名前空間を作った直後に
    /// **自分で `declare_var` して早期 return** していたが、その 3 箇所の束縛名は
    /// 共通の末尾と**逐語で同じ式**だった（`alias` か `module.last()`）。⇒ 早期 return を畳み、
    /// 各経路は**名前空間を返すだけ**にしてある。束縛名は今はパーサが決める（`bind`・
    /// 束縛名を計算していた `import_bind_name` は 2026-10-02 に削除済み）。
    ///
    /// ⚠ 束縛は AST の `bind`（パーサが決める・CPython 準拠）。`import a.b.c` は**パッケージ `a`**
    ///   を束縛し、`a.b.c` は属性でたどる（[`Self::attach_submodule`]）。`None` は束縛しない
    ///   （パッケージの連鎖を読み込むためにパーサが足した文）。
    pub(crate) fn exec_import(
        &mut self,
        lang: &str,
        module: &[String],
        source_module: Option<&str>,
        body: &[Stmt],
        origin: &ImportOrigin,
        bind: Option<&ImportBind>,
    ) -> Result<ExecResult, String> {
        // ⚠ FFI 系の import は `native` 限定（評価コア切り出し #2）。
        //   評価コアビルドでは `_` へ落とさず**明示エラー**にする。落とすと
        //   `exec_module` が普通のモジュールとして探しに行き、理由の分からない
        //   「ファイルが無い」になる。
        let ns = match lang {
            #[cfg(feature = "native")]
            "cpp-dll" | "cpp-lib" => self.import_cpp_module(lang, module, source_module)?,
            #[cfg(feature = "native")]
            "cs-dll" => self.import_cs_dll(lang, module, body, origin)?,
            #[cfg(feature = "native")]
            "cs-proc" => self.import_cs_proc(lang, module, body, origin)?,
            #[cfg(feature = "native")]
            "js-proc" => self.import_js_proc(lang, module, body, origin)?,
            #[cfg(not(feature = "native"))]
            "cpp-dll" | "cpp-lib" | "cs-dll" | "cs-proc" | "js-proc" => {
                let _ = source_module;
                return Err(format!(
                    "ImportError: `import[{lang}]` is not available in the evaluation core                      build (FFI is excluded; rebuild with the `native` feature)"
                ));
            }
            _ => self.exec_module(lang, module, body, origin)?,
        };
        self.attach_submodule(module, &ns);
        let Some(bind) = bind else {
            return Ok(ExecResult::Normal);
        };
        // 束縛されるのがこのモジュールそのものか、連鎖の先頭のパッケージ（`import a.b` の `a`）か。
        let bound = if bind.module.as_slice() == module {
            ns
        } else {
            self.loaded_module(&bind.module).ok_or_else(|| {
                format!(
                    "ImportError: package '{}' is not loaded (internal: the parser should load it before '{}')",
                    bind.module.join("."),
                    module.join(".")
                )
            })?
        };
        self.declare_var(bind.name.clone(), Var::new(Value::Namespace(bound), false));
        Ok(ExecResult::Normal)
    }

    /// 読み込み済みのモジュール（`module` の名前で、言語を問わず）。
    fn loaded_module(&self, module: &[String]) -> Option<Rc<NamespaceData>> {
        let key = PathBuf::from(module.join("/"));
        self.module_cache.iter().find_map(|((_, path), state)| match state {
            ModuleState::Loaded(ns) if *path == key => Some(ns.clone()),
            _ => None,
        })
    }

    /// 読み込んだモジュールを、**親パッケージの属性**にする（CPython の `sys.modules` と同じ）。
    ///
    /// `a.b` を読み込むと、`a` の名前空間から `b` で引ける（[`Self::namespace_member`]）。
    /// ⚠ どこで読み込んでも付く（CPython と同じく、プログラム全体で 1 つの表）。
    /// ⚠ 名前空間の値には持たせない（`Interpreter::submodules`）。持たせると親と子が互いを
    ///   指す循環になり、非同期タスクへの深い複製（`Value::deep_clone`）が止まらない。
    /// ⚠ 親が**読み込み中**でも付ける。パッケージの `__init__` が自分のサブモジュールを読む形
    ///   （`from .core import X`）では、サブモジュールの方が先に読み終わる（CPython と同じ）。
    /// ⚠ 親を読み込んでいない（外部言語のドット区切り・エントリより上の `__parent__`）なら何もしない。
    pub(crate) fn attach_submodule(&mut self, module: &[String], ns: &Rc<NamespaceData>) {
        let Some((last, parent)) = module.split_last() else { return };
        let parent_key = PathBuf::from(parent.join("/"));
        if parent.is_empty() || !self.module_cache.keys().any(|(_, p)| *p == parent_key) {
            return;
        }
        self.submodules
            .entry(parent.join("."))
            .or_default()
            .insert(last.clone(), Value::Namespace(ns.clone()));
    }

    /// この import 文の**実行時の探索先**（[`crate::module_path`] の規則）。
    ///
    /// 起点（import 文を書いたファイルのディレクトリ。ドット付きなら段数を適用済み）を先頭に、
    /// ドット無しの書き方なら `python.search_paths`（起点から祖先へ遡って最初の設定）を続ける。
    /// 消費者は `import[py-int]` の `sys.path`・C# のブリッジ／ホスト探索。
    ///
    /// ⚠⚠ 以前はここが**エントリのディレクトリ**（`Self::python_search_dirs`）だった。
    ///   構文解析時はファイルの近くで見つけたのに、実行時は別の場所を探すことになり、
    ///   サブディレクトリのファイルが書いた `import[py-int]` だけが実行時に失敗した。
    /// ⚠ 起点を持たない import 文（合成した文・テストが組んだ AST）だけは、
    ///   登録済みの探索先（`Self::python_search_dirs`）を使う。
    fn import_search_dirs(&self, origin: &ImportOrigin) -> Vec<PathBuf> {
        let Some(base) = &origin.base_dir else {
            return self.python_search_dirs().cloned().collect();
        };
        #[allow(unused_mut)]
        let mut dirs = vec![base.clone()];
        // ⚠ `ar_config` は `native` 限定（評価コアには `.ar_config` を読む意味が無い・#6）。
        #[cfg(feature = "native")]
        if crate::module_path::uses_external_paths(origin.level) {
            for p in crate::ar_config::load_python_search_paths(base) {
                if !dirs.contains(&p) {
                    dirs.push(p);
                }
            }
        }
        dirs
    }


    /// `import[cpp-dll]` / `import[cpp-lib]`。
    /// ⚠ キャッシュキーは**ヘッダのパス**（`exec_module` の `module.join("/")` ではない）。
    // ⚠ FFI 専用。`native` 限定（評価コア切り出し #2）。
    #[cfg(feature = "native")]
    fn import_cpp_module(
        &mut self,
        lang: &str,
        module: &[String],
        source_module: Option<&str>,
    ) -> Result<Rc<NamespaceData>, String> {
        let file_path = module.first().map(|s| s.as_str()).unwrap_or("");
        let cache_key = (lang.to_string(), PathBuf::from(file_path));
        if let Some(ModuleState::Loaded(cached)) = self.module_cache.get(&cache_key).cloned() {
            return Ok(cached);
        }
        self.module_cache
            .insert(cache_key.clone(), ModuleState::Loading);
        let ns = self.load_cpp_module(lang, file_path, source_module)?;
        self.module_cache
            .insert(cache_key, ModuleState::Loaded(ns.clone()));
        Ok(ns)
    }

    /// `import[cs-dll]`。スタブ名前空間を作ってから NativeAOT ブリッジ DLL を探して読み込み、
    /// 見つかった DLL のパスを各クラスへ焼き込む。
    ///
    /// ⚠ **ブリッジが無い／読み込めない場合はスタブのまま返す**（警告のみ・従来どおり）。
    // ⚠ FFI 専用。`native` 限定（評価コア切り出し #2）。
    #[cfg(feature = "native")]
    fn import_cs_dll(
        &mut self,
        lang: &str,
        module: &[String],
        body: &[Stmt],
        origin: &ImportOrigin,
    ) -> Result<Rc<NamespaceData>, String> {
        let stub_ns = self.exec_module(lang, module, body, origin)?;
        let managed_name = module.last().unwrap();
        let native_dll_name = format!("{managed_name}_native.dll");
        let sub_dir: PathBuf = module[..module.len().saturating_sub(1)].iter().collect();

        let dirs = self.import_search_dirs(origin);
        let Some(bp) = Self::find_cs_dll_bridge(&dirs, &sub_dir, &native_dll_name) else {
            return Ok(stub_ns);
        };
        if let Err(e) = crate::interpreter::cs_dll_runtime::load_bridge(&bp) {
            eprintln!("Warning: cs-dll bridge not loaded: {e}");
            return Ok(stub_ns);
        }
        let ns = Self::inject_class_var(&stub_ns, "__cs_bridge_path__", &bp.to_string_lossy());
        // 戻り値のハンドルを `CsObject` に包むときにクラスを引けるようにする（2-1）
        crate::interpreter::cs_values::register_classes(&bp, &ns);
        Ok(ns)
    }

    /// `{Name}_native.dll` の探索（`import[cs-dll]`）。
    ///
    /// ⚠ **`find_cs_proc_host` の探索とは順序も候補も違う**ので畳んでいない（#58）。
    /// こちらは 1 候補名を探索先（[`Self::import_search_dirs`]）の順に探す。
    /// ⚠ 以前は探索先がエントリのディレクトリで、見つからなければ CWD 側へ落ちていた
    ///   （2026-10-02 に外した・[`crate::module_path`]）。
    // ⚠ FFI 専用。`native` 限定（評価コア切り出し #2）。
    #[cfg(feature = "native")]
    fn find_cs_dll_bridge(dirs: &[PathBuf], sub_dir: &Path, native_dll_name: &str) -> Option<PathBuf> {
        for search_dir in dirs {
            let c = search_dir.join(sub_dir).join(native_dll_name);
            if c.exists() {
                return Some(c);
            }
            let c2 = search_dir.join(native_dll_name);
            if c2.exists() {
                return Some(c2);
            }
        }
        None
    }

    /// `import[cs-proc]`。スタブ名前空間を作ってからホスト実行ファイルを探して起動し、
    /// そのパスを各クラスへ焼き込む。
    ///
    /// ⚠ **ホストが無い／起動できない場合はスタブのまま返す**（警告のみ・従来どおり）。
    // ⚠ FFI 専用。`native` 限定（評価コア切り出し #2）。
    #[cfg(feature = "native")]
    fn import_cs_proc(
        &mut self,
        lang: &str,
        module: &[String],
        body: &[Stmt],
        origin: &ImportOrigin,
    ) -> Result<Rc<NamespaceData>, String> {
        let stub_ns = self.exec_module(lang, module, body, origin)?;
        let managed_name = module.last().unwrap();
        let sub_dir: PathBuf = module[..module.len().saturating_sub(1)].iter().collect();

        let dirs = self.import_search_dirs(origin);
        let Some(pp) = Self::find_cs_proc_host(&dirs, module, managed_name, &sub_dir) else {
            return Ok(stub_ns);
        };
        if let Err(e) = crate::interpreter::cs_proc_runtime::launch_proc(&pp) {
            eprintln!("Warning: cs-proc host not started: {e}");
            return Ok(stub_ns);
        }
        let ns = Self::inject_class_var(&stub_ns, "__cs_proc_path__", &pp.to_string_lossy());
        // 戻り値のハンドルを `CsObject` に包むときにクラスを引けるようにする（2-1）
        crate::interpreter::cs_values::register_classes(&pp, &ns);
        Ok(ns)
    }

    /// `{Name}_proc.exe` → `{Name}.exe` の順で探す（`import[cs-proc]`）。
    ///
    /// ⚠ **`find_cs_dll_bridge` とは探索順が違う**（畳まない理由）。こちらは
    /// **候補名ごと**に全探索先（[`Self::import_search_dirs`]）を回すので、
    /// `{Name}_proc.exe` がどこかにあれば `{Name}.exe` が先の探索先にあっても前者が勝つ。
    /// さらに単一セグメントのときだけ `<dir>/{Name}/{exe}` も見る。
    /// ⚠ 以前は探索先がエントリのディレクトリで、見つからなければ CWD 側へ落ちていた
    ///   （2026-10-02 に外した・[`crate::module_path`]）。
    // ⚠ FFI 専用。`native` 限定（評価コア切り出し #2）。
    #[cfg(feature = "native")]
    fn find_cs_proc_host(
        dirs: &[PathBuf],
        module: &[String],
        managed_name: &str,
        sub_dir: &Path,
    ) -> Option<PathBuf> {
        let candidates_names = [
            format!("{managed_name}_proc.exe"),
            format!("{managed_name}.exe"),
        ];
        for name in &candidates_names {
            for search_dir in dirs {
                let c = search_dir.join(sub_dir).join(name);
                if c.exists() {
                    return Some(c);
                }
                let c2 = search_dir.join(name);
                if c2.exists() {
                    return Some(c2);
                }
                // Single-segment: also try <dir>/name_dir/exe_name
                if module.len() == 1 {
                    let c3 = search_dir.join(managed_name).join(name);
                    if c3.exists() {
                        return Some(c3);
                    }
                }
            }
        }
        None
    }

    /// `import[js-proc]`。Node.js の IPC サブプロセス経由で JS モジュールを呼ぶ。
    /// 1. `ar_config.json` から node_path と bridge_script を読む
    /// 2. ブリッジプロセスを起動（キャッシュ済みなら再利用）
    /// 3. `list` 操作でエクスポート関数名を取得し `Value::JsProcFn` として登録
    ///
    /// ⚠ **設定が無い／起動できない場合はスタブ（`exec_module` の結果）へ落ちる**（従来どおり）。
    // ⚠ FFI 専用。`native` 限定（評価コア切り出し #2）。
    #[cfg(feature = "native")]
    fn import_js_proc(
        &mut self,
        lang: &str,
        module: &[String],
        body: &[Stmt],
        origin: &ImportOrigin,
    ) -> Result<Rc<NamespaceData>, String> {
        // `ar_config.json`（`javascript`）は探索の起点から**祖先へ**探す
        // （`python.search_paths` / `csharp.lib_paths` と同じ方針・[`crate::module_path`]）。
        // ⚠ 以前はエントリのディレクトリ（`python_search_dirs`）だけを見ていた。
        let search_dirs: Vec<PathBuf> = match &origin.base_dir {
            Some(base) => crate::module_path::absolute(base)
                .ancestors()
                .map(Path::to_path_buf)
                .collect(),
            None => self.python_search_dirs().cloned().collect(),
        };
        let (node_exe, bridge_script, bridge_root) = match find_js_config(&search_dirs)
        {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("Warning: js-proc: {e}");
                return self.exec_module(lang, module, body, origin);
            }
        };
        // ⚠ ブリッジの鍵は**起動前**に確定させる（起動の成否に依らず同じ鍵になる）。
        let bridge_key = bridge_script
            .canonicalize()
            .unwrap_or_else(|_| bridge_script.clone())
            .to_string_lossy()
            .into_owned();

        if let Err(e) = crate::interpreter::js_proc_runtime::launch_proc(
            &node_exe,
            &bridge_script,
            &bridge_root,
        ) {
            eprintln!("Warning: js-proc bridge not started: {e}");
            return self.exec_module(lang, module, body, origin);
        }

        // JS ファイルの場所（[`crate::module_path`] の規則）:
        // 起点にあればその**絶対パス**をブリッジへ渡す（Node.js の require がそのまま読む）。
        // 無ければ、ドット無しの書き方に限りブリッジ側の解決（bridge_root・npm・組み込み）へ任せる。
        let module_name = match Self::find_local_js_module(origin, module) {
            Some(p) => p.to_string_lossy().into_owned(),
            None if crate::module_path::uses_external_paths(origin.level) => module.join("/"),
            None => {
                return Err(format!(
                    "ImportError: cannot find JavaScript module '{}' (looked in '{}')",
                    crate::module_path::written_spelling(origin.level, module),
                    origin.base_dir.as_deref().unwrap_or(Path::new("")).display()
                ));
            }
        };
        let fn_names =
            crate::interpreter::js_proc_runtime::list_functions(&bridge_key, &module_name)
                .unwrap_or_else(|e| {
                    eprintln!("Warning: js-proc list_functions: {e}");
                    vec![]
                });

        let mut members = HashMap::new();
        for fn_name in fn_names {
            members.insert(
                fn_name.clone(),
                Value::JsProcFn(Box::new(crate::interpreter::JsProcData {
                    bridge_key: bridge_key.clone(),
                    module_name: module_name.clone(),
                    fn_name,
                })),
            );
        }
        Ok(Rc::new(NamespaceData {
            name: module.join("."),
            members,
            live: None,
        }))
    }

    /// 探索の起点にある JS モジュール（`a/b.js` / `a/b.cjs` / `a/b/`）の絶対パス（`import[js-proc]`）。
    // ⚠ FFI 専用。`native` 限定（評価コア切り出し #2）。
    #[cfg(feature = "native")]
    fn find_local_js_module(origin: &ImportOrigin, module: &[String]) -> Option<PathBuf> {
        let base = origin.base_dir.as_ref()?;
        let rel: PathBuf = module.iter().collect();
        let stem = base.join(&rel);
        [stem.with_extension("js"), stem.with_extension("cjs"), stem.clone()]
            .into_iter()
            .find(|p| p.exists())
            .map(|p| crate::module_path::absolute(&p))
    }

    /// 名前空間中の**全クラス**に class 変数を 1 本注入した複製を返す（#58）。
    /// cs-dll / cs-proc がブリッジのパスをクラスへ焼き込むのに使う
    /// （#58 以前は**逐語で同じ 10 行**が 2 箇所にあった）。
    fn inject_class_var(ns: &Rc<NamespaceData>, key: &str, value: &str) -> Rc<NamespaceData> {
        let mut patched = (**ns).clone();
        for val in patched.members.values_mut() {
            if let Value::Class(cls) = val {
                let mut new_cls = cls.deep_clone();
                new_cls
                    .class_vars
                    .insert(key.to_string(), Value::str(value.to_string()));
                *val = Value::Class(Rc::new(new_cls));
            }
        }
        Rc::new(patched)
    }

    /// モジュールの body を孤立スコープで実行し、`Value::Namespace` を返す。
    /// キャッシュを使用し、循環 import はエラーにする。
    /// 読み込み済みのモジュール（大域の添字 `g`）に、`body` にあってまだ定義されていない
    /// 具体化（`Box[int]` のような名前の宣言）を定義する（タスク 2-16・`exec_module` の読み込み済みの経路）。
    fn define_missing_instances(&mut self, g: u32, body: &[Stmt]) -> Result<(), String> {
        let pending: Vec<&Stmt> = body
            .iter()
            .filter(|st| match st {
                Stmt::ClassDef { name, template_params, .. }
                | Stmt::FnDef { name, template_params, .. }
                | Stmt::GenDef { name, template_params, .. } => {
                    template_params.is_empty() && name.ends_with(']')
                }
                _ => false,
            })
            .collect();
        if pending.is_empty() {
            return Ok(());
        }
        let locals = self.scopes.split_off(1);
        let floor = std::mem::replace(&mut self.frame_floor, 1);
        let prev = self.switch_globals(g);
        let mut result = Ok(());
        for st in pending {
            let name = match st {
                Stmt::ClassDef { name, .. } | Stmt::FnDef { name, .. } | Stmt::GenDef { name, .. } => name,
                _ => continue,
            };
            if self.scopes[0].contains_key(name) {
                continue;
            }
            if let Err(e) = self.exec(st) {
                result = Err(e);
                break;
            }
        }
        self.scopes.truncate(1);
        self.switch_globals(prev);
        self.scopes.extend(locals);
        self.frame_floor = floor;
        result
    }

    /// モジュールの本体を今の大域で走らせる（`exec_module` / `try_load_native_module` 共通）。
    ///
    /// #42: モジュール本体も**バイトコード VM で実行する**。以前はここが丸ごと
    /// ツリーウォークで、`for`/`if` などの制御フローがツリーウォークで動いていた
    /// （`AR_TW_STATS` の `tw_control_flow` で実測）。定義文（`fn`/`class`/`import`）は
    /// 設計上インタプリタが実行する（#10-d）ので `try_run_module_stmt` が `None` を返す。
    fn run_module_body(
        &mut self,
        body: &[Stmt],
        module_globals: &HashMap<String, bool>,
        what: &str,
        module: &[String],
    ) -> Result<(), String> {
        for stmt in body {
            let res = match self.try_run_module_stmt(stmt, module_globals)? {
                Some(r) => r,
                None => self.exec(stmt)?,
            };
            if let ExecResult::Raise(_) = res {
                return Err(format!("RuntimeError: exception during {what}: {}", module.join(".")));
            }
        }
        Ok(())
    }

    pub(crate) fn exec_module(
        &mut self,
        lang: &str,
        module: &[String],
        body: &[Stmt],
        origin: &ImportOrigin,
    ) -> Result<Rc<NamespaceData>, String> {
        let cache_key = (lang.to_string(), PathBuf::from(module.join("/")));

        match self.module_cache.get(&cache_key).cloned() {
            Some(ModuleState::Loading) => {
                return Err(format!(
                    "RuntimeError: circular import detected: '{}'",
                    module.join(".")
                ));
            }
            Some(ModuleState::Loaded(ns)) => {
                // ⚠⚠ 読み込み済みでも、この `import` 文の本体にだけある具体化は定義する（タスク 2-16）。
                //   展開器は具体化を「その枠で最初の `import` 文」の本体に置くが、そのモジュールが
                //   **別のモジュールの中で先に**読み込まれていると、ここは 2 回目の読み込みになり
                //   本体は走らない。実行時の具体化は廃止した（D36）ので、ここで定義しないと
                //   その具体化はどこにも無くなる。
                if let Some(&g) = self.module_globals_ids.get(&cache_key) {
                    self.define_missing_instances(g, body)?;
                }
                return Ok(ns);
            }
            None => {}
        }

        self.module_cache
            .insert(cache_key.clone(), ModuleState::Loading);

        if lang == "py-int" {
            // ⚠ `sys.path` へ足すのは**この import 文の**探索先（[`Self::import_search_dirs`]）。
            let search_dirs: Vec<PathBuf> = self.import_search_dirs(origin);
            let ns = crate::interpreter::py_interop::load_py_int_module(module, &search_dirs)?;
            self.module_cache
                .insert(cache_key, ModuleState::Loaded(ns.clone()));
            return Ok(ns);
        }

        // ar-auto / arc / rs: native payload in cache → try to load natively
        // ("tl-auto" / "tlc" は旧名の別名)
        if lang == "ar-auto" || lang == "tl-auto" || lang == "arc" || lang == "tlc" || lang == "rs" {
            let module_name = module.join(".");
            // .arc files store only the stem as their module name; fall back to last segment.
            // ⚠ ネイティブ払い出しの取り出しは `native` 限定（評価コア切り出し #6）。
            //   評価コアには `partial_compiler` が無いので、常に「払い出し無し」とする。
            #[cfg(feature = "native")]
            let native_data = crate::partial_compiler::take_native_bytes(&module_name)
                .or_else(|| {
                    let stem = module.last().map(|s| s.as_str()).unwrap_or("");
                    if stem != module_name { crate::partial_compiler::take_native_bytes(stem) } else { None }
                });
            #[cfg(not(feature = "native"))]
            let native_data: Option<(Vec<String>, ())> = None;
            // ⚠ ネイティブ払い出し（`.arc` 同梱 DLL）の読み込みは `native` 限定
            //   （評価コア切り出し #2）。評価コアでは DLL を読めないので、
            //   **ネイティブ部分を無視して通常のモジュールとして続行する**
            //   （`.arc` は AST も持っているので解析は成立する）。
            #[cfg(feature = "native")]
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
        // ⚠⚠ **モジュールの本体は自分の大域で走らせる**（名前空間の分離・2026-09-26）。
        //   以前はスコープを 1 つ積んで走らせ、終わったらモジュールの名前を**呼び出し側の大域へ
        //   流し込んでいた**（モジュールの関数が同じモジュールの名前を引くため）。その結果、
        //   `import m` だけで呼び出し側の名前空間が侵されていた（実測）:
        //   - 同名の `let` / `const` が `already declared` になる
        //   - 同名の `fn` が多重定義として合成され、**呼び出し側の関数がモジュールの関数に乗っ取られる**
        //   - 修飾なしの名前（`helper(2)`・モジュールが import した `o`）が呼び出し側から見える
        //   - 逆に、モジュールの本体・関数が呼び出し側の名前を引けてしまう
        //   今は関数値が定義したモジュールの大域を持ち、呼び出しの間だけそれに差し替える
        //   （`FnValue::globals`・`switch_globals`）。呼び出し側へは名前空間の値だけを渡す。
        let frame = self.enter_module_frame(module);
        self.module_globals_ids.insert(cache_key.clone(), self.cur_globals);
        // 診断フック（#10-d）: ここから先は import モジュール本体（メイン最上位と区別して計上）。
        let _mod_guard = crate::interpreter::tw_stats::enabled()
            .then(crate::interpreter::tw_stats::ModuleBodyGuard::new);
        let module_globals = crate::interpreter::resolver::toplevel_declared_globals(body);
        let result = self.run_module_body(body, &module_globals, "module initialization", module);
        // ⚠ モジュールの中の import で束縛した名前も名前空間のメンバー（再エクスポート・CPython 準拠）。
        //   2026-10-02 のフェーズ 2 では外していたが、フェーズ 4 で CPython と同じに戻した。
        let members = self.module_members(&module_globals);
        // `mut` の名前は今の値をモジュールの大域から読む（`NamespaceData::live`・10-11）。
        let mutable_names: std::collections::HashSet<String> = members
            .keys()
            .filter(|n| self.scopes[0].get(n.as_str()).is_some_and(|v| v.is_mutable()))
            .cloned()
            .collect();
        let live = (!mutable_names.is_empty()).then(|| (self.cur_globals, Rc::new(mutable_names)));
        self.leave_module_frame(frame);
        self.in_python_module = prev_in_python;
        result?;

        let ns = Rc::new(NamespaceData {
            name: module.join("."),
            members,
            live,
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
    // ⚠ FFI 専用。`native` 限定（評価コア切り出し #2）。
    #[cfg(feature = "native")]
    pub(crate) fn build_cpp_typed_sig(
        sig: &crate::interpreter::cpp_bridge::CFnSig,
        raw_layouts: &HashMap<String, Arc<crate::interpreter::value::RawLayout>>,
        struct_names: &Arc<[Arc<str>]>,
    ) -> Option<crate::interpreter::value::TypedSig> {
        use crate::interpreter::cpp_bridge::CType;
        use crate::interpreter::value::{AbiTy, TypedSig};
        // ⚠ `cpp_header::types::typed_abi_ok` と同じ対応にすること（適格と判定したのに組めないと中継 DLL と食い違う）
        let elem_of = |ct: &CType| -> Option<Arc<str>> { ct.ptr_elem_name().map(|e| Arc::from(e.as_str())) };
        let ret = match &sig.ret {
            CType::Void => AbiTy::Void,
            // 中継コードが C の幅と符号どおりに受け取り、i64 に拡張して返す（1-6）
            CType::Int { .. } => AbiTy::I64,
            CType::Float | CType::Double => AbiTy::F64,
            // 4-4: `bool`・`char*` / `wchar_t*`（→ bytes）・ほかのポインタ（→ 借用の Ptr）
            CType::Bool => AbiTy::Bool,
            CType::CharPtr { wide, .. } => AbiTy::CStr { wide: *wide },
            CType::VoidPtr | CType::OpaqueStructPtr { .. } | CType::Ptr { .. } => AbiTy::PtrVal { elem: elem_of(&sig.ret)? },
            // 4-5: `ArPtrProtocol` の値返し → `Option[Ptr[T]]`（要素の型名はそのヘッダの構造体と照合する）
            CType::PtrProtocol { by_pointer: false, .. } => AbiTy::ProtocolRet { structs: struct_names.clone() },
            _ => return None,
        };
        let mut ptys = Vec::with_capacity(sig.params.len());
        for (_, ct) in &sig.params {
            match ct {
                CType::Int { .. } => ptys.push(AbiTy::I64),
                CType::Float | CType::Double => ptys.push(AbiTy::F64),
                // 正確なレイアウトの構造体ポインタはインスタンス（ゼロコピー・シャドウ）か `Ptr`、レイアウトの分からない
                // 構造体ポインタは `Ptr` だけ（4-4）
                CType::OpaqueStructPtr { type_name, mutable } => match raw_layouts.get(type_name) {
                    Some(layout) => ptys.push(AbiTy::Ptr {
                        mutable: *mutable,
                        by_value: false,
                        layout: layout.clone(),
                        elem: Arc::from(type_name.as_str()),
                    }),
                    None => ptys.push(AbiTy::PtrVal { elem: Arc::from(type_name.as_str()) }),
                },
                CType::ByValueStruct { type_name } => {
                    let layout = raw_layouts.get(type_name)?.clone();
                    ptys.push(AbiTy::Ptr { mutable: false, by_value: true, layout, elem: Arc::from(type_name.as_str()) });
                }
                // 4-4: `bool`・`const char*`（bytes）・`void*`（Ptr）・`ArPtrProtocol`
                CType::Bool => ptys.push(AbiTy::Bool),
                CType::CharPtr { wide: false, mutable: false } => ptys.push(AbiTy::Bytes),
                CType::VoidPtr => ptys.push(AbiTy::PtrVal { elem: Arc::from("void") }),
                CType::PtrProtocol { by_pointer, mutable } => {
                    ptys.push(AbiTy::Protocol { writable: *by_pointer && *mutable })
                }
                // プリミティブ書き込みポインタ（`int*` / `double*`）: OutPtr スロット。
                // 構造体ポインタと混在する関数（例: v3_norm(const V3*, double*)）を
                // typed 経路の対象にする — ハンドル経路は構造体ポインタ引数を扱えない。
                // 幅は rust_extern_type と同じ規約（C の幅と符号どおり・`long` は環境の幅・1-6）。
                // ⚠ 以前は int を 4 バイト・long を 8 バイトに決め打ちしていて、`short*` へ 4 バイト、
                //   Windows の `long*` へ 8 バイト書いていた。
                CType::Ptr { inner, mutable: true } => {
                    use crate::interpreter::value::RawWidth;
                    let width = match **inner {
                        CType::Int { width, signed } => RawWidth::from_ann(
                            crate::interpreter::cpp_bridge::int_raw_ann(width, signed),
                        )?,
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
    // ⚠ FFI 専用。`native` 限定（評価コア切り出し #2）。
    #[cfg(feature = "native")]
    pub(crate) fn try_load_native_module(
        &mut self,
        module: &[String],
        body: &[Stmt],
        lib_path: &std::path::Path,
    ) -> Result<Rc<NamespaceData>, String> {
        let lib = unsafe { libloading::Library::new(lib_path) }
            .map_err(|e| format!("libloading: {e}"))?;

        let lib_path_buf = lib_path.to_path_buf();

        // #42: ネイティブモジュールのスタブ本体も同じ経路で実行する。
        // ⚠ 自分の大域で走らせる（`exec_module` と同じ・名前空間の分離）。
        let frame = self.enter_module_frame(module);
        let module_globals = crate::interpreter::resolver::toplevel_declared_globals(body);
        let result = self.run_module_body(body, &module_globals, "native module init", module);
        let mut members = self.module_members(&module_globals);
        self.leave_module_frame(frame);
        result?;

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
                            param_defaults: Vec::new(),
                            param_mutabilities: params.iter().map(|p| p.mutable).collect(),
                            ptr_params: vec![crate::interpreter::PtrParam::None; params.len()],
                            cached_fn_ptr: std::sync::atomic::AtomicUsize::new(initial_ptr),
                            typed_fn_ptr: std::sync::atomic::AtomicUsize::new(typed_ptr),
                            typed_sig: if typed_ptr != 0 { typed_sig } else { None },
                            unavailable: None,
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
                            param_defaults: Vec::new(),
                            param_mutabilities: params.iter().map(|p| p.mutable).collect(),
                            ptr_params: vec![crate::interpreter::PtrParam::None; params.len()],
                            cached_fn_ptr: std::sync::atomic::AtomicUsize::new(initial_ptr),
                            typed_fn_ptr: std::sync::atomic::AtomicUsize::new(0),
                            typed_sig: None,
                            unavailable: None,
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
            if !self.scopes[0].contains_key(name) {
                self.scopes[0].insert(name.clone(), Var::new(value.clone(), false));
            }
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

        self.native_libs.insert(lib_path_buf, std::sync::Arc::new(NativeLibWrapper(lib)));

        let ns = Rc::new(NamespaceData {
            name: module.join("."),
            members,
            live: None,
        });
        Ok(ns)
    }

    // ---------------------------------------------------------------------------
    // C++ bridge module loading
    // ---------------------------------------------------------------------------

    /// C++ ライブラリ（`cpp-lib`）または DLL（`cpp-dll`）を tl モジュールとしてロードする。
    /// ヘッダーをパースして関数シグネチャを収集し、ラッパー DLL を構築・ロードして名前空間を返す。
    // ⚠ FFI 専用。`native` 限定（評価コア切り出し #2）。
    #[cfg(feature = "native")]
    pub(crate) fn load_cpp_module(
        &mut self,
        lang: &str,
        header_path_str: &str,
        _source_module: Option<&str>,
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
        // 関数・構造体の集め方は型スタブ（`parse_cpp_import`）と同じ 1 つの関数（`collect_header_decls`・7-28）
        let decls = crate::cpp_header::collect_header_decls(&raw_str, header_dir, lang, &config)
            .map_err(|e| format!("CppImport: {e}"))?;
        for (inc_path, added) in &decls.included {
            if *added > 0 {
                eprintln!("CppImport: {added} additional function(s) from '{}'", inc_path.display());
            }
        }
        let (sigs, struct_defs) = (decls.sigs, decls.structs);

        match lang {
            "cpp-lib" => {
                // Build tl_{stem}.dll next to the header (permanent cache).
                if sigs.is_empty() {
                    eprintln!("CppImport: no supported functions found in '{header_path_str}'");
                }
                eprintln!("CppImport[{lang}]: {} function(s) total", sigs.len());

                // ⚠ 全部の関数を渡す（7-28）。シムのコンパイルで外れた関数は中継 DLL に記号が無く、`load_cpp_wrapper_dll` が
                //   呼ぶと理由つきの誤りになる関数として登録する（以前は外れた関数を渡さず、「属性が無い」になっていた）
                let (dll_path, _effective_sigs) =
                    crate::interpreter::cpp_bridge::compile_tl_dll(header_path, &sigs, &struct_defs, &config)?;
                self.load_cpp_wrapper_dll(&dll_path, &sigs, &struct_defs, header_path_str)
            }
            "cpp-dll" => {
                // Find the DLL by stem next to the header and wrap it dynamically.
                let stem = header_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("lib");
                let dll_rel = header_dir.join(format!("{stem}.dll"));
                // 中継 DLL は %TEMP% に置かれ、生成ローダーは相対パスを中継 DLL 自身の場所から引く
                // （ffi_value_exchange_plan.md 1-5）。⇒ **絶対パスにして渡す**。1-5 では相対パスのまま渡していて、
                // ヘッダを相対パスで import した cpp-dll が読み込めなくなっていた（1-3 の例題で発見）。
                let dll_path = std::path::absolute(&dll_rel).map_err(|e| {
                    format!("CppImport: cannot resolve '{}': {e}", dll_rel.display())
                })?;
                // 読み込めない DLL を渡すと、中継 DLL の初期化（`ar_init_cpp`）が panic してプロセスごと落ちる。
                // 先に読み込めるかを確かめて、普通のエラーにする。
                if let Err(e) = unsafe { libloading::Library::new(&dll_path) } {
                    return Err(format!(
                        "CppImport: cannot load '{}' (the DLL next to the header): {e}",
                        dll_path.display()
                    ));
                }
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
    // ⚠ FFI 専用。`native` 限定（評価コア切り出し #2）。
    #[cfg(feature = "native")]
    pub(crate) fn sig_to_ptr_param_fn(
        ct: &crate::interpreter::cpp_bridge::CType,
        raw_layouts: &HashMap<String, Arc<crate::interpreter::value::RawLayout>>,
    ) -> crate::interpreter::PtrParam {
        use crate::interpreter::cpp_bridge::CType;
        use crate::interpreter::PtrParam;
        match ct {
            // 書き戻すもの（名前つきの `mut` 変数を求める）: 数値の書き込みポインタ・インスタンスを渡す（正確なレイアウトの）
            // 構造体ポインタ・`char*` の出力バッファ（4-7）。⚠ レイアウトの分からない構造体ポインタは `Ptr` を渡して何も書き
            // 戻さない（4-3）ので含めない（含めると `let` の `Ptr` を渡せなくなる）
            CType::Ptr { mutable: true, .. } | CType::CharPtr { wide: false, mutable: true } => PtrParam::MutPtr,
            CType::OpaqueStructPtr { mutable: true, type_name } if raw_layouts.contains_key(type_name) => {
                PtrParam::MutPtr
            }
            CType::Ptr { mutable: false, .. }
            | CType::OpaqueStructPtr { mutable: false, .. }
            | CType::CharPtr { .. } => PtrParam::ConstPtr,
            _ => PtrParam::None,
        }
    }

    /// コンパイル済み C++ ラッパー DLL をロードして名前空間を構築する。
    /// `ar_init_bridge`（あれば）でコールバックテーブルを初期化し、各関数を `NativeFunction` として登録する。
    // ⚠ FFI 専用。`native` 限定（評価コア切り出し #2）。
    #[cfg(feature = "native")]
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
        // `ArPtrProtocol` の要素の型名と照合する構造体の名前（4-5・D-12）
        let struct_names: Arc<[Arc<str>]> = struct_defs.iter().map(|d| Arc::from(d.name.as_str())).collect();
        let raw_layouts: HashMap<String, Arc<crate::interpreter::value::RawLayout>> = struct_defs
            .iter()
            .filter_map(|d| d.raw_layout().map(|l| (d.name.clone(), Arc::new(l))))
            .collect();

        // Build tl class values for each C struct so that tl code can construct
        // and access struct instances, and native code can call get_global/call_fn.
        for sdef in struct_defs {
            // 中身の見えない構造体（前方宣言だけ・4-3）は Arrow のクラスにしない（C の側が作り、`Ptr[T]` で受け渡す）
            if sdef.fields.is_empty() {
                continue;
            }
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
                        object: Box::new(crate::ast::Expr::Ident { name: "self".to_string(), node_id: 0, res: Resolution::Unresolved }),
                        attr: fname.clone(),
                        span: Span::unknown(),
                        cache: Default::default(),
                        node_id: 0, // #16: 合成/変換コード（注釈対象外）
                    },
                    value: crate::ast::Expr::Ident { name: fname.clone(), node_id: 0, res: Resolution::Unresolved },
                    span: crate::token::Span::unknown(),
                })
                .collect();

            let init_fn = Rc::new(FnValue {
                globals: self.cur_globals,
                owner_class: None,
                name: "__init__".to_string(),
                params: init_params,
                body: std::rc::Rc::from(init_body),
                is_python: false,
                captured_env: HashMap::new(),
            return_type: None,
            vm_chunk: None,
            });

            let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
            methods.insert("__init__".to_string(), vec![init_fn]);

            // C/C++ 構造体のレイアウトが完全に判明している場合、インスタンスを
            // C ABI 準拠の raw ブロックで生成する（フィールド幅・オフセットが C と一致
            // するため、`raw.as_ptr()+8` をそのまま構造体ポインタとして渡せる）。
            // `ClassValue.raw_layout` は `Rc`（スレッド境界を越えない）なので複製する。
            let raw_layout = raw_layouts.get(&sdef.name).map(|l| Rc::new((**l).clone()));
            // ポインタのフィールドを持つ構造体は、配列の要素ごとにフィールドを変換するための C のレイアウト（7-14）
            let c_struct = match raw_layout {
                Some(_) => None,
                None => Some(Arc::new(sdef.c_struct_layout(struct_defs))),
            };
            let cls = Rc::new(ClassValue {
                methods,
                field_mutability,
                field_index,
                field_count,
                field_mutability_vec,
                static_method_names: std::collections::HashSet::new(),
                class_method_names: std::collections::HashSet::new(),
                raw_layout,
                c_struct,
                ..ClassValue::synthetic(sdef.name.clone(), crate::interpreter::value::alloc_class_id())
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
                    .map(|(_, ct)| Self::sig_to_ptr_param_fn(ct, &raw_layouts))
                    .collect();
                // typed ABI: 全プリミティブ + raw レイアウト既知の構造体シグネチャなら
                // {name}_typed を解決する
                let typed_sig = Self::build_cpp_typed_sig(sig, &raw_layouts, &struct_names);
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
                    param_defaults: sig.defaults.clone(),
                    param_mutabilities: vec![false; sig.params.len()],
                    ptr_params,
                    cached_fn_ptr: std::sync::atomic::AtomicUsize::new(0),
                    typed_fn_ptr: std::sync::atomic::AtomicUsize::new(typed_ptr),
                    typed_sig: if typed_ptr != 0 { typed_sig } else { None },
                    unavailable: None,
                });
                members.insert(sig.name.clone(), Value::NativeFunction(fn_ref));
            } else {
                // 中継 DLL に入らなかった関数（7-28）: 型スタブにはあるので、呼ぶと理由つきの誤りにする（名前空間から消すと
                // 「属性が無い」になって理由が分からない）。未対応のシグネチャは型検査が先に止める
                let why = sig.unsupported_reason(struct_defs).unwrap_or_else(|| {
                    "the generated C++ shim does not compile it (often a Windows macro of the same name, such as \
                     `SetWindowText`; see the CppBridge messages when the bridge was built)"
                        .to_string()
                });
                let fn_ref = Arc::new(NativeFnRef {
                    lib_path: lib_path_buf.clone(),
                    fn_name: sig.name.clone(),
                    n_params: sig.params.len(),
                    min_params: sig.n_required,
                    param_defaults: sig.defaults.clone(),
                    param_mutabilities: vec![false; sig.params.len()],
                    ptr_params: vec![crate::interpreter::PtrParam::None; sig.params.len()],
                    cached_fn_ptr: std::sync::atomic::AtomicUsize::new(0),
                    typed_fn_ptr: std::sync::atomic::AtomicUsize::new(0),
                    typed_sig: None,
                    unavailable: Some(why),
                });
                members.insert(sig.name.clone(), Value::NativeFunction(fn_ref));
            }
        }

        // Register into global scope so module-level calls and get_global() from
        // native code resolve. Struct classes are registered so native wrappers can
        // call get_global("VECTOR") then call_fn to construct instances.
        for (name, value) in &members {
            if !self.scopes[0].contains_key(name) {
                self.scopes[0].insert(name.clone(), Var::new(value.clone(), false));
            }
        }

        self.native_libs.insert(lib_path_buf, std::sync::Arc::new(NativeLibWrapper(lib)));

        Ok(Rc::new(NamespaceData {
            name: module_name.to_string(),
            members,
            live: None,
        }))
    }

    // ---------------------------------------------------------------------------
    // Block execution helpers
    // ---------------------------------------------------------------------------

}
