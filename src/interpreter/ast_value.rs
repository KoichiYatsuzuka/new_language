// ast_value.rs — Convert Rust AST nodes into Arrow Value::Namespace trees.
//
// `stmts_to_value` is called by the `parse_ar()` built-in.  Every field name
// mirrors what converter.ar and node_utils.ar expect:
//
//  * __type__   — the node-type discriminator (replaces Python's __class__.__name__)
//  * op         — BinOp as operator-string ("+", "==", …); UnaryOp as name ("NEG", …)
//  * branches   — list of (cond, body) Tuple pairs (indexed with [0]/[1])
//  * pairs      — list of (key, value) Tuple pairs (for dict / from-import names)

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{
    BinOp, CallArg, Expr, ExceptHandler, FieldKind, MatchArm, MatchPattern, Param, Stmt,
    TemplateParam, TupleTarget, UnaryOp,
};

use super::value::{NamespaceData, TupleData, Value};

// ---------------------------------------------------------------------------
// Low-level helpers
// ---------------------------------------------------------------------------

fn ns(type_name: &str, fields: Vec<(&str, Value)>) -> Value {
    let mut members = HashMap::new();
    members.insert("__type__".to_string(), Value::str(type_name.to_string()));
    for (k, v) in fields {
        members.insert(k.to_string(), v);
    }
    Value::Namespace(Rc::new(NamespaceData {
        name: type_name.to_string(),
        members,
    }))
}

/// Wrap two values as a 2-tuple (for branch / pair / variant lists).
fn pair(a: Value, b: Value) -> Value {
    Value::Tuple(Rc::new(TupleData::new(
        vec![a, b],
        vec!["any".to_string(), "any".to_string()],
    )))
}

fn stmts_list(stmts: &[Stmt]) -> Value {
    Value::List(Rc::new(RefCell::new(
        stmts.iter().map(stmt_to_value).collect(),
    )))
}

fn exprs_list(exprs: &[Expr]) -> Value {
    Value::List(Rc::new(RefCell::new(
        exprs.iter().map(expr_to_value).collect(),
    )))
}

fn str_list(strings: &[String]) -> Value {
    Value::List(Rc::new(RefCell::new(
        strings
            .iter()
            .map(|s| Value::str(s.as_str()))
            .collect(),
    )))
}

fn opt_expr(e: Option<&Expr>) -> Value {
    match e {
        Some(expr) => expr_to_value(expr),
        None => Value::None,
    }
}

fn opt_str(s: &Option<String>) -> Value {
    match s {
        Some(s) => Value::str(s.as_str()),
        None => Value::None,
    }
}

fn opt_stmts(stmts: Option<&Vec<Stmt>>) -> Value {
    match stmts {
        Some(v) => stmts_list(v),
        None => Value::None,
    }
}

fn template_params_list(params: &[TemplateParam]) -> Value {
    Value::List(Rc::new(RefCell::new(
        params
            .iter()
            .map(|p| ns("TemplateParam", vec![("name", Value::str(p.name.as_str()))]))
            .collect(),
    )))
}

fn params_list(params: &[Param]) -> Value {
    Value::List(Rc::new(RefCell::new(
        params
            .iter()
            .map(|p| {
                ns(
                    "Param",
                    vec![
                        ("name", Value::str(p.name.as_str())),
                        ("mutable", Value::Bool(p.mutable)),
                        ("type_ann", opt_str(&p.type_ann)),
                        ("default", opt_expr(p.default.as_ref())),
                    ],
                )
            })
            .collect(),
    )))
}

fn match_arms_list(arms: &[MatchArm]) -> Value {
    Value::List(Rc::new(RefCell::new(
        arms.iter()
            .map(|arm| {
                let pattern = match &arm.pattern {
                    MatchPattern::Case(expr) => {
                        ns("MatchPatternCase", vec![("expr", expr_to_value(expr))])
                    }
                    MatchPattern::IsType(type_name) => ns(
                        "MatchPatternIsType",
                        vec![("type_name", Value::str(type_name.as_str()))],
                    ),
                };
                ns(
                    "MatchArm",
                    vec![
                        ("pattern", pattern),
                        ("body", stmts_list(&arm.body)),
                    ],
                )
            })
            .collect(),
    )))
}

fn binop_str(op: &BinOp) -> Value {
    Value::str(op.as_str().to_string())
}

fn unaryop_str(op: &UnaryOp) -> Value {
    let s = match op {
        UnaryOp::Neg => "NEG",
        UnaryOp::Not => "NOT",
        UnaryOp::BitNot => "BIT_NOT",
    };
    Value::str(s.to_string())
}

fn call_args_list(args: &[CallArg]) -> Value {
    Value::List(Rc::new(RefCell::new(
        args.iter()
            .map(|arg| match arg {
                CallArg::Positional(expr) => {
                    ns("CallArgPositional", vec![("expr", expr_to_value(expr))])
                }
                CallArg::Keyword { name, value } => ns(
                    "CallArgKeyword",
                    vec![
                        ("name", Value::str(name.as_str())),
                        ("value", expr_to_value(value)),
                    ],
                ),
                CallArg::Variadic(exprs) => ns(
                    "CallArgVariadic",
                    vec![("exprs", Value::List(Rc::new(RefCell::new(
                        exprs.iter().map(expr_to_value).collect()
                    ))))],
                ),
            })
            .collect(),
    )))
}

fn branches_list(branches: &[(Expr, Vec<Stmt>)]) -> Value {
    Value::List(Rc::new(RefCell::new(
        branches
            .iter()
            .map(|(cond, body)| pair(expr_to_value(cond), stmts_list(body)))
            .collect(),
    )))
}

fn field_kind_value(kind: &FieldKind) -> Value {
    let name = match kind {
        FieldKind::Mut => "MUT",
        FieldKind::Let => "LET",
        FieldKind::Const => "CONST",
        FieldKind::StaticMut => "STATIC_MUT",
    };
    ns("FieldKind", vec![("name", Value::str(name.to_string()))])
}

fn tuple_targets_list(targets: &[TupleTarget]) -> Value {
    Value::List(Rc::new(RefCell::new(
        targets
            .iter()
            .map(|t| match t {
                TupleTarget::Let(name) => {
                    ns("TupleTargetLet", vec![("name", Value::str(name.as_str()))])
                }
                TupleTarget::Mut(name) => {
                    ns("TupleTargetMut", vec![("name", Value::str(name.as_str()))])
                }
                TupleTarget::Wildcard => ns("TupleTargetWildcard", vec![]),
                TupleTarget::Bare(name) => {
                    ns("TupleTargetBare", vec![("name", Value::str(name.as_str()))])
                }
            })
            .collect(),
    )))
}

fn handlers_list(handlers: &[ExceptHandler]) -> Value {
    Value::List(Rc::new(RefCell::new(
        handlers
            .iter()
            .map(|h| {
                ns(
                    "ExceptHandler",
                    vec![
                        ("exc_type", opt_str(&h.exc_type)),
                        ("name", opt_str(&h.name)),
                        ("body", stmts_list(&h.body)),
                    ],
                )
            })
            .collect(),
    )))
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn stmts_to_value(stmts: &[Stmt]) -> Value {
    stmts_list(stmts)
}

// ---------------------------------------------------------------------------
// Statement conversion
// ---------------------------------------------------------------------------

fn stmt_to_value(stmt: &Stmt) -> Value {
    match stmt {
        Stmt::Expr(expr) => ns("StmtExpr", vec![("expr", expr_to_value(expr))]),

        Stmt::Let(name, _, expr) => ns(
            "StmtLet",
            vec![
                ("name", Value::str(name.as_str())),
                ("expr", expr_to_value(expr)),
            ],
        ),
        Stmt::Const(name, _, expr) => ns(
            "StmtConst",
            vec![
                ("name", Value::str(name.as_str())),
                ("expr", expr_to_value(expr)),
            ],
        ),
        Stmt::Mut(name, _, expr) => ns(
            "StmtMut",
            vec![
                ("name", Value::str(name.as_str())),
                ("expr", expr_to_value(expr)),
            ],
        ),
        Stmt::Static(name, expr, _) => ns(
            "StmtStatic",
            vec![
                ("name", Value::str(name.as_str())),
                ("expr", expr_to_value(expr)),
            ],
        ),
        Stmt::LetTuple { targets, value, .. } => ns(
            "StmtLetTuple",
            vec![
                ("targets", tuple_targets_list(targets)),
                ("value", expr_to_value(value)),
            ],
        ),

        Stmt::Assign { name, value, .. } => ns(
            "StmtAssign",
            vec![
                ("name", Value::str(name.as_str())),
                ("value", expr_to_value(value)),
            ],
        ),
        Stmt::AttrAssign { target, value } => ns(
            "StmtAttrAssign",
            vec![
                ("target", expr_to_value(target)),
                ("value", expr_to_value(value)),
            ],
        ),
        Stmt::AttrCompoundAssign { target, op, value } => ns(
            "StmtAttrCompoundAssign",
            vec![
                ("target", expr_to_value(target)),
                ("op", binop_str(op)),
                ("value", expr_to_value(value)),
            ],
        ),
        Stmt::CompoundAssign { name, op, value, .. } => ns(
            "StmtCompoundAssign",
            vec![
                ("name", Value::str(name.as_str())),
                ("op", binop_str(op)),
                ("value", expr_to_value(value)),
            ],
        ),

        Stmt::If { branches, else_body } => ns(
            "StmtIf",
            vec![
                ("branches", branches_list(branches)),
                ("else_body", opt_stmts(else_body.as_ref())),
            ],
        ),
        Stmt::While { cond, body } => ns(
            "StmtWhile",
            vec![
                ("cond", expr_to_value(cond)),
                ("body", stmts_list(body)),
            ],
        ),
        Stmt::For { targets, iter, body } => ns(
            "StmtFor",
            vec![
                ("targets", str_list(targets)),
                ("iter", expr_to_value(iter)),
                ("body", stmts_list(body)),
            ],
        ),
        Stmt::Match { subject, arms, .. } => ns(
            "StmtMatch",
            vec![
                ("subject", expr_to_value(subject)),
                ("arms", match_arms_list(arms)),
            ],
        ),
        Stmt::Block(stmts) => ns("StmtBlock", vec![("stmts", stmts_list(stmts))]),

        Stmt::Return(e) => ns("StmtReturn", vec![("expr", opt_expr(e.as_ref()))]),
        Stmt::Break => ns("StmtBreak", vec![]),
        Stmt::Continue => ns("StmtContinue", vec![]),
        Stmt::Pass => ns("StmtPass", vec![]),

        Stmt::BlockReturn(expr, _) => {
            ns("StmtBlockReturn", vec![("expr", expr_to_value(expr))])
        }
        Stmt::LoopYield(expr) => ns("StmtLoopYield", vec![("expr", expr_to_value(expr))]),
        Stmt::Yield(expr) => ns("StmtYield", vec![("expr", expr_to_value(expr))]),
        Stmt::Freeze(name, _) => ns("StmtFreeze", vec![("name", Value::str(name.as_str()))]),

        Stmt::FnDef {
            name,
            template_params,
            params,
            return_type,
            body,
            is_abstract,
            is_static,
            is_class_method,
            decorators,
            ..
        } => ns(
            "StmtFnDef",
            vec![
                ("name", Value::str(name.as_str())),
                ("template_params", template_params_list(template_params)),
                ("params", params_list(params)),
                ("return_type", opt_str(return_type)),
                ("body", stmts_list(body)),
                ("is_abstract", Value::Bool(*is_abstract)),
                ("is_static", Value::Bool(*is_static)),
                ("is_class_method", Value::Bool(*is_class_method)),
                ("decorators", exprs_list(decorators)),
            ],
        ),
        Stmt::GenDef {
            name,
            template_params,
            params,
            yield_type,
            body,
            ..
        } => ns(
            "StmtGenDef",
            vec![
                ("name", Value::str(name.as_str())),
                ("template_params", template_params_list(template_params)),
                ("params", params_list(params)),
                ("yield_type", opt_str(yield_type)),
                ("body", stmts_list(body)),
            ],
        ),
        Stmt::ClassDef {
            name,
            template_params,
            bases,
            decorators,
            body,
        } => ns(
            "StmtClassDef",
            vec![
                ("name", Value::str(name.as_str())),
                ("template_params", template_params_list(template_params)),
                ("bases", str_list(bases)),
                ("decorators", exprs_list(decorators)),
                ("body", stmts_list(body)),
            ],
        ),
        Stmt::TraitDef {
            name,
            template_params,
            body,
        } => ns(
            "StmtTraitDef",
            vec![
                ("name", Value::str(name.as_str())),
                ("template_params", template_params_list(template_params)),
                ("body", stmts_list(body)),
            ],
        ),
        Stmt::ProtocolDef { name, body } => ns(
            "StmtProtocolDef",
            vec![
                ("name", Value::str(name.as_str())),
                ("body", stmts_list(body)),
            ],
        ),
        Stmt::Field {
            name,
            kind,
            type_ann,
            default,
            ..
        } => ns(
            "StmtField",
            vec![
                ("name", Value::str(name.as_str())),
                ("kind", field_kind_value(kind)),
                ("type_ann", Value::str(type_ann.as_str())),
                ("default", opt_expr(default.as_ref())),
            ],
        ),
        Stmt::NewTypeDef { name, original } => ns(
            "StmtNewTypeDef",
            vec![
                ("name", Value::str(name.as_str())),
                ("original", Value::str(original.as_str())),
            ],
        ),
        Stmt::EnumDef { name, variants } => {
            let variants_val = Value::List(Rc::new(RefCell::new(
                variants
                    .iter()
                    .map(|(vname, vval)| {
                        pair(
                            Value::str(vname.as_str()),
                            vval.as_ref().map(expr_to_value).unwrap_or(Value::None),
                        )
                    })
                    .collect(),
            )));
            ns(
                "StmtEnumDef",
                vec![
                    ("name", Value::str(name.as_str())),
                    ("variants", variants_val),
                ],
            )
        }
        Stmt::Try {
            body,
            handlers,
            finally_body,
        } => ns(
            "StmtTry",
            vec![
                ("body", stmts_list(body)),
                ("handlers", handlers_list(handlers)),
                ("finally_body", opt_stmts(finally_body.as_ref())),
            ],
        ),
        Stmt::Raise { exc, .. } => ns("StmtRaise", vec![("exc", opt_expr(exc.as_ref()))]),

        Stmt::Import {
            lang,
            module,
            alias,
            ..
        } => ns(
            "StmtImport",
            vec![
                ("lang", Value::str(lang.as_str())),
                ("module", str_list(module)),
                ("alias", opt_str(alias)),
            ],
        ),
        Stmt::FromImport {
            lang,
            module,
            names,
            ..
        } => {
            let names_val = Value::List(Rc::new(RefCell::new(
                names
                    .iter()
                    .map(|(nm, al)| pair(Value::str(nm.as_str()), opt_str(al)))
                    .collect(),
            )));
            ns(
                "StmtFromImport",
                vec![
                    ("lang", Value::str(lang.as_str())),
                    ("module", str_list(module)),
                    ("names", names_val),
                ],
            )
        }
        Stmt::AsyncAssign {
            target,
            return_type,
            stmts,
        } => ns(
            "StmtAsyncAssign",
            vec![
                ("target", Value::str(target.as_str())),
                ("return_type", opt_str(return_type)),
                ("stmts", stmts_list(stmts)),
            ],
        ),

        Stmt::BreakPoint { .. } => ns("StmtBreakPoint", vec![]),
        Stmt::DebugLet(name, expr) => ns(
            "StmtDebugLet",
            vec![
                ("name", Value::str(name.as_str())),
                ("expr", expr_to_value(expr)),
            ],
        ),

        Stmt::EventSubscribe { source, handler, is_once, is_async, .. } => ns(
            "StmtEventSubscribe",
            vec![
                ("source", expr_to_value(source)),
                ("handler", expr_to_value(handler)),
                ("is_once", Value::Bool(*is_once)),
                ("is_async", Value::Bool(*is_async)),
            ],
        ),
        Stmt::EventUnsubscribe { source, handler, .. } => ns(
            "StmtEventUnsubscribe",
            vec![
                ("source", expr_to_value(source)),
                ("handler", expr_to_value(handler)),
            ],
        ),
    }
}

// ---------------------------------------------------------------------------
// Expression conversion
// ---------------------------------------------------------------------------

fn expr_to_value(expr: &Expr) -> Value {
    match expr {
        Expr::Int(v) => ns("ExprInt", vec![("value", Value::Int(*v))]),
        Expr::Float(v) | Expr::ImaginaryLit(v) => {
            ns("ExprFloat", vec![("value", Value::Float(*v))])
        }
        // v は既に Rc<str>。clone は参照カウント加算のみで確保しない（#15）。
        Expr::Str(v) => ns("ExprStr", vec![("value", Value::Str(v.clone()))]),
        Expr::Bool(v) => ns("ExprBool", vec![("value", Value::Bool(*v))]),
        Expr::None => ns("ExprNone", vec![]),
        Expr::Undefined => ns("ExprUndefined", vec![]),
        // 解決状態（`res`）は AST 値としては見せない（parse_ar は実行時に新規パースするので常に未解決）。
        Expr::Ident { name, .. } => {
            ns("ExprIdent", vec![("name", Value::str(name.as_str()))])
        }

        Expr::List(elements) => ns("ExprList", vec![("elements", exprs_list(elements))]),
        Expr::Dict(pairs) => {
            let pairs_val = Value::List(Rc::new(RefCell::new(
                pairs
                    .iter()
                    .map(|(k, v)| pair(expr_to_value(k), expr_to_value(v)))
                    .collect(),
            )));
            ns("ExprDict", vec![("pairs", pairs_val)])
        }
        Expr::Tuple(elements) => ns("ExprTuple", vec![("elements", exprs_list(elements))]),
        Expr::Set(elements) => ns("ExprSet", vec![("elements", exprs_list(elements))]),

        Expr::BinOp { op, left, right, .. } => ns(
            "ExprBinOp",
            vec![
                ("op", binop_str(op)),
                ("left", expr_to_value(left)),
                ("right", expr_to_value(right)),
            ],
        ),
        Expr::UnaryOp { op, operand } => ns(
            "ExprUnaryOp",
            vec![
                ("op", unaryop_str(op)),
                ("operand", expr_to_value(operand)),
            ],
        ),
        Expr::Call { func, args, .. } => ns(
            "ExprCall",
            vec![
                ("func", expr_to_value(func)),
                ("args", call_args_list(args)),
            ],
        ),

        Expr::Attr { object, attr, .. } => ns(
            "ExprAttr",
            vec![
                ("object", expr_to_value(object)),
                ("attr", Value::str(attr.as_str())),
            ],
        ),
        Expr::TraitAccess {
            object, attr, ..
        } => ns(
            "ExprTraitAccess",
            vec![
                ("object", expr_to_value(object)),
                ("attr", Value::str(attr.as_str())),
            ],
        ),
        Expr::Subscript { object, index, .. } => ns(
            "ExprSubscript",
            vec![
                ("object", expr_to_value(object)),
                ("index", expr_to_value(index)),
            ],
        ),
        Expr::Slice { begin, end, step } => ns(
            "ExprSlice",
            vec![
                ("begin", begin.as_deref().map(expr_to_value).unwrap_or(Value::None)),
                ("end", end.as_deref().map(expr_to_value).unwrap_or(Value::None)),
                ("step", step.as_deref().map(expr_to_value).unwrap_or(Value::None)),
            ],
        ),
        Expr::TemplateInstantiate { base, type_args } => ns(
            "ExprTemplateInstantiate",
            vec![
                ("base", expr_to_value(base)),
                ("type_args", str_list(type_args)),
            ],
        ),
        Expr::IsType {
            expr,
            negated,
            type_name,
            ..
        } => ns(
            "ExprIsType",
            vec![
                ("expr", expr_to_value(expr)),
                ("negated", Value::Bool(*negated)),
                ("type_name", Value::str(type_name.as_str())),
            ],
        ),
        Expr::Cast { object, type_name, .. } => ns(
            "ExprCast",
            vec![
                ("object", expr_to_value(object)),
                ("type_name", Value::str(type_name.as_str())),
            ],
        ),

        Expr::Block { stmts, .. } => ns("ExprBlock", vec![("stmts", stmts_list(stmts))]),
        Expr::IfExpr {
            branches,
            else_body,
            ..
        } => ns(
            "ExprIfExpr",
            vec![
                ("branches", branches_list(branches)),
                ("else_body", opt_stmts(else_body.as_ref())),
            ],
        ),
        Expr::ForExpr {
            target,
            iter,
            body,
            ..
        } => ns(
            "ExprForExpr",
            vec![
                ("target", Value::str(target.as_str())),
                ("iter", expr_to_value(iter)),
                ("body", stmts_list(body)),
            ],
        ),
        Expr::WhileExpr { cond, body, .. } => ns(
            "ExprWhileExpr",
            vec![
                ("cond", expr_to_value(cond)),
                ("body", stmts_list(body)),
            ],
        ),
        Expr::MatchExpr { subject, arms, .. } => ns(
            "ExprMatchExpr",
            vec![
                ("subject", expr_to_value(subject)),
                ("arms", match_arms_list(arms)),
            ],
        ),

        Expr::DebugVar(name) => ns("ExprDebugVar", vec![("name", Value::str(name.as_str()))]),
        Expr::LocalVar(name) => ns("ExprLocalVar", vec![("name", Value::str(name.as_str()))]),
        Expr::MustBe { expr, guard_type, .. } => ns(
            "ExprMustBe",
            vec![
                ("expr", expr_to_value(expr)),
                ("guard_type", Value::str(guard_type.as_str())),
            ],
        ),
    }
}
