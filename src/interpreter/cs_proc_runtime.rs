// cs_proc_runtime.rs — Runtime bridge for import[cs-proc].
//
// Communication: newline-delimited JSON over a Windows named pipe.
// The shared pipe plumbing (spawn handshake, send/recv) lives in
// `super::proc_bridge`; this module holds only the cs-proc request ops and the
// Value <-> JSON encoding.
//
// Request:  {"id":N,"op":"static"|"new"|"inst"|"quit","cls":"ClassName","mth":"method","hnd":handle,"args":[...]}
// Response: {"id":N,"ok":<value>} | {"id":N,"err":"message"}
//
// Arg/result type tags:
//   "i" = int64    "f" = float64    "b" = bool
//   "s" = string   "h" = handle     "n" = null
//   "o" = (result only) an integer that does not fit in int64 (C# ulong ≥ 2^63), as a decimal string
//
// What each declared type may receive is decided in `super::cs_values` (shared with cs-dll · plan 2-1).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::cs_values::{self, CsDecl, CsKind};
use super::proc_bridge::{new_pipe_name, PipeConn};
use super::Value;

const TAG: &str = "cs-proc";

// ── launch ───────────────────────────────────────────────────────────────────

/// Spawn the C# host for `proc_path` and complete the named-pipe handshake.
fn launch(proc_path: &Path) -> Result<PipeConn, String> {
    let pipe_name = new_pipe_name("cs");

    // Spawn the C# host; it creates the named-pipe server and prints "READY".
    let child = std::process::Command::new(proc_path)
        .arg(&pipe_name)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{TAG}: cannot spawn '{}': {e}", proc_path.display()))?;

    PipeConn::connect(child, &pipe_name, TAG)
}

// ── Thread-local proc registry ───────────────────────────────────────────────

thread_local! {
    static CS_PROCS: RefCell<HashMap<PathBuf, PipeConn>> = RefCell::new(HashMap::new());
}

/// Launch (and cache) a proc bridge for the given executable path.
pub fn launch_proc(proc_path: &Path) -> Result<(), String> {
    let canon = proc_path
        .canonicalize()
        .unwrap_or_else(|_| proc_path.to_path_buf());
    CS_PROCS.with(|m| {
        let mut map = m.borrow_mut();
        if map.contains_key(&canon) {
            return Ok(());
        }
        let bridge = launch(proc_path)?;
        map.insert(canon, bridge);
        Ok(())
    })
}

// ── Public call API ──────────────────────────────────────────────────────────

/// Call a static method via the proc bridge.
pub fn call_static(
    proc_path: &Path,
    class_name: &str,
    method: &str,
    args: &[Value],
    ret_type: Option<&str>,
) -> Result<Value, String> {
    let what = format!("{class_name}.{method}");
    let decl = cs_values::decl_of(ret_type)?;
    let args = encode_args(proc_path, args, &what)?;
    with_bridge(proc_path, |bridge| {
        let ok = bridge.send_recv(
            serde_json::json!({
                "op": "static",
                "cls": class_name,
                "mth": method,
                "args": args,
            }),
            TAG,
        )?;
        decode_result(&ok, &decl, proc_path, &what)
    })
}

/// Call a constructor via the proc bridge; returns a raw handle.
pub fn call_constructor(
    proc_path: &Path,
    class_name: &str,
    args: &[Value],
) -> Result<i64, String> {
    let what = format!("{class_name}.__init__");
    let args = encode_args(proc_path, args, &what)?;
    with_bridge(proc_path, |bridge| {
        let ok = bridge.send_recv(
            serde_json::json!({
                "op": "new",
                "cls": class_name,
                "args": args,
            }),
            TAG,
        )?;
        // Response: {"t":"h","v": handle}。⚠ 以前は読めなければ 0（無いオブジェクト）にしていた。
        match ok.get("v").and_then(|v| v.as_i64()) {
            Some(h) if h > 0 => Ok(h),
            _ => Err(format!("{TAG}: the host did not return an object for '{class_name}' (got {ok})")),
        }
    })
}

/// Call an instance method via the proc bridge.
pub fn call_instance(
    proc_path: &Path,
    class_name: &str,
    handle: i64,
    method: &str,
    args: &[Value],
    ret_type: Option<&str>,
) -> Result<Value, String> {
    let what = format!("{class_name}.{method}");
    let decl = cs_values::decl_of(ret_type)?;
    let args = encode_args(proc_path, args, &what)?;
    with_bridge(proc_path, |bridge| {
        let ok = bridge.send_recv(
            serde_json::json!({
                "op": "inst",
                "cls": class_name,
                "hnd": handle,
                "mth": method,
                "args": args,
            }),
            TAG,
        )?;
        decode_result(&ok, &decl, proc_path, &what)
    })
}

fn with_bridge<F, R>(proc_path: &Path, f: F) -> Result<R, String>
where
    F: FnOnce(&mut PipeConn) -> Result<R, String>,
{
    let canon = proc_path
        .canonicalize()
        .unwrap_or_else(|_| proc_path.to_path_buf());
    CS_PROCS.with(|m| {
        let mut map = m.borrow_mut();
        let bridge = map.get_mut(&canon).ok_or_else(|| {
            format!("{TAG}: bridge not launched for '{}'", proc_path.display())
        })?;
        f(bridge)
    })
}

// ── Encoding / decoding ──────────────────────────────────────────────────────

/// 引数を JSON にする。運べない値（list / dict / インスタンス …）はエラー（2-1）。
/// ⚠ 以前は `_ => null` で、リストを渡すと C# 側に `null` が届いていた。
fn encode_args(proc_path: &Path, args: &[Value], what: &str) -> Result<serde_json::Value, String> {
    let mut out = Vec::with_capacity(args.len());
    for (i, v) in args.iter().enumerate() {
        out.push(match v {
            Value::Int(n) => serde_json::json!({"t": "i", "v": n}),
            Value::UInt(n) => serde_json::json!({"t": "i", "v": n}),
            Value::Float(f) => serde_json::json!({"t": "f", "v": f}),
            Value::Bool(b) => serde_json::json!({"t": "b", "v": b}),
            Value::Str(s) => serde_json::json!({"t": "s", "v": &**s}),
            Value::CsObject(o) => {
                cs_values::check_same_bridge(o, proc_path, true, i, what)?;
                serde_json::json!({"t": "h", "v": o.handle})
            }
            Value::None => serde_json::json!({"t": "n"}),
            other => return Err(cs_values::unsupported_arg(other, i, what)),
        });
    }
    Ok(serde_json::Value::Array(out))
}

/// ホストの返した値を、宣言された型で受け取る（2-1）。
/// ⚠ 以前は宣言を見ずにタグだけで読み、オブジェクトのハンドル（`"h"`）は整数に、読めない値は `""` / `0` にしていた。
fn decode_result(v: &serde_json::Value, decl: &CsDecl, proc_path: &Path, what: &str) -> Result<Value, String> {
    if v.is_null() {
        return cs_values::null_result(decl, what);
    }
    let malformed = || format!("{TAG}: malformed result from the host for '{what}': {v}");
    let t = v.get("t").and_then(|t| t.as_str()).ok_or_else(malformed)?;
    let val = v.get("v");
    let any = decl.kind == CsKind::Any;
    match t {
        "n" => cs_values::null_result(decl, what),
        "i" if decl.kind == CsKind::Int || any => {
            Ok(Value::Int(val.and_then(|v| v.as_i64()).ok_or_else(malformed)?))
        }
        "f" if decl.kind == CsKind::Float || any => {
            Ok(Value::Float(val.and_then(|v| v.as_f64()).ok_or_else(malformed)?))
        }
        "b" if decl.kind == CsKind::Bool || any => {
            Ok(Value::Bool(val.and_then(|v| v.as_bool()).ok_or_else(malformed)?))
        }
        "s" if decl.kind == CsKind::Str || any => {
            Ok(Value::str(val.and_then(|v| v.as_str()).ok_or_else(malformed)?.to_string()))
        }
        // C# の ulong の 2^63 以上（cpp ブリッジの `unsigned long long` と同じ扱い・1-6）
        "o" if decl.kind == CsKind::Int || any => Err(format!(
            "OverflowError: C# method '{what}' returned {}, which does not fit in int",
            val.and_then(|v| v.as_str()).unwrap_or("?")
        )),
        "h" => {
            let h = val.and_then(|v| v.as_i64()).ok_or_else(malformed)?;
            cs_values::object_result(decl, proc_path, true, h, what)
        }
        "i" | "f" | "b" | "s" | "o" => {
            let got = match t {
                "i" | "o" => "an integer",
                "f" => "a float",
                "b" => "a bool",
                _ => "a string",
            };
            Err(cs_values::mismatched_result(decl, got, what))
        }
        _ => Err(malformed()),
    }
}
