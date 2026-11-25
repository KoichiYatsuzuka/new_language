// llvm_codegen/expr/direct_calls.rs — 直接呼び出しの実引数・呼び出し命令・文脈の切り替え。

use super::*;

impl<'a> GenCtx<'a> {
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
    pub(in crate::partial_compiler::llvm_codegen) fn build_fast_call_args(&mut self, sym: &str, object: &Expr, args: &[CallArg]) -> Option<String> {
        let sig = self.fn_sigs.get(sym)?;
        let (flat, muts, tys) = (sig.fast_flat.clone(), sig.param_mutabilities.clone(), sig.param_tys.clone());
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
                    parts.push(self.abi_operand(&v, vt, tys[i], op, muts[i], i == 0, sym, false));
                }
            }
        }
        Some(parts.join(", "))
    }

    /// モジュール内のメソッド `@{sym}_impl` へ直接渡すハンドル列（[0] = 受け手）。
    /// 呼ばれる側が `let` で受ける仮引数・`let self` には、`mut` の値を複製して渡す（D-8・1-5）。
    /// ⚠ 以前はどちらの直接呼び出しも複製せず、`mut` の値を `let` で受けたメソッドが返した値が
    /// 呼び出し元のその後の書き換えで一緒に変わった（§0.3 ⑤）。
    pub(super) fn method_call_operands(
        &mut self,
        sym: &str,
        object: &Expr,
        oh: &str,
        ot: Ty,
        args: &[CallArg],
        arg_exprs: &[(String, Ty)],
    ) -> (String, Vec<String>) {
        // `sym` はモジュール内の関数（`is_module_fn`）なので `fn_sigs` に必ずある。
        let muts = self.fn_sigs[sym].param_mutabilities.clone();
        let tys = self.fn_sigs[sym].param_tys.clone();
        // 仮引数より多い引数は `_impl` の型が合わず clang が落とす（ここの選択は結果に効かない）。
        let callee_mut = |i: usize| muts.get(i).copied().unwrap_or(true);
        let recv = self.copy_for_let(oh, ot, object, callee_mut(0), true, sym, false);
        let mut out = Vec::new();
        for (i, ((v, vt), a)) in arg_exprs.iter().zip(args).enumerate() {
            let pty = tys.get(i + 1).copied().unwrap_or(Ty::Handle);
            // 受け手とほかの引数のどれとも重ならない引数（同じ変数の別のフィールド）か（4-6）
            let others: Vec<&Expr> = std::iter::once(object)
                .chain(args.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, x)| x.expr()))
                .collect();
            let disjoint = self.disjoint_from_all(a.expr(), &others);
            out.push(self.abi_operand(v, *vt, pty, a.expr(), callee_mut(i + 1), false, sym, disjoint));
        }
        (recv, out)
    }

    /// 引数 1 つを呼ばれる側の内部 ABI（4-1・D-3）に合わせた被演算子（`i64 %x` / `double %y`）にする。
    /// `int` / `float` の仮引数は生値（型検査が同じ型しか通さない）、ほかはハンドルで、`let` で受ける `mut` の値は
    /// 複製する（D-8・1-5）。⚠ 以前は全部ハンドルに箱詰めし、呼ばれた側が入口で開けていた。
    #[allow(clippy::too_many_arguments)]
    fn abi_operand(&mut self, v: &str, vt: Ty, pty: Ty, operand: &Expr, callee_mutable: bool, is_self: bool, callee: &str, disjoint: bool) -> String {
        match pty {
            Ty::Int => format!("i64 {}", self.to_i64(v, vt)),
            Ty::Float => format!("double {}", self.to_f64(v, vt)),
            _ => {
                let h = self.to_handle(v, vt);
                format!("i64 {}", self.copy_for_let(&h, vt, operand, callee_mutable, is_self, callee, disjoint))
            }
        }
    }

    /// `@{sym}_impl` を内部 ABI（4-1）で呼び、結果をその型のまま返す: `float` → `double`、`int` → 生の i64、
    /// ほか → ハンドル。呼ばれた側がハンドルを作りうる（`pushing_fns`）か、被演算子を複製した（`save` がある）ときだけ、
    /// アリーナを保存点まで巻き戻す（1-15・9-2・copy_alloc_speed_plan.md 7-12）。
    /// `save`: 被演算子の複製の前に取った保存点（`finish_call_save`）。あればそこまで巻き戻して複製も捨てる。
    ///
    /// 値を積まない関数（`pushing_fns` に無い・`lcg` のような数値だけの関数や、フィールドを書いて `None` を返す
    /// メソッド）は、複製が無ければ前後で何もしない。ハンドルを返す場合も、呼ばれた側が何も積まないので、返るのは
    /// 呼ぶ前からある欄（引数・`None` など）で、積み直さずにそのまま使える。
    /// ⚠ 7-12 より前は `int` / ハンドルを返す呼び出しが、呼ばれた側が何も積まなくても毎回 `ar_arena_save` /
    ///   `ar_arena_compact` を払っていた（コンパイル済みの k1 で 1 体の更新に 3 組・うち 1 組は値を積まないメソッドの呼び出し）。
    /// ⚠ 「積まない」は呼ばれた側の本体が積む箇所（`push_sites`）を出さないこと。積む箇所に数えないコールバックが
    ///   走らせた利用者のコードの入れ子のネイティブ呼び出しは、その呼び出しが抜けるときに自分で巻き戻す（known_bugs.md #25）。
    ///
    /// `pre`: 番地を受け取る変種（`@{sym}_pre`・copy_alloc_speed_plan.md 7-19）へ渡す `self` の入れ子の受け手の並び。
    /// 呼ぶ直前に、この関数が覚えている同じ（属性, クラス）の置き場を読んで渡す（覚えていなければ空）。
    pub(super) fn call_impl(&mut self, sym: &str, params: &str, ret: Ty, save: Option<CallSave>, pre: Option<&[(String, String)]>) -> (String, Ty) {
        let save = self.save_before_call(sym, save);
        // 置き場は呼ぶ直前に読む: 間にほかの呼び出しがあれば `finish_block_cache` がその後で空にしているので、読んだ値は
        // 呼ぶ時点で正しい（空なら呼ばれる側が取り直す）
        let (callee, params) = match pre {
            Some(list) => {
                let mut ps = params.to_string();
                for (a, c) in list {
                    let slot = self
                        .ap_vars
                        .get(&("%_al_self".to_string(), a.clone()))
                        .filter(|(_, sc)| sc == c)
                        .map(|(slot, _)| slot.clone());
                    let v = match slot {
                        Some(slot) => {
                            let p = self.fresh_reg();
                            self.ec(&format!("{p} = load ptr, ptr {slot}"));
                            p
                        }
                        None => "null".to_string(),
                    };
                    ps.push_str(&format!(", ptr {v}"));
                }
                (format!("@{sym}_pre"), ps)
            }
            None => (format!("@{sym}_impl"), params.to_string()),
        };
        match ret {
            Ty::Float | Ty::Int => {
                let r = self.fresh_reg();
                let t = if ret == Ty::Float { "double" } else { "i64" };
                self.ec(&format!("{r} = call {t} {callee}({params})"));
                // 生値なので残すハンドルは無い（TL_NONE を渡して巻き戻すだけ）
                self.rewind_after_call(save, None);
                // 呼ばれた側が例外を積んだらすぐ戻る（巻き戻しの後・known_bugs.md #28）
                self.check_exc_after_call(sym);
                (r, ret)
            }
            _ => {
                let raw = self.fresh_reg();
                self.ec(&format!("{raw} = call i64 {callee}({params})"));
                let h = self.rewind_after_call(save, Some(&raw));
                self.check_exc_after_call(sym);
                (h, Ty::Handle)
            }
        }
    }

    /// モジュール内の関数を `_impl` で直接呼ぶ（内部 ABI・4-1）。結果はその型のまま返す。
    /// `let` 仮引数は、渡す値が `mut` のときだけ `copy_value`（`__copy__` があればそれ）で複製する（D-8・6-2・
    /// メソッドの 1-5 と同じ `abi_operand`）。⚠ 以前は `let` 仮引数を引数が `let` でも毎回 `deep_copy_value` で
    /// 複製しており、余計に遅いうえ、`__copy__` を持つクラスで VM（`copy_value`）と違う写しを作っていた。
    pub(super) fn emit_module_fn_call(&mut self, name: &str, args: &[CallArg], arg_exprs: &[(String, Ty)]) -> (String, Ty) {
        let sig = &self.fn_sigs[name];
        let (muts, tys, ret) = (sig.param_mutabilities.clone(), sig.param_tys.clone(), sig.ret);
        let outer = self.take_call_save();
        let ops: Vec<String> = arg_exprs.iter().zip(args).enumerate()
            .map(|(i, ((v, vt), a))| {
                let pty = tys.get(i).copied().unwrap_or(Ty::Handle);
                let others: Vec<&Expr> =
                    args.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, x)| x.expr()).collect();
                let disjoint = self.disjoint_from_all(a.expr(), &others);
                self.abi_operand(v, *vt, pty, a.expr(), muts.get(i).copied().unwrap_or(true), false, name, disjoint)
            })
            .collect();
        let save = self.finish_call_save(outer);
        self.call_impl(name, &ops.join(", "), ret, save, None)
    }

    /// `name` が**このモジュールでコンパイルされた関数**か（＝`@{name}_impl` を直接呼べるか）。
    ///
    /// ⚠ `locals` を必ず除く。同名のローカル変数があるとそちらが優先されるので、
    /// 直接呼び出しに落とすと**別のものを呼ぶ**。
    /// ⚠ #64 以前はこの 2 条件が `gen_call_expr` と `gen_call` に**計 4 箇所**書かれていた。
    fn is_module_fn(&self, name: &str) -> bool {
        self.module_fns.contains(name) && !self.locals.contains_key(name)
    }

    /// `sym` をモジュール内の直接呼び出し（`_impl` / `_fast`）で呼べるか（1-13・§0.3 ⑰）: モジュールの関数で、
    /// 書かれた引数の数が仮引数の数（メソッドは受け手を含む・`with_self`）とちょうど同じこと。
    /// 既定値を省いた呼び出しは直接呼ばず、名前による呼び出しへ回す（宣言どおりの束縛が既定値を埋める・1-11）。
    /// ⚠ 以前は書かれた数のまま `_impl` を呼び、足りない引数がごみになっていた（`fdef(10)` が 7 でなく 10）。
    pub(super) fn direct_callee(&self, sym: &str, n_args: usize, with_self: bool) -> bool {
        self.is_module_fn(sym)
            && self.fn_sigs.get(sym).is_some_and(|s| s.param_tys.len() == n_args + usize::from(with_self))
    }

    /// モジュール内のメソッドを `_impl` で直接呼ぶ前に、呼ばれるメソッドのクラスの文脈へ入る（1-9）。
    /// 呼ぶ側（今コンパイル中の関数のクラス・自由関数なら無し）と同じクラスなら何もしない（`self.m()` の大半）。
    /// 違えば `CB_ENTER_METHOD_CTX` で `current_class` を受け手のクラスにし、その印を返す。
    /// ⚠ `_fast` 変種はコールバックを呼ばない（アクセスの検査が無い）ので包まない。
    pub(super) fn enter_callee_ctx(&mut self, callee_class: &str, receiver_h: &str) -> Option<String> {
        if self.current_class.as_deref() == Some(callee_class) {
            return None;
        }
        Some(self.call_cb(CB_ENTER_METHOD_CTX, &[format!("i64 {receiver_h}")]))
    }

    /// [`Self::enter_callee_ctx`] の後始末（呼び出しの直後・エラーがあっても必ず通る）。
    pub(super) fn leave_callee_ctx(&mut self, token: Option<String>) {
        if let Some(t) = token {
            self.call_cb(CB_LEAVE_METHOD_CTX, &[format!("i64 {t}")]);
        }
    }
}
