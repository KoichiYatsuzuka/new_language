// tests/expressions.rs — 式としての block/match/if/for/while、および break の入れ子制御フロー式伝播のテスト。

use super::*;

// ---------------------------------------------------------------------------
// match statement tests
// ---------------------------------------------------------------------------

/// match_case_literal のテスト。
#[test]
fn test_match_case_literal() {
    let src = "
mut x = 0
mut result = 0
match (x):
    case 0:
        result = 1
    case 1:
        result = 2
";
    assert!(matches!(run_get(src, "result"), Value::Int(1)));
}

/// match_case_no_match のテスト。
#[test]
fn test_match_case_no_match() {
    let src = "
mut x = 5
mut result = 0
match (x):
    case 0:
        result = 1
    case 1:
        result = 2
";
    assert!(matches!(run_get(src, "result"), Value::Int(0)));
}

/// match_case_wildcard のテスト。
#[test]
fn test_match_case_wildcard() {
    let src = "
mut x = 99
mut result = 0
match (x):
    case 0:
        result = 1
    case _:
        result = 99
";
    assert!(matches!(run_get(src, "result"), Value::Int(99)));
}

/// match_case_string のテスト。
#[test]
fn test_match_case_string() {
    let src = r#"
mut s = "hello"
mut result = 0
match (s):
    case "world":
        result = 1
    case "hello":
        result = 2
    case _:
        result = 3
"#;
    assert!(matches!(run_get(src, "result"), Value::Int(2)));
}

/// match_is_int のテスト。
#[test]
fn test_match_is_int() {
    let src = "
mut x = 42
mut result = 0
match (x):
    is int:
        result = 1
    is str:
        result = 2
";
    assert!(matches!(run_get(src, "result"), Value::Int(1)));
}

/// match_is_str のテスト。
#[test]
fn test_match_is_str() {
    let src = r#"
mut x = "hello"
mut result = 0
match (x):
    is int:
        result = 1
    is str:
        result = 2
"#;
    assert!(matches!(run_get(src, "result"), Value::Int(2)));
}

/// match_is_no_match のテスト。
#[test]
fn test_match_is_no_match() {
    let src = "
mut x = 3.14
mut result = 0
match (x):
    is int:
        result = 1
    is str:
        result = 2
";
    assert!(matches!(run_get(src, "result"), Value::Int(0)));
}

/// match_block_return のテスト。
#[test]
fn test_match_block_return() {
    // block_return inside a match arm exits the enclosing block: early
    let src = "
mut x = 2
mut result = 0
block:
    match (x):
        case 1:
            result = 10
            block_return 0
        case 2:
            result = 20
            block_return 0
    result = 999
";
    assert!(matches!(run_get(src, "result"), Value::Int(20)));
}

/// match_return_from_function のテスト。
#[test]
fn test_match_return_from_function() {
    let src = "
fn get(x: int) -> int:
    match (x):
        case 1:
            return 10
        case 2:
            return 20
        case _:
            return 99
    return 0
let result = get(2)
";
    assert!(matches!(run_get(src, "result"), Value::Int(20)));
}

/// match_mixed_arms_parse_error のテスト。
#[test]
fn test_match_mixed_arms_parse_error() {
    let src = "
mut x = 0
match (x):
    case 0:
        pass
    is int:
        pass
";
    let tokens = crate::lexer::Lexer::new(src, "").tokenize();
    let result = crate::parser::Parser::new(tokens, None).parse_program();
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("mix"), "expected mix error, got: {msg}");
}

// ---------------------------------------------------------------------------
// 制御フロー式テスト (if/for/while/match as expressions)
// ---------------------------------------------------------------------------


/// if_expr_true_branch のテスト。
#[test]
fn test_if_expr_true_branch() {
    let src = "
let x = if True ->str:
    block_return \"yes\"
else:
    block_return \"no\"
";
    assert_str(run_get(src, "x"), "yes");
}

/// if_expr_false_branch のテスト。
#[test]
fn test_if_expr_false_branch() {
    let src = "
let x = if False ->str:
    block_return \"yes\"
else:
    block_return \"no\"
";
    assert_str(run_get(src, "x"), "no");
}

/// if_expr_no_else_returns_none のテスト。
#[test]
fn test_if_expr_no_else_returns_none() {
    let src = "
let x = if False ->str:
    block_return \"yes\"
";
    assert!(matches!(run_get(src, "x"), Value::None));
}

/// if_expr_elif のテスト。
#[test]
fn test_if_expr_elif() {
    let src = "
let n = 7
let s = if n < 5 ->str:
    block_return \"small\"
elif n < 10:
    block_return \"medium\"
else:
    block_return \"large\"
";
    assert_str(run_get(src, "s"), "medium");
}

/// for_expr_block_yield のテスト。
#[test]
fn test_for_expr_block_yield() {
    let src = "
let evens = for i in range(5) ->list[int]:
    if i % 2 == 0:
        loop_yield i
";
    assert_int_list(run_get(src, "evens"), &[0, 2, 4]);
}

/// for_expr_block_return_single_value のテスト。
#[test]
fn test_for_expr_block_return_single_value() {
    let src = "
let first = for i in range(1, 10) ->int:
    if i % 2 == 0:
        block_return i
";
    assert_int(run_get(src, "first"), 2);
}

/// for_expr_no_yields_returns_none のテスト。
#[test]
fn test_for_expr_no_yields_returns_none() {
    let src = "
let x = for i in range(0) ->list[int]:
    loop_yield i
";
    assert!(matches!(run_get(src, "x"), Value::None));
}

/// for_expr_break_returns_partial_list のテスト。
#[test]
fn test_for_expr_break_returns_partial_list() {
    let src = "
let partial = for i in range(10) ->list[int]:
    if i == 3:
        break
    loop_yield i
";
    assert_int_list(run_get(src, "partial"), &[0, 1, 2]);
}

/// while_expr_block_yield のテスト。
#[test]
fn test_while_expr_block_yield() {
    let src = "
mut n = 0
let vals = while n < 3 ->list[int]:
    loop_yield n
    n += 1
";
    assert_int_list(run_get(src, "vals"), &[0, 1, 2]);
}

/// while_expr_block_return のテスト。
#[test]
fn test_while_expr_block_return() {
    let src = "
mut n = 0
let found = while n < 100 ->int:
    n += 1
    if n * n > 50:
        block_return n
";
    assert_int(run_get(src, "found"), 8);
}

/// match_expr_block_return のテスト。
#[test]
fn test_match_expr_block_return() {
    let src = "
let v = 2
let s = match (v) ->str:
    case 1:
        block_return \"one\"
    case 2:
        block_return \"two\"
    case _:
        block_return \"other\"
";
    assert_str(run_get(src, "s"), "two");
}

/// match_expr_no_match_returns_none のテスト。
#[test]
fn test_match_expr_no_match_returns_none() {
    let src = "
let v = 99
let s = match (v) ->str:
    case 1:
        block_return \"one\"
";
    assert!(matches!(run_get(src, "s"), Value::None));
}

/// break_exits_regular_for_loop のテスト。
#[test]
fn test_break_exits_regular_for_loop() {
    let src = "
mut found = -1
for i in range(10):
    if i == 5:
        found = i
        break
";
    assert_int(run_get(src, "found"), 5);
}

// --- break propagation through nested control-flow expressions ---

/// break_inside_if_expr_exits_for_loop のテスト。
#[test]
fn test_break_inside_if_expr_exits_for_loop() {
    // break inside an if expression body should exit the enclosing for loop
    let src = "
mut found = -1
for i in range(10):
    let _ = if i == 4 ->int:
        found = i
        break
    else:
        0
";
    assert_int(run_get(src, "found"), 4);
    // loop must have stopped: next iteration would set found to 5+
    let src2 = "
mut count = 0
for i in range(10):
    let _ = if i == 3 ->int:
        break
    else:
        0
    count += 1
";
    assert_int(run_get(src2, "count"), 3); // iterations 0, 1, 2 complete
}

/// break_inside_if_expr_exits_while_loop のテスト。
#[test]
fn test_break_inside_if_expr_exits_while_loop() {
    let src = "
mut i = 0
mut stopped_at = -1
while i < 20:
    let _ = if i == 7 ->int:
        stopped_at = i
        break
    else:
        0
    i += 1
";
    assert_int(run_get(src, "stopped_at"), 7);
}

/// break_inside_block_expr_exits_loop のテスト。
#[test]
fn test_break_inside_block_expr_exits_loop() {
    // break inside a block: expression should exit the enclosing loop
    let src = "
mut found = -1
for i in range(10):
    let _ = block ->int:
        if i == 5:
            found = i
            break
        block_return i
";
    assert_int(run_get(src, "found"), 5);
}

/// for_expr_break_inside_if_expr_returns_yields のテスト。
#[test]
fn test_for_expr_break_inside_if_expr_returns_yields() {
    // break inside an if expression in a for expression should return accumulated yields
    let src = "
let result = for i in range(10) ->list[int]:
    let _ = if i == 3 ->int:
        break
    else:
        0
    loop_yield i
";
    assert_int_list(run_get(src, "result"), &[0, 1, 2]);
}

/// while_expr_break_inside_if_expr_returns_yields のテスト。
#[test]
fn test_while_expr_break_inside_if_expr_returns_yields() {
    let src = "
mut n = 0
let result = while True ->list[int]:
    let _ = if n == 4 ->int:
        break
    else:
        0
    loop_yield n
    n += 1
";
    assert_int_list(run_get(src, "result"), &[0, 1, 2, 3]);
}

// ---------------------------------------------------------------------------
// #34: break / continue が制御フロー「式」を貫通して外側ループへ届く
//
// ⚠ ここは長らく**例題が 1 本も無く**、`force_gate` にも `compare_vm_modes` にも
//    映らなかった領域。基準は参照実装（`python -m impl_python`）の出力。
//    例題は examples/basics/control_flow_expr_escape{,_error}.ar。
// ---------------------------------------------------------------------------

/// `continue` が block: 式を貫通して外側 for へ届く（#34）。
/// **以前は `SyntaxError` になっていた**（ツリーウォークのバグ）。
#[test]
fn test_continue_through_block_expr_reaches_loop() {
    let src = "
mut s = 0
for i in range(6):
    let v = 100 + block ->int:
        if i < 3:
            continue
        block_return i
    s = s + v
";
    assert_int(run_get(src, "s"), 312); // 103 + 104 + 105
}

/// `continue` が if 式を貫通して外側 for へ届く（#34）。
/// **以前は黙って握り潰されて `None` が返り** `int + None` の TypeError になっていた。
#[test]
fn test_continue_through_if_expr_reaches_loop() {
    let src = "
mut t = 0
for i in range(6):
    let v = if i < 3 ->int:
        continue
    else:
        block_return i
    t = t + v
";
    assert_int(run_get(src, "t"), 12); // 3 + 4 + 5
}

/// `continue` が while 文でも貫通する（#34）。
#[test]
fn test_continue_through_block_expr_in_while() {
    let src = "
mut s = 0
mut i = 0
while i < 6:
    i += 1
    let v = 100 + block ->int:
        if i < 3:
            continue
        block_return i
    s = s + v
";
    assert_int(run_get(src, "s"), 418); // 103 + 104 + 105 + 106
}

/// 跳ぶ時点でオペランドが積まれている形（`1 + 2 * block …`）でも正しく抜ける（#34）。
/// VM はここで積んだ値を捨ててからジャンプする（`stmt_base` 分の `Pop`）。
#[test]
fn test_break_with_pending_operands() {
    let src = "
mut r = 0
for i in range(5):
    r = 1 + 2 * block ->int:
        if i == 3:
            break
        block_return i
";
    assert_int(run_get(src, "r"), 5); // i=2 の 1 + 2*2
}

/// 入れ子のブロック式を 2 段貫通する（#34）。
#[test]
fn test_break_through_nested_block_exprs() {
    let src = "
mut deep = -1
for i in range(5):
    let a = 1 + block ->int:
        let b = 2 + block ->int:
            if i == 2:
                break
            block_return i
        block_return b
    deep = a
";
    assert_int(run_get(src, "deep"), 4); // i=1 の 1 + (2 + 1)
}

/// `break` は最内ループだけを抜ける（外側へ漏れない）（#34）。
#[test]
fn test_break_through_block_expr_reaches_innermost_loop_only() {
    let src = "
mut n = 0
for i in range(3):
    for j in range(5):
        let _ = block ->int:
            if j == 2:
                break
            block_return j
        n += 1
";
    assert_int(run_get(src, "n"), 6); // 外ループ 3 周 × 内ループ 2 回
}

/// `block:` **文**の中の `break` も外側ループへ届く（#34）。
#[test]
fn test_break_inside_block_stmt_exits_loop() {
    let src = "
mut bs = -1
for i in range(5):
    block:
        if i == 3:
            break
        bs = i
";
    assert_int(run_get(src, "bs"), 2);
}

/// 属性代入の右辺にあるブロック式から `break`（#34）。
///
/// ⚠ VM 側の回帰検知が本命。右辺の評価中は**レシーバが 1 つ積まれている**ので、
/// `Stmt::AttrAssign` が深さ `stmt_base + 1` を伝えないと `--vm=on` だけ
/// `VmForceError` になる（実測で見つけた伝播漏れ）。
#[test]
fn test_break_in_attr_assign_rhs() {
    let src = "
class Box:
    mut v: int

    fn __init__(mut self) -> None:
        self.v = 0

mut bx = Box()
for i in range(5):
    bx.v = 1 + block ->int:
        if i == 3:
            break
        block_return i
let got = bx.v
";
    assert_int(run_get(src, "got"), 3);
}

/// `try` 本体のブロック式から `break` で抜けても、**例外ハンドラが残らない**（#34）。
///
/// ⚠ VM 側の回帰検知が本命（ツリーウォークにハンドラスタックは無い）。
/// `emit_unwind_to_loop` の `PopTry` を消すと、ループを抜けた後の `raise` を
/// ループ内の `except` が横取りする（実際に踏んだ）。例題は
/// examples/basics/control_flow_expr_escape.ar のケース 12。
#[test]
fn test_break_out_of_try_does_not_leave_handler() {
    let src = "
mut fired = 0
mut caught = 0
fn scan() -> int:
    for i in range(5):
        try:
            let _ = block ->int:
                if i == 2:
                    break
                block_return i
        except ValueError:
            fired += 1
    raise ValueError(\"must escape\")
    return 0
try:
    let _ = scan()
except ValueError:
    caught = 1
";
    assert_int(run_get(src, "fired"), 0);
    assert_int(run_get(src, "caught"), 1);
}

/// `try` 本体からの `continue` も外側ループへ届く（#34）。
#[test]
fn test_continue_out_of_try_reaches_loop() {
    let src = "
mut s = 0
for i in range(6):
    try:
        if i < 3:
            continue
        s = s + i
    except ValueError:
        s = -1
";
    assert_int(run_get(src, "s"), 12); // 3 + 4 + 5
}

/// 囲むループが無ければ `continue` は実行時エラー（関数境界を越えない）（#34）。
#[test]
fn test_continue_outside_loop_in_block_expr_is_error() {
    let src = "
fn bad() -> int:
    let v = 1 + block ->int:
        continue
    return v
let x = bad()
";
    let err = run(src).unwrap_err();
    assert!(err.contains("'continue' outside for/while loop"), "got: {err}");
}

/// break_does_not_cross_function_boundary のテスト。
#[test]
fn test_break_does_not_cross_function_boundary() {
    // break inside a function that has no loop should be an error
    let src = "
fn bad():
    let _ = if True ->int:
        break
    else:
        0
bad()
";
    assert!(run(src).is_err());
}

/// break_inside_function_loop_does_not_exit_outer_loop のテスト。
#[test]
fn test_break_inside_function_loop_does_not_exit_outer_loop() {
    // break inside an inner function's loop must not affect the outer loop
    let src = "
mut outer_count = 0
fn inner() -> int:
    for i in range(5):
        if i == 2:
            break
    return 42
for _ in range(4):
    inner()
    outer_count += 1
";
    assert_int(run_get(src, "outer_count"), 4);
}

/// continue_in_while_loop のテスト。
#[test]
fn test_continue_in_while_loop() {
    let src = "
mut evens = 0
mut i = 0
while i < 10:
    i += 1
    if i % 2 != 0:
        continue
    evens += i
";
    assert_int(run_get(src, "evens"), 30); // 2+4+6+8+10
}

/// continue_in_for_loop のテスト。
#[test]
fn test_continue_in_for_loop() {
    let src = "
mut s = 0
for n in range(1, 11):
    if n % 3 == 0:
        continue
    s += n
";
    assert_int(run_get(src, "s"), 37); // 1+2+4+5+7+8+10
}

/// continue_skips_rest_of_body のテスト。
#[test]
fn test_continue_skips_rest_of_body() {
    // continue skips the remaining statements in the body
    let src = "
mut touched = 0
for i in range(5):
    continue
    touched += 1
";
    assert_int(run_get(src, "touched"), 0);
}

/// continue_in_nested_loop のテスト。
#[test]
fn test_continue_in_nested_loop() {
    // continue only skips the innermost loop iteration
    let src = "
mut s = 0
for i in range(1, 4):
    for j in range(1, 4):
        if j == 2:
            continue
        s += j
";
    // j=1 and j=3 contribute per outer iteration: (1+3)*3 = 12
    assert_int(run_get(src, "s"), 12);
}

/// continue_outside_loop_is_error のテスト。
#[test]
fn test_continue_outside_loop_is_error() {
    let src = "
fn bad():
    continue
bad()
";
    assert!(run(src).is_err());
}

/// continue_outside_loop_toplevel_is_error のテスト。
#[test]
fn test_continue_outside_loop_toplevel_is_error() {
    assert!(run("continue").is_err());
}

/// block_return_propagates_through_nested_if_to_for_expr のテスト。
#[test]
fn test_block_return_propagates_through_nested_if_to_for_expr() {
    let src = "
let result = for i in range(10) ->int:
    if i > 4:
        block_return i
";
    assert_int(run_get(src, "result"), 5);
}

/// block_expr_with_return_type_annotation のテスト。
#[test]
fn test_block_expr_with_return_type_annotation() {
    let src = "
let x = block ->int:
    block_return 42
";
    assert_int(run_get(src, "x"), 42);
}

/// if_expr_without_annotation_still_works のテスト。
#[test]
fn test_if_expr_without_annotation_still_works() {
    let src = "
let x = if True:
    block_return 100
else:
    block_return 0
";
    assert_int(run_get(src, "x"), 100);
}

/// block_return_type_check_ok のテスト。
#[test]
fn test_block_return_type_check_ok() {
    let src = "let x = block ->int:\n    block_return 42\n";
    assert_int(run_get(src, "x"), 42);
}

/// block_return_type_check_error のテスト。
#[test]
fn test_block_return_type_check_error() {
    let src = "let x = block ->int:\n    block_return \"hello\"\n";
    let err = run(src).unwrap_err();
    assert!(err.contains("TypeError"), "expected TypeError, got: {err}");
    assert!(err.contains("'int'"), "expected annotation in error: {err}");
}

/// if_expr_block_return_type_check_ok のテスト。
#[test]
fn test_if_expr_block_return_type_check_ok() {
    let src = "let x = if True ->str:\n    block_return \"ok\"\nelse:\n    block_return \"no\"\n";
    assert_str(run_get(src, "x"), "ok");
}

/// if_expr_block_return_type_check_error のテスト。
#[test]
fn test_if_expr_block_return_type_check_error() {
    let src = "let x = if True ->str:\n    block_return 42\n";
    let err = run(src).unwrap_err();
    assert!(err.contains("TypeError"), "expected TypeError, got: {err}");
}

/// for_expr_block_return_type_check_ok のテスト。
#[test]
fn test_for_expr_block_return_type_check_ok() {
    let src = "let x = for i in range(5) ->int:\n    if i == 3:\n        block_return i\n";
    assert_int(run_get(src, "x"), 3);
}

/// for_expr_block_return_type_check_error のテスト。
#[test]
fn test_for_expr_block_return_type_check_error() {
    let src = "let x = for i in range(5) ->int:\n    if i == 3:\n        block_return \"three\"\n";
    let err = run(src).unwrap_err();
    assert!(err.contains("TypeError"), "expected TypeError, got: {err}");
}

/// while_expr_block_return_type_check_ok のテスト。
#[test]
fn test_while_expr_block_return_type_check_ok() {
    let src = concat!(
        "mut n = 0\n",
        "let x = while n < 10 ->int:\n",
        "    n += 1\n",
        "    if n == 5:\n",
        "        block_return n\n",
    );
    assert_int(run_get(src, "x"), 5);
}

/// while_expr_block_return_type_check_error のテスト。
#[test]
fn test_while_expr_block_return_type_check_error() {
    let src = concat!(
        "mut n = 0\n",
        "let x = while n < 10 ->int:\n",
        "    n += 1\n",
        "    if n == 5:\n",
        "        block_return \"five\"\n",
    );
    let err = run(src).unwrap_err();
    assert!(err.contains("TypeError"), "expected TypeError, got: {err}");
}

/// match_expr_block_return_type_check_ok のテスト。
#[test]
fn test_match_expr_block_return_type_check_ok() {
    let src = concat!(
        "let x = match (1) ->str:\n",
        "    case 1:\n",
        "        block_return \"one\"\n",
        "    case _:\n",
        "        block_return \"other\"\n",
    );
    assert_str(run_get(src, "x"), "one");
}

/// match_expr_block_return_type_check_error のテスト。
#[test]
fn test_match_expr_block_return_type_check_error() {
    let src = concat!(
        "let x = match (1) ->str:\n",
        "    case 1:\n",
        "        block_return 1\n",
        "    case _:\n",
        "        block_return 0\n",
    );
    let err = run(src).unwrap_err();
    assert!(err.contains("TypeError"), "expected TypeError, got: {err}");
}

/// block_return_option_type_check_ok のテスト。
#[test]
fn test_block_return_option_type_check_ok() {
    let src = "let x = block ->Option[int]:\n    block_return None\n";
    assert!(matches!(run_get(src, "x"), Value::None));
}

/// block_return_no_annotation_no_check のテスト。
#[test]
fn test_block_return_no_annotation_no_check() {
    let src = "let x = block:\n    block_return \"anything\"\n";
    assert_str(run_get(src, "x"), "anything");
}

