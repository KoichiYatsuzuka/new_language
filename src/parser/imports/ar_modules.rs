// imports/ar_modules.rs — Arrow モジュール(.ar/.arc)の読み込み: load_tl_module / load_tl_source_module / load_tlc_module。

use {
    crate::parser::Parser,
    crate::ast::Stmt, crate::lexer, crate::module_path,
    super::dispatch::LoadedModule,
    std::path::{Path, PathBuf},
};

impl Parser {
    /// モジュールの探索先（**起点 1 か所だけ**・[`crate::module_path`]）。
    ///
    /// ⚠ **3 つのローダで完全に同じ**だったので #79 で 1 本化した。
    /// ⚠ 起点は [`crate::module_path::import_base`]: ドット無しはエントリのディレクトリ
    ///   （CPython の `sys.path[0]`）、ドット付きは書いたファイルのディレクトリから数える。
    fn module_search_dirs(&self, level: u32) -> Vec<PathBuf> {
        vec![self.import_base(level)]
    }

    /// 見つけたパスを、表示・読み込み用（字句的に正規化）と鍵用（絶対パス）に分ける。
    ///
    /// ⚠ 鍵（キャッシュ・循環検出）は**絶対パスに正規化**する。`pkg/../a.ar` と `a.ar` を
    ///   別物として扱うと、同じモジュールを 2 回読むうえ、`..` を含む相互 import が
    ///   循環検出をすり抜けて**無限に読み続ける**。
    /// ⚠ 表示用は相対のまま（エラーメッセージ・位置情報のファイル名を変えないため）。
    fn found_paths(found: &Path) -> (PathBuf, PathBuf) {
        let shown = module_path::normalize(found);
        let key = module_path::absolute(&shown);
        (shown, key)
    }

    /// キャッシュ命中と循環 import の検査（#79 で 3 箇所から 1 本化）。
    ///
    /// - `Ok(Some(body))` — キャッシュ命中。呼び出し側はそのまま返す。
    /// - `Ok(None)` — 続行してよい。
    /// - `Err(_)` — 循環 import。
    fn module_cache_probe(
        &self,
        cache_key: &(String, PathBuf),
        abs_path: &Path,
    ) -> Result<Option<Vec<Stmt>>, String> {
        if let Some(body) = self.module_cache.get(cache_key) {
            return Ok(Some(body.clone()));
        }
        if self.loading.contains(abs_path) {
            return Err(format!(
                "circular import detected: '{}'",
                abs_path.display()
            ));
        }
        Ok(None)
    }

    /// 取得済みのソースを**子パーサ**で解析して AST を返す — **唯一の実装**（#79）。
    ///
    /// 親のキャッシュ・循環検出セット・`root_dir`・`node_counter`・モジュール名の表を引き継ぎ、
    /// 終わったら子が作ったキャッシュを親へマージして `cache_key` に登録する。
    ///
    /// ⚠⚠ **#79 以前はこの 22 行が 3 つのローダに逐語コピーされていた**
    /// （`load_tl_module` / `load_tl_source_module` / `load_tlc_module`）。
    /// **3 つで違うのは「ソースをどこから取るか」だけ**なので、取得は呼び出し側に残し、
    /// ここには**解析と引き継ぎ**だけを置く。⚠ 下の `node_counter` の注意書きも
    /// 3 重化していた（＝ 直す人が 3 箇所とも直したか誰にも分からない形）。
    ///
    /// ⚠ `parse_program` が失敗すると `abs_path` は `loading` に**残る**。
    /// これは #79 以前からの挙動で、畳むときにそのまま保存した（変えると
    /// 「一度失敗したモジュールを再 import すると循環扱いになる」が変わる）。
    fn parse_sub_module(
        &mut self,
        shown_path: &Path,
        abs_path: &Path,
        source: &str,
        filename: &str,
        cache_key: (String, PathBuf),
    ) -> Result<Vec<Stmt>, String> {
        self.loading.insert(abs_path.to_path_buf());

        let tokens = lexer::Lexer::new(source, filename).tokenize();
        // ⚠ 子の `source_dir`（＝その中の import の探索の起点）は**そのファイルのディレクトリ**。
        let module_dir = shown_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        let mut sub = Parser::new(tokens, Some(module_dir));
        // 親のキャッシュ・循環検出セット・ルートディレクトリを引き継ぐ
        sub.module_cache = self.module_cache.clone();
        sub.loading = self.loading.clone();
        sub.root_dir = self.root_dir.clone();
        sub.module_names = std::rc::Rc::clone(&self.module_names);
        // node-id はプログラム全体で一意にする（#16・C1）。共有しないとモジュール間で
        // 衝突し、消費側が別モジュールの注釈を読む（FFI 境界検査が誤検知する）。
        sub.node_counter = self.node_counter.clone();

        // ⚠ 誤りには**そのファイルの**位置を付ける（外側の CLI が付けると `import` 文の位置になる・10-17）。
        let body = sub.parse_program().map_err(|e| {
            let at = sub.error_location(&e);
            format!("{e}{at}")
        })?;

        // 子パーサが生成したキャッシュエントリを親にマージする
        self.module_cache.extend(sub.module_cache);
        self.loading.remove(abs_path);
        self.module_cache.insert(cache_key, body.clone());

        Ok(body)
    }

    /// `.ar` / `.arc` モジュールをロードして AST を返す。
    ///
    /// 探索の起点（[`Self::module_search_dirs`]）で以下の優先順に試す:
    /// 1. `module.arc`         — コンパイル済みモジュール（埋め込みソース付きバイナリ）
    /// 2. `module.ar`          — ソースファイルモジュール
    /// 3. `module/__init__.ar` — パッケージモジュール
    pub(crate) fn load_tl_module(&mut self, level: u32, module: &[String]) -> Result<LoadedModule, String> {
        self.load_ar(ArKind::Auto, level, module)
    }

    /// `import[ar]`: `.ar` ソースのみをロードする。`.arc` があっても無視する。
    pub(crate) fn load_tl_source_module(&mut self, level: u32, module: &[String]) -> Result<LoadedModule, String> {
        self.load_ar(ArKind::Source, level, module)
    }

    /// `import[arc]`: `.arc` コンパイル済みモジュールのみをロードする。`.ar` があっても無視する。
    pub(crate) fn load_tlc_module(&mut self, level: u32, module: &[String]) -> Result<LoadedModule, String> {
        self.load_ar(ArKind::Compiled, level, module)
    }

    /// `module`（`a.b`）の `.ar` / `.arc` のファイルが探索の起点に在るか（読み込まない）。
    ///
    /// `from pkg import x` の `x` がサブモジュールかどうかの判定に使う（CPython 準拠・2026-10-02）。
    pub(crate) fn ar_module_exists(&self, lang: &str, level: u32, module: &[String]) -> bool {
        let module_base: PathBuf = module.iter().collect();
        let kind = ArKind::of(lang);
        self.module_search_dirs(level).iter().any(|dir| {
            kind.candidates(&module_base).iter().any(|(rel, _)| crate::import_fs::exists(&dir.join(rel)))
                || crate::import_fs::is_dir(&dir.join(&module_base))
        })
    }

    /// 3 つのローダの共通部分（候補の並べ方だけが `kind` で違う）。
    fn load_ar(&mut self, kind: ArKind, level: u32, module: &[String]) -> Result<LoadedModule, String> {
        let module_base: PathBuf = module.iter().collect();
        let search_dirs = self.module_search_dirs(level);

        // (探索先, パス, コンパイル済みか) の候補リスト — .arc が .ar より先になる
        let candidates: Vec<(PathBuf, PathBuf, bool)> = search_dirs
            .iter()
            .flat_map(|dir| {
                kind.candidates(&module_base)
                    .into_iter()
                    .map(move |(rel, compiled)| (dir.clone(), dir.join(rel), compiled))
            })
            .collect();

        let found = candidates.iter().find(|(_, p, _)| crate::import_fs::exists(p)).cloned();
        let Some((root, found, is_compiled)) = found else {
            // ⚠ `__init__` の無いディレクトリは**名前空間パッケージ**（CPython と同じ）。
            //   `import pkg`（pkg/ にファイルが 1 つも無くても）は空のモジュールになる。
            if let Some(dir) = search_dirs.iter().find(|d| crate::import_fs::is_dir(&d.join(&module_base))) {
                let mut loaded = self.load_ar_package(kind.lang(), &dir.join(&module_base), module)?;
                loaded.root = Some(dir.clone());
                return Ok(loaded);
            }
            let paths = candidates
                .iter()
                .map(|(_, p, _)| format!("'{}'", p.display()))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(kind.not_found(&module_path::written_spelling(level, module), &paths));
        };

        let mut loaded = self.load_found_ar(kind, found, is_compiled, module)?;
        loaded.root = Some(root);
        Ok(loaded)
    }

    /// パッケージ（ディレクトリ `dir`）を読み込む（CPython 準拠・2026-10-02）。
    ///
    /// `__init__` があればそれを読み、無ければ**空の名前空間パッケージ**（CPython の
    /// namespace package と同じ）。`written` は書いた綴り（名前を作れないときの代わり）。
    pub(crate) fn load_ar_package(
        &mut self,
        lang: &str,
        dir: &Path,
        written: &[String],
    ) -> Result<LoadedModule, String> {
        let kind = ArKind::of(lang);
        let init = kind
            .package_inits()
            .into_iter()
            .map(|(rel, compiled)| (dir.join(rel), compiled))
            .find(|(p, _)| crate::import_fs::exists(p));
        match init {
            Some((found, compiled)) => self.load_found_ar(kind, found, compiled, written),
            None => Ok(LoadedModule {
                body: Vec::new(),
                name: self.name_module_dir(dir, written, true)?,
                root: None,
            }),
        }
    }

    /// 見つけた `.ar` / `.arc` を読み込む（陳腐化検査・名前・キャッシュ・子パーサ）。
    fn load_found_ar(
        &mut self,
        kind: ArKind,
        found: PathBuf,
        is_compiled: bool,
        module: &[String],
    ) -> Result<LoadedModule, String> {
        let (mut found, mut is_compiled) = (found, is_compiled);
        // ── `.arc` の陳腐化検査（#14 の「ABI ハッシュ照合」を、実際に起きる食い違いへ適用）──
        //
        // `.arc` は**ソースを埋め込んで**おり、存在すると `.ar` より優先される。
        // そのため `.ar` を編集しても再コンパイルするまで一切反映されず、しかも
        // **警告も出ずに古い答えを返す**（実測: `offset` を 100→999 に直しても古い 101.0 が出た）。
        //
        // 埋め込みソースと隣の `.ar` を突き合わせ、食い違ったら**ソース側を正**として `.ar` を使う。
        // §6.3 の「不一致ならフォールバック（再解決できなければ明示エラー）」を、
        // 回復手段（＝ソースがそこにある）が常にある本ケースへ当てはめたもの。
        // ⚠ `import[arc]`（`ArKind::Compiled`）は `.arc` を強制するので見ない。
        if is_compiled && kind == ArKind::Auto {
            let src_sibling = found.with_extension("ar");
            if let (Ok(arc), Ok(on_disk)) = (
                crate::arc_format::read(&found),
                crate::import_fs::read_to_string(&src_sibling),
            ) {
                if arc.source != on_disk {
                    eprintln!(
                        "Warning: compiled module '{}' is out of date with '{}'; \
                         using the source (re-run `--compile` to refresh the .arc)",
                        found.display(),
                        src_sibling.display()
                    );
                    found = src_sibling;
                    is_compiled = false;
                }
            }
        }

        let (shown_path, abs_path) = Self::found_paths(&found);
        let name = self.name_module_file(&abs_path, module, true)?;
        let cache_key = (kind.cache_lang().to_string(), abs_path.clone());

        if let Some(body) = self.module_cache_probe(&cache_key, &abs_path)? {
            return Ok(LoadedModule { body, name, root: None });
        }

        // ソースを取得: .arc はバイナリから埋め込みソースを抽出、.ar は直読み
        let (source, filename) = if is_compiled {
            let (mod_name, src) = Self::load_arc_source(&shown_path)
                .map_err(|e| format!("cannot load compiled module '{}': {e}", module.join(".")))?;
            (src, format!("<compiled:{mod_name}>"))
        } else {
            let src = crate::import_fs::read_to_string(&shown_path)
                .map_err(|e| format!("cannot read module '{}': {e}", module.join(".")))?;
            (src, shown_path.to_string_lossy().into_owned())
        };

        let body = self.parse_sub_module(&shown_path, &abs_path, &source, &filename, cache_key)?;
        Ok(LoadedModule { body, name, root: None })
    }
}

impl Parser {
    /// `.arc` の名前と埋め込みソース。
    ///
    /// CLI は実行時のために埋め込み DLL も登録する（`partial_compiler::load_tlc`）。拡張（`editor`）は
    /// 型を読むだけなので形式を読むだけ（`crate::arc_format`・LLVM / DLL を扱う `partial_compiler` を持たない）。
    fn load_arc_source(path: &Path) -> std::io::Result<(String, String)> {
        #[cfg(not(feature = "editor"))]
        return crate::partial_compiler::load_tlc(path);
        #[cfg(feature = "editor")]
        return crate::arc_format::read(path).map(|arc| (arc.module_name, arc.source));
    }
}

/// `.ar` / `.arc` のローダの種類（`import` / `import[ar]` / `import[arc]`）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum ArKind {
    /// `.arc` を優先し、無ければ `.ar`（`ar-auto` / `tl-auto`）
    Auto,
    /// `.ar` だけ（`ar` / `tl`）
    Source,
    /// `.arc` だけ（`arc` / `tlc`）
    Compiled,
}

impl ArKind {
    fn of(lang: &str) -> Self {
        match lang {
            "ar" | "tl" => ArKind::Source,
            "arc" | "tlc" => ArKind::Compiled,
            _ => ArKind::Auto,
        }
    }

    fn lang(self) -> &'static str {
        match self {
            ArKind::Auto => "ar-auto",
            ArKind::Source => "ar",
            ArKind::Compiled => "arc",
        }
    }

    /// キャッシュの鍵の言語名（以前からの値を保つ）。
    fn cache_lang(self) -> &'static str {
        self.lang()
    }

    /// `module`（`a/b`）の候補（パス, コンパイル済みか）。
    fn candidates(self, module_base: &Path) -> Vec<(PathBuf, bool)> {
        let arc = (module_base.with_extension("arc"), true);
        let ar = (module_base.with_extension("ar"), false);
        let init = (module_base.join("__init__.ar"), false);
        match self {
            ArKind::Auto => vec![arc, ar, init],
            ArKind::Source => vec![ar, init],
            ArKind::Compiled => vec![arc],
        }
    }

    /// パッケージのディレクトリの中の `__init__` の候補（[`Self::candidates`] の `module/__init__.ar` と揃える）。
    fn package_inits(self) -> Vec<(PathBuf, bool)> {
        match self {
            ArKind::Auto | ArKind::Source => vec![(PathBuf::from("__init__.ar"), false)],
            ArKind::Compiled => vec![(PathBuf::from("__init__.arc"), true)],
        }
    }

    fn not_found(self, written: &str, paths: &str) -> String {
        match self {
            ArKind::Auto => format!("cannot find module '{written}' (looked at {paths})"),
            ArKind::Source => format!("cannot find source module '{written}' (looked at {paths})"),
            ArKind::Compiled => format!(
                "cannot find compiled module '{written}' (looked at {paths}; compile with: cargo run --release -- --compile <source.ar>)"
            ),
        }
    }
}
