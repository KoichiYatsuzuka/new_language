// imports/cs_js_modules.rs — C# / JS モジュールの読み込みとパス解決: load_cs_module / load_js_module / load_cs_lib_paths / python_search_dirs。

use {
    crate::parser::Parser,
    crate::ast::Stmt, crate::lexer,
    std::path::PathBuf,
};
use super::*;

impl Parser {
    /// `import[cs-dll]` / `import[cs-proc]` — .NET アセンブリから型スタブを生成する。
    ///
    /// DLL の検索順:
    ///   1. source_dir / path/to/LastSegment.dll
    ///   2. source_dir / LastSegment.dll
    ///   3. root_dir  / LastSegment.dll
    ///   4. ar_config.json の csharp.lib_paths に列挙されたディレクトリ
    ///
    /// DLL が見つからない場合は警告を出して空スタブを返す（型なし・実行時に解決）。
    /// `is_proc` は IPC サブプロセス方式かを示すが、型スタブは両方式で共通。
    pub(crate) fn load_cs_module(&mut self, module: &[String], is_proc: bool) -> Result<Vec<Stmt>, String> {
        let last = module.last().cloned().unwrap_or_default();
        let dll_name = format!("{last}.dll");

        // キャッシュキー
        let cache_key = (if is_proc { "cs-proc" } else { "cs-dll" }.to_string(),
                         std::path::PathBuf::from(&dll_name));
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        // 候補パスを順番に試す
        // 単一セグメント "name" の場合は source_dir/name/name.dll も試す（パッケージディレクトリ規約）。
        let sub_path: PathBuf = module.iter().collect::<PathBuf>().with_extension("dll");
        let mut candidates: Vec<PathBuf> = vec![
            self.source_dir.join(&sub_path),
            self.source_dir.join(&dll_name),
            self.root_dir.join(&dll_name),
        ];
        if module.len() == 1 {
            // import[cs-dll] foo → also try source_dir/foo/foo.dll
            candidates.push(self.source_dir.join(&last).join(&dll_name));
            candidates.push(self.root_dir.join(&last).join(&dll_name));
        }

        let mut dll_path: Option<PathBuf> = None;
        for c in &candidates {
            if c.exists() {
                dll_path = Some(c.clone());
                break;
            }
        }

        // ar_config.json の csharp.lib_paths も検索
        if dll_path.is_none() {
            if let Some(extra) = self.load_cs_lib_paths() {
                for dir in extra {
                    let p = dir.join(&dll_name);
                    if p.exists() {
                        dll_path = Some(p);
                        break;
                    }
                }
            }
        }

        let body = match dll_path {
            Some(path) => {
                match crate::parser::cs_assembly::load_cs_assembly(&path) {
                    Ok(stmts) => stmts,
                    Err(e) => {
                        eprintln!("Warning: import[cs-*]: {e}; falling back to empty stubs");
                        vec![]
                    }
                }
            }
            None => {
                eprintln!(
                    "Warning: import[cs-*]: cannot find '{dll_name}' for module '{}'; \
                     no type stubs available (add the DLL path to ar_config.json csharp.lib_paths)",
                    module.join(".")
                );
                vec![]
            }
        };

        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

    /// `import[js-proc]` — .ars スタブファイルが存在すれば読み込み、なければ空スタブを返す。
    ///
    /// スタブ検索順:
    ///   1. source_dir / path/to/module.ars
    ///   2. root_dir   / path/to/module.ars
    ///
    /// スタブが見つからない場合は空 body を返す（型なし・実行時にブリッジが関数リストを提供）。
    pub(crate) fn load_js_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let cache_key = ("js-proc".to_string(), module.iter().collect::<PathBuf>());
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        let sub_path: PathBuf = module.iter().collect::<PathBuf>().with_extension("ars");
        let candidates = [
            self.source_dir.join(&sub_path),
            self.root_dir.join(&sub_path),
        ];

        let body = candidates.iter().find_map(|p| -> Option<Vec<Stmt>> {
            if !p.exists() { return None; }
            let src = std::fs::read_to_string(p).ok()?;
            let filename = p.to_string_lossy().to_string();
            let module_dir = p.parent().map(|d| d.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let tokens = lexer::Lexer::new(&src, filename.as_str()).tokenize();
            let mut sub = Parser::new(tokens, Some(module_dir));
            sub.module_cache = self.module_cache.clone();
            sub.loading     = self.loading.clone();
            sub.root_dir    = self.root_dir.clone();
        // node-id はプログラム全体で一意にする（#16・C1）。共有しないとモジュール間で
        // 衝突し、消費側が別モジュールの注釈を読む（FFI 境界検査が誤検知する）。
        sub.node_counter = self.node_counter.clone();
            sub.parse_program().ok()
        }).unwrap_or_default();

        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

    /// ar_config.json の `csharp.lib_paths` を読んでパスリストを返す。
    pub(crate) fn load_cs_lib_paths(&self) -> Option<Vec<PathBuf>> {
        // Walk up from source_dir looking for ar_config.json
        let mut dir = self.source_dir.clone();
        loop {
            let cfg = dir.join("ar_config.json");
            if cfg.exists() {
                if let Ok(text) = std::fs::read_to_string(&cfg) {
                    return parse_cs_lib_paths(&text, &dir);
                }
            }
            if !dir.pop() { break; }
        }
        None
    }

    /// Python モジュールの検索ディレクトリリストを返す。
    /// source_dir を先頭に、ar_config.json の python.search_paths、PYTHONPATH 環境変数、Python site-packages を続ける。
    pub(crate) fn python_search_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.source_dir.clone()];
        // ar_config.json の python.search_paths を追加する（source_dir → root_dir の順に探す）
        let config_search = if self.source_dir == self.root_dir {
            vec![self.source_dir.clone()]
        } else {
            vec![self.source_dir.clone(), self.root_dir.clone()]
        };
        // ⚠ #72: JSON の読み取りは [`crate::ar_config`] へ委譲した（以前はここ専用の
        // 手書き文字列走査で、`python` の外の `search_paths` を拾う等の誤りが 3 件あった）。
        // ⚠⚠ **探索方針（`source_dir` と `root_dir` の 2 箇所だけ）はここの契約なので畳まない**
        // （`Interpreter` 側は祖先を root まで遡る。理由は `ar_config` のモジュール doc）。
        for config_dir in &config_search {
            let cfg_path = config_dir.join("ar_config.json");
            if cfg_path.exists() {
                for p in crate::ar_config::read_python_search_paths(&cfg_path, config_dir) {
                    if !dirs.contains(&p) {
                        dirs.push(p);
                    }
                }
                break;
            }
        }
        if let Ok(pythonpath) = std::env::var("PYTHONPATH") {
            for p in std::env::split_paths(&pythonpath) {
                dirs.push(p);
            }
        }
        // Python インタープリタの sys.prefix から site-packages を推測
        if let Ok(prefix) = std::env::var("PYTHONHOME") {
            dirs.push(PathBuf::from(&prefix).join("Lib").join("site-packages"));
        }
        // Python プロセスから標準ライブラリと site-packages のパスを取得して追加
        for p in python_lib_dirs() {
            if !dirs.contains(p) {
                dirs.push(p.clone());
            }
        }
        dirs
    }
}
