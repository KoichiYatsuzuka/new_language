// scope.rs — スコープ管理 (push_scope / pop_scope / get_var / declare_var / assign_var)

use std::collections::HashMap;

use super::{Interpreter, Value, Var};

impl Interpreter {
    pub(super) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(super) fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    pub(super) fn get_var(&self, name: &str) -> Option<&Var> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    pub(super) fn get_val(&self, name: &str) -> Option<Value> {
        self.get_var(name).map(|v| v.value.clone())
    }

    pub(super) fn declare_var(&mut self, name: String, var: Var) {
        self.scopes.last_mut().unwrap().insert(name, var);
    }

    pub(super) fn assign_var(&mut self, name: &str, value: Value) -> Result<(), String> {
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

    /// Demote a variable from `mut` to immutable in-place (used by `freeze`).
    pub(super) fn make_var_immutable(&mut self, name: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(v) = scope.get_mut(name) {
                v.mutable = false;
                return;
            }
        }
    }
}
