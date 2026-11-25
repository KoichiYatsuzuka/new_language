// llvm_codegen/expr/builtins.rs — 組み込み関数・数学関数・名前による呼び出しの生成。

use super::*;

impl<'a> GenCtx<'a> {
    /// `func` が組み込み関数 `name` そのものか（5-2）: 同名のローカルもモジュールの最上位の名前も無い識別子。
    /// 1-7 以降、生成コードはモジュール自身の大域で走るので、組み込みを隠しうるのはこの 2 つだけ。
    pub(in crate::partial_compiler::llvm_codegen) fn plain_builtin(&self, func: &Expr, name: &str) -> bool {
        ident_name(func) == Some(name) && !self.locals.contains_key(name) && !self.top_names.contains(name)
    }

    /// 数学の組み込み関数（`abs` / `sqrt` / `cbrt` / `exp` / `exp2` / `exp10` / `log` / `log2` / `log10`）の 1 引数の呼び出しを、
    /// 引数が `int` / `float` / `bool` と分かるときだけ LLVM の命令（intrinsic・libm）へ直接落とす。
    /// 当てはまらなければ（`None`）何も評価せず、呼ぶ側が今までの道（`CB_CALL_BUILTIN`）へ進む。
    ///
    /// - 名前が組み込みそのものであること（同名のローカル・最上位の名前があれば対象外・[`Self::plain_builtin`]）。
    /// - 引数が `Ty::Handle`（型が分からない）のときは、評価済みの引数をそのまま `CB_CALL_BUILTIN` へ渡す
    ///   （typed モードでは typed 変種を破棄する）。引数は 1 度しか評価しない。
    /// - 誤りはインタープリタ（`py_sqrt` / `py_exp_log`）と同じ: 定義域（`sqrt` の負数・`log` の 0 以下）は `ValueError`、
    ///   `exp` 系の溢れは `OverflowError`。インライン化した道の外れ側は、typed モードなら ErrSlot へ書いて `ret i32 1`、
    ///   そうでなければ同じ引数で組み込み関数を呼び直して**インタープリタに例外を作らせる**（文言を 1 か所に保つ）。
    /// - `abs`: `int` は `llvm.abs.i64`（`i64::MIN` はそのまま・`wrapping_abs` と同じ）、`float` は `llvm.fabs.f64`。
    /// - 底つきの 2 引数形（`exp(x, base)` / `log(x, base)`）は対象外（今までの道）。
    pub(super) fn gen_math_builtin(&mut self, func: &Expr, args: &[CallArg]) -> Option<(String, Ty)> {
        const MATH_BUILTINS: &[&str] = &["abs", "sqrt", "cbrt", "exp", "exp2", "exp10", "log", "log2", "log10"];
        let [CallArg::Positional(a)] = args else { return None };
        let name = *MATH_BUILTINS.iter().find(|n| self.plain_builtin(func, n))?;
        let (v, vt) = self.gen_expr(a);
        if name == "abs" {
            match vt {
                Ty::Int => {
                    let r = self.fresh_reg();
                    self.ec(&format!("{r} = call i64 @llvm.abs.i64(i64 {v}, i1 false)"));
                    return Some((r, Ty::Int));
                }
                Ty::Float => {
                    let r = self.fresh_reg();
                    self.ec(&format!("{r} = call double @llvm.fabs.f64(double {v})"));
                    return Some((r, Ty::Float));
                }
                _ => {}
            }
        } else if matches!(vt, Ty::Int | Ty::Float | Ty::Bool) {
            let x = self.to_f64(&v, vt);
            return Some((self.emit_math_float(name, &x), Ty::Float));
        }
        // 型が分からない引数: 今までの道（評価済みの値を渡す）
        if self.typed_mode {
            self.typed_failed = true;
            return Some(("0".to_string(), Ty::Handle));
        }
        let h = self.to_handle(&v, vt);
        Some((self.emit_builtin_call(name, &[h]), Ty::Handle))
    }

    /// [`Self::gen_math_builtin`] の本体: `double` の引数 `x` から `name` の結果（`double`）を出す。
    fn emit_math_float(&mut self, name: &str, x: &str) -> String {
        const INF: &str = "0x7FF0000000000000";
        // 計算は先に行う（intrinsic は定義域の外でも NaN / inf を返すだけで未定義動作にならない）。
        let r = self.fresh_reg();
        let (callee, bad): (&str, Option<(&str, &str, &str)>) = match name {
            "sqrt" => ("llvm.sqrt.f64", Some(("olt", "ValueError", "math domain error"))),
            "log" => ("llvm.log.f64", Some(("ole", "ValueError", "math domain error"))),
            "log2" => ("llvm.log2.f64", Some(("ole", "ValueError", "math domain error"))),
            "log10" => ("llvm.log10.f64", Some(("ole", "ValueError", "math domain error"))),
            "exp" => ("llvm.exp.f64", Some(("ovf", "OverflowError", "math range error"))),
            "exp2" => ("llvm.exp2.f64", Some(("ovf", "OverflowError", "math range error"))),
            // `cbrt` は負の数にも実数で定まり、誤りは無い
            "cbrt" => ("cbrt", None),
            // `exp10(x)` は `10 ** x`（インタープリタの `powf` と同じ）
            _ => ("llvm.pow.f64", Some(("ovf", "OverflowError", "math range error"))),
        };
        if name == "exp10" {
            self.ec(&format!("{r} = call double @llvm.pow.f64(double 1.0e+1, double {x})"));
        } else {
            self.ec(&format!("{r} = call double @{callee}(double {x})"));
        }
        let Some((kind, exc, msg)) = bad else { return r };
        let bad_c = self.fresh_reg();
        if kind == "ovf" {
            // 結果が +inf で、入力が +inf でない（有限の入力で溢れた）。NaN は結果も NaN なのでここに来ない。
            let c1 = self.fresh_reg();
            let c2 = self.fresh_reg();
            self.ec(&format!("{c1} = fcmp oeq double {r}, {INF}"));
            self.ec(&format!("{c2} = fcmp une double {x}, {INF}"));
            self.ec(&format!("{bad_c} = and i1 {c1}, {c2}"));
        } else {
            // `sqrt` は負（-0.0 と NaN は誤りでない）、`log` は 0 以下（NaN は誤りでない）
            self.ec(&format!("{bad_c} = fcmp {kind} double {x}, 0.0"));
        }
        let slot = format!("%_math{}", self.reg);
        self.reg += 1;
        self.ea(&format!("{slot} = alloca double, align 8"));
        let ok_blk = self.fresh_blk();
        let bad_blk = self.fresh_blk();
        let merge = self.fresh_blk();
        self.br_cond(&bad_c, &bad_blk, &ok_blk);
        self.start_block(&ok_blk);
        self.ec(&format!("store double {r}, ptr {slot}"));
        self.br(&merge);
        self.start_block(&bad_blk);
        if self.typed_mode {
            self.emit_typed_error_return(exc, msg);
        } else {
            // インタープリタに同じ引数で呼び直させ、例外（保留中の誤り）を作らせる。結果は使われない。
            let h = self.to_handle(x, Ty::Float);
            self.emit_builtin_call(name, &[h]);
            self.ec(&format!("store double 0.0, ptr {slot}"));
            self.br(&merge);
        }
        self.start_block(&merge);
        let out = self.fresh_reg();
        self.ec(&format!("{out} = load double, ptr {slot}"));
        out
    }

    /// 組み込みの型・関数を**値として引いて**呼ぶ（`CB_GET_GLOBAL` ＋ `CB_CALL_FN`・評価済みの引数で）。
    /// 5-2 の速い道に載らない型の引数のときの、今までの道。
    pub(in crate::partial_compiler::llvm_codegen) fn emit_value_call_by_name(&mut self, name: &str, arg_exprs: &[(String, Ty)]) -> String {
        let ptr = self.str_const(name.as_bytes());
        let fn_h = self.call_cb(CB_GET_GLOBAL, &[ptr, format!("i32 {}", name.len())]);
        let handles: Vec<String> = arg_exprs.iter().map(|(v, vt)| self.to_handle(v, *vt)).collect();
        if handles.is_empty() {
            self.call_cb(CB_CALL_FN, &[format!("i64 {fn_h}"), "ptr null".to_string(), "i32 0".to_string()])
        } else {
            let n = handles.len();
            let arr = self.spill_handle_array(&handles, "vargs");
            self.call_cb(CB_CALL_FN, &[format!("i64 {fn_h}"), format!("ptr {arr}"), format!("i32 {n}")])
        }
    }

    /// VM の `CallBuiltin` と同じ入口（`CB_CALL_BUILTIN`）で、評価済みのハンドルを渡して組み込み関数を呼ぶ（1-3）。
    pub(in crate::partial_compiler::llvm_codegen) fn emit_builtin_call(&mut self, name: &str, handles: &[String]) -> String {
        let ptr = self.str_const(name.as_bytes());
        let len = name.len() as i32;
        if handles.is_empty() {
            self.call_cb(CB_CALL_BUILTIN, &[ptr, format!("i32 {len}"), "ptr null".to_string(), "i32 0".to_string()])
        } else {
            let n   = handles.len();
            let arr = self.spill_handle_array(handles, "bargs");
            self.call_cb(CB_CALL_BUILTIN, &[ptr, format!("i32 {len}"), format!("ptr {arr}"), format!("i32 {n}")])
        }
    }
}
