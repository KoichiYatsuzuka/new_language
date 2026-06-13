// ops.rs — 演算・比較・真偽値・表示 (is_truthy / type_name / display / display_repr / repr_val / apply_unary / apply_binop / values_eq)
//
// `Value` に対する演算・表示・型名取得などの基本操作を実装する。
// これらはインタープリタ全体から頻繁に呼ばれる共通ユーティリティ群。

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{BinOp, Param, UnaryOp};

use super::str_methods::percent_format;
use super::{Interpreter, Value};

/// 関数パラメータリストを `(name: Type, name2)` 形式の文字列に変換する。
/// `self` パラメータは除外する。
fn format_fn_params(params: &[Param]) -> String {
    params
        .iter()
        .filter(|p| p.name != "self")
        .map(|p| {
            if let Some(t) = &p.type_ann {
                format!("{}: {}", p.name, t)
            } else {
                p.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

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
    pub(super) fn is_truthy(&self, val: &Value) -> bool {
        match val {
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::UInt(n) => *n != 0,
            Value::Float(f) => *f != 0.0,
            Value::Complex(re, im) => *re != 0.0 || *im != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::None => false,
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
            | Value::AsyncStatusVal(_) => true,
        }
    }

    /// 値のランタイム型名を文字列として返す（エラーメッセージや型検査に使用）。
    ///
    /// - `val`: 型名を取得する値
    ///
    /// 戻り値: `"int"`, `"str"`, `"list"`, `"object"` 等の静的文字列
    pub(super) fn type_name(&self, val: &Value) -> &'static str {
        match val {
            Value::Int(_) => "int",
            Value::UInt(_) => "uint",
            Value::Float(_) => "float",
            Value::Complex(_, _) => "complex",
            Value::Str(_) => "str",
            Value::Bool(_) => "bool",
            Value::None => "NoneType",
            Value::List(_) => "list",
            Value::FrozenList { .. } => "fixed_list",
            Value::Function(_) | Value::OverloadedFn(_) => "function",
            Value::Class(_) | Value::Type(_) => "type",
            Value::Trait(_) => "trait",
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
        }
    }

    /// ランタイム値が型アノテーション文字列に一致するかを判定する（block_return/loop_yield の型チェック用）。
    ///
    /// - `Any` は常に true。
    /// - `list[T]`, `dict[K,V]` 等はアウター型のみチェック（`list` として扱う）。
    /// - `Optional[T]` / `Option[T]` は None またはインナー型を受け入れる。
    /// - `Union[T,U,...]` は各候補のいずれかにマッチすれば true。
    /// - クラス名・トレイト名は `value_is_type` に委譲する。
    pub(super) fn value_matches_type_ann(&self, val: &Value, ann: &str) -> bool {
        match ann {
            "Any" => true,
            "None" => matches!(val, Value::None),
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
    pub(super) fn check_block_return_type(&self, val: &Value, ann: &str) -> Result<(), String> {
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
    pub(super) fn value_is_type(&self, val: &Value, type_name: &str) -> bool {
        match val {
            Value::Int(_) => type_name == "int",
            Value::UInt(_) => type_name == "uint",
            Value::Float(_) => type_name == "float",
            Value::Complex(_, _) => type_name == "complex",
            Value::Str(_) => type_name == "str",
            Value::Bool(_) => type_name == "bool",
            Value::None => type_name == "None",
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                inst.class.name == type_name || inst.class.bases.contains(&type_name.to_string())
            }
            Value::Class(cls) => cls.name == type_name,
            Value::Function(_)
            | Value::OverloadedFn(_)
            | Value::GeneratorFn(_)
            | Value::NativeFunction(_) => type_name == "function",
            Value::FileObject(_) => type_name == "FileObject",
            Value::Slice(_) => type_name == "slice",
            Value::Set(_) => type_name == "set",
            _ => false,
        }
    }

    /// 値を `print()` 出力用の文字列に変換する。
    /// 文字列値はクォートなしでそのまま返す（`display_repr` との違い）。
    ///
    /// - `val`: 表示する値
    ///
    /// 戻り値: 人間が読みやすい表示文字列
    pub(super) fn display(&self, val: &Value) -> String {
        match val {
            Value::Int(n) => n.to_string(),
            Value::UInt(n) => n.to_string(),
            Value::Float(f) => {
                if f.fract() == 0.0 && f.abs() < 1e15 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                }
            }
            Value::Complex(re, im) => {
                let fmt_f = |f: f64| -> String {
                    let f = if f == 0.0 { 0.0 } else { f }; // normalize -0.0
                    if f.fract() == 0.0 && f.abs() < 1e15 {
                        format!("{f:.1}")
                    } else {
                        f.to_string()
                    }
                };
                let re_n = if *re == 0.0 { 0.0 } else { *re };
                let im_n = if *im == 0.0 { 0.0 } else { *im };
                if im_n >= 0.0 {
                    format!("({}+{}j)", fmt_f(re_n), fmt_f(im_n))
                } else {
                    format!("({}-{}j)", fmt_f(re_n), fmt_f(im_n.abs()))
                }
            }
            Value::Str(s) => s.clone(),
            Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            Value::None => "None".to_string(),
            Value::List(items) => {
                let parts: Vec<String> = items
                    .borrow()
                    .iter()
                    .map(|v| self.display_repr(v))
                    .collect();
                format!("[{}]", parts.join(", "))
            }
            Value::Function(fn_rc) => {
                let addr = Rc::as_ptr(fn_rc) as usize;
                let sig = format_fn_params(&fn_rc.params);
                format!("<function '{}'({}) at 0x{:x}>", fn_rc.name, sig, addr)
            }
            Value::OverloadedFn(fns) => {
                let first = &fns[0];
                let addr = Rc::as_ptr(first) as usize;
                format!(
                    "<function '{}' ({} overloads) at 0x{:x}>",
                    first.name,
                    fns.len(),
                    addr
                )
            }
            Value::Class(c) => format!("<class '{}'>", c.name),
            Value::Instance(i) => {
                let class_name = i.borrow().class.name.clone();
                let addr = Rc::as_ptr(i) as usize;
                format!("<{} object at 0x{:x}>", class_name, addr)
            }
            Value::Type(name) => format!("<class '{name}'>"),
            Value::Trait(name) => format!("<trait '{name}'>"),
            Value::TemplateFn(t) => format!("<template function '{}'>", t.name),
            Value::TemplateClass(t) => format!("<template class '{}'>", t.name),
            Value::GeneratorFn(gf) => format!("<generator function '{}'>", gf.name),
            Value::TemplateGenFn(t) => format!("<template generator function '{}'>", t.name),
            Value::Generator(s) => {
                let state = s.borrow();
                let addr = Rc::as_ptr(s) as usize;
                let yield_type = if state.values.is_empty() {
                    "Any".to_string()
                } else {
                    self.type_name(&state.values[0]).to_string()
                };
                format!("<generator object[{}] at 0x{:x}>", yield_type, addr)
            }
            Value::Dict(d) => {
                let d = d.borrow();
                if d.is_empty() {
                    "{}".to_string()
                } else {
                    let keys = d.all_keys();
                    let vals = d.all_items();
                    let parts: Vec<String> = keys
                        .iter()
                        .zip(vals.iter())
                        .map(|(k, v)| format!("{}: {}", self.display_repr(k), self.display_repr(v)))
                        .collect();
                    format!("{{{}}}", parts.join(", "))
                }
            }
            Value::Tuple(t) => {
                let vals = t.all_values();
                if vals.len() == 1 {
                    format!("({},)", self.display_repr(&vals[0]))
                } else {
                    let parts: Vec<String> = vals.iter().map(|v| self.display_repr(v)).collect();
                    format!("({})", parts.join(", "))
                }
            }
            Value::Set(s) => {
                let s = s.borrow();
                if s.is_empty() {
                    "set()".to_string()
                } else {
                    let parts: Vec<String> = s.iter().map(|v| self.display_repr(v)).collect();
                    format!("{{{}}}", parts.join(", "))
                }
            }
            Value::Namespace(ns) => format!("<module '{}'>", ns.name),
            Value::FileObject(fd_rc) => {
                let fd = fd_rc.borrow();
                if fd.is_closed {
                    format!("<FileObject '{}' (closed)>", fd.path)
                } else {
                    format!("<FileObject '{}' pos={}>", fd.path, fd.pointer)
                }
            }
            Value::PyObject(h) => pyo3::Python::with_gil(|py| {
                use pyo3::types::PyAnyMethods;
                h.inner
                    .bind(py)
                    .repr()
                    .and_then(|r| r.extract::<String>())
                    .unwrap_or_else(|_| "<PyObject>".to_string())
            }),
            Value::NativeFunction(r) => format!("<native function '{}'>", r.fn_name),
            Value::Slice(s) => {
                let b = s
                    .begin
                    .as_ref()
                    .map(|v| self.display(v))
                    .unwrap_or_else(|| "None".to_string());
                let e = s
                    .end
                    .as_ref()
                    .map(|v| self.display(v))
                    .unwrap_or_else(|| "None".to_string());
                let st = s
                    .step
                    .as_ref()
                    .map(|v| self.display(v))
                    .unwrap_or_else(|| "None".to_string());
                format!("slice({b}, {e}, {st})")
            }
            Value::AsyncManager(rc) => {
                let mgr = rc.borrow();
                format!(
                    "<AsyncManager num_thread={} tasks={}>",
                    mgr.num_thread,
                    mgr.progress.len()
                )
            }
            Value::AsyncStatusVal(s) => s.display_str().to_string(),
            Value::FrozenList { state, layout } => {
                let st = state.borrow();
                let parts: Vec<String> = (0..st.len)
                    .map(|i| self.display_repr(&layout.reconstruct_item(&st.data, i)))
                    .collect();
                format!("[{}]", parts.join(", "))
            }
        }
    }

    /// 値をコレクション内要素の表示用文字列に変換する。
    /// 文字列値はシングルクォートで囲み、リストは各要素を再帰的に repr 表示する。
    /// `display` との違い: 文字列値が `'...'` 形式で出力される点。
    ///
    /// - `val`: 表示する値
    ///
    /// 戻り値: repr 形式の表示文字列
    pub(super) fn display_repr(&self, val: &Value) -> String {
        match val {
            Value::Str(s) => format!("'{s}'"),
            Value::List(items) => {
                let parts: Vec<String> = items
                    .borrow()
                    .iter()
                    .map(|v| self.display_repr(v))
                    .collect();
                format!("[{}]", parts.join(", "))
            }
            Value::Dict(_) | Value::Tuple(_) | Value::Slice(_) => self.display(val),
            _ => self.display(val),
        }
    }

    /// `repr(val)` の実装。ユーザー定義 `__repr__` メソッドを呼び出し、
    /// 定義されていない場合はデフォルトの repr 文字列を返す。
    /// コレクション内のインスタンスに対しても再帰的に `__repr__` を呼び出す。
    ///
    /// 戻り値: `Ok(String)` — repr 文字列。`Err(message)` — `__repr__` 内でエラーが発生した場合。
    pub(super) fn repr_val(&mut self, val: &Value) -> Result<String, String> {
        match val {
            // コレクション: 各要素に repr_val を再帰適用
            Value::List(items) => {
                let items_clone: Vec<Value> = items.borrow().clone();
                let parts: Result<Vec<String>, _> =
                    items_clone.iter().map(|v| self.repr_val(v)).collect();
                Ok(format!("[{}]", parts?.join(", ")))
            }
            Value::Dict(d) => {
                let (keys, vals) = {
                    let db = d.borrow();
                    (db.all_keys(), db.all_items())
                };
                if keys.is_empty() {
                    return Ok("{}".to_string());
                }
                let mut parts = Vec::new();
                for (k, v) in keys.iter().zip(vals.iter()) {
                    let kr = self.repr_val(k)?;
                    let vr = self.repr_val(v)?;
                    parts.push(format!("{kr}: {vr}"));
                }
                Ok(format!("{{{}}}", parts.join(", ")))
            }
            Value::Tuple(t) => {
                let vals = t.all_values().to_vec();
                if vals.len() == 1 {
                    let r = self.repr_val(&vals[0])?;
                    Ok(format!("({r},)"))
                } else {
                    let parts: Result<Vec<String>, _> =
                        vals.iter().map(|v| self.repr_val(v)).collect();
                    Ok(format!("({})", parts?.join(", ")))
                }
            }
            Value::Set(s) => {
                let items_clone: Vec<Value> = s.borrow().clone();
                if items_clone.is_empty() {
                    return Ok("set()".to_string());
                }
                let parts: Result<Vec<String>, _> =
                    items_clone.iter().map(|v| self.repr_val(v)).collect();
                Ok(format!("{{{}}}", parts?.join(", ")))
            }
            // インスタンス: new_type_base または __repr__ を優先して使用
            Value::Instance(inst_rc) => {
                let class = inst_rc.borrow().class.clone();

                // new_type でプリミティブを基底とする場合: ClassName(repr_of_value)
                if let Some(ref base) = class.new_type_base {
                    if matches!(base.as_str(), "int" | "float" | "str" | "bool" | "uint") {
                        let inner_val =
                            inst_rc.borrow().fields.get("value").map(|(v, _)| v.clone());
                        if let Some(v) = inner_val {
                            let inner = self.repr_val(&v)?;
                            return Ok(format!("{}({})", class.name, inner));
                        }
                    }
                }

                // ユーザー定義 __repr__ を呼び出す
                if class.methods.contains_key("__repr__") {
                    let result = self.eval_method_call_evaled(val.clone(), "__repr__", vec![])?;
                    return match result {
                        Value::Str(s) => Ok(s),
                        other => Ok(self.display(&other)),
                    };
                }

                // デフォルト: <ClassName object at 0xADDR>
                Ok(self.display(val))
            }
            // その他の型はデフォルト表示
            _ => Ok(self.display_repr(val)),
        }
    }

    /// 単項演算子を適用した結果の値を返す。
    ///
    /// - `op`: 適用する単項演算子（`Neg`=`-`, `Not`=`not`, `BitNot`=`~`）
    /// - `val`: オペランドの値
    ///
    /// 戻り値: `Ok(Value)` — 演算結果。`Err(message)` — 型エラー（例: `~str`）
    pub(super) fn apply_unary(&self, op: &UnaryOp, val: Value) -> Result<Value, String> {
        match op {
            UnaryOp::Neg => match val {
                Value::Int(n) => Ok(Value::Int(-n)),
                Value::Float(f) => Ok(Value::Float(-f)),
                Value::Complex(re, im) => Ok(Value::Complex(-re, -im)),
                _ => Err(format!(
                    "TypeError: bad operand type for unary `-`: {}",
                    self.type_name(&val)
                )),
            },
            UnaryOp::Not => Ok(Value::Bool(!self.is_truthy(&val))),
            UnaryOp::BitNot => match val {
                Value::Int(n) => Ok(Value::Int(!n)),
                Value::UInt(n) => Ok(Value::UInt(!n)),
                _ => Err(format!(
                    "TypeError: bad operand type for unary `~`: {}",
                    self.type_name(&val)
                )),
            },
        }
    }

    /// 二項演算子を適用した結果の値を返す。
    ///
    /// サポートする演算カテゴリ:
    /// - 算術: `+`, `-`, `*`, `/`, `//`, `%`, `**`（int/float 混在時は昇格）
    /// - 文字列連結: `+`（str + str）
    /// - 比較: `==`, `!=`, `<`, `>`, `<=`, `>=`
    /// - ビット演算: `&`, `|`, `^`, `<<`, `>>`（int のみ）
    ///
    /// - `op`: 適用する二項演算子
    /// - `lv`: 左オペランドの値（評価済み）
    /// - `rv`: 右オペランドの値（評価済み）
    ///
    /// 戻り値: `Ok(Value)` — 演算結果。`Err(message)` — 型エラーまたはゼロ除算エラー
    pub(super) fn apply_binop(&self, op: &BinOp, lv: Value, rv: Value) -> Result<Value, String> {
        // いずれかのオペランドが PyObject の場合は Python に委譲する
        if let Value::PyObject(h) = &lv {
            return super::py_interop::py_binop(h, op, &rv);
        }
        if let Value::PyObject(h) = &rv {
            return super::py_interop::py_rbinop(h, op, &lv);
        }
        match (op, &lv, &rv) {
            // セット演算（算術演算より先にチェック）
            (BinOp::BitOr, Value::Set(a), Value::Set(b)) => {
                let mut result = a.borrow().clone();
                for v in b.borrow().iter() {
                    if !result.iter().any(|x| self.values_eq(x, v)) {
                        result.push(v.clone());
                    }
                }
                Ok(Value::Set(Rc::new(RefCell::new(result))))
            }
            (BinOp::BitAnd, Value::Set(a), Value::Set(b)) => {
                let b_ref = b.borrow();
                let result: Vec<Value> = a
                    .borrow()
                    .iter()
                    .filter(|v| b_ref.iter().any(|x| self.values_eq(x, v)))
                    .cloned()
                    .collect();
                Ok(Value::Set(Rc::new(RefCell::new(result))))
            }
            (BinOp::Sub, Value::Set(a), Value::Set(b)) => {
                let b_ref = b.borrow();
                let result: Vec<Value> = a
                    .borrow()
                    .iter()
                    .filter(|v| !b_ref.iter().any(|x| self.values_eq(x, v)))
                    .cloned()
                    .collect();
                Ok(Value::Set(Rc::new(RefCell::new(result))))
            }
            (BinOp::BitXor, Value::Set(a), Value::Set(b)) => {
                let a_ref = a.borrow();
                let b_ref = b.borrow();
                let mut result: Vec<Value> = a_ref
                    .iter()
                    .filter(|v| !b_ref.iter().any(|x| self.values_eq(x, v)))
                    .cloned()
                    .collect();
                for v in b_ref.iter() {
                    if !a_ref.iter().any(|x| self.values_eq(x, v)) {
                        result.push(v.clone());
                    }
                }
                Ok(Value::Set(Rc::new(RefCell::new(result))))
            }
            // 包含検査 `in` / `not in`
            (BinOp::In, item, Value::List(lst)) => Ok(Value::Bool(
                lst.borrow().iter().any(|v| self.values_eq(v, item)),
            )),
            (BinOp::In, item, Value::FrozenList { state, layout }) => {
                let st = state.borrow();
                Ok(Value::Bool(
                    (0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).any(|v| self.values_eq(&v, item)),
                ))
            }
            (BinOp::In, item, Value::Set(s)) => Ok(Value::Bool(
                s.borrow().iter().any(|v| self.values_eq(v, item)),
            )),
            (BinOp::In, Value::Str(sub), Value::Str(s)) => {
                Ok(Value::Bool(s.contains(sub.as_str())))
            }
            (BinOp::In, item, Value::Dict(d)) => Ok(Value::Bool(d.borrow().get(item).is_some())),
            (BinOp::In, item, Value::Tuple(t)) => Ok(Value::Bool(
                t.all_values().iter().any(|v| self.values_eq(v, item)),
            )),
            (BinOp::NotIn, item, Value::List(lst)) => Ok(Value::Bool(
                !lst.borrow().iter().any(|v| self.values_eq(v, item)),
            )),
            (BinOp::NotIn, item, Value::FrozenList { state, layout }) => {
                let st = state.borrow();
                Ok(Value::Bool(
                    !(0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).any(|v| self.values_eq(&v, item)),
                ))
            }
            (BinOp::NotIn, item, Value::Set(s)) => Ok(Value::Bool(
                !s.borrow().iter().any(|v| self.values_eq(v, item)),
            )),
            (BinOp::NotIn, Value::Str(sub), Value::Str(s)) => {
                Ok(Value::Bool(!s.contains(sub.as_str())))
            }
            (BinOp::NotIn, item, Value::Dict(d)) => Ok(Value::Bool(d.borrow().get(item).is_none())),
            (BinOp::NotIn, item, Value::Tuple(t)) => Ok(Value::Bool(
                !t.all_values().iter().any(|v| self.values_eq(v, item)),
            )),
            (BinOp::In, _, rv) => Err(format!(
                "TypeError: argument of type '{}' is not iterable",
                self.type_name(rv)
            )),
            (BinOp::NotIn, _, rv) => Err(format!(
                "TypeError: argument of type '{}' is not iterable",
                self.type_name(rv)
            )),
            // 算術演算
            (BinOp::Add, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a + *b)),
            (BinOp::Add, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a + *b)),
            (BinOp::Add, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + *b)),
            (BinOp::Add, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a + *b as f64)),
            (BinOp::Add, Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
            // str * int / int * str → repeat
            (BinOp::Mul, Value::Str(s), Value::Int(n)) => {
                Ok(Value::Str(s.repeat((*n).max(0) as usize)))
            }
            (BinOp::Mul, Value::Int(n), Value::Str(s)) => {
                Ok(Value::Str(s.repeat((*n).max(0) as usize)))
            }
            // str % args → printf-style format
            (BinOp::Mod, Value::Str(fmt), rv) => {
                let display_fn = |v: &Value| self.display(v);
                let args: Vec<Value> = match rv {
                    Value::Tuple(t) => t.all_values().to_vec(),
                    other => vec![other.clone()],
                };
                let result = percent_format(fmt, &args, &display_fn)?;
                Ok(Value::Str(result))
            }
            (BinOp::Sub, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a - *b)),
            (BinOp::Sub, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a - *b)),
            (BinOp::Sub, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 - *b)),
            (BinOp::Sub, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a - *b as f64)),
            (BinOp::Mul, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a * *b)),
            (BinOp::Mul, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a * *b)),
            (BinOp::Mul, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 * *b)),
            (BinOp::Mul, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a * *b as f64)),
            (BinOp::Div, Value::Int(a), Value::Int(b)) => {
                if *b == 0 {
                    return Err("ZeroDivisionError: division by zero".to_string());
                }
                Ok(Value::Float(*a as f64 / *b as f64))
            }
            (BinOp::Div, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a / *b)),
            (BinOp::Div, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 / *b)),
            (BinOp::Div, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a / *b as f64)),
            (BinOp::FloorDiv, Value::Int(a), Value::Int(b)) => {
                if *b == 0 {
                    return Err("ZeroDivisionError: integer division by zero".to_string());
                }
                Ok(Value::Int(a.div_euclid(*b)))
            }
            (BinOp::Mod, Value::Int(a), Value::Int(b)) => {
                if *b == 0 {
                    return Err("ZeroDivisionError: modulo by zero".to_string());
                }
                Ok(Value::Int(a.rem_euclid(*b)))
            }
            (BinOp::Pow, Value::Int(a), Value::Int(b)) => {
                if *b >= 0 {
                    Ok(Value::Int(a.pow(*b as u32)))
                } else {
                    Ok(Value::Float((*a as f64).powi(*b as i32)))
                }
            }
            (BinOp::Pow, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a.powf(*b))),
            (BinOp::Pow, Value::Int(a), Value::Float(b)) => Ok(Value::Float((*a as f64).powf(*b))),
            (BinOp::Pow, Value::Float(a), Value::Int(b)) => Ok(Value::Float(a.powi(*b as i32))),
            // 比較演算
            (BinOp::Eq, _, _) => Ok(Value::Bool(self.values_eq(&lv, &rv))),
            (BinOp::NotEq, _, _) => Ok(Value::Bool(!self.values_eq(&lv, &rv))),
            (BinOp::Lt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(*a < *b)),
            (BinOp::Lt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(*a < *b)),
            (BinOp::Lt, Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) < *b)),
            (BinOp::Lt, Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a < (*b as f64))),
            (BinOp::Gt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(*a > *b)),
            (BinOp::Gt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(*a > *b)),
            (BinOp::Gt, Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) > *b)),
            (BinOp::Gt, Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a > (*b as f64))),
            (BinOp::LtEq, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(*a <= *b)),
            (BinOp::LtEq, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(*a <= *b)),
            (BinOp::GtEq, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(*a >= *b)),
            (BinOp::GtEq, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(*a >= *b)),
            // uint 算術・比較
            (BinOp::Add, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(a.wrapping_add(*b))),
            (BinOp::Sub, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(a.wrapping_sub(*b))),
            (BinOp::Mul, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(a.wrapping_mul(*b))),
            (BinOp::Div, Value::UInt(a), Value::UInt(b)) => {
                if *b == 0 {
                    return Err("ZeroDivisionError: division by zero".to_string());
                }
                Ok(Value::UInt(*a / *b))
            }
            (BinOp::FloorDiv, Value::UInt(a), Value::UInt(b)) => {
                if *b == 0 {
                    return Err("ZeroDivisionError: integer division by zero".to_string());
                }
                Ok(Value::UInt(*a / *b))
            }
            (BinOp::Mod, Value::UInt(a), Value::UInt(b)) => {
                if *b == 0 {
                    return Err("ZeroDivisionError: modulo by zero".to_string());
                }
                Ok(Value::UInt(*a % *b))
            }
            (BinOp::Lt, Value::UInt(a), Value::UInt(b)) => Ok(Value::Bool(*a < *b)),
            (BinOp::LtEq, Value::UInt(a), Value::UInt(b)) => Ok(Value::Bool(*a <= *b)),
            (BinOp::Gt, Value::UInt(a), Value::UInt(b)) => Ok(Value::Bool(*a > *b)),
            (BinOp::GtEq, Value::UInt(a), Value::UInt(b)) => Ok(Value::Bool(*a >= *b)),
            (BinOp::BitAnd, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(*a & *b)),
            (BinOp::BitOr, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(*a | *b)),
            (BinOp::BitXor, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(*a ^ *b)),
            (BinOp::LShift, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(*a << *b)),
            (BinOp::RShift, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(*a >> *b)),
            // ビット演算（int のみ対応）
            (BinOp::BitAnd, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a & *b)),
            (BinOp::BitOr, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a | *b)),
            (BinOp::BitXor, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a ^ *b)),
            (BinOp::LShift, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a << *b)),
            (BinOp::RShift, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a >> *b)),
            // 複素数算術（complex との加減乗除）
            (BinOp::Add, Value::Complex(r1, i1), Value::Complex(r2, i2)) => {
                Ok(Value::Complex(r1 + r2, i1 + i2))
            }
            (BinOp::Sub, Value::Complex(r1, i1), Value::Complex(r2, i2)) => {
                Ok(Value::Complex(r1 - r2, i1 - i2))
            }
            (BinOp::Mul, Value::Complex(r1, i1), Value::Complex(r2, i2)) => {
                Ok(Value::Complex(r1 * r2 - i1 * i2, r1 * i2 + i1 * r2))
            }
            (BinOp::Div, Value::Complex(r1, i1), Value::Complex(r2, i2)) => {
                let denom = r2 * r2 + i2 * i2;
                if denom == 0.0 {
                    return Err("ZeroDivisionError: complex division by zero".to_string());
                }
                Ok(Value::Complex(
                    (r1 * r2 + i1 * i2) / denom,
                    (i1 * r2 - r1 * i2) / denom,
                ))
            }
            // complex と scalar の混合
            (BinOp::Add, Value::Complex(re, im), Value::Float(s)) => {
                Ok(Value::Complex(re + s, *im))
            }
            (BinOp::Add, Value::Float(s), Value::Complex(re, im)) => {
                Ok(Value::Complex(s + re, *im))
            }
            (BinOp::Add, Value::Complex(re, im), Value::Int(n)) => {
                Ok(Value::Complex(re + *n as f64, *im))
            }
            (BinOp::Add, Value::Int(n), Value::Complex(re, im)) => {
                Ok(Value::Complex(*n as f64 + re, *im))
            }
            (BinOp::Sub, Value::Complex(re, im), Value::Float(s)) => {
                Ok(Value::Complex(re - s, *im))
            }
            (BinOp::Sub, Value::Float(s), Value::Complex(re, im)) => {
                Ok(Value::Complex(s - re, -im))
            }
            (BinOp::Sub, Value::Complex(re, im), Value::Int(n)) => {
                Ok(Value::Complex(re - *n as f64, *im))
            }
            (BinOp::Sub, Value::Int(n), Value::Complex(re, im)) => {
                Ok(Value::Complex(*n as f64 - re, -im))
            }
            (BinOp::Mul, Value::Complex(re, im), Value::Float(s)) => {
                Ok(Value::Complex(re * s, im * s))
            }
            (BinOp::Mul, Value::Float(s), Value::Complex(re, im)) => {
                Ok(Value::Complex(s * re, s * im))
            }
            (BinOp::Mul, Value::Complex(re, im), Value::Int(n)) => {
                let ns = *n as f64;
                Ok(Value::Complex(re * ns, im * ns))
            }
            (BinOp::Mul, Value::Int(n), Value::Complex(re, im)) => {
                let ns = *n as f64;
                Ok(Value::Complex(ns * re, ns * im))
            }
            (BinOp::Div, Value::Complex(re, im), Value::Float(s)) => {
                if *s == 0.0 {
                    return Err("ZeroDivisionError: complex division by zero".to_string());
                }
                Ok(Value::Complex(re / s, im / s))
            }
            (BinOp::Div, Value::Float(s), Value::Complex(re, im)) => {
                let denom = re * re + im * im;
                if denom == 0.0 {
                    return Err("ZeroDivisionError: complex division by zero".to_string());
                }
                Ok(Value::Complex(s * re / denom, -s * im / denom))
            }
            (BinOp::Div, Value::Complex(re, im), Value::Int(n)) => {
                let ns = *n as f64;
                if ns == 0.0 {
                    return Err("ZeroDivisionError: complex division by zero".to_string());
                }
                Ok(Value::Complex(re / ns, im / ns))
            }
            (BinOp::Div, Value::Int(n), Value::Complex(re, im)) => {
                let ns = *n as f64;
                let denom = re * re + im * im;
                if denom == 0.0 {
                    return Err("ZeroDivisionError: complex division by zero".to_string());
                }
                Ok(Value::Complex(ns * re / denom, -ns * im / denom))
            }
            _ => Err(format!(
                "TypeError: unsupported operand types for `{op:?}`: {} and {}",
                self.type_name(&lv),
                self.type_name(&rv)
            )),
        }
    }

    /// 2つの値が等値かどうかを判定する（`==` / `!=` 演算子および `values_eq` で使用）。
    ///
    /// - プリミティブ型（int/float/str/bool/None）は値で比較。int と float は昇格して比較
    /// - `Instance` と `Class` は参照の同一性（ポインタ比較）で判定
    /// - `Tuple` は要素数と各要素を再帰的に比較
    /// - 異なる型同士（例: int と str）は常に `false`
    ///
    /// - `a`, `b`: 比較する2つの値
    ///
    /// 戻り値: `true` — 等値、`false` — 非等値
    pub(super) fn values_eq(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::UInt(a), Value::UInt(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Complex(r1, i1), Value::Complex(r2, i2)) => r1 == r2 && i1 == i2,
            // int と float の混在比較: int を float に昇格して比較
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::None, Value::None) => true,
            // インスタンスの等値判定:
            // enum バリアント (class name が "enum_item_" で始まる) はフィールド値で比較する。
            // let バインドで深いコピーが作成されるため Rc::ptr_eq は使えない。
            // それ以外のインスタンスは参照の同一性 (Rc::ptr_eq) で比較する。
            (Value::Instance(a), Value::Instance(b)) => {
                let a_borrow = a.borrow();
                let b_borrow = b.borrow();
                if a_borrow.class.name.starts_with("enum_item_")
                    && a_borrow.class.name == b_borrow.class.name
                {
                    // enum バリアントは "value" フィールドの値で等値判定
                    match (a_borrow.fields.get("value"), b_borrow.fields.get("value")) {
                        (Some((va, _)), Some((vb, _))) => self.values_eq(va, vb),
                        _ => Rc::ptr_eq(a, b),
                    }
                } else {
                    Rc::ptr_eq(a, b)
                }
            }
            (Value::Type(a), Value::Type(b)) => a == b,
            (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b),
            // タプルは要素数と各要素を再帰的に比較
            (Value::Tuple(a), Value::Tuple(b)) => {
                let av = a.all_values();
                let bv = b.all_values();
                av.len() == bv.len() && av.iter().zip(bv.iter()).all(|(x, y)| self.values_eq(x, y))
            }
            // セットは要素数と各要素の包含関係で比較（順序無関係）
            (Value::Set(a), Value::Set(b)) => {
                let ar = a.borrow();
                let br = b.borrow();
                ar.len() == br.len() && ar.iter().all(|v| br.iter().any(|w| self.values_eq(v, w)))
            }
            _ => false,
        }
    }
}
