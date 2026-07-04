// stmts/mod.rs — 文パースサブシステムのモジュール束ね。
// 共有ヘルパー(token_to_compound_op)を保持し、役割別サブモジュール
// (core/control_flow/assignment/functions/definitions)を宣言する。

use crate::ast::BinOp;
use crate::token::Token;

/// 複合代入演算子トークンを対応する二項演算子（`BinOp`）に変換する。
///
/// `+=`, `-=`, `*=` などのトークンを対応する `BinOp` にマッピングする。
/// 複合代入演算子でないトークンの場合は `None` を返す。
fn token_to_compound_op(token: &Token) -> Option<BinOp> {
    match token {
        Token::PlusEq => Some(BinOp::Add),
        Token::MinusEq => Some(BinOp::Sub),
        Token::StarEq => Some(BinOp::Mul),
        Token::SlashEq => Some(BinOp::Div),
        Token::SlashSlashEq => Some(BinOp::FloorDiv),
        Token::PercentEq => Some(BinOp::Mod),
        Token::StarStarEq => Some(BinOp::Pow),
        Token::AmpEq => Some(BinOp::BitAnd),
        Token::PipeEq => Some(BinOp::BitOr),
        Token::CaretEq => Some(BinOp::BitXor),
        Token::LtLtEq => Some(BinOp::LShift),
        Token::GtGtEq => Some(BinOp::RShift),
        _ => None,
    }
}


mod core;
mod control_flow;
mod assignment;
mod functions;
mod definitions;
