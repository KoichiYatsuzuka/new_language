// ops/typecheck.rs — 型・真偽値の判定: is_truthy / type_name / value_matches_type_ann / check_block_return_type / value_is_type / type_name_of。

use crate::interpreter::{Interpreter, Value};

impl Interpreter {
    /// 値の真偽判定を行う。
    ///
    /// Python ライクなルール:
    /// - `Bool` → そのまま
    /// - `Int` → `0` なら偽、それ以外は真
    /// - `Float` → `0.0` なら偽、それ以外は真
    /// - `Str` → 空文字列なら偽、それ以外は真
    /// - `None` → 偽
    /// - `List` → 空リストなら偽、非空なら真
    /// - `Dict` → 空辞書なら偽、非空なら真
    /// - `Tuple` → 空タプルなら偽、非空なら真
    /// - 関数・クラス・インスタンス等 → 常に真
    ///
    /// - `val`: 真偽を判定する値
    ///
    /// 戻り値: `true` または `false`
    pub(crate) fn is_truthy(&self, val: &Value) -> bool {
        match val {
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::UInt(n) => *n != 0,
            Value::Float(f) => *f != 0.0,
            Value::Complex(re, im) => *re != 0.0 || *im != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::None => false,
            Value::Undefined => false,
            Value::List(items) => !items.borrow().is_empty(),
            Value::FrozenList { state, .. } => state.borrow().len > 0,
            Value::Dict(d) => !d.borrow().is_empty(),
            Value::Tuple(t) => !t.is_empty(),
            Value::Set(s) => !s.borrow().is_empty(),
            // 関数・クラス・インスタンス・ジェネレータ・名前空間・Python オブジェクト等は常に真
            Value::Function(_)
            | Value::OverloadedFn(_)
            | Value::Class(_)
            | Value::Instance(_)
            | Value::Type(_)
            | Value::Trait(_)
            | Value::Protocol(_)
            | Value::TemplateFn(_)
            | Value::TemplateClass(_)
            | Value::GeneratorFn(_)
            | Value::TemplateGenFn(_)
            | Value::Generator(_)
            | Value::Namespace(_)
            | Value::PyObject(_)
            | Value::FileObject(_)
            | Value::NativeFunction(_)
            | Value::Slice(_)
            | Value::AsyncManager(_)
            | Value::AsyncStatusVal(_)
            | Value::Signal(_)
            | Value::EventLoop(_)
            | Value::CsObject(_)
            | Value::JsProcFn { .. }
            | Value::ResultVal { .. } => true,
        }
    }

    /// 値のランタイム型名を文字列として返す（エラーメッセージや型検査に使用）。
    ///
    /// - `val`: 型名を取得する値
    ///
    /// 戻り値: `"int"`, `"str"`, `"list"`, `"object"` 等の静的文字列
    pub(crate) fn type_name(&self, val: &Value) -> &'static str {
        match val {
            Value::Int(_) => "int",
            Value::UInt(_) => "uint",
            Value::Float(_) => "float",
            Value::Complex(_, _) => "complex",
            Value::Str(_) => "str",
            Value::Bool(_) => "bool",
            Value::None => "NoneType",
            Value::Undefined => "Undefined",
            Value::List(_) => "list",
            Value::FrozenList { .. } => "fixed_list",
            Value::Function(_) | Value::OverloadedFn(_) => "function",
            Value::Class(_) | Value::Type(_) => "type",
            Value::Trait(_) => "trait",
            Value::Protocol(_) => "protocol",
            Value::Instance(_) => "object",
            Value::TemplateFn(_) | Value::TemplateClass(_) => "template",
            Value::GeneratorFn(_) | Value::TemplateGenFn(_) => "gen_function",
            Value::Generator(_) => "generator",
            Value::Dict(_) => "dict",
            Value::Tuple(_) => "tuple",
            Value::Set(_) => "set",
            Value::Namespace(_) => "module",
            Value::PyObject(_) => "object",
            Value::FileObject(_) => "FileObject",
            Value::NativeFunction(_) => "function",
            Value::Slice(_) => "slice",
            Value::AsyncManager(_) => "AsyncManager",
            Value::AsyncStatusVal(_) => "Async",
            Value::Signal(_) => "Signal",
            Value::EventLoop(_) => "EventLoop",
            Value::CsObject(o) => {
                let _ = o;
                "cs_object"
            }
            Value::JsProcFn { .. } => "function",
            Value::ResultVal { ok, .. } => {
                if *ok { "Ok" } else { "Err" }
            }
        }
    }

    /// ランタイム値が型アノテーション文字列に一致するかを判定する（block_return/loop_yield の型チェック用）。
    ///
    /// - `Any` は常に true。
    /// - `list[T]`, `dict[K,V]` 等はアウター型のみチェック（`list` として扱う）。
    /// - `Optional[T]` / `Option[T]` は None またはインナー型を受け入れる。
    /// - `Union[T,U,...]` は各候補のいずれかにマッチすれば true。
    /// - クラス名・トレイト名は `value_is_type` に委譲する。
    pub(crate) fn value_matches_type_ann(&self, val: &Value, ann: &str) -> bool {
        // C ABI 型（int32 等）は基底型（int/float）の別名として扱う
        let ann = crate::ast::c_abi_base_type(ann).unwrap_or(ann);
        match ann {
            "Any" => true,
            "None" => matches!(val, Value::None),
            "Undefined" => matches!(val, Value::Undefined),
            "int" => matches!(val, Value::Int(_)),
            "uint" => matches!(val, Value::UInt(_)),
            "float" => matches!(val, Value::Float(_)),
            "complex" => matches!(val, Value::Complex(_, _)),
            "str" => matches!(val, Value::Str(_)),
            "bool" => matches!(val, Value::Bool(_)),
            "list" => matches!(val, Value::List(_)),
            "dict" => matches!(val, Value::Dict(_)),
            "tuple" => matches!(val, Value::Tuple(_)),
            "set" => matches!(val, Value::Set(_)),
            "function" => matches!(
                val,
                Value::Function(_)
                    | Value::OverloadedFn(_)
                    | Value::GeneratorFn(_)
                    | Value::NativeFunction(_)
                    | Value::JsProcFn { .. }
            ),
            _ if ann.starts_with("list[") => matches!(val, Value::List(_)),
            _ if ann.starts_with("dict[") => matches!(val, Value::Dict(_)),
            _ if ann.starts_with("set[") => matches!(val, Value::Set(_)),
            _ if ann.starts_with("tuple[") => matches!(val, Value::Tuple(_)),
            _ if ann.starts_with("Optional[") || ann.starts_with("Option[") => {
                if matches!(val, Value::None) {
                    return true;
                }
                let inner_start = ann.find('[').map_or(ann.len(), |i| i + 1);
                let inner = ann[inner_start..].trim_end_matches(']');
                self.value_matches_type_ann(val, inner.trim())
            }
            _ if ann.starts_with("Union[") => {
                let inner = ann[6..].trim_end_matches(']');
                inner
                    .split(',')
                    .any(|t| self.value_matches_type_ann(val, t.trim()))
            }
            _ => self.value_is_type(val, ann),
        }
    }

    /// `block_return` の値の型をアノテーション文字列に対してチェックする。
    /// 不一致の場合は `Err(TypeError: ...)` を返す。
    pub(crate) fn check_block_return_type(&self, val: &Value, ann: &str) -> Result<(), String> {
        if self.value_matches_type_ann(val, ann) {
            Ok(())
        } else {
            Err(format!(
                "TypeError: block_return value has type '{}', but '{}' was expected",
                self.type_name(val),
                ann
            ))
        }
    }

    /// 値が指定した型名に一致するかを判定する（`is` 型ガードのランタイム検査）。
    ///
    /// - プリミティブ型: `type_name` が `"int"`, `"float"` 等と一致するか確認する。
    /// - インスタンス: クラス名または `bases`（実装 trait・基底クラス）に含まれるか確認する。
    /// - `None` 値: `type_name == "None"` の場合のみ `true`。
    pub(crate) fn value_is_type(&self, val: &Value, type_name: &str) -> bool {
        // Protocol の実行時チェック: 必須メンバー名が全て存在するか確認する
        if let Some(required) = self.protocol_required_members.get(type_name) {
            if let Value::Instance(inst_rc) = val {
                let inst = inst_rc.borrow();
                return required
                    .iter()
                    .all(|m| inst.class.field_index.contains_key(m.as_str()) || inst.class.methods.contains_key(m));
            }
            return false;
        }
        match val {
            Value::Int(_) => type_name == "int",
            Value::UInt(_) => type_name == "uint",
            Value::Float(_) => type_name == "float",
            Value::Complex(_, _) => type_name == "complex",
            Value::Str(_) => type_name == "str",
            Value::Bool(_) => type_name == "bool",
            Value::None => type_name == "None",
            Value::Undefined => type_name == "Undefined",
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                if type_name == "function" {
                    inst.class.methods.contains_key("__call__")
                } else {
                    inst.class.name == type_name
                        || inst.class.bases.contains(&type_name.to_string())
                }
            }
            Value::Class(cls) => {
                if type_name == "function" {
                    cls.methods.contains_key("__call__")
                } else {
                    cls.name == type_name
                }
            }
            Value::Function(_)
            | Value::OverloadedFn(_)
            | Value::GeneratorFn(_)
            | Value::NativeFunction(_) => type_name == "function",
            Value::FileObject(_) => type_name == "FileObject",
            Value::Slice(_) => type_name == "slice",
            Value::List(_) => type_name == "list" || type_name == "list_like",
            Value::FrozenList { .. } => {
                type_name == "fixed_list" || type_name == "list_like"
            }
            Value::Set(_) => type_name == "set",
            Value::Dict(_) => type_name == "dict",
            Value::Tuple(_) => type_name == "tuple",
            _ => false,
        }
    }

    /// `mustbe` エラーメッセージ用: 値のランタイム型名を返す。
    pub(crate) fn type_name_of(&self, val: &Value) -> String {
        match val {
            Value::Int(_) => "int".to_string(),
            Value::UInt(_) => "uint".to_string(),
            Value::Float(_) => "float".to_string(),
            Value::Complex(_, _) => "complex".to_string(),
            Value::Str(_) => "str".to_string(),
            Value::Bool(_) => "bool".to_string(),
            Value::None => "None".to_string(),
            Value::Undefined => "Undefined".to_string(),
            Value::List(_) => "list".to_string(),
            Value::FrozenList { .. } => "fixed_list".to_string(),
            Value::Dict(_) => "dict".to_string(),
            Value::Set(_) => "set".to_string(),
            Value::Tuple(_) => "tuple".to_string(),
            Value::Function(f) => format!("function({})", f.name),
            Value::OverloadedFn(_) => "function(overloaded)".to_string(),
            Value::GeneratorFn(g) => format!("generator_fn({})", g.name),
            Value::NativeFunction(n) => format!("function({})", n.fn_name),
            Value::Class(c) => format!("class({})", c.name),
            Value::Instance(i) => i.borrow().class.name.clone(),
            Value::Slice(_) => "slice".to_string(),
            _ => "unknown".to_string(),
        }
    }

}
