// tests/basics.rs — 算術・比較・論理演算、変数宣言(let/mut)、複合代入、print、ゼロ除算の基本テスト。

use super::*;

#[test]
fn test_arithmetic() {
    assert!(matches!(eval_expr("2 + 3"), Value::Int(5)));
    assert!(matches!(eval_expr("10 - 4"), Value::Int(6)));
    assert!(matches!(eval_expr("3 * 4"), Value::Int(12)));
    assert!(matches!(eval_expr("7 // 2"), Value::Int(3)));
    assert!(matches!(eval_expr("7 % 3"), Value::Int(1)));
    assert!(matches!(eval_expr("2 ** 10"), Value::Int(1024)));
}

/// float_arithmetic のテスト。
#[test]
fn test_float_arithmetic() {
    if let Value::Float(f) = eval_expr("1.0 + 2.0") {
        assert!((f - 3.0).abs() < f64::EPSILON);
    } else {
        panic!();
    }
}

/// string_concat のテスト。
#[test]
fn test_string_concat() {
    if let Value::Str(s) = eval_expr(r#""hello" + " " + "world""#) {
        assert_eq!(&*s, "hello world");
    } else {
        panic!();
    }
}

/// comparison のテスト。
#[test]
fn test_comparison() {
    assert!(matches!(eval_expr("1 < 2"), Value::Bool(true)));
    assert!(matches!(eval_expr("2 > 3"), Value::Bool(false)));
    assert!(matches!(eval_expr("4 == 4"), Value::Bool(true)));
    assert!(matches!(eval_expr("4 != 5"), Value::Bool(true)));
}

/// logical のテスト。
#[test]
fn test_logical() {
    assert!(matches!(eval_expr("True and False"), Value::Bool(false)));
    assert!(matches!(eval_expr("True or False"), Value::Bool(true)));
    assert!(matches!(eval_expr("not True"), Value::Bool(false)));
}

/// let_immutable のテスト。
#[test]
fn test_let_immutable() {
    assert!(run("let x = 1\nx = 2").is_err());
}

/// let_redeclaration_same_scope のテスト。
#[test]
fn test_let_redeclaration_same_scope() {
    let err = run("let a = 5\nlet a = 6\n").expect_err("redeclaration should error");
    assert!(err.contains("already declared"), "got: {err}");
}

/// mut_redeclaration_same_scope のテスト。
#[test]
fn test_mut_redeclaration_same_scope() {
    assert!(run("mut a = 5\nmut a = 6\n").is_err());
}

/// let_then_mut_redeclaration のテスト。
#[test]
fn test_let_then_mut_redeclaration() {
    assert!(run("let a = 5\nmut a = 6\n").is_err());
}

/// redeclaration_in_inner_scope のテスト（外側スコープの変数と同名）。
#[test]
///
/// ⚠ **静的検査で固定する**（#36）。本番（`run_program`）はこの形を型検査で弾くので
/// 実行時までは到達しない。VM の最上位 Chunk は内側 `let` を slot 宣言に落とすため
/// 実行時の重複検査（`exec_let`）も通らない。同じスコープの再宣言は実行時にも出るので
/// 上 2 つのテストは `run` のまま。
fn test_redeclaration_in_inner_scope() {
    let errs = static_errors("let x = 1\nif True:\n    let x = 2\n");
    assert!(
        errs.iter().any(|e| matches!(
            &e.kind,
            crate::type_check::TypeErrorKind::VariableRedeclaration { name } if name == "x"
        )),
        "expected a redeclaration error for 'x', got: {errs:?}"
    );
}

/// underscore_redeclaration_allowed のテスト（_ は再宣言を許可）。
#[test]
fn test_underscore_redeclaration_allowed() {
    assert!(run("let _ = 1\nlet _ = 2\n").is_ok());
}

/// redeclaration_error_message のテスト（エラーメッセージに変数名が含まれる）。
#[test]
fn test_redeclaration_error_message() {
    let err = run("let foo = 1\nlet foo = 2\n").expect_err("should error");
    assert!(err.contains("foo"), "error should mention variable name, got: {err}");
}

/// mut_mutable のテスト。
#[test]
fn test_mut_mutable() {
    assert!(run("mut x = 1\nx = 2").is_ok());
}

/// compound_assign のテスト。
#[test]
fn test_compound_assign() {
    if let Value::Int(n) = run_get("mut x = 10\nx += 5", "x") {
        assert_eq!(n, 15);
    } else {
        panic!();
    }
}

/// print_runs のテスト。
#[test]
fn test_print_runs() {
    assert!(run(r#"print("hello", "world")"#).is_ok());
}

/// zero_division のテスト。
#[test]
fn test_zero_division() {
    assert!(run("1 // 0").is_err());
}

// ---------------------------------------------------------------------------
// 組み込みの名前を値として使う（python_builtins_plan.md のタスク 1-7）
// ---------------------------------------------------------------------------

/// 束縛の無い組み込みの名前は値になり、呼べる。`list(it)` / `tuple(it)` はどのイテラブルも回す。
#[test]
fn test_builtin_names_are_values() {
    let src = concat!(
        "let f = repr\n",
        "let a = f(3)\n",
        "fn apply(g: function->str, x: int) -> str:\n",
        "    return g(x)\n",
        "let b = apply(repr, 5)\n",
        "let d: dict[str, int] = {\"k\": 1, \"j\": 2}\n",
        "let c = str(list(d))\n",
        "let t = str(tuple(range(3)))\n",
        "gen nums() -> int:\n",
        "    yield 1\n",
        "    yield 2\n",
        "let g = str(list(nums()))\n",
        "let xs: list[int] = [3, 1]\n",
        "mut ys = list(xs)\n",
        "ys.append(9)\n",
        "let n = len(xs)\n",
        "let shown = str(repr)\n",
    );
    assert_str(run_get(src, "a"), "3");
    assert_str(run_get(src, "b"), "5");
    assert_str(run_get(src, "c"), "['k', 'j']");
    assert_str(run_get(src, "t"), "(0, 1, 2)");
    assert_str(run_get(src, "g"), "[1, 2]");
    // `list(xs)` は写し（`ys.append` で `xs` は伸びない）。
    assert!(matches!(run_get(src, "n"), Value::Int(2)));
    assert_str(run_get(src, "shown"), "<built-in function repr>");
}

/// 束縛した名前が勝つ（組み込みの値は最後の段）。
#[test]
fn test_builtin_value_is_shadowed_by_a_binding() {
    let src = concat!(
        "fn repr(x: int) -> str:\n",
        "    return \"mine\"\n",
        "let f = repr\n",
        "let a = f(3)\n",
    );
    assert_str(run_get(src, "a"), "mine");
}

/// 値として使える名前の表（`BUILTIN_VALUE_NAMES`）は、型検査の名前の表にあり、組み込み関数の表が扱う。
/// ⚠ どちらかが欠けると「型検査は通るのに実行時に NameError」か「値なのに呼べない」になる。
#[test]
fn builtin_value_names_are_known_and_callable() {
    let mut interp = Interpreter::new();
    for name in crate::interpreter::eval::BUILTIN_VALUE_NAMES {
        assert!(crate::type_check::names::is_runtime_builtin_name(name), "{name} が型検査の表に無い");
        assert!(interp.eval_builtin_evaled(name, Vec::new()).is_some(), "{name} を組み込み関数の表が扱わない");
    }
}

// ---------------------------------------------------------------------------
// isinstance（python_builtins_plan.md のタスク 2-1）
// ---------------------------------------------------------------------------

/// CPython と同じ判定か（組は要素ごと・`bool` は `int` の派生・クラスは派生も・trait は `is` と同じ）。
#[test]
fn test_isinstance_follows_cpython() {
    let src = concat!(
        "trait Named:\n",
        "    fn name(self) -> str:\n",
        "        ...\n",
        "class Dog(Named):\n",
        "    fn name(self) -> str:\n",
        "        return \"d\"\n",
        "class Cat:\n",
        "    mut n: int\n",
        "let a = isinstance(True, int)\n",
        "let b = isinstance(1, float)\n",
        "let c = isinstance([1], (dict, list))\n",
        "let d = isinstance(Dog(), Dog)\n",
        "let e = isinstance(Cat(1), Dog)\n",
        "let f = isinstance(Dog(), Named)\n",
        "let k = Dog\n",
        "let g = isinstance(Dog(), k)\n",
        "let h = isinstance(None, int)\n",
    );
    for (var, want) in [("a", true), ("b", false), ("c", true), ("d", true), ("e", false), ("f", true), ("g", true), ("h", false)] {
        assert!(matches!(run_get(src, var), Value::Bool(b) if b == want), "{var}");
    }
    // 型でない第 2 引数は CPython と同じ `TypeError`。
    assert!(run_err_msg("let z = isinstance(3, len)\n").contains("isinstance() arg 2 must be a type"));
}

// ---------------------------------------------------------------------------
// type(x)（python_builtins_plan.md のタスク 2-2）
// ---------------------------------------------------------------------------

/// `type(x)` はクラスの値か組み込みの型の値。呼べ、`__name__` が引け、比べられる。
#[test]
fn test_type_of_follows_cpython() {
    let src = concat!(
        "class Box:\n",
        "    mut v: int\n",
        "let b = Box(1)\n",
        "let again = type(b)(5)\n",
        "let v = again.v\n",
        "let n = type(b).__name__\n",
        "let same = type(b) === Box\n",
        "let f = type(1.5) == float\n",
        "let not_int = type(True) === int\n",
        "let none = type(None).__name__\n",
        "let shown = str(type(3))\n",
    );
    assert!(matches!(run_get(src, "v"), Value::Int(5)));
    assert_str(run_get(src, "n"), "Box");
    assert!(matches!(run_get(src, "same"), Value::Bool(true)));
    assert!(matches!(run_get(src, "f"), Value::Bool(true)));
    assert!(matches!(run_get(src, "not_int"), Value::Bool(false)));
    assert_str(run_get(src, "none"), "NoneType");
    assert_str(run_get(src, "shown"), "<class 'int'>");
    assert!(run_err_msg("let d: dict[str, int] = {}\nlet z = type(\"M\", (), d)\n").contains("type() with 3 arguments"));
}

// ---------------------------------------------------------------------------
// getattr / hasattr / setattr（python_builtins_plan.md のタスク 2-3）
// ---------------------------------------------------------------------------

/// 名前での読み書きは `o.name` / `o.name = v` と同じ経路（既定値は `AttributeError` のときだけ）。
#[test]
fn test_getattr_hasattr_setattr() {
    let src = concat!(
        "class Box:\n",
        "    mut v: int\n",
        "mut b = Box(1)\n",
        "let a = getattr(b, \"v\")\n",
        "let d = getattr(b, \"w\", 0)\n",
        "let h1 = hasattr(b, \"v\")\n",
        "let h2 = hasattr(b, \"w\")\n",
        "setattr(b, \"v\", 9)\n",
        "let after = b.v\n",
    );
    assert!(matches!(run_get(src, "a"), Value::Int(1)));
    assert!(matches!(run_get(src, "d"), Value::Int(0)));
    assert!(matches!(run_get(src, "h1"), Value::Bool(true)));
    assert!(matches!(run_get(src, "h2"), Value::Bool(false)));
    assert!(matches!(run_get(src, "after"), Value::Int(9)));
    // 既定値が無ければ `AttributeError`、名前が文字列でなければ `TypeError`、`let` のインスタンスは書けない。
    assert!(run_err_msg("class Box:\n    mut v: int\nlet b = Box(1)\nlet z = getattr(b, \"w\")\n").contains("AttributeError"));
    assert!(run_err_msg("class Box:\n    mut v: int\nlet b = Box(1)\nlet z = getattr(b, 3)\n").contains("attribute name must be string"));
    assert!(run_err_msg("class Box:\n    mut v: int\nlet b = Box(1)\nsetattr(b, \"v\", 5)\n").contains("immutable"));
}

// ---------------------------------------------------------------------------
// issubclass / callable（python_builtins_plan.md のタスク 2-4）
// ---------------------------------------------------------------------------

/// `issubclass` は `isinstance` と同じ判定（クラスは派生も・`bool` ⊂ `int`・trait は実装）。`callable` は CPython と同じ。
#[test]
fn test_issubclass_and_callable() {
    let src = concat!(
        "trait Named:\n",
        "    fn name(self) -> str:\n",
        "        ...\n",
        "class Dog(Named):\n",
        "    fn name(self) -> str:\n",
        "        return \"d\"\n",
        "class Plain:\n",
        "    mut n: int\n",
        "let a = issubclass(Dog, Named)\n",
        "let b = issubclass(Plain, Named)\n",
        "let c = issubclass(bool, int)\n",
        "let d = issubclass(int, bool)\n",
        "let e = issubclass(Dog, (Plain, Dog))\n",
        "let f = callable(Plain)\n",
        "let g = callable(len)\n",
        "let h = callable(Plain(1))\n",
        "let i = callable(3)\n",
    );
    for (var, want) in [("a", true), ("b", false), ("c", true), ("d", false), ("e", true), ("f", true), ("g", true), ("h", false), ("i", false)] {
        assert!(matches!(run_get(src, var), Value::Bool(x) if x == want), "{var}");
    }
    assert!(run_err_msg("class Plain:\n    mut n: int\nlet z = issubclass(Plain(1), Plain)\n").contains("issubclass() arg 1 must be a class"));
}

// ---------------------------------------------------------------------------
// 集計の組み込み（python_builtins_plan.md のタスク 3-2）
// ---------------------------------------------------------------------------

/// CPython と同じ結果か（補償つきの sum・偶数への丸め・正確な 10 進での round・安定な並べ替え・逆元）。
#[test]
fn test_aggregates_follow_cpython() {
    let src = concat!(
        "let tenth: list[float] = [0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1]\n",
        "let s = sum(tenth)\n",
        "let r1 = round(2.5)\n",
        "let r2 = round(2.675, 2)\n",
        "let r3 = round(1250, -2)\n",
        "let pairs: list[tuple[str, int]] = [(\"b\", 2), (\"a\", 2), (\"c\", 1)]\n",
        "fn second(p: tuple[str, int]) -> int:\n",
        "    return p[1]\n",
        "let st = str(sorted(pairs, key=second, reverse=True))\n",
        "let mx = max([], default=7)\n",
        "let inv = pow(3, -1, 7)\n",
        "let dm = str(divmod(7, -2))\n",
        "let a = all([1, 0])\n",
        "let rv = str(list(reversed(range(3))))\n",
        "mut xs: list[int] = [3, 1, 2]\n",
        "xs.sort()\n",
        "let sorted_in_place = str(xs)\n",
    );
    assert!(matches!(run_get(src, "s"), Value::Float(f) if f == 1.0));
    assert!(matches!(run_get(src, "r1"), Value::Int(2)));
    assert!(matches!(run_get(src, "r2"), Value::Float(f) if f == 2.67));
    assert!(matches!(run_get(src, "r3"), Value::Int(1200)));
    assert_str(run_get(src, "st"), "[('b', 2), ('a', 2), ('c', 1)]");
    assert!(matches!(run_get(src, "mx"), Value::Int(7)));
    assert!(matches!(run_get(src, "inv"), Value::Int(5)));
    assert_str(run_get(src, "dm"), "(-4, -1)");
    assert!(matches!(run_get(src, "a"), Value::Bool(false)));
    assert_str(run_get(src, "rv"), "[2, 1, 0]");
    assert_str(run_get(src, "sorted_in_place"), "[1, 2, 3]");
    assert!(run_err_msg("let xs: list[int] = []\nlet z = max(xs)\n").contains("max() iterable argument is empty"));
    assert!(run_err_msg("let z = pow(2, -1, 4)\n").contains("base is not invertible"));
}

// ---------------------------------------------------------------------------
// 変換・遅延の組み込み（python_builtins_plan.md のタスク 3-3）
// ---------------------------------------------------------------------------

/// map / filter は遅延（無限の列にも使える）、iter(f, sentinel)、文字コード・基数の変換は CPython と同じ。
#[test]
fn test_conversions_follow_cpython() {
    let src = concat!(
        "fn sq(x: int) -> int:\n",
        "    return x * x\n",
        "gen naturals() -> int:\n",
        "    mut i = 0\n",
        "    while True:\n",
        "        yield i\n",
        "        i += 1\n",
        "mut m = map(sq, naturals())\n",
        "let a = m.next()\n",
        "let b = m.next()\n",
        "let c = m.next()\n",
        "let f = str(list(filter(None, [0, 1, 2, 0])))\n",
        "mut k = 0\n",
        "fn counter() -> int:\n",
        "    k += 1\n",
        "    return k\n",
        "let it = str(list(iter(counter, 3)))\n",
        "let h = hex(255) + oct(8) + bin(-5)\n",
        "let o = ord(\"a\")\n",
        "let ch = chr(233)\n",
        "let asc = ascii(\"h\u{e9}\")\n",
    );
    assert!(matches!(run_get(src, "c"), Value::Int(4)));
    assert_str(run_get(src, "f"), "[1, 2]");
    assert_str(run_get(src, "it"), "[1, 2]");
    assert_str(run_get(src, "h"), "0xff0o10-0b101");
    assert!(matches!(run_get(src, "o"), Value::Int(97)));
    assert_str(run_get(src, "ch"), "\u{e9}");
    assert_str(run_get(src, "asc"), "'h\\xe9'");
    assert!(run_err_msg("let z = chr(1114112)\n").contains("chr() arg not in range(0x110000)"));
    assert!(run_err_msg("let z = map(repr, 5)\n").contains("'int' object is not iterable"));
}

/// 遅延のジェネレータを zip / enumerate / set / list に渡すと最後まで回す（以前は黙って空・TypeError）。
#[test]
fn test_lazy_generators_feed_consumers() {
    let src = concat!(
        "gen two() -> int:\n",
        "    yield 1\n",
        "    yield 2\n",
        "let z = str(list(zip(two(), [10, 20])))\n",
        "let e = str(list(enumerate(two())))\n",
        "let s = len(set(two()))\n",
        "let l = str([*two()])\n",
    );
    assert_str(run_get(src, "z"), "[(1, 10), (2, 20)]");
    assert_str(run_get(src, "e"), "[(0, 1), (1, 2)]");
    assert!(matches!(run_get(src, "s"), Value::Int(2)));
    assert_str(run_get(src, "l"), "[1, 2]");
}

// ---------------------------------------------------------------------------
// print のキーワード引数（python_builtins_plan.md のタスク 5-1）
// ---------------------------------------------------------------------------

/// `print(.., sep=.., end=..)` を含む関数・最上位の文がバイトコードになる（以前は `VmForceError`）。
/// `sep` / `end` は `None` か文字列で、ほかは CPython と同じ文言の `TypeError`。
#[test]
fn test_print_keyword_arguments() {
    let src = concat!(
        "fn row(let xs: list[int]) -> int:\n",
        "    for x in xs:\n",
        "        print(x, end=\"\")\n",
        "    print(\"\", sep=None, end=None)\n",
        "    return len(xs)\n",
        "print(\"a\", \"b\", sep=\"\", end=\"\n\", flush=True)\n",
        "let n = row([1, 2])\n",
    );
    assert!(matches!(run_get(src, "n"), Value::Int(2)));
    assert!(run_err_msg("print(\"x\", end=5)\n").contains("end must be None or a string, not int"));
    assert!(run_err_msg("print(\"x\", color=1)\n").contains("'color' is an invalid keyword argument for print()"));
}

// ---------------------------------------------------------------------------
// open の Python の形（python_builtins_plan.md のタスク 5-2）
// ---------------------------------------------------------------------------

/// `open(path, "w", encoding="utf-8")` / `open(path)` / `open(path, "a")` / `"rb"` / `"x"` を受け、
/// mode・encoding の誤りは CPython と同じ文言で止める。
#[test]
fn test_open_python_form() {
    let p = std::env::temp_dir().join(format!("arrow_py_open_{}.txt", std::process::id()));
    let ps = p.to_string_lossy().replace('\\', "/");
    let _ = std::fs::remove_file(&p);
    let src = format!(
        concat!(
            "let w = open(\"{0}\", \"w\", encoding=\"utf-8\")\n",
            "w.write(\"ab\\n\")\n",
            "close(w)\n",
            "let a = open(\"{0}\", \"a\")\n",
            "a.write(\"cd\\n\")\n",
            "close(a)\n",
            "let r = open(\"{0}\")\n",
            "let text = r.read()\n",
            "close(r)\n",
            "let b = open(\"{0}\", mode=\"rb\")\n",
            "let n = len(b.read())\n",
            "close(b)\n",
        ),
        ps
    );
    assert_str(run_get(&src, "text"), "ab\ncd\n");
    assert!(matches!(run_get(&src, "n"), Value::Int(6)));
    let exists = run_err_msg(&format!("let x = open(\"{ps}\", \"x\")\n"));
    assert!(exists.starts_with("FileExistsError"), "{exists}");
    let _ = std::fs::remove_file(&p);
    assert!(run_err_msg("let f = open(\"z.txt\", \"rw\")\n")
        .contains("must have exactly one of create/read/write/append mode"));
    assert!(run_err_msg("let f = open(\"z.txt\", \"q\")\n").contains("invalid mode: 'q'"));
    assert!(run_err_msg("let f = open(\"z.txt\", \"rb\", encoding=\"utf-8\")\n")
        .contains("binary mode doesn't take an encoding argument"));
    assert!(run_err_msg("let f = open(\"z.txt\", \"r+\")\n").contains("NotImplementedError"));
}

/// 関数の中の `block:` 文から `return` で抜ける（python_builtins_plan.md のタスク 5-3）。
/// 以前は関数ごと `VmForceError` だった（`block_body_bails` がブロック文にも掛かっていた）。
#[test]
fn test_return_inside_block_statement() {
    let src = concat!(
        "fn first_even(let xs: list[int]) -> int:\n",
        "    block:\n",
        "        for x in xs:\n",
        "            if x % 2 == 0:\n",
        "                return x\n",
        "    return -1\n",
        "let a = first_even([1, 4, 5])\n",
        "let b = first_even([1, 3])\n",
    );
    assert!(matches!(run_get(src, "a"), Value::Int(4)));
    assert!(matches!(run_get(src, "b"), Value::Int(-1)));
}

// ---------------------------------------------------------------------------
// freeze は let と同じ結果になる（python_builtins_plan.md のタスク 1-9）
// ---------------------------------------------------------------------------

/// 関数の中の `freeze` が動き（以前は `NameError`）、仮引数を `freeze` しても呼び出し側のオブジェクトは
/// 可変のまま（`let` と同じ・以前は**その場で**不変にしていた）。関数の中の `let x = <式>` も複製する。
#[test]
fn test_freeze_is_the_same_as_let() {
    let src = concat!(
        "class C:\n",
        "    mut n: int\n",
        "    fn add(mut self, let k: int) -> int:\n",
        "        self.n = self.n + k\n",
        "        return self.n\n",
        "fn local_freeze() -> int:\n",
        "    mut c = C(1)\n",
        "    c.add(1)\n",
        "    freeze c\n",
        "    return c.n\n",
        "fn seal(mut c: C) -> None:\n",
        "    freeze c\n",
        "fn snapshot() -> int:\n",
        "    mut rows: list[list[int]] = [[1]]\n",
        "    let first = rows[0]\n",
        "    rows[0].append(9)\n",
        "    return len(first)\n",
        "let a = local_freeze()\n",
        "mut mine = C(1)\n",
        "seal(mine)\n",
        "let b = mine.add(10)\n",
        "let s = snapshot()\n",
    );
    assert!(matches!(run_get(src, "a"), Value::Int(2)));
    assert!(matches!(run_get(src, "b"), Value::Int(11)));
    assert!(matches!(run_get(src, "s"), Value::Int(1)));
    let captured = concat!(
        "fn make() -> int:\n",
        "    mut count = 0\n",
        "    fn inc() -> int:\n",
        "        count += 1\n",
        "        return count\n",
        "    inc()\n",
        "    freeze count\n",
        "    return count\n",
        "let r = make()\n",
    );
    assert!(run_err_msg(captured).contains("cannot freeze 'count' because it is captured by a closure"));
}
