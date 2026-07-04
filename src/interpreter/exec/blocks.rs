// exec/blocks.rs — ブロック実行とクロージャ補助: exec_block / exec_scoped_block / capture_env / apply_value_call。

#[allow(unused_imports)]
use {
    std::cell::RefCell, std::collections::{HashMap, HashSet}, std::path::PathBuf,
    std::rc::Rc, std::sync::Arc,
    crate::ast::{
        Accessibility, BinOp, ExceptHandler, Expr, FieldKind, MatchArm, MatchPattern, Param,
        Stmt, TemplateParam, TupleTarget,
    },
    crate::token::Span,
    crate::interpreter::{
        debugger::DbgMode, CapturedVar, ExecResult, FnValue, GeneratorFnValue, GeneratorState,
        Interpreter, ModuleState, NamespaceData, NativeFnRef, NativeLibWrapper, RaisedError,
        StackFrame, TemplateClassValue, TemplateFnValue, TemplateGenFnValue, Value, Var,
        BLOCK_RETURN_EXPECTED_TYPE, BLOCK_YIELDS, BREAK_SENTINEL, GENERATOR_YIELDS, LOOP_DEPTH,
        RAISE_SENTINEL,
    },
};
#[allow(unused_imports)]
use super::*;

impl Interpreter {
    /// 文のリストを順に実行する。Normal 以外のシグナルが発生したら即返す。
    pub(crate) fn exec_block(&mut self, stmts: &[Stmt]) -> Result<ExecResult, String> {
        for stmt in stmts {
            match self.exec(stmt)? {
                ExecResult::Normal => {}
                signal => return Ok(signal),
            }
        }
        Ok(ExecResult::Normal)
    }

    /// 新しいスコープを積んでから文のリストを実行し、完了後にスコープを取り除く。
    pub(crate) fn exec_scoped_block(&mut self, stmts: &[Stmt]) -> Result<ExecResult, String> {
        self.push_scope();
        let result = self.exec_block(stmts);
        self.pop_scope();
        result
    }

    // ---------------------------------------------------------------------------
    // Closure capture
    // ---------------------------------------------------------------------------

    /// 関数本体のフリー変数を分析して、現在の非グローバルスコープからキャプチャ環境を構築する。
    pub(crate) fn capture_env(
        &mut self,
        body: &[Stmt],
        params: &[crate::ast::Param],
    ) -> HashMap<String, CapturedVar> {
        let mut own_names: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
        collect_declared_names(body, &mut own_names);

        let mut referenced: HashSet<String> = HashSet::new();
        collect_referenced_names(body, &mut referenced);

        let free_vars: Vec<String> = referenced
            .into_iter()
            .filter(|n| !own_names.contains(n))
            .collect();

        let mut captured: HashMap<String, CapturedVar> = HashMap::new();
        let n_scopes = self.scopes.len();

        for name in &free_vars {
            for scope_idx in (1..n_scopes).rev() {
                let found = self.scopes[scope_idx]
                    .get(name.as_str())
                    .map(|var| (var.is_mutable(), var.cell(), var.get_value()));

                if let Some((is_mutable, existing_cell, current_value)) = found {
                    if is_mutable {
                        let cell = if let Some(cell) = existing_cell {
                            cell
                        } else {
                            let cell = Rc::new(RefCell::new(current_value));
                            // Upgrade Mutable → Cell so the outer scope shares the same Rc.
                            if let Some(var) = self.scopes[scope_idx].get_mut(name.as_str()) {
                                *var = Var::Cell(cell.clone());
                            }
                            cell
                        };
                        captured.insert(name.clone(), CapturedVar::Mutable(cell));
                    } else {
                        captured.insert(
                            name.clone(),
                            CapturedVar::Immutable(Self::deep_copy_value(current_value)),
                        );
                    }
                    break;
                }
            }
        }

        captured
    }

    /// 評価済みの値 `callee` を単一の評価済み引数 `arg` で呼び出す（デコレータ適用用）。
    pub(crate) fn apply_value_call(
        &mut self,
        callee: Value,
        arg: Value,
        label: &str,
    ) -> Result<Value, String> {
        let evaled: Vec<(Option<String>, Value, bool)> = vec![(None, arg, true)];
        match callee {
            Value::Function(fn_val) => self.exec_fn_evaled(fn_val, &evaled, None, label, None),
            Value::OverloadedFn(candidates) => {
                self.dispatch_overload_evaled(candidates, evaled, None, label, None)
            }
            Value::Class(cls) => self.instantiate_evaled(cls, evaled),
            Value::Instance(ref inst_rc) => {
                let class = inst_rc.borrow().class.clone();
                let overloads =
                    self.lookup_method_in_class(&class, "__call__")
                        .ok_or_else(|| {
                            format!(
                                "TypeError: '{}' object is not callable (no __call__ method)",
                                class.name
                            )
                        })?;
                if overloads.len() == 1 {
                    self.exec_fn_evaled(overloads[0].clone(), &evaled, Some(callee), "__call__", None)
                } else {
                    self.dispatch_overload_evaled(overloads, evaled, Some(callee), "__call__", None)
                }
            }
            other => Err(format!(
                "TypeError: '{}' object is not callable as decorator",
                self.type_name(&other)
            )),
        }
    }
}
