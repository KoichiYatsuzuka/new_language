// tests/mustbe.rs — mustbe 表明のテスト。

use super::*;
use crate::interpreter::*;

// ============================================================================
// mustbe テスト
// ============================================================================

/// mustbe: プリミティブ型の成功ケース
#[test]
fn test_mustbe_primitive_pass() {
    let val = run_get("let a = 42 mustbe int\n", "a");
    assert!(matches!(val, Value::Int(42)));
}

/// mustbe: プリミティブ型の失敗ケース (TypeError が raise される)
#[test]
fn test_mustbe_primitive_fail() {
    let exc = run_exc("let a = \"hello\" mustbe int\n").unwrap();
    assert!(exc.is_some());
    let err = exc.unwrap();
    if let Value::Instance(rc) = &err.exception {
            assert_eq!(rc.borrow().class.name, "TypeError");
        } else { panic!("expected TypeError instance"); }
}

/// mustbe: list の外側型チェック (成功)
#[test]
fn test_mustbe_list_pass() {
    let val = run_get("let a = [1, 2, 3] mustbe list\n", "a");
    assert!(matches!(val, Value::List(_)));
}

/// mustbe: list[int] の外側型チェック (list であれば合格・要素型は無視)
#[test]
fn test_mustbe_list_typed_pass() {
    let val = run_get("let a = [1, 2, 3] mustbe list[int]\n", "a");
    assert!(matches!(val, Value::List(_)));
}

/// mustbe: list の失敗ケース
#[test]
fn test_mustbe_list_fail() {
    let exc = run_exc("let a = 42 mustbe list\n").unwrap();
    assert!(exc.is_some());
    let err = exc.unwrap();
    if let Value::Instance(rc) = &err.exception {
        assert_eq!(rc.borrow().class.name, "TypeError");
    } else { panic!("expected TypeError instance"); }
}

/// mustbe: function の呼び出し可能チェック (成功)
#[test]
fn test_mustbe_function_pass() {
    let src = "fn double(let x: int) -> int:\n    return x * 2\nlet f = double mustbe function\n";
    let val = run_get(src, "f");
    assert!(matches!(val, Value::Function(_)));
}

/// mustbe: function の失敗ケース
#[test]
fn test_mustbe_function_fail() {
    let exc = run_exc("let a = 42 mustbe function\n").unwrap();
    assert!(exc.is_some());
    let err = exc.unwrap();
    if let Value::Instance(rc) = &err.exception {
        assert_eq!(rc.borrow().class.name, "TypeError");
    } else { panic!("expected TypeError instance"); }
}

/// mustbe: __call__ を持つクラスは function チェックに合格する
#[test]
fn test_mustbe_class_with_call_passes_function() {
    let src = r#"
class Callable:
    fn __call__(self) -> int:
        return 1
let c = Callable
let f = c mustbe function
"#;
    let val = run_get(src, "f");
    assert!(matches!(val, Value::Class(_)));
}

/// mustbe: __call__ を持たないクラスは function チェックに失敗する
#[test]
fn test_mustbe_class_without_call_fails_function() {
    let src = "class Plain:\n    let x: int\nlet c = Plain\nlet f = c mustbe function\n";
    let exc = run_exc(src).unwrap();
    assert!(exc.is_some());
    let err = exc.unwrap();
    if let Value::Instance(rc) = &err.exception {
        assert_eq!(rc.borrow().class.name, "TypeError");
    } else { panic!("expected TypeError instance"); }
}

/// mustbe: Any 型の値を int に絞り込む
#[test]
fn test_mustbe_any_to_int() {
    // Any として返す関数を模擬
    let src = r#"
fn get_val() -> int:
    return 99
let x: Any = get_val()
let a = x mustbe int
"#;
    let val = run_get(src, "a");
    assert!(matches!(val, Value::Int(99)));
}

/// mustbe: 添字アクセスでの型推論連携
#[test]
fn test_mustbe_list_subscript_inference() {
    // ランタイム: list[int] のアサーションと添字アクセス
    let src = "let xs = [10, 20, 30] mustbe list[int]\nlet b = xs[1]\n";
    let val = run_get(src, "b");
    assert!(matches!(val, Value::Int(20)));
}

/// mustbe: Protocol のフィールドチェック (成功)
#[test]
fn test_mustbe_protocol_pass() {
    let src = r#"
protocol Greetable:
    fn greet(self) -> str:
        ...

class Person:
    let name: str
    fn __init__(mut self, let n: str) -> None:
        self.name = n
    fn greet(self) -> str:
        return "Hello"

let p = Person("Alice")
let g = p mustbe Greetable
"#;
    let val = run_get(src, "g");
    assert!(matches!(val, Value::Instance(_)));
}

/// mustbe: Protocol のフィールドチェック (失敗: greet がない)
#[test]
fn test_mustbe_protocol_fail() {
    let src = r#"
protocol Greetable:
    fn greet(self) -> str:
        ...

class Robot:
    let id: int
    fn __init__(mut self, let i: int) -> None:
        self.id = i

let r = Robot(1)
let g = r mustbe Greetable
"#;
    let exc = run_exc(src).unwrap();
    assert!(exc.is_some());
    let err = exc.unwrap();
    if let Value::Instance(rc) = &err.exception {
        assert_eq!(rc.borrow().class.name, "TypeError");
    } else { panic!("expected TypeError instance"); }
}
