// type_check_tests/ptr_types.rs — `Ptr[T]` の型検査（ffi_value_exchange_plan.md 3-3・D-3）。
//
// ⚠ Arrow から `Ptr` を作る手段は無い（外部の関数だけが返す）ので、どのテストも仮引数で受け取る。

use super::*;

fn kinds(source: &str) -> Vec<TypeErrorKind> {
    check(source).into_iter().map(|e| e.kind).collect()
}

fn has_ptr_method_error(source: &str, method: &str) -> bool {
    kinds(source)
        .iter()
        .any(|k| matches!(k, TypeErrorKind::PtrMethodUnavailable { method: m, .. } if m == method))
}

#[test]
fn to_list_has_the_arrow_element_type() {
    assert!(ok("fn f(let p: Ptr[int32]) -> list[int]:\n    return p.to_list()\n"));
    assert!(ok("fn f(let p: Ptr[float32]) -> list[float]:\n    return p.to_list()\n"));
    // 要素型が付くので取り違えは静的エラー
    assert!(err("fn f(let p: Ptr[int32]) -> list[str]:\n    return p.to_list()\n"));
}

#[test]
fn to_bytes_only_for_character_data() {
    assert!(ok("fn f(let p: Ptr[uint8]) -> bytes:\n    return p.to_bytes()\n"));
    assert!(ok("fn f(let p: Ptr[uint16]) -> bytes:\n    return p.to_bytes()\n"));
    assert!(has_ptr_method_error("fn f(let p: Ptr[int32]) -> None:\n    let b = p.to_bytes()\n", "to_bytes"));
}

#[test]
fn void_pointer_cannot_be_expanded_until_converted() {
    assert!(has_ptr_method_error("fn f(let p: Ptr[void]) -> None:\n    let xs = p.to_list()\n", "to_list"));
    assert!(ok("fn f(let p: Ptr[void]) -> list[int]:\n    let q = p mustbe Ptr[int32]\n    return q.to_list()\n"));
}

#[test]
fn address_is_never_exposed() {
    assert!(has_ptr_method_error("fn f(let p: Ptr[int32]) -> None:\n    let a = p.address()\n", "address"));
    assert!(has_ptr_method_error("fn f(let p: Ptr[int32]) -> None:\n    p.release(1)\n", "release"));
    assert!(ok("fn f(let p: Ptr[int32]) -> None:\n    p.release()\n"));
}

#[test]
fn any_ptr_goes_to_void_but_not_back() {
    let take_void = "fn take(let p: Ptr[void]) -> None:\n    return\n";
    assert!(ok(&format!("{take_void}fn f(let p: Ptr[int32]) -> None:\n    take(p)\n")));
    let take_int = "fn take(let p: Ptr[int32]) -> None:\n    return\n";
    assert!(err(&format!("{take_int}fn f(let p: Ptr[void]) -> None:\n    take(p)\n")));
    // 幅が違えば別の型
    assert!(err(&format!("{take_int}fn f(let p: Ptr[int64]) -> None:\n    take(p)\n")));
}

#[test]
fn distinct_struct_pointers_are_distinct_types() {
    let src = "class Db:\n    mut n: int\nclass File:\n    mut n: int\n\
               fn take(let p: Ptr[Db]) -> None:\n    return\n\
               fn f(let p: Ptr[File]) -> None:\n    take(p)\n";
    assert!(err(src));
    let ok_src = "class Db:\n    mut n: int\nfn take(let p: Ptr[Db]) -> None:\n    return\n\
                  fn f(let p: Ptr[Db]) -> None:\n    take(p)\n";
    assert!(ok(ok_src));
}

#[test]
fn mustbe_between_known_element_types_can_never_succeed() {
    let ks = kinds("fn f(let p: Ptr[int32]) -> None:\n    let q = p mustbe Ptr[float64]\n");
    assert!(ks.iter().any(|k| matches!(k, TypeErrorKind::MustBeCanNeverSucceed { .. })), "{ks:?}");
}

#[test]
fn element_must_be_a_c_type() {
    let ks = kinds("fn f(let p: Ptr[int]) -> None:\n    return\n");
    assert!(ks.iter().any(|k| matches!(k, TypeErrorKind::PtrElemNotCType { .. })), "{ks:?}");
    assert!(err("fn f(let p: Ptr[str]) -> None:\n    return\n"));
    assert!(err("fn f(let p: Ptr[NoSuchStruct]) -> None:\n    return\n"));
}

#[test]
fn bare_ptr_annotation_is_rejected_but_type_test_is_fine() {
    let ks = kinds("fn f(let p: Ptr) -> None:\n    return\n");
    assert!(ks.iter().any(|k| matches!(k, TypeErrorKind::BareContainerAnnotation { .. })), "{ks:?}");
    assert!(ok("fn f(let x: Any) -> bool:\n    return x is Ptr\n"));
}

#[test]
fn let_ptr_can_be_passed_to_a_mut_parameter() {
    // 書き込み用ポインタ（`T*`）の引数は `mut` 扱い。`Ptr` は中身を書き換えられないので `let` のまま渡せる
    let src = "fn take(mut p: Ptr[int32]) -> None:\n    return\nfn f(let p: Ptr[int32]) -> None:\n    take(p)\n";
    assert!(ok(src), "{:?}", kinds(src));
}
