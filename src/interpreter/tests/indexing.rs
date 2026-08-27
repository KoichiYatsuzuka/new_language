// tests/indexing.rs — __getitem__ / __setitem__ (list/str/dict/instance/PyObject) のテスト。

use super::*;

// ---------------------------------------------------------------------------
// __getitem__ / __setitem__ — list, str, dict, instance, PyObject
// ---------------------------------------------------------------------------

/// list_getitem のテスト。
#[test]
fn test_list_getitem() {
    // list[int] インデックスアクセス（正・負）
    let src = concat!(
        "let xs = [10, 20, 30]\n",
        "let a = xs[0]\n",
        "let b = xs[2]\n",
        "let c = xs[-1]\n",
    );
    assert!(matches!(run_get(src, "a"), Value::Int(10)));
    assert!(matches!(run_get(src, "b"), Value::Int(30)));
    assert!(matches!(run_get(src, "c"), Value::Int(30)));
}

/// list_setitem のテスト。
#[test]
fn test_list_setitem() {
    // list[int] = value による要素の書き換え
    let src = concat!("mut xs = [1, 2, 3]\n", "xs[1] = 99\n", "let r = xs[1]\n",);
    assert!(matches!(run_get(src, "r"), Value::Int(99)));
}

/// list_setitem_negative のテスト。
#[test]
fn test_list_setitem_negative() {
    // 負インデックスでの書き換え
    let src = concat!("mut xs = [1, 2, 3]\n", "xs[-1] = 77\n", "let r = xs[2]\n",);
    assert!(matches!(run_get(src, "r"), Value::Int(77)));
}

/// list_getitem_out_of_range のテスト。
#[test]
fn test_list_getitem_out_of_range() {
    let src = concat!("let xs = [1, 2, 3]\n", "let r = xs[5]\n",);
    assert!(run(src).is_err());
}

/// str_getitem のテスト。
#[test]
fn test_str_getitem() {
    // str[int] インデックスアクセス（正・負）
    let src = concat!("let s = \"hello\"\n", "let a = s[0]\n", "let b = s[-1]\n",);
    if let Value::Str(a) = run_get(src, "a") {
        assert_eq!(&*a, "h");
    } else {
        panic!("expected Str");
    }
    if let Value::Str(b) = run_get(src, "b") {
        assert_eq!(&*b, "o");
    } else {
        panic!("expected Str");
    }
}

/// instance_getitem_setitem のテスト。
#[test]
fn test_instance_getitem_setitem() {
    // ユーザー定義クラスの __getitem__ / __setitem__
    let src = concat!(
        "class Box:\n",
        "    mut data: int\n",
        "    fn __init__(mut self) -> None:\n",
        "        self.data = 0\n",
        "    fn __getitem__(self, let key: int) -> int:\n",
        "        return self.data + key\n",
        "    fn __setitem__(mut self, let key: int, let val: int) -> None:\n",
        "        self.data = val\n",
        "mut b = Box()\n",
        "b[10] = 5\n",
        "let r = b[1]\n",
    );
    assert!(matches!(run_get(src, "r"), Value::Int(6)));
}

/// pyobject_getitem のテスト。
#[test]
fn test_pyobject_getitem() {
    // PyObject の subscript read: Container.__getitem__
    let src = concat!(
        "import[py-int] py_calculator as calc\n",
        "let c = calc.make_container([10, 20, 30])\n",
        "let r = c[1]\n",
    );
    if let Value::Int(n) = run_py_get(src, "r") {
        assert_eq!(n, 20);
    } else {
        panic!("expected Int(20)");
    }
}

/// pyobject_setitem のテスト。
#[test]
fn test_pyobject_setitem() {
    // PyObject の subscript write: Container.__setitem__
    let src = concat!(
        "import[py-int] py_calculator as calc\n",
        "let c = calc.make_container([1, 2, 3])\n",
        "c[0] = 99\n",
        "let r = c[0]\n",
    );
    if let Value::Int(n) = run_py_get(src, "r") {
        assert_eq!(n, 99);
    } else {
        panic!("expected Int(99)");
    }
}

/// tuple_getitem のテスト。
#[test]
fn test_tuple_getitem() {
    // Python から返ってきた tuple が Value::Tuple に変換された場合の subscript
    let src = concat!("let t = (100, 200, 300)\n", "let r = t[1]\n",);
    assert!(matches!(run_get(src, "r"), Value::Int(200)));
}

