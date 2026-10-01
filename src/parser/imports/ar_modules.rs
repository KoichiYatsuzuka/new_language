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
    /// ⚠⚠ 以前は `source_dir` で見つからなければ `root_dir`（エントリのディレクトリ）も
    ///   探していた（2026-10-02 に外した）。エントリが上の階層にあるときだけ上を指せる、
    ///   という「どこから実行したかで結果が変わる」規則だった。上の階層は `..a` と書く。
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
        let module_base: PathBuf = module.iter().collect();
        let tlc_rel = module_base.with_extension("arc");
        let file_rel = module_base.with_extension("ar");
        let init_rel = module_base.join("__init__.ar");

        let search_dirs = self.module_search_dirs(level);

        // (パス, コンパイル済みか) の候補リスト — .arc が .ar より先になる
        let candidates: Vec<(PathBuf, bool)> = search_dirs
            .iter()
            .flat_map(|dir| {
                [
                    (dir.join(&tlc_rel), true),
                    (dir.join(&file_rel), false),
                    (dir.join(&init_rel), false),
                ]
            })
            .collect();

        let (mut abs_path, mut is_compiled) = candidates
            .iter()
            .find(|(p, _)| p.exists())
            .cloned()
            .ok_or_else(|| {
                let paths = candidates
                    .iter()
                    .map(|(p, _)| format!("'{}'", p.display()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "cannot find module '{}' (looked at {})",
                    module_path::written_spelling(level, module),
                    paths
                )
            })?;

        // ── `.arc` の陳腐化検査（#14 の「ABI ハッシュ照合」を、実際に起きる食い違いへ適用）──
        //
        // `.arc` は**ソースを埋め込んで**おり、存在すると `.ar` より優先される。
        // そのため `.ar` を編集しても再コンパイルするまで一切反映されず、しかも
        // **警告も出ずに古い答えを返す**（実測: `offset` を 100→999 に直しても古い 101.0 が出た）。
        //
        // 埋め込みソースと隣の `.ar` を突き合わせ、食い違ったら**ソース側を正**として `.ar` を使う。
        // §6.3 の「不一致ならフォールバック（再解決できなければ明示エラー）」を、
        // 回復手段（＝ソースがそこにある）が常にある本ケースへ当てはめたもの。
        if is_compiled {
            let src_sibling = abs_path.with_extension("ar");
            if let (Ok((_, embedded)), Ok(on_disk)) = (
                crate::partial_compiler::read_tlc_source(&abs_path),
                std::fs::read_to_string(&src_sibling),
            ) {
                if embedded != on_disk {
                    eprintln!(
                        "Warning: compiled module '{}' is out of date with '{}'; \
                         using the source (re-run `--compile` to refresh the .arc)",
                        abs_path.display(),
                        src_sibling.display()
                    );
                    abs_path = src_sibling;
                    is_compiled = false;
                }
            }
        }

        let (shown_path, abs_path) = Self::found_paths(&abs_path);
        let name = self.name_module_file(&abs_path, module, true)?;
        let cache_key = ("ar-auto".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache_probe(&cache_key, &abs_path)? {
            return Ok(LoadedModule { body, name });
        }

        // ソースを取得: .arc はバイナリから埋め込みソースを抽出、.ar は直読み
        let (source, filename) = if is_compiled {
            let (mod_name, src) = crate::partial_compiler::load_tlc(&abs_path)
                .map_err(|e| format!("cannot load compiled module '{}': {e}", module.join(".")))?;
            let label = format!("<compiled:{mod_name}>");
            (src, label)
        } else {
            let src = std::fs::read_to_string(&shown_path)
                .map_err(|e| format!("cannot read module '{}': {e}", module.join(".")))?;
            (src, shown_path.to_string_lossy().into_owned())
        };

        let body = self.parse_sub_module(&shown_path, &abs_path, &source, &filename, cache_key)?;
        Ok(LoadedModule { body, name })
    }

    /// `import[ar]`: `.ar` ソースのみをロードする。`.arc` があっても無視する。
    pub(crate) fn load_tl_source_module(&mut self, level: u32, module: &[String]) -> Result<LoadedModule, String> {
        let module_base: PathBuf = module.iter().collect();
        let file_rel = module_base.with_extension("ar");
        let init_rel = module_base.join("__init__.ar");

        let search_dirs = self.module_search_dirs(level);

        let candidates: Vec<PathBuf> = search_dirs
            .iter()
            .flat_map(|dir| [dir.join(&file_rel), dir.join(&init_rel)])
            .collect();

        let abs_path = candidates
            .iter()
            .find(|p| p.exists())
            .cloned()
            .ok_or_else(|| {
                let paths = candidates
                    .iter()
                    .map(|p| format!("'{}'", p.display()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "cannot find source module '{}' (looked at {})",
                    module_path::written_spelling(level, module),
                    paths
                )
            })?;

        let (shown_path, abs_path) = Self::found_paths(&abs_path);
        let name = self.name_module_file(&abs_path, module, true)?;
        let cache_key = ("ar".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache_probe(&cache_key, &abs_path)? {
            return Ok(LoadedModule { body, name });
        }

        let source = std::fs::read_to_string(&shown_path)
            .map_err(|e| format!("cannot read module '{}': {e}", module.join(".")))?;
        let filename = shown_path.to_string_lossy().into_owned();

        let body = self.parse_sub_module(&shown_path, &abs_path, &source, &filename, cache_key)?;
        Ok(LoadedModule { body, name })
    }

    /// `import[arc]`: `.arc` コンパイル済みモジュールのみをロードする。`.ar` があっても無視する。
    pub(crate) fn load_tlc_module(&mut self, level: u32, module: &[String]) -> Result<LoadedModule, String> {
        let module_base: PathBuf = module.iter().collect();
        let tlc_rel = module_base.with_extension("arc");

        let search_dirs = self.module_search_dirs(level);

        let candidates: Vec<PathBuf> =
            search_dirs.iter().map(|dir| dir.join(&tlc_rel)).collect();

        let abs_path = candidates
            .iter()
            .find(|p| p.exists())
            .cloned()
            .ok_or_else(|| {
                let paths = candidates
                    .iter()
                    .map(|p| format!("'{}'", p.display()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "cannot find compiled module '{}' (looked at {}; compile with: cargo run --release -- --compile <source.ar>)",
                    module_path::written_spelling(level, module), paths
                )
            })?;

        let (shown_path, abs_path) = Self::found_paths(&abs_path);
        let name = self.name_module_file(&abs_path, module, true)?;
        let cache_key = ("arc".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache_probe(&cache_key, &abs_path)? {
            return Ok(LoadedModule { body, name });
        }

        let (mod_name, source) = crate::partial_compiler::load_tlc(&shown_path)
            .map_err(|e| format!("cannot load compiled module '{}': {e}", module.join(".")))?;
        let filename = format!("<compiled:{mod_name}>");

        let body = self.parse_sub_module(&shown_path, &abs_path, &source, &filename, cache_key)?;
        Ok(LoadedModule { body, name })
    }

}
