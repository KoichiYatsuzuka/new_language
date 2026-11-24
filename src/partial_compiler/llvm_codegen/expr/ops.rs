// llvm_codegen/expr/ops.rs — 二項演算・論理演算の生成と型の特殊化。

use super::*;

impl<'a> GenCtx<'a> {
    pub(in crate::partial_compiler::llvm_codegen) fn gen_binop(&mut self, op: &BinOp, left: &Expr, right: &Expr) -> (String, Ty) {
        // Short-circuit and/or
        match op {
            BinOp::And | BinOp::Or => {
                let (l, lt) = self.gen_expr(left);
                // 左辺がネイティブの真偽値（`i1`）なら、そのまま分岐に使う（4-3）
                if lt == Ty::Bool {
                    return self.gen_bool_logic(matches!(op, BinOp::And), &l, right);
                }
                return self.gen_logic_handle(matches!(op, BinOp::And), &l, lt, right);
            }
            _ => {}
        }

        let (l, lt) = self.gen_expr(left);
        let (r, rt) = self.gen_expr(right);
        self.specialize_binop(op, &l, lt, &r, rt)
    }

    /// 左辺が `i1` の `and` / `or`（4-3）。短絡の順は変えない（`and` は左辺が偽なら右辺を評価しない・`or` は真なら）。
    /// 結果は Python と同じく「決めた側の値」で、右辺も `i1` なら `i1`（`CB_IS_TRUTHY` の往復なし）、
    /// 右辺がほかの型ならハンドル（左辺の `i1` は `select` でハンドルにする）。
    /// ⚠ 以前は左辺を箱に入れて `CB_IS_TRUTHY` で開け、結果もハンドルで返していた（`if a < b and c < d:` で往復 3 回）。
    fn gen_bool_logic(&mut self, is_and: bool, l: &str, right: &Expr) -> (String, Ty) {
        let rblk = self.fresh_blk();
        let mblk = self.fresh_blk();
        let tag = if is_and { "and" } else { "or" };
        let slot_b = format!("%_{tag}_b{}", self.blk);
        let slot_h = format!("%_{tag}_h{}", self.blk);
        self.blk += 1;
        self.ea(&format!("{slot_b} = alloca i1, align 1"));
        self.ea(&format!("{slot_h} = alloca i64, align 8"));
        self.ec(&format!("store i1 {l}, ptr {slot_b}"));
        let lh = self.to_handle(l, Ty::Bool);
        self.ec(&format!("store i64 {lh}, ptr {slot_h}"));
        if is_and {
            self.br_cond(l, &rblk, &mblk);
        } else {
            self.br_cond(l, &mblk, &rblk);
        }
        self.start_block(&rblk);
        let (r, rt) = self.gen_expr(right);
        if rt == Ty::Bool {
            self.ec(&format!("store i1 {r}, ptr {slot_b}"));
        } else {
            let rh = self.to_handle(&r, rt);
            self.ec(&format!("store i64 {rh}, ptr {slot_h}"));
        }
        self.br(&mblk);
        self.start_block(&mblk);
        let res = self.fresh_reg();
        if rt == Ty::Bool {
            self.ec(&format!("{res} = load i1, ptr {slot_b}"));
            (res, Ty::Bool)
        } else {
            self.ec(&format!("{res} = load i64, ptr {slot_h}"));
            (res, Ty::Handle)
        }
    }

    /// 左辺が `i1` でない `and` / `or`: 左辺をハンドルにして `CB_IS_TRUTHY` で判定し、結果はハンドル（今までどおり）。
    fn gen_logic_handle(&mut self, is_and: bool, l: &str, lt: Ty, right: &Expr) -> (String, Ty) {
        match is_and {
            true => {
                let lh   = self.to_handle(l, lt);
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
                (result, Ty::Handle)
            }
            false => {
                let lh   = self.to_handle(l, lt);
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
                (result, Ty::Handle)
            }
        }
    }

    pub(in crate::partial_compiler::llvm_codegen) fn specialize_binop(&mut self, op: &BinOp, l: &str, lt: Ty, r: &str, rt: Ty) -> (String, Ty) {
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
            // ⚠ CPython の `_float_div_mod` と同じ手順（`fmod` で余りを出し、符号を割る数へ合わせる）。
            //   以前は `floor(a/b)` / `a - floor(a/b)*b` で、`a/b` の丸めを拾って端でずれていた
            //   （`0.1 % 0.01` が `0.0`・`-1.0 % inf` が `nan`）。インタープリタの `ops::py_float_div_mod` と同じ式。
            // ⚠ ゼロ除算は検査しない（int の `@_tl_idiv` と同じ・ネイティブは例外を投げる経路を持たない）。
            (BinOp::FloorDiv, Ty::Float) => {
                let res = self.fresh_reg();
                self.ec(&format!("{res} = call double @_tl_ffloordiv(double {l}, double {r})"));
                (res, Ty::Float)
            }
            (BinOp::Mod, Ty::Float) => {
                let res = self.fresh_reg();
                self.ec(&format!("{res} = call double @_tl_fmod(double {l}, double {r})"));
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
}
