#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use crate::ast::{Accessibility, BinOp, CallArg, Expr, MatchPattern, Stmt, TupleTarget, UnaryOp};
use crate::token::Span;

// ---------------------------------------------------------------------------
// Inferred type
// ---------------------------------------------------------------------------

/// 静的型検査で推論される型の種別。
///
/// AST を走査して式に割り当てられる型を表す。実行時の `Value` とは独立した型表現であり、
/// 静的解析フェーズでのみ使用される。型が静的に確定しない場合は `Unresolved` を使用し、
/// 実行時の型検査に委ねる。
/// 関数型アノテーション（`function[...]->T` / `function{...}->T`）の1パラメータ。
#[derive(Debug, Clone, PartialEq)]
pub struct FnTypeParam {
    /// パラメータ名（位置引数は自動生成 `param1`, `param2`, ...）。
    pub name: String,
    /// `mut` 修飾子の有無。
    pub mutable: bool,
    /// パラメータの型（アノテーションなしは `Any`）。
    pub ty: InferredType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InferredType {
    /// 整数型（`int`）
    Int,
    /// 浮動小数点型（`float`）
    Float,
    /// 文字列型（`str`）
    Str,
    /// 真偽値型（`bool`）
    Bool,
    /// ヌル型（`None`）
    None,
    /// リスト型（bare `list`、要素型不明）。`list[T]` を受け付けるパラメータにも使用する。
    List,
    /// 要素型が既知のリスト型（`list[T]`）。
    ListOf(Box<InferredType>),
    /// 型そのものを値として保持する型（bare `type` アノテーション、制約なし）。
    TypeVal,
    /// 特定の型に制約された型値（`type[T]` アノテーション）。
    /// `type[int]` は `int` 型またはその派生型（new_type / trait 実装）のみを受け付ける。
    TypeValOf(Box<InferredType>),
    /// クラス・trait 本体でのみ使用できる特殊な型キーワード。
    /// 呼び出し時にレシーバのクラスに解決される。
    SelfType,
    /// 名前付きクラスまたは new_type のインスタンス。
    /// 文字列はクラス名（または new_type 名）を表す。
    NamedInstance(String),
    /// 任意の値を受け付ける型。使用前に明示的なダウンキャストが必要。
    Any,
    /// `Union[T1, T2, ...]` または `Option[T]`（`Union[T, None]` に脱糖される）。
    /// 使用前に明示的なダウンキャストが必要。
    Union(Vec<InferredType>),
    /// 辞書型（bare `dict`、キー・値型不明）。
    Dict,
    /// キー型と値型が既知の辞書型（`dict[K, V]`）。
    DictOf(Box<InferredType>, Box<InferredType>),
    /// セット型（bare `set`、要素型不明）。
    Set,
    /// 要素型が既知のセット型（`set[T]`）。
    SetOf(Box<InferredType>),
    /// 各要素の型が既知のタプル型 `tuple[T1, T2, ...]`。
    /// `Vec` の各要素がタプルの各フィールドの型に対応する。
    Tuple(Vec<InferredType>),
    /// import されたモジュール（名前空間）の型。
    /// `HashMap` のキーはメンバ名、値はそのメンバの型。
    Namespace(std::collections::HashMap<String, InferredType>),
    /// 静的に確定できない型。実行時の型検査に委ねる。
    Unresolved,
    /// 関数型（`function`, `function[...]->T`, `function{...}->T`）。
    ///
    /// - `params = None`  : 任意の引数リスト（bare `function`）
    /// - `params = Some(v)`: 明示的なパラメータリスト（空なら引数なし）
    /// - `return_type`    : 戻り値型（アノテーションなしは `Any`）
    Function {
        params: Option<Vec<FnTypeParam>>,
        return_type: Box<InferredType>,
    },
}

/// 文字列 `s` をブラケット深さ 0 のカンマで分割する。
///
/// `Union[int, str]` や `Option[int]` のようなネストした型引数を正しく分割するために使用する。
/// ブラケット内のカンマは区切りとして扱われない。
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            ',' if depth == 0 => {
                result.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    result.push(&s[start..]);
    result
}

/// `split_top_level_commas` と同様だが `[` / `]` と `{` / `}` の両方を深さに数える。
/// 関数型アノテーション内のパラメータリスト分割に使用する。
fn split_top_level_commas_fn(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' | '{' => depth += 1,
            ']' | '}' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            ',' if depth == 0 => {
                result.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    result.push(&s[start..]);
    result
}

impl InferredType {
    /// 型注釈文字列（例: `"int"`, `"Union[int,str]"`, `"tuple[int, str]"`, `"function[let int]->int"`）を
    /// [`InferredType`] に変換する。
    ///
    /// 認識できない文字列は `None` を返す。
    fn from_ann(ann: &str) -> Option<Self> {
        if let Some(inner) = ann.strip_prefix("Union[").and_then(|s| s.strip_suffix(']')) {
            let parts = split_top_level_commas(inner);
            let types: Vec<InferredType> = parts
                .iter()
                .filter_map(|t| InferredType::from_ann(t.trim()))
                .collect();
            return if types.len() >= 2 {
                Some(Self::Union(types))
            } else {
                None
            };
        }
        if let Some(inner) = ann
            .strip_prefix("Option[")
            .and_then(|s| s.strip_suffix(']'))
        {
            return InferredType::from_ann(inner.trim()).map(|t| Self::Union(vec![t, Self::None]));
        }
        if let Some(inner) = ann.strip_prefix("list[").and_then(|s| s.strip_suffix(']')) {
            return Some(match InferredType::from_ann(inner.trim()) {
                Some(t) => Self::ListOf(Box::new(t)),
                None => Self::List,
            });
        }
        if let Some(inner) = ann.strip_prefix("set[").and_then(|s| s.strip_suffix(']')) {
            return Some(match InferredType::from_ann(inner.trim()) {
                Some(t) => Self::SetOf(Box::new(t)),
                None => Self::Set,
            });
        }
        if let Some(inner) = ann.strip_prefix("dict[").and_then(|s| s.strip_suffix(']')) {
            let parts = split_top_level_commas(inner);
            if parts.len() >= 2 {
                if let (Some(k), Some(v)) = (
                    InferredType::from_ann(parts[0].trim()),
                    InferredType::from_ann(parts[1].trim()),
                ) {
                    return Some(Self::DictOf(Box::new(k), Box::new(v)));
                }
            }
            return Some(Self::Dict);
        }
        if let Some(inner) = ann.strip_prefix("tuple[").and_then(|s| s.strip_suffix(']')) {
            let parts = split_top_level_commas(inner);
            let types: Vec<InferredType> = parts
                .iter()
                .filter_map(|t| InferredType::from_ann(t.trim()))
                .collect();
            return Some(Self::Tuple(types));
        }
        if let Some(inner) = ann.strip_prefix("type[").and_then(|s| s.strip_suffix(']')) {
            let inner = inner.trim();
            // 既知プリミティブ型 + NamedInstance フォールバック（クラス・new_type・trait 名）
            let inner_ty = Self::from_ann(inner).or_else(|| {
                if inner
                    .chars()
                    .next()
                    .map(|c| c.is_alphabetic() || c == '_')
                    .unwrap_or(false)
                    && inner.chars().all(|c| c.is_alphanumeric() || c == '_')
                {
                    Some(Self::NamedInstance(inner.to_string()))
                } else {
                    None
                }
            });
            return inner_ty.map(|t| Self::TypeValOf(Box::new(t)));
        }
        if let Some(rest) = ann.strip_prefix("function") {
            return Self::parse_fn_type_ann(rest);
        }
        match ann {
            "int" => Some(Self::Int),
            "float" => Some(Self::Float),
            "str" => Some(Self::Str),
            "bool" => Some(Self::Bool),
            "None" => Some(Self::None),
            "list" => Some(Self::List),
            "dict" => Some(Self::Dict),
            "set" => Some(Self::Set),
            "type" => Some(Self::TypeVal),
            "Self" => Some(Self::SelfType),
            "Any" => Some(Self::Any),
            _ => None,
        }
    }

    /// `function` の後続文字列（`""`, `"[...]->T"`, `"{...}->T"` など）から
    /// `InferredType::Function` を構築する。
    fn parse_fn_type_ann(rest: &str) -> Option<Self> {
        let (params, after_params) = if rest.starts_with('[') {
            let close = Self::find_closing_bracket(rest, '[', ']')?;
            let inner = &rest[1..close];
            let after = &rest[close + 1..];
            let params = if inner.trim().is_empty() {
                vec![]
            } else {
                let parts = split_top_level_commas_fn(inner);
                let mut out = Vec::new();
                for (i, part) in parts.iter().enumerate() {
                    let p = part.trim();
                    let (mutable, type_str) = if let Some(t) = p.strip_prefix("mut ") {
                        (true, t.trim())
                    } else if let Some(t) = p.strip_prefix("let ") {
                        (false, t.trim())
                    } else {
                        (false, p)
                    };
                    // positional params serialized as "let paramN:type"
                    let (name, ty_s) = if let Some(colon) = type_str.find(':') {
                        (
                            type_str[..colon].trim().to_string(),
                            type_str[colon + 1..].trim(),
                        )
                    } else {
                        (format!("param{}", i + 1), type_str)
                    };
                    let ty = Self::from_ann(ty_s).unwrap_or(Self::Any);
                    out.push(FnTypeParam { name, mutable, ty });
                }
                out
            };
            (Some(params), after)
        } else if rest.starts_with('{') {
            let close = Self::find_closing_bracket(rest, '{', '}')?;
            let inner = &rest[1..close];
            let after = &rest[close + 1..];
            let params = if inner.trim().is_empty() {
                vec![]
            } else {
                let parts = split_top_level_commas_fn(inner);
                let mut out = Vec::new();
                for part in parts.iter() {
                    let p = part.trim();
                    let (mutable, rest_p) = if let Some(t) = p.strip_prefix("mut ") {
                        (true, t.trim())
                    } else if let Some(t) = p.strip_prefix("let ") {
                        (false, t.trim())
                    } else {
                        (false, p)
                    };
                    let colon = rest_p.find(':')?;
                    let name = rest_p[..colon].trim().to_string();
                    let ty_s = rest_p[colon + 1..].trim();
                    let ty = Self::from_ann(ty_s).unwrap_or(Self::Any);
                    out.push(FnTypeParam { name, mutable, ty });
                }
                out
            };
            (Some(params), after)
        } else {
            (None, rest)
        };

        let return_type = if let Some(ret_s) = after_params.strip_prefix("->") {
            Self::from_ann(ret_s.trim()).unwrap_or(Self::Any)
        } else {
            Self::Any
        };

        Some(Self::Function {
            params,
            return_type: Box::new(return_type),
        })
    }

    /// 文字列 `s` の先頭から対応するブラケットの閉じ位置を返す。
    fn find_closing_bracket(s: &str, open: char, close: char) -> Option<usize> {
        let mut depth = 0usize;
        for (i, c) in s.char_indices() {
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Function signature (param types + return type)
// ---------------------------------------------------------------------------

/// 関数（またはメソッド）のシグネチャ情報。
///
/// 前処理パス（[`TypeChecker::collect_fn_sigs`]）で収集し、呼び出し検査で参照する。
/// オーバーロードがある場合は同じ関数名に複数の `FnSig` が紐付けられる。
#[derive(Clone)]
struct FnSig {
    /// パラメータのリスト。各要素は `(パラメータ名, 宣言された型)` のタプル。
    /// 型アノテーションがないパラメータの型は `None`。
    params: Vec<(String, Option<InferredType>)>,
    /// デフォルト値なしの必須パラメータ数（`self` を含む）。
    required_count: usize,
    /// 戻り値の型。型アノテーションがない場合は `None`。
    return_type: Option<InferredType>,
}

impl std::fmt::Display for InferredType {
    /// `InferredType` を人間が読める文字列に変換する。
    ///
    /// エラーメッセージ中の型名表示に使用される。
    /// `Union[T, None]` は `Option[T]` として表示し、ユーザーが型注釈で書いた形式と一致させる。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int => write!(f, "int"),
            Self::Float => write!(f, "float"),
            Self::Str => write!(f, "str"),
            Self::Bool => write!(f, "bool"),
            Self::None => write!(f, "None"),
            Self::List => write!(f, "list"),
            Self::ListOf(t) => write!(f, "list[{t}]"),
            Self::Dict => write!(f, "dict"),
            Self::DictOf(k, v) => write!(f, "dict[{k},{v}]"),
            Self::Set => write!(f, "set"),
            Self::SetOf(t) => write!(f, "set[{t}]"),
            Self::TypeVal => write!(f, "type"),
            Self::TypeValOf(inner) => write!(f, "type[{inner}]"),
            Self::SelfType => write!(f, "Self"),
            Self::NamedInstance(name) => write!(f, "{name}"),
            Self::Any => write!(f, "Any"),
            Self::Union(types) => {
                // Union[T, None] は Option[T] として表示（型注釈の脱糖と逆変換）
                if types.len() == 2 && types[1] == Self::None {
                    write!(f, "Option[{}]", types[0])
                } else {
                    let parts: Vec<String> = types.iter().map(|t| t.to_string()).collect();
                    write!(f, "Union[{}]", parts.join(", "))
                }
            }
            Self::Tuple(types) => {
                let parts: Vec<String> = types.iter().map(|t| t.to_string()).collect();
                write!(f, "tuple[{}]", parts.join(", "))
            }
            Self::Namespace(members) => write!(f, "<module({} members)>", members.len()),
            Self::Unresolved => write!(f, "unknown"),
            Self::Function {
                params,
                return_type,
            } => {
                match params {
                    None => write!(f, "function")?,
                    Some(ps) => {
                        let parts: Vec<String> = ps
                            .iter()
                            .map(|p| {
                                let prefix = if p.mutable { "mut" } else { "let" };
                                format!("{prefix} {}:{}", p.name, p.ty)
                            })
                            .collect();
                        write!(f, "function{{{}}}", parts.join(","))?;
                    }
                }
                if **return_type != Self::Any {
                    write!(f, "->{return_type}")?;
                }
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Error kind — add new variants here when extending checks
// ---------------------------------------------------------------------------

/// 静的型エラーの種別。
///
/// 新しい型検査ルールを追加する場合はこの enum に variant を追加し、
/// [`TypeChecker::check_stmt`] / [`TypeChecker::check_binop`] に対応する arm を追加する。
/// エラーのフォーマットは [`StaticTypeError`] の `Display` 実装で定義される。
#[derive(Debug, Clone)]
pub enum TypeErrorKind {
    /// 順序比較演算子（`<` / `>` / `<=` / `>=`）に互換性のない型を使用した
    /// （例: `str < int`）。
    IncompatibleComparison {
        /// 左辺の推論型
        lhs: InferredType,
        /// 右辺の推論型
        rhs: InferredType,
        /// 使用された演算子記号（`"<"` / `">"` / `"<="` / `">="` のいずれか）
        op: &'static str,
    },
    /// 不変変数（`let` / `const`）への代入または複合代入を試みた。
    AssignToImmutable {
        /// 代入対象の変数名
        name: String,
    },
    /// 関数呼び出し時に渡した引数の個数が定義と異なる（単一定義の場合）。
    CallArgCountMismatch {
        /// 呼び出した関数名
        func_name: String,
        /// 必須引数の最小個数
        expected_min: usize,
        /// 受け付ける最大引数個数（デフォルト値なしなら expected_min と同じ）
        expected_max: usize,
        /// 実際に渡された引数個数
        got: usize,
    },
    /// 関数呼び出し時に渡した引数の型がパラメータ宣言型と不一致。
    CallArgTypeMismatch {
        /// 呼び出した関数名
        func_name: String,
        /// 不一致が発生したパラメータの 0 始まりインデックス
        param_index: usize,
        /// パラメータに宣言された期待型
        expected: InferredType,
        /// 実際に渡した引数の推論型
        got: InferredType,
    },
    /// 関数パラメータに型アノテーションがない（`self` は除外）。
    MissingParamTypeAnn {
        /// 対象の関数名
        func_name: String,
        /// アノテーションが欠如しているパラメータ名
        param_name: String,
    },
    /// 関数定義に戻り値型アノテーションがない。
    MissingReturnTypeAnn {
        /// 対象の関数名
        func_name: String,
    },
    /// キーワード引数の名前が関数のいずれのパラメータとも一致しない。
    UnknownKeywordArg {
        /// 呼び出した関数名
        func_name: String,
        /// 使用した存在しないキーワード引数名
        arg_name: String,
    },
    /// オーバーロード関数において、渡した引数個数と一致するオーバーロードが存在しない。
    NoMatchingOverload {
        /// 呼び出した関数名
        func_name: String,
        /// 実際に渡された引数個数
        got: usize,
        /// 利用可能なオーバーロードが受け付ける引数個数のリスト
        available: Vec<usize>,
    },
    /// `Self` 型パラメータに異なるクラス / new_type のインスタンスを渡した。
    SelfTypeMismatch {
        /// 呼び出したメソッド名
        method: String,
        /// `Self` 型が宣言されているパラメータ名
        param_name: String,
        /// レシーバのクラス名（期待されるクラス）
        expected_class: String,
        /// 実際に渡した引数のクラス名
        got_class: String,
    },
    /// `Any` 型の値に対して明示的なダウンキャストなしで演算子を適用した。
    OperationOnAny {
        /// 適用した演算子の文字列表現
        op: String,
    },
    /// `Union` / `Option` 型の値に対して明示的なダウンキャストなしで演算子を適用した。
    OperationOnUnion {
        /// 対象の Union 型の文字列表現（例: `"Union[int, str]"`）
        union_type: String,
        /// 適用した演算子の文字列表現
        op: String,
    },
    /// `x is not T` 型ガードの対象 `x` が `Union` / `Optional` 型ではない。
    /// `is not` ガードは Union 型にのみ意味があるため、それ以外の型はエラーとする。
    IsNotOnNonUnion {
        /// ガード対象の変数名
        var_name: String,
        /// 変数の実際の推論型
        var_type: InferredType,
    },
    /// 関数型変数の `mut` パラメータに不変の引数を渡した。
    CallMutParamWithImmutableArg {
        /// 呼び出した関数型変数の名前または説明
        func_name: String,
        /// `mut` が宣言されているパラメータ名
        param_name: String,
    },
    /// デコレータが対象（関数またはクラス）に対して無効な型シグネチャを持つ。
    InvalidDecorator {
        /// エラーの詳細理由
        reason: String,
    },
    /// タプルアンパック宣言でひとつの変数に `let` / `mut` 修飾子がない。
    TupleUnpackMissingQualifier {
        /// 修飾子のない変数名
        name: String,
    },
    /// タプルアンパック宣言で変数の個数とタプルの要素数が合わない。
    TupleUnpackArityMismatch {
        /// タプルの要素数
        tuple_len: usize,
        /// 宣言した変数の個数（`_` を除く）
        target_count: usize,
        /// `_` が含まれているか（true なら target_count <= tuple_len でよい）
        has_wildcard: bool,
    },
    /// `let` フィールドへの代入を `__init__` 以外から試みた。
    AssignToImmutableField {
        /// 代入対象のフィールド名
        field_name: String,
        /// フィールドを持つクラス名
        class_name: String,
    },
    /// `private` メンバへのアクセスをクラス外から試みた。
    PrivateAccessError {
        /// アクセス対象のメンバ名
        member_name: String,
        /// メンバを所有するクラス名
        class_name: String,
    },
    /// `protected` メンバへのアクセスをクラス外（かつ派生クラス外）から試みた。
    ProtectedAccessError {
        /// アクセス対象のメンバ名
        member_name: String,
        /// メンバを所有するクラス名
        class_name: String,
    },
    /// `static` メソッドをインスタンスから呼び出そうとした。
    StaticMethodOnInstance {
        /// 呼び出したメソッド名
        method_name: String,
        /// メソッドを所有するクラス名
        class_name: String,
    },
    /// `block_return` を `for` または `while` 式の直接本体内で使用した。
    /// `for`/`while` 式の直接本体では `loop_yield` を使用する必要がある。
    /// `block_return` は `for`/`while` 内にネストされた `if`/`match`/`block:` 式の中でのみ有効。
    BlockReturnInLoopExpr,
    /// `raise` に `Error` trait を実装していない値を渡した。
    InvalidRaiseType {
        /// `raise` 式の推論型
        got: InferredType,
    },
}

// ---------------------------------------------------------------------------
// StaticTypeError
// ---------------------------------------------------------------------------

/// 静的型検査で検出されたエラー。
///
/// エラーの種別（[`TypeErrorKind`]）とソースコード上の位置情報（[`Span`]）を保持する。
/// 収集された全エラーは `TypeChecker::check` の戻り値として返され、
/// 呼び出し元がまとめて表示する。
#[derive(Debug, Clone)]
pub struct StaticTypeError {
    /// エラーの種別と詳細情報
    pub kind: TypeErrorKind,
    /// エラー発生箇所のソースコード上の位置。
    /// スパン情報が取得できない場合のみ `None`。
    pub span: Option<Span>,
}

impl StaticTypeError {
    /// `IncompatibleComparison` エラーを生成するコンストラクタ。
    ///
    /// # 引数
    /// - `lhs`: 左辺の推論型
    /// - `rhs`: 右辺の推論型
    /// - `op`: 演算子記号（`"<"` など）
    /// - `span`: エラー発生箇所のスパン
    fn incompatible_cmp(
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

    /// `AssignToImmutable` エラーを生成するコンストラクタ。
    ///
    /// # 引数
    /// - `name`: 代入しようとした不変変数の名前
    /// - `span`: エラー発生箇所のスパン
    fn assign_immutable(name: &str, span: Span) -> Self {
        Self {
            kind: TypeErrorKind::AssignToImmutable {
                name: name.to_string(),
            },
            span: Some(span),
        }
    }

    /// ファイル名部分を返す（スパンがない場合は `"<unknown>"`）。
    pub fn file_str(&self) -> String {
        match &self.span {
            Some(span) if span.line != 0 && !span.file.is_empty() => span.file.to_string(),
            _ => "<unknown>".to_string(),
        }
    }

    /// 行:列 文字列を返す（スパンがない場合は `"-"`）。
    pub fn line_col_str(&self) -> String {
        match &self.span {
            Some(span) if span.line != 0 => format!("{}:{}", span.line, span.col),
            _ => "-".to_string(),
        }
    }

    /// エラー種別名を返す（常に `"StaticTypeError"`）。
    pub fn error_type_str(&self) -> &'static str {
        "StaticTypeError"
    }

    /// エラー詳細メッセージを返す（ANSI カラーコード付き、エラー種別プレフィックスなし）。
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
        }
    }
}

/// Wraps `s` in single quotes with magenta+bold ANSI codes: `'value'`
fn hl_q(s: impl std::fmt::Display) -> String {
    format!("'\x1b[1;35m{s}\x1b[0m'")
}

/// Wraps `s` in backticks with magenta+bold ANSI codes: `` `value` ``
fn hl_bt(s: impl std::fmt::Display) -> String {
    format!("`\x1b[1;35m{s}\x1b[0m`")
}

impl std::fmt::Display for StaticTypeError {
    /// `StaticTypeError` を人間が読めるエラーメッセージに変換する。
    ///
    /// フォーマット: `ファイル名:行:列: StaticTypeError: <エラー内容>`
    /// スパン情報がない場合は `<unknown>:` でプレフィックスされる。
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

// ---------------------------------------------------------------------------
// Type environment
// ---------------------------------------------------------------------------

/// スコープ内の変数情報。
///
/// 各スコープは `HashMap<String, VarInfo>` として管理され、
/// [`TypeChecker::scope_stack`] の末尾要素が現在の最内スコープとなる。
struct VarInfo {
    /// 変数の静的推論型
    ty: InferredType,
    /// `true` なら可変（`mut`）、`false` なら不変（`let` / `const`）
    mutable: bool,
}

// ---------------------------------------------------------------------------
// TypeChecker
// ---------------------------------------------------------------------------

/// 静的型検査器。
///
/// プログラム全体の AST を走査し、型エラーを収集する。
/// エントリポイントは [`TypeChecker::check`]。
/// 検査はパース後・インタープリタ実行前に行われ、エラーが 1 件以上あれば
/// 全エラーを表示してプログラムを終了する（終了処理は呼び出し元が担う）。
pub struct TypeChecker {
    /// スコープのスタック。末尾要素が最内スコープ（現在のスコープ）。
    /// 変数の検索は末尾から先頭方向（内側から外側）に行う。
    scope_stack: Vec<HashMap<String, VarInfo>>,
    /// 関数名ごとのシグネチャ一覧。オーバーロードがある場合は複数エントリを持つ。
    /// 前方参照に対応するため前処理パスで収集される。
    fn_sigs: HashMap<String, Vec<FnSig>>,
    /// クラス名 → メソッド名 → シグネチャ一覧のマップ。
    /// `Self` 型パラメータの検査に使用する。
    class_method_sigs: HashMap<String, HashMap<String, Vec<FnSig>>>,
    /// 型検査時点で既知のクラス名および new_type 名のセット。
    /// コンストラクタ呼び出しを [`InferredType::NamedInstance`] として解決するために使用する。
    known_class_names: HashSet<String>,
    /// `new_type` 名 → 元の型名のマップ。`type[T]` 互換性チェックに使用する。
    new_type_originals: HashMap<String, String>,
    /// クラス名 → 基底クラス・トレイト名リストのマップ。`type[Trait]` 互換性チェックに使用する。
    class_bases: HashMap<String, Vec<String>>,
    /// クラス名 → (フィールド名 → 可変フラグ) のマップ。`let` フィールドへの代入を静的検査するために使用する。
    class_fields: HashMap<String, HashMap<String, bool>>,
    /// クラス名 → (メンバ名 → アクセス可能性) のマップ。`private`/`protected` メンバへのアクセス静的検査に使用する。
    /// デフォルト（`public`）のメンバは格納しない。
    class_member_access: HashMap<String, HashMap<String, Accessibility>>,
    /// クラス名 → static メソッド名集合のマップ。インスタンスからの static メソッド呼び出しを静的検査するために使用する。
    class_static_methods: HashMap<String, HashSet<String>>,
    /// 現在型検査中の関数名。`__init__` 内での `let` フィールド初期化を許可するために使用する。
    current_fn_name: Option<String>,
    /// 現在型検査中のクラス名。クラス本体内での `self.<field>` 代入検査に使用する。
    current_class_name: Option<String>,
    /// `for`/`while` 式の直接本体内にいる深さ。
    /// 正値のとき `block_return` は静的エラーとなる。
    /// `if`/`match`/`block:` 式に入ると 0 にリセットされ、関数定義でも 0 にリセットされる。
    block_return_forbidden_depth: usize,
    /// 収集された型エラーのリスト。
    pub errors: Vec<StaticTypeError>,
}

impl TypeChecker {
    /// 型検査器を初期化する。
    ///
    /// グローバルスコープに組み込み型名（`int`, `str`, `float`, `bool`, `Any`）を
    /// [`InferredType::TypeVal`] として事前登録する。
    pub fn new() -> Self {
        let mut global: HashMap<String, VarInfo> = HashMap::new();
        // 組み込み型名を TypeValOf として事前登録し、式コンテキストで認識できるようにする。
        let builtins: &[(&str, InferredType)] = &[
            ("int", InferredType::Int),
            ("float", InferredType::Float),
            ("str", InferredType::Str),
            ("bool", InferredType::Bool),
            ("Any", InferredType::Any),
            (
                "function",
                InferredType::Function {
                    params: None,
                    return_type: Box::new(InferredType::Any),
                },
            ),
        ];
        for (name, inner) in builtins {
            global.insert(
                name.to_string(),
                VarInfo {
                    ty: InferredType::TypeValOf(Box::new(inner.clone())),
                    mutable: false,
                },
            );
        }
        // 組み込み new_type: path (str), Index (int), Size (int)
        let mut known_class_names: HashSet<String> = HashSet::new();
        let mut new_type_originals: HashMap<String, String> = HashMap::new();
        for (cls_name, prim_type) in [("path", "str"), ("Index", "int"), ("Size", "int")] {
            known_class_names.insert(cls_name.to_string());
            new_type_originals.insert(cls_name.to_string(), prim_type.to_string());
        }
        // 組み込み slice 型: `slice(...)` 呼び出しが NamedInstance("slice") として推論されるよう登録
        known_class_names.insert("slice".to_string());
        // 組み込み定数: begin / last は Index インスタンス
        for name in ["begin", "last"] {
            global.insert(
                name.to_string(),
                VarInfo {
                    ty: InferredType::NamedInstance("Index".to_string()),
                    mutable: false,
                },
            );
        }
        // 組み込み Error trait と標準例外クラスを静的環境にも登録する。
        global.insert(
            "Error".to_string(),
            VarInfo {
                ty: InferredType::TypeValOf(Box::new(InferredType::NamedInstance(
                    "Error".to_string(),
                ))),
                mutable: false,
            },
        );
        let exception_names = [
            "Exception",
            "ValueError",
            "TypeError",
            "NameError",
            "AttributeError",
            "IndexError",
            "KeyError",
            "ZeroDivisionError",
            "RuntimeError",
            "StopIteration",
            "NotImplementedError",
            "OverflowError",
            "IOError",
            "OSError",
            "AssertionError",
            "ArithmeticError",
            "AccessError",
        ];
        for class_name in exception_names {
            known_class_names.insert(class_name.to_string());
            global.insert(
                class_name.to_string(),
                VarInfo {
                    ty: InferredType::TypeValOf(Box::new(InferredType::NamedInstance(
                        class_name.to_string(),
                    ))),
                    mutable: false,
                },
            );
        }
        let mut class_bases: HashMap<String, Vec<String>> = HashMap::new();
        for class_name in exception_names {
            class_bases.insert(class_name.to_string(), vec!["Error".to_string()]);
        }
        Self {
            scope_stack: vec![global],
            fn_sigs: HashMap::new(),
            class_method_sigs: HashMap::new(),
            known_class_names,
            new_type_originals,
            class_bases,
            class_fields: HashMap::new(),
            class_member_access: HashMap::new(),
            class_static_methods: HashMap::new(),
            current_fn_name: None,
            current_class_name: None,
            block_return_forbidden_depth: 0,
            errors: Vec::new(),
        }
    }

    /// プログラム全体の静的型検査を実行し、収集されたエラーを返す。
    ///
    /// 内部では前処理パス（[`collect_fn_sigs`]）を先に実行して関数シグネチャを収集し、
    /// 前方参照がある場合でも呼び出し検査が正しく行えるようにする。
    ///
    /// # 引数
    /// - `stmts`: プログラムのトップレベル文のスライス
    ///
    /// # 戻り値
    /// 検出されたすべての [`StaticTypeError`] のリスト。エラーがなければ空 `Vec`。
    pub fn check(stmts: &[Stmt]) -> Vec<StaticTypeError> {
        let mut tc = Self::new();
        tc.collect_fn_sigs(stmts);
        tc.check_stmts(stmts);
        tc.errors
    }

    /// 前処理パス: 全関数・クラス・new_type のシグネチャを収集する。
    ///
    /// 呼び出しが定義より前に出現する前方参照ケースでも型検査を行えるよう、
    /// 実際の検査（[`check_stmts`]）より先に実行する。
    ///
    /// 2 パスで処理する:
    /// 1. `FnDef` / `ClassDef` / `TraitDef` のシグネチャを収集
    /// 2. `NewTypeDef` で定義された型エイリアスが元クラスのメソッドシグネチャを継承
    ///
    /// # 副作用
    /// - `self.fn_sigs` に関数シグネチャを追加する
    /// - `self.class_method_sigs` にクラスメソッドシグネチャを追加する
    /// - `self.known_class_names` にクラス名・new_type 名を追加する
    fn collect_fn_sigs(&mut self, stmts: &[Stmt]) {
        // パス 1: 関数・クラス・trait のシグネチャを収集する。
        for stmt in stmts {
            match stmt {
                Stmt::FnDef {
                    name,
                    params,
                    return_type,
                    body,
                    ..
                } => {
                    let sig = FnSig {
                        params: params
                            .iter()
                            .map(|p| {
                                (
                                    p.name.clone(),
                                    p.type_ann.as_deref().and_then(InferredType::from_ann),
                                )
                            })
                            .collect(),
                        required_count: params.iter().filter(|p| p.default.is_none()).count(),
                        return_type: return_type.as_deref().and_then(InferredType::from_ann),
                    };
                    self.fn_sigs.entry(name.clone()).or_default().push(sig);
                    // ネストした関数定義も再帰的に収集する。
                    self.collect_fn_sigs(body);
                }
                Stmt::ClassDef {
                    name, bases, body, ..
                } => {
                    self.known_class_names.insert(name.clone());
                    self.class_bases.insert(name.clone(), bases.clone());
                    // クラスメソッドのシグネチャを収集（Self 型検査に使用）。
                    let mut cls_methods: HashMap<String, Vec<FnSig>> = HashMap::new();
                    for s in body.iter() {
                        if let Stmt::FnDef {
                            name: mname,
                            template_params,
                            params,
                            return_type,
                            ..
                        } = s
                        {
                            // `__cast__[TypeName]` メソッドはキャスト専用のキー名で格納する。
                            let storage_name = if mname == "__cast__" && !template_params.is_empty()
                            {
                                format!("__cast__[{}]", template_params[0].name)
                            } else {
                                mname.clone()
                            };
                            let sig = FnSig {
                                params: params
                                    .iter()
                                    .map(|p| {
                                        (
                                            p.name.clone(),
                                            p.type_ann.as_deref().and_then(InferredType::from_ann),
                                        )
                                    })
                                    .collect(),
                                required_count: params
                                    .iter()
                                    .filter(|p| p.default.is_none())
                                    .count(),
                                return_type: return_type
                                    .as_deref()
                                    .and_then(InferredType::from_ann),
                            };
                            cls_methods.entry(storage_name).or_default().push(sig);
                        }
                    }
                    self.class_method_sigs.insert(name.clone(), cls_methods);
                    // フィールド名 → 可変フラグ のマップと、メンバ名 → アクセス可能性 のマップを収集する。
                    let mut fields: HashMap<String, bool> = HashMap::new();
                    let mut member_access: HashMap<String, Accessibility> = HashMap::new();
                    let mut static_methods: HashSet<String> = HashSet::new();
                    for s in body.iter() {
                        match s {
                            Stmt::Field {
                                name: fname,
                                kind,
                                access,
                                ..
                            } => {
                                let mutable = matches!(kind, crate::ast::FieldKind::Mut);
                                fields.insert(fname.clone(), mutable);
                                if *access != Accessibility::Public {
                                    member_access.insert(fname.clone(), access.clone());
                                }
                            }
                            Stmt::FnDef {
                                name: mname,
                                is_static,
                                access,
                                ..
                            } => {
                                if *access != Accessibility::Public {
                                    member_access.insert(mname.clone(), access.clone());
                                }
                                if *is_static {
                                    static_methods.insert(mname.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                    self.class_fields.insert(name.clone(), fields);
                    self.class_member_access.insert(name.clone(), member_access);
                    if !static_methods.is_empty() {
                        self.class_static_methods
                            .insert(name.clone(), static_methods);
                    }
                    self.collect_fn_sigs(body);
                }
                Stmt::EnumDef { name, .. } => {
                    // enum Name は class Name と enum_item_Name の両方を既知クラスとして登録する。
                    self.known_class_names.insert(name.clone());
                    let item_type_name = format!("enum_item_{}", name);
                    self.known_class_names.insert(item_type_name);
                }
                Stmt::TraitDef { body, .. } => self.collect_fn_sigs(body),
                Stmt::Match { arms, .. } => {
                    for arm in arms {
                        self.collect_fn_sigs(&arm.body);
                    }
                }
                Stmt::If {
                    branches,
                    else_body,
                } => {
                    for (_, body) in branches {
                        self.collect_fn_sigs(body);
                    }
                    if let Some(body) = else_body {
                        self.collect_fn_sigs(body);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Block(body) => {
                    self.collect_fn_sigs(body);
                }
                _ => {}
            }
        }
        // パス 2: new_type エイリアスが元クラスのメソッドシグネチャを継承する。
        for stmt in stmts {
            if let Stmt::NewTypeDef { name, original } = stmt {
                self.known_class_names.insert(name.clone());
                self.new_type_originals
                    .insert(name.clone(), original.clone());
                if let Some(orig_sigs) = self.class_method_sigs.get(original).cloned() {
                    self.class_method_sigs.insert(name.clone(), orig_sigs);
                }
            }
        }
    }

    // --- スコープ操作ヘルパー ---

    /// 新しい内部スコープをスタックにプッシュする。
    ///
    /// `if` / `while` / `for` / `block` / 関数定義などのブロックに入る際に呼び出す。
    fn push_scope(&mut self) {
        self.scope_stack.push(HashMap::new());
    }

    /// 現在の内部スコープをスタックからポップする。
    ///
    /// グローバルスコープ（スタック先頭）は削除しない。
    /// ブロックを抜ける際に [`push_scope`] と対で呼び出す。
    fn pop_scope(&mut self) {
        if self.scope_stack.len() > 1 {
            self.scope_stack.pop();
        }
    }

    /// 現在のスコープに変数を宣言する。
    ///
    /// # 引数
    /// - `name`: 変数名
    /// - `ty`: 変数の静的推論型
    /// - `mutable`: `true` なら可変（`mut`）、`false` なら不変（`let` / `const`）
    fn declare(&mut self, name: String, ty: InferredType, mutable: bool) {
        self.scope_stack
            .last_mut()
            .unwrap()
            .insert(name, VarInfo { ty, mutable });
    }

    /// スコープチェーンを内側から外側に向かって変数を検索する。
    ///
    /// # 引数
    /// - `name`: 検索する変数名
    ///
    /// # 戻り値
    /// 見つかった変数の [`VarInfo`] への参照。スコープ内に存在しなければ `None`。
    fn lookup(&self, name: &str) -> Option<&VarInfo> {
        self.scope_stack.iter().rev().find_map(|s| s.get(name))
    }

    /// `class_name` のフィールド `member_name` へのアクセスが現在のコンテキストで許可されているか検査する。
    ///
    /// メソッドへのアクセスはランタイムが強制しないため、フィールドのみを対象とする。
    /// - `private`: 同じクラス内からのみ許可。
    /// - `protected`: 同じクラスまたは派生クラスからのみ許可。
    fn check_member_access_static(
        &mut self,
        class_name: &str,
        member_name: &str,
        span: Option<Span>,
    ) {
        // フィールドのみ検査する（メソッドへのアクセスはランタイムでも強制されない）。
        let is_field = self
            .class_fields
            .get(class_name)
            .map(|f| f.contains_key(member_name))
            .unwrap_or(false);
        if !is_field {
            return;
        }

        let access = self
            .class_member_access
            .get(class_name)
            .and_then(|m| m.get(member_name))
            .cloned()
            .unwrap_or(Accessibility::Public);
        match access {
            Accessibility::Public => {}
            Accessibility::Private => {
                if self.current_class_name.as_deref() == Some(class_name) {
                    return;
                }
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::PrivateAccessError {
                        member_name: member_name.to_string(),
                        class_name: class_name.to_string(),
                    },
                    span,
                });
            }
            Accessibility::Protected => {
                if let Some(cur) = self.current_class_name.clone() {
                    if cur == class_name {
                        return;
                    }
                    if self
                        .class_bases
                        .get(&cur)
                        .map(|b| b.contains(&class_name.to_string()))
                        .unwrap_or(false)
                    {
                        return;
                    }
                }
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::ProtectedAccessError {
                        member_name: member_name.to_string(),
                        class_name: class_name.to_string(),
                    },
                    span,
                });
            }
        }
    }

    /// `obj.attr = val` のとき `attr` が `let` フィールドであれば `AssignToImmutableField` エラーを記録する。
    ///
    /// `__init__` メソッド内で `self.attr` に代入する場合（フィールド初期化）はエラーとしない。
    fn check_immutable_field_assign(&mut self, target: &Expr) {
        if let Expr::Attr { object, attr, span } = target {
            // `__init__` 内の `self.<field>` への代入はフィールド初期化として許可する。
            let is_self_in_init = matches!(object.as_ref(), Expr::Ident(n) if n == "self")
                && self.current_fn_name.as_deref() == Some("__init__");
            if is_self_in_init {
                return;
            }
            // クラス名を決定する:
            // (a) `self.<field>` の場合は current_class_name から取得する。
            // (b) 型が NamedInstance として解決された場合はそこから取得する。
            let class_name_opt: Option<String> = if matches!(object.as_ref(), Expr::Ident(n) if n == "self")
            {
                self.current_class_name.clone()
            } else {
                let obj_ty = self.infer(object);
                if let InferredType::NamedInstance(cls) = obj_ty {
                    Some(cls)
                } else {
                    None
                }
            };
            if let Some(class_name) = class_name_opt {
                if let Some(fields) = self.class_fields.get(&class_name) {
                    if fields.get(attr.as_str()) == Some(&false) {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::AssignToImmutableField {
                                field_name: attr.clone(),
                                class_name,
                            },
                            span: Some(span.clone()),
                        });
                    }
                }
            }
        }
    }

    /// サブスクリプトチェーン `x[i][j]...` のルート識別子名を返す。
    /// `Expr::Subscript` でない式（属性アクセスなど）は `None`。
    fn subscript_root_ident(expr: &Expr) -> Option<&str> {
        match expr {
            Expr::Ident(name) => Some(name.as_str()),
            Expr::Subscript { object, .. } => Self::subscript_root_ident(object),
            _ => None,
        }
    }

    /// 型エラーを収集リストに追加する。
    ///
    /// # 引数
    /// - `err`: 追加する [`StaticTypeError`]
    ///
    /// # 副作用
    /// `self.errors` ベクタに `err` を追記する。
    fn report_error(&mut self, err: StaticTypeError) {
        self.errors.push(err);
    }

    // --- 文の型検査 ---

    /// 文のスライスを順番に型検査する。
    ///
    /// # 引数
    /// - `stmts`: 検査対象の文のスライス
    ///
    /// # 副作用
    /// 各文で検出されたエラーが `self.errors` に追記される。
    fn check_stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.check_stmt(stmt);
        }
    }

    /// 1 つの文を型検査する。
    ///
    /// 各文の種別に応じて変数宣言・代入・スコープ管理・型アノテーション検査を行う。
    /// エラーが発生した場合は即時中断せず `self.errors` に追記して続行する。
    ///
    /// # 引数
    /// - `stmt`: 検査対象の文
    ///
    /// # 副作用
    /// 検出されたエラーが `self.errors` に追記される。
    /// スコープの変数テーブル（`self.scope_stack`）が更新される場合がある。
    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            // --- 変数宣言 ---
            Stmt::Let(name, expr) => {
                // let 宣言: 式を推論し、不変変数としてスコープに登録する。
                let ty = self.infer(expr);
                self.declare(name.clone(), ty, false);
            }
            Stmt::Const(name, expr) => {
                // const 宣言: 式を推論し、不変変数としてスコープに登録する。
                let ty = self.infer(expr);
                self.declare(name.clone(), ty, false);
            }
            Stmt::Mut(name, expr) => {
                // mut 宣言: 式を推論し、可変変数としてスコープに登録する。
                let ty = self.infer(expr);
                self.declare(name.clone(), ty, true);
            }
            Stmt::Static(name, expr, _) => {
                // static mut 宣言: mut と同様に可変変数として登録する。
                let ty = self.infer(expr);
                self.declare(name.clone(), ty, true);
            }
            Stmt::LetTuple {
                targets,
                value,
                span,
            } => {
                let rhs_ty = self.infer(value);

                // Check each target for missing qualifier
                for target in targets.iter() {
                    if let TupleTarget::Bare(name) = target {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::TupleUnpackMissingQualifier { name: name.clone() },
                            span: Some(span.clone()),
                        });
                    }
                }

                // Arity check when the RHS is a known-length tuple type
                if let InferredType::Tuple(ref elem_types) = rhs_ty {
                    let has_wildcard = targets.iter().any(|t| matches!(t, TupleTarget::Wildcard));
                    let named = targets
                        .iter()
                        .filter(|t| !matches!(t, TupleTarget::Wildcard))
                        .count();
                    let tlen = elem_types.len();
                    let bad = if has_wildcard {
                        named > tlen
                    } else {
                        named != tlen
                    };
                    if bad {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::TupleUnpackArityMismatch {
                                tuple_len: tlen,
                                target_count: named,
                                has_wildcard,
                            },
                            span: Some(span.clone()),
                        });
                    }
                }

                // Declare each named target variable
                let elem_types = if let InferredType::Tuple(ref v) = rhs_ty {
                    v.clone()
                } else {
                    vec![]
                };
                for (i, target) in targets.iter().enumerate() {
                    let ty = elem_types.get(i).cloned().unwrap_or(InferredType::Any);
                    match target {
                        TupleTarget::Let(name) | TupleTarget::Bare(name) => {
                            self.declare(name.clone(), ty, false)
                        }
                        TupleTarget::Mut(name) => self.declare(name.clone(), ty, true),
                        TupleTarget::Wildcard => {}
                    }
                }
            }

            // --- 代入 ---
            Stmt::Assign { name, value, span } => {
                // 不変変数への代入はエラーとして記録する。
                if let Some(info) = self.lookup(name) {
                    if !info.mutable {
                        self.report_error(StaticTypeError::assign_immutable(name, span.clone()));
                    }
                }
                self.infer(value);
            }
            Stmt::CompoundAssign {
                name,
                op: _,
                value,
                span,
            } => {
                // 複合代入（`+=` など）でも不変変数への代入はエラーとして記録する。
                if let Some(info) = self.lookup(name) {
                    if !info.mutable {
                        self.report_error(StaticTypeError::assign_immutable(name, span.clone()));
                    }
                }
                self.infer(value);
            }
            Stmt::AttrAssign { target, value } => {
                // サブスクリプト代入（`x[i] = v`）のとき、ルート変数が let なら静的エラー。
                if matches!(target, Expr::Subscript { .. }) {
                    if let Some(name) = Self::subscript_root_ident(target) {
                        if let Some(info) = self.lookup(name) {
                            if !info.mutable {
                                self.report_error(StaticTypeError {
                                    kind: TypeErrorKind::AssignToImmutable {
                                        name: name.to_string(),
                                    },
                                    span: None,
                                });
                            }
                        }
                    }
                }
                // `obj.attr = v` のとき、attr が let フィールドなら静的エラー（__init__ 内の self への初期化は除く）。
                self.check_immutable_field_assign(target);
                self.infer(target);
                self.infer(value);
            }
            Stmt::AttrCompoundAssign {
                target,
                op: _,
                value,
            } => {
                // サブスクリプト複合代入（`x[i] += v`）のとき、ルート変数が let なら静的エラー。
                if matches!(target, Expr::Subscript { .. }) {
                    if let Some(name) = Self::subscript_root_ident(target) {
                        if let Some(info) = self.lookup(name) {
                            if !info.mutable {
                                self.report_error(StaticTypeError {
                                    kind: TypeErrorKind::AssignToImmutable {
                                        name: name.to_string(),
                                    },
                                    span: None,
                                });
                            }
                        }
                    }
                }
                // `obj.attr += v` のとき、attr が let フィールドなら静的エラー。
                self.check_immutable_field_assign(target);
                self.infer(target);
                self.infer(value);
            }

            // --- 式文 ---
            Stmt::Expr(expr) => {
                // 式文: 式を推論して副作用（エラー収集）だけ利用する。
                self.infer(expr);
            }

            // --- 制御構文 ---
            Stmt::If {
                branches,
                else_body,
            } => {
                // if/elif/else: 各分岐で独立したスコープを生成する。
                // 条件式が型ガード（`x is T` / `x is not T`）の場合、
                // 分岐本体内では変数の型を絞り込む（type narrowing）。
                for (cond, body) in branches {
                    // 型ガード情報を条件式 AST から取り出す（借用なし）。
                    let guard_opt: Option<(String, String, bool, Span)> = if let Expr::IsType {
                        expr,
                        type_name,
                        negated,
                        span,
                    } = cond
                    {
                        if let Expr::Ident(var_name) = expr.as_ref() {
                            Some((var_name.clone(), type_name.clone(), *negated, span.clone()))
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    // 絞り込み後の型と、エラー情報を計算する（`self.lookup` による不変借用はここで完了）。
                    // narrowed: (変数名, 絞り込み後の型, 可変フラグ)
                    // error_info: (変数名, 変数の元の型, スパン) — `is not` 非 Union 使用時に報告
                    let (narrowed, error_info): (
                        Option<(String, InferredType, bool)>,
                        Option<(String, InferredType, Span)>,
                    ) = match &guard_opt {
                        None => (None, None),
                        Some((var_name, type_name, negated, span)) => {
                            let guard_ty = Self::type_from_guard_name(type_name);
                            let (var_ty, is_mut) = self
                                .lookup(var_name)
                                .map(|v| (v.ty.clone(), v.mutable))
                                .unwrap_or((InferredType::Unresolved, false));

                            if *negated {
                                // `x is not T`: x は Union / Optional でなければならない。
                                match &var_ty {
                                    InferredType::Union(types) => {
                                        // Union からガード型を除いた残りの型を求める。
                                        let remaining: Vec<InferredType> = types
                                            .iter()
                                            .filter(|t| **t != guard_ty)
                                            .cloned()
                                            .collect();
                                        let narrowed_ty = match remaining.len() {
                                            0 => InferredType::Unresolved,
                                            1 => remaining.into_iter().next().unwrap(),
                                            _ => InferredType::Union(remaining),
                                        };
                                        (Some((var_name.clone(), narrowed_ty, is_mut)), None)
                                    }
                                    InferredType::Unresolved => (None, None),
                                    _ => {
                                        // Union でも Unresolved でもない型 → エラー
                                        (
                                            None,
                                            Some((var_name.clone(), var_ty.clone(), span.clone())),
                                        )
                                    }
                                }
                            } else {
                                // `x is T`: x の型をガード型に絞り込む。
                                (Some((var_name.clone(), guard_ty, is_mut)), None)
                            }
                        }
                    };

                    self.infer(cond);

                    // `is not` 非 Union エラーを報告する。
                    if let Some((var_name, var_type, span)) = error_info {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::IsNotOnNonUnion { var_name, var_type },
                            span: Some(span),
                        });
                    }

                    // 分岐本体を新しいスコープで検査し、必要なら変数型を絞り込む。
                    self.push_scope();
                    if let Some((var_name, narrowed_ty, is_mut)) = narrowed {
                        self.declare(var_name, narrowed_ty, is_mut);
                    }
                    self.check_stmts(body);
                    self.pop_scope();
                }
                if let Some(body) = else_body {
                    self.push_scope();
                    self.check_stmts(body);
                    self.pop_scope();
                }
            }
            Stmt::Match { subject, arms, .. } => {
                // match: subject を推論し、各アームを独立したスコープで検査する。
                let subject_ty = self.infer(subject);
                // subject が単純な変数参照かどうかを取得（`is` アームでの型絞り込み用）
                let subject_name: Option<String> = if let Expr::Ident(n) = subject {
                    Some(n.clone())
                } else {
                    None
                };
                for arm in arms {
                    self.push_scope();
                    match &arm.pattern {
                        MatchPattern::Case(expr) => {
                            self.infer(expr);
                        }
                        MatchPattern::IsType(type_name) => {
                            // `is` アーム: subject が変数なら、アームのスコープ内で型を絞り込む
                            if let Some(ref var_name) = subject_name {
                                let narrowed = Self::type_from_guard_name(type_name);
                                let is_mut =
                                    self.lookup(var_name).map(|v| v.mutable).unwrap_or(false);
                                self.declare(var_name.clone(), narrowed, is_mut);
                            }
                            let _ = subject_ty.clone(); // suppress unused warning
                        }
                    }
                    self.check_stmts(&arm.body);
                    self.pop_scope();
                }
            }
            Stmt::While { cond, body } => {
                // while: 条件式を推論し、ループ本体を独立したスコープで検査する。
                self.infer(cond);
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::For {
                targets,
                iter,
                body,
            } => {
                // for...in: イテレータ式を推論し、ループ変数を Unresolved として宣言する。
                // コレクション要素型のトラッキングは未実装のため実行時に委ねる。
                self.infer(iter);
                self.push_scope();
                for t in targets {
                    self.declare(t.clone(), InferredType::Unresolved, true);
                }
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::Block(body) => {
                // block: 独立したスコープで本体を検査する。
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
            }

            // --- 関数定義 ---
            Stmt::FnDef {
                name,
                params,
                return_type,
                body,
                decorators,
                ..
            } => {
                // デコレータの型シグネチャを検査する（関数デコレータなので is_fn_target=true）。
                for dec in decorators {
                    self.check_decorator(dec, true, name);
                }
                // パラメータの型アノテーション欠如を検査する（`self` は除外）。
                for param in params.iter() {
                    if param.name == "self" {
                        continue;
                    }
                    if param.type_ann.is_none() {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::MissingParamTypeAnn {
                                func_name: name.clone(),
                                param_name: param.name.clone(),
                            },
                            span: None,
                        });
                    }
                }
                // 戻り値型アノテーション欠如を検査する。
                if return_type.is_none() {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::MissingReturnTypeAnn {
                            func_name: name.clone(),
                        },
                        span: None,
                    });
                }
                // 関数名を Unresolved（不変）としてスコープに登録し、本体を検査する。
                self.declare(name.clone(), InferredType::Unresolved, false);
                self.push_scope();
                for param in params {
                    let ty = if param.name == "self" {
                        // クラスメソッド内の `self` はクラスのインスタンス型として宣言する。
                        self.current_class_name
                            .as_ref()
                            .map(|c| InferredType::NamedInstance(c.clone()))
                            .unwrap_or(InferredType::Unresolved)
                    } else {
                        param
                            .type_ann
                            .as_deref()
                            .and_then(InferredType::from_ann)
                            .unwrap_or(InferredType::Unresolved)
                    };
                    self.declare(param.name.clone(), ty, param.mutable);
                }
                let prev_fn = self.current_fn_name.take();
                self.current_fn_name = Some(name.clone());
                let saved_depth = self.block_return_forbidden_depth;
                self.block_return_forbidden_depth = 0;
                self.check_stmts(body);
                self.block_return_forbidden_depth = saved_depth;
                self.current_fn_name = prev_fn;
                self.pop_scope();
            }

            // --- クラス・trait 定義 ---
            Stmt::ClassDef {
                name,
                body,
                decorators,
                ..
            } => {
                // デコレータの型シグネチャを検査する（クラスデコレータなので is_fn_target=false）。
                for dec in decorators {
                    self.check_decorator(dec, false, name);
                }
                // クラス名を TypeValOf(NamedInstance) としてスコープに登録し、本体を独立スコープで検査する。
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
                self.push_scope();
                let prev_class = self.current_class_name.replace(name.clone());
                self.check_stmts(body);
                self.current_class_name = prev_class;
                self.pop_scope();
            }
            Stmt::TraitDef { name, body, .. } => {
                // trait 名を TypeValOf(NamedInstance) としてスコープに登録し、本体を独立スコープで検査する。
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
            }

            // --- ジャンプ文 ---
            Stmt::Return(expr) => {
                // return 文: 戻り値式を推論する（戻り値型の整合性検査は未実装）。
                if let Some(e) = expr {
                    self.infer(e);
                }
            }
            Stmt::BlockReturn(expr, span) => {
                if self.block_return_forbidden_depth > 0 {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::BlockReturnInLoopExpr,
                        span: Some(span.clone()),
                    });
                }
                self.infer(expr);
            }
            Stmt::LoopYield(expr) | Stmt::Yield(expr) => {
                self.infer(expr);
            }

            // --- クラスフィールド宣言 ---
            Stmt::Field {
                name,
                kind,
                type_ann,
                default,
                ..
            } => {
                // フィールドを型アノテーションに基づいてスコープに登録する。
                // FieldKind::Mut のみ可変として扱う（Let / Const は不変）。
                let ty = InferredType::from_ann(type_ann).unwrap_or(InferredType::Unresolved);
                if let Some(expr) = default {
                    self.infer(expr);
                }
                let mutable = matches!(kind, crate::ast::FieldKind::Mut);
                self.declare(name.clone(), ty, mutable);
            }

            // --- ジェネレータ関数定義 ---
            Stmt::GenDef {
                name,
                params,
                yield_type,
                body,
                ..
            } => {
                // パラメータの型アノテーション欠如を検査する（`self` は除外）。
                for param in params.iter() {
                    if param.name == "self" {
                        continue;
                    }
                    if param.type_ann.is_none() {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::MissingParamTypeAnn {
                                func_name: name.clone(),
                                param_name: param.name.clone(),
                            },
                            span: None,
                        });
                    }
                }
                // yield 型アノテーション欠如を戻り値型アノテーション欠如として検査する。
                if yield_type.is_none() {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::MissingReturnTypeAnn {
                            func_name: name.clone(),
                        },
                        span: None,
                    });
                }
                self.declare(name.clone(), InferredType::Unresolved, false);
                self.push_scope();
                for param in params {
                    let ty = param
                        .type_ann
                        .as_deref()
                        .and_then(InferredType::from_ann)
                        .unwrap_or(InferredType::Unresolved);
                    self.declare(param.name.clone(), ty, param.mutable);
                }
                let saved_depth = self.block_return_forbidden_depth;
                self.block_return_forbidden_depth = 0;
                self.check_stmts(body);
                self.block_return_forbidden_depth = saved_depth;
                self.pop_scope();
            }

            // --- new_type 定義 ---
            Stmt::NewTypeDef { name, .. } => {
                // new_type バインドは常に const（パーサーが再代入を禁止）。
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
            }

            // --- enum 定義 ---
            Stmt::EnumDef { name, .. } => {
                // enum Name → クラス Name と enum_item_Name の両方をスコープに登録する。
                let item_type_name = format!("enum_item_{}", name);
                self.declare(
                    item_type_name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(item_type_name))),
                    false,
                );
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
            }

            // --- 副作用のない文 ---
            Stmt::Pass
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Freeze(..)
            | Stmt::BreakPoint { .. }
            | Stmt::DebugLet(..) => {}

            // --- 例外処理 ---
            Stmt::Try {
                body,
                handlers,
                finally_body,
            } => {
                // try ブロック本体を独立スコープで検査する。
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
                // 各 except ハンドラを独立スコープで検査する。
                // 補足した例外がバインドされる変数は Unresolved として宣言する。
                for handler in handlers {
                    self.push_scope();
                    if let Some(name) = &handler.name {
                        self.declare(name.clone(), InferredType::Unresolved, true);
                    }
                    self.check_stmts(&handler.body);
                    self.pop_scope();
                }
                // finally ブロックを独立スコープで検査する。
                if let Some(fb) = finally_body {
                    self.push_scope();
                    self.check_stmts(fb);
                    self.pop_scope();
                }
            }
            Stmt::Raise { exc, span } => {
                // raise 文: Error trait 実装インスタンスだけを送出できる。
                if let Some(e) = exc {
                    let ty = self.infer(e);
                    if !self.is_error_instance_type(&ty) {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::InvalidRaiseType { got: ty },
                            span: Some(span.clone()),
                        });
                    }
                }
            }

            // --- import ---
            Stmt::Import {
                module,
                alias,
                body,
                ..
            } => {
                // Python モジュールの body を走査してメンバ型を収集し、
                // InferredType::Namespace としてスコープに登録する。
                // Python コード内部の型検査は行わない（collect_module_types のみ）。
                let member_types = self.collect_module_types(body);
                let bind_name = alias
                    .clone()
                    .unwrap_or_else(|| module.last().unwrap().clone());
                self.declare(bind_name, InferredType::Namespace(member_types), false);
            }

            Stmt::FromImport { names, body, .. } => {
                // モジュールのメンバ型を収集し、各名前を直接スコープに登録する。
                let member_types = self.collect_module_types(body);
                for (orig_name, alias) in names {
                    let bind_name = alias.clone().unwrap_or_else(|| orig_name.clone());
                    let ty = member_types
                        .get(orig_name.as_str())
                        .cloned()
                        .unwrap_or(InferredType::Unresolved);
                    self.declare(bind_name, ty, false);
                }
            }

            Stmt::AsyncAssign { stmts, .. } => {
                self.push_scope();
                self.check_stmts(stmts);
                self.pop_scope();
            }
        }
    }

    /// モジュールの tl AST を浅くスキャンして「名前 → 型」マップを返す。
    /// Python コード本体は型検査しない（クラス定義・関数定義の宣言のみ収集）。
    fn collect_module_types(
        &self,
        body: &[Stmt],
    ) -> std::collections::HashMap<String, InferredType> {
        let mut map = std::collections::HashMap::new();
        for stmt in body {
            match stmt {
                Stmt::ClassDef { name, .. } => {
                    // クラス定義 → TypeValOf(NamedInstance)（コンストラクタとして使用可能）
                    map.insert(
                        name.clone(),
                        InferredType::TypeValOf(Box::new(InferredType::NamedInstance(
                            name.clone(),
                        ))),
                    );
                }
                Stmt::FnDef {
                    name, return_type, ..
                } => {
                    // 戻り値型アノテーションがあれば推論型に変換する（C++ スタブなど）。
                    let ty = return_type
                        .as_deref()
                        .map(Self::type_ann_to_inferred)
                        .unwrap_or(InferredType::Unresolved);
                    map.insert(name.clone(), ty);
                }
                Stmt::Mut(name, _)
                | Stmt::Let(name, _)
                | Stmt::Const(name, _)
                | Stmt::Static(name, _, _) => {
                    map.insert(name.clone(), InferredType::Unresolved);
                }
                Stmt::LetTuple { targets, .. } => {
                    for t in targets {
                        match t {
                            TupleTarget::Let(n) | TupleTarget::Mut(n) | TupleTarget::Bare(n) => {
                                map.insert(n.clone(), InferredType::Unresolved);
                            }
                            TupleTarget::Wildcard => {}
                        }
                    }
                }
                _ => {}
            }
        }
        map
    }

    /// 型アノテーション文字列（`"int"`, `"float"` など）を `InferredType` に変換する。
    /// C++ ヘッダスタブの戻り値型など、単純なプリミティブ型名のみを処理する。
    fn type_ann_to_inferred(s: &str) -> InferredType {
        match s {
            "int" => InferredType::Int,
            "float" => InferredType::Float,
            "str" => InferredType::Str,
            "bool" => InferredType::Bool,
            "None" => InferredType::None,
            _ => InferredType::Unresolved,
        }
    }

    // --- 式の型推論 ---

    /// 式の静的型を推論して返す。
    ///
    /// 式に副作用（エラー収集）が伴う場合はこのメソッドの中で `report_error` を呼ぶ。
    /// 型が静的に決定できない場合は [`InferredType::Unresolved`] を返し、
    /// 実行時の型検査に委ねる。
    ///
    /// # 引数
    /// - `expr`: 推論対象の式
    ///
    /// # 戻り値
    /// 式の静的推論型。
    ///
    /// # 副作用
    /// 型エラーが検出された場合に `self.errors` へ追記する場合がある。
    fn infer(&mut self, expr: &Expr) -> InferredType {
        match expr {
            // --- リテラル ---
            Expr::Int(_) => InferredType::Int,
            Expr::Float(_) => InferredType::Float,
            Expr::Str(_) => InferredType::Str,
            Expr::Bool(_) => InferredType::Bool,
            Expr::None => InferredType::None,
            Expr::List(elems) => {
                // 全要素の型を推論し、全て同じ型であれば ListOf(T) を返す。
                // 空リストや型が混在する場合は bare List を返す。
                if elems.is_empty() {
                    InferredType::List
                } else {
                    let types: Vec<InferredType> = elems.iter().map(|e| self.infer(e)).collect();
                    let first = &types[0];
                    if *first != InferredType::Unresolved && types.iter().all(|t| t == first) {
                        InferredType::ListOf(Box::new(first.clone()))
                    } else {
                        InferredType::List
                    }
                }
            }
            Expr::Set(elems) => {
                if elems.is_empty() {
                    InferredType::Set
                } else {
                    let types: Vec<InferredType> = elems.iter().map(|e| self.infer(e)).collect();
                    let first = &types[0];
                    if *first != InferredType::Unresolved && types.iter().all(|t| t == first) {
                        InferredType::SetOf(Box::new(first.clone()))
                    } else {
                        InferredType::Set
                    }
                }
            }
            Expr::Tuple(exprs) => {
                // タプルリテラル: 各要素を推論して Tuple 型として返す。
                let types: Vec<InferredType> = exprs.iter().map(|e| self.infer(e)).collect();
                InferredType::Tuple(types)
            }

            // --- 属性アクセス ---
            Expr::Attr { object, attr, span } => {
                // `Any` / `Union` に対する属性アクセスは明示的ダウンキャストが必要。
                let obj_ty = self.infer(object);
                // NamedInstance のとき `private`/`protected` アクセス制御を静的に検査する。
                let class_name_opt = if let InferredType::NamedInstance(cls) = &obj_ty {
                    Some(cls.clone())
                } else {
                    None
                };
                match &obj_ty {
                    InferredType::Any => self.report_error(StaticTypeError {
                        kind: TypeErrorKind::OperationOnAny {
                            op: "attribute access".to_string(),
                        },
                        span: Some(span.clone()),
                    }),
                    InferredType::Union(_) => self.report_error(StaticTypeError {
                        kind: TypeErrorKind::OperationOnUnion {
                            union_type: obj_ty.to_string(),
                            op: "attribute access".to_string(),
                        },
                        span: Some(span.clone()),
                    }),
                    _ => {}
                }
                if let Some(class_name) = class_name_opt {
                    self.check_member_access_static(&class_name, attr, Some(span.clone()));
                }
                // 属性の型は静的に追跡しないため Unresolved を返す。
                InferredType::Unresolved
            }
            Expr::TraitAccess { object, .. } => {
                // trait アクセス（`T::method` 等）: オブジェクト式を推論するのみ。
                self.infer(object);
                InferredType::Unresolved
            }

            // --- 関数呼び出し ---
            Expr::Call { func, args, .. } => self.infer_call(func, args),

            // --- 識別子 ---
            Expr::Ident(name) => {
                // 識別子: スコープから型を取得する。未宣言なら Unresolved（実行時エラー委譲）。
                self.lookup(name)
                    .map(|v| v.ty.clone())
                    .unwrap_or(InferredType::Unresolved)
            }

            // --- 単項演算子 ---
            Expr::UnaryOp { op, operand } => {
                let ty = self.infer(operand);
                let op_str = match op {
                    UnaryOp::Neg => "-",
                    UnaryOp::Not => "not",
                    UnaryOp::BitNot => "~",
                };
                // Any / Union オペランドには明示的ダウンキャストが必要。
                match &ty {
                    InferredType::Any => {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::OperationOnAny {
                                op: op_str.to_string(),
                            },
                            span: None,
                        });
                        return InferredType::Unresolved;
                    }
                    InferredType::Union(_) => {
                        self.report_error(StaticTypeError {
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
                // 演算子ごとに結果型を返す。
                match op {
                    UnaryOp::Not => InferredType::Bool,
                    UnaryOp::Neg => match ty {
                        InferredType::Int => InferredType::Int,
                        InferredType::Float => InferredType::Float,
                        _ => InferredType::Unresolved,
                    },
                    UnaryOp::BitNot => InferredType::Int,
                }
            }

            // --- 二項演算子 ---
            Expr::BinOp {
                op,
                left,
                right,
                span,
            } => {
                // 左右オペランドを推論し、演算の型制約を検査してから結果型を返す。
                let lt = self.infer(left);
                let rt = self.infer(right);
                self.check_binop(op, &lt, &rt, span.clone());
                Self::infer_binop_result(op, &lt, &rt)
            }

            // --- テンプレート実体化 ---
            Expr::TemplateInstantiate { base, .. } => {
                // テンプレート実体化: 制約チェックは実行時に行うため静的検査を委譲する。
                self.infer(base);
                InferredType::Unresolved
            }

            // --- 辞書・サブスクリプト ---
            Expr::Dict(pairs) => {
                if pairs.is_empty() {
                    InferredType::Dict
                } else {
                    let key_types: Vec<InferredType> =
                        pairs.iter().map(|(k, _)| self.infer(k)).collect();
                    let val_types: Vec<InferredType> =
                        pairs.iter().map(|(_, v)| self.infer(v)).collect();
                    let first_k = &key_types[0];
                    let first_v = &val_types[0];
                    if *first_k != InferredType::Unresolved
                        && *first_v != InferredType::Unresolved
                        && key_types.iter().all(|t| t == first_k)
                        && val_types.iter().all(|t| t == first_v)
                    {
                        InferredType::DictOf(Box::new(first_k.clone()), Box::new(first_v.clone()))
                    } else {
                        InferredType::Dict
                    }
                }
            }
            Expr::Subscript { object, index } => {
                // サブスクリプト（`expr[index]`）: 要素型の追跡は未実装のため Unresolved を返す。
                self.infer(object);
                self.infer(index);
                InferredType::Unresolved
            }
            Expr::Slice { begin, end, step } => {
                if let Some(e) = begin {
                    self.infer(e);
                }
                if let Some(e) = end {
                    self.infer(e);
                }
                if let Some(e) = step {
                    self.infer(e);
                }
                InferredType::NamedInstance("slice".to_string())
            }

            // --- 型ガード式 ---
            Expr::IsType { expr, .. } => {
                // 対象式を推論してから Bool を返す。型の絞り込みは Stmt::If 側で行う。
                self.infer(expr);
                InferredType::Bool
            }
            Expr::Block { stmts, return_type } => {
                // block: 式は block_return を吸収する: 外側の for/while 式の直接本体フラグをリセット。
                let saved_depth = self.block_return_forbidden_depth;
                self.block_return_forbidden_depth = 0;
                self.push_scope();
                self.check_stmts(stmts);
                self.pop_scope();
                self.block_return_forbidden_depth = saved_depth;
                if let Some(t) = return_type {
                    InferredType::from_ann(t).unwrap_or(InferredType::Unresolved)
                } else {
                    InferredType::Unresolved
                }
            }
            Expr::IfExpr {
                branches,
                else_body,
                return_type,
            } => {
                // if 式は block_return を吸収する: 外側の for/while 式の直接本体フラグをリセット。
                let saved_depth = self.block_return_forbidden_depth;
                self.block_return_forbidden_depth = 0;
                for (cond, body) in branches {
                    self.infer(cond);
                    self.push_scope();
                    self.check_stmts(body);
                    self.pop_scope();
                }
                if let Some(body) = else_body {
                    self.push_scope();
                    self.check_stmts(body);
                    self.pop_scope();
                }
                self.block_return_forbidden_depth = saved_depth;
                if let Some(t) = return_type {
                    InferredType::from_ann(t).unwrap_or(InferredType::Unresolved)
                } else {
                    InferredType::Unresolved
                }
            }
            Expr::ForExpr {
                iter,
                body,
                return_type,
                ..
            } => {
                self.infer(iter);
                // for 式の直接本体では block_return は禁止。
                self.block_return_forbidden_depth += 1;
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
                self.block_return_forbidden_depth -= 1;
                if let Some(t) = return_type {
                    InferredType::from_ann(t).unwrap_or(InferredType::Unresolved)
                } else {
                    InferredType::Unresolved
                }
            }
            Expr::WhileExpr {
                cond,
                body,
                return_type,
            } => {
                self.infer(cond);
                // while 式の直接本体では block_return は禁止。
                self.block_return_forbidden_depth += 1;
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
                self.block_return_forbidden_depth -= 1;
                if let Some(t) = return_type {
                    InferredType::from_ann(t).unwrap_or(InferredType::Unresolved)
                } else {
                    InferredType::Unresolved
                }
            }
            Expr::MatchExpr {
                subject,
                arms,
                return_type,
            } => {
                // match 式は block_return を吸収する: 外側の for/while 式の直接本体フラグをリセット。
                let saved_depth = self.block_return_forbidden_depth;
                self.block_return_forbidden_depth = 0;
                self.infer(subject);
                for arm in arms {
                    if let crate::ast::MatchPattern::Case(e) = &arm.pattern {
                        self.infer(e);
                    }
                    self.push_scope();
                    self.check_stmts(&arm.body);
                    self.pop_scope();
                }
                self.block_return_forbidden_depth = saved_depth;
                if let Some(t) = return_type {
                    InferredType::from_ann(t).unwrap_or(InferredType::Unresolved)
                } else {
                    InferredType::Unresolved
                }
            }
            // キャスト式: ターゲット型を返す（静的型検査は省略し、実行時に委ねる）。
            Expr::Cast { type_name, .. } => {
                InferredType::from_ann(type_name).unwrap_or(InferredType::Unresolved)
            }
            Expr::DebugVar(_) => InferredType::Unresolved,
        }
    }

    /// 引数の型 `arg_ty` がパラメータの期待型 `expected` と互換性があるかを判定する。
    ///
    /// 以下のルールに従って判定する:
    /// - `arg_ty` が `Unresolved` → 静的に確定しないため実行時に委ねる（互換性あり）
    /// - `expected` が `Any` → あらゆる引数型を受け付ける（互換性あり）
    /// - `arg_ty == expected` → 完全一致（互換性あり）
    /// - `expected` が `Union[T1, T2, ...]` → `arg_ty` が Union の直接メンバなら互換性あり
    /// - `arg_ty` が `Any` / `Union` で `expected` がそれ以外 → 明示的ダウンキャストが必要（互換性なし）
    ///
    /// # 引数
    /// - `arg_ty`: 呼び出し側が渡した引数の推論型
    /// - `expected`: パラメータ宣言で要求される型
    ///
    /// # 戻り値
    /// 互換性があれば `true`、なければ `false`。
    fn type_matches(&self, arg_ty: &InferredType, expected: &InferredType) -> bool {
        if *arg_ty == InferredType::Unresolved {
            return true;
        }
        if *expected == InferredType::Any {
            return true;
        }
        if arg_ty == expected {
            return true;
        }
        // bare `type`（TypeVal）: 任意の型値（TypeValOf も含む）を受け付ける。
        if *expected == InferredType::TypeVal {
            return matches!(arg_ty, InferredType::TypeValOf(_) | InferredType::TypeVal);
        }
        // type[T]: 渡された型値が T またはその派生型かを検査する。
        if let InferredType::TypeValOf(expected_inner) = expected {
            return match arg_ty {
                InferredType::TypeVal => true, // bare `type` 型の変数は寛容に受け付ける
                InferredType::TypeValOf(arg_inner) => {
                    self.type_val_compatible(arg_inner, expected_inner)
                }
                _ => false,
            };
        }
        // list / set / dict サブタイプ規則:
        //   - list[T] は list を満たす（具体型は汎用型の部分型）
        //   - list は list[T] を満たす（要素型不明のため寛容に受け入れる）
        //   - list[T] は list[U] を満たす iff type_matches(T, U)
        match (arg_ty, expected) {
            (InferredType::ListOf(_), InferredType::List) => return true,
            (InferredType::List, InferredType::ListOf(_)) => return true,
            (InferredType::ListOf(a), InferredType::ListOf(e)) => return self.type_matches(a, e),
            (InferredType::SetOf(_), InferredType::Set) => return true,
            (InferredType::Set, InferredType::SetOf(_)) => return true,
            (InferredType::SetOf(a), InferredType::SetOf(e)) => return self.type_matches(a, e),
            (InferredType::DictOf(_, _), InferredType::Dict) => return true,
            (InferredType::Dict, InferredType::DictOf(_, _)) => return true,
            (InferredType::DictOf(ak, av), InferredType::DictOf(ek, ev)) => {
                return self.type_matches(ak, ek) && self.type_matches(av, ev);
            }
            _ => {}
        }
        // Union パラメータ: 引数が Union のいずれかのメンバと互換性があるか確認する。
        // サブタイプ関係（list[T] ∈ Union[list, None] など）を考慮して type_matches を使用する。
        if let InferredType::Union(union_types) = expected {
            return union_types.iter().any(|ut| self.type_matches(arg_ty, ut));
        }
        // 自動キャスト: 引数の型がインスタンス型で、そのクラスが期待型への __cast__ を持つ場合は許可。
        if let InferredType::NamedInstance(class_name) = arg_ty {
            let expected_name = expected.to_string();
            let cast_key = format!("__cast__[{}]", expected_name);
            if let Some(methods) = self.class_method_sigs.get(class_name.as_str()) {
                if methods.contains_key(&cast_key) {
                    return true;
                }
            }
        }
        false
    }

    /// `arg_inner` が `expected_inner` と互換性のある型値かを判定する。
    ///
    /// 以下の場合に `true` を返す:
    /// - 完全一致（`arg_inner == expected_inner`）
    /// - `arg_inner` が `NamedInstance` で、その new_type チェーンが `expected_inner` に到達する
    /// - `arg_inner` が `NamedInstance` で、そのクラス基底に `expected_inner` の名前が含まれる
    fn type_val_compatible(&self, arg_inner: &InferredType, expected_inner: &InferredType) -> bool {
        if arg_inner == expected_inner {
            return true;
        }

        let InferredType::NamedInstance(arg_name) = arg_inner else {
            return false;
        };

        // expected_inner を文字列名に変換（プリミティブは Display、NamedInstance は名前を使用）
        let expected_name = expected_inner.to_string();

        // new_type チェーンを辿って expected_name に到達するか確認する
        let mut current = arg_name.clone();
        let mut seen = std::collections::HashSet::new();
        loop {
            let Some(orig_name) = self.new_type_originals.get(&current).cloned() else {
                break;
            };
            if !seen.insert(orig_name.clone()) {
                break;
            }
            if orig_name == expected_name {
                return true;
            }
            current = orig_name;
        }

        // クラス基底・トレイト実装を確認する（`class Foo(Bar)` → type[Bar] に互換）
        if let Some(bases) = self.class_bases.get(arg_name.as_str()) {
            return bases.contains(&expected_name);
        }

        false
    }

    /// `raise` できる型かを判定する。
    ///
    /// `raise` は `Error` trait を実装したクラスのインスタンスだけを受け付ける。
    /// 静的に型が確定しない値は安全側で拒否し、必要なら `Error` 実装クラスの
    /// インスタンスとして明示的に構築してから送出させる。
    fn is_error_instance_type(&self, ty: &InferredType) -> bool {
        match ty {
            InferredType::NamedInstance(class_name) => {
                self.class_implements_trait(class_name, "Error")
            }
            InferredType::Union(types) => types.iter().all(|t| self.is_error_instance_type(t)),
            _ => false,
        }
    }

    /// クラスが指定 trait を実装しているかを基底リストから確認する。
    fn class_implements_trait(&self, class_name: &str, trait_name: &str) -> bool {
        let mut stack = vec![class_name.to_string()];
        let mut seen = HashSet::new();
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur.clone()) {
                continue;
            }
            let Some(bases) = self.class_bases.get(cur.as_str()) else {
                continue;
            };
            if bases.iter().any(|base| base == trait_name) {
                return true;
            }
            stack.extend(bases.iter().cloned());
        }
        false
    }

    // --- 関数呼び出しの型検査 ---

    /// 関数呼び出し式の型を推論し、引数の型・個数・Self 型パラメータを検査する。
    ///
    /// 以下の検査を行う:
    /// 1. `ident.method(args)` 形式のメソッド呼び出しを検出し、Self 型パラメータ制約を検査する
    /// 2. 全引数を推論してキーワード引数名・引数個数・引数型を検査する
    /// 3. コンストラクタ呼び出し（既知クラス名の関数呼び出し）は `NamedInstance` を返す
    /// 4. 引数個数が一意に一致するオーバーロードがあれば、その戻り値型を返す
    ///
    /// # 引数
    /// - `func`: 呼び出す関数式（`Expr::Ident`, `Expr::Attr` など）
    /// - `args`: 引数リスト（位置引数・キーワード引数の混在可）
    ///
    /// # 戻り値
    /// 推論された戻り値型。静的に確定できない場合は [`InferredType::Unresolved`]。
    ///
    /// # 副作用
    /// 引数個数・型の不一致や Self 型不一致が検出された場合に `self.errors` へ追記する。
    fn infer_call(&mut self, func: &Expr, args: &[CallArg]) -> InferredType {
        // `ident.method(...)` 形式を検出してメソッド呼び出し情報を取得する。
        // Self 型パラメータ検査のためにレシーバのクラス名とメソッド名が必要。
        let method_call_info: Option<(String, String)> =
            if let Expr::Attr { object, attr, span } = func {
                let obj_ty = match object.as_ref() {
                    Expr::Ident(n) => self
                        .lookup(n)
                        .map(|v| v.ty.clone())
                        .unwrap_or(InferredType::Unresolved),
                    _ => InferredType::Unresolved,
                };
                if let InferredType::NamedInstance(cls_name) = obj_ty {
                    // static メソッドをインスタンスから呼び出していないか検査する。
                    let is_static = self
                        .class_static_methods
                        .get(&cls_name)
                        .map(|s| s.contains(attr.as_str()))
                        .unwrap_or(false);
                    if is_static {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::StaticMethodOnInstance {
                                method_name: attr.clone(),
                                class_name: cls_name.clone(),
                            },
                            span: Some(span.clone()),
                        });
                    }
                    Some((cls_name, attr.clone()))
                } else {
                    None
                }
            } else {
                None
            };

        // 可変借用の前に関数名を取得しておく。
        let func_name = if let Expr::Ident(name) = func {
            Some(name.clone())
        } else {
            None
        };
        let func_type = self.infer(func);

        // 全引数の型を推論し、キーワード引数名と型のペアとして収集する。
        let mut arg_data: Vec<(Option<String>, InferredType)> = Vec::new();
        for arg in args.iter() {
            match arg {
                CallArg::Positional(e) => arg_data.push((None, self.infer(e))),
                CallArg::Keyword { name, value } => {
                    arg_data.push((Some(name.clone()), self.infer(value)))
                }
            }
        }

        // 関数型変数の呼び出し: func_type が Function なら専用の検査・戻り値推論を行う。
        match func_type {
            InferredType::Function {
                params: Some(fn_params),
                return_type,
            } => {
                let fname = func_name.as_deref().unwrap_or("<function>").to_string();
                let ret = *return_type;
                self.check_fn_type_call(&fname, args, &arg_data, &fn_params);
                return ret;
            }
            InferredType::Function { params: None, .. } => {
                // bare `function` — 任意の引数を受け付け、Any を返す。
                return InferredType::Any;
            }
            _ => {}
        }

        // Self 型パラメータの制約を検査する。
        if let Some((ref cls_name, ref method_name)) = method_call_info {
            self.check_self_type_params(cls_name, method_name, &arg_data);
        }

        // 引数個数・型・キーワード引数名を検査する。
        if let Some(ref fname) = func_name {
            self.check_call_args(fname, &arg_data);
        }

        // 既知クラス名のコンストラクタ呼び出しは NamedInstance を返す。
        // これにより new_type などの異なる型を静的に区別できる。
        if let Some(ref fname) = func_name {
            if self.known_class_names.contains(fname.as_str()) {
                return InferredType::NamedInstance(fname.clone());
            }
        }

        // 引数個数が一意に一致するオーバーロードがあれば、その戻り値型を返す。
        // 複数候補がある場合は型が確定しないため Unresolved を返す。
        func_name
            .as_deref()
            .and_then(|n| self.fn_sigs.get(n))
            .and_then(|sigs| {
                let call_count = arg_data.len();
                let matching: Vec<_> = sigs
                    .iter()
                    .filter(|s| call_count >= s.required_count && call_count <= s.params.len())
                    .collect();
                if matching.len() == 1 {
                    matching[0].return_type.clone()
                } else {
                    None
                }
            })
            .unwrap_or(InferredType::Unresolved)
    }

    /// 名前付きインスタンスのメソッド呼び出しにおける `Self` 型パラメータ制約を検査する。
    ///
    /// `Self` 型のパラメータに渡された引数がレシーバクラスと異なるクラスのインスタンスである場合、
    /// [`TypeErrorKind::SelfTypeMismatch`] エラーを生成する。
    ///
    /// # 引数
    /// - `cls_name`: レシーバのクラス名（`Self` が解決されるクラス名）
    /// - `method_name`: 呼び出されたメソッド名
    /// - `arg_data`: `(キーワード引数名, 引数の推論型)` のペアのスライス
    ///
    /// # 副作用
    /// `Self` 型不一致が検出された場合に `self.errors` へ追記する。
    fn check_self_type_params(
        &mut self,
        cls_name: &str,
        method_name: &str,
        arg_data: &[(Option<String>, InferredType)],
    ) {
        let sigs = match self
            .class_method_sigs
            .get(cls_name)
            .and_then(|m| m.get(method_name))
            .cloned()
        {
            Some(s) => s,
            None => return,
        };
        // メソッドパラメータには `self` が含まれるが引数リストには含まれないため、
        // 有効範囲を +1 してチェックする（self 分のオフセット）。
        let effective_count = arg_data.len() + 1;
        let count_matching: Vec<FnSig> = sigs
            .iter()
            .filter(|s| effective_count >= s.required_count && effective_count <= s.params.len())
            .cloned()
            .collect();
        if count_matching.len() != 1 {
            // 候補が一意でない場合は実行時に委ねる。
            return;
        }
        let sig = &count_matching[0];
        for (arg_idx, (_, arg_ty)) in arg_data.iter().enumerate() {
            let param_idx = arg_idx + 1; // `self` をスキップしてパラメータインデックスに変換する。
            if let Some((param_name, Some(InferredType::SelfType))) = sig.params.get(param_idx) {
                if let InferredType::NamedInstance(got_cls) = arg_ty {
                    if got_cls != cls_name {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::SelfTypeMismatch {
                                method: method_name.to_string(),
                                param_name: param_name.clone(),
                                expected_class: cls_name.to_string(),
                                got_class: got_cls.clone(),
                            },
                            span: None,
                        });
                    }
                }
            }
        }
    }

    /// 名前付き関数呼び出しの引数個数・型・キーワード引数名を検査する。
    ///
    /// オーバーロードがある場合の動作:
    /// - 引数個数が一致するオーバーロードが 0 件 → `NoMatchingOverload` または `CallArgCountMismatch` を報告
    /// - 引数個数が一致するオーバーロードが複数 → 実行時ディスパッチに委ねる（型検査をスキップ）
    /// - 引数個数が一致するオーバーロードが 1 件 → 引数型・キーワード引数名を詳細検査する
    ///
    /// # 引数
    /// - `fname`: 呼び出した関数名
    /// - `arg_data`: `(キーワード引数名, 引数の推論型)` のペアのスライス
    ///
    /// # 副作用
    /// 引数個数・型・キーワード引数名の不一致が検出された場合に `self.errors` へ追記する。
    fn check_call_args(&mut self, fname: &str, arg_data: &[(Option<String>, InferredType)]) {
        let sigs = match self.fn_sigs.get(fname).cloned() {
            Some(s) => s,
            None => return, // 未知の関数は実行時エラーに委ねる。
        };
        let call_count = arg_data.len();
        // 呼び出し引数数が有効範囲（required_count..=params.len()）に収まるオーバーロードを絞り込む。
        let count_matching: Vec<FnSig> = sigs
            .iter()
            .filter(|s| call_count >= s.required_count && call_count <= s.params.len())
            .cloned()
            .collect();

        if count_matching.is_empty() {
            // 引数個数が合う候補がない。
            if sigs.len() == 1 {
                // 単一定義の場合は CallArgCountMismatch で期待個数を明示する。
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::CallArgCountMismatch {
                        func_name: fname.to_string(),
                        expected_min: sigs[0].required_count,
                        expected_max: sigs[0].params.len(),
                        got: call_count,
                    },
                    span: None,
                });
            } else {
                // オーバーロードがある場合は NoMatchingOverload で候補一覧を表示する。
                let available = sigs.iter().map(|s| s.params.len()).collect();
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::NoMatchingOverload {
                        func_name: fname.to_string(),
                        got: call_count,
                        available,
                    },
                    span: None,
                });
            }
            return;
        }
        if count_matching.len() > 1 {
            // 複数のオーバーロードが個数一致 → 実行時ディスパッチに委ねる。
            return;
        }

        // 個数が一意に一致するオーバーロードが見つかったため、引数型とキーワード引数名を検査する。
        let sig = &count_matching[0];
        let mut positional_idx = 0usize;
        for (key, arg_ty) in arg_data {
            match key {
                Some(kwarg_name) => {
                    // キーワード引数: パラメータ名でマッチングし、型を検査する。
                    match sig.params.iter().position(|(n, _)| n == kwarg_name) {
                        None => self.report_error(StaticTypeError {
                            kind: TypeErrorKind::UnknownKeywordArg {
                                func_name: fname.to_string(),
                                arg_name: kwarg_name.clone(),
                            },
                            span: None,
                        }),
                        Some(param_pos) => {
                            if let Some(expected) = &sig.params[param_pos].1 {
                                if !self.type_matches(arg_ty, expected) {
                                    self.report_error(StaticTypeError {
                                        kind: TypeErrorKind::CallArgTypeMismatch {
                                            func_name: fname.to_string(),
                                            param_index: param_pos,
                                            expected: expected.clone(),
                                            got: arg_ty.clone(),
                                        },
                                        span: None,
                                    });
                                }
                            }
                        }
                    }
                }
                None => {
                    // 位置引数: インデックス順にパラメータと対応付けて型を検査する。
                    if let Some((_, param_ty)) = sig.params.get(positional_idx) {
                        if let Some(expected) = param_ty {
                            if !self.type_matches(arg_ty, expected) {
                                self.report_error(StaticTypeError {
                                    kind: TypeErrorKind::CallArgTypeMismatch {
                                        func_name: fname.to_string(),
                                        param_index: positional_idx,
                                        expected: expected.clone(),
                                        got: arg_ty.clone(),
                                    },
                                    span: None,
                                });
                            }
                        }
                    }
                    positional_idx += 1;
                }
            }
        }
    }

    // --- 関数型呼び出しの検査 ---

    /// 関数型変数の呼び出し検査：引数個数・型・キーワード名・`mut` 引数の可変性を検査する。
    fn check_fn_type_call(
        &mut self,
        func_name: &str,
        args: &[CallArg],
        arg_data: &[(Option<String>, InferredType)],
        params: &[FnTypeParam],
    ) {
        if arg_data.len() != params.len() {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::CallArgCountMismatch {
                    func_name: func_name.to_string(),
                    expected_min: params.len(),
                    expected_max: params.len(),
                    got: arg_data.len(),
                },
                span: None,
            });
            return;
        }

        let mut positional_idx = 0usize;
        for (i, (key, arg_ty)) in arg_data.iter().enumerate() {
            let arg_expr = args[i].expr();
            match key {
                Some(kwarg_name) => match params.iter().position(|p| &p.name == kwarg_name) {
                    None => self.report_error(StaticTypeError {
                        kind: TypeErrorKind::UnknownKeywordArg {
                            func_name: func_name.to_string(),
                            arg_name: kwarg_name.clone(),
                        },
                        span: None,
                    }),
                    Some(param_pos) => {
                        let param = &params[param_pos];
                        if param.ty != InferredType::Any && !self.type_matches(arg_ty, &param.ty) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::CallArgTypeMismatch {
                                    func_name: func_name.to_string(),
                                    param_index: param_pos,
                                    expected: param.ty.clone(),
                                    got: arg_ty.clone(),
                                },
                                span: None,
                            });
                        }
                        if param.mutable && !self.is_mutable_expr(arg_expr) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::CallMutParamWithImmutableArg {
                                    func_name: func_name.to_string(),
                                    param_name: param.name.clone(),
                                },
                                span: None,
                            });
                        }
                    }
                },
                None => {
                    if let Some(param) = params.get(positional_idx) {
                        if param.ty != InferredType::Any && !self.type_matches(arg_ty, &param.ty) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::CallArgTypeMismatch {
                                    func_name: func_name.to_string(),
                                    param_index: positional_idx,
                                    expected: param.ty.clone(),
                                    got: arg_ty.clone(),
                                },
                                span: None,
                            });
                        }
                        if param.mutable && !self.is_mutable_expr(arg_expr) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::CallMutParamWithImmutableArg {
                                    func_name: func_name.to_string(),
                                    param_name: param.name.clone(),
                                },
                                span: None,
                            });
                        }
                    }
                    positional_idx += 1;
                }
            }
        }
    }

    /// 式が可変変数の参照かどうかを判定する。
    /// `Expr::Ident(name)` で `name` がスコープ内の可変変数の場合にのみ `true` を返す。
    fn is_mutable_expr(&self, expr: &Expr) -> bool {
        if let Expr::Ident(name) = expr {
            self.lookup(name).map(|v| v.mutable).unwrap_or(false)
        } else {
            false
        }
    }

    // --- 二項演算子の型検査 ---

    /// 二項演算子の型制約を検査する。
    ///
    /// 以下の順で検査を行う:
    /// 1. `Any` オペランドが片方でもあればエラー（明示的ダウンキャストが必要）
    /// 2. `Union` / `Option` オペランドが片方でもあればエラー（明示的ダウンキャストが必要）
    /// 3. 順序比較演算子（`<` / `>` / `<=` / `>=`）は互換性のある型かを検査する
    ///
    /// # 引数
    /// - `op`: 二項演算子
    /// - `lt`: 左辺の推論型
    /// - `rt`: 右辺の推論型
    /// - `span`: エラー報告に使用するスパン情報
    ///
    /// # 副作用
    /// 型制約違反が検出された場合に `self.errors` へ追記する。
    fn check_binop(&mut self, op: &BinOp, lt: &InferredType, rt: &InferredType, span: Span) {
        // Any オペランドが片方でもあればエラーとする。明示的なダウンキャストが必要。
        if *lt == InferredType::Any || *rt == InferredType::Any {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::OperationOnAny {
                    op: op.as_str().to_string(),
                },
                span: Some(span),
            });
            return;
        }
        // Union / Option オペランドが片方でもあればエラーとする。
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
        // 順序比較演算子の型互換性を検査する（`==` / `!=` は異なる型間でも許容）。
        match op {
            BinOp::Lt => self.check_ordered_cmp(lt, rt, "<", span),
            BinOp::Gt => self.check_ordered_cmp(lt, rt, ">", span),
            BinOp::LtEq => self.check_ordered_cmp(lt, rt, "<=", span),
            BinOp::GtEq => self.check_ordered_cmp(lt, rt, ">=", span),
            _ => {}
        }
    }

    /// 順序比較演算子（`<` / `>` / `<=` / `>=`）の型互換性を検査する。
    ///
    /// 互換性がない場合は [`StaticTypeError::incompatible_cmp`] を生成して報告する。
    ///
    /// # 引数
    /// - `lt`: 左辺の推論型
    /// - `rt`: 右辺の推論型
    /// - `op`: 演算子記号（`"<"` など）
    /// - `span`: エラー報告に使用するスパン情報
    ///
    /// # 副作用
    /// 型不一致が検出された場合に `self.errors` へ追記する。
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

    /// 順序比較演算子が適用可能な型の組み合わせかどうかを判定する。
    ///
    /// どちらか一方が `Unresolved` の場合は「互換性あり」として実行時に委ねる。
    ///
    /// # 引数
    /// - `lt`: 左辺の推論型
    /// - `rt`: 右辺の推論型
    ///
    /// # 戻り値
    /// `lt op rt` が順序比較として有効なら `true`、そうでなければ `false`。
    fn ordered_comparable(lt: &InferredType, rt: &InferredType) -> bool {
        use InferredType::*;
        matches!(
            (lt, rt),
            (Unresolved, _)     // 片方が未解決 → 実行時に委ねる
                | (_, Unresolved)
                | (Int, Int)
                | (Float, Float)
                | (Int, Float)
                | (Float, Int)
                | (Str, Str)
        )
    }

    /// 二項演算の結果型を推論して返す。
    ///
    /// `Any` / `Union` オペランドが含まれる場合は既にエラーが報告されているため
    /// `Unresolved` を返す。
    ///
    /// # 引数
    /// - `op`: 二項演算子
    /// - `lt`: 左辺の推論型
    /// - `rt`: 右辺の推論型
    ///
    /// # 戻り値
    /// 演算結果の推論型。静的に確定できない場合は [`InferredType::Unresolved`]。
    fn infer_binop_result(op: &BinOp, lt: &InferredType, rt: &InferredType) -> InferredType {
        use InferredType::*;
        // Any / Union オペランドは既にエラーが報告されているため、結果は Unresolved にする。
        if *lt == Any || *rt == Any {
            return Unresolved;
        }
        if matches!(lt, Union(_)) || matches!(rt, Union(_)) {
            return Unresolved;
        }
        // セット演算
        if *lt == Set && *rt == Set {
            return match op {
                BinOp::BitOr | BinOp::BitAnd | BinOp::BitXor | BinOp::Sub => Set,
                BinOp::Eq | BinOp::NotEq => Bool,
                _ => Unresolved,
            };
        }
        match op {
            // 比較・論理演算子は常に Bool を返す。
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
            // 加算: int+int → int、浮動小数混在 → float、str+str → str。
            BinOp::Add => match (lt, rt) {
                (Int, Int) => Int,
                (Float, Float) | (Int, Float) | (Float, Int) => Float,
                (Str, Str) => Str,
                _ => Unresolved,
            },
            // 減算・乗算・べき乗: 数値型のみ確定、それ以外は Unresolved。
            BinOp::Sub | BinOp::Mul | BinOp::Pow => match (lt, rt) {
                (Int, Int) => Int,
                (Float, Float) | (Int, Float) | (Float, Int) => Float,
                _ => Unresolved,
            },
            // 除算: 常に Float（Python 同様）。
            BinOp::Div => Float,
            // 整数除算・剰余: int 同士のみ int、それ以外は Unresolved。
            BinOp::FloorDiv | BinOp::Mod => match (lt, rt) {
                (Int, Int) => Int,
                _ => Unresolved,
            },
            // ビット演算: 常に Int を返す。
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::LShift | BinOp::RShift => Int,
        }
    }

    /// デコレータ式の型シグネチャを検査する。
    ///
    /// - `target_is_fn`: 対象が関数定義なら `true`、クラス定義なら `false`
    /// - `target_name`: デコレート対象の名前（エラーメッセージ用）
    ///
    /// 単純な識別子デコレータのみ静的検査する。複合式は実行時に委ねる。
    ///
    /// ### 関数デコレータ（`target_is_fn = true`）の制約
    /// - 関数の場合: 第 1 引数型は `function`、戻り値型は `function`
    /// - クラスの場合: `__init__` の第 2 引数（`self` の次）が `function`、`__call__` 戻り値が `function`
    ///
    /// ### クラスデコレータ（`target_is_fn = false`）の制約
    /// - 関数の場合: 第 1 引数型は `type`、戻り値型は `type`
    /// - クラスの場合: `__init__` の第 2 引数が `type`、`__call__` 戻り値が `type`
    fn check_decorator(&mut self, decorator: &Expr, target_is_fn: bool, target_name: &str) {
        self.infer(decorator);

        let dec_name = match decorator {
            Expr::Ident(name) => name.clone(),
            _ => return, // 複合式は静的検査不可
        };

        let expected_what = if target_is_fn { "function" } else { "type" };
        let target_kind = if target_is_fn { "function" } else { "class" };

        let is_fn_type = |ty: &InferredType| matches!(ty, InferredType::Function { .. });
        let is_type_type =
            |ty: &InferredType| matches!(ty, InferredType::TypeVal | InferredType::TypeValOf(_));
        let kind_matches = |ty: &InferredType| {
            if target_is_fn {
                is_fn_type(ty)
            } else {
                is_type_type(ty)
            }
        };

        // --- Case 1: 関数デコレータ ---
        if let Some(sigs) = self.fn_sigs.get(&dec_name).cloned() {
            if sigs.len() != 1 {
                return;
            } // オーバーロードは実行時に委ねる
            let sig = sigs[0].clone();

            // 第 1 引数の型を検査する
            match sig.params.first() {
                None => {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::InvalidDecorator {
                            reason: format!(
                                "decorator '{dec_name}' applied to {target_kind} '{target_name}': \
                                 must have at least one parameter of '{expected_what}' type"
                            ),
                        },
                        span: None,
                    });
                }
                Some((_, Some(first_param_ty))) => {
                    if !kind_matches(first_param_ty) {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::InvalidDecorator {
                                reason: format!(
                                    "decorator '{dec_name}' applied to {target_kind} '{target_name}': \
                                     first parameter must be '{expected_what}' type, got '{first_param_ty}'"
                                ),
                            },
                            span: None,
                        });
                    }
                }
                Some((_, None)) => {} // 型アノテーションなし → 実行時に委ねる
            }

            // 戻り値型を検査する
            if let Some(return_ty) = &sig.return_type.clone() {
                if !kind_matches(return_ty) {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::InvalidDecorator {
                            reason: format!(
                                "decorator '{dec_name}' applied to {target_kind} '{target_name}': \
                                 return type must be '{expected_what}', got '{return_ty}'"
                            ),
                        },
                        span: None,
                    });
                }
            }
            return;
        }

        // --- Case 2: クラスデコレータ ---
        if self.known_class_names.contains(dec_name.as_str()) {
            let cls_methods = match self.class_method_sigs.get(&dec_name).cloned() {
                Some(m) => m,
                None => return,
            };

            // `__init__` の第 2 引数（インデックス 1、`self` の次）を検査する
            if let Some(init_sigs) = cls_methods.get("__init__").cloned() {
                if init_sigs.len() == 1 {
                    if let Some((_, second_ty_opt)) = init_sigs[0].params.get(1) {
                        if let Some(second_ty) = second_ty_opt {
                            if !kind_matches(second_ty) {
                                self.report_error(StaticTypeError {
                                    kind: TypeErrorKind::InvalidDecorator {
                                        reason: format!(
                                            "class decorator '{dec_name}' applied to {target_kind} '{target_name}': \
                                             '__init__' second parameter must be '{expected_what}' type, got '{second_ty}'"
                                        ),
                                    },
                                    span: None,
                                });
                            }
                        }
                    }
                }
            }

            // `__call__` の戻り値型を検査する
            if let Some(call_sigs) = cls_methods.get("__call__").cloned() {
                if call_sigs.len() == 1 {
                    if let Some(return_ty) = &call_sigs[0].return_type.clone() {
                        if !kind_matches(return_ty) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::InvalidDecorator {
                                    reason: format!(
                                        "class decorator '{dec_name}' applied to {target_kind} '{target_name}': \
                                         '__call__' return type must be '{expected_what}', got '{return_ty}'"
                                    ),
                                },
                                span: None,
                            });
                        }
                    }
                }
            }
            return;
        }

        // 未知の識別子（import 由来など）は実行時に委ねる
    }

    /// 型ガード式の右辺に書かれた型名文字列を [`InferredType`] に変換する。
    ///
    /// プリミティブ型名はそれぞれの型に、その他はすべて `NamedInstance` として扱う。
    /// これにより、ユーザー定義クラス・new_type・trait 名もカバーする。
    fn type_from_guard_name(name: &str) -> InferredType {
        match name {
            "int" => InferredType::Int,
            "float" => InferredType::Float,
            "str" => InferredType::Str,
            "bool" => InferredType::Bool,
            "None" => InferredType::None,
            other => InferredType::NamedInstance(other.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
