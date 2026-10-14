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
// Arg/result type tags: `super::proc_values` (shared with js-proc · plan 5-2): i / f / b / s / n / h and the copied
// containers a (list) / u (tuple) / d (dict, [[key, value], …]) / e (set) / y (bytes, base64).
//   "o" = (result only) an integer that does not fit in int64 (C# ulong ≥ 2^63), as a decimal string
//
// What each declared type may receive is decided in `super::cs_values` (shared with cs-dll · plan 2-1).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::cs_values::{self, CsDecl};
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

// ── Process-wide proc registry ───────────────────────────────────────────────

/// The hosts, keyed by their canonical path. ⚠ Process-wide (ffi_value_exchange_plan.md 6-2, like js-proc): it used to
/// be thread-local, so a call from an async task failed with `bridge not launched`. One host serves every thread; the
/// mutex serialises the requests on its pipe.
fn procs() -> &'static std::sync::Mutex<HashMap<PathBuf, PipeConn>> {
    static CS_PROCS: std::sync::OnceLock<std::sync::Mutex<HashMap<PathBuf, PipeConn>>> = std::sync::OnceLock::new();
    CS_PROCS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Launch (and cache) a proc bridge for the given executable path.
pub fn launch_proc(proc_path: &Path) -> Result<(), String> {
    let canon = proc_path
        .canonicalize()
        .unwrap_or_else(|_| proc_path.to_path_buf());
    let mut map = procs().lock().map_err(|_| format!("{TAG}: the host registry is poisoned"))?;
    if map.contains_key(&canon) {
        return Ok(());
    }
    let bridge = launch(proc_path)?;
    map.insert(canon, bridge);
    Ok(())
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
    let mut map = procs().lock().map_err(|_| format!("{TAG}: the host registry is poisoned"))?;
    let bridge = map.get_mut(&canon).ok_or_else(|| {
        format!("{TAG}: bridge not launched for '{}'", proc_path.display())
    })?;
    f(bridge)
}

// ── Encoding / decoding ──────────────────────────────────────────────────────

/// 引数を JSON にする（`proc_values`・コレクションと bytes はコピー・5-2）。運べない値はエラー。
/// ⚠ 以前は `_ => null` で、リストを渡すと C# 側に `null` が届いていた（2-1 でエラーにし、5-2 でコピーで運ぶようにした）。
fn encode_args(proc_path: &Path, args: &[Value], what: &str) -> Result<serde_json::Value, String> {
    let peer = super::proc_values::Peer::Cs(proc_path);
    let mut out = Vec::with_capacity(args.len());
    for (i, v) in args.iter().enumerate() {
        out.push(
            super::proc_values::encode(v, &peer)
                .map_err(|e| e.message(i, &format!("C# method '{what}'"), "cs-proc"))?,
        );
    }
    Ok(serde_json::Value::Array(out))
}

/// ホストの返した値を、宣言された型で受け取る（2-1・5-2。規則は cs-dll と共有・[`cs_values::decode_tagged`]）。
fn decode_result(v: &serde_json::Value, decl: &CsDecl, proc_path: &Path, what: &str) -> Result<Value, String> {
    cs_values::decode_tagged(v, decl, proc_path, true, what)
}
