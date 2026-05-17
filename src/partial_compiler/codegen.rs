// codegen.rs — Rust source code generator for tl module functions
//
// Transpiles simple tl functions (int/bool parameters, arithmetic, control flow)
// to Rust source code for native compilation via `rustc --crate-type cdylib`.
//
// Eligible functions must:
//   - Have no template parameters
//   - Not be abstract
//   - Have all parameter types and return type be `int` or `bool`
//   - Contain only codegen-supported statements (if/while/return/assign/let/mut)
//
// Generated convention:
//   - Each fn `foo(a: int, b: int) -> int` gets a private `foo_impl(a: i64, b: i64) -> i64`
//   - Plus a #[no_mangle] extern "C" wrapper: `foo_tl(args: *const i64, n_args: i32) -> i64`

use std::collections::HashSet;

use crate::ast::{BinOp, CallArg, Expr, Param, Stmt, UnaryOp};

/// Metadata for one exported native function.
#[derive(Debug, Clone)]
pub struct FnExport {
    pub name: String,
    pub n_params: usize,
}

/// Generate a Rust source file for all codegen-eligible functions in `stmts`.
///
/// Returns `Some((rust_source, exports))` when at least one function is eligible,
/// `None` when nothing can be compiled natively.
pub fn generate_rust_module(stmts: &[Stmt]) -> Option<(String, Vec<FnExport>)> {
    // Set of all top-level fn names — used to recognise intra-module calls.
    let module_fns: HashSet<String> = stmts
        .iter()
        .filter_map(|s| {
            if let Stmt::FnDef { name, .. } = s {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect();

    // Collect eligible (fn_name, params, body) triples.
    struct EligibleFn<'a> {
        name: &'a str,
        params: &'a [Param],
        body: &'a [Stmt],
    }

    let eligible: Vec<EligibleFn> = stmts
        .iter()
        .filter_map(|s| {
            if let Stmt::FnDef {
                name,
                template_params,
                params,
                return_type,
                body,
                is_abstract,
                ..
            } = s
            {
                if !template_params.is_empty() || *is_abstract {
                    return None;
                }
                if !is_native_type(return_type.as_deref()) {
                    return None;
                }
                if params.iter().any(|p| !is_native_type(p.type_ann.as_deref())) {
                    return None;
                }
                if !body_eligible(body, &module_fns) {
                    return None;
                }
                Some(EligibleFn { name, params, body })
            } else {
                None
            }
        })
        .collect();

    if eligible.is_empty() {
        return None;
    }

    let mut out = String::new();

    out.push_str("#![allow(dead_code, unused_variables, unused_assignments)]\n");
    out.push_str("\n");

    // Forward declarations (not needed in Rust — just need ordering, but Rust resolves
    // within a crate so forward refs are fine).

    // Implementations
    for f in &eligible {
        let param_str: Vec<String> = f
            .params
            .iter()
            .map(|p| format!("{}: i64", p.name))
            .collect();
        out.push_str(&format!(
            "fn {}_impl({}) -> i64 {{\n",
            f.name,
            param_str.join(", ")
        ));

        let mut declared: HashSet<String> = f.params.iter().map(|p| p.name.clone()).collect();
        for stmt in f.body {
            gen_stmt(stmt, 1, &mut declared, &module_fns, &mut out);
        }
        // Implicit return 0 to satisfy the compiler if no explicit return at end
        out.push_str("    #[allow(unreachable_code)] 0i64\n");
        out.push_str("}\n\n");
    }

    // Extern "C" wrappers
    let exports: Vec<FnExport> = eligible
        .iter()
        .map(|f| FnExport {
            name: f.name.to_string(),
            n_params: f.params.len(),
        })
        .collect();

    for export in &exports {
        let name = &export.name;
        let arg_exprs: Vec<String> = (0..export.n_params)
            .map(|i| format!("unsafe {{ *args.add({i}) }}")
            )
            .collect();
        out.push_str(&format!(
            "#[no_mangle]\npub unsafe extern \"C\" fn {name}_tl(args: *const i64, _n_args: i32) -> i64 {{\n"
        ));
        out.push_str(&format!(
            "    {name}_impl({})\n",
            arg_exprs.join(", ")
        ));
        out.push_str("}\n\n");
    }

    Some((out, exports))
}

// ---------------------------------------------------------------------------
// Eligibility checks
// ---------------------------------------------------------------------------

fn is_native_type(t: Option<&str>) -> bool {
    matches!(t, Some("int") | Some("bool"))
}

fn body_eligible(stmts: &[Stmt], module_fns: &HashSet<String>) -> bool {
    stmts.iter().all(|s| stmt_eligible(s, module_fns))
}

fn stmt_eligible(stmt: &Stmt, module_fns: &HashSet<String>) -> bool {
    match stmt {
        Stmt::Let(_, e) | Stmt::Mut(_, e) | Stmt::Const(_, e) => expr_eligible(e, module_fns),
        Stmt::Assign { value, .. } => expr_eligible(value, module_fns),
        Stmt::CompoundAssign { value, .. } => expr_eligible(value, module_fns),
        Stmt::Return(Some(e)) => expr_eligible(e, module_fns),
        Stmt::Return(None) => true,
        Stmt::Pass => true,
        Stmt::If { branches, else_body } => {
            branches.iter().all(|(cond, body)| {
                expr_eligible(cond, module_fns) && body_eligible(body, module_fns)
            }) && else_body
                .as_ref()
                .map_or(true, |b| body_eligible(b, module_fns))
        }
        Stmt::While { cond, body } => {
            expr_eligible(cond, module_fns) && body_eligible(body, module_fns)
        }
        _ => false,
    }
}

fn expr_eligible(expr: &Expr, module_fns: &HashSet<String>) -> bool {
    match expr {
        Expr::Int(_) | Expr::Bool(_) | Expr::Ident(_) => true,
        Expr::BinOp { op, left, right, .. } => {
            !matches!(op, BinOp::Pow)
                && expr_eligible(left, module_fns)
                && expr_eligible(right, module_fns)
        }
        Expr::UnaryOp { operand, .. } => expr_eligible(operand, module_fns),
        Expr::Call { func, args } => {
            if let Expr::Ident(name) = func.as_ref() {
                if module_fns.contains(name) {
                    return args.iter().all(|a| {
                        if let CallArg::Positional(e) = a {
                            expr_eligible(e, module_fns)
                        } else {
                            false // keyword args not supported in C gen
                        }
                    });
                }
            }
            false
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Code generation
// ---------------------------------------------------------------------------

fn gen_stmt(
    stmt: &Stmt,
    indent: usize,
    declared: &mut HashSet<String>,
    module_fns: &HashSet<String>,
    out: &mut String,
) {
    let pad = "    ".repeat(indent);
    match stmt {
        Stmt::Let(name, e) | Stmt::Mut(name, e) | Stmt::Const(name, e) => {
            if declared.contains(name) {
                out.push_str(&format!(
                    "{pad}{name} = {};\n",
                    gen_expr_as_i64(e, module_fns)
                ));
            } else {
                declared.insert(name.clone());
                let kw = if matches!(stmt, Stmt::Mut(..)) { "mut " } else { "" };
                out.push_str(&format!(
                    "{pad}let {kw}{name}: i64 = {};\n",
                    gen_expr_as_i64(e, module_fns)
                ));
            }
        }
        Stmt::Assign { name, value, .. } => {
            out.push_str(&format!(
                "{pad}{name} = {};\n",
                gen_expr_as_i64(value, module_fns)
            ));
        }
        Stmt::CompoundAssign { name, op, value, .. } => {
            let op_str = compound_op_str(op);
            out.push_str(&format!(
                "{pad}{name} {op_str} {};\n",
                gen_expr_as_i64(value, module_fns)
            ));
        }
        Stmt::Return(Some(e)) => {
            out.push_str(&format!(
                "{pad}return {};\n",
                gen_expr_as_i64(e, module_fns)
            ));
        }
        Stmt::Return(None) => {
            out.push_str(&format!("{pad}return 0i64;\n"));
        }
        Stmt::Pass => {}
        Stmt::If { branches, else_body } => {
            for (i, (cond, body)) in branches.iter().enumerate() {
                if i == 0 {
                    out.push_str(&format!(
                        "{pad}if {} {{\n",
                        gen_expr_as_bool(cond, module_fns)
                    ));
                } else {
                    out.push_str(&format!(
                        "{pad}}} else if {} {{\n",
                        gen_expr_as_bool(cond, module_fns)
                    ));
                }
                for s in body {
                    gen_stmt(s, indent + 1, declared, module_fns, out);
                }
            }
            if let Some(else_stmts) = else_body {
                out.push_str(&format!("{pad}}} else {{\n"));
                for s in else_stmts {
                    gen_stmt(s, indent + 1, declared, module_fns, out);
                }
            }
            out.push_str(&format!("{pad}}}\n"));
        }
        Stmt::While { cond, body } => {
            out.push_str(&format!(
                "{pad}while {} {{\n",
                gen_expr_as_bool(cond, module_fns)
            ));
            for s in body {
                gen_stmt(s, indent + 1, declared, module_fns, out);
            }
            out.push_str(&format!("{pad}}}\n"));
        }
        _ => {} // not reachable if body_eligible check passed
    }
}

/// Generate a Rust expression that evaluates to `bool`.
/// Used for `if` and `while` conditions.
fn gen_expr_as_bool(expr: &Expr, module_fns: &HashSet<String>) -> String {
    match expr {
        Expr::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Expr::BinOp { op, left, right, .. } => {
            let l = gen_expr_as_i64(left, module_fns);
            let r = gen_expr_as_i64(right, module_fns);
            match op {
                BinOp::Eq    => format!("({l} == {r})"),
                BinOp::NotEq => format!("({l} != {r})"),
                BinOp::Lt    => format!("({l} < {r})"),
                BinOp::Gt    => format!("({l} > {r})"),
                BinOp::LtEq  => format!("({l} <= {r})"),
                BinOp::GtEq  => format!("({l} >= {r})"),
                BinOp::And   => format!("({} && {})", gen_expr_as_bool(left, module_fns), gen_expr_as_bool(right, module_fns)),
                BinOp::Or    => format!("({} || {})", gen_expr_as_bool(left, module_fns), gen_expr_as_bool(right, module_fns)),
                _ => format!("({} != 0i64)", gen_expr_as_i64(expr, module_fns)),
            }
        }
        Expr::UnaryOp { op: UnaryOp::Not, operand } => {
            format!("(!{})", gen_expr_as_bool(operand, module_fns))
        }
        _ => format!("({} != 0i64)", gen_expr_as_i64(expr, module_fns)),
    }
}

/// Generate a Rust expression that evaluates to `i64`.
fn gen_expr_as_i64(expr: &Expr, module_fns: &HashSet<String>) -> String {
    match expr {
        Expr::Int(n)   => format!("{n}i64"),
        Expr::Bool(b)  => if *b { "1i64" } else { "0i64" }.to_string(),
        Expr::Ident(n) => n.clone(),
        Expr::BinOp { op, left, right, .. } => {
            match op {
                // Comparison: returns bool in Rust, cast to i64
                BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => {
                    format!("({} as i64)", gen_expr_as_bool(expr, module_fns))
                }
                // Logical: returns bool in Rust, cast to i64
                BinOp::And | BinOp::Or => {
                    format!("({} as i64)", gen_expr_as_bool(expr, module_fns))
                }
                // Arithmetic
                BinOp::Add => {
                    format!("({} + {})", gen_expr_as_i64(left, module_fns), gen_expr_as_i64(right, module_fns))
                }
                BinOp::Sub => {
                    format!("({} - {})", gen_expr_as_i64(left, module_fns), gen_expr_as_i64(right, module_fns))
                }
                BinOp::Mul => {
                    format!("({} * {})", gen_expr_as_i64(left, module_fns), gen_expr_as_i64(right, module_fns))
                }
                BinOp::Div | BinOp::FloorDiv => {
                    format!("({} / {})", gen_expr_as_i64(left, module_fns), gen_expr_as_i64(right, module_fns))
                }
                BinOp::Mod => {
                    format!("({} % {})", gen_expr_as_i64(left, module_fns), gen_expr_as_i64(right, module_fns))
                }
                BinOp::BitAnd => {
                    format!("({} & {})", gen_expr_as_i64(left, module_fns), gen_expr_as_i64(right, module_fns))
                }
                BinOp::BitOr => {
                    format!("({} | {})", gen_expr_as_i64(left, module_fns), gen_expr_as_i64(right, module_fns))
                }
                BinOp::BitXor => {
                    format!("({} ^ {})", gen_expr_as_i64(left, module_fns), gen_expr_as_i64(right, module_fns))
                }
                BinOp::LShift => {
                    format!("({} << {})", gen_expr_as_i64(left, module_fns), gen_expr_as_i64(right, module_fns))
                }
                BinOp::RShift => {
                    format!("({} >> {})", gen_expr_as_i64(left, module_fns), gen_expr_as_i64(right, module_fns))
                }
                BinOp::Pow => "0i64".to_string(), // not supported — filtered out by eligibility check
            }
        }
        Expr::UnaryOp { op, operand } => match op {
            UnaryOp::Neg    => format!("(-{})", gen_expr_as_i64(operand, module_fns)),
            UnaryOp::Not    => format!("(!{} as i64)", gen_expr_as_bool(operand, module_fns)),
            UnaryOp::BitNot => format!("(!{})", gen_expr_as_i64(operand, module_fns)),
        },
        Expr::Call { func, args } => {
            if let Expr::Ident(name) = func.as_ref() {
                let arg_strs: Vec<String> = args
                    .iter()
                    .map(|a| gen_expr_as_i64(a.expr(), module_fns))
                    .collect();
                return format!("{name}_impl({})", arg_strs.join(", "));
            }
            "0i64".to_string()
        }
        _ => "0i64".to_string(),
    }
}

fn compound_op_str(op: &BinOp) -> &'static str {
    match op {
        BinOp::Add      => "+=",
        BinOp::Sub      => "-=",
        BinOp::Mul      => "*=",
        BinOp::Div | BinOp::FloorDiv => "/=",
        BinOp::Mod      => "%=",
        BinOp::BitAnd   => "&=",
        BinOp::BitOr    => "|=",
        BinOp::BitXor   => "^=",
        BinOp::LShift   => "<<=",
        BinOp::RShift   => ">>=",
        _               => "+=",
    }
}
