// vm/compiler/calls.rs — 呼び出しのコンパイル（引数・キーワード・FFI 情報・書き戻し先・async 投入）。
//
// ⚠ **FFI 境界検査の表示情報（`ffi_call_info`）を落とすと VM 経路だけ素通りする**（#22-a と同型）。
// ⚠ native の `mut` ポインタ書き戻しは**引数式が有るコンパイル時に**先を決めて副表へ置く（#48）。


use std::collections::HashSet;

use crate::ast::{
    CallArg, Expr, Param, Stmt, Resolution,
};

use crate::vm::op::Op;
use super::*;


impl Compiler {
    /// メソッド呼び出しの表示名と位置を副表へ記録する（#27-b・FFI 境界検査のメッセージ用）。
    ///
    /// ツリーウォークの `callee_display_name` と**同じ規則**で作ること
    /// （`obj.attr` 形は `base.attr`、それ以外は `attr`）。ずれると off/auto で
    /// エラーメッセージが食い違う（`ffi_boundary_check_error.ar` が検出した）。
    pub(super) fn record_ffi_call_info(
        &mut self,
        node_id: u32,
        object: &Expr,
        attr: &str,
        span: &crate::token::Span,
    ) {
        if node_id == 0 {
            return; // 未採番（合成 AST 等）は検査キーが引けない
        }
        let display = match object {
            Expr::Ident { name, .. } => format!("{name}.{attr}"),
            _ => attr.to_string(),
        };
        let ni = self.add_name(&display);
        let si = self.add_span(span);
        self.chunk.ffi_call_info.insert(node_id, (ni, si));
    }

    /// 入れ子 `fn` がキャプチャする外側ローカル（名前, slot）を求める（#27）。
    ///
    /// `capture_env` と**同じ自由変数の定義**（参照 − 自前の名前）を使い、現在の `slots`
    /// （＝外側関数のローカル）と交わるものを返す。交わらなければ空 Vec（キャプチャなし）。
    /// ⚠ **`capture_env` と定義がずれると閉包変数が黙って消える**。片方を変えたら両方見ること。
    ///
    /// **可変ローカルを 1 つでも掴むなら `None`**。ツリーウォークはそこで `Var::Cell` へ昇格して
    /// 外側と共有するが、VM のフラット slot（`Value` 直値）では共有セルを表現できない。
    ///
    /// 返す順序は **slot 昇順**（決定的。`captured_env` は `HashMap` なので順序は挙動に影響しないが、
    /// Chunk が実行ごとに変わらないようにするため）。
    #[allow(clippy::type_complexity)]
    pub(super) fn nested_fn_captures(
        &mut self,
        params: &[Param],
        body: &[Stmt],
    ) -> Option<(Vec<(String, u16)>, Vec<(String, u16)>, Vec<(String, u32)>)> {
        use std::collections::HashSet;
        let mut own: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
        crate::interpreter::collect_declared_names(body, &mut own);
        let mut referenced: HashSet<String> = HashSet::new();
        crate::interpreter::collect_referenced_names(body, &mut referenced);

        let mut caps: Vec<(String, u16)> = Vec::new();
        let mut cell_caps: Vec<(String, u16)> = Vec::new();
        let mut static_caps: Vec<(String, u32)> = Vec::new();
        // `add_span` が `&mut self` を要るので、走査順を安定させるためソートしてから回す。
        let mut referenced: Vec<&String> = referenced.iter().collect();
        referenced.sort();
        for n in referenced {
            if own.contains(n) {
                continue;
            }
            // `static mut` のキャプチャ（#27-d 段階 2b）。セルは `Interpreter::static_cells` に
            // あるので、span を運んで実行時に共有する（値のコピーでは書き戻りが消える）。
            if let Some(span) = self.statics.get(n).cloned() {
                let si = self.add_span(&span);
                static_caps.push((n.clone(), si));
                continue;
            }
            // 外側フレームのセル変数のキャプチャ（#27-d 段階 2b）。セル index をそのまま渡す。
            if let Some(&i) = self.cells.get(n) {
                cell_caps.push((n.clone(), i));
                continue;
            }
            let Some(&slot) = self.slots.get(n) else {
                continue; // 外側ローカルでない（グローバル等）＝キャプチャ対象外
            };
            if self.slot_mut.get(slot as usize).copied().unwrap_or(true) {
                // 可変ローカルのキャプチャ。セル化は `nested_fn_free_names` の
                // 事前解析が担うので、ここへ来るのは解析漏れ（保守的に諦める）。
                return None;
            }
            caps.push((n.clone(), slot));
        }
        caps.sort_by_key(|(_, s)| *s);
        cell_caps.sort_by_key(|(_, i)| *i);
        static_caps.sort_by(|a, b| a.0.cmp(&b.0));
        Some((caps, cell_caps, static_caps))
    }

    /// 呼び出し引数の `is_mutable`（`eval_call_args` と同じ判定: 変数 ident は変数の可変性、
    /// それ以外の式は保守的に true）。VM は base ローカルしか読まないので slot_mut で判定できる。
    pub(super) fn arg_is_mutable(&self, e: &Expr) -> bool {
        match e {
            Expr::Ident { res: Resolution::Local(slot), .. } => {
                self.slot_mut.get(*slot as usize).copied().unwrap_or(true)
            }
            Expr::Ident { name, res: Resolution::Unresolved, .. } => self
                .slots
                .get(name)
                .and_then(|&s| self.slot_mut.get(s as usize).copied())
                .unwrap_or(true),
            _ => true,
        }
    }

    /// native の `mut` ポインタ引数の**書き戻し先**を 1 つ解決する（#48）。
    ///
    /// [`store_target`](Self::store_target) と**同じ順序で同じ記憶域を見る**が、
    /// 決められないときに `bail` せず `None` を返す点だけが違う。
    /// ここは「載せられるか」ではなく「書き戻し先が判るか」の判定であり、
    /// 判らない＝**書き戻さない（従来どおり）**で正しく、プログラムを拒否してはいけない。
    ///
    /// ⚠ `static mut` は対象外（`Interpreter::static_cells` を span キーで直読みする別経路。
    /// native 書き戻しの実例が無いので `None` にして黙って見送る）。
    pub(super) fn wb_store_target(&mut self, name: &str) -> Option<crate::vm::chunk::WbStore> {
        use crate::vm::chunk::WbStore;
        // セル変数は slot ではなく共有セル。**slot より先に見る**（`store_target` と同順）。
        if let Some(&i) = self.cells.get(name) {
            return Some(WbStore::Cell(i));
        }
        if self.statics.contains_key(name) {
            return None;
        }
        if let Some(&slot) = self.slots.get(name) {
            return Some(WbStore::Local(slot));
        }
        if self.mode.is_module_body() {
            let ni = self.add_name(name);
            return Some(WbStore::Name(ni));
        }
        // デバッガ REPL／定義文脈は停止フレームの生スコープを見るので `StoreGlobal` では
        // 別の変数を書く（`store_target` が bail するのと同じ理由）。書き戻しは見送る。
        if self.mode.uses_name_lookup() {
            return None;
        }
        let ni = self.add_name(name);
        // `Op::StoreGlobal` と同じく emit 1 回につきキャッシュ枠を 1 本割り当てる。
        let ci = self.chunk.global_caches.len() as u32;
        self.chunk.global_caches.push(crate::ast::SlotCache::default());
        Some(WbStore::Global(ni, ci))
    }

    /// 呼び出し 1 件ぶんの書き戻し先を採取して副表へ記録する（#48）。
    ///
    /// **呼び先が native かどうかはコンパイル時には判らない**ので、`f(mut x)` の形なら
    /// 一律に記録する。実行時に「呼び先が write-back 持ちの `NativeFunction`」だった
    /// ときだけ引かれるので、余分な記録は害にならない（表が少し太るだけ）。
    ///
    /// ⚠ 記録するのは**引数式が識別子**のものだけ。ツリーウォークの
    /// `call_native_function` が `Expr::Ident` のときだけ書き戻すのと同じ規則。
    pub(super) fn record_wb_targets(&mut self, node_id: u32, args: &[CallArg]) {
        // node_id 0 = 未採番（実行時に引けない）。合成ノードなど。
        if node_id == 0 || args.len() > 32 {
            return;
        }
        let mut mask: u32 = 0;
        let mut targets: Vec<(u8, crate::vm::chunk::WbStore)> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let CallArg::Positional(e) = arg else { continue };
            let Expr::Ident { name, .. } = e else { continue };
            if !self.arg_is_mutable(e) {
                continue;
            }
            let name = name.clone();
            if let Some(t) = self.wb_store_target(&name) {
                mask |= 1 << i;
                targets.push((i as u8, t));
            }
        }
        if !targets.is_empty() {
            self.chunk.wb_targets
                .insert(node_id, crate::vm::chunk::WbCall { mask, targets });
        }
    }

    /// 位置引数をスタックへ push し、各引数の is_mutable を bit にした mask を返す。
    /// keyword/可変長引数・33個以上は非対応（`None`）。
    /// 呼び出し引数をスタックへ積み、`(mut_mask, 引数名)` を返す（#27-c）。
    ///
    /// 引数名が全て `None`（＝純粋な位置引数）なら 2 番目は `None` を返し、呼び出し側は
    /// `Op::Call` を使う。1 つでも名前付き・可変長があれば `Some(names)` になり、
    /// **`Op::CallKw` を使える呼び出し形でだけ**受け付ける（それ以外は `no_kw` で bail）。
    ///
    /// 可変長 `f(... = A, B, C)` は要素を積んでから `BuildList` で 1 値に畳む。
    /// `eval_call_args` が作る `(Some("..."), Value::List, true)` と同じ形。
    #[allow(clippy::type_complexity)]
    pub(super) fn compile_call_args(
        &mut self,
        args: &[CallArg],
        wb_node: Option<u32>,
    ) -> Option<(u32, Option<Vec<Option<String>>>)> {
        if args.len() > 32 {
            bail("too-many-args", None);
            return None;
        }
        // native の `mut` ポインタ書き戻し先を副表へ（#48）。呼び先が native か判らないので
        // 「識別子の mut 引数がある呼び出し」を一律に記録する。組み込み・テンプレートは
        // ポインタ引数を取らないので呼び出し側が `None` を渡して記録しない。
        if let Some(node_id) = wb_node {
            self.record_wb_targets(node_id, args);
        }
        let mut mask: u32 = 0;
        let mut names: Vec<Option<String>> = Vec::new();
        let mut any_named = false;
        for (i, arg) in args.iter().enumerate() {
            match arg {
                CallArg::Positional(e) => {
                    if self.arg_is_mutable(e) {
                        mask |= 1 << i;
                    }
                    self.compile_expr(e)?;
                    names.push(None);
                }
                CallArg::Keyword { name, value } => {
                    if self.arg_is_mutable(value) {
                        mask |= 1 << i;
                    }
                    self.compile_expr(value)?;
                    names.push(Some(name.clone()));
                    any_named = true;
                }
                CallArg::Variadic(exprs) => {
                    let n = u16::try_from(exprs.len()).ok()?;
                    for e in exprs {
                        self.compile_expr(e)?;
                    }
                    self.emit(Op::BuildList(n));
                    mask |= 1 << i; // variadic は保守的に mutable 扱い（ツリーウォークと同じ）
                    names.push(Some("...".to_string()));
                    any_named = true;
                }
            }
        }
        Some((mask, any_named.then_some(names)))
    }

    /// `Op::CallKw` を使えない呼び出し形で名前付き引数が来たら bail する（#27-c）。
    pub(super) fn no_kw(kw: Option<Vec<Option<String>>>) -> Option<()> {
        if kw.is_some() {
            bail("call-arg", None);
            return None;
        }
        Some(())
    }

    /// 通常の呼び出しを発行する（#27-c）。名前付き引数の有無で `Call` / `CallKw` を選ぶ。
    pub(super) fn emit_call(
        &mut self,
        argc: usize,
        mask: u32,
        name_idx: u32,
        span_idx: u32,
        node_id: u32,
        kw: Option<Vec<Option<String>>>,
    ) -> Option<()> {
        match kw {
            None => self.emit(Op::Call(argc as u16, mask, name_idx, span_idx, node_id)),
            Some(arg_names) => {
                let i = u32::try_from(self.chunk.kw_calls.len()).ok()?;
                self.chunk.kw_calls.push(crate::vm::chunk::KwCall {
                    argc: u16::try_from(argc).ok()?,
                    mut_mask: mask,
                    name_idx,
                    span_idx,
                    node_id,
                    arg_names,
                });
                self.emit(Op::CallKw(i))
            }
        };
        Some(())
    }

    /// `target <- async->T: body` をコンパイルする（タスク #9）。
    /// 本体が参照する enclosing フレームの slot（`collect_referenced_names ∩ slots`）を捕捉対象に記録し、
    /// マネージャをスタックへロードして `AsyncSubmit(idx)` を発行する。実行時は frame から捕捉値を読み、
    /// グローバルと合わせて `capture_env` で env を組む（ツリーウォークの `exec_async_assign` と同一）。
    /// 捕捉は「本体が参照する slot」に限定（未参照ローカルは env に載せない）＝task 挙動は byte-identical。
    pub(super) fn compile_async_assign(&mut self, target: &str, stmts: &[Stmt]) -> Option<()> {
        // 本体の参照名を収集し、enclosing frame の slot と交差したものを捕捉する。
        let mut refs: HashSet<String> = HashSet::new();
        crate::interpreter::collect_referenced_names(stmts, &mut refs);
        let mut captures: Vec<(String, u16, bool)> = refs
            .iter()
            .filter_map(|name| {
                // ⚠ ここは `slot_of` を使わない。**当たらないのが正常**（グローバル参照は
                // 捕捉対象外）で、`slot_of` を通すと計測に幻の bail が 35 件載る。
                // 「対象外」と「失敗」を同じ `None` で表さないこと（#27-c で再発）。
                let &slot = self.slots.get(name)?;
                let is_mut = self.slot_mut.get(slot as usize).copied().unwrap_or(false);
                Some((name.clone(), slot, is_mut))
            })
            .collect();
        // 決定的順序（HashSet は非決定）: slot 昇順。env の順序は task 挙動に影響しないが再現性のため固定。
        captures.sort_by_key(|(_, slot, _)| *slot);

        let idx = u32::try_from(self.chunk.async_blocks.len()).ok()?;
        self.chunk.async_blocks.push(crate::vm::chunk::AsyncBlock {
            body: stmts.to_vec(),
            captures,
        });
        // マネージャ値をスタックへ（ローカル slot 優先、なければグローバル名引き）。
        if let Some(&slot) = self.slots.get(target) {
            self.emit(Op::LoadLocal(slot));
        } else {
            self.emit_load_global(target);
        }
        self.emit(Op::AsyncSubmit(idx));
        Some(())
    }
}
