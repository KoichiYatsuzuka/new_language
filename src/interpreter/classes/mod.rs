// classes/mod.rs — クラス・インスタンス管理サブシステムのモジュール束ね。
//
// クラスのインスタンス化、メソッド呼び出し、継承チェーン探索、組み込み型メソッドディスパッチを提供する。
// このファイルは共有の自由ヘルパー(ファイル引数解析・バイト変換)を保持し、役割別サブモジュールを宣言する。

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

/// ファイルから読んだバイト列を値にする。
/// - Text モード: UTF-8 として `str`。⚠ 不正な UTF-8 は `IOError`（ffi_value_exchange_plan.md 7-11。以前は U+FFFD に黙って
///   置き換えていた。1 文字ずつ読む `read_letter` は以前からエラー）
/// - Byte モード: `bytes`（7-11。以前は `list[int]`。Python の `"rb"` と同じ）
fn bytes_to_value(data: &[u8], byte_mode: &ByteModeRust) -> Result<Value, String> {
    match byte_mode {
        ByteModeRust::Text => std::str::from_utf8(data)
            .map(|s| Value::str(s.to_string()))
            .map_err(|_| "IOError: invalid UTF-8 in file (open it with ByteRecognizingMode.byte to read bytes)".to_string()),
        ByteModeRust::Byte => Ok(Value::bytes(data.to_vec())),
    }
}

/// `Value` をバイトモードに応じた `Vec<u8>` に変換する（`write` / `write_line` の中身）。
/// - Text モード: `str` → UTF-8 のバイト列
/// - Byte モード: `bytes`（7-11。以前は `list[int]`。list は `bytes(xs)` で作るよう促す）
fn value_to_bytes(val: &Value, byte_mode: &ByteModeRust) -> Result<Vec<u8>, String> {
    match byte_mode {
        ByteModeRust::Text => match val {
            Value::Str(s) => Ok(s.as_bytes().to_vec()),
            _ => Err("content must be str in text mode".to_string()),
        },
        ByteModeRust::Byte => match val {
            Value::Bytes(b) => Ok(b.to_vec()),
            Value::List(_) => Err("content must be bytes in byte mode (got a list; write `bytes(xs)`)".to_string()),
            _ => Err("content must be bytes in byte mode".to_string()),
        },
    }
}


mod freeze;
mod instantiate;
pub(crate) use instantiate::AutoInitSlot;
mod lookup;
mod method_call;
// レシーバ種別ごとのメソッドディスパッチ（#63 で `method_call.rs` から切り出し）。
// ⚠ **1 レシーバ = 1 ファイル**。`eval_method_call_full` のアームは委譲 1〜7 行に保つこと。
mod async_manager_methods;
mod bytes_methods;
mod ptr_methods;
mod class_methods;
mod frozen_list_methods;
mod object_methods;
mod set_methods;
mod string_methods;
