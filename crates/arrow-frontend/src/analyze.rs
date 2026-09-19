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
//! | `typeRefs`    | `editor_index.type_refs`（`parse_type_expr` が控えた型位置） | Semantic tokens / Hover（型名を関数と誤認させない） |
//! | `tokens`      | `lexer::editor_tokens`（トークンの範囲＋コメント） | Semantic tokens / 語の特定 / 受け手判定 / 呼び出し文脈 |
//! | `members`     | AST のクラス/トレイト/列挙本体 ＋ **import した名前空間** | `.` 補完 |
//!
//! ⚠ `tokens` だけは **`ok: false` のときも現在のテキストのもの**を返す。
//!   `Lexer::tokenize()` は失敗しないので構文エラー中でも正しく、拡張が
//!   lastGood（古いテキスト）の位置を今のバッファに当てずに済む。
//!
//! ⚠ 位置はすべて **0 始まり・列は UTF-16 コードユニット**（VS Code の `Position` と同じ）。
//!   `Span` は 1 始まり・文字単位なので、変換は [`Utf16Cols`] が 1 箇所で行う。

use serde_json::{json, Map, Value};

use crate::ast::{Expr, Param, Stmt};
use crate::lexer::editor_tokens::tokenize_with_spans;
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

/// 行ごとの「文字インデックス → UTF-16 オフセット」変換表。
///
/// # なぜ要るのか
///
/// `Span.col` は**文字（Unicode スカラ値）単位**だが、VS Code の `Position.character` は
/// **UTF-16 コードユニット単位**。BMP 内の文字しかない行では一致するので今まで表面化して
/// いなかったが、絵文字などサロゲートペアが 1 つでも行にあると以降の列が 1 ずつずれる。
///
/// 拡張が自前の走査（TypeScript は最初から UTF-16 で動く）をやめて**この JSON の位置だけ**を
/// 信じるようになると、ここが唯一の真実源になる。だから変換をこの 1 箇所に置く。
struct Utf16Cols {
    /// `lines[line][char_idx]` = その行頭からの UTF-16 オフセット。
    /// 末尾に行全体の長さを 1 つ余分に持つので、終端位置の変換にも使える。
    lines: Vec<Vec<usize>>,
}

impl Utf16Cols {
    fn new(source: &str) -> Self {
        let mut lines = Vec::new();
        for line in source.split('\n') {
            let mut offsets = Vec::with_capacity(line.chars().count() + 1);
            let mut acc = 0usize;
            for ch in line.chars() {
                offsets.push(acc);
                acc += ch.len_utf16();
            }
            offsets.push(acc);
            lines.push(offsets);
        }
        Self { lines }
    }

    /// 1 始まりの (行, 文字列) を、0 始まりの (行, UTF-16 列) へ直す。
    fn to_vscode(&self, line: usize, col: usize) -> (usize, usize) {
        let l = line.saturating_sub(1);
        let c = col.saturating_sub(1);
        let utf16 = self
            .lines
            .get(l)
            .and_then(|offsets| offsets.get(c).copied())
            // 表の外（行末より後ろ）は行の全長に丸める。位置不明で落とすよりまし。
            .or_else(|| self.lines.get(l).and_then(|o| o.last().copied()))
            .unwrap_or(c);
        (l, utf16)
    }
}

/// 1 始まりの行・列を VS Code の 0 始まり・UTF-16 列へ直す。変換はここ 1 箇所だけで行う。
fn pos_json(cols: &Utf16Cols, line: usize, col: usize) -> Value {
    let (l, c) = cols.to_vscode(line, col);
    json!({ "line": l, "col": c })
}

/// `Span` を JSON に落とす。`line == 0` は「位置不明」なので `null`。
fn span_json(cols: &Utf16Cols, span: &Span) -> Value {
    if span.line == 0 {
        return Value::Null;
    }
    pos_json(cols, span.line, span.col)
}

fn diag_json(
    cols: &Utf16Cols,
    span: Option<&Span>,
    severity: u8,
    message: String,
    source: &str,
) -> Value {
    json!({
        "severity": severity,
        "message": message,
        "source": source,
        "at": span.map(|s| span_json(cols, s)).unwrap_or(Value::Null),
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
    let cols = Utf16Cols::new(source);

    // 字句解析はトークン列と**その範囲**の両方を返す。範囲は拡張の着色・語の特定・
    // 受け手判定に使う（そこから自前の正規表現走査を消すため）。
    let (tokens, token_spans) = tokenize_with_spans(source, filename);
    let tokens_json: Vec<Value> = token_spans
        .iter()
        .map(|t| {
            let (sl, sc) = cols.to_vscode(t.start.0, t.start.1);
            let (el, ec) = cols.to_vscode(t.end.0, t.end.1);
            json!({
                "kind": t.kind.as_str(),
                "line": sl, "col": sc,
                "endLine": el, "endCol": ec,
            })
        })
        .collect();

    let mut parser = Parser::new(tokens, None);
    let stmts = match parser.parse_program() {
        Ok(stmts) => stmts,
        Err(e) => {
            // ⚠ `tokens` だけは**現在のテキストのもの**を返す。`Lexer::tokenize()` は
            //    失敗しないので、構文エラー中でも必ず正しい。ここを空にすると拡張は
            //    lastGood（古いテキスト）の位置を今のバッファに当てることになり、
            //    打っている最中ずっと色がずれる。
            // 失敗位置は**パーサが止まったトークン**。以前は拡張がエラー文字列を
            // 正規表現で読み直して波線を置いていた（`parse_program` が位置を人間向けの
            // 文章に埋めて捨てるため）。索引から取れば推測が消える。
            let at = parser
                .editor_index()
                .parse_error_pos
                .map(|(l, c)| pos_json(&cols, l, c))
                .unwrap_or(Value::Null);
            return json!({
                "ok": false,
                "parseError": strip_ansi(&e),
                "parseErrorAt": at,
                "diagnostics": [],
                "symbols": [],
                "scopes": [],
                "exprTypes": [],
                "typeRefs": [],
                "tokens": tokens_json,
                "members": {},
            })
            .to_string();
        }
    };

    let (errors, warnings, annotations) = TypeChecker::check_program(&stmts);

    let mut diagnostics: Vec<Value> = Vec::with_capacity(errors.len() + warnings.len());
    for e in &errors {
        diagnostics.push(diag_json(
            &cols,
            e.span.as_ref(),
            SEVERITY_ERROR,
            strip_ansi(&e.detail_str()),
            e.error_type_str(),
        ));
    }
    for w in &warnings {
        diagnostics.push(diag_json(
            &cols,
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
                "at": pos_json(&cols, d.pos.0, d.pos.1),
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
                "at": pos_json(&cols, pos.0, pos.1),
                "type": rendered,
            }));
        }
    }

    // ── 型参照 ────────────────────────────────────────────────────────────
    // 「この識別子は型位置にある」というパーサだけが知る事実。拡張はこれを
    // 名前引きより**先に**見る。無いと `int` のような型名/組み込み関数の兼用名が
    // すべて関数として着色・hover される（`EditorIndex::type_refs` の doc 参照）。
    let type_refs: Vec<Value> = index
        .type_refs
        .iter()
        .map(|(pos, name)| {
            json!({
                "at": pos_json(&cols, pos.0, pos.1),
                "name": name,
            })
        })
        .collect();

    // ── メンバ表 ──────────────────────────────────────────────────────────
    let mut members = Map::new();
    collect_members(&stmts, &mut members);
    // ⚠ **順番が効く**。名前空間は「空いている鍵」にだけ入るので、型名を先に確定させる
    //   （`collect_module_members` の doc）。
    collect_module_members(&stmts, &mut members);

    json!({
        "ok": true,
        "parseError": Value::Null,
        "diagnostics": diagnostics,
        "symbols": symbols,
        "scopes": scopes,
        "exprTypes": expr_types,
        "typeRefs": type_refs,
        "tokens": tokens_json,
        "parseErrorAt": Value::Null,
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

/// **import した名前空間**のメンバ表を集める（`.` 補完用）。
///
/// # なぜ [`collect_members`] と別なのか
///
/// あちらの鍵は**型名**（クラス／トレイト／列挙）で、`.` の受け手が型のときに引く。
/// 名前空間の受け手は**束縛名**（`import[...] X as sakura` の `sakura`）で、
/// 種類が違う。`Stmt::Import` を `collect_members` の match に足すと、
/// 「型名の表」に別の意味の鍵が混ざる。
///
/// ⚠⚠ **衝突したら型が勝つ**。`class Foo` と `import ... as Foo` が同居しうるので、
/// ここは**空いている鍵にだけ**入れる（`collect_members` を先に走らせる）。
/// 型名のほうが強い主張で、`members` はもともとそのために作られた表だから。
///
/// ⚠ import 先の body に入れ子で定義された**型**も登録する。`wpf.HostWindow.` の
/// 受け手は `HostWindow`（import 先のクラス）なので、これが無いと 2 段目が引けない。
fn collect_module_members(stmts: &[Stmt], out: &mut Map<String, Value>) {
    for stmt in stmts {
        let (module, alias, body) = match stmt {
            Stmt::Import { module, alias, body, .. } => (module, alias.as_ref(), body),
            // `from X import a, b` は名前を直接束縛するので、名前空間の受け手にはならない。
            // ただし body に載っている型定義は `collect_members` で拾わせたい。
            Stmt::FromImport { body, .. } => {
                merge_vacant(body, out);
                continue;
            }
            _ => continue,
        };
        if body.is_empty() {
            continue;
        }
        // import 先で定義された型（`wpf.HostWindow` の `HostWindow` 等）を型名の表へ。
        merge_vacant(body, out);

        let bind = alias
            .cloned()
            .or_else(|| module.last().cloned())
            .unwrap_or_default();
        if bind.is_empty() || out.contains_key(&bind) {
            continue; // 型名が既に取っている鍵は奪わない
        }
        out.insert(bind, json!({ "members": namespace_members(body), "bases": [] }));
    }
}

/// モジュール本体の**最上位宣言**をメンバとして並べる。
///
/// ⚠ `members_of_body`（クラス本体用）と違い、`class` / `enum` / `trait` も並べる。
/// 名前空間のメンバにはそれらが含まれるため（`wpf.WpfApp` はクラス）。
/// ⚠ `Stmt::Let` も拾う。py スタブは `let name: function->T` の形で来るので
/// （`parser::py_stub_extract`）、落とすと py モジュールの補完が空になる。
fn namespace_members(body: &[Stmt]) -> Vec<Value> {
    let mut items = Vec::new();
    for s in body {
        let v = match s {
            Stmt::FnDef { name, params, return_type, body, .. } => json!({
                "name": name, "kind": "function", "type": return_type,
                "params": params_json(params), "access": "public", "doc": docstring(body),
            }),
            Stmt::GenDef { name, params, yield_type, body, .. } => json!({
                "name": name, "kind": "generator", "type": yield_type,
                "params": params_json(params), "access": "public", "doc": docstring(body),
            }),
            Stmt::ClassDef { name, body, .. } => json!({
                "name": name, "kind": "class", "type": name,
                "access": "public", "doc": docstring(body),
            }),
            Stmt::TraitDef { name, .. } => json!({
                "name": name, "kind": "trait", "type": name, "access": "public",
            }),
            Stmt::ProtocolDef { name, .. } => json!({
                "name": name, "kind": "protocol", "type": name, "access": "public",
            }),
            Stmt::EnumDef { name, .. } => json!({
                "name": name, "kind": "enum", "type": name, "access": "public",
            }),
            Stmt::NewTypeDef { name, original } => json!({
                "name": name, "kind": "new_type", "type": original, "access": "public",
            }),
            // py スタブ（`let loads: function->str`）と、モジュールの定数。
            Stmt::Let(name, type_ann, _) | Stmt::Const(name, type_ann, _) => json!({
                "name": name, "kind": "variable", "type": type_ann, "access": "public",
            }),
            _ => continue,
        };
        items.push(v);
    }
    items
}

/// import 先の型定義を、**空いている鍵にだけ**入れる。
///
/// ⚠⚠ `collect_members` は `insert` なので、そのまま import 先に対して呼ぶと
/// **同名の利用者のクラスを上書きする**（`class Config` を自分で書いていて、
/// import 先にも `Config` がある、はふつうに起こる）。上書きすると `.` 補完が
/// 別の型のメンバを出すので、静かに間違う。⇒ 自分のファイルの型が常に勝つ。
fn merge_vacant(body: &[Stmt], out: &mut Map<String, Value>) {
    let mut sub = Map::new();
    collect_members(body, &mut sub);
    for (k, v) in sub {
        out.entry(k).or_insert(v);
    }
}
