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


    // --- Private / protected methods（フェーズ10 10-6）---
    // ⚠ 以前はメソッドだけアクセス指定を見ておらず、`private:` の下のメソッドをクラスの外から呼べた。

    /// 呼び出しの誤りの種類だけを取り出す（メソッドの検査は `PrivateAccessError` で出る）。
    fn private_errors(source: &str) -> usize {
        check(source)
            .iter()
            .filter(|e| matches!(&e.kind, TypeErrorKind::PrivateAccessError { .. }))
            .count()
    }

    const ACCOUNT: &str = concat!(
        "class Account:\n",
        "    mut balance: int\n",
        "    fn deposit(mut self, let n: int) -> None:\n",
        "        self.audit(n)\n",
        "        self.balance += n\n",
        "    static fn open() -> Account:\n",
        "        return Account(Account.start())\n",
        "    private:\n",
        "    fn audit(self, let n: int) -> None:\n",
        "        print(n)\n",
        "    static fn start() -> int:\n",
        "        return 0\n",
    );

    /// private メソッドをクラスの外から呼ぶと誤り。
    #[test]
    fn private_method_call_outside_err() {
        let src = format!("{ACCOUNT}mut a = Account(0)\na.audit(5)\n");
        assert_eq!(private_errors(&src), 1);
    }

    /// クラスの中（メソッド・静的メソッド）から呼ぶのは通る。
    #[test]
    fn private_method_call_inside_ok() {
        let src = format!("{ACCOUNT}mut a = Account.open()\na.deposit(5)\n");
        assert!(ok(&src));
    }

    /// private メソッドを値として読むのも誤り（呼び出しと同じ規則）。
    #[test]
    fn private_method_read_outside_err() {
        let src = format!("{ACCOUNT}let a = Account(0)\nlet f = a.audit\n");
        assert_eq!(private_errors(&src), 1);
    }

    /// private の静的メソッドをクラス経由で外から呼ぶのも誤り。
    #[test]
    fn private_static_method_call_outside_err() {
        let src = format!("{ACCOUNT}print(Account.start())\n");
        assert_eq!(private_errors(&src), 1);
    }

    /// 別のクラスのメソッドから呼ぶのも誤り（「外」はクラスの外全部）。
    #[test]
    fn private_method_call_from_other_class_err() {
        let src = format!(
            "{ACCOUNT}class Auditor:\n    mut n: int\n    fn check(self, let a: Account) -> None:\n        a.audit(1)\n"
        );
        assert_eq!(private_errors(&src), 1);
    }

    /// メソッドの中の入れ子の関数はクラスの中（字句の規則）。
    #[test]
    fn private_method_call_in_nested_fn_ok() {
        assert!(ok(concat!(
            "class C:\n",
            "    mut n: int\n",
            "    fn f(self) -> int:\n",
            "        fn g() -> int:\n",
            "            return self.h()\n",
            "        return g()\n",
            "    private:\n",
            "    fn h(self) -> int:\n",
            "        return 10\n",
            "let c = C(0)\n",
            "print(c.f())\n",
        )));
    }

    /// private の gen メソッドも同じ。
    #[test]
    fn private_gen_method_call_outside_err() {
        assert_eq!(
            private_errors(concat!(
                "class C:\n",
                "    mut n: int\n",
                "    private:\n",
                "    gen items(self) -> int:\n",
                "        yield self.n\n",
                "let c = C(1)\n",
                "for x in c.items():\n",
                "    print(x)\n",
            )),
            1
        );
    }

    // --- 組み込みの値の属性（フェーズ10 10-14）---
    // ⚠ 以前は何も見ておらず、`x.name`（`x: int`）が実行時の AttributeError まで通っていた。

    fn no_member_errors(src: &str) -> usize {
        check(src)
            .iter()
            .filter(|e| matches!(&e.kind, TypeErrorKind::NoSuchMember { .. }))
            .count()
    }

    /// `int` の属性の読み（10-14 の再現）。
    #[test]
    fn builtin_int_attr_read_err() {
        assert_eq!(no_member_errors("fn label(let x: int) -> str:\n    return x.name\n"), 1);
    }

    /// 組み込みの値は読める属性を持たない（`str` / `list` / `dict` も）。
    #[test]
    fn builtin_attr_read_err() {
        assert_eq!(
            no_member_errors(concat!(
                "let s = \"abc\"\n",
                "let a = s.length\n",
                "let xs: list[int] = [1]\n",
                "let b = xs.size\n",
                "let d: dict[str, int] = {\"k\": 1}\n",
                "let c = d.count\n",
            )),
            3
        );
    }

    /// `int` / `float` / `bool` / `None` はメソッドを持たない。`complex` は `real` / `imag` / `angle` だけ。
    #[test]
    fn builtin_scalar_method_err() {
        assert_eq!(
            no_member_errors(concat!(
                "let f: float = 1.5\n",
                "let a = f.hex()\n",
                "let z: complex = 1 + 2j\n",
                "let b = z.conjugate()\n",
            )),
            2
        );
    }

    /// `str` / 容器のメソッド呼び出しと `complex` のメソッドは通る（呼び先は読みではない）。
    #[test]
    fn builtin_method_calls_ok() {
        assert!(ok(concat!(
            "let s = \"abc\"\n",
            "print(s.upper())\n",
            "mut xs: list[int] = [1]\n",
            "xs.append(2)\n",
            "let z: complex = 1 + 2j\n",
            "print(z.real())\n",
            "print(\"a,b\".split(\",\"))\n",
        )));
    }
