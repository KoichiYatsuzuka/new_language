// ⚠ 評価コアビルド（`native` 無効）では FFI 経路が全部落ちるので、typed ABI の marshal/cleanup 補助が
//   未使用になる。型（`NativeFnRef` / `RawLayout` など）は `Value` が参照するため
//   残す必要があり、**関数だけ** dead になる。⇒ そのビルドに限り警告を抑える。
#![cfg_attr(not(feature = "native"), allow(dead_code))]
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
        /// 構造体の名前。インスタンスの代わりに `Ptr[名前]` も渡せる（4-4）。
        elem: Arc<str>,
    },
    /// C の `bool`（引数・戻り値・4-4）。
    Bool,
    /// `const char*` の引数（`bytes` か `None`・4-4・D-13）。スロットは NUL を付けたコピーのアドレス。
    Bytes,
    /// `char*` / `wchar_t*` の戻り値（`Option[bytes]`・4-4）。スロットは C が返したアドレス。
    CStr { wide: bool },
    /// `void*` / 構造体・数値へのポインタ（引数は `Ptr` か `None`、戻り値は借用の `Ptr[elem]`・4-4）。スロットはアドレス。
    PtrVal { elem: Arc<str> },
    /// `ArPtrProtocol` の引数（値・`const` ポインタ・`writable` は書き込み用ポインタ・4-4）。スロットは組み立てた構造体の
    /// アドレス（値渡しは中継コードがそこからコピーする）。
    Protocol { writable: bool },
    /// `ArPtrProtocol` の戻り値（値返し・4-5）。`*_ret` に呼び出し側の領域のアドレスを入れて呼び、中継コードがそこへ構造体を
    /// 書く（`invoke_typed_abi`）。`structs` はそのヘッダが宣言した構造体の名前（要素の型名の照合・D-12）。
    ProtocolRet { structs: Arc<[Arc<str>]> },
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

/// C の符号なし 64 ビットの 2^63 以上を Arrow の `int` に入れようとしたときのメッセージ（1-6）。
/// 中継 DLL の生成コード（`cpp_bridge/codegen.rs` の `_U64_MSG`）と同じ文言にする（単体テストが突き合わせる）。
pub const U64_OVERFLOW_MSG: &str = "a C unsigned 64-bit value of 2^63 or more does not fit in int";

/// C 呼び出し後の `AbiTy::OutPtr` ローカルスロットから値をデコードする
/// （幅変換規則は raw フィールド読み出しと同一: 符号拡張 / f32→f64 拡張）。
/// ⚠ 符号なし 64 ビットの 2^63 以上は `int` に入らないので OverflowError にする（1-6。以前は黙って負にしていた）。
pub fn decode_out_ptr(local: u64, width: RawWidth) -> Result<Value, String> {
    if matches!(width, RawWidth::U64) && local > i64::MAX as u64 {
        return Err(format!("OverflowError: {U64_OVERFLOW_MSG}"));
    }
    Ok(match width {
        RawWidth::I8 => Value::Int(local as u8 as i8 as i64),
        RawWidth::U8 => Value::Int(local as u8 as i64),
        RawWidth::I16 => Value::Int(local as u16 as i16 as i64),
        RawWidth::U16 => Value::Int(local as u16 as i64),
        RawWidth::I32 => Value::Int(local as u32 as i32 as i64),
        RawWidth::U32 => Value::Int(local as u32 as i64),
        RawWidth::I64 | RawWidth::U64 => Value::Int(local as i64),
        RawWidth::F32 => Value::Float(f32::from_bits(local as u32) as f64),
        RawWidth::F64 => Value::Float(f64::from_bits(local)),
    })
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
    /// `ArPtrProtocol` の引数（4-4）。呼び出しの間だけ生かし、書き込み用ポインタなら C が `address` を 0 に
    /// 書き戻していたかを見る（`PtrProtocolArg::finish`）。
    Protocol(super::PtrProtocolArg),
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
/// 成功時は `ret_ty` に従ってデコードした戻り値を返す（[`decode_typed_ret`]。引数のバッファが生きているうちに読む）。
///
/// # Safety
/// `typed_ptr` は `build_cpp_typed_sig` が検証した **typed ABI の関数ポインタ**でなければならない
/// （`unsafe extern "C" fn(*const u64, *mut u64, *mut ErrSlot) -> u32`）。
/// `slots` は少なくともシグネチャの引数個数ぶんの有効な要素を持つこと。
pub unsafe fn invoke_typed_abi(
    typed_ptr: usize,
    slots: &[u64],
    cleanups: Vec<PtrArgCleanup>,
    ret_ty: &AbiTy,
) -> Result<Value, String> {
    let mut ret: u64 = 0;
    // `ArPtrProtocol` の値返し（4-5）: 中継コードが構造体を書く領域。`ret` にそのアドレスを入れて渡す
    let mut proto_ret = super::PtrProtocolRaw {
        address: 0,
        elem_type: std::ptr::null(),
        flags: 0,
        _reserved: 0,
        length: 0,
        release: None,
        release_ctx: std::ptr::null_mut(),
    };
    if matches!(ret_ty, AbiTy::ProtocolRet { .. }) {
        ret = std::ptr::addr_of_mut!(proto_ret) as u64;
    }
    let mut err = crate::interpreter::native_api::ErrSlot::default();
    let status = {
        let f: unsafe extern "C" fn(
            *const u64,
            *mut u64,
            *mut crate::interpreter::native_api::ErrSlot,
        ) -> u32 = std::mem::transmute(typed_ptr);
        f(slots.as_ptr(), &mut ret, &mut err)
    };
    // ⚠ 戻り値は**後処理の前に**読む（4-4）。C の関数は引数のバッファを指すポインタを返しうる（`const char* echo(const char* s)`）
    //   ので、`bytes` のコピー・組み立てた `ArPtrProtocol` を解放してから読むと解放済みの領域を読む。
    let value = if status == 0 { Some(decode_typed_ret(ret_ty, ret)) } else { None };
    // ⚠ **cleanup は成否によらず必ず走らせる**（3 経路とも元からこの順序だった）。
    for c in cleanups {
        crate::interpreter::value::finish_ptr_arg_cleanup(c);
    }
    match value {
        Some(v) => v,
        // 既存の raise 経路と同じ "TypeName: msg" 形式で伝播
        None => Err(err.to_error_string()),
    }
}

/// typed ABI の raw な戻り値を `Value` へデコードする（#54 で 1 本化）。
///
/// ⚠ typed ABI の戻り値に `Ptr`/`OutPtr`・引数専用の種類は使わない（`build_cpp_typed_sig` が除外する）。
/// ポインタの戻り値（4-4）: `char*` / `wchar_t*` は `bytes`、ほかは借用の `Ptr[elem]`、NULL は `None`。
pub fn decode_typed_ret(ret_ty: &AbiTy, ret: u64) -> Result<Value, String> {
    use AbiTy;
    Ok(match ret_ty {
        AbiTy::I64 => Value::Int(ret as i64),
        AbiTy::F64 => Value::Float(f64::from_bits(ret)),
        AbiTy::Void => Value::None,
        AbiTy::Bool => Value::Bool(ret != 0),
        // SAFETY: 中継コードが C の関数の戻り値（NUL 終端の文字列か NULL）をそのまま返す
        AbiTy::CStr { wide } => unsafe { super::c_string_ret(ret, *wide)? },
        AbiTy::PtrVal { elem } => super::raw_pointer_ret(ret, elem)?,
        // `ret` は `invoke_typed_abi` が用意した領域のアドレス（中継コードがそこへ構造体を書いた）
        AbiTy::ProtocolRet { structs } => {
            super::ptr_from_protocol(unsafe { &*(ret as usize as *const super::PtrProtocolRaw) }, structs)?
        }
        AbiTy::Ptr { .. } | AbiTy::OutPtr { .. } | AbiTy::Bytes | AbiTy::Protocol { .. } => {
            unreachable!("typed ABI ret excludes argument-only kinds")
        }
    })
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
    /// ⚠ エラーではない（この形は正常系）。ただし、ハンドル経路では正しく受け渡せない食い違い
    ///   （構造体ポインタへのインスタンス以外・数値への書き込みポインタへの数値以外）は、ここではなく
    ///   `marshal` が `Err` を返す（ffi_value_exchange_plan.md 1-2）。
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
                // 構造体ポインタへ `Ptr[構造体]` を渡し返す（4-4）。値渡しの構造体は指す先からコピーする（中継コード）ので
                // NULL は渡せない。
                (Value::Ptr(_), AbiTy::Ptr { elem, .. }) => self.slots[i] = super::ptr_arg_address(v, elem)?,
                (Value::None, AbiTy::Ptr { by_value: false, .. }) => self.slots[i] = 0,
                (Value::None, AbiTy::Ptr { by_value: true, elem, .. }) => {
                    return Err(format!(
                        "TypeError: argument {} passes struct `{elem}` by value, so it needs an instance or a Ptr, not None",
                        i + 1
                    ))
                }
                (_, AbiTy::Ptr { mutable, by_value, layout, .. }) => {
                    match resolve_typed_ptr_arg(v, *mutable, *by_value, layout, nm)? {
                        Some((slot, cleanup)) => {
                            self.slots[i] = slot;
                            self.cleanups.push(cleanup);
                        }
                        // ⚠ 構造体**ポインタ**にインスタンス以外が来た。ハンドル経路は構造体ポインタを扱えない
                        //   （Arrow 内部のハンドル番号をポインタとして渡して落ちる・c-abi-interop P7 の残課題）ので、
                        //   落とさずにエラーにする（ffi_value_exchange_plan.md 1-2）。値渡しの構造体はハンドル経路が
                        //   フィールドから組み立てられる（1-3）ので従来どおり。
                        None if !*by_value => {
                            return Err(format!(
                                "TypeError: argument {} must be an instance of the C struct the function points to (or a Ptr to it), not '{}'",
                                i + 1,
                                crate::interpreter::ops::typecheck::runtime_type_name(v)
                            ))
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
                    // 数値どうしの食い違い（float を int* へ等）は従来どおりハンドル経路で変換する。
                    None if matches!(v, Value::Int(_) | Value::UInt(_) | Value::Float(_) | Value::Bool(_)) => {
                        return Ok(Marshalled::TypeMismatch)
                    }
                    // ⚠ 数値でない値（None・str など）。ハンドル経路は 0 として渡して 0 を書き戻してしまう
                    //   （黙った変換）ので、落とさずにエラーにする（1-2）。
                    None => {
                        return Err(format!(
                            "TypeError: argument {} must be a number for a pointer-to-number parameter, not '{}'",
                            i + 1,
                            crate::interpreter::ops::typecheck::runtime_type_name(v)
                        ))
                    }
                },
                // ── 4-4: 真偽値・生のポインタ・`ArPtrProtocol`（規則は `value/ptr.rs` の共通関数）──
                (Value::Bool(b), AbiTy::Bool) => self.slots[i] = *b as u64,
                (_, AbiTy::Bytes) => match super::c_string_arg(v)? {
                    Some(buf) => {
                        // ⚠ `Vec` を後処理へ移してもヒープ上の中身は動かないので、スロットのアドレスは有効なまま
                        self.slots[i] = buf.as_ptr() as u64;
                        self.cleanups.push(PtrArgCleanup::KeepAlive(buf));
                    }
                    None => self.slots[i] = 0,
                },
                (_, AbiTy::PtrVal { elem }) => self.slots[i] = super::ptr_arg_address(v, elem)?,
                (_, AbiTy::Protocol { writable }) => {
                    let arg = super::protocol_arg(v, *writable)?;
                    self.slots[i] = arg.address();
                    self.cleanups.push(PtrArgCleanup::Protocol(arg));
                }
                _ => return Ok(Marshalled::TypeMismatch),
            }
        }
        Ok(Marshalled::Ready)
    }

    /// `OutPtr` 引数 `i` に C が書いた値を読み出す（`out_wb` の要素に対して呼ぶ）。
    pub fn decode_out(&self, i: usize, width: RawWidth) -> Result<Value, String> {
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
        PtrArgCleanup::Protocol(arg) => arg.finish(),
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
    /// 省略できる引数の既定値（C の `DEFAULTPARAM( = x )`・`CFnSig::defaults`・ffi_value_exchange_plan.md 7-13）。
    /// `min_params` 以降の引数を省略したときに `omitted_arg` が入れる。Arrow の関数（`.arc`）は空。
    pub param_defaults: Vec<Option<crate::cpp_header::types::CDefault>>,
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
    /// 中継 DLL に入らなかった関数の理由（cpp-lib のシムのコンパイルで外れた・未対応のシグネチャ・ffi_value_exchange_plan.md
    /// 7-28）。`Some` のとき呼ぶと理由つきの RuntimeError（`missing_symbol_error`）。型スタブにはあるので、名前空間からは
    /// 消さない（消すと「属性が無い」になり、理由が分からない）。
    pub unavailable: Option<String>,
    /// コンパイル済み Arrow モジュール（`.arc`）の関数なら、そのモジュールの大域の番号（`FnValue::globals` と同じ・
    /// native_object_access_plan.md 1-7）。呼び出しの間だけ大域を差し替える（生成コードが名前で引く `G` が
    /// モジュールのものになる・`Interpreter::enter_native_globals`）。C / C++ の関数は Arrow の名前を引かないので `None`。
    /// ⚠ 以前は大域を差し替えずに呼び、名前を引けるよう取り込み側の大域へモジュールの名前を `let` で写していた。
    pub globals: Option<u32>,
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

    /// 省略した引数 `i`（0 始まり）に入れる値: C の既定値（7-13）。既定値の無い引数は誤り（黙って `None` / 0 にしない。
    /// 呼び出しの引数の数は `min_params` が先に確かめるので、ふつうは来ない）。
    /// ⚠ 以前は一律に `None` を入れていた（`DEFAULTPARAM( = -1 )` も 0 / NULL になり、7-12 以降は数値の引数が TypeError）。
    /// 中継 DLL に `{fn_name}_tl` が無いときの誤り。入らなかった理由が分かっていればそれを書く（7-28）。
    pub fn missing_symbol_error(&self, e: &dyn std::fmt::Display) -> String {
        match &self.unavailable {
            Some(why) => format!("RuntimeError: C function '{}' is not in the bridge: {why}", self.fn_name),
            None => format!("RuntimeError: symbol '{}' not found: {e}", self.fn_name),
        }
    }

    pub fn omitted_arg(&self, i: usize) -> Result<Value, String> {
        use crate::cpp_header::types::CDefault;
        match self.param_defaults.get(i).copied().flatten() {
            Some(CDefault::Int(n)) => Ok(Value::Int(n)),
            Some(CDefault::Float(f)) => Ok(Value::Float(f)),
            Some(CDefault::Bool(b)) => Ok(Value::Bool(b)),
            Some(CDefault::Null) => Ok(Value::None),
            None => Err(format!(
                "TypeError: native function '{}' needs argument {} (it has no default value)",
                self.fn_name,
                i + 1
            )),
        }
    }
}

impl Clone for NativeFnRef {
    fn clone(&self) -> Self {
        Self {
            lib_path: self.lib_path.clone(),
            fn_name: self.fn_name.clone(),
            n_params: self.n_params,
            min_params: self.min_params,
            param_defaults: self.param_defaults.clone(),
            param_mutabilities: self.param_mutabilities.clone(),
            ptr_params: self.ptr_params.clone(),
            cached_fn_ptr: std::sync::atomic::AtomicUsize::new(
                self.cached_fn_ptr.load(std::sync::atomic::Ordering::Relaxed),
            ),
            typed_fn_ptr: std::sync::atomic::AtomicUsize::new(
                self.typed_fn_ptr.load(std::sync::atomic::Ordering::Relaxed),
            ),
            typed_sig: self.typed_sig.clone(),
            unavailable: self.unavailable.clone(),
            globals: self.globals,
        }
    }
}


/// Wrapper around `libloading::Library` that implements `Debug`.
///
/// ⚠ 評価コアビルド（`native` 無効）では `libloading` が無いので**構築不能な空型**にする
/// （評価コア切り出し #2）。`Value::NativeFunction` の variant 自体は残すので、
/// 網羅 `match` には触らない。
#[cfg(feature = "native")]
pub struct NativeLibWrapper(pub libloading::Library);

/// 評価コアビルドの `NativeLibWrapper`。構築不能。
#[cfg(not(feature = "native"))]
pub struct NativeLibWrapper(pub std::convert::Infallible);


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
        assert!(matches!(decode_out_ptr(enc, F64).unwrap(), Value::Float(f) if f == 5.0));
        // f32: 縮小格納 → 拡張読み出し（f32 で表現可能な値は往復不変）
        let enc = encode_out_ptr_init(&Value::Float(2.5), F32).unwrap();
        assert!(matches!(decode_out_ptr(enc, F32).unwrap(), Value::Float(f) if f == 2.5));
        // i32: 切り詰め格納・符号拡張読み出し
        let enc = encode_out_ptr_init(&Value::Int(-7), I32).unwrap();
        assert!(matches!(decode_out_ptr(enc, I32).unwrap(), Value::Int(-7)));
        // i64
        let enc = encode_out_ptr_init(&Value::Int(1 << 40), I64).unwrap();
        assert!(matches!(decode_out_ptr(enc, I64).unwrap(), Value::Int(n) if n == 1 << 40));
        // int → float 昇格（ハンドル経路の ar_to_float と同義）
        let enc = encode_out_ptr_init(&Value::Int(3), F64).unwrap();
        assert!(matches!(decode_out_ptr(enc, F64).unwrap(), Value::Float(f) if f == 3.0));
        // 型不一致（float を int* へ / 非数値）→ None（typed 経路を諦める）
        assert!(encode_out_ptr_init(&Value::Float(1.5), I32).is_none());
        assert!(encode_out_ptr_init(&Value::Bool(true), F64).is_none());
    }

    /// C の幅と符号どおりに広げる。符号なし 64 ビットの 2^63 以上は OverflowError（1-6）。
    #[test]
    fn out_ptr_decode_follows_c_width_and_sign() {
        use RawWidth::*;
        assert!(matches!(decode_out_ptr(0xFA, U8).unwrap(), Value::Int(250)));
        assert!(matches!(decode_out_ptr(0xFA, I8).unwrap(), Value::Int(-6)));
        assert!(matches!(decode_out_ptr(0xFFFE, I16).unwrap(), Value::Int(-2)));
        assert!(matches!(decode_out_ptr(0xFFFF_FFFF, U32).unwrap(), Value::Int(4294967295)));
        assert!(matches!(decode_out_ptr(42, U64).unwrap(), Value::Int(42)));
        assert!(matches!(decode_out_ptr(i64::MAX as u64, U64).unwrap(), Value::Int(i64::MAX)));
        let err = decode_out_ptr(u64::MAX, U64).unwrap_err();
        assert!(err.starts_with("OverflowError: "), "{err}");
        // 符号付き 64 ビットは同じビットでも負の値として読む（エラーではない）
        assert!(matches!(decode_out_ptr(u64::MAX, I64).unwrap(), Value::Int(-1)));
    }
}

#[cfg(test)]
mod marshal_tests {
    use super::*;

    fn layout() -> Arc<RawLayout> {
        Arc::new(RawLayout::from_fields(&[("x".into(), "int32".into())]).unwrap())
    }

    /// 構造体ポインタにインスタンス以外が来たら、ハンドル経路へ落とさずエラー（ffi_value_exchange_plan.md 1-2）。
    /// 値渡しの構造体は従来どおりハンドル経路へ（フィールドから組み立てられる・1-3）。
    #[test]
    fn struct_pointer_needs_an_instance() {
        let mut ta = TypedArgs::new();
        let ptr = AbiTy::Ptr { mutable: false, by_value: false, layout: layout(), elem: Arc::from("P") };
        let err = ta.marshal(&[Value::Int(5)], &[ptr], &[]).err().expect("must be an error");
        assert!(err.contains("instance of the C struct") && err.contains("or a Ptr") && err.contains("'int'"), "{err}");

        let mut ta = TypedArgs::new();
        let by_value = AbiTy::Ptr { mutable: false, by_value: true, layout: layout(), elem: Arc::from("P") };
        assert!(matches!(ta.marshal(&[Value::Int(5)], &[by_value], &[]), Ok(Marshalled::TypeMismatch)));
    }

    fn borrowed(address: u64, elem: &str) -> Value {
        let h = crate::interpreter::PtrHandle::new(address, elem, 0, None, crate::interpreter::PtrOwner::Borrowed, None)
            .unwrap();
        Value::Ptr(Arc::new(h))
    }

    /// 型つき経路の `bool`・`bytes`・`Ptr`（4-4）。規則はハンドル経路と共有（`value/ptr.rs`）。
    #[test]
    fn typed_bool_bytes_and_pointers() {
        let mut ta = TypedArgs::new();
        let vals = [Value::Bool(true), Value::bytes(b"ab".to_vec()), Value::None, borrowed(0x40, "Db"), Value::None];
        let tys = [
            AbiTy::Bool,
            AbiTy::Bytes,
            AbiTy::Bytes,
            AbiTy::PtrVal { elem: Arc::from("Db") },
            AbiTy::PtrVal { elem: Arc::from("void") },
        ];
        assert!(matches!(ta.marshal(&vals, &tys, &[]), Ok(Marshalled::Ready)));
        assert_eq!(ta.slots[0], 1);
        // SAFETY: スロットは `ta.cleanups` が生かしている NUL 終端のコピーを指す
        let s = unsafe { std::ffi::CStr::from_ptr(ta.slots[1] as usize as *const std::ffi::c_char) };
        assert_eq!(s.to_bytes(), b"ab");
        assert_eq!(ta.slots[2], 0, "None is NULL");
        assert_eq!(ta.slots[3], 0x40);
        assert_eq!(ta.slots[4], 0);
        // 合わない値はハンドル経路へ落とさずエラー
        let mut ta = TypedArgs::new();
        let e = ta.marshal(&[Value::str("x")], &[AbiTy::Bytes], &[]).err().unwrap();
        assert!(e.contains("needs bytes, not 'str'"), "{e}");
        let e = ta.marshal(&[borrowed(0x40, "Db")], &[AbiTy::PtrVal { elem: Arc::from("File") }], &[]).err().unwrap();
        assert!(e.contains("Ptr[File]"), "{e}");
        // 構造体ポインタへ `Ptr[構造体]`（値渡しは None を受けない）
        let ptr = AbiTy::Ptr { mutable: false, by_value: false, layout: layout(), elem: Arc::from("P") };
        let mut ta = TypedArgs::new();
        assert!(matches!(ta.marshal(&[borrowed(0x80, "P")], &[ptr], &[]), Ok(Marshalled::Ready)));
        assert_eq!(ta.slots[0], 0x80);
        let by_value = AbiTy::Ptr { mutable: false, by_value: true, layout: layout(), elem: Arc::from("P") };
        let e = TypedArgs::new().marshal(&[Value::None], &[by_value], &[]).err().unwrap();
        assert!(e.contains("by value"), "{e}");
    }

    /// `ArPtrProtocol` の引数（4-4）: `Ptr` から組み立て、書き込み用ポインタで C が `address` を 0 にしたら使えなくする。
    /// 値渡し・`const` では解放関数を渡さない。
    #[test]
    fn typed_protocol_arguments() {
        let p = borrowed(0x100, "int32");
        let mut ta = TypedArgs::new();
        assert!(matches!(
            ta.marshal(&[p.clone(), p.clone()], &[AbiTy::Protocol { writable: false }, AbiTy::Protocol { writable: true }], &[]),
            Ok(Marshalled::Ready)
        ));
        // SAFETY: スロットは `ta.cleanups` が生かしている構造体を指す
        let ro = unsafe { &*(ta.slots[0] as usize as *const super::super::PtrProtocolRaw) };
        assert_eq!(ro.address, 0x100);
        assert_eq!(unsafe { std::ffi::CStr::from_ptr(ro.elem_type) }.to_bytes(), b"int32");
        assert_eq!(ro.length, -1, "a raw pointer has no length");
        assert!(ro.release.is_none());
        // C の側で解放して address を 0 に書き戻した
        let rw = unsafe { &mut *(ta.slots[1] as usize as *mut super::super::PtrProtocolRaw) };
        rw.address = 0;
        for c in std::mem::take(&mut ta.cleanups) {
            finish_ptr_arg_cleanup(c);
        }
        let Value::Ptr(h) = &p else { unreachable!() };
        assert!(!h.is_live(), "the library freed it");
    }

    /// 戻り値（4-4）: `char*` は bytes、ほかのポインタは借用の Ptr、NULL は None、`bool` は真偽値。
    #[test]
    fn typed_pointer_returns() {
        let text = b"hey\0";
        let v = decode_typed_ret(&AbiTy::CStr { wide: false }, text.as_ptr() as u64).unwrap();
        assert!(matches!(v, Value::Bytes(b) if &*b == b"hey"));
        assert!(matches!(decode_typed_ret(&AbiTy::CStr { wide: false }, 0).unwrap(), Value::None));
        let v = decode_typed_ret(&AbiTy::PtrVal { elem: Arc::from("Db") }, 0x40).unwrap();
        assert!(matches!(&v, Value::Ptr(p) if p.elem_type() == "Db"));
        assert!(matches!(decode_typed_ret(&AbiTy::PtrVal { elem: Arc::from("Db") }, 0).unwrap(), Value::None));
        assert!(matches!(decode_typed_ret(&AbiTy::Bool, 1).unwrap(), Value::Bool(true)));
    }

    /// 数値への書き込みポインタに数値でない値が来たらエラー。数値どうしの食い違いは従来どおりハンドル経路へ。
    #[test]
    fn out_pointer_needs_a_number() {
        let mut ta = TypedArgs::new();
        let err = ta
            .marshal(&[Value::None], &[AbiTy::OutPtr { width: RawWidth::F64 }], &[])
            .err()
            .expect("must be an error");
        assert!(err.contains("must be a number") && err.contains("NoneType"), "{err}");

        let mut ta = TypedArgs::new();
        assert!(matches!(
            ta.marshal(&[Value::Float(1.5)], &[AbiTy::OutPtr { width: RawWidth::I32 }], &[]),
            Ok(Marshalled::TypeMismatch)
        ));
    }
}
