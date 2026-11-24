// llvm_codegen/expr.rs — 式の LLVM IR 生成: gen_expr とその補助(二項演算・特殊化・高速呼び出し引数・呼び出し生成)。

use crate::ast::{BinOp, CallArg, Expr, MatchPattern, Resolution, UnaryOp};
use super::*;

mod ops;
mod direct_calls;
mod builtins;
mod fields;
mod calls;
mod methods;

pub(super) use self::fields::*;
pub(super) use self::calls::*;

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
                // VM の定数表と同じく、箇所ごとに 1 回だけ作って使い回す（11-2・D-12）
                let bytes = s.as_bytes();
                let ptr   = self.str_const(bytes);
                let len   = bytes.len() as i32;
                let cell  = self.ic_cell();
                let r = self.call_cb(CB_MAKE_STR_IC, &[cell, ptr, format!("i32 {len}")]);
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

            // `not` は必ず bool を返す（インタープリタも `!is_truthy`）ので、結果は `i1` のまま（4-3）。
            // `bool` / `int` / `float` は命令だけで、ほかは `CB_IS_TRUTHY`（`CB_UNOP` と同じ真偽の規則）で判定する。
            // ⚠ 以前は `CB_UNOP` を呼んで結果をハンドルで返し、`if not x:` はそれをまた `CB_IS_TRUTHY` で開けていた。
            Expr::UnaryOp { op: UnaryOp::Not, operand } => {
                let (v, vt) = self.gen_expr(operand);
                let c = self.to_cond(&v, vt);
                let r = self.fresh_reg();
                self.ec(&format!("{r} = xor i1 {c}, true"));
                (r, Ty::Bool)
            }

            Expr::UnaryOp { op, operand } => {
                let op_code = match op { UnaryOp::Neg => 0i32, UnaryOp::Not => 1, UnaryOp::BitNot => 2 };
                let (v, vt) = self.gen_expr(operand);
                let h = self.to_handle(&v, vt);
                let r = self.call_cb(CB_UNOP, &[format!("i32 {op_code}"), format!("i64 {h}")]);
                (r, Ty::Handle)
            }

            Expr::Call { func, args, node_id, span, .. } => self.gen_call_expr(func, args, (*node_id, span)),

            Expr::Attr { object, attr, node_id, .. } => {
                // 番地で呼ぶ変種の本体の `self.f`（`mut self` を番地で受けた・7-14）: 検査なしで読む
                if self.blk_self.is_some() && ident_name(object) == Some("self") {
                    return self.emit_blk_self_read(attr);
                }
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
                if let Some(r) = self.emit_nested_field_read(object, attr, known_ty) {
                    return r;
                }
                let (obj, ot) = self.gen_expr(object);
                let h = self.to_handle(&obj, ot);
                self.emit_field_read(object, &h, attr, known_ty)
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
                self.block_stack.push(BlockCtx { result_al: result_al.clone(), exit_label: exit_lbl.clone(), list_al: None, iter_depth: self.iter_scopes.len() });
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
                self.block_stack.push(BlockCtx { result_al: result_al.clone(), exit_label: merge.clone(), list_al: None, iter_depth: self.iter_scopes.len() });
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
                // 要素の並びか 1 つずつ（文の `for` と同じ・7-20）
                let fl = self.emit_for_begin(&ih);
                let iter_al = fl.iter_al.clone();
                // 名前の場所は `alloca_var` が決める（同じ名前の 2 つ目の宣言・1-10）
                let tgt_al = self.alloca_var(target, Ty::Handle);
                let fuse = self.new_loop_fuse_slot();
                let pre_pos = self.loop_pre_pos();
                self.br(&loop_blk);
                self.start_block(&loop_blk);
                self.emit_for_next(&fl, &fuse, &tgt_al, &exit_blk);
                self.block_stack.push(BlockCtx {
                    result_al: result_al.clone(), exit_label: exit_blk.clone(), list_al: list_al.clone(),
                    iter_depth: self.iter_scopes.len(),
                });
                self.loop_fuse_slot = Some(fuse);
                self.gen_loop_body(body, &loop_blk, &exit_blk, pre_pos, |_| {});
                self.block_stack.pop();
                self.start_block(&exit_blk);
                self.drop_iter_scope(&iter_al);
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
                let pre_pos = self.loop_pre_pos();
                self.br(&cond_blk);
                self.start_block(&cond_blk);
                let (cv, ct) = self.gen_expr(cond);
                let cc = self.to_cond(&cv, ct);
                self.br_cond(&cc, &body_blk, &exit_blk);
                self.start_block(&body_blk);
                self.block_stack.push(BlockCtx {
                    result_al: result_al.clone(), exit_label: exit_blk.clone(), list_al: list_al.clone(),
                    iter_depth: self.iter_scopes.len(),
                });
                self.gen_loop_body(body, &cond_blk, &exit_blk, pre_pos, |_| {});
                self.block_stack.pop();
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
                self.block_stack.push(BlockCtx { result_al: result_al.clone(), exit_label: merge.clone(), list_al: None, iter_depth: self.iter_scopes.len() });
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
}
