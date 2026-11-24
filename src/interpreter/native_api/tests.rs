/// 生成コードが渡す演算の番号（`binop_code`）を、コールバックがすべて同じ演算に読み戻す（1-12・§0.3 ⑯）。
/// ⚠ 以前は `===`（20）が読み取りに無く、コンパイル済みコードの `===` が必ずエラーになっていた。
#[test]
fn every_binop_code_is_decoded_by_the_callbacks() {
    use crate::ast::BinOp;
    // ⚠ `BinOp` に種類を足すとこの `match` が通らなくなる。足した種類を下の `all` にも入れること。
    fn listed(op: &BinOp) {
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::FloorDiv | BinOp::Mod
            | BinOp::Pow | BinOp::Eq | BinOp::RefEq | BinOp::NotEq | BinOp::Lt | BinOp::LtEq
            | BinOp::Gt | BinOp::GtEq | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor
            | BinOp::LShift | BinOp::RShift | BinOp::In | BinOp::NotIn => {}
            // 短絡評価なのでコールバックへは渡さない（`gen_bool_logic`）
            BinOp::And | BinOp::Or => {}
        }
    }
    let all = [
        BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::FloorDiv, BinOp::Mod, BinOp::Pow,
        BinOp::Eq, BinOp::RefEq, BinOp::NotEq, BinOp::Lt, BinOp::LtEq, BinOp::Gt, BinOp::GtEq,
        BinOp::BitAnd, BinOp::BitOr, BinOp::BitXor, BinOp::LShift, BinOp::RShift, BinOp::In,
        BinOp::NotIn,
    ];
    for op in &all {
        listed(op);
        let code = crate::partial_compiler::llvm_codegen::binop_code(op);
        assert_eq!(
            super::i32_to_binop(code).as_ref(),
            Some(op),
            "binop code {code} of {op:?} is not decoded back by the callbacks"
        );
    }
}

/// 入れ子の呼び出し（コンパイル済みコード → コールバック → 利用者のコード → コンパイル済みの関数）は、抜けるときに
/// 入ったときのアリーナの長さまで巻き戻す（known_bugs.md #25）。結果は巻き戻す前に取り出し、外の呼び出しが
/// 積んだ値は残る。エラーで抜けても（`abort_native_call`）、入れ子の中の入れ子でも同じ。
#[test]
fn nested_native_calls_drop_what_they_pushed() {
    use super::*;
    let arena_len = || with_state(|s| s.borrow().arena.len());
    let is_str = |v: &Value, want: &str| matches!(v, Value::Str(s) if &**s == want);
    assert!(enter_native_call(std::ptr::null_mut()), "the test starts outside any native call");
    let kept = push_handle(Value::str("outer"));
    let before = arena_len();
    // 正常に抜ける入れ子: 引数と結果の箱を積んで抜ける
    assert!(!enter_native_call(std::ptr::null_mut()));
    push_handle(Value::str("arg"));
    let r = push_handle(Value::str("result"));
    assert!(is_str(&exit_native_call(r, false), "result"), "the result is taken before the arena is cut back");
    assert_eq!(arena_len(), before, "a nested call leaves nothing behind");
    // エラーで抜ける入れ子の中の入れ子
    assert!(!enter_native_call(std::ptr::null_mut()));
    push_handle(Value::str("a"));
    let mid = arena_len();
    assert!(!enter_native_call(std::ptr::null_mut()));
    push_handle(Value::str("b"));
    abort_native_call(false);
    assert_eq!(arena_len(), mid, "the inner nested call is cut back to its own entry");
    abort_native_call(false);
    assert_eq!(arena_len(), before, "an aborted nested call leaves nothing behind");
    assert!(is_str(&clone_value_at(kept), "outer"), "what the outer call pushed stays");
    exit_native_call(TL_NONE, true);
    assert!(!native_call_active());
}
