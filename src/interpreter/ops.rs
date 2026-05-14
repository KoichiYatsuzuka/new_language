// ops.rs — 演算・比較・真偽値・表示 (is_truthy / type_name / display / display_repr / apply_unary / apply_binop / values_eq)
//
// `Value` に対する演算・表示・型名取得などの基本操作を実装する。
// これらはインタープリタ全体から頻繁に呼ばれる共通ユーティリティ群。

use std::rc::Rc;

use crate::ast::{BinOp, UnaryOp};

use super::{Interpreter, Value};

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
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::None => false,
            Value::List(items) => !items.is_empty(),
            Value::Dict(d) => !d.borrow().keys.is_empty(),
            Value::Tuple(t) => !t.is_empty(),
            // 関数・クラス・インスタンス・ジェネレータ・名前空間・Python オブジェクト等は常に真
            Value::Function(_) | Value::OverloadedFn(_) | Value::Class(_) | Value::Instance(_) | Value::Type(_)
            | Value::Trait(_)
            | Value::TemplateFn(_) | Value::TemplateClass(_)
            | Value::GeneratorFn(_) | Value::TemplateGenFn(_) | Value::Generator(_)
            | Value::Namespace(_) | Value::PyObject(_) => true,
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
            Value::Float(_) => "float",
            Value::Str(_) => "str",
            Value::Bool(_) => "bool",
            Value::None => "NoneType",
            Value::List(_) => "list",
            Value::Function(_) | Value::OverloadedFn(_) => "function",
            Value::Class(_) | Value::Type(_) => "type",
            Value::Trait(_) => "trait",
            Value::Instance(_) => "object",
            Value::TemplateFn(_) | Value::TemplateClass(_) => "template",
            Value::GeneratorFn(_) | Value::TemplateGenFn(_) => "gen_function",
            Value::Generator(_) => "generator",
            Value::Dict(_) => "dict",
            Value::Tuple(_) => "tuple",
            Value::Namespace(_) => "module",
            Value::PyObject(_) => "object",
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
            Value::Float(_) => type_name == "float",
            Value::Str(_) => type_name == "str",
            Value::Bool(_) => type_name == "bool",
            Value::None => type_name == "None",
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                inst.class.name == type_name
                    || inst.class.bases.contains(&type_name.to_string())
            }
            Value::Class(cls) => cls.name == type_name,
            Value::Function(_) | Value::OverloadedFn(_) | Value::GeneratorFn(_) => type_name == "function",
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
            Value::Float(f) => {
                if f.fract() == 0.0 && f.abs() < 1e15 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                }
            }
            Value::Str(s) => s.clone(),
            Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            Value::None => "None".to_string(),
            Value::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| self.display_repr(v)).collect();
                format!("[{}]", parts.join(", "))
            }
            Value::Function(_) => "<function>".to_string(),
            Value::OverloadedFn(fns) => format!("<function ({} overloads)>", fns.len()),
            Value::Class(c) => format!("<class '{}'>", c.name),
            Value::Instance(i) => format!("<{} object>", i.borrow().class.name),
            Value::Type(name) => format!("<type '{name}'>"),
            Value::Trait(name) => format!("<trait '{name}'>"),
            Value::TemplateFn(t) => format!("<template fn ({} type params)>", t.template_params.len()),
            Value::TemplateClass(t) => format!("<template class '{}'>", t.name),
            Value::GeneratorFn(_) => "<generator function>".to_string(),
            Value::TemplateGenFn(t) => format!("<template gen ({} type params)>", t.template_params.len()),
            Value::Generator(s) => {
                let s = s.borrow();
                format!("<generator {}/{}>", s.index, s.values.len())
            }
            Value::Dict(d) => {
                let d = d.borrow();
                if d.keys.is_empty() {
                    "{}".to_string()
                } else {
                    let parts: Vec<String> = d.keys.iter().zip(d.items.iter())
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
            Value::Namespace(ns) => format!("<module '{}'>", ns.name),
            Value::PyObject(h) => pyo3::Python::with_gil(|py| {
                use pyo3::types::PyAnyMethods;
                h.inner.bind(py).repr()
                    .and_then(|r| r.extract::<String>())
                    .unwrap_or_else(|_| "<PyObject>".to_string())
            }),
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
                let parts: Vec<String> = items.iter().map(|v| self.display_repr(v)).collect();
                format!("[{}]", parts.join(", "))
            }
            Value::Dict(_) | Value::Tuple(_) => self.display(val),
            _ => self.display(val),
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
                _ => Err(format!("TypeError: bad operand type for unary `-`: {}", self.type_name(&val))),
            },
            UnaryOp::Not => Ok(Value::Bool(!self.is_truthy(&val))),
            UnaryOp::BitNot => match val {
                Value::Int(n) => Ok(Value::Int(!n)),
                _ => Err(format!("TypeError: bad operand type for unary `~`: {}", self.type_name(&val))),
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
        match (op, &lv, &rv) {
            // 算術演算
            (BinOp::Add, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a + *b)),
            (BinOp::Add, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a + *b)),
            (BinOp::Add, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + *b)),
            (BinOp::Add, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a + *b as f64)),
            (BinOp::Add, Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
            (BinOp::Sub, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a - *b)),
            (BinOp::Sub, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a - *b)),
            (BinOp::Sub, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 - *b)),
            (BinOp::Sub, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a - *b as f64)),
            (BinOp::Mul, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a * *b)),
            (BinOp::Mul, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a * *b)),
            (BinOp::Mul, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 * *b)),
            (BinOp::Mul, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a * *b as f64)),
            (BinOp::Div, Value::Int(a), Value::Int(b)) => {
                if *b == 0 { return Err("ZeroDivisionError: division by zero".to_string()); }
                Ok(Value::Float(*a as f64 / *b as f64))
            }
            (BinOp::Div, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a / *b)),
            (BinOp::Div, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 / *b)),
            (BinOp::Div, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a / *b as f64)),
            (BinOp::FloorDiv, Value::Int(a), Value::Int(b)) => {
                if *b == 0 { return Err("ZeroDivisionError: integer division by zero".to_string()); }
                Ok(Value::Int(a.div_euclid(*b)))
            }
            (BinOp::Mod, Value::Int(a), Value::Int(b)) => {
                if *b == 0 { return Err("ZeroDivisionError: modulo by zero".to_string()); }
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
            // ビット演算（int のみ対応）
            (BinOp::BitAnd, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a & *b)),
            (BinOp::BitOr, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a | *b)),
            (BinOp::BitXor, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a ^ *b)),
            (BinOp::LShift, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a << *b)),
            (BinOp::RShift, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a >> *b)),
            _ => Err(format!(
                "TypeError: unsupported operand types for `{op:?}`: {} and {}",
                self.type_name(&lv), self.type_name(&rv)
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
            (Value::Float(a), Value::Float(b)) => a == b,
            // int と float の混在比較: int を float に昇格して比較
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::None, Value::None) => true,
            // インスタンスとクラスは参照の同一性（ポインタ比較）で等値判定
            (Value::Instance(a), Value::Instance(b)) => Rc::ptr_eq(a, b),
            (Value::Type(a), Value::Type(b)) => a == b,
            (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b),
            // タプルは要素数と各要素を再帰的に比較
            (Value::Tuple(a), Value::Tuple(b)) => {
                let av = a.all_values();
                let bv = b.all_values();
                av.len() == bv.len() && av.iter().zip(bv.iter()).all(|(x, y)| self.values_eq(x, y))
            }
            _ => false,
        }
    }
}
