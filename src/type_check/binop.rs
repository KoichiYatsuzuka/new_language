#![allow(dead_code)]

use crate::ast::BinOp;
use crate::token::Span;

use super::errors::{StaticTypeError, TypeErrorKind};
use super::types::InferredType;
use super::TypeChecker;

impl TypeChecker {
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
                (Str, Str) => Str,
                _ => Unresolved,
            },
            BinOp::Sub | BinOp::Mul | BinOp::Pow => match (lt, rt) {
                (Int, Int) => Int,
                (Float, Float) | (Int, Float) | (Float, Int) => Float,
                _ => Unresolved,
            },
            BinOp::Div => Float,
            BinOp::FloorDiv | BinOp::Mod => match (lt, rt) {
                (Int, Int) => Int,
                _ => Unresolved,
            },
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::LShift | BinOp::RShift => Int,
        }
    }
}
