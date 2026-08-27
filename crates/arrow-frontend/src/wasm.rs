//! wasm32 向けの ABI。
//!
//! `wasm-bindgen` は**使わない**。追加のツールチェーン（`wasm-bindgen-cli` / `wasm-pack`）を
//! ビルド手順に持ち込まずに済ませたいので、素の `extern "C"` と線形メモリだけで
//! やり取りする。ホスト（VS Code 拡張）側は Node 標準の `WebAssembly` API だけで動く。
//!
//! # 呼び出し手順（ホスト側）
//! ```js
//! const ptr = ar_alloc(bytes.length);
//! new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
//! ar_analyze(ptr, bytes.length);
//! const out = new Uint8Array(memory.buffer, ar_result_ptr(), ar_result_len());
//! const json = new TextDecoder().decode(out);
//! ar_free(ptr, bytes.length);
//! ```
//! `ar_analyze` の結果は次の `ar_analyze` まで有効。

use std::cell::RefCell;

thread_local! {
    /// 直近の `ar_analyze` が生成した JSON。ホストが読み終わるまで生かしておく。
    static RESULT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// ホストがソースを書き込むためのバッファを確保する。
///
/// # Safety
/// 戻り値は `ar_free` に**同じ `len`** を添えて返すこと。
#[no_mangle]
pub extern "C" fn ar_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// `ar_alloc` で確保したバッファを解放する。
///
/// # Safety
/// `ptr` は `ar_alloc(len)` の戻り値でなければならない。
#[no_mangle]
pub unsafe extern "C" fn ar_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    drop(Vec::from_raw_parts(ptr, 0, len));
}

/// `ptr[..len]` を UTF-8 の Arrow ソースとして解析し、結果 JSON を内部に保持する。
/// 戻り値は JSON のバイト長（`ar_result_len` と同じ）。
///
/// # Safety
/// `ptr[..len]` が有効な UTF-8 でなければならない（ホストは `TextEncoder` を使うこと）。
#[no_mangle]
pub unsafe extern "C" fn ar_analyze(ptr: *const u8, len: usize) -> usize {
    let bytes = std::slice::from_raw_parts(ptr, len);
    // 不正な UTF-8 が来ても落とさない。エディタのバッファは常に途中状態でありうる。
    let source = String::from_utf8_lossy(bytes);
    let json = crate::analyze::analyze_json(&source, "<buffer>");
    RESULT.with(|r| {
        let mut r = r.borrow_mut();
        *r = json.into_bytes();
        r.len()
    })
}

/// 直近の解析結果 JSON の先頭ポインタ。
#[no_mangle]
pub extern "C" fn ar_result_ptr() -> *const u8 {
    RESULT.with(|r| r.borrow().as_ptr())
}

/// 直近の解析結果 JSON のバイト長。
#[no_mangle]
pub extern "C" fn ar_result_len() -> usize {
    RESULT.with(|r| r.borrow().len())
}
