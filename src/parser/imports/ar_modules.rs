// imports/ar_modules.rs — Arrow モジュール(.ar/.arc)の読み込み: load_tl_module / load_tl_source_module / load_tlc_module。

use {
    crate::parser::Parser,
    crate::ast::Stmt, crate::lexer,
    std::path::PathBuf,
};

impl Parser {
    /// `.ar` / `.arc` モジュールをロードして AST を返す。
    ///
    /// 各検索ディレクトリ (`source_dir` → `root_dir`) に対して以下の優先順で試す:
    /// 1. `module.arc`         — コンパイル済みモジュール（埋め込みソース付きバイナリ）
    /// 2. `module.ar`          — ソースファイルモジュール
    /// 3. `module/__init__.ar` — パッケージモジュール
    pub(crate) fn load_tl_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let tlc_rel = module_base.with_extension("arc");
        let file_rel = module_base.with_extension("ar");
        let init_rel = module_base.join("__init__.ar");

        // 検索ディレクトリリスト（source_dir と root_dir が同じなら重複させない）
        let search_dirs: Vec<PathBuf> = {
            let a = self.source_dir.clone();
            let b = self.root_dir.clone();
            if a == b { vec![a] } else { vec![a, b] }
        };

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

        let (abs_path, is_compiled) = candidates
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
                    module.join("."),
                    paths
                )
            })?;

        let cache_key = ("ar-auto".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        if self.loading.contains(&abs_path) {
            return Err(format!(
                "circular import detected: '{}'",
                abs_path.display()
            ));
        }

        // ソースを取得: .arc はバイナリから埋め込みソースを抽出、.ar は直読み
        let (source, filename) = if is_compiled {
            let (mod_name, src) = crate::partial_compiler::load_tlc(&abs_path)
                .map_err(|e| format!("cannot load compiled module '{}': {e}", module.join(".")))?;
            let label = format!("<compiled:{mod_name}>");
            (src, label)
        } else {
            let src = std::fs::read_to_string(&abs_path)
                .map_err(|e| format!("cannot read module '{}': {e}", module.join(".")))?;
            (src, abs_path.to_string_lossy().into_owned())
        };

        self.loading.insert(abs_path.clone());

        let tokens = lexer::Lexer::new(&source, filename.as_str()).tokenize();
        let module_dir = abs_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        let mut sub = Parser::new(tokens, Some(module_dir));
        // 親のキャッシュ・循環検出セット・ルートディレクトリを引き継ぐ
        sub.module_cache = self.module_cache.clone();
        sub.loading = self.loading.clone();
        sub.root_dir = self.root_dir.clone();

        let body = sub.parse_program()?;

        // 子パーサが生成したキャッシュエントリを親にマージする
        self.module_cache.extend(sub.module_cache);
        self.loading.remove(&abs_path);
        self.module_cache.insert(cache_key, body.clone());

        Ok(body)
    }

    /// `import[ar]`: `.ar` ソースのみをロードする。`.arc` があっても無視する。
    pub(crate) fn load_tl_source_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let file_rel = module_base.with_extension("ar");
        let init_rel = module_base.join("__init__.ar");

        let search_dirs: Vec<PathBuf> = {
            let a = self.source_dir.clone();
            let b = self.root_dir.clone();
            if a == b { vec![a] } else { vec![a, b] }
        };

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
                    module.join("."),
                    paths
                )
            })?;

        let cache_key = ("ar".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }
        if self.loading.contains(&abs_path) {
            return Err(format!(
                "circular import detected: '{}'",
                abs_path.display()
            ));
        }

        let source = std::fs::read_to_string(&abs_path)
            .map_err(|e| format!("cannot read module '{}': {e}", module.join(".")))?;
        let filename = abs_path.to_string_lossy().into_owned();

        self.loading.insert(abs_path.clone());

        let tokens = lexer::Lexer::new(&source, filename.as_str()).tokenize();
        let module_dir = abs_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut sub = Parser::new(tokens, Some(module_dir));
        sub.module_cache = self.module_cache.clone();
        sub.loading = self.loading.clone();
        sub.root_dir = self.root_dir.clone();

        let body = sub.parse_program()?;
        self.module_cache.extend(sub.module_cache);
        self.loading.remove(&abs_path);
        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

    /// `import[arc]`: `.arc` コンパイル済みモジュールのみをロードする。`.ar` があっても無視する。
    pub(crate) fn load_tlc_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let tlc_rel = module_base.with_extension("arc");

        let search_dirs: Vec<PathBuf> = {
            let a = self.source_dir.clone();
            let b = self.root_dir.clone();
            if a == b { vec![a] } else { vec![a, b] }
        };

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
                    module.join("."), paths
                )
            })?;

        let cache_key = ("arc".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }
        if self.loading.contains(&abs_path) {
            return Err(format!(
                "circular import detected: '{}'",
                abs_path.display()
            ));
        }

        let (mod_name, source) = crate::partial_compiler::load_tlc(&abs_path)
            .map_err(|e| format!("cannot load compiled module '{}': {e}", module.join(".")))?;
        let filename = format!("<compiled:{mod_name}>");

        self.loading.insert(abs_path.clone());

        let tokens = lexer::Lexer::new(&source, filename.as_str()).tokenize();
        let module_dir = abs_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut sub = Parser::new(tokens, Some(module_dir));
        sub.module_cache = self.module_cache.clone();
        sub.loading = self.loading.clone();
        sub.root_dir = self.root_dir.clone();

        let body = sub.parse_program()?;
        self.module_cache.extend(sub.module_cache);
        self.loading.remove(&abs_path);
        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

}
