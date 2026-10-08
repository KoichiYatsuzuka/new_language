// imports/py_modules.rs — Python モジュール(.py/.pyi)の読み込み: load_python_module / load_python_interface_module / load_py_type_body。

use {
    crate::parser::Parser,
    crate::ast::Stmt, crate::module_path, crate::python_converter,
    super::dispatch::LoadedModule,
    std::path::{Path, PathBuf},
};
use super::*;

impl Parser {
    /// Python の import の探索先（[`crate::module_path`] の規則・CPython と同じ）。
    ///
    /// - ドット無し … エントリのディレクトリ（CPython の `sys.path[0]`）→ 外部の探索先
    ///   （[`Self::python_search_dirs_from`]）。書いたファイルのディレクトリは**探さない**
    /// - ドット付き … 起点だけ（`from_dir` から数える。Python の相対 import と同じく外部は見ない）
    ///
    /// 戻り値の先頭が「エントリ・相対の起点」の探索先（そこで見つけたモジュールの名前は
    /// エントリのディレクトリ基準になる・[`Self::name_module_file`]）。
    fn python_import_dirs(&self, from_dir: &Path, level: u32) -> Vec<PathBuf> {
        if module_path::uses_external_paths(level) {
            let entry = self.root_dir.clone();
            self.python_search_dirs_from(&entry)
        } else {
            vec![module_path::search_base(from_dir, level)]
        }
    }

    /// `module` の `.py`（またはパッケージのディレクトリ）が探索先に在るか（読み込まない）。
    pub(crate) fn py_module_exists(&self, from_dir: &Path, level: u32, module: &[String]) -> bool {
        let module_base: PathBuf = module.iter().collect();
        self.python_import_dirs(from_dir, level).iter().any(|d| {
            d.join(module_base.with_extension("py")).exists()
                || d.join(module_base.with_extension("pyi")).exists()
                || d.join(&module_base).is_dir()
        })
    }

    /// Python モジュールを検索・変換する（キャッシュ込み）。
    /// [`Self::python_import_dirs`] の順に .py または __init__.py を探す。
    ///
    /// `from_dir` は import 文を書いたファイルのディレクトリ（相対 import の起点）。`.ar` から
    /// 読むときはその `.ar` の、Python モジュールの中の import（[`Self::fill_python_imports`]）では
    /// **その `.py` の**ディレクトリ。
    pub(crate) fn load_python_module(
        &mut self,
        from_dir: &Path,
        level: u32,
        module: &[String],
    ) -> Result<LoadedModule, String> {
        let module_base: PathBuf = module.iter().collect();
        let rel_py   = module_base.with_extension("py");
        let rel_init = module_base.join("__init__.py");
        let search_dirs = self.python_import_dirs(from_dir, level);

        // 検索ディレクトリを順に試して最初に見つかった .py / __init__.py を使う
        let found = search_dirs
            .iter()
            .enumerate()
            .flat_map(|(i, d)| [(i, d.join(&rel_py)), (i, d.join(&rel_init))])
            .find(|(_, p)| p.exists());
        let Some((found_at, found_path)) = found else {
            // ⚠ `__init__.py` の無いディレクトリは**名前空間パッケージ**（CPython と同じ）。
            if let Some((i, d)) = search_dirs.iter().enumerate().find(|(_, d)| d.join(&module_base).is_dir()) {
                let mut pkg = self.load_py_package(&d.join(&module_base), module, i == 0)?;
                pkg.root = Some(d.clone());
                return Ok(pkg);
            }
            let looked = search_dirs
                .iter()
                .flat_map(|d| [d.join(&rel_py), d.join(&rel_init)])
                .map(|p| format!("'{}'", p.display()))
                .collect::<Vec<_>>()
                .join(", ");
            // ⚠ C 拡張（`sys` / `numpy` …）は `.py` の実体が無いのでここに来る。
            //   何が起きたのか判るように、代替手段まで書く。
            return Err(format!(
                "cannot find Python source for module '{}' \
                 — `import[py]` translates `.py` sources, so modules without one \
                 (C extensions such as sys, numpy, ...) cannot be loaded this way; \
                 use `import[py-int]` to call them through CPython instead \
                 (looked at {})",
                module_path::written_spelling(level, module),
                looked
            ));
        };
        // 先頭の探索先（エントリ・相対の起点）で見つけたか（モジュールの名前の付け方が変わる）。
        let mut loaded = self.load_found_py(&found_path, module, found_at == 0)?;
        loaded.root = Some(search_dirs[found_at].clone());
        Ok(loaded)
    }

    /// Python のパッケージ（ディレクトリ `dir`）を読み込む。`__init__.py` が無ければ
    /// 空の名前空間パッケージ（CPython と同じ）。
    pub(crate) fn load_py_package(
        &mut self,
        dir: &Path,
        written: &[String],
        local: bool,
    ) -> Result<LoadedModule, String> {
        let init = dir.join("__init__.py");
        if init.exists() {
            return self.load_found_py(&init, written, local);
        }
        Ok(LoadedModule {
            body: Vec::new(),
            name: self.name_module_dir(dir, written, local)?,
            root: None,
        })
    }

    /// 見つけた `.py` を変換して読み込む（標準ライブラリの拒否・名前・キャッシュ・中の import の充填）。
    fn load_found_py(
        &mut self,
        found_path: &Path,
        module: &[String],
        local: bool,
    ) -> Result<LoadedModule, String> {
        let shown_path = module_path::normalize(found_path);
        let abs_path = module_path::absolute(&shown_path);

        // ⚠⚠ **標準ライブラリは明示エラーで止める**（項目 27）。
        //   stdlib は検索パスに入っている（`python_lib_dirs`）ので `.py` は**見つかる**。
        //   だが中身は `from ... import *`・C 拡張への委譲・動的な仕掛けの塊で、
        //   変換器が通ることはまず無い。黙って翻訳を試みると
        //   **stdlib のソースを指すエラー**（`.../Lib/os.py: ...`）が出て、
        //   利用者は自分のコードのどこが悪いのか判らない。
        //   ⇒ 「stdlib は翻訳しない」と最初に言い切る。
        //   ⚠ site-packages（purelib）は**対象外**。純 Python のパッケージは
        //     翻訳できる可能性があるので、従来どおり試す。
        if is_python_stdlib_path(&abs_path) {
            return Err(format!(
                "module '{}' is part of the Python standard library ('{}') \
                 — `import[py]` translates `.py` sources to Arrow and does not handle \
                 the standard library; use `import[py-int] {}` to call it through \
                 CPython instead",
                module.join("."),
                abs_path.display(),
                module.join("."),
            ));
        }

        let name = self.name_module_file(&abs_path, module, local)?;
        let cache_key = ("py".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(LoadedModule { body: body.clone(), name, root: None });
        }

        if self.loading.contains(&abs_path) {
            return Err(format!("circular import detected: '{}'", shown_path.display()));
        }

        let source = std::fs::read_to_string(&shown_path)
            .map_err(|e| format!("cannot read '{}': {e}", shown_path.display()))?;

        self.loading.insert(abs_path.clone());

        let filename = shown_path.to_string_lossy().to_string();
        // ⚠ 変換が失敗しても `self.loading` を残さない（次の import で偽の循環になる）。
        let converted = python_converter::convert_python_source(&source, &filename);
        let mut body = match converted {
            Ok(b) => b,
            Err(e) => {
                self.loading.remove(&abs_path);
                return Err(e);
            }
        };
        // ★ Python モジュール内の `import` を**再帰で充填**する（項目 27）。
        //   ⚠ `self.loading` に自分が入ったままここを通るので、循環 import は
        //     既存の検出（`circular import detected`）がそのまま効く。
        let py_dir = shown_path.parent().map(Path::to_path_buf).unwrap_or_default();
        let filled = self.fill_python_imports(&mut body, &py_dir);
        self.loading.remove(&abs_path);
        filled?;
        self.module_cache.insert(cache_key, body.clone());

        Ok(LoadedModule { body, name, root: None })
    }

    /// Python から変換した body の中の `import[py]` / `from ... import[py]` の
    /// `body` を**再帰で充填**する（項目 27）。
    ///
    /// ⚠⚠ **見つからないモジュールは明示エラー**（`load_python_module` が出す）。
    /// stdlib や C 拡張（`os` / `sys` / `numpy` …）は翻訳対象の `.py` が無いのでここで
    /// 止まる。以前は import を丸ごと捨てていたので、**未定義の名前が別の行で
    /// `NameError` になる**という判りにくい壊れ方をしていた。
    ///
    /// ⚠ 走査するのは**モジュール本体の直下だけ**。Python の関数内 import は
    /// 遅延読み込みの意図があり、変換器は文の位置を保つので、そこはそのまま残る
    /// （＝現状は充填されず、実行時に未定義になる。必要になったら別途対応する）。
    ///
    /// ⚠ 規則は CPython と同じ（[`crate::module_path`]）: ドット無しはエントリのディレクトリ
    ///   （`sys.path[0]`）と外部の探索先から、`from .m import x`（変換器が `origin.level` に
    ///   段数を載せる）は**その `.py` のディレクトリ**（`py_dir`）から。
    /// ⚠ `import a.b` はパッケージの連鎖を先に読み込む文を**手前に足す**（`packages.rs`）ので、
    ///   本体の文の並びを作り直す。
    fn fill_python_imports(&mut self, body: &mut Vec<Stmt>, py_dir: &Path) -> Result<(), String> {
        let old = std::mem::take(body);
        for mut stmt in old {
            let mut before: Vec<Stmt> = Vec::new();
            match &mut stmt {
                Stmt::Import { lang, module, source_module, alias, body: sub, origin, bind }
                    if lang == "py" && sub.is_empty() =>
                {
                    let level = origin.level;
                    let written = module.clone();
                    let loaded = self.load_python_module(py_dir, level, &written)?;
                    let (chain, b) =
                        self.chain_and_bind("py", py_dir, level, &written, alias.as_deref(), &loaded)?;
                    before = chain;
                    *bind = Some(b);
                    *source_module = Self::written_if_renamed(level, &written, &loaded.name);
                    *module = loaded.name;
                    *sub = loaded.body;
                    *origin = self.origin_for(py_dir, level);
                }
                Stmt::FromImport { lang, module, source_module, names, body: sub, origin }
                    if lang == "py" && sub.is_empty() =>
                {
                    let level = origin.level;
                    let written = module.clone();
                    let loaded = self.load_python_module(py_dir, level, &written)?;
                    before = self
                        .package_chain("py", py_dir, level, &written, &loaded)?
                        .into_iter()
                        .map(|(_, st)| st)
                        .collect();
                    before.extend(
                        self.from_import_submodules("py", py_dir, level, &written, names, &loaded)?,
                    );
                    *source_module = Self::written_if_renamed(level, &written, &loaded.name);
                    *module = loaded.name;
                    *sub = loaded.body;
                    *origin = self.origin_for(py_dir, level);
                }
                _ => {}
            }
            body.append(&mut before);
            body.push(stmt);
        }
        Ok(())
    }

    /// `import[py-int]` 用: .pyi を優先して検索し、なければ .py にフォールバックする。
    /// `__init__.pyi` / `__init__.py` も検索対象に含める。
    /// body は型検査専用（実行時は PyO3 経由で別ロジックが動く）。
    ///
    /// ⚠ 名前は書いた綴りのまま（実行時は CPython がその名前で import する）。
    pub(crate) fn load_python_interface_module(&mut self, level: u32, module: &[String]) -> Result<LoadedModule, String> {
        self.load_python_interface_body(level, module)
            .map(|body| LoadedModule::as_written(body, module))
    }

    fn load_python_interface_body(&mut self, level: u32, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let from_dir = self.source_dir.clone();
        let search_dirs = self.python_import_dirs(&from_dir, level);

        // 候補パスを生成: module.pyi, module/__init__.pyi, module.py, module/__init__.py
        let candidates: Vec<(PathBuf, bool)> = {
            let mut v = Vec::new();
            for dir in &search_dirs {
                v.push((dir.join(module_base.with_extension("pyi")), true));
                v.push((dir.join(module_base.join("__init__.pyi")), true));
            }
            for dir in &search_dirs {
                v.push((dir.join(module_base.with_extension("py")), false));
                v.push((dir.join(module_base.join("__init__.py")), false));
            }
            v
        };

        for (abs_path, is_pyi) in candidates {
            if !abs_path.exists() { continue; }
            return self.load_py_type_body(module, &module_path::normalize(&abs_path), is_pyi);
        }
        // ⚠⚠ **型の出所が無ければエラー**（`editor_import_resolution_plan.md` 1-2）。以前は黙って空の body
        //    （型なし・実行時は PyO3 に任せる）にしていた。未解決の import は黙って型情報を落とさない。
        let not_found = || {
            format!(
                "import[py-int] '{}': cannot find its types (looked for '{}.pyi' / '{}.py' and a package \
                 '__init__' in {}); put a '.pyi' stub next to the importing file",
                module.join("."),
                module_base.display(),
                module_base.display(),
                search_dirs
                    .iter()
                    .map(|d| if d.as_os_str().is_empty() { "'.'".to_string() } else { format!("'{}'", d.display()) })
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        // ⚠ `__init__` の無いディレクトリは**名前空間パッケージ**（CPython・PEP 420）。解決できている
        //   （メンバーはサブモジュールだけ）ので誤りにしない。`import[py-int] pkg.mod` はパッケージの
        //   連鎖で `pkg` をここへ通す。
        if search_dirs.iter().any(|d| d.join(&module_base).is_dir()) {
            return Ok(vec![]);
        }
        // ⚠ 同梱スタブは「どこにも無い外部のモジュール」の代わり。相対の書き方では引かない。
        if !module_path::uses_external_paths(level) {
            return Err(not_found());
        }

        // ★ ファイルが 1 つも見つからなかったときだけ**同梱スタブ**を引く（#19 / 群6 S2）。
        //   ⚠ **利用者のファイルが常に勝つ**ので、この順序を崩さないこと。
        //     自分で `time.pyi` を置けば上書きできる。
        //   ⚠ `time` / `math` は C 組み込みで `.py` が存在せず、検索ディレクトリを
        //     いくら辿っても空振りする ＝ ここが唯一の供給源。
        if let Some(src) = crate::py_stubs::builtin_stub(module) {
            // キャッシュ／循環検出はパスで引くので、実在しない**疑似パス**を割り当てる。
            let pseudo = PathBuf::from("<builtin>").join(format!("{}.pyi", module.join(".")));
            return self.build_py_type_body(module, &pseudo, src, true, true);
        }

        Err(not_found())
    }

    /// Python ソースファイルから型検査用の body を生成する。
    ///
    /// - `.pyi` ファイル: `python_converter` でベストエフォート変換 + スタブで補完
    /// - `.py` ファイル: `extract_py_type_stubs` でシグネチャを直接抽出（`python_converter` は使わない）
    pub(crate) fn load_py_type_body(
        &mut self,
        module: &[String],
        abs_path: &PathBuf,
        is_pyi: bool,
    ) -> Result<Vec<Stmt>, String> {
        let cache_key = ("py-int".to_string(), abs_path.clone());
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }
        if self.loading.contains(abs_path) {
            return Ok(vec![]);
        }
        let source = std::fs::read_to_string(abs_path).map_err(|_| {
            format!("cannot read interface file for module '{}'", module.join("."))
        })?;
        self.build_py_type_body(module, abs_path, &source, is_pyi, false)
    }

    /// 読み込み済みのソース文字列から型検査用の body を組む。
    ///
    /// `load_py_type_body`（ファイル版）と**同梱スタブ**（`py_stubs`）の共通部分。
    /// `bundled` はエラー報告の出し分けに使う（群6 S4）。
    fn build_py_type_body(
        &mut self,
        module: &[String],
        abs_path: &PathBuf,
        source: &str,
        is_pyi: bool,
        bundled: bool,
    ) -> Result<Vec<Stmt>, String> {
        let cache_key = ("py-int".to_string(), abs_path.clone());
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }
        if self.loading.contains(abs_path) {
            return Ok(vec![]);
        }
        self.loading.insert(abs_path.clone());

        let body = if is_pyi {
            // .pyi: python_converter でベストエフォート変換
            let filename = abs_path.to_string_lossy().to_string();
            // ⚠⚠ **変換エラーを握り潰して**行ベース抽出へ落ちる経路（群6 S4）。
            //   利用者の `.pyi` は今までどおり黙って精度を落とすだけにするが、
            //   **同梱スタブは自分で書いたもの**なので、失敗したら警告を出す
            //   （黙って精度が落ちると「スタブを整備したのに検査が効かない」に気付けない）。
            let mut converted = match python_converter::convert_python_source(source, &filename) {
                Ok(stmts) => stmts,
                // ⚠ **同梱スタブは自分で書いたもの**なので、変換できないのは誤り（以前は警告を出して
                //   行単位の抽出へ落ちていた・`editor_import_resolution_plan.md` 1-2）。
                Err(e) if bundled => {
                    self.loading.remove(abs_path);
                    return Err(format!("bundled stub for '{}' failed to convert: {e}", module.join(".")));
                }
                Err(_) => Vec::new(),
            };
            // スタブで不足を補完（変換できなかった関数を追加）
            let known: std::collections::HashSet<String> = converted
                .iter()
                .filter_map(|s| if let Stmt::FnDef { name, .. } = s { Some(name.clone()) } else { None })
                .collect();
            for stub in extract_py_type_stubs(source) {
                if let Stmt::Let(ref name, _, _, _) = stub {
                    if !known.contains(name.as_str()) {
                        converted.push(stub);
                    }
                }
            }
            converted
        } else {
            // .py: 直接スタブ抽出（python_converter は複雑な構文に対応できないため使わない）
            extract_py_type_stubs(source)
        };

        self.loading.remove(abs_path);
        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

}
