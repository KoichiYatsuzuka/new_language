// interpreter/vm_toplevel.rs — VM から呼ぶインタプリタ側の入口。
//
// - モジュール最上位の文を VM で実行する経路（#10-b）
// - 一部 op の実体（`StoreGlobal` #10-b / `LoadSelfClass` #27）
//
// ⚠ **この関数を `functions/execution.rs` に置いてはいけない。**
// 同じ `impl Interpreter` でもファイルが違えば済む話ではなく、`exec_fn_evaled` と同居させると
// LLVM のインライン判断が変わり、ネイティブ→Arrow コールバックのループで **10% 級の退行**が出た
// （`partial_call_overhead.ar` で実測・`--vm=off` でも同じ幅で出るので VM 経路とは無関係だった）。
// #1-x の「`#[inline]` は効いているとは限らない」と同じ現象を逆向きに踏んだもの。

use std::rc::Rc;

use crate::interpreter::{ExecResult, Interpreter, Value, Var};

impl Interpreter {
    /// 最上位ループの VM 実行を**試す価値があるか**の即断（#10-b）。
    ///
    /// ⚠ **`exec()` から必ずこれを先に呼ぶこと。** `Stmt::While`/`Stmt::For` の実行は
    /// 最上位だけでなく**ツリーウォーク関数の中でも起きる**ので、いきなり
    /// `try_run_toplevel_stmt`（非インライン）を呼ぶと関数内ループの実行ごとに
    /// 呼び出しコストを払う。実測でそれが数 % の退行になった。
    /// ここはフィールド 3 本の比較だけなのでインライン展開される。
    #[inline(always)]
    pub(crate) fn toplevel_vm_candidate(&self) -> bool {
        // `scopes.len() == 1` = モジュール最上位（関数フレームも push 済みブロックも無い）。
        self.scopes.len() == 1
            && self.vm_mode != crate::vm::VmMode::Off
            && !self.toplevel_globals.is_empty()
    }

    /// 最上位の文を VM で実行する（#10-b/#10-c）。対象はループ文と宣言文
    /// （`compile_toplevel_stmt` が受け付ける形）。
    ///
    /// 適格でなければ `Ok(None)` を返し、呼び出し側がツリーウォークへ落ちる。
    ///
    /// 適格条件:
    /// - `--vm` が Off でない
    /// - **`scopes.len() == 1`**（＝モジュール最上位。関数フレームや push 済みブロックスコープの
    ///   中ではない）。この 1 条件が「名前は `scopes[0]` を指す」の根拠であり、
    ///   コンパイル時の `toplevel_globals` 判定と実行時の記憶域を一致させる。
    /// - 当該文が Chunk へコンパイルできる
    ///
    /// ⚠ **`exec()` の中から呼ぶ**こと（`run_program` からではなく）。
    /// デバッガの `should_pause_at` は `exec()` 冒頭で走るので、そこを飛ばすと
    /// off/auto でステッピングが食い違う（#1 で修正した既存バグと同じ形）。
    pub(crate) fn try_run_toplevel_stmt(
        &mut self,
        stmt: &crate::ast::Stmt,
    ) -> Result<Option<ExecResult>, String> {
        debug_assert!(self.toplevel_vm_candidate(), "caller must gate on toplevel_vm_candidate");
        // 対象外（定義文）は**失敗として数えない**。キャッシュにも入れない（#27-c）。
        if !crate::vm::is_toplevel_compile_target(stmt) {
            return Ok(None);
        }
        let key = stmt as *const crate::ast::Stmt as usize;
        let chunk = match self.vm_toplevel_chunks.get(&key) {
            Some(cached) => cached.clone(),
            None => {
                let compiled = crate::vm::compile_toplevel_stmt(
                    stmt,
                    self.annotations.clone(),
                    &self.toplevel_globals,
                )
                .map(Rc::new);
                if crate::interpreter::tw_stats::enabled() {
                    crate::interpreter::tw_stats::record_compile("toplevel", compiled.is_some());
                }
                self.vm_toplevel_chunks.insert(key, compiled.clone());
                compiled
            }
        };
        let Some(chunk) = chunk else {
            return Ok(None);
        };

        // フレームは Chunk のローカル数ぶんだけ。パラメータは無いので全て None で始める。
        let mut buf = std::mem::take(&mut self.vm_stack);
        let base = buf.len();
        buf.resize(base + chunk.n_locals, Value::None);
        let result = crate::vm::run(self, &chunk, &mut buf, base);
        buf.truncate(base);
        self.vm_stack = buf;
        // 最上位文は値を返さない（`ReturnNil`）。制御は必ず次の文へ進む。
        result.map(|_| Some(ExecResult::Normal))
    }

    /// VM のメソッド呼び出しのうち **非 Instance レシーバ**の経路（#27-b）。
    /// list/str/dict/set/CsObject/Signal/Namespace… を統一実装へ流す。
    ///
    /// ツリーウォークの `eval_call` の `Expr::Attr` 分岐と**同じ 3 手順**を踏む:
    /// ①呼ぶ前にレシーバから外部言語を覗く ②ディスパッチ ③外部言語なら戻り値を宣言型と照合。
    /// ⚠ **③ を落とすと FFI 境界検査が VM 経路だけ素通りする**（`Op::Call` で #22-a が踏んだ穴と同型）。
    /// そのために `node_id` を op で運んでいる。
    ///
    /// ⚠ **Instance はここへ来ない**。`exec_op` 側で先に `call_instance_method_evaled` へ直行する
    /// （method IC を効かせるため＋最頻路に判定を足さないため。実測でここを経由させると 3% 落ちた）。
    /// ⚠ **`#[inline(never)]` を外さないこと**（`exec_op` は `#[inline(always)]`）。
    #[inline(never)]
    pub(crate) fn vm_method_call_other(
        &mut self,
        obj: Value,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
        node_id: u32,
        chunk: &crate::vm::Chunk,
    ) -> Result<Value, String> {
        let lang = Self::foreign_call_lang(&obj, method_name);
        let r = self.eval_method_call_evaled(obj, method_name, evaled)?;
        let Some(l) = lang else { return Ok(r) };
        // ⚠ 表示名と位置はツリーウォークと**同じもの**を使う（`L.get_int` / `file:line:col`）。
        // `method_name` と `None` で済ませると `get_int` / `<unknown>` になり
        // off/auto でエラーメッセージが食い違う（`ffi_boundary_check_error.ar` が検出）。
        let (name, span) = match chunk.ffi_call_info.get(&node_id) {
            Some(&(ni, si)) => (
                chunk.names[ni as usize].as_str(),
                chunk.spans.get(si as usize),
            ),
            None => (method_name, None),
        };
        self.check_ffi_return(l, r, node_id, name, span)
    }

    /// `Op::DeclareGlobal` の実体（#10-c）: 最上位の `let`/`mut`/`const` を宣言する。
    ///
    /// ツリーウォークの `exec_let` / `exec` の `Const`・`Mut` アームと**同じ判断を同じ順序で**行う。
    /// コンパイル時に決まるのは「どの分岐を取るか」（`DeclKind`）だけで、コピー・フリーズ・
    /// 再宣言検査の実装はここに 1 つだけ置く。
    ///
    /// ⚠ **再宣言の `NameError` を落とさないこと。** 型検査も再宣言を弾くが、
    /// `redeclare_error.ar` が実行時メッセージを stderr で比較しているので挙動が変わると検出される。
    pub(crate) fn vm_declare_global(
        &mut self,
        name: &str,
        kind: crate::vm::op::DeclKind,
        value: Value,
    ) -> Result<(), String> {
        use crate::vm::op::DeclKind;
        // `_` は束縛せず捨てる（ツリーウォークも同じ）。
        if name == "_" {
            return Ok(());
        }
        if self.get_var(name).is_some() {
            return Err(format!("NameError: variable '{name}' is already declared"));
        }
        let (value, mutable) = match kind {
            DeclKind::Const => (value, false),
            // `mut` は常に deep_copy（`exec` の `Stmt::Mut` アームと同一）。
            DeclKind::Mut => (Self::deep_copy_value(value), true),
            DeclKind::LetPlain => (value, false),
            // 非識別子式からの `let`: `Instance` のときだけ copy + freeze。
            // 可変コレクションから取り出した `Instance` を直接フリーズすると
            // 共有 `Rc` 経由で元まで不変化されるため（`exec_let` のコメント参照）。
            DeclKind::LetFreezeInstance => {
                if matches!(value, Value::Instance(_)) {
                    let copied = Self::deep_copy_value(value);
                    self.apply_freeze_to_value(&copied, true)?;
                    (copied, false)
                } else {
                    (value, false)
                }
            }
        };
        self.declare_var(name.to_string(), Var::new(value, mutable));
        Ok(())
    }

    /// `Op::LoadSelfClass` の実体（#27）: メソッド本体の `Self` の値。
    ///
    /// ツリーウォークは `exec_fn_evaled` が `Self` をスコープへ宣言し、VM は
    /// `run_vm_method` が同じクラスを `current_class` に入れる。**同じ出どころ**なので
    /// 値は一致する。メソッド外（`current_class` が `None`）では `None` を返し、
    /// 呼び出し側が `NameError` にする（ツリーウォークの名前引き失敗と同じ）。
    pub(crate) fn vm_self_class(&self) -> Option<Value> {
        self.current_class.clone().map(Value::Class)
    }

    /// `Op::StoreGlobal` の実体（#10-b）: 既存グローバルへの代入。
    ///
    /// **`assign_var` へそのまま委譲する**のが要点。最上位 Chunk の実行中は
    /// `scopes.len() == 1`（VM は `scopes` を使わずフラットな `vm_stack` で動く）なので、
    /// `assign_var` のローカル走査 `scopes[frame_floor..]` は空回りし、グローバル分岐に落ちる。
    /// ＝ ツリーウォークの `Stmt::Assign` と**同じコードが同じ判断をする**（#22 系列の型）。
    pub(crate) fn vm_assign_global(&mut self, name: &str, value: Value) -> Result<(), String> {
        self.assign_var(name, value)
    }

    /// `Op::StoreGlobal` の索引経路（#10-b）: 昇格済みセルへ直接書き込む。
    ///
    /// 戻り値 `Some(value)` = index が失効していて書けなかった（**値を返す**ので
    /// 呼び出し側は clone せずミス経路へ渡し直せる）。`None` = 書き込み成功。
    ///
    /// ツリーウォークの `Stmt::Assign` スロット命中経路（[exec/dispatch.rs]）と同じ書き込み。
    #[inline]
    pub(crate) fn vm_store_global_by_cell(&mut self, idx: usize, value: Value) -> Option<Value> {
        match self.global_slot_cells.get(idx) {
            Some(cell) => {
                *cell.borrow_mut() = value;
                None
            }
            None => Some(value),
        }
    }

    /// `Op::StoreGlobal` のキャッシュ充填（#10-b）。ツリーウォークと同一の `try_fill_slot` へ委譲する。
    /// 昇格できない変数（`Var::Immutable` 等）では何も焼かれず、次回もこの経路を通る。
    pub(crate) fn vm_fill_global_store_cache(&mut self, name: &str, cache: &crate::ast::SlotCache) {
        self.try_fill_slot(name, cache);
    }
}
