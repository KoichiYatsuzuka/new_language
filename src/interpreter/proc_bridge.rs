// proc_bridge.rs — Shared plumbing for newline-delimited JSON-over-named-pipe
// runtime bridges (import[cs-proc], import[js-proc]).
//
// Each language bridge spawns a host process that:
//   1. creates a Windows named-pipe server named as passed on the command line,
//   2. prints "READY\n" on stdout once the server is listening,
// then this side connects to the pipe as a synchronous duplex client and
// exchanges one JSON object per line.
//
// The per-language differences (spawn arguments, request ops, value encoding)
// stay in the caller; only the pipe mechanics live here.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::sync::atomic::{AtomicU64, Ordering};

// ── Pipe name generator ──────────────────────────────────────────────────────

static PIPE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a process-unique pipe name of the form
/// `\\.\pipe\arrow_<prefix>_<pid>_<n>`.
pub fn new_pipe_name(prefix: &str) -> String {
    let pid = std::process::id();
    let n = PIPE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(r"\\.\pipe\arrow_{prefix}_{pid}_{n}")
}

// ── PipeConn ─────────────────────────────────────────────────────────────────

/// A live connection to a spawned host process over a named pipe.
/// Owns the child so it stays alive; `Drop` sends a best-effort `quit`.
pub struct PipeConn {
    // Keep the child alive; dropped (after quit) when the connection is dropped.
    _child: std::process::Child,
    reader: BufReader<std::fs::File>,
    writer: BufWriter<std::fs::File>,
    next_id: u64,
}

impl PipeConn {
    /// Complete the handshake with an already-spawned `child` (whose stdout must
    /// be piped): read its "READY" line, then connect to `pipe_name` as a client.
    /// `tag` only prefixes error messages (e.g. "cs-proc" / "js-proc").
    pub fn connect(
        mut child: std::process::Child,
        pipe_name: &str,
        tag: &str,
    ) -> Result<Self, String> {
        // Read "READY" from the child's stdout (signals the pipe server is up).
        {
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| format!("{tag}: no stdout from host process"))?;
            let mut line = String::new();
            BufReader::new(stdout)
                .read_line(&mut line)
                .map_err(|e| format!("{tag}: reading READY: {e}"))?;
            if line.trim() != "READY" {
                return Err(format!(
                    "{tag}: unexpected startup message: {:?}",
                    line.trim()
                ));
            }
        }

        // Open the named pipe as a synchronous duplex client.
        let file = open_pipe_client(pipe_name, tag)?;
        let reader = BufReader::new(
            file.try_clone()
                .map_err(|e| format!("{tag}: clone file handle: {e}"))?,
        );
        let writer = BufWriter::new(file);

        Ok(PipeConn {
            _child: child,
            reader,
            writer,
            next_id: 1,
        })
    }

    /// Send one JSON request (an `id` field is injected) and read one JSON
    /// response line. Returns the `ok` value, or an error if the response
    /// carries an `err`.
    pub fn send_recv(
        &mut self,
        mut req: serde_json::Value,
        tag: &str,
    ) -> Result<serde_json::Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        req["id"] = serde_json::json!(id);

        let line = serde_json::to_string(&req).map_err(|e| format!("{tag}: encode: {e}"))?;
        self.writer
            .write_all(line.as_bytes())
            .and_then(|_| self.writer.write_all(b"\n"))
            .and_then(|_| self.writer.flush())
            .map_err(|e| format!("{tag}: write: {e}"))?;

        let mut resp = String::new();
        self.reader
            .read_line(&mut resp)
            .map_err(|e| format!("{tag}: read: {e}"))?;

        let v: serde_json::Value = serde_json::from_str(resp.trim())
            .map_err(|e| format!("{tag}: bad JSON response: {e}\nraw: {resp}"))?;

        if let Some(err) = v.get("err").and_then(|e| e.as_str()) {
            return Err(format!("{tag} remote: {err}"));
        }

        Ok(v["ok"].clone())
    }
}

impl Drop for PipeConn {
    fn drop(&mut self) {
        // Best-effort shutdown; ignore errors.
        let _ = self.send_recv(serde_json::json!({"op": "quit"}), "proc");
    }
}

// ── Named-pipe client open ───────────────────────────────────────────────────

/// Open a Windows named pipe as a synchronous duplex client.
/// Retries to handle the race between host startup and connect.
#[cfg(windows)]
fn open_pipe_client(pipe_name: &str, tag: &str) -> Result<std::fs::File, String> {
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
        } else if attempt < 8 {
            // Pipe may not exist yet — brief pause and retry.
            std::thread::sleep(std::time::Duration::from_millis(150));
        } else {
            return Err(format!(
                "{tag}: CreateFileW failed with error {err} on pipe '{pipe_name}'"
            ));
        }
    }

    Err(format!(
        "{tag}: timeout connecting to named pipe '{pipe_name}'"
    ))
}

#[cfg(not(windows))]
fn open_pipe_client(pipe_name: &str, tag: &str) -> Result<std::fs::File, String> {
    Err(format!(
        "{tag}: named pipes not supported on this platform (pipe: {pipe_name})"
    ))
}
