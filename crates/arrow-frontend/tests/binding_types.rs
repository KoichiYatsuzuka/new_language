//! 変数の型（hover / inlay hint）が、型検査器の**束縛の記録**（`BindingRecord`）から届くことの回帰テスト。
//!
//! # ⚠ ここが守る性質
//!
//! 1. **初期化式の種類に左右されない** — 以前は初期化式の node-id で型を引いていたので、
//!    node-id を持たない式（リテラル・`if` 式・`for` 式）と、node-id の取り出しに漏れた式
//!    （`mustbe`）の宣言に型が出なかった。
//! 2. **初期化式の無い束縛にも型が出る** — ループ変数・`except as`・分割代入。
//! 3. **同じ文・同じ名前の束縛を取り違えない** — 突き合わせは「文の位置＋名前」の中の順番で行う。
//! 4. **別のファイルの束縛を混ぜない** — import 先の本体（スタブ）も検査の途中で束縛するが、
//!    その位置は別のファイルの行・列を指す。
//! 5. **テンプレートは本体の型を出す** — 単相化した具体化は同じ位置を型引数ごとに検査する。
//!
//! ⚠ このクレートはワークスペースから `exclude` されている。**ルートの `cargo test` では
//!    走らない。** `cd crates/arrow-frontend && cargo test` で実行すること。

use arrow_frontend::analyze::{analyze_file_json, analyze_json};
use serde_json::Value;

fn analyze(source: &str) -> Value {
    let raw = analyze_json(source, "test.ar");
    let v: Value = serde_json::from_str(&raw).expect("analyze_json must return valid JSON");
    assert_eq!(
        v["ok"],
        Value::Bool(true),
        "source failed to parse: {}",
        v["parseError"]
    );
    v
}

/// 名前が `name` の変数宣言の型を、ソースに現れる順（行・列）に並べて返す。
fn types_of(v: &Value, name: &str) -> Vec<Option<String>> {
    let mut found: Vec<((u64, u64), Option<String>)> = v["symbols"]
        .as_array()
        .expect("symbols")
        .iter()
        .filter(|s| s["name"] == name && s["kind"] == "variable")
        .map(|s| {
            let at = (s["at"]["line"].as_u64().unwrap(), s["at"]["col"].as_u64().unwrap());
            (at, s["inferred"].as_str().map(str::to_string))
        })
        .collect();
    found.sort_by_key(|(at, _)| *at);
    found.into_iter().map(|(_, t)| t).collect()
}

/// 名前が `name` の変数宣言がちょうど 1 つあり、その型が `expected` であること。
#[track_caller]
fn assert_type(v: &Value, name: &str, expected: &str) {
    assert_eq!(types_of(v, name), vec![Some(expected.to_string())], "variable `{name}`");
}

/// 元の不具合: `mustbe` を初期化式に持つ宣言に型が出なかった。
#[test]
fn mustbe_initializer_has_the_guard_type() {
    let v = analyze(concat!(
        "fn get() -> Any:\n",
        "    return 1\n",
        "\n",
        "fn main() -> None:\n",
        "    let a = get() mustbe int\n",
        "    let d = get() mustbe dict[str, Any]\n",
        "    print(a, d)\n",
    ));
    assert_type(&v, "a", "int");
    assert_type(&v, "d", "dict[str,Any]");
}

/// node-id を持たない初期化式（リテラル・単項演算・`if` 式・`for` 式）。
#[test]
fn initializers_without_node_id_have_types() {
    let v = analyze(concat!(
        "fn main() -> None:\n",
        "    let n = 1\n",
        "    mut s = \"x\"\n",
        "    let lst = [1, 2]\n",
        "    let w = -n\n",
        "    let e = if n == 1 ->int:\n",
        "        block_return 1\n",
        "    else:\n",
        "        block_return 0\n",
        "    let names: list[str] = [\"a\"]\n",
        "    let ys = for x in names ->list[str]:\n",
        "        loop_yield x\n",
        "    print(n, s, lst, w, e, ys)\n",
    ));
    assert_type(&v, "n", "int");
    assert_type(&v, "s", "str");
    assert_type(&v, "lst", "list[int]");
    assert_type(&v, "w", "int");
    assert_type(&v, "e", "int");
    assert_type(&v, "ys", "list[str]");
    assert_type(&v, "x", "str");
}

/// 初期化式の無い束縛: ループ変数（単一・複数ターゲット）。
#[test]
fn loop_variables_have_element_types() {
    let v = analyze(concat!(
        "fn main() -> None:\n",
        "    let names: list[str] = [\"a\", \"b\"]\n",
        "    for name in names:\n",
        "        print(name)\n",
        "    let pairs: list[tuple[int, str]] = [(1, \"a\")]\n",
        "    for k, t in pairs:\n",
        "        print(k, t)\n",
    ));
    assert_type(&v, "name", "str");
    assert_type(&v, "k", "int");
    assert_type(&v, "t", "str");
}

/// 初期化式の無い束縛: 分割代入と `except as`。
#[test]
fn tuple_targets_and_except_names_have_types() {
    let v = analyze(concat!(
        "fn pair() -> tuple[int, str]:\n",
        "    return (1, \"a\")\n",
        "\n",
        "fn main() -> None:\n",
        "    let a, mut b = pair()\n",
        "    try:\n",
        "        print(a, b)\n",
        "    except ValueError as err:\n",
        "        print(err)\n",
    ));
    assert_type(&v, "a", "int");
    assert_type(&v, "b", "str");
    assert_type(&v, "err", "ValueError");
}

/// 同じ文（`try`）の中の同じ名前の束縛は、束縛した順に突き合わせる。
#[test]
fn same_name_in_one_statement_is_matched_in_order() {
    let v = analyze(concat!(
        "fn main() -> None:\n",
        "    try:\n",
        "        print(1)\n",
        "    except ValueError as e:\n",
        "        print(e)\n",
        "    except KeyError as e:\n",
        "        print(e)\n",
    ));
    assert_eq!(
        types_of(&v, "e"),
        vec![Some("ValueError".to_string()), Some("KeyError".to_string())]
    );
}

/// `let x = for x in ...` — 初期化式の中の束縛（`for` 式の `x`）は、外の `let x` より**先に**束縛される。
/// パーサが `let x` を先に読むからといって先に番号を振ると、2 つの型が入れ替わる。
#[test]
fn binding_inside_the_initializer_comes_first() {
    let v = analyze(concat!(
        "fn main() -> None:\n",
        "    let names: list[str] = [\"a\"]\n",
        "    let x = for x in names ->list[str]:\n",
        "        loop_yield x\n",
        "    print(x)\n",
    ));
    // ソース順: `let x`（列 8）、`for x`（列 21）
    assert_eq!(
        types_of(&v, "x"),
        vec![Some("list[str]".to_string()), Some("str".to_string())]
    );
}

/// クラスのフィールドの初期値の中の束縛。メンバーは `parse_stmt` を通らないので、
/// `parse_class_stmt` でも文の位置を決めていないと鍵が食い違う。
#[test]
fn binding_in_a_field_initializer_has_a_type() {
    let v = analyze(concat!(
        "class Listed:\n",
        "    const xs: list[int] = for i in range(3) ->list[int]:\n",
        "        loop_yield i\n",
        "\n",
        "print(Listed().xs)\n",
    ));
    assert_type(&v, "i", "int");
}

/// テンプレートの本体の変数は型変数のまま。具体化（`ident[int]` / `ident[str]`）の型を出さない。
#[test]
fn template_body_shows_the_type_parameter() {
    let v = analyze(concat!(
        "fn ident[T](let a: T) -> T:\n",
        "    let y = a\n",
        "    return y\n",
        "\n",
        "print(ident[int](1))\n",
        "print(ident[str](\"s\"))\n",
    ));
    assert_type(&v, "y", "T");
}

/// import 先の本体の束縛は、このファイルの同じ行・列・名前の束縛に混ざらない。
///
/// ⚠ モジュールの `let v` は `m.ar` の 1 行 1 列、このファイルの `let v` も 1 行 1 列。
///   ファイルで分けないと 1 つの鍵に 2 つの型（`int` と `str`）が並び、どちらも出せなくなる。
#[test]
fn module_bindings_do_not_leak_into_this_file() {
    let dir = std::env::temp_dir().join(format!("arrow_frontend_binding_types_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("m.ar"), "let v: int = 1\n").unwrap();
    let main = dir.join("main.ar");
    let raw = analyze_file_json(
        concat!("let v = \"s\"\n", "import m\n", "print(v, m.v)\n"),
        &main.to_string_lossy(),
    );
    let v: Value = serde_json::from_str(&raw).expect("analyze_file_json must return valid JSON");
    assert_eq!(v["ok"], Value::Bool(true), "source failed to parse: {}", v["parseError"]);
    assert_type(&v, "v", "str");
}
