//! ソース 1 本を lex → parse → type_check して、エディタが必要とする情報を JSON で返す。
//!
//! ここは**判断をしない**層である。型が何か・その名前がどのスコープに属するかは、
//! すべて `parser` / `type_check` の答えをそのまま転記する。エディタ固有の推測を
//! ここに足すと、「拡張だけ解釈がずれる」という元の問題が再発する。
//!
//! # 出力に含まれるもの
//!
//! | キー | 供給元 | 拡張側の用途 |
//! |---|---|---|
//! | `diagnostics` | `TypeChecker` のエラー・警告 | Diagnostics |
//! | `symbols`     | `parser::editor_index` の宣言表 | Hover / Inlay / Go-to-def / Semantic tokens |
//! | `scopes`      | 同上のスコープ木 | Completion（可視名の絞り込み） |
//! | `exprTypes`   | `editor_index.node_spans` × `AstAnnotations` | Hover（式の推論型）/ Inlay |
//! | `members`     | AST のクラス/トレイト/列挙本体 | `.` 補完 |

use serde_json::{json, Map, Value};

use crate::ast::{Expr, Param, Stmt};
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::token::Span;
use crate::type_check::TypeChecker;

/// 診断の深刻度。VS Code の `DiagnosticSeverity` に対応する。
const SEVERITY_ERROR: u8 = 0;
const SEVERITY_WARNING: u8 = 1;

/// ANSI エスケープシーケンス（`ESC [ … m`）を取り除く。
///
/// `StaticTypeError::detail_str()` は端末表示用に色を埋め込んで返す。エディタでは
/// そのまま出すと制御文字が見えてしまうので、**出力側で落とす**。
/// 検査器の側を変えないのは、端末出力がゲート（`compare_outputs.ps1` 等）の比較対象
/// だからで、色を消すと既存の基準がすべてずれる。
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // `ESC [` に続く終端文字（`m` 等、0x40..=0x7E）までを読み捨てる。
        if chars.next() != Some('[') {
            continue;
        }
        for c in chars.by_ref() {
            if ('\u{40}'..='\u{7e}').contains(&c) {
                break;
            }
        }
    }
    out
}

/// 1 始まりの行・列を VS Code の 0 始まりへ直す。変換はここ 1 箇所だけで行う。
fn pos_json(line: usize, col: usize) -> Value {
    json!({ "line": line.saturating_sub(1), "col": col.saturating_sub(1) })
}

/// `Span` を JSON に落とす。`line == 0` は「位置不明」なので `null`。
fn span_json(span: &Span) -> Value {
    if span.line == 0 {
        return Value::Null;
    }
    pos_json(span.line, span.col)
}

fn diag_json(span: Option<&Span>, severity: u8, message: String, source: &str) -> Value {
    json!({
        "severity": severity,
        "message": message,
        "source": source,
        "at": span.map(span_json).unwrap_or(Value::Null),
    })
}

/// `fn f(a: int, b: str) -> bool` からパラメータ表示を作る（signature help 用）。
fn params_json(params: &[Param]) -> Value {
    Value::Array(
        params
            .iter()
            .map(|p| {
                let label = if p.name == "self" {
                    "self".to_string()
                } else {
                    let q = if p.mutable { "mut" } else { "let" };
                    let n = if p.variadic { "..." } else { p.name.as_str() };
                    match &p.type_ann {
                        Some(t) => format!("{q} {n}: {t}"),
                        None => format!("{q} {n}"),
                    }
                };
                json!({
                    "name": p.name,
                    "label": label,
                    "type": p.type_ann,
                    "optional": p.default.is_some(),
                    "variadic": p.variadic,
                })
            })
            .collect(),
    )
}

/// クラス・トレイト・プロトコル・列挙のメンバ表を AST から集める（`.` 補完用）。
///
/// 型検査器の registry ではなく AST から取るのは、registry が `pub` 面を持たないため。
/// ここで行うのは転記だけで、可視性やメンバ名の判断は AST がすでに持っている。
fn collect_members(stmts: &[Stmt], out: &mut Map<String, Value>) {
    for stmt in stmts {
        match stmt {
            Stmt::ClassDef { name, body, bases, .. } => {
                out.insert(name.clone(), members_of_body(body, bases));
                collect_members(body, out);
            }
            Stmt::TraitDef { name, body, .. } => {
                out.insert(name.clone(), members_of_body(body, &[]));
                collect_members(body, out);
            }
            Stmt::ProtocolDef { name, body, .. } => {
                out.insert(name.clone(), members_of_body(body, &[]));
            }
            Stmt::EnumDef { name, variants } => {
                let items: Vec<Value> = variants
                    .iter()
                    .map(|(v, _)| {
                        json!({ "name": v, "kind": "enum_member", "type": name, "access": "public" })
                    })
                    .collect();
                out.insert(name.clone(), json!({ "members": items, "bases": [] }));
            }
            // 入れ子の定義も拾う（関数の中でクラスを定義できる）。
            Stmt::FnDef { body, .. } | Stmt::GenDef { body, .. } | Stmt::Block(body) => {
                collect_members(body, out)
            }
            Stmt::If { branches, else_body } => {
                for (_, b) in branches {
                    collect_members(b, out);
                }
                if let Some(e) = else_body {
                    collect_members(e, out);
                }
            }
            Stmt::Try { body, finally_body, .. } => {
                collect_members(body, out);
                if let Some(f) = finally_body {
                    collect_members(f, out);
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => collect_members(body, out),
            _ => {}
        }
    }
}

fn members_of_body(body: &[Stmt], bases: &[String]) -> Value {
    let mut items: Vec<Value> = Vec::new();
    for s in body {
        match s {
            Stmt::Field { name, type_ann, kind, access, .. } => items.push(json!({
                "name": name,
                "kind": "field",
                "type": type_ann,
                "mutability": format!("{kind:?}").to_lowercase(),
                "access": format!("{access:?}").to_lowercase(),
            })),
            Stmt::FnDef { name, params, return_type, access, is_static, body, .. } => {
                items.push(json!({
                    "name": name,
                    "kind": if *is_static { "static_method" } else { "method" },
                    "type": return_type,
                    "params": params_json(params),
                    "access": format!("{access:?}").to_lowercase(),
                    "doc": docstring(body),
                }))
            }
            Stmt::GenDef { name, params, yield_type, access, body, .. } => {
                items.push(json!({
                    "name": name,
                    "kind": "generator",
                    "type": yield_type,
                    "params": params_json(params),
                    "access": format!("{access:?}").to_lowercase(),
                    "doc": docstring(body),
                }))
            }
            _ => {}
        }
    }
    json!({ "members": items, "bases": bases })
}

/// 解析結果を JSON 文字列で返す。
///
/// `ok` が false のときは構文エラーで AST が得られなかったことを意味する。その場合
/// `symbols` などは空配列になるので、拡張側は**前回成功時の結果を保持**して使う
/// （入力途中は常に構文不正なので、そこで情報を全部消すと使い物にならない）。
pub fn analyze_json(source: &str, filename: &str) -> String {
    let tokens = Lexer::new(source, filename).tokenize();

    let mut parser = Parser::new(tokens, None);
    let stmts = match parser.parse_program() {
        Ok(stmts) => stmts,
        Err(e) => {
            return json!({
                "ok": false,
                "parseError": strip_ansi(&e),
                "diagnostics": [],
                "symbols": [],
                "scopes": [],
                "exprTypes": [],
                "members": {},
            })
            .to_string();
        }
    };

    let (errors, warnings, annotations) = TypeChecker::check_program(&stmts);

    let mut diagnostics: Vec<Value> = Vec::with_capacity(errors.len() + warnings.len());
    for e in &errors {
        diagnostics.push(diag_json(
            e.span.as_ref(),
            SEVERITY_ERROR,
            strip_ansi(&e.detail_str()),
            e.error_type_str(),
        ));
    }
    for w in &warnings {
        diagnostics.push(diag_json(
            w.span.as_ref(),
            SEVERITY_WARNING,
            strip_ansi(&w.detail_str()),
            "TypeWarning",
        ));
    }

    let index = parser.editor_index();

    // ── 宣言表 ────────────────────────────────────────────────────────────
    let symbols: Vec<Value> = index
        .decls
        .iter()
        .filter(|d| d.pos.0 != 0)
        .map(|d| {
            // 型注釈が無い宣言の推論型。初期化式の node-id で型検査器の注釈表を引く。
            // 位置から探すのではなく id で引くので、`mut c = Circle(5.0)` のように
            // 右辺が名前から離れていても正しく取れる。
            let inferred = d
                .init_node
                .and_then(|id| annotations.resolved_type(id))
                .map(|t| t.to_string())
                .filter(|t| t != "unknown");
            json!({
                "name": d.name,
                "kind": d.kind.as_str(),
                "at": pos_json(d.pos.0, d.pos.1),
                "mutability": d.mutability,
                "typeAnn": d.type_ann,
                "inferred": inferred,
                "signature": d.signature,
                "doc": d.doc,
                "access": d.access,
                "container": d.container,
                "bases": d.bases,
                "scope": d.scope,
                "bodyScope": d.body_scope,
            })
        })
        .collect();

    // ── スコープ木 ────────────────────────────────────────────────────────
    let scopes: Vec<Value> = index
        .scopes
        .iter()
        .map(|s| {
            json!({
                "parent": s.parent.map(|p| p as i64).unwrap_or(-1),
                "startLine": (s.start_line as i64) - 1,
                // 開いたまま終わったスコープ（構文エラー時）はファイル末尾まで有効とみなす。
                "endLine": if s.end_line == usize::MAX { -1i64 } else { (s.end_line as i64) - 1 },
            })
        })
        .collect();

    // ── 式の推論型 ────────────────────────────────────────────────────────
    // `node_spans`（パーサが控えた node-id → 位置）と `AstAnnotations`（型検査器が
    // 焼いた node-id → 推論型）の突き合わせ。両者を繋ぐのがこの 1 箇所だけなので、
    // 「エディタが表示する型」と「型検査器が使う型」が構造的にずれない。
    let mut expr_types: Vec<Value> = Vec::new();
    for (node_id, pos) in &index.node_spans {
        if let Some(ty) = annotations.resolved_type(*node_id) {
            let rendered = ty.to_string();
            // `unknown`（`InferredType::Unresolved`）は出さない。出すと
            // 「型が付いていない」ことを「型が unknown である」と誤解させる。
            if rendered == "unknown" {
                continue;
            }
            expr_types.push(json!({
                "at": pos_json(pos.0, pos.1),
                "type": rendered,
            }));
        }
    }

    // ── メンバ表 ──────────────────────────────────────────────────────────
    let mut members = Map::new();
    collect_members(&stmts, &mut members);

    json!({
        "ok": true,
        "parseError": Value::Null,
        "diagnostics": diagnostics,
        "symbols": symbols,
        "scopes": scopes,
        "exprTypes": expr_types,
        "members": Value::Object(members),
        "stmtCount": stmts.len(),
    })
    .to_string()
}

/// docstring 取得のための薄いラッパ（`Expr::Str` 判定は 1 箇所に置く）。
pub(crate) fn docstring(body: &[Stmt]) -> Option<&str> {
    match body.first() {
        Some(Stmt::Expr(Expr::Str(s))) => Some(s),
        _ => None,
    }
}
