// tests/callables.rs — 関数型(function type)、クロージャ、デコレータのテスト。

use super::*;

// --- function type ---

/// function_type_call_positional のテスト。
#[test]
fn test_function_type_call_positional() {
    let src = concat!(
        "fn make() -> function[let int]->int:\n",
        "    fn inner(let x: int) -> int:\n",
        "        return x\n",
        "    return inner\n",
        "let f = make()\n",
        "let r = f(42)\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 42);
    } else {
        panic!("expected Int(42)");
    }
}

/// function_type_call_named_param のテスト。
#[test]
fn test_function_type_call_named_param() {
    let src = concat!(
        "fn make() -> function{let value:int}->int:\n",
        "    fn inner(let value: int) -> int:\n",
        "        return value\n",
        "    return inner\n",
        "let f = make()\n",
        "let r = f(value = 99)\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 99);
    } else {
        panic!("expected Int(99)");
    }
}

/// function_type_chained_call のテスト。
#[test]
fn test_function_type_chained_call() {
    let src = concat!(
        "fn make() -> function[let int]->int:\n",
        "    fn inner(let x: int) -> int:\n",
        "        return x\n",
        "    return inner\n",
        "mut r = make()(7)\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 7);
    } else {
        panic!("expected Int(7)");
    }
}

/// function_type_bare_any_call のテスト。
#[test]
fn test_function_type_bare_any_call() {
    // bare `function` type parameter should work with any call.
    let src = concat!(
        "fn apply(let f: function, let x: int) -> int:\n",
        "    return f(x)\n",
        "fn double(let n: int) -> int:\n",
        "    return n * 2\n",
        "let r = apply(double, 5)\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 10);
    } else {
        panic!("expected Int(10)");
    }
}

/// function_type_zero_params のテスト。
#[test]
fn test_function_type_zero_params() {
    let src = concat!(
        "fn make() -> function[]->int:\n",
        "    fn inner() -> int:\n",
        "        return 100\n",
        "    return inner\n",
        "let f = make()\n",
        "let r = f()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 100);
    } else {
        panic!("expected Int(100)");
    }
}

/// function_type_is_guard のテスト。
#[test]
fn test_function_type_is_guard() {
    // `f is function` should be True for a function value.
    let src = concat!(
        "fn add(let x: int) -> int:\n",
        "    return x + 1\n",
        "let r = add is function\n",
    );
    if let Value::Bool(b) = run_get(src, "r") {
        assert!(b);
    } else {
        panic!("expected Bool(true)");
    }
}

// --- closures ---

/// closure_captures_immutable のテスト。
#[test]
fn test_closure_captures_immutable() {
    // 不変変数のキャプチャ: 定義時の値が内側関数に保持される
    let src = concat!(
        "fn make(let n: int) -> function[]->int:\n",
        "    fn inner() -> int:\n",
        "        return n\n",
        "    return inner\n",
        "let f = make(42)\n",
        "let r = f()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 42);
    } else {
        panic!("expected Int(42)");
    }
}

/// closure_captures_mutable_shared のテスト。
#[test]
fn test_closure_captures_mutable_shared() {
    // 可変変数のキャプチャ: 内側関数が外側スコープの変数を変更できる
    let src = concat!(
        "fn make_counter() -> function[]->int:\n",
        "    mut count = 0\n",
        "    fn inc() -> int:\n",
        "        count += 1\n",
        "        return count\n",
        "    return inc\n",
        "let counter = make_counter()\n",
        "let r1 = counter()\n",
        "let r2 = counter()\n",
        "let r3 = counter()\n",
    );
    let interp = run_interp(src);
    assert!(matches!(interp.get_val("r1").unwrap(), Value::Int(1)));
    assert!(matches!(interp.get_val("r2").unwrap(), Value::Int(2)));
    assert!(matches!(interp.get_val("r3").unwrap(), Value::Int(3)));
}

/// closure_each_call_new_env のテスト。
#[test]
fn test_closure_each_call_new_env() {
    // 呼び出しごとに独立したクロージャ環境が生成される
    let src = concat!(
        "fn make(let start: int) -> function[]->int:\n",
        "    mut n = start\n",
        "    fn inc() -> int:\n",
        "        n += 1\n",
        "        return n\n",
        "    return inc\n",
        "let a = make(0)\n",
        "let b = make(100)\n",
        "let r_a = a()\n",
        "let r_b = b()\n",
    );
    let interp = run_interp(src);
    assert!(matches!(interp.get_val("r_a").unwrap(), Value::Int(1)));
    assert!(matches!(interp.get_val("r_b").unwrap(), Value::Int(101)));
}

/// closure_inner_called_from_outer のテスト。
#[test]
fn test_closure_inner_called_from_outer() {
    // 内側関数が外側関数の実行中に呼ばれ、変更が外側に反映される
    let src = concat!(
        "fn outer() -> int:\n",
        "    mut x = 0\n",
        "    fn inc() -> int:\n",
        "        x += 1\n",
        "        return x\n",
        "    inc()\n",
        "    inc()\n",
        "    return x\n",
        "let r = outer()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 2);
    } else {
        panic!("expected Int(2)");
    }
}

/// closure_static_shared_across_calls のテスト。
#[test]
fn test_closure_static_shared_across_calls() {
    // static mut 変数: 複数の呼び出しで同じセルを共有する
    let src = concat!(
        "fn make_counter() -> function[]->int:\n",
        "    static mut count = 0\n",
        "    fn inc() -> int:\n",
        "        count += 1\n",
        "        return count\n",
        "    return inc\n",
        // make_counter() を2回呼ぶ → 両方とも同じ count セルを共有する
        "let a = make_counter()\n",
        "let b = make_counter()\n",
        "let r1 = a()\n",
        "let r2 = b()\n",
        "let r3 = a()\n",
    );
    let interp = run_interp(src);
    assert!(matches!(interp.get_val("r1").unwrap(), Value::Int(1)));
    assert!(matches!(interp.get_val("r2").unwrap(), Value::Int(2)));
    assert!(matches!(interp.get_val("r3").unwrap(), Value::Int(3)));
}

/// #30 の不変条件: **多数のクロージャ実体が 1 つの Chunk を共有しても環境は独立**。
///
/// `get_or_compile_chunk` は定義サイト（`ChunkFnDef::compiled`）ごとに 1 回だけ
/// コンパイルし、実体間で `Rc<Chunk>` を共有する。共有してよい根拠は
/// 「キャプチャの slot 採番が `sort()` 済みで実体に依らない」＋「束縛は名前で引く」の 2 つ。
/// **どちらかが崩れると、後から作った実体が先の実体の値を見る**ので、
/// ここでは実体を**交互に**呼んで取り違えを検出する。
#[test]
fn test_closure_chunk_shared_across_instances_keeps_env_independent() {
    // 不変キャプチャ: 3 実体を作り、作った順と違う順で呼ぶ
    let src = concat!(
        "fn make_adder(let x: int) -> function[let int]->int:
",
        "    fn add(let y: int) -> int:
",
        "        return x + y
",
        "    return add
",
        "let a1 = make_adder(1)
",
        "let a2 = make_adder(20)
",
        "let a3 = make_adder(300)
",
        "let r3 = a3(7)
",
        "let r1 = a1(7)
",
        "let r2 = a2(7)
",
        // 実体を作り直しても以前の実体は影響を受けない
        "let a4 = make_adder(4000)
",
        "let r1b = a1(7)
",
        "let r4 = a4(7)
",
    );
    let interp = run_interp(src);
    assert!(matches!(interp.get_val("r1").unwrap(), Value::Int(8)));
    assert!(matches!(interp.get_val("r2").unwrap(), Value::Int(27)));
    assert!(matches!(interp.get_val("r3").unwrap(), Value::Int(307)));
    assert!(matches!(interp.get_val("r1b").unwrap(), Value::Int(8)));
    assert!(matches!(interp.get_val("r4").unwrap(), Value::Int(4007)));
}

/// #30 の不変条件（可変キャプチャ版）: セルは**実体ごとに独立**でなければならない。
///
/// 可変キャプチャは slot ではなく**セル表**（`captured_cells`）を通るので、
/// 共有する Chunk が持つのは「セル index」だけで、セル自体は `captured_env` から来る。
/// index が実体間でずれると**別の実体のカウンタを進める**。
#[test]
fn test_closure_chunk_shared_across_instances_keeps_cells_independent() {
    let src = concat!(
        "fn make_counter(let start: int) -> function[]->int:
",
        "    mut n = start
",
        "    fn inc() -> int:
",
        "        n += 1
",
        "        return n
",
        "    return inc
",
        "let c1 = make_counter(0)
",
        "let c2 = make_counter(100)
",
        "let c3 = make_counter(200)
",
        // 交互に呼ぶ（取り違えると値が飛ぶ）
        "let r1 = c1()
",
        "let r2 = c2()
",
        "let r3 = c1()
",
        "let r4 = c3()
",
        "let r5 = c2()
",
        "let r6 = c1()
",
    );
    let interp = run_interp(src);
    assert!(matches!(interp.get_val("r1").unwrap(), Value::Int(1)));
    assert!(matches!(interp.get_val("r2").unwrap(), Value::Int(101)));
    assert!(matches!(interp.get_val("r3").unwrap(), Value::Int(2)));
    assert!(matches!(interp.get_val("r4").unwrap(), Value::Int(201)));
    assert!(matches!(interp.get_val("r5").unwrap(), Value::Int(102)));
    assert!(matches!(interp.get_val("r6").unwrap(), Value::Int(3)));
}

/// #30: 不変キャプチャと可変キャプチャを**同時に**持つクロージャ。
///
/// 両方を持つと slot 採番（不変）とセル採番（可変）が同じ Chunk に同居する。
/// 片方だけ正しい実装でも通ってしまわないよう、混在形を独立に押さえる。
#[test]
fn test_closure_chunk_shared_mixed_captures() {
    let src = concat!(
        "fn make(let step: int, let base: int) -> function[]->int:
",
        "    mut acc = base
",
        "    fn bump() -> int:
",
        "        acc += step
",
        "        return acc
",
        "    return bump
",
        "let m1 = make(1, 0)
",
        "let m2 = make(10, 1000)
",
        "let r1 = m1()
",
        "let r2 = m2()
",
        "let r3 = m1()
",
        "let r4 = m2()
",
    );
    let interp = run_interp(src);
    assert!(matches!(interp.get_val("r1").unwrap(), Value::Int(1)));
    assert!(matches!(interp.get_val("r2").unwrap(), Value::Int(1010)));
    assert!(matches!(interp.get_val("r3").unwrap(), Value::Int(2)));
    assert!(matches!(interp.get_val("r4").unwrap(), Value::Int(1020)));
}

/// #45 の不変条件: **`deep_clone` は本体 AST の `Rc` を共有してはいけない**。
///
/// `FnValue.body` は `Rc<[Stmt]>`（クロージャ実体ごとの AST 複製を消すため）。
/// `Rc` の参照カウントは**非アトミック**なので、スレッドへ送る `deep_clone` が
/// `body.clone()`（＝参照カウント加算）で済ませると、複数スレッドが同じカウンタを
/// 叩いて壊れる（解放済みメモリの再利用 / 二重解放）— #15 と同じ形。
///
/// ⚠⚠ **この誤りはコンパイルエラーにならない**。`body` の型が `Vec<Stmt>` から
/// `Rc<[Stmt]>` に変わった瞬間、`body.clone()` の意味が「中身の複製」から
/// 「参照カウント加算」へ**黙って**変わる。⇒ 型ではなくテストで固定する。
///
/// ⚠ async の実地ストレス（[async_closure_share.ar](examples/async/async_closure_share.ar)）は
/// **この誤りを再現しない** — worker は捕捉したクロージャの内側 `Rc` を
/// タスク終了時に 1 回 drop するだけで、競合窓が狭すぎる。
/// **決定的に押さえるのはこのテストだけ**なので消さないこと。
#[test]
fn test_deep_clone_does_not_share_fn_body_rc() {
    use std::rc::Rc;
    let src = concat!(
        "fn make_adder(let x: int) -> function[let int]->int:
",
        "    fn add(let y: int) -> int:
",
        "        let unused = x + 1
",
        "        return x + y
",
        "    return add
",
        "let f = make_adder(10)
",
    );
    let interp = run_interp(src);
    let original = interp.get_val("f").expect("f must exist");
    let cloned = original.deep_clone();

    let (a, b) = match (&original, &cloned) {
        (Value::Function(a), Value::Function(b)) => (a, b),
        _ => panic!("expected Value::Function on both sides"),
    };
    // 本体は同じ内容でなければならない（複製の失敗＝空や欠損を弾く）
    assert_eq!(a.body.len(), b.body.len(), "deep_clone changed the body length");
    assert!(!a.body.is_empty(), "test is vacuous if the body is empty");
    // ⚠ 本体は**別のアロケーション**でなければならない（ここが本題）
    assert!(
        !std::ptr::eq(a.body.as_ptr(), b.body.as_ptr()),
        "deep_clone shared the body Rc across the copy (non-atomic refcount would race across threads)"
    );
    // 外側の `Rc<FnValue>` も当然別物
    assert!(!Rc::ptr_eq(a, b), "deep_clone returned the same FnValue");
}

/// #45 の不変条件（`OverloadedFn` 版）。`deep_clone` は**オーバーロードの全要素**で
/// 本体 `Rc` を複製しなければならない（`Value::Function` だけ直して満足しない）。
#[test]
fn test_deep_clone_does_not_share_overloaded_fn_body_rc() {
    let src = concat!(
        "fn dup(let a: int) -> int:
",
        "    return a * 2
",
        "fn dup(let a: str) -> str:
",
        "    return a + a
",
        "let g = dup
",
    );
    let interp = run_interp(src);
    let original = interp.get_val("g").expect("g must exist");
    let cloned = original.deep_clone();
    let (a, b) = match (&original, &cloned) {
        (Value::OverloadedFn(a), Value::OverloadedFn(b)) => (a, b),
        _ => panic!("expected Value::OverloadedFn on both sides (got {original:?})"),
    };
    assert_eq!(a.len(), b.len());
    assert!(a.len() >= 2, "test is vacuous without at least 2 overloads");
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert!(!x.body.is_empty());
        assert!(
            !std::ptr::eq(x.body.as_ptr(), y.body.as_ptr()),
            "overload {i} shared its body Rc across deep_clone"
        );
    }
}

/// #45 の不変条件（クラスのメソッド版）。インスタンスをスレッドへ送ると
/// `ClassValue::deep_clone` がメソッドの `FnValue` を作り直すので、そこでも共有しない。
#[test]
fn test_deep_clone_does_not_share_method_body_rc() {
    use crate::interpreter::Value as V;
    let src = concat!(
        "class Counter:
",
        "    mut n: int
",
        "    fn __init__(mut self, let n: int) -> None:
",
        "        self.n = n
",
        "    fn bump(mut self) -> int:
",
        "        self.n += 1
",
        "        return self.n
",
        "let c = Counter(1)
",
    );
    let interp = run_interp(src);
    let original = interp.get_val("c").expect("c must exist");
    let cloned = original.deep_clone();
    let (ca, cb) = match (&original, &cloned) {
        (V::Instance(a), V::Instance(b)) => (a.borrow().class.clone(), b.borrow().class.clone()),
        _ => panic!("expected Value::Instance on both sides"),
    };
    let mut checked = 0;
    for (name, overloads) in &ca.methods {
        let other = cb.methods.get(name).expect("method missing after deep_clone");
        for (x, y) in overloads.iter().zip(other.iter()) {
            if x.body.is_empty() {
                continue;
            }
            assert!(
                !std::ptr::eq(x.body.as_ptr(), y.body.as_ptr()),
                "method `{name}` shared its body Rc across deep_clone"
            );
            checked += 1;
        }
    }
    assert!(checked >= 2, "test is vacuous: only {checked} method bodies checked");
}

/// closure_freeze_captured_var_error のテスト。
#[test]
fn test_closure_freeze_captured_var_error() {
    // クロージャにキャプチャされた可変変数は freeze できない
    let src = concat!(
        "fn outer() -> None:\n",
        "    mut x = 0\n",
        "    fn inner() -> None:\n",
        "        x += 1\n",
        "    freeze x\n",
        "outer()\n",
    );
    assert!(run(src).is_err());
}

/// closure_nested のテスト。
#[test]
fn test_closure_nested() {
    // 二重ネストしたクロージャ
    let src = concat!(
        "fn outer(let a: int) -> function[]->function[]->int:\n",
        "    fn middle(let b: int) -> function[]->int:\n",
        "        fn inner() -> int:\n",
        "            return a + b\n",
        "        return inner\n",
        "    return middle\n",
        "let f = outer(10)(20)\n",
        "let r = f()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 30);
    } else {
        panic!("expected Int(30)");
    }
}

// --- Decorator ---

/// decorator_fn_basic のテスト。
#[test]
fn test_decorator_fn_basic() {
    // 関数デコレータ: @log で包まれた関数を呼ぶと wrapper が実行される
    let src = concat!(
        "fn log(let f: function) -> function:\n",
        "    fn wrapper() -> int:\n",
        "        return 99\n",
        "    return wrapper\n",
        "@log\n",
        "fn original() -> int:\n",
        "    return 1\n",
        "let r = original()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 99);
    } else {
        panic!("expected Int(99)");
    }
}

/// decorator_fn_passes_original のテスト。
#[test]
fn test_decorator_fn_passes_original() {
    // デコレータは元の関数を受け取ってラップできる
    let src = concat!(
        "fn identity(let f: function) -> function:\n",
        "    return f\n",
        "@identity\n",
        "fn add(let x: int) -> int:\n",
        "    return x + 10\n",
        "let r = add(5)\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 15);
    } else {
        panic!("expected Int(15)");
    }
}

/// decorator_stacked のテスト。
#[test]
fn test_decorator_stacked() {
    // スタックされたデコレータは下から順に適用される
    let src = concat!(
        "fn add1(let f: function) -> function:\n",
        "    fn wrapper() -> int:\n",
        "        return f() + 1\n",
        "    return wrapper\n",
        "@add1\n",
        "@add1\n",
        "fn base() -> int:\n",
        "    return 10\n",
        "let r = base()\n",
    );
    // add1 applied to base first → 11, then add1 again → 12
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 12);
    } else {
        panic!("expected Int(12)");
    }
}

/// decorator_class_as_decorator_for_fn のテスト。
#[test]
fn test_decorator_class_as_decorator_for_fn() {
    // クラスデコレータ（関数に適用）
    let src = concat!(
        "class Wrap:\n",
        "    mut inner: function\n",
        "    fn __init__(mut self, let f: function) -> None:\n",
        "        self.inner = f\n",
        "    fn __call__(self) -> function:\n",
        "        let fn_copy = self.inner\n",
        "        fn wrapper() -> int:\n",
        "            return fn_copy() + 100\n",
        "        return wrapper\n",
        "@Wrap\n",
        "fn base() -> int:\n",
        "    return 7\n",
        "let wrapped = base()\n",
        "let r = wrapped()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 107);
    } else {
        panic!("expected Int(107)");
    }
}

/// decorator_instance_callable のテスト。
#[test]
fn test_decorator_instance_callable() {
    // Value::Instance が __call__ を持つ場合に関数として呼び出せる
    let src = concat!(
        "class Adder:\n",
        "    mut n: int\n",
        "    fn __init__(mut self, let n: int) -> None:\n",
        "        self.n = n\n",
        "    fn __call__(self) -> int:\n",
        "        return self.n + 1\n",
        "let a = Adder(41)\n",
        "let r = a()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 42);
    } else {
        panic!("expected Int(42)");
    }
}

/// ar_to_py_dict のテスト。
#[test]
fn test_ar_to_py_dict() {
    // Value::Dict を Python に渡せることを確認する (sum_dict はすべての int 値を合計する)
    let src = concat!(
        "import[py-int] py_calculator as calc\n",
        "let d = {\"x\": 10, \"y\": 20, \"z\": 12}\n",
        "let r = calc.sum_dict(d)\n",
    );
    if let Value::Int(n) = run_py_get(src, "r") {
        assert_eq!(n, 42);
    } else {
        panic!("expected Int(42)");
    }
}

/// ar_to_py_tuple のテスト。
#[test]
fn test_ar_to_py_tuple() {
    // Value::Tuple を Python に渡せることを確認する (first_of_tuple は先頭要素を返す)
    let src = concat!(
        "import[py-int] py_calculator as calc\n",
        "let t = (99, 1, 2)\n",
        "let r = calc.first_of_tuple(t)\n",
    );
    if let Value::Int(n) = run_py_get(src, "r") {
        assert_eq!(n, 99);
    } else {
        panic!("expected Int(99)");
    }
}

