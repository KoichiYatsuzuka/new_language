// exec/vars.rs — 変数宣言・代入の実行: let / タプル束縛 / static / 複合代入 / loop_yield とスロットキャッシュ。

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
    /// `let` 宣言を実行する。
    pub(crate) fn exec_let(&mut self, name: &str, expr: &Expr) -> Result<ExecResult, String> {
        if name != "_" && self.get_var(name).is_some() {
            return Err(format!("NameError: variable '{name}' is already declared"));
        }
        // mut → let: deep copy してからフリーズする。
        // let → let: そのまま代入（コピー不要・再フリーズ不要）。
        // 式 → let: Instance の場合は deep copy してからフリーズする。
        //   可変コレクション (list[i] など) から取り出した Instance を直接フリーズすると
        //   共有 Rc を通じて元のオブジェクトまで不変化されてしまうため、コピーが必要。
        let source_var = if let Expr::Ident(src) = expr {
            self.get_var(src)
                .map(|v| (v.is_mutable(), v.cell().is_some()))
        } else {
            None
        };
        let value = self.eval(expr)?;
        let value = match source_var {
            Some((true, _)) => {
                // mut 変数から let へ: 深いコピーを作成してフリーズする。
                let copied = Self::deep_copy_value(value);
                self.apply_freeze_to_value(&copied, true)?;
                copied
            }
            Some((false, _)) => value,
            None => {
                // 識別子以外の式 (例: list[i], SomeClass()) から let へ。
                // Instance の場合は深いコピーを作成してからフリーズする。
                // これにより元のオブジェクト (例: リスト内のカード) の可変性が保たれる。
                if matches!(value, Value::Instance(_)) {
                    let copied = Self::deep_copy_value(value);
                    self.apply_freeze_to_value(&copied, true)?;
                    copied
                } else {
                    value
                }
            }
        };
        self.declare_var(name.to_string(), Var::new(value, false));
        Ok(ExecResult::Normal)
    }

    /// タプル分解を伴う `let (a, b, ...) = expr` 宣言を実行する。
    pub(crate) fn exec_let_tuple(
        &mut self,
        targets: &[TupleTarget],
        value: &Expr,
    ) -> Result<ExecResult, String> {
        let val = self.eval(value)?;
        let tuple_rc = match val {
            Value::Tuple(rc) => rc,
            _ => {
                return Err(
                    "TypeError: cannot unpack non-tuple value in tuple assignment".to_string(),
                )
            }
        };
        let has_wildcard = targets.iter().any(|t| matches!(t, TupleTarget::Wildcard));
        let named = targets
            .iter()
            .filter(|t| !matches!(t, TupleTarget::Wildcard))
            .count();
        let tlen = tuple_rc.len();
        if has_wildcard {
            if named > tlen {
                return Err(format!(
                    "TypeError: not enough values to unpack (need at least {named}, got {tlen})"
                ));
            }
        } else if named != tlen {
            return Err(format!(
                "TypeError: not enough values to unpack (expected {named}, got {tlen})"
            ));
        }
        for target in targets.iter() {
            match target {
                TupleTarget::Let(n) | TupleTarget::Bare(n) | TupleTarget::Mut(n) => {
                    if n != "_" && self.get_var(n).is_some() {
                        return Err(format!("NameError: variable '{n}' is already declared"));
                    }
                }
                TupleTarget::Wildcard => {}
            }
        }
        let mut idx = 0usize;
        for target in targets.iter() {
            match target {
                TupleTarget::Wildcard => break,
                TupleTarget::Let(n) | TupleTarget::Bare(n) => {
                    let v = tuple_rc.get(idx).unwrap().clone();
                    self.apply_freeze_to_value(&v, false)?;
                    self.declare_var(n.clone(), Var::new(v, false));
                    idx += 1;
                }
                TupleTarget::Mut(n) => {
                    let v = Self::deep_copy_value(tuple_rc.get(idx).unwrap().clone());
                    self.declare_var(n.clone(), Var::new(v, true));
                    idx += 1;
                }
            }
        }
        Ok(ExecResult::Normal)
    }

    /// `static mut` 変数宣言を実行する。ソース位置をキーに静的セルを確保し、呼び出し間で値を共有する。
    pub(crate) fn exec_static_var(
        &mut self,
        name: &str,
        expr: &Expr,
        span: &Span,
    ) -> Result<ExecResult, String> {
        let key = (span.file.to_string(), span.line, span.col);
        let cell = if let Some(existing) = self.static_cells.get(&key) {
            existing.clone()
        } else {
            let value = self.eval(expr)?;
            let new_cell = Rc::new(RefCell::new(value));
            self.static_cells.insert(key, new_cell.clone());
            new_cell
        };
        self.declare_var(name.to_string(), Var::new_cell(cell));
        Ok(ExecResult::Normal)
    }

    /// `+=` / `-=` などの複合代入文を実行する。変数の可変性を確認してから演算結果を書き戻す。
    pub(crate) fn exec_compound_assign(
        &mut self,
        name: &str,
        op: &BinOp,
        value: &Expr,
        slot: &crate::ast::SlotCache,
    ) -> Result<ExecResult, String> {
        // スロットキャッシュ命中: 読み・書きともスコープ検索なしの直接セルアクセス
        if let Some(idx) = slot.get(self.slot_epoch) {
            let rhs = self.eval(value)?;
            let cell = self.global_slot_cells[idx].clone();
            let lhs = cell.borrow().clone();
            let result = self.apply_binop_dyn(op, lhs, rhs)?;
            *cell.borrow_mut() = result;
            return Ok(ExecResult::Normal);
        }
        let rhs = self.eval(value)?;
        let lhs = match self.get_var(name) {
            Some(v) if !v.is_mutable() => {
                return Err(format!(
                    "TypeError: cannot assign to immutable variable '{name}'"
                ));
            }
            Some(v) => v.get_value(),
            None => return Err(format!("NameError: '{name}' is not defined")),
        };
        let value = self.apply_binop_dyn(op, lhs, rhs)?;
        self.assign_var(name, value)?;
        self.try_fill_slot(name, slot);
        Ok(ExecResult::Normal)
    }

    /// 代入文のスロットキャッシュ充填を試みる（変数のスロット化）。
    ///
    /// 条件: `name` がローカルスコープ（1..）に存在せず、グローバルスコープ（0）の
    /// 可変変数として解決されること。`Var::Mutable` は `Var::SlotCell` に昇格し、
    /// セルを `global_slot_cells` レジストリに登録してインデックスを AST に焼き込む。
    ///
    /// 健全性: グローバルは再宣言禁止のためバインディングは固定。`freeze` は
    /// `make_var_immutable` が `SlotCell` を `Immutable` に降格させ `slot_epoch` を
    /// 進めるので、全キャッシュが自動失効する。
    pub(crate) fn try_fill_slot(&mut self, name: &str, slot: &crate::ast::SlotCache) {
        // ローカルスコープに同名があればグローバル解決ではない
        if self.scopes[1..].iter().any(|s| s.contains_key(name)) {
            return;
        }
        let Some(var) = self.scopes[0].get_mut(name) else {
            return;
        };
        let cell = match var {
            Var::SlotCell(rc) | Var::Cell(rc) => rc.clone(),
            Var::Mutable(v) => {
                let rc = Rc::new(RefCell::new(std::mem::replace(v, Value::None)));
                *var = Var::SlotCell(rc.clone());
                rc
            }
            Var::Immutable(_) => return,
        };
        let idx = self.global_slot_cells.len();
        if idx >= (u32::MAX - 1) as usize {
            return;
        }
        self.global_slot_cells.push(cell);
        slot.fill(self.slot_epoch, idx as u32);
    }

    // ---------------------------------------------------------------------------
    // Control flow signals
    // ---------------------------------------------------------------------------

    /// `loop_yield expr` 文を実行する。for/while 式の中で値を蓄積する制御フロー信号。
    pub(crate) fn exec_loop_yield(&mut self, expr: &Expr) -> Result<ExecResult, String> {
        let val = self.eval(expr)?;

        // Type-check the yielded value against the element type from a `->list[T]` annotation.
        let expected = BLOCK_RETURN_EXPECTED_TYPE.with(|t| t.borrow().last().cloned().flatten());
        if let Some(ref ann) = expected {
            if let Some(elem_type) = extract_list_elem_type(ann) {
                if !self.value_matches_type_ann(&val, elem_type) {
                    return Err(format!(
                        "TypeError: loop_yield value has type '{}', but element type '{}' was expected (from ->{})",
                        self.type_name(&val), elem_type, ann
                    ));
                }
            }
        }

        let mut in_loop_expr = false;
        BLOCK_YIELDS.with(|y| {
            if let Some(yields) = y.borrow_mut().as_mut() {
                yields.push(val);
                in_loop_expr = true;
            }
        });
        if !in_loop_expr {
            return Err("SyntaxError: 'loop_yield' can only be used inside a for/while expression (with ->list[T] annotation)".to_string());
        }
        Ok(ExecResult::Normal)
    }

    // ---------------------------------------------------------------------------
    // Control flow structures
    // ---------------------------------------------------------------------------

}
