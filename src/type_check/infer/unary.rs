// type_check/infer/unary.rs — 単項演算子の型。

use super::*;

impl TypeChecker {
    /// 単項演算子の結果型を推論する。`Any`/`Union` オペランドは診断する。
    pub(super) fn infer_unaryop(&mut self, op: &UnaryOp, operand: &Expr) -> InferredType {
        let ty = self.infer(operand);
        let op_str = match op {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "not",
            UnaryOp::BitNot => "~",
        };
        match &ty {
            // 単項演算の誤りは**対象の式**を指す（3-4。単項演算の式は錨を持たない）。
            InferredType::Any => {
                self.report_error_at(operand, StaticTypeError {
                    kind: TypeErrorKind::OperationOnAny {
                        op: op_str.to_string(),
                    },
                    span: None,
                });
                return InferredType::Unresolved;
            }
            InferredType::Union(_) => {
                self.report_error_at(operand, StaticTypeError {
                    kind: TypeErrorKind::OperationOnUnion {
                        union_type: ty.to_string(),
                        op: op_str.to_string(),
                    },
                    span: None,
                });
                return InferredType::Unresolved;
            }
            _ => {}
        }
        // ⚠⚠ **可否と結果型を同じ表で決める**（4.4 の `check_arith` と同じ形・タスク 7.1）。
        //    以前は「不可の組み合わせ」でも黙って `Unresolved`（`BitNot` に至っては
        //    無条件 `Int`）を返しており、**型が判っているのに誤りを報告しない**
        //    ＝根本原因①（`Unresolved` は万能受容体）がここに残っていた。
        //
        // ## 実測した実行時の規則
        //
        // | 演算子 | 許される型 | 外れたとき |
        // |---|---|---|
        // | `-`  | `int` ・ `float` ・ `complex` | `bad operand type for unary '-'` |
        // | `~`  | `int` のみ | `bad operand type for unary '~'` |
        // | `not`| 何でも（`eval_truthy` を通る） | — |
        //
        // ⚠⚠ **`bool` は `-` も `~` も不可**（`-True` / `~True` は実行時 TypeError）。
        //    整数として通る言語の癖で書くと落ちるので静的に弾く価値がある。
        // ⚠ クラスは `__neg__` を定義できる（実測）ので素通しする。
        if matches!(op, UnaryOp::Not) {
            return InferredType::Bool;
        }
        if Self::opaque_for_unary(&ty) {
            return InferredType::Unresolved;
        }
        let result = match op {
            UnaryOp::Neg => match ty {
                InferredType::Int => Some(InferredType::Int),
                InferredType::Float => Some(InferredType::Float),
                InferredType::Complex => Some(InferredType::Complex),
                _ => None,
            },
            UnaryOp::BitNot => match ty {
                InferredType::Int => Some(InferredType::Int),
                _ => None,
            },
            UnaryOp::Not => unreachable!("`not` は上で返している"),
        };
        match result {
            Some(t) => t,
            None => {
                self.report_error_at(operand, StaticTypeError {
                    kind: TypeErrorKind::IncompatibleUnaryOp {
                        op: op_str.to_string(),
                        operand: ty,
                    },
                    span: None,
                });
                InferredType::Unresolved
            }
        }
    }

    /// 単項演算子の検査で「判定材料が無い」とみなす型（タスク 7.1）。
    ///
    /// ⚠ クラス（`__neg__` を定義できる）を含めるのが要点。
    pub(super) fn opaque_for_unary(ty: &InferredType) -> bool {
        use InferredType as T;
        matches!(
            ty,
            T::Unresolved
                | T::Any
                | T::Never
                | T::NamedInstance(_)
                | T::GenericInstance { .. }
                | T::SelfType
                | T::Protocol(..)
                | T::Union(_)
                | T::Intersection(_)
                | T::Result(_, _)
                | T::Namespace(..)
                | T::PyNamespace(..)
                | T::TypeVal
                | T::TypeValOf(_)
        )
    }
}
