// exec.rs — 文の実行 (exec / exec_block / exec_scoped_block)

use std::cell::RefCell;
use std::rc::Rc;
use std::collections::HashMap;

use crate::ast::{Expr, FieldKind, Stmt};

use super::{
    Interpreter, Value, Var, ExecResult,
    FnValue, TemplateFnValue, GeneratorFnValue, TemplateGenFnValue, TemplateClassValue,
    GeneratorState,
    RaisedError, StackFrame,
    RAISE_SENTINEL, GENERATOR_YIELDS,
};

impl Interpreter {
    pub fn exec(&mut self, stmt: &Stmt) -> Result<ExecResult, String> {
        match stmt {
            Stmt::Expr(expr) => {
                self.eval(expr)?;
                Ok(ExecResult::Normal)
            }
            Stmt::Let(name, expr) => {
                let value = self.eval(expr)?;
                if let Value::Instance(ref inst_rc) = value {
                    Self::freeze_instance(inst_rc);
                }
                self.declare_var(name.clone(), Var { value, mutable: false });
                Ok(ExecResult::Normal)
            }
            Stmt::Const(name, expr) => {
                let value = self.eval(expr)?;
                self.declare_var(name.clone(), Var { value, mutable: false });
                Ok(ExecResult::Normal)
            }
            Stmt::Mut(name, expr) => {
                let value = self.eval(expr)?;
                self.declare_var(name.clone(), Var { value, mutable: true });
                Ok(ExecResult::Normal)
            }
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
                let rhs = self.eval(value)?;
                let lhs = match self.get_var(name) {
                    Some(v) if !v.mutable => {
                        return Err(format!(
                            "TypeError: cannot assign to immutable variable '{name}'"
                        ));
                    }
                    Some(v) => v.value.clone(),
                    None => return Err(format!("NameError: '{name}' is not defined")),
                };
                let value = self.apply_binop(op, lhs, rhs)?;
                self.assign_var(name, value)?;
                Ok(ExecResult::Normal)
            }
            Stmt::Pass => Ok(ExecResult::Normal),
            Stmt::Field { .. } => Ok(ExecResult::Normal), // only valid in class bodies
            Stmt::Break => Ok(ExecResult::Break),
            Stmt::Continue => Ok(ExecResult::Continue),
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
            Stmt::BlockYield(expr) => {
                let val = self.eval(expr)?;
                Ok(ExecResult::BlockReturn(val))
            }
            Stmt::If { branches, else_body } => {
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
            Stmt::While { cond, body } => {
                loop {
                    let val = self.eval(cond)?;
                    if !self.is_truthy(&val) {
                        break;
                    }
                    match self.exec_scoped_block(body)? {
                        ExecResult::Break => break,
                        ExecResult::Continue | ExecResult::Normal => {}
                        r => return Ok(r),
                    }
                }
                Ok(ExecResult::Normal)
            }
            Stmt::For { target, iter, body } => {
                let iter_val = self.eval(iter)?;
                // Obtain a Generator from the iterable via the iterator protocol.
                let generator = match iter_val {
                    Value::List(items) => {
                        Value::Generator(Rc::new(RefCell::new(GeneratorState { values: items, index: 0 })))
                    }
                    Value::Str(s) => {
                        let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                        Value::Generator(Rc::new(RefCell::new(GeneratorState { values: chars, index: 0 })))
                    }
                    Value::Generator(_) => iter_val,
                    Value::Instance(_) => {
                        // Call __iter__() to obtain the generator.
                        self.eval_method_call(iter_val, "__iter__", &[])?
                    }
                    _ => return Err("TypeError: object is not iterable".to_string()),
                };
                loop {
                    match self.eval_method_call(generator.clone(), "next", &[]) {
                        Ok(item) => {
                            self.push_scope();
                            self.declare_var(target.clone(), Var { value: item, mutable: true });
                            let result = self.exec_block(body);
                            self.pop_scope();
                            match result? {
                                ExecResult::Break => break,
                                ExecResult::Continue | ExecResult::Normal => {}
                                r => return Ok(r),
                            }
                        }
                        Err(ref e) if e.starts_with("EndOfIteration") => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(ExecResult::Normal)
            }
            Stmt::Block(body) => {
                match self.exec_scoped_block(body)? {
                    ExecResult::BlockReturn(_) | ExecResult::Normal => Ok(ExecResult::Normal),
                    r => Ok(r),
                }
            }
            Stmt::FnDef { name, template_params, params, body, .. } => {
                if !template_params.is_empty() {
                    // Template function: store as TemplateFn (no overloading for now).
                    let tmpl = Rc::new(TemplateFnValue {
                        template_params: template_params.clone(),
                        params: params.clone(),
                        body: body.clone(),
                    });
                    self.scopes.last_mut().unwrap()
                        .insert(name.clone(), Var { value: Value::TemplateFn(tmpl), mutable: false });
                } else {
                    let fn_val = Rc::new(FnValue { params: params.clone(), body: body.clone() });

                    // Accumulate overloads within the same scope level.
                    let existing = self.scopes.last()
                        .and_then(|s| s.get(name.as_str()))
                        .map(|v| v.value.clone());
                    let new_value = match existing {
                        Some(Value::Function(prev)) => Value::OverloadedFn(vec![prev, fn_val]),
                        Some(Value::OverloadedFn(mut fns)) => {
                            fns.push(fn_val);
                            Value::OverloadedFn(fns)
                        }
                        _ => Value::Function(fn_val),
                    };
                    self.scopes.last_mut().unwrap()
                        .insert(name.clone(), Var { value: new_value, mutable: false });
                }
                Ok(ExecResult::Normal)
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
                if !template_params.is_empty() {
                    let tmpl = Rc::new(TemplateGenFnValue {
                        template_params: template_params.clone(),
                        params: params.clone(),
                        body: body.clone(),
                    });
                    self.scopes.last_mut().unwrap()
                        .insert(name.clone(), Var { value: Value::TemplateGenFn(tmpl), mutable: false });
                } else {
                    let gen_fn = Rc::new(GeneratorFnValue { params: params.clone(), body: body.clone() });
                    self.scopes.last_mut().unwrap()
                        .insert(name.clone(), Var { value: Value::GeneratorFn(gen_fn), mutable: false });
                }
                Ok(ExecResult::Normal)
            }
            Stmt::TraitDef { name, .. } => {
                self.declare_var(name.clone(), Var { value: Value::Trait(name.clone()), mutable: false });
                Ok(ExecResult::Normal)
            }
            Stmt::NewTypeDef { name, original } => {
                let orig_val = self.get_val(original)
                    .ok_or_else(|| format!("NameError: type '{original}' is not defined"))?;
                match orig_val {
                    Value::Class(orig_cls) => {
                        // Structural copy of the class with a new name.
                        // Instances of the new type will have class.name = new name,
                        // so `Self` inside methods correctly refers to the new type.
                        let new_cls = Rc::new(super::ClassValue {
                            name: name.clone(),
                            bases: orig_cls.bases.clone(),
                            methods: orig_cls.methods.clone(),
                            gen_methods: orig_cls.gen_methods.clone(),
                            field_defaults: orig_cls.field_defaults.clone(),
                            class_vars: orig_cls.class_vars.clone(),
                            field_mutability: orig_cls.field_mutability.clone(),
                        });
                        self.declare_var(name.clone(), Var { value: Value::Class(new_cls), mutable: false });
                    }
                    Value::Type(type_name) => {
                        // Primitive type: create a single-field wrapper class.
                        // new_type Meters: int  →  class Meters: mut value: int
                        let init_body = vec![
                            Stmt::AttrAssign {
                                target: Expr::Attr {
                                    object: Box::new(Expr::Ident("self".to_string())),
                                    attr: "value".to_string(),
                                },
                                value: Expr::Ident("value".to_string()),
                            },
                        ];
                        let init_fn = Rc::new(FnValue {
                            params: vec![
                                crate::ast::Param { name: "self".to_string(), mutable: true, type_ann: None },
                                crate::ast::Param { name: "value".to_string(), mutable: false, type_ann: Some(type_name.clone()) },
                            ],
                            body: init_body,
                        });
                        let mut methods = HashMap::new();
                        methods.insert("__init__".to_string(), vec![init_fn]);
                        let new_cls = Rc::new(super::ClassValue {
                            name: name.clone(),
                            bases: vec![],
                            methods,
                            gen_methods: HashMap::new(),
                            field_defaults: vec![],
                            class_vars: HashMap::new(),
                            field_mutability: HashMap::from([("value".to_string(), true)]),
                        });
                        self.declare_var(name.clone(), Var { value: Value::Class(new_cls), mutable: false });
                    }
                    _ => {
                        return Err(format!(
                            "TypeError: cannot create new_type from '{original}' — only classes and primitive types are supported"
                        ));
                    }
                }
                Ok(ExecResult::Normal)
            }
            Stmt::ClassDef { name, template_params, bases, body } => {
                if !template_params.is_empty() {
                    // Template class: store as TemplateClass without building ClassValue yet.
                    let tmpl = Rc::new(TemplateClassValue {
                        name: name.clone(),
                        template_params: template_params.clone(),
                        bases: bases.clone(),
                        body: body.clone(),
                    });
                    self.declare_var(name.clone(), Var { value: Value::TemplateClass(tmpl), mutable: false });
                    return Ok(ExecResult::Normal);
                }
                let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
                let mut gen_methods: HashMap<String, Rc<GeneratorFnValue>> = HashMap::new();
                let mut field_defaults = Vec::new();
                let mut class_vars: HashMap<String, Value> = HashMap::new();
                let mut field_mutability: HashMap<String, bool> = HashMap::new();
                for stmt in body {
                    match stmt {
                        Stmt::FnDef { name: mname, params, body: mbody, .. } => {
                            methods.entry(mname.clone()).or_default().push(Rc::new(FnValue {
                                params: params.clone(),
                                body: mbody.clone(),
                            }));
                        }
                        Stmt::GenDef { name: mname, params, body: mbody, .. } => {
                            gen_methods.insert(mname.clone(), Rc::new(GeneratorFnValue {
                                params: params.clone(),
                                body: mbody.clone(),
                            }));
                        }
                        Stmt::Field { name: fname, kind: FieldKind::Const, default: Some(init), .. } => {
                            let val = self.eval(init)?;
                            class_vars.insert(fname.clone(), val);
                        }
                        Stmt::Field { name: fname, kind, default, .. } => {
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
                    name: name.clone(),
                    bases: bases.clone(),
                    methods,
                    gen_methods,
                    field_defaults,
                    class_vars,
                    field_mutability,
                });
                self.declare_var(name.clone(), Var { value: Value::Class(cls), mutable: false });
                Ok(ExecResult::Normal)
            }
            Stmt::Freeze(name, span) => {
                let var = self.get_var(name)
                    .ok_or_else(|| format!("{span}: NameError: '{name}' is not defined"))?;
                if !var.mutable {
                    return Err(format!(
                        "{span}: TypeError: cannot freeze immutable variable '{name}'"
                    ));
                }
                let val = var.value.clone();

                if let Value::Instance(ref inst_rc) = val {
                    let class = inst_rc.borrow().class.clone();
                    // Call __freeze__ before freezing if the class defines it
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

            Stmt::Raise { exc, span } => {
                // Bare `raise` — re-raise the active exception.
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

                // Populate context fields on the instance.
                if let Value::Instance(ref inst_rc) = exc_val {
                    let context = self.get_context_lines(&span.file, span.line, 5);
                    let mut inst = inst_rc.borrow_mut();
                    // Set fields directly so `e.message`, `e.file` etc. work via normal attr access.
                    inst.fields.insert("file".to_string(),         (Value::Str(span.file.to_string()), true));
                    inst.fields.insert("line".to_string(),         (Value::Int(span.line as i64),       true));
                    inst.fields.insert("col".to_string(),          (Value::Int(span.col as i64),        true));
                    inst.fields.insert("code_context".to_string(), (Value::Str(context.clone()),        true));
                    // Also store under the "Error::" namespace for trait-field access.
                    inst.fields.insert("Error::file".to_string(),         (Value::Str(span.file.to_string()), true));
                    inst.fields.insert("Error::line".to_string(),         (Value::Int(span.line as i64),       true));
                    inst.fields.insert("Error::col".to_string(),          (Value::Int(span.col as i64),        true));
                    inst.fields.insert("Error::code_context".to_string(), (Value::Str(context),                true));
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

            Stmt::Try { body, handlers, finally_body } => {
                let body_result = self.exec_scoped_block(body);

                // Determine if the body produced a Raise signal.
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
                            None => true, // bare `except:` catches everything
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
                                self.declare_var(alias.clone(), Var { value: exc_val, mutable: false });
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
                        // No handler matched — `final_result` is already `body_result`,
                        // which preserves the original propagation path unchanged
                        // (Ok(ExecResult::Raise) for direct raises, Err(sentinel) for function raises).
                    }
                }

                // Execute `finally` block regardless of outcome.
                if let Some(finally) = finally_body {
                    let finally_result = self.exec_scoped_block(finally);
                    // If finally itself raises or returns, it takes precedence.
                    match finally_result {
                        Ok(ExecResult::Normal) => {}
                        Ok(signal) => return Ok(signal),
                        Err(e) => return Err(e),
                    }
                }

                final_result
            }
        }
    }

    pub(super) fn exec_block(&mut self, stmts: &[Stmt]) -> Result<ExecResult, String> {
        for stmt in stmts {
            match self.exec(stmt)? {
                ExecResult::Normal => {}
                signal => return Ok(signal),
            }
        }
        Ok(ExecResult::Normal)
    }

    pub(super) fn exec_scoped_block(&mut self, stmts: &[Stmt]) -> Result<ExecResult, String> {
        self.push_scope();
        let result = self.exec_block(stmts);
        self.pop_scope();
        result
    }
}
