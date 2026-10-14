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
// Arg/result type tags: `super::proc_values` (shared with cs-proc · plan 5-2): i / f / b / s / n and the copied
// containers a (array) / u (tuple → a JS array) / d (dict → a JS object when every key is a str, else a Map;
// [[key, value], …]) / e (set → Set) / y (bytes → Buffer, base64).
//   "o" = (result only) a plain JS object, {key: {t,v}, …} → dict[str, …]
//   "x" = (result only) what cannot be carried: a function / symbol / BigInt / an instance of a class (payload: its
//         typeof or class name) — an error
//
// Values the bridge cannot carry are errors in both directions (ffi_value_exchange_plan.md 2-3): a dict / tuple /
// set / instance argument used to reach JS as `null`, and an object result became a list of "k=v" strings.
// Since 5-2 dicts / tuples / sets / bytes are copied; Arrow instances, functions and Ptr are still errors.

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
        out.push(
            super::proc_values::encode(v, &super::proc_values::Peer::Js)
                .map_err(|e| e.message(i, &format!("JS function '{what}'"), "js-proc"))?,
        );
    }
    Ok(serde_json::Value::Array(out))
}

/// JS の返した値を Arrow の値にする（2-3）。宣言との照合と数値の寄せは `ffi_boundary`。
/// ⚠ 以前はオブジェクトを `"k=v"` の文字列のリストにし、読めない値（int64 に入らない整数・NaN など）を `0` / `0.0` に、
///   知らないタグを整数にしていた。どれもエラーにする。オブジェクト・Map・Set・Uint8Array は 5-2 からコピーで受け取る。
fn decode_result(v: &serde_json::Value, what: &str) -> Result<Value, String> {
    if v.is_null() {
        return Ok(Value::None);
    }
    let malformed = || format!("{TAG}: malformed result from the bridge for '{what}': {v}");
    let t = v.get("t").and_then(|t| t.as_str()).ok_or_else(malformed)?;
    let val = v.get("v");
    let items = || val.and_then(|v| v.as_array()).ok_or_else(malformed);
    let each = |xs: &Vec<serde_json::Value>| xs.iter().map(|e| decode_result(e, what)).collect::<Result<Vec<_>, _>>();
    Ok(match t {
        "n" => Value::None,
        "s" => Value::str(val.and_then(|v| v.as_str()).ok_or_else(malformed)?),
        "b" => Value::Bool(val.and_then(|v| v.as_bool()).ok_or_else(malformed)?),
        "i" => Value::Int(val.and_then(|v| v.as_i64()).ok_or_else(malformed)?),
        "f" => Value::Float(super::proc_values::decode_float(val).ok_or_else(malformed)?),
        "a" => Value::List(std::rc::Rc::new(std::cell::RefCell::new(each(items()?)?))),
        "e" => super::proc_values::make_set(each(items()?)?).map_err(|e| in_result(e, what))?,
        "d" => {
            let pairs = items()?
                .iter()
                .map(|p| match p.as_array().map(Vec::as_slice) {
                    Some([k, x]) => Ok((decode_result(k, what)?, decode_result(x, what)?)),
                    _ => Err(malformed()),
                })
                .collect::<Result<Vec<_>, String>>()?;
            super::proc_values::make_dict(pairs).map_err(|e| in_result(e, what))?
        }
        // 素のオブジェクト → dict[str, …]
        "o" => {
            let obj = val.and_then(|v| v.as_object()).ok_or_else(malformed)?;
            let pairs = obj
                .iter()
                .map(|(k, x)| Ok((Value::str(k.as_str()), decode_result(x, what)?)))
                .collect::<Result<Vec<_>, String>>()?;
            super::proc_values::make_dict(pairs).map_err(|e| in_result(e, what))?
        }
        "y" => Value::bytes(
            super::proc_values::base64_decode(val.and_then(|v| v.as_str()).ok_or_else(malformed)?)
                .map_err(|e| format!("{TAG}: {e} from the bridge for '{what}'"))?,
        ),
        "x" => {
            return Err(format!(
                "TypeError: JS function '{what}' returned (or its result contains) a JS {}, which cannot cross the js-proc bridge",
                val.and_then(|v| v.as_str()).unwrap_or("value")
            ))
        }
        _ => return Err(malformed()),
    })
}

/// 受け取った値を Arrow の dict / set にできなかったとき（キーが None など）。
fn in_result(e: String, what: &str) -> String {
    format!("{e} (in the result of JS function '{what}')")
}
