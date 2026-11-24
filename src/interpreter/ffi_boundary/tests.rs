use super::*;
use std::cell::RefCell;
use std::rc::Rc;

fn list(vals: Vec<Value>) -> Value {
    Value::List(Rc::new(RefCell::new(vals)))
}

fn is_ok(v: &Verdict) -> bool {
    matches!(v, Verdict::Ok)
}
fn is_mismatch(v: &Verdict) -> bool {
    matches!(v, Verdict::Mismatch { .. })
}

// ── Python: 共通判定をそのまま使う（int/float を区別して届く） ──

#[test]
fn python_accepts_matching_primitive() {
    let c = checker_for("py").unwrap();
    assert!(is_ok(&c.check(&Value::Int(1), &InferredType::Int)));
    assert!(is_ok(&c.check(&Value::Str("s".into()), &InferredType::Str)));
    assert!(is_ok(&c.check(&Value::None, &InferredType::None)));
}

#[test]
fn python_rejects_lying_stub() {
    // `def f() -> int: return "s"` — Python は注釈を強制しないので実際に起きる。
    let c = checker_for("py").unwrap();
    assert!(is_mismatch(&c.check(&Value::Str("s".into()), &InferredType::Int)));
    assert!(is_mismatch(&c.check(&Value::None, &InferredType::Int)));
    // float は int ではない（Python は両者を区別して届けるため厳密に見る）。
    assert!(is_mismatch(&c.check(&Value::Float(3.0), &InferredType::Int)));
}

#[test]
fn python_checks_list_elements() {
    let c = checker_for("py").unwrap();
    let declared = InferredType::ListOf(Box::new(InferredType::Int));
    assert!(is_ok(&c.check(&list(vec![Value::Int(1), Value::Int(2)]), &declared)));
    // 要素に別型が混ざる（`-> list[int]` が [1, "two"] を返す）。
    assert!(is_mismatch(&c.check(
        &list(vec![Value::Int(1), Value::Str("two".into())]),
        &declared
    )));
}

#[test]
fn unverifiable_declarations_pass_through() {
    let c = checker_for("py").unwrap();
    // 宣言が無いに等しい型は検査しない（スタブ未整備の箇所の挙動を変えない）。
    assert!(matches!(
        c.check(&Value::Str("s".into()), &InferredType::Any),
        Verdict::Unverifiable
    ));
    assert!(matches!(
        c.check(&Value::Str("s".into()), &InferredType::Unresolved),
        Verdict::Unverifiable
    ));
}

#[test]
fn union_accepts_any_variant() {
    let c = checker_for("py").unwrap();
    let opt_int = InferredType::Union(vec![InferredType::Int, InferredType::None]);
    assert!(is_ok(&c.check(&Value::Int(1), &opt_int)));
    assert!(is_ok(&c.check(&Value::None, &opt_int)));
    assert!(is_mismatch(&c.check(&Value::Str("s".into()), &opt_int)));
}

// ── JavaScript: 整数値は Int、それ以外は Float で届く（`bridge/js_bridge.cjs`）ので宣言型で寄せる ──

#[test]
fn javascript_coerces_integral_float_to_int() {
    // 2^63 以上の整数値は Float で届く。int64 に入るものだけ Int へ寄せる。
    let c = checker_for("js-proc").unwrap();
    match c.check(&Value::Float(3.0), &InferredType::Int) {
        Verdict::Coerce(Value::Int(3)) => {}
        other => panic!("expected Coerce(Int(3)), got {other:?}"),
    }
    // int64 に入らない整数値は OverflowError（7-9。以前は不一致・`javascript_integral_out_of_range_is_overflow`）
    assert!(matches!(c.check(&Value::Float(1e20), &InferredType::Int), Verdict::Overflow { .. }));
    assert!(is_mismatch(&c.check(&Value::Float(f64::NAN), &InferredType::Int)));
}

#[test]
fn javascript_rejects_fractional_float_for_int() {
    let c = checker_for("js-proc").unwrap();
    assert!(is_mismatch(&c.check(&Value::Float(3.5), &InferredType::Int)));
}

/// Python: `float` の宣言には int も入る（PEP 484・7-30）。float → int は寄せない
#[test]
fn python_coerces_int_to_float_only() {
    let c = checker_for("py").unwrap();
    assert!(matches!(c.check(&Value::Int(4), &InferredType::Float), Verdict::Coerce(Value::Float(f)) if f == 4.0));
    let opt = InferredType::Union(vec![InferredType::Float, InferredType::None]);
    assert!(matches!(c.check(&Value::Int(4), &opt), Verdict::Coerce(Value::Float(_))));
    assert!(matches!(c.check(&list(vec![Value::Int(1), Value::Float(2.5)]), &InferredType::ListOf(Box::new(InferredType::Float))), Verdict::Coerce(_)));
    assert!(is_mismatch(&c.check(&Value::Float(2.0), &InferredType::Int)), "float -> int is not coerced");
    assert!(is_mismatch(&c.check(&Value::Str("s".into()), &InferredType::Float)));
}

/// JS の `2.0` は整数値なので Int で届く。`float` 宣言では Float へ寄せる（2-3。以前は FfiTypeError）。
#[test]
fn javascript_coerces_int_to_float() {
    let c = checker_for("js-proc").unwrap();
    match c.check(&Value::Int(2), &InferredType::Float) {
        Verdict::Coerce(Value::Float(f)) if f == 2.0 => {}
        other => panic!("expected Coerce(Float(2.0)), got {other:?}"),
    }
    let opt = InferredType::Union(vec![InferredType::Float, InferredType::None]);
    assert!(matches!(c.check(&Value::Int(2), &opt), Verdict::Coerce(Value::Float(_))));
    assert!(is_ok(&c.check(&Value::None, &opt)));
}

#[test]
fn javascript_coerces_list_elements_recursively() {
    let c = checker_for("js-proc").unwrap();
    let floats = InferredType::ListOf(Box::new(InferredType::Float));
    match c.check(&list(vec![Value::Int(1), Value::Float(2.5)]), &floats) {
        Verdict::Coerce(Value::List(items)) => {
            let got: Vec<Value> = items.borrow().clone();
            assert!(matches!(got.as_slice(), [Value::Float(a), Value::Float(b)] if *a == 1.0 && *b == 2.5));
        }
        other => panic!("expected coerced float list, got {other:?}"),
    }
    let ints = InferredType::ListOf(Box::new(InferredType::Int));
    assert!(is_ok(&c.check(&list(vec![Value::Int(1), Value::Int(2)]), &ints)));
    // 要素に別の型が混ざる（`-> list[int]` が [1, "two"] を返す）
    assert!(is_mismatch(&c.check(&list(vec![Value::Int(1), Value::Str("two".into())]), &ints)));
    assert!(is_mismatch(&c.check(&list(vec![Value::Float(1.5)]), &ints)));
    let nested = InferredType::ListOf(Box::new(InferredType::ListOf(Box::new(InferredType::Float))));
    assert!(matches!(c.check(&list(vec![list(vec![Value::Int(1)])]), &nested), Verdict::Coerce(_)));
}

/// 寄せた値も照合し直す・dict の値と set の要素も寄せる・配列を宣言のタプルにする（5-2）。
#[test]
fn javascript_coerces_containers_and_rechecks() {
    let floats = InferredType::ListOf(Box::new(InferredType::Float));
    assert!(is_mismatch(&JavaScriptChecker.check(&list(vec![Value::Int(1), Value::str("two")]), &floats)));
    let d = InferredType::DictOf(Box::new(InferredType::Str), Box::new(InferredType::Float));
    let mut dict = crate::interpreter::value::DictData::new("Any".to_string(), "Any".to_string());
    crate::interpreter::Interpreter::dict_set_pure(&mut dict, Value::str("a"), Value::Int(2)).unwrap();
    let dv = Value::Dict(Rc::new(RefCell::new(dict)));
    match JavaScriptChecker.check(&dv, &d) {
        Verdict::Coerce(Value::Dict(x)) => assert!(matches!(x.borrow().all_pairs()[0].1, Value::Float(f) if f == 2.0)),
        other => panic!("{other:?}"),
    }
    let wrong = InferredType::DictOf(Box::new(InferredType::Str), Box::new(InferredType::Str));
    assert!(is_mismatch(&JavaScriptChecker.check(&dv, &wrong)), "dict values are checked");
    let t = InferredType::Tuple(vec![InferredType::Int, InferredType::Float]);
    match JavaScriptChecker.check(&list(vec![Value::Int(1), Value::Int(2)]), &t) {
        Verdict::Coerce(Value::Tuple(td)) => assert!(matches!(td.all_values()[1], Value::Float(f) if f == 2.0)),
        other => panic!("{other:?}"),
    }
    assert!(is_mismatch(&JavaScriptChecker.check(&list(vec![Value::Int(1)]), &t)), "length");
}

#[test]
fn javascript_falls_back_to_common_for_non_numeric() {
    let c = checker_for("js-proc").unwrap();
    assert!(is_ok(&c.check(&Value::Str("s".into()), &InferredType::Str)));
    assert!(is_mismatch(&c.check(&Value::Str("s".into()), &InferredType::Bool)));
}

// ── 言語の登録 ──

/// 整数値だが int64 に入らない JS の数は OverflowError（7-9）。小数・NaN は不一致のまま。コンテナ・`Option` の中も見る。
#[test]
fn javascript_integral_out_of_range_is_overflow() {
    let c = checker_for("js-proc").unwrap();
    let overflow = |v: &Verdict| matches!(v, Verdict::Overflow { value } if value == "100000000000000000000");
    assert!(overflow(&c.check(&Value::Float(1e20), &InferredType::Int)));
    assert!(overflow(&c.check(&list(vec![Value::Int(1), Value::Float(1e20)]), &InferredType::ListOf(Box::new(InferredType::Int)))));
    let opt_int = InferredType::Union(vec![InferredType::Int, InferredType::None]);
    assert!(overflow(&c.check(&Value::Float(1e20), &opt_int)));
    assert!(matches!(c.check(&Value::Float(-1e20), &InferredType::Int), Verdict::Overflow { .. }));
    // 2^63 ちょうども入らない（`i64::MAX as f64` は 2^63 に丸まる）
    assert!(matches!(c.check(&Value::Float(9223372036854775808.0), &InferredType::Int), Verdict::Overflow { .. }));
    // 本当の型の違いは不一致のまま
    assert!(is_mismatch(&c.check(&Value::Float(2.5), &InferredType::Int)));
    assert!(is_mismatch(&c.check(&Value::Float(f64::NAN), &InferredType::Int)));
    assert!(is_mismatch(&c.check(&Value::Float(1e20), &InferredType::Str)));
}

#[test]
fn only_dynamic_languages_are_checked() {
    assert!(checker_for("py").is_some());
    assert!(checker_for("js-proc").is_some());
    // 静的型付け側は向こうが型を守るので対象外。
    // C/C++ の `void*` は Arrow の int に落ちるため型タグでは何も言えない（別軸の課題）。
    assert!(checker_for("cpp-lib").is_none());
    assert!(checker_for("cs-dll").is_none());
    assert!(checker_for("rs").is_none());
}
