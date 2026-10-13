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
//   "f" with a string payload "NaN" / "Infinity" / "-Infinity" (JSON has no such numbers)
//   "o" = (result only) a JS object — it cannot cross the bridge yet (plan 5-2), so decoding it is an error
//   "x" = (result only) a function / symbol / BigInt (payload: its typeof) — an error too
//
// Values the bridge cannot carry are errors in both directions (ffi_value_exchange_plan.md 2-3): a dict / tuple /
// set / instance argument used to reach JS as `null`, and an object result became a list of "k=v" strings.

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
//
// ⚠ This table is a static and is never dropped, so `PipeConn`'s `Drop` (which sends "quit") does not run at exit.
//   The bridge therefore exits by itself when the pipe closes (`bridge/js_bridge.cjs`, socket 'close'), which also
//   covers Arrow crashing or being killed (ffi_value_exchange_plan.md 2-4). Before that, one node process was left
//   behind per run.

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
    // メッセージ用の名前。`module_name` は解決した JS ファイルの絶対パスのことがあるので、ファイル名の幹だけ出す。
    let short = Path::new(module_name).file_stem().and_then(|s| s.to_str()).unwrap_or(module_name);
    let what = format!("{short}.{fn_name}");
    let args = encode_args(args, &what)?;
    with_bridge(bridge_key, |bridge| {
        let ok = bridge.send_recv(
            serde_json::json!({
                "op":     "call",
                "module": module_name,
                "fn":     fn_name,
                "args":   args,
            }),
            TAG,
        )?;
        decode_result(&ok, &what)
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

fn encode_args(args: &[Value], what: &str) -> Result<serde_json::Value, String> {
    let mut out = Vec::with_capacity(args.len());
    for (i, v) in args.iter().enumerate() {
        out.push(encode_arg(v).map_err(|got| {
            format!(
                "TypeError: argument {} of JS function '{what}' is (or contains) a '{got}', which cannot cross the js-proc bridge yet",
                i + 1
            )
        })?);
    }
    Ok(serde_json::Value::Array(out))
}

/// 1 つの値を JSON にする。運べない値は、その型名を `Err` で返す。
/// ⚠ 以前は `_ => null` で、dict / tuple / set / インスタンスを渡すと JS 側に `null` が届いていた（2-3）。
fn encode_arg(v: &Value) -> Result<serde_json::Value, &'static str> {
    Ok(match v {
        Value::Int(n)  => serde_json::json!({"t":"i","v": n}),
        Value::UInt(n) => serde_json::json!({"t":"i","v": n}),
        Value::Float(f) if f.is_finite() => serde_json::json!({"t":"f","v": f}),
        // JSON に NaN / ±Infinity は無い（serde_json は null にしてしまう）ので文字列で送る
        Value::Float(f) => serde_json::json!({"t":"f","v": non_finite_name(*f)}),
        Value::Bool(b)  => serde_json::json!({"t":"b","v": b}),
        Value::Str(s)   => serde_json::json!({"t":"s","v": &**s}),
        Value::None     => serde_json::json!({"t":"n"}),
        Value::List(rc) => {
            let arr = rc.borrow().iter().map(encode_arg).collect::<Result<Vec<_>, _>>()?;
            serde_json::json!({"t":"a","v": arr})
        }
        other => return Err(crate::interpreter::ops::typecheck::runtime_type_name(other)),
    })
}

fn non_finite_name(f: f64) -> &'static str {
    if f.is_nan() { "NaN" } else if f > 0.0 { "Infinity" } else { "-Infinity" }
}

/// JS の返した値を Arrow の値にする（2-3）。
/// ⚠ 以前はオブジェクトを `"k=v"` の文字列のリストにし、読めない値（int64 に入らない整数・NaN など）を `0` / `0.0` に、
///   知らないタグを整数にしていた。どれもエラーにする（オブジェクトの受け渡しはフェーズ 5・5-2）。
fn decode_result(v: &serde_json::Value, what: &str) -> Result<Value, String> {
    if v.is_null() {
        return Ok(Value::None);
    }
    let malformed = || format!("{TAG}: malformed result from the bridge for '{what}': {v}");
    let t = v.get("t").and_then(|t| t.as_str()).ok_or_else(malformed)?;
    let val = v.get("v");
    Ok(match t {
        "n" => Value::None,
        "s" => Value::str(val.and_then(|v| v.as_str()).ok_or_else(malformed)?),
        "b" => Value::Bool(val.and_then(|v| v.as_bool()).ok_or_else(malformed)?),
        "i" => Value::Int(val.and_then(|v| v.as_i64()).ok_or_else(malformed)?),
        "f" => match val {
            Some(serde_json::Value::String(s)) => match s.as_str() {
                "NaN" => Value::Float(f64::NAN),
                "Infinity" => Value::Float(f64::INFINITY),
                "-Infinity" => Value::Float(f64::NEG_INFINITY),
                _ => return Err(malformed()),
            },
            Some(n) => Value::Float(n.as_f64().ok_or_else(malformed)?),
            None => return Err(malformed()),
        },
        "a" => {
            let arr = val.and_then(|v| v.as_array()).ok_or_else(malformed)?;
            let items = arr.iter().map(|e| decode_result(e, what)).collect::<Result<Vec<_>, _>>()?;
            Value::List(std::rc::Rc::new(std::cell::RefCell::new(items)))
        }
        "x" => {
            return Err(format!(
                "TypeError: JS function '{what}' returned (or its result contains) a JS {}, which cannot cross the js-proc bridge",
                val.and_then(|v| v.as_str()).unwrap_or("value")
            ))
        }
        "o" => {
            return Err(format!(
                "TypeError: JS function '{what}' returned (or its result contains) an object, which cannot cross the js-proc bridge yet (return an array or primitive values)"
            ))
        }
        _ => return Err(malformed()),
    })
}
