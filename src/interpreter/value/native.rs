// value/native.rs — ネイティブ関数サポートと typed ABI ポインタ引数解決: PtrParam / AbiTy / TypedSig / PtrArgCleanup / resolve_typed_ptr_arg / finish_ptr_arg_cleanup / NativeFnRef / NativeLibWrapper。

use {
    std::cell::RefCell, std::fmt,
    std::path::PathBuf, std::rc::Rc, std::sync::Arc,
};
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
    /// プリミティブ書き込みポインタ引数（`int*` / `double*` 等、非 const）。
    /// 呼び出し側がローカル u64 スロットに初期値を width 幅でエンコードして
    /// そのアドレスを渡し、呼び出し後にデコードして名前付き `mut` 変数へ書き戻す
    /// （名前情報のない経路では書き戻しなし = 安全側）。
    /// 構造体ポインタと混在する関数（例: `v3_norm(const V3*, double*)`）を
    /// typed 経路の対象にするために必要 — ハンドル経路は構造体ポインタ引数を
    /// 扱えない（アリーナハンドルをポインタとしてビットキャストしてしまう）。
    OutPtr { width: RawWidth },
}

/// `AbiTy::OutPtr` のローカルスロットへ初期値を width 幅でエンコードする。
/// 値の型がスロット形式に合わなければ `None`（typed 経路を諦める）。
pub fn encode_out_ptr_init(v: &Value, width: RawWidth) -> Option<u64> {
    Some(match (width, v) {
        (RawWidth::I8 | RawWidth::U8, Value::Int(n)) => *n as u8 as u64,
        (RawWidth::I16 | RawWidth::U16, Value::Int(n)) => *n as u16 as u64,
        (RawWidth::I32 | RawWidth::U32, Value::Int(n)) => *n as u32 as u64,
        (RawWidth::I64 | RawWidth::U64, Value::Int(n)) => *n as u64,
        (RawWidth::F32, Value::Float(f)) => (*f as f32).to_bits() as u64,
        (RawWidth::F64, Value::Float(f)) => f.to_bits(),
        (RawWidth::F32, Value::Int(n)) => (*n as f32).to_bits() as u64,
        (RawWidth::F64, Value::Int(n)) => (*n as f64).to_bits(),
        _ => return None,
    })
}

/// C 呼び出し後の `AbiTy::OutPtr` ローカルスロットから値をデコードする
/// （幅変換規則は raw フィールド読み出しと同一: 符号拡張 / f32→f64 拡張）。
pub fn decode_out_ptr(local: u64, width: RawWidth) -> Value {
    match width {
        RawWidth::I8 => Value::Int(local as u8 as i8 as i64),
        RawWidth::U8 => Value::Int(local as u8 as i64),
        RawWidth::I16 => Value::Int(local as u16 as i16 as i64),
        RawWidth::U16 => Value::Int(local as u16 as i64),
        RawWidth::I32 => Value::Int(local as u32 as i32 as i64),
        RawWidth::U32 => Value::Int(local as u32 as i64),
        RawWidth::I64 | RawWidth::U64 => Value::Int(local as i64),
        RawWidth::F32 => Value::Float(f32::from_bits(local as u32) as f64),
        RawWidth::F64 => Value::Float(f64::from_bits(local)),
    }
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
    /// バッファは「読まれない」が、C 呼び出し中ポインタの参照先として生存させるために
    /// 保持する（drop されると dangling pointer になる）。dead_code 警告は意図的に抑制。
    KeepAlive(#[allow(dead_code)] Vec<u8>),
    /// 呼び出し後、シャドウバッファの内容をインスタンスへ読み戻す（mutable かつ名前付き
    /// `mut` 変数）。
    WriteBack {
        inst: Rc<RefCell<InstanceData>>,
        layout: Arc<RawLayout>,
        shadow: Vec<u8>,
    },
}


/// **統一 typed ABI の呼び出し本体**（#54 で 1 本化）。
///
/// `status = fn(args*, ret*, err*)` の直接 C ABI 呼び出し。TLS・アリーナ・ハンドルを
/// 一切通らない。raise は `ErrSlot` 経由で伝播する。
///
/// ⚠⚠ **#54 以前はこの 15 行が 3 箇所に手書きされていた**
/// （`call_native_function` / `dispatch_native_typed_exprs` / `dispatch_native_evaled_wb`）。
/// #48 の実バグ（VM 経路だけ書き戻しが起きず 0.0 を返す）は、まさにこの二重化が原因。
/// **3 経路で違ってよいのは「書き戻し先をどこへ返すか」だけ**なので、呼び出し自体はここに集約する。
///
/// 成功時は raw な戻り値（`u64`）を返す。呼び出し側が `sig.ret` に従って
/// [`decode_typed_ret`] でデコードすること。
///
/// # Safety
/// `typed_ptr` は `build_cpp_typed_sig` が検証した **typed ABI の関数ポインタ**でなければならない
/// （`unsafe extern "C" fn(*const u64, *mut u64, *mut ErrSlot) -> u32`）。
/// `slots` は少なくともシグネチャの引数個数ぶんの有効な要素を持つこと。
pub unsafe fn invoke_typed_abi(
    typed_ptr: usize,
    slots: &[u64],
    cleanups: Vec<PtrArgCleanup>,
) -> Result<u64, String> {
    let mut ret: u64 = 0;
    let mut err = crate::interpreter::native_api::ErrSlot::default();
    let status = {
        let f: unsafe extern "C" fn(
            *const u64,
            *mut u64,
            *mut crate::interpreter::native_api::ErrSlot,
        ) -> u32 = std::mem::transmute(typed_ptr);
        f(slots.as_ptr(), &mut ret, &mut err)
    };
    // ⚠ **cleanup は成否によらず必ず走らせる**（3 経路とも元からこの順序だった）。
    for c in cleanups {
        crate::interpreter::value::finish_ptr_arg_cleanup(c);
    }
    if status != 0 {
        // 既存の raise 経路と同じ "TypeName: msg" 形式で伝播
        return Err(err.to_error_string());
    }
    Ok(ret)
}

/// typed ABI の raw な戻り値を `Value` へデコードする（#54 で 1 本化）。
///
/// ⚠ typed ABI の戻り値に `Ptr`/`OutPtr` は使わない（`build_cpp_typed_sig` が除外する）。
pub fn decode_typed_ret(ret_ty: &AbiTy, ret: u64) -> Value {
    use AbiTy;
    match ret_ty {
        AbiTy::I64 => Value::Int(ret as i64),
        AbiTy::F64 => Value::Float(f64::from_bits(ret)),
        AbiTy::Void => Value::None,
        AbiTy::Ptr { .. } | AbiTy::OutPtr { .. } => {
            unreachable!("typed ABI ret excludes Ptr/OutPtr")
        }
    }
}

/// typed ABI 引数マーシャリングの**唯一の実装**（#76）。
///
/// `slots` を組み、`Ptr` はゼロコピー／シャドウ変換で解決し（[`resolve_typed_ptr_arg`]）、
/// `OutPtr` は自前のローカル領域に初期値を入れてそのアドレスを渡す。
///
/// ⚠⚠ **#76 以前はこのループが 4 箇所に手書きされていた**
/// （`call_native_function` / `dispatch_native_typed_exprs` / `dispatch_native_evaled_wb` /
/// `native_api::callbacks::ar_call_fn`）。#48 の実バグ（VM 経路だけ書き戻しが起きず 0.0 を返す）は
/// **同じ形の二重化**が原因で、#54 は「呼び出し本体」だけを畳んで**引数側は 4 コピーのまま**だった。
/// ⇒ **4 経路で違ってよいのは `named_mut`（＝何を知っているか）と、書き戻し先だけ**。
///
/// ⚠⚠ **この構造体は呼び出しが終わるまで move してはいけない。**
/// `OutPtr` のスロットには `out_locals` の要素のアドレスが入るため、move すると
/// **C が解放済みスタックへ書く**。⇒ 呼び出し側のローカルに置き `&mut` で [`Self::marshal`]
/// を呼ぶこと（値で返す API にしていないのはこのため）。
pub struct TypedArgs {
    /// C へ渡す u64 スロット列（`marshal` が埋める）。
    pub slots: [u64; 16],
    /// `OutPtr` 引数の実体。⚠ `slots` がこの要素を指す。
    out_locals: [u64; 16],
    /// 書き戻すべき `OutPtr` 引数の `(引数 index, 幅)`。
    /// ⚠ **「誰に書き戻すか」は呼び出し側の責任**（名前へ代入するか、呼び出し元へ返すか）。
    pub out_wb: Vec<(usize, RawWidth)>,
    /// 呼び出し後に必ず実行する後処理（シャドウの生存・書き戻し）。
    pub cleanups: Vec<PtrArgCleanup>,
}

/// [`TypedArgs::marshal`] の結果。
pub enum Marshalled {
    /// 全引数を組めた ＝ typed ABI を呼んでよい。
    Ready,
    /// 実行時型がシグネチャと合わない ＝ **ハンドル経路へフォールバックする**。
    /// ⚠ エラーではない（この形は正常系）。
    TypeMismatch,
}

impl Default for TypedArgs {
    fn default() -> Self {
        Self::new()
    }
}

impl TypedArgs {
    pub fn new() -> Self {
        TypedArgs {
            slots: [0u64; 16],
            out_locals: [0u64; 16],
            out_wb: Vec::new(),
            cleanups: Vec::new(),
        }
    }

    /// 評価済みの引数列を u64 スロットへ組む。
    ///
    /// `named_mut[i]` は「引数 i が名前付き変数か・可変か」で、**経路ごとに違ってよい唯一の入力**:
    /// - `Some(true)`  — 名前付き `mut` 変数（書き戻しの対象になる）
    /// - `Some(false)` — 名前付き `let` 変数（書き込みポインタへ渡すと `Err`）
    /// - `None`        — 判定できない経路（VM のマスク未設定・ネイティブコールバック）。
    ///   **常に安全側＝書き戻ししない**。⚠ `Some(false)` と `None` は**同義ではない**
    ///   （前者だけがエラーになる。畳むときはここを潰さないこと）。
    ///
    /// `named_mut` は短くてよい（足りない分は `None` 扱い）。
    pub fn marshal(
        &mut self,
        vals: &[Value],
        params: &[AbiTy],
        named_mut: &[Option<bool>],
    ) -> Result<Marshalled, String> {
        for (i, (v, ty)) in vals.iter().zip(params).enumerate() {
            let nm = named_mut.get(i).copied().flatten();
            match (v, ty) {
                (Value::Int(n), AbiTy::I64) => self.slots[i] = *n as u64,
                (Value::Float(f), AbiTy::F64) => self.slots[i] = f.to_bits(),
                // int → float 引数の自動昇格（ハンドル経路の `ar_to_float` と同義）。
                (Value::Int(n), AbiTy::F64) => self.slots[i] = (*n as f64).to_bits(),
                (_, AbiTy::Ptr { mutable, by_value, layout }) => {
                    match resolve_typed_ptr_arg(v, *mutable, *by_value, layout, nm)? {
                        Some((slot, cleanup)) => {
                            self.slots[i] = slot;
                            self.cleanups.push(cleanup);
                        }
                        None => return Ok(Marshalled::TypeMismatch),
                    }
                }
                // プリミティブ書き込みポインタ（`double*` 等）: 初期値を width 幅でエンコードした
                // ローカルのアドレスを渡し、呼び出し後にデコードして書き戻す。
                (_, AbiTy::OutPtr { width }) => match encode_out_ptr_init(v, *width) {
                    Some(enc) => {
                        self.out_locals[i] = enc;
                        self.slots[i] = std::ptr::addr_of_mut!(self.out_locals[i]) as u64;
                        // 書き戻すのは「名前付き mut 変数」と分かっているときだけ（#48）。
                        if nm == Some(true) {
                            self.out_wb.push((i, *width));
                        }
                    }
                    None => return Ok(Marshalled::TypeMismatch),
                },
                _ => return Ok(Marshalled::TypeMismatch),
            }
        }
        Ok(Marshalled::Ready)
    }

    /// `OutPtr` 引数 `i` に C が書いた値を読み出す（`out_wb` の要素に対して呼ぶ）。
    pub fn decode_out(&self, i: usize, width: RawWidth) -> Value {
        decode_out_ptr(self.out_locals[i], width)
    }
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
                // raw_bytes() は 8 バイトの class_id/flags ヘッダを既にスキップ済み
                // （= C 構造体先頭）。ここでさらに +8 すると 1 フィールド分ずれた
                // 領域を C に渡してしまう（過去の実バグ: v3_add が隣接フィールド
                // へ読み書きし "0 0 9" になった）。
                let ptr = inst.raw_bytes().as_ptr() as u64;
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
/// Dispatch is always through a DLL loaded with `libloading`: `lib_path` names
/// the library and `fn_name` the exported symbol.
///
/// `cached_fn_ptr` is a lazily-populated cache: set to the resolved symbol
/// address on first call so subsequent calls skip `GetProcAddress`.
#[derive(Debug)]
pub struct NativeFnRef {
    /// Absolute path of the `.dll` / `.so` / `.dylib`.
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
    /// Lazily cached address of the `{fn_name}_tl` symbol.
    /// Written once on first call via ar_call_fn fast path; 0 = not yet resolved.
    pub cached_fn_ptr: std::sync::atomic::AtomicUsize,
    /// `{fn_name}_typed` エントリのアドレス（0 = typed 変種なし）。
    /// TLS・アリーナを一切通らない直接 C ABI 呼び出しに使う。
    pub typed_fn_ptr: std::sync::atomic::AtomicUsize,
    /// typed エントリのシグネチャ。`typed_fn_ptr != 0` のときのみ Some。
    pub typed_sig: Option<TypedSig>,
}


impl NativeFnRef {
    /// `mut` ポインタ引数（＝呼び出し元へ書き戻す引数）を持つか（#48）。
    ///
    /// ツリーウォークの `call_native_function` が「書き戻し経路へ入るか」を決めるのと
    /// **同じ判定**。VM 側は「この呼び出しで書き戻し副表を引くか」の門に使う。
    #[inline]
    pub fn has_writeback(&self) -> bool {
        self.ptr_params.contains(&PtrParam::MutPtr)
    }
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


#[cfg(test)]
mod tests {
    use super::*;

    /// AbiTy::OutPtr のエンコード（初期値 → width 幅）とデコード（C 書き込み後 → Value）の
    /// 幅変換規則（切り詰め・符号拡張・f32↔f64）を検証する。
    #[test]
    fn out_ptr_encode_decode_roundtrip() {
        use RawWidth::*;
        // f64: ビットパターンそのまま
        let enc = encode_out_ptr_init(&Value::Float(5.0), F64).unwrap();
        assert!(matches!(decode_out_ptr(enc, F64), Value::Float(f) if f == 5.0));
        // f32: 縮小格納 → 拡張読み出し（f32 で表現可能な値は往復不変）
        let enc = encode_out_ptr_init(&Value::Float(2.5), F32).unwrap();
        assert!(matches!(decode_out_ptr(enc, F32), Value::Float(f) if f == 2.5));
        // i32: 切り詰め格納・符号拡張読み出し
        let enc = encode_out_ptr_init(&Value::Int(-7), I32).unwrap();
        assert!(matches!(decode_out_ptr(enc, I32), Value::Int(-7)));
        // i64
        let enc = encode_out_ptr_init(&Value::Int(1 << 40), I64).unwrap();
        assert!(matches!(decode_out_ptr(enc, I64), Value::Int(n) if n == 1 << 40));
        // int → float 昇格（ハンドル経路の ar_to_float と同義）
        let enc = encode_out_ptr_init(&Value::Int(3), F64).unwrap();
        assert!(matches!(decode_out_ptr(enc, F64), Value::Float(f) if f == 3.0));
        // 型不一致（float を int* へ / 非数値）→ None（typed 経路を諦める）
        assert!(encode_out_ptr_init(&Value::Float(1.5), I32).is_none());
        assert!(encode_out_ptr_init(&Value::Bool(true), F64).is_none());
    }
}
