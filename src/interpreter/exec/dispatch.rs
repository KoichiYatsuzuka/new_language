// exec/dispatch.rs — exec のメインディスパッチャ: 文の種類に応じて各専用メソッドへ委譲する。

// ⚠ #58 で `Stmt::Import` の本体（210 行）を `exec/modules.rs` へ移したので、
// このファイルは**もう import 関連の型を一切知らない**（`PathBuf` / `ModuleState` /
// `Value` / `use super::*` はそこでだけ要るものだった）。**ディスパッチ表に必要な物だけ**を残す。
use {
    crate::ast::Stmt,
    crate::interpreter::{
        debugger::DbgMode, ExecResult,
        Interpreter, Var,
        GENERATOR_YIELDS,
    },
};

impl Interpreter {
    /// 文（`Stmt`）を実行して `ExecResult` を返す。各 Stmt バリアントを専用メソッドに委譲する。
    pub fn exec(&mut self, stmt: &Stmt) -> Result<ExecResult, String> {
        // 実行時間分布の計測（`--features prof`）: ここから先は「VM の外」。
        // ⚠ 既定ビルドでは消える（#10-a: env 判定だけにすると 11% 退行する）。
        #[cfg(feature = "prof")]
        crate::prof::note_outside();

        // Step-mode check: pause before this statement if the debugger asked us to.
        // Skip the check when we're already inside a break_point (would re-enter).
        if crate::interpreter::debugger::DBG_MODE.with(|m| *m.borrow() != DbgMode::Inactive) {
            if let Some(span) = self.should_pause_at(stmt) {
                self.exec_breakpoint(&span)?;
            }
        }

        // 最上位の文は VM で回せることがある（#10-b/#10-c/#10-c2）。
        //
        // ⚠ **各アームに分散させず、ここ 1 箇所に置くこと。** 対象の文種別が増えるたびに
        // アームへ足していくと同じ 3 行が 10 箇所以上に散る。
        // ⚠ **デバッガの `should_pause_at` より後**であること（#1 で直した既存バグと同じ形で、
        // 先に置くと off/auto でステッピングが食い違う）。
        // ⚠ `toplevel_vm_candidate` は**フィールド 3 本の比較だけ**（`#[inline(always)]`）。
        // `exec()` は全文で呼ばれるので、ここに重い判定を足すと全体が遅くなる（#10-a で 11% 実測）。
        if self.toplevel_vm_candidate() {
            if let Some(r) = self.try_run_toplevel_stmt(stmt)? {
                return Ok(r);
            }
        }

        // 診断フック（#10）: ツリーウォークが実際に実行している文を数える。
        // ⚠ **VM 試行より後に置くこと。** 前に置くと「VM へ渡した文」まで数えてしまい、
        // 「ツリーウォークの負荷」という指標の意味が崩れる（#10-c2 で 1,368 件を過大計上した）。
        if crate::interpreter::tw_stats::enabled() {
            crate::interpreter::tw_stats::record_stmt(stmt);
        }

        match stmt {
            Stmt::Expr(expr) => {
                self.eval(expr)?;
                Ok(ExecResult::Normal)
            }
            Stmt::Let(name, _, expr) => self.exec_let(name, expr),
            Stmt::Const(name, _, expr) => {
                if name != "_" && self.get_var(name).is_some() {
                    return Err(format!("NameError: variable '{name}' is already declared"));
                }
                let value = self.eval(expr)?;
                self.declare_var(name.clone(), Var::new(value, false));
                Ok(ExecResult::Normal)
            }
            Stmt::Mut(name, _, expr) => {
                if name != "_" && self.get_var(name).is_some() {
                    return Err(format!("NameError: variable '{name}' is already declared"));
                }
                let value = Self::deep_copy_value(self.eval(expr)?);
                self.declare_var(name.clone(), Var::new(value, true));
                Ok(ExecResult::Normal)
            }
            Stmt::LetTuple { targets, value, .. } => self.exec_let_tuple(targets, value),
            Stmt::Static(name, expr, span) => self.exec_static_var(name, expr, span),
            Stmt::Assign { name, value, slot, .. } => {
                // スロットキャッシュ命中: スコープ検索なしの直接セル書き込み
                if let Some(idx) = slot.get(self.slot_epoch) {
                    let value = self.eval(value)?;
                    *self.global_slot_cells[idx].borrow_mut() = value;
                    return Ok(ExecResult::Normal);
                }
                let value = self.eval(value)?;
                self.assign_var(name, value)?;
                self.try_fill_slot(name, slot);
                Ok(ExecResult::Normal)
            }
            Stmt::AttrAssign { target, value } => {
                let rhs = self.eval(value)?;
                self.attr_assign(target, rhs)?;
                Ok(ExecResult::Normal)
            }
            Stmt::AttrCompoundAssign { target, op, value } => {
                let rhs = self.eval(value)?;
                let lhs = self.eval(target)?;
                let result = self.apply_binop_dyn(op, lhs, rhs)?;
                self.attr_assign(target, result)?;
                Ok(ExecResult::Normal)
            }
            Stmt::CompoundAssign {
                name, op, value, slot, ..
            } => self.exec_compound_assign(name, op, value, slot),
            Stmt::Pass => Ok(ExecResult::Normal),
            Stmt::Field { .. } => Ok(ExecResult::Normal),
            // #33: 制御フロー文は**必ずバイトコード VM が実行する**（ツリーウォークの実装は削除した）。
            // 入口の一覧と根拠は `eval()` の同じアーム（`eval/core.rs`）を参照。
            // ⚠ ここへ来たら配線の穴なので、黙って動かず落とす。
            Stmt::Break
            | Stmt::Continue
            | Stmt::Return(_)
            | Stmt::BlockReturn(..)
            | Stmt::LoopYield(_)
            | Stmt::If { .. }
            | Stmt::Match { .. }
            | Stmt::While { .. }
            | Stmt::For { .. }
            | Stmt::Block(_)
            | Stmt::Try { .. } => Err(format!(
                "VmForceError: control-flow statement `{}` reached the tree-walk executor",
                crate::interpreter::tw_stats::stmt_kind_of(stmt)
            )),
            Stmt::FnDef {
                name,
                template_params,
                params,
                body,
                decorators,
                return_type,
                ..
            } => self.exec_fn_def(name, template_params, params, body, decorators, return_type.as_deref()),
            Stmt::Yield(expr) => {
                let val = self.eval(expr)?;
                GENERATOR_YIELDS.with(|y| {
                    if let Some(yields) = y.borrow_mut().as_mut() {
                        yields.push(val.clone());
                    }
                });
                Ok(ExecResult::Normal)
            }
            Stmt::GenDef {
                name,
                template_params,
                params,
                body,
                ..
            } => self.exec_gen_def(name, template_params, params, body),
            Stmt::TraitDef { name, body, .. } => self.exec_trait_def(name, body),
            Stmt::ProtocolDef { name, body } => self.exec_protocol_def(name, body),
            Stmt::NewTypeDef { name, original } => self.exec_new_type_def(name, original),
            Stmt::EnumDef { name, variants } => self.exec_enum_def(name, variants),
            Stmt::ClassDef {
                name,
                template_params,
                bases,
                body,
                decorators,
            } => self.exec_class_def(name, template_params, bases, body, decorators),
            Stmt::Freeze(name, span) => self.exec_freeze(name, span),
            Stmt::Raise { exc, span } => self.exec_raise(exc, span),

            // ⚠ 本体は `exec/modules.rs` の `exec_import`（#58 で 210 行のアームを切り出した）。
            Stmt::Import {
                lang,
                module,
                with_file,
                alias,
                body,
            } => self.exec_import(lang, module, with_file.as_deref(), alias.as_deref(), body),
            Stmt::FromImport {
                lang,
                module,
                with_file: _,
                names,
                body,
            } => {
                let ns = self.exec_module(lang, module, body)?;
                for (orig_name, alias) in names {
                    let bind_name = alias.clone().unwrap_or_else(|| orig_name.clone());
                    let val = ns.members.get(orig_name.as_str()).cloned().ok_or_else(|| {
                        format!(
                            "ImportError: cannot import name '{}' from '{}'",
                            orig_name,
                            module.join(".")
                        )
                    })?;
                    self.declare_var(bind_name, Var::new(val, false));
                }
                Ok(ExecResult::Normal)
            }
            Stmt::AsyncAssign { target, stmts, .. } => self.exec_async_assign(target, stmts),
            Stmt::BreakPoint { span } => self.exec_breakpoint(span),
            Stmt::DebugLet(name, expr) => {
                let value = self.eval(expr)?;
                self.dbg.vars.insert(name.clone(), Var::new(value, false));
                Ok(ExecResult::Normal)
            }
            Stmt::EventSubscribe {
                source,
                handler,
                is_once,
                is_async,
                ..
            } => self.exec_event_subscribe(source, handler, *is_once, *is_async),
            Stmt::EventUnsubscribe {
                source, handler, ..
            } => self.exec_event_unsubscribe(source, handler),
        }
    }

    // ---------------------------------------------------------------------------
    // Variable declarations & assignment
    // ---------------------------------------------------------------------------

}
