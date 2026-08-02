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
