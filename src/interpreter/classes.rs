// classes.rs — クラス・インスタンス管理
// (instantiate / eval_method_call / lookup_method_in_class / lookup_class_var / freeze_instance)

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::CallArg;

use super::{Interpreter, Value, ClassValue, FnValue, InstanceData, GeneratorState};

impl Interpreter {
    // --- Instance freeze ---

    /// Mark an instance as immutable: set `immutable = true` and flip all `mut` fields to immutable.
    pub(super) fn freeze_instance(inst_rc: &Rc<RefCell<InstanceData>>) {
        let mut inst = inst_rc.borrow_mut();
        inst.immutable = true;
        for (_, mutable) in inst.fields.values_mut() {
            *mutable = false;
        }
    }

    // --- Class instantiation ---

    pub(super) fn instantiate(&mut self, class: Rc<ClassValue>, call_args: &[CallArg]) -> Result<Value, String> {
        let mut fields = HashMap::new();
        for (name, default_val, mutable) in &class.field_defaults {
            fields.insert(name.clone(), (default_val.clone(), *mutable));
        }
        let inst_rc = Rc::new(RefCell::new(InstanceData { class: class.clone(), fields, immutable: false }));
        let inst_val = Value::Instance(inst_rc);

        if let Some(init_overloads) = self.lookup_method_in_class(&class, "__init__") {
            if init_overloads.len() == 1 {
                self.exec_fn(init_overloads[0].clone(), call_args, Some(inst_val.clone()), "__init__")?;
            } else {
                self.dispatch_overload(init_overloads, call_args, Some(inst_val.clone()))?;
            }
        }

        Ok(inst_val)
    }

    pub(super) fn eval_method_call(
        &mut self,
        obj: Value,
        method_name: &str,
        args: &[CallArg],
    ) -> Result<Value, String> {
        match &obj {
            Value::List(items) => {
                if method_name == "__iter__" {
                    if !args.is_empty() {
                        return Err("TypeError: list.__iter__() takes no arguments".to_string());
                    }
                    return Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                        values: items.clone(),
                        index: 0,
                    }))));
                }
                Err(format!("AttributeError: 'list' object has no method '{method_name}'"))
            }
            Value::Str(s) => {
                if method_name == "__iter__" {
                    if !args.is_empty() {
                        return Err("TypeError: str.__iter__() takes no arguments".to_string());
                    }
                    let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                    return Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                        values: chars,
                        index: 0,
                    }))));
                }
                Err(format!("AttributeError: 'str' object has no method '{method_name}'"))
            }
            Value::Instance(inst_rc) => {
                let class = inst_rc.borrow().class.clone();
                let inst_immutable = inst_rc.borrow().immutable;

                // Check gen_methods first (e.g. __iter__).
                if let Some(gen_fn) = class.gen_methods.get(method_name).cloned() {
                    return self.exec_generator(gen_fn, args, Some(obj.clone()));
                }

                let overloads = self.lookup_method_in_class(&class, method_name)
                    .ok_or_else(|| format!("AttributeError: '{}' has no method '{method_name}'", class.name))?;

                let callable: Vec<Rc<FnValue>> = if inst_immutable {
                    // Exclude overloads that require `mut self`
                    overloads.iter().filter(|f| {
                        f.params.first().map(|p| p.name != "self" || !p.mutable).unwrap_or(true)
                    }).cloned().collect()
                } else {
                    overloads
                };

                if callable.is_empty() {
                    return Err(format!(
                        "TypeError: cannot call mutable method '{method_name}' on immutable instance of '{}'",
                        class.name
                    ));
                }

                if callable.len() == 1 {
                    self.exec_fn(callable[0].clone(), args, Some(obj.clone()), method_name)
                } else {
                    self.dispatch_overload(callable, args, Some(obj.clone()))
                }
            }
            Value::Generator(state) => {
                if method_name != "next" {
                    return Err(format!(
                        "AttributeError: Generator object has no method '{method_name}'"
                    ));
                }
                if !args.is_empty() {
                    return Err("TypeError: Generator.next() takes no arguments".to_string());
                }
                let mut s = state.borrow_mut();
                if s.index < s.values.len() {
                    let val = s.values[s.index].clone();
                    s.index += 1;
                    Ok(val)
                } else {
                    Err("EndOfIteration: generator is exhausted".to_string())
                }
            }
            _ => Err(format!(
                "AttributeError: '{}' object has no method '{method_name}'",
                self.type_name(&obj)
            )),
        }
    }

    pub(super) fn lookup_method_in_class(&self, class: &Rc<ClassValue>, method_name: &str) -> Option<Vec<Rc<FnValue>>> {
        if let Some(overloads) = class.methods.get(method_name) {
            return Some(overloads.clone());
        }
        // Class-to-class inheritance is disabled; only trait-based inheritance is supported at parse time.
        None
    }

    /// Look up a `const` class variable by walking up the inheritance chain.
    pub(super) fn lookup_class_var(class: &Rc<ClassValue>, name: &str) -> Option<Value> {
        class.class_vars.get(name).cloned()
        // Note: base-class lookup would require access to the interpreter's scope for name
        // resolution; class-var inheritance can be added later if needed.
    }
}
