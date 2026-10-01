// module_path.rs — import の探索規則とモジュールの同一性（相対 import・2026-10-02）。
//
// ⚠⚠ **探索の起点の規則はここが唯一の定義。** パーサ（全言語のローダ）と実行時
//   （`import[py-int]` の `sys.path`・C# のブリッジ探索・js-proc の設定探索）が同じ関数を呼ぶ。
//   以前は言語と段ごとに起点がばらばらだった:
//   - `.ar` / `.arc` … 書いたファイルのディレクトリ → **エントリのディレクトリ**
//   - py … 書いたファイルのディレクトリ → 設定（見つからなければエントリ側の設定）
//   - 実行時（py-int / cs / js）… **エントリのディレクトリ**だけ
//   その結果、サブディレクトリのファイルをエントリにすると上の階層を指す手段が無く、
//   同じモジュールが「どのファイルから実行したか」で読めたり読めなかったりした。
//
// ## 規則（全言語共通）
//
// | 書き方 | 探す場所 |
// |---|---|
// | `import a.b` | import 文を書いたファイルのディレクトリ → 言語ごとの外部の探索先 |
// | `import .a.b` | 同じディレクトリ**だけ** |
// | `import ..a.b` | 1 つ上のディレクトリ**だけ**（ドットが 1 つ増えるごとにさらに 1 つ上） |
//
// - 言語ごとの外部の探索先: Python の `python.search_paths` / `PYTHONPATH` / site-packages、
//   C# の `csharp.lib_paths`、js-proc のブリッジ側の解決。**ドット付きの書き方では見ない**
//   （Python の相対 import と同じ）。
// - ⚠ **エントリのディレクトリ（`root_dir`）からは探さない**。`root_dir` は下の
//   「モジュールの名前」の基準にだけ使う。
//
// ## モジュールの同一性
//
// 実行時のモジュールキャッシュ・型レジストリ・展開器は、import 文の `module`
// （`["a", "b"]`）でモジュールを区別する。書いた綴りのままだと、別のディレクトリの
// 別のファイルが同じ `util` になり、**後から読んだ側が先に読んだ側の名前空間を受け取っていた**
// （実測: `util.ar` と `pkg/util.ar`）。
// ⇒ パーサが `module` を**ファイルごとに一意な名前**（[`ModuleNames::assign`]）へ書き換える。
//   名前はエントリのディレクトリからの相対パス（`pkg.util`）。エントリより上のディレクトリは
//   [`PARENT_SEGMENT`] で表す（`__parent__.util`）。型の修飾名（`pkg.util.Tag`）にも使うので、
//   各部分は識別子でなければならない（`..` は使えない）。

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

/// エントリのディレクトリより上を表すモジュール名の部分（`..` の代わり）。
pub const PARENT_SEGMENT: &str = "__parent__";

/// パスを**字句的に**正規化する（`.` を消し、`..` を手前の部分と打ち消す）。fs は引かない。
///
/// ⚠ 打ち消せない `..`（相対パスの先頭）はそのまま残す。
/// ⚠ シンボリックリンクは解かない（`canonicalize` は Windows で `\\?\` 付きになり、
///   エラーメッセージに出るパスが読みにくくなる）。
pub fn normalize(p: &Path) -> PathBuf {
    let mut out: Vec<Component> = Vec::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                // ルートより上へは行けない（`/..` は `/`）。
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                _ => out.push(c),
            },
            _ => out.push(c),
        }
    }
    out.iter().collect()
}

/// 絶対パスにしてから正規化する（キャッシュ・同一性の鍵用）。
pub fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return normalize(p);
    }
    match std::env::current_dir() {
        Ok(cwd) => normalize(&cwd.join(p)),
        Err(_) => normalize(p),
    }
}

/// 探索の起点。`dir` は import 文を書いたファイルのディレクトリ、`level` は先頭のドットの数。
///
/// - `level` 0（ドット無し）・1（`.`）… `dir` そのもの
/// - `level` k（k ≥ 2）… `dir` の k-1 個上
pub fn search_base(dir: &Path, level: u32) -> PathBuf {
    if level <= 1 {
        return dir.to_path_buf();
    }
    let mut p = dir.to_path_buf();
    for _ in 1..level {
        p.push("..");
    }
    normalize(&p)
}

/// 言語ごとの外部の探索先を見てよいか（ドット付きの書き方では見ない）。
pub fn uses_external_paths(level: u32) -> bool {
    level == 0
}

/// ソースに書かれた綴り（`..lib.helper`）。
pub fn written_spelling(level: u32, module: &[String]) -> String {
    format!("{}{}", ".".repeat(level as usize), module.join("."))
}

/// モジュールファイルの**同一性の鍵**（絶対パスから拡張子を落としたもの）。
///
/// - `a/b.ar` と `a/b.arc` は同じモジュール（コンパイル済みかどうかの違いだけ）。
/// - `a/b/__init__.ar` は `a/b`。
pub fn module_identity(file: &Path) -> PathBuf {
    let abs = absolute(file);
    let is_init = abs.file_stem().is_some_and(|s| s == "__init__");
    if is_init {
        return abs.parent().map(Path::to_path_buf).unwrap_or(abs);
    }
    abs.with_extension("")
}

/// 識別子として書ける綴りか（モジュール名の各部分・型の修飾名に使うため）。
fn is_ident(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && cs.all(|c| c.is_alphanumeric() || c == '_')
}

/// エントリのディレクトリ（`root`）からの相対で、モジュールの名前を作る。
///
/// `identity` は [`module_identity`] の結果。`root` より上は [`PARENT_SEGMENT`] になる。
/// 作れない（別のドライブ・識別子でないディレクトリ名を通る）ときは `None`。
pub fn root_relative_name(identity: &Path, root: &Path) -> Option<Vec<String>> {
    let root = absolute(root);
    let file: Vec<Component> = identity.components().collect();
    let base: Vec<Component> = root.components().collect();
    let common = file.iter().zip(&base).take_while(|(a, b)| a == b).count();
    // ⚠ 先頭（ドライブ・ルート）すら共有しないなら相対にできない。
    if common == 0 {
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    for _ in common..base.len() {
        out.push(PARENT_SEGMENT.to_string());
    }
    for c in &file[common..] {
        let Component::Normal(s) = c else { return None };
        let s = s.to_str()?;
        if !is_ident(s) {
            return None;
        }
        out.push(s.to_string());
    }
    // エントリのディレクトリそのもの（`import .` 相当）は名前にならない。
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// ファイル ↔ モジュール名の対応表（プログラム全体で 1 つ。サブパーサと共有する）。
///
/// - 同じファイルは、どの綴りで import しても**同じ名前**になる（最初に付けた名前）。
///   例: `import util`（エントリから）と `import ..util`（`pkg/` から）。
/// - 違うファイルに同じ名前は付けない（付けようとしたら明示エラー）。
#[derive(Default, Debug)]
pub struct ModuleNames {
    by_identity: HashMap<PathBuf, Vec<String>>,
    by_name: HashMap<String, PathBuf>,
}

impl ModuleNames {
    /// `identity` のモジュールの名前を決める。初めてなら `preferred` を付ける。
    ///
    /// ⚠ `preferred` が別のファイルに付いていたら `Err`（2 つのファイルが同じ名前を取り合う）。
    pub fn assign(&mut self, identity: &Path, preferred: Vec<String>) -> Result<Vec<String>, String> {
        if let Some(name) = self.by_identity.get(identity) {
            return Ok(name.clone());
        }
        let joined = preferred.join(".");
        if let Some(other) = self.by_name.get(&joined) {
            if other != identity {
                return Err(format!(
                    "module name '{joined}' refers to two different files: '{}' and '{}'",
                    other.display(),
                    identity.display()
                ));
            }
        }
        self.by_name.insert(joined, identity.to_path_buf());
        self.by_identity.insert(identity.to_path_buf(), preferred.clone());
        Ok(preferred)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn normalize_folds_parent_components() {
        assert_eq!(normalize(&p("pkg/../util.ar")), p("util.ar"));
        assert_eq!(normalize(&p("a/./b/../c")), p("a/c"));
        // 打ち消せない `..` は残す。
        assert_eq!(normalize(&p("../x")), p("../x"));
        assert_eq!(normalize(&p("a/../../x")), p("../x"));
        // 空のディレクトリ（`main.ar` の親）は空のまま。
        assert_eq!(normalize(&p("")), p(""));
    }

    #[test]
    fn search_base_by_level() {
        assert_eq!(search_base(&p("pkg"), 0), p("pkg"));
        assert_eq!(search_base(&p("pkg"), 1), p("pkg"));
        assert_eq!(search_base(&p("pkg"), 2), p(""));
        assert_eq!(search_base(&p("pkg"), 3), p(".."));
        assert_eq!(search_base(&p("a/b"), 3), p(""));
    }

    #[test]
    fn written_spelling_keeps_dots() {
        let m = vec!["lib".to_string(), "helper".to_string()];
        assert_eq!(written_spelling(0, &m), "lib.helper");
        assert_eq!(written_spelling(2, &m), "..lib.helper");
    }

    #[test]
    fn identity_drops_extension_and_init() {
        let root = absolute(&p("proj"));
        assert_eq!(module_identity(&p("proj/a.ar")), root.join("a"));
        assert_eq!(module_identity(&p("proj/a.arc")), root.join("a"));
        assert_eq!(module_identity(&p("proj/a/__init__.ar")), root.join("a"));
    }

    #[test]
    fn root_relative_names() {
        let root = absolute(&p("proj/pkg"));
        let id = |s: &str| module_identity(&p(s));
        assert_eq!(root_relative_name(&id("proj/pkg/util.ar"), &root).unwrap(), ["util"]);
        assert_eq!(root_relative_name(&id("proj/pkg/a/b.ar"), &root).unwrap(), ["a", "b"]);
        assert_eq!(
            root_relative_name(&id("proj/util.ar"), &root).unwrap(),
            [PARENT_SEGMENT, "util"]
        );
        assert_eq!(
            root_relative_name(&id("proj/lib/helper.ar"), &root).unwrap(),
            [PARENT_SEGMENT, "lib", "helper"]
        );
        // 識別子でないディレクトリ名を通るなら作れない。
        assert!(root_relative_name(&id("proj/pkg/my-dir/x.ar"), &root).is_none());
    }

    #[test]
    fn names_are_unique_per_file() {
        let mut names = ModuleNames::default();
        let a = p("/proj/util");
        let b = p("/proj/pkg/util");
        assert_eq!(names.assign(&a, vec!["util".into()]).unwrap(), ["util"]);
        // 同じファイルは最初の名前のまま。
        assert_eq!(names.assign(&a, vec!["other".into()]).unwrap(), ["util"]);
        // 違うファイルが同じ名前を取ろうとしたら誤り。
        assert!(names.assign(&b, vec!["util".into()]).is_err());
        assert_eq!(names.assign(&b, vec!["pkg".into(), "util".into()]).unwrap(), ["pkg", "util"]);
    }
}
