//! import で束縛した名前の**種類・定義の場所**と、`.` 補完のモジュールのメンバ表の鍵の回帰テスト
//! （VSIX_update_plan.md 4-2〜4-4）。
//!
//! # ここが守る性質
//!
//! 1. `from X import Y` の `Y` は、import 先での正体（関数・クラス・定数・サブモジュール）になる。
//!    以前は一律に `module` で、拡張は関数をモジュールの色で塗り、ホバーにも `from X import Y` しか出さなかった。
//! 2. 正体が分からない名前は `unresolved`（黙って `module` などにしない）。
//! 3. import で束縛した名前は、定義の場所（`def`）が import 先のファイルを指す。以前は定義へのジャンプが
//!    import 文に飛んで終わっていた。
//! 4. モジュールのメンバ表の鍵は CLI が束縛する名前（`import pkg.util` なら `pkg`）。以前は末尾（`util`）だった。
//!
//! ⚠ ルートの `cargo test` では走らない（`tests/imports.rs` の注意と同じ）。

use std::path::PathBuf;

use arrow_frontend::analyze::analyze_file_json;
use serde_json::Value;

fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("arrow_frontend_import_kinds_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, src) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, src).unwrap();
    }
    dir
}

fn analyze_at(dir: &PathBuf, main: &str) -> Value {
    let path = dir.join(main);
    let source = std::fs::read_to_string(&path).unwrap();
    let raw = analyze_file_json(&source, &path.to_string_lossy());
    let v: Value = serde_json::from_str(&raw).expect("analyze_file_json must return valid JSON");
    assert_eq!(v["ok"], Value::Bool(true), "failed to parse: {}", v["parseError"]);
    v
}

fn symbol<'a>(v: &'a Value, name: &str) -> &'a Value {
    v["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == name)
        .unwrap_or_else(|| panic!("no symbol `{name}`: {}", v["symbols"]))
}

/// `def` が指すファイルの名前（ディレクトリを除く）と 0 始まりの行・文字列。
fn def_of(sym: &Value) -> Option<(String, u64, u64)> {
    let d = sym.get("def")?.as_object()?;
    let file = PathBuf::from(d["file"].as_str()?).file_name()?.to_string_lossy().into_owned();
    Some((file, d["line"].as_u64()?, d["charCol"].as_u64()?))
}

fn member_names(v: &Value, key: &str) -> Vec<String> {
    v["members"][key]["members"]
        .as_array()
        .unwrap_or_else(|| panic!("no member table `{key}`: keys = {:?}", v["members"].as_object().map(|m| m.keys().collect::<Vec<_>>())))
        .iter()
        .map(|m| m["name"].as_str().unwrap().to_string())
        .collect()
}

const UTIL: &str = "\
fn helper(let x: int) -> int:
    \"\"\"1 を足す\"\"\"
    return x + 1

gen count(let n: int) -> int:
    yield n

class Other:
    let v: int = 0

const LIMIT: int = 10
";

/// `from X import Y` の `Y` は import 先での正体になり、定義の場所はそのファイルを指す。
#[test]
fn from_import_names_take_the_kind_they_have_in_the_module() {
    let dir = project("kinds", &[
        ("pkg/__init__.ar", ""),
        ("pkg/util.ar", UTIL),
        ("main.ar", "from pkg.util import helper, count, Other as O, LIMIT\n\nlet a = helper(1)\n"),
    ]);
    let v = analyze_at(&dir, "main.ar");

    let helper = symbol(&v, "helper");
    assert_eq!(helper["kind"], "function", "{helper}");
    assert_eq!(helper["signature"], "fn helper(let x: int) -> int", "{helper}");
    assert_eq!(helper["doc"], "1 を足す", "{helper}");
    assert_eq!(def_of(helper), Some(("util.ar".to_string(), 0, 3)), "{helper}");

    assert_eq!(symbol(&v, "count")["kind"], "generator");
    let o = symbol(&v, "O");
    assert_eq!(o["kind"], "class", "{o}");
    assert_eq!(def_of(o), Some(("util.ar".to_string(), 7, 6)), "{o}");
    let limit = symbol(&v, "LIMIT");
    assert_eq!(limit["kind"], "variable", "{limit}");
    assert_eq!(limit["mutability"], "const", "{limit}");
    assert_eq!(limit["typeAnn"], "int", "{limit}");
}

/// サブモジュール（`from pkg import util`）・再エクスポート（`from mid import helper`・`mid` が `from pkg.util import helper`）。
#[test]
fn submodules_and_reexports_are_followed() {
    let dir = project("follow", &[
        ("pkg/__init__.ar", ""),
        ("pkg/util.ar", UTIL),
        ("mid.ar", "from pkg.util import helper\n"),
        ("main.ar", "from pkg import util\nfrom mid import helper\n\nlet a = helper(util.helper(1))\n"),
    ]);
    let v = analyze_at(&dir, "main.ar");
    let util = symbol(&v, "util");
    assert_eq!(util["kind"], "module", "{util}");
    assert_eq!(def_of(util), Some(("util.ar".to_string(), 0, 0)), "{util}");
    let helper = symbol(&v, "helper");
    assert_eq!(helper["kind"], "function", "{helper}");
    assert_eq!(def_of(helper), Some(("util.ar".to_string(), 0, 3)), "再エクスポートを元の定義までたどっていない: {helper}");
}

/// `from . import x`（サブモジュール）と `from .m import y`。
#[test]
fn relative_from_imports() {
    let dir = project("relative", &[
        ("util.ar", UTIL),
        ("main.ar", "from . import util\nfrom .util import Other\n\nlet o = Other()\n"),
    ]);
    let v = analyze_at(&dir, "main.ar");
    assert_eq!(symbol(&v, "util")["kind"], "module");
    assert_eq!(symbol(&v, "Other")["kind"], "class");
}

/// 正体が分からない名前は `unresolved`（モジュールに無い名前・読めない import）。
#[test]
fn unknown_names_are_unresolved_not_modules() {
    let dir = project("unknown", &[
        ("util.ar", UTIL),
        ("main.ar", "from util import no_such_name\nfrom no_such_module_xyz import thing\n"),
    ]);
    let v = analyze_at(&dir, "main.ar");
    for name in ["no_such_name", "thing"] {
        let s = symbol(&v, name);
        assert_eq!(s["kind"], "unresolved", "{s}");
        assert_eq!(s["def"], Value::Null, "{s}");
    }
}

/// `import m` の定義の場所はモジュールのファイル。`import a.b` は束縛するパッケージ `a` のファイル。
#[test]
fn import_binds_point_at_the_module_file() {
    let dir = project("module_def", &[
        ("pkg/__init__.ar", "const VERSION: int = 1\n"),
        ("pkg/util.ar", UTIL),
        ("main.ar", "import pkg.util\nimport pkg.util as u\n"),
    ]);
    let v = analyze_at(&dir, "main.ar");
    assert_eq!(def_of(symbol(&v, "pkg")), Some(("__init__.ar".to_string(), 0, 0)));
    assert_eq!(def_of(symbol(&v, "u")), Some(("util.ar".to_string(), 0, 0)));
}

/// メンバ表の鍵は束縛名。`import pkg.util` の `pkg.` にはサブモジュール `util` が出て、その表は `pkg.util`。
#[test]
fn member_tables_are_keyed_by_the_bound_name() {
    let dir = project("members", &[
        ("pkg/__init__.ar", "const VERSION: int = 1\n"),
        ("pkg/util.ar", UTIL),
        ("main.ar", "import pkg.util\nimport pkg.util as u\n"),
    ]);
    let v = analyze_at(&dir, "main.ar");
    let pkg = member_names(&v, "pkg");
    assert!(pkg.contains(&"VERSION".to_string()) && pkg.contains(&"util".to_string()), "{pkg:?}");
    let util_entry = v["members"]["pkg"]["members"].as_array().unwrap().iter().find(|m| m["name"] == "util").unwrap();
    assert_eq!(util_entry["kind"], "module");
    assert_eq!(util_entry["type"], "pkg.util", "サブモジュールの表を指していない: {util_entry}");
    for key in ["pkg.util", "u"] {
        let names = member_names(&v, key);
        assert!(names.contains(&"helper".to_string()) && names.contains(&"Other".to_string()), "{key}: {names:?}");
    }
    assert!(v["members"].get("util").is_none(), "束縛されていない `util` を鍵にしている");

    // メンバーにも定義の場所が付く（`u.helper` から飛べる）。
    let helper = v["members"]["u"]["members"].as_array().unwrap().iter().find(|m| m["name"] == "helper").unwrap();
    assert_eq!(def_of(helper), Some(("util.ar".to_string(), 0, 3)), "{helper}");

    // 型検査器が呼ぶ修飾名（`pkg.util.Other`）でもクラスの表が引ける（`f().` の受け手の型・5-5）。
    assert_eq!(member_names(&v, "pkg.util.Other"), vec!["v".to_string()]);
}

/// import で束縛した名前には `imported` が立つ（`at` は import 文なので、拡張は `def` が無ければ飛ばない）。
#[test]
fn import_bindings_are_marked_imported() {
    let dir = project("imported", &[
        ("util.ar", UTIL),
        ("main.ar", "import util\nfrom util import helper, no_such_name\n\nfn local() -> int:\n    return 1\n"),
    ]);
    let v = analyze_at(&dir, "main.ar");
    for name in ["util", "helper", "no_such_name"] {
        assert_eq!(symbol(&v, name)["imported"], true, "{name}");
    }
    assert_eq!(symbol(&v, "local")["imported"], false);
    assert_eq!(symbol(&v, "local")["def"], Value::Null);
}
