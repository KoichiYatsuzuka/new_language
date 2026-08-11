// eval/native.rs — ネイティブ(コンパイル済みモジュール)関数のディスパッチと呼び出し。

use crate::ast::Resolution;
use {
    std::sync::Arc,
    crate::ast::CallArg,
    crate::interpreter::{
        Interpreter, NativeFnRef, Value,
    },
};

impl Interpreter {
    /// `Value::NativeFunction` を呼び出す（ハンドルベース ABI）。
    ///
    /// 全引数をバリューアリーナのハンドルに変換し、C ABI ラッパーを呼ぶ。
    /// 結果ハンドルをアリーナから取り出して返す。
    /// `enter_native_call` / `exit_native_call` でアリーナのセーブポイントを管理し、
    /// 呼び出しツリーが終わると一括クリーンアップする。
    pub(crate) fn call_native_function(
        &mut self,
        fn_ref: &Arc<NativeFnRef>,
        args: &[crate::ast::CallArg],
    ) -> Result<Value, String> {
        use crate::interpreter::PtrParam;

        // Fast path: no write-back parameters
        let has_writeback = fn_ref.ptr_params.contains(&PtrParam::MutPtr);
        if !has_writeback {
            let evaled = self.eval_call_args(args)?;
            let vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
            return self.dispatch_native_evaled(fn_ref, vals);
        }

        // Write-back path — verify MutPtr args that are named variables are mutable.
        // Non-variable expressions (literals, calls, etc.) are allowed but receive no write-back.
        for (i, pp) in fn_ref.ptr_params.iter().enumerate() {
            if *pp != PtrParam::MutPtr {
                continue;
            }
            if let Some(crate::ast::Expr::Ident { name, res: Resolution::Unresolved, .. }) = args.get(i).map(|a| a.expr()) {
                let is_mut = self.get_var(name).map(|v| v.is_mutable()).unwrap_or(false);
                if !is_mut {
                    return Err(format!(
                        "TypeError: pointer parameter {i} requires a `mut` variable, '{}' is not mutable",
                        name
                    ));
                }
            }
        }

        let evaled = self.eval_call_args(args)?;
        let mut vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();

        if vals.len() < fn_ref.min_params || vals.len() > fn_ref.n_params {
            let expected = if fn_ref.min_params == fn_ref.n_params {
                format!("{}", fn_ref.n_params)
            } else {
                format!("{}–{}", fn_ref.min_params, fn_ref.n_params)
            };
            return Err(format!(
                "TypeError: native function '{}' expects {} argument(s), got {}",
                fn_ref.fn_name,
                expected,
                vals.len()
            ));
        }
        // Pad missing optional args with None (handle 0 → NULL for pointer params)
        while vals.len() < fn_ref.n_params {
            vals.push(Value::None);
        }

        // ── 統一 typed ABI 高速パス ──────────────────────────────────────────
        // `status = fn(args*, ret*, err*)` の直接 C ABI 呼び出し。
        // TLS・アリーナ・ハンドルを一切通らない。raise は ErrSlot 経由で伝播する。
        // 引数の実行時型がシグネチャと合わない場合はハンドル経路へフォールバック。
        // 構造体ポインタ引数（AbiTy::Ptr）は resolve_typed_ptr_arg でゼロコピー／
        // シャドウ変換のどちらかを解決する（P3/P4 — .claude/skills/c-abi-interop/SKILL.md）。
        if let Some(sig) = &fn_ref.typed_sig {
            use crate::interpreter::value::{AbiTy, PtrArgCleanup};
            let typed_ptr = fn_ref.typed_fn_ptr.load(std::sync::atomic::Ordering::Relaxed);
            if typed_ptr != 0 && vals.len() == sig.params.len() && vals.len() <= 16 {
                let mut slots = [0u64; 16];
                // OutPtr（プリミティブ書き込みポインタ）用のローカル領域。
                // スロットにはこの要素のアドレスが入るため、呼び出しが終わるまで生存する。
                let mut out_locals = [0u64; 16];
                let mut out_wb: Vec<(usize, crate::interpreter::value::RawWidth, String)> =
                    Vec::new();
                let mut cleanups: Vec<PtrArgCleanup> = Vec::new();
                let mut ptr_err: Option<String> = None;
                let mut ok = true;
                for (i, (v, ty)) in vals.iter().zip(&sig.params).enumerate() {
                    match (v, ty) {
                        (Value::Int(n), AbiTy::I64) => slots[i] = *n as u64,
                        (Value::Float(f), AbiTy::F64) => slots[i] = f.to_bits(),
                        // int → float 引数の自動昇格（ハンドル経路の ar_to_float と同義）
                        (Value::Int(n), AbiTy::F64) => slots[i] = (*n as f64).to_bits(),
                        (_, AbiTy::Ptr { mutable, by_value, layout }) => {
                            let named_mut = match args.get(i).map(|a| a.expr()) {
                                Some(crate::ast::Expr::Ident { name, res: Resolution::Unresolved, .. }) => {
                                    self.get_var(name).map(|v| v.is_mutable())
                                }
                                _ => None,
                            };
                            match crate::interpreter::value::resolve_typed_ptr_arg(
                                v, *mutable, *by_value, layout, named_mut,
                            ) {
                                Ok(Some((slot, cleanup))) => {
                                    slots[i] = slot;
                                    cleanups.push(cleanup);
                                }
                                Ok(None) => {
                                    ok = false;
                                    break;
                                }
                                Err(e) => {
                                    ptr_err = Some(e);
                                    ok = false;
                                    break;
                                }
                            }
                        }
                        // プリミティブ書き込みポインタ（`double*` 等）: 初期値を width 幅で
                        // エンコードしたローカルのアドレスを渡し、呼び出し後に名前付き
                        // `mut` 変数へ書き戻す（let 拒否は冒頭の MutPtr 事前チェック済み）。
                        (_, AbiTy::OutPtr { width }) => {
                            match crate::interpreter::value::encode_out_ptr_init(v, *width) {
                                Some(enc) => {
                                    out_locals[i] = enc;
                                    slots[i] = std::ptr::addr_of_mut!(out_locals[i]) as u64;
                                    if let Some(crate::ast::Expr::Ident { name, res: Resolution::Unresolved, .. }) =
                                        args.get(i).map(|a| a.expr())
                                    {
                                        out_wb.push((i, *width, name.clone()));
                                    }
                                }
                                None => {
                                    ok = false;
                                    break;
                                }
                            }
                        }
                        _ => {
                            ok = false;
                            break;
                        }
                    };
                }
                if let Some(e) = ptr_err {
                    return Err(e);
                }
                if ok {
                    let mut ret: u64 = 0;
                    let mut err = crate::interpreter::native_api::ErrSlot::default();
                    let status = unsafe {
                        let f: unsafe extern "C" fn(
                            *const u64,
                            *mut u64,
                            *mut crate::interpreter::native_api::ErrSlot,
                        ) -> u32 = std::mem::transmute(typed_ptr);
                        f(slots.as_ptr(), &mut ret, &mut err)
                    };
                    for c in cleanups {
                        crate::interpreter::value::finish_ptr_arg_cleanup(c);
                    }
                    if status != 0 {
                        // 既存の raise 経路と同じ "TypeName: msg" 形式で伝播
                        return Err(err.to_error_string());
                    }
                    // OutPtr の書き戻し（C が書いた値を named mut 変数へ反映 — 成功時のみ）
                    for (i, width, name) in out_wb {
                        let val = crate::interpreter::value::decode_out_ptr(out_locals[i], width);
                        self.assign_var(&name, val)?;
                    }
                    return Ok(match sig.ret {
                        AbiTy::I64 => Value::Int(ret as i64),
                        AbiTy::F64 => Value::Float(f64::from_bits(ret)),
                        AbiTy::Void => Value::None,
                        // typed ABI の戻り値に Ptr/OutPtr は使わない（build_cpp_typed_sig が除外する）。
                        AbiTy::Ptr { .. } | AbiTy::OutPtr { .. } => {
                            unreachable!("typed ABI ret excludes Ptr/OutPtr")
                        }
                    });
                }
            }
        }

        let is_outermost = crate::interpreter::native_api::enter_native_call(self as *mut Interpreter);

        // Push handles; MutPtr params get writable arena slots
        let mut writebacks: Vec<(String, i64)> = Vec::new();
        let handles: Vec<i64> = vals
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let pp = fn_ref.ptr_params.get(i).copied().unwrap_or(PtrParam::None);
                if pp == PtrParam::MutPtr {
                    // Only do write-back when the argument is a named mut variable.
                    // Literals / expressions are passed read-only (no write-back needed).
                    if let Some(crate::ast::Expr::Ident { name: n, res: Resolution::Unresolved, .. }) = args.get(i).map(|a| a.expr()) {
                        let h = crate::interpreter::native_api::push_handle_writeback(v.clone());
                        writebacks.push((n.clone(), h));
                        h
                    } else {
                        crate::interpreter::native_api::push_handle(v.clone())
                    }
                } else {
                    let is_mut = fn_ref.param_mutabilities.get(i).copied().unwrap_or(true);
                    let owned = if is_mut {
                        v.clone()
                    } else {
                        Self::deep_copy_value(v.clone())
                    };
                    crate::interpreter::native_api::push_handle(owned)
                }
            })
            .collect();

        let call_result = {
            use std::sync::atomic::Ordering;
            let cached = fn_ref.cached_fn_ptr.load(Ordering::Relaxed);
            let fn_ptr = if cached != 0 {
                cached
            } else {
                let lib = match self.native_libs.get(&fn_ref.lib_path) {
                    Some(l) => l,
                    None => {
                        crate::interpreter::native_api::abort_native_call(is_outermost);
                        return Err(format!(
                            "RuntimeError: native library not loaded: {}",
                            fn_ref.lib_path.display()
                        ));
                    }
                };
                let symbol_name = format!("{}_tl\0", fn_ref.fn_name);
                match unsafe {
                    lib.0.get::<unsafe extern "C" fn(*const i64, i32) -> i64>(symbol_name.as_bytes())
                } {
                    Ok(func) => {
                        let fp = *func as usize;
                        fn_ref.cached_fn_ptr.store(fp, Ordering::Relaxed);
                        fp
                    }
                    Err(e) => {
                        crate::interpreter::native_api::abort_native_call(is_outermost);
                        return Err(format!("RuntimeError: symbol '{}' not found: {e}", fn_ref.fn_name));
                    }
                }
            };
            unsafe {
                let func: unsafe extern "C" fn(*const i64, i32) -> i64 = std::mem::transmute(fn_ptr);
                Ok(func(handles.as_ptr(), handles.len() as i32))
            }
        };

        match call_result {
            Err(e) => {
                crate::interpreter::native_api::abort_native_call(is_outermost);
                Err(e)
            }
            Ok(result_h) => {
                if let Some(err) = crate::interpreter::native_api::take_error() {
                    crate::interpreter::native_api::abort_native_call(is_outermost);
                    return Err(err);
                }
                if result_h == crate::interpreter::native_api::TL_EXCEPTION {
                    crate::interpreter::native_api::abort_native_call(is_outermost);
                    if let Some((type_name, msg)) = crate::interpreter::native_api::take_pending_raise() {
                        return Err(format!("{type_name}: {msg}"));
                    }
                    return Err("NativeError: CB_RAISE called but no pending raise".to_string());
                }
                // Read back write-back values BEFORE exit_native_call truncates the arena
                let updated: Vec<(String, Value)> = writebacks
                    .iter()
                    .map(|(name, h)| (name.clone(), crate::interpreter::native_api::clone_value_at(*h)))
                    .collect();
                let result = crate::interpreter::native_api::exit_native_call(result_h, is_outermost);
                // Assign updated values back to the mut variables
                for (name, val) in updated {
                    self.assign_var(&name, val)?;
                }
                Ok(result)
            }
        }
    }

    /// Core native-dispatch path: push already-evaluated args into the arena and invoke
    /// `{fn_name}_tl` via libloading (シンボルは初回解決後キャッシュされる)。
    /// Used by both `call_native_function` (called
    /// from AST) and `call_value_with_args` (called from native callbacks).
    /// インラインキャッシュ命中時の typed ディスパッチ（AST 焼き込み経路）。
    ///
    /// 引数式を直接 u64 スロット配列（スタック上）へ評価し、スコープ検索・TLS・
    /// アリーナ・ハンドルを一切通らずに `{name}_typed` を呼び出す。
    /// raise は ErrSlot（スタック上）経由で status=1 とともに届く。
    /// 実行時の引数型がシグネチャと合わない場合はハンドル経路へフォールバックする
    /// （評価済みの値はスロットから復元するため副作用の二重実行はない）。
    pub(crate) fn dispatch_native_typed_exprs(
        &mut self,
        fn_ref: &NativeFnRef,
        any_arc: &Arc<dyn std::any::Any + Send + Sync>,
        args: &[CallArg],
    ) -> Result<Value, String> {
        use crate::interpreter::value::{AbiTy, PtrArgCleanup};
        let sig = fn_ref.typed_sig.as_ref().expect("cache guarantees typed_sig");
        let mut slots = [0u64; 16];
        // OutPtr（プリミティブ書き込みポインタ）用のローカル領域（呼び出し終了まで生存）。
        let mut out_locals = [0u64; 16];
        let mut out_wb: Vec<(usize, crate::interpreter::value::RawWidth, String)> = Vec::new();
        // 評価済みの値を常に保持しておく（フォールバック時にスロットから復元する必要がなくなる）。
        let mut evaled: Vec<Value> = Vec::with_capacity(args.len());
        let mut cleanups: Vec<PtrArgCleanup> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let v = self.eval(arg.expr())?;
            let ok = match (&v, &sig.params[i]) {
                (Value::Int(n), AbiTy::I64) => {
                    slots[i] = *n as u64;
                    true
                }
                (Value::Float(f), AbiTy::F64) => {
                    slots[i] = f.to_bits();
                    true
                }
                // int → float 引数の自動昇格
                (Value::Int(n), AbiTy::F64) => {
                    slots[i] = (*n as f64).to_bits();
                    true
                }
                (_, AbiTy::Ptr { mutable, by_value, layout }) => {
                    let named_mut = match arg.expr() {
                        crate::ast::Expr::Ident { name, res: Resolution::Unresolved, .. } => {
                            self.get_var(name).map(|v| v.is_mutable())
                        }
                        _ => None,
                    };
                    match crate::interpreter::value::resolve_typed_ptr_arg(
                        &v, *mutable, *by_value, layout, named_mut,
                    ) {
                        Ok(Some((slot, cleanup))) => {
                            slots[i] = slot;
                            cleanups.push(cleanup);
                            true
                        }
                        Ok(None) => false,
                        Err(e) => return Err(e),
                    }
                }
                // プリミティブ書き込みポインタ（`double*` 等）: ローカルのアドレスを渡し、
                // 呼び出し後に名前付き `mut` 変数へ書き戻す。let 変数はエラー
                // （ハンドル経路 call_native_function の事前チェックと同じ規則）。
                (_, AbiTy::OutPtr { width }) => {
                    match crate::interpreter::value::encode_out_ptr_init(&v, *width) {
                        Some(enc) => {
                            out_locals[i] = enc;
                            slots[i] = std::ptr::addr_of_mut!(out_locals[i]) as u64;
                            if let crate::ast::Expr::Ident { name, res: Resolution::Unresolved, .. } = arg.expr() {
                                let is_mut =
                                    self.get_var(name).map(|v| v.is_mutable()).unwrap_or(false);
                                if !is_mut {
                                    return Err(format!(
                                        "TypeError: pointer parameter {i} requires a `mut` variable, '{name}' is not mutable"
                                    ));
                                }
                                out_wb.push((i, *width, name.clone()));
                            }
                            true
                        }
                        None => false,
                    }
                }
                _ => false,
            };
            evaled.push(v);
            if !ok {
                // 型不一致 → ここまで評価済みの値をそのままハンドル経路へ渡す
                // （副作用の二重実行はない — 各引数式は一度しか eval していない）。
                let arc = any_arc
                    .clone()
                    .downcast::<NativeFnRef>()
                    .expect("cache holds NativeFnRef");
                return self.dispatch_native_evaled(&arc, evaled);
            }
        }
        let typed_ptr = fn_ref.typed_fn_ptr.load(std::sync::atomic::Ordering::Relaxed);
        let mut ret: u64 = 0;
        let mut err = crate::interpreter::native_api::ErrSlot::default();
        let status = unsafe {
            let f: unsafe extern "C" fn(
                *const u64,
                *mut u64,
                *mut crate::interpreter::native_api::ErrSlot,
            ) -> u32 = std::mem::transmute(typed_ptr);
            f(slots.as_ptr(), &mut ret, &mut err)
        };
        for c in cleanups {
            crate::interpreter::value::finish_ptr_arg_cleanup(c);
        }
        if status != 0 {
            return Err(err.to_error_string());
        }
        // OutPtr の書き戻し（C が書いた値を named mut 変数へ反映 — 成功時のみ）
        for (i, width, name) in out_wb {
            let val = crate::interpreter::value::decode_out_ptr(out_locals[i], width);
            self.assign_var(&name, val)?;
        }
        Ok(match sig.ret {
            AbiTy::I64 => Value::Int(ret as i64),
            AbiTy::F64 => Value::Float(f64::from_bits(ret)),
            AbiTy::Void => Value::None,
            AbiTy::Ptr { .. } | AbiTy::OutPtr { .. } => {
                unreachable!("typed ABI ret excludes Ptr/OutPtr")
            }
        })
    }

    pub(crate) fn dispatch_native_evaled(
        &mut self,
        fn_ref: &Arc<NativeFnRef>,
        mut vals: Vec<Value>,
    ) -> Result<Value, String> {
        if vals.len() < fn_ref.min_params || vals.len() > fn_ref.n_params {
            let expected = if fn_ref.min_params == fn_ref.n_params {
                format!("{}", fn_ref.n_params)
            } else {
                format!("{}–{}", fn_ref.min_params, fn_ref.n_params)
            };
            return Err(format!(
                "TypeError: native function '{}' expects {} argument(s), got {}",
                fn_ref.fn_name,
                expected,
                vals.len()
            ));
        }
        while vals.len() < fn_ref.n_params {
            vals.push(Value::None);
        }

        // ── 統一 typed ABI 高速パス ──────────────────────────────────────────
        // `status = fn(args*, ret*, err*)` の直接 C ABI 呼び出し。
        // TLS・アリーナ・ハンドルを一切通らない。raise は ErrSlot 経由で伝播する。
        // 引数の実行時型がシグネチャと合わない場合はハンドル経路へフォールバック。
        //
        // 構造体ポインタ引数（AbiTy::Ptr）にも対応する。このパスは元の CallArg 式に
        // アクセスできないため named-mut 判定は常に `None`（let/mut どちらでも許可 —
        // 書き戻しは「名前付き mut 変数」のときのみ発生するため、判定不能な場合は
        // 単に書き戻しをしないだけで安全側に倒れる）。
        // 重要な呼び出し元: `has_writeback`（MutPtr パラメータの有無）が false の
        // 関数はすべて `call_native_function` からここへ直接来る — つまり
        // const 構造体ポインタのみを取る関数（`VectorInnerProduct` 等）は
        // **このパスでしか typed 高速化を受けられない**。
        if let Some(sig) = &fn_ref.typed_sig {
            use crate::interpreter::value::{AbiTy, PtrArgCleanup};
            let typed_ptr = fn_ref.typed_fn_ptr.load(std::sync::atomic::Ordering::Relaxed);
            if typed_ptr != 0 && vals.len() == sig.params.len() && vals.len() <= 16 {
                let mut slots = [0u64; 16];
                // OutPtr 用ローカル領域。この経路は CallArg 情報がなく named-mut 判定が
                // できないため書き戻しは行わない（Ptr の named_mut=None と同じ安全側）。
                let mut out_locals = [0u64; 16];
                let mut cleanups: Vec<PtrArgCleanup> = Vec::new();
                let mut ptr_err: Option<String> = None;
                let mut ok = true;
                for (i, (v, ty)) in vals.iter().zip(&sig.params).enumerate() {
                    match (v, ty) {
                        (Value::Int(n), AbiTy::I64) => slots[i] = *n as u64,
                        (Value::Float(f), AbiTy::F64) => slots[i] = f.to_bits(),
                        // int → float 引数の自動昇格（ハンドル経路の ar_to_float と同義）
                        (Value::Int(n), AbiTy::F64) => slots[i] = (*n as f64).to_bits(),
                        (_, AbiTy::Ptr { mutable, by_value, layout }) => {
                            match crate::interpreter::value::resolve_typed_ptr_arg(
                                v, *mutable, *by_value, layout, None,
                            ) {
                                Ok(Some((slot, cleanup))) => {
                                    slots[i] = slot;
                                    cleanups.push(cleanup);
                                }
                                Ok(None) => {
                                    ok = false;
                                    break;
                                }
                                Err(e) => {
                                    ptr_err = Some(e);
                                    ok = false;
                                    break;
                                }
                            }
                        }
                        (_, AbiTy::OutPtr { width }) => {
                            match crate::interpreter::value::encode_out_ptr_init(v, *width) {
                                Some(enc) => {
                                    out_locals[i] = enc;
                                    slots[i] = std::ptr::addr_of_mut!(out_locals[i]) as u64;
                                }
                                None => {
                                    ok = false;
                                    break;
                                }
                            }
                        }
                        _ => {
                            ok = false;
                            break;
                        }
                    };
                }
                if let Some(e) = ptr_err {
                    return Err(e);
                }
                if ok {
                    let mut ret: u64 = 0;
                    let mut err = crate::interpreter::native_api::ErrSlot::default();
                    let status = unsafe {
                        let f: unsafe extern "C" fn(
                            *const u64,
                            *mut u64,
                            *mut crate::interpreter::native_api::ErrSlot,
                        ) -> u32 = std::mem::transmute(typed_ptr);
                        f(slots.as_ptr(), &mut ret, &mut err)
                    };
                    for c in cleanups {
                        crate::interpreter::value::finish_ptr_arg_cleanup(c);
                    }
                    if status != 0 {
                        // 既存の raise 経路と同じ "TypeName: msg" 形式で伝播
                        return Err(err.to_error_string());
                    }
                    return Ok(match sig.ret {
                        AbiTy::I64 => Value::Int(ret as i64),
                        AbiTy::F64 => Value::Float(f64::from_bits(ret)),
                        AbiTy::Void => Value::None,
                        AbiTy::Ptr { .. } | AbiTy::OutPtr { .. } => {
                            unreachable!("typed ABI ret excludes Ptr/OutPtr")
                        }
                    });
                }
            }
        }

        let is_outermost = crate::interpreter::native_api::enter_native_call(self as *mut Interpreter);

        let handles: Vec<i64> = vals
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let is_mut = fn_ref.param_mutabilities.get(i).copied().unwrap_or(true);
                let owned = if is_mut {
                    v.clone()
                } else {
                    Self::deep_copy_value(v.clone())
                };
                crate::interpreter::native_api::push_handle(owned)
            })
            .collect();

        let call_result = {
            use std::sync::atomic::Ordering;
            let cached = fn_ref.cached_fn_ptr.load(Ordering::Relaxed);
            let fn_ptr = if cached != 0 {
                cached
            } else {
                let lib = match self.native_libs.get(&fn_ref.lib_path) {
                    Some(l) => l,
                    None => {
                        crate::interpreter::native_api::abort_native_call(is_outermost);
                        return Err(format!(
                            "RuntimeError: native library not loaded: {}",
                            fn_ref.lib_path.display()
                        ));
                    }
                };
                let symbol_name = format!("{}_tl\0", fn_ref.fn_name);
                match unsafe {
                    lib.0.get::<unsafe extern "C" fn(*const i64, i32) -> i64>(symbol_name.as_bytes())
                } {
                    Ok(func) => {
                        let fp = *func as usize;
                        fn_ref.cached_fn_ptr.store(fp, Ordering::Relaxed);
                        fp
                    }
                    Err(e) => {
                        crate::interpreter::native_api::abort_native_call(is_outermost);
                        return Err(format!("RuntimeError: symbol '{}' not found: {e}", fn_ref.fn_name));
                    }
                }
            };
            unsafe {
                let func: unsafe extern "C" fn(*const i64, i32) -> i64 = std::mem::transmute(fn_ptr);
                Ok(func(handles.as_ptr(), handles.len() as i32))
            }
        };

        match call_result {
            Err(e) => {
                crate::interpreter::native_api::abort_native_call(is_outermost);
                Err(e)
            }
            Ok(result_h) => {
                if let Some(err) = crate::interpreter::native_api::take_error() {
                    crate::interpreter::native_api::abort_native_call(is_outermost);
                    return Err(err);
                }
                if result_h == crate::interpreter::native_api::TL_EXCEPTION {
                    crate::interpreter::native_api::abort_native_call(is_outermost);
                    if let Some((type_name, msg)) = crate::interpreter::native_api::take_pending_raise() {
                        return Err(format!("{type_name}: {msg}"));
                    }
                    return Err("NativeError: CB_RAISE called but no pending raise".to_string());
                }
                Ok(crate::interpreter::native_api::exit_native_call(result_h, is_outermost))
            }
        }
    }

    // --- ネイティブコールバック用ヘルパー ---

}
