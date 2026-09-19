// imports/py_modules.rs — Python モジュール(.py/.pyi)の読み込み: load_python_module / load_python_interface_module / load_py_type_body。

use {
    crate::parser::Parser,
    crate::ast::Stmt, crate::python_converter,
    std::path::PathBuf,
};
use super::*;

impl Parser {
    /// Python モジュールを検索・変換する（キャッシュ込み）。
    /// python_search_dirs() の順に .py または __init__.py を探す。
    pub(crate) fn load_python_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let rel_py   = module_base.with_extension("py");
        let rel_init = module_base.join("__init__.py");
        let search_dirs = self.python_search_dirs();

        // 検索ディレクトリを順に試して最初に見つかった .py / __init__.py を使う
        let abs_path = search_dirs
            .iter()
            .flat_map(|d| [d.join(&rel_py), d.join(&rel_init)])
            .find(|p| p.exists())
            .ok_or_else(|| {
                let looked = search_dirs
                    .iter()
                    .flat_map(|d| [d.join(&rel_py), d.join(&rel_init)])
                    .map(|p| format!("'{}'", p.display()))
                    .collect::<Vec<_>>()
                    .join(", ");
                // ⚠ C 拡張（`sys` / `numpy` …）は `.py` の実体が無いのでここに来る。
                //   何が起きたのか判るように、代替手段まで書く。
                format!(
                    "cannot find Python source for module '{}' \
                     — `import[py]` translates `.py` sources, so modules without one \
                     (C extensions such as sys, numpy, ...) cannot be loaded this way; \
                     use `import[py-int]` to call them through CPython instead \
                     (looked at {})",
                    module.join("."),
                    looked
                )
            })?;

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

        let cache_key = ("py".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        if self.loading.contains(&abs_path) {
            return Err(format!("circular import detected: '{}'", abs_path.display()));
        }

        let source = std::fs::read_to_string(&abs_path)
            .map_err(|e| format!("cannot read '{}': {e}", abs_path.display()))?;

        self.loading.insert(abs_path.clone());

        let filename = abs_path.to_string_lossy().to_string();
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
        let filled = self.fill_python_imports(&mut body);
        self.loading.remove(&abs_path);
        filled?;
        self.module_cache.insert(cache_key, body.clone());

        Ok(body)
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
    fn fill_python_imports(&mut self, body: &mut [Stmt]) -> Result<(), String> {
        for stmt in body.iter_mut() {
            match stmt {
                Stmt::Import { lang, module, body: sub, .. }
                | Stmt::FromImport { lang, module, body: sub, .. }
                    if lang == "py" && sub.is_empty() =>
                {
                    *sub = self.load_python_module(module)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `import[py-int]` 用: .pyi を優先して検索し、なければ .py にフォールバックする。
    /// `__init__.pyi` / `__init__.py` も検索対象に含める。
    /// body は型検査専用（実行時は PyO3 経由で別ロジックが動く）。
    pub(crate) fn load_python_interface_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let search_dirs = self.python_search_dirs();

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
            return self.load_py_type_body(module, &abs_path, is_pyi);
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

        // それでも無ければ空の body を返す（型検査スキップ、実行時は PyO3 が担当）
        Ok(vec![])
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
                Err(e) => {
                    if bundled {
                        eprintln!(
                            "Warning: bundled stub for '{}' failed to convert ({e}); \
                             falling back to line-based extraction",
                            module.join(".")
                        );
                    }
                    Vec::new()
                }
            };
            // スタブで不足を補完（変換できなかった関数を追加）
            let known: std::collections::HashSet<String> = converted
                .iter()
                .filter_map(|s| if let Stmt::FnDef { name, .. } = s { Some(name.clone()) } else { None })
                .collect();
            for stub in extract_py_type_stubs(source) {
                if let Stmt::Let(ref name, _, _) = stub {
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
