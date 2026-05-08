// eval.rs — 式の評価・attr_assign (eval / attr_assign)

use crate::ast::{BinOp, Expr};

use super::{Interpreter, Value};

impl Interpreter {
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
                    _ => Err("AttributeError: cannot access trait field on non-instance".to_string()),
                }
            }
            Expr::Attr { object, attr } => {
                let obj_val = self.eval(object)?;
                match &obj_val {
                    Value::Instance(inst_rc) => {
                        let inst = inst_rc.borrow();
                        // 1. Instance field (direct key)
                        if let Some((v, _)) = inst.fields.get(attr.as_str()) {
                            return Ok(v.clone());
                        }
                        // 1b. Trait-prefixed field fallback: find any "Trait::attr" key.
                        let suffix = format!("::{attr}");
                        if let Some((v, _)) = inst.fields.iter().find_map(|(k, v)| {
                            if k.ends_with(suffix.as_str()) { Some(v) } else { None }
                        }) {
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
                                _ => Err("TypeError: range() takes 1\u{2013}3 integer arguments".to_string()),
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

                // User-defined function / overloaded function / class constructor / generator
                // Derive a name for traceback frames (best-effort from the func expression).
                let call_name = match func.as_ref() {
                    Expr::Ident(n) => n.clone(),
                    _ => "<anonymous>".to_string(),
                };
                let callee = self.eval(func)?;
                match callee {
                    Value::Function(fn_val) => self.exec_fn(fn_val, args, None, &call_name),
                    Value::OverloadedFn(candidates) => {
                        let evaled_args = self.eval_call_args(args)?;
                        self.dispatch_overload_evaled(candidates, evaled_args, None, &call_name)
                    }
                    Value::Class(cls) => self.instantiate(cls, args),
                    Value::GeneratorFn(gen_fn) => self.exec_generator(gen_fn, args, None),
                    Value::TemplateFn(_) | Value::TemplateClass(_) | Value::TemplateGenFn(_) => Err(
                        "TemplateError: template must be called with explicit type arguments (e.g. `Func[T](args)`)".to_string()
                    ),
                    _ => Err(format!("TypeError: '{}' object is not callable", self.type_name(&callee))),
                }
            }
        }
    }

    // --- Attribute assignment helper ---

    pub(super) fn attr_assign(&mut self, target: &Expr, rhs: Value) -> Result<(), String> {
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
                        if inst.immutable {
                            return Err(format!(
                                "TypeError: cannot assign field '{attr}' on immutable instance"
                            ));
                        }
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
                    if let Some((_, false)) = inst.fields.get(&key) {
                        return Err(format!(
                            "TypeError: cannot assign to immutable trait field '{attr}'"
                        ));
                    }
                    if inst.immutable {
                        return Err(format!(
                            "TypeError: cannot assign field '{attr}' on immutable instance"
                        ));
                    }
                    inst.fields.insert(key, (rhs, true));
                    Ok(())
                }
                _ => Err("AttributeError: cannot set trait field on non-instance".to_string()),
            }
        } else {
            Err("SyntaxError: invalid assignment target".to_string())
        }
    }
}
