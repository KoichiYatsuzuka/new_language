use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{BinOp, CallArg, Expr, FieldKind, Param, Stmt, TemplateParam, UnaryOp};

// ---------------------------------------------------------------------------
// Function / Class / Instance value types
// ---------------------------------------------------------------------------

/// A template function definition (not yet instantiated with concrete types).
#[derive(Debug)]
pub struct TemplateFnValue {
    pub template_params: Vec<TemplateParam>,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
}

/// A template class definition (not yet instantiated with concrete types).
#[derive(Debug)]
pub struct TemplateClassValue {
    pub name: String,
    pub template_params: Vec<TemplateParam>,
    pub bases: Vec<String>,
    pub body: Vec<Stmt>,
}

#[derive(Debug)]
pub struct FnValue {
    params: Vec<Param>,
    body: Vec<Stmt>,
}

#[derive(Debug)]
pub struct ClassValue {
    name: String,
    bases: Vec<String>,
    /// Each method name maps to one or more overloads.
    methods: HashMap<String, Vec<Rc<FnValue>>>,
    /// Default values for `mut`/`let` instance fields: (name, default, mutable).
    field_defaults: Vec<(String, Value, bool)>,
    /// `const` class variables shared by all instances (always immutable).
    class_vars: HashMap<String, Value>,
    /// Declared mutability for every instance field (used when first assigning
    /// fields that have no default value, i.e., not yet in `inst.fields`).
    field_mutability: HashMap<String, bool>,
}

#[derive(Debug)]
pub struct InstanceData {
    pub class: Rc<ClassValue>,
    /// name → (value, mutable)
    pub fields: HashMap<String, (Value, bool)>,
}

// ---------------------------------------------------------------------------
// Value
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    None,
    List(Vec<Value>),
    Function(Rc<FnValue>),
    /// Two or more overloads of the same function name in the same scope.
    OverloadedFn(Vec<Rc<FnValue>>),
    Class(Rc<ClassValue>),
    Instance(Rc<RefCell<InstanceData>>),
    /// A type value — holds a built-in type name (`int`, `str`, `float`, `bool`).
    /// User-defined class types are represented by `Value::Class`.
    Type(String),
    /// An uninstantiated template function (parameterized over type variables).
    TemplateFn(Rc<TemplateFnValue>),
    /// An uninstantiated template class (parameterized over type variables).
    TemplateClass(Rc<TemplateClassValue>),
}

// ---------------------------------------------------------------------------
// Control-flow signals
// ---------------------------------------------------------------------------

#[derive(Debug)]
#[allow(dead_code)]
pub enum ExecResult {
    Normal,
    Break,
    Continue,
    Return(Value),
    BlockReturn(Value),
}

// ---------------------------------------------------------------------------
// Interpreter internals
// ---------------------------------------------------------------------------

struct Var {
    value: Value,
    mutable: bool,
}

pub struct Interpreter {
    scopes: Vec<HashMap<String, Var>>,
}

impl Interpreter {
    pub fn new() -> Self {
        let mut global: HashMap<String, Var> = HashMap::new();
        // Pre-define built-in type values so `int`, `str`, `float`, `bool`
        // can be used as expressions of type `type`.
        for name in ["int", "str", "float", "bool"] {
            global.insert(name.to_string(), Var { value: Value::Type(name.to_string()), mutable: false });
        }
        Self { scopes: vec![global] }
    }

    // --- Scope helpers ---

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    fn get_var(&self, name: &str) -> Option<&Var> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    fn get_val(&self, name: &str) -> Option<Value> {
        self.get_var(name).map(|v| v.value.clone())
    }

    fn declare_var(&mut self, name: String, var: Var) {
        self.scopes.last_mut().unwrap().insert(name, var);
    }

    fn assign_var(&mut self, name: &str, value: Value) -> Result<(), String> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(v) = scope.get_mut(name) {
                if !v.mutable {
                    return Err(format!(
                        "TypeError: cannot assign to immutable variable '{name}'"
                    ));
                }
                v.value = value;
                return Ok(());
            }
        }
        Err(format!("NameError: '{name}' is not defined"))
    }

    // --- Statement execution ---

    pub fn exec(&mut self, stmt: &Stmt) -> Result<ExecResult, String> {
        match stmt {
            Stmt::Expr(expr) => {
                self.eval(expr)?;
                Ok(ExecResult::Normal)
            }
            Stmt::Let(name, expr) => {
                let value = self.eval(expr)?;
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
                let items = match iter_val {
                    Value::List(items) => items,
                    Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string())).collect(),
                    _ => return Err("TypeError: object is not iterable".to_string()),
                };
                for item in items {
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
            Stmt::TraitDef { .. } => {
                // Traits are type-check-time constructs; no runtime representation yet.
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
                let cls = Rc::new(ClassValue {
                    name: name.clone(),
                    bases: bases.clone(),
                    methods,
                    field_defaults,
                    class_vars,
                    field_mutability,
                });
                self.declare_var(name.clone(), Var { value: Value::Class(cls), mutable: false });
                Ok(ExecResult::Normal)
            }
        }
    }

    fn exec_block(&mut self, stmts: &[Stmt]) -> Result<ExecResult, String> {
        for stmt in stmts {
            match self.exec(stmt)? {
                ExecResult::Normal => {}
                signal => return Ok(signal),
            }
        }
        Ok(ExecResult::Normal)
    }

    fn exec_scoped_block(&mut self, stmts: &[Stmt]) -> Result<ExecResult, String> {
        self.push_scope();
        let result = self.exec_block(stmts);
        self.pop_scope();
        result
    }

    // --- Attribute assignment helper ---

    fn attr_assign(&mut self, target: &Expr, rhs: Value) -> Result<(), String> {
        if let Expr::Attr { object, attr } = target {
            let obj_val = self.eval(object)?;
            match obj_val {
                Value::Instance(inst_rc) => {
                    let inst_class = inst_rc.borrow().class.clone();
                    if Self::lookup_class_var(&inst_class, attr).is_some() {
                        return Err(format!(
                            "TypeError: cannot assign to class variable '{attr}' (declared const)"
                        ));
                    }
                    let mut inst = inst_rc.borrow_mut();
                    if let Some((_, mutable)) = inst.fields.get(attr.as_str()) {
                        if !mutable {
                            return Err(format!(
                                "TypeError: cannot assign to immutable field '{attr}'"
                            ));
                        }
                        inst.fields.insert(attr.clone(), (rhs, true));
                    } else {
                        let is_mutable = inst.class.field_mutability
                            .get(attr.as_str()).copied().unwrap_or(true);
                        inst.fields.insert(attr.clone(), (rhs, is_mutable));
                    }
                    Ok(())
                }
                _ => Err("AttributeError: cannot set attribute on non-instance".to_string()),
            }
        } else if let Expr::TraitAccess { object, trait_name, attr } = target {
            let obj_val = self.eval(object)?;
            match obj_val {
                Value::Instance(inst_rc) => {
                    // Trait fields are stored with a namespaced key "TraitName::field"
                    let key = format!("{}::{}", trait_name, attr);
                    let mut inst = inst_rc.borrow_mut();
                    inst.fields.insert(key, (rhs, true));
                    Ok(())
                }
                _ => Err("AttributeError: cannot set trait field on non-instance".to_string()),
            }
        } else {
            Err("SyntaxError: invalid assignment target".to_string())
        }
    }

    // --- Function execution ---

    /// Execute a function with pre-evaluated argument list.
    fn exec_fn_evaled(
        &mut self,
        fn_val: Rc<FnValue>,
        evaled: &[(Option<String>, Value)],
        self_val: Option<Value>,
    ) -> Result<Value, String> {
        let bindings = Self::bind_args(&fn_val.params, evaled, self_val)?;

        let outer_scopes: Vec<_> = self.scopes.drain(1..).collect();
        self.push_scope();
        for (name, val, mutable) in bindings {
            self.declare_var(name, Var { value: val, mutable });
        }

        let result = self.exec_block(&fn_val.body);

        self.scopes.truncate(1);
        self.scopes.extend(outer_scopes);

        match result? {
            ExecResult::Return(v) => Ok(v),
            ExecResult::Normal | ExecResult::BlockReturn(_) => Ok(Value::None),
            ExecResult::Break => Err("SyntaxError: 'break' outside loop".to_string()),
            ExecResult::Continue => Err("SyntaxError: 'continue' outside loop".to_string()),
        }
    }

    fn exec_fn(
        &mut self,
        fn_val: Rc<FnValue>,
        call_args: &[CallArg],
        self_val: Option<Value>,
    ) -> Result<Value, String> {
        let evaled = self.eval_call_args(call_args)?;
        self.exec_fn_evaled(fn_val, &evaled, self_val)
    }

    fn eval_call_args(&mut self, call_args: &[CallArg]) -> Result<Vec<(Option<String>, Value)>, String> {
        let mut result = Vec::new();
        for arg in call_args {
            match arg {
                CallArg::Positional(e) => result.push((None, self.eval(e)?)),
                CallArg::Keyword { name, value } => result.push((Some(name.clone()), self.eval(value)?)),
            }
        }
        Ok(result)
    }

    fn bind_args(
        params: &[Param],
        evaled: &[(Option<String>, Value)],
        self_val: Option<Value>,
    ) -> Result<Vec<(String, Value, bool)>, String> {
        let mut result = Vec::new();

        let params_to_bind = if let (Some(sv), Some(p)) = (&self_val, params.first()) {
            if p.name == "self" {
                result.push(("self".to_string(), sv.clone(), p.mutable));
                &params[1..]
            } else {
                params
            }
        } else {
            params
        };

        if evaled.len() != params_to_bind.len() {
            return Err(format!(
                "TypeError: function takes {} argument(s), got {}",
                params_to_bind.len(),
                evaled.len()
            ));
        }

        let mut slots: Vec<Option<Value>> = vec![None; params_to_bind.len()];
        let mut positional_idx = 0usize;

        for (key, val) in evaled {
            match key {
                None => {
                    slots[positional_idx] = Some(val.clone());
                    positional_idx += 1;
                }
                Some(name) => {
                    let pos = params_to_bind.iter().position(|p| p.name == *name)
                        .ok_or_else(|| format!("TypeError: unexpected keyword argument '{name}'"))?;
                    if slots[pos].is_some() {
                        return Err(format!("TypeError: argument '{name}' given twice"));
                    }
                    slots[pos] = Some(val.clone());
                }
            }
        }

        for (i, slot) in slots.into_iter().enumerate() {
            match slot {
                Some(v) => result.push((params_to_bind[i].name.clone(), v, params_to_bind[i].mutable)),
                None => return Err(format!("TypeError: missing argument '{}'", params_to_bind[i].name)),
            }
        }

        Ok(result)
    }

    // --- Overload dispatch ---

    fn dispatch_overload(
        &mut self,
        candidates: Vec<Rc<FnValue>>,
        args: &[CallArg],
        self_val: Option<Value>,
    ) -> Result<Value, String> {
        let evaled = self.eval_call_args(args)?;
        self.dispatch_overload_evaled(candidates, evaled, self_val)
    }

    fn dispatch_overload_evaled(
        &mut self,
        candidates: Vec<Rc<FnValue>>,
        evaled: Vec<(Option<String>, Value)>,
        self_val: Option<Value>,
    ) -> Result<Value, String> {
        let call_count = evaled.len();
        let has_self = self_val.is_some();

        // Count effective params excluding `self`.
        let effective_param_count = |f: &FnValue| -> usize {
            let self_offset = if has_self && f.params.first().map(|p| p.name == "self").unwrap_or(false) { 1 } else { 0 };
            f.params.len() - self_offset
        };

        // Filter by argument count.
        let count_matching: Vec<Rc<FnValue>> = candidates.iter()
            .filter(|f| effective_param_count(f) == call_count)
            .cloned()
            .collect();

        if count_matching.is_empty() {
            let available: Vec<String> = candidates.iter()
                .map(|f| effective_param_count(f).to_string())
                .collect();
            return Err(format!(
                "TypeError: no overload takes {} argument(s) (overloads take: {})",
                call_count, available.join(", ")
            ));
        }

        if count_matching.len() == 1 {
            return self.exec_fn_evaled(count_matching[0].clone(), &evaled, self_val);
        }

        // Multiple count-matching candidates: try type matching.
        for candidate in &count_matching {
            if Self::overload_types_match(candidate, &evaled, &self_val) {
                return self.exec_fn_evaled(candidate.clone(), &evaled, self_val.clone());
            }
        }

        // No exact type match; fall back to the first count-matching candidate.
        self.exec_fn_evaled(count_matching[0].clone(), &evaled, self_val)
    }

    /// Returns true when every annotated parameter of `fn_val` matches the corresponding argument value.
    fn overload_types_match(
        fn_val: &FnValue,
        evaled: &[(Option<String>, Value)],
        self_val: &Option<Value>,
    ) -> bool {
        let params = if self_val.is_some() && fn_val.params.first().map(|p| p.name == "self").unwrap_or(false) {
            &fn_val.params[1..]
        } else {
            &fn_val.params[..]
        };

        // Map each argument to its target parameter slot.
        let mut slots: Vec<Option<&Value>> = vec![None; params.len()];
        let mut positional_idx = 0usize;

        for (key, val) in evaled {
            match key {
                None => {
                    if positional_idx >= params.len() { return false; }
                    slots[positional_idx] = Some(val);
                    positional_idx += 1;
                }
                Some(name) => {
                    if let Some(pos) = params.iter().position(|p| p.name == *name) {
                        slots[pos] = Some(val);
                    } else {
                        return false;
                    }
                }
            }
        }

        for (i, slot) in slots.iter().enumerate() {
            if let (Some(val), Some(ann)) = (slot, &params[i].type_ann) {
                if !Self::value_matches_ann(val, ann) {
                    return false;
                }
            }
        }
        true
    }

    fn value_matches_ann(val: &Value, ann: &str) -> bool {
        matches!(
            (ann, val),
            ("int",   Value::Int(_))
            | ("float", Value::Float(_))
            | ("str",   Value::Str(_))
            | ("bool",  Value::Bool(_))
            | ("None",  Value::None)
            | ("list",  Value::List(_))
            | ("type",  Value::Type(_))
            | ("type",  Value::Class(_))
        )
    }

    // --- Class instantiation ---

    fn instantiate(&mut self, class: Rc<ClassValue>, call_args: &[CallArg]) -> Result<Value, String> {
        let mut fields = HashMap::new();
        for (name, default_val, mutable) in &class.field_defaults {
            fields.insert(name.clone(), (default_val.clone(), *mutable));
        }
        let inst_rc = Rc::new(RefCell::new(InstanceData { class: class.clone(), fields }));
        let inst_val = Value::Instance(inst_rc);

        if let Some(init_overloads) = self.lookup_method_in_class(&class, "__init__") {
            if init_overloads.len() == 1 {
                self.exec_fn(init_overloads[0].clone(), call_args, Some(inst_val.clone()))?;
            } else {
                self.dispatch_overload(init_overloads, call_args, Some(inst_val.clone()))?;
            }
        }

        Ok(inst_val)
    }

    fn eval_method_call(
        &mut self,
        obj: Value,
        method_name: &str,
        args: &[CallArg],
    ) -> Result<Value, String> {
        match &obj {
            Value::Instance(inst_rc) => {
                let class = inst_rc.borrow().class.clone();
                let overloads = self.lookup_method_in_class(&class, method_name)
                    .ok_or_else(|| format!("AttributeError: '{}' has no method '{method_name}'", class.name))?;
                if overloads.len() == 1 {
                    self.exec_fn(overloads[0].clone(), args, Some(obj.clone()))
                } else {
                    self.dispatch_overload(overloads, args, Some(obj.clone()))
                }
            }
            _ => Err(format!(
                "AttributeError: '{}' object has no method '{method_name}'",
                self.type_name(&obj)
            )),
        }
    }

    fn lookup_method_in_class(&self, class: &Rc<ClassValue>, method_name: &str) -> Option<Vec<Rc<FnValue>>> {
        if let Some(overloads) = class.methods.get(method_name) {
            return Some(overloads.clone());
        }
        // Class-to-class inheritance is disabled; only trait-based inheritance is supported at parse time.
        // for base_name in &class.bases {
        //     if let Some(Value::Class(base_cls)) = self.get_val(base_name) {
        //         if let Some(overloads) = self.lookup_method_in_class(&base_cls, method_name) {
        //             return Some(overloads);
        //         }
        //     }
        // }
        None
    }

    /// Look up a `const` class variable by walking up the inheritance chain.
    fn lookup_class_var(class: &Rc<ClassValue>, name: &str) -> Option<Value> {
        class.class_vars.get(name).cloned()
        // Note: base-class lookup would require access to the interpreter's scope for name
        // resolution; class-var inheritance can be added later if needed.
    }

    // --- Expression evaluation ---

    pub fn eval(&mut self, expr: &Expr) -> Result<Value, String> {
        match expr {
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Float(f) => Ok(Value::Float(*f)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::None => Ok(Value::None),
            Expr::Ident(name) => self
                .get_var(name)
                .map(|v| v.value.clone())
                .ok_or_else(|| format!("NameError: '{name}' is not defined")),
            Expr::TraitAccess { object, trait_name, attr } => {
                let obj_val = self.eval(object)?;
                match obj_val {
                    Value::Instance(inst_rc) => {
                        let inst = inst_rc.borrow();
                        let key = format!("{}::{}", trait_name, attr);
                        if let Some((v, _)) = inst.fields.get(&key) {
                            return Ok(v.clone());
                        }
                        Err(format!(
                            "AttributeError: trait field '{trait_name}::{attr}' not found on '{}'",
                            inst.class.name
                        ))
                    }
                    _ => Err(format!(
                        "AttributeError: cannot access trait field on non-instance"
                    )),
                }
            }
            Expr::Attr { object, attr } => {
                let obj_val = self.eval(object)?;
                match &obj_val {
                    Value::Instance(inst_rc) => {
                        let inst = inst_rc.borrow();
                        // 1. Instance field
                        if let Some((v, _)) = inst.fields.get(attr.as_str()) {
                            return Ok(v.clone());
                        }
                        // 2. Class variable (const)
                        if let Some(v) = Self::lookup_class_var(&inst.class, attr) {
                            return Ok(v);
                        }
                        // 3. Method
                        if let Some(overloads) = inst.class.methods.get(attr.as_str()) {
                            return Ok(if overloads.len() == 1 {
                                Value::Function(overloads[0].clone())
                            } else {
                                Value::OverloadedFn(overloads.clone())
                            });
                        }
                        Err(format!(
                            "AttributeError: '{}' object has no attribute '{attr}'",
                            inst.class.name
                        ))
                    }
                    Value::Class(cls) => {
                        // Class variable
                        if let Some(v) = Self::lookup_class_var(cls, attr) {
                            return Ok(v);
                        }
                        if let Some(overloads) = cls.methods.get(attr.as_str()) {
                            return Ok(if overloads.len() == 1 {
                                Value::Function(overloads[0].clone())
                            } else {
                                Value::OverloadedFn(overloads.clone())
                            });
                        }
                        Err(format!("AttributeError: class '{}' has no attribute '{attr}'", cls.name))
                    }
                    _ => Err(format!(
                        "AttributeError: '{}' object has no attribute '{attr}'",
                        self.type_name(&obj_val)
                    )),
                }
            }
            Expr::List(items) => {
                let mut vals = Vec::new();
                for item in items {
                    vals.push(self.eval(item)?);
                }
                Ok(Value::List(vals))
            }
            Expr::UnaryOp { op, operand } => {
                let val = self.eval(operand)?;
                self.apply_unary(op, val)
            }
            Expr::BinOp { op, left, right, .. } => {
                match op {
                    BinOp::And => {
                        let lv = self.eval(left)?;
                        return if !self.is_truthy(&lv) { Ok(lv) } else { self.eval(right) };
                    }
                    BinOp::Or => {
                        let lv = self.eval(left)?;
                        return if self.is_truthy(&lv) { Ok(lv) } else { self.eval(right) };
                    }
                    _ => {}
                }
                let lv = self.eval(left)?;
                let rv = self.eval(right)?;
                self.apply_binop(op, lv, rv)
            }
            Expr::TemplateInstantiate { .. } => {
                Err("TemplateError: template expression must be immediately called (e.g. `Func[T](args)`)".to_string())
            }
            Expr::Call { func, args } => {
                // Template call: expr[T1, T2](args)
                if let Expr::TemplateInstantiate { base, type_args } = func.as_ref() {
                    let tmpl_val = self.eval(base)?;
                    return self.instantiate_template(tmpl_val, type_args, args);
                }

                // Method call: obj.method(args)
                if let Expr::Attr { object, attr } = func.as_ref() {
                    let obj_val = self.eval(object)?;
                    return self.eval_method_call(obj_val, attr, args);
                }

                // Builtin functions (not stored in scope)
                if let Expr::Ident(name) = func.as_ref() {
                    match name.as_str() {
                        "print" => {
                            let parts: Result<Vec<_>, _> = args.iter()
                                .map(|a| self.eval(a.expr()).map(|v| self.display(&v)))
                                .collect();
                            println!("{}", parts?.join(" "));
                            return Ok(Value::None);
                        }
                        "range" => {
                            let evaled: Result<Vec<_>, _> =
                                args.iter().map(|a| self.eval(a.expr())).collect();
                            let evaled = evaled?;
                            return match evaled.as_slice() {
                                [Value::Int(stop)] => {
                                    Ok(Value::List((0..*stop).map(Value::Int).collect()))
                                }
                                [Value::Int(start), Value::Int(stop)] => {
                                    Ok(Value::List((*start..*stop).map(Value::Int).collect()))
                                }
                                [Value::Int(start), Value::Int(stop), Value::Int(step)] => {
                                    let mut items = Vec::new();
                                    let mut i = *start;
                                    if *step > 0 {
                                        while i < *stop { items.push(Value::Int(i)); i += step; }
                                    } else if *step < 0 {
                                        while i > *stop { items.push(Value::Int(i)); i += step; }
                                    }
                                    Ok(Value::List(items))
                                }
                                _ => Err("TypeError: range() takes 1–3 integer arguments".to_string()),
                            };
                        }
                        "len" => {
                            if args.len() != 1 {
                                return Err("TypeError: len() takes exactly one argument".to_string());
                            }
                            let val = self.eval(args[0].expr())?;
                            return match val {
                                Value::List(items) => Ok(Value::Int(items.len() as i64)),
                                Value::Str(s) => Ok(Value::Int(s.len() as i64)),
                                _ => Err(format!("TypeError: object of type '{}' has no len()", self.type_name(&val))),
                            };
                        }
                        _ => {} // fall through to user-defined lookup
                    }
                }

                // User-defined function / overloaded function / class constructor
                let callee = self.eval(func)?;
                match callee {
                    Value::Function(fn_val) => self.exec_fn(fn_val, args, None),
                    Value::OverloadedFn(candidates) => self.dispatch_overload(candidates, args, None),
                    Value::Class(cls) => self.instantiate(cls, args),
                    Value::TemplateFn(_) | Value::TemplateClass(_) => Err(
                        "TemplateError: template must be called with explicit type arguments (e.g. `Func[T](args)`)".to_string()
                    ),
                    _ => Err(format!("TypeError: '{}' object is not callable", self.type_name(&callee))),
                }
            }
        }
    }

    // --- Display helpers ---

    fn type_name(&self, val: &Value) -> &'static str {
        match val {
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "str",
            Value::Bool(_) => "bool",
            Value::None => "NoneType",
            Value::List(_) => "list",
            Value::Function(_) | Value::OverloadedFn(_) => "function",
            Value::Class(_) | Value::Type(_) => "type",
            Value::Instance(_) => "object",
            Value::TemplateFn(_) | Value::TemplateClass(_) => "template",
        }
    }

    fn display(&self, val: &Value) -> String {
        match val {
            Value::Int(n) => n.to_string(),
            Value::Float(f) => {
                if f.fract() == 0.0 && f.abs() < 1e15 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                }
            }
            Value::Str(s) => s.clone(),
            Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            Value::None => "None".to_string(),
            Value::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| self.display_repr(v)).collect();
                format!("[{}]", parts.join(", "))
            }
            Value::Function(_) => "<function>".to_string(),
            Value::OverloadedFn(fns) => format!("<function ({} overloads)>", fns.len()),
            Value::Class(c) => format!("<class '{}'>", c.name),
            Value::Instance(i) => format!("<{} object>", i.borrow().class.name),
            Value::Type(name) => format!("<type '{name}'>"),
            Value::TemplateFn(t) => format!("<template fn ({} type params)>", t.template_params.len()),
            Value::TemplateClass(t) => format!("<template class '{}'>", t.name),
        }
    }

    // --- Template instantiation ---

    /// Verify that each concrete type arg satisfies the template parameter's trait constraints.
    fn check_template_constraints(
        &self,
        template_params: &[TemplateParam],
        type_args: &[String],
    ) -> Result<(), String> {
        if template_params.len() != type_args.len() {
            return Err(format!(
                "TemplateError: expected {} type argument(s), got {}",
                template_params.len(),
                type_args.len()
            ));
        }
        for (param, type_name) in template_params.iter().zip(type_args.iter()) {
            for constraint in &param.constraints {
                if !self.type_satisfies_trait(type_name, constraint)? {
                    return Err(format!(
                        "TemplateError: type `{type_name}` does not satisfy trait `{constraint}` \
                         (required for template parameter `{}`)",
                        param.name
                    ));
                }
            }
        }
        Ok(())
    }

    /// Returns true if the named type implements the given trait (i.e. has it in its bases).
    fn type_satisfies_trait(&self, type_name: &str, trait_name: &str) -> Result<bool, String> {
        match self.get_val(type_name) {
            Some(Value::Class(cls)) => Ok(cls.bases.contains(&trait_name.to_string())),
            Some(_) => Ok(false), // built-in types and non-class values have no trait implementations
            None => Err(format!("NameError: type `{type_name}` is not defined")),
        }
    }

    /// Instantiate a template function or class with the given concrete type arguments.
    fn instantiate_template(
        &mut self,
        tmpl_val: Value,
        type_args: &[String],
        call_args: &[CallArg],
    ) -> Result<Value, String> {
        match tmpl_val {
            Value::TemplateFn(tmpl) => {
                self.check_template_constraints(&tmpl.template_params, type_args)?;
                let type_map: HashMap<String, String> = tmpl.template_params.iter()
                    .zip(type_args.iter())
                    .map(|(p, t)| (p.name.clone(), t.clone()))
                    .collect();
                let concrete_params = subst_params(&tmpl.params, &type_map);
                let concrete_body = subst_stmts(&tmpl.body, &type_map);
                let fn_val = Rc::new(FnValue { params: concrete_params, body: concrete_body });
                self.exec_fn(fn_val, call_args, None)
            }
            Value::TemplateClass(tmpl) => {
                self.check_template_constraints(&tmpl.template_params, type_args)?;
                let type_map: HashMap<String, String> = tmpl.template_params.iter()
                    .zip(type_args.iter())
                    .map(|(p, t)| (p.name.clone(), t.clone()))
                    .collect();
                let concrete_body = subst_stmts(&tmpl.body, &type_map);
                self.instantiate_template_class(&tmpl, concrete_body, call_args)
            }
            _ => Err("TemplateError: expression is not a template".to_string()),
        }
    }

    /// Build a concrete ClassValue from a substituted template class body, then instantiate it.
    fn instantiate_template_class(
        &mut self,
        tmpl: &TemplateClassValue,
        concrete_body: Vec<Stmt>,
        call_args: &[CallArg],
    ) -> Result<Value, String> {
        let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
        let mut field_defaults = Vec::new();
        let mut class_vars: HashMap<String, Value> = HashMap::new();
        let mut field_mutability: HashMap<String, bool> = HashMap::new();
        for stmt in &concrete_body {
            match stmt {
                Stmt::FnDef { name: mname, params, body: mbody, .. } => {
                    methods.entry(mname.clone()).or_default().push(Rc::new(FnValue {
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
        let cls = Rc::new(ClassValue {
            name: tmpl.name.clone(),
            bases: tmpl.bases.clone(),
            methods,
            field_defaults,
            class_vars,
            field_mutability,
        });
        self.instantiate(cls, call_args)
    }

    fn display_repr(&self, val: &Value) -> String {
        match val {
            Value::Str(s) => format!("'{s}'"),
            Value::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| self.display_repr(v)).collect();
                format!("[{}]", parts.join(", "))
            }
            _ => self.display(val),
        }
    }

    fn is_truthy(&self, val: &Value) -> bool {
        match val {
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::None => false,
            Value::List(items) => !items.is_empty(),
            Value::Function(_) | Value::OverloadedFn(_) | Value::Class(_) | Value::Instance(_) | Value::Type(_)
            | Value::TemplateFn(_) | Value::TemplateClass(_) => true,
        }
    }

    fn apply_unary(&self, op: &UnaryOp, val: Value) -> Result<Value, String> {
        match op {
            UnaryOp::Neg => match val {
                Value::Int(n) => Ok(Value::Int(-n)),
                Value::Float(f) => Ok(Value::Float(-f)),
                _ => Err(format!("TypeError: bad operand type for unary `-`: {}", self.type_name(&val))),
            },
            UnaryOp::Not => Ok(Value::Bool(!self.is_truthy(&val))),
            UnaryOp::BitNot => match val {
                Value::Int(n) => Ok(Value::Int(!n)),
                _ => Err(format!("TypeError: bad operand type for unary `~`: {}", self.type_name(&val))),
            },
        }
    }

    fn apply_binop(&self, op: &BinOp, lv: Value, rv: Value) -> Result<Value, String> {
        match (op, &lv, &rv) {
            // Arithmetic
            (BinOp::Add, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a + *b)),
            (BinOp::Add, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a + *b)),
            (BinOp::Add, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + *b)),
            (BinOp::Add, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a + *b as f64)),
            (BinOp::Add, Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
            (BinOp::Sub, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a - *b)),
            (BinOp::Sub, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a - *b)),
            (BinOp::Sub, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 - *b)),
            (BinOp::Sub, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a - *b as f64)),
            (BinOp::Mul, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a * *b)),
            (BinOp::Mul, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a * *b)),
            (BinOp::Mul, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 * *b)),
            (BinOp::Mul, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a * *b as f64)),
            (BinOp::Div, Value::Int(a), Value::Int(b)) => {
                if *b == 0 { return Err("ZeroDivisionError: division by zero".to_string()); }
                Ok(Value::Float(*a as f64 / *b as f64))
            }
            (BinOp::Div, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a / *b)),
            (BinOp::Div, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 / *b)),
            (BinOp::Div, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a / *b as f64)),
            (BinOp::FloorDiv, Value::Int(a), Value::Int(b)) => {
                if *b == 0 { return Err("ZeroDivisionError: integer division by zero".to_string()); }
                Ok(Value::Int(a.div_euclid(*b)))
            }
            (BinOp::Mod, Value::Int(a), Value::Int(b)) => {
                if *b == 0 { return Err("ZeroDivisionError: modulo by zero".to_string()); }
                Ok(Value::Int(a.rem_euclid(*b)))
            }
            (BinOp::Pow, Value::Int(a), Value::Int(b)) => {
                if *b >= 0 {
                    Ok(Value::Int(a.pow(*b as u32)))
                } else {
                    Ok(Value::Float((*a as f64).powi(*b as i32)))
                }
            }
            (BinOp::Pow, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a.powf(*b))),
            (BinOp::Pow, Value::Int(a), Value::Float(b)) => Ok(Value::Float((*a as f64).powf(*b))),
            (BinOp::Pow, Value::Float(a), Value::Int(b)) => Ok(Value::Float(a.powi(*b as i32))),
            // Comparison
            (BinOp::Eq, _, _) => Ok(Value::Bool(self.values_eq(&lv, &rv))),
            (BinOp::NotEq, _, _) => Ok(Value::Bool(!self.values_eq(&lv, &rv))),
            (BinOp::Lt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(*a < *b)),
            (BinOp::Lt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(*a < *b)),
            (BinOp::Lt, Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) < *b)),
            (BinOp::Lt, Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a < (*b as f64))),
            (BinOp::Gt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(*a > *b)),
            (BinOp::Gt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(*a > *b)),
            (BinOp::Gt, Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) > *b)),
            (BinOp::Gt, Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a > (*b as f64))),
            (BinOp::LtEq, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(*a <= *b)),
            (BinOp::LtEq, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(*a <= *b)),
            (BinOp::GtEq, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(*a >= *b)),
            (BinOp::GtEq, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(*a >= *b)),
            // Bitwise
            (BinOp::BitAnd, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a & *b)),
            (BinOp::BitOr, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a | *b)),
            (BinOp::BitXor, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a ^ *b)),
            (BinOp::LShift, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a << *b)),
            (BinOp::RShift, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a >> *b)),
            _ => Err(format!(
                "TypeError: unsupported operand types for `{op:?}`: {} and {}",
                self.type_name(&lv), self.type_name(&rv)
            )),
        }
    }

    fn values_eq(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::None, Value::None) => true,
            (Value::Instance(a), Value::Instance(b)) => Rc::ptr_eq(a, b),
            (Value::Type(a), Value::Type(b)) => a == b,
            (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// AST substitution helpers (for template instantiation)
// ---------------------------------------------------------------------------

fn subst_type(type_name: &str, type_map: &HashMap<String, String>) -> String {
    type_map.get(type_name).cloned().unwrap_or_else(|| type_name.to_string())
}

fn subst_params(params: &[Param], type_map: &HashMap<String, String>) -> Vec<Param> {
    params.iter().map(|p| Param {
        name: p.name.clone(),
        mutable: p.mutable,
        type_ann: p.type_ann.as_ref().map(|t| subst_type(t, type_map)),
    }).collect()
}

fn subst_call_arg(arg: &CallArg, type_map: &HashMap<String, String>) -> CallArg {
    match arg {
        CallArg::Positional(e) => CallArg::Positional(subst_expr(e, type_map)),
        CallArg::Keyword { name, value } => CallArg::Keyword {
            name: name.clone(),
            value: subst_expr(value, type_map),
        },
    }
}

fn subst_expr(expr: &Expr, type_map: &HashMap<String, String>) -> Expr {
    match expr {
        Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Bool(_) | Expr::None => expr.clone(),
        Expr::Ident(name) => Expr::Ident(name.clone()),
        Expr::List(items) => Expr::List(items.iter().map(|e| subst_expr(e, type_map)).collect()),
        Expr::Attr { object, attr } => Expr::Attr {
            object: Box::new(subst_expr(object, type_map)),
            attr: attr.clone(),
        },
        Expr::TraitAccess { object, trait_name, attr } => Expr::TraitAccess {
            object: Box::new(subst_expr(object, type_map)),
            trait_name: trait_name.clone(),
            attr: attr.clone(),
        },
        Expr::BinOp { op, left, right, span } => Expr::BinOp {
            op: op.clone(),
            left: Box::new(subst_expr(left, type_map)),
            right: Box::new(subst_expr(right, type_map)),
            span: span.clone(),
        },
        Expr::UnaryOp { op, operand } => Expr::UnaryOp {
            op: op.clone(),
            operand: Box::new(subst_expr(operand, type_map)),
        },
        Expr::Call { func, args } => Expr::Call {
            func: Box::new(subst_expr(func, type_map)),
            args: args.iter().map(|a| subst_call_arg(a, type_map)).collect(),
        },
        Expr::TemplateInstantiate { base, type_args } => Expr::TemplateInstantiate {
            base: Box::new(subst_expr(base, type_map)),
            type_args: type_args.iter().map(|t| subst_type(t, type_map)).collect(),
        },
    }
}

fn subst_stmts(stmts: &[Stmt], type_map: &HashMap<String, String>) -> Vec<Stmt> {
    stmts.iter().map(|s| subst_stmt(s, type_map)).collect()
}

fn subst_stmt(stmt: &Stmt, type_map: &HashMap<String, String>) -> Stmt {
    match stmt {
        Stmt::Expr(e) => Stmt::Expr(subst_expr(e, type_map)),
        Stmt::Let(name, e) => Stmt::Let(name.clone(), subst_expr(e, type_map)),
        Stmt::Const(name, e) => Stmt::Const(name.clone(), subst_expr(e, type_map)),
        Stmt::Mut(name, e) => Stmt::Mut(name.clone(), subst_expr(e, type_map)),
        Stmt::Assign { name, value, span } => Stmt::Assign {
            name: name.clone(),
            value: subst_expr(value, type_map),
            span: span.clone(),
        },
        Stmt::AttrAssign { target, value } => Stmt::AttrAssign {
            target: subst_expr(target, type_map),
            value: subst_expr(value, type_map),
        },
        Stmt::AttrCompoundAssign { target, op, value } => Stmt::AttrCompoundAssign {
            target: subst_expr(target, type_map),
            op: op.clone(),
            value: subst_expr(value, type_map),
        },
        Stmt::CompoundAssign { name, op, value, span } => Stmt::CompoundAssign {
            name: name.clone(),
            op: op.clone(),
            value: subst_expr(value, type_map),
            span: span.clone(),
        },
        Stmt::If { branches, else_body } => Stmt::If {
            branches: branches.iter()
                .map(|(cond, body)| (subst_expr(cond, type_map), subst_stmts(body, type_map)))
                .collect(),
            else_body: else_body.as_ref().map(|b| subst_stmts(b, type_map)),
        },
        Stmt::While { cond, body } => Stmt::While {
            cond: subst_expr(cond, type_map),
            body: subst_stmts(body, type_map),
        },
        Stmt::For { target, iter, body } => Stmt::For {
            target: target.clone(),
            iter: subst_expr(iter, type_map),
            body: subst_stmts(body, type_map),
        },
        Stmt::Block(body) => Stmt::Block(subst_stmts(body, type_map)),
        Stmt::Return(e) => Stmt::Return(e.as_ref().map(|e| subst_expr(e, type_map))),
        Stmt::Break => Stmt::Break,
        Stmt::Continue => Stmt::Continue,
        Stmt::Pass => Stmt::Pass,
        Stmt::BlockReturn(e) => Stmt::BlockReturn(subst_expr(e, type_map)),
        Stmt::BlockYield(e) => Stmt::BlockYield(subst_expr(e, type_map)),
        Stmt::FnDef { name, template_params, params, return_type, body, is_virtual } => Stmt::FnDef {
            name: name.clone(),
            template_params: template_params.clone(),
            params: subst_params(params, type_map),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
            body: subst_stmts(body, type_map),
            is_virtual: *is_virtual,
        },
        Stmt::ClassDef { name, template_params, bases, body } => Stmt::ClassDef {
            name: name.clone(),
            template_params: template_params.clone(),
            bases: bases.clone(),
            body: subst_stmts(body, type_map),
        },
        Stmt::TraitDef { name, template_params, body } => Stmt::TraitDef {
            name: name.clone(),
            template_params: template_params.clone(),
            body: subst_stmts(body, type_map),
        },
        Stmt::Field { name, kind, type_ann, default } => Stmt::Field {
            name: name.clone(),
            kind: kind.clone(),
            type_ann: subst_type(type_ann, type_map),
            default: default.as_ref().map(|e| subst_expr(e, type_map)),
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn run(src: &str) -> Result<(), String> {
        let tokens = Lexer::new(src, "").tokenize();
        let stmts = Parser::new(tokens).parse_program()?;
        let mut interp = Interpreter::new();
        for stmt in &stmts {
            let _ = interp.exec(stmt)?;
        }
        Ok(())
    }

    fn eval(src: &str) -> Value {
        let tokens = Lexer::new(src, "").tokenize();
        let stmts = Parser::new(tokens).parse_program().unwrap();
        let mut interp = Interpreter::new();
        interp.eval(match &stmts[0] {
            Stmt::Expr(e) => e,
            _ => panic!("not an expr"),
        }).unwrap()
    }

    fn run_get(src: &str, var: &str) -> Value {
        let tokens = Lexer::new(src, "").tokenize();
        let stmts = Parser::new(tokens).parse_program().unwrap();
        let mut interp = Interpreter::new();
        for stmt in &stmts {
            let _ = interp.exec(stmt).unwrap();
        }
        interp.get_var(var).unwrap().value.clone()
    }

    #[test]
    fn test_arithmetic() {
        assert!(matches!(eval("2 + 3"), Value::Int(5)));
        assert!(matches!(eval("10 - 4"), Value::Int(6)));
        assert!(matches!(eval("3 * 4"), Value::Int(12)));
        assert!(matches!(eval("7 // 2"), Value::Int(3)));
        assert!(matches!(eval("7 % 3"), Value::Int(1)));
        assert!(matches!(eval("2 ** 10"), Value::Int(1024)));
    }

    #[test]
    fn test_float_arithmetic() {
        if let Value::Float(f) = eval("1.0 + 2.0") {
            assert!((f - 3.0).abs() < f64::EPSILON);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_string_concat() {
        if let Value::Str(s) = eval(r#""hello" + " " + "world""#) {
            assert_eq!(s, "hello world");
        } else {
            panic!();
        }
    }

    #[test]
    fn test_comparison() {
        assert!(matches!(eval("1 < 2"), Value::Bool(true)));
        assert!(matches!(eval("2 > 3"), Value::Bool(false)));
        assert!(matches!(eval("4 == 4"), Value::Bool(true)));
        assert!(matches!(eval("4 != 5"), Value::Bool(true)));
    }

    #[test]
    fn test_logical() {
        assert!(matches!(eval("True and False"), Value::Bool(false)));
        assert!(matches!(eval("True or False"), Value::Bool(true)));
        assert!(matches!(eval("not True"), Value::Bool(false)));
    }

    #[test]
    fn test_let_immutable() {
        assert!(run("let x = 1\nx = 2").is_err());
    }

    #[test]
    fn test_mut_mutable() {
        assert!(run("mut x = 1\nx = 2").is_ok());
    }

    #[test]
    fn test_compound_assign() {
        if let Value::Int(n) = run_get("mut x = 10\nx += 5", "x") {
            assert_eq!(n, 15);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_print_runs() {
        assert!(run(r#"print("hello", "world")"#).is_ok());
    }

    #[test]
    fn test_zero_division() {
        assert!(run("1 // 0").is_err());
    }

    // --- if ---

    #[test]
    fn test_if_true_branch() {
        if let Value::Int(n) = run_get("mut x = 0\nif True:\n    x = 1\n", "x") {
            assert_eq!(n, 1);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_if_false_else_branch() {
        if let Value::Int(n) = run_get("mut x = 0\nif False:\n    x = 1\nelse:\n    x = 2\n", "x") {
            assert_eq!(n, 2);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_if_scope_isolation() {
        assert!(run("if True:\n    let x = 1\nprint(x)\n").is_err());
    }

    // --- while ---

    #[test]
    fn test_while_loop() {
        if let Value::Int(n) = run_get("mut i = 0\nwhile i < 5:\n    i += 1\n", "i") {
            assert_eq!(n, 5);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_while_break() {
        if let Value::Int(n) = run_get(
            "mut i = 0\nwhile True:\n    i += 1\n    if i == 3:\n        break\n",
            "i",
        ) {
            assert_eq!(n, 3);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_while_scope_isolation() {
        assert!(run("mut cond = True\nwhile cond:\n    let x = 1\n    cond = False\nprint(x)\n").is_err());
    }

    // --- for ---

    #[test]
    fn test_for_range() {
        if let Value::Int(n) = run_get("mut s = 0\nfor i in range(5):\n    s += i\n", "s") {
            assert_eq!(n, 10);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_for_list() {
        if let Value::Int(n) = run_get("mut s = 0\nfor x in [1, 2, 3]:\n    s += x\n", "s") {
            assert_eq!(n, 6);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_for_loop_var_scope_isolation() {
        assert!(run("for i in range(3):\n    pass\nprint(i)\n").is_err());
    }

    #[test]
    fn test_for_body_scope_isolation() {
        assert!(run("for i in range(1):\n    let x = 99\nprint(x)\n").is_err());
    }

    // --- block ---

    #[test]
    fn test_block_scope_isolation() {
        assert!(run("block:\n    let x = 1\nprint(x)\n").is_err());
    }

    #[test]
    fn test_block_reads_outer() {
        assert!(run("let x = 1\nblock:\n    print(x)\n").is_ok());
    }

    #[test]
    fn test_block_modifies_outer() {
        if let Value::Int(n) = run_get("mut x = 0\nblock:\n    x = 42\n", "x") {
            assert_eq!(n, 42);
        } else {
            panic!();
        }
    }

    // --- builtins ---

    #[test]
    fn test_range_builtin() {
        if let Value::List(items) = eval("range(3)") {
            assert_eq!(items.len(), 3);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_len_builtin() {
        assert!(matches!(eval("len([1, 2, 3])"), Value::Int(3)));
    }

    // --- functions ---

    #[test]
    fn test_fn_call_returns_value() {
        let src = "fn add(a: int, b: int) -> int:\n    return a + b\nlet result = add(3, 4)\n";
        if let Value::Int(n) = run_get(src, "result") {
            assert_eq!(n, 7);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_fn_no_return_gives_none() {
        let src = "fn noop() -> None:\n    pass\nlet r = noop()\n";
        assert!(matches!(run_get(src, "r"), Value::None));
    }

    #[test]
    fn test_fn_recursion() {
        let src = "fn fact(n: int) -> int:\n    if n <= 1:\n        return 1\n    return n * fact(n - 1)\nlet r = fact(5)\n";
        if let Value::Int(n) = run_get(src, "r") {
            assert_eq!(n, 120);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_fn_kwarg_call() {
        let src = "fn sub(a: int, b: int) -> int:\n    return a - b\nlet r = sub(b=1, a=10)\n";
        if let Value::Int(n) = run_get(src, "r") {
            assert_eq!(n, 9);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_fn_scope_isolation() {
        let src = "fn f() -> None:\n    let x = 99\nf()\n";
        assert!(run(&format!("{src}print(x)\n")).is_err());
    }

    // --- overloading ---

    #[test]
    fn test_overload_by_count() {
        // Two overloads differing only in argument count.
        let src = concat!(
            "fn describe(x: int) -> str:\n",
            "    return \"one\"\n",
            "fn describe(x: int, y: int) -> str:\n",
            "    return \"two\"\n",
            "let a = describe(1)\n",
            "let b = describe(1, 2)\n",
        );
        if let (Value::Str(a), Value::Str(b)) = (run_get(src, "a"), run_get(src, "b")) {
            assert_eq!(a, "one");
            assert_eq!(b, "two");
        } else {
            panic!();
        }
    }

    #[test]
    fn test_overload_by_type() {
        // Two overloads with the same argument count but different types.
        let src = concat!(
            "fn process(x: int) -> str:\n",
            "    return \"int\"\n",
            "fn process(x: str) -> str:\n",
            "    return \"str\"\n",
            "let a = process(42)\n",
            "let b = process(\"hello\")\n",
        );
        if let (Value::Str(a), Value::Str(b)) = (run_get(src, "a"), run_get(src, "b")) {
            assert_eq!(a, "int");
            assert_eq!(b, "str");
        } else {
            panic!();
        }
    }

    #[test]
    fn test_overload_three_variants() {
        let src = concat!(
            "fn show(x: int) -> str:\n",
            "    return \"int\"\n",
            "fn show(x: str) -> str:\n",
            "    return \"str\"\n",
            "fn show(x: bool) -> str:\n",
            "    return \"bool\"\n",
            "let a = show(1)\n",
            "let b = show(\"hi\")\n",
            "let c = show(True)\n",
        );
        if let (Value::Str(a), Value::Str(b), Value::Str(c)) =
            (run_get(src, "a"), run_get(src, "b"), run_get(src, "c"))
        {
            assert_eq!(a, "int");
            assert_eq!(b, "str");
            assert_eq!(c, "bool");
        } else {
            panic!();
        }
    }

    #[test]
    fn test_overload_wrong_count_err() {
        let src = concat!(
            "fn f(x: int) -> None:\n    pass\n",
            "fn f(x: int, y: int) -> None:\n    pass\n",
            "f(1, 2, 3)\n",
        );
        assert!(run(src).is_err());
    }

    #[test]
    fn test_overload_method_by_type() {
        // Method overloading inside a class.
        let src = concat!(
            "class Printer:\n",
            "    fn print_val(self, x: int) -> str:\n",
            "        return \"int\"\n",
            "    fn print_val(self, x: str) -> str:\n",
            "        return \"str\"\n",
            "let p = Printer()\n",
            "let a = p.print_val(42)\n",
            "let b = p.print_val(\"hi\")\n",
        );
        if let (Value::Str(a), Value::Str(b)) = (run_get(src, "a"), run_get(src, "b")) {
            assert_eq!(a, "int");
            assert_eq!(b, "str");
        } else {
            panic!();
        }
    }

    #[test]
    fn test_overload_method_by_count() {
        let src = concat!(
            "class Calc:\n",
            "    fn add(self, x: int) -> int:\n",
            "        return x\n",
            "    fn add(self, x: int, y: int) -> int:\n",
            "        return x + y\n",
            "let c = Calc()\n",
            "let a = c.add(5)\n",
            "let b = c.add(3, 4)\n",
        );
        if let (Value::Int(a), Value::Int(b)) = (run_get(src, "a"), run_get(src, "b")) {
            assert_eq!(a, 5);
            assert_eq!(b, 7);
        } else {
            panic!();
        }
    }

    // --- classes ---

    #[test]
    fn test_class_instantiate() {
        // Fields have defaults → no required args → Point() is the right call.
        let src = "class Point:\n    mut x: int = 0\n    mut y: int = 0\nlet p = Point()\n";
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_class_instantiate_required_fields() {
        // Fields without defaults → auto-init requires args.
        let src = "class Point:\n    mut x: int\n    mut y: int\nlet p = Point(3, 4)\n";
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_class_init_sets_field() {
        let src = "class Dog:\n    mut name: str = \"\"\n    fn __init__(mut self, name: str) -> None:\n        self.name = name\nlet d = Dog(\"Rex\")\n";
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_class_method_call() {
        let src = "class Greeter:\n    fn greet(self) -> str:\n        return \"hello\"\nlet g = Greeter()\nlet r = g.greet()\n";
        if let Value::Str(s) = run_get(src, "r") {
            assert_eq!(s, "hello");
        } else {
            panic!();
        }
    }

    #[test]
    fn test_class_field_access() {
        // Fields have defaults; use defaults when instantiating.
        let src = "class Pair:\n    mut x: int = 10\n    mut y: int = 20\nlet p = Pair()\nlet r = p.x\n";
        if let Value::Int(n) = run_get(src, "r") {
            assert_eq!(n, 10);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_class_field_access_required() {
        // Fields without defaults require constructor args.
        let src = "class Pair:\n    mut x: int\n    mut y: int\nlet p = Pair(10, 20)\nlet r = p.x\n";
        if let Value::Int(n) = run_get(src, "r") {
            assert_eq!(n, 10);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_class_self_field_in_method() {
        let src = concat!(
            "class Box:\n",
            "    mut value: int = 0\n",
            "    fn set(mut self, v: int) -> None:\n",
            "        self.value = v\n",
            "    fn get(self) -> int:\n",
            "        return self.value\n",
            "let b = Box()\n",
            "b.set(42)\n",
            "let r = b.get()\n",
        );
        if let Value::Int(n) = run_get(src, "r") {
            assert_eq!(n, 42);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_class_inheritance_non_trait_parse_error() {
        // Class-to-class inheritance is not supported; must use traits instead.
        let src = concat!(
            "class Animal:\n",
            "    fn speak(self) -> str:\n",
            "        return \"...\"\n",
            "class Dog(Animal):\n",
            "    fn speak(self) -> str:\n",
            "        return \"Woof\"\n",
        );
        let tokens = crate::lexer::Lexer::new(src, "").tokenize();
        let result = crate::parser::Parser::new(tokens).parse_program();
        assert!(result.is_err(), "expected parse error for class-to-class inheritance");
        assert!(result.unwrap_err().contains("cannot inherit from `Animal`"));
    }

    #[test]
    fn test_class_inherit_non_trait_base_parse_error() {
        // Class-to-class inheritance is no longer supported; the parser must reject it.
        let src = concat!(
            "class Base:\n",
            "    fn hello(self) -> str:\n",
            "        return \"hi\"\n",
            "class Child(Base):\n",
            "    pass\n",
        );
        let tokens = crate::lexer::Lexer::new(src, "").tokenize();
        let result = crate::parser::Parser::new(tokens).parse_program();
        assert!(result.is_err(), "expected parse error for class-to-class inheritance");
        assert!(result.unwrap_err().contains("cannot inherit from `Base`"));
    }

    // --- trait ---

    #[test]
    fn test_trait_class_instantiate_combined_constructor() {
        // Class inheriting a trait; combined __init__ takes trait fields then class fields.
        let src = concat!(
            "trait HasValue:\n",
            "    mut value: int\n",
            "class Container(HasValue):\n",
            "    mut tag: str\n",
            "let c = Container(42, \"hello\")\n",
        );
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_trait_field_read_via_class_method() {
        // A method defined in the CLASS body reads a trait field via TraitAccess.
        let src = concat!(
            "trait HasValue:\n",
            "    mut value: int\n",
            "class Container(HasValue):\n",
            "    mut tag: str\n",
            "    fn get_value(self) -> int:\n",
            "        return self::HasValue.value\n",
            "    fn get_tag(self) -> str:\n",
            "        return self.tag\n",
            "let c = Container(99, \"hi\")\n",
            "let v = c.get_value()\n",
            "let t = c.get_tag()\n",
        );
        if let Value::Int(n) = run_get(src, "v") {
            assert_eq!(n, 99);
        } else {
            panic!("expected int for v");
        }
        if let Value::Str(s) = run_get(src, "t") {
            assert_eq!(s, "hi");
        } else {
            panic!("expected str for t");
        }
    }

    #[test]
    fn test_trait_virtual_override_executes() {
        // Virtual method overridden in class; override body actually runs.
        let src = concat!(
            "trait Shape:\n",
            "    fn area(self) -> float:\n",
            "        ...\n",
            "class Square(Shape):\n",
            "    mut side: float\n",
            "    fn area(self) -> float:\n",
            "        return self.side * self.side\n",
            "let s = Square(3.0)\n",
            "let a = s.area()\n",
        );
        if let Value::Float(f) = run_get(src, "a") {
            assert!((f - 9.0).abs() < 1e-9, "expected 9.0, got {f}");
        } else {
            panic!("expected float for a");
        }
    }

    #[test]
    fn test_trait_only_required_fields_no_class_fields() {
        // Class body has no required fields; only the trait's required field.
        let src = concat!(
            "trait Named:\n",
            "    mut name: str\n",
            "class Widget(Named):\n",
            "    fn get_name(self) -> str:\n",
            "        return self::Named.name\n",
            "let w = Widget(\"button\")\n",
            "let n = w.get_name()\n",
        );
        if let Value::Str(s) = run_get(src, "n") {
            assert_eq!(s, "button");
        } else {
            panic!("expected str for n");
        }
    }

    #[test]
    fn test_trait_field_write_via_method() {
        // A class method writes to a trait field using TraitAccess assignment.
        let src = concat!(
            "trait HasCount:\n",
            "    mut count: int\n",
            "class Counter(HasCount):\n",
            "    fn increment(mut self) -> None:\n",
            "        self::HasCount.count = self::HasCount.count + 1\n",
            "    fn get(self) -> int:\n",
            "        return self::HasCount.count\n",
            "let c = Counter(0)\n",
            "c.increment()\n",
            "c.increment()\n",
            "let r = c.get()\n",
        );
        if let Value::Int(n) = run_get(src, "r") {
            assert_eq!(n, 2);
        } else {
            panic!("expected int 2");
        }
    }
}
