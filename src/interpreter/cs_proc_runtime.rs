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

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    with_bridge(proc_path, |bridge| {
        let ok = bridge.send_recv(
            serde_json::json!({
                "op": "static",
                "cls": class_name,
                "mth": method,
                "args": encode_args(args),
            }),
            TAG,
        )?;
        Ok(decode_result(&ok, ret_type))
    })
}

/// Call a constructor via the proc bridge; returns a raw handle.
pub fn call_constructor(
    proc_path: &Path,
    class_name: &str,
    args: &[Value],
) -> Result<i64, String> {
    with_bridge(proc_path, |bridge| {
        let ok = bridge.send_recv(
            serde_json::json!({
                "op": "new",
                "cls": class_name,
                "args": encode_args(args),
            }),
            TAG,
        )?;
        // Response: {"t":"h","v": handle}
        Ok(ok.get("v").and_then(|v| v.as_i64()).unwrap_or(0))
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
    with_bridge(proc_path, |bridge| {
        let ok = bridge.send_recv(
            serde_json::json!({
                "op": "inst",
                "cls": class_name,
                "hnd": handle,
                "mth": method,
                "args": encode_args(args),
            }),
            TAG,
        )?;
        Ok(decode_result(&ok, ret_type))
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

fn encode_args(args: &[Value]) -> serde_json::Value {
    serde_json::Value::Array(args.iter().map(encode_arg).collect())
}

fn encode_arg(v: &Value) -> serde_json::Value {
    match v {
        Value::Int(n) => serde_json::json!({"t": "i", "v": n}),
        Value::UInt(n) => serde_json::json!({"t": "i", "v": n}),
        Value::Float(f) => serde_json::json!({"t": "f", "v": f}),
        Value::Bool(b) => serde_json::json!({"t": "b", "v": b}),
        Value::Str(s) => serde_json::json!({"t": "s", "v": &**s}),
        Value::CsObject(o) => serde_json::json!({"t": "h", "v": o.handle}),
        Value::None => serde_json::json!({"t": "n"}),
        _ => serde_json::json!({"t": "n"}),
    }
}

fn decode_result(v: &serde_json::Value, ret_type: Option<&str>) -> Value {
    if v.is_null() {
        return Value::None;
    }
    let t = v.get("t").and_then(|t| t.as_str()).unwrap_or("i");
    let val = v.get("v");
    match t {
        "s" => Value::str(val.and_then(|v| v.as_str()).unwrap_or("").to_string()),
        "f" => Value::Float(val.and_then(|v| v.as_f64()).unwrap_or(0.0)),
        "b" => Value::Bool(val.and_then(|v| v.as_bool()).unwrap_or(false)),
        "n" => Value::None,
        // "h": object handle returned as int; caller wraps in CsObject if needed
        _ => {
            let n = val.and_then(|v| v.as_i64()).unwrap_or(0);
            match ret_type {
                Some("float") => Value::Float(f64::from_bits(n as u64)),
                Some("bool") => Value::Bool(n != 0),
                Some("None") | Some("void") => Value::None,
                _ => Value::Int(n),
            }
        }
    }
}
