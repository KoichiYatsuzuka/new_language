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
        // リゾルバが AST に書いた slot 割り当てを収穫する（#11 R2-a′）。
        // codegen は採番せず、この表を権威として `Resolution::Local` 読みを slot 索引で解決する。
        self.name_to_slot = harvest_local_slots(body);
        self.binding_mut = crate::vm::compiler::binding_mutability(params, body);
        let max_slot = self.name_to_slot.values().copied().max().map_or(0, |m| m as usize + 1);
        self.locals_by_slot = vec![None; max_slot];
        self.loop_stack.clear();
        self.iter_scopes.clear();
        self.fresh_regs.clear();
        self.handle_store_log.clear();
        self.ip_vars.clear();
        self.ap_vars.clear();
        self.block_stack.clear();
        self.preread_fields.clear();
        self.fn_param_trampolines.clear();
        self.current_fn_ret = ret_ty;
        // この関数の `_impl` がアリーナへ積むかを数え始める（入口の先読みを含む・9-2）
        let pushes_at_start = self.push_sites;

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

        // 内部 ABI（4-1）: `int` / `float` 注釈の仮引数は生値で受ける（箱を開けるのは `_tl` の境界だけ）。
        // ⚠ 以前は全部ハンドルで受けて入口で `CB_TO_INT` / `CB_TO_FLOAT` を呼び、呼ぶ側も箱詰めしていた。
        let ptys = self.fn_sigs[name].param_tys.clone();
        let param_sigs: Vec<String> = params.iter().enumerate()
            .map(|(i, _)| format!("{} %_h{i}", llvm_ty(ptys[i])))
            .collect();
        for (i, p) in params.iter().enumerate() {
            let pt  = ptys[i];
            let st  = store_ty(pt);
            let ptr = self.alloca_var(&p.name, st);
            self.store_val(pt, &format!("%_h{i}"), &ptr.clone());
        }
        // 番地を受け取る変種（copy_alloc_speed_plan.md 7-19）: `mut self` のメソッドで、前の周回で `self.属性` の番地を覚えたなら、
        // 本体は `@{name}_pre(.., ptr ..)` として出し、その置き場を仮引数の番地で始める（置き場は本体を出すまで決まらないので、
        // ここに印を置いて後で差し替える。仮引数を変数へ書いた後なので、`finish_block_cache` が足す「空にする」の後になる）
        let is_mut_method = self.current_class.is_some() && params.first().is_some_and(|p| p.name == "self" && p.mutable);
        let pre = if is_mut_method { self.pre_lists.get(name).cloned().filter(|l| !l.is_empty()) } else { None };
        if pre.is_some() {
            self.ec(PRE_BLOCKS_MARK);
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
            // Only pre-read `let` receivers that the body never writes (`param_prereadable`).
            if !param_prereadable(p, body) { continue; }
            let fields = match self.class_fields_ord.get(&class_name) {
                Some(f) => f.clone(),
                None => continue,
            };
            // ⚠ `_fast` 変種を出すかは「型付きのフィールドを持つ先読み可能な受け手があるか」で決める（呼ぶ側の
            //   `fast_fns` と同じ条件）。下で読むフィールドを絞っても変えない（変えると呼ぶ側が無い `@X_fast` を呼ぶ）。
            if fields.iter().any(|(_, t)| matches!(t, Ty::Float | Ty::Int)) {
                has_preread = true;
            }
            // 4-4: 本体が `p.f` の読みでしか `p` を使わないなら、参照するフィールドだけを読む。
            let uses = param_field_uses(body, &p.name);
            let wanted: Vec<&(String, Ty)> = fields
                .iter()
                .filter(|(n, t)| matches!(t, Ty::Float | Ty::Int) && !uses.as_ref().is_some_and(|u| !u.contains(n)))
                .collect();
            // 8-9: 受け手が raw レイアウトのクラスなら、番地を 1 回だけ取ってフィールドは生成コードが読む
            // （以前はフィールド 1 つにつきコールバック 1 回。`dot(let self, let o)` は 6 回）。番地が空・未初期化・
            // 幅の合わないフィールドは今までのコールバック（`emit_direct_read` の外れの道・`emit_attr_read`）。
            let handle = format!("%_h{i}");
            let block = if wanted.is_empty() { None } else { self.emit_inst_block_once(&class_name, &handle) };
            for (field_name, field_ty) in wanted {
                let direct = block.as_deref().and_then(|p| {
                    self.emit_direct_read(&class_name, BlockSrc::Block { p, h: &handle }, field_name, *field_ty)
                });
                let r = match direct {
                    Some((r, _)) => r,
                    None => self.emit_attr_read(&handle, field_name, Some(*field_ty)).0,
                };
                let al = format!("%_prf_{}_{}", p.name, field_name);
                let tstr = llvm_ty(*field_ty);
                self.alloca_buf.push_str(&format!("  {al} = alloca {tstr}, align 8\n"));
                self.ec(&format!("store {tstr} {r}, ptr {al}"));
                self.preread_fields.insert(format!("{}.{field_name}", p.name), (al, *field_ty));
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
        if self.push_sites > pushes_at_start {
            self.fn_pushes.insert(name.to_string());
        }

        let attr = export_attr();
        let vis  = if attr.is_empty() { "" } else { attr };
        let sig  = param_sigs.join(", ");

        // Float-returning functions use double ABI in _impl; others use i64 handle ABI.
        let impl_ret     = if ret_ty == Ty::Float { "double" } else { "i64" };
        let impl_ret_str = impl_ret;
        // 本体が `self.属性` の番地を覚えた置き場: (属性, 子のクラス, 置き場)。次の周回の並び（`pre_found`）と、この周回の
        // 置き場の始め方に使う（`finish_block_cache` が `ap_vars` を空にする前に取る）
        let mut self_slots: Vec<(String, String, String)> = Vec::new();
        if is_mut_method {
            self_slots = self
                .ap_vars
                .iter()
                .filter(|((al, _), _)| al == "%_al_self")
                .map(|((_, a), (slot, c))| (a.clone(), c.clone(), slot.clone()))
                .collect();
            self_slots.sort();
            if !self_slots.is_empty() {
                self.pre_found.insert(name.to_string(), self_slots.iter().map(|(a, c, _)| (a.clone(), c.clone())).collect());
            }
        }
        self.finish_block_cache();
        match &pre {
            Some(list) => {
                // 並びの k 番目の番地で、同じ（属性, クラス）の置き場を始める。並びに無い置き場は空のまま
                let mut init = String::new();
                for (k, (a, c)) in list.iter().enumerate() {
                    if let Some((_, _, slot)) = self_slots.iter().find(|(sa, sc, _)| sa == a && sc == c) {
                        init.push_str(&format!("  store ptr %_pb{k}, ptr {slot}\n"));
                    }
                }
                self.code_buf = self.code_buf.replacen(&format!("  {PRE_BLOCKS_MARK}\n"), &init, 1);
                let extra = (0..list.len()).map(|k| format!("ptr %_pb{k}"));
                let full = param_sigs.iter().cloned().chain(extra).collect::<Vec<_>>().join(", ");
                self.fn_defs.push_str(&format!(
                    "\ndefine internal {impl_ret} @{name}_pre({full}) {{\nentry:\n{}{}}}\n",
                    self.alloca_buf, self.code_buf
                ));
                // `_impl`（`_tl`・番地を渡さない呼び出し）は空の番地で `_pre` を呼ぶ
                let nulls = std::iter::repeat_n("ptr null".to_string(), list.len());
                let args = param_sigs.iter().cloned().chain(nulls).collect::<Vec<_>>().join(", ");
                self.fn_defs.push_str(&format!(
                    "\ndefine internal {impl_ret} @{name}_impl({sig}) {{\nentry:\n  %_r = call {impl_ret} @{name}_pre({args})\n  ret {impl_ret} %_r\n}}\n"
                ));
            }
            None => self.fn_defs.push_str(&format!(
                "\ndefine internal {impl_ret} @{name}_impl({sig}) {{\nentry:\n{}{}}}\n",
                self.alloca_buf, self.code_buf
            )),
        }

        // Public wrapper: {name}_tl(ptr %args, i32 %n) -> i64 (handle for interpreter)
        // インタープリタとの境界（4-1）: `int` / `float` の仮引数はここで箱を開け、戻り値はここで箱に入れる。
        let n = params.len();
        let mut wrapper = format!("\ndefine {vis}i64 @{name}_tl(ptr %args, i32 %_n) {{\n");
        wrapper.push_str("  %_cb_tl = load ptr, ptr @CB\n");
        // 境界のコールバック 1 つ分（`%_cb_tl` から関数ポインタを引いて呼ぶ）
        let cb_call = |w: &mut String, tag: &str, cb: usize, ret: &str, arg_ty: &str, arg: &str| {
            w.push_str(&format!("  %_{tag}_p = getelementptr inbounds %ArCallbacks, ptr %_cb_tl, i32 0, i32 {cb}\n"));
            w.push_str(&format!("  %_{tag}_f = load ptr, ptr %_{tag}_p\n"));
            w.push_str(&format!("  %_{tag} = call {ret} ({arg_ty}) %_{tag}_f({arg_ty} {arg})\n"));
        };
        let mut load_args: Vec<String> = Vec::new();
        for i in 0..n {
            wrapper.push_str(&format!("  %_a{i} = getelementptr inbounds i64, ptr %args, i32 {i}\n"));
            wrapper.push_str(&format!("  %_v{i} = load i64, ptr %_a{i}\n"));
            match ptys[i] {
                Ty::Int => {
                    cb_call(&mut wrapper, &format!("u{i}"), CB_TO_INT, "i64", "i64", &format!("%_v{i}"));
                    load_args.push(format!("i64 %_u{i}"));
                }
                Ty::Float => {
                    cb_call(&mut wrapper, &format!("u{i}"), CB_TO_FLOAT, "double", "i64", &format!("%_v{i}"));
                    load_args.push(format!("double %_u{i}"));
                }
                _ => load_args.push(format!("i64 %_v{i}")),
            }
        }
        let call_args_str = load_args.join(", ");
        wrapper.push_str(&format!("  %_raw = call {impl_ret_str} @{name}_impl({call_args_str})\n"));
        match ret_ty {
            Ty::Float => cb_call(&mut wrapper, "res", CB_MAKE_FLOAT, "i64", "double", "%_raw"),
            Ty::Int => cb_call(&mut wrapper, "res", CB_MAKE_INT, "i64", "i64", "%_raw"),
            _ => wrapper.push_str("  %_res = add i64 %_raw, 0\n"),
        }
        wrapper.push_str("  ret i64 %_res\n}\n");
        self.fn_defs.push_str(&wrapper);

        // Emit _fast variant if there were pre-reads (class params with typed fields).
        //
        if has_preread {
            self.emit_fn_fast(name, params, ret_ty, body);
        }

        // 番地で呼ぶ変種（7-14）: 作れなければ `blk_evicted` に入れ、`generate_llvm_module` が呼ぶ側ごと作り直す
        if self.blk_ok.contains(name) {
            if let Some(sig) = self.blk_sigs.get(name).cloned() {
                if !self.emit_fn_blk(name, params, ret_ty, body, &sig) {
                    self.blk_evicted.insert(name.to_string());
                }
            }
        }

        self.current_fn_ret = Ty::Handle;
        self.preread_fields.clear();
    }

    /// `@{name}_blk(...)` — 番地で呼ぶ変種（copy_alloc_speed_plan.md 7-14）。仮引数は `sig` のとおり: `mut self` は先頭の
    /// 塊の番地（`ptr %_bp_self`）、`let self` / `let` のクラスの仮引数は数値のフィールドの生値（宣言の順・`_fast` と同じ）、
    /// `int` / `float` は生値。戻り値は `_impl` と同じ内部 ABI。
    ///
    /// 呼ぶ側は、受け手のクラス（ガード付きなら `class_id`）・全フィールドの初期化の印・不変の印を確かめてから呼ぶので、
    /// `self.f` の読み書きは番地への検査なしの `load` / `store` にする（[`Self::emit_blk_self_read`] /
    /// [`Self::emit_blk_self_write`]）。本体がコールバック（`blk_failed`）・LLVM の組み込みのほかの関数を呼ぶか、平たくした
    /// 仮引数・`mut self` を値として使う（`fast_failed`）なら作らずに `false` を返す。
    pub(super) fn emit_fn_blk(&mut self, name: &str, params: &[Param], ret_ty: Ty, body: &[Stmt], sig: &BlkSig) -> bool {
        self.alloca_buf.clear();
        self.code_buf.clear();
        self.reg = 0;
        self.blk = 0;
        self.terminated = false;
        self.locals.clear();
        self.name_to_slot = harvest_local_slots(body);
        self.binding_mut = crate::vm::compiler::binding_mutability(params, body);
        let max_slot = self.name_to_slot.values().copied().max().map_or(0, |m| m as usize + 1);
        self.locals_by_slot = vec![None; max_slot];
        self.loop_stack.clear();
        self.iter_scopes.clear();
        self.fresh_regs.clear();
        self.handle_store_log.clear();
        self.ip_vars.clear();
        self.ap_vars.clear();
        self.block_stack.clear();
        self.preread_fields.clear();
        self.current_fn_ret = ret_ty;
        self.param_classes.clear();
        for p in params {
            if let Some(ann) = &p.type_ann {
                if self.class_fields.contains_key(ann.as_str()) {
                    self.param_classes.insert(p.name.clone(), ann.clone());
                }
            }
        }
        self.blk_mode = true;
        self.blk_failed = false;
        self.blk_self = None;
        // 平たくした仮引数と番地で受けた `self` は値として表せない（使ったら `fast_failed`・`_fast` と同じ仕組み）
        self.fast_mode = true;
        self.fast_failed = false;
        self.fast_flattened.clear();

        let mut sig_parts: Vec<String> = Vec::new();
        for (i, (p, op)) in params.iter().zip(&sig.params).enumerate() {
            match op {
                BlkOp::Ptr(_) => {
                    sig_parts.push("ptr %_bp_self".to_string());
                    self.blk_self = Some("%_bp_self".to_string());
                    self.fast_flattened.insert(p.name.clone());
                }
                BlkOp::Flat(cls) => {
                    for (field, ty) in self.class_fields_ord.get(cls).cloned().unwrap_or_default() {
                        let reg = format!("%_fp_{}__{field}", p.name);
                        let al = format!("%_fpral_{}__{field}", p.name);
                        let t = llvm_ty(ty);
                        sig_parts.push(format!("{t} {reg}"));
                        self.alloca_buf.push_str(&format!("  {al} = alloca {t}, align 8\n"));
                        self.ec(&format!("store {t} {reg}, ptr {al}"));
                        self.preread_fields.insert(format!("{}.{field}", p.name), (al, ty));
                    }
                    self.fast_flattened.insert(p.name.clone());
                }
                BlkOp::Num(ty) => {
                    let reg = format!("%_fph{i}");
                    sig_parts.push(format!("{} {reg}", llvm_ty(*ty)));
                    let al = self.alloca_var(&p.name, store_ty(*ty));
                    self.store_val(*ty, &reg, &al);
                }
            }
        }

        self.gen_stmts(body);
        if ret_ty == Ty::Float {
            if !self.terminated {
                self.ec("ret double 0.0");
                self.terminated = true;
            }
        } else {
            self.ret_handle("0");
        }

        let ok = !self.blk_failed && !self.fast_failed && calls_only_intrinsics(&self.code_buf);
        if ok {
            let impl_ret = if ret_ty == Ty::Float { "double" } else { "i64" };
            self.fn_defs.push_str(&format!(
                "\ndefine internal {impl_ret} @{name}_blk({}) {{\nentry:\n{}{}}}\n",
                sig_parts.join(", "),
                self.alloca_buf,
                self.code_buf
            ));
        }
        self.blk_mode = false;
        self.blk_failed = false;
        self.blk_self = None;
        self.fast_mode = false;
        self.fast_failed = false;
        self.fast_flattened.clear();
        self.ip_vars.clear();
        self.ap_vars.clear();
        self.preread_fields.clear();
        ok
    }

    /// Emit `@{name}_fast(...)` — like _impl but receives class param fields as
    /// raw scalars instead of arena handles.  No callbacks inside the body; the
    /// function is pure arithmetic and LLVM can inline and hoist it freely.
    pub(super) fn emit_fn_fast(&mut self, name: &str, params: &[Param], ret_ty: Ty, body: &[Stmt]) {
        self.fast_mode = true;
        self.fast_failed = false;
        self.fast_flattened.clear();
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
                if param_prereadable(p, body) {
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
                        // このパラメータは平坦化された（値そのものは表現できない）。
                        self.fast_flattened.insert(p.name.clone());
                        continue; // skip handle param
                    }
                }
            }
            // Non-class or written param: the internal ABI of `_impl`（int / float は生値・4-1）
            let reg = format!("%_fph{handle_idx}");
            handle_idx += 1;
            fast_sig.push(format!("{} {reg}", llvm_ty(ann_ty(p.type_ann.as_deref()))));
        }

        if fast_sig.is_empty() { return; }

        // Reset per-function codegen state for the fast variant
        self.alloca_buf.clear();
        self.code_buf.clear();
        self.reg = 0;
        self.blk = 0;
        self.terminated = false;
        self.locals.clear();
        // リゾルバが AST に書いた slot 割り当てを収穫する（#11 R2-a′）。
        // codegen は採番せず、この表を権威として `Resolution::Local` 読みを slot 索引で解決する。
        self.name_to_slot = harvest_local_slots(body);
        self.binding_mut = crate::vm::compiler::binding_mutability(params, body);
        let max_slot = self.name_to_slot.values().copied().max().map_or(0, |m| m as usize + 1);
        self.locals_by_slot = vec![None; max_slot];
        self.loop_stack.clear();
        self.iter_scopes.clear();
        self.fresh_regs.clear();
        self.handle_store_log.clear();
        self.ip_vars.clear();
        self.ap_vars.clear();
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
                if param_prereadable(p, body) {
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
            self.store_val(pt, &reg, &al_ptr.clone());
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
        // 平坦化パラメータの値そのものが要求されていたら、この変種は不正なので捨てる。
        // 呼び出し側は `fast_fns` を見て `_fast` を選ぶため、破棄集合にも記録する。
        if self.fast_failed {
            self.discarded_fast.insert(name.to_string());
        } else {
            self.finish_block_cache();
            self.fn_defs.push_str(&format!(
                "\ndefine internal {impl_ret} @{name}_fast({sig}) {{\nentry:\n{}{}}}\n",
                self.alloca_buf, self.code_buf
            ));
        }
        self.fast_mode = false;
        self.fast_flattened.clear();

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
        // リゾルバが AST に書いた slot 割り当てを収穫する（#11 R2-a′）。
        // codegen は採番せず、この表を権威として `Resolution::Local` 読みを slot 索引で解決する。
        self.name_to_slot = harvest_local_slots(body);
        self.binding_mut = crate::vm::compiler::binding_mutability(params, body);
        let max_slot = self.name_to_slot.values().copied().max().map_or(0, |m| m as usize + 1);
        self.locals_by_slot = vec![None; max_slot];
        self.loop_stack.clear();
        self.iter_scopes.clear();
        self.fresh_regs.clear();
        self.handle_store_log.clear();
        self.ip_vars.clear();
        self.ap_vars.clear();
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
        self.finish_block_cache();
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
        // リゾルバが AST に書いた slot 割り当てを収穫する（#11 R2-a′）。
        // codegen は採番せず、この表を権威として `Resolution::Local` 読みを slot 索引で解決する。
        self.name_to_slot = harvest_local_slots(body);
        self.binding_mut = crate::vm::compiler::binding_mutability(params, body);
        let max_slot = self.name_to_slot.values().copied().max().map_or(0, |m| m as usize + 1);
        self.locals_by_slot = vec![None; max_slot];
        self.loop_stack.clear();
        self.iter_scopes.clear();
        self.fresh_regs.clear();
        self.handle_store_log.clear();
        self.ip_vars.clear();
        self.ap_vars.clear();
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
            iter_depth: self.iter_scopes.len(),
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
        self.finish_block_cache();
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

/// 本体の文面の `call` が LLVM の組み込み（`@llvm.*`）だけか（番地で呼ぶ変種・7-14）。コールバックは `call_cb` が
/// `blk_failed` を立てるが、直接呼び出し（`@X_impl` など）・関数の値の呼び出しはここで見る。
/// ⚠ 呼ぶ先を読めない `call` は「組み込みでない」に倒す。
fn calls_only_intrinsics(code: &str) -> bool {
    code.lines().all(|line| {
        let body = line.trim_start();
        let at = if body.starts_with("call ") { Some(0) } else { body.find(" call ").map(|i| i + 1) };
        let Some(at) = at else { return true };
        let rest = &body[at + "call ".len()..];
        rest.find(['%', '@']).is_some_and(|s| rest[s..].starts_with("@llvm."))
    })
}

/// [`GenCtx::emit_fn`] が本体の頭に置く印（copy_alloc_speed_plan.md 7-19）。本体を出した後で、`self.属性` の番地の置き場を
/// 仮引数の番地で始める `store` に差し替える（LLVM の注釈の行なので、差し替え忘れても意味は変わらない）。
const PRE_BLOCKS_MARK: &str = "; @pre-blocks";
