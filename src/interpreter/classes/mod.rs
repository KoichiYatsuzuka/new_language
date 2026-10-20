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

// ---------------------------------------------------------------------------
// Python のファイルのメソッド用（python_builtins_plan.md のタスク 5-4・`exec_file_py_method`）
// ---------------------------------------------------------------------------

/// 引数を取らないメソッド（`close()` / `flush()` / `tell()`）。
fn file_no_args(evaled: &[(Option<String>, Value, bool)], name: &str) -> Result<(), String> {
    if evaled.is_empty() {
        Ok(())
    } else {
        Err(format!("TypeError: {name}() takes no arguments ({} given)", evaled.len()))
    }
}

/// `read(size=-1)` / `readline(size=-1)` / `readlines(hint=-1)` の数。負・`None`・省略は「制限なし」。
fn file_size_arg(
    evaled: &[(Option<String>, Value, bool)],
    method: &str,
    kw_name: &str,
) -> Result<Option<usize>, String> {
    match evaled {
        [] => Ok(None),
        [(kw, v, _)] => {
            if let Some(k) = kw.as_deref().filter(|k| *k != kw_name) {
                return Err(format!("TypeError: {method}() got an unexpected keyword argument '{k}'"));
            }
            match v {
                Value::None => Ok(None),
                Value::Int(n) => Ok(usize::try_from(*n).ok()),
                Value::Bool(b) => Ok(Some(usize::from(*b))),
                other => Err(format!(
                    "TypeError: argument should be integer or None, not '{}'",
                    crate::interpreter::ops::typecheck::runtime_type_name(other)
                )),
            }
        }
        _ => Err(format!("TypeError: {method} expected at most 1 argument, got {}", evaled.len())),
    }
}

/// 位置から読む範囲の終わり（バイトの添字）。`line` なら `\n` まで（含む）。`size` はテキストなら文字の数
/// （UTF-8 の続きのバイトを飛ばして数える）、バイト列ならバイトの数。
fn file_read_end(fd: &super::FileData, size: Option<usize>, line: bool) -> usize {
    let start = fd.pointer;
    let content = &fd.content;
    let limit = if line {
        content[start..].iter().position(|&b| b == b'\n').map_or(content.len(), |i| start + i + 1)
    } else {
        content.len()
    };
    let Some(n) = size else { return limit };
    match fd.byte_mode {
        ByteModeRust::Byte => (start + n).min(limit),
        ByteModeRust::Text => {
            let mut i = start;
            let mut count = 0;
            while i < limit && count < n {
                i += 1;
                while i < limit && (content[i] & 0xC0) == 0x80 {
                    i += 1;
                }
                count += 1;
            }
            i
        }
    }
}

/// 読んだバイトを値にする。テキストは `str`（不正な UTF-8 は、Python の形は CPython と同じく `UnicodeDecodeError`、
/// Arrow の形は `IOError`・ffi_value_exchange_plan.md 7-11）。バイト列はどちらの形も `bytes`（`bytes_to_value`）。
fn file_bytes_value(data: &[u8], fd: &super::FileData) -> Result<Value, String> {
    match (&fd.byte_mode, fd.py_mode.is_some()) {
        (ByteModeRust::Text, true) => {
            crate::interpreter::ops::codecs::decode(data, crate::interpreter::ops::codecs::Codec::Utf8, "strict")
                .map(Value::str)
        }
        (mode, _) => bytes_to_value(data, mode),
    }
}

/// 書く値をバイトにする。返すのは（バイト, 書いた数＝テキストは文字・バイト列はバイト）。
fn file_encode(val: &Value, fd: &super::FileData) -> Result<(Vec<u8>, usize), String> {
    let type_name = crate::interpreter::ops::typecheck::runtime_type_name(val);
    match (&fd.byte_mode, val) {
        (ByteModeRust::Text, Value::Str(s)) => Ok((s.as_bytes().to_vec(), s.chars().count())),
        (ByteModeRust::Byte, Value::Bytes(b)) if fd.py_mode.is_some() => Ok((b.to_vec(), b.len())),
        (ByteModeRust::Text, _) if fd.py_mode.is_some() => {
            Err(format!("TypeError: write() argument must be str, not {type_name}"))
        }
        (ByteModeRust::Byte, _) if fd.py_mode.is_some() => {
            Err(format!("TypeError: a bytes-like object is required, not '{type_name}'"))
        }
        (mode, v) => {
            let bytes = value_to_bytes(v, mode).map_err(|e| format!("TypeError: write(): {e}"))?;
            let n = bytes.len();
            Ok((bytes, n))
        }
    }
}

/// 位置へ書いて位置を進める（Arrow の `write` と同じ**挿入**。末尾なら追記）。
fn file_insert(fd: &mut super::FileData, bytes: &[u8]) {
    let ptr = fd.pointer;
    let rest = fd.content.split_off(ptr);
    fd.content.extend_from_slice(bytes);
    fd.content.extend_from_slice(&rest);
    fd.pointer = ptr + bytes.len();
}

mod freeze;
mod instantiate;
mod lookup;
mod method_call;
// レシーバ種別ごとのメソッドディスパッチ（#63 で `method_call.rs` から切り出し）。
// ⚠ **1 レシーバ = 1 ファイル**。`eval_method_call_full` のアームは委譲 1〜7 行に保つこと。
mod async_manager_methods;
mod bytes_methods;
mod ptr_methods;
mod class_methods;
mod dict_methods;
mod frozen_list_methods;
mod object_methods;
mod set_methods;
mod string_methods;
