// value/native.rs — ネイティブ関数サポートと typed ABI ポインタ引数解決: PtrParam / AbiTy / TypedSig / PtrArgCleanup / resolve_typed_ptr_arg / finish_ptr_arg_cleanup / NativeFnRef / NativeLibWrapper。

#[allow(unused_imports)]
use {
    std::cell::RefCell, std::collections::{HashMap, HashSet}, std::fmt,
    std::path::PathBuf, std::rc::Rc, std::sync::atomic::{AtomicU32, Ordering}, std::sync::Arc,
    indexmap::IndexMap,
    crate::ast::{Accessibility, Param, Stmt},
    crate::interpreter::async_mgr,
};
#[allow(unused_imports)]
use super::*;


// ---------------------------------------------------------------------------
// Native function support
// ---------------------------------------------------------------------------

/// Describes how a C function parameter should be handled at the native boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PtrParam {
    /// Not a pointer — value passed by value.
    None,
    /// `const T*` — read-only pointer; any expression is accepted, no write-back.
    ConstPtr,
    /// `T*` — mutable pointer; requires a `mut` variable argument; value is written
    /// back to the variable after the call returns.
    MutPtr,
}


/// 統一 typed ABI の引数・戻り値のマシン型。C ABI の u64 スロットと一対一対応する。
///
/// - `I64`: Arrow `int` — スロットに `i64` をそのまま格納
/// - `F64`: Arrow `float` — スロットに `f64::to_bits()` のビットパターンを格納
/// - `Void`: 戻り値専用（C の `void` 関数）。Arrow 側では `None` になる
/// - `Ptr`: C 構造体ポインタ引数（`T*` / `const T*` / by-value 構造体）。
///   `layout` を持つため `Copy`/`PartialEq` は導出しない。
#[derive(Debug, Clone)]
pub enum AbiTy {
    I64,
    F64,
    Void,
    /// C 構造体ポインタ引数。`layout` は対象 C 構造体の raw レイアウト。
    /// `NativeFnRef` が `NativeCallCache`（`Arc<dyn Any + Send + Sync>`）へ格納されるため
    /// `Rc` ではなく `Arc` を用いる。
    Ptr {
        /// C 側が `T*`（非 const）で書き込みうるか。`by_value` が真のときは無関係。
        mutable: bool,
        /// by-value 構造体引数。値渡し意味論のため決して write-back の対象にならない。
        by_value: bool,
        layout: Arc<RawLayout>,
    },
}


/// `{name}_typed` エントリポイントの型シグネチャ。
/// C ABI: `extern "C" fn(args: *const u64, ret: *mut u64, err: *mut ErrSlot) -> u32`
/// （戻り値 0 = 正常、1 = raise 発生。err に例外情報が書き込まれる）
#[derive(Debug, Clone)]
pub struct TypedSig {
    pub params: Vec<AbiTy>,
    pub ret: AbiTy,
}


// ---------------------------------------------------------------------------
// typed ABI ポインタ引数の解決（.claude/skills/c-abi-interop/SKILL.md P3/P4）
// ---------------------------------------------------------------------------

/// `resolve_typed_ptr_arg` が返す、C 呼び出し後に実行すべき後処理。
///
/// シャドウ変換のバッファは C 呼び出し中ポインタが指す先として生存し続ける必要があるため、
/// 書き戻し不要な場合も `KeepAlive` としてバッファを保持する。
pub enum PtrArgCleanup {
    /// 後処理不要（ゼロコピー直接渡し — インスタンスのメモリを直接指している）。
    None,
    /// シャドウバッファを呼び出し終了まで生存させるだけ（書き戻しなし: const / by-value /
    /// 非名前付き引数）。
    KeepAlive(Vec<u8>),
    /// 呼び出し後、シャドウバッファの内容をインスタンスへ読み戻す（mutable かつ名前付き
    /// `mut` 変数）。
    WriteBack {
        inst: Rc<RefCell<InstanceData>>,
        layout: Arc<RawLayout>,
        shadow: Vec<u8>,
    },
}


/// typed ABI のポインタ引数（`AbiTy::Ptr`）を u64 スロット値へ解決する。全呼び出し経路で共有。
///
/// - `Ok(Some((slot, cleanup)))`: `slot` を u64 スロットへ格納し、呼び出し後に
///   `finish_ptr_arg_cleanup(cleanup)` を必ず呼ぶ（`cleanup` がバッファ生存・書き戻しを担う）。
/// - `Ok(None)`: typed 経路を諦めハンドル経路へフォールバックする（値がインスタンスでない）。
/// - `Err(msg)`: 呼び出しを中止すべきエラー（`let` 変数を書き込みポインタへ渡した等）。
///
/// `named_mut`: 呼び出し元の実引数が名前付き変数のとき `Some(可変か)`、判定できない経路では
/// `None`（`None` のときは常に安全側＝書き戻ししない）。
pub fn resolve_typed_ptr_arg(
    v: &Value,
    mutable: bool,
    by_value: bool,
    layout: &Arc<RawLayout>,
    named_mut: Option<bool>,
) -> Result<Option<(u64, PtrArgCleanup)>, String> {
    // 1. インスタンス以外は typed 経路を諦める（ハンドル経路へ）。
    let Value::Instance(rc) = v else {
        return Ok(None);
    };
    // 2. 書き込み用ポインタ（mutable かつ非 by-value）へ `let` 変数を渡す誤りを拒否
    //    （ハンドルベース `PtrParam::MutPtr` と同じ規則）。
    if mutable && !by_value && named_mut == Some(false) {
        return Err(
            "TypeError: cannot pass an immutable (let) variable as a mutable pointer argument"
                .to_string(),
        );
    }
    // 書き戻しは mutable・非 by-value かつ名前付き `mut` 変数のときだけ。
    let needs_writeback = mutable && !by_value && named_mut == Some(true);

    let inst = rc.borrow();
    // 3. インスタンスの raw レイアウトが対象 `layout` と構造的に完全一致 → ゼロコピー。
    //    `raw.as_ptr() + 8`（8 バイトの class_id/flags ヘッダの後ろ）が C 構造体先頭。
    if inst.has_raw_layout() {
        if let Some(inst_layout) = inst.class.raw_layout.as_ref() {
            if raw_layouts_compatible(inst_layout, layout) {
                let ptr = unsafe { inst.raw_bytes().as_ptr().add(8) } as u64;
                return Ok(Some((ptr, PtrArgCleanup::None)));
            }
        }
    }
    // 4. 一致しなければシャドウ変換（フィールドを宣言順の位置で対応付け）。
    let shadow = build_shadow_raw(&inst, layout).ok_or_else(|| {
        "TypeError: cannot marshal instance to the expected C struct layout".to_string()
    })?;
    drop(inst);
    let ptr = shadow.as_ptr() as u64;
    let cleanup = if needs_writeback {
        PtrArgCleanup::WriteBack {
            inst: rc.clone(),
            layout: layout.clone(),
            shadow,
        }
    } else {
        PtrArgCleanup::KeepAlive(shadow)
    };
    Ok(Some((ptr, cleanup)))
}


/// `resolve_typed_ptr_arg` が返した後処理を実行する（シャドウバッファの解放・書き戻し）。
pub fn finish_ptr_arg_cleanup(cleanup: PtrArgCleanup) {
    match cleanup {
        PtrArgCleanup::None | PtrArgCleanup::KeepAlive(_) => {}
        PtrArgCleanup::WriteBack {
            inst,
            layout,
            shadow,
        } => {
            let mut inst_mut = inst.borrow_mut();
            apply_shadow_raw(&mut inst_mut, &layout, &shadow);
        }
    }
}


/// Reference to a native (natively compiled) function.
///
/// Two dispatch modes:
///   - `raw_fn_ptr != 0`: inkwell JIT — call the pointer directly (no libloading).
///   - `raw_fn_ptr == 0`: DLL via libloading — use `lib_path` to look up the library.
///
/// `cached_fn_ptr` is a lazily-populated cache for the cpp-dll case: set to the
/// resolved symbol address on first call so subsequent calls skip `GetProcAddress`.
#[derive(Debug)]
pub struct NativeFnRef {
    /// Absolute path of the `.dll` / `.so` / `.dylib`.  Empty for JIT functions.
    pub lib_path: PathBuf,
    /// Base name of the tl function (e.g. `"is_prime"`).
    /// The actual exported symbol is `"{fn_name}_tl"`.
    pub fn_name: String,
    /// Total number of positional parameters (used to size the args array).
    pub n_params: usize,
    /// Minimum number of required arguments.
    pub min_params: usize,
    /// Per-parameter mutability flags (`true` = `mut`, `false` = `let`).
    pub param_mutabilities: Vec<bool>,
    /// Per-parameter pointer kind (cpp-bridge only).
    pub ptr_params: Vec<PtrParam>,
    /// Non-zero for inkwell JIT functions: address of `fname_tl` in JIT memory.
    /// Cast to `unsafe extern "C" fn(*const i64, i32) -> i64` at call time.
    pub raw_fn_ptr: usize,
    /// Lazily cached raw function pointer for cpp-dll functions (raw_fn_ptr == 0).
    /// Written once on first call via ar_call_fn fast path; 0 = not yet resolved.
    pub cached_fn_ptr: std::sync::atomic::AtomicUsize,
    /// `{fn_name}_typed` エントリのアドレス（0 = typed 変種なし）。
    /// TLS・アリーナを一切通らない直接 C ABI 呼び出しに使う。
    pub typed_fn_ptr: std::sync::atomic::AtomicUsize,
    /// typed エントリのシグネチャ。`typed_fn_ptr != 0` のときのみ Some。
    pub typed_sig: Option<TypedSig>,
}


impl Clone for NativeFnRef {
    fn clone(&self) -> Self {
        Self {
            lib_path: self.lib_path.clone(),
            fn_name: self.fn_name.clone(),
            n_params: self.n_params,
            min_params: self.min_params,
            param_mutabilities: self.param_mutabilities.clone(),
            ptr_params: self.ptr_params.clone(),
            raw_fn_ptr: self.raw_fn_ptr,
            cached_fn_ptr: std::sync::atomic::AtomicUsize::new(
                self.cached_fn_ptr.load(std::sync::atomic::Ordering::Relaxed),
            ),
            typed_fn_ptr: std::sync::atomic::AtomicUsize::new(
                self.typed_fn_ptr.load(std::sync::atomic::Ordering::Relaxed),
            ),
            typed_sig: self.typed_sig.clone(),
        }
    }
}


/// Wrapper around `libloading::Library` that implements `Debug`.
pub struct NativeLibWrapper(pub libloading::Library);


impl fmt::Debug for NativeLibWrapper {
    /// `NativeLibWrapper` のデバッグ表示。常に `"<NativeLib>"` を出力する。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<NativeLib>")
    }
}
