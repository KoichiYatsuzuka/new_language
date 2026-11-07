// imports/py_modules.rs — Python モジュール(.py/.pyi)の型スタブの読み込み: load_python_interface_module / load_py_type_body。

use {
    crate::parser::Parser,
    crate::ast::Stmt, crate::module_path,
    super::dispatch::LoadedModule,
    std::path::{Path, PathBuf},
};

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
            crate::import_fs::exists(&d.join(module_base.with_extension("py")))
                || crate::import_fs::exists(&d.join(module_base.with_extension("pyi")))
                || crate::import_fs::is_dir(&d.join(&module_base))
        })
    }

    /// `import[py]` 用: .pyi を優先して検索し、なければ .py にフォールバックする。
    /// `__init__.pyi` / `__init__.py` も検索対象に含める。
    /// body は型検査専用（実行時は CPython が import して実行する・pandas_import_plan.md の D-1）。
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
        let candidates: Vec<PathBuf> = {
            let mut v = Vec::new();
            for dir in &search_dirs {
                v.push(dir.join(module_base.with_extension("pyi")));
                v.push(dir.join(module_base.join("__init__.pyi")));
            }
            for dir in &search_dirs {
                v.push(dir.join(module_base.with_extension("py")));
                v.push(dir.join(module_base.join("__init__.py")));
            }
            v
        };

        for abs_path in candidates {
            if !crate::import_fs::exists(&abs_path) { continue; }
            return self.load_py_type_body(module, &module_path::normalize(&abs_path));
        }
        // ⚠⚠ **型の出所が無ければエラー**（`editor_import_resolution_plan.md` 1-2）。以前は黙って空の body
        //    （型なし・実行時は PyO3 に任せる）にしていた。未解決の import は黙って型情報を落とさない。
        let not_found = || {
            format!(
                "import[py] '{}': cannot find its types (looked for '{}.pyi' / '{}.py' and a package \
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
        //   （メンバーはサブモジュールだけ）ので誤りにしない。`import[py] pkg.mod` はパッケージの
        //   連鎖で `pkg` をここへ通す。
        if search_dirs.iter().any(|d| crate::import_fs::is_dir(&d.join(&module_base))) {
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
            return self.build_py_type_body(module, &pseudo, src);
        }

        Err(not_found())
    }

    /// Python ソースファイル（`.pyi` / `.py`）から型検査用の body を生成する。
    ///
    /// 行単位の抽出器（`extract_py_type_stubs_raw`）で宣言を読み、再エクスポート・注釈のよそのクラス・ほかのファイルの
    /// 基底をたどって宣言を足す（`link_py_types`・1-3）。
    /// ⚠ 以前は `.pyi` を変換器（Python のソースを Arrow の文へ変換する）で読んでから抽出器で補っていた。
    ///   変換器は pandas_import_plan.md の 3-3 で消した（同梱のスタブは 1-6 から抽出器だけで読んでいた）。
    pub(crate) fn load_py_type_body(
        &mut self,
        module: &[String],
        abs_path: &PathBuf,
    ) -> Result<Vec<Stmt>, String> {
        let cache_key = ("py".to_string(), abs_path.clone());
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }
        if self.loading.contains(abs_path) {
            return Ok(vec![]);
        }
        let source = crate::import_fs::read_to_string(abs_path).map_err(|_| {
            format!("cannot read interface file for module '{}'", module.join("."))
        })?;
        self.build_py_type_body(module, abs_path, &source)
    }

    /// 読み込み済みのソース文字列から型検査用の body を組む。
    ///
    /// `load_py_type_body`（ファイル版）と**同梱スタブ**（`py_stubs`）の共通部分。宣言を落としていないかは単体テスト
    /// `bundled_stubs_lose_no_declarations` が見る。
    fn build_py_type_body(&mut self, module: &[String], abs_path: &PathBuf, source: &str) -> Result<Vec<Stmt>, String> {
        let cache_key = ("py".to_string(), abs_path.clone());
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }
        if self.loading.contains(abs_path) {
            return Ok(vec![]);
        }
        self.loading.insert(abs_path.clone());
        let mut body = crate::parser::py_stub_extract::extract_py_type_stubs_raw(source);
        self.link_py_types(module, abs_path, source, &mut body);
        // このファイル（とたどって足した宣言）に無い型の名前は `Any`（7-30・`py_unknown_types_to_any`）
        crate::parser::py_stub_extract::py_unknown_types_to_any(&mut body);

        self.loading.remove(abs_path);
        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

}

// ---------------------------------------------------------------------------
// 型のつながり（再エクスポート・注釈のよそのクラス・ほかのファイルの基底・pandas_import_plan.md の 1-3）
// ---------------------------------------------------------------------------
//
// ⚠⚠ `import[py]` の型スタブは 1 つのファイルの宣言しか持たなかったので、`pandas/__init__.py` の
//   `from pandas.core.api import DataFrame`（再エクスポート）が型にならず、`pd` のメンバーがすべて `Any` だった。
//   注釈に書いたよそのクラス（`if TYPE_CHECKING:` の中で import した `DataFrame`）も `Any` になり、ほかのファイルの基底
//   （`DataFrame(NDFrame, ..)`）のメンバー（`head` / `describe`）には型が付かなかった。
// ⇒ モジュールの import の表（`py_import_links`）をたどり、宣言（関数・クラス）を定義元から写す。写したクラスの注釈と基底は、
//   **そのクラスを書いたモジュールの** import の表で引く（名前はモジュールごとに意味が違う）。
// ⚠ よそのクラスは素の名前で登録される（外部言語のスタブの規則）ので、違うモジュールの同じ名前のクラスは区別できない。
//   1 つの型スタブには同じ名前のクラスを 1 つだけ（先に見つけたもの）入れる。
// ⚠ たどれない名前（型の出所の無い C 拡張・未導入の任意の依存・`from x import *`）は今どおり `Any`（D-2: 推論できない
//   ときは `Any`）。誤りにはしない（その名前を使わないなら困らず、使えば `Any` の規則で静的に止まる）。

/// 再エクスポート・基底の連鎖をたどる深さの上限（循環の保険）。
const PY_LINK_DEPTH: u32 = 24;

/// 宣言の名前（関数・クラス）。
fn py_decl_name(st: &Stmt) -> Option<&str> {
    match st {
        Stmt::FnDef { name, .. } | Stmt::ClassDef { name, .. } | Stmt::Let(name, ..) => Some(name),
        _ => None,
    }
}

/// 宣言の注釈に出てくる、よそのモジュールのクラスかもしれない名前（大文字で始まる名前・`a.B` の形）。
fn py_type_refs(st: &Stmt, out: &mut Vec<String>) {
    const ARROW_TYPE_NAMES: &[&str] = &["Any", "None", "Union", "Option", "Intersection", "Result", "Self", "Undefined"];
    fn scan(t: &str, out: &mut Vec<String>) {
        for w in t.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')) {
            let last = w.rsplit('.').next().unwrap_or(w);
            if last.starts_with(|c: char| c.is_ascii_uppercase()) && !ARROW_TYPE_NAMES.contains(&w) {
                out.push(w.to_string());
            }
        }
    }
    fn fn_sig(params: &[crate::ast::Param], ret: &Option<String>, out: &mut Vec<String>) {
        for p in params {
            if let Some(t) = &p.type_ann {
                scan(t, out);
            }
        }
        if let Some(t) = ret {
            scan(t, out);
        }
    }
    match st {
        Stmt::FnDef { params, return_type, .. } => fn_sig(params, return_type, out),
        Stmt::Let(_, Some(t), _, _) => scan(t, out),
        Stmt::ClassDef { body, .. } => {
            for m in body {
                match m {
                    Stmt::FnDef { params, return_type, .. } => fn_sig(params, return_type, out),
                    Stmt::Field { type_ann, .. } => scan(type_ann, out),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

type PyStubRc = std::rc::Rc<crate::parser::py_import_links::PyRawStub>;

/// ほかのモジュール `home` から写したクラスの宣言の基底（`home` のファイルの素の名前）を、`home` で修飾した名前にする
/// （型はモジュールごとに登録する・pandas_import_plan.md 3-1）。素の名前のまま写すと、写した先のモジュールの同じ名前の
/// クラスを基底と取り違える。組み込みの例外はそのまま。
fn py_home_bases(decl: &mut Stmt, home: &PyStubRc, here: &[String]) {
    if home.module == here {
        return;
    }
    if let Stmt::ClassDef { bases, .. } = decl {
        for b in bases.iter_mut() {
            if !b.contains('.') && !crate::parser::py_stub_extract::is_builtin_exception_name(b) {
                *b = format!("{}.{}", home.module.join("."), b);
            }
        }
    }
}

impl Parser {
    /// 絶対のモジュール名 `module` の型スタブの元（`.pyi` を先に）。見つからない・読めないなら `None`。
    fn py_raw_stub(&mut self, module: &[String]) -> Option<PyStubRc> {
        if module.is_empty() {
            return None;
        }
        let key = module.join(".");
        if let Some(s) = self.py_raw_stubs.borrow().get(&key) {
            return s.clone();
        }
        let stub = self.py_raw_stub_load(module);
        self.py_raw_stubs.borrow_mut().insert(key, stub.clone());
        stub
    }

    /// [`Self::py_raw_stub`] の、探して読む本体（置き場に無かったときだけ呼ぶ）。
    fn py_raw_stub_load(&mut self, module: &[String]) -> Option<PyStubRc> {
        use crate::parser::py_import_links::{py_import_links, PyRawStub};
        let base: PathBuf = module.iter().collect();
        let from_dir = self.source_dir.clone();
        let dirs = self.python_import_dirs(&from_dir, 0);
        let mut found = None;
        'search: for ext in ["pyi", "py"] {
            for dir in &dirs {
                for (p, pkg) in [(dir.join(base.with_extension(ext)), false), (dir.join(base.join(format!("__init__.{ext}"))), true)] {
                    if crate::import_fs::exists(&p) {
                        found = Some((p, pkg));
                        break 'search;
                    }
                }
            }
        }
        // `.py` の無い built-in のモジュール（`_collections` など）は同梱のスタブ（1-4・collections.deque をたどる）
        let (source, filename, is_package) = match found {
            Some((path, is_package)) => {
                let path = module_path::normalize(&path);
                let source = crate::import_fs::read_to_string(&path).ok()?;
                (std::borrow::Cow::Owned(source), path.to_string_lossy().to_string(), is_package)
            }
            None => (std::borrow::Cow::Borrowed(crate::py_stubs::builtin_stub(module)?), module.join("."), false),
        };
        let stub = std::rc::Rc::new(PyRawStub {
            module: module.to_vec(),
            is_package,
            decls: crate::parser::py_stub_extract::extract_py_type_stubs_raw(&source),
            links: py_import_links(&source, &filename),
            outside_bases: crate::parser::py_stub_extract::py_class_outside_bases(&source),
        });
        Some(stub)
    }

    /// `ctx` のモジュールで名前 `name`（`a.B` の形も）が指す宣言を、再エクスポートをたどって探す。見つかれば（宣言, 宣言したモジュールの元）。
    fn py_chase_decl(&mut self, ctx: &PyStubRc, name: &str, depth: u32) -> Option<(Stmt, PyStubRc)> {
        use crate::parser::py_import_links::absolute_module;
        if depth > PY_LINK_DEPTH {
            return None;
        }
        // `a.B`: `a` が指すモジュールの `B`
        if let Some((head, rest)) = name.split_once('.') {
            let link = ctx.links.iter().find(|l| l.local == head)?.clone();
            let mut target = absolute_module(&ctx.module, ctx.is_package, link.level, &link.module)?;
            if let Some(sub) = &link.name {
                // `from a import b` の `b` はサブモジュール
                target.push(sub.clone());
            }
            let next = self.py_raw_stub(&target)?;
            return self.py_chase_decl(&next, rest, depth + 1);
        }
        // ⚠ `typing.Any` は Arrow の `Any`。`typing.py` には `class Any` があるので、たどると `from typing import Any` が
        //   そのクラスをモジュールへ写し、注釈の `Any` がモジュールのクラス（`m.Any`）になっていた（`m.Box(3)` が
        //   `expects 'm.Any' but got 'int'` の静的な誤り・numpy の `ndarray.tolist() -> Any` も `numpy.Any` になる）。
        if name == "Any" && ctx.module == ["typing"] {
            return None;
        }
        if let Some(d) = ctx.decls.iter().find(|d| py_decl_name(d) == Some(name)) {
            return Some((d.clone(), std::rc::Rc::clone(ctx)));
        }
        let link = ctx.links.iter().find(|l| l.local == name)?.clone();
        // `import x as y` はモジュールそのもの（宣言ではない）
        let orig = link.name.as_deref()?;
        let target = absolute_module(&ctx.module, ctx.is_package, link.level, &link.module)?;
        let next = self.py_raw_stub(&target)?;
        self.py_chase_decl(&next, orig, depth + 1)
    }

    /// クラスの宣言 `class`（`home` のモジュールで書かれたもの）に、ほかのファイルの基底のメンバーを写す。基底の注釈の名前は
    /// 基底を書いたモジュールで引くので、`queue` に（名前, 文脈）を積む。
    fn py_merge_outside_bases(&mut self, class: &mut Stmt, home: &PyStubRc, queue: &mut Vec<(String, PyStubRc)>, depth: u32) {
        let Stmt::ClassDef { name, .. } = &*class else { return };
        let Some(bases) = home.outside_bases.get(name).cloned() else { return };
        if depth > PY_LINK_DEPTH {
            return;
        }
        for b in bases {
            // `Generic[T]` などの型引数は外して名前で引く
            let bare = b.split('[').next().unwrap_or(&b).trim().to_string();
            let Some((mut base_decl, base_home)) = self.py_chase_decl(home, &bare, 0) else { continue };
            if !matches!(base_decl, Stmt::ClassDef { .. }) {
                continue;
            }
            self.py_merge_outside_bases(&mut base_decl, &base_home, queue, depth + 1);
            let mut refs = Vec::new();
            py_type_refs(&base_decl, &mut refs);
            queue.extend(refs.into_iter().map(|r| (r, std::rc::Rc::clone(&base_home))));
            crate::parser::py_stub_extract::py_merge_base_members(class, &base_decl);
            if let (Stmt::ClassDef { bases, .. }, Stmt::ClassDef { name: base_name, bases: base_bases, .. }) =
                (&mut *class, &base_decl)
            {
                // ほかのファイルの基底が例外なら、派生も同じ組み込みの例外を基底に持つ（`py_exception_base`・2-1）
                for b in base_bases.iter().filter(|b| crate::parser::py_stub_extract::is_builtin_exception_name(b)) {
                    if !bases.contains(b) {
                        bases.push(b.clone());
                    }
                }
                // 基底のクラスは、それを書いたモジュールで修飾した名前（`pkg.zoo.Dog`・型の登録はモジュールごと・3-1）。
                // ⚠ そのモジュールをプログラムが import していなければ型の表に無いので、その先の基底へはたどれない
                let qualified = format!("{}.{}", base_home.module.join("."), base_name);
                if !bases.contains(&qualified) {
                    bases.push(qualified);
                }
            }
        }
    }

    /// `import[py]` の型スタブ `body`（`module` の、まだよその型の名前を `Any` にしていない宣言）に、① 再エクスポートの宣言
    /// ② 宣言の注釈に出てくるよそのクラス ③ クラスのほかのファイルの基底のメンバー を足す（上の節の説明・1-3）。
    pub(crate) fn link_py_types(&mut self, module: &[String], abs_path: &Path, source: &str, body: &mut Vec<Stmt>) {
        use crate::parser::py_import_links::{py_import_links, PyRawStub};
        let is_package = abs_path.file_stem().is_some_and(|s| s == "__init__");
        let me = std::rc::Rc::new(PyRawStub {
            module: module.to_vec(),
            is_package,
            decls: body.clone(),
            links: py_import_links(source, &abs_path.to_string_lossy()),
            outside_bases: crate::parser::py_stub_extract::py_class_outside_bases(source),
        });
        let mut queue: Vec<(String, PyStubRc)> = Vec::new();
        // ③ このファイルのクラスの、ほかのファイルの基底
        for st in body.iter_mut() {
            self.py_merge_outside_bases(st, &me, &mut queue, 0);
        }
        // ① 再エクスポート（このファイルが宣言せず、import で束縛した名前）
        let mut classes: std::collections::HashSet<String> = body
            .iter()
            .filter_map(|s| if let Stmt::ClassDef { name, .. } = s { Some(name.clone()) } else { None })
            .collect();
        let declared: std::collections::HashSet<String> =
            body.iter().filter_map(|s| py_decl_name(s).map(str::to_string)).collect();
        for link in me.links.clone() {
            if declared.contains(&link.local) || link.name.is_none() {
                continue;
            }
            let Some((mut decl, home)) = self.py_chase_decl(&me, &link.local, 0) else { continue };
            match &mut decl {
                Stmt::FnDef { name, .. } => *name = link.local.clone(),
                // ⚠ 別名で取り込んだクラスは写さない（クラスの名前が型の名前なので、別名にすると別の型になる・その名前は `Any`）
                Stmt::ClassDef { name, .. } if *name == link.local => {
                    if !classes.insert(name.clone()) {
                        continue;
                    }
                }
                _ => continue,
            }
            py_home_bases(&mut decl, &home, module);
            self.py_merge_outside_bases(&mut decl, &home, &mut queue, 0);
            let mut refs = Vec::new();
            py_type_refs(&decl, &mut refs);
            queue.extend(refs.into_iter().map(|r| (r, std::rc::Rc::clone(&home))));
            body.push(decl);
        }
        // ② このファイルの宣言の注釈に出てくる名前
        for st in me.decls.iter() {
            let mut refs = Vec::new();
            py_type_refs(st, &mut refs);
            queue.extend(refs.into_iter().map(|r| (r, std::rc::Rc::clone(&me))));
        }
        // 注釈のよそのクラスを、たどれる限り足す（足したクラスの注釈も、それを書いたモジュールで引く）
        let mut tried: std::collections::HashSet<(String, Vec<String>)> = std::collections::HashSet::new();
        while let Some((name, ctx)) = queue.pop() {
            let short = name.rsplit('.').next().unwrap_or(&name).to_string();
            if classes.contains(&short) || !tried.insert((name.clone(), ctx.module.clone())) {
                continue;
            }
            let Some((mut decl, home)) = self.py_chase_decl(&ctx, &name, 0) else { continue };
            let cname = match &decl {
                Stmt::ClassDef { name, .. } => name.clone(),
                _ => continue,
            };
            if !classes.insert(cname) {
                continue;
            }
            py_home_bases(&mut decl, &home, module);
            self.py_merge_outside_bases(&mut decl, &home, &mut queue, 0);
            let mut refs = Vec::new();
            py_type_refs(&decl, &mut refs);
            queue.extend(refs.into_iter().map(|r| (r, std::rc::Rc::clone(&home))));
            body.push(decl);
        }
    }
}
