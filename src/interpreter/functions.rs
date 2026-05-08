// functions.rs — 関数・ジェネレータ・オーバーロード実行
// (exec_fn_evaled / exec_fn / exec_generator / eval_call_args / bind_args /
//  dispatch_overload / dispatch_overload_evaled / overload_types_match / value_matches_ann)

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{CallArg, Param};

use super::{
    Interpreter, Value, Var, FnValue, GeneratorFnValue, GeneratorState,
    ExecResult, StackFrame,
    RAISE_SENTINEL, GENERATOR_YIELDS,
};

impl Interpreter {
    /// Execute a function with pre-evaluated argument list.
    /// `fn_name` is used only for traceback frames when an exception propagates through.
    pub(super) fn exec_fn_evaled(
        &mut self,
        fn_val: Rc<FnValue>,
        evaled: &[(Option<String>, Value)],
        self_val: Option<Value>,
        fn_name: &str,
    ) -> Result<Value, String> {
        let bindings = Self::bind_args(&fn_val.params, evaled, self_val.clone())?;

        let outer_scopes: Vec<_> = self.scopes.drain(1..).collect();
        self.push_scope();
        for (name, val, mutable) in bindings {
            self.declare_var(name, Var { value: val, mutable });
        }
        // Bind `Self` to the class when executing a method on an instance.
        if let Some(Value::Instance(inst_rc)) = &self_val {
            let class = inst_rc.borrow().class.clone();
            self.declare_var("Self".to_string(), Var { value: Value::Class(class), mutable: false });
        }

        self.call_stack.push(fn_name.to_string());
        let result = self.exec_block(&fn_val.body);
        self.call_stack.pop();

        self.scopes.truncate(1);
        self.scopes.extend(outer_scopes);

        // If an exception is propagating through this function (via ExecResult::Raise),
        // add a traceback frame, store in current_exception, and return the sentinel.
        if let Ok(ExecResult::Raise(mut raised)) = result {
            raised.frames.push(StackFrame {
                file: String::new(),
                line: 0,
                col: 0,
                fn_name: fn_name.to_string(),
                context: String::new(),
            });
            self.current_exception = Some(raised);
            return Err(RAISE_SENTINEL.to_string());
        }

        // If sentinel already propagated as Err (raise from a nested function), add frame.
        if let Err(ref e) = result {
            if e.as_str() == RAISE_SENTINEL {
                if let Some(ref mut raised) = self.current_exception {
                    raised.frames.push(StackFrame {
                        file: String::new(),
                        line: 0,
                        col: 0,
                        fn_name: fn_name.to_string(),
                        context: String::new(),
                    });
                }
                return Err(RAISE_SENTINEL.to_string());
            }
        }

        match result? {
            ExecResult::Return(v) => Ok(v),
            ExecResult::Normal | ExecResult::BlockReturn(_) => Ok(Value::None),
            ExecResult::Break => Err("SyntaxError: 'break' outside loop".to_string()),
            ExecResult::Continue => Err("SyntaxError: 'continue' outside loop".to_string()),
            ExecResult::Raise(_) => unreachable!("Raise already handled above"),
        }
    }

    pub(super) fn exec_fn(
        &mut self,
        fn_val: Rc<FnValue>,
        call_args: &[CallArg],
        self_val: Option<Value>,
        fn_name: &str,
    ) -> Result<Value, String> {
        let evaled = self.eval_call_args(call_args)?;
        self.exec_fn_evaled(fn_val, &evaled, self_val, fn_name)
    }

    /// Execute a generator function body, collecting all yielded values eagerly,
    /// and return a Generator object.
    /// `self_val` is `Some(instance)` when called as a method (binds `self` parameter).
    pub(super) fn exec_generator(&mut self, gen_fn: Rc<GeneratorFnValue>, call_args: &[CallArg], self_val: Option<Value>) -> Result<Value, String> {
        let evaled = self.eval_call_args(call_args)?;
        let bindings = Self::bind_args(&gen_fn.params, &evaled, self_val.clone())?;

        // Activate yield collection for this generator run.
        GENERATOR_YIELDS.with(|y| {
            *y.borrow_mut() = Some(Vec::new());
        });

        // Execute in a fresh scope (same isolation as exec_fn_evaled).
        let outer_scopes: Vec<_> = self.scopes.drain(1..).collect();
        self.push_scope();
        for (name, val, mutable) in bindings {
            self.declare_var(name, Var { value: val, mutable });
        }
        // Bind `Self` when executing a generator method on an instance.
        if let Some(Value::Instance(inst_rc)) = &self_val {
            let class = inst_rc.borrow().class.clone();
            self.declare_var("Self".to_string(), Var { value: Value::Class(class), mutable: false });
        }
        let exec_result = self.exec_block(&gen_fn.body);
        self.scopes.truncate(1);
        self.scopes.extend(outer_scopes);

        // Always collect (and clean up) the thread-local even on error.
        let yields = GENERATOR_YIELDS.with(|y| y.borrow_mut().take().unwrap_or_default());

        match exec_result? {
            ExecResult::Normal | ExecResult::BlockReturn(_) => {}
            ExecResult::Break    => return Err("SyntaxError: 'break' outside loop".to_string()),
            ExecResult::Continue => return Err("SyntaxError: 'continue' outside loop".to_string()),
            ExecResult::Return(_) => {} // silently ignored (parser already forbids return in gen)
            ExecResult::Raise(raised) => {
                self.current_exception = Some(raised);
                return Err(RAISE_SENTINEL.to_string());
            }
        }

        Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState { values: yields, index: 0 }))))
    }

    pub(super) fn eval_call_args(&mut self, call_args: &[CallArg]) -> Result<Vec<(Option<String>, Value)>, String> {
        let mut result = Vec::new();
        for arg in call_args {
            match arg {
                CallArg::Positional(e) => result.push((None, self.eval(e)?)),
                CallArg::Keyword { name, value } => result.push((Some(name.clone()), self.eval(value)?)),
            }
        }
        Ok(result)
    }

    pub(super) fn bind_args(
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

    pub(super) fn dispatch_overload(
        &mut self,
        candidates: Vec<Rc<FnValue>>,
        args: &[CallArg],
        self_val: Option<Value>,
    ) -> Result<Value, String> {
        let evaled = self.eval_call_args(args)?;
        self.dispatch_overload_evaled(candidates, evaled, self_val, "<overloaded>")
    }

    pub(super) fn dispatch_overload_evaled(
        &mut self,
        candidates: Vec<Rc<FnValue>>,
        evaled: Vec<(Option<String>, Value)>,
        self_val: Option<Value>,
        fn_name: &str,
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
            return self.exec_fn_evaled(count_matching[0].clone(), &evaled, self_val, fn_name);
        }

        // Multiple count-matching candidates: try type matching.
        for candidate in &count_matching {
            if Self::overload_types_match(candidate, &evaled, &self_val) {
                return self.exec_fn_evaled(candidate.clone(), &evaled, self_val.clone(), fn_name);
            }
        }

        // No exact type match; fall back to the first count-matching candidate.
        self.exec_fn_evaled(count_matching[0].clone(), &evaled, self_val, fn_name)
    }

    /// Returns true when every annotated parameter of `fn_val` matches the corresponding argument value.
    pub(super) fn overload_types_match(
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

    pub(super) fn value_matches_ann(val: &Value, ann: &str) -> bool {
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
            | ("Self",  Value::Instance(_))
        )
    }
}
