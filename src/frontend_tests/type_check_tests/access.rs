// type_check_tests/access.rs — private / protected フィールドアクセスの静的型検査テスト。

use super::*;

    // --- Private / protected field access ---

    /// private_field_read_outside_err のテスト。
    #[test]
    fn private_field_read_outside_err() {
        assert!(err(concat!(
            "class MyClass:\n",
            "    private:\n",
            "    mut y: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.y = 0\n",
            "let obj = MyClass()\n",
            "print(obj.y)\n",
        )));
    }

    /// private_field_read_inside_ok のテスト。
    #[test]
    fn private_field_read_inside_ok() {
        assert!(ok(concat!(
            "class MyClass:\n",
            "    private:\n",
            "    mut y: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.y = 0\n",
            "    fn get_y(self) -> int:\n",
            "        return self.y\n",
        )));
    }

    /// private_field_write_outside_err のテスト。
    #[test]
    fn private_field_write_outside_err() {
        assert!(err(concat!(
            "class MyClass:\n",
            "    private:\n",
            "    mut y: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.y = 0\n",
            "let obj = MyClass()\n",
            "obj.y = 5\n",
        )));
    }

    /// public_field_read_outside_ok のテスト。
    #[test]
    fn public_field_read_outside_ok() {
        assert!(ok(concat!(
            "class MyClass:\n",
            "    public:\n",
            "    mut x: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.x = 1\n",
            "let obj = MyClass()\n",
            "print(obj.x)\n",
        )));
    }

    /// protected_field_read_same_class_ok のテスト。
    #[test]
    fn protected_field_read_same_class_ok() {
        assert!(ok(concat!(
            "trait T:\n",
            "    protected:\n",
            "    mut z: int\n",
            "class MyClass(T):\n",
            "    fn __init__(mut self, z: int) -> None:\n",
            "        self.z = z\n",
            "    fn get_z(self) -> int:\n",
            "        return self.z\n",
        )));
    }

    /// private_field_error_message のテスト。
    #[test]
    fn private_field_error_message() {
        let errors = check(concat!(
            "class A:\n",
            "    private:\n",
            "    mut secret: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.secret = 1\n",
            "let a = A()\n",
            "print(a.secret)\n",
        ));
        let msg = errors
            .iter()
            .find(|error| matches!(&error.kind, TypeErrorKind::PrivateAccessError { .. }))
            .unwrap()
            .to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("secret"));
        assert!(msg.contains("A"));
    }

