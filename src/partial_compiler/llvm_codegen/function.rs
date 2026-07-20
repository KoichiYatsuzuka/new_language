// llvm_codegen/function.rs — 関数単位の LLVM IR 出力: emit_fn / emit_fn_fast / emit_fn_typed / emit_gen_fn。

use crate::ast::{Param, Stmt};
use super::*;

impl<'a> GenCtx<'a> {
    pub(super) fn emit_fn(&mut self, name: &str, params: &[Param], ret_ann: Option<&str>, body: &[Stmt]) {
        let ret_ty = ann_ty(ret_ann);

        // Reset per-function state
        self.alloca_buf.clear();
        self.code_buf.clear();
        self.reg = 0;
        self.blk = 0;
        self.terminated = false;
        self.locals.clear();
        self.loop_stack.clear();
        self.block_stack.clear();
        self.preread_fields.clear();
        self.fn_param_trampolines.clear();
        self.current_fn_ret = ret_ty;

        // Populate param_classes for type-specialised field reads.
        self.param_classes.clear();
        self.flat_list_params.clear();
        for p in params {
            if let Some(ann) = &p.type_ann {
                if self.class_fields.contains_key(ann.as_str()) {
                    self.param_classes.insert(p.name.clone(), ann.clone());
                }
            }
        }

        // Populate flat_list_params for `let fixed_list[ClassName]` parameters.
        // Uses recursive collect_flat_leaves to support nested SWD classes.
        self.flat_list_params.clear();
        for p in params {
            if p.mutable { continue; } // only `let` params
            let Some(ann) = &p.type_ann else { continue };
            if !ann.starts_with("fixed_list[") || !ann.ends_with(']') { continue; }
            let class_name = &ann[11..ann.len() - 1]; // strip "fixed_list[" and "]"
            let leaves = collect_flat_leaves(self.all_class_fields, class_name, "", 0);
            if leaves.is_empty() { continue; }
            let stride = leaves.len() * 8;
            self.flat_list_params.insert(p.name.clone(), FlatListInfo { leaves, stride });
        }

        let param_sigs: Vec<String> = params.iter().enumerate()
            .map(|(i, _)| format!("i64 %_h{i}"))
            .collect();

        // Unwrap typed (int/float) params; store class-instance params as handles.
        for (i, p) in params.iter().enumerate() {
            let pt  = ann_ty(p.type_ann.as_deref());
            let st  = store_ty(pt);
            let ptr = self.alloca_var(&p.name, st);
            match pt {
                Ty::Int => {
                    let r = self.call_cb(CB_TO_INT, &[format!("i64 %_h{i}")]);
                    self.store_val(Ty::Int, &r, &ptr.clone());
                }
                Ty::Float => {
                    let r = self.call_cb(CB_TO_FLOAT, &[format!("i64 %_h{i}")]);
                    self.store_val(Ty::Float, &r, &ptr.clone());
                }
                _ => {
                    self.store_val(Ty::Handle, &format!("%_h{i}"), &ptr.clone());
                }
            }
        }

        // ── Approach-1 pre-reads: for class-instance params that are never written
        // in the body, read all typed fields once at function entry via a single
        // callback per field.  gen_expr then converts Expr::Attr accesses on these
        // params into plain loads (zero callbacks in the hot path).
        let mut has_preread = false;
        for (i, p) in params.iter().enumerate() {
            // self param in methods has type_ann = None; use current_class instead.
            let class_name = match &p.type_ann {
                Some(ann) if self.class_fields_ord.contains_key(ann.as_str()) => ann.clone(),
                None if p.name == "self" => match self.current_class.as_deref() {
                    Some(cls) if self.class_fields_ord.contains_key(cls) => cls.to_string(),
                    _ => continue,
                },
                _ => continue,
            };
            // Only pre-read if the param is never written in this function body.
            if body_writes_param(body, &p.name) { continue; }
            let fields = match self.class_fields_ord.get(&class_name) {
                Some(f) => f.clone(),
                None => continue,
            };
            for (field_name, field_ty) in &fields {
                let ptr = self.str_const(field_name.as_bytes());
                let len = field_name.len() as i32;
                let r = match field_ty {
                    Ty::Float => self.call_cb(CB_GET_FLOAT_FIELD, &[
                        format!("i64 %_h{i}"), ptr, format!("i32 {len}"),
                    ]),
                    Ty::Int => self.call_cb(CB_GET_INT_FIELD, &[
                        format!("i64 %_h{i}"), ptr, format!("i32 {len}"),
                    ]),
                    _ => continue,
                };
                let al = format!("%_prf_{}_{}", p.name, field_name);
                let tstr = llvm_ty(*field_ty);
                self.alloca_buf.push_str(&format!("  {al} = alloca {tstr}, align 8\n"));
                self.ec(&format!("store {tstr} {r}, ptr {al}"));
                self.preread_fields.insert(format!("{}.{field_name}", p.name), (al, *field_ty));
                has_preread = true;
            }
        }

        // Cache trampoline ptrs for `function[...]->R` typed parameters.
        // Called once at function entry so the hot path avoids the ArCallbacks GEP chain.
        for (i, p) in params.iter().enumerate() {
            let Some(ann) = &p.type_ann else { continue };
            if !ann.starts_with("function[") && !ann.starts_with("function{") { continue; }
            let tp_al = format!("%_tp_{}", p.name);
            self.alloca_buf.push_str(&format!("  {tp_al} = alloca ptr, align 8\n"));
            let tp = self.call_cb(CB_FN_TRAMPOLINE, &[format!("i64 %_h{i}")]);
            self.ec(&format!("store ptr {tp}, ptr {tp_al}"));
            self.fn_param_trampolines.insert(p.name.clone(), tp_al);
        }

        self.gen_stmts(body);

        // Fallback return (unreachable after optimiser)
        if ret_ty == Ty::Float {
            if !self.terminated { self.ec("ret double 0.0"); self.terminated = true; }
        } else {
            self.ret_handle("0");
        }

        let attr = export_attr();
        let vis  = if attr.is_empty() { "" } else { attr };
        let sig  = param_sigs.join(", ");

        // Float-returning functions use double ABI in _impl; others use i64 handle ABI.
        let impl_ret     = if ret_ty == Ty::Float { "double" } else { "i64" };
        let impl_ret_str = impl_ret;
        self.fn_defs.push_str(&format!(
            "\ndefine internal {impl_ret} @{name}_impl({sig}) {{\nentry:\n{}{}}}\n",
            self.alloca_buf, self.code_buf
        ));

        // Public wrapper: {name}_tl(ptr %args, i32 %n) -> i64 (handle for interpreter)
        let n = params.len();
        let mut wrapper = format!("\ndefine {vis}i64 @{name}_tl(ptr %args, i32 %_n) {{\n");
        let mut load_args: Vec<String> = Vec::new();
        for i in 0..n {
            wrapper.push_str(&format!("  %_a{i} = getelementptr inbounds i64, ptr %args, i32 {i}\n"));
            wrapper.push_str(&format!("  %_v{i} = load i64, ptr %_a{i}\n"));
            load_args.push(format!("i64 %_v{i}"));
        }
        let call_args_str = load_args.join(", ");
        if ret_ty == Ty::Float {
            wrapper.push_str(&format!("  %_raw = call double @{name}_impl({call_args_str})\n"));
            wrapper.push_str("  %_cb_tl = load ptr, ptr @CB\n");
            wrapper.push_str(&format!(
                "  %_mf_p = getelementptr inbounds %ArCallbacks, ptr %_cb_tl, i32 0, i32 {CB_MAKE_FLOAT}\n"
            ));
            wrapper.push_str("  %_mf = load ptr, ptr %_mf_p\n");
            wrapper.push_str("  %_res = call i64 (double) %_mf(double %_raw)\n");
            wrapper.push_str("  ret i64 %_res\n}\n");
        } else {
            wrapper.push_str(&format!("  %_res = call {impl_ret_str} @{name}_impl({call_args_str})\n"));
            wrapper.push_str("  ret i64 %_res\n}\n");
        }
        self.fn_defs.push_str(&wrapper);

        // Emit _fast variant if there were pre-reads (class params with typed fields).
        if has_preread {
            self.emit_fn_fast(name, params, ret_ty, body);
        }

        self.current_fn_ret = Ty::Handle;
        self.preread_fields.clear();
    }

    /// Emit `@{name}_fast(...)` — like _impl but receives class param fields as
    /// raw scalars instead of arena handles.  No callbacks inside the body; the
    /// function is pure arithmetic and LLVM can inline and hoist it freely.
    pub(super) fn emit_fn_fast(&mut self, name: &str, params: &[Param], ret_ty: Ty, body: &[Stmt]) {
        // Build fast signature: for each param, if it's a class instance with
        // pre-readable fields → expand to scalars; otherwise keep as i64 handle.
        let mut fast_sig: Vec<String> = Vec::new();
        // Mapping: (param_name, fast_llvm_reg_name, Ty) for each scalar field
        let mut fast_field_setup: Vec<(String, String, Ty)> = Vec::new();
        // Handle params (non-class or un-pre-readable) with their LLVM names
        let mut handle_idx = 0usize;

        for p in params {
            // self param in methods has type_ann = None; resolve via current_class.
            let class_name: Option<String> = match &p.type_ann {
                Some(ann) if self.class_fields_ord.contains_key(ann.as_str()) => Some(ann.clone()),
                None if p.name == "self" => self.current_class.as_deref()
                    .filter(|cls| self.class_fields_ord.contains_key(*cls))
                    .map(|cls| cls.to_string()),
                _ => None,
            };
            if let Some(cls) = class_name {
                if !body_writes_param(body, &p.name) {
                    if let Some(fields) = self.class_fields_ord.get(&cls).cloned() {
                        for (field_name, field_ty) in &fields {
                            let reg = format!("%_fp_{}__{field_name}", p.name);
                            let tstr = llvm_ty(*field_ty);
                            fast_sig.push(format!("{tstr} {reg}"));
                            fast_field_setup.push((
                                format!("{}.{field_name}", p.name),
                                reg,
                                *field_ty,
                            ));
                        }
                        continue; // skip handle param
                    }
                }
            }
            // Non-class or written param: pass as i64 handle
            let reg = format!("%_fph{handle_idx}");
            handle_idx += 1;
            fast_sig.push(format!("i64 {reg}"));
        }

        if fast_sig.is_empty() { return; }

        // Reset per-function codegen state for the fast variant
        self.alloca_buf.clear();
        self.code_buf.clear();
        self.reg = 0;
        self.blk = 0;
        self.terminated = false;
        self.locals.clear();
        self.loop_stack.clear();
        self.block_stack.clear();
        self.preread_fields.clear();
        self.current_fn_ret = ret_ty;

        // Rebuild param_classes
        self.param_classes.clear();
        for p in params {
            if let Some(ann) = &p.type_ann {
                if self.class_fields.contains_key(ann.as_str()) {
                    self.param_classes.insert(p.name.clone(), ann.clone());
                }
            }
        }

        // Populate preread_fields from the fast scalar params (no callbacks).
        let mut handle_idx2 = 0usize;
        for p in params {
            let class_name: Option<String> = match &p.type_ann {
                Some(ann) if self.class_fields_ord.contains_key(ann.as_str()) => Some(ann.clone()),
                None if p.name == "self" => self.current_class.as_deref()
                    .filter(|cls| self.class_fields_ord.contains_key(*cls))
                    .map(|cls| cls.to_string()),
                _ => None,
            };
            if let Some(cls) = class_name {
                if !body_writes_param(body, &p.name) {
                    if let Some(fields) = self.class_fields_ord.get(&cls).cloned() {
                        for (field_name, field_ty) in &fields {
                            let reg = format!("%_fp_{}__{field_name}", p.name);
                            let al  = format!("%_fpral_{}__{field_name}", p.name);
                            let tstr = llvm_ty(*field_ty);
                            self.alloca_buf.push_str(&format!("  {al} = alloca {tstr}, align 8\n"));
                            self.ec(&format!("store {tstr} {reg}, ptr {al}"));
                            self.preread_fields.insert(format!("{}.{field_name}", p.name), (al, *field_ty));
                        }
                        // Store the handle param as the local variable so existing code
                        // that reads it for CB_CALL_METHOD etc. still works (even though
                        // for a fully-typed method the handle is never needed in the body).
                        // We don't have the handle in the fast variant, so skip alloca_var.
                        continue;
                    }
                }
            }
            // Non-class param: unwrap from handle as in _impl
            let reg = format!("%_fph{handle_idx2}");
            handle_idx2 += 1;
            let pt  = ann_ty(p.type_ann.as_deref());
            let st  = store_ty(pt);
            let al_ptr = self.alloca_var(&p.name, st);
            match pt {
                Ty::Int   => { let r = self.call_cb(CB_TO_INT,   &[format!("i64 {reg}")]); self.store_val(Ty::Int,   &r, &al_ptr.clone()); }
                Ty::Float => { let r = self.call_cb(CB_TO_FLOAT, &[format!("i64 {reg}")]); self.store_val(Ty::Float, &r, &al_ptr.clone()); }
                _         => { self.store_val(Ty::Handle, &reg, &al_ptr.clone()); }
            }
        }
        let _ = fast_field_setup; // already handled above

        self.gen_stmts(body);

        if ret_ty == Ty::Float {
            if !self.terminated { self.ec("ret double 0.0"); self.terminated = true; }
        } else {
            self.ret_handle("0");
        }

        let impl_ret = if ret_ty == Ty::Float { "double" } else { "i64" };
        let sig = fast_sig.join(", ");
        self.fn_defs.push_str(&format!(
            "\ndefine internal {impl_ret} @{name}_fast({sig}) {{\nentry:\n{}{}}}\n",
            self.alloca_buf, self.code_buf
        ));

        self.current_fn_ret = Ty::Handle;
        self.preread_fields.clear();
    }

    /// `@{name}_typed(ptr %_args, ptr %_ret, ptr %_err) -> i32` — 統一 typed ABI。
    ///
    /// - 引数は u64 スロット列から生値（i64 / double ビットパターン）を直接ロード
    /// - 戻り値は `%_ret` に生値で格納、status 0 を返す
    /// - raise は `%_err`（ErrSlot）へ静的文字列を書いて status 1 を返す
    /// - コールバック（TLS・アリーナ）を一切使わない。使用が必要になった時点で破棄
    ///
    /// 戻り値: 生成に成功したら関数定義テキスト、コールバックが必要なら `None`。
    pub(super) fn emit_fn_typed(
        &mut self,
        name: &str,
        params: &[Param],
        ret_ann: Option<&str>,
        body: &[Stmt],
    ) -> Option<String> {
        let ret_ty = ann_ty(ret_ann);

        // Reset per-function state
        self.alloca_buf.clear();
        self.code_buf.clear();
        self.reg = 0;
        self.blk = 0;
        self.terminated = false;
        self.locals.clear();
        self.loop_stack.clear();
        self.block_stack.clear();
        self.preread_fields.clear();
        self.fn_param_trampolines.clear();
        self.param_classes.clear();
        self.flat_list_params.clear();
        self.current_fn_ret = ret_ty;
        self.typed_mode = true;
        self.typed_failed = false;

        // 引数展開: u64 スロットから生値をロード（コールバックなし）。
        // float は同じ 8 バイトを double としてロードするだけ（ビット再解釈）。
        for (i, p) in params.iter().enumerate() {
            let pt = ann_ty(p.type_ann.as_deref()); // 候補選別済みなので Int | Float
            let ptr = self.alloca_var(&p.name, pt);
            let slot = self.fresh_reg();
            self.ec(&format!("{slot} = getelementptr inbounds i64, ptr %_args, i32 {i}"));
            let r = self.fresh_reg();
            self.ec(&format!("{r} = load {}, ptr {slot}", llvm_ty(pt)));
            self.store_val(pt, &r, &ptr.clone());
        }

        self.gen_stmts(body);

        // Fallback terminator（到達不能だが LLVM 上必須）
        if !self.terminated {
            let t = llvm_ty(ret_ty);
            let zero = if ret_ty == Ty::Float { "0.0" } else { "0" };
            self.ec(&format!("store {t} {zero}, ptr %_ret"));
            self.ec("ret i32 0");
            self.terminated = true;
        }

        self.typed_mode = false;
        self.current_fn_ret = Ty::Handle;
        if self.typed_failed {
            return None;
        }

        let attr = export_attr();
        let vis = if attr.is_empty() { "" } else { attr };
        Some(format!(
            "\ndefine {vis}i32 @{name}_typed(ptr %_args, ptr %_ret, ptr %_err) {{\nentry:\n{}{}}}\n",
            self.alloca_buf, self.code_buf
        ))
    }

    pub(super) fn emit_gen_fn(&mut self, name: &str, params: &[Param], body: &[Stmt]) {
        // Reset per-function state
        self.alloca_buf.clear();
        self.code_buf.clear();
        self.reg = 0; self.blk = 0; self.terminated = false;
        self.locals.clear();
        self.loop_stack.clear();
        self.block_stack.clear();

        let param_sigs: Vec<String> = params.iter().enumerate()
            .map(|(i, _)| format!("i64 %_h{i}"))
            .collect();

        for (i, p) in params.iter().enumerate() {
            let pt = ann_ty(p.type_ann.as_deref());
            let st = store_ty(pt);
            let ptr = self.alloca_var(&p.name, st);
            match pt {
                Ty::Int   => { let r = self.call_cb(CB_TO_INT,   &[format!("i64 %_h{i}")]); self.store_val(Ty::Int,   &r, &ptr.clone()); }
                Ty::Float => { let r = self.call_cb(CB_TO_FLOAT, &[format!("i64 %_h{i}")]); self.store_val(Ty::Float, &r, &ptr.clone()); }
                _         => { self.store_val(Ty::Handle, &format!("%_h{i}"), &ptr.clone()); }
            }
        }

        // Pre-allocate the accumulator list
        let list_al = "%_gen_list".to_string();
        self.ea(&format!("{list_al} = alloca i64, align 8"));
        let empty = self.call_cb(CB_MAKE_LIST, &["ptr null".to_string(), "i32 0".to_string()]);
        self.ec(&format!("store i64 {empty}, ptr {list_al}"));

        // Push a block context so Stmt::Yield can append to it
        let exit_lbl = self.fresh_blk();
        self.block_stack.push(BlockCtx {
            result_al: list_al.clone(),
            exit_label: exit_lbl.clone(),
            list_al: Some(list_al.clone()),
        });

        self.gen_stmts(body);
        self.block_stack.pop();

        // Return the accumulated list
        let list_final = self.fresh_reg();
        self.ec(&format!("{list_final} = load i64, ptr {list_al}"));
        self.ret_handle(&list_final);

        let attr = export_attr();
        let vis = if attr.is_empty() { "" } else { attr };
        let sig = param_sigs.join(", ");
        self.fn_defs.push_str(&format!(
            "\ndefine internal i64 @{name}_impl({sig}) {{\nentry:\n{}{}}}\n",
            self.alloca_buf, self.code_buf
        ));

        // Public wrapper: fname_tl(ptr %args, i32 %n) -> i64
        let n = params.len();
        let mut wrapper = format!("\ndefine {vis}i64 @{name}_tl(ptr %args, i32 %_n) {{\n");
        let mut load_args: Vec<String> = Vec::new();
        for i in 0..n {
            wrapper.push_str(&format!("  %_a{i} = getelementptr inbounds i64, ptr %args, i32 {i}\n"));
            wrapper.push_str(&format!("  %_v{i} = load i64, ptr %_a{i}\n"));
            load_args.push(format!("i64 %_v{i}"));
        }
        let call_args = load_args.join(", ");
        wrapper.push_str(&format!("  %_res = call i64 @{name}_impl({call_args})\n"));
        wrapper.push_str("  ret i64 %_res\n}\n");
        self.fn_defs.push_str(&wrapper);
    }

}
