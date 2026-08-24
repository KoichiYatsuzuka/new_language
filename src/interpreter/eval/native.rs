// eval/native.rs — ネイティブ(コンパイル済みモジュール)関数のディスパッチと呼び出し。

use {
    std::sync::Arc,
    crate::ast::CallArg,
    crate::interpreter::{
        Interpreter, NativeFnRef, Value,
    },
    // typed ABI の仕組み（引数マーシャリング・呼び出し・戻り値デコード）は
    // すべて `value::native` に 1 箇所で置いてある（#54 / #76）。
    crate::interpreter::value::{decode_typed_ret, invoke_typed_abi, Marshalled, TypedArgs},
};

/// 引数 `i` が名前付き変数（`Expr::Ident`）ならその名前（#76）。
fn arg_ident_name(args: &[CallArg], i: usize) -> Option<&str> {
    match args.get(i).map(|a| a.expr()) {
        Some(crate::ast::Expr::Ident { name, .. }) => Some(name),
        _ => None,
    }
}

/// `wb_mask`（VM が知っている「書き戻し先を持つ引数」のビットマスク）を
/// [`TypedArgs::marshal`] の `named_mut` 形式へ変換する（#76）。
///
/// ⚠ マスクは `Some(true)` / `None` しか作れない。**`Some(false)`（名前付き `let`）は
/// 表現できない**ので、この経路は `let` を書き込みポインタへ渡してもエラーにならず
/// **単に書き戻さない**（安全側）。ツリーウォーク側は元の式を見て `Some(false)` を作れる。
fn named_mut_from_mask(wb_mask: u32) -> [Option<bool>; 16] {
    // ⚠ **ヒープ確保しないこと**。ここは FFI 呼び出しごとに通る（typed 経路の引数上限が
    // 16 なので固定長で足りる）。`Vec` にすると cdll ベンチが 0.90x に落ちる（実測）。
    std::array::from_fn(|i| ((wb_mask >> i) & 1 == 1).then_some(true))
}

impl Interpreter {
    /// `Value::NativeFunction` を呼び出す（ハンドルベース ABI）。
    ///
    /// 全引数をバリューアリーナのハンドルに変換し、C ABI ラッパーを呼ぶ。
    /// 結果ハンドルをアリーナから取り出して返す。
    /// `enter_native_call` / `exit_native_call` でアリーナのセーブポイントを管理し、
    /// 呼び出しツリーが終わると一括クリーンアップする。
    /// 引数式から [`TypedArgs::marshal`] の `named_mut` を作る（ツリーウォーク経路・#76）。
    ///
    /// ⚠ こちらは **3 状態すべて**を作れる（マスク経路は `Some(false)` を作れない）。
    /// これが 4 経路で唯一違ってよい入力。
    fn named_mut_from_args(&self, args: &[CallArg]) -> [Option<bool>; 16] {
        // ⚠ **ヒープ確保しないこと**（`named_mut_from_mask` と同じ理由）。
        std::array::from_fn(|i| match arg_ident_name(args, i) {
            Some(name) => self.get_var(name).map(|v| v.is_mutable()),
            None => None,
        })
    }

    pub(crate) fn call_native_function(
        &mut self,
        fn_ref: &Arc<NativeFnRef>,
        args: &[crate::ast::CallArg],
    ) -> Result<Value, String> {
        // #55: AST 式を取るツリーウォーク入口の通過を数える（既定ビルドでは消える）。
        crate::interpreter::tw_stats::record_site(1);
        use crate::interpreter::PtrParam;

        // Fast path: no write-back parameters（判定は VM 側と同一実装・#48）
        if !fn_ref.has_writeback() {
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
            if let Some(crate::ast::Expr::Ident { name, .. }) = args.get(i).map(|a| a.expr()) {
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
            let typed_ptr = fn_ref.typed_fn_ptr.load(std::sync::atomic::Ordering::Relaxed);
            if typed_ptr != 0 && vals.len() == sig.params.len() && vals.len() <= 16 {
                // ツリーウォークは元の引数式を見られるので **3 状態すべて**を作れる
                // （`Some(false)` ＝ 名前付き `let`。書き込みポインタへ渡すと `Err`）。
                let named_mut = self.named_mut_from_args(args);
                // ⚠ **move 禁止**（`slots` が `out_locals` を指す）。ここに置いたまま使う。
                let mut ta = TypedArgs::new();
                if matches!(
                    ta.marshal(&vals, &sig.params, &named_mut)?,
                    Marshalled::Ready
                ) {
                    // SAFETY: `typed_ptr` は typed_sig 付き（`build_cpp_typed_sig` 検証済み）の関数。
                    let ret = unsafe {
                        invoke_typed_abi(typed_ptr, &ta.slots, std::mem::take(&mut ta.cleanups))?
                    };
                    // ⚠ **4 経路で違ってよいのはここだけ**（#76）。こちらは**自分で変数へ代入**する。
                    // ⚠ `out_wb` は `named_mut == Some(true)` のときだけ積まれる ＝ 必ず名前がある。
                    for (i, width) in std::mem::take(&mut ta.out_wb) {
                        let val = ta.decode_out(i, width);
                        let name = arg_ident_name(args, i)
                            .expect("out_wb は named mut と判定した引数にだけ積まれる")
                            .to_string();
                        self.assign_var(&name, val)?;
                    }
                    return Ok(decode_typed_ret(&sig.ret, ret));
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
                    if let Some(crate::ast::Expr::Ident { name: n, .. }) = args.get(i).map(|a| a.expr()) {
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
        // #55: AST 式を取るツリーウォーク入口の通過を数える（既定ビルドでは消える）。
        crate::interpreter::tw_stats::record_site(2);
        let sig = fn_ref.typed_sig.as_ref().expect("cache guarantees typed_sig");

        // ⚠ **引数はここで全部評価する**（#76 で 3 経路と揃えた）。以前はマーシャリングと
        // 交互に評価し、型不一致で打ち切ると**残りの引数式が一度も評価されなかった**。
        // 他の 2 経路（`eval_call_args`）は元から全部評価しているので、こちらが例外だった。
        let mut vals: Vec<Value> = Vec::with_capacity(args.len());
        for arg in args {
            vals.push(self.eval(arg.expr())?);
        }

        let named_mut = self.named_mut_from_args(args);
        // ⚠ OutPtr へ名前付き `let` を渡す誤りは、ハンドル経路（`call_native_function` の
        // MutPtr 事前チェック）と同じ規則でここでも拒否する
        //（`AbiTy::OutPtr` は必ず `PtrParam::MutPtr`。どちらも `CType::Ptr{mutable:true}` 由来）。
        for (i, ty) in sig.params.iter().enumerate() {
            if matches!(ty, crate::interpreter::value::AbiTy::OutPtr { .. })
                && named_mut.get(i).copied().flatten() == Some(false)
            {
                let name = arg_ident_name(args, i).unwrap_or("");
                return Err(format!(
                    "TypeError: pointer parameter {i} requires a `mut` variable, '{name}' is not mutable"
                ));
            }
        }

        // ⚠ **move 禁止**（`slots` が `out_locals` を指す）。ここに置いたまま使う。
        let mut ta = TypedArgs::new();
        if !matches!(
            ta.marshal(&vals, &sig.params, &named_mut)?,
            Marshalled::Ready
        ) {
            // 型不一致 → 評価済みの値をそのままハンドル経路へ渡す（副作用の二重実行はない）。
            let arc = any_arc
                .clone()
                .downcast::<NativeFnRef>()
                .expect("cache holds NativeFnRef");
            return self.dispatch_native_evaled(&arc, vals);
        }

        let typed_ptr = fn_ref.typed_fn_ptr.load(std::sync::atomic::Ordering::Relaxed);
        // SAFETY: この経路はインラインキャッシュ命中時のみで、キャッシュには typed_sig 付きの
        // `NativeFnRef` しか入らない（`eval_call` の充填条件）。
        let ret =
            unsafe { invoke_typed_abi(typed_ptr, &ta.slots, std::mem::take(&mut ta.cleanups))? };
        // ⚠ **4 経路で違ってよいのはここだけ**（#76）。こちらは**自分で変数へ代入**する。
        for (i, width) in std::mem::take(&mut ta.out_wb) {
            let val = ta.decode_out(i, width);
            let name = arg_ident_name(args, i)
                .expect("out_wb は named mut と判定した引数にだけ積まれる")
                .to_string();
            self.assign_var(&name, val)?;
        }
        Ok(decode_typed_ret(&sig.ret, ret))
    }

    pub(crate) fn dispatch_native_evaled(
        &mut self,
        fn_ref: &Arc<NativeFnRef>,
        vals: Vec<Value>,
    ) -> Result<Value, String> {
        // 書き戻し先を持たない呼び出し（＝従来の全経路）。**唯一の実装は `_wb` 版**に置き、
        // ここは「書き戻し先ゼロ」で委譲するだけにする（`*_evaled` 版とずれた実装を作らない）。
        self.dispatch_native_evaled_wb(fn_ref, vals, 0, &mut Vec::new())
    }

    /// `dispatch_native_evaled` の書き戻し対応版（#48）。
    ///
    /// `wb_mask` の bit i が立っている引数は「**呼び出し元が書き戻し先を知っている
    /// 名前付き mut 変数**」で、ツリーウォークの `call_native_function` が
    /// 引数式を `Expr::Ident` と判定したのと同じ意味を持つ。C が書いた値は
    /// `wb_out` に `(arg index, 値)` で積んで返し、**格納は呼び出し元が行う**
    /// （VM のローカルは `vm_stack` の slot にあり、ここからは触れないため）。
    ///
    /// ⚠ `wb_mask` を 0 にすると従来どおり**書き戻しをしない**。判定不能な呼び出し元
    /// （`call_value_evaled` 経由など）はそのまま 0 を渡せばよい。
    pub(crate) fn dispatch_native_evaled_wb(
        &mut self,
        fn_ref: &Arc<NativeFnRef>,
        mut vals: Vec<Value>,
        wb_mask: u32,
        wb_out: &mut Vec<(u8, Value)>,
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
            let typed_ptr = fn_ref.typed_fn_ptr.load(std::sync::atomic::Ordering::Relaxed);
            if typed_ptr != 0 && vals.len() == sig.params.len() && vals.len() <= 16 {
                // 書き戻し先を知っている引数だけ `Some(true)`。⚠ **`Some(false)` は作れない**
                // （元の `CallArg` 式が無いので「名前付き let」を判別できない）＝ 安全側。
                let named_mut = named_mut_from_mask(wb_mask);
                // ⚠ **move 禁止**（`slots` が `out_locals` を指す）。ここに置いたまま使う。
                let mut ta = TypedArgs::new();
                if matches!(
                    ta.marshal(&vals, &sig.params, &named_mut)?,
                    Marshalled::Ready
                ) {
                    // SAFETY: `typed_ptr` は typed_sig 付き（`build_cpp_typed_sig` 検証済み）の関数。
                    let ret = unsafe {
                        invoke_typed_abi(typed_ptr, &ta.slots, std::mem::take(&mut ta.cleanups))?
                    };
                    // ⚠ **4 経路で違ってよいのはここだけ**（#76）。こちらは**呼び出し元へ返す**
                    // （VM のローカルは `vm_stack` の slot にあり、ここからは触れない・#48）。
                    for (i, width) in std::mem::take(&mut ta.out_wb) {
                        wb_out.push((i as u8, ta.decode_out(i, width)));
                    }
                    return Ok(decode_typed_ret(&sig.ret, ret));
                }
            }
        }

        let is_outermost = crate::interpreter::native_api::enter_native_call(self as *mut Interpreter);

        // ハンドル経路の書き戻し（#48）: `MutPtr` パラメータかつ `wb_mask` が立っている
        // 引数には**書き込み可能なアリーナ枠**を渡す。`call_native_function` の
        // handles 構築と同じ規則（あちらは `Expr::Ident` か、こちらは mask）。
        let mut handle_wb: Vec<(u8, i64)> = Vec::new();
        let handles: Vec<i64> = vals
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let pp = fn_ref.ptr_params.get(i).copied().unwrap_or(crate::interpreter::PtrParam::None);
                if pp == crate::interpreter::PtrParam::MutPtr && (wb_mask >> i) & 1 == 1 {
                    let h = crate::interpreter::native_api::push_handle_writeback(v.clone());
                    handle_wb.push((i as u8, h));
                    return h;
                }
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
                // ⚠ **`exit_native_call` がアリーナを切り詰める前に**読み出すこと（#48）。
                // `call_native_function` の `updated` と同じ順序・同じ関数を使う。
                for (i, h) in &handle_wb {
                    wb_out.push((*i, crate::interpreter::native_api::clone_value_at(*h)));
                }
                Ok(crate::interpreter::native_api::exit_native_call(result_h, is_outermost))
            }
        }
    }

    // --- ネイティブコールバック用ヘルパー ---

}
