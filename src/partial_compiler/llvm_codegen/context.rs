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
            module_fns,
            fn_sigs,
            str_consts:      HashMap::new(),
            str_ctr:         0,
            block_stack:     Vec::new(),
            class_fields,
            class_fields_ord,
            all_class_fields,
            current_class:   None,
            param_classes:   HashMap::new(),
            current_fn_ret:  Ty::Handle,
            fast_fns,
            preread_fields:        HashMap::new(),
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
        }
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
        let reg = format!("%_al_{name}");
        let t = llvm_ty(ty);
        self.ea(&format!("{reg} = alloca {t}, align 8"));
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

    // ── Callback dispatch ─────────────────────────────────────────────────────

    /// Load @CB, GEP to field, load fn ptr, call it. Returns result register (or "void").
    pub(super) fn call_cb(&mut self, field: usize, args: &[String]) -> String {
        // typed モード中はコールバック禁止 — 必要になった時点で typed 変種を破棄する。
        // （typed エントリは TLS・アリーナに一切触れないことが保証されるため）
        if self.typed_mode {
            self.typed_failed = true;
        }
        let (ret_ty, param_tys) = cb_sig(field);
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
