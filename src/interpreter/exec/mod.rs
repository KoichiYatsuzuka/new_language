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

        // ② どこへ降りるか＋入れ子スコープの束縛（**この walker 固有**・#84 で構造を 1 箇所へ）。
        // ⚠ **`_ => {}` を書かない** — `StmtPart` に種類が増えるとここが止まる。
        crate::stmt_walk::each_subpart(stmt, &mut |part| {
            use crate::stmt_walk::StmtPart as P;
            match part {
                // ⚠⚠ #84 で `Stmt::Match` のアーム本体へ降りるようになった（実バグの修正）。
                // 降りていなかったため、`match` アームの中で宣言した名前が「自前の名前」から
                // 漏れて**自由変数として捕捉**され、採番側（`collect_nested_decls` は降りる）が
                // 振った slot と衝突して `capture-slot-conflict` で `VmForceError` になっていた。
                P::Control(b) => collect_declared_names(b, out),
                // ⚠⚠ #84 で式の中のブロック式へも降りるようになった（同じ実バグの別経路）。
                // `let q = block ->T: let z = …` を持つ入れ子 `fn` が同じ衝突で落ちていた。
                P::Expr(e) => collect_declared_in_expr(e, out),
                // 入れ子スコープの束縛（#59 が意図的に `decl_names` へ載せなかったもの）。
                P::ForTarget(t) => {
                    out.insert(t.to_string());
                }
                P::ExceptAlias(a) => {
                    out.insert(a.to_string());
                }
                // ⚠ 入れ子定義の**本体には降りない**（別フレーム）。名前そのものは①が入れる。
                P::FnBody { .. } | P::GenBody { .. } | P::TypeBody(_) | P::ProtocolBody(_) => {}
                // 別モジュールの本体。呼び出し元のフレームの名前ではない。
                P::ModuleBody(_) => {}
                // async 本体は送出時にディープクローンされ別チャンクになる
                // （採番側の `collect_nested_decls` も降りない＝2 本の判断が揃っている）。
                P::AsyncBody(_) => {}
                // ⚠ パターンへは降りない（採番側と揃える）。
                P::MatchPattern(_) => {}
                // 既存の名前への代入は**自前の宣言ではない**（参照側が別に拾う）。
                P::TargetName(_) => {}
            }
        });
    }
}

/// 式の中の**ブロック式**（`block:`/if/while/for/match 式）が宣言する名前も集める（#84）。
///
/// ⚠ [`crate::interpreter::resolver`] の `collect_bound_in_expr` と**同じ形**だが、
/// 集めた名前の使い道が違う（あちらは「シャドウしうる名前」＝解決を諦める根拠、
/// こちらは「クロージャの自前の名前」＝捕捉しない根拠）。判断は各 walker が持つ。
fn collect_declared_in_expr(expr: &Expr, out: &mut HashSet<String>) {
    // 部分式の構造は 1 箇所（#81）。⚠ **`_ => {}` を書かない**。
    crate::expr_walk::each_subpart(expr, &mut |part| {
        use crate::expr_walk::SubPart as P;
        match part {
            P::Plain(x) | P::Control(x) => collect_declared_in_expr(x, out),
            P::Body(b) => collect_declared_names(b, out),
            // `for i in …` 式のループ変数はブロック内の束縛（`Stmt::For` と揃える）。
            P::ForTarget(t) => {
                out.insert(t.to_string());
            }
            // ⚠ パターンへは降りない（採番側 `collect_expr_decls` と揃える）。
            P::MatchPattern(_) => {}
        }
    });
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
    // 文の直下の構造は 1 箇所（#84）。⚠ **`_ => {}` を書かない** — `StmtPart` に
    // 種類が増えるとここが止まり、「この walker ではどう扱うか」を決めさせられる。
    //
    // ⚠⚠ #75 まではここが自前の `match stmt` で、`Expr` 30 variant のうち 8 個・
    // `Stmt` 40 variant のうち 6 個を `_ => {}` で黙って落としていた（**式形式の制御構文や
    // `match` 文の中だけで外側変数を参照するクロージャが `NameError`**）。#75 で exhaustive 化し、
    // #84 で構造そのものを [`crate::stmt_walk`] へ移した。
    crate::stmt_walk::each_subpart(stmt, &mut |part| {
        use crate::stmt_walk::StmtPart as P;
        match part {
            P::Expr(e) => collect_refs_expr(e, out),
            // `case <expr>` のパターンは**参照**（#75 で拾うようにした）。
            P::MatchPattern(e) => collect_refs_expr(e, out),
            P::Control(b) => collect_referenced_names(b, out),
            // 入れ子定義の本体も参照を持つ（その中の自由変数は外側から捕捉される）。
            P::FnBody { body, .. } | P::GenBody(body) | P::TypeBody(body) => {
                collect_referenced_names(body, out);
            }
            // `protocol` の本体はシグネチャ宣言だけだが、既定値の式がありうるので降りる
            // （#75 以前からの挙動）。
            P::ProtocolBody(body) => collect_referenced_names(body, out),
            // ⚠ `import` の `body` は**別モジュールの本体**。呼び出し元のフレームからは捕捉しない。
            P::ModuleBody(_) => {}
            // `mng <- async->T:` の本体（送出時にディープクローンされるが、参照は解決が要る）。
            P::AsyncBody(b) => collect_referenced_names(b, out),
            // 既存の名前を指す ＝ **参照**（`x = …` の左辺・`freeze x`・`mng <- async` の `mng`）。
            P::TargetName(n) => {
                out.insert(n.to_string());
            }
            // ⚠ `for` ターゲットと `except ... as` の別名は**入れ子スコープの束縛**なので
            // 参照として拾わない（拾うと自前の名前を自由変数と誤認する）。
            P::ForTarget(_) | P::ExceptAlias(_) => {}
        }
    });
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
