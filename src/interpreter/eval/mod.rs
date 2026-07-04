// eval/mod.rs — 式評価サブシステムのモジュール束ね。
//
// `Interpreter::eval` が式(`Expr`)を再帰的にツリーウォークして `Value` を返す。
// このファイルは共有の自由ヘルパー関数(スライス計算・パス/enum抽出など)を保持し、
// 役割別サブモジュール(core/calls/native/attrs/control_expr/subscript)を宣言する。

use super::{Interpreter, Value};

/// ヘルパー: セットに要素を重複なしで追加する。
fn set_insert(set: &mut Vec<Value>, item: Value, interp: &Interpreter) {
    if !set.iter().any(|v| interp.values_eq(v, &item)) {
        set.push(item);
    }
}

/// `mustbe` 用: ガード型文字列から外側の型名（型パラメータを除いた部分）を返す。
/// 例: `"list[int]"` → `"list"`,  `"function[int]->str"` → `"function"`, `"int"` → `"int"`
fn mustbe_outer_type(guard_type: &str) -> String {
    // `[` または `{` より前の部分を取り出す
    let end = guard_type.find(|c| c == '[' || c == '{').unwrap_or(guard_type.len());
    // `->` より前の部分も考慮（function->R の形式）
    let end = end.min(guard_type.find("->").unwrap_or(guard_type.len()));
    guard_type[..end].trim().to_string()
}

// ---------------------------------------------------------------------------
// スライス計算ヘルパー
// ---------------------------------------------------------------------------

/// step=1 スライス代入用: begin 境界を `[0, len]` にクランプして `usize` で返す。
fn normalize_slice_bound_start(begin: Option<i64>, len: i64) -> usize {
    match begin {
        None => 0,
        Some(i) if i < 0 => (i + len).max(0) as usize,
        Some(i) => i.min(len) as usize,
    }
}

/// step=1 スライス代入用: end 境界を `[0, len]` にクランプして `usize` で返す。
fn normalize_slice_bound_stop(end: Option<i64>, len: i64) -> usize {
    match end {
        None => len as usize,
        Some(i) if i < 0 => (i + len).max(0) as usize,
        Some(i) => i.min(len) as usize,
    }
}

/// `Optional[Index]` 値から i64 インデックスを取り出す。None または Value::None → None。
fn index_val_to_i64(val: &Option<Value>) -> Option<i64> {
    match val {
        None => None,
        Some(v) => value_as_index(v),
    }
}

/// `Value` を整数インデックスとして解釈する。
/// `Value::Int(n)` または `Index` インスタンス（`.value` フィールドが `int`）を受け入れる。
fn value_as_index(val: &Value) -> Option<i64> {
    match val {
        Value::Int(n) => Some(*n),
        Value::Instance(inst) => {
            let b = inst.borrow();
            if b.class.name == "Index" {
                if let Some(&idx) = b.class.field_index.get("value") {
                    if let Some(Value::Int(n)) = b.field_value(idx) {
                        return Some(n);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Python 互換のスライスインデックスリストを返す（`obj[begin:end:step]`）。
fn compute_slice_indices(len: i64, begin: Option<i64>, end: Option<i64>, step: i64) -> Vec<usize> {
    let (start, stop) = if step > 0 {
        let s = match begin {
            None => 0,
            Some(i) if i < 0 => (i + len).max(0),
            Some(i) => i.min(len),
        };
        let e = match end {
            None => len,
            Some(i) if i < 0 => (i + len).max(0),
            Some(i) => i.min(len),
        };
        (s, e)
    } else {
        let s = match begin {
            None => len - 1,
            Some(i) if i < 0 => (i + len).max(-1),
            Some(i) => i.min(len - 1),
        };
        let e = match end {
            None => -(len + 1),
            Some(i) if i < 0 => (i + len).max(-1),
            Some(i) => i.min(len - 1),
        };
        (s, e)
    };

    let mut result = Vec::new();
    let mut i = start;
    loop {
        if step > 0 {
            if i >= stop {
                break;
            }
        } else {
            if i <= stop || i < 0 {
                break;
            }
        }
        if i >= 0 && i < len {
            result.push(i as usize);
        }
        i += step;
    }
    result
}

// ---------------------------------------------------------------------------
// open() / close() ヘルパー
// ---------------------------------------------------------------------------

/// str または path インスタンスからファイルパス文字列を取り出す。
fn extract_path_str(val: &Value) -> Result<String, String> {
    match val {
        Value::Str(s) => Ok(s.clone()),
        Value::Instance(inst_rc) => {
            let inst = inst_rc.borrow();
            if inst.class.name == "path" {
                if let Some(&idx) = inst.class.field_index.get("value") {
                    if let Some(Value::Str(s)) = inst.field_value(idx) {
                        return Ok(s);
                    }
                }
            }
            Err(format!(
                "TypeError: open() 'file_path' must be str or path, got instance of '{}'",
                inst.class.name
            ))
        }
        other => Err(format!(
            "TypeError: open() 'file_path' must be str or path, got '{}'",
            match other {
                Value::Int(_) => "int",
                Value::Float(_) => "float",
                Value::Bool(_) => "bool",
                Value::None => "NoneType",
                _ => "other",
            }
        )),
    }
}

/// enum インスタンスの整数値を取り出す。クラス名が一致しない場合はエラー。
fn extract_enum_int(val: &Value, expected_class: &str) -> Result<i64, String> {
    if let Value::Instance(inst_rc) = val {
        let inst = inst_rc.borrow();
        if inst.class.name == expected_class {
            if let Some(&idx) = inst.class.field_index.get("value") {
                if let Some(Value::Int(n)) = inst.field_value(idx) {
                    return Ok(n);
                }
            }
        }
        return Err(format!(
            "TypeError: expected {expected_class} instance, got instance of '{}'",
            inst.class.name
        ));
    }
    Err(format!("TypeError: expected {expected_class} instance"))
}

/// 位置引数とキーワード引数のどちらからでも値を取り出すヘルパー。
fn get_arg<'a>(
    pos: &'a [Value],
    kw: &'a std::collections::HashMap<String, Value>,
    idx: usize,
    name: &str,
) -> Option<&'a Value> {
    kw.get(name).or_else(|| pos.get(idx))
}

mod core;
mod calls;
mod builtins;
mod native;
mod attrs;
mod control_expr;
mod subscript;
