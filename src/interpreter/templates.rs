// templates.rs — テンプレート展開・AST置換
// (check_template_constraints / type_satisfies_trait / instantiate_template / instantiate_template_class)
// + subst_* フリー関数 (AST substitution helpers for template instantiation)

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{CallArg, ExceptHandler, Expr, FieldKind, Param, Stmt, TemplateParam};

use super::{
    Interpreter, Value, FnValue, ClassValue, GeneratorFnValue,
    TemplateClassValue,
};

impl Interpreter {
    /// Verify that each concrete type arg satisfies the template parameter's trait constraints.
    pub(super) fn check_template_constraints(
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
    pub(super) fn type_satisfies_trait(&self, type_name: &str, trait_name: &str) -> Result<bool, String> {
        match self.get_val(type_name) {
            Some(Value::Class(cls)) => Ok(cls.bases.contains(&trait_name.to_string())),
            Some(_) => Ok(false), // built-in types and non-class values have no trait implementations
            None => Err(format!("NameError: type `{type_name}` is not defined")),
        }
    }

    /// Instantiate a template function or class with the given concrete type arguments.
    pub(super) fn instantiate_template(
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
                self.exec_fn(fn_val, call_args, None, "<template_fn>")
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
            Value::TemplateGenFn(tmpl) => {
                self.check_template_constraints(&tmpl.template_params, type_args)?;
                let type_map: HashMap<String, String> = tmpl.template_params.iter()
                    .zip(type_args.iter())
                    .map(|(p, t)| (p.name.clone(), t.clone()))
                    .collect();
                let concrete_params = subst_params(&tmpl.params, &type_map);
                let concrete_body = subst_stmts(&tmpl.body, &type_map);
                let gen_fn = Rc::new(GeneratorFnValue { params: concrete_params, body: concrete_body });
                self.exec_generator(gen_fn, call_args, None)
            }
            _ => Err("TemplateError: expression is not a template".to_string()),
        }
    }

    /// Build a concrete ClassValue from a substituted template class body, then instantiate it.
    pub(super) fn instantiate_template_class(
        &mut self,
        tmpl: &TemplateClassValue,
        concrete_body: Vec<Stmt>,
        call_args: &[CallArg],
    ) -> Result<Value, String> {
        let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
        let mut gen_methods: HashMap<String, Rc<GeneratorFnValue>> = HashMap::new();
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
        let cls = Rc::new(ClassValue {
            name: tmpl.name.clone(),
            bases: tmpl.bases.clone(),
            methods,
            gen_methods,
            field_defaults,
            class_vars,
            field_mutability,
        });
        self.instantiate(cls, call_args)
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
        Stmt::Yield(e) => Stmt::Yield(subst_expr(e, type_map)),
        Stmt::GenDef { name, template_params, params, yield_type, body } => Stmt::GenDef {
            name: name.clone(),
            template_params: template_params.clone(),
            params: params.clone(),
            yield_type: yield_type.clone(),
            body: subst_stmts(body, type_map),
        },
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
        Stmt::Freeze(name, span) => Stmt::Freeze(name.clone(), span.clone()),
        Stmt::NewTypeDef { name, original } => Stmt::NewTypeDef {
            name: name.clone(),
            original: subst_type(original, type_map),
        },
        Stmt::Try { body, handlers, finally_body } => Stmt::Try {
            body: subst_stmts(body, type_map),
            handlers: handlers.iter().map(|h| ExceptHandler {
                exc_type: h.exc_type.clone(),
                name: h.name.clone(),
                body: subst_stmts(&h.body, type_map),
            }).collect(),
            finally_body: finally_body.as_ref().map(|b| subst_stmts(b, type_map)),
        },
        Stmt::Raise { exc, span } => Stmt::Raise {
            exc: exc.as_ref().map(|e| subst_expr(e, type_map)),
            span: span.clone(),
        },
    }
}
