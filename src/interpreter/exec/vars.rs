// exec/vars.rs — 変数宣言・代入の実行: let / タプル束縛 / static / 複合代入 / loop_yield とスロットキャッシュ。

use {
    std::cell::RefCell,
    std::rc::Rc,
    crate::ast::{
        BinOp, Expr, TupleTarget,
    },
    crate::token::Span,
    crate::interpreter::{
        ExecResult,
        Interpreter, Value, Var,
        BLOCK_RETURN_EXPECTED_TYPE, BLOCK_YIELDS,
    },
};

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
        let source_var = if let Expr::Ident { name: src, .. } = expr {
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
        self.exec_let_tuple_evaled(targets, val)
    }

    /// 評価済みの値でタプル分解宣言を行う（最上位 VM の `Op::LetTuple` 用・#27-c）。
    /// 束縛先が現スコープ（＝グローバル）の場合。
    pub(crate) fn exec_let_tuple_evaled(
        &mut self,
        targets: &[TupleTarget],
        val: Value,
    ) -> Result<ExecResult, String> {
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
        for (i, v) in self.let_tuple_values(targets, val)? {
            let mutable = matches!(targets[i], TupleTarget::Mut(_));
            let name = match &targets[i] {
                TupleTarget::Let(n) | TupleTarget::Bare(n) | TupleTarget::Mut(n) => n.clone(),
                TupleTarget::Wildcard => continue,
            };
            self.declare_var(name, Var::new(v, mutable));
        }
        Ok(ExecResult::Normal)
    }

    /// タプル分解の**検査と値の取り出し**（#27-c）。
    ///
    /// ツリーウォーク（`exec_let_tuple`）と VM（`Op::LetTuple` の両経路）の**唯一の実装**。
    /// 型検査・要素数検査・エラー文言・`let` の freeze / `mut` の deep_copy がここに集約されている。
    /// 戻り値は `(targets の index, 束縛する値)` を宣言順に並べたもの（`Wildcard` 以降は打ち切り）。
    ///
    /// ⚠ 「既に宣言済み」検査は**ここに入れない**。最上位はスコープに宣言するので必要だが、
    /// VM のフラット slot には宣言集合が無く、ツリーウォークも反復ごとにスコープを push し直す
    /// ため入れ子では発生しない。束縛先を知っている呼び出し側の責務。
    pub(crate) fn let_tuple_values(
        &mut self,
        targets: &[TupleTarget],
        val: Value,
    ) -> Result<Vec<(usize, Value)>, String> {
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
        let mut out = Vec::with_capacity(named);
        let mut idx = 0usize;
        for (i, target) in targets.iter().enumerate() {
            match target {
                TupleTarget::Wildcard => break,
                TupleTarget::Let(_) | TupleTarget::Bare(_) => {
                    let v = tuple_rc.get(idx).unwrap().clone();
                    self.apply_freeze_to_value(&v, false)?;
                    out.push((i, v));
                    idx += 1;
                }
                TupleTarget::Mut(_) => {
                    let v = Self::deep_copy_value(tuple_rc.get(idx).unwrap().clone());
                    out.push((i, v));
                    idx += 1;
                }
            }
        }
        Ok(out)
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
        // 現関数のローカル（frame_floor..）に同名があればグローバル解決ではない
        if self.scopes[self.frame_floor..].iter().any(|s| s.contains_key(name)) {
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

        // `->list[T]` アノテーションの要素型に対する検査（#35 で VM と 1 実装に集約）。
        let expected = BLOCK_RETURN_EXPECTED_TYPE.with(|t| t.borrow().last().cloned().flatten());
        if let Some(ref ann) = expected {
            self.check_loop_yield_type(&val, ann)?;
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
