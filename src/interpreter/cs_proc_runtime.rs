// cs_proc_runtime.rs — Runtime bridge for import[cs-proc].
//
// Communication: newline-delimited JSON over a Windows named pipe.
//
// Request:  {"id":N,"op":"static"|"new"|"inst"|"quit","cls":"ClassName","mth":"method","hnd":handle,"args":[...]}
// Response: {"id":N,"ok":<value>} | {"id":N,"err":"message"}
//
// Arg/result type tags:
//   "i" = int64    "f" = float64    "b" = bool
//   "s" = string   "h" = handle     "n" = null

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::Value;

// ── Pipe name generator ──────────────────────────────────────────────────────

static PIPE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn new_pipe_name() -> String {
    let pid = std::process::id();
    let n = PIPE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(r"\\.\pipe\arrow_cs_{pid}_{n}")
}

// ── ProcBridge ───────────────────────────────────────────────────────────────

pub struct ProcBridge {
    // Keep child alive; dropped when bridge is dropped (sends quit first via Drop).
    _child: std::process::Child,
    reader: BufReader<std::fs::File>,
    writer: BufWriter<std::fs::File>,
    next_id: u64,
    pub path: PathBuf,
}

impl ProcBridge {
    pub fn launch(proc_path: &Path) -> Result<Self, String> {
        let pipe_name = new_pipe_name();

        // Spawn the C# host; it creates the named pipe server and prints "READY".
        let mut child = std::process::Command::new(proc_path)
            .arg(&pipe_name)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("cs-proc: cannot spawn '{}': {e}", proc_path.display()))?;

        // Read "READY" from the child's stdout.
        {
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| "cs-proc: no stdout from child process".to_string())?;
            let mut line = String::new();
            BufReader::new(stdout)
                .read_line(&mut line)
                .map_err(|e| format!("cs-proc: reading READY: {e}"))?;
            if line.trim() != "READY" {
                return Err(format!(
                    "cs-proc: unexpected startup message: {:?}",
                    line.trim()
                ));
            }
        }

        // Open named pipe as synchronous client.
        // The C# host has created the server and is waiting for a connection.
        let file = open_pipe_client(&pipe_name)?;
        let reader = BufReader::new(
            file.try_clone()
                .map_err(|e| format!("cs-proc: clone file handle: {e}"))?,
        );
        let writer = BufWriter::new(file);

        Ok(ProcBridge {
            _child: child,
            reader,
            writer,
            next_id: 1,
            path: proc_path.to_path_buf(),
        })
    }

    fn send_recv(&mut self, mut req: serde_json::Value) -> Result<serde_json::Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        req["id"] = serde_json::json!(id);

        let line = serde_json::to_string(&req).map_err(|e| format!("cs-proc: encode: {e}"))?;
        self.writer
            .write_all(line.as_bytes())
            .and_then(|_| self.writer.write_all(b"\n"))
            .and_then(|_| self.writer.flush())
            .map_err(|e| format!("cs-proc: write: {e}"))?;

        let mut resp = String::new();
        self.reader
            .read_line(&mut resp)
            .map_err(|e| format!("cs-proc: read: {e}"))?;

        let v: serde_json::Value = serde_json::from_str(resp.trim())
            .map_err(|e| format!("cs-proc: bad JSON response: {e}\nraw: {resp}"))?;

        if let Some(err) = v.get("err").and_then(|e| e.as_str()) {
            return Err(format!("cs-proc remote: {err}"));
        }

        Ok(v["ok"].clone())
    }
}

impl Drop for ProcBridge {
    fn drop(&mut self) {
        // Best-effort shutdown; ignore errors.
        let _ = self.send_recv(serde_json::json!({"op": "quit"}));
    }
}

// ── Named pipe client open ───────────────────────────────────────────────────

/// Open a Windows named pipe as a synchronous duplex client.
/// Retries up to 20 times to handle the race between host startup and connect.
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

    const GENERIC_READ: u32 = 0x80000000;
    const GENERIC_WRITE: u32 = 0x40000000;
    const OPEN_EXISTING: u32 = 3;

    extern "system" {
        fn CreateFileW(
            lpFileName: *const u16,
            dwDesiredAccess: u32,
            dwShareMode: u32,
            lpSecurityAttributes: *const u8,
            dwCreationDisposition: u32,
            dwFlagsAndAttributes: u32,
            hTemplateFile: *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
        fn WaitNamedPipeW(lpNamedPipeName: *const u16, nTimeOut: u32) -> i32;
        fn GetLastError() -> u32;
    }

    for attempt in 0..20u32 {
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };

        if (handle as isize) != -1 {
            return Ok(unsafe { std::fs::File::from_raw_handle(handle as _) });
        }

        let err = unsafe { GetLastError() };
        const ERROR_PIPE_BUSY: u32 = 231;
        if err == ERROR_PIPE_BUSY {
            unsafe { WaitNamedPipeW(wide.as_ptr(), 5000) };
        } else if attempt < 5 {
            // Pipe may not exist yet — brief pause and retry.
            std::thread::sleep(std::time::Duration::from_millis(100));
        } else {
            return Err(format!(
                "cs-proc: CreateFileW failed with error {err} on pipe '{pipe_name}'"
            ));
        }
    }

    Err(format!(
        "cs-proc: timeout connecting to named pipe '{pipe_name}'"
    ))
}

#[cfg(not(windows))]
fn open_pipe_client(pipe_name: &str) -> Result<std::fs::File, String> {
    Err(format!(
        "cs-proc: named pipes not supported on this platform (pipe: {pipe_name})"
    ))
}

// ── Thread-local proc registry ───────────────────────────────────────────────

thread_local! {
    static CS_PROCS: RefCell<HashMap<PathBuf, ProcBridge>> =
        RefCell::new(HashMap::new());
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
        let bridge = ProcBridge::launch(proc_path)?;
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
        let ok = bridge.send_recv(serde_json::json!({
            "op": "static",
            "cls": class_name,
            "mth": method,
            "args": encode_args(args),
        }))?;
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
        let ok = bridge.send_recv(serde_json::json!({
            "op": "new",
            "cls": class_name,
            "args": encode_args(args),
        }))?;
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
        let ok = bridge.send_recv(serde_json::json!({
            "op": "inst",
            "cls": class_name,
            "hnd": handle,
            "mth": method,
            "args": encode_args(args),
        }))?;
        Ok(decode_result(&ok, ret_type))
    })
}

fn with_bridge<F, R>(proc_path: &Path, f: F) -> Result<R, String>
where
    F: FnOnce(&mut ProcBridge) -> Result<R, String>,
{
    let canon = proc_path
        .canonicalize()
        .unwrap_or_else(|_| proc_path.to_path_buf());
    CS_PROCS.with(|m| {
        let mut map = m.borrow_mut();
        let bridge = map.get_mut(&canon).ok_or_else(|| {
            format!(
                "cs-proc: bridge not launched for '{}'",
                proc_path.display()
            )
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
        Value::Str(s) => serde_json::json!({"t": "s", "v": s}),
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
        "s" => Value::Str(
            val.and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        ),
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
