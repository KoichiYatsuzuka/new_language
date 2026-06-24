# cs_proc_runtime.py — Arrow cs-proc IPC runtime (Python implementation).
#
# Mirrors src/interpreter/cs_proc_runtime.rs.
# Protocol: newline-delimited JSON over a Windows named pipe.
#
# Request:  {"id":N,"op":"static"|"new"|"inst"|"quit","cls":"Cls","mth":"mth","hnd":h,"args":[...]}
# Response: {"id":N,"ok":<value>} | {"id":N,"err":"message"}
# Tags: "i"=int64  "f"=float64  "b"=bool  "s"=string  "h"=handle  "n"=null

from __future__ import annotations
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Optional

# ── Pipe name generator ──────────────────────────────────────────────────────

_PIPE_COUNTER: int = 0

def _new_pipe_name() -> str:
    global _PIPE_COUNTER
    _PIPE_COUNTER += 1
    return rf"\\.\pipe\arrow_cs_{os.getpid()}_{_PIPE_COUNTER}"

# ── ProcBridge ───────────────────────────────────────────────────────────────

class ProcBridge:
    def __init__(self, proc: subprocess.Popen, pipe_file, proc_path: str) -> None:
        self._proc = proc
        self._pipe = pipe_file  # raw binary file object (buffering=0)
        self._next_id: int = 1
        self.path: str = proc_path
        self._read_buf: bytes = b""

    def _read_line(self) -> bytes:
        """Read one newline-delimited line from the pipe (raw, unbuffered)."""
        while b"\n" not in self._read_buf:
            chunk = self._pipe.read(256)
            if not chunk:
                raise RuntimeError("cs-proc: pipe closed unexpectedly")
            self._read_buf += chunk
        idx = self._read_buf.index(b"\n")
        line = self._read_buf[:idx]
        self._read_buf = self._read_buf[idx + 1:]
        return line

    def send_recv(self, req: dict):
        req["id"] = self._next_id
        self._next_id += 1
        data = (json.dumps(req) + "\n").encode("utf-8")
        self._pipe.write(data)
        resp_bytes = self._read_line()
        resp = json.loads(resp_bytes.decode("utf-8"))
        if "err" in resp:
            raise RuntimeError(f"cs-proc remote: {resp['err']}")
        return resp.get("ok")

    def close(self) -> None:
        try:
            self.send_recv({"op": "quit"})
        except Exception:
            pass
        try:
            self._pipe.close()
        except Exception:
            pass
        try:
            self._proc.terminate()
        except Exception:
            pass

# ── Registry ─────────────────────────────────────────────────────────────────

_PROC_CACHE: dict[str, ProcBridge] = {}  # canonical path → bridge

# ── Open named pipe client (Windows) ─────────────────────────────────────────

def _open_pipe_client(pipe_name: str):
    """Open a Windows named pipe as a synchronous duplex client."""
    if sys.platform != "win32":
        raise RuntimeError(
            f"cs-proc: named pipes are only supported on Windows (pipe: {pipe_name})"
        )

    import ctypes
    import ctypes.wintypes
    import msvcrt

    GENERIC_READ  = 0x80000000
    GENERIC_WRITE = 0x40000000
    OPEN_EXISTING = 3
    ERROR_PIPE_BUSY = 231

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateFileW.restype  = ctypes.c_void_p
    kernel32.CreateFileW.argtypes = [
        ctypes.c_wchar_p, ctypes.wintypes.DWORD, ctypes.wintypes.DWORD,
        ctypes.c_void_p, ctypes.wintypes.DWORD, ctypes.wintypes.DWORD,
        ctypes.c_void_p,
    ]
    kernel32.WaitNamedPipeW.restype  = ctypes.wintypes.BOOL
    kernel32.WaitNamedPipeW.argtypes = [ctypes.c_wchar_p, ctypes.wintypes.DWORD]

    # -1 cast to void* (INVALID_HANDLE_VALUE)
    INVALID = ctypes.c_void_p(-1).value

    for attempt in range(20):
        handle = kernel32.CreateFileW(
            pipe_name, GENERIC_READ | GENERIC_WRITE,
            0, None, OPEN_EXISTING, 0, None,
        )
        if handle is not None and handle != INVALID:
            fd = msvcrt.open_osfhandle(handle, os.O_RDWR | os.O_BINARY)
            return os.fdopen(fd, "r+b", buffering=0)

        err = ctypes.get_last_error()
        if err == ERROR_PIPE_BUSY:
            kernel32.WaitNamedPipeW(pipe_name, 5000)
        else:
            time.sleep(0.1)

    raise RuntimeError(f"cs-proc: timeout connecting to named pipe '{pipe_name}'")

# ── Public launch API ─────────────────────────────────────────────────────────

def launch_proc(proc_path: str) -> None:
    """Launch the cs-proc host executable and register in the cache."""
    canon = str(Path(proc_path).resolve())
    if canon in _PROC_CACHE:
        return

    pipe_name = _new_pipe_name()

    proc = subprocess.Popen(
        [proc_path, pipe_name],
        stdout=subprocess.PIPE,
        stderr=None,
    )

    # Wait for "READY" signal on stdout
    line = proc.stdout.readline().strip()  # type: ignore[union-attr]
    if line != b"READY":
        proc.kill()
        raise RuntimeError(f"cs-proc: unexpected startup message: {line!r}")

    # Connect to named pipe
    pipe_file = _open_pipe_client(pipe_name)
    _PROC_CACHE[canon] = ProcBridge(proc, pipe_file, canon)


def has_proc(proc_path: str) -> bool:
    return str(Path(proc_path).resolve()) in _PROC_CACHE

# ── Encoding / decoding ───────────────────────────────────────────────────────

def _encode_arg(v) -> dict:
    # Import inline to avoid circular dependency
    from .value import TlCsObject
    if isinstance(v, bool):
        return {"t": "b", "v": v}
    if isinstance(v, int):
        return {"t": "i", "v": v}
    if isinstance(v, float):
        return {"t": "f", "v": v}
    if isinstance(v, str):
        return {"t": "s", "v": v}
    if isinstance(v, TlCsObject):
        return {"t": "h", "v": v.handle}
    return {"t": "n"}


def _decode_result(ok, ret_type: Optional[str]):
    if ok is None:
        return None
    t = ok.get("t", "i")
    v = ok.get("v")
    if t == "s":
        return str(v) if v is not None else ""
    if t == "f":
        return float(v) if v is not None else 0.0
    if t == "b":
        return bool(v)
    if t == "n":
        return None
    # "i" or "h" (handle)
    n = int(v) if v is not None else 0
    if ret_type == "float":
        import struct
        return struct.unpack("<d", struct.pack("<q", n))[0]
    if ret_type == "bool":
        return n != 0
    if ret_type in ("None", "void"):
        return None
    return n

# ── Public call API ───────────────────────────────────────────────────────────

def _get_bridge(proc_path: str) -> ProcBridge:
    canon = str(Path(proc_path).resolve())
    bridge = _PROC_CACHE.get(canon)
    if bridge is None:
        raise RuntimeError(f"cs-proc: bridge not launched for '{proc_path}'")
    return bridge


def call_static(proc_path: str, class_name: str, method: str,
                args: list, ret_type: Optional[str]):
    bridge = _get_bridge(proc_path)
    ok = bridge.send_recv({
        "op": "static",
        "cls": class_name,
        "mth": method,
        "args": [_encode_arg(a) for a in args],
    })
    return _decode_result(ok, ret_type)


def call_constructor(proc_path: str, class_name: str, args: list) -> int:
    bridge = _get_bridge(proc_path)
    ok = bridge.send_recv({
        "op": "new",
        "cls": class_name,
        "args": [_encode_arg(a) for a in args],
    })
    if ok is None:
        return 0
    return int(ok.get("v", 0))


def call_instance(proc_path: str, class_name: str, handle: int,
                  method: str, args: list, ret_type: Optional[str]):
    bridge = _get_bridge(proc_path)
    ok = bridge.send_recv({
        "op": "inst",
        "cls": class_name,
        "hnd": handle,
        "mth": method,
        "args": [_encode_arg(a) for a in args],
    })
    return _decode_result(ok, ret_type)
