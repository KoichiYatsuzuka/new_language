// imports/packages.rs — CPython と同じパッケージの扱い（2026-10-02）:
// `import a.b.c` の連鎖の読み込み・束縛・`from pkg import submodule`・`from . import x`。
//
// ## CPython の規則（ここで再現するもの）
//
// - `import a.b.c` は `a` → `a.b` → `a.b.c` の順に読み込み（各パッケージの `__init__` を 1 回だけ実行）、
//   **`a` を束縛**する。`a.b.c` は属性でたどる（サブモジュールは親パッケージの属性になる）。
// - `import a.b.c as m` は同じ順に読み込み、`m` に `a.b.c` を束縛する。
// - `from a.b import x` は `a` → `a.b` を読み込み、`x` が `a.b` の名前に無ければ
//   **サブモジュール `a.b.x` を読み込んで**束縛する。
// - `from . import x` は `x` をこのファイルのディレクトリ（パッケージ）から取り込む。
// - `__init__` の無いディレクトリは空の**名前空間パッケージ**。
//
// ## Arrow での表し方
//
// 連鎖のパッケージは**束縛しない `Stmt::Import`**（`bind: None`）として、元の文の**前に**置く
// （`Parser::pending_stmts` → `parse_program`）。実行時・型検査・型レジストリ・展開器は
// それをふつうの import として順に処理するので、パッケージの本体も同じ経路で読まれる。
// 束縛は元の文の `bind`（`ImportBind`）だけが行う。
//
// ⚠ 外部言語（cpp / cs / js / rs）のドット区切りは**ファイルの場所**であってパッケージではないので、
//   連鎖を作らず、束縛も従来どおり（別名か末尾）。計画: implementation_logs/IMPORT_RESOLUTION_PLAN.md。

use {
    crate::ast::{ImportBind, ImportOrigin, Stmt},
    crate::module_path,
    crate::parser::Parser,
    super::dispatch::LoadedModule,
    std::path::{Path, PathBuf},
};

impl Parser {
    /// 言語がパッケージの階層を持つか（[`module_path::has_packages`]）。
    pub(crate) fn has_packages(lang: &str) -> bool {
        module_path::has_packages(lang)
    }

    /// `file_dir` に書かれた import 文の探索の起点（[`Parser::import_origin`] の `file_dir` 版）。
    ///
    /// ⚠ Python のモジュールの中の import（`fill_python_imports`）は、`.ar` ではなく
    ///   **その `.py` のディレクトリ**が `file_dir`。
    pub(crate) fn origin_for(&self, file_dir: &Path, level: u32) -> ImportOrigin {
        ImportOrigin {
            level,
            base_dir: self
                .has_source_dir
                .then(|| module_path::import_base(file_dir, &self.root_dir, level)),
            span: crate::token::Span::unknown(),
        }
    }

    /// `import[lang] <dots>a.b.c [as m]` の連鎖（`a`・`a.b`）を先に読み込む文を `pending_stmts` に足し、
    /// この文の束縛を返す。
    pub(crate) fn import_chain_and_bind(
        &mut self,
        lang: &str,
        level: u32,
        module: &[String],
        alias: Option<&str>,
        loaded: &LoadedModule,
    ) -> Result<ImportBind, String> {
        let file_dir = self.source_dir.clone();
        let (pending, bind) = self.chain_and_bind(lang, &file_dir, level, module, alias, loaded)?;
        self.pending_stmts.extend(pending);
        Ok(bind)
    }

    /// [`Self::import_chain_and_bind`] の本体（`file_dir` を明示する版・Python のモジュールの中でも使う）。
    pub(crate) fn chain_and_bind(
        &mut self,
        lang: &str,
        file_dir: &Path,
        level: u32,
        module: &[String],
        alias: Option<&str>,
        loaded: &LoadedModule,
    ) -> Result<(Vec<Stmt>, ImportBind), String> {
        if !Self::has_packages(lang) {
            // 外部言語: 別名か末尾（ドット区切りはファイルの場所）。
            let name = alias
                .map(str::to_string)
                .unwrap_or_else(|| module.last().cloned().unwrap_or_default());
            return Ok((Vec::new(), ImportBind { name, module: loaded.name.clone() }));
        }
        let chain = self.package_chain(lang, file_dir, level, module, loaded)?;
        let bind = match alias {
            Some(a) => ImportBind { name: a.to_string(), module: loaded.name.clone() },
            None => ImportBind {
                // ⚠ CPython: `import a.b.c` は `a` を束縛する（`a.b.c` は属性でたどる）。
                name: module[0].clone(),
                module: chain
                    .first()
                    .map(|(name, _)| name.clone())
                    .unwrap_or_else(|| loaded.name.clone()),
            },
        };
        Ok((chain.into_iter().map(|(_, st)| st).collect(), bind))
    }

    /// 連鎖のパッケージ（`module` の真の接頭辞 `a`・`a.b`）を読み込む文（束縛しない `Stmt::Import`）。
    /// 戻り値は `(パッケージの名前, 文)` を浅い順に。
    pub(crate) fn package_chain(
        &mut self,
        lang: &str,
        file_dir: &Path,
        level: u32,
        module: &[String],
        loaded: &LoadedModule,
    ) -> Result<Vec<(Vec<String>, Stmt)>, String> {
        let mut out = Vec::new();
        for i in 1..module.len() {
            let prefix = &module[..i];
            // ⚠ **読み込み中のパッケージ**（その `__init__` の中からの import）はもう一度読まない。
            //   CPython ではパッケージの `__init__` が自分のサブモジュールを読める（初期化中の
            //   パッケージがそのまま使われる）。読みに行くと循環 import の誤りになる（実測）。
            if self.package_in_progress(lang, prefix, loaded) {
                continue;
            }
            let pkg = self.load_package(lang, level, prefix, loaded)?;
            let st = self.synthetic_import(lang, file_dir, level, prefix, &pkg);
            out.push((pkg.name, st));
        }
        Ok(out)
    }

    /// 束縛しない `Stmt::Import`（連鎖のパッケージ・`from` のサブモジュールを先に読み込むための文）。
    pub(crate) fn synthetic_import(
        &self,
        lang: &str,
        file_dir: &Path,
        level: u32,
        written: &[String],
        loaded: &LoadedModule,
    ) -> Stmt {
        Stmt::Import {
            lang: lang.to_string(),
            module: loaded.name.clone(),
            source_module: Self::written_if_renamed(level, written, &loaded.name),
            alias: None,
            body: loaded.body.clone(),
            origin: self.origin_for(file_dir, level),
            bind: None,
        }
    }

    /// パッケージ `prefix`（`a` / `a.b`）を読み込む。
    ///
    /// ⚠ パッケージのディレクトリは、モジュール本体を見つけた**同じ探索先**（`loaded.root`）の下。
    ///   Python の外部の探索先（site-packages など）で見つけたモジュールのパッケージもそこにある。
    fn load_package(
        &mut self,
        lang: &str,
        level: u32,
        prefix: &[String],
        loaded: &LoadedModule,
    ) -> Result<LoadedModule, String> {
        if lang == "py-int" {
            // ⚠ py-int は CPython 自身が読む（本体は型検査用のスタブだけ）。名前は書いた綴り。
            return self.load_python_interface_module(level, prefix);
        }
        let Some(root) = loaded.root.clone() else {
            return Ok(LoadedModule::as_written(Vec::new(), prefix));
        };
        let dir = root.join(prefix.iter().collect::<PathBuf>());
        if lang == "py" {
            // エントリ・相対の起点で見つけたならエントリ基準の名前、外部の探索先なら書いた綴り。
            let local = level > 0 || self.is_entry_dir(&root);
            return self.load_py_package(&dir, prefix, local);
        }
        self.load_ar_package(lang, &dir, prefix)
    }

    /// パッケージ `prefix` の `__init__` が**今読み込み中**か（`self.loading` に入っている）。
    fn package_in_progress(&self, lang: &str, prefix: &[String], loaded: &LoadedModule) -> bool {
        let Some(root) = loaded.root.as_ref() else { return false };
        let dir = root.join(prefix.iter().collect::<PathBuf>());
        let inits: &[&str] = match lang {
            "py" | "py-int" => &["__init__.py"],
            _ => &["__init__.ar", "__init__.arc"],
        };
        inits
            .iter()
            .any(|f| self.loading.contains(&module_path::absolute(&dir.join(f))))
    }

    /// `dir` がエントリのディレクトリ（CPython の `sys.path[0]`）か。
    pub(crate) fn is_entry_dir(&self, dir: &Path) -> bool {
        module_path::absolute(dir) == module_path::absolute(&self.root_dir)
    }

    /// `module` のファイル（またはパッケージのディレクトリ）が探索の起点に在るか（読み込まない）。
    pub(crate) fn module_exists(&self, lang: &str, file_dir: &Path, level: u32, module: &[String]) -> bool {
        if module_path::is_arrow_source_lang(lang) {
            return self.ar_module_exists(lang, level, module);
        }
        if lang == "py" || lang == "py-int" {
            return self.py_module_exists(file_dir, level, module);
        }
        false
    }

    /// `from a.b import x, y` の `x` / `y` が**サブモジュール**なら、それを先に読み込む文
    /// （CPython: 名前が `a.b` に無ければサブモジュール `a.b.x` を読み込む）。
    ///
    /// ⚠ 先に `a.b` 自身も読み込んでおく（サブモジュールは親の属性になるので、親が先に要る）。
    ///   `from` の文の実行時の読み込みはキャッシュに当たるだけ。
    pub(crate) fn from_import_submodules(
        &mut self,
        lang: &str,
        file_dir: &Path,
        level: u32,
        module: &[String],
        names: &[(String, Option<String>)],
        loaded: &LoadedModule,
    ) -> Result<Vec<Stmt>, String> {
        let exports = crate::decl_names::module_exports(&loaded.body);
        let mut out = Vec::new();
        for (orig, _) in names {
            if exports.defined.contains(orig) || exports.imported.contains(orig) {
                continue;
            }
            let sub: Vec<String> = module.iter().cloned().chain([orig.clone()]).collect();
            if !self.module_exists(lang, file_dir, level, &sub) {
                continue;
            }
            if out.is_empty() {
                out.push(self.synthetic_import(lang, file_dir, level, module, loaded));
            }
            let sl = if lang == "py" {
                self.load_python_module(file_dir, level, &sub)?
            } else {
                self.load_module(lang, level, &sub, None)?
            };
            out.push(self.synthetic_import(lang, file_dir, level, &sub, &sl));
        }
        Ok(out)
    }

    /// `from <dots> import x, y as z`（モジュールの名前の無い相対 import・CPython と同じ書き方）。
    ///
    /// - `x` がサブモジュール（`<起点>/x.ar` など）なら、それを読み込んで `x` に束縛する
    ///   （`import <dots>x as x` と同じ）
    /// - それ以外は、起点のディレクトリのパッケージ（`__init__`）の名前から取り込む
    ///
    /// 戻り値は文の並び（最後の 1 つを `parse_stmt` が返し、残りは `pending_stmts` へ）。
    pub(crate) fn from_dots_import(
        &mut self,
        lang: &str,
        file_dir: &Path,
        level: u32,
        names: Vec<(String, Option<String>)>,
    ) -> Result<Vec<Stmt>, String> {
        if !Self::has_packages(lang) {
            return Err(format!(
                "`from {} import[{lang}] ...` needs a module name (only Arrow / Python modules are packages)",
                ".".repeat(level as usize)
            ));
        }
        let mut out = Vec::new();
        let mut rest: Vec<(String, Option<String>)> = Vec::new();
        for (orig, alias) in names {
            let sub = vec![orig.clone()];
            if !self.module_exists(lang, file_dir, level, &sub) {
                rest.push((orig, alias));
                continue;
            }
            let loaded = if lang == "py" {
                self.load_python_module(file_dir, level, &sub)?
            } else {
                self.load_module(lang, level, &sub, None)?
            };
            out.push(Stmt::Import {
                lang: lang.to_string(),
                module: loaded.name.clone(),
                source_module: Some(module_path::written_spelling(level, &sub)),
                bind: Some(ImportBind {
                    name: alias.clone().unwrap_or_else(|| orig.clone()),
                    module: loaded.name.clone(),
                }),
                alias,
                body: loaded.body,
                origin: self.origin_for(file_dir, level),
            });
        }
        if !rest.is_empty() {
            // 起点のディレクトリそのもの（パッケージ）の名前から取り込む。
            let dir = module_path::import_base(file_dir, &self.root_dir, level);
            let has_init = ["__init__.ar", "__init__.arc", "__init__.py", "__init__.pyi"]
                .iter()
                .any(|f| dir.join(f).exists());
            if !has_init {
                let (name, _) = &rest[0];
                return Err(format!(
                    "cannot import name '{name}' from '{}' (no module '{name}' in '{}', and it is not a package with an __init__)",
                    ".".repeat(level as usize),
                    dir.display()
                ));
            }
            let pkg = if lang == "py" {
                self.load_py_package(&dir, &[], true)?
            } else if lang == "py-int" {
                LoadedModule::as_written(Vec::new(), &[])
            } else {
                self.load_ar_package(lang, &dir, &[])?
            };
            out.push(Stmt::FromImport {
                lang: lang.to_string(),
                module: pkg.name,
                source_module: Some(".".repeat(level as usize)),
                names: rest,
                body: pkg.body,
                origin: self.origin_for(file_dir, level),
            });
        }
        Ok(out)
    }
}
