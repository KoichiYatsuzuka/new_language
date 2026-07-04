// imports/py_modules.rs — Python モジュール(.py/.pyi)の読み込み: load_python_module / load_python_interface_module / load_py_type_body。

#[allow(unused_imports)]
use {
    crate::parser::Parser,
    crate::ast::{Accessibility, FieldKind, Param, Stmt},
    crate::token::Token, crate::lexer, crate::python_converter,
    std::path::PathBuf,
};
#[allow(unused_imports)]
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
                format!("cannot find module '{}' (looked at {})", module.join("."), looked)
            })?;

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
        let body = python_converter::convert_python_source(&source, &filename)?;

        self.loading.remove(&abs_path);
        self.module_cache.insert(cache_key, body.clone());

        Ok(body)
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

        // 見つからなければ空の body を返す（型検査スキップ、実行時は PyO3 が担当）
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
        self.loading.insert(abs_path.clone());

        let body = if is_pyi {
            // .pyi: python_converter でベストエフォート変換
            let filename = abs_path.to_string_lossy().to_string();
            let mut converted = python_converter::convert_python_source(&source, &filename)
                .unwrap_or_default();
            // スタブで不足を補完（変換できなかった関数を追加）
            let known: std::collections::HashSet<String> = converted
                .iter()
                .filter_map(|s| if let Stmt::FnDef { name, .. } = s { Some(name.clone()) } else { None })
                .collect();
            for stub in extract_py_type_stubs(&source) {
                if let Stmt::Let(ref name, _, _) = stub {
                    if !known.contains(name.as_str()) {
                        converted.push(stub);
                    }
                }
            }
            converted
        } else {
            // .py: 直接スタブ抽出（python_converter は複雑な構文に対応できないため使わない）
            extract_py_type_stubs(&source)
        };

        self.loading.remove(abs_path);
        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

}
