// type_check_tests/annotations.rs — AST 型解決層（タスク #16・段階(a)）のパイプライン検証。
// パース→node-id 採番→型検査で注釈充填→取り出し、が end-to-end で動くことを確認する。

use crate::ast::{Expr, Stmt};
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::type_check::{Directive, InferredType, TypeChecker};

/// `let` の値式が `MustBe` のとき、その node_id を返す（テスト用の限定的ウォーカ）。
fn find_mustbe_node_id(stmts: &[Stmt]) -> Option<u32> {
    for s in stmts {
        if let Stmt::Let(_, _, Expr::MustBe { node_id, .. }) = s {
            return Some(*node_id);
        }
    }
    None
}

#[test]
fn mustbe_node_id_is_assigned_by_parser() {
    let src = "let x: Any = 5\nlet y = x mustbe int\n";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let node_id = find_mustbe_node_id(&stmts).expect("MustBe node not found");
    // パーサが per-module で 1 始まり採番する。0（未採番）でないこと。
    assert_ne!(node_id, 0, "parser must assign a non-zero node_id to MustBe");
}

#[test]
fn mustbe_annotation_recorded_resolved_and_directive() {
    let src = "let x: Any = 5\nlet y = x mustbe int\n";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let node_id = find_mustbe_node_id(&stmts).expect("MustBe node not found");

    let (_errors, ann) = TypeChecker::check_and_annotate(&stmts);

    // ① 解決型テーブル: mustbe は確定後の型（int）を焼く。
    let tid = ann
        .resolved(node_id)
        .expect("resolved type must be recorded for the MustBe node");
    assert_eq!(
        ann.type_of(tid),
        Some(&InferredType::Int),
        "MustBe should resolve to int"
    );

    // 型インターン表に少なくとも int が登録されている。
    assert!(ann.intern_len() >= 1, "type intern table should contain int");

    // ② 検査指示テーブル: mustbe は実行時に対象型で動的検査 → CheckBefore(int)。
    match ann.directive(node_id) {
        Directive::CheckBefore(t) => {
            assert_eq!(
                ann.type_of(t),
                Some(&InferredType::Int),
                "CheckBefore should carry the int type id"
            );
        }
        other => panic!("expected CheckBefore directive, got {other:?}"),
    }
}

/// `let` の値式が `BinOp` のとき、その node_id を返す。
fn find_binop_node_id(stmts: &[Stmt]) -> Option<u32> {
    for s in stmts {
        if let Stmt::Let(_, _, Expr::BinOp { node_id, .. }) = s {
            return Some(*node_id);
        }
    }
    None
}

#[test]
fn binop_result_type_recorded() {
    // int + int → 結果型 int を解決型テーブルへ焼く。
    let src = "let a = 1\nlet b = 2\nlet c = a + b\n";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let node_id = find_binop_node_id(&stmts).expect("BinOp node not found");
    assert_ne!(node_id, 0);
    let (errors, ann) = TypeChecker::check_and_annotate(&stmts);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let tid = ann.resolved(node_id).expect("binop result type recorded");
    assert_eq!(ann.type_of(tid), Some(&InferredType::Int));
    // 二項演算に検査指示は付かない。
    assert_eq!(ann.directive(node_id), Directive::None);
}

#[test]
fn binop_float_result_type_recorded() {
    let src = "let a = 1.0\nlet b = 2\nlet c = a + b\n";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let node_id = find_binop_node_id(&stmts).expect("BinOp node not found");
    let (_errors, ann) = TypeChecker::check_and_annotate(&stmts);
    let tid = ann.resolved(node_id).expect("binop result type recorded");
    // int + float → float（昇格）。
    assert_eq!(ann.type_of(tid), Some(&InferredType::Float));
}

/// `let`/`return` の値式が `Attr` の node_id を（関数本体へ再帰して）返す。
fn find_attr_node_id(stmts: &[Stmt]) -> Option<u32> {
    for s in stmts {
        match s {
            Stmt::Let(_, _, Expr::Attr { node_id, .. })
            | Stmt::Return(Some(Expr::Attr { node_id, .. })) => return Some(*node_id),
            Stmt::FnDef { body, .. } => {
                if let Some(id) = find_attr_node_id(body) {
                    return Some(id);
                }
            }
            _ => {}
        }
    }
    None
}

#[test]
fn attr_field_type_recorded() {
    // p.x（int フィールド・p は NamedInstance("P")）→ 結果型 int を焼く。
    let src = concat!(
        "class P:\n",
        "    public:\n",
        "    mut x: int\n",
        "    fn __init__(mut self) -> None:\n",
        "        self.x = 5\n",
        "fn read(p: P) -> int:\n",
        "    let y = p.x\n",
        "    return y\n",
    );
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let node_id = find_attr_node_id(&stmts).expect("Attr node not found");
    assert_ne!(node_id, 0);
    let (errors, ann) = TypeChecker::check_and_annotate(&stmts);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let tid = ann.resolved(node_id).expect("attr field type recorded");
    assert_eq!(ann.type_of(tid), Some(&InferredType::Int));
}

#[test]
fn subscript_element_type_recorded() {
    let src = "let lst = [10, 20, 30]\nlet y = lst[0]\n";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let node_id = match &stmts[1] {
        Stmt::Let(_, _, Expr::Subscript { node_id, .. }) => *node_id,
        other => panic!("expected Subscript, got {other:?}"),
    };
    let (_errors, ann) = TypeChecker::check_and_annotate(&stmts);
    let tid = ann.resolved(node_id).expect("subscript element type recorded");
    assert_eq!(ann.type_of(tid), Some(&InferredType::Int));
}

#[test]
fn istype_records_bool() {
    let src = "let x: Any = 5\nlet y = x is int\n";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let node_id = match &stmts[1] {
        Stmt::Let(_, _, Expr::IsType { node_id, .. }) => *node_id,
        other => panic!("expected IsType, got {other:?}"),
    };
    let (_errors, ann) = TypeChecker::check_and_annotate(&stmts);
    let tid = ann.resolved(node_id).expect("istype bool recorded");
    assert_eq!(ann.type_of(tid), Some(&InferredType::Bool));
    // `is` 自体は検査なので CheckBefore は付かない。
    assert_eq!(ann.directive(node_id), Directive::None);
}

#[test]
fn cast_records_target_type_and_check_directive() {
    let src = "let x: Any = 5\nlet y = x => int\n";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let node_id = match &stmts[1] {
        Stmt::Let(_, _, Expr::Cast { node_id, .. }) => *node_id,
        other => panic!("expected Cast, got {other:?}"),
    };
    let (_errors, ann) = TypeChecker::check_and_annotate(&stmts);
    let tid = ann.resolved(node_id).expect("cast target type recorded");
    assert_eq!(ann.type_of(tid), Some(&InferredType::Int));
    match ann.directive(node_id) {
        Directive::CheckBefore(t) => assert_eq!(ann.type_of(t), Some(&InferredType::Int)),
        other => panic!("expected CheckBefore, got {other:?}"),
    }
}

#[test]
fn call_result_type_recorded() {
    let src = "fn f(x: int) -> int:\n    return x\nlet y = f(5)\n";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    // stmts[1] = let y = f(5)
    let node_id = match &stmts[1] {
        Stmt::Let(_, _, Expr::Call { node_id, .. }) => *node_id,
        other => panic!("expected Call, got {other:?}"),
    };
    assert_ne!(node_id, 0);
    let (errors, ann) = TypeChecker::check_and_annotate(&stmts);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let tid = ann.resolved(node_id).expect("call result type recorded");
    assert_eq!(ann.type_of(tid), Some(&InferredType::Int));
}

#[test]
fn call_info_records_callee_and_arg_types() {
    let src = "fn f(a: int, b: int) -> int:\n    return a + b\nlet y = f(3, 4)\n";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let node_id = match &stmts[1] {
        Stmt::Let(_, _, Expr::Call { node_id, .. }) => *node_id,
        other => panic!("expected Call, got {other:?}"),
    };
    let (errors, ann) = TypeChecker::check_and_annotate(&stmts);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let info = ann.call_info(node_id).expect("CallInfo recorded");
    // 呼び先シンボル参照 = "f"
    assert_eq!(info.callee.as_deref(), Some("f"));
    // 引数 2つ・ともに int
    assert_eq!(info.args.len(), 2);
    for a in &info.args {
        assert_eq!(ann.type_of(a.ty), Some(&InferredType::Int));
        assert_eq!(a.directive, Directive::None);
    }
}

/// 関数本体へ再帰して最初の `let ... = Call(...)` の node_id を返す。
fn find_call_node_id(stmts: &[Stmt]) -> Option<u32> {
    for s in stmts {
        match s {
            Stmt::Let(_, _, Expr::Call { node_id, .. })
            | Stmt::Return(Some(Expr::Call { node_id, .. })) => return Some(*node_id),
            Stmt::FnDef { body, .. } => {
                if let Some(id) = find_call_node_id(body) {
                    return Some(id);
                }
            }
            _ => {}
        }
    }
    None
}

#[test]
fn call_arg_dynamic_gets_check_before_directive() {
    // f(a: int) を Any 引数 x で呼ぶ → 引数0に CheckBefore(int)（境界検査）。
    let src = concat!(
        "fn f(a: int) -> int:\n",
        "    return a\n",
        "fn g(x: Any) -> int:\n",
        "    let y = f(x)\n",
        "    return y\n",
    );
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let node_id = find_call_node_id(&stmts).expect("Call node not found");
    let (_errors, ann) = TypeChecker::check_and_annotate(&stmts);
    let info = ann.call_info(node_id).expect("CallInfo recorded");
    assert_eq!(info.callee.as_deref(), Some("f"));
    assert_eq!(info.args.len(), 1);
    match &info.args[0].directive {
        Directive::CheckBefore(t) => {
            assert_eq!(ann.type_of(*t), Some(&InferredType::Int));
        }
        other => panic!("expected CheckBefore(int), got {other:?}"),
    }
}

#[test]
fn method_call_arg_dynamic_gets_check_directive() {
    // c.m(x): m(self, a: int) を Any 引数で呼ぶ → 引数0（self を除いた対応）に CheckBefore(int)。
    let src = concat!(
        "class C:\n",
        "    public:\n",
        "    fn m(self, a: int) -> int:\n",
        "        return a\n",
        "fn g(c: C, x: Any) -> int:\n",
        "    let y = c.m(x)\n",
        "    return y\n",
    );
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let node_id = find_call_node_id(&stmts).expect("Call node not found");
    let (_errors, ann) = TypeChecker::check_and_annotate(&stmts);
    let info = ann.call_info(node_id).expect("CallInfo recorded");
    assert_eq!(info.callee.as_deref(), Some("m"));
    assert_eq!(info.args.len(), 1);
    match &info.args[0].directive {
        Directive::CheckBefore(t) => assert_eq!(ann.type_of(*t), Some(&InferredType::Int)),
        other => panic!("expected CheckBefore(int) for method arg, got {other:?}"),
    }
}

#[test]
fn call_arg_static_has_no_check_directive() {
    // 静的に int を渡す → 検査不要（None）。
    let src = "fn f(a: int) -> int:\n    return a\nlet y = f(5)\n";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let node_id = match &stmts[1] {
        Stmt::Let(_, _, Expr::Call { node_id, .. }) => *node_id,
        other => panic!("expected Call, got {other:?}"),
    };
    let (_errors, ann) = TypeChecker::check_and_annotate(&stmts);
    let info = ann.call_info(node_id).expect("CallInfo recorded");
    assert_eq!(info.args[0].directive, Directive::None);
}

#[test]
fn unannotated_node_has_no_directive() {
    // node_id 0（未採番）や注釈のない node は None 相当。
    let src = "let x: Any = 5\nlet y = x mustbe int\n";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let (_errors, ann) = TypeChecker::check_and_annotate(&stmts);
    // 存在しない node_id は None ディレクティブ。
    assert_eq!(ann.directive(9_999_999), Directive::None);
    assert!(ann.resolved(9_999_999).is_none());
}
