// llvm_codegen/context.rs — GenCtx の低レベル IR 出力補助: コンストラクタ、レジスタ/ブロック採番、命令/alloca 出力、型変換(handle/i64/f64/cond)、CB 呼び出し、文字列定数など。

use std::collections::{HashMap, HashSet};
use crate::ast::{CallArg, Expr};
use super::*;

impl<'a> GenCtx<'a> {
    pub(super) fn new(
        module_fns:        &'a HashSet<String>,
        fn_sigs:           &'a HashMap<String, FnSig>,
        class_fields:      &'a HashMap<String, HashMap<String, Ty>>,
        class_fields_ord:  &'a HashMap<String, Vec<(String, Ty)>>,
        all_class_fields:  &'a HashMap<String, Vec<(String, String)>>,
        fast_fns:          &'a HashSet<String>,
        annotations:       &'a crate::type_check::AstAnnotations,
    ) -> Self {
        Self {
            str_globals: String::new(),
            fn_defs:     String::new(),
            alloca_buf:  String::new(),
            code_buf:    String::new(),
            reg: 0, blk: 0, terminated: false,
            locals:          HashMap::new(),
            loop_stack:      Vec::new(),
            iter_scopes:     Vec::new(),
            module_fns,
            fn_sigs,
            str_consts:      HashMap::new(),
            str_ctr:         0,
            ic_ctr:          0,
            pending_save:    None,
            push_sites:      0,
            pushing_fns:     HashSet::new(),
            fn_pushes:       HashSet::new(),
            handle_store_log: Vec::new(),
            top_names:       HashSet::new(),
            block_stack:     Vec::new(),
            class_fields,
            class_fields_ord,
            all_class_fields,
            current_class:   None,
            param_classes:   HashMap::new(),
            current_fn_ret:  Ty::Handle,
            fast_fns,
            preread_fields:        HashMap::new(),
            binding_mut:           HashMap::new(),
            fresh_regs:            HashSet::new(),
            flat_list_params:      HashMap::new(),
            fn_param_trampolines:  HashMap::new(),
            typed_mode:            false,
            typed_failed:          false,
            typed_ok:              HashSet::new(),
            typed_sigs:            HashMap::new(),
            annotations,
            name_to_slot:          HashMap::new(),
            locals_by_slot:        Vec::new(),
            slot_reads:            0,
            fast_mode:             false,
            fast_failed:           false,
            fast_flattened:        HashSet::new(),
            discarded_fast:        HashSet::new(),
            attr_stats:            AttrResolutionStats::default(),
            expr_stats:            HandleFallbackStats::default(),
            ident_stats:           IdentHandleStats::default(),
            raw_classes:           HashMap::new(),
            direct_descs:          HashMap::new(),
            ip_vars:               HashMap::new(),
        }
    }

    /// 引数・受け手の式が `mut` の値か（VM の `arg_is_mutable` と同じ規則・D-8・native_object_access_plan.md 1-5）。
    /// `let` で束縛された名前だけが `false`（共有）。表に無い名前（グローバル等）・属性・一時値は保守的に
    /// `true`（複製する。結果は変わらず、複製が多いだけ）。
    /// ⚠ VM はモジュール最上位の `let` も共有する（`toplevel_mut`）が、ここは最上位の表を持たないので複製する。
    pub(super) fn operand_is_mutable(&self, e: &Expr) -> bool {
        // VM の `arg_is_mutable` と同じ: 型検査が不変と証明した経路は複製しない（copy_alloc_speed_plan.md 3-3）
        if e.node_id().is_some_and(|id| self.annotations.is_immutable_path(id)) {
            return false;
        }
        match e {
            Expr::Ident { name, .. } => self.binding_mut.get(name.as_str()).copied().unwrap_or(true),
            _ => true,
        }
    }

    /// 名前によるメソッド呼び出しの `mut_mask`（`CB_CALL_METHOD_M`）: bit 0 = 受け手、bit i+1 = 引数 i が `mut` の値か。
    /// ⚠ 31 個目より後の引数は保守的に `mut`（呼ばれる側の `ar_call_method_m` も同じ解釈）。
    pub(super) fn method_mut_mask(&self, object: &Expr, args: &[CallArg]) -> u32 {
        let mut mask = u32::from(self.operand_is_mutable(object));
        for (i, a) in args.iter().enumerate().take(31) {
            if self.operand_is_mutable(a.expr()) {
                mask |= 1 << (i + 1);
            }
        }
        mask
    }

    /// 引数のハンドルのうち「作りたての値を積んだばかりの欄」（`fresh_regs`）の印（bit i+1 = 引数 i・2-4）。0 でなければ
    /// 直後の `CB_CONSTRUCT_IC` / `CB_CALL_METHOD_IC` の前に `CB_SET_FRESH` で渡す（コールバックは欄から値を取り出す）。
    /// ⚠ 印を付けたレジスタは、その呼び出しの引数としてしか使わないこと（欄は空になる）。
    pub(super) fn emit_fresh_mask(&mut self, handles: &[String]) {
        let mut mask = 0u32;
        for (i, h) in handles.iter().enumerate().take(31) {
            if self.fresh_regs.contains(h) {
                mask |= 1 << (i + 1);
            }
        }
        if mask != 0 {
            self.call_cb(CB_SET_FRESH, &[format!("i32 {mask}")]);
        }
    }

    /// 受け手の無い呼び出し（生成・5-1）の `mut_mask`: bit i+1 = 引数 i が `mut` の値か（bit 0 は使わない）。
    pub(super) fn method_mut_mask_args(&self, args: &[CallArg]) -> u32 {
        let mut mask = 0u32;
        for (i, a) in args.iter().enumerate().take(31) {
            if self.operand_is_mutable(a.expr()) {
                mask |= 1 << (i + 1);
            }
        }
        mask
    }

    /// 呼ばれた側が `let` で受ける値を、渡す値が `mut` のときだけ複製する（D-8・1-5）。
    /// `is_self`: `let self` は `deep_copy_value`（VM も `self` には `__copy__` を呼ばない）、仮引数は `copy_value`
    /// （`__copy__` があればそれ）。`int` / `float` の生値・`mut` で受ける側・`let` の値はそのまま渡す。
    pub(super) fn copy_for_let(
        &mut self,
        h: &str,
        vt: Ty,
        operand: &Expr,
        callee_mutable: bool,
        is_self: bool,
        callee: &str,
        disjoint: bool,
    ) -> String {
        if callee_mutable || vt != Ty::Handle || !self.operand_is_mutable(operand) {
            return h.to_string();
        }
        // 複製の前に保存点を取り、呼び出しの後で巻き戻す（1-15）。⚠ 取らないと複製がいちばん外の呼び出しが
        // 終わるまでアリーナに残り、ループの中の呼び出しでは周の数だけ溜まった（`let self` の深いコピーで 1 GB 超）。
        if self.pending_save.is_none() {
            self.pending_save = Some(self.call_cb(CB_ARENA_SAVE, &[]));
        }
        // 作りたての値を積んだばかりの欄（`fresh_regs`）は、誰とも共有していなければ複製しない（2-4・`ar_copy_fresh`）
        if self.fresh_regs.contains(h) {
            return self.call_cb(CB_COPY_FRESH, &[format!("i64 {h}"), format!("i32 {}", i32::from(is_self))]);
        }
        // 呼ぶ関数が読むだけなら複製しない（4-4・`ar_borrow_or_copy`）。読むだけかは実行時に、プログラム全体の
        // 判定（`Interpreter::readonly_fns`）を呼ぶ関数の名前で引く（箇所ごとのセルに覚える）。そうでなければ
        // コールバックの中で今までの複製（`let self` は `deep_copy_value`・仮引数は `copy_value`）をする。
        let name = self.callee_fn_name(callee).to_string();
        let name_ptr = self.str_const(name.as_bytes());
        let cell = self.ic_cell();
        // 種類: 1 = `let self`・2 = 受け手やほかの引数と重ならない `let` 仮引数（4-6）・0 = ほかの `let` 仮引数
        let kind = if is_self { 1 } else if disjoint { 2 } else { 0 };
        self.call_cb(CB_BORROW_OR_COPY, &[
            format!("i64 {h}"),
            format!("i32 {kind}"),
            name_ptr,
            format!("i32 {}", name.len()),
            cell,
        ])
    }

    /// `arg` が、`others`（受け手とほかの引数）のどれとも**重ならない**と言えるか（copy_alloc_speed_plan.md 4-6）。
    /// 言えるのは、`arg` と相手が同じ変数を根にした属性だけの経路で、途中で別のフィールドへ分かれるとき
    /// （`self.pos` と `self.vel`。格納のたびに複製するので、別のフィールドの中身は共有しない）。
    /// 相手が `let` の値（書き換えられない）なら見なくてよい。それ以外（別の変数・添字・呼び出しの結果・片方が
    /// もう片方の中）は「重なりうる」。⚠ 迷ったら `false`（複製する側）。
    pub(super) fn disjoint_from_all(&self, arg: &Expr, others: &[&Expr]) -> bool {
        // 規則は型検査と同じ 1 つ（`type_check::annotations::arg_is_disjoint`）。「書き換えられない値」の判定だけが
        // 生成コードの `operand_is_mutable`
        let mut immutable = |e: &Expr| !self.operand_is_mutable(e);
        crate::type_check::arg_is_disjoint(arg, others, &mut immutable)
    }

    /// 直接呼び出しのシンボル（`{クラス}__{メソッド}` か関数名）から、関数・メソッドの名前を取り出す。
    pub(super) fn callee_fn_name<'s>(&self, sym: &'s str) -> &'s str {
        match sym.split_once("__") {
            Some((class, method)) if self.all_class_fields.contains_key(class) => method,
            _ => sym,
        }
    }

    /// 直接呼び出しの被演算子を組み立てる前に呼ぶ: 外側の呼び出しの保存点を退避する（1-15）。
    pub(super) fn take_call_save(&mut self) -> Option<String> {
        self.pending_save.take()
    }

    /// 被演算子を組み立てた後に呼ぶ: この呼び出しの複製の前に取った保存点（無ければ `None`）を返し、外側の
    /// 呼び出しの保存点を戻す（1-15）。
    pub(super) fn finish_call_save(&mut self, outer: Option<String>) -> Option<String> {
        std::mem::replace(&mut self.pending_save, outer)
    }

    /// 宣言（`let` / `mut`）で束縛する値を VM と同じ規則で複製する（§0.3 ⑥・1-6）。
    /// `mut` は常に深いコピー、`let` は `let_bind`（VM の `compile_let` と共有）の答えに従う。
    /// `int` / `float` / `bool` の生値は複製しても同じなのでそのまま。`const` はここを通らない（VM も複製しない）。
    pub(super) fn bind_decl(&mut self, h: &str, vt: Ty, is_mut: bool, expr: &Expr) -> String {
        use crate::interpreter::BindCopy;
        use crate::vm::compiler::LetBind;
        if vt != Ty::Handle {
            return h.to_string();
        }
        let mode = if is_mut {
            BindCopy::DeepCopy
        } else {
            let local_mut = |e: &Expr| ident_name(e).and_then(|n| self.binding_mut.get(n).copied());
            match crate::vm::compiler::let_bind(expr, local_mut) {
                LetBind::Share => return h.to_string(),
                LetBind::CopyFreezeName | LetBind::CopyFreezeExpr => BindCopy::CopyFreeze,
                LetBind::FromName(n) => {
                    let p = self.str_const(n.as_bytes());
                    return self.call_cb(CB_LET_FROM_NAME, &[p, format!("i32 {}", n.len()), format!("i64 {h}")]);
                }
            }
        };
        // 作りたての値を積んだばかりの欄なら、共有していないときに複製を省いてよい（12-1・`ar_bind_value`）
        let fresh = if self.fresh_regs.contains(h) { crate::interpreter::BindCopy::FRESH_BIT } else { 0 };
        self.call_cb(CB_BIND_VALUE, &[format!("i32 {}", mode as i32 | fresh), format!("i64 {h}")])
    }

    /// Look up the native type of a field access on a known class instance.
    /// Returns Some(Ty) only if the field is typed int/float in the class definition.
    ///
    /// **自前の型再導出**（#16 段階(c) で置換対象）。受け手が `self` か、型注釈から
    /// クラスが判るパラメータのときしか解決できない。
    pub(super) fn field_ty(&self, object: &Expr, attr: &str) -> Option<Ty> {
        let class_name = match ident_name(object) {
            Some("self") => self.current_class.as_deref(),
            Some(n) => self.param_classes.get(n).map(|s| s.as_str()),
            None => None,
        }?;
        self.class_fields.get(class_name)?.get(attr).copied()
    }

    /// AST 型解決層の注釈（#16）から属性アクセスの型を引く。
    ///
    /// 型検査は `Expr::Attr` の node-id に**フィールドの宣言型**を焼いている
    /// （受け手が `NamedInstance` と解決できたとき・`infer_attr`）。`field_ty` と違い
    /// 受け手の式の形を問わないため、局所変数や入れ子属性（`a.b.c`）でも解決できる。
    ///
    /// `Ty::Int` / `Ty::Float` に対応する型のみ返す（`field_ty` と同じ判定粒度に揃える）。
    /// node_id==0（未採番＝合成 AST・テンプレート置換）や未注釈は `None`。
    pub(super) fn field_ty_annotated(&self, node_id: u32) -> Option<Ty> {
        use crate::type_check::InferredType;
        match self.annotations.resolved_type(node_id)? {
            InferredType::Int => Some(Ty::Int),
            InferredType::Float => Some(Ty::Float),
            _ => None,
        }
    }

    /// 属性アクセスの型を決める（#16 段階 c-3）。
    ///
    /// **AST 型解決層の注釈を第一の根拠にする**（＝ツリーウォーク／VM／ネイティブが同じ解決を共有する）。
    /// 自前導出 `field_ty` は注釈が無いときのフォールバックとしてのみ残す。
    ///
    /// 実測（代表 6 モジュール）では `legacy_only = 0`・`conflict = 0` で、
    /// **自前導出が注釈より広く解けるケースは 1 件も無い**。それでも撤去せず残しているのは、
    /// node-id が付かない合成 AST 等でゼロコストの保険になるため。
    /// 一致状況は `AR_ANNOT_DIFF=1` で引き続き観測できる。
    pub(super) fn field_ty_resolved(&mut self, object: &Expr, attr: &str, node_id: u32) -> Option<Ty> {
        let legacy = self.field_ty(object, attr);
        let annotated = self.field_ty_annotated(node_id);
        match (legacy, annotated) {
            (Some(a), Some(b)) if a == b => self.attr_stats.agree += 1,
            (Some(_), Some(_)) => self.attr_stats.conflict += 1,
            (Some(_), None) => self.attr_stats.legacy_only += 1,
            (None, Some(_)) => self.attr_stats.annot_only += 1,
            (None, None) => self.attr_stats.neither += 1,
        }
        annotated.or(legacy)
    }

    pub(super) fn fresh_reg(&mut self) -> String { let r = self.reg; self.reg += 1; format!("%_r{r}") }
    pub(super) fn fresh_blk(&mut self) -> String { let b = self.blk; self.blk += 1; format!("_bb{b}") }

    /// Emit a line into the alloca buffer (entry block).
    pub(super) fn ea(&mut self, line: &str) { self.alloca_buf.push_str("  "); self.alloca_buf.push_str(line); self.alloca_buf.push('\n'); }

    /// Emit a line into the code buffer.
    pub(super) fn ec(&mut self, line: &str) {
        if !self.terminated {
            self.code_buf.push_str("  ");
            self.code_buf.push_str(line);
            self.code_buf.push('\n');
        }
    }

    /// Start a new basic block label in the code buffer.
    pub(super) fn start_block(&mut self, label: &str) {
        self.code_buf.push('\n');
        self.code_buf.push_str(label);
        self.code_buf.push_str(":\n");
        self.terminated = false;
    }

    /// Emit an unconditional branch if the current block is not yet terminated.
    pub(super) fn br(&mut self, target: &str) {
        if !self.terminated {
            self.ec(&format!("br label %{target}"));
            self.terminated = true;
        }
    }

    /// Emit a conditional branch.
    pub(super) fn br_cond(&mut self, cond: &str, then_lbl: &str, else_lbl: &str) {
        if !self.terminated {
            self.ec(&format!("br i1 {cond}, label %{then_lbl}, label %{else_lbl}"));
            self.terminated = true;
        }
    }

    /// Emit a ret instruction.
    pub(super) fn ret_handle(&mut self, val: &str) {
        if !self.terminated {
            self.ec(&format!("ret i64 {val}"));
            self.terminated = true;
        }
    }

    // ── Typed ABI helpers ─────────────────────────────────────────────────────

    /// typed モードの return: 値を関数の戻り値型に合わせて `%_ret` に格納し status 0 を返す。
    /// Handle 値の強制変換はコールバックを要するため typed_failed が立つ（自動検出）。
    pub(super) fn emit_typed_return(&mut self, v: &str, vt: Ty) {
        if self.terminated { return; }
        match self.current_fn_ret {
            Ty::Float => {
                let f = self.to_f64(v, vt);
                self.ec(&format!("store double {f}, ptr %_ret"));
            }
            _ => {
                let iv = self.to_i64(v, vt);
                self.ec(&format!("store i64 {iv}, ptr %_ret"));
            }
        }
        self.ec("ret i32 0");
        self.terminated = true;
    }

    /// typed モードの raise: `raise Name("literal")` / `raise Name()` パターンを
    /// ErrSlot への静的文字列書き込み + `ret i32 1` に展開する。
    /// それ以外のパターン（動的メッセージなど）は typed_failed を立てる。
    pub(super) fn emit_typed_raise(&mut self, exc_expr: &Expr) {
        let (type_name, msg): (String, String) = match exc_expr {
            Expr::Call { func, args, .. } => {
                let Some(name) = ident_name(func.as_ref()) else {
                    self.typed_failed = true;
                    return;
                };
                let msg = match args.first() {
                    None => String::new(),
                    Some(CallArg::Positional(Expr::Str(s))) => s.to_string(),
                    _ => {
                        self.typed_failed = true;
                        return;
                    }
                };
                (name.to_string(), msg)
            }
            e if ident_name(e).is_some() => {
                (ident_name(e).unwrap().to_string(), String::new())
            }
            _ => {
                self.typed_failed = true;
                return;
            }
        };
        if self.terminated { return; }
        let tp = self.str_const(type_name.as_bytes());
        let mp = self.str_const(msg.as_bytes());
        let tlen = type_name.len();
        let mlen = msg.len();
        // ErrSlot layout: +0 type_ptr, +8 type_len, +16 msg_ptr, +24 msg_len
        let e1 = self.fresh_reg();
        let e2 = self.fresh_reg();
        let e3 = self.fresh_reg();
        self.ec(&format!("store {tp}, ptr %_err"));
        self.ec(&format!("{e1} = getelementptr inbounds i8, ptr %_err, i64 8"));
        self.ec(&format!("store i64 {tlen}, ptr {e1}"));
        self.ec(&format!("{e2} = getelementptr inbounds i8, ptr %_err, i64 16"));
        self.ec(&format!("store {mp}, ptr {e2}"));
        self.ec(&format!("{e3} = getelementptr inbounds i8, ptr %_err, i64 24"));
        self.ec(&format!("store i64 {mlen}, ptr {e3}"));
        self.ec("ret i32 1");
        self.terminated = true;
    }

    /// typed モードのモジュール内呼び出し: `@{name}_typed(args*, ret*, err*)` を発行し、
    /// status != 0 なら即 return で呼び出し元へ伝播する（C のエラー伝播と同型）。
    /// `%_err` ポインタは横流しなので、最内の raise 情報がそのまま最外へ届く。
    pub(super) fn gen_typed_call(&mut self, name: &str, args: &[CallArg], ptys: &[Ty], rty: Ty) -> (String, Ty) {
        let n = ptys.len();
        // 呼び出しサイトごとの引数バッファ・戻り値スロット（entry ブロックに alloca）
        let args_al = if n > 0 {
            let a = format!("%_tca{}", self.reg);
            self.reg += 1;
            self.ea(&format!("{a} = alloca [{n} x i64], align 8"));
            a
        } else {
            String::new()
        };
        let ret_al = format!("%_tcr{}", self.reg);
        self.reg += 1;
        self.ea(&format!("{ret_al} = alloca i64, align 8"));

        for (i, (arg, pty)) in args.iter().zip(ptys).enumerate() {
            let (v, vt) = self.gen_expr(arg.expr());
            let slot = self.fresh_reg();
            self.ec(&format!(
                "{slot} = getelementptr inbounds [{n} x i64], ptr {args_al}, i32 0, i32 {i}"
            ));
            match pty {
                Ty::Float => {
                    let f = self.to_f64(&v, vt);
                    self.ec(&format!("store double {f}, ptr {slot}"));
                }
                _ => {
                    let iv = self.to_i64(&v, vt);
                    self.ec(&format!("store i64 {iv}, ptr {slot}"));
                }
            }
        }
        let args_ref = if n > 0 { format!("ptr {args_al}") } else { "ptr null".to_string() };
        let st = self.fresh_reg();
        self.ec(&format!(
            "{st} = call i32 @{name}_typed({args_ref}, ptr {ret_al}, ptr %_err)"
        ));
        let ok = self.fresh_reg();
        let cont = self.fresh_blk();
        let eprop = self.fresh_blk();
        self.ec(&format!("{ok} = icmp eq i32 {st}, 0"));
        self.br_cond(&ok, &cont, &eprop);
        self.start_block(&eprop);
        self.ec(&format!("ret i32 {st}"));
        self.terminated = true;
        self.start_block(&cont);
        let r = self.fresh_reg();
        self.ec(&format!("{r} = load {}, ptr {ret_al}", llvm_ty(rty)));
        (r, rty)
    }

    // ── Alloca helpers ────────────────────────────────────────────────────────

    pub(super) fn alloca_var(&mut self, name: &str, ty: Ty) -> String {
        // ⚠⚠ 同じ名前をもう一度宣言する（2 つ目の `for i in ..`・兄弟のブロックの `let x`）: 型が同じなら同じ場所を
        //   使う（VM も関数ごとに名前 1 つの slot・`collect_nested_decls`）。型が違えば別の名前で取る。
        //   以前は毎回 `%_al_{name}` を出していたので、LLVM が同じ名前の定義を 2 つ持てず clang が落ち、
        //   **モジュール全体が**ネイティブにならなかった（`--compile` は「skipped」と出すだけ・§0.3 ⑫・1-10）。
        let reg = match self.locals.get(name) {
            Some((r, t)) if *t == ty && self.alloca_buf.contains(&format!("  {r} = alloca")) => r.clone(),
            _ => {
                let base = format!("%_al_{name}");
                let mut reg = base.clone();
                let mut k = 1;
                while self.alloca_buf.contains(&format!("  {reg} = alloca")) {
                    reg = format!("{base}.{k}");
                    k += 1;
                }
                let t = llvm_ty(ty);
                self.ea(&format!("{reg} = alloca {t}, align 8"));
                reg
            }
        };
        self.locals.insert(name.to_string(), (reg.clone(), ty));
        // リゾルバが slot を割り当てている名前なら slot 索引側にも載せる（#11 R2-a′）。
        // 合成ローカル（preread の一時変数など）は slot を持たないので名前引きのみ。
        if let Some(&slot) = self.name_to_slot.get(name) {
            if let Some(e) = self.locals_by_slot.get_mut(slot as usize) {
                *e = Some((reg.clone(), ty));
            }
        }
        reg
    }

    /// `Resolution::Local(slot)` の読み取り（#11 R2-a′）。
    /// リゾルバの割り当てた slot で直接引き、未登録なら名前引きへフォールバックする
    /// （リゾルバが解決を諦めた関数・合成ローカル）。
    pub(super) fn load_var_by_slot(&mut self, slot: u32, name: &str) -> (String, Ty) {
        if let Some(Some((ptr, ty))) = self.locals_by_slot.get(slot as usize).cloned() {
            self.slot_reads += 1;
            let t = llvm_ty(ty);
            let r = self.fresh_reg();
            self.ec(&format!("{r} = load {t}, ptr {ptr}"));
            return (r, ty);
        }
        self.load_var(name)
    }

    pub(super) fn store_val(&mut self, ty: Ty, val: &str, ptr: &str) {
        let t = llvm_ty(ty);
        self.ec(&format!("store {t} {val}, ptr {ptr}"));
        if ty == Ty::Handle {
            self.note_handle_store(ptr);
        }
    }

    /// ローカル変数の場所へハンドルを書き込んだことを記録する（9-1・ループの周の終わりに残す値）。
    pub(super) fn note_handle_store(&mut self, ptr: &str) {
        if !self.terminated {
            self.handle_store_log.push(ptr.to_string());
        }
    }

    pub(super) fn load_var(&mut self, name: &str) -> (String, Ty) {
        let (ptr, ty) = self.locals.get(name).cloned()
            .unwrap_or_else(|| ("%_UNDEF".to_string(), Ty::Handle));
        let t = llvm_ty(ty);
        let r = self.fresh_reg();
        self.ec(&format!("{r} = load {t}, ptr {ptr}"));
        (r, ty)
    }

    // ── String constant pool ──────────────────────────────────────────────────

    pub(super) fn str_const(&mut self, bytes: &[u8]) -> String {
        if let Some(name) = self.str_consts.get(bytes) {
            return format!("ptr @{name}");
        }
        let name = format!("_s{}", self.str_ctr);
        self.str_ctr += 1;
        self.str_consts.insert(bytes.to_vec(), name.clone());
        let esc = escape_for_llvm(bytes);
        let len = bytes.len() + 1;
        self.str_globals.push_str(&format!(
            "@{name} = private unnamed_addr constant [{len} x i8] c\"{esc}\\00\", align 1\n"
        ));
        format!("ptr @{name}")
    }

    /// `obj.attr` を**箇所ごとの IC 付きで**読む（native_object_access_plan.md 2-2・D-1）。`ty` が `Float` / `Int` なら
    /// その型で（変換規則は `get_float_field` / `get_int_field` と同じ）、ほかはハンドルで返す。
    /// IC のセルは DLL 内の `i64` の大域（初期値 0）で、命中・ミスの判定は VM の `GetAttr` と同じ関数が行う。
    /// ⚠ 以前は IC を持たず、毎回 `get_attr_val` の名前引き（`format!` と全フィールドの走査を含む）を通っていた。
    pub(super) fn emit_attr_read(&mut self, h: &str, attr: &str, ty: Option<Ty>) -> (String, Ty) {
        let cell = self.ic_cell();
        let ptr = self.str_const(attr.as_bytes());
        let args = [format!("i64 {h}"), ptr, format!("i32 {}", attr.len()), cell];
        match ty {
            Some(Ty::Float) => (self.call_cb(CB_GET_FLOAT_FIELD_IC, &args), Ty::Float),
            Some(Ty::Int) => (self.call_cb(CB_GET_INT_FIELD_IC, &args), Ty::Int),
            _ => (self.call_cb(CB_GET_ATTR_IC, &args), Ty::Handle),
        }
    }

    /// クラスの表（`DirectClassDesc` ＋ フィールドごとの `DirectFieldDesc`）を DLL の定数として出し、`ptr @名前` を返す
    /// （copy_alloc_speed_plan.md 8-3）。クラスごとに 1 回だけ出す。番地を返すコールバックが、実行時のクラスと照合する。
    fn direct_class_desc(&mut self, class: &str) -> Option<String> {
        if let Some(g) = self.direct_descs.get(class) {
            return Some(format!("ptr @{g}"));
        }
        let rc = self.raw_classes.get(class)?.clone();
        let n = rc.fields.len();
        let cname = self.str_const(class.as_bytes());
        let mut items = Vec::with_capacity(n);
        for ((fname, mutable), desc) in rc.fields.iter().zip(&rc.layout.fields) {
            let fp = self.str_const(fname.as_bytes());
            let info = crate::interpreter::value::direct_field_info(*desc, *mutable);
            items.push(format!("{{ ptr, i64, i64 }} {{ {fp}, i64 {}, i64 {info} }}", fname.len()));
        }
        let g = format!("_dcd{}", self.direct_descs.len());
        self.str_globals.push_str(&format!(
            "@{g} = private constant {{ ptr, i64, i64, [{n} x {{ ptr, i64, i64 }}] }} {{ {cname}, i64 {}, i64 {n}, [{n} x {{ ptr, i64, i64 }}] [{}] }}, align 8\n",
            class.len(),
            items.join(", ")
        ));
        self.direct_descs.insert(class.to_string(), g.clone());
        Some(format!("ptr @{g}"))
    }

    /// 数値のフィールド `class.attr` を型 `ty` として番地から直接読めるか（読めるなら slot・位置と幅・読む型と広げ方）。
    /// 読めるのは: 到達するコードで、`class` がこのモジュールの raw レイアウトのクラスで全フィールドが public、フィールドの
    /// 幅が `ty` に合う（整数の幅 → `Int`・`float32` / `float` → `Float`。合わなければ今までの道が変換する）。
    /// ⚠ コードを出す前の判定にも使う（[`Self::emit_direct_read`] はこれが `Some` なら必ず出す）。
    #[allow(clippy::type_complexity)]
    pub(super) fn direct_read_plan(
        &self,
        class: &str,
        attr: &str,
        ty: Ty,
    ) -> Option<(usize, crate::interpreter::value::RawFieldDesc, (&'static str, Option<&'static str>))> {
        use crate::interpreter::value::RawWidth;
        // 終わったブロックの後（到達しないコード）には分岐を出さない
        if self.terminated {
            return None;
        }
        let rc = self.raw_classes.get(class).filter(|rc| rc.all_public)?;
        let idx = rc.fields.iter().position(|(n, _)| n == attr)?;
        let desc = rc.layout.fields[idx];
        // (読む型, 広げ方)。`None` はそのまま
        let conv = match (desc.width, ty) {
            (RawWidth::I64 | RawWidth::U64, Ty::Int) => ("i64", None),
            (RawWidth::I32, Ty::Int) => ("i32", Some("sext")),
            (RawWidth::U32, Ty::Int) => ("i32", Some("zext")),
            (RawWidth::I16, Ty::Int) => ("i16", Some("sext")),
            (RawWidth::U16, Ty::Int) => ("i16", Some("zext")),
            (RawWidth::I8, Ty::Int) => ("i8", Some("sext")),
            (RawWidth::U8, Ty::Int) => ("i8", Some("zext")),
            (RawWidth::F64, Ty::Float) => ("double", None),
            (RawWidth::F32, Ty::Float) => ("float", Some("fpext")),
            _ => return None,
        };
        Some((idx, desc, conv))
    }

    /// `class.attr` が、番地から直接書けるフィールドか（値の型を見る前の判定）: raw レイアウトのクラスで全フィールドが
    /// public、フィールドが `mut`。
    pub(super) fn direct_write_field(&self, class: &str, attr: &str) -> bool {
        !self.terminated
            && self
                .raw_classes
                .get(class)
                .filter(|rc| rc.all_public)
                .is_some_and(|rc| rc.fields.iter().any(|(n, m)| n == attr && *m))
    }

    /// 型 `vt` の値を `class.attr` へ番地から直接書けるか（書けるなら slot・位置と幅・格納する型と値の変換）。
    /// [`Self::direct_write_field`] に加えて、値の型がフィールドに入ること（`InstanceData::store_field` と同じ規則）。
    #[allow(clippy::type_complexity)]
    pub(super) fn direct_write_plan(
        &self,
        class: &str,
        attr: &str,
        vt: Ty,
    ) -> Option<(usize, crate::interpreter::value::RawFieldDesc, (&'static str, Option<&'static str>))> {
        use crate::interpreter::value::RawWidth;
        if !self.direct_write_field(class, attr) {
            return None; // `let` のフィールド・private のフィールドを持つクラスなど
        }
        let rc = self.raw_classes.get(class)?;
        let idx = rc.fields.iter().position(|(n, _)| n == attr)?;
        let desc = rc.layout.fields[idx];
        // (格納する型, 値の変換)。`None` はそのまま
        let conv = match (desc.width, vt) {
            (RawWidth::I64 | RawWidth::U64, Ty::Int) => ("i64", None),
            (RawWidth::I32 | RawWidth::U32, Ty::Int) => ("i32", Some("trunc i64")),
            (RawWidth::I16 | RawWidth::U16, Ty::Int) => ("i16", Some("trunc i64")),
            (RawWidth::I8 | RawWidth::U8, Ty::Int) => ("i8", Some("trunc i64")),
            (RawWidth::F64, Ty::Float) => ("double", None),
            (RawWidth::F32, Ty::Float) => ("float", Some("fptrunc double")),
            // `int` → `float` のフィールドの昇格
            (RawWidth::F64, Ty::Int) => ("double", Some("sitofp i64")),
            (RawWidth::F32, Ty::Int) => ("float", Some("sitofp i64")),
            // 入らない型（`float` を整数のフィールドへ・`bool`・ハンドル）: 今までの道がエラーにする
            _ => return None,
        };
        Some((idx, desc, conv))
    }

    /// 受け手 `src` の先頭の塊の番地を得るコードを出す。戻り値は (番地, 入れ子の受け手なら親の読み出しの IC のセル)。
    /// 入れ子の受け手（[`BlockSrc::Field`]・8-7）は `CB_ATTR_BLOCK` の 1 回で、子をアリーナに積まない。番地は直後の
    /// 読み書きにだけ使う（子を生かしているのは親のフィールドなので、間にコールバックを挟まない）。
    fn emit_block_src(&mut self, class: &str, src: BlockSrc<'_>) -> Option<(String, Option<String>)> {
        match src {
            BlockSrc::Handle(h) => Some((self.emit_block_ptr(class, h)?, None)),
            BlockSrc::Field { parent_h, attr } => {
                let table = self.direct_class_desc(class)?;
                let class_cell = self.ic_cell();
                let ic = self.ic_cell();
                let name = self.str_const(attr.as_bytes());
                let p = self.call_cb(CB_ATTR_BLOCK, &[
                    format!("i64 {parent_h}"), name, format!("i32 {}", attr.len()), ic.clone(), table, class_cell,
                ]);
                Some((p, Some(ic)))
            }
        }
    }

    /// 直接の道から外れたときに使う受け手のハンドル。入れ子の受け手は、ここで初めて今までの読み出し（`CB_GET_ATTR_IC`）を
    /// 呼んでアリーナに積む（`CB_ATTR_BLOCK` と**同じ IC のセル**を渡す。外れた最初の 1 回でセルが埋まり、次から当たる）。
    fn block_src_handle(&mut self, src: BlockSrc<'_>, ic: Option<String>) -> String {
        match src {
            BlockSrc::Handle(h) => h.to_string(),
            BlockSrc::Field { parent_h, attr } => {
                let cell = ic.unwrap_or_else(|| self.ic_cell());
                let name = self.str_const(attr.as_bytes());
                self.call_cb(CB_GET_ATTR_IC, &[format!("i64 {parent_h}"), name, format!("i32 {}", attr.len()), cell])
            }
        }
    }

    /// ハンドル `h` の指すインスタンスの先頭の塊の番地（クラスが `class` の表のとおりでなければ空）を得るコードを出す
    /// （copy_alloc_speed_plan.md 8-3・8-5）。
    ///
    /// `h` がローカル変数から読んだばかりの値なら、**番地を変数ごとに覚えて使い回す**（8-5・D-13 ⑤）: 変数の隣に番地の
    /// 置き場（関数の入口で空）を持ち、空のときだけ `CB_INST_BLOCK` を呼ぶ。そうでなければ毎回呼ぶ。
    ///
    /// 覚えた番地が正しい条件と、その守り方:
    /// - **変数が同じ値を指している間だけ使う。** 変数へ書いたら捨てる。書き込みは 1 つ残らず変数の置き場への `store` なので、
    ///   関数を出力するときに本体の文面を走査し、その `store` の直後へ「番地を捨てる」命令を差し込む
    ///   （[`Self::finish_block_cache`]。書き込みを出す側の関数に印を足す方式だと、足し忘れた 1 か所が解放済みの番地を読む）。
    /// - **`h` が変数の今の値であること。** `h` を読んだ `load` の後に、その変数への `store` が 1 つも出ていないときだけ覚える
    ///   （`x.f = block: x = other ..` のように、受け手を読んだ後で値の評価が変数を差し替える式は覚えない）。
    /// - **番地の指す塊が生きていること。** 変数の欄（アリーナ）がインスタンスを持っている間、塊は動かず解放されない。
    ///   周の巻き戻しは残す変数の欄を積み直して変数へ書き戻す（= `store` なので捨てる）。残さない変数は次に使う前に代入される。
    ///   外部言語の `T*` の書き戻しは、インタープリタが作った別の欄へ書き、結果は値として呼ぶ側へ返る
    ///   （生成コードの変数の欄は差し替えられない・`eval/native.rs`）。
    /// - **1 つの変数に 1 つのクラス。** 同じ変数を別のクラスとして読む箇所は覚えない（毎回呼ぶ）。
    fn emit_block_ptr(&mut self, class: &str, h: &str) -> Option<String> {
        let table = self.direct_class_desc(class)?;
        let cell = self.ic_cell();
        let args = [format!("i64 {h}"), table, cell];
        let Some(al) = self.cacheable_var(h, class) else {
            return Some(self.call_cb(CB_INST_BLOCK, &args));
        };
        let ip = format!("%_ip_{}", &al["%_al_".len()..]);
        if !self.ip_vars.contains_key(&al) {
            self.ea(&format!("{ip} = alloca ptr, align 8"));
            self.ea(&format!("store ptr null, ptr {ip}"));
            self.ip_vars.insert(al, (ip.clone(), class.to_string()));
        }
        let p0 = self.fresh_reg();
        self.ec(&format!("{p0} = load ptr, ptr {ip}"));
        let empty = self.fresh_reg();
        self.ec(&format!("{empty} = icmp eq ptr {p0}, null"));
        let (bfetch, bhave) = (self.fresh_blk(), self.fresh_blk());
        self.br_cond(&empty, &bfetch, &bhave);
        self.start_block(&bfetch);
        let p1 = self.call_cb(CB_INST_BLOCK, &args);
        self.ec(&format!("store ptr {p1}, ptr {ip}"));
        self.br(&bhave);
        self.start_block(&bhave);
        let p = self.fresh_reg();
        self.ec(&format!("{p} = load ptr, ptr {ip}"));
        Some(p)
    }

    /// `h` が「ローカル変数の置き場から読んだばかりで、その後その変数へ書いていない値」なら、その置き場（`%_al_x`）。
    /// 生成済みの本体の文面から決める（[`Self::emit_block_ptr`] の 2 つ目の条件）。別のクラスで覚えた変数は `None`。
    fn cacheable_var(&self, h: &str, class: &str) -> Option<String> {
        let def = format!("  {h} = load i64, ptr ");
        let at = self.code_buf.rfind(&def)?;
        let rest = &self.code_buf[at + def.len()..];
        let (al, after) = rest.split_once('\n')?;
        if !al.starts_with("%_al_") {
            return None;
        }
        let store_tail = format!(", ptr {al}");
        if after.lines().any(|l| l.trim_start().starts_with("store ") && l.ends_with(&store_tail)) {
            return None;
        }
        match self.ip_vars.get(al) {
            Some((_, c)) if c != class => None,
            _ => Some(al.to_string()),
        }
    }

    /// 関数の本体を出力する直前に呼ぶ（8-5）: 番地を覚えた変数の置き場への `store` の直後に、番地の置き場を空にする
    /// 命令を差し込む。変数の置き場が `load` / `store` 以外（呼び出しの引数など）に出てきた行の後にも差し込む
    /// （置き場の番地を外へ渡す所は今は無いが、できたときに古い番地を読まないように）。
    pub(super) fn finish_block_cache(&mut self) {
        if self.ip_vars.is_empty() {
            return;
        }
        let vars: Vec<(String, String)> = self.ip_vars.drain().map(|(al, (ip, _))| (al, ip)).collect();
        let mut out = String::with_capacity(self.code_buf.len() + 64);
        for line in self.code_buf.lines() {
            out.push_str(line);
            out.push('\n');
            let body = line.trim_start();
            for (al, ip) in &vars {
                let writes = if body.starts_with("store ") {
                    line.ends_with(&format!(", ptr {al}"))
                } else if body.contains(" = load ") {
                    false
                } else {
                    // 置き場の名前そのもの（`%_al_x` の後に名前の続きが無い）が出てくる行
                    line.match_indices(al.as_str()).any(|(i, _)| {
                        !line[i + al.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '.')
                    })
                };
                if writes {
                    out.push_str(&format!("  store ptr null, ptr {ip}\n"));
                }
            }
        }
        self.code_buf = out;
    }

    /// `obj.attr`（数値のフィールド）を**番地から直接読む**（copy_alloc_speed_plan.md 8-3・D-9・D-13）。出せなければ
    /// 何も出さずに `None`（呼ぶ側が [`Self::emit_attr_read`] を使う）。
    ///
    /// 出せるのは、受け手のクラス `class` がこのモジュールの raw レイアウトのクラスで、フィールドの幅が読む型 `ty` に合うとき
    /// （整数の幅 → `Int`・`float32` / `float` → `Float`）。出すコード:
    /// 1. 先頭の塊の番地を得る（[`Self::emit_block_ptr`]: `CB_INST_BLOCK`。受け手がローカル変数なら覚えた番地を使い回す）。
    ///    インスタンスのクラスが表のとおりでなければ空（注釈が外れた・private のフィールドがある・別のモジュールの同じ名前のクラス）
    /// 2. 先頭の語の初期化の印を見る
    /// 3. 立っていれば、塊の先頭から静的に決めた位置を、幅どおりに読んで広げる（`InstanceData::field_value` と同じ変換）
    /// 4. 番地が空・未初期化なら今までのコールバック（[`Self::emit_attr_read`]。結果もエラーの文面も今までと同じ）
    ///
    /// ⚠ 位置・幅・印の bit は `interpreter::value::block` と `RawLayout` から取る（並びの定義は 1 か所・8-2）。
    pub(super) fn emit_direct_read(&mut self, class: &str, src: BlockSrc<'_>, attr: &str, ty: Ty) -> Option<(String, Ty)> {
        use crate::interpreter::value::block;
        let (idx, desc, (ity, ext)) = self.direct_read_plan(class, attr, ty)?;
        let offset = block::field_offset(desc.byte_offset);
        let init_bit = block::init_bit(idx);
        let t = llvm_ty(ty);

        let (p, ic) = self.emit_block_src(class, src)?;
        let (bchk, bfast, bslow, bdone) = (self.fresh_blk(), self.fresh_blk(), self.fresh_blk(), self.fresh_blk());
        let isnull = self.fresh_reg();
        self.ec(&format!("{isnull} = icmp eq ptr {p}, null"));
        self.br_cond(&isnull, &bslow, &bchk);

        self.start_block(&bchk);
        let head = self.fresh_reg();
        self.ec(&format!("{head} = load i64, ptr {p}, align 8"));
        let bit = self.fresh_reg();
        self.ec(&format!("{bit} = and i64 {head}, {init_bit}"));
        let inited = self.fresh_reg();
        self.ec(&format!("{inited} = icmp ne i64 {bit}, 0"));
        self.br_cond(&inited, &bfast, &bslow);

        self.start_block(&bfast);
        let fp = self.fresh_reg();
        self.ec(&format!("{fp} = getelementptr inbounds i8, ptr {p}, i64 {offset}"));
        let rawv = self.fresh_reg();
        self.ec(&format!("{rawv} = load {ity}, ptr {fp}, align {}", desc.width.byte_width()));
        let fast = match ext {
            None => rawv,
            Some(op) => {
                let w = self.fresh_reg();
                self.ec(&format!("{w} = {op} {ity} {rawv} to {t}"));
                w
            }
        };
        self.br(&bdone);

        self.start_block(&bslow);
        let h = self.block_src_handle(src, ic);
        let (slow, _) = self.emit_attr_read(&h, attr, Some(ty));
        self.br(&bdone);

        self.start_block(&bdone);
        let r = self.fresh_reg();
        self.ec(&format!("{r} = phi {t} [ {fast}, %{bfast} ], [ {slow}, %{bslow} ]"));
        Some((r, ty))
    }

    /// `obj.attr = v`（数値のフィールド・`v` は `int` / `float` の生値）を**番地へ直接書く**（copy_alloc_speed_plan.md 8-4・
    /// D-13 ③）。出せなければ何も出さずに `false`（呼ぶ側が [`Self::emit_attr_assign_typed`] を使う）。
    ///
    /// 出せるのは、受け手のクラス `class` がこのモジュールの raw レイアウトのクラスで、フィールドが `mut`、値の型がフィールドに
    /// 入るとき（`int` → 整数の幅・`float` → 小数の幅・`int` → 小数の幅は変換する。`InstanceData::store_field` と同じ規則）。
    /// `let` のフィールドと、入らない型（`float` を整数のフィールドへ）は静的に分かるので直接の道を出さない（今までの
    /// コールバックがエラーにする）。出すコード:
    /// 1. `CB_INST_BLOCK` で先頭の塊の番地を得る（クラスが表のとおりでなければ空。表は `mut` かどうかも照合する）
    /// 2. 先頭の語の不変の印を見る。立っていれば今までのコールバック（エラーの文面を保つ）
    /// 3. 値を幅に合わせて（切り詰め・変換）書き、初期化の印を立てる
    /// ⚠ 書き込みの IC が当たったときに省いている検査（定数のクラス変数・`static mut`・setter・アクセスの検査）は、
    ///   クラスが表のとおり（全フィールドが public のふつうのフィールド）と確かめてあるので要らない。
    pub(super) fn emit_direct_write(&mut self, class: &str, src: BlockSrc<'_>, attr: &str, v: &str, vt: Ty) -> bool {
        use crate::interpreter::value::block;
        let Some((idx, desc, (ity, conv))) = self.direct_write_plan(class, attr, vt) else { return false };
        let offset = block::field_offset(desc.byte_offset);
        let init_bit = block::init_bit(idx);
        let Some((p, ic)) = self.emit_block_src(class, src) else { return false };
        let (bchk, bfast, bslow, bdone) = (self.fresh_blk(), self.fresh_blk(), self.fresh_blk(), self.fresh_blk());
        let isnull = self.fresh_reg();
        self.ec(&format!("{isnull} = icmp eq ptr {p}, null"));
        self.br_cond(&isnull, &bslow, &bchk);

        self.start_block(&bchk);
        let head = self.fresh_reg();
        self.ec(&format!("{head} = load i64, ptr {p}, align 8"));
        let imm = self.fresh_reg();
        // LLVM の整数の定数は符号つきの 10 進で書く
        self.ec(&format!("{imm} = and i64 {head}, {}", block::IMMUTABLE as i64));
        let frozen = self.fresh_reg();
        self.ec(&format!("{frozen} = icmp ne i64 {imm}, 0"));
        self.br_cond(&frozen, &bslow, &bfast);

        self.start_block(&bfast);
        let stored = match conv {
            None => v.to_string(),
            Some(op) => {
                let c = self.fresh_reg();
                self.ec(&format!("{c} = {op} {v} to {ity}"));
                c
            }
        };
        let fp = self.fresh_reg();
        self.ec(&format!("{fp} = getelementptr inbounds i8, ptr {p}, i64 {offset}"));
        self.ec(&format!("store {ity} {stored}, ptr {fp}, align {}", desc.width.byte_width()));
        let head2 = self.fresh_reg();
        self.ec(&format!("{head2} = or i64 {head}, {init_bit}"));
        self.ec(&format!("store i64 {head2}, ptr {p}, align 8"));
        self.br(&bdone);

        self.start_block(&bslow);
        let h = self.block_src_handle(src, ic);
        self.emit_attr_assign_typed(&h, attr, v, vt);
        self.br(&bdone);

        self.start_block(&bdone);
        true
    }

    /// `obj.attr = v` を**箇所ごとの書き込みの IC 付きで**行う（2-3・VM の `SetAttr` と同じ `attr_assign_ic`）。
    /// セルは読み出しとは別（`attr_assign_ic` の doc）。
    pub(super) fn emit_attr_assign(&mut self, oh: &str, attr: &str, vh: &str) {
        let cell = self.ic_cell();
        let ptr = self.str_const(attr.as_bytes());
        self.call_cb(CB_ASSIGN_ATTR_IC, &[format!("i64 {oh}"), ptr, format!("i32 {}", attr.len()), format!("i64 {vh}"), cell]);
    }

    /// `obj.attr = v` を値の型のまま書く（4-2）: `int` / `float` の生値は箱に入れずに渡す（fields 65 / 66）、
    /// ほかはハンドル（`emit_attr_assign`）。中身はどれも VM の `SetAttr` と同じ `attr_assign_ic`。
    pub(super) fn emit_attr_assign_typed(&mut self, oh: &str, attr: &str, v: &str, vt: Ty) {
        let (cb, val) = match vt {
            Ty::Int => (CB_ASSIGN_INT_FIELD_IC, format!("i64 {v}")),
            Ty::Float => (CB_ASSIGN_FLOAT_FIELD_IC, format!("double {v}")),
            _ => {
                let vh = self.to_handle(v, vt);
                return self.emit_attr_assign(oh, attr, &vh);
            }
        };
        let cell = self.ic_cell();
        let ptr = self.str_const(attr.as_bytes());
        self.call_cb(cb, &[format!("i64 {oh}"), ptr, format!("i32 {}", attr.len()), val, cell]);
    }

    /// 箇所ごとの IC のセル（DLL 内の `i64` の大域・初期値 0）を 1 つ作り、`ptr @_icN` を返す（2-2 / 2-3 / 3-2）。
    pub(super) fn ic_cell(&mut self) -> String {
        let cell = format!("_ic{}", self.ic_ctr);
        self.ic_ctr += 1;
        self.str_globals.push_str(&format!("@{cell} = internal global i64 0, align 8\n"));
        format!("ptr @{cell}")
    }

    // ── Callback dispatch ─────────────────────────────────────────────────────

    /// Load @CB, GEP to field, load fn ptr, call it. Returns result register (or "void").
    pub(super) fn call_cb(&mut self, field: usize, args: &[String]) -> String {
        // typed モード中はコールバック禁止 — 必要になった時点で typed 変種を破棄する。
        // （typed エントリは TLS・アリーナに一切触れないことが保証されるため）
        if self.typed_mode {
            self.typed_failed = true;
        }
        let (ret_ty, param_tys) = cb_sig(field);
        // アリーナへ積みうるか（9-1）: ハンドルを返すもの。生の整数・保存点・トークンを返すものは積まない
        // 結果を残さない巻き戻し（`int` / `float` を返す呼び出しの後・残す値 0）も積まない（9-2）
        let compact_none = field == CB_ARENA_COMPACT && args.first().is_some_and(|a| a == "i64 0");
        if ret_ty == "i64"
            && !compact_none
            && !matches!(field, CB_TO_INT | CB_ARENA_SAVE | CB_GET_INT_FIELD_IC | CB_LEN | CB_ENTER_METHOD_CTX
                | CB_FLAT_DATA_PTR | CB_FLAT_LEN)
        {
            self.push_sites += 1;
        }
        let cb  = self.fresh_reg();
        let fp  = self.fresh_reg();
        let fn_ = self.fresh_reg();
        self.ec(&format!("{cb} = load ptr, ptr @CB"));
        self.ec(&format!("{fp} = getelementptr inbounds %ArCallbacks, ptr {cb}, i32 0, i32 {field}"));
        self.ec(&format!("{fn_} = load ptr, ptr {fp}"));
        let args_str = args.join(", ");
        let fn_ty = if param_tys.is_empty() {
            format!("{ret_ty} ()")
        } else {
            format!("{ret_ty} ({param_tys})")
        };
        if ret_ty == "void" {
            self.ec(&format!("call {fn_ty} {fn_}({args_str})"));
            "void".to_string()
        } else {
            let r = self.fresh_reg();
            self.ec(&format!("{r} = call {fn_ty} {fn_}({args_str})"));
            // 結果を必ず新しい欄に積むコールバック（`push_handle` で返す・12-1）
            if matches!(field, CB_CONSTRUCT_IC | CB_CONSTRUCT | CB_BINOP | CB_MAKE_LIST | CB_MAKE_TUPLE | CB_MAKE_DICT) {
                self.fresh_regs.insert(r.clone());
            }
            r
        }
    }

    // ── Type coercions ────────────────────────────────────────────────────────

    /// Coerce an (expr_reg, Ty) to an i64 handle.
    pub(super) fn to_handle(&mut self, val: &str, ty: Ty) -> String {
        match ty {
            Ty::Handle => val.to_string(),
            Ty::Int    => self.call_cb(CB_MAKE_INT,   &[format!("i64 {val}")]),
            Ty::Float  => self.call_cb(CB_MAKE_FLOAT, &[format!("double {val}")]),
            Ty::Bool   => {
                let r = self.fresh_reg();
                self.ec(&format!("{r} = select i1 {val}, i64 1, i64 2"));
                r
            }
        }
    }

    /// Coerce an (expr_reg, Ty) to a raw i64.
    pub(super) fn to_i64(&mut self, val: &str, ty: Ty) -> String {
        match ty {
            Ty::Int    => val.to_string(),
            Ty::Float  => { let r = self.fresh_reg(); self.ec(&format!("{r} = fptosi double {val} to i64")); r }
            Ty::Bool   => { let r = self.fresh_reg(); self.ec(&format!("{r} = zext i1 {val} to i64")); r }
            Ty::Handle => self.call_cb(CB_TO_INT, &[format!("i64 {val}")]),
        }
    }

    /// Coerce to a double.
    pub(super) fn to_f64(&mut self, val: &str, ty: Ty) -> String {
        match ty {
            Ty::Float  => val.to_string(),
            Ty::Int    => { let r = self.fresh_reg(); self.ec(&format!("{r} = sitofp i64 {val} to double")); r }
            Ty::Bool   => { let r = self.fresh_reg(); self.ec(&format!("{r} = uitofp i1 {val} to double")); r }
            Ty::Handle => self.call_cb(CB_TO_FLOAT, &[format!("i64 {val}")]),
        }
    }

    /// Coerce to i1 for use as a branch condition.
    pub(super) fn to_cond(&mut self, val: &str, ty: Ty) -> String {
        match ty {
            Ty::Bool   => val.to_string(),
            Ty::Int    => { let r = self.fresh_reg(); self.ec(&format!("{r} = icmp ne i64 {val}, 0")); r }
            Ty::Float  => { let r = self.fresh_reg(); self.ec(&format!("{r} = fcmp une double {val}, 0.0")); r }
            Ty::Handle => {
                let tr = self.call_cb(CB_IS_TRUTHY, &[format!("i64 {val}")]);
                let r  = self.fresh_reg();
                self.ec(&format!("{r} = icmp ne i32 {tr}, 0"));
                r
            }
        }
    }

    // ── Expression generation ─────────────────────────────────────────────────

}

