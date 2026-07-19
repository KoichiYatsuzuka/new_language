// js_proc_runtime.rs — Runtime bridge for import[js-proc].
//
// Communication: newline-delimited JSON over a Windows named pipe.
// The shared pipe plumbing (spawn handshake, send/recv) lives in
// `super::proc_bridge`; this module holds only the js-proc request ops and the
// Value <-> JSON encoding.
//
// Launch sequence:
//   1. Arrow spawns:  node <bridge_script> <pipe_name> <bridge_root>
//   2. Node.js prints "READY\n" on stdout when the pipe server is listening.
//   3. Arrow opens the pipe as a synchronous duplex client.
//
// Request:  {"id":N,"op":"list"|"call"|"quit","module":"a/b","fn":"name","args":[...]}
// Response: {"id":N,"ok":{t,v}} | {"id":N,"err":"message"}
//
// Arg/result type tags:
//   "i" = int64    "f" = float64    "b" = bool
//   "s" = string   "n" = null       "a" = array

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::proc_bridge::{new_pipe_name, PipeConn};
use super::Value;

const TAG: &str = "js-proc";

// ── launch ───────────────────────────────────────────────────────────────────

/// Spawn the Node.js host and complete the named-pipe handshake.
fn launch(node_exe: &Path, bridge_script: &Path, bridge_root: &Path) -> Result<PipeConn, String> {
    let pipe_name = new_pipe_name("js");

    let child = std::process::Command::new(node_exe)
        .arg(bridge_script)
        .arg(&pipe_name)
        .arg(bridge_root)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{TAG}: cannot spawn node '{}': {e}", node_exe.display()))?;

    PipeConn::connect(child, &pipe_name, TAG)
}

// ── Global bridge registry (shared across threads) ───────────────────────────

fn global_bridges() -> &'static Mutex<HashMap<PathBuf, PipeConn>> {
    static BRIDGES: OnceLock<Mutex<HashMap<PathBuf, PipeConn>>> = OnceLock::new();
    BRIDGES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Launch and cache a bridge for the given bridge script.
pub fn launch_proc(node_exe: &Path, bridge_script: &Path, bridge_root: &Path)
    -> Result<(), String>
{
    let key = bridge_script
        .canonicalize()
        .unwrap_or_else(|_| bridge_script.to_path_buf());

    let mut map = global_bridges().lock()
        .map_err(|e| format!("{TAG}: bridge lock poisoned: {e}"))?;
    if map.contains_key(&key) {
        return Ok(());
    }
    let bridge = launch(node_exe, bridge_script, bridge_root)?;
    map.insert(key, bridge);
    Ok(())
}

// ── Public call API ───────────────────────────────────────────────────────────

/// Query the JS module for its exported function names.
pub fn list_functions(bridge_key: &str, module_name: &str)
    -> Result<Vec<String>, String>
{
    with_bridge(bridge_key, |bridge| {
        let ok = bridge.send_recv(
            serde_json::json!({
                "op":     "list",
                "module": module_name,
            }),
            TAG,
        )?;
        // ok = {t:"a", v:[{t:"s",v:"fn1"}, ...]}
        let arr = ok.get("v").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let names = arr.iter()
            .filter_map(|entry| {
                entry.get("v").and_then(|v| v.as_str()).map(|s| s.to_string())
            })
            .collect();
        Ok(names)
    })
}

/// Call a module-level function in the JS bridge.
pub fn call_function(
    bridge_key:  &str,
    module_name: &str,
    fn_name:     &str,
    args:        &[Value],
) -> Result<Value, String> {
    with_bridge(bridge_key, |bridge| {
        let ok = bridge.send_recv(
            serde_json::json!({
                "op":     "call",
                "module": module_name,
                "fn":     fn_name,
                "args":   encode_args(args),
            }),
            TAG,
        )?;
        Ok(decode_result(&ok))
    })
}

fn with_bridge<F, R>(bridge_key: &str, f: F) -> Result<R, String>
where
    F: FnOnce(&mut PipeConn) -> Result<R, String>,
{
    let key = PathBuf::from(bridge_key)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(bridge_key));
    let mut map = global_bridges().lock()
        .map_err(|e| format!("{TAG}: bridge lock poisoned: {e}"))?;
    let bridge = map.get_mut(&key).ok_or_else(|| {
        format!("{TAG}: bridge not launched for '{bridge_key}'")
    })?;
    f(bridge)
}

// ── Encoding / decoding ───────────────────────────────────────────────────────

fn encode_args(args: &[Value]) -> serde_json::Value {
    serde_json::Value::Array(args.iter().map(encode_arg).collect())
}

fn encode_arg(v: &Value) -> serde_json::Value {
    match v {
        Value::Int(n)  => serde_json::json!({"t":"i","v": n}),
        Value::UInt(n) => serde_json::json!({"t":"i","v": n}),
        Value::Float(f) => serde_json::json!({"t":"f","v": f}),
        Value::Bool(b)  => serde_json::json!({"t":"b","v": b}),
        Value::Str(s)   => serde_json::json!({"t":"s","v": s}),
        Value::None     => serde_json::json!({"t":"n"}),
        Value::List(rc) => {
            let arr: Vec<serde_json::Value> = rc.borrow().iter().map(encode_arg).collect();
            serde_json::json!({"t":"a","v": arr})
        }
        _ => serde_json::json!({"t":"n"}),
    }
}

fn decode_result(v: &serde_json::Value) -> Value {
    if v.is_null() { return Value::None; }
    let t   = v.get("t").and_then(|t| t.as_str()).unwrap_or("n");
    let val = v.get("v");
    match t {
        "s" => Value::Str(
            val.and_then(|v| v.as_str()).unwrap_or("").to_string(),
        ),
        "f" => Value::Float(val.and_then(|v| v.as_f64()).unwrap_or(0.0)),
        "b" => Value::Bool(val.and_then(|v| v.as_bool()).unwrap_or(false)),
        "n" => Value::None,
        "a" => {
            let items: Vec<Value> = val
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().map(decode_result).collect())
                .unwrap_or_default();
            Value::List(std::rc::Rc::new(std::cell::RefCell::new(items)))
        }
        "o" => {
            // Object: encode as Dict[str, str] (best-effort)
            // For a richer mapping, callers should extract specific keys.
            let items: Vec<Value> = val
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter().map(|(k, ov)| {
                        let decoded = decode_result(ov);
                        Value::Str(format!("{k}={}", value_to_str(&decoded)))
                    }).collect()
                })
                .unwrap_or_default();
            Value::List(std::rc::Rc::new(std::cell::RefCell::new(items)))
        }
        _ => {
            // "i" or unknown: integer
            Value::Int(val.and_then(|v| v.as_i64()).unwrap_or(0))
        }
    }
}

fn value_to_str(v: &Value) -> String {
    match v {
        Value::Str(s)   => s.clone(),
        Value::Int(n)   => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b)  => b.to_string(),
        Value::None     => "None".to_string(),
        _               => "<value>".to_string(),
    }
}
