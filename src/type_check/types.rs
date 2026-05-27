#![allow(dead_code)]

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Inferred type
// ---------------------------------------------------------------------------

/// 文字列 `s` をブラケット深さ 0 のカンマで分割する。
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

/// 関数型アノテーションの 1 パラメータ。
#[derive(Debug, Clone, PartialEq)]
pub struct FnTypeParam {
    pub name: String,
    pub mutable: bool,
    pub ty: InferredType,
}

/// 型推論システムが扱う型を表す列挙型。プリミティブ型・コレクション型・Union 型・関数型などを網羅する。
#[derive(Debug, Clone, PartialEq)]
pub enum InferredType {
    Int,
    Float,
    Str,
    Bool,
    None,
    List,
    ListOf(Box<InferredType>),
    TypeVal,
    TypeValOf(Box<InferredType>),
    SelfType,
    NamedInstance(String),
    Any,
    Union(Vec<InferredType>),
    Dict,
    DictOf(Box<InferredType>, Box<InferredType>),
    Set,
    SetOf(Box<InferredType>),
    Tuple(Vec<InferredType>),
    Namespace(HashMap<String, InferredType>),
    Unresolved,
    Function {
        params: Option<Vec<FnTypeParam>>,
        return_type: Box<InferredType>,
    },
}

impl InferredType {
    /// 型アノテーション文字列を [`InferredType`] に変換する。解析できない場合は `None` を返す。
    pub fn from_ann(ann: &str) -> Option<Self> {
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

    /// `function[...]->R` または `function{...}->R` 形式の関数型アノテーションを解析する。
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

    /// 対応する閉じブラケットの位置を返す。見つからない場合は `None` を返す。
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

impl std::fmt::Display for InferredType {
    /// [`InferredType`] を人間可読な型アノテーション文字列に変換して表示する。
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
// Function signature
// ---------------------------------------------------------------------------

/// 関数シグネチャ情報。パラメータ名・型アノテーション・必須引数数・戻り値型を保持する。
#[derive(Clone)]
pub(crate) struct FnSig {
    pub(crate) params: Vec<(String, Option<InferredType>)>,
    pub(crate) required_count: usize,
    pub(crate) return_type: Option<InferredType>,
}

// ---------------------------------------------------------------------------
// Variable info (scope entry)
// ---------------------------------------------------------------------------

/// スコープ内の変数情報。推論済み型と可変性フラグを保持する。
pub(crate) struct VarInfo {
    pub(crate) ty: InferredType,
    pub(crate) mutable: bool,
}
