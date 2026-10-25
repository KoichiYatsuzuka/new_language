// llvm_codegen/expr.rs — 式の LLVM IR 生成: gen_expr とその補助(二項演算・特殊化・高速呼び出し引数・呼び出し生成)。

use crate::ast::{BinOp, CallArg, Expr, MatchPattern, Resolution, UnaryOp};
use super::*;

/// メソッド呼び出しの受け手のクラス（[`GenCtx::receiver_class`] の答え・3-1）。
struct Receiver {
    class: String,
    /// 型検査の注釈で決めた（実行時に class_id を確かめてから直接呼ぶ・3-2・D-2）
    guarded: bool,
}

/// 型検査が型を記録する式の node-id（受け手の注釈を引くため・3-1）。0 は未採番（合成 AST）。
fn expr_node_id(e: &Expr) -> Option<u32> {
    match e {
        Expr::Ident { node_id, .. }
        | Expr::Attr { node_id, .. }
        | Expr::Call { node_id, .. }
        | Expr::Subscript { node_id, .. }
        | Expr::Cast { node_id, .. } => Some(*node_id),
        _ => None,
    }
}

impl<'a> GenCtx<'a> {
    /// 式の IR を生成し `(値, 型区分)` を返す。
    ///
    /// #16 段階 c-2: 生成結果が `Ty::Handle`（＝型が判らずボックス化ハンドルのまま）に
    /// 落ちた式のうち、AST 型解決層の注釈が具象型を持っているものを集計する。
    /// これが「注釈へ移行すると新たに型特化できる箇所」の実測値になる（`AR_ANNOT_DIFF=1`）。
    pub fn gen_expr(&mut self, expr: &Expr) -> (String, Ty) {
        let out = self.gen_expr_inner(expr);
        // #15b の消費者判定: 識別子読みが実際に型を落としているか。
        // 下の `expr_stats` 集計は `annotatable_node_id` でゲートされており、同関数は
        // 識別子系に `None` を返すので構造上ここを数えられない。だから別に数える。
        if ident_name(expr).is_some() {
            let typed = out.1 != Ty::Handle;
            // 型を落とした識別子読みについて「注釈なら答えられたか」を分ける。
            // annot_only > 0 が #15b を消費側へ配線する根拠になる。
            if !typed {
                use crate::type_check::InferredType as IT;
                let annot = annotatable_node_id(expr)
                    .and_then(|nid| self.annotations.resolved_type(nid));
                match annot {
                    // 注釈が int/float ＝ codegen は特化できたはず（#15b で取り込める箇所）。
                    Some(IT::Int) | Some(IT::Float) => self.ident_stats.annot_only += 1,
                    // 注釈が「具象だがハンドル表現が正しい型」＝ 取りこぼしではない。
                    Some(t) if !matches!(t, IT::Unresolved | IT::Any) =>
                        self.ident_stats.annot_boxed += 1,
                    // 注釈が無い or Unresolved/Any ＝ 律速は型検査の解像度。
                    _ => self.ident_stats.annot_none += 1,
                }
            }
            let s = &mut self.ident_stats;
            match expr {
                Expr::Ident { res: Resolution::Local(slot), .. }
                    if matches!(self.locals_by_slot.get(*slot as usize), Some(Some(_))) =>
                {
                    if typed { s.slot_typed += 1 } else { s.slot_handle += 1 }
                }
                _ if self.locals.contains_key(ident_name(expr).unwrap()) => {
                    if typed { s.name_typed += 1 } else { s.name_handle += 1 }
                }
                _ => s.global_ref += 1,
            }
        }
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
                if let Expr::Ident { res: Resolution::Local(slot), .. } = e {
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

            Expr::Call { func, args, .. } => self.gen_call_expr(func, args),

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

                // Callback path: typed single-callback read for known typed fields, a handle otherwise —
                // both with a per-site inline cache shared with the VM's `GetAttr` (2-2).
                let known_ty = self.field_ty_resolved(object, attr, *node_id);
                let (obj, ot) = self.gen_expr(object);
                let h = self.to_handle(&obj, ot);
                self.emit_attr_read(&h, attr, known_ty)
            }

            // `obj::Trait.attr`: VM の `GetTraitAttr` と同じ `trait_access_evaled`（1-4）。
            // 以前は `CB_GET_ATTR` に `"Trait::attr"` という名前を渡していた。
            Expr::TraitAccess { object, trait_name, attr } => {
                let (obj, ot) = self.gen_expr(object);
                let h    = self.to_handle(&obj, ot);
                let tptr = self.str_const(trait_name.as_bytes());
                let tlen = trait_name.len() as i32;
                let ptr  = self.str_const(attr.as_bytes());
                let len  = attr.len() as i32;
                let r    = self.call_cb(CB_GET_TRAIT_ATTR, &[
                    format!("i64 {h}"), tptr, format!("i32 {tlen}"), ptr, format!("i32 {len}"),
                ]);
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
                    // ⚠ `*other` は `expr_eligible` が弾いているのでここには来ない。
                    //   来たら黙って落とさず panic させる（silent drop を作らない）。
                    let crate::ast::SeqEntry::Item(item) = item else {
                        unreachable!("`*` を含む列リテラルはネイティブ非適格（expr_eligible）")
                    };
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
                    // ⚠ `*other` は `expr_eligible` が弾いているのでここには来ない。
                    //   来たら黙って落とさず panic させる（silent drop を作らない）。
                    let crate::ast::SeqEntry::Item(item) = item else {
                        unreachable!("`*` を含む列リテラルはネイティブ非適格（expr_eligible）")
                    };
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
                // ⚠ `**other` は `expr_eligible` が弾いているのでここには来ない。
                //   来たら黙って落とさず panic させる（silent drop を作らない）。
                let pairs: Vec<(&Expr, &Expr)> = pairs
                    .iter()
                    .map(|e| match e {
                        crate::ast::DictEntry::Pair(k, v) => (k, v),
                        crate::ast::DictEntry::Spread(_) => unreachable!(
                            "`**` を含む辞書リテラルはネイティブ非適格（expr_eligible）"
                        ),
                    })
                    .collect();
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

            // ⚠ 多ターゲット（`for k, v in ...`）はネイティブ非適格にしてある
            //   （`expr_eligible` 側）。ここでは先頭 1 つだけを見る。
            Expr::ForExpr { targets, iter, body, .. } => {
                let target = &targets[0];
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
    /// Returns None if a flattened operand has no pre-read values here (fallback to _impl).
    ///
    /// The _fast ABI of `sym` is given by the CALLEE (`FnSig::fast_flat`), parameter by parameter
    /// (the receiver is param 0): a flattened class param takes all its typed fields as scalars,
    /// any other param an i64 handle.
    ///
    /// ⚠ **全部確かめてから出力する。** 以前は途中の `?` で `None` を返すと、それまでに出した
    /// 引数の評価コードが残ったまま `_impl` 経路で評価し直していた（副作用のある引数が 2 回走る）。
    /// ⚠ 展開は呼ばれる側の表に従う。以前は「呼ぶ側に先読みがあるか」で展開を決めており、
    /// 呼ばれる側がその仮引数をハンドルで受ける（`mut`・本体で書き換える）と並びが食い違った。
    pub(super) fn build_fast_call_args(&mut self, sym: &str, object: &Expr, args: &[CallArg]) -> Option<String> {
        let sig = self.fn_sigs.get(sym)?;
        let (flat, muts) = (sig.fast_flat.clone(), sig.param_mutabilities.clone());
        let operands: Vec<&Expr> = std::iter::once(object).chain(args.iter().map(|a| a.expr())).collect();
        if flat.len() != operands.len() { return None; }

        // 1) check: every flattened operand is a pre-read receiver of the same class here
        let mut plans: Vec<Option<Vec<(String, Ty)>>> = Vec::with_capacity(operands.len());
        for (op, want) in operands.iter().zip(&flat) {
            let Some(want_cls) = want else { plans.push(None); continue };
            let name = ident_name(op)?;
            let have_cls = if name == "self" {
                self.current_class.clone()?
            } else {
                self.param_classes.get(name)?.clone()
            };
            if &have_cls != want_cls { return None; }
            let mut loads = Vec::new();
            for (field_name, _) in self.class_fields_ord.get(want_cls)? {
                loads.push(self.preread_fields.get(&format!("{name}.{field_name}"))?.clone());
            }
            plans.push(Some(loads));
        }

        // 2) emit
        let mut parts: Vec<String> = Vec::new();
        for (i, (op, plan)) in operands.iter().zip(plans).enumerate() {
            match plan {
                Some(loads) => {
                    for (al, ty) in loads {
                        let r = self.fresh_reg();
                        self.ec(&format!("{r} = load {}, ptr {al}", llvm_ty(ty)));
                        parts.push(format!("{} {r}", llvm_ty(ty)));
                    }
                }
                None => {
                    let (v, vt) = self.gen_expr(op);
                    let h = self.to_handle(&v, vt);
                    let h = self.copy_for_let(&h, vt, op, muts[i], i == 0);
                    parts.push(format!("i64 {h}"));
                }
            }
        }
        Some(parts.join(", "))
    }

    /// モジュール内のメソッド `@{sym}_impl` へ直接渡すハンドル列（[0] = 受け手）。
    /// 呼ばれる側が `let` で受ける仮引数・`let self` には、`mut` の値を複製して渡す（D-8・1-5）。
    /// ⚠ 以前はどちらの直接呼び出しも複製せず、`mut` の値を `let` で受けたメソッドが返した値が
    /// 呼び出し元のその後の書き換えで一緒に変わった（§0.3 ⑤）。
    fn method_call_operands(
        &mut self,
        sym: &str,
        object: &Expr,
        oh: &str,
        ot: Ty,
        args: &[CallArg],
        arg_exprs: &[(String, Ty)],
    ) -> Vec<String> {
        // `sym` はモジュール内の関数（`is_module_fn`）なので `fn_sigs` に必ずある。
        let muts = self.fn_sigs[sym].param_mutabilities.clone();
        // 仮引数より多い引数は `_impl` の型が合わず clang が落とす（ここの選択は結果に効かない）。
        let callee_mut = |i: usize| muts.get(i).copied().unwrap_or(true);
        let mut out = vec![self.copy_for_let(oh, ot, object, callee_mut(0), true)];
        for (i, ((v, vt), a)) in arg_exprs.iter().zip(args).enumerate() {
            let h = self.to_handle(v, *vt);
            out.push(self.copy_for_let(&h, *vt, a.expr(), callee_mut(i + 1), false));
        }
        out
    }


    /// `name` が**このモジュールでコンパイルされた関数**か（＝`@{name}_impl` を直接呼べるか）。
    ///
    /// ⚠ `locals` を必ず除く。同名のローカル変数があるとそちらが優先されるので、
    /// 直接呼び出しに落とすと**別のものを呼ぶ**。
    /// ⚠ #64 以前はこの 2 条件が `gen_call_expr` と `gen_call` に**計 4 箇所**書かれていた。
    fn is_module_fn(&self, name: &str) -> bool {
        self.module_fns.contains(name) && !self.locals.contains_key(name)
    }

    /// モジュール内のメソッドを `_impl` で直接呼ぶ前に、呼ばれるメソッドのクラスの文脈へ入る（1-9）。
    /// 呼ぶ側（今コンパイル中の関数のクラス・自由関数なら無し）と同じクラスなら何もしない（`self.m()` の大半）。
    /// 違えば `CB_ENTER_METHOD_CTX` で `current_class` を受け手のクラスにし、その印を返す。
    /// ⚠ `_fast` 変種はコールバックを呼ばない（アクセスの検査が無い）ので包まない。
    fn enter_callee_ctx(&mut self, callee_class: &str, receiver_h: &str) -> Option<String> {
        if self.current_class.as_deref() == Some(callee_class) {
            return None;
        }
        Some(self.call_cb(CB_ENTER_METHOD_CTX, &[format!("i64 {receiver_h}")]))
    }

    /// [`Self::enter_callee_ctx`] の後始末（呼び出しの直後・エラーがあっても必ず通る）。
    fn leave_callee_ctx(&mut self, token: Option<String>) {
        if let Some(t) = token {
            self.call_cb(CB_LEAVE_METHOD_CTX, &[format!("i64 {t}")]);
        }
    }

    /// メソッド呼び出しのレシーバ式から**クラス**を引く（`obj.m()` の `obj`）。
    ///
    /// - `self` … 現在コンパイル中のクラス（ガード不要）
    /// - 型注釈付きの仮引数（`param_classes`）… その型（ガード不要: 引数は型検査が呼び出しで確かめる。`Any` は渡せない）
    /// - それ以外（フィールド `self.pos`・ループ変数・注釈付きのローカル・`int` / `float` のフィールドを持たないクラスの
    ///   仮引数など）… 型検査の注釈（`resolved_type` が Arrow のクラスの `NamedInstance`）。**実行時のガード付き**
    ///   （3-1・3-2・D-2: 注釈はヒントなので、外れたら名前による呼び出しへ回る）。注釈が無い／`node_id` 0 は不明
    ///
    /// ⚠ #64 以前は `gen_call_expr` と `gen_call` に**逐語で同じ `match`** が置かれていた。
    /// ⚠ 3-1 以前は `self` と `int` / `float` のフィールドを持つクラスの仮引数しか引けず、ほかは全部名前で呼んでいた。
    fn receiver_class(&self, object: &Expr) -> Option<Receiver> {
        match object {
            e if ident_name(e) == Some("self") => {
                self.current_class.clone().map(|class| Receiver { class, guarded: false })
            }
            e if ident_name(e).is_some_and(|n| self.param_classes.contains_key(n)) => {
                let class = self.param_classes[ident_name(e).unwrap()].clone();
                Some(Receiver { class, guarded: false })
            }
            _ => {
                use crate::type_check::InferredType;
                let nid = expr_node_id(object).filter(|&n| n != 0)?;
                match self.annotations.resolved_type(nid)? {
                    InferredType::NamedInstance(n) if self.annotations.is_arrow_class(n) => {
                        Some(Receiver { class: n.clone(), guarded: true })
                    }
                    _ => None,
                }
            }
        }
    }

    /// ハンドル ABI（[`Self::gen_call`]）で呼んでから、戻り値型に応じて**開けて返す**（#64）。
    ///
    /// `Ty::Float` はここへ来ない（呼び出し側が `double` 直返しの高速経路を選ぶ）。
    /// ⚠ #64 以前は関数版とメソッド版に逐語で同じものが書かれていた。
    fn gen_call_unwrapped(&mut self, func: &Expr, args: &[CallArg], ret_ty: Ty) -> (String, Ty) {
        let h = self.gen_call(func, args);
        match ret_ty {
            Ty::Int => {
                let r = self.call_cb(CB_TO_INT, &[format!("i64 {h}")]);
                (r, Ty::Int)
            }
            _ => (h, Ty::Handle),
        }
    }

    /// 引数ハンドルを **entry ブロックの `alloca` 配列**へ積んでポインタ名を返す（#64）。
    ///
    /// ⚠ `alloca` は **entry ブロック**へ出す（`ea`）。ループ本体に置くと反復ごとに
    /// スタックが伸びる。⚠ #64 以前は `gen_call` に**逐語で 3 回**書かれていた
    /// （違ったのは生成 IR 側の配列名の接頭辞 margs / targs / cargs だけ）。
    fn spill_handle_array(&mut self, handles: &[String], tag: &str) -> String {
        let n = handles.len();
        let arr = format!("%_{tag}{}", self.reg);
        self.reg += 1;
        self.ea(&format!("{arr} = alloca [{n} x i64], align 8"));
        for (i, h) in handles.iter().enumerate() {
            let ep = self.fresh_reg();
            self.ec(&format!("{ep} = getelementptr inbounds [{n} x i64], ptr {arr}, i32 0, i32 {i}"));
            self.ec(&format!("store i64 {h}, ptr {ep}"));
        }
        arr
    }

    /// `f(args)` / `obj.m(args)` の**式としての**コード生成（#64 で `gen_expr_inner` から切り出し）。
    ///
    /// ## ⚠⚠ [`gen_call`] との役割分担（#64 で確かめた結果・**畳んではいけない**）
    ///
    /// | | ここ（`gen_call_expr`） | [`gen_call`] |
    /// |---|---|---|
    /// | 戻り値 | `(値, ネイティブ型)` | **i64 ハンドル 1 本** |
    /// | 呼び出し規約 | 戻り値が `Ty::Float` のときだけ **`double` を返す `_impl` / `_fast`** を直接呼ぶ | **ハンドル ABI 一本** |
    /// | アリーナ | **触らない**（save/compact を出さない） | `CB_ARENA_SAVE` → `CB_ARENA_COMPACT` を必ず挟む |
    /// | boxing | 無し（スカラーのまま） | 有り |
    ///
    /// ⇒ **同じ呼び先に対する 2 つの ABI** であって、重複実装ではない。
    /// 片方に寄せると **float の高速経路が消える**（アリーナ操作と boxing が復活する）か、
    /// 逆に**アリーナの巻き戻しが漏れる**。`Ty::Float` 以外はここから `gen_call` へ委譲し、
    /// `Ty::Int` だけ `CB_TO_INT` で開ける（[`Self::gen_call_unwrapped`]）。
    ///
    /// ⚠ **重複していたのは判断のほうだった** — 「この名前はモジュール内関数か」
    /// （[`Self::is_module_fn`]）と「レシーバのクラスは何か」（[`Self::receiver_class`]）を
    /// 両方が各自で書いていた。#64 でそこだけ 1 箇所に畳んである。
    fn gen_call_expr(&mut self, func: &Expr, args: &[CallArg]) -> (String, Ty) {
        // Cell built-ins: __make_cell / __get_cell / __set_cell
        if let Some(n) = ident_name(func) {
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
            if let Some(name) = ident_name(func) {
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
        if let Some(name) = ident_name(func) {
            if self.is_module_fn(name) {
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
                return self.gen_call_unwrapped(func, args, ret_ty);
            }
        }
        // ── Typed intra-module method calls on known class instances ──
        if let Expr::Attr { object, attr, .. } = func {
            let receiver = self.receiver_class(object);
            if let Some(Receiver { class: cls, guarded }) = &receiver {
                let sym = method_symbol(cls, attr);
                if self.is_module_fn(&sym) {
                    let ret_ty = self.fn_sigs.get(&sym).map(|s| s.ret).unwrap_or(Ty::Handle);

                    // ガード付き（注釈で決めた受け手）は `double` を返す高速経路を使わず、ハンドル ABI の
                    // ガード付き呼び出し（`gen_call`）で呼ぶ（外れたときの名前による呼び出しはハンドルを返す）。
                    if ret_ty == Ty::Float && !*guarded {
                        // Preferred: _fast variant — passes pre-read field values as
                        // scalars; zero callbacks and LLVM-inlineable pure arithmetic.
                        // 破棄された `_fast` 変種は選ばない（生成側と条件を揃える）。
                        if self.fast_fns.contains(&sym)
                            && !self.discarded_fast.contains(&sym)
                        {
                            if let Some(fast_args) = self.build_fast_call_args(&sym, object, args) {
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
                        let explicit = self.method_call_operands(&sym, object, &oh, ot, args, &arg_exprs);
                        let oh = explicit[0].clone();
                        let explicit: Vec<String> = explicit[1..].iter().map(|h| format!("i64 {h}")).collect();
                        let all_params = std::iter::once(format!("i64 {oh}"))
                            .chain(explicit).collect::<Vec<_>>().join(", ");
                        let ctx = self.enter_callee_ctx(cls, &oh);
                        let r = self.fresh_reg();
                        self.ec(&format!("{r} = call double @{sym}_impl({all_params})"));
                        self.leave_callee_ctx(ctx);
                        return (r, Ty::Float);
                    }

                    // Non-float typed return: handle ABI + CB_TO_INT.
                    return self.gen_call_unwrapped(func, args, ret_ty);
                }
            }
        }
        (self.gen_call(func, args), Ty::Handle)
    }
    /// モジュール内のメソッドを `_impl`（ハンドル ABI）で直接呼ぶ（値渡しの複製・1-5、呼ばれる側の文脈・1-9 込み）。
    #[allow(clippy::too_many_arguments)]
    fn emit_direct_method_call(
        &mut self,
        sym: &str,
        cls: &str,
        object: &Expr,
        oh: &str,
        ot: Ty,
        args: &[CallArg],
        arg_exprs: &[(String, Ty)],
    ) -> String {
        // Collect explicit args as handles; self_h is prepended.
        let explicit = self.method_call_operands(sym, object, oh, ot, args, arg_exprs);
        let oh = explicit[0].clone();
        let explicit: Vec<String> = explicit[1..].iter().map(|h| format!("i64 {h}")).collect();
        let all_params = std::iter::once(format!("i64 {oh}"))
            .chain(explicit).collect::<Vec<_>>().join(", ");
        // ⚠⚠ `float` を返すメソッドの `_impl` は `double` を返す（`gen_call_expr` の高速経路と同じ ABI）。
        //   `call i64` で呼ぶと LLVM は通すが、整数レジスタのごみを返り値のハンドルとして読む（3-2 で
        //   ガード付きの呼び出しが float を返すメソッドに当たり `float * None` になった。以前も文として
        //   呼んだときは同じ食い違いがあり、結果を捨てるので表に出ていなかった）。アリーナは触らず箱に入れる。
        if self.fn_sigs[sym].ret == Ty::Float {
            let ctx = self.enter_callee_ctx(cls, &oh);
            let r = self.fresh_reg();
            self.ec(&format!("{r} = call double @{sym}_impl({all_params})"));
            self.leave_callee_ctx(ctx);
            return self.to_handle(&r, Ty::Float);
        }
        let save = self.call_cb(CB_ARENA_SAVE, &[]);
        let ctx = self.enter_callee_ctx(cls, &oh);
        let raw = self.fresh_reg();
        self.ec(&format!("{raw} = call i64 @{sym}_impl({all_params})"));
        self.leave_callee_ctx(ctx);
        // Always return i64 handle; gen_expr unwraps to native type.
        self.call_cb(CB_ARENA_COMPACT, &[format!("i64 {raw}"), format!("i64 {save}")])
    }

    /// 注釈で決めた受け手のメソッド呼び出し（3-2・D-2）: ガード（`CB_METHOD_GUARD`）を通ったときだけ直接呼び、
    /// 外れたら名前で呼ぶ。両方の道の結果は entry の alloca に置いて合流する（道の中でブロックが増えても正しい・
    /// mem2reg が phi にする）。引数と受け手は分岐の前に 1 回だけ評価してある。
    #[allow(clippy::too_many_arguments)]
    fn emit_guarded_method_call(
        &mut self,
        sym: &str,
        cls: &str,
        key: &str,
        object: &Expr,
        oh: &str,
        ot: Ty,
        args: &[CallArg],
        arg_exprs: &[(String, Ty)],
    ) -> String {
        // `mut self` のメソッドは不変インスタンスでガードを外す（名前による呼び出しがエラーにする）
        let need_mut = self.fn_sigs[sym].param_mutabilities.first().copied().unwrap_or(true);
        let cell = self.ic_cell();
        let cptr = self.str_const(cls.as_bytes());
        let ok = self.call_cb(CB_METHOD_GUARD, &[
            format!("i64 {oh}"), cptr, format!("i32 {}", cls.len()), cell, format!("i32 {}", i32::from(need_mut)),
        ]);
        let slot = format!("%_mres{}", self.reg);
        self.reg += 1;
        self.ea(&format!("{slot} = alloca i64, align 8"));
        let cond = self.fresh_reg();
        self.ec(&format!("{cond} = icmp ne i32 {ok}, 0"));
        let (b_direct, b_name, b_join) = (self.fresh_blk(), self.fresh_blk(), self.fresh_blk());
        self.ec(&format!("br i1 {cond}, label %{b_direct}, label %{b_name}"));
        self.terminated = true;
        self.start_block(&b_direct);
        let rd = self.emit_direct_method_call(sym, cls, object, oh, ot, args, arg_exprs);
        self.ec(&format!("store i64 {rd}, ptr {slot}"));
        self.br(&b_join);
        self.start_block(&b_name);
        let rn = self.emit_method_call_by_name(key, object, oh, args, arg_exprs);
        self.ec(&format!("store i64 {rn}, ptr {slot}"));
        self.br(&b_join);
        self.start_block(&b_join);
        let r = self.fresh_reg();
        self.ec(&format!("{r} = load i64, ptr {slot}"));
        r
    }

    /// 名前によるメソッド呼び出し（`CB_CALL_METHOD_M`・外のメソッド・コンパイルしていないメソッド・ガードが外れたとき）。
    /// 呼ばれる側の仮引数の `let` / `mut` はここでは分からないので、渡す値が `mut` かを `mut_mask` で渡し、
    /// 複製は `ar_call_method_m` が呼ばれる側の宣言を見て行う（D-8・1-5）。
    fn emit_method_call_by_name(
        &mut self,
        key: &str,
        object: &Expr,
        oh: &str,
        args: &[CallArg],
        arg_exprs: &[(String, Ty)],
    ) -> String {
        let method_ptr = self.str_const(key.as_bytes());
        let method_len = key.len() as i32;
        let mask = self.method_mut_mask(object, args);
        let handles: Vec<String> = arg_exprs.iter()
            .map(|(v, vt)| self.to_handle(v, *vt))
            .collect();
        if handles.is_empty() {
            self.call_cb(CB_CALL_METHOD_M, &[
                format!("i64 {oh}"), method_ptr, format!("i32 {method_len}"),
                "ptr null".to_string(), "i32 0".to_string(), format!("i32 {mask}")
            ])
        } else {
            let n = handles.len();
            let arr = self.spill_handle_array(&handles, "margs");
            self.call_cb(CB_CALL_METHOD_M, &[
                format!("i64 {oh}"), method_ptr, format!("i32 {method_len}"),
                format!("ptr {arr}"), format!("i32 {n}"), format!("i32 {mask}")
            ])
        }
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
            if let Some(Receiver { class: cls, guarded }) = self.receiver_class(object) {
                let sym = crate::partial_compiler::llvm_codegen::method_symbol(&cls, &key);
                if self.is_module_fn(&sym) {
                    if !guarded {
                        return self.emit_direct_method_call(&sym, &cls, object, &oh, ot, args, &arg_exprs);
                    }
                    return self.emit_guarded_method_call(&sym, &cls, &key, object, &oh, ot, args, &arg_exprs);
                }
            }
            return self.emit_method_call_by_name(&key, object, &oh, args, &arg_exprs);
        }

        // Intra-module direct call
        if let Some(name) = ident_name(func) {
            if self.is_module_fn(name) {
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

                let param_str = call_args.join(", ");
                // `float` を返す関数の `_impl` は `double` を返す（メソッドの `emit_direct_method_call` と同じ理由）。
                // ここへ来るのは文として呼んだときなど（式なら `gen_call_expr` が `double` で受ける）。
                if self.fn_sigs.get(name).map(|s| s.ret) == Some(Ty::Float) {
                    let r = self.fresh_reg();
                    self.ec(&format!("{r} = call double @{name}_impl({param_str})"));
                    return self.to_handle(&r, Ty::Float);
                }
                let save = self.call_cb(CB_ARENA_SAVE, &[]);
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
                    let arr = self.spill_handle_array(&handles, "targs");
                    let r = self.fresh_reg();
                    self.ec(&format!("{r} = call i64 (i64, ptr, i32) {tp}(i64 {fn_h}, ptr {arr}, i32 {n})"));
                    r
                };
            }
        }

        // 組み込み関数（VM が `CallBuiltin` を出す名前）: グローバルから値として引かず、VM と同じ入口
        // （`ar_call_builtin` → `call_builtin_by_name_evaled`）で呼ぶ（native_object_access_plan.md 1-3）。
        // 判定は VM のコンパイラと同じ: 未解決の識別子で、ローカルに同名が無いこと（グローバルの同名は
        // 呼び出し時に `builtin_is_shadowed_global` が見る）。
        if let Expr::Ident { name, res: Resolution::Unresolved, .. } = func {
            if crate::vm::compiler::VM_BUILTIN_NAMES.contains(&name.as_str())
                && !self.locals.contains_key(name.as_str())
            {
                let ptr = self.str_const(name.as_bytes());
                let len = name.len() as i32;
                let handles: Vec<String> = arg_exprs.iter()
                    .map(|(v, vt)| self.to_handle(v, *vt))
                    .collect();
                return if handles.is_empty() {
                    self.call_cb(CB_CALL_BUILTIN, &[ptr, format!("i32 {len}"), "ptr null".to_string(), "i32 0".to_string()])
                } else {
                    let n   = handles.len();
                    let arr = self.spill_handle_array(&handles, "bargs");
                    self.call_cb(CB_CALL_BUILTIN, &[ptr, format!("i32 {len}"), format!("ptr {arr}"), format!("i32 {n}")])
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
            let arr = self.spill_handle_array(&handles, "cargs");
            self.call_cb(CB_CALL_FN, &[format!("i64 {fn_h}"), format!("ptr {arr}"), format!("i32 {n}")])
        }
    }

    // ── Statement generation ──────────────────────────────────────────────────

}
