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
use std::rc::Rc;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::ast::{
    Accessibility, BinOp, Expr, ExceptHandler, FieldKind, MatchArm, MatchPattern,
    Param, Stmt, TemplateParam, TupleTarget,
};
use crate::token::Span;

use super::{
    CapturedVar, Interpreter, Value, Var, ExecResult,
    FnValue, TemplateFnValue, GeneratorFnValue, TemplateGenFnValue, TemplateClassValue,
    GeneratorState, NamespaceData, ModuleState, NativeFnRef, NativeLibWrapper,
    RaisedError, StackFrame,
    RAISE_SENTINEL, GENERATOR_YIELDS, BLOCK_YIELDS, LOOP_DEPTH,
};

impl Interpreter {
    /// 文（`Stmt`）を実行して `ExecResult` を返す。各 Stmt バリアントを専用メソッドに委譲する。
    pub fn exec(&mut self, stmt: &Stmt) -> Result<ExecResult, String> {
        match stmt {
            Stmt::Expr(expr) => {
                self.eval(expr)?;
                Ok(ExecResult::Normal)
            }
            Stmt::Let(name, expr) => self.exec_let(name, expr),
            Stmt::Const(name, expr) => {
                let value = self.eval(expr)?;
                self.declare_var(name.clone(), Var::new(value, false));
                Ok(ExecResult::Normal)
            }
            Stmt::Mut(name, expr) => {
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
                let result = self.apply_binop(op, lhs, rhs)?;
                self.attr_assign(target, result)?;
                Ok(ExecResult::Normal)
            }
            Stmt::CompoundAssign { name, op, value, .. } => {
                self.exec_compound_assign(name, op, value)
            }
            Stmt::Pass => Ok(ExecResult::Normal),
            Stmt::Field { .. } => Ok(ExecResult::Normal),
            Stmt::Break => {
                if !LOOP_DEPTH.with(|d| *d.borrow() > 0) {
                    return Err("SyntaxError: 'break' outside for/while loop".to_string());
                }
                Ok(ExecResult::BlockReturn(Value::None))
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
            Stmt::BlockReturn(expr) => {
                let val = self.eval(expr)?;
                Ok(ExecResult::BlockReturn(val))
            }
            Stmt::LoopYield(expr) => self.exec_loop_yield(expr),
            Stmt::If { branches, else_body } => self.exec_if_stmt(branches, else_body),
            Stmt::Match { subject, arms, .. } => self.exec_match_stmt(subject, arms),
            Stmt::While { cond, body } => self.exec_while_stmt(cond, body),
            Stmt::For { targets, iter, body } => self.exec_for_stmt(targets, iter, body),
            Stmt::Block(body) => self.exec_block_stmt(body),
            Stmt::FnDef { name, template_params, params, body, decorators, .. } => {
                self.exec_fn_def(name, template_params, params, body, decorators)
            }
            Stmt::Yield(expr) => {
                let val = self.eval(expr)?;
                GENERATOR_YIELDS.with(|y| {
                    if let Some(yields) = y.borrow_mut().as_mut() {
                        yields.push(val.clone());
                    }
                });
                Ok(ExecResult::Normal)
            }
            Stmt::GenDef { name, template_params, params, body, .. } => {
                self.exec_gen_def(name, template_params, params, body)
            }
            Stmt::TraitDef { name, body, .. } => self.exec_trait_def(name, body),
            Stmt::NewTypeDef { name, original } => self.exec_new_type_def(name, original),
            Stmt::EnumDef { name, variants } => self.exec_enum_def(name, variants),
            Stmt::ClassDef { name, template_params, bases, body, decorators } => {
                self.exec_class_def(name, template_params, bases, body, decorators)
            }
            Stmt::Freeze(name, span) => self.exec_freeze(name, span),
            Stmt::Raise { exc, span } => self.exec_raise(exc, span),
            Stmt::Try { body, handlers, finally_body } => {
                self.exec_try(body, handlers, finally_body)
            }
            Stmt::Import { lang, module, alias, body } => {
                let ns = self.exec_module(lang, module, body)?;
                let bind_name = alias.clone()
                    .unwrap_or_else(|| module.last().unwrap().clone());
                self.declare_var(bind_name, Var::new(Value::Namespace(ns), false));
                Ok(ExecResult::Normal)
            }
            Stmt::FromImport { lang, module, names, body } => {
                let ns = self.exec_module(lang, module, body)?;
                for (orig_name, alias) in names {
                    let bind_name = alias.clone().unwrap_or_else(|| orig_name.clone());
                    let val = ns.members.get(orig_name.as_str())
                        .cloned()
                        .ok_or_else(|| format!(
                            "ImportError: cannot import name '{}' from '{}'",
                            orig_name, module.join(".")
                        ))?;
                    self.declare_var(bind_name, Var::new(val, false));
                }
                Ok(ExecResult::Normal)
            }
            Stmt::AsyncAssign { target, stmts, .. } => self.exec_async_assign(target, stmts),
        }
    }

    // ---------------------------------------------------------------------------
    // Variable declarations & assignment
    // ---------------------------------------------------------------------------

    fn exec_let(&mut self, name: &str, expr: &Expr) -> Result<ExecResult, String> {
        // mut → let: deep copy してからフリーズプロトコルを適用する。
        // let → let: そのまま代入（コピー不要・再フリーズ不要）。
        // 式 → let: フリーズのみ（新規値なのでコピー不要）。
        let source_var = if let Expr::Ident(src) = expr {
            self.get_var(src).map(|v| (v.mutable, v.mutable_cell.is_some()))
        } else {
            None
        };
        let value = self.eval(expr)?;
        let value = match source_var {
            Some((true, _)) => {
                let copied = Self::deep_copy_value(value);
                self.apply_freeze_to_value(&copied)?;
                copied
            }
            Some((false, _)) => value,
            None => {
                self.apply_freeze_to_value(&value)?;
                value
            }
        };
        self.declare_var(name.to_string(), Var::new(value, false));
        Ok(ExecResult::Normal)
    }

    fn exec_let_tuple(&mut self, targets: &[TupleTarget], value: &Expr) -> Result<ExecResult, String> {
        let val = self.eval(value)?;
        let tuple_rc = match val {
            Value::Tuple(rc) => rc,
            _ => return Err("TypeError: cannot unpack non-tuple value in tuple assignment".to_string()),
        };
        let has_wildcard = targets.iter().any(|t| matches!(t, TupleTarget::Wildcard));
        let named = targets.iter().filter(|t| !matches!(t, TupleTarget::Wildcard)).count();
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
                    self.apply_freeze_to_value(&v)?;
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

    fn exec_static_var(&mut self, name: &str, expr: &Expr, span: &Span) -> Result<ExecResult, String> {
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

    fn exec_compound_assign(&mut self, name: &str, op: &BinOp, value: &Expr) -> Result<ExecResult, String> {
        let rhs = self.eval(value)?;
        let lhs = match self.get_var(name) {
            Some(v) if !v.mutable => {
                return Err(format!(
                    "TypeError: cannot assign to immutable variable '{name}'"
                ));
            }
            Some(v) => v.get_value(),
            None => return Err(format!("NameError: '{name}' is not defined")),
        };
        let value = self.apply_binop(op, lhs, rhs)?;
        self.assign_var(name, value)?;
        Ok(ExecResult::Normal)
    }

    // ---------------------------------------------------------------------------
    // Control flow signals
    // ---------------------------------------------------------------------------

    fn exec_loop_yield(&mut self, expr: &Expr) -> Result<ExecResult, String> {
        let val = self.eval(expr)?;
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

    fn exec_if_stmt(
        &mut self,
        branches: &[(Expr, Vec<Stmt>)],
        else_body: &Option<Vec<Stmt>>,
    ) -> Result<ExecResult, String> {
        for (cond, body) in branches {
            let val = self.eval(cond)?;
            if self.is_truthy(&val) {
                return self.exec_scoped_block(body);
            }
        }
        if let Some(body) = else_body {
            return self.exec_scoped_block(body);
        }
        Ok(ExecResult::Normal)
    }

    fn exec_match_stmt(&mut self, subject: &Expr, arms: &[MatchArm]) -> Result<ExecResult, String> {
        let subject_val = self.eval(subject)?;
        for arm in arms {
            let matched = match &arm.pattern {
                MatchPattern::Case(pattern_expr) => {
                    if matches!(pattern_expr, Expr::Ident(n) if n == "_") {
                        true
                    } else {
                        let pattern_val = self.eval(pattern_expr)?;
                        let result = self.apply_binop(
                            &BinOp::Eq,
                            subject_val.clone(),
                            pattern_val,
                        )?;
                        matches!(result, Value::Bool(true))
                    }
                }
                MatchPattern::IsType(type_name) => {
                    self.value_is_type(&subject_val, type_name)
                }
            };
            if matched {
                return self.exec_scoped_block(&arm.body);
            }
        }
        Ok(ExecResult::Normal)
    }

    fn exec_while_stmt(&mut self, cond: &Expr, body: &[Stmt]) -> Result<ExecResult, String> {
        LOOP_DEPTH.with(|d| *d.borrow_mut() += 1);
        let result = (|| {
            loop {
                let val = self.eval(cond)?;
                if !self.is_truthy(&val) {
                    break;
                }
                match self.exec_scoped_block(body)? {
                    ExecResult::Break | ExecResult::BlockReturn(Value::None) => break,
                    ExecResult::Continue | ExecResult::Normal => {}
                    r => return Ok(r),
                }
            }
            Ok(ExecResult::Normal)
        })();
        LOOP_DEPTH.with(|d| *d.borrow_mut() -= 1);
        result
    }

    fn exec_for_stmt(
        &mut self,
        targets: &[String],
        iter: &Expr,
        body: &[Stmt],
    ) -> Result<ExecResult, String> {
        let iter_val = self.eval(iter)?;
        let generator = match iter_val {
            Value::List(items) => {
                Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: items.borrow().clone(),
                    index: 0,
                })))
            }
            Value::Str(s) => {
                let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                Value::Generator(Rc::new(RefCell::new(GeneratorState { values: chars, index: 0 })))
            }
            Value::Set(items) => {
                Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: items.borrow().clone(),
                    index: 0,
                })))
            }
            Value::Tuple(td) => {
                Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: td.all_values().to_vec(),
                    index: 0,
                })))
            }
            Value::Generator(_) => iter_val,
            Value::Instance(_) => self.eval_method_call(iter_val, "__iter__", &[])?,
            Value::PyObject(ref handle) => {
                let items = super::py_interop::py_collect_iter(handle)?;
                Value::Generator(Rc::new(RefCell::new(GeneratorState { values: items, index: 0 })))
            }
            _ => return Err("TypeError: object is not iterable".to_string()),
        };
        LOOP_DEPTH.with(|d| *d.borrow_mut() += 1);
        let result = (|| {
            loop {
                match self.eval_method_call(generator.clone(), "next", &[]) {
                    Ok(item) => {
                        self.push_scope();
                        if targets.len() == 1 {
                            self.declare_var(targets[0].clone(), Var::new(item, true));
                        } else {
                            let elems = match &item {
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
                        match result? {
                            ExecResult::Break | ExecResult::BlockReturn(Value::None) => break,
                            ExecResult::Continue | ExecResult::Normal => {}
                            r => return Ok(r),
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

    fn exec_block_stmt(&mut self, body: &[Stmt]) -> Result<ExecResult, String> {
        // BlockReturn(non-None) は値を吸収して Normal を返す。
        // BlockReturn(None) (= break) は伝播させる（外側のループが捕捉できるよう）。
        match self.exec_scoped_block(body)? {
            ExecResult::Normal => Ok(ExecResult::Normal),
            ExecResult::BlockReturn(v) if !matches!(v, Value::None) => Ok(ExecResult::Normal),
            r => Ok(r),
        }
    }

    // ---------------------------------------------------------------------------
    // Function / generator definitions
    // ---------------------------------------------------------------------------

    fn exec_fn_def(
        &mut self,
        name: &str,
        template_params: &[TemplateParam],
        params: &[Param],
        body: &[Stmt],
        decorators: &[Expr],
    ) -> Result<ExecResult, String> {
        if !template_params.is_empty() {
            let tmpl = Rc::new(TemplateFnValue {
                template_params: template_params.to_vec(),
                params: params.to_vec(),
                body: body.to_vec(),
            });
            self.scopes.last_mut().unwrap()
                .insert(name.to_string(), Var::new(Value::TemplateFn(tmpl), false));
            return Ok(ExecResult::Normal);
        }

        let captured_env = if self.scopes.len() > 1 {
            self.capture_env(body, params)
        } else {
            HashMap::new()
        };
        let fn_val = Rc::new(FnValue {
            params: params.to_vec(),
            body: body.to_vec(),
            is_python: self.in_python_module,
            captured_env,
        });

        if decorators.is_empty() {
            let existing = self.scopes.last()
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
            self.scopes.last_mut().unwrap()
                .insert(name.to_string(), Var::new(new_value, false));
        } else {
            let mut value = Value::Function(fn_val);
            for dec_expr in decorators.iter().rev() {
                let dec = self.eval(dec_expr)?;
                value = self.apply_value_call(dec, value, name)?;
            }
            self.scopes.last_mut().unwrap()
                .insert(name.to_string(), Var::new(value, false));
        }
        Ok(ExecResult::Normal)
    }

    fn exec_gen_def(
        &mut self,
        name: &str,
        template_params: &[TemplateParam],
        params: &[Param],
        body: &[Stmt],
    ) -> Result<ExecResult, String> {
        if !template_params.is_empty() {
            let tmpl = Rc::new(TemplateGenFnValue {
                template_params: template_params.to_vec(),
                params: params.to_vec(),
                body: body.to_vec(),
            });
            self.scopes.last_mut().unwrap()
                .insert(name.to_string(), Var::new(Value::TemplateGenFn(tmpl), false));
        } else {
            let captured_env = if self.scopes.len() > 1 {
                self.capture_env(body, params)
            } else {
                HashMap::new()
            };
            let gen_fn = Rc::new(GeneratorFnValue {
                params: params.to_vec(),
                body: body.to_vec(),
                captured_env,
            });
            self.scopes.last_mut().unwrap()
                .insert(name.to_string(), Var::new(Value::GeneratorFn(gen_fn), false));
        }
        Ok(ExecResult::Normal)
    }

    // ---------------------------------------------------------------------------
    // Type definitions
    // ---------------------------------------------------------------------------

    fn exec_trait_def(&mut self, name: &str, body: &[Stmt]) -> Result<ExecResult, String> {
        let mut trait_access: HashMap<String, Accessibility> = HashMap::new();
        for stmt in body {
            if let Stmt::Field { name: fname, access, .. } = stmt {
                if *access != Accessibility::Public {
                    trait_access.insert(fname.clone(), access.clone());
                }
            }
            if let Stmt::FnDef { name: mname, access, .. } = stmt {
                if *access != Accessibility::Public {
                    trait_access.insert(mname.clone(), access.clone());
                }
            }
        }
        if !trait_access.is_empty() {
            self.trait_field_access.insert(name.to_string(), trait_access);
        }
        self.declare_var(name.to_string(), Var::new(Value::Trait(name.to_string()), false));
        Ok(ExecResult::Normal)
    }

    fn exec_new_type_def(&mut self, name: &str, original: &str) -> Result<ExecResult, String> {
        let orig_val = self.get_val(original)
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
                });
                self.declare_var(name.to_string(), Var::new(Value::Class(new_cls), false));
            }
            Value::Type(type_name) => {
                // `new_type Meters: int` → `class Meters: mut value: int` と等価
                let init_body = vec![Stmt::AttrAssign {
                    target: Expr::Attr {
                        object: Box::new(Expr::Ident("self".to_string())),
                        attr: "value".to_string(),
                    },
                    value: Expr::Ident("value".to_string()),
                }];
                let init_fn = Rc::new(FnValue {
                    params: vec![
                        crate::ast::Param {
                            name: "self".to_string(),
                            mutable: true,
                            type_ann: None,
                            default: None,
                        },
                        crate::ast::Param {
                            name: "value".to_string(),
                            mutable: false,
                            type_ann: Some(type_name.clone()),
                            default: None,
                        },
                    ],
                    body: init_body,
                    is_python: false,
                    captured_env: HashMap::new(),
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
            },
            value: Expr::Ident("value".to_string()),
        }];
        let init_fn = Rc::new(FnValue {
            params: vec![
                crate::ast::Param {
                    name: "self".to_string(),
                    mutable: true,
                    type_ann: None,
                    default: None,
                },
                crate::ast::Param {
                    name: "value".to_string(),
                    mutable: false,
                    type_ann: Some("int".to_string()),
                    default: None,
                },
            ],
            body: init_body,
            is_python: false,
            captured_env: HashMap::new(),
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
        });
        self.declare_var(item_type_name.clone(), Var::new(Value::Class(item_cls.clone()), false));

        // 各バリアントの値を計算し、enum クラスの const クラス変数として登録する
        let mut class_vars: HashMap<String, Value> = HashMap::new();
        let mut next_value: i64 = 0;
        for (variant_name, value_expr) in variants {
            let int_val = if let Some(expr) = value_expr {
                match self.eval(expr)? {
                    Value::Int(n) => n,
                    other => return Err(format!(
                        "TypeError: enum variant '{}' value must be int, got '{}'",
                        variant_name,
                        self.type_name(&other)
                    )),
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
        });
        self.declare_var(name.to_string(), Var::new(Value::Class(enum_cls), false));
        Ok(ExecResult::Normal)
    }

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
            self.declare_var(name.to_string(), Var::new(Value::TemplateClass(tmpl), false));
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
                    params,
                    body: mbody,
                    decorators: mdecs,
                    access: macc,
                    is_static,
                    is_class_method,
                    ..
                } => {
                    let fn_val = Rc::new(FnValue {
                        params: params.clone(),
                        body: mbody.clone(),
                        is_python: self.in_python_module,
                        captured_env: HashMap::new(),
                    });
                    if *is_static {
                        static_method_names.insert(mname.clone());
                    }
                    if *is_class_method {
                        class_method_names.insert(mname.clone());
                    }
                    if *macc != Accessibility::Public {
                        method_access.insert(mname.clone(), macc.clone());
                    }
                    if mdecs.is_empty() {
                        methods.entry(mname.clone()).or_default().push(fn_val);
                    } else {
                        let mut value = Value::Function(fn_val);
                        for dec_expr in mdecs.iter().rev() {
                            let dec = self.eval(dec_expr)?;
                            value = self.apply_value_call(dec, value, mname)?;
                        }
                        match value {
                            Value::Function(f) => {
                                methods.entry(mname.clone()).or_default().push(f)
                            }
                            other => return Err(format!(
                                "TypeError: method decorator on '{}' must return a function, got '{}'",
                                mname,
                                self.type_name(&other)
                            )),
                        }
                    }
                }
                Stmt::GenDef { name: mname, params, body: mbody, access: macc, .. } => {
                    if *macc != Accessibility::Public {
                        method_access.insert(mname.clone(), macc.clone());
                    }
                    gen_methods.insert(mname.clone(), Rc::new(GeneratorFnValue {
                        params: params.clone(),
                        body: mbody.clone(),
                        captured_env: HashMap::new(),
                    }));
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
                Stmt::Field { name: fname, kind, default, access: facc, .. } => {
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

    fn exec_freeze(&mut self, name: &str, span: &Span) -> Result<ExecResult, String> {
        let var = self.get_var(name)
            .ok_or_else(|| format!("{span}: NameError: '{name}' is not defined"))?;
        if !var.mutable {
            return Err(format!(
                "{span}: TypeError: cannot freeze immutable variable '{name}'"
            ));
        }
        if var.mutable_cell.is_some() {
            return Err(format!(
                "{span}: TypeError: cannot freeze '{name}' because it is captured by a closure"
            ));
        }
        let val = var.get_value();

        if let Value::Instance(ref inst_rc) = val {
            let class = inst_rc.borrow().class.clone();
            if let Some(overloads) = self.lookup_method_in_class(&class, "__freeze__") {
                if overloads.len() == 1 {
                    self.exec_fn(overloads[0].clone(), &[], Some(val.clone()), "__freeze__")?;
                } else {
                    self.dispatch_overload(overloads, &[], Some(val.clone()))?;
                }
            }
            Self::freeze_instance(inst_rc);
        }

        self.make_var_immutable(name);
        Ok(ExecResult::Normal)
    }

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
            inst.fields.insert("file".to_string(),         (Value::Str(span.file.to_string()), true));
            inst.fields.insert("line".to_string(),         (Value::Int(span.line as i64),      true));
            inst.fields.insert("col".to_string(),          (Value::Int(span.col as i64),       true));
            inst.fields.insert("code_context".to_string(), (Value::Str(context.clone()),       true));
            inst.fields.insert("Error::file".to_string(),         (Value::Str(span.file.to_string()), true));
            inst.fields.insert("Error::line".to_string(),         (Value::Int(span.line as i64),      true));
            inst.fields.insert("Error::col".to_string(),          (Value::Int(span.col as i64),       true));
            inst.fields.insert("Error::code_context".to_string(), (Value::Str(context),               true));
        }

        let fn_name = self.call_stack.last().cloned().unwrap_or_else(|| "<module>".to_string());
        let frame = StackFrame {
            file: span.file.to_string(),
            line: span.line,
            col: span.col,
            fn_name,
            context: self.get_context_lines(&span.file, span.line, 5),
        };
        Ok(ExecResult::Raise(RaisedError { exception: exc_val, frames: vec![frame] }))
    }

    fn exec_try(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        finally_body: &Option<Vec<Stmt>>,
    ) -> Result<ExecResult, String> {
        let body_result = self.exec_scoped_block(body);

        let raise_opt: Option<RaisedError> = match &body_result {
            Ok(ExecResult::Raise(r)) => Some(r.clone()),
            Err(e) if e.as_str() == RAISE_SENTINEL => self.current_exception.clone(),
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
            if !handled {
                // どの handler にもマッチしなかった場合:
                // final_result はそのまま body_result を維持し、元の伝播パスを保持する
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

    fn exec_async_assign(&mut self, target: &str, stmts: &[Stmt]) -> Result<ExecResult, String> {
        let mgr_val = self.get_var(target)
            .map(|v| v.get_value())
            .ok_or_else(|| format!("NameError: '{}' is not defined", target))?;

        let mgr_rc = match mgr_val {
            Value::AsyncManager(rc) => rc,
            other => return Err(format!(
                "TypeError: '<-' operator requires an AsyncManager, got '{}'",
                self.type_name(&other)
            )),
        };

        let env = super::async_mgr::capture_env(self);
        mgr_rc.borrow_mut().add_task(stmts.to_vec(), env);
        Ok(ExecResult::Normal)
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

        self.module_cache.insert(cache_key.clone(), ModuleState::Loading);

        if lang == "py-int" {
            let search_dirs = self.python_search_dirs.clone();
            let ns = super::py_interop::load_py_int_module(module, &search_dirs)
                .map_err(|e| e)?;
            self.module_cache.insert(cache_key, ModuleState::Loaded(ns.clone()));
            return Ok(ns);
        }

        // tl-auto / tlc: .tlc v1 に埋め込まれたネイティブ DLL がキャッシュにあれば優先する
        if lang == "tl-auto" || lang == "tlc" {
            let module_name = module.join(".");
            if let Some((_exports, dll_bytes)) = crate::partial_compiler::take_native_bytes(&module_name) {
                let ext = crate::partial_compiler::native_lib_ext();
                let stem = module.last().cloned().unwrap_or_default();
                let tmp_path = std::env::temp_dir().join(format!("{stem}_tl.{ext}"));
                match std::fs::write(&tmp_path, &dll_bytes) {
                    Ok(()) => {
                        match self.try_load_native_module(module, body, &tmp_path) {
                            Ok(ns) => {
                                self.module_cache.insert(cache_key, ModuleState::Loaded(ns.clone()));
                                return Ok(ns);
                            }
                            Err(e) => {
                                eprintln!("NativeLoad: failed: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("NativeLoad: cannot write temp DLL: {e}");
                    }
                }
            }
        }

        let prev_in_python = self.in_python_module;
        if lang == "py" { self.in_python_module = true; }
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
            .map(|(k, v)| (k.clone(), v.value.clone()))
            .collect();
        self.pop_scope();
        self.in_python_module = prev_in_python;

        // Python モジュールのメソッドが同モジュール内の他の関数を呼び出せるように
        // モジュールメンバをグローバルスコープに登録する（既存エントリは上書きしない）。
        for (name, value) in &members {
            self.scopes[0].entry(name.clone())
                .or_insert_with(|| Var::new(value.clone(), false));
        }

        let ns = Rc::new(NamespaceData { name: module.join("."), members });
        self.module_cache.insert(cache_key, ModuleState::Loaded(ns.clone()));
        Ok(ns)
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
            if let Stmt::FnDef { name, params, .. } = stmt {
                let symbol_name = format!("{name}_tl\0");
                let has_symbol = unsafe {
                    lib.get::<unsafe extern "C" fn(*const i64, i32) -> i64>(
                        symbol_name.as_bytes()
                    ).is_ok()
                };
                if has_symbol {
                    let fn_ref = Rc::new(NativeFnRef {
                        lib_path: lib_path_buf.clone(),
                        fn_name: name.clone(),
                        n_params: params.len(),
                        param_mutabilities: params.iter().map(|p| p.mutable).collect(),
                    });
                    members.insert(name.clone(), Value::NativeFunction(fn_ref));
                }
            }
        }

        for (name, value) in &members {
            self.scopes[0]
                .entry(name.clone())
                .or_insert_with(|| Var::new(value.clone(), false));
        }

        {
            let cb_ptr = super::native_api::get_callbacks();
            let symbol_name = b"tl_init\0";
            let init_result = unsafe {
                lib.get::<unsafe extern "C" fn(*const super::native_api::TlCallbacks)>(symbol_name)
            };
            if let Ok(tl_init) = init_result {
                unsafe { tl_init(cb_ptr) };
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
                let found = self.scopes[scope_idx].get(name.as_str()).map(|var| {
                    (var.mutable, var.mutable_cell.clone(), var.get_value())
                });

                if let Some((is_mutable, existing_cell, current_value)) = found {
                    if is_mutable {
                        let cell = if let Some(cell) = existing_cell {
                            cell
                        } else {
                            let cell = Rc::new(RefCell::new(current_value));
                            if let Some(var) = self.scopes[scope_idx].get_mut(name.as_str()) {
                                var.mutable_cell = Some(cell.clone());
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
            Value::Function(fn_val) => self.exec_fn_evaled(fn_val, &evaled, None, label),
            Value::OverloadedFn(candidates) => {
                self.dispatch_overload_evaled(candidates, evaled, None, label)
            }
            Value::Class(cls) => self.instantiate_evaled(cls, evaled),
            Value::Instance(ref inst_rc) => {
                let class = inst_rc.borrow().class.clone();
                let overloads = self.lookup_method_in_class(&class, "__call__")
                    .ok_or_else(|| format!(
                        "TypeError: '{}' object is not callable (no __call__ method)",
                        class.name
                    ))?;
                if overloads.len() == 1 {
                    self.exec_fn_evaled(overloads[0].clone(), &evaled, Some(callee), "__call__")
                } else {
                    self.dispatch_overload_evaled(overloads, evaled, Some(callee), "__call__")
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
// フリー変数分析ヘルパー（モジュールプライベート）
// ---------------------------------------------------------------------------

fn collect_declared_names(stmts: &[Stmt], out: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::Let(name, _) | Stmt::Const(name, _) | Stmt::Mut(name, _)
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
            Stmt::FnDef { name, .. } | Stmt::GenDef { name, .. }
            | Stmt::ClassDef { name, .. } | Stmt::TraitDef { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::For { targets, body, .. } => {
                for t in targets { out.insert(t.clone()); }
                collect_declared_names(body, out);
            }
            Stmt::If { branches, else_body } => {
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
            Stmt::Try { body, handlers, finally_body } => {
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
        Stmt::Let(_, e) | Stmt::Const(_, e) | Stmt::Mut(_, e) | Stmt::Static(_, e, _) => {
            collect_refs_expr(e, out);
        }
        Stmt::LetTuple { value, .. } => { collect_refs_expr(value, out); }
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
        Stmt::Return(Some(e)) | Stmt::BlockReturn(e) | Stmt::LoopYield(e) | Stmt::Yield(e) => {
            collect_refs_expr(e, out);
        }
        Stmt::Raise { exc: Some(e), .. } => collect_refs_expr(e, out),
        Stmt::If { branches, else_body } => {
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
        Stmt::ClassDef { body, .. } | Stmt::TraitDef { body, .. } => {
            collect_referenced_names(body, out);
        }
        Stmt::Try { body, handlers, finally_body } => {
            collect_referenced_names(body, out);
            for h in handlers {
                collect_referenced_names(&h.body, out);
            }
            if let Some(body) = finally_body {
                collect_referenced_names(body, out);
            }
        }
        Stmt::Freeze(name, _) => { out.insert(name.clone()); }
        _ => {}
    }
}

fn collect_refs_expr(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Ident(name) => { out.insert(name.clone()); }
        Expr::BinOp { left, right, .. } => {
            collect_refs_expr(left, out);
            collect_refs_expr(right, out);
        }
        Expr::UnaryOp { operand, .. } => collect_refs_expr(operand, out),
        Expr::Call { func, args } => {
            collect_refs_expr(func, out);
            for arg in args { collect_refs_expr(arg.expr(), out); }
        }
        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => {
            collect_refs_expr(object, out);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for item in items { collect_refs_expr(item, out); }
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
            if let Some(e) = begin { collect_refs_expr(e, out); }
            if let Some(e) = end   { collect_refs_expr(e, out); }
            if let Some(e) = step  { collect_refs_expr(e, out); }
        }
        Expr::TemplateInstantiate { base, .. } => collect_refs_expr(base, out),
        Expr::IsType { expr, .. } => collect_refs_expr(expr, out),
        _ => {}
    }
}
