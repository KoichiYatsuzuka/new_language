// llvm_codegen/expr/methods.rs — メソッド呼び出しの生成（展開・直接・検査つき・名前）。

use super::*;

impl<'a> GenCtx<'a> {
    /// `e` がこの関数のローカル変数（仮引数を含む）の読みか。
    pub(in crate::partial_compiler::llvm_codegen) fn is_local_ident(&self, e: &Expr) -> bool {
        match e {
            Expr::Ident { res: Resolution::Local(slot), .. }
                if matches!(self.locals_by_slot.get(*slot as usize), Some(Some(_))) =>
            {
                true
            }
            _ => ident_name(e).is_some_and(|n| self.locals.contains_key(n)),
        }
    }

    /// 番地で呼ぶ変種（`@X_blk`・copy_alloc_speed_plan.md 7-14）でメソッドを呼ぶ。当てはまらなければ何も出さずに `None`
    /// （呼ぶ側が今までの呼び出しを使う）。
    ///
    /// 当てはまるのは、呼ぶメソッドに番地で呼ぶ変種があり（`blk_ok`）、受け手と `let` のクラスの引数が「ローカル変数」か
    /// 「ローカル変数の属性」（`a.b`）、数値の引数が `int` / `float` と分かっている項（[`Self::numeric_leaf`]・値の評価が
    /// 何も起こさない）のとき。出すコード:
    /// 1. 受け手と引数の番地を取る（覚えた番地 `%_ip_x` / `%_apN`・8-5 / 7-13）
    /// 2. 番地ごとに: 空でない・全フィールドの初期化の印が立っている・`mut self` なら不変の印が立っていない。ガード付きの
    ///    受け手は塊の `class_id` がガードのセル（名前で引いたクラスの番号・0 はまだ）と同じ（番地を返すコールバックの表の
    ///    照合は名前と並びだけで、本体とモジュールの同じ名前・同じ並びのクラスを区別しない）
    /// 3. 平たくして渡す引数（受け手を除く）が `mut` の値なら、`__copy__` を持つクラスが無い（`let` で受ける写しが呼ぶ）
    /// 4. そろえば平たくするフィールドを読み（呼んだときの写し）、`@X_blk` を呼ぶ。外れたら今までの呼び出し
    ///    （[`Self::gen_method_call_expr`]。ガード付きなら先にガードを呼んでセルを埋める）。結果は置き場で合流する。
    /// ⚠ 外れた道は受け手と引数を AST からもう一度評価する（読むのは変数・属性・数の項だけで、何も起こさない）。
    /// ⚠ `let self` の写しは `__copy__` を呼ばない（`deep_copy`・7-7 の `ar_borrow_or_copy` と同じ）ので、受け手は 3 の対象外。
    pub(super) fn emit_blk_method_call(
        &mut self,
        func: &Expr,
        object: &Expr,
        attr: &str,
        args: &[CallArg],
        site: CallSite<'_>,
    ) -> Option<(String, Ty)> {
        use crate::interpreter::value::block;
        if self.typed_mode || self.blk_mode || self.terminated {
            return None;
        }
        let recv = self.receiver_class(object)?;
        let sym = method_symbol(&recv.class, attr);
        if !self.blk_ok.contains(&sym) || !self.direct_callee(&sym, args.len(), true) {
            return None;
        }
        let sig = self.blk_sigs.get(&sym)?.clone();
        if sig.params.len() != args.len() + 1 || args.iter().any(|a| !matches!(a, CallArg::Positional(_))) {
            return None;
        }
        let operands: Vec<&Expr> = std::iter::once(object).chain(args.iter().map(|a| a.expr())).collect();

        // ── 1) 当てはまるかを、コードを出す前に全部確かめる ──
        for (op, want) in operands.iter().zip(&sig.params) {
            match want {
                BlkOp::Ptr(c) | BlkOp::Flat(c) => {
                    if self.receiver_class(op).map(|r| r.class).as_ref() != Some(c) {
                        return None;
                    }
                    let local_src = match op {
                        Expr::Attr { object: parent, .. } => self.is_local_ident(parent),
                        e => self.is_local_ident(e),
                    };
                    if !local_src {
                        return None;
                    }
                    if let BlkOp::Flat(c) = want {
                        let fields = self.class_fields_ord.get(c)?.clone();
                        if fields.iter().any(|(f, t)| self.direct_read_plan(c, f, *t).is_none()) {
                            return None;
                        }
                    }
                }
                BlkOp::Num(_) => {
                    if !self.numeric_leaf(op) {
                        return None;
                    }
                }
            }
        }
        let ret = sig.ret;
        let rt = if ret == Ty::Float { "double" } else { "i64" };

        // ── 2) 受け手と引数の番地を取る: (親か変数のハンドル, 入れ子なら属性, 番地, 入れ子なら IC のセル) ──
        // 根（ローカル変数）を先に全部読んでから番地を取る: 同じ親の入れ子の受け手（`self.pos.add_scaled(self.vel, dt)`）は
        // `CB_ATTR_BLOCKS` の 1 回でまとめて取る（7-18）。根を読むのは何も起こさないので、順を変えても見分けられない
        type Got = (String, Option<String>, String, Option<String>);
        let mut srcs: Vec<(usize, String, String, Option<String>)> = Vec::new();
        for (i, (op, want)) in operands.iter().zip(&sig.params).enumerate() {
            let (BlkOp::Ptr(c) | BlkOp::Flat(c)) = want else { continue };
            let (src_e, a) = match op {
                Expr::Attr { object: parent, attr: a, .. } => (parent.as_ref(), Some(a.clone())),
                e => (*e, None),
            };
            let (v, t) = self.gen_expr(src_e);
            let h = self.to_handle(&v, t);
            srcs.push((i, c.clone(), h, a));
        }
        let items: Vec<(String, String, Option<String>)> =
            srcs.iter().map(|(_, c, h, a)| (c.clone(), h.clone(), a.clone())).collect();
        // クラスは raw レイアウト（`blk_sig_of` で確かめた）なので表があり、必ず番地のコードを出す
        let blocks = self.emit_block_srcs(&items).expect("a raw-layout class has a direct class table");
        let mut got: Vec<Option<Got>> = vec![None; operands.len()];
        for ((i, _, h, a), (p, ic)) in srcs.into_iter().zip(blocks) {
            got[i] = Some((h, a, p, ic));
        }

        // ── 3) 確かめる（外れたら遅い道へ） ──
        let slot = format!("%_bres{}", self.reg);
        self.reg += 1;
        self.ea(&format!("{slot} = alloca {rt}, align 8"));
        let (b_fast, b_slow, b_join) = (self.fresh_blk(), self.fresh_blk(), self.fresh_blk());
        let guard_cell = recv.guarded.then(|| self.ic_cell());
        for (i, (g, want)) in got.iter().zip(&sig.params).enumerate() {
            let Some((_, _, p, _)) = g else { continue };
            let (BlkOp::Ptr(c) | BlkOp::Flat(c)) = want else { continue };
            let nonnull = self.fresh_reg();
            self.ec(&format!("{nonnull} = icmp ne ptr {p}, null"));
            let b_chk = self.fresh_blk();
            self.br_cond(&nonnull, &b_chk, &b_slow);
            self.start_block(&b_chk);
            let n_fields = self.raw_classes.get(c).map_or(0, |rc| rc.layout.fields.len());
            let init = (0..n_fields).fold(0u64, |m, k| m | block::init_bit(k));
            let mask = if matches!(want, BlkOp::Ptr(_)) { init | block::IMMUTABLE } else { init };
            let head = self.fresh_reg();
            self.ec(&format!("{head} = load i64, ptr {p}, align 8"));
            let masked = self.fresh_reg();
            // LLVM の整数の定数は符号つきの 10 進で書く
            self.ec(&format!("{masked} = and i64 {head}, {}", mask as i64));
            let mut ok = self.fresh_reg();
            self.ec(&format!("{ok} = icmp eq i64 {masked}, {}", init as i64));
            if let (0, Some(cell)) = (i, &guard_cell) {
                let cell_ref = cell.trim_start_matches("ptr ").to_string();
                let cid = self.fresh_reg();
                self.ec(&format!("{cid} = and i64 {head}, {}", block::CLASS_ID_MASK as i64));
                let want_id = self.fresh_reg();
                self.ec(&format!("{want_id} = load atomic i64, ptr {cell_ref} monotonic, align 8"));
                let same = self.fresh_reg();
                self.ec(&format!("{same} = icmp eq i64 {cid}, {want_id}"));
                let known = self.fresh_reg();
                self.ec(&format!("{known} = icmp ne i64 {want_id}, 0"));
                let both = self.fresh_reg();
                self.ec(&format!("{both} = and i1 {same}, {known}"));
                let all = self.fresh_reg();
                self.ec(&format!("{all} = and i1 {ok}, {both}"));
                ok = all;
            }
            let b_next = self.fresh_blk();
            self.br_cond(&ok, &b_next, &b_slow);
            self.start_block(&b_next);
        }
        let flat_mut_arg = operands.iter().zip(&sig.params).skip(1)
            .any(|(op, want)| matches!(want, BlkOp::Flat(_)) && self.operand_is_mutable(op));
        if flat_mut_arg {
            let hooks = self.load_cb_data("i8", CB_COPY_HOOKS);
            let none = self.fresh_reg();
            self.ec(&format!("{none} = icmp eq i8 {hooks}, 0"));
            self.br_cond(&none, &b_fast, &b_slow);
        } else {
            self.br(&b_fast);
        }

        // ── 4) 速い道: 平たくするフィールドを読み、`@X_blk` を呼ぶ ──
        self.start_block(&b_fast);
        let mut parts: Vec<String> = Vec::new();
        for ((op, want), g) in operands.iter().zip(&sig.params).zip(&got) {
            match (want, g) {
                (BlkOp::Ptr(_), Some((_, _, p, _))) => parts.push(format!("ptr {p}")),
                (BlkOp::Flat(c), Some((_, _, p, _))) => {
                    let p = p.clone();
                    for (f, t) in self.class_fields_ord.get(c).cloned().unwrap_or_default() {
                        let (r, _) = self.emit_trusted_read(c, &p, &f, t).expect("checked before emitting");
                        parts.push(format!("{} {r}", llvm_ty(t)));
                    }
                }
                (BlkOp::Num(t), _) => {
                    let (v, vt) = self.gen_expr(op);
                    let v = if *t == Ty::Float { self.to_f64(&v, vt) } else { self.to_i64(&v, vt) };
                    parts.push(format!("{} {v}", llvm_ty(*t)));
                }
                _ => unreachable!("class operands always have a block"),
            }
        }
        let r = self.fresh_reg();
        self.ec(&format!("{r} = call {rt} @{sym}_blk({})", parts.join(", ")));
        self.ec(&format!("store {rt} {r}, ptr {slot}"));
        self.br(&b_join);

        // ── 5) 遅い道: 今までの呼び出し ──
        self.start_block(&b_slow);
        // 遅い道が積む値（受け手・引数のハンドル・結果）は、結果が生の数か使わない値なら道の中で巻き戻す（7-17）
        let region = (matches!(ret, Ty::Float | Ty::Int) || self.call_value_discarded(func))
            .then(|| self.begin_fallback_region())
            .flatten();
        // 次から速い道を通れるように、空だった入れ子の受け手・引数の番地の読み出しの IC のセルを埋める（`CB_ATTR_BLOCK` は
        // セルが空のうちは空を返す）。ガード付きの受け手は、ガードのセルが空なら埋める（結果は今までの呼び出しのガードが
        // もう一度決める）。番地が取れてほかの検査で外れたとき（`__copy__` を持つクラスがある など）は何も足さない。
        // ⚠ 以前は受け手のセルしか埋めず、`self.pos.add_scaled(self.vel, dt)` の `self.vel` がいつも空で、毎回この道を通った。
        for (h, a, p, ic) in got.iter().flatten().cloned().collect::<Vec<_>>() {
            let Some(a) = a else { continue };
            let empty = self.fresh_reg();
            self.ec(&format!("{empty} = icmp eq ptr {p}, null"));
            let (b_fill, b_cont) = (self.fresh_blk(), self.fresh_blk());
            self.br_cond(&empty, &b_fill, &b_cont);
            self.start_block(&b_fill);
            self.block_src_handle(BlockSrc::Field { parent_h: &h, attr: &a }, ic);
            self.br(&b_cont);
            self.start_block(&b_cont);
        }
        if let (Some(cell), Some(Some((h, a, _, ic)))) = (&guard_cell, got.first().cloned()) {
            let cell_ref = cell.trim_start_matches("ptr ").to_string();
            let have = self.fresh_reg();
            self.ec(&format!("{have} = load atomic i64, ptr {cell_ref} monotonic, align 8"));
            let empty = self.fresh_reg();
            self.ec(&format!("{empty} = icmp eq i64 {have}, 0"));
            let (b_fill, b_cont) = (self.fresh_blk(), self.fresh_blk());
            self.br_cond(&empty, &b_fill, &b_cont);
            self.start_block(&b_fill);
            let oh = match &a {
                Some(a) => self.block_src_handle(BlockSrc::Field { parent_h: &h, attr: a }, ic),
                None => h,
            };
            let cls = recv.class.clone();
            let cptr = self.str_const(cls.as_bytes());
            let need_mut = i32::from(matches!(sig.params[0], BlkOp::Ptr(_)));
            self.call_cb(CB_METHOD_GUARD, &[
                format!("i64 {oh}"), cptr, format!("i32 {}", cls.len()), cell.clone(), format!("i32 {need_mut}"),
            ]);
            self.br(&b_cont);
            self.start_block(&b_cont);
        }
        let (v, vt) = match self.gen_method_call_expr(func, args, site) {
            Some(x) => x,
            None => (self.gen_call(func, args, site), Ty::Handle),
        };
        let v = match ret {
            Ty::Float => self.to_f64(&v, vt),
            Ty::Int => self.to_i64(&v, vt),
            _ => self.to_handle(&v, vt),
        };
        if !self.terminated {
            self.ec(&format!("store {rt} {v}, ptr {slot}"));
        }
        self.end_fallback_region(region);
        self.br(&b_join);

        self.start_block(&b_join);
        let out = self.fresh_reg();
        self.ec(&format!("{out} = load {rt}, ptr {slot}"));
        Some((out, ret))
    }

    /// モジュール内のメソッドを `_impl`（ハンドル ABI）で直接呼ぶ（値渡しの複製・1-5、呼ばれる側の文脈・1-9 込み）。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_direct_method_call(
        &mut self,
        sym: &str,
        cls: &str,
        object: &Expr,
        oh: &str,
        ot: Ty,
        args: &[CallArg],
        arg_exprs: &[(String, Ty)],
    ) -> String {
        let (v, t) = self.emit_direct_method_call_typed(sym, cls, object, oh, ot, args, arg_exprs);
        self.to_handle(&v, t)
    }

    /// [`Self::emit_direct_method_call`] の本体: 結果を内部 ABI の型のまま返す（`float` → `double`・`int` → 生の i64）。
    /// ⚠⚠ `float` / `int` を返すメソッドの `_impl` はハンドルを返さない（4-1）。ハンドルとして読むと
    ///   整数レジスタのごみ・生の整数をハンドルとして扱う（3-2 で `float * None` になったのがこれ）。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_direct_method_call_typed(
        &mut self,
        sym: &str,
        cls: &str,
        object: &Expr,
        oh: &str,
        ot: Ty,
        args: &[CallArg],
        arg_exprs: &[(String, Ty)],
    ) -> (String, Ty) {
        // self_h is prepended to the explicit operands (already in the callee's internal ABI).
        let outer = self.take_call_save();
        let (oh, explicit) = self.method_call_operands(sym, object, oh, ot, args, arg_exprs);
        let save = self.finish_call_save(outer);
        let all_params = std::iter::once(format!("i64 {oh}"))
            .chain(explicit).collect::<Vec<_>>().join(", ");
        let ret = self.fn_sigs[sym].ret;
        // 番地を受け取る変種（7-19）へ渡すのは、受け手がこの関数の `self` の今の値そのもので（`mut self` なので写さない）、
        // 呼ばれる側が `mut self` のメソッドのとき。同じクラスの `self.m(..)` だけ（Arrow のクラスは継承しないので、
        // `self` のクラスは呼ばれる側のクラス）
        let pre = (!self.typed_mode
            && !self.blk_mode
            && ident_name(object) == Some("self")
            && self.fn_sigs[sym].param_mutabilities.first().copied() == Some(true)
            && self.fresh_var_of(&oh).as_deref() == Some("%_al_self"))
            .then(|| self.pre_lists.get(sym).cloned())
            .flatten()
            .filter(|l| !l.is_empty());
        let ctx = self.enter_callee_ctx(cls, &oh);
        let out = self.call_impl(sym, &all_params, ret, save, pre.as_deref());
        self.leave_callee_ctx(ctx);
        out
    }

    /// 注釈で決めた受け手のメソッド呼び出し（3-2・D-2）: ガード（`CB_METHOD_GUARD`）を通ったときだけ直接呼び、
    /// 外れたら名前で呼ぶ。両方の道の結果は entry の alloca に置いて合流する（道の中でブロックが増えても正しい・
    /// mem2reg が phi にする）。引数と受け手は分岐の前に 1 回だけ評価してある。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_guarded_method_call(
        &mut self,
        sym: &str,
        cls: &str,
        key: &str,
        object: &Expr,
        oh: &str,
        ot: Ty,
        args: &[CallArg],
        arg_exprs: &[(String, Ty)],
        site: CallSite<'_>,
        discarded: bool,
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
        // 値を使わない呼び出し（式の文）なら、名前による呼び出しが積む値（引数の箱・結果）を道の中で巻き戻す（7-17）
        let region = discarded.then(|| self.begin_fallback_region()).flatten();
        let rn = self.emit_method_call_by_name(key, object, oh, args, arg_exprs, site);
        self.ec(&format!("store i64 {rn}, ptr {slot}"));
        self.end_fallback_region(region);
        self.br(&b_join);
        self.start_block(&b_join);
        let r = self.fresh_reg();
        self.ec(&format!("{r} = load i64, ptr {slot}"));
        r
    }

    /// [`Self::emit_guarded_method_call`] の、`int` / `float` を返すメソッドの版（copy_alloc_speed_plan.md 7-5）: 両方の道の結果を
    /// **生の値**（`i64` / `double`）で合流する。以前はハンドルで合流していたので、当たりの道でも `_impl` の `double` を
    /// 箱に詰め直し、結果を使う演算も型の無い道（`binop` のコールバック）になっていた（`e += b.kinetic()` で 1 回 4 コールバック）。
    /// 外れた道（名前による呼び出し）の結果は `CB_EXPECT_RET` で宣言の型どおりに開け、違う型なら `TypeError`
    /// （黙って変換しない・利用者の決定 2026-11-17・旧 known_problems.md #1）。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_guarded_method_call_typed(
        &mut self,
        sym: &str,
        cls: &str,
        key: &str,
        object: &Expr,
        oh: &str,
        ot: Ty,
        args: &[CallArg],
        arg_exprs: &[(String, Ty)],
        site: CallSite<'_>,
    ) -> (String, Ty) {
        let ret = self.fn_sigs[sym].ret;
        let t = llvm_ty(ret);
        let need_mut = self.fn_sigs[sym].param_mutabilities.first().copied().unwrap_or(true);
        let cell = self.ic_cell();
        let cptr = self.str_const(cls.as_bytes());
        let ok = self.call_cb(CB_METHOD_GUARD, &[
            format!("i64 {oh}"), cptr, format!("i32 {}", cls.len()), cell, format!("i32 {}", i32::from(need_mut)),
        ]);
        let slot = format!("%_mres{}", self.reg);
        self.reg += 1;
        self.ea(&format!("{slot} = alloca {t}, align 8"));
        let cond = self.fresh_reg();
        self.ec(&format!("{cond} = icmp ne i32 {ok}, 0"));
        let (b_direct, b_name, b_join) = (self.fresh_blk(), self.fresh_blk(), self.fresh_blk());
        self.ec(&format!("br i1 {cond}, label %{b_direct}, label %{b_name}"));
        self.terminated = true;
        self.start_block(&b_direct);
        let (rd, rt) = self.emit_direct_method_call_typed(sym, cls, object, oh, ot, args, arg_exprs);
        let rd = if ret == Ty::Float { self.to_f64(&rd, rt) } else { self.to_i64(&rd, rt) };
        self.ec(&format!("store {t} {rd}, ptr {slot}"));
        self.br(&b_join);
        self.start_block(&b_name);
        // 結果は宣言の型どおりに開けた生の数なので、名前による呼び出しが積む値は道の中で巻き戻す（7-17）
        let region = self.begin_fallback_region();
        let rn = self.emit_method_call_by_name(key, object, oh, args, arg_exprs, site);
        let shown = format!("{cls}.{key}");
        let name = self.str_const(shown.as_bytes());
        let kind = i32::from(ret == Ty::Float);
        let bits = self.call_cb(CB_EXPECT_RET, &[format!("i64 {rn}"), format!("i32 {kind}"), name, format!("i32 {}", shown.len())]);
        let v = if ret == Ty::Float {
            let f = self.fresh_reg();
            self.ec(&format!("{f} = bitcast i64 {bits} to double"));
            f
        } else {
            bits
        };
        self.ec(&format!("store {t} {v}, ptr {slot}"));
        self.end_fallback_region(region);
        self.br(&b_join);
        self.start_block(&b_join);
        let r = self.fresh_reg();
        self.ec(&format!("{r} = load {t}, ptr {slot}"));
        (r, ret)
    }

    /// 名前によるメソッド呼び出し（`CB_CALL_METHOD_IC`・外のメソッド・コンパイルしていないメソッド・ガードが外れたとき）。
    /// 呼ばれる側の仮引数の `let` / `mut` はここでは分からないので、渡す値が `mut` かを `mut_mask` で渡し、
    /// 複製は `ar_call_method_ic` が呼ばれる側の宣言を見て行う（D-8・1-5）。箇所ごとのセル（名前の検査を 1 回にする）と、
    /// 受け手が外部言語だったときの戻り値の検査の情報（`ffi_call_info`）も渡す（11-3・D-12）。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_method_call_by_name(
        &mut self,
        key: &str,
        object: &Expr,
        oh: &str,
        args: &[CallArg],
        arg_exprs: &[(String, Ty)],
        site: CallSite<'_>,
    ) -> String {
        let method_ptr = self.str_const(key.as_bytes());
        let method_len = key.len() as i32;
        let mask = self.method_mut_mask(object, args);
        let handles: Vec<String> = arg_exprs.iter()
            .map(|(v, vt)| self.to_handle(v, *vt))
            .collect();
        let (args_arr, n) = if handles.is_empty() {
            ("ptr null".to_string(), 0)
        } else {
            let n = handles.len();
            (format!("ptr {}", self.spill_handle_array(&handles, "margs")), n)
        };
        // `Trait::attr` の形（`TraitAccess`）は VM も表示情報を記録しない（外部言語の受け手にならない）
        let info = (!key.contains("::"))
            .then(|| crate::partial_compiler::llvm_codegen::ffi_call_info(self.annotations, site.0, site.1, object, key))
            .flatten();
        let (info_ptr, info_len) = match info {
            Some(i) => (self.str_const(i.as_bytes()), i.len()),
            None => ("ptr null".to_string(), 0),
        };
        let cell = self.ic_cell();
        self.emit_fresh_mask(&handles);
        self.call_cb(CB_CALL_METHOD_IC, &[
            cell, format!("i64 {oh}"), method_ptr, format!("i32 {method_len}"),
            args_arr, format!("i32 {n}"), format!("i32 {mask}"), info_ptr, format!("i32 {info_len}"),
        ])
    }
}
