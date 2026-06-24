// exec.rs — 文の実行 (exec / exec_block / exec_scoped_block)
//
// `Interpreter::exec` が文（`Stmt`）を再帰的にツリーウォークして `ExecResult` を返す。
// 変数宣言・代入・制御構造・関数/クラス定義・例外処理など、すべての文の実行を担当する。
//
// exec() はディスパッチャとして機能し、各カテゴリの処理は専用メソッドに委譲する:
//   - exec_let / exec_let_tuple / exec_static_var / exec_compound_assign  (変数宣言・代入)
//   - exec_loop_yield                                                       (制御フロー信号)
//   - exec_if_stmt / exec_match_stmt / exec_while_stmt / exec_for_stmt /
//     exec_block_stmt                                                        (制御構造)
//   - exec_fn_def / exec_gen_def                                            (関数定義)
//   - exec_trait_def / exec_new_type_def / exec_enum_def / exec_class_def  (型定義)
//   - exec_freeze / exec_raise / exec_try                                   (例外処理)
//   - exec_async_assign                                                      (非同期)

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use crate::ast::{
    Accessibility, BinOp, ExceptHandler, Expr, FieldKind, MatchArm, MatchPattern, Param, Stmt,
    TemplateParam, TupleTarget,
};
use crate::token::Span;

use super::{
    debugger::DbgMode, CapturedVar, ExecResult, FnValue, GeneratorFnValue, GeneratorState,
    Interpreter, ModuleState, NamespaceData, NativeFnRef, NativeLibWrapper, RaisedError,
    StackFrame, TemplateClassValue, TemplateFnValue, TemplateGenFnValue, Value, Var,
    BLOCK_RETURN_EXPECTED_TYPE, BLOCK_YIELDS, BREAK_SENTINEL, GENERATOR_YIELDS, LOOP_DEPTH,
    RAISE_SENTINEL,
};

/// `"list[T]"` からアイテム型 `"T"` を取り出す。`"list"` や他の型は `None` を返す。
fn extract_list_elem_type(ann: &str) -> Option<&str> {
    let inner = ann.strip_prefix("list[")?.strip_suffix(']')?;
    Some(inner.trim())
}

/// `x.is_OK()` / `x.is_ERR()` の形式の式から `(変数名, is_ok_flag)` を抽出する。
/// Result ガード節の変数バインディングに使う。
fn extract_result_guard_call(cond: &Expr) -> Option<(String, bool)> {
    if let Expr::Call { func, args, .. } = cond {
        if !args.is_empty() {
            return None;
        }
        if let Expr::Attr { object, attr, .. } = func.as_ref() {
            if let Expr::Ident(var_name) = object.as_ref() {
                match attr.as_str() {
                    "is_OK" => return Some((var_name.clone(), true)),
                    "is_ERR" => return Some((var_name.clone(), false)),
                    _ => {}
                }
            }
        }
    }
    None
}

impl Interpreter {
    /// 文（`Stmt`）を実行して `ExecResult` を返す。各 Stmt バリアントを専用メソッドに委譲する。
    pub fn exec(&mut self, stmt: &Stmt) -> Result<ExecResult, String> {
        // Step-mode check: pause before this statement if the debugger asked us to.
        // Skip the check when we're already inside a break_point (would re-enter).
        if super::debugger::DBG_MODE.with(|m| *m.borrow() != DbgMode::Inactive) {
            if let Some(span) = self.should_pause_at(stmt) {
                self.exec_breakpoint(&span)?;
            }
        }

        match stmt {
            Stmt::Expr(expr) => {
                self.eval(expr)?;
                Ok(ExecResult::Normal)
            }
            Stmt::Let(name, _, expr) => self.exec_let(name, expr),
            Stmt::Const(name, _, expr) => {
                let value = self.eval(expr)?;
                self.declare_var(name.clone(), Var::new(value, false));
                Ok(ExecResult::Normal)
            }
            Stmt::Mut(name, _, expr) => {
                let value = Self::deep_copy_value(self.eval(expr)?);
                self.declare_var(name.clone(), Var::new(value, true));
                Ok(ExecResult::Normal)
            }
            Stmt::LetTuple { targets, value, .. } => self.exec_let_tuple(targets, value),
            Stmt::Static(name, expr, span) => self.exec_static_var(name, expr, span),
            Stmt::Assign { name, value, .. } => {
                let value = self.eval(value)?;
                self.assign_var(name, value)?;
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
                name, op, value, ..
            } => self.exec_compound_assign(name, op, value),
            Stmt::Pass => Ok(ExecResult::Normal),
            Stmt::Field { .. } => Ok(ExecResult::Normal),
            Stmt::Break => {
                if !LOOP_DEPTH.with(|d| *d.borrow() > 0) {
                    return Err("SyntaxError: 'break' outside for/while loop".to_string());
                }
                Ok(ExecResult::Break)
            }
            Stmt::Continue => {
                if !LOOP_DEPTH.with(|d| *d.borrow() > 0) {
                    return Err("SyntaxError: 'continue' outside for/while loop".to_string());
                }
                Ok(ExecResult::Continue)
            }
            Stmt::Return(expr) => {
                let val = match expr {
                    Some(e) => self.eval(e)?,
                    None => Value::None,
                };
                Ok(ExecResult::Return(val))
            }
            Stmt::BlockReturn(expr, _span) => {
                let val = self.eval(expr)?;
                let expected =
                    BLOCK_RETURN_EXPECTED_TYPE.with(|t| t.borrow().last().cloned().flatten());
                if let Some(ann) = expected {
                    self.check_block_return_type(&val, &ann)?;
                }
                Ok(ExecResult::BlockReturn(val))
            }
            Stmt::LoopYield(expr) => self.exec_loop_yield(expr),
            Stmt::If {
                branches,
                else_body,
            } => self.exec_if_stmt(branches, else_body),
            Stmt::Match { subject, arms, .. } => self.exec_match_stmt(subject, arms),
            Stmt::While { cond, body } => self.exec_while_stmt(cond, body),
            Stmt::For {
                targets,
                iter,
                body,
            } => self.exec_for_stmt(targets, iter, body),
            Stmt::Block(body) => self.exec_block_stmt(body),
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
            Stmt::Try {
                body,
                handlers,
                finally_body,
            } => self.exec_try(body, handlers, finally_body),
            Stmt::Import {
                lang,
                module,
                with_file,
                alias,
                body,
            } => {
                let ns = if lang == "cpp-dll" || lang == "cpp-lib" {
                    let file_path = module.first().map(|s| s.as_str()).unwrap_or("");
                    let cache_key = (lang.clone(), PathBuf::from(file_path));
                    if let Some(ModuleState::Loaded(cached)) =
                        self.module_cache.get(&cache_key).cloned()
                    {
                        cached
                    } else {
                        self.module_cache
                            .insert(cache_key.clone(), ModuleState::Loading);
                        let ns = self.load_cpp_module(lang, file_path, with_file.as_deref())?;
                        self.module_cache
                            .insert(cache_key, ModuleState::Loaded(ns.clone()));
                        ns
                    }
                } else if lang == "cs-dll" {
                    let stub_ns = self.exec_module(lang, module, body)?;
                    // Locate the NativeAOT bridge DLL next to the managed DLL.
                    let managed_name = module.last().unwrap();
                    let native_dll_name = format!("{managed_name}_native.dll");
                    let sub_dir: PathBuf = module[..module.len().saturating_sub(1)].iter().collect();
                    let bridge_path = {
                        let mut found: Option<PathBuf> = None;
                        for search_dir in &self.python_search_dirs {
                            let c = search_dir.join(&sub_dir).join(&native_dll_name);
                            if c.exists() { found = Some(c); break; }
                            let c2 = search_dir.join(&native_dll_name);
                            if c2.exists() { found = Some(c2); break; }
                        }
                        if found.is_none() {
                            let c = sub_dir.join(&native_dll_name);
                            if c.exists() { found = Some(c); }
                        }
                        if found.is_none() {
                            let c = PathBuf::from(&native_dll_name);
                            if c.exists() { found = Some(c); }
                        }
                        found.map(std::rc::Rc::new)
                    };
                    if let Some(ref bp) = bridge_path {
                        if let Err(e) = super::cs_dll_runtime::load_bridge(bp.as_ref()) {
                            eprintln!("Warning: cs-dll bridge not loaded: {e}");
                        } else {
                            let bp_str = bp.to_string_lossy().into_owned();
                            let mut patched = (*stub_ns).clone();
                            for val in patched.members.values_mut() {
                                if let Value::Class(cls) = val {
                                    let mut new_cls = cls.deep_clone();
                                    new_cls.class_vars.insert(
                                        "__cs_bridge_path__".to_string(),
                                        Value::Str(bp_str.clone()),
                                    );
                                    *val = Value::Class(std::rc::Rc::new(new_cls));
                                }
                            }
                            return {
                                let bind_name = alias.clone().unwrap_or_else(|| module.last().unwrap().clone());
                                self.declare_var(bind_name, Var::new(Value::Namespace(std::rc::Rc::new(patched)), false));
                                Ok(ExecResult::Normal)
                            };
                        }
                    }
                    stub_ns
                } else if lang == "cs-proc" {
                    let stub_ns = self.exec_module(lang, module, body)?;
                    // Locate the cs-proc host executable.
                    // Searches for {Name}_proc.exe first, then {Name}.exe.
                    let managed_name = module.last().unwrap();
                    let sub_dir: PathBuf = module[..module.len().saturating_sub(1)].iter().collect();
                    let proc_path = {
                        let candidates_names = [
                            format!("{managed_name}_proc.exe"),
                            format!("{managed_name}.exe"),
                        ];
                        let mut found: Option<PathBuf> = None;
                        'outer: for name in &candidates_names {
                            for search_dir in &self.python_search_dirs {
                                let c = search_dir.join(&sub_dir).join(name);
                                if c.exists() { found = Some(c); break 'outer; }
                                let c2 = search_dir.join(name);
                                if c2.exists() { found = Some(c2); break 'outer; }
                                // Single-segment: also try source_dir/name_dir/exe_name
                                if module.len() == 1 {
                                    let c3 = search_dir.join(managed_name).join(name);
                                    if c3.exists() { found = Some(c3); break 'outer; }
                                }
                            }
                            let c = sub_dir.join(name);
                            if c.exists() { found = Some(c); break; }
                            // Single-segment CWD fallback: managed_name/exe_name
                            if module.len() == 1 {
                                let c2 = PathBuf::from(managed_name).join(name);
                                if c2.exists() { found = Some(c2); break; }
                            }
                            let c = PathBuf::from(name);
                            if c.exists() { found = Some(c); break; }
                        }
                        found
                    };
                    if let Some(ref pp) = proc_path {
                        match super::cs_proc_runtime::launch_proc(pp.as_ref()) {
                            Err(e) => eprintln!("Warning: cs-proc host not started: {e}"),
                            Ok(()) => {
                                let pp_str = pp.to_string_lossy().into_owned();
                                let mut patched = (*stub_ns).clone();
                                for val in patched.members.values_mut() {
                                    if let Value::Class(cls) = val {
                                        let mut new_cls = cls.deep_clone();
                                        new_cls.class_vars.insert(
                                            "__cs_proc_path__".to_string(),
                                            Value::Str(pp_str.clone()),
                                        );
                                        *val = Value::Class(std::rc::Rc::new(new_cls));
                                    }
                                }
                                return {
                                    let bind_name = alias.clone().unwrap_or_else(|| module.last().unwrap().clone());
                                    self.declare_var(bind_name, Var::new(Value::Namespace(std::rc::Rc::new(patched)), false));
                                    Ok(ExecResult::Normal)
                                };
                            }
                        }
                    }
                    stub_ns
                } else {
                    self.exec_module(lang, module, body)?
                };
                // Default bind name: for cpp imports use the file stem; otherwise last module segment
                let bind_name = alias.clone().unwrap_or_else(|| {
                    if lang == "cpp-dll" || lang == "cpp-lib" {
                        let path = module.first().map(|s| s.as_str()).unwrap_or("lib");
                        std::path::Path::new(path)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("lib")
                            .to_string()
                    } else {
                        module.last().unwrap().clone()
                    }
                });
                self.declare_var(bind_name, Var::new(Value::Namespace(ns), false));
                Ok(ExecResult::Normal)
            }
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
                self.dbg_vars.insert(name.clone(), Var::new(value, false));
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

    /// `let` 宣言を実行する。
    fn exec_let(&mut self, name: &str, expr: &Expr) -> Result<ExecResult, String> {
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
    fn exec_let_tuple(
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
    fn exec_static_var(
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
    fn exec_compound_assign(
        &mut self,
        name: &str,
        op: &BinOp,
        value: &Expr,
    ) -> Result<ExecResult, String> {
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
        Ok(ExecResult::Normal)
    }

    // ---------------------------------------------------------------------------
    // Control flow signals
    // ---------------------------------------------------------------------------

    /// `loop_yield expr` 文を実行する。for/while 式の中で値を蓄積する制御フロー信号。
    fn exec_loop_yield(&mut self, expr: &Expr) -> Result<ExecResult, String> {
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

    /// `if / elif / else` 文を実行する。最初に真となった条件のブランチをスコープ付きブロックとして実行する。
    fn exec_if_stmt(
        &mut self,
        branches: &[(Expr, Vec<Stmt>)],
        else_body: &Option<Vec<Stmt>>,
    ) -> Result<ExecResult, String> {
        for (cond, body) in branches {
            // Result ガード検出: `x.is_OK()` / `x.is_ERR()` の形式を確認する
            let result_rebind: Option<(String, bool)> = extract_result_guard_call(cond);

            let val = self.eval(cond)?;
            if self.eval_truthy(&val)? {
                // ガード節なら x を内部値（unwrap済み）に差し替えたスコープでボディを実行する
                if let Some((var_name, _is_ok)) = result_rebind {
                    let rebind_info = self.get_var(&var_name).and_then(|rv| {
                        if let Value::ResultVal { inner, .. } = rv.get_value() {
                            Some((*inner, rv.is_mutable()))
                        } else {
                            None
                        }
                    });
                    if let Some((inner_val, is_mut)) = rebind_info {
                        self.push_scope();
                        self.declare_var(var_name, Var::new(inner_val, is_mut));
                        let result = self.exec_block(body);
                        self.pop_scope();
                        return result;
                    }
                }
                return self.exec_scoped_block(body);
            }
        }
        if let Some(body) = else_body {
            return self.exec_scoped_block(body);
        }
        Ok(ExecResult::Normal)
    }

    /// `match` 文を実行する。サブジェクトを各アームのパターンと照合し、最初に一致したアームのボディを実行する。
    fn exec_match_stmt(&mut self, subject: &Expr, arms: &[MatchArm]) -> Result<ExecResult, String> {
        let subject_val = self.eval(subject)?;
        for arm in arms {
            let matched = match &arm.pattern {
                MatchPattern::Case(pattern_expr) => {
                    if matches!(pattern_expr, Expr::Ident(n) if n == "_") {
                        true
                    } else {
                        let pattern_val = self.eval(pattern_expr)?;
                        let result =
                            self.apply_binop_dyn(&BinOp::Eq, subject_val.clone(), pattern_val)?;
                        matches!(result, Value::Bool(true))
                    }
                }
                MatchPattern::IsType(type_name) => self.value_is_type(&subject_val, type_name),
            };
            if matched {
                return self.exec_scoped_block(&arm.body);
            }
        }
        Ok(ExecResult::Normal)
    }

    /// `while cond: body` 文を実行する。条件が偽になるか `break` が発生するまでボディを繰り返す。
    fn exec_while_stmt(&mut self, cond: &Expr, body: &[Stmt]) -> Result<ExecResult, String> {
        LOOP_DEPTH.with(|d| *d.borrow_mut() += 1);
        let result = (|| {
            loop {
                let val = self.eval(cond)?;
                if !self.eval_truthy(&val)? {
                    break;
                }
                match self.exec_scoped_block(body) {
                    Ok(ExecResult::Break) | Ok(ExecResult::BlockReturn(Value::None)) => break,
                    Ok(ExecResult::Continue) | Ok(ExecResult::Normal) => {}
                    Ok(r) => return Ok(r),
                    Err(ref e) if e.as_str() == BREAK_SENTINEL => break,
                    Err(e) => return Err(e),
                }
            }
            Ok(ExecResult::Normal)
        })();
        LOOP_DEPTH.with(|d| *d.borrow_mut() -= 1);
        result
    }

    /// `for target in iter: body` 文を実行する。イテラブルを展開して各要素でボディを繰り返す。
    fn exec_for_stmt(
        &mut self,
        targets: &[String],
        iter: &Expr,
        body: &[Stmt],
    ) -> Result<ExecResult, String> {
        let iter_val = self.eval(iter)?;
        let generator = match iter_val {
            Value::List(items) => Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: items.borrow().clone(),
                index: 0,
            }))),
            Value::FrozenList { ref state, ref layout } => {
                let st = state.borrow();
                let values = (0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect();
                Value::Generator(Rc::new(RefCell::new(GeneratorState { values, index: 0 })))
            }
            Value::Str(s) => {
                let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: chars,
                    index: 0,
                })))
            }
            Value::Set(items) => Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: items.borrow().clone(),
                index: 0,
            }))),
            Value::Tuple(td) => Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: td.all_values().to_vec(),
                index: 0,
            }))),
            Value::Generator(_) => iter_val,
            Value::Instance(_) => self.eval_method_call(iter_val, "__iter__", &[])?,
            Value::PyObject(ref handle) => {
                let items = super::py_interop::py_collect_iter(handle)?;
                Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: items,
                    index: 0,
                })))
            }
            _ => return Err("TypeError: object is not iterable".to_string()),
        };
        LOOP_DEPTH.with(|d| *d.borrow_mut() += 1);
        let result =
            (|| {
                loop {
                    match self.eval_method_call(generator.clone(), "next", &[]) {
                        Ok(item) => {
                            self.push_scope();
                            if targets.len() == 1 {
                                self.declare_var(targets[0].clone(), Var::new(item, true));
                            } else {
                                let elems =
                                    match &item {
                                        Value::Tuple(td) => {
                                            if td.len() != targets.len() {
                                                return Err(format!(
                                                    "ValueError: not enough values to unpack \
                                             (expected {}, got {})",
                                                    targets.len(),
                                                    td.len()
                                                ));
                                            }
                                            td.all_values().to_vec()
                                        }
                                        _ => return Err(
                                            "TypeError: cannot unpack non-tuple value in for loop"
                                                .to_string(),
                                        ),
                                    };
                                for (name, val) in targets.iter().zip(elems) {
                                    self.declare_var(name.clone(), Var::new(val, true));
                                }
                            }
                            let result = self.exec_block(body);
                            self.pop_scope();
                            match result {
                                Ok(ExecResult::Break)
                                | Ok(ExecResult::BlockReturn(Value::None)) => break,
                                Ok(ExecResult::Continue) | Ok(ExecResult::Normal) => {}
                                Ok(r) => return Ok(r),
                                Err(ref e) if e.as_str() == BREAK_SENTINEL => break,
                                Err(e) => return Err(e),
                            }
                        }
                        Err(ref e) if e.starts_with("EndOfIteration") => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(ExecResult::Normal)
            })();
        LOOP_DEPTH.with(|d| *d.borrow_mut() -= 1);
        result
    }

    /// `block: body` 文を実行する。`BlockReturn` を消費し、それ以外の制御フローは外へ伝播させる。
    fn exec_block_stmt(&mut self, body: &[Stmt]) -> Result<ExecResult, String> {
        // All BlockReturn values are absorbed (the block: statement consumes them).
        // Break, Continue, Return, Raise propagate outward to the enclosing loop/function.
        match self.exec_scoped_block(body)? {
            ExecResult::Normal | ExecResult::BlockReturn(_) => Ok(ExecResult::Normal),
            r => Ok(r),
        }
    }

    // ---------------------------------------------------------------------------
    // Function / generator definitions
    // ---------------------------------------------------------------------------

    /// `fn` 定義を実行して関数値をスコープに登録する。テンプレート関数はテンプレート値として格納する。
    fn exec_fn_def(
        &mut self,
        name: &str,
        template_params: &[TemplateParam],
        params: &[Param],
        body: &[Stmt],
        decorators: &[Expr],
        return_type: Option<&str>,
    ) -> Result<ExecResult, String> {
        if !template_params.is_empty() {
            let tmpl = Rc::new(TemplateFnValue {
                name: name.to_string(),
                template_params: template_params.to_vec(),
                params: params.to_vec(),
                body: body.to_vec(),
            });
            self.scopes
                .last_mut()
                .unwrap()
                .insert(name.to_string(), Var::new(Value::TemplateFn(tmpl), false));
            return Ok(ExecResult::Normal);
        }

        let captured_env = if self.scopes.len() > 1 {
            self.capture_env(body, params)
        } else {
            HashMap::new()
        };
        let fn_val = Rc::new(FnValue {
            name: name.to_string(),
            params: params.to_vec(),
            body: body.to_vec(),
            is_python: self.in_python_module,
            captured_env,
            return_type: return_type.map(|s| s.to_string()),
        });

        if decorators.is_empty() {
            let existing = self
                .scopes
                .last()
                .and_then(|s| s.get(name))
                .map(|v| v.get_value());
            let new_value = match existing {
                Some(Value::Function(prev)) => Value::OverloadedFn(vec![prev, fn_val]),
                Some(Value::OverloadedFn(mut fns)) => {
                    fns.push(fn_val);
                    Value::OverloadedFn(fns)
                }
                _ => Value::Function(fn_val),
            };
            self.scopes
                .last_mut()
                .unwrap()
                .insert(name.to_string(), Var::new(new_value, false));
        } else {
            let mut value = Value::Function(fn_val);
            for dec_expr in decorators.iter().rev() {
                let dec = self.eval(dec_expr)?;
                value = self.apply_value_call(dec, value, name)?;
            }
            self.scopes
                .last_mut()
                .unwrap()
                .insert(name.to_string(), Var::new(value, false));
        }
        Ok(ExecResult::Normal)
    }

    /// `gen` 定義を実行してジェネレータ関数値をスコープに登録する。
    fn exec_gen_def(
        &mut self,
        name: &str,
        template_params: &[TemplateParam],
        params: &[Param],
        body: &[Stmt],
    ) -> Result<ExecResult, String> {
        if !template_params.is_empty() {
            let tmpl = Rc::new(TemplateGenFnValue {
                name: name.to_string(),
                template_params: template_params.to_vec(),
                params: params.to_vec(),
                body: body.to_vec(),
            });
            self.scopes.last_mut().unwrap().insert(
                name.to_string(),
                Var::new(Value::TemplateGenFn(tmpl), false),
            );
        } else {
            let captured_env = if self.scopes.len() > 1 {
                self.capture_env(body, params)
            } else {
                HashMap::new()
            };
            let gen_fn = Rc::new(GeneratorFnValue {
                name: name.to_string(),
                params: params.to_vec(),
                body: body.to_vec(),
                captured_env,
            });
            self.scopes.last_mut().unwrap().insert(
                name.to_string(),
                Var::new(Value::GeneratorFn(gen_fn), false),
            );
        }
        Ok(ExecResult::Normal)
    }

    // ---------------------------------------------------------------------------
    // Type definitions
    // ---------------------------------------------------------------------------

    /// `trait` 定義を実行してトレイト値をスコープに登録する。アクセス制御情報も収集する。
    fn exec_trait_def(&mut self, name: &str, body: &[Stmt]) -> Result<ExecResult, String> {
        let mut trait_access: HashMap<String, Accessibility> = HashMap::new();
        for stmt in body {
            if let Stmt::Field {
                name: fname,
                access,
                ..
            } = stmt
            {
                if *access != Accessibility::Public {
                    trait_access.insert(fname.clone(), access.clone());
                }
            }
            if let Stmt::FnDef {
                name: mname,
                access,
                ..
            } = stmt
            {
                if *access != Accessibility::Public {
                    trait_access.insert(mname.clone(), access.clone());
                }
            }
        }
        if !trait_access.is_empty() {
            self.trait_field_access
                .insert(name.to_string(), trait_access);
        }
        self.declare_var(
            name.to_string(),
            Var::new(Value::Trait(name.to_string()), false),
        );
        Ok(ExecResult::Normal)
    }

    /// `protocol` 定義を実行してプロトコル値をスコープに登録する。
    /// プロトコルは静的型チェック専用で、インスタンス化できない。
    fn exec_protocol_def(&mut self, name: &str, body: &[Stmt]) -> Result<ExecResult, String> {
        // 必須メンバー名を収集（is Protocol 実行時チェック用）
        let mut members: Vec<String> = Vec::new();
        for s in body {
            match s {
                Stmt::Field { name: fname, .. } => members.push(fname.clone()),
                Stmt::FnDef { name: mname, .. } => members.push(mname.clone()),
                _ => {}
            }
        }
        self.protocol_required_members.insert(name.to_string(), members);
        self.declare_var(
            name.to_string(),
            Var::new(Value::Protocol(name.to_string()), false),
        );
        Ok(ExecResult::Normal)
    }

    /// `new_type name: OriginalType` を実行して新しい型をスコープに登録する。
    fn exec_new_type_def(&mut self, name: &str, original: &str) -> Result<ExecResult, String> {
        let orig_val = self
            .get_val(original)
            .ok_or_else(|| format!("NameError: type '{original}' is not defined"))?;
        match orig_val {
            Value::Class(orig_cls) => {
                let new_cls = Rc::new(super::ClassValue {
                    name: name.to_string(),
                    bases: orig_cls.bases.clone(),
                    methods: orig_cls.methods.clone(),
                    gen_methods: orig_cls.gen_methods.clone(),
                    field_defaults: orig_cls.field_defaults.clone(),
                    class_vars: orig_cls.class_vars.clone(),
                    field_mutability: orig_cls.field_mutability.clone(),
                    field_access: orig_cls.field_access.clone(),
                    method_access: orig_cls.method_access.clone(),
                    static_method_names: orig_cls.static_method_names.clone(),
                    class_method_names: orig_cls.class_method_names.clone(),
                    static_vars: orig_cls.static_vars.clone(),
                    new_type_base: orig_cls.new_type_base.clone(),
                });
                self.declare_var(name.to_string(), Var::new(Value::Class(new_cls), false));
            }
            Value::Type(type_name) => {
                // `new_type Meters: int` → `class Meters: mut value: int` と等価
                let init_body = vec![Stmt::AttrAssign {
                    target: Expr::Attr {
                        object: Box::new(Expr::Ident("self".to_string())),
                        attr: "value".to_string(),
                        span: crate::token::Span::unknown(),
                    },
                    value: Expr::Ident("value".to_string()),
                }];
                let init_fn = Rc::new(FnValue {
                    name: "__init__".to_string(),
                    params: vec![
                        crate::ast::Param {
                            name: "self".to_string(),
                            mutable: true,
                            type_ann: None,
                            default: None,
                            variadic: false,
                        },
                        crate::ast::Param {
                            name: "value".to_string(),
                            mutable: false,
                            type_ann: Some(type_name.clone()),
                            default: None,
                            variadic: false,
                        },
                    ],
                    body: init_body,
                    is_python: false,
                    captured_env: HashMap::new(),
                return_type: None,
                });
                let mut methods = HashMap::new();
                methods.insert("__init__".to_string(), vec![init_fn]);
                let new_cls = Rc::new(super::ClassValue {
                    name: name.to_string(),
                    bases: vec![],
                    methods,
                    gen_methods: HashMap::new(),
                    field_defaults: vec![],
                    class_vars: HashMap::new(),
                    field_mutability: HashMap::from([("value".to_string(), true)]),
                    field_access: HashMap::new(),
                    method_access: HashMap::new(),
                    static_method_names: HashSet::new(),
                    class_method_names: HashSet::new(),
                    static_vars: HashMap::new(),
                    new_type_base: Some(type_name.clone()),
                });
                self.declare_var(name.to_string(), Var::new(Value::Class(new_cls), false));
            }
            _ => {
                return Err(format!(
                    "TypeError: cannot create new_type from '{original}' — only classes and primitive types are supported"
                ));
            }
        }
        Ok(ExecResult::Normal)
    }

    /// `enum` 定義を実行して列挙型クラスと各バリアントをスコープに登録する。
    fn exec_enum_def(
        &mut self,
        name: &str,
        variants: &[(String, Option<Expr>)],
    ) -> Result<ExecResult, String> {
        // enum_item_Name クラスを生成する（new_type enum_item_Name: int 相当）
        let item_type_name = format!("enum_item_{}", name);
        let init_body = vec![Stmt::AttrAssign {
            target: Expr::Attr {
                object: Box::new(Expr::Ident("self".to_string())),
                attr: "value".to_string(),
                span: crate::token::Span::unknown(),
            },
            value: Expr::Ident("value".to_string()),
        }];
        let init_fn = Rc::new(FnValue {
            name: "__init__".to_string(),
            params: vec![
                crate::ast::Param {
                    name: "self".to_string(),
                    mutable: true,
                    type_ann: None,
                    default: None,
                    variadic: false,
                },
                crate::ast::Param {
                    name: "value".to_string(),
                    mutable: false,
                    type_ann: Some("int".to_string()),
                    default: None,
                    variadic: false,
                },
            ],
            body: init_body,
            is_python: false,
            captured_env: HashMap::new(),
        return_type: None,
        });
        let mut item_methods = HashMap::new();
        item_methods.insert("__init__".to_string(), vec![init_fn]);
        let item_cls = Rc::new(super::ClassValue {
            name: item_type_name.clone(),
            bases: vec![],
            methods: item_methods,
            gen_methods: HashMap::new(),
            field_defaults: vec![],
            class_vars: HashMap::new(),
            field_mutability: HashMap::from([("value".to_string(), true)]),
            field_access: HashMap::new(),
            method_access: HashMap::new(),
            static_method_names: HashSet::new(),
            class_method_names: HashSet::new(),
            static_vars: HashMap::new(),
            new_type_base: None,
        });
        self.declare_var(
            item_type_name.clone(),
            Var::new(Value::Class(item_cls.clone()), false),
        );

        // 各バリアントの値を計算し、enum クラスの const クラス変数として登録する
        let mut class_vars: HashMap<String, Value> = HashMap::new();
        let mut next_value: i64 = 0;
        for (variant_name, value_expr) in variants {
            let int_val = if let Some(expr) = value_expr {
                match self.eval(expr)? {
                    Value::Int(n) => n,
                    other => {
                        return Err(format!(
                            "TypeError: enum variant '{}' value must be int, got '{}'",
                            variant_name,
                            self.type_name(&other)
                        ))
                    }
                }
            } else {
                next_value
            };
            next_value = int_val + 1;
            let inst =
                self.instantiate_evaled(item_cls.clone(), vec![(None, Value::Int(int_val))])?;
            class_vars.insert(variant_name.clone(), inst);
        }

        let enum_cls = Rc::new(super::ClassValue {
            name: name.to_string(),
            bases: vec![],
            methods: HashMap::new(),
            gen_methods: HashMap::new(),
            field_defaults: vec![],
            class_vars,
            field_mutability: HashMap::new(),
            field_access: HashMap::new(),
            method_access: HashMap::new(),
            static_method_names: HashSet::new(),
            class_method_names: HashSet::new(),
            static_vars: HashMap::new(),
            new_type_base: None,
        });
        self.declare_var(name.to_string(), Var::new(Value::Class(enum_cls), false));
        Ok(ExecResult::Normal)
    }

    /// `class` 定義を実行してクラス値をスコープに登録する。トレイト継承・フィールド・メソッドを処理する。
    fn exec_class_def(
        &mut self,
        name: &str,
        template_params: &[TemplateParam],
        bases: &[String],
        body: &[Stmt],
        decorators: &[Expr],
    ) -> Result<ExecResult, String> {
        if !template_params.is_empty() {
            let tmpl = Rc::new(TemplateClassValue {
                name: name.to_string(),
                template_params: template_params.to_vec(),
                bases: bases.to_vec(),
                body: body.to_vec(),
            });
            self.declare_var(
                name.to_string(),
                Var::new(Value::TemplateClass(tmpl), false),
            );
            return Ok(ExecResult::Normal);
        }

        let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
        let mut gen_methods: HashMap<String, Rc<GeneratorFnValue>> = HashMap::new();
        let mut field_defaults = Vec::new();
        let mut class_vars: HashMap<String, Value> = HashMap::new();
        let mut field_mutability: HashMap<String, bool> = HashMap::new();
        let mut field_access: HashMap<String, Accessibility> = HashMap::new();
        let mut method_access: HashMap<String, Accessibility> = HashMap::new();
        let mut static_method_names: HashSet<String> = HashSet::new();
        let mut class_method_names: HashSet<String> = HashSet::new();
        let mut static_vars: HashMap<String, Rc<RefCell<Value>>> = HashMap::new();

        // 継承トレイトのフィールドアクセス可能性を引き継ぐ
        for base in bases {
            if let Some(trait_acc) = self.trait_field_access.get(base) {
                for (fname, acc) in trait_acc {
                    field_access.insert(format!("{}::{}", base, fname), acc.clone());
                }
            }
        }

        for stmt in body {
            match stmt {
                Stmt::FnDef {
                    name: mname,
                    template_params,
                    params,
                    body: mbody,
                    decorators: mdecs,
                    access: macc,
                    is_static,
                    is_class_method,
                    return_type: mret,
                    ..
                } => {
                    let fn_val = Rc::new(FnValue {
                        name: mname.clone(),
                        params: params.clone(),
                        body: mbody.clone(),
                        is_python: self.in_python_module,
                        captured_env: HashMap::new(),
                        return_type: mret.clone(),
                    });
                    // `__cast__[TypeName]` メソッドはキャスト専用のキー名で格納する。
                    // テンプレートパラメータの名前（具体型名）をキーとして使用する。
                    let storage_name = if mname == "__cast__" && !template_params.is_empty() {
                        format!("__cast__[{}]", template_params[0].name)
                    } else {
                        mname.clone()
                    };
                    if *is_static {
                        static_method_names.insert(storage_name.clone());
                    }
                    if *is_class_method {
                        class_method_names.insert(storage_name.clone());
                    }
                    if *macc != Accessibility::Public {
                        method_access.insert(storage_name.clone(), macc.clone());
                    }
                    if mdecs.is_empty() {
                        methods.entry(storage_name).or_default().push(fn_val);
                    } else {
                        let mut value = Value::Function(fn_val);
                        for dec_expr in mdecs.iter().rev() {
                            let dec = self.eval(dec_expr)?;
                            value = self.apply_value_call(dec, value, mname)?;
                        }
                        match value {
                            Value::Function(f) => {
                                methods.entry(storage_name).or_default().push(f)
                            }
                            other => return Err(format!(
                                "TypeError: method decorator on '{}' must return a function, got '{}'",
                                mname,
                                self.type_name(&other)
                            )),
                        }
                    }
                }
                Stmt::GenDef {
                    name: mname,
                    params,
                    body: mbody,
                    access: macc,
                    ..
                } => {
                    if *macc != Accessibility::Public {
                        method_access.insert(mname.clone(), macc.clone());
                    }
                    gen_methods.insert(
                        mname.clone(),
                        Rc::new(GeneratorFnValue {
                            name: mname.clone(),
                            params: params.clone(),
                            body: mbody.clone(),
                            captured_env: HashMap::new(),
                        }),
                    );
                }
                Stmt::Field {
                    name: fname,
                    kind: FieldKind::Const,
                    default: Some(init),
                    access: facc,
                    ..
                } => {
                    if *facc != Accessibility::Public {
                        field_access.insert(fname.clone(), facc.clone());
                    }
                    let val = self.eval(init)?;
                    class_vars.insert(fname.clone(), val);
                }
                Stmt::Field {
                    name: fname,
                    kind: FieldKind::StaticMut,
                    default,
                    access: facc,
                    ..
                } => {
                    if *facc != Accessibility::Public {
                        field_access.insert(fname.clone(), facc.clone());
                    }
                    let val = if let Some(init) = default {
                        self.eval(init)?
                    } else {
                        Value::None
                    };
                    static_vars.insert(fname.clone(), Rc::new(RefCell::new(val)));
                }
                Stmt::Field {
                    name: fname,
                    kind,
                    default,
                    access: facc,
                    ..
                } => {
                    if *facc != Accessibility::Public {
                        field_access.insert(fname.clone(), facc.clone());
                    }
                    let mutable = *kind == FieldKind::Mut;
                    field_mutability.insert(fname.clone(), mutable);
                    if let Some(init) = default {
                        let val = self.eval(init)?;
                        field_defaults.push((fname.clone(), val, mutable));
                    }
                }
                _ => {}
            }
        }

        let cls = Rc::new(super::ClassValue {
            name: name.to_string(),
            bases: bases.to_vec(),
            methods,
            gen_methods,
            field_defaults,
            class_vars,
            field_mutability,
            field_access,
            method_access,
            static_method_names,
            class_method_names,
            static_vars,
            new_type_base: None,
        });
        if decorators.is_empty() {
            self.declare_var(name.to_string(), Var::new(Value::Class(cls), false));
        } else {
            let mut value = Value::Class(cls);
            for dec_expr in decorators.iter().rev() {
                let dec = self.eval(dec_expr)?;
                value = self.apply_value_call(dec, value, name)?;
            }
            self.declare_var(name.to_string(), Var::new(value, false));
        }
        Ok(ExecResult::Normal)
    }

    // ---------------------------------------------------------------------------
    // Exception handling
    // ---------------------------------------------------------------------------

    /// `freeze name` 文を実行する。変数を不変化し、インスタンスフィールドも再帰的にフリーズする。
    fn exec_freeze(&mut self, name: &str, span: &Span) -> Result<ExecResult, String> {
        let var = self
            .get_var(name)
            .ok_or_else(|| format!("{span}: NameError: '{name}' is not defined"))?;
        if !var.is_mutable() {
            return Err(format!(
                "{span}: TypeError: cannot freeze immutable variable '{name}'"
            ));
        }
        if var.cell().is_some() {
            return Err(format!(
                "{span}: TypeError: cannot freeze '{name}' because it is captured by a closure"
            ));
        }
        let val = var.get_value();

        let replacement = match &val {
            Value::Instance(ref inst_rc) => {
                let class = inst_rc.borrow().class.clone();
                if let Some(overloads) = self.lookup_method_in_class(&class, "__freeze__") {
                    if overloads.len() == 1 {
                        self.exec_fn(overloads[0].clone(), &[], Some(val.clone()), "__freeze__", None)?;
                    } else {
                        self.dispatch_overload(overloads, &[], Some(val.clone()), None)?;
                    }
                }
                Self::freeze_instance(inst_rc);
                None
            }
            Value::List(ref rc) => {
                let items = rc.borrow().clone();
                for item in &items {
                    self.apply_freeze_to_value(item, true)?;
                }
                None
            }
            Value::Set(ref rc) => {
                let items = rc.borrow().clone();
                for item in &items {
                    self.apply_freeze_to_value(item, true)?;
                }
                None
            }
            Value::Dict(ref rc) => {
                let vals = rc.borrow().all_items();
                for v in &vals {
                    self.apply_freeze_to_value(v, true)?;
                }
                None
            }
            Value::Tuple(ref td) => {
                for item in td.all_values() {
                    self.apply_freeze_to_value(item, true)?;
                }
                None
            }
            // fixed_list: trim unused allocated capacity on freeze
            Value::FrozenList { ref state, ref layout } => {
                let mut st = state.borrow_mut();
                let exact = st.len * layout.stride;
                st.data.truncate(exact);
                st.data.shrink_to_fit();
                st.allocated_size = st.len;
                None
            }
            _ => None,
        };

        // If a flat conversion was produced, update the variable value before sealing it.
        if let Some(flat) = replacement {
            self.assign_var(name, flat)
                .map_err(|e| format!("{span}: {e}"))?;
        }
        self.make_var_immutable(name);
        Ok(ExecResult::Normal)
    }

    /// `raise [exc]` 文を実行する。例外値を評価して `ExecResult::Raise` を返す。引数なしは再 raise。
    fn exec_raise(&mut self, exc: &Option<Expr>, span: &Span) -> Result<ExecResult, String> {
        if exc.is_none() {
            match &self.current_exception {
                Some(err) => {
                    let err = err.clone();
                    return Ok(ExecResult::Raise(err));
                }
                None => return Err("RuntimeError: no active exception to re-raise".to_string()),
            }
        }

        let exc_val = self.eval(exc.as_ref().unwrap())?;

        // 例外インスタンスに file / line / col / code_context を直接書き込む
        if let Value::Instance(ref inst_rc) = exc_val {
            let context = self.get_context_lines(&span.file, span.line, 5);
            let mut inst = inst_rc.borrow_mut();
            inst.fields.insert(
                "file".to_string(),
                (Value::Str(span.file.to_string()), false),
            );
            inst.fields
                .insert("line".to_string(), (Value::Int(span.line as i64), false));
            inst.fields
                .insert("col".to_string(), (Value::Int(span.col as i64), false));
            inst.fields.insert(
                "code_context".to_string(),
                (Value::Str(context.clone()), false),
            );
            inst.fields.insert(
                "Error::file".to_string(),
                (Value::Str(span.file.to_string()), false),
            );
            inst.fields.insert(
                "Error::line".to_string(),
                (Value::Int(span.line as i64), false),
            );
            inst.fields.insert(
                "Error::col".to_string(),
                (Value::Int(span.col as i64), false),
            );
            inst.fields.insert(
                "Error::code_context".to_string(),
                (Value::Str(context), false),
            );
        }

        let fn_name = self
            .call_stack
            .last()
            .cloned()
            .unwrap_or_else(|| "<module>".to_string());
        let frame = StackFrame {
            file: span.file.to_string(),
            line: span.line,
            col: span.col,
            fn_name,
            context: self.get_context_lines(&span.file, span.line, 5),
        };
        Ok(ExecResult::Raise(RaisedError {
            exception: exc_val,
            frames: vec![frame],
        }))
    }

    /// `try / except / finally` 文を実行する。例外を捕捉してハンドラを実行し、finally ブロックは常に実行する。
    fn exec_try(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        finally_body: &Option<Vec<Stmt>>,
    ) -> Result<ExecResult, String> {
        let body_result = self.exec_scoped_block(body);

        let mut converted_internal = false;
        let raise_opt: Option<RaisedError> = match &body_result {
            Ok(ExecResult::Raise(r)) => Some(r.clone()),
            Err(e) if e.as_str() == RAISE_SENTINEL => self.current_exception.clone(),
            Err(e) => {
                let msg = e.clone();
                let r = self.make_internal_raised_error(&msg);
                if r.is_some() {
                    converted_internal = true;
                }
                r
            }
            _ => None,
        };

        let mut final_result: Result<ExecResult, String> = body_result;

        if let Some(raised) = raise_opt {
            let mut handled = false;
            for handler in handlers {
                let matches = match &handler.exc_type {
                    None => true,
                    Some(type_name) => {
                        if let Value::Instance(ref inst_rc) = raised.exception {
                            Self::exc_matches(&inst_rc.borrow().class, type_name)
                        } else {
                            false
                        }
                    }
                };
                if matches {
                    let prev_exc = self.current_exception.clone();
                    self.current_exception = Some(raised.clone());

                    self.push_scope();
                    if let Some(alias) = &handler.name {
                        let exc_val = raised.exception.clone();
                        self.declare_var(alias.clone(), Var::new(exc_val, false));
                    }
                    let handler_result = self.exec_block(&handler.body);
                    self.pop_scope();

                    self.current_exception = prev_exc;
                    final_result = handler_result;
                    handled = true;
                    break;
                }
            }
            if !handled && converted_internal {
                // 内部エラーから変換された RaisedError がどのハンドラにもマッチしなかった場合:
                // ExecResult::Raise として上位に伝播させ、トレースバック表示が機能するようにする
                final_result = Ok(ExecResult::Raise(raised));
            }
        }

        if let Some(finally) = finally_body {
            let finally_result = self.exec_scoped_block(finally);
            match finally_result {
                Ok(ExecResult::Normal) => {}
                Ok(signal) => return Ok(signal),
                Err(e) => return Err(e),
            }
        }

        final_result
    }

    // ---------------------------------------------------------------------------
    // Async
    // ---------------------------------------------------------------------------

    /// `target <- async->T: body` 文を実行する。`AsyncManager` にタスクを追加する。
    fn exec_async_assign(&mut self, target: &str, stmts: &[Stmt]) -> Result<ExecResult, String> {
        let mgr_val = self
            .get_var(target)
            .map(|v| v.get_value())
            .ok_or_else(|| format!("NameError: '{}' is not defined", target))?;

        let mgr_rc = match mgr_val {
            Value::AsyncManager(rc) => rc,
            other => {
                return Err(format!(
                    "TypeError: '<-' operator requires an AsyncManager, got '{}'",
                    self.type_name(&other)
                ))
            }
        };

        let env = super::async_mgr::capture_env(self);
        mgr_rc.borrow_mut().add_task(stmts.to_vec(), env);
        Ok(ExecResult::Normal)
    }

    // ---------------------------------------------------------------------------
    // External event queue draining (C#/Go bridge)
    // ---------------------------------------------------------------------------

    /// 外部イベントキュー（C#/Go ブリッジが ar_event_fire() で書き込んだもの）をすべて処理する。
    pub(super) fn drain_external_events(&mut self) -> Result<(), String> {
        let events: Vec<super::event_loop::ExternalEvent> = {
            let mut guard = self.external_event_queue.lock().unwrap();
            guard.drain(..).collect()
        };
        for ev in events {
            let sig_rc = self.external_handler_registry.get(&ev.handler_id).cloned();
            if let Some(sig_rc) = sig_rc {
                // データは MessagePack でシリアライズされているが、現時点では str として渡す。
                let val = Value::Str(String::from_utf8_lossy(&ev.data).into_owned());
                let handlers = sig_rc.borrow_mut().collect_handlers_for_emit();
                for (h, _) in handlers {
                    self.call_value_with_args(h, vec![val.clone()])?;
                }
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------
    // Event handler subscription / unsubscription
    // ---------------------------------------------------------------------------

    /// `source on handler` / `source once handler` — イベントハンドラを登録する。
    fn exec_event_subscribe(
        &mut self,
        source: &crate::ast::Expr,
        handler: &crate::ast::Expr,
        is_once: bool,
        is_async: bool,
    ) -> Result<ExecResult, String> {
        let source_val = self.eval(source)?;
        let handler_val = self.eval(handler)?;
        match source_val {
            Value::Signal(sig_rc) => {
                sig_rc
                    .borrow_mut()
                    .subscribe(handler_val, is_once, is_async);
                Ok(ExecResult::Normal)
            }
            other => Err(format!(
                "TypeError: 'on'/'once' operator requires a Signal, got '{}'",
                self.type_name(&other)
            )),
        }
    }

    /// `source off handler` — ハンドラを解除する。
    fn exec_event_unsubscribe(
        &mut self,
        source: &crate::ast::Expr,
        handler: &crate::ast::Expr,
    ) -> Result<ExecResult, String> {
        let source_val = self.eval(source)?;
        let handler_val = self.eval(handler)?;
        match source_val {
            Value::Signal(sig_rc) => {
                sig_rc.borrow_mut().unsubscribe_by_value(&handler_val);
                Ok(ExecResult::Normal)
            }
            other => Err(format!(
                "TypeError: 'off' operator requires a Signal, got '{}'",
                self.type_name(&other)
            )),
        }
    }

    // ---------------------------------------------------------------------------
    // Module loading
    // ---------------------------------------------------------------------------

    /// モジュールの body を孤立スコープで実行し、`Value::Namespace` を返す。
    /// キャッシュを使用し、循環 import はエラーにする。
    fn exec_module(
        &mut self,
        lang: &str,
        module: &[String],
        body: &[Stmt],
    ) -> Result<Rc<NamespaceData>, String> {
        let cache_key = (lang.to_string(), PathBuf::from(module.join("/")));

        match self.module_cache.get(&cache_key).cloned() {
            Some(ModuleState::Loading) => {
                return Err(format!(
                    "RuntimeError: circular import detected: '{}'",
                    module.join(".")
                ));
            }
            Some(ModuleState::Loaded(ns)) => return Ok(ns),
            None => {}
        }

        self.module_cache
            .insert(cache_key.clone(), ModuleState::Loading);

        if lang == "py-int" {
            let search_dirs = self.python_search_dirs.clone();
            let ns = super::py_interop::load_py_int_module(module, &search_dirs).map_err(|e| e)?;
            self.module_cache
                .insert(cache_key, ModuleState::Loaded(ns.clone()));
            return Ok(ns);
        }

        // tl-auto / tlc / rs: native payload in cache → try to load natively
        if lang == "tl-auto" || lang == "ar-auto" || lang == "tlc" || lang == "arc" || lang == "rs" {
            let module_name = module.join(".");
            // .arc files store only the stem as their module name; fall back to last segment.
            let native_data = crate::partial_compiler::take_native_bytes(&module_name)
                .or_else(|| {
                    let stem = module.last().map(|s| s.as_str()).unwrap_or("");
                    if stem != module_name { crate::partial_compiler::take_native_bytes(stem) } else { None }
                });
            if let Some((_exports, payload)) = native_data
            {
                use crate::partial_compiler::NativePayload;
                match payload {
                    // ── inkwell JIT path (v2 bitcode) ─────────────────────────
                    #[cfg(feature = "llvm")]
                    NativePayload::Bitcode(bitcode) => {
                        match crate::partial_compiler::inkwell_codegen::jit_from_bitcode(
                            &bitcode, &exports,
                        ) {
                            Ok((jit_handle, fn_ptrs)) => {
                                match self.load_jit_module(module, body, &exports, &fn_ptrs, jit_handle) {
                                    Ok(ns) => {
                                        self.module_cache.insert(cache_key, ModuleState::Loaded(ns.clone()));
                                        return Ok(ns);
                                    }
                                    Err(e) => eprintln!("NativeLoad(JIT): {e}"),
                                }
                            }
                            Err(e) => eprintln!("NativeLoad(JIT): bitcode re-JIT failed: {e}"),
                        }
                    }
                    // ── DLL path (v1) ─────────────────────────────────────────
                    NativePayload::Dll(dll_bytes) => {
                        let ext = crate::partial_compiler::native_lib_ext();
                        let stem = module.last().cloned().unwrap_or_default();
                        let tmp_path = std::env::temp_dir().join(format!("{stem}_tl.{ext}"));
                        match std::fs::write(&tmp_path, &dll_bytes) {
                            Ok(()) => match self.try_load_native_module(module, body, &tmp_path) {
                                Ok(ns) => {
                                    self.module_cache.insert(cache_key, ModuleState::Loaded(ns.clone()));
                                    return Ok(ns);
                                }
                                Err(e) => eprintln!("NativeLoad(DLL): {e}"),
                            },
                            Err(e) => eprintln!("NativeLoad(DLL): cannot write temp DLL: {e}"),
                        }
                    }
                    // When the llvm feature is disabled, silently ignore Bitcode payloads
                    #[cfg(not(feature = "llvm"))]
                    NativePayload::Bitcode(_) => {
                        eprintln!("NativeLoad: bitcode .arc requires --features llvm");
                    }
                }
            }
        }

        let prev_in_python = self.in_python_module;
        if lang == "py" {
            self.in_python_module = true;
        }
        self.push_scope();
        for stmt in body {
            match self.exec(stmt)? {
                ExecResult::Normal => {}
                ExecResult::Raise(_) => {
                    self.pop_scope();
                    return Err(format!(
                        "RuntimeError: exception during module initialization: {}",
                        module.join(".")
                    ));
                }
                _ => {}
            }
        }
        let members: HashMap<String, Value> = self
            .scopes
            .last()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.get_value()))
            .collect();
        self.pop_scope();
        self.in_python_module = prev_in_python;

        // Python モジュールのメソッドが同モジュール内の他の関数を呼び出せるように
        // モジュールメンバをグローバルスコープに登録する（既存エントリは上書きしない）。
        for (name, value) in &members {
            self.scopes[0]
                .entry(name.clone())
                .or_insert_with(|| Var::new(value.clone(), false));
        }

        let ns = Rc::new(NamespaceData {
            name: module.join("."),
            members,
        });
        self.module_cache
            .insert(cache_key, ModuleState::Loaded(ns.clone()));
        Ok(ns)
    }

    /// inkwell JIT モジュールから `Namespace` を構築する。
    /// `fn_ptrs` は `(fn_name, raw_address)` のリスト。
    /// `jit_handle` は JIT エンジンの所有権を保持するボックスで、インタープリタに格納される。
    #[cfg(feature = "llvm")]
    fn load_jit_module(
        &mut self,
        module: &[String],
        body:   &[Stmt],
        exports: &[crate::partial_compiler::llvm_codegen::FnExport],
        fn_ptrs: &[(String, usize)],
        jit_handle: crate::partial_compiler::inkwell_codegen::JitHandle,
    ) -> Result<Rc<NamespaceData>, String> {
        use crate::interpreter::native_api::{get_callbacks, ArCallbacks};

        // Call ar_init via the JIT module to set the @CB global
        {
            // Look up ar_init by address if we can, otherwise skip
            // (the @CB global in the JIT module is separate from any DLL @CB)
            let ar_init_sym = fn_ptrs.iter()
                .find(|(n, _)| n == "ar_init")
                .map(|(_, p)| *p);
            if let Some(addr) = ar_init_sym {
                let cb_ptr = get_callbacks();
                unsafe {
                    let ar_init: unsafe extern "C" fn(*const ArCallbacks) =
                        std::mem::transmute(addr);
                    ar_init(cb_ptr);
                }
            } else {
                // ar_init address not in fn_ptrs; get it via the engine
                // (we'd need to expose it — for now assume the engine was
                //  already initialised by jit_from_bitcode which calls it)
            }
        }

        // Build the namespace: for each body statement that is a FnDef
        // and has a raw fn_ptr, create a NativeFnRef with raw_fn_ptr set.
        let mut members: HashMap<String, Value> = HashMap::new();
        self.push_scope();
        for stmt in body {
            self.exec(stmt)?;
        }
        members = self.scopes.last()
            .map(|s| s.iter().map(|(k, v)| (k.clone(), v.value.clone())).collect())
            .unwrap_or_default();
        self.pop_scope();

        // Override tree-walk functions with JIT versions
        let ptr_map: HashMap<&str, usize> =
            fn_ptrs.iter().map(|(n, p)| (n.as_str(), *p)).collect();
        for exp in exports {
            if let Some(&fn_ptr) = ptr_map.get(exp.name.as_str()) {
                if fn_ptr != 0 {
                    let fn_ref = Arc::new(NativeFnRef {
                        lib_path: PathBuf::new(), // not used for JIT
                        fn_name: exp.name.clone(),
                        n_params: exp.n_params,
                        min_params: exp.n_params,
                        param_mutabilities: vec![false; exp.n_params],
                        ptr_params: vec![crate::interpreter::PtrParam::None; exp.n_params],
                        raw_fn_ptr: fn_ptr,
                        cached_fn_ptr: std::sync::atomic::AtomicUsize::new(0),
                    });
                    members.insert(exp.name.clone(), Value::NativeFunction(fn_ref));
                }
            }
        }

        // Keep the JIT engine alive
        self.jit_handles.push(Box::new(jit_handle));

        eprintln!("NativeLoad(JIT): {} function(s) loaded", exports.len());
        Ok(Rc::new(NamespaceData {
            name: module.join("."),
            members,
        }))
    }

    /// ネイティブ共有ライブラリをロードして、そのモジュールの `Namespace` を構築する。
    fn try_load_native_module(
        &mut self,
        module: &[String],
        body: &[Stmt],
        lib_path: &std::path::Path,
    ) -> Result<Rc<NamespaceData>, String> {
        let lib = unsafe { libloading::Library::new(lib_path) }
            .map_err(|e| format!("libloading: {e}"))?;

        let lib_path_buf = lib_path.to_path_buf();

        self.push_scope();
        for stmt in body {
            match self.exec(stmt)? {
                ExecResult::Normal => {}
                ExecResult::Raise(_raised) => {
                    self.pop_scope();
                    return Err(format!(
                        "RuntimeError: exception during native module init: {}",
                        module.join(".")
                    ));
                }
                _ => {}
            }
        }
        let mut members: HashMap<String, Value> = self
            .scopes
            .last()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.get_value()))
            .collect();
        self.pop_scope();

        for stmt in body {
            match stmt {
                Stmt::FnDef { name, params, .. } | Stmt::GenDef { name, params, .. } => {
                    let symbol_name = format!("{name}_tl\0");
                    let has_symbol = unsafe {
                        lib.get::<unsafe extern "C" fn(*const i64, i32) -> i64>(symbol_name.as_bytes()).is_ok()
                    };
                    if has_symbol {
                        let fn_ref = Arc::new(NativeFnRef {
                            lib_path: lib_path_buf.clone(),
                            fn_name: name.clone(),
                            n_params: params.len(),
                            min_params: params.len(),
                            param_mutabilities: params.iter().map(|p| p.mutable).collect(),
                            ptr_params: vec![crate::interpreter::PtrParam::None; params.len()],
                            raw_fn_ptr: 0,
                            cached_fn_ptr: std::sync::atomic::AtomicUsize::new(0),
                        });
                        members.insert(name.clone(), Value::NativeFunction(fn_ref));
                    }
                }
                Stmt::ClassDef { name: class_name, body: class_body, .. } => {
                    for method_stmt in class_body {
                        let (mname, params) = match method_stmt {
                            Stmt::FnDef { name, params, .. } => (name, params),
                            Stmt::GenDef { name, params, .. } => (name, params),
                            _ => continue,
                        };
                        let symbol = crate::partial_compiler::llvm_codegen::method_symbol(class_name, mname);
                        let symbol_name = format!("{symbol}_tl\0");
                        if let Ok(func) = unsafe {
                            lib.get::<unsafe extern "C" fn(*const i64, i32) -> i64>(symbol_name.as_bytes())
                        } {
                            let fn_ptr = unsafe { *func } as usize;
                            super::native_api::register_native_method(class_name, mname, fn_ptr);
                            eprintln!("NativeMethod: {class_name}.{mname} ({} param(s)) → native", params.len());
                        }
                    }
                }
                _ => {}
            }
        }

        for (name, value) in &members {
            self.scopes[0]
                .entry(name.clone())
                .or_insert_with(|| Var::new(value.clone(), false));
        }

        {
            let cb_ptr = super::native_api::get_callbacks();
            let init_result = unsafe {
                lib.get::<unsafe extern "C" fn(*const super::native_api::ArCallbacks)>(b"ar_init\0")
            }.or_else(|_| unsafe {
                // backward compat: DLLs compiled before rename still export hv_init
                lib.get::<unsafe extern "C" fn(*const super::native_api::ArCallbacks)>(b"hv_init\0")
            });
            if let Ok(ar_init) = init_result {
                unsafe { ar_init(cb_ptr) };
            }
        }

        self.native_libs.insert(lib_path_buf, NativeLibWrapper(lib));

        let ns = Rc::new(NamespaceData {
            name: module.join("."),
            members,
        });
        Ok(ns)
    }

    // ---------------------------------------------------------------------------
    // C++ bridge module loading
    // ---------------------------------------------------------------------------

    /// C++ ライブラリ（`cpp-lib`）または DLL（`cpp-dll`）を tl モジュールとしてロードする。
    /// ヘッダーをパースして関数シグネチャを収集し、ラッパー DLL を構築・ロードして名前空間を返す。
    /// Look for a cs-dll bridge DLL by searching:
    /// 1. Next to any already-loaded module that matches the dll stem
    /// 2. Current working directory
    fn find_cs_bridge_dll(&self, dll_name: &str) -> Option<PathBuf> {
        // Search next to already-cached module paths
        for ((_lang, p), _) in &self.module_cache {
            if let Some(dir) = p.parent() {
                let candidate = dir.join(dll_name);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
        // Fall back to cwd
        let candidate = PathBuf::from(dll_name);
        if candidate.exists() { Some(candidate) } else { None }
    }

    fn load_cpp_module(
        &mut self,
        lang: &str,
        header_path_str: &str,
        _with_file: Option<&str>,
    ) -> Result<Rc<NamespaceData>, String> {
        let header_path = std::path::Path::new(header_path_str);
        let header_dir = header_path.parent().unwrap_or(std::path::Path::new("."));

        // Parse the header to extract function signatures.
        // Read as raw bytes then convert lossily: non-UTF-8 bytes (e.g. Shift-JIS
        // in Japanese comments) become U+FFFD replacement chars, which strip_comments
        // discards along with the surrounding comment text.
        let raw = std::fs::read(header_path)
            .map_err(|e| format!("CppImport: cannot read header '{header_path_str}': {e}"))?;
        let raw_str = String::from_utf8_lossy(&raw);
        // Load config before parsing so custom_type_map is available for all parse_header calls.
        let config = super::cpp_bridge::load_cpp_config(header_dir);
        let typedefs = super::cpp_bridge::load_system_typedefs(
            &config.system_headers,
            &config.precompile_macros,
        );
        let (mut sigs, mut struct_defs) =
            super::cpp_bridge::parse_header_full(&raw_str, &config.custom_type_map, &typedefs);

        match lang {
            "cpp-lib" => {
                // Build tl_{stem}.dll next to the header (permanent cache).

                // When precompile_macros are set, the main header may conditionally
                // include other headers (e.g. WINDOWS_DESKTOP_OS → DxFunctionWin.h).
                // Scan for local #include directives and parse those headers too so
                // their function signatures are available in the tl namespace.
                if !config.precompile_macros.is_empty() {
                    let included =
                        super::cpp_bridge::collect_included_headers(&raw_str, header_dir);
                    let mut known_names: std::collections::HashSet<String> =
                        sigs.iter().map(|s| s.name.clone()).collect();
                    let mut known_structs: std::collections::HashSet<String> =
                        struct_defs.iter().map(|d| d.name.clone()).collect();
                    for inc_path in &included {
                        if let Ok(inc_raw) = std::fs::read(inc_path) {
                            let inc_str = String::from_utf8_lossy(&inc_raw);
                            let (inc_sigs, inc_structs) =
                                super::cpp_bridge::parse_header_full(&inc_str, &config.custom_type_map, &typedefs);
                            let new_count = inc_sigs
                                .iter()
                                .filter(|s| !known_names.contains(&s.name))
                                .count();
                            if new_count > 0 {
                                eprintln!(
                                    "CppImport: {} additional function(s) from '{}'",
                                    new_count,
                                    inc_path.display()
                                );
                            }
                            for s in inc_sigs {
                                if known_names.insert(s.name.clone()) {
                                    sigs.push(s);
                                }
                            }
                            for d in inc_structs {
                                if known_structs.insert(d.name.clone()) {
                                    struct_defs.push(d);
                                }
                            }
                        }
                    }
                }

                if sigs.is_empty() {
                    eprintln!("CppImport: no supported functions found in '{header_path_str}'");
                }
                eprintln!("CppImport[{lang}]: {} function(s) total", sigs.len());

                let (dll_path, effective_sigs) =
                    super::cpp_bridge::compile_tl_dll(header_path, &sigs, &struct_defs, &config)?;
                self.load_cpp_wrapper_dll(&dll_path, &effective_sigs, &struct_defs, header_path_str)
            }
            "cpp-dll" => {
                // Find the DLL by stem next to the header and wrap it dynamically.
                let stem = header_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("lib");
                let dll_path = header_dir.join(format!("{stem}.dll"));
                let dll_str = dll_path.to_string_lossy().into_owned();
                let rust_src = super::cpp_bridge::gen_dll_wrapper(&dll_str, &sigs, &struct_defs);
                let dll_bytes = super::cpp_bridge::compile_wrapper(&rust_src, &[])?;
                let ext = crate::partial_compiler::native_lib_ext();
                let tmp_path = std::env::temp_dir()
                    .join(format!("_tl_cpp_{:x}.{ext}", simple_hash(header_path_str)));
                std::fs::write(&tmp_path, &dll_bytes)
                    .map_err(|e| format!("CppImport: cannot write wrapper DLL: {e}"))?;
                self.load_cpp_wrapper_dll(&tmp_path, &sigs, &struct_defs, header_path_str)
            }
            _ => unreachable!(),
        }
    }

    /// C パラメータ型を tl の `PtrParam` 種別にマッピングする。
    fn sig_to_ptr_param_fn(ct: &super::cpp_bridge::CType) -> crate::interpreter::PtrParam {
        use super::cpp_bridge::CType;
        use crate::interpreter::PtrParam;
        match ct {
            CType::Ptr { mutable: true, .. } => PtrParam::MutPtr,
            CType::Ptr { mutable: false, .. } | CType::CharPtr => PtrParam::ConstPtr,
            _ => PtrParam::None,
        }
    }

    /// コンパイル済み C++ ラッパー DLL をロードして名前空間を構築する。
    /// `ar_init_bridge`（あれば）でコールバックテーブルを初期化し、各関数を `NativeFunction` として登録する。
    fn load_cpp_wrapper_dll(
        &mut self,
        lib_path: &std::path::Path,
        sigs: &[super::cpp_bridge::CFnSig],
        struct_defs: &[super::cpp_bridge::CStructDef],
        module_name: &str,
    ) -> Result<Rc<NamespaceData>, String> {
        let lib = unsafe { libloading::Library::new(lib_path) }
            .map_err(|e| format!("CppImport: cannot load wrapper DLL: {e}"))?;

        let lib_path_buf = lib_path.to_path_buf();

        // Initialise: prefer ar_init_bridge (cpp-dll), fall back to ar_init / hv_init (compat)
        let cb_ptr = super::native_api::get_callbacks();
        let bridge_init = unsafe {
            lib.get::<unsafe extern "C" fn(*const super::native_api::ArCallbacks)>(
                b"ar_init_bridge\0",
            )
        }.or_else(|_| unsafe {
            lib.get::<unsafe extern "C" fn(*const super::native_api::ArCallbacks)>(b"hv_init_bridge\0")
        });
        if let Ok(f) = bridge_init {
            unsafe { f(cb_ptr) };
        } else if let Ok(f) = unsafe {
            lib.get::<unsafe extern "C" fn(*const super::native_api::ArCallbacks)>(b"ar_init\0")
        }.or_else(|_| unsafe {
            lib.get::<unsafe extern "C" fn(*const super::native_api::ArCallbacks)>(b"hv_init\0")
        }) {
            unsafe { f(cb_ptr) };
        }

        let mut members: HashMap<String, Value> = HashMap::new();

        // Build tl class values for each C struct so that tl code can construct
        // and access struct instances, and native code can call get_global/call_fn.
        for sdef in struct_defs {
            use crate::interpreter::{ClassValue, FnValue};
            use crate::ast::Param;
            use crate::token::Span;

            let mut field_mutability: HashMap<String, bool> = HashMap::new();
            let mut init_params: Vec<Param> = vec![Param {
                name: "self".to_string(),
                mutable: true,
                type_ann: None,
                default: None,
                variadic: false,
            }];
            for (fname, _) in &sdef.fields {
                field_mutability.insert(fname.clone(), true);
                init_params.push(Param {
                    name: fname.clone(),
                    mutable: false,
                    type_ann: None,
                    default: None,
                    variadic: false,
                });
            }

            // __init__ body: `self.field = field` for each field
            let init_body: Vec<crate::ast::Stmt> = sdef
                .fields
                .iter()
                .map(|(fname, _)| crate::ast::Stmt::AttrAssign {
                    target: crate::ast::Expr::Attr {
                        object: Box::new(crate::ast::Expr::Ident("self".to_string())),
                        attr: fname.clone(),
                        span: Span::unknown(),
                    },
                    value: crate::ast::Expr::Ident(fname.clone()),
                })
                .collect();

            let init_fn = Rc::new(FnValue {
                name: "__init__".to_string(),
                params: init_params,
                body: init_body,
                is_python: false,
                captured_env: HashMap::new(),
            return_type: None,
            });

            let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
            methods.insert("__init__".to_string(), vec![init_fn]);

            let cls = Rc::new(ClassValue {
                name: sdef.name.clone(),
                bases: vec![],
                methods,
                gen_methods: HashMap::new(),
                class_vars: HashMap::new(),
                field_defaults: vec![],
                field_mutability,
                field_access: HashMap::new(),
                method_access: HashMap::new(),
                static_method_names: std::collections::HashSet::new(),
                class_method_names: std::collections::HashSet::new(),
                static_vars: HashMap::new(),
                new_type_base: None,
            });

            members.insert(sdef.name.clone(), Value::Class(cls));
        }

        for sig in sigs {
            let symbol = format!("{}_tl\0", sig.name);
            let has_sym = unsafe {
                lib.get::<unsafe extern "C" fn(*const i64, i32) -> i64>(symbol.as_bytes())
                    .is_ok()
            };
            if has_sym {
                let ptr_params: Vec<crate::interpreter::PtrParam> = sig
                    .params
                    .iter()
                    .map(|(_, ct)| Self::sig_to_ptr_param_fn(ct))
                    .collect();
                let fn_ref = Arc::new(NativeFnRef {
                    lib_path: lib_path_buf.clone(),
                    fn_name: sig.name.clone(),
                    n_params: sig.params.len(),
                    min_params: sig.n_required,
                    param_mutabilities: vec![false; sig.params.len()],
                    ptr_params,
                    raw_fn_ptr: 0,
                    cached_fn_ptr: std::sync::atomic::AtomicUsize::new(0),
                });
                members.insert(sig.name.clone(), Value::NativeFunction(fn_ref));
            }
        }

        // Register into global scope so module-level calls and get_global() from
        // native code resolve. Struct classes are registered so native wrappers can
        // call get_global("VECTOR") then call_fn to construct instances.
        for (name, value) in &members {
            self.scopes[0]
                .entry(name.clone())
                .or_insert_with(|| Var::new(value.clone(), false));
        }

        self.native_libs.insert(lib_path_buf, NativeLibWrapper(lib));

        Ok(Rc::new(NamespaceData {
            name: module_name.to_string(),
            members,
        }))
    }

    // ---------------------------------------------------------------------------
    // Block execution helpers
    // ---------------------------------------------------------------------------

    /// 文のリストを順に実行する。Normal 以外のシグナルが発生したら即返す。
    pub(super) fn exec_block(&mut self, stmts: &[Stmt]) -> Result<ExecResult, String> {
        for stmt in stmts {
            match self.exec(stmt)? {
                ExecResult::Normal => {}
                signal => return Ok(signal),
            }
        }
        Ok(ExecResult::Normal)
    }

    /// 新しいスコープを積んでから文のリストを実行し、完了後にスコープを取り除く。
    pub(super) fn exec_scoped_block(&mut self, stmts: &[Stmt]) -> Result<ExecResult, String> {
        self.push_scope();
        let result = self.exec_block(stmts);
        self.pop_scope();
        result
    }

    // ---------------------------------------------------------------------------
    // Closure capture
    // ---------------------------------------------------------------------------

    /// 関数本体のフリー変数を分析して、現在の非グローバルスコープからキャプチャ環境を構築する。
    pub(super) fn capture_env(
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
    pub(super) fn apply_value_call(
        &mut self,
        callee: Value,
        arg: Value,
        label: &str,
    ) -> Result<Value, String> {
        let evaled = vec![(None, arg)];
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

// ---------------------------------------------------------------------------
// Misc free helpers
// ---------------------------------------------------------------------------

/// Simple non-cryptographic hash of a string — used to generate stable temp
/// file names for cpp bridge DLLs.
fn simple_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// ---------------------------------------------------------------------------
// フリー変数分析ヘルパー（モジュールプライベート）
// ---------------------------------------------------------------------------

fn collect_declared_names(stmts: &[Stmt], out: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::Let(name, _, _)
            | Stmt::Const(name, _, _)
            | Stmt::Mut(name, _, _)
            | Stmt::Static(name, _, _) => {
                out.insert(name.clone());
            }
            Stmt::LetTuple { targets, .. } => {
                for t in targets {
                    match t {
                        TupleTarget::Let(n) | TupleTarget::Mut(n) | TupleTarget::Bare(n) => {
                            out.insert(n.clone());
                        }
                        TupleTarget::Wildcard => {}
                    }
                }
            }
            Stmt::FnDef { name, .. }
            | Stmt::GenDef { name, .. }
            | Stmt::ClassDef { name, .. }
            | Stmt::TraitDef { name, .. }
            | Stmt::ProtocolDef { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::For { targets, body, .. } => {
                for t in targets {
                    out.insert(t.clone());
                }
                collect_declared_names(body, out);
            }
            Stmt::If {
                branches,
                else_body,
            } => {
                for (_, body) in branches {
                    collect_declared_names(body, out);
                }
                if let Some(body) = else_body {
                    collect_declared_names(body, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Block(body) => {
                collect_declared_names(body, out);
            }
            Stmt::Try {
                body,
                handlers,
                finally_body,
            } => {
                collect_declared_names(body, out);
                for h in handlers {
                    if let Some(alias) = &h.name {
                        out.insert(alias.clone());
                    }
                    collect_declared_names(&h.body, out);
                }
                if let Some(body) = finally_body {
                    collect_declared_names(body, out);
                }
            }
            _ => {}
        }
    }
}

fn collect_referenced_names(stmts: &[Stmt], out: &mut HashSet<String>) {
    for stmt in stmts {
        collect_referenced_names_stmt(stmt, out);
    }
}

fn collect_referenced_names_stmt(stmt: &Stmt, out: &mut HashSet<String>) {
    match stmt {
        Stmt::Expr(e) => collect_refs_expr(e, out),
        Stmt::Let(_, _, e) | Stmt::Const(_, _, e) | Stmt::Mut(_, _, e) | Stmt::Static(_, e, _) => {
            collect_refs_expr(e, out);
        }
        Stmt::LetTuple { value, .. } => {
            collect_refs_expr(value, out);
        }
        Stmt::Assign { name, value, .. } => {
            out.insert(name.clone());
            collect_refs_expr(value, out);
        }
        Stmt::CompoundAssign { name, value, .. } => {
            out.insert(name.clone());
            collect_refs_expr(value, out);
        }
        Stmt::AttrAssign { target, value } | Stmt::AttrCompoundAssign { target, value, .. } => {
            collect_refs_expr(target, out);
            collect_refs_expr(value, out);
        }
        Stmt::Return(Some(e)) | Stmt::BlockReturn(e, _) | Stmt::LoopYield(e) | Stmt::Yield(e) => {
            collect_refs_expr(e, out);
        }
        Stmt::Raise { exc: Some(e), .. } => collect_refs_expr(e, out),
        Stmt::If {
            branches,
            else_body,
        } => {
            for (cond, body) in branches {
                collect_refs_expr(cond, out);
                collect_referenced_names(body, out);
            }
            if let Some(body) = else_body {
                collect_referenced_names(body, out);
            }
        }
        Stmt::While { cond, body } => {
            collect_refs_expr(cond, out);
            collect_referenced_names(body, out);
        }
        Stmt::For { iter, body, .. } => {
            collect_refs_expr(iter, out);
            collect_referenced_names(body, out);
        }
        Stmt::Block(body) => collect_referenced_names(body, out),
        Stmt::FnDef { body, .. } | Stmt::GenDef { body, .. } => {
            collect_referenced_names(body, out);
        }
        Stmt::ClassDef { body, .. } | Stmt::TraitDef { body, .. } | Stmt::ProtocolDef { body, .. } => {
            collect_referenced_names(body, out);
        }
        Stmt::Try {
            body,
            handlers,
            finally_body,
        } => {
            collect_referenced_names(body, out);
            for h in handlers {
                collect_referenced_names(&h.body, out);
            }
            if let Some(body) = finally_body {
                collect_referenced_names(body, out);
            }
        }
        Stmt::Freeze(name, _) => {
            out.insert(name.clone());
        }
        _ => {}
    }
}

fn collect_refs_expr(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Ident(name) => {
            out.insert(name.clone());
        }
        Expr::BinOp { left, right, .. } => {
            collect_refs_expr(left, out);
            collect_refs_expr(right, out);
        }
        Expr::UnaryOp { operand, .. } => collect_refs_expr(operand, out),
        Expr::Call { func, args, .. } => {
            collect_refs_expr(func, out);
            for arg in args {
                collect_refs_expr(arg.expr(), out);
            }
        }
        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => {
            collect_refs_expr(object, out);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for item in items {
                collect_refs_expr(item, out);
            }
        }
        Expr::Dict(pairs) => {
            for (k, v) in pairs {
                collect_refs_expr(k, out);
                collect_refs_expr(v, out);
            }
        }
        Expr::Subscript { object, index } => {
            collect_refs_expr(object, out);
            collect_refs_expr(index, out);
        }
        Expr::Slice { begin, end, step } => {
            if let Some(e) = begin {
                collect_refs_expr(e, out);
            }
            if let Some(e) = end {
                collect_refs_expr(e, out);
            }
            if let Some(e) = step {
                collect_refs_expr(e, out);
            }
        }
        Expr::TemplateInstantiate { base, .. } => collect_refs_expr(base, out),
        Expr::IsType { expr, .. } => collect_refs_expr(expr, out),
        _ => {}
    }
}