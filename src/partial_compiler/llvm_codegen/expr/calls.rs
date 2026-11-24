// llvm_codegen/expr/calls.rs — 呼び出し式の生成（関数・クラスの生成・ハンドルの呼び出し）。

use super::*;

impl<'a> GenCtx<'a> {
    /// ハンドル ABI（[`Self::gen_call`]）で呼んでから、戻り値型に応じて**開けて返す**（#64）。
    ///
    /// `Ty::Float` はここへ来ない（呼び出し側が `double` 直返しの高速経路を選ぶ）。
    /// ⚠ #64 以前は関数版とメソッド版に逐語で同じものが書かれていた。
    fn gen_call_unwrapped(&mut self, func: &Expr, args: &[CallArg], ret_ty: Ty, site: CallSite<'_>) -> (String, Ty) {
        let h = self.gen_call(func, args, site);
        match ret_ty {
            Ty::Int => {
                let r = self.call_cb(CB_TO_INT, &[format!("i64 {h}")]);
                (r, Ty::Int)
            }
            _ => (h, Ty::Handle),
        }
    }

    /// `Class(args)` を、数値の引数を箱に入れずに作ったばかりのインスタンスの番地へ直接書いて作る（copy_alloc_speed_plan.md
    /// 7-11）。出せなければ何も出さずに `None`（呼ぶ側が今の `CB_CONSTRUCT_IC` を使う）。
    ///
    /// 出せるのは、`class` がこのモジュールの raw レイアウトのクラスで全フィールドが public、`__init__` がパーサの合成したもの
    /// （`StaticRawClass::auto_init`）、引数（位置引数・評価済みの `arg_exprs`）の数がその仮引数と同じで、どの引数も格納先の
    /// フィールドに同じ種類の数として入るとき（[`raw_store_conv`]。`int` を `float` のフィールドへ渡す形は型検査が止めるので
    /// 出さない）。出すコード:
    /// 1. `CB_NEW_INSTANCE`: インスタンスを作ってアリーナに積み、先頭の塊の番地を返す。クラスが表のとおりでない・合成した
    ///    `__init__` の格納の順が違うときは 0（何も作らない）
    /// 2. 作れたら、引数を幅に合わせて（切り詰め・丸め）番地へ書き、初期化の印をまとめて立てる。`store_auto_init` の格納
    ///    （束縛は数なのでそのまま・格納の検査は作ったばかりのインスタンスなので通る）と同じ結果
    /// 3. 作れなければ、引数を箱に入れて今までの `CB_CONSTRUCT_IC` を呼ぶ（**同じセル**を渡す。結果もエラーも今までと同じ）
    /// ⚠ 引数は呼ぶ前に評価してある（評価の順は今までと同じ）。番地は 2 の間だけ使う（間にコールバックを挟まない）。
    fn emit_direct_construct(&mut self, class: &str, arg_exprs: &[(String, Ty)], mask: u32) -> Option<String> {
        use crate::interpreter::value::block;
        if self.terminated {
            return None;
        }
        let rc = self.raw_classes.get(class).filter(|rc| rc.all_public)?;
        let order = rc.auto_init.as_ref()?;
        if order.is_empty() || order.len() != arg_exprs.len() {
            return None;
        }
        let mut stores = Vec::with_capacity(order.len());
        for (&idx, (v, vt)) in order.iter().zip(arg_exprs) {
            let desc = rc.layout.fields[idx];
            let (ity, conv) = crate::partial_compiler::llvm_codegen::context::raw_store_conv(desc.width, *vt)?;
            stores.push((desc, ity, conv, v.clone()));
        }
        let init_bits = order.iter().fold(0u64, |b, &i| b | block::init_bit(i));
        let n = order.len();
        let table = self.direct_class_desc(class)?;
        let ord = format!("{table}_ord");
        // セルは今までの道（`CB_CONSTRUCT_IC`）と共有する（どちらもクラスの番号 + 1 を入れる）。照合のセルは別（確かめた
        // クラスを覚える）
        let cell = self.ic_cell();
        let checked = self.ic_cell();
        let name = self.str_const(class.as_bytes());
        let len = class.len();
        let out = format!("%_nio{}", self.reg);
        self.reg += 1;
        self.ea(&format!("{out} = alloca ptr, align 8"));
        let h = self.call_cb(CB_NEW_INSTANCE, &[
            cell.clone(), checked, name.clone(), format!("i32 {len}"), table, ord, format!("i32 {n}"), format!("ptr {out}"),
        ]);
        let made = self.fresh_reg();
        self.ec(&format!("{made} = icmp ne i64 {h}, 0"));
        let (bfast, bslow, bdone) = (self.fresh_blk(), self.fresh_blk(), self.fresh_blk());
        self.br_cond(&made, &bfast, &bslow);

        self.start_block(&bfast);
        let p = self.fresh_reg();
        self.ec(&format!("{p} = load ptr, ptr {out}, align 8"));
        for (desc, ity, conv, v) in &stores {
            let stored = match conv {
                None => v.clone(),
                Some(op) => {
                    let c = self.fresh_reg();
                    self.ec(&format!("{c} = {op} {v} to {ity}"));
                    c
                }
            };
            let fp = self.fresh_reg();
            self.ec(&format!("{fp} = getelementptr inbounds i8, ptr {p}, i64 {}", block::field_offset(desc.byte_offset)));
            self.ec(&format!("store {ity} {stored}, ptr {fp}, align {}", desc.width.byte_width()));
        }
        let head = self.fresh_reg();
        self.ec(&format!("{head} = load i64, ptr {p}, align 8"));
        let head2 = self.fresh_reg();
        // LLVM の整数の定数は符号つきの 10 進で書く
        self.ec(&format!("{head2} = or i64 {head}, {}", init_bits as i64));
        self.ec(&format!("store i64 {head2}, ptr {p}, align 8"));
        self.br(&bdone);

        self.start_block(&bslow);
        let handles: Vec<String> = arg_exprs.iter().map(|(v, vt)| self.to_handle(v, *vt)).collect();
        let arr = self.spill_handle_array(&handles, "kargs");
        self.emit_fresh_mask(&handles);
        let h2 = self.call_cb(CB_CONSTRUCT_IC, &[
            cell, name, format!("i32 {len}"), format!("ptr {arr}"), format!("i32 {n}"), format!("i32 {mask}"),
        ]);
        self.br(&bdone);

        self.start_block(&bdone);
        let r = self.fresh_reg();
        self.ec(&format!("{r} = phi i64 [ {h}, %{bfast} ], [ {h2}, %{bslow} ]"));
        // どちらの道も作りたての値を新しい欄に積む（`CB_CONSTRUCT_IC` と同じ・12-1）
        self.fresh_regs.insert(r.clone());
        Some(r)
    }

    /// 引数ハンドルを **entry ブロックの `alloca` 配列**へ積んでポインタ名を返す（#64）。
    ///
    /// ⚠ `alloca` は **entry ブロック**へ出す（`ea`）。ループ本体に置くと反復ごとに
    /// スタックが伸びる。⚠ #64 以前は `gen_call` に**逐語で 3 回**書かれていた
    /// （違ったのは生成 IR 側の配列名の接頭辞 margs / targs / cargs だけ）。
    pub(super) fn spill_handle_array(&mut self, handles: &[String], tag: &str) -> String {
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
    /// | アリーナ | 呼ばれた側が積むとき・被演算子を複製したときだけ `CB_ARENA_SAVE` → `CB_ARENA_COMPACT` を挟む（9-2・7-12・どちらも `call_impl`） | 同左（7-12 より前は必ず挟んでいた） |
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
    pub(super) fn gen_call_expr(&mut self, func: &Expr, args: &[CallArg], site: CallSite<'_>) -> (String, Ty) {
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
        // ── 数学の組み込み関数（abs / sqrt / cbrt / exp / log …）: 数値の引数なら LLVM の命令へ直接落とす ──
        // ⚠ typed モードより前に置く（コールバックを要さないので、これらを呼んでも typed 変種は破棄されない）。
        if let Some(r) = self.gen_math_builtin(func, args) {
            return r;
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
        // ── 引数の型が分かる組み込み関数（5-2・D-4） ──────────────────
        if let [CallArg::Positional(a)] = args {
            if self.plain_builtin(func, "float") {
                let (v, vt) = self.gen_expr(a);
                return match vt {
                    Ty::Int => {
                        let r = self.fresh_reg();
                        self.ec(&format!("{r} = sitofp i64 {v} to double"));
                        (r, Ty::Float)
                    }
                    Ty::Float => (v, Ty::Float),
                    _ => (self.emit_value_call_by_name("float", &[(v, vt)]), Ty::Handle),
                };
            }
            if self.plain_builtin(func, "len") {
                let (v, vt) = self.gen_expr(a);
                let h = self.to_handle(&v, vt);
                return (self.call_cb(CB_LEN, &[format!("i64 {h}")]), Ty::Int);
            }
            if self.plain_builtin(func, "str") {
                let (v, vt) = self.gen_expr(a);
                if vt == Ty::Int {
                    return (self.call_cb(CB_STR_OF_INT, &[format!("i64 {v}")]), Ty::Handle);
                }
                return (self.emit_value_call_by_name("str", &[(v, vt)]), Ty::Handle);
            }
        }
        // ── Typed intra-module direct function calls ──────────────────
        if let Some(name) = ident_name(func) {
            if self.direct_callee(name, args.len(), false) {
                // 内部 ABI（4-1）: `int` / `float` は生値で渡し、生値で受ける（箱詰めしない）。
                let name = name.to_string();
                let arg_exprs: Vec<(String, Ty)> = args.iter()
                    .map(|a| self.gen_expr(a.expr())).collect();
                return self.emit_module_fn_call(&name, args, &arg_exprs);
            }
        }
        // ── 番地で呼ぶ変種（7-14）: 受け手と引数の番地が確かめられたときだけ。外れたら下の今までの呼び出し ──
        if let Expr::Attr { object, attr, .. } = func {
            if let Some(r) = self.emit_blk_method_call(func, object, attr, args, site) {
                return r;
            }
        }
        if let Some(r) = self.gen_method_call_expr(func, args, site) {
            return r;
        }
        (self.gen_call(func, args, site), Ty::Handle)
    }

    /// `obj.m(args)` を、このモジュールのメソッドの直接呼び出しで呼ぶ（受け手のクラスが分かり、呼べるメソッドのとき）。
    /// 当てはまらなければ何も出さずに `None`（呼ぶ側がハンドル ABI の [`Self::gen_call`] を使う）。
    pub(super) fn gen_method_call_expr(&mut self, func: &Expr, args: &[CallArg], site: CallSite<'_>) -> Option<(String, Ty)> {
        // ── Typed intra-module method calls on known class instances ──
        if let Expr::Attr { object, attr, .. } = func {
            let receiver = self.receiver_class(object);
            if let Some(Receiver { class: cls, guarded }) = &receiver {
                let sym = method_symbol(cls, attr);
                if self.direct_callee(&sym, args.len(), true) {
                    let ret_ty = self.fn_sigs.get(&sym).map(|s| s.ret).unwrap_or(Ty::Handle);

                    // ガード付き（注釈で決めた受け手）は生値を返す道を使わず、ハンドル ABI の
                    // ガード付き呼び出し（`gen_call`）で呼ぶ（外れたときの名前による呼び出しはハンドルを返す）。
                    if matches!(ret_ty, Ty::Float | Ty::Int) && !*guarded {
                        // Preferred: _fast variant — passes pre-read field values as
                        // scalars; zero callbacks and LLVM-inlineable pure arithmetic.
                        // 破棄された `_fast` 変種は選ばない（生成側と条件を揃える）。
                        if ret_ty == Ty::Float
                            && self.fast_fns.contains(&sym)
                            && !self.discarded_fast.contains(&sym)
                        {
                            let outer = self.take_call_save();
                            let fast_args = self.build_fast_call_args(&sym, object, args);
                            let save = self.finish_call_save(outer);
                            if let Some(fast_args) = fast_args {
                                let r = self.fresh_reg();
                                // 呼ばれた側がハンドルを作りうるなら前後で巻き戻す（9-2・`call_impl` と同じ）
                                let save = self.save_before_call(&sym, save);
                                self.ec(&format!("{r} = call double @{sym}_fast({fast_args})"));
                                // 被演算子を複製したとき（1-15・7-12）・呼ばれた側が積むとき（9-2）に巻き戻す
                                self.rewind_after_call(save, None);
                                // `_fast` の本体が例外を出しうるなら確かめる（`_impl` とは別に決める・#28）
                                self.check_exc_after_call(&fast_fail_key(&sym));
                                return Some((r, Ty::Float));
                            }
                            // `build_fast_call_args` は被演算子を作る前にしか `None` を返さない（作った複製は無い）
                            debug_assert!(save.is_none());
                        }
                        // _impl を内部 ABI で呼び、`double` / 生の i64 をそのまま受ける（4-1）
                        let arg_exprs: Vec<(String, Ty)> = args.iter()
                            .map(|a| self.gen_expr(a.expr())).collect();
                        let (ov, ot) = self.gen_expr(object);
                        let oh = self.to_handle(&ov, ot);
                        let cls = cls.clone();
                        return Some(self.emit_direct_method_call_typed(&sym, &cls, object, &oh, ot, args, &arg_exprs));
                    }

                    // ガード付きの受け手で `int` / `float` を返すメソッド: 当たりの道の生の値のまま合流する（7-5）
                    if matches!(ret_ty, Ty::Float | Ty::Int) {
                        let arg_exprs: Vec<(String, Ty)> = args.iter()
                            .map(|a| self.gen_expr(a.expr())).collect();
                        let (ov, ot) = self.gen_expr(object);
                        let oh = self.to_handle(&ov, ot);
                        let cls = cls.clone();
                        return Some(self.emit_guarded_method_call_typed(&sym, &cls, attr, object, &oh, ot, args, &arg_exprs, site));
                    }

                    // ハンドルを返すメソッド: ハンドル ABI。
                    return Some(self.gen_call_unwrapped(func, args, ret_ty, site));
                }
            }
        }
        None
    }

    pub(in crate::partial_compiler::llvm_codegen) fn gen_call(&mut self, func: &Expr, args: &[CallArg], site: CallSite<'_>) -> String {
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
                if self.direct_callee(&sym, args.len(), true) {
                    if !guarded {
                        return self.emit_direct_method_call(&sym, &cls, object, &oh, ot, args, &arg_exprs);
                    }
                    let discarded = self.call_value_discarded(func);
                    return self.emit_guarded_method_call(&sym, &cls, &key, object, &oh, ot, args, &arg_exprs, site, discarded);
                }
            }
            return self.emit_method_call_by_name(&key, object, &oh, args, &arg_exprs, site);
        }

        // Intra-module direct call（内部 ABI・4-1。ここは文として呼んだときなどハンドルが要る所）
        if let Some(name) = ident_name(func) {
            if self.direct_callee(name, args.len(), false) {
                let name = name.to_string();
                let (v, t) = self.emit_module_fn_call(&name, args, &arg_exprs);
                return self.to_handle(&v, t); // always return a handle; gen_expr unwraps for typed callees
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
                let handles: Vec<String> = arg_exprs.iter()
                    .map(|(v, vt)| self.to_handle(v, *vt))
                    .collect();
                let name = name.clone();
                return self.emit_builtin_call(&name, &handles);
            }
        }

        // モジュール内のクラスの生成（5-1・D-4）: 値として引かず、VM の `Call` と同じ `instantiate_evaled` を直接呼ぶ。
        // ローカルに同名があればそちら（`is_module_fn` と同じ理由）。キーワード引数は今までどおり汎用の道。
        if let Some(name) = ident_name(func) {
            if self.all_class_fields.contains_key(name)
                && !self.locals.contains_key(name)
                && args.iter().all(|a| matches!(a, CallArg::Positional(_)))
            {
                let mask = self.method_mut_mask_args(args);
                // 全フィールドが数値のクラスを合成した `__init__` で作るなら、数値の引数を番地へ直接書く（7-11）
                if let Some(r) = self.emit_direct_construct(name, &arg_exprs, mask) {
                    return r;
                }
                let ptr = self.str_const(name.as_bytes());
                let len = name.len() as i32;
                let handles: Vec<String> = arg_exprs.iter()
                    .map(|(v, vt)| self.to_handle(v, *vt))
                    .collect();
                // 呼び出し箇所ごとのセルにクラスの番号を覚えさせ、2 回目からは名前で引かない（10-2・`ar_construct_ic`）
                let cell = self.ic_cell();
                return if handles.is_empty() {
                    self.call_cb(CB_CONSTRUCT_IC, &[cell, ptr, format!("i32 {len}"), "ptr null".to_string(), "i32 0".to_string(), format!("i32 {mask}")])
                } else {
                    let n   = handles.len();
                    let arr = self.spill_handle_array(&handles, "kargs");
                    self.emit_fresh_mask(&handles);
                    self.call_cb(CB_CONSTRUCT_IC, &[cell, ptr, format!("i32 {len}"), format!("ptr {arr}"), format!("i32 {n}"), format!("i32 {mask}")])
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
}

/// 呼び出し式の node-id と位置（FFI の検査の情報・`ffi_call_info`・11-3）。
pub(in crate::partial_compiler::llvm_codegen) type CallSite<'s> = (u32, &'s crate::token::Span);
