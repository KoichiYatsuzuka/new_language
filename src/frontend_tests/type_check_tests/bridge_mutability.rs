// type_check_tests/bridge_mutability.rs — 外部言語ブリッジの静的可変性検査
// （Param::bridge / CallMutParamWithImmutableArg）の統合テスト。
//
// C の書き込み用ポインタ（`T*` / `V3*`）パラメータは Arrow の `mut` パラメータとして
// 型検査される（.claude/skills/c-abi-interop/SKILL.md P5）。ヘッダは
// examples/test_modules/vec_math.h（cargo test はリポジトリルートで実行される前提）。

use super::*;
use std::path::PathBuf;

/// cpp import 用: source_dir を examples/ に向けて字句解析・構文解析・型検査する。
fn check_cpp(source: &str) -> Vec<StaticTypeError> {
    let tokens = Lexer::new(source, "").tokenize();
    let stmts = Parser::new(tokens, Some(PathBuf::from("examples")))
        .parse_program()
        .expect("parse error");
    TypeChecker::check(&stmts)
}

const IMPORT: &str = "import[cpp-lib] test_modules.vec_math as vm\n";

/// `let` 変数を書き込み用構造体ポインタ（`V3* out`）へ渡すと静的エラー。
#[test]
fn cpp_struct_ptr_let_rejected() {
    let src = format!(
        "{IMPORT}\
         let r = V3(0.0, 0.0, 0.0)\n\
         let a = V3(1.0, 2.0, 3.0)\n\
         let b = V3(4.0, 5.0, 6.0)\n\
         vm.v3_add(r, a, b)\n"
    );
    let errors = check_cpp(&src);
    assert!(
        errors.iter().any(|e| matches!(
            &e.kind,
            TypeErrorKind::CallMutParamWithImmutableArg { func_name, param_name }
                if func_name == "v3_add" && param_name == "out"
        )),
        "expected CallMutParamWithImmutableArg for 'out', got: {errors:?}"
    );
}

/// `mut` 変数（型未解決）を書き込み用構造体ポインタへ渡すのは正しい。
#[test]
fn cpp_struct_ptr_mut_var_ok() {
    let src = format!(
        "{IMPORT}\
         mut r = V3(0.0, 0.0, 0.0)\n\
         let a = V3(1.0, 2.0, 3.0)\n\
         let b = V3(4.0, 5.0, 6.0)\n\
         vm.v3_add(r, a, b)\n"
    );
    let errors = check_cpp(&src);
    assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
}

/// 呼び出し元関数の `mut` パラメータ（型解決済み `V3`）をそのまま
/// 書き込みポインタへ渡す — 可変参照の受け渡し。
/// ctype_to_tl_str が構造体ポインタを "int" と注釈していた頃は
/// `expects 'int' but got 'V3'` の偽エラーになっていた回帰テスト。
#[test]
fn cpp_struct_ptr_mut_param_passthrough_ok() {
    let src = format!(
        "{IMPORT}\
         fn add_into(mut out: V3, a: V3, b: V3) -> None:\n\
         \x20   vm.v3_add(out, a, b)\n\
         mut r = V3(0.0, 0.0, 0.0)\n\
         let a = V3(1.0, 2.0, 3.0)\n\
         let b = V3(4.0, 5.0, 6.0)\n\
         add_into(r, a, b)\n"
    );
    let errors = check_cpp(&src);
    assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
}

/// プリミティブポインタ（`double* out_len`）はポインティ型（float）で注釈される:
/// `mut` の float 変数は通る。
#[test]
fn cpp_prim_ptr_mut_float_ok() {
    let src = format!(
        "{IMPORT}\
         let v = V3(3.0, 4.0, 0.0)\n\
         mut n: float = 0.0\n\
         vm.v3_norm(v, n)\n"
    );
    let errors = check_cpp(&src);
    assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
}

/// `let` の float 変数を `double*` へ渡すと可変性エラー。
#[test]
fn cpp_prim_ptr_let_rejected() {
    let src = format!(
        "{IMPORT}\
         let v = V3(3.0, 4.0, 0.0)\n\
         let n: float = 0.0\n\
         vm.v3_norm(v, n)\n"
    );
    let errors = check_cpp(&src);
    assert!(
        errors.iter().any(|e| matches!(
            &e.kind,
            TypeErrorKind::CallMutParamWithImmutableArg { func_name, param_name }
                if func_name == "v3_norm" && param_name == "out_len"
        )),
        "expected CallMutParamWithImmutableArg for 'out_len', got: {errors:?}"
    );
}

/// `double*` へ int 変数を渡すと型不一致（ポインティ型 float で注釈されるため）。
#[test]
fn cpp_prim_ptr_int_arg_type_mismatch() {
    let src = format!(
        "{IMPORT}\
         let v = V3(3.0, 4.0, 0.0)\n\
         mut n: int = 0\n\
         vm.v3_norm(v, n)\n"
    );
    let errors = check_cpp(&src);
    assert!(
        errors.iter().any(|e| matches!(
            &e.kind,
            TypeErrorKind::CallArgTypeMismatch { func_name, .. } if func_name == "v3_norm"
        )),
        "expected CallArgTypeMismatch for v3_norm, got: {errors:?}"
    );
}
