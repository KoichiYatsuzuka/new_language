// llvm_codegen/expr.rs — 式の LLVM IR 生成: gen_expr とその補助(二項演算・特殊化・高速呼び出し引数・呼び出し生成)。

use crate::ast::{BinOp, CallArg, Expr, MatchPattern, UnaryOp};
use super::*;

impl<'a> GenCtx<'a> {
    /// 式の IR を生成し `(値, 型区分)` を返す。
    ///
    /// #16 段階 c-2: 生成結果が `Ty::Handle`（＝型が判らずボックス化ハンドルのまま）に
    /// 落ちた式のうち、AST 型解決層の注釈が具象型を持っているものを集計する。
    /// これが「注釈へ移行すると新たに型特化できる箇所」の実測値になる（`AR_ANNOT_DIFF=1`）。
    pub fn gen_expr(&mut self, expr: &Expr) -> (String, Ty) {
        let out = self.gen_expr_inner(expr);
        if out.1 == Ty::Handle {
            if let Some(node_id) = annotatable_node_id(expr) {
                if let Some(t) = self.field_ty_annotated(node_id) {
                    let slot = match expr {
                        Expr::Attr { .. }      => &mut self.expr_stats.attr,
                        Expr::Subscript { .. } => &mut self.expr_stats.subscript,
                        Expr::Call { .. }      => &mut self.expr_stats.call,
                        _                      => &mut self.expr_stats.other,
                    };
                    match t {
                        Ty::Int => slot.int += 1,
                        _       => slot.float += 1,
                    }
                }
            }
        }
        out
    }

    fn gen_expr_inner(&mut self, expr: &Expr) -> (String, Ty) {
        match expr {
            Expr::Int(n)    => (format!("{n}"), Ty::Int),
            Expr::Float(f)  => (fmt_float(*f), Ty::Float),
            Expr::Bool(b)   => (if *b { "1" } else { "2" }.to_string(), Ty::Handle), // TL_TRUE / TL_FALSE
            Expr::None      => ("0".to_string(), Ty::Handle),
            Expr::Undefined => ("0".to_string(), Ty::Handle),
            Expr::Str(s) => {
                let bytes = s.as_bytes();
                let ptr   = self.str_const(bytes);
                let len   = bytes.len() as i32;
                let r = self.call_cb(CB_MAKE_STR, &[ptr, format!("i32 {len}")]);
                (r, Ty::Handle)
            }

            e if ident_name(e).is_some() => {
                let name = ident_name(e).unwrap();
                // 解決済みローカル参照は slot 索引で引く（#11 R2-a′）。
                // **slot 表に載っているときだけ**この経路に入る。未登録なら以降の
                // 従来分岐（名前引き → module_fn → グローバル）へそのまま落とす。
                if let Expr::LocalRef { slot, .. } = e {
                    if matches!(self.locals_by_slot.get(*slot as usize), Some(Some(_))) {
                        let slot = *slot;
                        return self.load_var_by_slot(slot, "");
                    }
                }
                if self.locals.contains_key(name) {
                    self.load_var(name)
                } else if self.module_fns.contains(name) {
                    // Intra-module fn reference: fetch as global
                    let bytes  = name.as_bytes();
                    let ptr    = self.str_const(bytes);
                    let len    = bytes.len() as i32;
                    let r      = self.call_cb(CB_GET_GLOBAL, &[ptr, format!("i32 {len}")]);
                    (r, Ty::Handle)
                } else {
                    // `_fast` 生成中に、平坦化したパラメータの値そのものを要求された。
                    // このまま出すと「存在しないグローバル」を引く不正コードになるので
                    // 変種を破棄する（`typed_failed` と同じ扱い）。
                    if self.fast_mode && self.fast_flattened.contains(name) {
                        self.fast_failed = true;
                    }
                    let bytes = name.as_bytes();
                    let ptr   = self.str_const(bytes);
                    let len   = bytes.len() as i32;
                    let r     = self.call_cb(CB_GET_GLOBAL, &[ptr, format!("i32 {len}")]);
                    (r, Ty::Handle)
                }
            }

            Expr::BinOp { op, left, right, .. } => self.gen_binop(op, left, right),

            Expr::UnaryOp { op, operand } => {
                let op_code = match op { UnaryOp::Neg => 0i32, UnaryOp::Not => 1, UnaryOp::BitNot => 2 };
                let (v, vt) = self.gen_expr(operand);
                let h = self.to_handle(&v, vt);
                let r = self.call_cb(CB_UNOP, &[format!("i32 {op_code}"), format!("i64 {h}")]);
                (r, Ty::Handle)
            }

            Expr::Call { func, args, .. } => {
                // Cell built-ins: __make_cell / __get_cell / __set_cell
                if let Some(n) = ident_name(func.as_ref()) {
                    if n == "__make_cell" || n == "__get_cell" || n == "__set_cell" {
                        let arg_vals: Vec<(String, Ty)> = args.iter()
                            .map(|a| self.gen_expr(a.expr()))
                            .collect();
                        let handles: Vec<String> = arg_vals.iter()
                            .map(|(v, vt)| { let h = self.to_handle(v, *vt); format!("i64 {h}") })
                            .collect();
                        return match n {
                            "__make_cell" => {
                                let init = handles.first().cloned().unwrap_or("i64 0".to_string());
                                let r = self.call_cb(CB_MAKE_CELL, &[init]);
                                (r, Ty::Handle)
                            }
                            "__get_cell" => {
                                let cell = handles.first().cloned().unwrap_or("i64 0".to_string());
                                let r = self.call_cb(CB_GET_CELL, &[cell]);
                                (r, Ty::Handle)
                            }
                            _ => { // __set_cell
                                if handles.len() >= 2 {
                                    self.call_cb(CB_SET_CELL, &[handles[0].clone(), handles[1].clone()]);
                                }
                                let r = handles.first().cloned().unwrap_or("0".to_string());
                                (r.trim_start_matches("i64 ").to_string(), Ty::Handle)
                            }
                        };
                    }
                }
                // ── typed モード: _typed 同士の直接呼び出し（status 伝播） ─────
                if self.typed_mode {
                    if let Some(name) = ident_name(func.as_ref()) {
                        if self.typed_ok.contains(name)
                            && !self.locals.contains_key(name)
                        {
                            if let Some((ptys, rty)) = self.typed_sigs.get(name).cloned() {
                                if args.len() == ptys.len() {
                                    let name = name.to_string();
                                    return self.gen_typed_call(&name, args, &ptys, rty);
                                }
                            }
                        }
                    }
                    // typed 変種のない呼び出し先 → ハンドル経路が必要 → typed 破棄
                    self.typed_failed = true;
                    return ("0".to_string(), Ty::Handle);
                }
                // ── Typed intra-module direct function calls ──────────────────
                if let Some(name) = ident_name(func.as_ref()) {
                    if self.module_fns.contains(name)
                        && !self.locals.contains_key(name)
                    {
                        let ret_ty = self.fn_sigs.get(name)
                            .map(|s| s.ret).unwrap_or(Ty::Handle);

                        if ret_ty == Ty::Float {
                            // Fast path: _impl returns double — no arena save/compact/boxing.
                            let mutabilities = self.fn_sigs.get(name)
                                .map(|s| s.param_mutabilities.clone());
                            let arg_exprs: Vec<(String, Ty)> = args.iter()
                                .map(|a| self.gen_expr(a.expr())).collect();
                            let call_args: Vec<String> = arg_exprs.iter().enumerate()
                                .map(|(i, (v, vt))| {
                                    let h = self.to_handle(v, *vt);
                                    let is_mut = mutabilities.as_ref()
                                        .and_then(|m| m.get(i)).copied().unwrap_or(true);
                                    if is_mut { format!("i64 {h}") }
                                    else {
                                        let dc = self.call_cb(CB_DEEP_COPY, &[format!("i64 {h}")]);
                                        format!("i64 {dc}")
                                    }
                                })
                                .collect();
                            let param_str = call_args.join(", ");
                            let r = self.fresh_reg();
                            self.ec(&format!("{r} = call double @{name}_impl({param_str})"));
                            return (r, Ty::Float);
                        }

                        // Non-float typed return: use handle ABI + CB_TO_INT unwrap.
                        let h = self.gen_call(func, args);
                        return match ret_ty {
                            Ty::Int => {
                                let r = self.call_cb(CB_TO_INT, &[format!("i64 {h}")]);
                                (r, Ty::Int)
                            }
                            _ => (h, Ty::Handle),
                        };
                    }
                }
                // ── Typed intra-module method calls on known class instances ──
                if let Expr::Attr { object, attr, .. } = func.as_ref() {
                    let class_name = match object.as_ref() {
                        e if ident_name(e) == Some("self") => self.current_class.clone(),
                        e if ident_name(e).is_some() => {
                            self.param_classes.get(ident_name(e).unwrap()).cloned()
                        }
                        _ => None,
                    };
                    if let Some(cls) = &class_name {
                        let sym = method_symbol(cls, attr);
                        if self.module_fns.contains(&sym) && !self.locals.contains_key(&sym) {
                            let ret_ty = self.fn_sigs.get(&sym).map(|s| s.ret).unwrap_or(Ty::Handle);

                            if ret_ty == Ty::Float {
                                // Preferred: _fast variant — passes pre-read field values as
                                // scalars; zero callbacks and LLVM-inlineable pure arithmetic.
                                // 破棄された `_fast` 変種は選ばない（生成側と条件を揃える）。
                                if self.fast_fns.contains(&sym)
                                    && !self.discarded_fast.contains(&sym)
                                {
                                    if let Some(fast_args) = self.build_fast_call_args(object, args) {
                                        let r = self.fresh_reg();
                                        self.ec(&format!("{r} = call double @{sym}_fast({fast_args})"));
                                        return (r, Ty::Float);
                                    }
                                }
                                // Fallback: _impl returns double, no arena overhead.
                                let arg_exprs: Vec<(String, Ty)> = args.iter()
                                    .map(|a| self.gen_expr(a.expr())).collect();
                                let (ov, ot) = self.gen_expr(object);
                                let oh = self.to_handle(&ov, ot);
                                let explicit: Vec<String> = arg_exprs.iter()
                                    .map(|(v, vt)| format!("i64 {}", self.to_handle(v, *vt)))
                                    .collect();
                                let all_params = std::iter::once(format!("i64 {oh}"))
                                    .chain(explicit).collect::<Vec<_>>().join(", ");
                                let r = self.fresh_reg();
                                self.ec(&format!("{r} = call double @{sym}_impl({all_params})"));
                                return (r, Ty::Float);
                            }

                            // Non-float typed return: handle ABI + CB_TO_INT.
                            let h = self.gen_call(func, args);
                            return match ret_ty {
                                Ty::Int => { let r = self.call_cb(CB_TO_INT, &[format!("i64 {h}")]); (r, Ty::Int) }
                                _       => (h, Ty::Handle),
                            };
                        }
                    }
                }
                (self.gen_call(func, args), Ty::Handle)
            }

            Expr::Attr { object, attr, node_id, .. } => {
                // Fast path: check preread_fields using the full dotted path.
                // Covers class params ("self.x", "p.x"), flat list loop vars ("item.v"),
                // and nested flat fields ("item.start.x").
                if let Some(path) = preread_path(expr) {
                    if let Some((al_ptr, ty)) = self.preread_fields.get(&path).cloned() {
                        let r = self.fresh_reg();
                        self.ec(&format!("{r} = load {}, ptr {al_ptr}", llvm_ty(ty)));
                        return (r, ty);
                    }
                }
                // Also check single-level preread for "self.attr" and param class attrs.
                let preread_key = match object.as_ref() {
                    e if ident_name(e) == Some("self") && self.current_class.is_some() => {
                        Some(format!("self.{attr}"))
                    }
                    e if ident_name(e).is_some_and(|n| self.param_classes.contains_key(n)) => {
                        Some(format!("{}.{attr}", ident_name(e).unwrap()))
                    }
                    _ => None,
                };
                if let Some(key) = preread_key {
                    if let Some((al_ptr, ty)) = self.preread_fields.get(&key).cloned() {
                        let r = self.fresh_reg();
                        self.ec(&format!("{r} = load {}, ptr {al_ptr}", llvm_ty(ty)));
                        return (r, ty);
                    }
                }

                // Callback path: typed single-callback read (CB_GET_FLOAT_FIELD /
                // CB_GET_INT_FIELD) for known typed fields, plain CB_GET_ATTR otherwise.
                let known_ty = self.field_ty_resolved(object, attr, *node_id);
                let (obj, ot) = self.gen_expr(object);
                let h   = self.to_handle(&obj, ot);
                let ptr = self.str_const(attr.as_bytes());
                let len = attr.len() as i32;
                match known_ty {
                    Some(Ty::Float) => {
                        let r = self.call_cb(CB_GET_FLOAT_FIELD, &[format!("i64 {h}"), ptr, format!("i32 {len}")]);
                        (r, Ty::Float)
                    }
                    Some(Ty::Int) => {
                        let r = self.call_cb(CB_GET_INT_FIELD, &[format!("i64 {h}"), ptr, format!("i32 {len}")]);
                        (r, Ty::Int)
                    }
                    _ => {
                        let raw = self.call_cb(CB_GET_ATTR, &[format!("i64 {h}"), ptr, format!("i32 {len}")]);
                        (raw, Ty::Handle)
                    }
                }
            }

            Expr::TraitAccess { object, trait_name, attr } => {
                let key  = format!("{trait_name}::{attr}");
                let (obj, ot) = self.gen_expr(object);
                let h    = self.to_handle(&obj, ot);
                let ptr  = self.str_const(key.as_bytes());
                let len  = key.len() as i32;
                let r    = self.call_cb(CB_GET_ATTR, &[format!("i64 {h}"), ptr, format!("i32 {len}")]);
                (r, Ty::Handle)
            }

            Expr::Subscript { object, index, .. } => {
                let (obj, ot) = self.gen_expr(object);
                let (idx, it) = self.gen_expr(index);
                let h1 = self.to_handle(&obj, ot);
                let h2 = self.to_handle(&idx, it);
                let r  = self.call_cb(CB_SUBSCRIPT, &[format!("i64 {h1}"), format!("i64 {h2}")]);
                (r, Ty::Handle)
            }

            Expr::IsType { expr, negated, type_name, .. } => {
                let (v, vt) = self.gen_expr(expr);
                let h   = self.to_handle(&v, vt);
                let ptr = self.str_const(type_name.as_bytes());
                let len = type_name.len() as i32;
                let r   = self.call_cb(CB_IS_TYPE, &[format!("i64 {h}"), ptr, format!("i32 {len}")]);
                if *negated {
                    let cmp = self.fresh_reg();
                    let nr  = self.fresh_reg();
                    self.ec(&format!("{cmp} = icmp eq i64 {r}, 1")); // 1 = TL_TRUE
                    self.ec(&format!("{nr}  = select i1 {cmp}, i64 2, i64 1")); // swap TRUE↔FALSE
                    (nr, Ty::Handle)
                } else {
                    (r, Ty::Handle)
                }
            }

            Expr::List(items) => {
                if items.is_empty() {
                    let r = self.call_cb(CB_MAKE_LIST, &["ptr null".to_string(), "i32 0".to_string()]);
                    return (r, Ty::Handle);
                }
                let n    = items.len();
                let arr  = format!("%_la{}", self.reg); self.reg += 1;
                self.ea(&format!("{arr} = alloca [{n} x i64], align 8"));
                for (i, item) in items.iter().enumerate() {
                    let (v, vt) = self.gen_expr(item);
                    let h  = self.to_handle(&v, vt);
                    let ep = self.fresh_reg();
                    self.ec(&format!("{ep} = getelementptr inbounds [{n} x i64], ptr {arr}, i32 0, i32 {i}"));
                    self.ec(&format!("store i64 {h}, ptr {ep}"));
                }
                let r = self.call_cb(CB_MAKE_LIST, &[format!("ptr {arr}"), format!("i32 {n}")]);
                (r, Ty::Handle)
            }

            Expr::Tuple(items) => {
                if items.is_empty() {
                    let r = self.call_cb(CB_MAKE_TUPLE, &["ptr null".to_string(), "i32 0".to_string()]);
                    return (r, Ty::Handle);
                }
                let n   = items.len();
                let arr = format!("%_ta{}", self.reg); self.reg += 1;
                self.ea(&format!("{arr} = alloca [{n} x i64], align 8"));
                for (i, item) in items.iter().enumerate() {
                    let (v, vt) = self.gen_expr(item);
                    let h  = self.to_handle(&v, vt);
                    let ep = self.fresh_reg();
                    self.ec(&format!("{ep} = getelementptr inbounds [{n} x i64], ptr {arr}, i32 0, i32 {i}"));
                    self.ec(&format!("store i64 {h}, ptr {ep}"));
                }
                let r = self.call_cb(CB_MAKE_TUPLE, &[format!("ptr {arr}"), format!("i32 {n}")]);
                (r, Ty::Handle)
            }

            Expr::Dict(pairs) => {
                if pairs.is_empty() {
                    let r = self.call_cb(CB_MAKE_DICT, &["ptr null".to_string(), "ptr null".to_string(), "i32 0".to_string()]);
                    return (r, Ty::Handle);
                }
                let n    = pairs.len();
                let karr = format!("%_dka{}", self.reg); self.reg += 1;
                let varr = format!("%_dva{}", self.reg); self.reg += 1;
                self.ea(&format!("{karr} = alloca [{n} x i64], align 8"));
                self.ea(&format!("{varr} = alloca [{n} x i64], align 8"));
                for (i, (k, v)) in pairs.iter().enumerate() {
                    let (kv, kt) = self.gen_expr(k);
                    let (vv, vt) = self.gen_expr(v);
                    let kh = self.to_handle(&kv, kt);
                    let vh = self.to_handle(&vv, vt);
                    let kp = self.fresh_reg();
                    let vp = self.fresh_reg();
                    self.ec(&format!("{kp} = getelementptr inbounds [{n} x i64], ptr {karr}, i32 0, i32 {i}"));
                    self.ec(&format!("{vp} = getelementptr inbounds [{n} x i64], ptr {varr}, i32 0, i32 {i}"));
                    self.ec(&format!("store i64 {kh}, ptr {kp}"));
                    self.ec(&format!("store i64 {vh}, ptr {vp}"));
                }
                let r = self.call_cb(CB_MAKE_DICT, &[format!("ptr {karr}"), format!("ptr {varr}"), format!("i32 {n}")]);
                (r, Ty::Handle)
            }

            Expr::TemplateInstantiate { base, .. } => self.gen_expr(base),

            // ── Control-flow expressions ──────────────────────────────────────

            Expr::Block { stmts, .. } => {
                let result_al = format!("%_blk_res{}", self.blk);
                self.blk += 1;
                self.alloca_buf.push_str(&format!("  {result_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 0, ptr {result_al}"));
                let exit_lbl = self.fresh_blk();
                self.block_stack.push(BlockCtx { result_al: result_al.clone(), exit_label: exit_lbl.clone(), list_al: None });
                self.gen_stmts(stmts);
                self.block_stack.pop();
                self.br(&exit_lbl);
                self.start_block(&exit_lbl);
                let r = self.fresh_reg();
                self.ec(&format!("{r} = load i64, ptr {result_al}"));
                (r, Ty::Handle)
            }

            Expr::IfExpr { branches, else_body, .. } => {
                let result_al = format!("%_if_res{}", self.blk);
                self.blk += 1;
                self.alloca_buf.push_str(&format!("  {result_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 0, ptr {result_al}"));
                let merge = self.fresh_blk();
                self.block_stack.push(BlockCtx { result_al: result_al.clone(), exit_label: merge.clone(), list_al: None });
                for (cond, body) in branches {
                    let then_blk = self.fresh_blk();
                    let next_blk = self.fresh_blk();
                    let (cv, ct) = self.gen_expr(cond);
                    let cc = self.to_cond(&cv, ct);
                    self.br_cond(&cc, &then_blk, &next_blk);
                    self.start_block(&then_blk);
                    self.gen_stmts(body);
                    self.br(&merge);
                    self.start_block(&next_blk);
                }
                if let Some(else_stmts) = else_body {
                    self.gen_stmts(else_stmts);
                }
                self.br(&merge);
                self.block_stack.pop();
                self.start_block(&merge);
                let r = self.fresh_reg();
                self.ec(&format!("{r} = load i64, ptr {result_al}"));
                (r, Ty::Handle)
            }

            Expr::ForExpr { target, iter, body, .. } => {
                // Accumulator list (for loop_yield) or result slot (for block_return)
                let result_al = format!("%_for_res{}", self.blk);
                self.blk += 1;
                self.alloca_buf.push_str(&format!("  {result_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 0, ptr {result_al}"));
                let has_yield = body_has_loop_yield(body);
                let list_al = if has_yield {
                    let la = format!("%_for_list{}", self.blk);
                    self.blk += 1;
                    self.alloca_buf.push_str(&format!("  {la} = alloca i64, align 8\n"));
                    let empty = self.call_cb(CB_MAKE_LIST, &["ptr null".to_string(), "i32 0".to_string()]);
                    self.ec(&format!("store i64 {empty}, ptr {la}"));
                    Some(la)
                } else { None };

                let exit_blk = self.fresh_blk();
                let loop_blk = self.fresh_blk();
                let (iv, it) = self.gen_expr(iter);
                let ih = self.to_handle(&iv, it);
                let iter_h = self.call_cb(CB_ITER_FROM, &[format!("i64 {ih}")]);
                let iter_al = format!("%_iter{}", self.reg);
                self.reg += 1;
                self.alloca_buf.push_str(&format!("  {iter_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 {iter_h}, ptr {iter_al}"));
                let tgt_al = format!("%_al_{target}");
                self.alloca_buf.push_str(&format!("  {tgt_al} = alloca i64, align 8\n"));
                self.locals.insert(target.clone(), (tgt_al.clone(), Ty::Handle));
                self.br(&loop_blk);
                self.start_block(&loop_blk);
                let ir = self.fresh_reg();
                self.ec(&format!("{ir} = load i64, ptr {iter_al}"));
                let next = self.call_cb(CB_ITER_NEXT, &[format!("i64 {ir}")]);
                self.ec(&format!("store i64 {next}, ptr {tgt_al}"));
                let done = self.fresh_reg();
                self.ec(&format!("{done} = icmp eq i64 {next}, -1"));
                let body_blk = self.fresh_blk();
                self.br_cond(&done, &exit_blk, &body_blk);
                self.start_block(&body_blk);
                self.block_stack.push(BlockCtx {
                    result_al: result_al.clone(), exit_label: exit_blk.clone(), list_al: list_al.clone()
                });
                self.loop_stack.push((loop_blk.clone(), exit_blk.clone()));
                self.gen_stmts(body);
                self.loop_stack.pop();
                self.block_stack.pop();
                self.br(&loop_blk);
                self.start_block(&exit_blk);
                // Return list if loop_yield was used, else result slot
                let r = self.fresh_reg();
                if let Some(la) = &list_al {
                    self.ec(&format!("{r} = load i64, ptr {la}"));
                } else {
                    self.ec(&format!("{r} = load i64, ptr {result_al}"));
                }
                (r, Ty::Handle)
            }

            Expr::WhileExpr { cond, body, .. } => {
                let result_al = format!("%_whl_res{}", self.blk);
                self.blk += 1;
                self.alloca_buf.push_str(&format!("  {result_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 0, ptr {result_al}"));
                let has_yield = body_has_loop_yield(body);
                let list_al = if has_yield {
                    let la = format!("%_whl_list{}", self.blk);
                    self.blk += 1;
                    self.alloca_buf.push_str(&format!("  {la} = alloca i64, align 8\n"));
                    let empty = self.call_cb(CB_MAKE_LIST, &["ptr null".to_string(), "i32 0".to_string()]);
                    self.ec(&format!("store i64 {empty}, ptr {la}"));
                    Some(la)
                } else { None };
                let cond_blk = self.fresh_blk();
                let body_blk = self.fresh_blk();
                let exit_blk = self.fresh_blk();
                self.br(&cond_blk);
                self.start_block(&cond_blk);
                let (cv, ct) = self.gen_expr(cond);
                let cc = self.to_cond(&cv, ct);
                self.br_cond(&cc, &body_blk, &exit_blk);
                self.start_block(&body_blk);
                self.block_stack.push(BlockCtx {
                    result_al: result_al.clone(), exit_label: exit_blk.clone(), list_al: list_al.clone()
                });
                self.loop_stack.push((cond_blk.clone(), exit_blk.clone()));
                self.gen_stmts(body);
                self.loop_stack.pop();
                self.block_stack.pop();
                self.br(&cond_blk);
                self.start_block(&exit_blk);
                let r = self.fresh_reg();
                if let Some(la) = &list_al {
                    self.ec(&format!("{r} = load i64, ptr {la}"));
                } else {
                    self.ec(&format!("{r} = load i64, ptr {result_al}"));
                }
                (r, Ty::Handle)
            }

            Expr::MatchExpr { subject, arms, .. } => {
                let result_al = format!("%_mtch_res{}", self.blk);
                self.blk += 1;
                self.alloca_buf.push_str(&format!("  {result_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 0, ptr {result_al}"));
                let merge = self.fresh_blk();
                let (sv, st) = self.gen_expr(subject);
                let subj_h = self.to_handle(&sv, st);
                let subj_al = format!("%_msubj{}", self.reg);
                self.reg += 1;
                self.alloca_buf.push_str(&format!("  {subj_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 {subj_h}, ptr {subj_al}"));
                self.block_stack.push(BlockCtx { result_al: result_al.clone(), exit_label: merge.clone(), list_al: None });
                for (i, arm) in arms.iter().enumerate() {
                    let is_last = i == arms.len() - 1;
                    let body_blk = self.fresh_blk();
                    let next_blk = if is_last { merge.clone() } else { self.fresh_blk() };
                    let subj_r = self.fresh_reg();
                    self.ec(&format!("{subj_r} = load i64, ptr {subj_al}"));
                    match &arm.pattern {
                        MatchPattern::Case(e) if ident_name(e) == Some("_") => { self.br(&body_blk); }
                        MatchPattern::Case(pat) => {
                            let (pv, pt) = self.gen_expr(pat);
                            let ph = self.to_handle(&pv, pt);
                            let eq = self.call_cb(CB_BINOP, &["i32 7".to_string(), format!("i64 {subj_r}"), format!("i64 {ph}")]);
                            let cnd = self.to_cond(&eq, Ty::Handle);
                            self.br_cond(&cnd, &body_blk, &next_blk);
                        }
                        MatchPattern::IsType(tn) => {
                            let ptr = self.str_const(tn.as_bytes());
                            let len = tn.len() as i32;
                            let r = self.call_cb(CB_IS_TYPE, &[format!("i64 {subj_r}"), ptr, format!("i32 {len}")]);
                            let cnd = self.fresh_reg();
                            self.ec(&format!("{cnd} = icmp eq i64 {r}, 1"));
                            self.br_cond(&cnd, &body_blk, &next_blk);
                        }
                    }
                    self.start_block(&body_blk);
                    self.gen_stmts(&arm.body);
                    self.br(&merge);
                    if !is_last { self.start_block(&next_blk); }
                }
                self.block_stack.pop();
                self.start_block(&merge);
                let r = self.fresh_reg();
                self.ec(&format!("{r} = load i64, ptr {result_al}"));
                (r, Ty::Handle)
            }

            _ => ("0".to_string(), Ty::Handle), // unsupported expr → None handle
        }
    }

    pub(super) fn gen_binop(&mut self, op: &BinOp, left: &Expr, right: &Expr) -> (String, Ty) {
        // Short-circuit and/or
        match op {
            BinOp::And => {
                let (l, lt) = self.gen_expr(left);
                let lh   = self.to_handle(&l, lt);
                let lc   = self.to_cond(&lh, Ty::Handle);
                let rblk = self.fresh_blk();
                let mblk = self.fresh_blk();
                let res_al = format!("%_and_res{}", self.blk);
                self.blk += 1;
                self.alloca_buf.push_str(&format!("  {res_al} = alloca i64, align 8\n"));
                // store lh into res before branch
                self.ec(&format!("store i64 {lh}, ptr {res_al}"));
                self.br_cond(&lc, &rblk, &mblk);
                self.start_block(&rblk);
                let (r2, r2t) = self.gen_expr(right);
                let rh = self.to_handle(&r2, r2t);
                self.ec(&format!("store i64 {rh}, ptr {res_al}"));
                self.br(&mblk);
                self.start_block(&mblk);
                let result = self.fresh_reg();
                self.ec(&format!("{result} = load i64, ptr {res_al}"));
                return (result, Ty::Handle);
            }
            BinOp::Or => {
                let (l, lt) = self.gen_expr(left);
                let lh   = self.to_handle(&l, lt);
                let lc   = self.to_cond(&lh, Ty::Handle);
                let rblk = self.fresh_blk();
                let mblk = self.fresh_blk();
                let res_al = format!("%_or_res{}", self.blk);
                self.blk += 1;
                self.alloca_buf.push_str(&format!("  {res_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 {lh}, ptr {res_al}"));
                self.br_cond(&lc, &mblk, &rblk); // if truthy, skip right
                self.start_block(&rblk);
                let (r2, r2t) = self.gen_expr(right);
                let rh = self.to_handle(&r2, r2t);
                self.ec(&format!("store i64 {rh}, ptr {res_al}"));
                self.br(&mblk);
                self.start_block(&mblk);
                let result = self.fresh_reg();
                self.ec(&format!("{result} = load i64, ptr {res_al}"));
                return (result, Ty::Handle);
            }
            _ => {}
        }

        let (l, lt) = self.gen_expr(left);
        let (r, rt) = self.gen_expr(right);
        self.specialize_binop(op, &l, lt, &r, rt)
    }

    pub(super) fn specialize_binop(&mut self, op: &BinOp, l: &str, lt: Ty, r: &str, rt: Ty) -> (String, Ty) {
        // If either side is a handle, fall back to cb_binop.
        if lt == Ty::Handle || rt == Ty::Handle {
            let op_code = binop_code(op);
            let lh = self.to_handle(l, lt);
            let rh = self.to_handle(r, rt);
            let res = self.call_cb(CB_BINOP, &[format!("i32 {op_code}"), format!("i64 {lh}"), format!("i64 {rh}")]);
            return (res, Ty::Handle);
        }

        // Promote mixed Int/Float → Float
        let (l_s, r_s, nt): (String, String, Ty) = match (lt, rt) {
            (Ty::Int, Ty::Float) => {
                let lf = self.to_f64(l, lt); (lf, r.to_string(), Ty::Float)
            }
            (Ty::Float, Ty::Int) => {
                let rf = self.to_f64(r, rt); (l.to_string(), rf, Ty::Float)
            }
            _ => (l.to_string(), r.to_string(), lt),
        };
        let (l, r) = (l_s.as_str(), r_s.as_str());

        match (op, nt) {
            (BinOp::Add, Ty::Int)  => { let res = self.fresh_reg(); self.ec(&format!("{res} = add i64 {l}, {r}")); (res, Ty::Int) }
            (BinOp::Sub, Ty::Int)  => { let res = self.fresh_reg(); self.ec(&format!("{res} = sub i64 {l}, {r}")); (res, Ty::Int) }
            (BinOp::Mul, Ty::Int)  => { let res = self.fresh_reg(); self.ec(&format!("{res} = mul i64 {l}, {r}")); (res, Ty::Int) }
            (BinOp::Div, Ty::Int)  => {
                let lf = self.to_f64(l, Ty::Int);
                let rf = self.to_f64(r, Ty::Int);
                let res = self.fresh_reg();
                self.ec(&format!("{res} = fdiv double {lf}, {rf}"));
                (res, Ty::Float)
            }
            (BinOp::FloorDiv, Ty::Int) => {
                let res = self.fresh_reg();
                self.ec(&format!("{res} = call i64 @_tl_idiv(i64 {l}, i64 {r})"));
                (res, Ty::Int)
            }
            (BinOp::Mod, Ty::Int) => {
                let res = self.fresh_reg();
                self.ec(&format!("{res} = call i64 @_tl_imod(i64 {l}, i64 {r})"));
                (res, Ty::Int)
            }
            (BinOp::BitAnd, Ty::Int) => { let res = self.fresh_reg(); self.ec(&format!("{res} = and i64 {l}, {r}")); (res, Ty::Int) }
            (BinOp::BitOr,  Ty::Int) => { let res = self.fresh_reg(); self.ec(&format!("{res} = or i64 {l}, {r}")); (res, Ty::Int) }
            (BinOp::BitXor, Ty::Int) => { let res = self.fresh_reg(); self.ec(&format!("{res} = xor i64 {l}, {r}")); (res, Ty::Int) }
            (BinOp::LShift, Ty::Int) => { let res = self.fresh_reg(); self.ec(&format!("{res} = shl i64 {l}, {r}")); (res, Ty::Int) }
            (BinOp::RShift, Ty::Int) => { let res = self.fresh_reg(); self.ec(&format!("{res} = ashr i64 {l}, {r}")); (res, Ty::Int) }

            (BinOp::Add, Ty::Float) => { let res = self.fresh_reg(); self.ec(&format!("{res} = fadd double {l}, {r}")); (res, Ty::Float) }
            (BinOp::Sub, Ty::Float) => { let res = self.fresh_reg(); self.ec(&format!("{res} = fsub double {l}, {r}")); (res, Ty::Float) }
            (BinOp::Mul, Ty::Float) => { let res = self.fresh_reg(); self.ec(&format!("{res} = fmul double {l}, {r}")); (res, Ty::Float) }
            (BinOp::Div, Ty::Float) => { let res = self.fresh_reg(); self.ec(&format!("{res} = fdiv double {l}, {r}")); (res, Ty::Float) }
            (BinOp::FloorDiv, Ty::Float) => {
                let d   = self.fresh_reg();
                let res = self.fresh_reg();
                self.ec(&format!("{d} = fdiv double {l}, {r}"));
                self.ec(&format!("{res} = call double @llvm.floor.f64(double {d})"));
                (res, Ty::Float)
            }
            (BinOp::Mod, Ty::Float) => {
                // Python float mod: a - floor(a/b)*b
                let d   = self.fresh_reg();
                let fl  = self.fresh_reg();
                let mul = self.fresh_reg();
                let res = self.fresh_reg();
                self.ec(&format!("{d}   = fdiv double {l}, {r}"));
                self.ec(&format!("{fl}  = call double @llvm.floor.f64(double {d})"));
                self.ec(&format!("{mul} = fmul double {fl}, {r}"));
                self.ec(&format!("{res} = fsub double {l}, {mul}"));
                (res, Ty::Float)
            }
            (BinOp::Pow, Ty::Float) => {
                let res = self.fresh_reg();
                self.ec(&format!("{res} = call double @llvm.pow.f64(double {l}, double {r})"));
                (res, Ty::Float)
            }

            // Comparisons (work for both Int and Float)
            (BinOp::Eq,    Ty::Int)   => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = icmp eq  i64 {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::NotEq, Ty::Int)   => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = icmp ne  i64 {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::Lt,    Ty::Int)   => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = icmp slt i64 {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::LtEq,  Ty::Int)   => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = icmp sle i64 {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::Gt,    Ty::Int)   => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = icmp sgt i64 {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::GtEq,  Ty::Int)   => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = icmp sge i64 {l}, {r}")); (r2, Ty::Bool) }

            (BinOp::Eq,    Ty::Float) => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = fcmp oeq double {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::NotEq, Ty::Float) => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = fcmp one double {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::Lt,    Ty::Float) => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = fcmp olt double {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::LtEq,  Ty::Float) => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = fcmp ole double {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::Gt,    Ty::Float) => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = fcmp ogt double {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::GtEq,  Ty::Float) => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = fcmp oge double {l}, {r}")); (r2, Ty::Bool) }

            _ => {
                // Fall back to cb_binop for anything not directly lowered
                let op_code = binop_code(op);
                let lh  = self.to_handle(l, nt);
                let rh  = self.to_handle(r, nt);
                let res = self.call_cb(CB_BINOP, &[format!("i32 {op_code}"), format!("i64 {lh}"), format!("i64 {rh}")]);
                (res, Ty::Handle)
            }
        }
    }

    /// Build the argument string for a `@{sym}_fast(...)` call.
    /// Returns None if any required pre-read value is missing (fallback to _impl).
    ///
    /// The _fast ABI for a method is:
    ///   for each param in declaration order:
    ///     if it's a class instance with pre-read fields → expand all typed fields as scalars
    ///     else → pass as i64 handle
    /// The receiver (`self` / first arg of Expr::Attr) maps to param 0.
    pub(super) fn build_fast_call_args(&mut self, object: &Expr, args: &[CallArg]) -> Option<String> {
        // Determine receiver param name
        let recv_name: &str = match object {
            e if ident_name(e) == Some("self") => "self",
            e if ident_name(e).is_some() => ident_name(e).unwrap(),
            _ => return None,
        };

        let mut parts: Vec<String> = Vec::new();

        // Expand receiver's fields
        let recv_class = if recv_name == "self" {
            self.current_class.as_deref()?.to_string()
        } else {
            self.param_classes.get(recv_name)?.clone()
        };
        let recv_fields = self.class_fields_ord.get(&recv_class)?.clone();
        for (field_name, field_ty) in &recv_fields {
            let key = format!("{recv_name}.{field_name}");
            let (al, ty) = self.preread_fields.get(&key)?.clone();
            let r = self.fresh_reg();
            self.ec(&format!("{r} = load {}, ptr {al}", llvm_ty(ty)));
            parts.push(format!("{} {r}", llvm_ty(*field_ty)));
        }

        // Expand each explicit arg if it's a class instance with pre-read fields;
        // otherwise pass as i64 handle.
        for call_arg in args {
            let expr = call_arg.expr();
            let arg_class = match expr {
                e if ident_name(e) == Some("self") => {
                    self.current_class.as_deref().map(|s| s.to_string())
                }
                e if ident_name(e).is_some() => {
                    self.param_classes.get(ident_name(e).unwrap()).cloned()
                }
                _ => None,
            };
            if let Some(ac) = arg_class {
                let arg_name = match ident_name(expr) { Some(n) => n, None => return None };
                let afields = self.class_fields_ord.get(&ac)?.clone();
                for (field_name, field_ty) in &afields {
                    let key = format!("{arg_name}.{field_name}");
                    let (al, ty) = self.preread_fields.get(&key)?.clone();
                    let r = self.fresh_reg();
                    self.ec(&format!("{r} = load {}, ptr {al}", llvm_ty(ty)));
                    parts.push(format!("{} {r}", llvm_ty(*field_ty)));
                }
            } else {
                // Non-class arg: pass as i64 handle
                let (v, vt) = self.gen_expr(expr);
                let h = self.to_handle(&v, vt);
                parts.push(format!("i64 {h}"));
            }
        }

        Some(parts.join(", "))
    }

    pub(super) fn gen_call(&mut self, func: &Expr, args: &[CallArg]) -> String {
        let arg_exprs: Vec<(String, Ty)> = args.iter()
            .map(|a| self.gen_expr(a.expr()))
            .collect();

        // Method call via attribute access: obj.method(args)
        if let Expr::Attr { object, attr, .. } | Expr::TraitAccess { object, attr, .. } = func {
            let key = if let Expr::TraitAccess { trait_name, .. } = func {
                format!("{trait_name}::{attr}")
            } else {
                attr.clone()
            };
            let (ov, ot) = self.gen_expr(object);
            let oh = self.to_handle(&ov, ot);

            // ── Direct intra-module method dispatch ───────────────────────────
            // If the method was compiled in this module, call its _impl directly —
            // no CB_CALL_METHOD overhead, no NATIVE_METHODS table lookup.
            let class_name = match object.as_ref() {
                e if ident_name(e) == Some("self") => self.current_class.clone(),
                e if ident_name(e).is_some() => {
                    self.param_classes.get(ident_name(e).unwrap()).cloned()
                }
                _ => None,
            };
            if let Some(cls) = &class_name {
                let sym = crate::partial_compiler::llvm_codegen::method_symbol(cls, &key);
                if self.module_fns.contains(&sym) && !self.locals.contains_key(&sym) {
                    let ret_ty = self.fn_sigs.get(&sym).map(|s| s.ret).unwrap_or(Ty::Handle);
                    // Collect explicit args as handles; self_h is prepended.
                    let explicit: Vec<String> = arg_exprs.iter()
                        .map(|(v, vt)| format!("i64 {}", self.to_handle(v, *vt)))
                        .collect();
                    let save = self.call_cb(CB_ARENA_SAVE, &[]);
                    let all_params = std::iter::once(format!("i64 {oh}"))
                        .chain(explicit).collect::<Vec<_>>().join(", ");
                    let raw = self.fresh_reg();
                    self.ec(&format!("{raw} = call i64 @{sym}_impl({all_params})"));
                    let result = self.call_cb(CB_ARENA_COMPACT,
                        &[format!("i64 {raw}"), format!("i64 {save}")]);
                    // Always return i64 handle; gen_expr unwraps to native type.
                    let _ = ret_ty;
                    return result;
                }
            }

            // ── Fall back to CB_CALL_METHOD (external / non-compiled method) ──
            let method_ptr = self.str_const(key.as_bytes());
            let method_len = key.len() as i32;
            let handles: Vec<String> = arg_exprs.iter()
                .map(|(v, vt)| self.to_handle(v, *vt))
                .collect();
            return if handles.is_empty() {
                self.call_cb(CB_CALL_METHOD, &[
                    format!("i64 {oh}"), method_ptr, format!("i32 {method_len}"),
                    "ptr null".to_string(), "i32 0".to_string()
                ])
            } else {
                let n = handles.len();
                // Use entry-block alloca to avoid stack growth inside loops.
                let arr = format!("%_margs{}", self.reg);
                self.reg += 1;
                self.ea(&format!("{arr} = alloca [{n} x i64], align 8"));
                for (i, h) in handles.iter().enumerate() {
                    let ep = self.fresh_reg();
                    self.ec(&format!("{ep} = getelementptr inbounds [{n} x i64], ptr {arr}, i32 0, i32 {i}"));
                    self.ec(&format!("store i64 {h}, ptr {ep}"));
                }
                self.call_cb(CB_CALL_METHOD, &[
                    format!("i64 {oh}"), method_ptr, format!("i32 {method_len}"),
                    format!("ptr {arr}"), format!("i32 {n}")
                ])
            };
        }

        // Intra-module direct call
        if let Some(name) = ident_name(func) {
            if self.module_fns.contains(name) && !self.locals.contains_key(name) {
                let mutabilities = self.fn_sigs.get(name)
                    .map(|s| s.param_mutabilities.clone());
                let call_args: Vec<String> = arg_exprs.iter().enumerate()
                    .map(|(i, (v, vt))| {
                        let h = self.to_handle(v, *vt);
                        let is_mut = mutabilities.as_ref()
                            .and_then(|m| m.get(i)).copied().unwrap_or(true);
                        if is_mut {
                            format!("i64 {h}")
                        } else {
                            let dc = self.call_cb(CB_DEEP_COPY, &[format!("i64 {h}")]);
                            format!("i64 {dc}")
                        }
                    })
                    .collect();

                let save = self.call_cb(CB_ARENA_SAVE, &[]);
                let param_str = call_args.join(", ");
                let raw = self.fresh_reg();
                self.ec(&format!("{raw} = call i64 @{name}_impl({param_str})"));
                let result = self.call_cb(CB_ARENA_COMPACT, &[format!("i64 {raw}"), format!("i64 {save}")]);

                return result; // always return a handle; gen_expr unwraps for typed callees
            }
        }

        // Fast path: function-typed param with a cached trampoline pointer.
        // Loads one local ptr instead of the three-instruction ArCallbacks GEP chain,
        // letting LLVM hoist the load out of loops and keep it in a register.
        if let Some(name) = ident_name(func) {
            if let Some(tp_al) = self.fn_param_trampolines.get(name).cloned() {
                let (fn_h_val, fn_h_ty) = self.gen_expr(func);
                let fn_h = self.to_handle(&fn_h_val, fn_h_ty);
                let handles: Vec<String> = arg_exprs.iter()
                    .map(|(v, vt)| self.to_handle(v, *vt))
                    .collect();
                let tp = self.fresh_reg();
                self.ec(&format!("{tp} = load ptr, ptr {tp_al}"));
                return if handles.is_empty() {
                    let r = self.fresh_reg();
                    self.ec(&format!("{r} = call i64 (i64, ptr, i32) {tp}(i64 {fn_h}, ptr null, i32 0)"));
                    r
                } else {
                    let n   = handles.len();
                    let arr = format!("%_targs{}", self.reg); self.reg += 1;
                    self.ea(&format!("{arr} = alloca [{n} x i64], align 8"));
                    for (i, h) in handles.iter().enumerate() {
                        let ep = self.fresh_reg();
                        self.ec(&format!("{ep} = getelementptr inbounds [{n} x i64], ptr {arr}, i32 0, i32 {i}"));
                        self.ec(&format!("store i64 {h}, ptr {ep}"));
                    }
                    let r = self.fresh_reg();
                    self.ec(&format!("{r} = call i64 (i64, ptr, i32) {tp}(i64 {fn_h}, ptr {arr}, i32 {n})"));
                    r
                };
            }
        }

        // Generic call through cb_call_fn
        let (fn_h_val, fn_h_ty) = self.gen_expr(func);
        let fn_h = self.to_handle(&fn_h_val, fn_h_ty);
        let handles: Vec<String> = arg_exprs.iter()
            .map(|(v, vt)| self.to_handle(v, *vt))
            .collect();
        if handles.is_empty() {
            self.call_cb(CB_CALL_FN, &[format!("i64 {fn_h}"), "ptr null".to_string(), "i32 0".to_string()])
        } else {
            let n   = handles.len();
            let arr = format!("%_cargs{}", self.reg); self.reg += 1;
            self.ea(&format!("{arr} = alloca [{n} x i64], align 8"));
            for (i, h) in handles.iter().enumerate() {
                let ep = self.fresh_reg();
                self.ec(&format!("{ep} = getelementptr inbounds [{n} x i64], ptr {arr}, i32 0, i32 {i}"));
                self.ec(&format!("store i64 {h}, ptr {ep}"));
            }
            self.call_cb(CB_CALL_FN, &[format!("i64 {fn_h}"), format!("ptr {arr}"), format!("i32 {n}")])
        }
    }

    // ── Statement generation ──────────────────────────────────────────────────

}
