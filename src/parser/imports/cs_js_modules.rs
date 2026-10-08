// imports/cs_js_modules.rs — C# / JS モジュールの読み込みとパス解決: load_cs_module / load_js_module / load_cs_lib_paths / python_search_dirs。

use {
    crate::parser::Parser,
    crate::ast::Stmt, crate::lexer, crate::module_path,
    std::path::{Path, PathBuf},
};

impl Parser {
    /// `import[cs-dll]` / `import[cs-proc]` — .NET アセンブリから型スタブを生成する。
    ///
    /// DLL の検索順（`base` は探索の起点・[`crate::module_path`]）:
    ///   1. base / path/to/LastSegment.dll
    ///   2. base / LastSegment.dll
    ///   3. base / LastSegment/LastSegment.dll（単一セグメントのときだけ）
    ///   4. ar_config.json の csharp.lib_paths に列挙されたディレクトリ（ドット無しの書き方のときだけ）
    ///
    /// ⚠ 以前は 2 の後に `root_dir`（エントリのディレクトリ）も見ていた（2026-10-02 に外した）。
    ///
    /// DLL が見つからない場合は警告を出して空スタブを返す（型なし・実行時に解決）。
    /// `is_proc` は IPC サブプロセス方式かを示すが、型スタブは両方式で共通。
    pub(crate) fn load_cs_module(&mut self, level: u32, module: &[String], is_proc: bool) -> Result<Vec<Stmt>, String> {
        let last = module.last().cloned().unwrap_or_default();
        let dll_name = format!("{last}.dll");

        // 候補パスを順番に試す
        // 単一セグメント "name" の場合は base/name/name.dll も試す（パッケージディレクトリ規約）。
        let base = self.import_base(level);
        let sub_path: PathBuf = module.iter().collect::<PathBuf>().with_extension("dll");
        let mut candidates: Vec<PathBuf> = vec![base.join(&sub_path), base.join(&dll_name)];
        if module.len() == 1 {
            // import[cs-dll] foo → also try base/foo/foo.dll
            candidates.push(base.join(&last).join(&dll_name));
        }

        // キャッシュキー（⚠ 起点ごとに別物。別のディレクトリの同名 DLL を取り違えない）
        let cache_key = (if is_proc { "cs-proc" } else { "cs-dll" }.to_string(),
                         module_path::absolute(&base).join(&sub_path));
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        let mut dll_path: Option<PathBuf> = None;
        for c in &candidates {
            if crate::import_fs::exists(c) {
                dll_path = Some(c.clone());
                break;
            }
        }

        // ar_config.json の csharp.lib_paths も検索（外部の探索先なのでドット無しのときだけ）
        if dll_path.is_none() && module_path::uses_external_paths(level) {
            if let Some(extra) = self.load_cs_lib_paths() {
                for dir in extra {
                    let p = dir.join(&dll_name);
                    if crate::import_fs::exists(&p) {
                        dll_path = Some(p);
                        break;
                    }
                }
            }
        }

        // ⚠⚠ **見つからない・読めなければエラー**（`editor_import_resolution_plan.md` 1-2）。以前は警告を
        //    出して空の body（型なし）にしていた。未解決の import は黙って型情報を落とさない。
        let kind = if is_proc { "cs-proc" } else { "cs-dll" };
        let body = match dll_path {
            Some(path) => crate::parser::cs_assembly::load_cs_assembly(&path)
                .map_err(|e| format!("import[{kind}] '{}': cannot read the .NET metadata of '{}': {e}", module.join("."), path.display()))?,
            None => {
                return Err(format!(
                    "import[{kind}] '{}': cannot find '{dll_name}' (looked at {}; add its directory to \
                     ar_config.json csharp.lib_paths)",
                    module.join("."),
                    {
                        let mut shown: Vec<String> = Vec::new();
                        for c in &candidates {
                            let s = format!("'{}'", c.display());
                            if !shown.contains(&s) {
                                shown.push(s);
                            }
                        }
                        shown.join(", ")
                    }
                ));
            }
        };

        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

    /// `import[js-proc]` — .ars スタブファイルが存在すれば読み込み、なければ空スタブを返す。
    ///
    /// スタブ検索先: 探索の起点（[`crate::module_path`]）/ path/to/module.ars
    ///
    /// ⚠ 以前は `root_dir`（エントリのディレクトリ）も見ていた（2026-10-02 に外した）。
    ///
    /// スタブが見つからない場合は空 body を返す（型なし・実行時にブリッジが関数リストを提供）。
    pub(crate) fn load_js_module(&mut self, level: u32, module: &[String]) -> Result<Vec<Stmt>, String> {
        let base = self.import_base(level);
        let sub_path: PathBuf = module.iter().collect::<PathBuf>().with_extension("ars");
        // ⚠ 起点ごとに別物（別のディレクトリの同名スタブを取り違えない）。
        let cache_key = ("js-proc".to_string(), module_path::absolute(&base).join(&sub_path));
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        // ⚠⚠ **`.ars` が無い・読めない・構文解析できなければエラー**（`editor_import_resolution_plan.md` 1-2）。
        //    以前は黙って空の body（型なし）にしていた。`.ars` が js-proc の型の唯一の出所。
        let stub = base.join(&sub_path);
        if !crate::import_fs::exists(&stub) {
            return Err(format!(
                "import[js-proc] '{}': cannot find the type stub '{}'; write the module's declarations there \
                 (see examples/interop for the .ars format)",
                module.join("."),
                stub.display()
            ));
        }
        let candidates = [stub];
        let body = candidates.iter().find_map(|p| -> Option<Result<Vec<Stmt>, String>> {
            let src = match crate::import_fs::read_to_string(p) {
                Ok(s) => s,
                Err(e) => return Some(Err(format!("import[js-proc]: cannot read '{}': {e}", p.display()))),
            };
            let filename = p.to_string_lossy().to_string();
            let module_dir = p.parent().map(|d| d.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let tokens = lexer::Lexer::new(&src, filename.as_str()).tokenize();
            let mut sub = Parser::new(tokens, Some(module_dir));
            sub.module_cache = self.module_cache.clone();
            sub.loading     = self.loading.clone();
            sub.root_dir    = self.root_dir.clone();
            sub.module_names = std::rc::Rc::clone(&self.module_names);
        // node-id はプログラム全体で一意にする（#16・C1）。共有しないとモジュール間で
        // 衝突し、消費側が別モジュールの注釈を読む（FFI 境界検査が誤検知する）。
        sub.node_counter = self.node_counter.clone();
            Some(sub.parse_program().map_err(|e| format!("import[js-proc]: in '{}': {e}", p.display())))
        }).unwrap_or_else(|| Ok(Vec::new()))?;

        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

    /// ar_config.json の `csharp.lib_paths` を読んでパスリストを返す。
    ///
    /// ⚠ JSON の読み取りは [`crate::ar_config`] へ委譲（#73）。ここが持つのは
    /// **探索方針（エントリのディレクトリから祖先へ遡り、最初に読めた設定で確定）**だけ。
    /// ⚠ `cfg.exists()` でも読めなかった場合は**さらに上へ遡る**（`python` 側は空を返して打ち切る）。
    /// 既存の挙動なのでそのまま保存してある。
    pub(crate) fn load_cs_lib_paths(&self) -> Option<Vec<PathBuf>> {
        // エントリのディレクトリ（CPython の `sys.path[0]` 相当・`crate::module_path`）から
        // 祖先へ ar_config.json を探す。⚠ 2026-10-02 午前の版までは import 文のファイルのディレクトリから。
        let mut dir = self.root_dir.clone();
        loop {
            let cfg = dir.join("ar_config.json");
            if crate::import_fs::exists(&cfg) {
                if let Ok(text) = crate::import_fs::read_to_string(&cfg) {
                    return crate::ar_config::read_cs_lib_paths_from_str(&text, &dir);
                }
            }
            if !dir.pop() { break; }
        }
        None
    }

    /// Python モジュールの検索ディレクトリリストを返す（ドット無しの import 用）。
    /// `from_dir`（import 文を書いたファイルのディレクトリ）を先頭に、ar_config.json の
    /// python.search_paths、PYTHONPATH 環境変数、Python site-packages を続ける。
    pub(crate) fn python_search_dirs_from(&self, from_dir: &Path) -> Vec<PathBuf> {
        let mut dirs = vec![from_dir.to_path_buf()];
        // ar_config.json の python.search_paths を追加する。
        //
        // ⚠ #72: JSON の読み取りは [`crate::ar_config`] へ委譲した（以前はここ専用の
        // 手書き文字列走査で、`python` の外の `search_paths` を拾う等の誤りが 3 件あった）。
        //
        // ⚠⚠ #74: **探索方針を祖先ウォークへ揃えた**（以前は `source_dir` と `root_dir` の
        // 2 箇所だけ）。揃える前は、`examples/interop/py_subdir/` のように**自分の設定を
        // 持たないサブディレクトリ**から実行すると、同じ `ar_config.json` が
        // `import[py-int]` からは見えて `import[py]` からは見えず **ParseError** になっていた。
        // ⚠ ドット無しの import では `from_dir` はエントリのディレクトリ（CPython の `sys.path[0]`
        //   相当・[`crate::module_path`]・2026-10-02）。
        let cfg = crate::ar_config::find_ancestor_config(from_dir);
        if let Some((cfg_path, base)) = cfg {
            for p in crate::ar_config::read_python_search_paths(&cfg_path, &base) {
                if !dirs.contains(&p) {
                    dirs.push(p);
                }
            }
        }
        dirs.extend(crate::import_fs::env_paths("PYTHONPATH"));
        // Python インタープリタの sys.prefix から site-packages を推測
        if let Some(prefix) = crate::import_fs::env_path("PYTHONHOME") {
            dirs.push(prefix.join("Lib").join("site-packages"));
        }
        // Python プロセスから標準ライブラリと site-packages のパスを取得して追加
        for p in crate::import_fs::python_lib_dirs() {
            if !dirs.contains(p) {
                dirs.push(p.clone());
            }
        }
        dirs
    }
}


#[cfg(test)]
mod cs_lib_path_search_tests {
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    /// `load_cs_lib_paths` の**探索方針**（`source_dir` から祖先へ遡り、最初に読めた設定で確定）を
    /// 実ファイルで固定する（#73）。
    ///
    /// ⚠ **エンドツーエンドの例題は無い** — `csharp.lib_paths` を踏むには DLL と
    /// ネイティブブリッジを既定の候補パスの**外**に置く必要があり、リポジトリに
    /// バイナリを増やすことになる。そこでここが唯一の網になっている。
    #[test]
    fn walks_up_to_the_nearest_config() {
        let root = std::env::temp_dir().join(format!("ar73_{}_{}", std::process::id(), line!()));
        let deep = root.join("a").join("b");
        std::fs::create_dir_all(&deep).unwrap();
        // 上位に設定を置き、途中には置かない ⇒ 祖先ウォークで届くこと。
        std::fs::write(
            root.join("ar_config.json"),
            r#"{"csharp":{"lib_paths":["libs","/tmp/abs73"]}}"#,
        )
        .unwrap();

        let tokens = Lexer::new("", "<test>").tokenize();
        let parser = Parser::new(tokens, Some(deep.clone()));
        let got = parser.load_cs_lib_paths().expect("lib_paths must be found");

        // 相対パスは**設定ファイルのある場所**（root）基準で解決される。
        assert_eq!(got[0], root.join("libs"));
        assert_eq!(got.len(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 設定が 1 つも無ければ `None`（root まで遡って空振り）。
    #[test]
    fn returns_none_without_any_config() {
        let root = std::env::temp_dir().join(format!("ar73_{}_{}", std::process::id(), line!()));
        std::fs::create_dir_all(&root).unwrap();
        let tokens = Lexer::new("", "<test>").tokenize();
        let parser = Parser::new(tokens, Some(root.clone()));
        // ⚠ temp の祖先に ar_config.json が置かれている環境では成立しないので、
        //    「見つかったとしても temp 配下ではない」ことだけを確かめる。
        if let Some(paths) = parser.load_cs_lib_paths() {
            assert!(paths.iter().all(|p| !p.starts_with(&root)));
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
