// vm/compiler/stmt_assign.rs — **代入文のコンパイル**（#62 で `compile_stmt` から切り出し）。
//
// `x = e` / `x <op>= e` / `obj.attr = e` / `obj[k] = e` / `obj.attr <op>= e` / `obj[k] <op>= e`。
//
// ⚠⚠ **この族に共通する不変条件は「ツリーウォークと同じ評価順・同じ再評価回数」**。
// ツリーウォークは複合代入で `rhs = eval(value)` → `lhs = eval(target)` → 二項演算 →
// `attr_assign(target, ..)` の順に進み、**`object` / `index` を 2 回評価する**。
// 副作用まで一致させるため、VM も原則そのまま 2 回積む。
// 例外は「レシーバが局所 slot で再評価が無害」と分かるときの融合だけ（下記）。
//
// ⚠ 変更したら [compare_bytecode.ps1](../../../compare_bytecode.ps1) で全例題 byte-identical を確認すること。

use crate::ast::{BinOp, Expr};
use crate::vm::op::Op;
use super::*;

impl Compiler {
    /// `x = e`（#10-b）。パラメータ（`mut`）への代入。`let` への代入は型検査が弾く。
    /// 最上位モードでは slot に無い名前は可視グローバルへの代入になる。
    pub(super) fn compile_assign(&mut self, name: &str, value: &Expr) -> Option<()> {
        match self.store_target(name)? {
            StoreTarget::Local(slot) => {
                self.compile_expr(value)?;
                self.emit(Op::StoreLocal(slot));
            }
            StoreTarget::Global(ni, ci) => {
                self.compile_expr(value)?;
                self.emit(Op::StoreGlobal(ni, ci));
            }
            // モジュール本体の代入（#42）。名前でチェーンを探して書く。
            StoreTarget::Name(ni) => {
                self.compile_expr(value)?;
                self.emit(Op::StoreName(ni));
            }
            // `static mut` への代入（#27-d）。共有セルへ直接書く。
            StoreTarget::Static(si) => {
                self.compile_expr(value)?;
                self.emit(Op::StoreStatic(si));
            }
            // セル変数への代入（#27-d 段階 2b）。共有相手からも見える。
            StoreTarget::Cell(i) => {
                self.compile_expr(value)?;
                self.emit(Op::StoreCell(i));
            }
        }
        Some(())
    }

    /// `x <op>= e`。`x = x <op> e` と同じ命令列になる（`StoreLocal` は deep_copy しない）ので、
    /// `Expr::BinOp` と同じ融合＋型特化を通す（#2b）。通さないと複合代入だけが
    /// `LoadLocal; <e>; Bin; StoreLocal` の 4 命令＋汎用ディスパッチに落ちる（実測 1.9x 遅い）。
    ///
    /// ⚠ 融合（`emit_bin_fused_slot`）が効くのは **`StoreTarget::Local` のときだけ**。
    /// 他の 4 種は記憶域が slot ではないので「読み → 右辺 → 型特化 Bin → 書き」の
    /// 同じ形になる（`emit_compound_via_stack` に畳んである）。
    pub(super) fn compile_compound_assign(
        &mut self,
        name: &str,
        op: &BinOp,
        value: &Expr,
        node_id: u32,
    ) -> Option<()> {
        use crate::type_check::BinOperandKind as K;
        match self.store_target(name)? {
            StoreTarget::Local(slot) => {
                let kind = self.specialized_bin_kind_slot(op, node_id, slot, value);
                if !self.emit_bin_fused_slot(slot, kind, value, op) {
                    // 融合できない右辺（属性・添字・呼び出し結果など）でもスタック版の型特化には乗る。
                    self.emit(Op::LoadLocal(slot));
                    self.compile_expr(value)?;
                    match kind {
                        Some(K::Int) => self.emit(Op::IntBinSS(op.clone())),
                        Some(K::Float) => self.emit(Op::FloatBinSS(op.clone())),
                        None => self.emit(Op::Bin(op.clone())),
                    };
                }
                self.emit(Op::StoreLocal(slot));
            }
            // 最上位のグローバルへの複合代入（#10-b）。`x = x <op> e` と同じ命令列。
            // 融合 op（`BinLocalLocal` 等）は slot 前提なので使えないが、注釈由来の
            // スタック版型特化（`IntBinSS`/`FloatBinSS`）はそのまま乗る（#2b と同じ扱い）。
            StoreTarget::Global(ni, ci) => {
                // ⚠ #70: `x <op>= e` は `x = x <op> e` と同じなので、**グローバル融合**が使える。
                // 融合前は `LoadGlobal; Const; IntBinSS; StoreGlobal` の 4 命令で、
                // 同じ `i += 1` が fn の中では 2 命令（`IntBinLC; StoreLocal`）だった。
                if self.try_emit_compound_fused_global(name, value, op, node_id) {
                    self.emit(Op::StoreGlobal(ni, ci));
                } else {
                    self.emit_load_global(name);
                    self.emit_compound_via_stack(op, node_id, value, Op::StoreGlobal(ni, ci))?;
                }
            }
            // モジュール本体への複合代入（#42）。読みは `LoadName`、書きは `StoreName`。
            StoreTarget::Name(ni) => {
                self.emit(Op::LoadName(ni));
                self.emit_compound_via_stack(op, node_id, value, Op::StoreName(ni))?;
            }
            // `static mut` への複合代入（#27-d）。グローバル版と同じ形。
            StoreTarget::Static(si) => {
                self.emit(Op::LoadStatic(si));
                self.emit_compound_via_stack(op, node_id, value, Op::StoreStatic(si))?;
            }
            // セル変数への複合代入（#27-d 段階 2b）。`static` 版と同じ形。
            StoreTarget::Cell(i) => {
                self.emit(Op::LoadCell(i));
                self.emit_compound_via_stack(op, node_id, value, Op::StoreCell(i))?;
            }
        }
        Some(())
    }

    /// 「現在値が既に積まれている」状態から `<右辺>; <型特化 Bin>; <store>` を出す（#62）。
    ///
    /// slot を持たない 4 種の記憶域（グローバル / モジュール名 / `static mut` / セル）で
    /// **逐語で同じ 8 行**が書かれていたのを畳んだもの。出る命令列は畳む前と 1 バイトも変わらない。
    fn emit_compound_via_stack(
        &mut self,
        op: &BinOp,
        node_id: u32,
        value: &Expr,
        store: Op,
    ) -> Option<()> {
        use crate::type_check::BinOperandKind as K;
        let kind = self.annot_binop_kind(node_id);
        self.compile_expr(value)?;
        match kind {
            Some(K::Int) => self.emit(Op::IntBinSS(op.clone())),
            Some(K::Float) => self.emit(Op::FloatBinSS(op.clone())),
            None => self.emit(Op::Bin(op.clone())),
        };
        self.emit(store);
        Some(())
    }

    /// `obj.attr = value` / `obj[k] = value` / `obj::Trait.attr = value`。
    ///
    /// ⚠ **レシーバの種類で絞らない**（#27-c）。`attr_assign_evaled` が
    /// ツリーウォークの `attr_assign` の**唯一の実装**になったので、
    /// `Value::Instance` / `Value::Class` / それ以外のエラーまで一致する。
    /// 以前は `object_is_instance` で絞っていたが、それは 2 実装の差を
    /// 隠すためのもので、型注釈の無いグローバルが bail する原因だった。
    pub(super) fn compile_attr_assign(&mut self, target: &Expr, value: &Expr) -> Option<()> {
        match target {
            // `obj.attr = value`。obj を push → value を push → SetAttr。
            Expr::Attr { object, attr, .. } => {
                self.compile_expr(object)?;
                // #34: 右辺の評価中は obj が 1 つ積まれている。伝えないと
                // `obj.x = 1 + block ->int: … break …` が bail する（実測で見つけた漏れ）。
                self.pending = self.stmt_base.map(|d| d + 1);
                self.compile_expr(value)?;
                let ni = self.add_name(attr);
                self.emit(Op::SetAttr(ni));
            }
            // `obj[i] = value` — tree-walk は value(rhs) を先に評価するので temp に退避して順序を合わせる。
            Expr::Subscript { object, index, .. } => {
                let vtmp = self.alloc_temp()?;
                self.compile_expr(value)?; // value を先に評価
                self.emit(Op::StoreLocal(vtmp));
                self.compile_expr(object)?; // obj
                self.compile_expr(index)?; // key
                self.emit(Op::LoadLocal(vtmp)); // value
                self.emit(Op::SetIndex);
                self.free_temp();
            }
            // `obj::Trait.attr = value`（#27）。`SetAttr` と同じく `[obj, value]` の順で積む。
            Expr::TraitAccess { object, trait_name, attr } => {
                self.compile_expr(object)?;
                self.pending = self.stmt_base.map(|d| d + 1); // 同上（#34）
                self.compile_expr(value)?;
                let ti = self.add_name(trait_name);
                let ai = self.add_name(attr);
                self.emit(Op::SetTraitAttr(ti, ai));
            }
            // 非 instance 属性は非対応。
            other => {
                bail_expr("assign-target", other);
                return None;
            }
        }
        Some(())
    }

    /// `obj.attr <op>= value` / `obj[k] <op>= value`。
    ///
    /// ⚠ #62 以前は `Stmt::AttrCompoundAssign` の**アームが 2 つ**（添字用のパターンガード付きと
    /// 一般用）に分かれていた。同じ文種別を 2 箇所で受けると片方だけ直す事故が起きるので、
    /// **入口を 1 つにして中で分岐**させてある。
    pub(super) fn compile_attr_compound_assign(
        &mut self,
        target: &Expr,
        op: &BinOp,
        value: &Expr,
    ) -> Option<()> {
        match target {
            // 添字への複合代入 `obj[k] op= value`（#27-c）。
            //
            // ツリーウォークは `rhs = eval(value)` → `lhs = eval(target)` → 二項演算 →
            // `attr_assign(target, result)` の順で、**`object`/`index` を 2 回評価する**
            // （読みで 1 回、代入で 1 回）。副作用まで一致させるため、そのまま 2 回積む。
            Expr::Subscript { object, index, .. } => {
                let rhs_tmp = self.alloc_temp()?;
                self.compile_expr(value)?; // 1. rhs を先に評価
                self.emit(Op::StoreLocal(rhs_tmp));
                self.compile_expr(object)?; // 2. 現在値の読み
                self.compile_expr(index)?;
                self.emit(Op::Subscript);
                self.emit(Op::LoadLocal(rhs_tmp));
                self.emit(Op::Bin(op.clone())); // 3. 二項演算
                let res_tmp = self.alloc_temp()?;
                self.emit(Op::StoreLocal(res_tmp));
                self.compile_expr(object)?; // 4. 代入（`attr_assign` と同じく再評価）
                self.compile_expr(index)?;
                self.emit(Op::LoadLocal(res_tmp));
                self.emit(Op::SetIndex);
                self.free_temp();
                self.free_temp();
                Some(())
            }
            // 属性複合代入 `obj.attr op= value`。
            //
            // ⚠ **レシーバの種類で絞らない**（#27）。読みは `GetAttr`（ツリーウォークの
            // `eval_attr` と同じ `get_attr_val`）、書きは `SetAttr`（`attr_assign` と**同一の**
            // `attr_assign_evaled`）なので、`Value::Class` の `static mut` まで意味論が一致する。
            // 以前あった `object_is_instance` の条件は「2 実装の差」ではなく、下の
            // **局所 slot 前提の最適化**（レシーバを 1 回しか評価しない融合）を守るためのもの。
            // 局所 slot でないレシーバはツリーウォークどおり 2 回評価する経路へ回す。
            Expr::Attr { object, attr, node_id, .. } => {
                let ni = self.add_name(attr);
                // 型特化（#2b）: フィールドの型は注釈テーブルが `Expr::Attr` の node_id に焼いている。
                // 右辺は `expr_prim`（リテラル / 型注釈つき局所変数）で見る。
                let kind = self
                    .annot_prim(*node_id)
                    .zip(self.expr_prim(value))
                    .and_then(|(l, r)| Self::pair_kind(l, r))
                    .and_then(|k| Self::gate_bin_kind(k, op));
                match self.as_local(object) {
                    // レシーバが局所 slot（`self`・ローカル変数）のとき。**再評価が副作用を
                    // 持たない**ので、`SetAttr` のベースを 1 回積むだけで読み書き両方に使える。
                    Some(obj_slot) => {
                        self.compile_expr(object)?; // SetAttr のベース

                        // 評価順（#2a）。ツリーウォークは **value を先に評価してから**現在値を読むので、
                        // 素直に組むと [value, cur] の順にスタックへ乗り `Swap` が要る。
                        // ただし value が**副作用を持たない**（局所変数読み or 定数リテラル）なら、
                        // 先に現在値を読んでも観測結果は同じなので `Swap` を丸ごと落とせる。
                        // レシーバ slot が value の評価中に再束縛されないことは `CallMethodLocal` と
                        // 同じ根拠（再束縛は文＝`StoreLocal` でしか起きず、クロージャ捕捉は VM 非対応
                        // で bail する）。
                        //
                        // 現在値の読み出しは `LoadLocal; GetAttr` の 2 命令を `GetAttrLocal` 1 命令へ
                        // 畳む（レシーバを **clone せず frame から参照で読む**ので `Rc` の refcount
                        // 増減も消える）。`Expr::Attr` の compile と同じ融合。
                        let value_pure =
                            self.as_local(value).is_some() || Self::as_const_lit(value).is_some();
                        if !value_pure {
                            self.compile_expr(value)?; // rhs を先に評価（順序保存）
                        }
                        self.emit(Op::GetAttrLocal(obj_slot, ni, ni));
                        if value_pure {
                            // [obj, cur, value] → Bin → [obj, new]（Swap 不要）
                            self.compile_expr(value)?;
                        } else {
                            // [obj, value, cur] → Swap → [obj, cur, value] → Bin → [obj, new]
                            self.emit(Op::Swap);
                        }
                        self.emit_bin_specialized(kind, op);
                        self.emit(Op::SetAttr(ni));
                    }
                    // 一般レシーバ（グローバル変数・クラス名・属性・呼び出し結果／`CompileMode::DebugRepl`）。
                    //
                    // ツリーウォークは `eval(value)` → `eval(target)`（**object 1 回目**）→ 二項演算
                    // → `attr_assign(target, ..)`（**object 2 回目**）の順で、`object` を 2 回評価する。
                    // 副作用まで一致させるため**そのまま 2 回積む**（添字複合代入 `d[k] op= v` と
                    // 同じ扱い・#27-c）。上の融合を使えるのは再評価が無害な局所 slot のときだけ。
                    None => {
                        let rhs_tmp = self.alloc_temp()?;
                        self.compile_expr(value)?; // 1. rhs を先に評価
                        self.emit(Op::StoreLocal(rhs_tmp));
                        self.compile_expr(object)?; // 2. 現在値の読み
                        self.emit(Op::GetAttr(ni, ni));
                        self.emit(Op::LoadLocal(rhs_tmp));
                        self.emit_bin_specialized(kind, op); // 3. 二項演算
                        let res_tmp = self.alloc_temp()?;
                        self.emit(Op::StoreLocal(res_tmp));
                        self.compile_expr(object)?; // 4. 代入（`attr_assign` と同じく再評価）
                        self.emit(Op::LoadLocal(res_tmp));
                        self.emit(Op::SetAttr(ni));
                        self.free_temp();
                        self.free_temp();
                    }
                }
                Some(())
            }
            other => {
                bail_expr("attr-compound-target", other);
                None
            }
        }
    }
}
