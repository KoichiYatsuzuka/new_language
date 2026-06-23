#![allow(dead_code)]

use super::types::InferredType;
use crate::token::Span;

// ---------------------------------------------------------------------------
// Error kind
// ---------------------------------------------------------------------------

/// 静的型エラーの種別を表す列挙型。各バリアントに診断に必要な情報を保持する。
#[derive(Debug, Clone)]
pub enum TypeErrorKind {
    /// 互換性のない型同士の比較演算（例: `int == str`）。
    IncompatibleComparison {
        lhs: InferredType,
        rhs: InferredType,
        op: &'static str,
    },
    /// 不変変数（`let` / `const`）への代入。
    AssignToImmutable {
        name: String,
    },
    /// 関数呼び出しの引数数が宣言と一致しない（少なすぎる / 多すぎる）。
    CallArgCountMismatch {
        func_name: String,
        /// 最小必須引数数（デフォルト引数を除いた引数数）。
        expected_min: usize,
        /// 最大許容引数数（全引数数）。
        expected_max: usize,
        /// 実際に渡された引数数。
        got: usize,
    },
    /// 関数呼び出しの特定位置の引数型が宣言の型と不一致。
    CallArgTypeMismatch {
        func_name: String,
        /// 0始まりの引数インデックス。
        param_index: usize,
        expected: InferredType,
        got: InferredType,
    },
    /// 関数パラメータに型アノテーションがない。
    MissingParamTypeAnn {
        func_name: String,
        param_name: String,
    },
    /// 関数定義に戻り値型アノテーションがない。
    MissingReturnTypeAnn {
        func_name: String,
    },
    /// 関数に存在しないキーワード引数が渡された。
    UnknownKeywordArg {
        func_name: String,
        arg_name: String,
    },
    /// オーバーロード関数でどのオーバーロードにも引数数が一致しない。
    NoMatchingOverload {
        func_name: String,
        /// 実際に渡された引数数。
        got: usize,
        /// 各オーバーロードが受け付ける引数数のリスト。
        available: Vec<usize>,
    },
    /// メソッドの `self` / `cls` パラメータの型注釈がクラス名と不一致。
    SelfTypeMismatch {
        method: String,
        param_name: String,
        expected_class: String,
        got_class: String,
    },
    /// `Any` 型の値に対して演算子を適用した（明示的ダウンキャストが必要）。
    OperationOnAny {
        op: String,
    },
    /// `Union` 型の値に対して演算子を適用した（明示的ダウンキャストが必要）。
    OperationOnUnion {
        union_type: String,
        op: String,
    },
    /// `is not` 型ガードを非 Union 型に適用した（Union/Optional 型にのみ有効）。
    IsNotOnNonUnion {
        var_name: String,
        var_type: InferredType,
    },
    /// `mut` パラメータに不変変数（`let`/`const`）を渡した。
    CallMutParamWithImmutableArg {
        func_name: String,
        param_name: String,
    },
    /// デコレータの型シグネチャが無効（引数型や戻り値型が不一致など）。
    InvalidDecorator {
        reason: String,
    },
    /// タプルアンパック宣言の変数に `let` / `mut` 修飾子がない。
    TupleUnpackMissingQualifier {
        name: String,
    },
    /// タプルアンパックのターゲット数と右辺タプルの要素数が一致しない。
    TupleUnpackArityMismatch {
        /// 右辺タプルの要素数。
        tuple_len: usize,
        /// タプルアンパックで宣言されたターゲット数（ワイルドカードを除く）。
        target_count: usize,
        /// ワイルドカード `_` が含まれているかどうか。
        has_wildcard: bool,
    },
    /// 不変フィールド（`let` フィールド）への代入（`__init__` 外）。
    AssignToImmutableField {
        field_name: String,
        class_name: String,
    },
    /// `private` メンバーにクラス外からアクセスした。
    PrivateAccessError {
        member_name: String,
        class_name: String,
    },
    /// `protected` メンバーに継承クラス外からアクセスした。
    ProtectedAccessError {
        member_name: String,
        class_name: String,
    },
    /// `static` メソッドをインスタンス経由で呼び出した（クラス名経由でのみ呼び出せる）。
    StaticMethodOnInstance {
        method_name: String,
        class_name: String,
    },
    /// `for`/`while` 式の直下に `block_return` が使われた（`loop_yield` か内側のブロック式に移す必要がある）。
    BlockReturnInLoopExpr,
    /// `raise` の対象が例外インスタンスでない。
    InvalidRaiseType {
        got: InferredType,
    },
    /// `mut` / `let` フィールドにデフォルト値が指定された（`const` フィールドのみデフォルト値を持てる）。
    FieldDefaultNotAllowed {
        field_name: String,
        /// フィールドの種別文字列（`"mut"` または `"let"`）。
        kind: String,
    },
    /// `__freeze__` メソッドを直接呼び出した（`freeze` キーワード経由でのみ使用可能）。
    DirectFreezeCall,
}

// ---------------------------------------------------------------------------
// StaticTypeError
// ---------------------------------------------------------------------------

/// 静的型検査で収集されるエラー情報。エラー種別とソース位置を保持する。
#[derive(Debug, Clone)]
pub struct StaticTypeError {
    pub kind: TypeErrorKind,
    pub span: Option<Span>,
}

impl StaticTypeError {
    /// 互換性のない型同士の比較エラーを生成するファクトリ関数。
    pub(super) fn incompatible_cmp(
        lhs: InferredType,
        rhs: InferredType,
        op: &'static str,
        span: Span,
    ) -> Self {
        Self {
            kind: TypeErrorKind::IncompatibleComparison { lhs, rhs, op },
            span: Some(span),
        }
    }

    /// イミュータブル変数への代入エラーを生成するファクトリ関数。
    pub(super) fn assign_immutable(name: &str, span: Span) -> Self {
        Self {
            kind: TypeErrorKind::AssignToImmutable {
                name: name.to_string(),
            },
            span: Some(span),
        }
    }

    /// エラーが発生したファイル名を返す。スパン情報がない場合は `"<unknown>"` を返す。
    pub fn file_str(&self) -> String {
        match &self.span {
            Some(span) if span.line != 0 && !span.file.is_empty() => span.file.to_string(),
            _ => "<unknown>".to_string(),
        }
    }

    /// エラー発生位置の行番号と列番号を `"行:列"` 形式で返す。不明な場合は `"-"` を返す。
    pub fn line_col_str(&self) -> String {
        match &self.span {
            Some(span) if span.line != 0 => format!("{}:{}", span.line, span.col),
            _ => "-".to_string(),
        }
    }

    /// エラー種別の固定文字列 `"StaticTypeError"` を返す。
    pub fn error_type_str(&self) -> &'static str {
        "StaticTypeError"
    }

    /// エラーの詳細メッセージ文字列を ANSI エスケープシーケンス付きで返す。
    pub fn detail_str(&self) -> String {
        match &self.kind {
            TypeErrorKind::IncompatibleComparison { lhs, rhs, op } => format!(
                "cannot compare {} and {} with {}", hl_q(lhs), hl_q(rhs), hl_bt(op)
            ),
            TypeErrorKind::AssignToImmutable { name } => format!(
                "cannot assign to immutable variable {}", hl_q(name)
            ),
            TypeErrorKind::CallArgCountMismatch { func_name, expected_min, expected_max, got } => {
                if expected_min == expected_max {
                    format!("{} takes {expected_min} argument(s) but {got} were given", hl_q(func_name))
                } else {
                    format!("{} takes {expected_min} to {expected_max} argument(s) but {got} were given", hl_q(func_name))
                }
            }
            TypeErrorKind::CallArgTypeMismatch { func_name, param_index, expected, got } => format!(
                "argument {param_index} of {} expects {} but got {}",
                hl_q(func_name), hl_q(expected), hl_q(got)
            ),
            TypeErrorKind::MissingParamTypeAnn { func_name, param_name } => format!(
                "parameter {} of function {} is missing a type annotation",
                hl_q(param_name), hl_q(func_name)
            ),
            TypeErrorKind::MissingReturnTypeAnn { func_name } => format!(
                "function {} is missing a return type annotation", hl_q(func_name)
            ),
            TypeErrorKind::UnknownKeywordArg { func_name, arg_name } => format!(
                "{} has no parameter named {}", hl_q(func_name), hl_q(arg_name)
            ),
            TypeErrorKind::NoMatchingOverload { func_name, got, available } => {
                let avail = available.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
                format!("no overload of {} takes {got} argument(s) (overloads take: {avail})", hl_q(func_name))
            }
            TypeErrorKind::SelfTypeMismatch { method, param_name, expected_class, got_class } => format!(
                "parameter {} of {} expects {} = {} but got {}",
                hl_q(param_name), hl_q(method), hl_q("Self"), hl_q(expected_class), hl_q(got_class)
            ),
            TypeErrorKind::OperationOnAny { op } => format!(
                "cannot apply {} to {} — explicit downcast required", hl_bt(op), hl_q("Any")
            ),
            TypeErrorKind::OperationOnUnion { union_type, op } => format!(
                "cannot apply {} to {} — explicit downcast required", hl_bt(op), hl_q(union_type)
            ),
            TypeErrorKind::IsNotOnNonUnion { var_name, var_type } => format!(
                "{} type guard on {} requires a Union or Optional type, but got {}",
                hl_q("is not"), hl_q(var_name), hl_q(var_type)
            ),
            TypeErrorKind::CallMutParamWithImmutableArg { func_name, param_name } => format!(
                "parameter {} of {} expects a mutable argument, but got an immutable value",
                hl_q(param_name), hl_q(func_name)
            ),
            TypeErrorKind::InvalidDecorator { reason } => format!(
                "invalid decorator: \x1b[1;35m{reason}\x1b[0m"
            ),
            TypeErrorKind::TupleUnpackMissingQualifier { name } => format!(
                "variable {} in tuple unpack requires {} or {} qualifier",
                hl_q(name), hl_bt("let"), hl_bt("mut")
            ),
            TypeErrorKind::TupleUnpackArityMismatch { tuple_len, target_count, has_wildcard } => {
                if *has_wildcard {
                    format!("tuple unpack has \x1b[1;35m{target_count}\x1b[0m variable(s) but tuple has only \x1b[1;35m{tuple_len}\x1b[0m element(s)")
                } else {
                    format!("tuple unpack expects \x1b[1;35m{target_count}\x1b[0m element(s) but tuple has \x1b[1;35m{tuple_len}\x1b[0m")
                }
            }
            TypeErrorKind::AssignToImmutableField { field_name, class_name } => format!(
                "cannot assign to immutable field {} of class {}", hl_q(field_name), hl_q(class_name)
            ),
            TypeErrorKind::PrivateAccessError { member_name, class_name } => format!(
                "{} is private and cannot be accessed outside {}", hl_q(member_name), hl_q(class_name)
            ),
            TypeErrorKind::ProtectedAccessError { member_name, class_name } => format!(
                "{} is protected and cannot be accessed outside {} or its subclasses",
                hl_q(member_name), hl_q(class_name)
            ),
            TypeErrorKind::StaticMethodOnInstance { method_name, class_name } => format!(
                "static method {} must be called on {}, not an instance",
                hl_q(method_name), hl_bt(class_name)
            ),
            TypeErrorKind::BlockReturnInLoopExpr => format!(
                "{} cannot be used directly in a {} or {} expression body; use {} to accumulate values or nest inside an {} / {} / {} expression",
                hl_bt("block_return"), hl_bt("for"), hl_bt("while"),
                hl_bt("loop_yield"), hl_bt("if"), hl_bt("match"), hl_bt("block:")
            ),
            TypeErrorKind::InvalidRaiseType { got } => format!(
                "{} expects an instance implementing trait {}, but got {}",
                hl_bt("raise"), hl_q("Error"), hl_q(got)
            ),
            TypeErrorKind::FieldDefaultNotAllowed { field_name, kind } => format!(
                "{} field {} cannot have a default value in the class declaration; only {} or {} fields may have defaults",
                hl_bt(kind), hl_q(field_name), hl_bt("const"), hl_bt("static mut")
            ),
            TypeErrorKind::DirectFreezeCall => format!(
                "{} cannot be called directly; use the {} keyword instead",
                hl_bt("__freeze__"), hl_bt("freeze")
            ),
        }
    }
}

/// 値をシングルクォートで囲み、マゼンタ太字の ANSI 装飾を付けた文字列を返す。
fn hl_q(s: impl std::fmt::Display) -> String {
    format!("'\x1b[1;35m{s}\x1b[0m'")
}

/// 値をバッククォートで囲み、マゼンタ太字の ANSI 装飾を付けた文字列を返す。
fn hl_bt(s: impl std::fmt::Display) -> String {
    format!("`\x1b[1;35m{s}\x1b[0m`")
}

impl std::fmt::Display for StaticTypeError {
    /// エラーを `"位置: StaticTypeError: 詳細"` 形式の ANSI 色付き文字列として出力する。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const R: &str = "\x1b[31m";
        const X: &str = "\x1b[0m";
        let loc = match &self.span {
            Some(span) => format!("{span}"),
            None => "\x1b[33m<unknown>\x1b[0m".to_string(),
        };
        write!(f, "{loc}: {R}StaticTypeError{X}: {}", self.detail_str())
    }
}
