// type_check_tests/protocols.rs — protocol の構造的な適合（pandas_import_plan.md の 6-5〜）。

use super::*;

const HOLDER: &str = concat!(
    "protocol Holder[T]:\n",
    "    fn get(self) -> T:\n",
    "        ...\n",
    "class IntBox:\n",
    "    fn get(self) -> int:\n",
    "        return 1\n",
    "class StrBox:\n",
    "    fn get(self) -> str:\n",
    "        return \"s\"\n",
);

/// 総称の protocol は型引数を置き換えた要求と照合する（6-5）。
#[test]
fn generic_protocol_conforms_with_type_args() {
    assert!(ok(&format!("{HOLDER}let a: Holder[int] = IntBox()\nlet b: Holder[str] = StrBox()\n")));
    let errs = check(&format!("{HOLDER}let a: Holder[int] = StrBox()\n"));
    assert!(
        errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::ProtocolConformanceFailed { protocol_name, .. }
            if protocol_name == "Holder[int]")),
        "{errs:?}"
    );
}

/// 値の型が総称の protocol なら、メンバーの型は型引数で読む（`Holder[int]` の `get()` は `int`）。
#[test]
fn generic_protocol_member_types_use_type_args() {
    assert!(ok(&format!("{HOLDER}fn f(h: Holder[int]) -> int:\n    return h.get()\n")));
    assert!(err(&format!("{HOLDER}fn f(h: Holder[int]) -> str:\n    return h.get()\n")));
}

/// メソッドの照合は部分型: 戻り値は狭くてよく、引数は広くてよい（6-5）。逆向きは満たさない。
#[test]
fn protocol_method_match_is_subtyping() {
    let narrow_return = concat!(
        "protocol P:\n",
        "    fn value(self) -> Option[int]:\n",
        "        ...\n",
        "class C:\n",
        "    fn value(self) -> int:\n",
        "        return 1\n",
        "let p: P = C()\n",
    );
    assert!(ok(narrow_return));
    let wide_return = concat!(
        "protocol P:\n",
        "    fn value(self) -> int:\n",
        "        ...\n",
        "class C:\n",
        "    fn value(self) -> Option[int]:\n",
        "        return 1\n",
        "let p: P = C()\n",
    );
    assert!(err(wide_return));
    let wide_param = concat!(
        "protocol P:\n",
        "    fn put(self, x: int) -> None:\n",
        "        ...\n",
        "class C:\n",
        "    fn put(self, x: Option[int]) -> None:\n",
        "        pass\n",
        "let p: P = C()\n",
    );
    assert!(ok(wide_param));
    let narrow_param = concat!(
        "protocol P:\n",
        "    fn put(self, x: Option[int]) -> None:\n",
        "        ...\n",
        "class C:\n",
        "    fn put(self, x: int) -> None:\n",
        "        pass\n",
        "let p: P = C()\n",
    );
    assert!(err(narrow_param));
}

/// 自分を参照する protocol は、照合中の組に戻ってきたら満たすと仮定して止まる（無限に再帰しない）。
#[test]
fn self_referential_protocol_terminates() {
    let src = concat!(
        "protocol Node[T]:\n",
        "    fn next(self) -> Option[Node[T]]:\n",
        "        ...\n",
        "class Link:\n",
        "    fn next(self) -> Option[Link]:\n",
        "        return None\n",
        "class Bad:\n",
        "    fn next(self) -> int:\n",
        "        return 1\n",
    );
    assert!(ok(&format!("{src}let n: Node[int] = Link()\n")));
    assert!(err(&format!("{src}let n: Node[int] = Bad()\n")));
}

/// 型引数の個数は宣言と同じでなければならない。素の名前も誤り（6-5）。
#[test]
fn protocol_type_arg_count_is_checked() {
    let bare = check(&format!("{HOLDER}fn f(h: Holder) -> None:\n    pass\n"));
    assert!(
        bare.iter().any(|e| matches!(&e.kind, TypeErrorKind::ProtocolTypeArgCount { expected: 1, got: 0, .. })),
        "{bare:?}"
    );
    let two = check(&format!("{HOLDER}let h: Holder[int, str] = IntBox()\n"));
    assert!(
        two.iter().any(|e| matches!(&e.kind, TypeErrorKind::ProtocolTypeArgCount { expected: 1, got: 2, .. })),
        "{two:?}"
    );
}

/// 型引数つきの protocol は型テストに書けない。素の名前は書ける（6-5）。
#[test]
fn generic_protocol_in_type_guard() {
    assert!(ok(&format!("{HOLDER}let b = IntBox()\nlet t = b is Holder\n")));
    let errs = check(&format!("{HOLDER}let b = IntBox()\nlet t = b is Holder[int]\n"));
    assert!(
        errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::GuardTypeNotTestable { .. })),
        "{errs:?}"
    );
}

/// protocol の型引数に制約は書けない（照合で使わないので、黙って捨てない）。
#[test]
fn protocol_type_param_constraint_is_a_parse_error() {
    let tokens = Lexer::new("trait Tr:\n    pass\nprotocol P[T: Tr]:\n    fn get(self) -> T:\n        ...\n", "").tokenize();
    let res = Parser::new(tokens, None).parse_program();
    assert!(res.is_err_and(|e| e.contains("cannot have a constraint")));
}

/// protocol の型の値のフィールドは、型引数で読んだ要求の型（6-5）。
#[test]
fn generic_protocol_field_types_use_type_args() {
    let src = concat!(
        "protocol HasN[T]:\n",
        "    let n: T\n",
    );
    assert!(ok(&format!("{src}fn f(h: HasN[int]) -> int:\n    return h.n\n")));
    assert!(err(&format!("{src}fn f(h: HasN[int]) -> str:\n    return h.n\n")));
}

/// 型引数の無い protocol でも、メソッドの戻り値の型は要求のシグネチャで決まる（6-5 より前は `Unresolved` だった）。
#[test]
fn protocol_method_return_type_is_checked() {
    let src = concat!(
        "protocol Pr:\n",
        "    fn get(self) -> int:\n",
        "        ...\n",
    );
    assert!(ok(&format!("{src}fn f(h: Pr) -> int:\n    return h.get()\n")));
    assert!(err(&format!("{src}fn f(h: Pr) -> str:\n    return h.get()\n")));
    assert!(err(&format!("{src}fn f(h: Pr) -> int:\n    return h.get(1)\n")));
}

const HAS_LEN: &str = concat!(
    "protocol HasLen:\n",
    "    fn __len__(self) -> int:\n",
    "        ...\n",
    "protocol Seq[T]:\n",
    "    fn __getitem__(self, i: int) -> T:\n",
    "        ...\n",
);

/// 組み込みの値も protocol を構造で満たす。要素の型も見る（6-6）。
#[test]
fn builtin_values_conform_structurally() {
    assert!(ok(&format!("{HAS_LEN}let a: HasLen = [1]\nlet b: HasLen = \"s\"\nlet c: HasLen = {{1: 2}}\nlet d: HasLen = (1, 2)\n")));
    assert!(ok(&format!("{HAS_LEN}let a: Seq[int] = [1]\nlet b: Seq[str] = \"s\"\nlet c: Seq[int] = (1, 2)\n")));
    assert!(err(&format!("{HAS_LEN}let a: HasLen = 5\n")));
    assert!(err(&format!("{HAS_LEN}let a: Seq[int] = {{1, 2}}\n")));
    assert!(err(&format!("{HAS_LEN}let a: Seq[int] = [\"x\"]\n")));
    // タプルの要素の型が混ざれば、要素の型はその和（`Seq[int]` ではない）
    assert!(err(&format!("{HAS_LEN}let a: Seq[int] = (1, \"x\")\n")));
}

/// 組み込みの値の表のメンバーは直接も呼べ、型が付く（`[1].__len__()` は int・6-6）。
#[test]
fn builtin_member_direct_calls_are_typed() {
    assert!(ok("let n: int = [1, 2].__len__()\nlet i: int = (5).__index__()\nlet c: int = [1, 2, 1].count(1)\n"));
    assert!(err("let n: str = [1, 2].__len__()\n"));
    assert!(err("let n = [1, 2].__len__(3)\n"));
    assert!(err("let f = (2.5).__index__()\n"));
}

/// 組み込みの protocol は import せずに書け、`for` の要素・添字の型は要求から決まる（6-7）。
#[test]
fn builtin_protocols_type_operations() {
    let sum = "fn f(xs: Iterable[int]) -> int:\n    mut s = 0\n    for x in xs:\n        s += x\n    return s\n";
    assert!(ok(&format!("{sum}let a = f([1, 2])\nlet b = f((1, 2))\nlet c = f({{1}})\n")));
    assert!(err(&format!("{sum}let a = f([\"x\"])\n")));
    assert!(err("fn f(xs: Iterable[int]) -> str:\n    for x in xs:\n        return x\n    return \"\"\n"));
    assert!(ok("fn f(xs: Sequence[str]) -> str:\n    return xs[0]\n"));
    assert!(err("fn f(xs: Sequence[str]) -> int:\n    return xs[0]\n"));
    assert!(ok("fn f(m: Mapping[str, int]) -> int:\n    return m[\"a\"]\nlet a = f({\"a\": 1})\n"));
    // 要求に無い操作は誤り（`Sized` は回せず、添字も取れない）
    assert!(err("fn f(s: Sized) -> None:\n    for x in s:\n        pass\n"));
    assert!(err("fn f(s: Sized) -> int:\n    return s[0]\n"));
}

/// 利用者が組み込みの protocol と同じ名前の型を宣言したら、そちらが勝つ（6-7）。
#[test]
fn user_declarations_shadow_builtin_protocols() {
    let class = concat!(
        "class Sized:\n",
        "    mut w: int\n",
        "    fn __init__(mut self, let w: int) -> None:\n",
        "        self.w = w\n",
        "let s: Sized = Sized(1)\n",
        "let n: int = s.w\n",
    );
    assert!(ok(class));
    assert!(ok("enum Sized:\n    a = 1\nlet x: Sized = Sized.a\n"));
    assert!(ok("trait Container[T]:\n    fn get(self) -> T:\n        ...\n"));
}
