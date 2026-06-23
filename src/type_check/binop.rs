#![allow(dead_code)]

use crate::ast::BinOp;
use crate::token::Span;

use super::errors::{StaticTypeError, TypeErrorKind};
use super::types::InferredType;
use super::TypeChecker;

impl TypeChecker {
    /// 二項演算子の型検査を行い、`Any` 型・`Union` 型への演算や順序比較の不整合をエラーとして記録する。
    pub(super) fn check_binop(
        &mut self,
        op: &BinOp,
        lt: &InferredType,
        rt: &InferredType,
        span: Span,
    ) {
        if *lt == InferredType::Any || *rt == InferredType::Any {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::OperationOnAny {
                    op: op.as_str().to_string(),
                },
                span: Some(span),
            });
            return;
        }
        let union_side = if matches!(lt, InferredType::Union(_)) {
            Some(lt)
        } else if matches!(rt, InferredType::Union(_)) {
            Some(rt)
        } else {
            None
        };
        if let Some(union_ty) = union_side {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::OperationOnUnion {
                    union_type: union_ty.to_string(),
                    op: op.as_str().to_string(),
                },
                span: Some(span),
            });
            return;
        }
        match op {
            BinOp::Lt => self.check_ordered_cmp(lt, rt, "<", span),
            BinOp::Gt => self.check_ordered_cmp(lt, rt, ">", span),
            BinOp::LtEq => self.check_ordered_cmp(lt, rt, "<=", span),
            BinOp::GtEq => self.check_ordered_cmp(lt, rt, ">=", span),
            _ => {}
        }
    }

    /// 順序比較演算子 (`<`, `>`, `<=`, `>=`) の型整合性を検査し、不整合があればエラーを記録する。
    fn check_ordered_cmp(
        &mut self,
        lt: &InferredType,
        rt: &InferredType,
        op: &'static str,
        span: Span,
    ) {
        if !Self::ordered_comparable(lt, rt) {
            self.report_error(StaticTypeError::incompatible_cmp(
                lt.clone(),
                rt.clone(),
                op,
                span,
            ));
        }
    }

    /// 2 つの型が順序比較可能な組み合わせかどうかを判定する。
    fn ordered_comparable(lt: &InferredType, rt: &InferredType) -> bool {
        use InferredType::*;
        matches!(
            (lt, rt),
            (Unresolved, _)
                | (_, Unresolved)
                | (Int, Int)
                | (Float, Float)
                | (Int, Float)
                | (Float, Int)
                | (Str, Str)
        )
    }

    /// 二項演算子と両辺の型から演算結果の型を推論して返す。
    pub(super) fn infer_binop_result(
        op: &BinOp,
        lt: &InferredType,
        rt: &InferredType,
    ) -> InferredType {
        use InferredType::*;
        if *lt == Any || *rt == Any {
            return Unresolved;
        }
        if matches!(lt, Union(_)) || matches!(rt, Union(_)) {
            return Unresolved;
        }
        if *lt == Set && *rt == Set {
            return match op {
                BinOp::BitOr | BinOp::BitAnd | BinOp::BitXor | BinOp::Sub => Set,
                BinOp::Eq | BinOp::NotEq => Bool,
                _ => Unresolved,
            };
        }
        match op {
            BinOp::Eq
            | BinOp::RefEq
            | BinOp::NotEq
            | BinOp::Lt
            | BinOp::Gt
            | BinOp::LtEq
            | BinOp::GtEq
            | BinOp::And
            | BinOp::Or
            | BinOp::In
            | BinOp::NotIn => Bool,
            BinOp::Add => match (lt, rt) {
                (Int, Int) => Int,
                (Float, Float) | (Int, Float) | (Float, Int) => Float,
                (Complex, Complex)
                | (Complex, Float) | (Float, Complex)
                | (Complex, Int)   | (Int, Complex) => Complex,
                (Str, Str) => Str,
                _ => Unresolved,
            },
            BinOp::Sub | BinOp::Mul => match (lt, rt) {
                (Int, Int) => Int,
                (Float, Float) | (Int, Float) | (Float, Int) => Float,
                (Complex, Complex)
                | (Complex, Float) | (Float, Complex)
                | (Complex, Int)   | (Int, Complex) => Complex,
                _ => Unresolved,
            },
            BinOp::Pow => match (lt, rt) {
                (Int, Int) => Int,
                (Float, Float) | (Int, Float) | (Float, Int) => Float,
                _ => Unresolved,
            },
            BinOp::Div => match (lt, rt) {
                (Complex, _) | (_, Complex) => Complex,
                _ => Float,
            },
            BinOp::FloorDiv | BinOp::Mod => match (lt, rt) {
                (Int, Int) => Int,
                _ => Unresolved,
            },
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::LShift | BinOp::RShift => Int,
        }
    }
}
