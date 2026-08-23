// vm/compiler/emit.rs — 命令発行のプリミティブ・書き込み先の決定・**型特化の判定**。
//
// ⚠ コード索引を持つ op を足したら [`peephole::code_target_mut`](super::super::peephole) に
// 登録すること（忘れてもテストも例題も通ってしまう・#27-d）。


use crate::ast::{
    BinOp, Expr, Stmt, Resolution,
};
use crate::interpreter::Value;

use crate::vm::op::Op;
use super::*;


impl Compiler {
    /// 自由な識別子を**名前で読む**か（`LoadName`）。偽なら `LoadGlobal`（`scopes[0]` 限定）。
    ///
    /// ⚠ **`toplevel_globals` の空・非空を見る**のはモードに畳めないため（`CompileMode` の doc）。
    /// 最上位モードで「その名前が最上位で宣言されている」ことは条件では**ない** —
    /// 集合が非空でありさえすれば名前引きに落ちる（#27-c）。
    #[inline]
    pub(super) fn reads_by_name(&self) -> bool {
        self.mode.uses_name_lookup() || !self.toplevel_globals.is_empty()
    }

    /// 「slot に無い名前をグローバルへ書ける」モードか（#10-b）。
    /// ⚠ 空・非空の判定そのものが条件（`reads_by_name` と同じ理由でモードに畳めない）。
    #[inline]
    pub(super) fn writes_toplevel_globals(&self) -> bool {
        !self.toplevel_globals.is_empty()
    }

    #[inline]
    pub(super) fn emit(&mut self, op: Op) -> usize {
        self.chunk.code.push(op);
        // 行テーブルを code と 1:1 に保つ（#1）。`compile_stmt` が予約していれば
        // **この op が文の先頭**なので、そこに文の位置を記録する。
        self.chunk.stmt_spans
            .push(self.pending_stmt.take().unwrap_or(crate::vm::chunk::NOT_STMT));
        self.chunk.code.len() - 1
    }

    /// スタック規律の一時 slot を確保する（match サブジェクト等）。名前付き slot の上に積む。
    /// `free_temp` と対で使う。フレーム総 slot 数（`n_locals`）を必要に応じて拡張する。
    pub(super) fn alloc_temp(&mut self) -> Option<u16> {
        let Some(slot) = self.named_locals.checked_add(self.temps_in_use) else {
            bail("temp-slot-overflow", None);
            return None;
        };
        self.temps_in_use = self.temps_in_use.checked_add(1)?;
        let total = self.named_locals as usize + self.temps_in_use as usize;
        if total > self.chunk.n_locals {
            self.chunk.n_locals = total;
        }
        Some(slot)
    }

    pub(super) fn free_temp(&mut self) {
        self.temps_in_use -= 1;
    }

    pub(super) fn add_const(&mut self, v: Value) -> u32 {
        let idx = self.chunk.consts.len() as u32;
        self.chunk.consts.push(v);
        idx
    }

    /// 式が「単純なローカル読み」なら slot を返す（超命令の融合判定, #2）。
    /// `LoadLocal` に落ちる形（`Resolution::Local` / slot 表に載る未解決 `Ident`）のみ。
    /// debug_mode では融合しない（未解決 `Ident` は `LoadName` に落ちるため）。
    /// `Resolution::Global` は対象外（`_` に落ちる）。
    pub(super) fn as_local(&self, e: &Expr) -> Option<u16> {
        if self.mode.is_debug_repl() {
            return None;
        }
        match e {
            // `static mut` / セル変数は slot を持たない（#27-d）。融合の対象外。
            Expr::Ident { name, .. }
                if self.statics.contains_key(name) || self.cells.contains_key(name) =>
            {
                None
            }
            Expr::Ident { res: Resolution::Local(slot), .. } => u16::try_from(*slot)
                .ok()
                .filter(|s| !self.cell_by_slot.contains_key(s)),
            Expr::Ident { name, res: Resolution::Unresolved, .. } => self.slots.get(name).copied(),
            _ => None,
        }
    }

    /// 式が数値/真偽リテラルなら定数値を返す（超命令の融合判定, #2）。
    pub(super) fn as_const_lit(e: &Expr) -> Option<Value> {
        match e {
            Expr::Int(n) => Some(Value::Int(*n)),
            Expr::Float(f) => Some(Value::Float(*f)),
            Expr::Bool(b) => Some(Value::Bool(*b)),
            _ => None,
        }
    }

    /// 局所変数の型注釈から二項演算の特化種別を導出する（#16 段階 E）。
    ///
    /// 両オペランドが**同一プリミティブの型注釈を持つ局所変数**のときだけ種別を返す。
    /// リテラル（`Expr::Int`/`Expr::Float`）は型が自明なので相方に合わせて認める。
    /// テンプレート実体化後の関数のように「AST には具体型が書かれているが注釈テーブルは
    /// 原型を指している」ケースを拾うのが目的。
    pub(super) fn local_operand_kind(
        &self,
        left: &Expr,
        right: &Expr,
    ) -> Option<crate::type_check::BinOperandKind> {
        let (l, r) = (self.expr_prim(left)?, self.expr_prim(right)?);
        // 両方がリテラルなら特化しても意味が無い（定数同士）。
        if matches!(left, Expr::Int(_) | Expr::Float(_))
            && matches!(right, Expr::Int(_) | Expr::Float(_))
        {
            return None;
        }
        Self::pair_kind(l, r)
    }

    /// 左辺が **slot 番号で与えられる** 場合の特化種別（複合代入 `x <op>= e` 用・#2b）。
    /// `local_operand_kind` の左辺を「式」から「slot」に替えただけで、判断基準は同一。
    pub(super) fn slot_operand_kind(
        &self,
        slot: u16,
        right: &Expr,
    ) -> Option<crate::type_check::BinOperandKind> {
        Self::pair_kind(self.slot_prim(slot)?, self.expr_prim(right)?)
    }

    /// 式のプリミティブ型名。局所変数は型注釈から、数値リテラルは自明な型から。
    pub(super) fn expr_prim(&self, e: &Expr) -> Option<&'static str> {
        match e {
            Expr::Int(_) => Some("int"),
            Expr::Float(_) => Some("float"),
            _ => self.slot_prim(self.as_local(e)?),
        }
    }

    /// 局所 slot のプリミティブ型名（型注釈が int/float のときのみ）。
    pub(super) fn slot_prim(&self, slot: u16) -> Option<&'static str> {
        match self.slot_type.get(slot as usize)?.as_deref()? {
            "int" => Some("int"),
            "float" => Some("float"),
            _ => None,
        }
    }

    /// 注釈テーブルが焼いた**結果型**からプリミティブ型名を引く（#2b）。
    /// `slot_type`（AST に書かれた型注釈）では届かないノード（属性読みなど）用。
    pub(super) fn annot_prim(&self, node_id: u32) -> Option<&'static str> {
        match self.annotations.resolved_type(node_id)? {
            crate::type_check::InferredType::Int => Some("int"),
            crate::type_check::InferredType::Float => Some("float"),
            _ => None,
        }
    }

    /// 両オペランドのプリミティブ型名が一致していれば特化種別に落とす。
    pub(super) fn pair_kind(l: &str, r: &str) -> Option<crate::type_check::BinOperandKind> {
        use crate::type_check::BinOperandKind as K;
        if l != r {
            return None;
        }
        match l {
            "int" => Some(K::Int),
            "float" => Some(K::Float),
            _ => None,
        }
    }

    /// 注釈が「両オペランド int/float 確定」かつ型特化してよい op なら、その種別を返す（#16 段階(b)）。
    ///
    /// 許可する op は種別ごとに違う。`apply_binop` に対応するアームが存在するものだけを特化し、
    /// それ以外（float の `//`・`%` など）は汎用パスに委ねてエラー処理を一箇所に保つ。
    /// ゼロ除算は特化側が `None` を返して汎用へ落ちるので、op としては許可してよい。
    pub(super) fn specialized_bin_kind(
        &self,
        op: &BinOp,
        node_id: u32,
        left: &Expr,
        right: &Expr,
    ) -> Option<crate::type_check::BinOperandKind> {
        let kind = match self.annotations.binop_kind(node_id) {
            Some(k) => k,
            // 注釈が無いときは**局所変数の型注釈**から導出する（#16 段階 E）。
            //
            // テンプレート実体化では `subst_params` が param の型注釈を具体型へ置き換える
            // （`fn add[T](a: T, b: T)` → `add[int]` なら `a: int, b: int`）が、
            // node-id は原型からコピーされるため注釈テーブルは**型変数のままの原型**を指す。
            // そこで実体化後の AST に書かれている型注釈を直接見る。
            // 注釈テーブルが届かない箇所（import 先モジュール等）にも同じ理由で効く。
            //
            // 特化 op は実行時型が想定外なら汎用へフォールバックするので、
            // この導出が外れていても**結果は変わらない**（速度の無駄が出るだけ）。
            None => self.local_operand_kind(left, right)?,
        };
        Self::gate_bin_kind(kind, op)
    }

    /// 左辺が **slot 番号で与えられる** 場合の特化種別（複合代入 `x <op>= e` 用・#2b）。
    /// 注釈の引き方・op の許可判定は `specialized_bin_kind` と同一で、
    /// 型導出のフォールバックだけが slot 版になる。
    /// 注釈テーブルだけから二項演算の種別を引く（#10-b のグローバル複合代入用）。
    /// slot 版（`specialized_bin_kind_slot`）の「slot の型注釈から推す」経路はグローバルには
    /// 使えないので、型検査が焼いた `binop_kind` のみを見る。
    pub(super) fn annot_binop_kind(&self, node_id: u32) -> Option<crate::type_check::BinOperandKind> {
        // op のゲートは呼び出し側が渡す op で行う（`gate_bin_kind`）。ここでは種別だけ返す。
        self.annotations.binop_kind(node_id)
    }

    /// 最上位宣言の名前なら name プールの index を返す（#10-c）。
    ///
    /// 条件は 2 つ: **最上位モードであること**（`toplevel_globals` が非空）と、
    /// **その名前が slot に無いこと**。後者が要るのは、最上位文の内側（ループ本体・
    /// ブロック式の中）の宣言は slot だから — そちらは従来どおり `StoreLocal*` に落とす。
    pub(super) fn toplevel_decl_name(&mut self, name: &str) -> Option<u32> {
        if !self.writes_toplevel_globals() || self.slots.contains_key(name) {
            return None;
        }
        Some(self.add_name(name))
    }

    /// 名前を VM フレームの slot へ解決する（`static mut`／セル変数は slot を持たないので `None`）。
    ///
    /// ⚠ 引けなかった理由を**必ず計上する**（#27-c）。素の `self.slots.get(name)?` で諦めると
    /// 「未帰属」として計測から消え、`For/unattributed:For` のように出所が追えなくなる。
    pub(super) fn slot_of(&self, name: &str) -> Option<u16> {
        // ⚠ `static mut` / セル変数は slot を持たない（#27-d）。ここへ来る経路（for ターゲット・
        // `let a,b = t`・`except as`・入れ子 `fn` の格納先）は共有セルを扱えないので諦める。
        if self.cells.contains_key(name) {
            if crate::interpreter::tw_stats::enabled() {
                crate::interpreter::tw_stats::record_bail("cell-as-slot", name);
            }
            return None;
        }
        if self.statics.contains_key(name) {
            if crate::interpreter::tw_stats::enabled() {
                crate::interpreter::tw_stats::record_bail("static-as-slot", name);
            }
            return None;
        }
        match self.slots.get(name) {
            Some(&s) => Some(s),
            None => {
                if crate::interpreter::tw_stats::enabled() {
                    crate::interpreter::tw_stats::record_bail("decl-no-slot", name);
                }
                None
            }
        }
    }

    /// 変数への書き込み先を決める（#10-b）。
    ///
    /// 1. セル変数 → `Cell` ／ `static mut` → `Static`（どちらも slot を持たない・**slot より先**）
    /// 2. VM の slot にある名前 → `Local`（関数本体でもブロック内宣言でもここに来る）
    /// 3. モジュール本体（`CompileMode::Module`）→ `Name`（チェーン探索・#42）
    /// 4. 最上位モードで可視グローバルと確定できる名前 → `Global`
    /// 5. どれでもない → `None`（＝この文は VM に載らない ⇒ `VmForceError` で停止・#33）
    ///
    /// ⚠ 順序が重要。ループ本体の `let` は毎回スコープに入る**ローカル**なので、
    /// 同名グローバルより先に slot を見なければならない。
    ///
    /// ⚠⚠ この doc は #51 まで**下の `slot_of` に付いていた**（`slot_of` が後から
    /// この doc と本体の間へ挿入されたため）。orphan doc は `#[inline(never)]` を
    /// 別関数へ運ぶこともある（`vm_toplevel.rs` の実例）。**関数を挿入するときは
    /// doc ブロックの上へ置くこと**。
    pub(super) fn store_target(&mut self, name: &str) -> Option<StoreTarget> {
        // セル変数は slot ではなく共有セル（#27-d 段階 2b）。**slot より先に見る**。
        if let Some(&i) = self.cells.get(name) {
            return Some(StoreTarget::Cell(i));
        }
        // `static mut` も slot ではなく共有セル（#27-d）。**slot より先に見る**。
        if let Some(span) = self.statics.get(name).cloned() {
            let si = self.add_span(&span);
            return Some(StoreTarget::Static(si));
        }
        if let Some(&slot) = self.slots.get(name) {
            return Some(StoreTarget::Local(slot));
        }
        // ⚠ デバッガ REPL（`compile_debug`）だけは例外（#39）。停止フレームの**生スコープ**へ
        // 書かねばならず、`scopes[0]` 限定の `StoreGlobal` では別の変数を書いてしまう。
        // 読み側が `LoadName` に落ちているのと同じ理由。ここは従来どおり bail する。
        if self.mode.is_module_body() {
            // モジュール本体は push 済みスコープに名前がある（#42）。
            let ni = self.add_name(name);
            return Some(StoreTarget::Name(ni));
        }
        if self.mode.uses_name_lookup() {
            // 停止フレーム／外側スコープの**生の変数**へ書く必要があるが、
            // `scopes[0]` 限定の `StoreGlobal` では別の変数を書いてしまう（#39）。
            // ⚠ チャンク内で宣言したローカルは上の `slots` で先に拾われるので影響しない。
            bail("store-target-name-lookup", None);
            return None;
        }
        // ここまで全部外れた名前は**この関数のローカルでもキャプチャでもない**（#39）。
        //
        // 根拠は `Op::LoadGlobal` を関数本体で使うのと同じ（#27）: base slot の採番と
        // `collect_nested_decls` が本体の全宣言を**先に** `slots` へ入れ、可変キャプチャは
        // `capture_env` が作った集合ごと `cells` に入る。つまり `slots`/`cells`/`statics` を
        // 引いて外れた名前は、ツリーウォークの `assign_var` でもローカル走査を必ず素通りして
        // グローバル分岐へ落ちる。⇒ `scopes[0]` へ書く `StoreGlobal` と答えが一致する。
        //
        // ⚠ **最上位で宣言されているか（`toplevel_globals`）は条件にしない**。未宣言の名前は
        // `vm_assign_global` が `NameError: '<name>' is not defined` を返し、これも
        // ツリーウォークと同一文言（以前はここで bail し、関数本体からのグローバル代入が
        // 丸ごと `VmForceError` になっていた）。
        let ni = self.add_name(name);
        // `LoadGlobal` と同じく emit 1 回につきキャッシュ枠を 1 本割り当てる
        // （枠は共有しない。op ごとに焼く index の意味が違うため — `Op::StoreGlobal` 参照）。
        let ci = self.chunk.global_caches.len() as u32;
        self.chunk.global_caches.push(crate::ast::SlotCache::default());
        Some(StoreTarget::Global(ni, ci))
    }

    pub(super) fn specialized_bin_kind_slot(
        &self,
        op: &BinOp,
        node_id: u32,
        slot: u16,
        right: &Expr,
    ) -> Option<crate::type_check::BinOperandKind> {
        let kind = match self.annotations.binop_kind(node_id) {
            Some(k) => k,
            None => self.slot_operand_kind(slot, right)?,
        };
        Self::gate_bin_kind(kind, op)
    }

    /// 特化してよい op かを判定する（種別ごとの許可リスト）。
    /// `specialized_bin_kind` / `specialized_bin_kind_slot` の共通判断。
    pub(super) fn gate_bin_kind(
        kind: crate::type_check::BinOperandKind,
        op: &BinOp,
    ) -> Option<crate::type_check::BinOperandKind> {
        use crate::type_check::BinOperandKind as K;
        let allowed = match kind {
            // int/int は `apply_binop` の Int/Int アームを全て特化できる。
            K::Int => matches!(
                op,
                BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::FloorDiv
                    | BinOp::Mod
                    | BinOp::Pow
                    | BinOp::BitAnd
                    | BinOp::BitOr
                    | BinOp::BitXor
                    | BinOp::LShift
                    | BinOp::RShift
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::LtEq
                    | BinOp::GtEq
                    | BinOp::Eq
                    | BinOp::NotEq
            ),
            // float は `//`・`%`・ビット演算のアームが無いので算術と比較のみ。
            K::Float => matches!(
                op,
                BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::Pow
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::LtEq
                    | BinOp::GtEq
                    | BinOp::Eq
                    | BinOp::NotEq
            ),
        };
        if allowed {
            Some(kind)
        } else {
            None
        }
    }

    /// 二項演算 `left <op> right` を超命令へ融合できれば emit して `true`（#2 ＋ plan A 型特化）。
    /// `local <op> local` → `BinLocalLocal`、`local <op> リテラル` → `BinLocalConst`。
    /// さらに注釈が「両オペランド int/float 確定」かつ対応 op（Add/Sub/Mul・比較）なら**型特化 op**
    /// （`IntBinLL`/`FloatBinLC` 等）を emit（タグ検査・op ディスパッチ・clone を削減）。
    /// 型特化 op は実行時型が想定外なら**汎用へフォールバック**するので、注釈が古くても健全。
    /// 融合できなければ `false`（呼び出し側が通常経路 `LoadLocal…; Bin` を出す）。意味論は不変。
    pub(super) fn try_emit_bin_fused(&mut self, left: &Expr, right: &Expr, op: &BinOp, node_id: u32) -> bool {
        if let Some(a) = self.as_local(left) {
            let kind = self.specialized_bin_kind(op, node_id, left, right);
            return self.emit_bin_fused_slot(a, kind, right, op);
        }
        // ⚠ #70: 左辺が slot でなくても、**最上位のグローバル**なら融合できる。
        // それまでは `LoadGlobal; LoadGlobal; IntBinSS` の 3 命令に落ちており、
        // 同じループが fn の中より **2.3〜2.5x 遅かった**（`bench_toplevel_loop.ar`）。
        self.try_emit_bin_fused_global(left, right, op, node_id)
    }

    /// 式が **`LoadGlobal` に落ちる読み**なら、その名前を返す（**副作用なし**・#70）。
    ///
    /// ⚠⚠ **判定は `compile_expr` の `Expr::Ident` アームと同じでなければならない。**
    /// あちらは cells → statics → `Resolution::Local` → `Resolution::Global` の順に見るので、
    /// ここでも **cells / statics を明示的に除く**（`Resolution::Global` は `Local` と排他）。
    /// 間違えると**別の記憶域を読む**（`static mut` のセルは `Interpreter::static_cells` にある）。
    ///
    /// ⚠ `CompileMode::DebugRepl` は名前引き（`LoadName`）なので融合しない。
    fn global_read_name<'a>(&self, e: &'a Expr) -> Option<&'a str> {
        if self.mode.is_debug_repl() {
            return None;
        }
        match e {
            Expr::Ident { name, res: Resolution::Global(_), .. }
                if !self.cells.contains_key(name) && !self.statics.contains_key(name) =>
            {
                Some(name)
            }
            _ => None,
        }
    }

    /// グローバル参照表へ 1 件積んで index を返す（#70）。
    ///
    /// ⚠ **同じ名前でも毎回 1 枠取る**（`emit_load_global` の索引キャッシュと同じ方針）。
    /// 枠を共有すると、別の使用地点のキャッシュの当たり外れが混ざる。
    fn add_global_ref(&mut self, name: &str) -> u32 {
        let ni = self.add_name(name);
        let gi = self.chunk.global_refs.len() as u32;
        self.chunk
            .global_refs
            .push((ni, crate::ast::SlotCache::default()));
        gi
    }

    /// `x <op>= e` の左辺が**最上位グローバル**のとき、読み＋二項演算を 1 命令へ融合する（#70）。
    ///
    /// 成功すると**結果がスタックに 1 つ積まれた状態**で返るので、呼び出し側が `StoreGlobal` を出す。
    /// ⚠ 融合しないときは**何も emit しない**（副作用なしで `false`）。
    ///
    /// ⚠ 左辺が `LoadGlobal` に落ちることは呼び出し側が `StoreTarget::Global` で確かめている
    /// （書きが `scopes[0]` なら読みも `emit_load_global` — 従来の経路がそう組んでいる）。
    pub(super) fn try_emit_compound_fused_global(
        &mut self,
        name: &str,
        right: &Expr,
        op: &BinOp,
        node_id: u32,
    ) -> bool {
        use crate::type_check::BinOperandKind as K;
        if self.annot_binop_kind(node_id) != Some(K::Int) {
            return false;
        }
        if let Some(rname) = self.global_read_name(right).map(str::to_string) {
            let ga = self.add_global_ref(name);
            let gb = self.add_global_ref(&rname);
            self.emit(Op::IntBinGG(ga, gb, op.clone()));
            return true;
        }
        if let Some(cv) = Self::as_const_lit(right) {
            let ga = self.add_global_ref(name);
            let ci = self.add_const(cv);
            self.emit(Op::IntBinGC(ga, ci, op.clone()));
            return true;
        }
        false
    }

    /// 最上位グローバル同士（またはグローバル＋定数）の二項演算を融合する（#70）。
    ///
    /// ⚠ **int 特化が確定しているときだけ**融合する。`Float` / 型不明のグローバル版 op は
    /// 足していない — op を増やすほど `Op` のサイズ余裕と命令列のキャッシュ密度を削るので、
    /// **実測で効くと分かった形（int のループ制御）だけ**に絞ってある。
    ///
    /// ⚠⚠ **判定を全部済ませてから表に積む**。`add_global_ref` / `add_const` は
    /// 副作用（表への push）を持つので、途中で諦めると**使われない枠が残る**。
    fn try_emit_bin_fused_global(
        &mut self,
        left: &Expr,
        right: &Expr,
        op: &BinOp,
        node_id: u32,
    ) -> bool {
        use crate::type_check::BinOperandKind as K;
        if self.specialized_bin_kind(op, node_id, left, right) != Some(K::Int) {
            return false;
        }
        let Some(lname) = self.global_read_name(left).map(str::to_string) else {
            return false;
        };
        // グローバル `<op>` グローバル
        if let Some(rname) = self.global_read_name(right).map(str::to_string) {
            let ga = self.add_global_ref(&lname);
            let gb = self.add_global_ref(&rname);
            self.emit(Op::IntBinGG(ga, gb, op.clone()));
            return true;
        }
        // グローバル `<op>` 定数
        if let Some(cv) = Self::as_const_lit(right) {
            let ga = self.add_global_ref(&lname);
            let ci = self.add_const(cv);
            self.emit(Op::IntBinGC(ga, ci, op.clone()));
            return true;
        }
        false
    }

    /// 左辺が slot と確定している二項演算を 1 命令へ融合 emit する（`try_emit_bin_fused` の中核）。
    /// 右辺が局所変数なら `*BinLL`、定数リテラルなら `*BinLC`。どちらでもなければ `false` を返し、
    /// 呼び出し側が通常経路（オペランドを積んでから `Bin`/`*BinSS`）を出す。
    ///
    /// **評価順について**: 融合後はスタックへ積まず frame から左辺を読むので、形の上では
    /// 「右辺を用意してから左辺を読む」順になる。ただし融合する右辺は局所変数読みか定数
    /// リテラルのみで**副作用が無い**ため、観測される値は融合前と同一（`CallMethodLocal` と同じ理由）。
    pub(super) fn emit_bin_fused_slot(
        &mut self,
        a: u16,
        kind: Option<crate::type_check::BinOperandKind>,
        right: &Expr,
        op: &BinOp,
    ) -> bool {
        use crate::type_check::BinOperandKind as K;
        if let Some(b) = self.as_local(right) {
            match kind {
                Some(K::Int) => self.emit(Op::IntBinLL(a, b, op.clone())),
                Some(K::Float) => self.emit(Op::FloatBinLL(a, b, op.clone())),
                None => self.emit(Op::BinLocalLocal(a, b, op.clone())),
            };
            true
        } else if let Some(cv) = Self::as_const_lit(right) {
            let ci = self.add_const(cv);
            match kind {
                Some(K::Int) => self.emit(Op::IntBinLC(a, ci, op.clone())),
                Some(K::Float) => self.emit(Op::FloatBinLC(a, ci, op.clone())),
                None => self.emit(Op::BinLocalConst(a, ci, op.clone())),
            };
            true
        } else {
            false
        }
    }

    /// `LoadGlobal` を index キャッシュ付きで emit する（#11）。name プールとキャッシュ枠を確保。
    pub(super) fn emit_load_global(&mut self, name: &str) {
        let ni = self.add_name(name);
        let ci = self.chunk.global_caches.len() as u32;
        self.chunk.global_caches.push(crate::ast::SlotCache::default());
        self.emit(Op::LoadGlobal(ni, ci));
    }

    /// スタック上位 2 値への二項演算を、型特化つきで emit する（#2b）。
    /// `kind` が決まらなければ動的ディスパッチの `Bin`。属性複合代入の 2 経路で共有する。
    pub(super) fn emit_bin_specialized(&mut self, kind: Option<crate::type_check::BinOperandKind>, op: &BinOp) {
        match kind {
            Some(crate::type_check::BinOperandKind::Int) => self.emit(Op::IntBinSS(op.clone())),
            Some(crate::type_check::BinOperandKind::Float) => self.emit(Op::FloatBinSS(op.clone())),
            None => self.emit(Op::Bin(op.clone())),
        };
    }

    pub(super) fn add_name(&mut self, name: &str) -> u32 {
        let idx = self.chunk.names.len() as u32;
        self.chunk.names.push(name.to_string());
        self.chunk.attr_caches.push(crate::ast::AttrCache::default());
        idx
    }

    /// AST 型解決層の**検査指示**（`CheckBefore`）を消費する（#16 段階(b)(ii)）。
    ///
    /// `mustbe` / `=>` は型検査が常に `CheckBefore` を付けるので、現状これは実質いつも `true` を返す。
    /// それでも指示を経由するのは、将来チェッカが「この検査は静的に冗長」と証明できるようになったとき、
    /// **この一点を変えるだけで VM とネイティブの双方が検査を落とせる**ようにするため（＝解決の一元化）。
    ///
    /// 指示が `None`（未採番ノード・合成 AST・モジュール横断で注釈が無い）の場合は
    /// **検査が要るのか判らない**ので、その関数の VM 化自体を諦める（`false`）。
    /// 検査を省く方向へは決して倒さない。
    pub(super) fn check_required(&self, node_id: u32) -> bool {
        matches!(
            self.annotations.directive(node_id),
            crate::type_check::Directive::CheckBefore(_)
        )
    }

    pub(super) fn add_span(&mut self, span: &crate::token::Span) -> u32 {
        let idx = self.chunk.spans.len() as u32;
        self.chunk.spans.push(span.clone());
        idx
    }

    /// 次に emit する op を「文の先頭」として予約する（#1・行テーブル）。
    ///
    /// ツリーウォークは `exec()` の冒頭で**すべての文**について `should_pause_at` を呼ぶので、
    /// VM も**すべての文**の先頭を記録しないと停止位置が食い違う。
    /// 位置情報を持たない種類の文（`if`/`while`/`return` 等）は `STMT_NO_SPAN` を記録し、
    /// 表示スパンは `best_span_for` のフォールバック（`DebugState::last_span`）に委ねる。
    ///
    /// ⚠ `CompileMode::DebugRepl`（デバッガ REPL 用の `compile_debug`）では記録しない。
    /// あちらは停止対象ではなく、REPL 入力を評価するだけの Chunk。
    pub(super) fn mark_stmt_start(&mut self, stmt: &Stmt) {
        if self.mode.is_debug_repl() {
            return;
        }
        let idx = match crate::interpreter::debugger::stmt_span_of(stmt) {
            Some(span) => self.add_span(&span),
            None => crate::vm::chunk::STMT_NO_SPAN,
        };
        self.pending_stmt = Some(idx);
    }

    /// バックパッチ用: 直後に置く命令の index を現在位置として返す。
    #[inline]
    pub(super) fn here(&self) -> u32 {
        self.chunk.code.len() as u32
    }

    pub(super) fn patch_jump(&mut self, at: usize, target: u32) {
        self.chunk.code[at] = match &self.chunk.code[at] {
            Op::Jump(_) => Op::Jump(target),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(target),
            Op::JumpIfFalseOrPop(_) => Op::JumpIfFalseOrPop(target),
            Op::JumpIfTrueOrPop(_) => Op::JumpIfTrueOrPop(target),
            _ => unreachable!("patch_jump on non-jump op"),
        };
    }

    /// `break`/`continue` が外側ループへ跳ぶ前に、途中のブロック式が積んだオペランドを捨てる（#34）。
    ///
    /// Arrow の `break` は入れ子の `if`/`match`/`block:` **式**を貫通して外側ループへ届く。
    /// ブロック式は値をすべて temp slot に置くので、跳ぶ時点でオペランドスタックに残るのは
    /// **そのブロック式より外側の式が積んだ分**だけ（`let s = 1 + block ->int: … break …` の `1`）。
    /// その数が `stmt_base`。ループ入口は必ず深さ 0 基準なので、ここまで戻せば跳び先と平衡する。
    ///
    /// `None`（深さ不明）なら bail する。深さを伝えていない式の形は「壊れる」のではなく
    /// 「VM に載らない」で止まるので、伝播の書き漏らしは安全側に倒れる。
    /// ブロック式の `->T` アノテーションを名前プールへ入れて index を返す（#35）。
    /// 注釈が無ければ `None`（＝実行時検査を出さない＝ツリーウォークと同じ）。
    pub(super) fn add_return_type(&mut self, return_type: &Option<String>) -> Option<u32> {
        return_type.as_ref().map(|t| self.add_name(t))
    }
}
