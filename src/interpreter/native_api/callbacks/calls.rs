// native_api/callbacks/calls.rs — 関数・メソッド・組み込みの呼び出しのコールバック。

use super::*;

/// 純スカラー引数のハンドル 1 個を u64 スロット値へデコードする（#83 で切り出し）。
///
/// ⚠ **アリーナから直接読む**（`Value` のクローンを挟まない）。これが `TypedArgs::marshal` を
/// 通さない唯一の理由で、STATE アクセスが ~8 回 → 1〜2 回になる（#76 で残した例外）。
/// 型の許容表は共有マーシャラと同一に保つこと（`I64` は int のみ・`F64` は float と int）。
#[inline]
fn scalar_handle_slot(st: &NativeCallState, h: i64, ty: &crate::interpreter::value::AbiTy) -> Option<u64> {
    use crate::interpreter::value::AbiTy;
    let (iv, fv): (Option<i64>, Option<f64>) = match h {
        x if (3..INT_CACHE_BASE as i64).contains(&x) => (Some(x - 3), None),
        x if x >= INT_CACHE_BASE as i64 => match st.arena.get(x as usize) {
            Some(Value::Int(v)) => (Some(*v), None),
            Some(Value::Float(f)) => (None, Some(*f)),
            _ => return None,
        },
        _ => return None, // None/Bool/sentinel → ハンドル経路へ
    };
    match (iv, fv, ty) {
        (Some(v), _, AbiTy::I64) => Some(v as u64),
        (Some(v), _, AbiTy::F64) => Some((v as f64).to_bits()),
        (_, Some(f), AbiTy::F64) => Some(f.to_bits()),
        _ => None,
    }
}

/// 全引数が純スカラーのときのマーシャリング（#83 で切り出し）。`false` ＝ ハンドル経路へ。
///
/// ⚠ **1 回の STATE ボローで全引数を処理する**のが要点（借用を跨がないこと）。
#[inline]
fn marshal_scalar_handles(
    ta: &mut crate::interpreter::value::TypedArgs,
    params: &[crate::interpreter::value::AbiTy],
    args_ptr: *const i64,
    n: usize,
) -> bool {
    with_state(|s| {
        let st = s.borrow();
        for (i, ty) in params.iter().enumerate().take(n) {
            let h = unsafe { *args_ptr.add(i) };
            match scalar_handle_slot(&st, h, ty) {
                Some(v) => ta.slots[i] = v,
                None => return false,
            }
        }
        true
    })
}

/// 統一 typed ABI 最速パス（ネイティブ → C DLL）の試行（#83 で `ar_call_fn` から切り出し）。
///
/// 引数ハンドルを 1 回の STATE ボローで u64 スロットへ展開し、`{name}_typed` を直接呼ぶ。
/// `enter_native_call` / `exit_native_call`・per-arg unmarshal コールバック・結果 marshal が
/// すべて消える（STATE アクセス ~8 回 → 1〜2 回）。
///
/// - `Some(h)`: この経路で決着した（エラーも `STATE.error` に載せて `TL_NONE` を返す形で含む）
/// - `None`: この経路に載らない ⇒ **呼び出し側がハンドル経路へ落とす**
///
/// ⚠ この層は named-variable の概念がない（アリーナハンドル止まり）ため、
/// `named_mut` は常に空＝**書き戻しなし**（安全側・#76）。
///
/// ⚠⚠ **#83 以前はこれが `ar_call_fn` に直書きで、ガードが 4 段入れ子になっていた**
/// （制御ネスト 8 ＝ src 最深）。早期 return へ反転してある — **`if` を足して深くしないこと**。
///
/// ⚠⚠ **`#[inline(always)]` を外さないこと**（#83）。呼び出し元は `ar_call_fn` の 1 箇所だけで、
/// #83 以前は文字どおりインライン展開されていたコード。切り出しただけで生成コードが変わると
/// **cdll ベンチが 0.94x に動く**（実測。⚠ cdll は `ar_call_fn` を通らないので
/// **これは経路の速度ではなくコード配置の話** — 属性を戻すだけで 1.04x に振れた）。
#[inline(always)]
fn try_typed_fast_call(fn_val: &Value, args_ptr: *const i64, n_args: i32) -> Option<i64> {
    use crate::interpreter::value::{AbiTy, Marshalled, TypedArgs};
    use std::sync::atomic::Ordering;

    let Value::NativeFunction(fn_ref) = fn_val else {
        return None;
    };
    let sig = fn_ref.typed_sig.as_ref()?;
    let typed_ptr = fn_ref.typed_fn_ptr.load(Ordering::Relaxed);
    let n = n_args as usize;
    if typed_ptr == 0 || n != sig.params.len() || n > 16 {
        return None;
    }

    // 数値（`I64` / `F64`）以外の引数（構造体ポインタ・書き込みポインタ・`bool`・`bytes`・`Ptr`・`ArPtrProtocol`）を
    // 含む場合は、値に戻して共有の `marshal` を通す（P3/P4・4-4 — .claude/skills/c-abi-interop/SKILL.md）。
    let has_ptr = sig.params.iter().any(|p| !matches!(p, AbiTy::I64 | AbiTy::F64));
    // ⚠⚠ **move 禁止**（`slots` が `out_locals` を指す）。ここに置いたまま使う。
    let mut ta = TypedArgs::new();

    let ok = if !has_ptr {
        marshal_scalar_handles(&mut ta, &sig.params, args_ptr, n)
    } else {
        // 1 回の STATE ボローでハンドルを Value に解決（Instance は Rc クローンのみで安価）、
        // 以降は STATE の外で処理する。マーシャリング本体は共有実装（#76）。
        let vals: Vec<Value> = with_state(|s| {
            let st = s.borrow();
            (0..n).map(|i| st.clone_value(unsafe { *args_ptr.add(i) })).collect()
        });
        match ta.marshal(&vals, &sig.params, &[]) {
            Ok(Marshalled::Ready) => true,
            Ok(Marshalled::TypeMismatch) => false,
            Err(e) => {
                with_state(|s| s.borrow_mut().error = Some(e));
                return Some(TL_NONE);
            }
        }
    };
    if !ok {
        return None; // 型不一致 → 既存のハンドル経路にフォールバック
    }

    // 呼び出し本体は 4 経路で共有（#54 / #76）。⚠ この層はエラーを戻り値で返せないので
    // `STATE.error` に載せて TL_NONE を返す — 違いはそこだけ。
    // SAFETY: `typed_ptr` は typed_sig 付き（`build_cpp_typed_sig` 検証済み）の関数。
    // 戻り値は呼び出し本体が引数のバッファの生きているうちにデコードする（4-4・`invoke_typed_abi`）。
    // ⚠ OutPtr の書き戻しは無い（名前が無いので安全側 ＝ `ta.out_wb` は常に空）。
    match unsafe {
        crate::interpreter::value::invoke_typed_abi(typed_ptr, &ta.slots, std::mem::take(&mut ta.cleanups), &sig.ret)
    } {
        // 小整数はキャッシュ済みハンドル（`push_handle` が STATE を触らない）・`None` は TL_NONE
        Ok(v) => Some(push_handle(v)),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            Some(TL_NONE)
        }
    }
}

/// 関数ハンドルと引数ハンドル配列を受け取り関数を呼び出す C コールバック。
///
/// Fast path: `Value::NativeFunction` はインタープリタを経由せず直接 DLL 関数を呼び出す。
/// Slow path: クロージャ・ユーザー定義関数などは `call_value_with_args` に委譲する。
pub(super) extern "C" fn ar_call_fn(fn_h: i64, args_ptr: *const i64, n_args: i32) -> i64 {
    use std::sync::atomic::Ordering;

    // Phase 1: extract fn_val and interp_ptr, check for pending error.
    let (has_err, fn_val, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(fn_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }

    // ── 統一 typed ABI 最速パス（ネイティブ→C DLL） ─────────────────────────
    // 本体は下の `try_typed_fast_call`（#83 で切り出した）。`None` ＝ この経路に載らない
    // ので、そのままハンドル経路へ落ちる。
    if let Some(h) = try_typed_fast_call(&fn_val, args_ptr, n_args) {
        return h;
    }

    // Fast path: NativeFunction (native DLL).
    // Arg handles are already live in the arena — pass them straight through.
    // ⚠ `.arc` の関数で個数が合わない（既定値を省いた）呼び出しは下の遅い道で宣言どおりに束縛する（1-11）。
    //   生成した `_tl` は渡された個数を見ずに仮引数の数だけ読むので、ここで素通しすると引数の配列の外を読む。
    if let Value::NativeFunction(ref fn_ref) = fn_val {
        if !interp_ptr.is_null() && (fn_ref.globals.is_none() || n_args as usize == fn_ref.n_params) {
            let is_outermost = enter_native_call(interp_ptr);

            // DLL: use cached_fn_ptr; resolve on first call.
            let cached = fn_ref.cached_fn_ptr.load(Ordering::Relaxed);
            let fn_ptr = if cached != 0 {
                cached
            } else {
                let sym_name = format!("{}_tl\0", fn_ref.fn_name);
                let interp = unsafe { &*interp_ptr };
                match interp.native_libs.get(&fn_ref.lib_path) {
                    Some(lib) => {
                        match unsafe {
                            lib.0.get::<unsafe extern "C" fn(*const i64, i32) -> i64>(
                                sym_name.as_bytes(),
                            )
                        } {
                            Ok(f) => {
                                let fp = *f as usize;
                                fn_ref.cached_fn_ptr.store(fp, Ordering::Relaxed);
                                fp
                            }
                            Err(e) => {
                                with_state(|s| s.borrow_mut().error = Some(fn_ref.missing_symbol_error(&e)));
                                abort_native_call(is_outermost);
                                return TL_NONE;
                            }
                        }
                    }
                    None => {
                        with_state(|s| s.borrow_mut().error = Some(format!(
                            "RuntimeError: native library not loaded: {}",
                            fn_ref.lib_path.display()
                        )));
                        abort_native_call(is_outermost);
                        return TL_NONE;
                    }
                }
            };
            // `let` 仮引数は `copy_value` で複製してから渡す（6-2・D-8）。ここには引数の可変性が無いので保守的に
            // `mut` とみなす。⚠ 以前はハンドルを素通しし、`let` で受けた値が呼び出し元の後の変更で変わりえた（1-5 と同じ形）。
            let mut owned: Vec<i64> = (0..n_args as usize).map(|i| unsafe { *args_ptr.add(i) }).collect();
            // 複製を作る前のアリーナの長さ（1-15）。呼び出しの後でここまで巻き戻し、複製を呼び出しごとに捨てる。
            let mut copied_from: Option<u64> = None;
            for (i, h) in owned.iter_mut().enumerate() {
                if !fn_ref.param_mutabilities.get(i).copied().unwrap_or(true)
                    && fn_ref.ptr_params.get(i).copied().unwrap_or(crate::interpreter::PtrParam::None)
                        == crate::interpreter::PtrParam::None
                {
                    copied_from.get_or_insert_with(|| with_state(|s| s.borrow().arena.len() as u64));
                    match unsafe { &mut *interp_ptr }.copy_value(clone_value_at(*h)) {
                        Ok(v) => *h = push_handle(v),
                        Err(e) => {
                            with_state(|s| s.borrow_mut().error = Some(e));
                            abort_native_call(is_outermost);
                            return TL_NONE;
                        }
                    }
                }
            }
            // コンパイル済みモジュールの関数はそのモジュールの大域で走る（1-7）
            let prev = unsafe { &mut *interp_ptr }.enter_native_globals(fn_ref.globals);
            let result_h = unsafe {
                let f: unsafe extern "C" fn(*const i64, i32) -> i64 =
                    std::mem::transmute(fn_ptr);
                f(owned.as_ptr(), n_args)
            };
            unsafe { &mut *interp_ptr }.leave_native_globals(prev);

            if with_state(|s| s.borrow().error.is_some()) {
                abort_native_call(is_outermost);
                return TL_NONE;
            }
            // 複製を呼び出しごとに捨てる（1-15）。いちばん外の呼び出しなら下の `exit_native_call` が全部片づける
            let result_h = match copied_from {
                Some(saved) if !is_outermost => ar_arena_compact(result_h, saved),
                _ => result_h,
            };
            let result_val = exit_native_call(result_h, is_outermost);
            return push_handle(result_val);
        }
    }

    // Slow path: HV functions, closures, overloaded fns, etc.
    let args: Vec<Value> = (0..n_args as usize)
        .map(|i| clone_value_at(unsafe { *args_ptr.add(i) }))
        .collect();
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some(
            "NativeError: interpreter not set for call_fn".to_string()
        ));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.call_value_with_args(fn_val, args) {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

/// field 32（以前に生成された `.arc` と中継コードが使う）。値渡しの複製はしない（引数の可変性が分からない）。
/// 今の生成コードは `ar_call_method_m`（field 55）を使う。
pub(super) extern "C" fn ar_call_method(
    obj_h: i64,
    name_ptr: *const u8,
    name_len: i32,
    args_ptr: *const i64,
    n_args: i32,
) -> i64 {
    let Some(name) = native_str(name_ptr, name_len, "method name") else { return TL_NONE };
    call_method_common(obj_h, name, args_ptr, n_args, None, None, 0)
}

/// 名前によるメソッド呼び出し＋値渡しの複製（field 55・native_object_access_plan.md 1-5・D-8）。
/// `mut_mask` は bit 0 = 受け手が `mut` の値か、bit i+1 = 引数 i が `mut` の値か（VM の `arg_is_mutable` と同じ規則で
/// 生成コードが決める）。`mut` の値を `let self` で受けるなら受け手を `deep_copy_value`、`let` 仮引数で受けるなら
/// 引数を `copy_value`（`__copy__` があればそれ）で複製してから呼ぶ（VM の `try_fast_bind` と同じ）。
/// ⚠ 以前はネイティブのメソッドへハンドルを素通しし、呼ばれた側の `let` が呼び出し元の後の変更で変わっていた。
pub(super) extern "C" fn ar_call_method_m(
    obj_h: i64,
    name_ptr: *const u8,
    name_len: i32,
    args_ptr: *const i64,
    n_args: i32,
    mut_mask: u32,
) -> i64 {
    let Some(name) = native_str(name_ptr, name_len, "method name") else { return TL_NONE };
    call_method_common(obj_h, name, args_ptr, n_args, Some(mut_mask), None, 0)
}

/// 名前によるメソッド呼び出しの、箇所ごとのセルと FFI の検査の情報つきの版（field 73・11-3・D-12）。
///
/// VM の `CallMethodLocal` と同じく、名前は `&str` のまま振り分けへ渡し（検査はセルが空のときだけ・8-2 の `site_name`）、
/// 組み込み型の受け手は `vm_method_call_other` と同じ手順（外部言語なら戻り値を宣言型と照合）で呼ぶ。
/// セルの値は「名前を検査した」印（1）だけ。
pub(super) extern "C" fn ar_call_method_ic(
    cell: *mut u64,
    obj_h: i64,
    name_ptr: *const u8,
    name_len: i32,
    args_ptr: *const i64,
    n_args: i32,
    mut_mask: u32,
    info_ptr: *const u8,
    info_len: i32,
) -> i64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    let fresh = take_fresh_mask();
    let Some(name) = site_name(name_ptr, name_len, cell, "method name") else { return TL_NONE };
    if !cell.is_null() {
        // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（8 バイト境界）で、`AtomicU64` と同じ並び。
        unsafe { &*(cell as *const AtomicU64) }.store(1, Ordering::Relaxed);
    }
    let info = if info_ptr.is_null() {
        None
    } else {
        let Some(info) = native_str(info_ptr, info_len, "call info") else { return TL_NONE };
        Some(info)
    };
    call_method_common(obj_h, name, args_ptr, n_args, Some(mut_mask), info, fresh)
}

/// 生成コードが埋め込んだ FFI の検査の情報（`表示名 \x1f 宣言型 \x1f 位置`）で戻り値を照合する（VM の
/// `check_ffi_return` と同じ検査・同じ文言）。宣言型はコンパイルのときに `InferredType::from_ann` で読み戻せることを
/// 確かめてある（読み戻せない型の呼び出しを含む関数はコンパイルしない）ので、読めなければ壊れた `.arc` としてエラーにする。
fn check_native_ffi_return(
    interp: &mut crate::interpreter::Interpreter,
    lang: &str,
    value: Value,
    info: &str,
) -> Result<Value, String> {
    let mut parts = info.splitn(3, '\x1f');
    let (Some(name), Some(decl), Some(where_)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(format!("NativeError: malformed call info from native code: {info:?}"));
    };
    let Some(declared) = crate::type_check::InferredType::from_ann(decl) else {
        return Err(format!("NativeError: the declared type `{decl}` of `{name}` from native code cannot be read"));
    };
    interp.check_ffi_return_declared(lang, value, &declared, name, &|| where_.to_string())
}

/// `mut_mask` の引数 `i`（bit i+1）が `mut` の値か。31 個目より後の引数はビットが無いので保守的に `mut`
/// （生成コードの `method_mut_mask` と同じ解釈）。
pub(super) fn mask_arg_is_mut(mask: u32, i: usize) -> bool {
    i + 1 >= 32 || mask & (1 << (i + 1)) != 0
}

/// 名前によるメソッド呼び出しの共通部。`name` は生成コードの定数（呼び出しの間は有効）なので複製しない（11-3）。
/// `ffi` は FFI の検査の情報（field 73 だけが渡す・`check_native_ffi_return`）。
fn call_method_common(
    obj_h: i64,
    name: &str,
    args_ptr: *const i64,
    n_args: i32,
    mut_mask: Option<u32>,
    ffi: Option<&str>,
    fresh: u32,
) -> i64 {

    let (has_err, obj, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }

    // Native dispatch: check NATIVE_METHODS table first.
    if let Value::Instance(ref inst_rc) = obj {
        let native = crate::interpreter::native_api::lookup_native_method(inst_rc.borrow().class.class_id, name);
        let fn_ptr = native.map(|(ptr, _)| ptr);
        // 値渡しの複製（mask があるとき）に要る呼ばれた側の仮引数の可変性。オーバーロードがあれば
        // どれが呼ばれるか分からないので、ネイティブへ直行せずインタープリタの振り分けへ回す。
        let callee_params: Option<Vec<bool>> = match (mut_mask, fn_ptr) {
            (Some(_), Some(_)) => {
                let cls = inst_rc.borrow().class.clone();
                match cls.methods.get(name) {
                    Some(ov) if ov.len() == 1 => Some(ov[0].params.iter().map(|p| p.mutable).collect()),
                    _ => None,
                }
            }
            _ => None,
        };
        // ⚠ 個数が宣言と合わない（既定値を省いた・1-11）呼び出しはインタープリタの振り分けへ回す（宣言どおりに束縛
        //   する）。生成した `_tl` は渡された個数を見ずに仮引数の数だけ読むので、直行すると引数の配列の外を読む。
        let arity_ok = {
            let cls = inst_rc.borrow().class.clone();
            match cls.methods.get(name) {
                Some(ov) if ov.len() == 1 => ov[0].params.len() == n_args as usize + 1,
                _ => false,
            }
        };
        let direct = fn_ptr.filter(|_| arity_ok && (mut_mask.is_none() || callee_params.is_some()));
        if let Some(ptr) = direct {
            let mut all_args = Vec::with_capacity(1 + n_args as usize);
            // 複製を作る前のアリーナの長さ（1-15）。呼び出しの後でここまで巻き戻し、複製を呼び出しごとに捨てる。
            let mut copied_from: Option<u64> = None;
            let arena_len = || with_state(|s| s.borrow().arena.len() as u64);
            match (mut_mask, &callee_params) {
                (Some(mask), Some(pm)) => {
                    if interp_ptr.is_null() {
                        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set for call_method '{name}'")));
                        return TL_NONE;
                    }
                    let interp = unsafe { &mut *interp_ptr };
                    // 受け手（param 0 = self）
                    let self_let = pm.first().is_some_and(|m| !m);
                    // 読むだけのメソッドへは複製せずに渡す（4-4・VM と同じ判定）。引くのは複製が起きるときだけ
                    let wants_copy = (self_let && mask & 1 != 0)
                        || (0..n_args as usize).any(|i| pm.get(i + 1).is_some_and(|m| !m) && mask_arg_is_mut(mask, i));
                    let borrow = wants_copy && interp.may_borrow_for(name);
                    if self_let && mask & 1 != 0 && borrow {
                        all_args.push(obj_h);
                    } else if self_let && mask & 1 != 0 {
                        copied_from.get_or_insert_with(arena_len);
                        crate::interpreter::copy_stats::record(crate::interpreter::copy_stats::CopyKind::LetSelf, &obj, false);
                        all_args.push(push_handle(crate::interpreter::Interpreter::deep_copy_value(obj.clone())));
                    } else {
                        all_args.push(obj_h);
                    }
                    for i in 0..n_args as usize {
                        let h = unsafe { *args_ptr.add(i) };
                        let param_let = pm.get(i + 1).is_some_and(|m| !m);
                        if param_let && mask_arg_is_mut(mask, i) && borrow && !interp.has_copy_hook(&clone_value_at(h)) {
                            all_args.push(h);
                        } else if param_let && mask_arg_is_mut(mask, i) {
                            copied_from.get_or_insert_with(arena_len);
                            // 作りたての印の付いた引数は、欄の 1 個を数えずに共有を判定する（2-4。写さなければ同じ欄を渡す）
                            let is_fresh = i < 31 && fresh & (1 << (i + 1)) != 0 && h >= crate::interpreter::native_api::INT_CACHE_BASE as i64;
                            let val = clone_value_at(h);
                            let copied = if is_fresh {
                                interp.copy_value_held(val.clone(), 2).map(|v| {
                                    if crate::interpreter::Interpreter::same_heap_value(&v, &val) { h } else { push_handle(v) }
                                })
                            } else {
                                interp.copy_value(val).map(push_handle)
                            };
                            match copied {
                                Ok(nh) => all_args.push(nh),
                                Err(e) => {
                                    with_state(|s| s.borrow_mut().error = Some(e));
                                    return TL_NONE;
                                }
                            }
                        } else {
                            all_args.push(h);
                        }
                    }
                }
                _ => {
                    all_args.push(obj_h);
                    for i in 0..n_args as usize {
                        all_args.push(unsafe { *args_ptr.add(i) });
                    }
                }
            }
            // メソッドはクラスを定義したモジュールの大域で、受け手のクラスの文脈で走る（1-7・1-9）
            if interp_ptr.is_null() {
                with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set for call_method '{name}'")));
                return TL_NONE;
            }
            let Some((_, globals)) = native else { unreachable!("`ptr` comes from `native`") };
            let class = inst_rc.borrow().class.clone();
            let prev = unsafe { &mut *interp_ptr }.enter_native_method(globals, class);
            let r = unsafe {
                let func: unsafe extern "C" fn(*const i64, i32) -> i64 = std::mem::transmute(ptr);
                func(all_args.as_ptr(), all_args.len() as i32)
            };
            unsafe { &mut *interp_ptr }.leave_native_method(prev);
            return match copied_from {
                Some(saved) => ar_arena_compact(r, saved),
                None => r,
            };
        }
    }

    // Interpreter fallback.
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set for call_method '{name}'")));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    // 引数の可変性: mask があればそれ（bit i+1）、無ければ（古い生成コード）保守的に `mut` とみなす。
    // インタープリタの束縛が `let` 仮引数 ← `mut` の値を複製する。
    // 引数は 1 つの `Vec` に直接（11-3・以前は値の `Vec` を作ってから詰め直していた）
    let evaled: Vec<(Option<String>, Value, bool)> = (0..n_args as usize)
        .map(|i| {
            let is_mut = match mut_mask {
                Some(mask) => mask_arg_is_mut(mask, i),
                None => true,
            };
            (None, arg_value(args_ptr, i, fresh), is_mut)
        })
        .collect();
    // インスタンスは VM の `CallMethod` と同じ入口へ、受け手の可変性（bit 0）を添えて渡す（6-1）。
    // `let self` は `mut` の受け手だけ複製する。mask の無い古い生成コードは保守的に `mut`。
    let r = if matches!(obj, Value::Instance(_)) {
        let recv_mut = mut_mask.is_none_or(|m| m & 1 != 0);
        interp.call_instance_method_evaled(obj, name, evaled, None, None, recv_mut)
    } else {
        // VM の `vm_method_call_other` と同じ手順: 呼ぶ前に外部言語かを見て、戻り値を宣言型と照合する（11-3）。
        // 情報が無い（型検査が戻り値の型を付けていない・古い `.arc` の field 55）ときは VM の未採番と同じく照合しない
        let lang = crate::interpreter::Interpreter::foreign_call_lang(&obj, name);
        let r = interp.eval_method_call_evaled(obj, name, evaled);
        match (r, lang, ffi) {
            (Ok(v), Some(l), Some(info)) => check_native_ffi_return(interp, l, v, info),
            (r, _, _) => r,
        }
    };
    match r {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

pub(super) extern "C" fn ar_fn_trampoline(_fn_h: i64) -> *const () {
    ar_call_fn as *const ()
}

/// 組み込み関数を名前で呼ぶ（field 50・native_object_access_plan.md 1-3）。本体は VM の `CallBuiltin` と同じ
/// `Interpreter::call_builtin_by_name_evaled`（同名のグローバル束縛があればそちらを呼ぶ）。
/// ⚠ 以前は `get_global` で組み込み関数を値として引いており、値として登録されていない
///   `range` / `print` / `repr` / `enumerate` / `zip` / `getenv` がコンパイル済みコードで `NameError` になっていた。
pub(super) extern "C" fn ar_call_builtin(name_ptr: *const u8, name_len: i32, args_ptr: *const i64, n_args: i32) -> i64 {
    let Some(name) = native_str(name_ptr, name_len, "builtin name") else { return TL_NONE };
    let (has_err, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (calling '{name}')")));
        return TL_NONE;
    }
    let args: Vec<Value> = (0..n_args.max(0) as usize)
        .map(|i| clone_value_at(unsafe { *args_ptr.add(i) }))
        .collect();
    let interp = unsafe { &mut *interp_ptr };
    match interp.call_builtin_by_name_evaled(name, args) {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}
