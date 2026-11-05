// type_check_tests/py_collections.rs — `py_list` / `py_dict` / `py_set` の型検査（pandas_import_plan.md の D-12・2-7）。
//
// 中身が CPython の物の組み込みの型。Arrow の list / dict / set とは別の型で、キャスト `=>` で写す。

use super::*;

fn kinds(source: &str) -> Vec<TypeErrorKind> {
    check(source).into_iter().map(|e| e.kind).collect()
}

#[test]
fn operations_have_python_collection_types() {
    let src = concat!(
        "mut h = [3, 1] => py_list[int]\n",
        "let a: int = h[0]\n",
        "mut part: py_list[int] = h[1:]\n",
        "mut both: py_list[int] = h + h\n",
        "let n: int = len(h) + h.pop() + h.index(1)\n",
        "for x in h:\n",
        "    let y: int = x\n",
        "mut d = {\"a\": 1} => py_dict[str, int]\n",
        "let v: int = d[\"a\"]\n",
        "for k, w in d.items():\n",
        "    let s: str = k\n",
        "mut s = {1} => py_set[int]\n",
        "mut u: py_set[int] = s | s\n",
        "let b: bool = 1 in s and h == h\n",
    );
    assert!(ok(src), "{:?}", check(src));
}

#[test]
fn element_types_and_members_are_checked() {
    assert!(err("mut h = [1] => py_list[int]\nlet a: str = h[0]\n"));
    assert!(err("mut h = [1] => py_list[int]\nh.append(\"s\")\n"));
    assert!(err("mut h = [1] => py_list[int]\nh[0] = \"s\"\n"));
    // メソッドは CPython の list と同じものだけ（閉じている）
    assert!(kinds("mut h = [1] => py_list[int]\nh.push(1)\n")
        .iter()
        .any(|k| matches!(k, TypeErrorKind::NoSuchMember { member, .. } if member == "push")));
    assert!(err("mut d = {\"a\": 1} => py_dict[str, int]\nd[1] = 2\n"));
    // set は添字を取らない
    assert!(err("mut s = {1} => py_set[int]\nlet x = s[0]\n"));
}

#[test]
fn not_interchangeable_with_arrow_collections() {
    assert!(err("mut h: py_list[int] = [1]\n"));
    assert!(err("let xs: list[int] = [1] => py_list[int]\n"));
    assert!(err("mut h = [1] => py_list[int]\nmut j = h + [2]\n"));
    assert!(ok("mut h = [1] => py_list[int]\nlet xs: list[int] = h => list[int]\n"));
}

#[test]
fn python_collections_are_bound_with_mut() {
    assert!(kinds("let h = [1] => py_list[int]\n")
        .iter()
        .any(|k| matches!(k, TypeErrorKind::ImmutablePythonObject { .. })));
    assert!(err("fn f(h: py_list[int]) -> None:\n    pass\n"));
    assert!(ok("fn f(mut h: py_list[int]) -> int:\n    return len(h)\n"));
}

#[test]
fn casts_convert_to_the_depth_of_the_target() {
    assert!(ok("mut g = [[1], [2]] => py_list[py_list[int]]\nlet a = g => list[list[int]]\nmut b = g => list[py_list[int]]\n"));
    assert!(ok("mut d = {\"a\": [1]} => py_dict[str, py_list[int]]\n"));
    assert!(err("mut g = [1, 2] => py_list[str]\n"));
    assert!(err("mut g = {\"a\": 1} => py_list[int]\n"));
    // Arrow の list は CPython の物の要素に置けない
    assert!(kinds("mut g = [[1]] => py_list[list[int]]\n")
        .iter()
        .any(|k| matches!(k, TypeErrorKind::ArrowCollectionInPython { .. })));
    assert!(kinds("mut g: py_dict[str, list[int]] = {} => py_dict[str, py_list[int]]\n")
        .iter()
        .any(|k| matches!(k, TypeErrorKind::ArrowCollectionInPython { .. })));
}

#[test]
fn bare_python_collections_only_in_type_tests() {
    assert!(err("mut h: py_list = [1] => py_list[int]\n"));
    assert!(ok("mut h = [1] => py_list[int]\nlet b: bool = h is py_list\n"));
}

#[test]
fn arrow_collections_need_a_cast_to_reach_python() {
    // `py_list` のメソッド・CPython の物の演算は、Arrow の list をキャストせずに受けない（2-11）
    let to_python = |src: &str| {
        kinds(src).iter().any(|k| matches!(k, TypeErrorKind::ArrowCollectionToPython { .. }))
    };
    assert!(to_python("mut h = [1] => py_list[Any]\nh.append([1])\n"));
    assert!(to_python("mut h = [1] => py_list[Any]\nh.extend([2])\n"));
    assert!(to_python("mut d = {\"a\": 1} => py_dict[str, Any]\nd[\"b\"] = {\"c\": 1}\n"));
    assert!(to_python("mut h = [1] => py_list[Any]\nh[0] = (1, [2])\n"));
    // 写せば渡せる。Arrow の list の中の Python の物を `in` で探すのは Arrow の側で比べる
    assert!(ok("mut h = [1] => py_list[Any]\nh.append([1] => py_list[int])\n"));
    assert!(ok("mut h = [1] => py_list[int]\nmut hs = [h]\nlet b: bool = h in hs\n"));
}

#[test]
fn the_suggested_conversion_names_python_collections() {
    use crate::type_check::TypeChecker;
    let t = InferredType::from_ann("tuple[list[int], dict[str, set[str]], int]").unwrap();
    assert_eq!(
        TypeChecker::arrow_collection_as_python(&t).map(|t| t.to_string()).as_deref(),
        Some("tuple[py_list[int], py_dict[str, py_set[str]], int]")
    );
    assert!(TypeChecker::arrow_collection_as_python(&InferredType::from_ann("tuple[int, str]").unwrap()).is_none());
}

#[test]
fn generator_parameters_may_hold_python_objects() {
    // `gen` の引数は `let` / `const` だけなので、Python の物も `let` で受けられる（2-13・D-3 の例外）
    assert!(ok("gen each(let xs: py_list[int]) -> int:\n    for x in xs:\n        yield x\n"));
    assert!(ok("gen each(xs: py_list[int]) -> int:\n    for x in xs:\n        yield x\n"));
    // ふつうの関数の引数は今までどおり `mut`
    assert!(err("fn f(let xs: py_list[int]) -> None:\n    pass\n"));
}
