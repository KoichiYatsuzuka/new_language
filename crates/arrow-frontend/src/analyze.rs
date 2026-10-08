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
//! | `symbols`     | `parser::editor_index` の宣言表（型は型検査器の束縛の記録 `BindingRecord`） | Hover / Inlay / Go-to-def / Semantic tokens |
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

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::ast::{Expr, Param, Stmt};
use crate::lexer::editor_tokens::tokenize_with_spans;
use crate::parser::Parser;
use crate::token::Span;
use crate::type_check::TypeChecker;

/// import 先のモジュール（パースした AST）。鍵は `Parser` のキャッシュと同じ `(言語, 絶対パス)`。
type Modules = HashMap<(String, PathBuf), Vec<Stmt>>;

/// 前回までの解析で読み込んだ import 先のモジュール（editor_import_resolution_plan.md D-2）。
///
/// 打鍵ごとに解析し直すのは**編集中のドキュメントだけ**で、import 先は一度読んだら保持する（一般的な
/// コード解析器と同じ）。ホストが import 先になりうるファイルの変更を知らせたら（[`invalidate_modules`]）
/// すべて捨てる。
struct ModuleStore {
    /// エントリのディレクトリ（解析するドキュメントのディレクトリ）ごと。モジュールの名前はエントリの
    /// ディレクトリで決まる（`pkg.util`・`crate::module_path`）ので、別のディレクトリの解析と AST を共有しない。
    by_root: HashMap<PathBuf, Modules>,
    /// 保持しているモジュールの AST が使った node-id の最大値。次の解析はその次から振る
    /// （`Parser::reuse_modules`）。
    last_node_id: u32,
}

thread_local! {
    static MODULES: RefCell<ModuleStore> =
        RefCell::new(ModuleStore { by_root: HashMap::new(), last_node_id: 0 });
}

/// 保持している import 先のモジュールをすべて捨てる（ホストが import 先になりうるファイルの変更を
/// 知らせたとき・`wasm::ar_invalidate_modules`）。
pub fn invalidate_modules() {
    MODULES.with(|m| {
        let mut m = m.borrow_mut();
        m.by_root.clear();
        // ⚠ 0 に戻してよいのは、node-id を使っている AST が 1 つも残らないから。
        m.last_node_id = 0;
    });
}

/// 解析の前: `root` の解析で保持しているモジュールを `parser` に引き継ぐ。
fn reuse_modules(parser: &mut Parser, root: &PathBuf) {
    MODULES.with(|m| {
        let m = m.borrow();
        let modules = m.by_root.get(root).cloned().unwrap_or_default();
        parser.reuse_modules(modules, m.last_node_id);
    });
}

/// 解析の後: この解析で新しく読み込んだモジュールを保持する。
///
/// ⚠ 新しいものが無ければ node-id の最大値は**進めない**。進めると、打鍵ごとにドキュメントの node-id の
///   分だけ増え続ける（u32 を使い切る）。
fn keep_modules(parser: &mut Parser, root: PathBuf, node_counter: u32) {
    let modules = parser.take_module_cache();
    MODULES.with(|m| {
        let mut m = m.borrow_mut();
        let known = m.by_root.get(&root);
        let added = modules.keys().any(|k| !known.is_some_and(|known| known.contains_key(k)));
        if added {
            m.by_root.insert(root, modules);
            m.last_node_id = m.last_node_id.max(node_counter);
        }
    });
}

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

/// 読めなかった `import`（`ImportOrigin::unresolved`・`Parser::try_import`）があるか（D5・タスク 5-0）。
fn has_unloaded_import(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| {
        matches!(s, Stmt::Import { origin, .. } | Stmt::FromImport { origin, .. } if origin.unresolved)
    })
}

/// 展開の失敗の文面から、**このファイルの中で**展開を始めた位置（行・列）を拾う（D5・タスク 5-0）。
///
/// 展開器は失敗に「どのメタ関数をどこで展開していたか」を添える（`while expanding 'f' at <位置>`・
/// 外側から順）。最初の 1 行が、このファイルの文（配置呼び出し・装飾子）の位置。
/// 位置は `ファイル:行:列`（ファイル名つき）か `line 行, col 列` の形。拾えなければ `None`（ファイル全体）。
fn meta_error_pos(message: &str) -> Option<(usize, usize)> {
    let line = message.lines().find(|l| l.trim_start().starts_with("while expanding '"))?;
    let at = &line[line.rfind(" at ")? + 4..];
    let at = at.trim();
    if let Some(rest) = at.strip_prefix("line ") {
        let (l, c) = rest.split_once(", col ")?;
        return Some((l.trim().parse().ok()?, c.trim().parse().ok()?));
    }
    let (head, col) = at.rsplit_once(':')?;
    let (_, line_no) = head.rsplit_once(':')?;
    Some((line_no.trim().parse().ok()?, col.trim().parse().ok()?))
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
            Stmt::EnumDef { name, variants, src: _ } => {
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
            Stmt::If { branches, else_body, span: _ } => {
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

/// 解析結果を JSON 文字列で返す（ファイルに無いソース。import の探索の起点を持たない）。
///
/// `ok` が false のときは構文エラーで AST が得られなかったことを意味する。その場合
/// `symbols` などは空配列になるので、拡張側は**前回成功時の結果を保持**して使う
/// （入力途中は常に構文不正なので、そこで情報を全部消すと使い物にならない）。
pub fn analyze_json(source: &str, filename: &str) -> String {
    analyze_impl(source, filename, None)
}

/// ファイル `path` のソースとして解析する（`analyze_json` と同じ JSON）。
///
/// CLI（`arrow <path>`・`src/main.rs`）と同じく、`path` をファイル名に、その親を import の探索の起点
/// （`Parser::new` の `source_dir`）にする。⚠ wasm ではパスは `/D:/a/b.ar` の形
/// （`src/import_fs.rs` の「wasm 版のパスの形」）。
pub fn analyze_file_json(source: &str, path: &str) -> String {
    let source_dir = std::path::Path::new(path).parent().map(|p| p.to_path_buf());
    analyze_impl(source, path, source_dir)
}

fn analyze_impl(source: &str, filename: &str, source_dir: Option<std::path::PathBuf>) -> String {
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

    // import 先のモジュールは前回までの解析のものを使い、読めなかった import では止まらない
    // （`ModuleStore`・`Parser::reuse_modules`）。
    let root = source_dir.clone().unwrap_or_else(|| PathBuf::from("."));
    let mut parser = Parser::new(tokens, source_dir);
    reuse_modules(&mut parser, &root);
    let parsed = parser.parse_program();
    let last_node_id = parser.node_counter().get();
    keep_modules(&mut parser, root, last_node_id);
    let stmts = match parsed {
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

    // ⚠⚠ **メタ関数を展開してから型検査する**（設計書 D5・タスク 5-0）。CLI と同じ AST を検査しないと、
    //   展開で生えた宣言・装飾子が足したメンバー・置いたコードの誤りが**エディタにだけ見えず**、
    //   CLI と診断が食い違う（`compare_wasm_frontend` が守る不変条件）。
    //   メタ関数の構文を含まないプログラムは単相化だけ（展開の結果は同じで、展開用インタプリタを
    //   作る分だけ無駄）。単相化もしないと、具体化した本体の誤りがエディタにだけ出ない（2-8）。
    // ⚠ メタ関数が終わらない（`while True:`）と打つたびに解析が固まるので、展開中の VM は命令数に
    //   上限がある（`META_OPS_BUDGET`・CLI と同じ値）。
    // ⚠ 展開に失敗したら、CLI と同じく**型エラーは出さず** `MetaError` だけを出す（CLI は展開の失敗で
    //   止まり、型検査へ進まない。展開前の AST の型エラーは当てにならない）。ホバーなどの情報は
    //   展開前の AST（単相化だけしたもの）の型検査から取る。
    let counter = parser.node_counter();
    // ⚠ 読めなかった import（`ImportOrigin::unresolved`）があると、そのモジュールのメタ関数・`const` を
    //   使う展開は失敗する。CLI はその import で止まる（展開まで進まない）ので、展開の失敗は報告しない
    //   （import の誤りは出している）。
    let unloaded_import = has_unloaded_import(&stmts);
    let (stmts, meta_error) = if crate::meta_expand::mentions_meta(&stmts) {
        match crate::meta_expand::expand_program(
            stmts.clone(),
            std::rc::Rc::clone(&counter),
            parser.known_traits(),
        ) {
            Ok(out) => (out, None),
            Err(e) => (crate::meta_expand::monomorphize(stmts, counter).0, Some(e)),
        }
    } else {
        // ⚠ 単相化の失敗（具体化が終わらない・2-9）も CLI と同じく `MetaError`（3-3）。
        crate::meta_expand::monomorphize(stmts, counter)
    };
    let (errors, warnings, annotations, bindings) = TypeChecker::check_program_for_editor(&stmts);

    let mut diagnostics: Vec<Value> = Vec::with_capacity(errors.len() + warnings.len());
    // 読み込めなかった import（`EditorIndex::import_errors`）。CLI はここで構文解析を止める（`ParseError`）。
    for ((line, col), message) in &parser.editor_index().import_errors {
        let at = (*line != 0).then(|| Span { file: std::sync::Arc::from(filename), line: *line, col: *col });
        diagnostics.push(diag_json(&cols, at.as_ref(), SEVERITY_ERROR, strip_ansi(message), "ParseError"));
    }
    // ⚠ このドキュメントの位置の誤りだけを出す。import 先のモジュールの中の誤り（位置がそのファイル）を
    //   ここに出すと、このドキュメントの同じ行・列に波線が付く。
    let here = |span: Option<&Span>| span.is_none_or(|s| &*s.file == filename);
    if let Some(e) = &meta_error {
        if unloaded_import {
            // 報告しない（上の注意）。型エラーも出さない（展開前の AST の誤りは当てにならない）。
        } else {
        let message = strip_ansi(e);
        let at = meta_error_pos(&message).map(|(line, col)| Span {
            file: std::sync::Arc::from(filename),
            line,
            col,
        });
        diagnostics.push(diag_json(&cols, at.as_ref(), SEVERITY_ERROR, message, "MetaError"));
        }
    } else {
        for e in errors.iter().filter(|e| here(e.span.as_ref())) {
            diagnostics.push(diag_json(
                &cols,
                e.span.as_ref(),
                SEVERITY_ERROR,
                strip_ansi(&e.detail_str()),
                e.error_type_str(),
            ));
        }
        for w in warnings.iter().filter(|w| here(w.span.as_ref())) {
            diagnostics.push(diag_json(
                &cols,
                w.span.as_ref(),
                SEVERITY_WARNING,
                strip_ansi(&w.detail_str()),
                "TypeWarning",
            ));
        }
    }

    let index = parser.editor_index();
    let bound = bound_types(index, &bindings, filename);

    // ── 宣言表 ────────────────────────────────────────────────────────────
    let symbols: Vec<Value> = index
        .decls
        .iter()
        .enumerate()
        .filter(|(_, d)| d.pos.0 != 0)
        .map(|(i, d)| {
            // 型注釈が無い宣言の型。型検査器が**束縛した瞬間に記録した型**を使う（`bound_types`）。
            // 初期化式の型ではなく変数の型そのものなので、リテラル・`if` 式・`mustbe` などの
            // 初期化式の種類にも、初期化式の無い束縛（ループ変数・`except as`）にも左右されない。
            let inferred = bound.get(&i).filter(|t| t.as_str() != "unknown");
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

/// 宣言表の束縛（`Decl::binding`）に、型検査器が記録した束縛の型（`BindingRecord`）を割り当てる。
/// 戻り値は「宣言表の添字 → 型の表示」。
///
/// 両側とも鍵は「束縛を含む文の位置（`Stmt::position`）＋名前」で、同じ鍵の中は束縛した順に並ぶ
/// （パーサは `bind_anchors`、型検査器は `bind_pos` が同じ規則で決める）。ここは**突き合わせるだけ**で、
/// 型を推し量ることはしない。
///
/// ⚠ 数が合わない鍵は、型検査器の側が多いときに**全部同じ型**なら使い、それ以外は出さない。
///   多くなるのは、宣言表に載せていない束縛（内包表記の変数）が同じ文・同じ名前にあるとき。
///   誤った型を見せるより、出さないほうがよい。
/// ⚠ このファイルの位置を持たない記録（import 先の本体＝`<stub>` など）は除く。位置を持つ文の外の
///   束縛（`pos` が `None`）も、宣言表の側の位置が無い（`(0, 0)`）ので突き合わせない。
fn bound_types(
    index: &crate::parser::editor_index::EditorIndex,
    records: &[crate::type_check::BindingRecord],
    filename: &str,
) -> HashMap<usize, String> {
    type Key<'a> = ((usize, usize), &'a str);
    let mut checked: HashMap<Key, Vec<String>> = HashMap::new();
    for r in records {
        let Some(pos) = r.pos.as_ref().filter(|p| &*p.file == filename) else {
            continue;
        };
        checked
            .entry(((pos.line, pos.col), r.name.as_str()))
            .or_default()
            .push(r.ty.to_string());
    }
    let mut declared: HashMap<Key, Vec<(usize, usize)>> = HashMap::new();
    for (i, d) in index.decls.iter().enumerate() {
        let Some(seq) = d.binding else { continue };
        let Some(&anchor) = index.bind_anchors.get(seq) else { continue };
        if anchor.0 == 0 {
            continue;
        }
        declared.entry((anchor, d.name.as_str())).or_default().push((seq, i));
    }
    let mut out = HashMap::new();
    for (key, mut decls) in declared {
        let Some(types) = checked.get(&key) else { continue };
        decls.sort_unstable();
        if decls.len() == types.len() {
            for ((_, i), t) in decls.iter().zip(types) {
                out.insert(*i, t.clone());
            }
        } else if types.len() > decls.len() && types.iter().all(|t| *t == types[0]) {
            for (_, i) in &decls {
                out.insert(*i, types[0].clone());
            }
        }
    }
    out
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
            Stmt::Let(name, type_ann, _, _) | Stmt::Const(name, type_ann, _, _) => json!({
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
