// tests/iterator.rs — イテレータ・反復処理のテスト。

use super::*;

// --- iterator ---

/// list_iter_for_loop のテスト。
#[test]
fn test_list_iter_for_loop() {
    // for loop over a list uses __iter__ internally
    let src = "mut s = 0\nfor x in [1, 2, 3, 4]:\n    s += x\n";
    if let Value::Int(n) = run_get(src, "s") {
        assert_eq!(n, 10);
    } else {
        panic!("expected int 10");
    }
}

/// list_iter_method_direct のテスト。
#[test]
fn test_list_iter_method_direct() {
    // list.__iter__() returns a Generator; .next() yields each element
    let src = concat!(
        "let lst = [10, 20, 30]\n",
        "let it = lst.__iter__()\n",
        "let a = it.next()\n",
        "let b = it.next()\n",
        "let c = it.next()\n",
    );
    assert!(matches!(run_get(src, "a"), Value::Int(10)));
    assert!(matches!(run_get(src, "b"), Value::Int(20)));
    assert!(matches!(run_get(src, "c"), Value::Int(30)));
}

/// generator_exhausted_raises_end_of_iteration のテスト。
#[test]
fn test_generator_exhausted_raises_end_of_iteration() {
    // .next() on an exhausted generator must raise EndOfIteration
    let src = concat!(
        "let lst = [1]\n",
        "let it = lst.__iter__()\n",
        "let _ = it.next()\n",
        "it.next()\n",
    );
    assert!(run(src).is_err());
    let err_msg = run_err_msg(src);
    assert!(
        err_msg.starts_with("EndOfIteration"),
        "expected EndOfIteration, got: {err_msg}"
    );
}

/// custom_iter_class のテスト。
#[test]
fn test_custom_iter_class() {
    // A class with gen __iter__ can be used in a for loop
    let src = concat!(
        "class Range3:\n",
        "    gen __iter__(self) -> int:\n",
        "        yield 0\n",
        "        yield 1\n",
        "        yield 2\n",
        "let r = Range3()\n",
        "mut s = 0\n",
        "for v in r:\n",
        "    s += v\n",
    );
    if let Value::Int(n) = run_get(src, "s") {
        assert_eq!(n, 3);
    } else {
        panic!("expected int 3");
    }
}

/// custom_iter_with_fields のテスト。
#[test]
fn test_custom_iter_with_fields() {
    // __iter__ can access self fields to yield instance data
    let src = concat!(
        "class Countdown:\n",
        "    mut start: int\n",
        "    gen __iter__(self) -> int:\n",
        "        mut i = self.start\n",
        "        while i > 0:\n",
        "            yield i\n",
        "            i -= 1\n",
        "let cd = Countdown(3)\n",
        "mut s = 0\n",
        "for v in cd:\n",
        "    s += v\n",
    );
    if let Value::Int(n) = run_get(src, "s") {
        assert_eq!(n, 6); // 3 + 2 + 1
    } else {
        panic!("expected int 6");
    }
}

/// str_iter_for_loop のテスト。
#[test]
fn test_str_iter_for_loop() {
    // for loop over a string iterates over characters
    let src = "mut n = 0\nfor c in \"abc\":\n    n += 1\n";
    if let Value::Int(n) = run_get(src, "n") {
        assert_eq!(n, 3);
    } else {
        panic!("expected int 3");
    }
}

/// for_break_with_iterator のテスト。
#[test]
fn test_for_break_with_iterator() {
    // break still works correctly inside an iterator-based for loop
    let src = "mut s = 0\nfor x in [1, 2, 3, 4, 5]:\n    if x == 3:\n        break\n    s += x\n";
    if let Value::Int(n) = run_get(src, "s") {
        assert_eq!(n, 3); // 1 + 2
    } else {
        panic!("expected int 3");
    }
}

