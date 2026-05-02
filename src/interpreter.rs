use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{BinOp, CallArg, Expr, Param, Stmt, UnaryOp};

// ---------------------------------------------------------------------------
// Function / Class / Instance value types
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct FnValue {
    params: Vec<Param>,
    body: Vec<Stmt>,
}

#[derive(Debug)]
pub struct ClassValue {
    name: String,
    bases: Vec<String>,
    methods: HashMap<String, Rc<FnValue>>,
    /// Field defaults initialized from `mut`/`let`/`const` statements in class body.
    field_defaults: Vec<(String, Value, bool)>, // (name, default, mutable)
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
    Class(Rc<ClassValue>),
    Instance(Rc<RefCell<InstanceData>>),
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
        Self { scopes: vec![HashMap::new()] }
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
            Stmt::FnDef { name, params, body, .. } => {
                let fn_val = Rc::new(FnValue {
                    params: params.clone(),
                    body: body.clone(),
                });
                self.declare_var(name.clone(), Var { value: Value::Function(fn_val), mutable: false });
                Ok(ExecResult::Normal)
            }
            Stmt::ClassDef { name, bases, body } => {
                let mut methods = HashMap::new();
                let mut field_defaults = Vec::new();
                for stmt in body {
                    match stmt {
                        Stmt::FnDef { name: mname, params, body: mbody, .. } => {
                            methods.insert(mname.clone(), Rc::new(FnValue {
                                params: params.clone(),
                                body: mbody.clone(),
                            }));
                        }
                        Stmt::Mut(fname, init) => {
                            let val = self.eval(init)?;
                            field_defaults.push((fname.clone(), val, true));
                        }
                        Stmt::Let(fname, init) | Stmt::Const(fname, init) => {
                            let val = self.eval(init)?;
                            field_defaults.push((fname.clone(), val, false));
                        }
                        _ => {}
                    }
                }
                let cls = Rc::new(ClassValue {
                    name: name.clone(),
                    bases: bases.clone(),
                    methods,
                    field_defaults,
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
                    let mut inst = inst_rc.borrow_mut();
                    if let Some((_, mutable)) = inst.fields.get(attr.as_str()) {
                        if !mutable {
                            return Err(format!(
                                "TypeError: cannot assign to immutable field '{attr}'"
                            ));
                        }
                    }
                    inst.fields.insert(attr.clone(), (rhs, true));
                    Ok(())
                }
                _ => Err("AttributeError: cannot set attribute on non-instance".to_string()),
            }
        } else {
            Err("SyntaxError: invalid assignment target".to_string())
        }
    }

    // --- Function execution ---

    fn exec_fn(
        &mut self,
        fn_val: Rc<FnValue>,
        call_args: &[CallArg],
        self_val: Option<Value>,
    ) -> Result<Value, String> {
        let evaled = self.eval_call_args(call_args)?;
        let bindings = Self::bind_args(&fn_val.params, &evaled, self_val)?;

        // Replace all non-global scopes with a fresh function scope.
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

    // --- Class instantiation ---

    fn instantiate(&mut self, class: Rc<ClassValue>, call_args: &[CallArg]) -> Result<Value, String> {
        let mut fields = HashMap::new();
        for (name, default_val, mutable) in &class.field_defaults {
            fields.insert(name.clone(), (default_val.clone(), *mutable));
        }
        let inst_rc = Rc::new(RefCell::new(InstanceData { class: class.clone(), fields }));
        let inst_val = Value::Instance(inst_rc);

        if let Some(init_fn) = self.lookup_method_in_class(&class, "__init__") {
            self.exec_fn(init_fn, call_args, Some(inst_val.clone()))?;
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
                let method = self.lookup_method_in_class(&class, method_name)
                    .ok_or_else(|| format!("AttributeError: '{}' has no method '{method_name}'", class.name))?;
                self.exec_fn(method, args, Some(obj.clone()))
            }
            _ => Err(format!(
                "AttributeError: '{}' object has no method '{method_name}'",
                self.type_name(&obj)
            )),
        }
    }

    fn lookup_method_in_class(&self, class: &Rc<ClassValue>, method_name: &str) -> Option<Rc<FnValue>> {
        if let Some(m) = class.methods.get(method_name) {
            return Some(m.clone());
        }
        for base_name in &class.bases {
            if let Some(Value::Class(base_cls)) = self.get_val(base_name) {
                if let Some(m) = self.lookup_method_in_class(&base_cls, method_name) {
                    return Some(m);
                }
            }
        }
        None
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
            Expr::Attr { object, attr } => {
                let obj_val = self.eval(object)?;
                match &obj_val {
                    Value::Instance(inst_rc) => {
                        let inst = inst_rc.borrow();
                        if let Some((v, _)) = inst.fields.get(attr.as_str()) {
                            return Ok(v.clone());
                        }
                        if let Some(m) = inst.class.methods.get(attr.as_str()) {
                            return Ok(Value::Function(m.clone()));
                        }
                        Err(format!(
                            "AttributeError: '{}' object has no attribute '{attr}'",
                            inst.class.name
                        ))
                    }
                    Value::Class(cls) => {
                        if let Some(m) = cls.methods.get(attr.as_str()) {
                            return Ok(Value::Function(m.clone()));
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
            Expr::Call { func, args } => {
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

                // User-defined function or class constructor
                let callee = self.eval(func)?;
                match callee {
                    Value::Function(fn_val) => self.exec_fn(fn_val, args, None),
                    Value::Class(cls) => self.instantiate(cls, args),
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
            Value::Function(_) => "function",
            Value::Class(_) => "type",
            Value::Instance(_) => "object",
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
            Value::Class(c) => format!("<class '{}'>", c.name),
            Value::Instance(i) => format!("<{} object>", i.borrow().class.name),
        }
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
            Value::Function(_) | Value::Class(_) | Value::Instance(_) => true,
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
            _ => false,
        }
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
        // Variable declared inside function is not visible outside.
        let src = "fn f() -> None:\n    let x = 99\nf()\n";
        assert!(run(&format!("{src}print(x)\n")).is_err());
    }

    // --- classes ---

    #[test]
    fn test_class_instantiate() {
        let src = "class Point:\n    mut x = 0\n    mut y = 0\nlet p = Point()\n";
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_class_field_default() {
        let src = "class Counter:\n    mut count = 0\nlet c = Counter()\n";
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_class_init_sets_field() {
        let src = "class Dog:\n    mut name = \"\"\n    fn __init__(mut self, name: str) -> None:\n        self.name = name\nlet d = Dog(\"Rex\")\n";
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
        let src = "class Pair:\n    mut x = 10\n    mut y = 20\nlet p = Pair()\nlet r = p.x\n";
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
            "    mut value = 0\n",
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
    fn test_class_inheritance() {
        let src = concat!(
            "class Animal:\n",
            "    fn speak(self) -> str:\n",
            "        return \"...\"\n",
            "class Dog(Animal):\n",
            "    fn speak(self) -> str:\n",
            "        return \"Woof\"\n",
            "let d = Dog()\n",
            "let r = d.speak()\n",
        );
        if let Value::Str(s) = run_get(src, "r") {
            assert_eq!(s, "Woof");
        } else {
            panic!();
        }
    }

    #[test]
    fn test_class_inherit_base_method() {
        let src = concat!(
            "class Base:\n",
            "    fn hello(self) -> str:\n",
            "        return \"hi\"\n",
            "class Child(Base):\n",
            "    pass\n",
            "let c = Child()\n",
            "let r = c.hello()\n",
        );
        if let Value::Str(s) = run_get(src, "r") {
            assert_eq!(s, "hi");
        } else {
            panic!();
        }
    }
}
