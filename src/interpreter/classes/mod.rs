// classes/mod.rs — クラス・インスタンス管理サブシステムのモジュール束ね。
//
// クラスのインスタンス化、メソッド呼び出し、継承チェーン探索、組み込み型メソッドディスパッチを提供する。
// このファイルは共有の自由ヘルパー(ファイル引数解析・バイト変換)を保持し、役割別サブモジュールを宣言する。

use std::cell::RefCell;
use std::rc::Rc;

use super::{ByteModeRust, Value};

// ---------------------------------------------------------------------------
// FileObject メソッド用ヘルパー（自由関数）
// ---------------------------------------------------------------------------

/// `(backward: bool = default)` 形式の単一引数を解析する。
fn file_bool_arg(
    evaled: &[(Option<String>, Value, bool)],
    name: &str,
    default: bool,
) -> Result<bool, String> {
    match evaled {
        [] => Ok(default),
        [(kw_opt, val, _)] => {
            if let Some(k) = kw_opt {
                if k != name {
                    return Err(format!("TypeError: unexpected keyword argument '{k}'"));
                }
            }
            match val {
                Value::Bool(b) => Ok(*b),
                _ => Err(format!("TypeError: '{name}' must be bool")),
            }
        }
        _ => Err(format!("TypeError: {name}() takes at most one argument")),
    }
}

/// `(content)` 形式の単一引数を解析して値への参照を返す。
fn file_content_arg<'a>(
    evaled: &'a [(Option<String>, Value, bool)],
    name: &str,
) -> Result<&'a Value, String> {
    match evaled {
        [(kw_opt, val, _)] => {
            if let Some(k) = kw_opt {
                if k != name {
                    return Err(format!("TypeError: unexpected keyword argument '{k}'"));
                }
            }
            Ok(val)
        }
        [] => Err(format!("TypeError: missing required argument '{name}'")),
        _ => Err("TypeError: too many arguments".to_string()),
    }
}

/// `Vec<u8>` をバイトモードに応じた `Value` に変換する。
/// - Text モード: UTF-8 として `Value::Str` に変換
/// - Byte モード: バイト値のリスト `Value::List[Value::Int]` に変換
fn bytes_to_value(data: &[u8], byte_mode: &ByteModeRust) -> Value {
    match byte_mode {
        ByteModeRust::Text => Value::str(String::from_utf8_lossy(data).into_owned()),
        ByteModeRust::Byte => Value::List(Rc::new(RefCell::new(
            data.iter().map(|&b| Value::Int(b as i64)).collect(),
        ))),
    }
}

/// `Value` をバイトモードに応じた `Vec<u8>` に変換する。
/// - Text モード: `Value::Str` → UTF-8 バイト列
/// - Byte モード: `Value::List[Value::Int]` → バイト列
fn value_to_bytes(val: &Value, byte_mode: &ByteModeRust) -> Result<Vec<u8>, String> {
    match byte_mode {
        ByteModeRust::Text => match val {
            Value::Str(s) => Ok(s.as_bytes().to_vec()),
            _ => Err("write() content must be str in text mode".to_string()),
        },
        ByteModeRust::Byte => match val {
            Value::List(items) => {
                let mut out = Vec::new();
                for item in items.borrow().iter() {
                    match item {
                        Value::Int(n) if (0..=255).contains(n) => out.push(*n as u8),
                        Value::Int(n) => {
                            return Err(format!("write() byte value {n} out of range 0-255"))
                        }
                        _ => {
                            return Err("write() content must be list[int] in byte mode".to_string())
                        }
                    }
                }
                Ok(out)
            }
            _ => Err("write() content must be list[int] in byte mode".to_string()),
        },
    }
}


mod freeze;
mod instantiate;
mod method_call;
mod object_methods;
mod string_methods;
mod lookup;
