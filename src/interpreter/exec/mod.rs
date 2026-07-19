// exec/mod.rs — 文実行サブシステムのモジュール束ね。
//
// `Interpreter::exec` が文(`Stmt`)を再帰的にツリーウォークして `ExecResult` を返す。
// このファイルは共有の自由ヘルパー関数(js設定探索・型注釈解析・名前収集・ハッシュ)を保持し、
// 役割別サブモジュール(dispatch/vars/control_flow/definitions/exceptions_async/modules/blocks)を宣言する。

use std::collections::HashSet;
use std::path::PathBuf;
use crate::ast::{Expr, Stmt, TupleTarget};

/// `ar_config.json` の `javascript` セクションを読んで
/// `(node_exe, bridge_script, bridge_root)` を返す。
///
/// 検索順: `search_dirs` 内の各ディレクトリ → カレントディレクトリ。
fn find_js_config(search_dirs: &[PathBuf])
    -> Result<(PathBuf, PathBuf, PathBuf), String>
{
    let cwd = std::env::current_dir().ok();
    let extra: &[PathBuf] = cwd.as_slice();

    for dir in search_dirs.iter().chain(extra.iter()) {
        let cfg_path = dir.join("ar_config.json");
        if !cfg_path.exists() { continue; }
        let text = match std::fs::read_to_string(&cfg_path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let root: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let js = match root.get("javascript") {
            Some(v) => v,
            None => continue,
        };

        let node_exe = js.get("node_path")
            .and_then(|v| v.as_str())
            .unwrap_or("node");
        let node_exe = PathBuf::from(node_exe);

        // bridge_script: 絶対パスまたは ar_config.json からの相対パス
        let bridge_script = js.get("bridge_script")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "ar_config.json: javascript.bridge_script is missing".to_string())?;
        let bridge_script = {
            let p = PathBuf::from(bridge_script);
            if p.is_absolute() { p } else { dir.join(p) }
        };
        if !bridge_script.exists() {
            return Err(format!(
                "ar_config.json: bridge_script '{}' not found",
                bridge_script.display()
            ));
        }

        let bridge_root = js.get("bridge_root")
            .and_then(|v| v.as_str())
            .map(|s| {
                let p = PathBuf::from(s);
                if p.is_absolute() { p } else { dir.join(p) }
            })
            .unwrap_or_else(|| dir.clone());

        return Ok((node_exe, bridge_script, bridge_root));
    }

    Err("ar_config.json: javascript section not found in any search directory".to_string())
}

/// `"list[T]"` からアイテム型 `"T"` を取り出す。`"list"` や他の型は `None` を返す。
fn extract_list_elem_type(ann: &str) -> Option<&str> {
    let inner = ann.strip_prefix("list[")?.strip_suffix(']')?;
    Some(inner.trim())
}

/// `x.is_OK()` / `x.is_ERR()` の形式の式から `(変数名, is_ok_flag)` を抽出する。
/// Result ガード節の変数バインディングに使う。
fn extract_result_guard_call(cond: &Expr) -> Option<(String, bool)> {
    if let Expr::Call { func, args, .. } = cond {
        if !args.is_empty() {
            return None;
        }
        if let Expr::Attr { object, attr, .. } = func.as_ref() {
            if let Expr::Ident(var_name) = object.as_ref() {
                match attr.as_str() {
                    "is_OK" => return Some((var_name.clone(), true)),
                    "is_ERR" => return Some((var_name.clone(), false)),
                    _ => {}
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Misc free helpers
// ---------------------------------------------------------------------------

/// Simple non-cryptographic hash of a string — used to generate stable temp
/// file names for cpp bridge DLLs.
fn simple_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// ---------------------------------------------------------------------------
// フリー変数分析ヘルパー（モジュールプライベート）
// ---------------------------------------------------------------------------

fn collect_declared_names(stmts: &[Stmt], out: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::Let(name, _, _)
            | Stmt::Const(name, _, _)
            | Stmt::Mut(name, _, _)
            | Stmt::Static(name, _, _) => {
                out.insert(name.clone());
            }
            Stmt::LetTuple { targets, .. } => {
                for t in targets {
                    match t {
                        TupleTarget::Let(n) | TupleTarget::Mut(n) | TupleTarget::Bare(n) => {
                            out.insert(n.clone());
                        }
                        TupleTarget::Wildcard => {}
                    }
                }
            }
            Stmt::FnDef { name, .. }
            | Stmt::GenDef { name, .. }
            | Stmt::ClassDef { name, .. }
            | Stmt::TraitDef { name, .. }
            | Stmt::ProtocolDef { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::For { targets, body, .. } => {
                for t in targets {
                    out.insert(t.clone());
                }
                collect_declared_names(body, out);
            }
            Stmt::If {
                branches,
                else_body,
            } => {
                for (_, body) in branches {
                    collect_declared_names(body, out);
                }
                if let Some(body) = else_body {
                    collect_declared_names(body, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Block(body) => {
                collect_declared_names(body, out);
            }
            Stmt::Try {
                body,
                handlers,
                finally_body,
            } => {
                collect_declared_names(body, out);
                for h in handlers {
                    if let Some(alias) = &h.name {
                        out.insert(alias.clone());
                    }
                    collect_declared_names(&h.body, out);
                }
                if let Some(body) = finally_body {
                    collect_declared_names(body, out);
                }
            }
            _ => {}
        }
    }
}

fn collect_referenced_names(stmts: &[Stmt], out: &mut HashSet<String>) {
    for stmt in stmts {
        collect_referenced_names_stmt(stmt, out);
    }
}

fn collect_referenced_names_stmt(stmt: &Stmt, out: &mut HashSet<String>) {
    match stmt {
        Stmt::Expr(e) => collect_refs_expr(e, out),
        Stmt::Let(_, _, e) | Stmt::Const(_, _, e) | Stmt::Mut(_, _, e) | Stmt::Static(_, e, _) => {
            collect_refs_expr(e, out);
        }
        Stmt::LetTuple { value, .. } => {
            collect_refs_expr(value, out);
        }
        Stmt::Assign { name, value, .. } => {
            out.insert(name.clone());
            collect_refs_expr(value, out);
        }
        Stmt::CompoundAssign { name, value, .. } => {
            out.insert(name.clone());
            collect_refs_expr(value, out);
        }
        Stmt::AttrAssign { target, value } | Stmt::AttrCompoundAssign { target, value, .. } => {
            collect_refs_expr(target, out);
            collect_refs_expr(value, out);
        }
        Stmt::Return(Some(e)) | Stmt::BlockReturn(e, _) | Stmt::LoopYield(e) | Stmt::Yield(e) => {
            collect_refs_expr(e, out);
        }
        Stmt::Raise { exc: Some(e), .. } => collect_refs_expr(e, out),
        Stmt::If {
            branches,
            else_body,
        } => {
            for (cond, body) in branches {
                collect_refs_expr(cond, out);
                collect_referenced_names(body, out);
            }
            if let Some(body) = else_body {
                collect_referenced_names(body, out);
            }
        }
        Stmt::While { cond, body } => {
            collect_refs_expr(cond, out);
            collect_referenced_names(body, out);
        }
        Stmt::For { iter, body, .. } => {
            collect_refs_expr(iter, out);
            collect_referenced_names(body, out);
        }
        Stmt::Block(body) => collect_referenced_names(body, out),
        Stmt::FnDef { body, .. } | Stmt::GenDef { body, .. } => {
            collect_referenced_names(body, out);
        }
        Stmt::ClassDef { body, .. } | Stmt::TraitDef { body, .. } | Stmt::ProtocolDef { body, .. } => {
            collect_referenced_names(body, out);
        }
        Stmt::Try {
            body,
            handlers,
            finally_body,
        } => {
            collect_referenced_names(body, out);
            for h in handlers {
                collect_referenced_names(&h.body, out);
            }
            if let Some(body) = finally_body {
                collect_referenced_names(body, out);
            }
        }
        Stmt::Freeze(name, _) => {
            out.insert(name.clone());
        }
        _ => {}
    }
}

fn collect_refs_expr(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Ident(name) => {
            out.insert(name.clone());
        }
        Expr::BinOp { left, right, .. } => {
            collect_refs_expr(left, out);
            collect_refs_expr(right, out);
        }
        Expr::UnaryOp { operand, .. } => collect_refs_expr(operand, out),
        Expr::Call { func, args, .. } => {
            collect_refs_expr(func, out);
            for arg in args {
                collect_refs_expr(arg.expr(), out);
            }
        }
        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => {
            collect_refs_expr(object, out);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for item in items {
                collect_refs_expr(item, out);
            }
        }
        Expr::Dict(pairs) => {
            for (k, v) in pairs {
                collect_refs_expr(k, out);
                collect_refs_expr(v, out);
            }
        }
        Expr::Subscript { object, index } => {
            collect_refs_expr(object, out);
            collect_refs_expr(index, out);
        }
        Expr::Slice { begin, end, step } => {
            if let Some(e) = begin {
                collect_refs_expr(e, out);
            }
            if let Some(e) = end {
                collect_refs_expr(e, out);
            }
            if let Some(e) = step {
                collect_refs_expr(e, out);
            }
        }
        Expr::TemplateInstantiate { base, .. } => collect_refs_expr(base, out),
        Expr::IsType { expr, .. } => collect_refs_expr(expr, out),
        _ => {}
    }
}

mod dispatch;
mod vars;
mod control_flow;
mod definitions;
mod exceptions_async;
mod modules;
mod blocks;
