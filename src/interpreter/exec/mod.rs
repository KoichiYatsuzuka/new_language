// exec/mod.rs — 文実行サブシステムのモジュール束ね。
//
// `Interpreter::exec` が文(`Stmt`)を再帰的にツリーウォークして `ExecResult` を返す。
// このファイルは共有の自由ヘルパー関数(js設定探索・型注釈解析・名前収集・ハッシュ)を保持し、
// 役割別サブモジュール(dispatch/vars/control_flow/definitions/exceptions_async/modules/blocks)を宣言する。

use std::collections::HashSet;
use std::path::PathBuf;
// ⚠ `TupleTarget` はもう要らない（#59 でタプル分解の判断が `decl_names` へ移った）。
use crate::ast::{Expr, Stmt};

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

/// 本体が**自前で束縛する名前**を集める（クロージャの自由変数分析）。
///
/// 消費者は 3 つ（`exec::blocks::capture_env` / `vm::compiler::decls::nested_fn_free_names` /
/// `vm::compiler::calls`）で、いずれも「参照名 − 自前名 ＝ 捕捉すべき自由変数」を出すのに使う。
///
/// ⚠ **直接の束縛の判断は [`crate::decl_names`] に集約してある**（#59）。
/// ここが持つのは「**どこへ降りるか**」と「入れ子スコープの束縛（`for` ターゲット・
/// `except ... as` の別名）」だけ。#68 の実バグ（`enum` の抜け）は、この 2 つを
/// 分けずに 1 つの `match` に混ぜて `_ => {}` で落としていたのが原因だった。
pub(crate) fn collect_declared_names(stmts: &[Stmt], out: &mut HashSet<String>) {
    for stmt in stmts {
        // ① この 1 文が直接束縛する名前（判断は 1 箇所・#59）。
        crate::decl_names::each_declared_name(stmt, &mut |name, origin, _| {
            use crate::decl_names::DeclOrigin as D;
            match origin {
                D::Let
                | D::Mut
                | D::Static
                | D::TupleLet
                | D::TupleMut
                | D::Fn
                | D::Gen
                | D::Class
                | D::Trait
                | D::Protocol
                | D::Enum => {
                    out.insert(name.to_string());
                }
                // ⚠ **#59 時点で拾っていないもの**（挙動を変えないためそのまま保存した）。
                // `new_type` は関数本体に書くと**パースが通らない**ので現れない（#68 の調査）。
                D::NewType => {}
                // 関数本体の `import` は `compile_stmt` にアームが無く VmForceError になるので
                // VM 経路には現れない。⚠ **ツリーウォークの `capture_env` は到達しうる**ので
                // ここは既知の穴。拾うとクロージャの捕捉が変わるため #59 では触らない。
                D::Import | D::FromImport => {}
            }
        });

        // ② どこへ降りるか＋入れ子スコープの束縛（**この walker 固有**・#59 で統合しない差）。
        match stmt {
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
            // ⚠ 入れ子定義（`fn`/`class`…）の**本体には降りない**（別フレーム）。
            _ => {}
        }
    }
}

/// 本体が**参照する名前**を集める（クロージャの自由変数解決の入力・#75）。
///
/// 消費者は 3 系統ともクロージャの捕捉:
/// [`crate::interpreter::exec::blocks`] の `capture_env`（ツリーウォークの捕捉）、
/// `vm::compiler::calls`（VM の捕捉 slot）、`vm::compiler::decls::nested_fn_free_names`。
///
/// ⚠⚠ **`_ => {}` を足さないこと**（#75 の実バグ）。この 2 本の walker は #75 まで
/// `_ => {}` で終わっており、`Expr` 30 variant のうち 8 個・`Stmt` 40 variant のうち 6 個を
/// 黙って落としていた。結果、**式形式の制御構文（`if`/`match`/`block`/`for`/`while` 式）や
/// `match` 文・set リテラルの中だけで外側変数を参照するクロージャが `NameError`** になっていた
/// （正しいプログラムが 6 形で動かない。参照実装 impl_python との差分で確定）。
/// ⇒ **降りないバリアントも「降りない理由」を書いて列挙する**（#59 と同じ方針）。
pub(crate) fn collect_referenced_names(stmts: &[Stmt], out: &mut HashSet<String>) {
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
        // ⚠ #75 で追加。`match` **文**はここに無く、本体の参照が丸ごと落ちていた。
        Stmt::Match { subject, arms, .. } => {
            collect_refs_expr(subject, out);
            for arm in arms {
                if let crate::ast::MatchPattern::Case(e) = &arm.pattern {
                    collect_refs_expr(e, out);
                }
                collect_referenced_names(&arm.body, out);
            }
        }
        // `mng <- async->T:` の本体（⚠ `target` は束縛ではない・`decl_names` と同じ扱い）。
        Stmt::AsyncAssign { target, stmts, .. } => {
            out.insert(target.clone());
            collect_referenced_names(stmts, out);
        }
        Stmt::EventSubscribe {
            source, handler, ..
        }
        | Stmt::EventUnsubscribe {
            source, handler, ..
        } => {
            collect_refs_expr(source, out);
            collect_refs_expr(handler, out);
        }
        // クラス本体のフィールド宣言。既定値の式だけが参照になりうる。
        Stmt::Field { default, .. } => {
            if let Some(e) = default {
                collect_refs_expr(e, out);
            }
        }
        Stmt::EnumDef { variants, .. } => {
            for (_, e) in variants {
                if let Some(e) = e {
                    collect_refs_expr(e, out);
                }
            }
        }
        Stmt::DebugLet(_, e) => collect_refs_expr(e, out),
        // ── ここから下は「降りない」バリアント。⚠ 理由を消さずに残すこと（#59/#75）──
        // 参照する式を持たない文。
        Stmt::Break | Stmt::Continue | Stmt::Pass | Stmt::BreakPoint { .. } => {}
        Stmt::Return(None) | Stmt::Raise { exc: None, .. } => {}
        Stmt::NewTypeDef { .. } => {}
        // `import` の `body` は**別モジュールの本体**。呼び出し元のフレームからは捕捉しない。
        Stmt::Import { .. } | Stmt::FromImport { .. } => {}
    }
}

fn collect_refs_expr(expr: &Expr, out: &mut HashSet<String>) {
    // 名前そのものだけがこの walker 固有の判断。⚠ `dbg::name` / `local::name` は
    // **専用の名前空間**への参照であって外側フレームのローカルではないので拾わない
    // （捕捉対象にすると別物を掴む）。
    if let Expr::Ident { name, .. } = expr {
        out.insert(name.clone());
    }
    // 部分式の構造は 1 箇所（#81）。⚠ **`_ => {}` を書かない**。
    //
    // ⚠⚠ #75 の実バグ（`if`/`match`/`block`/`for`/`while` **式**と set リテラルの中だけで
    // 外側変数を参照するクロージャが `NameError`）は、ここが `_ => {}` で
    // `Expr` 30 variant のうち 8 個を落としていたのが原因。#81 で構造そのものを
    // [`crate::expr_walk`] へ移し、**variant を足すとコンパイルが止まる**ようにした。
    crate::expr_walk::each_subpart(expr, &mut |part| {
        use crate::expr_walk::SubPart as P;
        match part {
            P::Plain(x) | P::Control(x) => collect_refs_expr(x, out),
            P::Body(b) => collect_referenced_names(b, out),
            // ⚠ `for` ターゲットは**入れ子スコープの束縛**なので参照として拾わない
            //（`Stmt::For` の既存の扱いと揃える）。
            P::ForTarget(_) => {}
            // `case <expr>` のパターンは**参照**（#75 で拾うようにした）。
            P::MatchPattern(x) => collect_refs_expr(x, out),
        }
    });
}

mod dispatch;
mod vars;
mod control_flow;
mod definitions;
mod exceptions_async;
mod modules;
mod blocks;
