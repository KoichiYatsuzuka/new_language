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

/// ホストの返した値を、宣言された型で受け取る（2-1）。コレクションは要素の宣言で読む（5-2）。
/// ⚠ 以前は宣言を見ずにタグだけで読み、オブジェクトのハンドル（`"h"`）は整数に、読めない値は `""` / `0` にしていた。
fn decode_result(v: &serde_json::Value, decl: &CsDecl, proc_path: &Path, what: &str) -> Result<Value, String> {
    if v.is_null() {
        return cs_values::null_result(decl, what);
    }
    let malformed = || format!("{TAG}: malformed result from the host for '{what}': {v}");
    let t = v.get("t").and_then(|t| t.as_str()).ok_or_else(malformed)?;
    let val = v.get("v");
    let any_decl = CsDecl { kind: CsKind::Any, optional: true, shown: "Any".to_string() };
    let items = || val.and_then(|v| v.as_array()).ok_or_else(malformed);
    // 要素を、その宣言で読む
    let each = |xs: &Vec<serde_json::Value>, d: &CsDecl| -> Result<Vec<Value>, String> {
        xs.iter().map(|x| decode_result(x, d, proc_path, what)).collect()
    };
    match (t, &decl.kind) {
        ("n", _) => cs_values::null_result(decl, what),
        ("i", CsKind::Int | CsKind::Any) => Ok(Value::Int(val.and_then(|v| v.as_i64()).ok_or_else(malformed)?)),
        ("f", CsKind::Float | CsKind::Any) => {
            Ok(Value::Float(super::proc_values::decode_float(val).ok_or_else(malformed)?))
        }
        ("b", CsKind::Bool | CsKind::Any) => Ok(Value::Bool(val.and_then(|v| v.as_bool()).ok_or_else(malformed)?)),
        ("s", CsKind::Str | CsKind::Any) => {
            Ok(Value::str(val.and_then(|v| v.as_str()).ok_or_else(malformed)?.to_string()))
        }
        // C# の ulong の 2^63 以上（cpp ブリッジの `unsigned long long` と同じ扱い・1-6）
        ("o", CsKind::Int | CsKind::Any) => Err(format!(
            "OverflowError: C# method '{what}' returned {}, which does not fit in int",
            val.and_then(|v| v.as_str()).unwrap_or("?")
        )),
        ("h", _) => {
            let h = val.and_then(|v| v.as_i64()).ok_or_else(malformed)?;
            cs_values::object_result(decl, proc_path, true, h, what)
        }
        // コピーで運んだコレクション・bytes（5-2）
        ("a", CsKind::List(e)) => Ok(Value::List(std::rc::Rc::new(RefCell::new(each(items()?, e)?)))),
        ("a", CsKind::Any) => Ok(Value::List(std::rc::Rc::new(RefCell::new(each(items()?, &any_decl)?)))),
        ("u", CsKind::Tuple(es)) => {
            let xs = items()?;
            if xs.len() != es.len() {
                return Err(cs_values::mismatched_result(decl, &format!("a tuple of {}", xs.len()), what));
            }
            let vals = xs.iter().zip(es).map(|(x, d)| decode_result(x, d, proc_path, what)).collect::<Result<_, _>>()?;
            Ok(super::proc_values::make_tuple(vals))
        }
        ("u", CsKind::Any) => Ok(super::proc_values::make_tuple(each(items()?, &any_decl)?)),
        ("e", CsKind::Set(e)) => super::proc_values::make_set(each(items()?, e)?),
        ("e", CsKind::Any) => super::proc_values::make_set(each(items()?, &any_decl)?),
        ("d", CsKind::Dict(..) | CsKind::Any) => {
            let (kd, vd) = match &decl.kind {
                CsKind::Dict(k, x) => (&**k, &**x),
                _ => (&any_decl, &any_decl),
            };
            let pairs = items()?
                .iter()
                .map(|p| match p.as_array().map(Vec::as_slice) {
                    Some([k, x]) => Ok((decode_result(k, kd, proc_path, what)?, decode_result(x, vd, proc_path, what)?)),
                    _ => Err(malformed()),
                })
                .collect::<Result<Vec<_>, String>>()?;
            super::proc_values::make_dict(pairs)
        }
        ("y", CsKind::Bytes | CsKind::Any) => {
            let b = super::proc_values::base64_decode(val.and_then(|v| v.as_str()).ok_or_else(malformed)?)
                .map_err(|e| format!("{TAG}: {e} from the host for '{what}'"))?;
            Ok(Value::bytes(b))
        }
        ("i" | "f" | "b" | "s" | "o" | "a" | "u" | "e" | "d" | "y", _) => {
            let got = match t {
                "i" | "o" => "an integer",
                "f" => "a float",
                "b" => "a bool",
                "s" => "a string",
                "a" => "a list",
                "u" => "a tuple",
                "e" => "a set",
                "d" => "a dict",
                _ => "bytes",
            };
            Err(cs_values::mismatched_result(decl, got, what))
        }
        _ => Err(malformed()),
    }
}
