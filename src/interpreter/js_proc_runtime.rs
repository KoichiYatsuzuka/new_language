// js_proc_runtime.rs — Runtime bridge for import[js-proc].
//
// Communication: newline-delimited JSON over a Windows named pipe.
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
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use super::Value;

// ── Pipe name generator ──────────────────────────────────────────────────────

static PIPE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn new_pipe_name() -> String {
    let pid = std::process::id();
    let n = PIPE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(r"\\.\pipe\arrow_js_{pid}_{n}")
}

// ── JsBridge ─────────────────────────────────────────────────────────────────

pub struct JsBridge {
    _child:  std::process::Child,
    reader:  BufReader<std::fs::File>,
    writer:  BufWriter<std::fs::File>,
    next_id: u64,
}

impl JsBridge {
    pub fn launch(node_exe: &Path, bridge_script: &Path, bridge_root: &Path)
        -> Result<Self, String>
    {
        let pipe_name = new_pipe_name();

        let mut child = std::process::Command::new(node_exe)
            .arg(bridge_script)
            .arg(&pipe_name)
            .arg(bridge_root)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!(
                "js-proc: cannot spawn node '{}': {e}",
                node_exe.display()
            ))?;

        // Wait for "READY" on child stdout.
        {
            let stdout = child.stdout.take()
                .ok_or_else(|| "js-proc: no stdout from Node.js process".to_string())?;
            let mut line = String::new();
            BufReader::new(stdout)
                .read_line(&mut line)
                .map_err(|e| format!("js-proc: reading READY: {e}"))?;
            if line.trim() != "READY" {
                return Err(format!(
                    "js-proc: unexpected startup message from Node.js: {:?}",
                    line.trim()
                ));
            }
        }

        // Open named pipe as synchronous duplex client.
        let file = open_pipe_client(&pipe_name)?;
        let reader = BufReader::new(
            file.try_clone()
                .map_err(|e| format!("js-proc: clone file handle: {e}"))?,
        );
        let writer = BufWriter::new(file);

        Ok(JsBridge {
            _child: child,
            reader,
            writer,
            next_id: 1,
        })
    }

    fn send_recv(&mut self, mut req: serde_json::Value)
        -> Result<serde_json::Value, String>
    {
        let id = self.next_id;
        self.next_id += 1;
        req["id"] = serde_json::json!(id);

        let line = serde_json::to_string(&req)
            .map_err(|e| format!("js-proc: encode: {e}"))?;
        self.writer
            .write_all(line.as_bytes())
            .and_then(|_| self.writer.write_all(b"\n"))
            .and_then(|_| self.writer.flush())
            .map_err(|e| format!("js-proc: write: {e}"))?;

        let mut resp = String::new();
        self.reader
            .read_line(&mut resp)
            .map_err(|e| format!("js-proc: read: {e}"))?;

        let v: serde_json::Value = serde_json::from_str(resp.trim())
            .map_err(|e| format!("js-proc: bad JSON response: {e}\nraw: {resp}"))?;

        if let Some(err) = v.get("err").and_then(|e| e.as_str()) {
            return Err(format!("js-proc remote error: {err}"));
        }

        Ok(v["ok"].clone())
    }
}

impl Drop for JsBridge {
    fn drop(&mut self) {
        let _ = self.send_recv(serde_json::json!({"op": "quit"}));
    }
}

// ── Named-pipe client (Windows) ───────────────────────────────────────────────

#[cfg(windows)]
fn open_pipe_client(pipe_name: &str) -> Result<std::fs::File, String> {
    use std::os::windows::io::FromRawHandle;

    let wide: Vec<u16> = {
        use std::os::windows::ffi::OsStrExt;
        std::ffi::OsStr::new(pipe_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };

    const GENERIC_READ:  u32 = 0x80000000;
    const GENERIC_WRITE: u32 = 0x40000000;
    const OPEN_EXISTING: u32 = 3;

    extern "system" {
        fn CreateFileW(
            lpFileName: *const u16, dwDesiredAccess: u32, dwShareMode: u32,
            lpSecurityAttributes: *const u8, dwCreationDisposition: u32,
            dwFlagsAndAttributes: u32, hTemplateFile: *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
        fn WaitNamedPipeW(lpNamedPipeName: *const u16, nTimeOut: u32) -> i32;
        fn GetLastError() -> u32;
    }

    for attempt in 0..20u32 {
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0,
                std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut(),
            )
        };
        if (handle as isize) != -1 {
            return Ok(unsafe { std::fs::File::from_raw_handle(handle as _) });
        }
        let err = unsafe { GetLastError() };
        const ERROR_PIPE_BUSY: u32 = 231;
        if err == ERROR_PIPE_BUSY {
            unsafe { WaitNamedPipeW(wide.as_ptr(), 5000) };
        } else if attempt < 8 {
            std::thread::sleep(std::time::Duration::from_millis(150));
        } else {
            return Err(format!(
                "js-proc: CreateFileW failed (err {err}) on pipe '{pipe_name}'"
            ));
        }
    }
    Err(format!("js-proc: timeout connecting to named pipe '{pipe_name}'"))
}

#[cfg(not(windows))]
fn open_pipe_client(pipe_name: &str) -> Result<std::fs::File, String> {
    Err(format!(
        "js-proc: named pipes not supported on this platform (pipe: {pipe_name})"
    ))
}

// ── Global bridge registry (shared across threads) ───────────────────────────

fn global_bridges() -> &'static Mutex<HashMap<PathBuf, JsBridge>> {
    static BRIDGES: OnceLock<Mutex<HashMap<PathBuf, JsBridge>>> = OnceLock::new();
    BRIDGES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Launch and cache a JsBridge for the given bridge script.
pub fn launch_proc(node_exe: &Path, bridge_script: &Path, bridge_root: &Path)
    -> Result<(), String>
{
    let key = bridge_script
        .canonicalize()
        .unwrap_or_else(|_| bridge_script.to_path_buf());

    let mut map = global_bridges().lock()
        .map_err(|e| format!("js-proc: bridge lock poisoned: {e}"))?;
    if map.contains_key(&key) {
        return Ok(());
    }
    let bridge = JsBridge::launch(node_exe, bridge_script, bridge_root)?;
    map.insert(key, bridge);
    Ok(())
}

// ── Public call API ───────────────────────────────────────────────────────────

/// Query the JS module for its exported function names.
pub fn list_functions(bridge_key: &str, module_name: &str)
    -> Result<Vec<String>, String>
{
    with_bridge(bridge_key, |bridge| {
        let ok = bridge.send_recv(serde_json::json!({
            "op":     "list",
            "module": module_name,
        }))?;
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
        let ok = bridge.send_recv(serde_json::json!({
            "op":     "call",
            "module": module_name,
            "fn":     fn_name,
            "args":   encode_args(args),
        }))?;
        Ok(decode_result(&ok))
    })
}

fn with_bridge<F, R>(bridge_key: &str, f: F) -> Result<R, String>
where
    F: FnOnce(&mut JsBridge) -> Result<R, String>,
{
    let key = PathBuf::from(bridge_key)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(bridge_key));
    let mut map = global_bridges().lock()
        .map_err(|e| format!("js-proc: bridge lock poisoned: {e}"))?;
    let bridge = map.get_mut(&key).ok_or_else(|| {
        format!("js-proc: bridge not launched for '{bridge_key}'")
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
