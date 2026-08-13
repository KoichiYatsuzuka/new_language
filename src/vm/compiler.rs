// vm/compiler.rs — 解決済み AST → Chunk のコンパイラ（Phase V, V-A）。
//
// 保守的コンパイル: 対応できない構文に出会ったら `None` を返し、呼び出し側は
// ツリーウォークにフォールバックする（デュアルモード, D2）。
//
// V-A の対応範囲（トップレベル関数のリーフ計算に限定）:
// - 文: `return` / `if` / `while` / 式文 / パラメータへの代入・複合代入。
// - 式: リテラル / `Resolution::Local`（パラメータ読み）/ 二項・単項演算 / 属性（フィールド）読み。
// - **非対応（=フォールバック）**: ローカル宣言（let/mut/const の freeze 意味論を避けるため）、
//   関数・メソッド呼び出し、クロージャ、for/match/block、例外、可変長引数、
//   グローバル/組み込み参照、添字、コレクションリテラル 等。

use std::collections::{HashMap, HashSet};

use crate::ast::{
    BinOp, CallArg, ExceptHandler, Expr, MatchArm, MatchPattern, Param, Stmt, TupleTarget, Resolution,
};
use crate::interpreter::Value;

use super::chunk::Chunk;
use super::op::{DeclKind, Op};

/// 診断フック（#10）: コンパイルを諦めた地点と構文種別を計上する。
/// `AR_TW_STATS=1` のときだけ働く（既定は enabled() の分岐 1 つで終わる）。
fn bail(site: &'static str, stmt: Option<&Stmt>) {
    if !crate::interpreter::tw_stats::enabled() {
        return;
    }
    let detail = stmt
        .map(crate::interpreter::tw_stats::stmt_kind_of)
        .unwrap_or("-");
    crate::interpreter::tw_stats::record_bail(site, detail);
}

/// `bail` の式版（`Expr` のバリアント名を採る）。
fn bail_expr(site: &'static str, expr: &Expr) {
    if !crate::interpreter::tw_stats::enabled() {
        return;
    }
    crate::interpreter::tw_stats::record_bail(site, expr_kind(expr));
}

/// 注釈テーブルを引くための node-id（#26）。持たないバリアントは `None`。
/// 0 は「未採番」（合成 AST・テンプレート本体など）なので同じく `None` にする。
fn expr_node_id(e: &Expr) -> Option<u32> {
    let id = match e {
        Expr::Ident { node_id, .. }
        | Expr::Attr { node_id, .. }
        | Expr::Call { node_id, .. }
        | Expr::Subscript { node_id, .. } => *node_id,
        _ => return None,
    };
    (id != 0).then_some(id)
}

/// `Expr` バリアント名（診断フック用）。
fn expr_kind(expr: &Expr) -> &'static str {
    match expr {
        Expr::Int(..) => "Int",
        Expr::Float(..) => "Float",
        Expr::ImaginaryLit(..) => "ImaginaryLit",
        Expr::Str(..) => "Str",
        Expr::Bool(..) => "Bool",
        Expr::None => "None",
        Expr::Undefined => "Undefined",
        Expr::Ident { .. } => "Ident",
        Expr::LocalVar { .. } => "LocalVar",
        Expr::DebugVar { .. } => "DebugVar",
        Expr::BinOp { .. } => "BinOp",
        Expr::UnaryOp { .. } => "UnaryOp",
        Expr::Call { .. } => "Call",
        Expr::Attr { .. } => "Attr",
        Expr::TraitAccess { .. } => "TraitAccess",
        Expr::Subscript { .. } => "Subscript",
        Expr::Slice { .. } => "Slice",
        Expr::List(..) => "List",
        Expr::Tuple(..) => "Tuple",
        Expr::Dict(..) => "Dict",
        Expr::Set(..) => "Set",
        Expr::IfExpr { .. } => "IfExpr",
        Expr::MatchExpr { .. } => "MatchExpr",
        Expr::ForExpr { .. } => "ForExpr",
        Expr::WhileExpr { .. } => "WhileExpr",
        Expr::Block { .. } => "Block",
        Expr::Cast { .. } => "Cast",
        Expr::IsType { .. } => "IsType",
        Expr::MustBe { .. } => "MustBe",
        Expr::TemplateInstantiate { .. } => "TemplateInstantiate",
    }
}

/// VM の `Call` op で解決できない呼び先名（純粋 builtin・型コンストラクタ）。
/// これらは `eval_builtin_ident_call` で特別扱いされるか、グローバル `Value::Type` として
/// 別セマンティクスで呼ばれるため、コンパイル時に弾いてツリーウォークへフォールバックする。
/// VM 内で評価済み引数から直接呼べる純粋組み込み（`eval_builtin_evaled` が扱う集合）。
/// `for x in range(n)` や `print(...)` を含む関数を VM に載せられるようにする。
/// キーワード/可変長引数を伴う呼び出しは `compile_call_args` が bail するので、ここに
/// 挙げた名前でも純粋な位置引数の呼び出しだけが `CallBuiltin` になる（＝評価済み引数で
/// 意味論が一致する形のみ）。
///
/// 型コンストラクタ（int/str/… は `Value::Type` グローバル）は**ここに含めない**。
/// 通常のグローバル呼び出し（`LoadGlobal`+`Call`）に流し、`call_value_evaled` の
/// `Value::Type` アーム＝`call_type_by_name_evaled` へ委譲する（ツリーウォークと同一経路・
/// ユーザーが同名をグローバル shadow しても `LoadGlobal` が拾うので健全）。
/// VM が `CallBuiltin` を発行する組み込み名。
///
/// ⚠ **この集合は `Interpreter::eval_builtin_evaled` が扱う名前の部分集合でなければならない**
/// （`run.rs` の `CallBuiltin` は `eval_builtin_evaled` が `None` を返すと `NameError` になる）。
/// 2 ファイルに跨る不変条件なので、`vm_builtin_names_are_all_handled` テストで固定してある（#22-d）。
pub(crate) const VM_BUILTIN_NAMES: &[&str] = &[
    "print", "range", "len", "next", "repr", "id", "enumerate", "zip", "getenv",
];

fn is_vm_builtin(name: &str) -> bool {
    VM_BUILTIN_NAMES.contains(&name)
}

/// VM が呼び先として扱えず、ツリーウォークへ bail すべき組み込み名。
/// - `eval_builtin_ident_call` 専用の builtin のうち **`eval_builtin_evaled` が扱わない**もの
///   （IO・flat リスト・parse_ar 等）。`is_vm_builtin` の集合はここより先に判定されるので重複不要。
/// - `Value::Type` グローバルとして**登録されていない**型名（`tuple`/`list`/`type`/`byte`）。
///   これらはツリーウォークでも `NameError`（呼び出し不可）なので、bail して同じ挙動にする。
///   登録済みの型コンストラクタ（int/str/… は `LoadGlobal`+`Call` で解決）はここに含めない。
fn is_builtin_callee(name: &str) -> bool {
    matches!(
        name,
        "create_flat_int_list" | "flat_get_int" | "flat_set_int" | "open" | "close" | "parse_ar"
            | "tuple" | "list" | "type" | "byte"
    )
}

struct Compiler {
    code: Vec<Op>,
    consts: Vec<Value>,
    names: Vec<String>,
    attr_caches: Vec<crate::ast::AttrCache>,
    spans: Vec<crate::token::Span>,
    /// 文境界の行テーブル（#1）。`code` と 1:1。詳細は [`Chunk::stmt_spans`](super::chunk::Chunk).
    stmt_spans: Vec<u32>,
    /// 「次に emit する op が文の先頭」を表す予約（#1）。`compile_stmt` が設定し `emit` が消費する。
    /// 文の先頭 op は `compile_stmt` の中で任意の深さから emit されるので、位置ではなく予約で持つ。
    pending_stmt: Option<u32>,
    /// AST 型解決層の注釈（#16 段階(b)/plan A）。node-id で型特化 op の判断に使う。空なら特化しない。
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    /// 名前 → slot（base スコープ: パラメータ + トップレベル let/mut/const、宣言順）。
    /// リゾルバの base slot 採番と同順（パラメータ→宣言）なので `Resolution::Local` と一致する。
    slots: HashMap<String, u16>,
    /// slot → 可変フラグ（`let x = <mut ソース>` の freeze 判定に使う）。
    slot_mut: Vec<bool>,
    /// slot → 型注釈（メソッド呼び出しの「obj は Instance」判定に使う）。
    slot_type: Vec<Option<String>>,
    /// `self` パラメータの slot（メソッド本体をコンパイルするとき Some）。
    /// `self` は型注釈を持たないが常に Instance なので、レシーバ判定で特別扱いする。
    self_slot: Option<u16>,
    /// ネストしたループのコンテキストスタック（break/continue のジャンプ先解決用）。
    loops: Vec<LoopCtx>,
    /// ネストしたブロック式のコンテキストスタック（block_return/loop_yield 用）。
    block_ctxs: Vec<BlockCtx>,
    /// デバッガ REPL モード: 変数参照は slot ではなく名前引き（`LoadName`）へ落とす。
    /// 停止スコープの生変数へアクセスし、`let dbg::x` を宣言できるようにする（V-E）。
    debug_mode: bool,
    /// 名前付き slot 数（パラメータ + 全ローカル宣言）。temp slot はこの上に積む。
    named_locals: u16,
    /// 現在使用中の temp slot 数（match サブジェクト等のスタック規律の一時領域）。
    temps_in_use: u16,
    /// フレームに必要な総 slot 数（名前付き + temp の最大同時数）。
    n_locals: usize,
    /// 非同期タスクブロック（`AsyncSubmit(idx)` が index で参照, タスク #9）。
    async_blocks: Vec<crate::vm::chunk::AsyncBlock>,
    /// `LoadGlobal` のグローバル索引キャッシュ（#11）。emit ごとに1本割り当てる。
    global_caches: Vec<crate::ast::SlotCache>,
    /// 最上位モード（#10-b）で「`scopes[0]` の同名を確実に指す」と言える名前の集合。
    /// 空 = 関数本体のコンパイル（従来どおり `slots` に無い書き込み先は bail）。
    ///
    /// ⚠ **書き込み専用の判定材料**。読み取りは AST の `Resolution::Global` に従う（そちらが正）。
    /// リゾルバの `toplevel_visible_globals` がそのまま入るので、判定は 1 実装で共有される。
    toplevel_globals: HashSet<String>,
}

/// 変数への書き込み先（#10-b）。`store_target` が決める。
enum StoreTarget {
    /// VM フレームの slot（`StoreLocal`）。
    Local(u16),
    /// `scopes[0]` のグローバル（`StoreGlobal`）。値は (name プール index, キャッシュ枠 index)。
    Global(u32, u32),
}

/// ループ1つ分の break/continue ジャンプ先。`continue` は `continue_target` へ、
/// `break` はループ末尾（コンパイル完了時にバックパッチ）へジャンプする。
/// Arrow の「break/continue が入れ子の if/match/block を貫通して外側ループへ届く」規則は、
/// これらが単なる絶対ジャンプなので自然に成立する（スタックは文境界で平衡）。
struct LoopCtx {
    /// `continue` のジャンプ先（while の条件先頭）。
    continue_target: u32,
    /// `break` 命令の位置（ループ末尾へバックパッチする）。
    break_jumps: Vec<usize>,
}

/// ブロック式1つ分のコンテキスト（block:/if/while/for/match 式）。
/// `block_return` は `result_slot` に値を格納して `end_jumps`（block_return 出口へバックパッチ）へ跳ぶ。
/// `loop_yield` は `yield_slot`（Some のブロック式＝block:/for/while）のリストへ追加する。
/// if/match 式は yield に対して透過（`yield_slot=None`）＝外側の for/while/block へ届く。
struct BlockCtx {
    /// `block_return` の値の格納先 slot。
    result_slot: u16,
    /// `block_return` の `Jump` 命令位置（block_return 出口へバックパッチ）。
    end_jumps: Vec<usize>,
    /// `loop_yield` の蓄積リスト slot（block:/for/while 式は Some、if/match 式は None＝透過）。
    yield_slot: Option<u16>,
}

/// 型注釈がユーザークラス/trait/protocol（＝実行時 Instance）であることを保守的に判定する。
/// 組み込み型・ジェネリック・Optional/union は false（フォールバック）。健全性優先で、
/// 少しでも Instance でない可能性があれば false を返す（型検査が Instance を保証する範囲のみ true）。
fn is_user_instance_type(ann: &str) -> bool {
    let t = ann.trim();
    // ジェネリック・union・optional・nullable は非対応。
    if t.is_empty()
        || t.contains('[')
        || t.contains('|')
        || t.contains('?')
        || t.contains(' ')
        || t.starts_with("Optional")
    {
        return false;
    }
    // 識別子として妥当か（英数字と `_` のみ）。
    if !t.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return false;
    }
    // 組み込み型（メソッドを別経路で持つ／プリミティブ）は除外。
    !matches!(
        t,
        "int" | "uint" | "str" | "float" | "bool" | "complex" | "list" | "dict" | "set"
            | "tuple" | "byte" | "bytes" | "char" | "Any" | "None" | "void" | "object"
            | "function" | "type" | "slice" | "range" | "Self"
    )
}

/// トップレベル関数本体を Chunk へコンパイルする。非対応構文があれば `None`。
///
/// - `params`: 仮引数（可変長があれば非対応）。
/// - `body`: 解決済み関数本体（リゾルバが `res` を付与済み）。
pub fn compile_fn(
    params: &[Param],
    body: &[Stmt],
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
) -> Option<Chunk> {
    // 診断フック（#10）: 失敗したのに bail 地点が 1 件も記録されなかったら「未帰属」として計上する。
    // 個々の `?` を全て計装する代わりに、取りこぼしを総量で可視化する。
    if !crate::interpreter::tw_stats::enabled() {
        return compile_fn_inner(params, body, annotations);
    }
    let before = crate::interpreter::tw_stats::bail_count();
    let out = compile_fn_inner(params, body, annotations);
    if out.is_none() && crate::interpreter::tw_stats::bail_count() == before {
        bail("unattributed", None);
    }
    out
}

/// モジュール最上位の**単一文**を Chunk へコンパイルする（#10-b）。
///
/// `compile_fn` との違いは 2 点だけ:
/// - **パラメータも base ローカルも無い**。slot は文の内側（ループ本体・for ターゲット等）の
///   宣言にだけ割り当てる（`collect_nested_decls`）。最上位の宣言は slot ではなくグローバル。
/// - **書き込み先にグローバルを許す**（`toplevel_globals`）。読み取りは従来どおり AST の
///   `Resolution::Global`（#21-b）に従うので、ここで渡すのは書き込み判定のためだけ。
///
/// 対象は**ループ文（`while`/`for`）と宣言文（`let`/`mut`/`const`）**（#10-b/#10-c）。
///
/// 宣言文を入れる理由は「宣言が多いから」ではない。**初期化子がループ式**のとき
/// （`mut xs = for i in range(N) -> list[T]: ... loop_yield ...`）本体が N 回まわるからで、
/// 実測では最上位に残っていたツリーウォークの **93% がこの形**だった。
/// 素朴に「1 回しか実行されない文はコンパイル損」と考えると取り逃す。
///
/// 残り（式文・`if`/`try`・定義文・`import`）は未対応。#3 にはそれらも要る。
pub fn compile_toplevel_stmt(
    stmt: &Stmt,
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    toplevel_globals: &HashSet<String>,
) -> Option<Chunk> {
    if !matches!(
        stmt,
        Stmt::While { .. }
            | Stmt::For { .. }
            | Stmt::Let(..)
            | Stmt::Mut(..)
            | Stmt::Const(..)
    ) {
        return None;
    }
    // 診断フック（#10）: `compile_fn` と同じく取りこぼしを「未帰属」として可視化する。
    if !crate::interpreter::tw_stats::enabled() {
        return compile_toplevel_stmt_inner(stmt, annotations, toplevel_globals);
    }
    // bail の分類先を「最上位」へ切り替える（#27。関数側と残タスクが別物なので混ぜない）。
    let _g = crate::interpreter::tw_stats::ToplevelCompileGuard::new();
    let before = crate::interpreter::tw_stats::bail_count();
    let out = compile_toplevel_stmt_inner(stmt, annotations, toplevel_globals);
    if out.is_none() && crate::interpreter::tw_stats::bail_count() == before {
        bail("unattributed", None);
    }
    out
}

fn compile_toplevel_stmt_inner(
    stmt: &Stmt,
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    toplevel_globals: &HashSet<String>,
) -> Option<Chunk> {
    let body = std::slice::from_ref(stmt);
    // for ターゲットが同名の宣言を覆う形は flat-slot で表現できない（関数側と同じ制約）。
    if has_for_target_shadow(&[], body) {
        bail("for-target-shadow", None);
        return None;
    }

    let mut slots: HashMap<String, u16> = HashMap::new();
    let mut slot_mut: Vec<bool> = Vec::new();
    let mut slot_type: Vec<Option<String>> = Vec::new();
    let mut n: u16 = 0;
    // ⚠ 宣言文は **`collect_nested_decls` に渡してはいけない**（#10-c）。
    // あれは「その文が宣言する名前」に slot を割り当てるが、最上位の宣言はグローバルであって
    // slot ではない。slot を振ると `store_target` が slot 側を優先してしまい、
    // `DeclareGlobal` が出ずに値がフレームへ消える。初期化子の**内側**の宣言だけを採番する。
    let collected = match stmt {
        Stmt::Let(_, _, e) | Stmt::Mut(_, _, e) | Stmt::Const(_, _, e) => {
            collect_expr_decls(e, &mut slots, &mut slot_mut, &mut slot_type, &mut n)
        }
        _ => collect_nested_decls(body, &mut slots, &mut slot_mut, &mut slot_type, &mut n),
    };
    if collected.is_none() {
        bail("nested-decls", None);
        return None;
    }

    let mut local_names = vec![String::new(); n as usize];
    for (name, &slot) in &slots {
        if let Some(entry) = local_names.get_mut(slot as usize) {
            *entry = name.clone();
        }
    }

    let mut c = Compiler {
        code: Vec::new(),
        consts: Vec::new(),
        names: Vec::new(),
        attr_caches: Vec::new(),
        spans: Vec::new(),
        stmt_spans: Vec::new(),
        pending_stmt: None,
        annotations,
        slots,
        slot_mut,
        slot_type,
        self_slot: None,
        loops: Vec::new(),
        block_ctxs: Vec::new(),
        debug_mode: false,
        named_locals: n,
        temps_in_use: 0,
        n_locals: n as usize,
        async_blocks: Vec::new(),
        global_caches: Vec::new(),
        toplevel_globals: toplevel_globals.clone(),
    };

    c.compile_stmt(stmt)?;
    // 最上位文は値を返さない（`Return` は型検査が最上位で禁じる）。
    c.emit(Op::ReturnNil);
    super::peephole::optimize(&mut c.code, &mut c.stmt_spans);

    let chunk = Chunk {
        code: c.code,
        consts: c.consts,
        names: c.names,
        attr_caches: c.attr_caches,
        spans: c.spans,
        stmt_spans: c.stmt_spans,
        local_names,
        n_locals: c.n_locals,
        n_params: 0,
        async_blocks: c.async_blocks,
        global_caches: c.global_caches,
    };

    if std::env::var("AR_VM_DUMP").is_ok_and(|v| !v.is_empty()) {
        eprintln!("{}", super::disasm::disassemble(&chunk, "<toplevel>"));
    }

    Some(chunk)
}

fn compile_fn_inner(
    params: &[Param],
    body: &[Stmt],
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
) -> Option<Chunk> {
    // `for` ループ変数が外側変数をシャドウする関数は flat-slot モデルで表現できないため諦める。
    if has_for_target_shadow(params, body) {
        bail("for-target-shadow", None);
        return None;
    }
    // base slot をリゾルバと同順で採番する: パラメータ → トップレベル let/mut/const。
    let mut slots: HashMap<String, u16> = HashMap::new();
    let mut slot_mut: Vec<bool> = Vec::new();
    let mut slot_type: Vec<Option<String>> = Vec::new();
    let mut self_slot: Option<u16> = None;
    let mut n: u16 = 0;
    for p in params {
        if p.variadic {
            bail("variadic-param", None);
            return None;
        }
        if p.name == "self" {
            self_slot = Some(n);
        }
        slots.insert(p.name.clone(), n);
        slot_mut.push(p.mutable);
        slot_type.push(p.type_ann.clone());
        n = n.checked_add(1)?;
    }
    // パラメータ数（`self` 含む）。デバッガが「まだ宣言されていないローカル」を隠すのに使う（#1）。
    let n_params = n as usize;
    // トップレベル宣言を事前採番。LetTuple/Static/入れ子定義など slot をずらす形は非対応。
    for stmt in body {
        match stmt {
            Stmt::Let(name, ty, _) | Stmt::Const(name, ty, _)
                if name != "_" && !slots.contains_key(name) =>
            {
                slots.insert(name.clone(), n);
                slot_mut.push(false);
                slot_type.push(ty.clone());
                n = n.checked_add(1)?;
            }
            Stmt::Mut(name, ty, _) if name != "_" && !slots.contains_key(name) => {
                slots.insert(name.clone(), n);
                slot_mut.push(true);
                slot_type.push(ty.clone());
                n = n.checked_add(1)?;
            }
            // `_` 名・既出名の宣言は base slot を増やさない（no-op）。
            Stmt::Let(..) | Stmt::Const(..) | Stmt::Mut(..) => {}
            // slot を採番する可能性のある未対応の宣言的文があれば、番号ずれを避けて丸ごと諦める。
            Stmt::LetTuple { .. }
            | Stmt::Static(..)
            | Stmt::FnDef { .. }
            | Stmt::GenDef { .. }
            | Stmt::ClassDef { .. }
            | Stmt::TraitDef { .. }
            | Stmt::ProtocolDef { .. }
            | Stmt::NewTypeDef { .. }
            | Stmt::EnumDef { .. }
            | Stmt::Import { .. }
            | Stmt::FromImport { .. } => {
                bail("decl-prepass", Some(stmt));
                return None;
            }
            _ => {}
        }
    }
    // ネストしたブロック（if/while/match のボディ）内の Let/Const/Mut にも
    // フレーム内固定 slot を割り当てる（R0-B: 関数内の全ローカルが平坦 slot）。
    // トップレベル decl は上で採番済みなのでスキップされる。順序は問わない
    // （compile は slots 引きで参照する）。シャドウイング禁止＝同名は非同時生存なので
    // slot 再利用は健全。リゾルバは nested 名を解決しない（Ident のまま）ので衝突しない。
    if collect_nested_decls(body, &mut slots, &mut slot_mut, &mut slot_type, &mut n).is_none() {
        bail("nested-decls", None);
        return None;
    }

    // V-E: slot → 変数名 のデバッグ名テーブル（named slot のみ。temp は無名）。
    let mut local_names = vec![String::new(); n as usize];
    for (name, &slot) in &slots {
        if let Some(entry) = local_names.get_mut(slot as usize) {
            *entry = name.clone();
        }
    }

    let mut c = Compiler {
        code: Vec::new(),
        consts: Vec::new(),
        names: Vec::new(),
        attr_caches: Vec::new(),
        spans: Vec::new(),
        stmt_spans: Vec::new(),
        pending_stmt: None,
        annotations,
        slots,
        slot_mut,
        slot_type,
        self_slot,
        loops: Vec::new(),
        block_ctxs: Vec::new(),
        debug_mode: false,
        named_locals: n,
        temps_in_use: 0,
        n_locals: n as usize,
        async_blocks: Vec::new(),
        global_caches: Vec::new(),
        toplevel_globals: HashSet::new(),
    };

    for stmt in body {
        c.compile_stmt(stmt)?;
    }
    // 本体末尾までフォールオフしたら None を返す。
    c.emit(Op::ReturnNil);

    // 覗き穴最適化（#2a）。コード生成は「素直に出す」ままにして、構造的に出る無駄
    // （`else` 無し `if` の次命令への `Jump` 等）はここで回収する。意味論は不変。
    super::peephole::optimize(&mut c.code, &mut c.stmt_spans);

    let chunk = Chunk {
        code: c.code,
        consts: c.consts,
        names: c.names,
        attr_caches: c.attr_caches,
        spans: c.spans,
        stmt_spans: c.stmt_spans,
        local_names,
        n_locals: c.n_locals,
        n_params,
        async_blocks: c.async_blocks,
        global_caches: c.global_caches,
    };

    // 開発用フック: `AR_VM_DUMP=1` で生成バイトコードを標準エラーへ逆アセンブルする。
    // どの式に型特化 op が乗ったかを目視で確認するために使う（disasm.rs の唯一の呼び元）。
    if std::env::var("AR_VM_DUMP").is_ok_and(|v| !v.is_empty()) {
        eprintln!("{}", super::disasm::disassemble(&chunk, "<fn>"));
    }

    Some(chunk)
}

/// デバッガ REPL の 1 文をデバッグモード（名前引きアクセス）でコンパイルする。
/// 対応: 式文（値を `Return`）・`let/const dbg::name = 式`（`DeclareName`）。
/// メソッド呼び出し・添字・制御フロー等は `None`（呼び出し側がツリーウォークへフォールバック）。
pub fn compile_debug(stmt: &Stmt) -> Option<Chunk> {
    let mut c = Compiler {
        code: Vec::new(),
        consts: Vec::new(),
        names: Vec::new(),
        attr_caches: Vec::new(),
        spans: Vec::new(),
        stmt_spans: Vec::new(),
        pending_stmt: None,
        // デバッグ経路は注釈を持たない（空＝型特化しない）。
        annotations: std::rc::Rc::new(crate::type_check::AstAnnotations::default()),
        slots: HashMap::new(),
        slot_mut: Vec::new(),
        slot_type: Vec::new(),
        self_slot: None,
        loops: Vec::new(),
        block_ctxs: Vec::new(),
        debug_mode: true,
        named_locals: 0,
        temps_in_use: 0,
        n_locals: 0,
        async_blocks: Vec::new(),
        global_caches: Vec::new(),
        toplevel_globals: HashSet::new(),
    };
    match stmt {
        Stmt::Expr(e) => {
            c.compile_expr(e)?;
            c.emit(Op::Return); // 式の値を返す（呼び出し側が表示）
        }
        Stmt::Let(name, _, e) | Stmt::Const(name, _, e) if name != "_" => {
            c.compile_expr(e)?;
            let ni = c.add_name(name);
            c.emit(Op::DeclareName(ni));
            c.emit(Op::ReturnNil);
        }
        _ => return None,
    }
    Some(Chunk {
        code: c.code,
        consts: c.consts,
        names: c.names,
        attr_caches: c.attr_caches,
        spans: c.spans,
        stmt_spans: c.stmt_spans,
        local_names: Vec::new(),
        n_locals: c.n_locals,
        // デバッグ REPL 用 Chunk は停止対象ではないので 0 でよい。
        n_params: 0,
        async_blocks: Vec::new(),
        global_caches: c.global_caches,
    })
}

/// `for` ループ変数（式形含む）が param または非 `for` 宣言と名前衝突するか（＝ブロックスコープの
/// シャドウ）を判定する。Arrow の `for` 変数はブロックスコープ（ループ後に外側へ戻る）だが、
/// flat-slot VM モデルは同名 slot を再利用するため、シャドウ時に外側変数を上書きしてツリーウォークと
/// 挙動が食い違う。該当関数はコンパイルを諦める（ツリーウォークへフォールバック）。
fn has_for_target_shadow(params: &[Param], body: &[Stmt]) -> bool {
    let mut for_names: HashSet<String> = HashSet::new();
    let mut decl_names: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
    scan_shadow_stmts(body, &mut for_names, &mut decl_names);
    for_names.iter().any(|n| decl_names.contains(n))
}

fn scan_shadow_stmts(
    stmts: &[Stmt],
    for_names: &mut HashSet<String>,
    decl_names: &mut HashSet<String>,
) {
    for s in stmts {
        match s {
            Stmt::Let(n, _, e) | Stmt::Const(n, _, e) | Stmt::Mut(n, _, e) => {
                decl_names.insert(n.clone());
                scan_shadow_expr(e, for_names, decl_names);
            }
            Stmt::LetTuple { targets, value, .. } => {
                for t in targets {
                    match t {
                        TupleTarget::Let(n) | TupleTarget::Bare(n) | TupleTarget::Mut(n) => {
                            decl_names.insert(n.clone());
                        }
                        TupleTarget::Wildcard => {}
                    }
                }
                scan_shadow_expr(value, for_names, decl_names);
            }
            Stmt::Static(n, e, _) => {
                decl_names.insert(n.clone());
                scan_shadow_expr(e, for_names, decl_names);
            }
            Stmt::For { targets, iter, body } => {
                for t in targets {
                    for_names.insert(t.clone());
                }
                scan_shadow_expr(iter, for_names, decl_names);
                scan_shadow_stmts(body, for_names, decl_names);
            }
            Stmt::Expr(e)
            | Stmt::BlockReturn(e, _)
            | Stmt::LoopYield(e)
            | Stmt::Yield(e)
            | Stmt::Return(Some(e))
            | Stmt::Assign { value: e, .. }
            | Stmt::CompoundAssign { value: e, .. } => scan_shadow_expr(e, for_names, decl_names),
            Stmt::AttrAssign { target, value } | Stmt::AttrCompoundAssign { target, value, .. } => {
                scan_shadow_expr(target, for_names, decl_names);
                scan_shadow_expr(value, for_names, decl_names);
            }
            Stmt::If { branches, else_body } => {
                for (c, b) in branches {
                    scan_shadow_expr(c, for_names, decl_names);
                    scan_shadow_stmts(b, for_names, decl_names);
                }
                if let Some(eb) = else_body {
                    scan_shadow_stmts(eb, for_names, decl_names);
                }
            }
            Stmt::While { cond, body } => {
                scan_shadow_expr(cond, for_names, decl_names);
                scan_shadow_stmts(body, for_names, decl_names);
            }
            Stmt::Match { subject, arms, .. } => {
                scan_shadow_expr(subject, for_names, decl_names);
                for a in arms {
                    scan_shadow_stmts(&a.body, for_names, decl_names);
                }
            }
            Stmt::Block(b) => scan_shadow_stmts(b, for_names, decl_names),
            Stmt::Try { body, handlers, finally_body } => {
                scan_shadow_stmts(body, for_names, decl_names);
                for h in handlers {
                    if let Some(alias) = &h.name {
                        decl_names.insert(alias.clone());
                    }
                    scan_shadow_stmts(&h.body, for_names, decl_names);
                }
                if let Some(fb) = finally_body {
                    scan_shadow_stmts(fb, for_names, decl_names);
                }
            }
            _ => {}
        }
    }
}

fn scan_shadow_expr(
    e: &Expr,
    for_names: &mut HashSet<String>,
    decl_names: &mut HashSet<String>,
) {
    macro_rules! rec {
        ($x:expr) => {
            scan_shadow_expr($x, for_names, decl_names)
        };
    }
    match e {
        Expr::Block { stmts, .. } => scan_shadow_stmts(stmts, for_names, decl_names),
        Expr::IfExpr { branches, else_body, .. } => {
            for (c, b) in branches {
                rec!(c);
                scan_shadow_stmts(b, for_names, decl_names);
            }
            if let Some(eb) = else_body {
                scan_shadow_stmts(eb, for_names, decl_names);
            }
        }
        Expr::MatchExpr { subject, arms, .. } => {
            rec!(subject);
            for a in arms {
                scan_shadow_stmts(&a.body, for_names, decl_names);
            }
        }
        Expr::ForExpr { target, iter, body, .. } => {
            for_names.insert(target.clone());
            rec!(iter);
            scan_shadow_stmts(body, for_names, decl_names);
        }
        Expr::WhileExpr { cond, body, .. } => {
            rec!(cond);
            scan_shadow_stmts(body, for_names, decl_names);
        }
        Expr::BinOp { left, right, .. } => {
            rec!(left);
            rec!(right);
        }
        Expr::UnaryOp { operand, .. } => rec!(operand),
        Expr::Call { func, args, .. } => {
            rec!(func);
            for a in args {
                match a {
                    CallArg::Positional(x) | CallArg::Keyword { value: x, .. } => rec!(x),
                    CallArg::Variadic(xs) => {
                        for x in xs {
                            rec!(x);
                        }
                    }
                }
            }
        }
        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => rec!(object),
        Expr::Subscript { object, index, .. } => {
            rec!(object);
            rec!(index);
        }
        Expr::Slice { begin, end, step } => {
            for x in [begin, end, step].into_iter().flatten() {
                rec!(x);
            }
        }
        Expr::List(items) | Expr::Tuple(items) | Expr::Set(items) => {
            for x in items {
                rec!(x);
            }
        }
        Expr::Dict(pairs) => {
            for (k, v) in pairs {
                rec!(k);
                rec!(v);
            }
        }
        Expr::TemplateInstantiate { base, .. } => rec!(base),
        Expr::IsType { expr, .. } | Expr::MustBe { expr, .. } => rec!(expr),
        Expr::Cast { object, .. } => rec!(object),
        _ => {}
    }
}

/// slot テーブルへ1つ宣言を追加する（既出名・`_` はスキップ）。
fn add_decl(
    name: &str,
    ty: &Option<String>,
    mutable: bool,
    slots: &mut HashMap<String, u16>,
    slot_mut: &mut Vec<bool>,
    slot_type: &mut Vec<Option<String>>,
    n: &mut u16,
) -> Option<()> {
    if name != "_" && !slots.contains_key(name) {
        slots.insert(name.to_string(), *n);
        slot_mut.push(mutable);
        slot_type.push(ty.clone());
        *n = n.checked_add(1)?;
    }
    Some(())
}

/// ネストしたブロック内の `let`/`const`/`mut` 宣言に平坦 slot を割り当てる（再帰）。
/// コンパイラが本体をコンパイルできる構文（if/while/match/for/try）と、**ブロック式**
/// （式の中の `block:`/if/while/for/match 式の本体宣言）にも踏み込む。
/// 既出名（トップレベル decl・別ブロックの同名）はスキップ（slot 再利用）。
fn collect_nested_decls(
    body: &[Stmt],
    slots: &mut HashMap<String, u16>,
    slot_mut: &mut Vec<bool>,
    slot_type: &mut Vec<Option<String>>,
    n: &mut u16,
) -> Option<()> {
    for stmt in body {
        match stmt {
            Stmt::Let(name, ty, e) | Stmt::Const(name, ty, e) => {
                add_decl(name, ty, false, slots, slot_mut, slot_type, n)?;
                collect_expr_decls(e, slots, slot_mut, slot_type, n)?;
            }
            Stmt::Mut(name, ty, e) => {
                add_decl(name, ty, true, slots, slot_mut, slot_type, n)?;
                collect_expr_decls(e, slots, slot_mut, slot_type, n)?;
            }
            Stmt::Expr(e)
            | Stmt::BlockReturn(e, _)
            | Stmt::LoopYield(e)
            | Stmt::Yield(e)
            | Stmt::Return(Some(e)) => collect_expr_decls(e, slots, slot_mut, slot_type, n)?,
            Stmt::Assign { value, .. } | Stmt::CompoundAssign { value, .. } => {
                collect_expr_decls(value, slots, slot_mut, slot_type, n)?
            }
            Stmt::AttrAssign { target, value }
            | Stmt::AttrCompoundAssign { target, value, .. } => {
                collect_expr_decls(target, slots, slot_mut, slot_type, n)?;
                collect_expr_decls(value, slots, slot_mut, slot_type, n)?;
            }
            Stmt::If { branches, else_body } => {
                for (c, b) in branches {
                    collect_expr_decls(c, slots, slot_mut, slot_type, n)?;
                    collect_nested_decls(b, slots, slot_mut, slot_type, n)?;
                }
                if let Some(eb) = else_body {
                    collect_nested_decls(eb, slots, slot_mut, slot_type, n)?;
                }
            }
            Stmt::While { cond, body } => {
                collect_expr_decls(cond, slots, slot_mut, slot_type, n)?;
                collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
            }
            Stmt::Match { subject, arms, .. } => {
                collect_expr_decls(subject, slots, slot_mut, slot_type, n)?;
                for arm in arms {
                    collect_nested_decls(&arm.body, slots, slot_mut, slot_type, n)?;
                }
            }
            Stmt::For { targets, iter, body } => {
                // ループ変数は可変（tree-walk は Var::new(item, true)）。型注釈なし。
                for t in targets {
                    add_decl(t, &None, true, slots, slot_mut, slot_type, n)?;
                }
                collect_expr_decls(iter, slots, slot_mut, slot_type, n)?;
                collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
            }
            Stmt::Try { body, handlers, finally_body } => {
                collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
                for h in handlers {
                    // `except E as e:` の別名は不変束縛（tree-walk は Var::new(exc, false)）。
                    if let Some(alias) = &h.name {
                        add_decl(alias, &None, false, slots, slot_mut, slot_type, n)?;
                    }
                    collect_nested_decls(&h.body, slots, slot_mut, slot_type, n)?;
                }
                if let Some(fb) = finally_body {
                    collect_nested_decls(fb, slots, slot_mut, slot_type, n)?;
                }
            }
            // その他（未対応構文）には踏み込まない。compile_stmt が到達時に bail する。
            _ => {}
        }
    }
    Some(())
}

/// 式の中の**ブロック式**（`block:`/if/while/for/match 式）の本体宣言に slot を割り当てる（再帰）。
/// ブロック式でない部分式も辿り、入れ子のブロック式を漏れなく採番する。
fn collect_expr_decls(
    e: &Expr,
    slots: &mut HashMap<String, u16>,
    slot_mut: &mut Vec<bool>,
    slot_type: &mut Vec<Option<String>>,
    n: &mut u16,
) -> Option<()> {
    macro_rules! rec {
        ($x:expr) => {
            collect_expr_decls($x, slots, slot_mut, slot_type, n)?
        };
    }
    match e {
        // ── ブロック式（本体に宣言を持つ） ──
        Expr::Block { stmts, .. } => collect_nested_decls(stmts, slots, slot_mut, slot_type, n)?,
        Expr::IfExpr { branches, else_body, .. } => {
            for (c, b) in branches {
                rec!(c);
                collect_nested_decls(b, slots, slot_mut, slot_type, n)?;
            }
            if let Some(eb) = else_body {
                collect_nested_decls(eb, slots, slot_mut, slot_type, n)?;
            }
        }
        Expr::MatchExpr { subject, arms, .. } => {
            rec!(subject);
            for arm in arms {
                collect_nested_decls(&arm.body, slots, slot_mut, slot_type, n)?;
            }
        }
        Expr::ForExpr { target, iter, body, .. } => {
            add_decl(target, &None, true, slots, slot_mut, slot_type, n)?;
            rec!(iter);
            collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
        }
        Expr::WhileExpr { cond, body, .. } => {
            rec!(cond);
            collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
        }
        // ── 部分式を辿る（入れ子のブロック式を探す） ──
        Expr::BinOp { left, right, .. } => {
            rec!(left);
            rec!(right);
        }
        Expr::UnaryOp { operand, .. } => rec!(operand),
        Expr::Call { func, args, .. } => {
            rec!(func);
            for a in args {
                match a {
                    CallArg::Positional(x) | CallArg::Keyword { value: x, .. } => rec!(x),
                    CallArg::Variadic(xs) => {
                        for x in xs {
                            rec!(x);
                        }
                    }
                }
            }
        }
        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => rec!(object),
        Expr::Subscript { object, index, .. } => {
            rec!(object);
            rec!(index);
        }
        Expr::Slice { begin, end, step } => {
            for x in [begin, end, step].into_iter().flatten() {
                rec!(x);
            }
        }
        Expr::List(items) | Expr::Tuple(items) | Expr::Set(items) => {
            for x in items {
                rec!(x);
            }
        }
        Expr::Dict(pairs) => {
            for (k, v) in pairs {
                rec!(k);
                rec!(v);
            }
        }
        Expr::TemplateInstantiate { base, .. } => rec!(base),
        Expr::IsType { expr, .. } | Expr::MustBe { expr, .. } => rec!(expr),
        Expr::Cast { object, .. } => rec!(object),
        // リテラル・Ident 等は宣言を含まない。
        _ => {}
    }
    Some(())
}

/// `stmts` に「囲む try を飛び越える」制御フローがあるかを保守的に判定する。
/// `include_return` が真なら `return` も脱出とみなす（finally は return でも走る必要があるため）。
/// `break`/`continue` は `stmts` 内の while/for（loop_depth>0）に囲まれていなければ脱出。
/// `block_return`/`loop_yield` は常に脱出とみなす。
/// ブロック式の本体が VM コンパイル不能な脱出を含むかを判定する。
/// `return` は常に不可（ブロック式内 return は構文エラー）。`break`/`continue` は、
/// 非ループ式（block:/if/match）では本体内 while/for に囲まれなければ脱出＝不可、
/// ループ式（for/while 式）では自身が最内ループなので loop_depth 0 のものは許容。
/// `block_return`/`loop_yield` は当該ブロック式が扱うので許容。
fn block_body_bails(stmts: &[Stmt], is_loop_expr: bool, loop_depth: usize) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Return(_) => true,
        Stmt::Break | Stmt::Continue => loop_depth == 0 && !is_loop_expr,
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            block_body_bails(body, is_loop_expr, loop_depth + 1)
        }
        Stmt::If { branches, else_body } => {
            branches
                .iter()
                .any(|(_, b)| block_body_bails(b, is_loop_expr, loop_depth))
                || else_body
                    .as_ref()
                    .is_some_and(|eb| block_body_bails(eb, is_loop_expr, loop_depth))
        }
        Stmt::Match { arms, .. } => arms
            .iter()
            .any(|a| block_body_bails(&a.body, is_loop_expr, loop_depth)),
        Stmt::Block(b) => block_body_bails(b, is_loop_expr, loop_depth),
        Stmt::Try { body, handlers, finally_body } => {
            block_body_bails(body, is_loop_expr, loop_depth)
                || handlers
                    .iter()
                    .any(|h| block_body_bails(&h.body, is_loop_expr, loop_depth))
                || finally_body
                    .as_ref()
                    .is_some_and(|fb| block_body_bails(fb, is_loop_expr, loop_depth))
        }
        _ => false,
    })
}

fn has_escape(stmts: &[Stmt], include_return: bool, loop_depth: usize) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Return(_) => include_return,
        Stmt::Break | Stmt::Continue => loop_depth == 0,
        Stmt::BlockReturn(..) | Stmt::LoopYield(_) => true,
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            has_escape(body, include_return, loop_depth + 1)
        }
        Stmt::If { branches, else_body } => {
            branches
                .iter()
                .any(|(_, b)| has_escape(b, include_return, loop_depth))
                || else_body
                    .as_ref()
                    .is_some_and(|eb| has_escape(eb, include_return, loop_depth))
        }
        Stmt::Match { arms, .. } => arms
            .iter()
            .any(|a| has_escape(&a.body, include_return, loop_depth)),
        Stmt::Block(b) => has_escape(b, include_return, loop_depth),
        Stmt::Try { body, handlers, finally_body } => {
            has_escape(body, include_return, loop_depth)
                || handlers
                    .iter()
                    .any(|h| has_escape(&h.body, include_return, loop_depth))
                || finally_body
                    .as_ref()
                    .is_some_and(|fb| has_escape(fb, include_return, loop_depth))
        }
        _ => false,
    })
}

impl Compiler {
    #[inline]
    fn emit(&mut self, op: Op) -> usize {
        self.code.push(op);
        // 行テーブルを code と 1:1 に保つ（#1）。`compile_stmt` が予約していれば
        // **この op が文の先頭**なので、そこに文の位置を記録する。
        self.stmt_spans
            .push(self.pending_stmt.take().unwrap_or(super::chunk::NOT_STMT));
        self.code.len() - 1
    }

    /// スタック規律の一時 slot を確保する（match サブジェクト等）。名前付き slot の上に積む。
    /// `free_temp` と対で使う。フレーム総 slot 数（`n_locals`）を必要に応じて拡張する。
    fn alloc_temp(&mut self) -> Option<u16> {
        let slot = self.named_locals.checked_add(self.temps_in_use)?;
        self.temps_in_use = self.temps_in_use.checked_add(1)?;
        let total = self.named_locals as usize + self.temps_in_use as usize;
        if total > self.n_locals {
            self.n_locals = total;
        }
        Some(slot)
    }

    fn free_temp(&mut self) {
        self.temps_in_use -= 1;
    }

    fn add_const(&mut self, v: Value) -> u32 {
        let idx = self.consts.len() as u32;
        self.consts.push(v);
        idx
    }

    /// 式が「単純なローカル読み」なら slot を返す（超命令の融合判定, #2）。
    /// `LoadLocal` に落ちる形（`Resolution::Local` / slot 表に載る未解決 `Ident`）のみ。
    /// debug_mode では融合しない（未解決 `Ident` は `LoadName` に落ちるため）。
    /// `Resolution::Global` は対象外（`_` に落ちる）。
    fn as_local(&self, e: &Expr) -> Option<u16> {
        if self.debug_mode {
            return None;
        }
        match e {
            Expr::Ident { res: Resolution::Local(slot), .. } => u16::try_from(*slot).ok(),
            Expr::Ident { name, res: Resolution::Unresolved, .. } => self.slots.get(name).copied(),
            _ => None,
        }
    }

    /// 式が数値/真偽リテラルなら定数値を返す（超命令の融合判定, #2）。
    fn as_const_lit(e: &Expr) -> Option<Value> {
        match e {
            Expr::Int(n) => Some(Value::Int(*n)),
            Expr::Float(f) => Some(Value::Float(*f)),
            Expr::Bool(b) => Some(Value::Bool(*b)),
            _ => None,
        }
    }

    /// 局所変数の型注釈から二項演算の特化種別を導出する（#16 段階 E）。
    ///
    /// 両オペランドが**同一プリミティブの型注釈を持つ局所変数**のときだけ種別を返す。
    /// リテラル（`Expr::Int`/`Expr::Float`）は型が自明なので相方に合わせて認める。
    /// テンプレート実体化後の関数のように「AST には具体型が書かれているが注釈テーブルは
    /// 原型を指している」ケースを拾うのが目的。
    fn local_operand_kind(
        &self,
        left: &Expr,
        right: &Expr,
    ) -> Option<crate::type_check::BinOperandKind> {
        let (l, r) = (self.expr_prim(left)?, self.expr_prim(right)?);
        // 両方がリテラルなら特化しても意味が無い（定数同士）。
        if matches!(left, Expr::Int(_) | Expr::Float(_))
            && matches!(right, Expr::Int(_) | Expr::Float(_))
        {
            return None;
        }
        Self::pair_kind(l, r)
    }

    /// 左辺が **slot 番号で与えられる** 場合の特化種別（複合代入 `x <op>= e` 用・#2b）。
    /// `local_operand_kind` の左辺を「式」から「slot」に替えただけで、判断基準は同一。
    fn slot_operand_kind(
        &self,
        slot: u16,
        right: &Expr,
    ) -> Option<crate::type_check::BinOperandKind> {
        Self::pair_kind(self.slot_prim(slot)?, self.expr_prim(right)?)
    }

    /// 式のプリミティブ型名。局所変数は型注釈から、数値リテラルは自明な型から。
    fn expr_prim(&self, e: &Expr) -> Option<&'static str> {
        match e {
            Expr::Int(_) => Some("int"),
            Expr::Float(_) => Some("float"),
            _ => self.slot_prim(self.as_local(e)?),
        }
    }

    /// 局所 slot のプリミティブ型名（型注釈が int/float のときのみ）。
    fn slot_prim(&self, slot: u16) -> Option<&'static str> {
        match self.slot_type.get(slot as usize)?.as_deref()? {
            "int" => Some("int"),
            "float" => Some("float"),
            _ => None,
        }
    }

    /// 注釈テーブルが焼いた**結果型**からプリミティブ型名を引く（#2b）。
    /// `slot_type`（AST に書かれた型注釈）では届かないノード（属性読みなど）用。
    fn annot_prim(&self, node_id: u32) -> Option<&'static str> {
        match self.annotations.resolved_type(node_id)? {
            crate::type_check::InferredType::Int => Some("int"),
            crate::type_check::InferredType::Float => Some("float"),
            _ => None,
        }
    }

    /// 両オペランドのプリミティブ型名が一致していれば特化種別に落とす。
    fn pair_kind(l: &str, r: &str) -> Option<crate::type_check::BinOperandKind> {
        use crate::type_check::BinOperandKind as K;
        if l != r {
            return None;
        }
        match l {
            "int" => Some(K::Int),
            "float" => Some(K::Float),
            _ => None,
        }
    }

    /// 注釈が「両オペランド int/float 確定」かつ型特化してよい op なら、その種別を返す（#16 段階(b)）。
    ///
    /// 許可する op は種別ごとに違う。`apply_binop` に対応するアームが存在するものだけを特化し、
    /// それ以外（float の `//`・`%` など）は汎用パスに委ねてエラー処理を一箇所に保つ。
    /// ゼロ除算は特化側が `None` を返して汎用へ落ちるので、op としては許可してよい。
    fn specialized_bin_kind(
        &self,
        op: &BinOp,
        node_id: u32,
        left: &Expr,
        right: &Expr,
    ) -> Option<crate::type_check::BinOperandKind> {
        let kind = match self.annotations.binop_kind(node_id) {
            Some(k) => k,
            // 注釈が無いときは**局所変数の型注釈**から導出する（#16 段階 E）。
            //
            // テンプレート実体化では `subst` が param の型注釈を具体型へ置き換える
            // （`fn add[T](a: T, b: T)` → `add[int]` なら `a: int, b: int`）が、
            // node-id は原型からコピーされるため注釈テーブルは**型変数のままの原型**を指す。
            // そこで実体化後の AST に書かれている型注釈を直接見る。
            // 注釈テーブルが届かない箇所（import 先モジュール等）にも同じ理由で効く。
            //
            // 特化 op は実行時型が想定外なら汎用へフォールバックするので、
            // この導出が外れていても**結果は変わらない**（速度の無駄が出るだけ）。
            None => self.local_operand_kind(left, right)?,
        };
        Self::gate_bin_kind(kind, op)
    }

    /// 左辺が **slot 番号で与えられる** 場合の特化種別（複合代入 `x <op>= e` 用・#2b）。
    /// 注釈の引き方・op の許可判定は `specialized_bin_kind` と同一で、
    /// 型導出のフォールバックだけが slot 版になる。
    /// 注釈テーブルだけから二項演算の種別を引く（#10-b のグローバル複合代入用）。
    /// slot 版（`specialized_bin_kind_slot`）の「slot の型注釈から推す」経路はグローバルには
    /// 使えないので、型検査が焼いた `binop_kind` のみを見る。
    fn annot_binop_kind(&self, node_id: u32) -> Option<crate::type_check::BinOperandKind> {
        // op のゲートは呼び出し側が渡す op で行う（`gate_bin_kind`）。ここでは種別だけ返す。
        self.annotations.binop_kind(node_id)
    }

    /// 最上位宣言の名前なら name プールの index を返す（#10-c）。
    ///
    /// 条件は 2 つ: **最上位モードであること**（`toplevel_globals` が非空）と、
    /// **その名前が slot に無いこと**。後者が要るのは、最上位文の内側（ループ本体・
    /// ブロック式の中）の宣言は slot だから — そちらは従来どおり `StoreLocal*` に落とす。
    fn toplevel_decl_name(&mut self, name: &str) -> Option<u32> {
        if self.toplevel_globals.is_empty() || self.slots.contains_key(name) {
            return None;
        }
        Some(self.add_name(name))
    }

    /// 変数への書き込み先を決める（#10-b）。
    ///
    /// 1. VM の slot にある名前 → `Local`（関数本体でもブロック内宣言でもここに来る）
    /// 2. 最上位モードで可視グローバルと確定できる名前 → `Global`
    /// 3. どちらでもない → `None`（＝ツリーウォークへフォールバック）
    ///
    /// ⚠ 順序が重要。ループ本体の `let` は毎回スコープに入る**ローカル**なので、
    /// 同名グローバルより先に slot を見なければならない。
    fn store_target(&mut self, name: &str) -> Option<StoreTarget> {
        if let Some(&slot) = self.slots.get(name) {
            return Some(StoreTarget::Local(slot));
        }
        if self.toplevel_globals.contains(name) {
            let ni = self.add_name(name);
            // `LoadGlobal` と同じく emit 1 回につきキャッシュ枠を 1 本割り当てる
            // （枠は共有しない。op ごとに焼く index の意味が違うため — `Op::StoreGlobal` 参照）。
            let ci = self.global_caches.len() as u32;
            self.global_caches.push(crate::ast::SlotCache::default());
            return Some(StoreTarget::Global(ni, ci));
        }
        bail("store-target", None);
        None
    }

    fn specialized_bin_kind_slot(
        &self,
        op: &BinOp,
        node_id: u32,
        slot: u16,
        right: &Expr,
    ) -> Option<crate::type_check::BinOperandKind> {
        let kind = match self.annotations.binop_kind(node_id) {
            Some(k) => k,
            None => self.slot_operand_kind(slot, right)?,
        };
        Self::gate_bin_kind(kind, op)
    }

    /// 特化してよい op かを判定する（種別ごとの許可リスト）。
    /// `specialized_bin_kind` / `specialized_bin_kind_slot` の共通判断。
    fn gate_bin_kind(
        kind: crate::type_check::BinOperandKind,
        op: &BinOp,
    ) -> Option<crate::type_check::BinOperandKind> {
        use crate::type_check::BinOperandKind as K;
        let allowed = match kind {
            // int/int は `apply_binop` の Int/Int アームを全て特化できる。
            K::Int => matches!(
                op,
                BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::FloorDiv
                    | BinOp::Mod
                    | BinOp::Pow
                    | BinOp::BitAnd
                    | BinOp::BitOr
                    | BinOp::BitXor
                    | BinOp::LShift
                    | BinOp::RShift
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::LtEq
                    | BinOp::GtEq
                    | BinOp::Eq
                    | BinOp::NotEq
            ),
            // float は `//`・`%`・ビット演算のアームが無いので算術と比較のみ。
            K::Float => matches!(
                op,
                BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::Pow
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::LtEq
                    | BinOp::GtEq
                    | BinOp::Eq
                    | BinOp::NotEq
            ),
        };
        if allowed {
            Some(kind)
        } else {
            None
        }
    }

    /// 二項演算 `left <op> right` を超命令へ融合できれば emit して `true`（#2 ＋ plan A 型特化）。
    /// `local <op> local` → `BinLocalLocal`、`local <op> リテラル` → `BinLocalConst`。
    /// さらに注釈が「両オペランド int/float 確定」かつ対応 op（Add/Sub/Mul・比較）なら**型特化 op**
    /// （`IntBinLL`/`FloatBinLC` 等）を emit（タグ検査・op ディスパッチ・clone を削減）。
    /// 型特化 op は実行時型が想定外なら**汎用へフォールバック**するので、注釈が古くても健全。
    /// 融合できなければ `false`（呼び出し側が通常経路 `LoadLocal…; Bin` を出す）。意味論は不変。
    fn try_emit_bin_fused(&mut self, left: &Expr, right: &Expr, op: &BinOp, node_id: u32) -> bool {
        let Some(a) = self.as_local(left) else {
            return false;
        };
        let kind = self.specialized_bin_kind(op, node_id, left, right);
        self.emit_bin_fused_slot(a, kind, right, op)
    }

    /// 左辺が slot と確定している二項演算を 1 命令へ融合 emit する（`try_emit_bin_fused` の中核）。
    /// 右辺が局所変数なら `*BinLL`、定数リテラルなら `*BinLC`。どちらでもなければ `false` を返し、
    /// 呼び出し側が通常経路（オペランドを積んでから `Bin`/`*BinSS`）を出す。
    ///
    /// **評価順について**: 融合後はスタックへ積まず frame から左辺を読むので、形の上では
    /// 「右辺を用意してから左辺を読む」順になる。ただし融合する右辺は局所変数読みか定数
    /// リテラルのみで**副作用が無い**ため、観測される値は融合前と同一（`CallMethodLocal` と同じ理由）。
    fn emit_bin_fused_slot(
        &mut self,
        a: u16,
        kind: Option<crate::type_check::BinOperandKind>,
        right: &Expr,
        op: &BinOp,
    ) -> bool {
        use crate::type_check::BinOperandKind as K;
        if let Some(b) = self.as_local(right) {
            match kind {
                Some(K::Int) => self.emit(Op::IntBinLL(a, b, op.clone())),
                Some(K::Float) => self.emit(Op::FloatBinLL(a, b, op.clone())),
                None => self.emit(Op::BinLocalLocal(a, b, op.clone())),
            };
            true
        } else if let Some(cv) = Self::as_const_lit(right) {
            let ci = self.add_const(cv);
            match kind {
                Some(K::Int) => self.emit(Op::IntBinLC(a, ci, op.clone())),
                Some(K::Float) => self.emit(Op::FloatBinLC(a, ci, op.clone())),
                None => self.emit(Op::BinLocalConst(a, ci, op.clone())),
            };
            true
        } else {
            false
        }
    }

    /// `LoadGlobal` を index キャッシュ付きで emit する（#11）。name プールとキャッシュ枠を確保。
    fn emit_load_global(&mut self, name: &str) {
        let ni = self.add_name(name);
        let ci = self.global_caches.len() as u32;
        self.global_caches.push(crate::ast::SlotCache::default());
        self.emit(Op::LoadGlobal(ni, ci));
    }

    fn add_name(&mut self, name: &str) -> u32 {
        let idx = self.names.len() as u32;
        self.names.push(name.to_string());
        self.attr_caches.push(crate::ast::AttrCache::default());
        idx
    }

    /// AST 型解決層の**検査指示**（`CheckBefore`）を消費する（#16 段階(b)(ii)）。
    ///
    /// `mustbe` / `=>` は型検査が常に `CheckBefore` を付けるので、現状これは実質いつも `true` を返す。
    /// それでも指示を経由するのは、将来チェッカが「この検査は静的に冗長」と証明できるようになったとき、
    /// **この一点を変えるだけで VM とネイティブの双方が検査を落とせる**ようにするため（＝解決の一元化）。
    ///
    /// 指示が `None`（未採番ノード・合成 AST・モジュール横断で注釈が無い）の場合は
    /// **検査が要るのか判らない**ので、その関数の VM 化自体を諦める（`false`）。
    /// 検査を省く方向へは決して倒さない。
    fn check_required(&self, node_id: u32) -> bool {
        matches!(
            self.annotations.directive(node_id),
            crate::type_check::Directive::CheckBefore(_)
        )
    }

    fn add_span(&mut self, span: &crate::token::Span) -> u32 {
        let idx = self.spans.len() as u32;
        self.spans.push(span.clone());
        idx
    }

    /// 次に emit する op を「文の先頭」として予約する（#1・行テーブル）。
    ///
    /// ツリーウォークは `exec()` の冒頭で**すべての文**について `should_pause_at` を呼ぶので、
    /// VM も**すべての文**の先頭を記録しないと停止位置が食い違う。
    /// 位置情報を持たない種類の文（`if`/`while`/`return` 等）は `STMT_NO_SPAN` を記録し、
    /// 表示スパンは `best_span_for` のフォールバック（`dbg_last_span`）に委ねる。
    ///
    /// ⚠ `debug_mode`（デバッガ REPL 用の `compile_debug`）では記録しない。
    /// あちらは停止対象ではなく、REPL 入力を評価するだけの Chunk。
    fn mark_stmt_start(&mut self, stmt: &Stmt) {
        if self.debug_mode {
            return;
        }
        let idx = match crate::interpreter::debugger::stmt_span_of(stmt) {
            Some(span) => self.add_span(&span),
            None => super::chunk::STMT_NO_SPAN,
        };
        self.pending_stmt = Some(idx);
    }

    /// バックパッチ用: 直後に置く命令の index を現在位置として返す。
    #[inline]
    fn here(&self) -> u32 {
        self.code.len() as u32
    }

    fn patch_jump(&mut self, at: usize, target: u32) {
        self.code[at] = match &self.code[at] {
            Op::Jump(_) => Op::Jump(target),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(target),
            Op::JumpIfFalseOrPop(_) => Op::JumpIfFalseOrPop(target),
            Op::JumpIfTrueOrPop(_) => Op::JumpIfTrueOrPop(target),
            _ => unreachable!("patch_jump on non-jump op"),
        };
    }

    /// 式 `e` が実行時に **確実に Instance** の base ローカルを指すかを保守的に判定する。
    /// `self` パラメータ（型注釈なしだが常に Instance）と、ユーザークラス型注釈の
    /// 識別子（解決済み・未解決とも）を true とする。メソッド呼び出し・属性代入のレシーバ判定に使う。
    fn object_is_instance(&self, e: &Expr) -> bool {
        let slot = match e {
            Expr::Ident { res: Resolution::Local(slot), .. } => *slot as usize,
            Expr::Ident { name, res: Resolution::Unresolved, .. } => match self.slots.get(name) {
                Some(&s) => s as usize,
                None => return false,
            },
            // slot を持たないレシーバ（グローバル変数・属性・呼び出し結果など）は
            // **型検査の注釈**で判定する（#26）。`slot_type` 経路と同じ 2 段の条件
            // （形＋出自）を通すので健全性は同じ。
            other => return self.annot_is_arrow_instance(other),
        };
        if Some(slot as u16) == self.self_slot {
            return true;
        }
        self.slot_type
            .get(slot)
            .and_then(|o| o.as_deref())
            .map(|t| self.is_arrow_instance_type(t))
            .unwrap_or(false)
    }

    /// 型注釈名が「実行時 `Value::Instance` になるユーザークラス」か（#27-a）。
    ///
    /// **2 段構え**である点が要点:
    /// 1. `is_user_instance_type` — 形（ジェネリック・union・組み込み型名を除く）
    /// 2. `annotations.is_arrow_class` — **出自**（外部言語スタブ由来のクラスを除く）
    ///
    /// 2 が無いと C# スタブのクラス（`import[cs-dll]`）が同じ `NamedInstance` 注釈で通り、
    /// 実行時の `Value::CsObject` を `Value::Instance` 前提の op へ流して落ちる
    /// （`event_cs_handler.ar` の off/auto 不一致で実際に踏んだ）。
    fn is_arrow_instance_type(&self, t: &str) -> bool {
        is_user_instance_type(t) && self.annotations.is_arrow_class(t)
    }

    /// 注釈テーブルが当該式を「Arrow クラスのインスタンス」と確定しているか（#26）。
    ///
    /// `slot_type` を持たないレシーバ（グローバル・属性・呼び出し結果）に対する
    /// `is_arrow_instance_type` の等価物。`NamedInstance` 以外（`Protocol`/`Union`/`Any`/
    /// 組み込み型）は保守的に false。
    ///
    /// ⚠ **`NamedInstance` だけで true にしてはいけない**。外部言語スタブのクラスも
    /// 同じ注釈になるので、`is_arrow_class`（出自）まで見る（#27-a）。
    fn annot_is_arrow_instance(&self, e: &Expr) -> bool {
        let Some(node_id) = expr_node_id(e) else {
            return false;
        };
        match self.annotations.resolved_type(node_id) {
            Some(crate::type_check::InferredType::NamedInstance(name)) => {
                self.is_arrow_instance_type(name)
            }
            _ => false,
        }
    }

    /// 呼び出し引数の `is_mutable`（`eval_call_args` と同じ判定: 変数 ident は変数の可変性、
    /// それ以外の式は保守的に true）。VM は base ローカルしか読まないので slot_mut で判定できる。
    fn arg_is_mutable(&self, e: &Expr) -> bool {
        match e {
            Expr::Ident { res: Resolution::Local(slot), .. } => {
                self.slot_mut.get(*slot as usize).copied().unwrap_or(true)
            }
            Expr::Ident { name, res: Resolution::Unresolved, .. } => self
                .slots
                .get(name)
                .and_then(|&s| self.slot_mut.get(s as usize).copied())
                .unwrap_or(true),
            _ => true,
        }
    }

    /// 位置引数をスタックへ push し、各引数の is_mutable を bit にした mask を返す。
    /// keyword/可変長引数・33個以上は非対応（`None`）。
    fn compile_call_args(&mut self, args: &[CallArg]) -> Option<u32> {
        if args.len() > 32 {
            return None;
        }
        let mut mask: u32 = 0;
        for (i, arg) in args.iter().enumerate() {
            match arg {
                CallArg::Positional(e) => {
                    if self.arg_is_mutable(e) {
                        mask |= 1 << i;
                    }
                    self.compile_expr(e)?;
                }
                // キーワード引数・可変長展開は非対応。
                _ => {
                    bail("call-arg", None);
                    return None;
                }
            }
        }
        Some(mask)
    }

    /// `target <- async->T: body` をコンパイルする（タスク #9）。
    /// 本体が参照する enclosing フレームの slot（`collect_referenced_names ∩ slots`）を捕捉対象に記録し、
    /// マネージャをスタックへロードして `AsyncSubmit(idx)` を発行する。実行時は frame から捕捉値を読み、
    /// グローバルと合わせて `capture_env` で env を組む（ツリーウォークの `exec_async_assign` と同一）。
    /// 捕捉は「本体が参照する slot」に限定（未参照ローカルは env に載せない）＝task 挙動は byte-identical。
    fn compile_async_assign(&mut self, target: &str, stmts: &[Stmt]) -> Option<()> {
        // 本体の参照名を収集し、enclosing frame の slot と交差したものを捕捉する。
        let mut refs: HashSet<String> = HashSet::new();
        crate::interpreter::collect_referenced_names(stmts, &mut refs);
        let mut captures: Vec<(String, u16, bool)> = refs
            .iter()
            .filter_map(|name| {
                let slot = *self.slots.get(name)?;
                let is_mut = self.slot_mut.get(slot as usize).copied().unwrap_or(false);
                Some((name.clone(), slot, is_mut))
            })
            .collect();
        // 決定的順序（HashSet は非決定）: slot 昇順。env の順序は task 挙動に影響しないが再現性のため固定。
        captures.sort_by_key(|(_, slot, _)| *slot);

        let idx = u32::try_from(self.async_blocks.len()).ok()?;
        self.async_blocks.push(crate::vm::chunk::AsyncBlock {
            body: stmts.to_vec(),
            captures,
        });
        // マネージャ値をスタックへ（ローカル slot 優先、なければグローバル名引き）。
        if let Some(&slot) = self.slots.get(target) {
            self.emit(Op::LoadLocal(slot));
        } else {
            self.emit_load_global(target);
        }
        self.emit(Op::AsyncSubmit(idx));
        Some(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Option<()> {
        self.mark_stmt_start(stmt);
        match stmt {
            Stmt::Expr(e) => {
                self.compile_expr(e)?;
                self.emit(Op::Pop);
            }
            Stmt::Return(Some(e)) => {
                self.compile_expr(e)?;
                self.emit(Op::Return);
            }
            Stmt::Return(None) => {
                self.emit(Op::ReturnNil);
            }
            // パラメータ（mut）への代入。let への代入は型検査で弾かれるので健全。
            // 最上位モード（#10-b）では slot に無い名前は可視グローバルへの代入になる。
            Stmt::Assign { name, value, .. } => match self.store_target(name)? {
                StoreTarget::Local(slot) => {
                    self.compile_expr(value)?;
                    self.emit(Op::StoreLocal(slot));
                }
                StoreTarget::Global(ni, ci) => {
                    self.compile_expr(value)?;
                    self.emit(Op::StoreGlobal(ni, ci));
                }
            },
            // `x <op>= e` は `x = x <op> e` と同じ命令列になる（`StoreLocal` は deep_copy しない）ので、
            // `Expr::BinOp` と同じ融合＋型特化を通す（#2b）。通さないと複合代入だけが
            // `LoadLocal; <e>; Bin; StoreLocal` の 4 命令＋汎用ディスパッチに落ちていた（実測 1.9x 遅い）。
            Stmt::CompoundAssign {
                name,
                op,
                value,
                node_id,
                ..
            } => {
                use crate::type_check::BinOperandKind as K;
                match self.store_target(name)? {
                    StoreTarget::Local(slot) => {
                        let kind = self.specialized_bin_kind_slot(op, *node_id, slot, value);
                        if !self.emit_bin_fused_slot(slot, kind, value, op) {
                            // 融合できない右辺（属性・添字・呼び出し結果など）でもスタック版の型特化には乗る。
                            self.emit(Op::LoadLocal(slot));
                            self.compile_expr(value)?;
                            match kind {
                                Some(K::Int) => self.emit(Op::IntBinSS(op.clone())),
                                Some(K::Float) => self.emit(Op::FloatBinSS(op.clone())),
                                None => self.emit(Op::Bin(op.clone())),
                            };
                        }
                        self.emit(Op::StoreLocal(slot));
                    }
                    // 最上位のグローバルへの複合代入（#10-b）。`x = x <op> e` と同じ命令列。
                    // 融合 op（`BinLocalLocal` 等）は slot 前提なので使えないが、注釈由来の
                    // スタック版型特化（`IntBinSS`/`FloatBinSS`）はそのまま乗る（#2b と同じ扱い）。
                    StoreTarget::Global(ni, ci) => {
                        let kind = self.annot_binop_kind(*node_id);
                        self.emit_load_global(name);
                        self.compile_expr(value)?;
                        match kind {
                            Some(K::Int) => self.emit(Op::IntBinSS(op.clone())),
                            Some(K::Float) => self.emit(Op::FloatBinSS(op.clone())),
                            None => self.emit(Op::Bin(op.clone())),
                        };
                        self.emit(Op::StoreGlobal(ni, ci));
                    }
                }
            }
            Stmt::If { branches, else_body } => {
                // 各分岐: cond, JumpIfFalse(next), body, Jump(end); next: ...
                let mut end_jumps: Vec<usize> = Vec::new();
                for (cond, body) in branches {
                    self.compile_expr(cond)?;
                    let jf = self.emit(Op::JumpIfFalse(0));
                    for s in body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
                if let Some(body) = else_body {
                    for s in body {
                        self.compile_stmt(s)?;
                    }
                }
                let end = self.here();
                for j in end_jumps {
                    self.patch_jump(j, end);
                }
            }
            Stmt::While { cond, body } => {
                let start = self.here();
                self.compile_expr(cond)?;
                let jf = self.emit(Op::JumpIfFalse(0));
                // ループコンテキストを積む: continue はここ（条件先頭）へ戻る。
                self.loops.push(LoopCtx {
                    continue_target: start,
                    break_jumps: Vec::new(),
                });
                for s in body {
                    self.compile_stmt(s)?;
                }
                self.emit(Op::Jump(start));
                let end = self.here();
                self.patch_jump(jf, end);
                // break はループ末尾（end）へバックパッチ。
                let ctx = self.loops.pop().unwrap();
                for j in ctx.break_jumps {
                    self.patch_jump(j, end);
                }
            }
            Stmt::Match { subject, arms, .. } => {
                self.compile_match(subject, arms)?;
            }
            Stmt::For { targets, iter, body } => {
                // 単一ターゲットのみ対応（タプルアンパックは非対応 → bail）。
                if targets.len() != 1 {
                    bail("for-tuple-target", None);
                    return None;
                }
                let target_slot = *self.slots.get(&targets[0])?;
                // イテレータを取得して temp slot に格納。
                let iter_temp = self.alloc_temp()?;
                self.compile_expr(iter)?;
                self.emit(Op::GetIter);
                self.emit(Op::StoreLocal(iter_temp));
                // loop_start: ForIter で next。EndOfIteration なら exit へ、要素なら target へ束縛。
                let loop_start = self.here();
                let fi = self.emit(Op::ForIter(iter_temp, target_slot, 0)); // exit は後でパッチ
                self.loops.push(LoopCtx {
                    continue_target: loop_start, // continue は次の ForIter へ戻る
                    break_jumps: Vec::new(),
                });
                for s in body {
                    self.compile_stmt(s)?;
                }
                self.emit(Op::Jump(loop_start));
                let exit = self.here();
                // ForIter の exit_ip をバックパッチ（patch_jump は Jump 系専用なので手動）。
                self.code[fi] = Op::ForIter(iter_temp, target_slot, exit);
                let ctx = self.loops.pop().unwrap();
                for j in ctx.break_jumps {
                    self.patch_jump(j, exit);
                }
                self.free_temp();
            }
            Stmt::Break => {
                // 最内ループの break_jumps に登録し、末尾へジャンプ（バックパッチ）。
                let j = self.emit(Op::Jump(0));
                self.loops.last_mut()?.break_jumps.push(j);
            }
            Stmt::Continue => {
                let target = self.loops.last()?.continue_target;
                self.emit(Op::Jump(target));
            }
            // ── ローカル宣言（exec_let / exec の const・mut と同一セマンティクス） ──
            // 最上位モード（#10-c）では slot ではなくグローバルへ宣言する（`DeclareGlobal`）。
            Stmt::Const(name, _, e) => {
                self.compile_expr(e)?;
                if name == "_" {
                    self.emit(Op::Pop);
                } else if let Some(ni) = self.toplevel_decl_name(name) {
                    self.emit(Op::DeclareGlobal(ni, DeclKind::Const));
                } else {
                    let slot = *self.slots.get(name)?;
                    self.emit(Op::StoreLocal(slot)); // const は copy/freeze しない
                }
            }
            Stmt::Mut(name, _, e) => {
                self.compile_expr(e)?;
                if name == "_" {
                    self.emit(Op::Pop);
                } else if let Some(ni) = self.toplevel_decl_name(name) {
                    self.emit(Op::DeclareGlobal(ni, DeclKind::Mut));
                } else {
                    let slot = *self.slots.get(name)?;
                    self.emit(Op::StoreLocalDeepCopy(slot)); // mut は常に deep_copy
                }
            }
            Stmt::Let(name, _, e) if self.toplevel_decl_name(name).is_some() && name != "_" => {
                // 最上位の `let`（#10-c）。ソースが識別子のときは**その変数の可変性**が要るが、
                // 最上位ではコンパイル時に分からない（`toplevel_globals` は名前の集合だけ）。
                // `exec_let` の「mut ソースなら copy+freeze」分岐を再現できないので bail する。
                let kind = match e {
                    Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Str(_) | Expr::None => {
                        DeclKind::LetPlain
                    }
                    Expr::Ident { .. } => {
                        bail("toplevel-let-from-ident", None);
                        return None;
                    }
                    _ => DeclKind::LetFreezeInstance,
                };
                let ni = self.toplevel_decl_name(name)?;
                self.compile_expr(e)?;
                self.emit(Op::DeclareGlobal(ni, kind));
            }
            Stmt::Let(name, _, e) => {
                if name == "_" {
                    self.compile_expr(e)?;
                    self.emit(Op::Pop);
                } else {
                    let slot = *self.slots.get(name)?;
                    // ソースの種類で store op を選ぶ（exec_let のセマンティクスに一致）。
                    let store = match e {
                        // ident/localref ソース: 可変なら copy+freeze、不変ならそのまま。
                        Expr::Ident { res: Resolution::Local(s), .. } => {
                            if self.slot_mut.get(*s as usize).copied().unwrap_or(false) {
                                Op::StoreLocalCopyFreeze(slot)
                            } else {
                                Op::StoreLocal(slot)
                            }
                        }
                        Expr::Ident { name: nm, res: Resolution::Unresolved, .. } => {
                            let s = *self.slots.get(nm)?; // base slot 以外（グローバル）は非対応
                            if self.slot_mut.get(s as usize).copied().unwrap_or(false) {
                                Op::StoreLocalCopyFreeze(slot)
                            } else {
                                Op::StoreLocal(slot)
                            }
                        }
                        // リテラル（プリミティブ）は freeze 不要。
                        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Str(_)
                        | Expr::None => Op::StoreLocal(slot),
                        // 非識別子式: Instance のときのみ copy+freeze（exec_let 非 ident 分岐）。
                        _ => Op::StoreLocalFreezeInstance(slot),
                    };
                    self.compile_expr(e)?;
                    self.emit(store);
                }
            }
            // 属性代入 `obj.attr = value` / 添字代入 `obj[i] = value`。
            Stmt::AttrAssign { target, value } => match target {
                Expr::Attr { object, attr, .. } if self.object_is_instance(object) => {
                    // obj（SetAttr のベース）を push → value を push → SetAttr。
                    // object は side-effect-free（self/base ローカル）なので先に push してよい。
                    self.compile_expr(object)?;
                    self.compile_expr(value)?;
                    let ni = self.add_name(attr);
                    self.emit(Op::SetAttr(ni));
                }
                // `obj[i] = value` — tree-walk は value(rhs) を先に評価するので temp に退避して順序を合わせる。
                Expr::Subscript { object, index, .. } => {
                    let vtmp = self.alloc_temp()?;
                    self.compile_expr(value)?; // value を先に評価
                    self.emit(Op::StoreLocal(vtmp));
                    self.compile_expr(object)?; // obj
                    self.compile_expr(index)?; // key
                    self.emit(Op::LoadLocal(vtmp)); // value
                    self.emit(Op::SetIndex);
                    self.free_temp();
                }
                // `obj::Trait.attr = value`（#27）。`SetAttr` と同じく `[obj, value]` の順で積む。
                Expr::TraitAccess { object, trait_name, attr } => {
                    self.compile_expr(object)?;
                    self.compile_expr(value)?;
                    let ti = self.add_name(trait_name);
                    let ai = self.add_name(attr);
                    self.emit(Op::SetTraitAttr(ti, ai));
                }
                // 非 instance 属性は非対応。
                other => {
                    bail_expr("assign-target", other);
                    return None;
                }
            },
            // 属性複合代入 `obj.attr op= value`（obj が `self`/instance のときのみ）。
            Stmt::AttrCompoundAssign { target, op, value } => {
                let (object, attr) = match target {
                    Expr::Attr { object, attr, .. } if self.object_is_instance(object) => {
                        (object, attr)
                    }
                    other => {
                        bail_expr("attr-compound-target", other);
                        return None;
                    }
                };
                let ni = self.add_name(attr);
                // 型特化（#2b）: フィールドの型は注釈テーブルが `Expr::Attr` の node_id に焼いている。
                // 右辺は `expr_prim`（リテラル / 型注釈つき局所変数）で見る。
                let kind = match target {
                    Expr::Attr { node_id, .. } => self
                        .annot_prim(*node_id)
                        .zip(self.expr_prim(value))
                        .and_then(|(l, r)| Self::pair_kind(l, r))
                        .and_then(|k| Self::gate_bin_kind(k, op)),
                    _ => None,
                };
                // `object_is_instance` が通った時点でレシーバは局所 slot（`self` か instance 型の
                // ローカル）と確定している。`as_local` は debug_mode でだけ `None` を返す。
                let obj_slot = self.as_local(object);
                self.compile_expr(object)?; // SetAttr のベース（ここは常に必要）

                // 評価順（#2a）。ツリーウォークは **value を先に評価してから**現在値を読むので、
                // 素直に組むと [value, cur] の順にスタックへ乗り `Swap` が要る。
                // ただし value が**副作用を持たない**（局所変数読み or 定数リテラル）なら、
                // 先に現在値を読んでも観測結果は同じなので `Swap` を丸ごと落とせる。
                // レシーバ slot が value の評価中に再束縛されないことは `CallMethodLocal` と同じ根拠
                // （再束縛は文＝`StoreLocal` でしか起きず、クロージャ捕捉は VM 非対応で bail する）。
                //
                // 現在値の読み出しは、レシーバが局所 slot なら `LoadLocal; GetAttr` の 2 命令を
                // `GetAttrLocal` 1 命令へ畳む（レシーバを **clone せず frame から参照で読む**ので
                // `Rc` の refcount 増減も消える）。`Expr::Attr` の compile と同じ融合。
                let value_pure =
                    self.as_local(value).is_some() || Self::as_const_lit(value).is_some();
                if !value_pure {
                    self.compile_expr(value)?; // rhs を先に評価（順序保存）
                }
                match obj_slot {
                    Some(s) => self.emit(Op::GetAttrLocal(s, ni, ni)),
                    None => {
                        // `debug_mode` では `as_local` が常に `None`。従来どおり 2 命令で組む。
                        self.compile_expr(object)?;
                        self.emit(Op::GetAttr(ni, ni))
                    }
                };
                if value_pure {
                    // [obj, cur, value] → Bin → [obj, new]（Swap 不要）
                    self.compile_expr(value)?;
                } else {
                    // [obj, value, cur] → Swap → [obj, cur, value] → Bin → [obj, new]
                    self.emit(Op::Swap);
                }
                match kind {
                    Some(crate::type_check::BinOperandKind::Int) => {
                        self.emit(Op::IntBinSS(op.clone()))
                    }
                    Some(crate::type_check::BinOperandKind::Float) => {
                        self.emit(Op::FloatBinSS(op.clone()))
                    }
                    None => self.emit(Op::Bin(op.clone())),
                };
                self.emit(Op::SetAttr(ni));
            }
            Stmt::Raise { exc, span } => match exc {
                Some(e) => {
                    self.compile_expr(e)?;
                    let si = self.add_span(span);
                    self.emit(Op::Raise(si));
                }
                None => {
                    self.emit(Op::Reraise); // bare raise（再送出）
                }
            },
            Stmt::Try { body, handlers, finally_body } => {
                self.compile_try(body, handlers, finally_body)?;
            }
            // ブロック式内: block_return は最内ブロック式の result_slot へ格納して出口へ跳ぶ。
            Stmt::BlockReturn(e, _) => {
                let result_slot = self.block_ctxs.last()?.result_slot;
                self.compile_expr(e)?;
                self.emit(Op::StoreLocal(result_slot));
                let j = self.emit(Op::Jump(0));
                self.block_ctxs.last_mut().unwrap().end_jumps.push(j);
            }
            // loop_yield は最内の「yield 先を持つ」ブロック式（block:/for/while 式）の蓄積リストへ追加。
            // if/match 式は透過（yield_slot=None）なので飛ばして外側へ届く。
            Stmt::LoopYield(e) => {
                let yield_slot = self.block_ctxs.iter().rev().find_map(|c| c.yield_slot)?;
                self.compile_expr(e)?;
                self.emit(Op::ListAppendLocal(yield_slot));
            }
            // ジェネレータ本体の `yield expr`（タスク #8）。値を評価して yield 収集バッファへ産出する。
            // eager 収集なので制御は継続（ツリーウォークの `Stmt::Yield` と同一）。
            Stmt::Yield(e) => {
                self.compile_expr(e)?;
                self.emit(Op::Yield);
            }
            // `target <- async->T: body`（タスク #9）。AsyncManager にタスクを投入する。
            Stmt::AsyncAssign { target, stmts, .. } => {
                self.compile_async_assign(target, stmts)?;
            }
            // `pass` は何も出さない（#27）。ツリーウォークも `ExecResult::Normal` を返すだけ。
            // 文境界の予約（`mark_stmt_start`）は入口で済んでいるので、
            // 命令を 1 つも出さなくてもデバッガの停止位置はずれない。
            Stmt::Pass => {}
            // `break_point`（#27）: デバッガ REPL へ入る。ツリーウォークと同じ `exec_breakpoint`。
            Stmt::BreakPoint { span } => {
                let si = self.add_span(span);
                self.emit(Op::BreakPoint(si));
            }
            // それ以外（定義・import 等）は非対応。
            _ => {
                bail("stmt", Some(stmt));
                return None;
            }
        }
        Some(())
    }

    /// `try/except`（finally なし）と `try/finally`（except なし）をコンパイルする。
    /// 両方揃う `try/except/finally` は現状 bail（finally とハンドラの相互作用が複雑なため）。
    fn compile_try(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        finally_body: &Option<Vec<Stmt>>,
    ) -> Option<()> {
        match finally_body {
            None => self.compile_try_except(body, handlers),
            Some(fin) if handlers.is_empty() => self.compile_try_finally(body, fin),
            Some(_) => None, // try/except/finally 併用は非対応
        }
    }

    /// `try: <body> except ...:` をハンドラスタック（SetupTry/PopTry）＋ landing pad にコンパイルする。
    fn compile_try_except(&mut self, body: &[Stmt], handlers: &[ExceptHandler]) -> Option<()> {
        // try を飛び越える制御フロー（break/continue/block_return/loop_yield）があると
        // SetupTry ハンドラが残るため bail。return は run から即復帰しハンドラは破棄されるので OK。
        if has_escape(body, false, 0) {
            bail("try-escape", None);
            return None;
        }
        for h in handlers {
            if has_escape(&h.body, false, 0) {
                bail("try-handler-escape", None);
                return None;
            }
        }

        let setup = self.emit(Op::SetupTry(0)); // handler_ip は後でパッチ
        for s in body {
            self.compile_stmt(s)?;
        }
        self.emit(Op::PopTry);
        let mut end_jumps = vec![self.emit(Op::Jump(0))]; // 正常終了 → END
        // landing pad: 例外時にここへ来る（スタック = [exc]）。
        let land = self.here();
        self.code[setup] = Op::SetupTry(land);
        for h in handlers {
            match &h.exc_type {
                // bare `except:` — 無条件マッチ。
                None => {
                    self.bind_or_pop_exc(h)?;
                    for s in &h.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    // bare except 以降のハンドラは到達不能（パーサが末尾を保証）。
                }
                // `except E [as e]:` — 型マッチ。
                Some(type_name) => {
                    self.emit(Op::Dup); // [exc, exc]
                    let ni = self.add_name(type_name);
                    self.emit(Op::ExcMatch(ni)); // [exc, bool]
                    let jf = self.emit(Op::JumpIfFalse(0)); // bool を pop・不一致は next へ
                    self.bind_or_pop_exc(h)?; // 一致: [exc] を束縛 or 破棄
                    for s in &h.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
            }
        }
        // どのハンドラにもマッチしなかった: [exc] を捨てて再送出。
        self.emit(Op::Pop);
        self.emit(Op::Reraise);
        let end = self.here();
        for j in end_jumps {
            self.patch_jump(j, end);
        }
        Some(())
    }

    /// `try: <body> finally: <fin>`（except なし）。正常経路・例外経路の両方で finally を走らせる。
    fn compile_try_finally(&mut self, body: &[Stmt], fin: &[Stmt]) -> Option<()> {
        // finally は全出口で走る必要があるので、脱出制御フロー（return 含む）があれば bail。
        if has_escape(body, true, 0) || has_escape(fin, true, 0) {
            bail("finally-escape", None);
            return None;
        }
        let setup = self.emit(Op::SetupTry(0));
        for s in body {
            self.compile_stmt(s)?;
        }
        self.emit(Op::PopTry);
        // 正常経路の finally。
        for s in fin {
            self.compile_stmt(s)?;
        }
        let normal_jump = self.emit(Op::Jump(0)); // END
        // 例外 landing pad: スタック = [exc]。finally はスタック中立なので [exc] は底に残る。
        let land = self.here();
        self.code[setup] = Op::SetupTry(land);
        for s in fin {
            self.compile_stmt(s)?;
        }
        self.emit(Op::Pop); // [exc] を捨てる
        self.emit(Op::Reraise); // 再伝播（current_exception は設定済み）
        let end = self.here();
        self.patch_jump(normal_jump, end);
        Some(())
    }

    /// except ハンドラ landing の [exc] を、別名があれば slot へ束縛、なければ捨てる。
    fn bind_or_pop_exc(&mut self, h: &ExceptHandler) -> Option<()> {
        if let Some(alias) = &h.name {
            let slot = *self.slots.get(alias)?;
            self.emit(Op::StoreLocal(slot)); // exc を束縛（消費）
        } else {
            self.emit(Op::Pop); // exc を破棄
        }
        Some(())
    }

    /// `match` 文を temp slot + ジャンプ列にコンパイルする（`exec_match_stmt` と同一意味論）。
    /// サブジェクトを一度だけ評価して temp に格納し、各アームを順に照合する。
    fn compile_match(&mut self, subject: &Expr, arms: &[MatchArm]) -> Option<()> {
        // サブジェクトを一度評価して temp に退避（各アームの照合で使い回す）。
        let temp = self.alloc_temp()?;
        self.compile_expr(subject)?;
        self.emit(Op::StoreLocal(temp));

        let mut end_jumps: Vec<usize> = Vec::new();
        for arm in arms {
            match &arm.pattern {
                // ワイルドカード `case _:` は無条件マッチ。
                MatchPattern::Case(Expr::Ident { name: n, .. }) if n == "_" => {
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    // 以降のアームは到達不能だが害はない（emit を続けても正しさは保たれる）。
                }
                MatchPattern::Case(pattern_expr) => {
                    self.emit(Op::LoadLocal(temp));
                    self.compile_expr(pattern_expr)?;
                    self.emit(Op::Bin(BinOp::Eq)); // subject == pattern（apply_binop_dyn 委譲）
                    let jf = self.emit(Op::JumpIfFalse(0));
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
                MatchPattern::IsType(type_name) => {
                    self.emit(Op::LoadLocal(temp));
                    let ni = self.add_name(type_name);
                    self.emit(Op::IsType(ni));
                    let jf = self.emit(Op::JumpIfFalse(0));
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
            }
        }
        let end = self.here();
        for j in end_jumps {
            self.patch_jump(j, end);
        }
        self.free_temp();
        Some(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> Option<()> {
        match expr {
            Expr::Int(n) => {
                let c = self.add_const(Value::Int(*n));
                self.emit(Op::Const(c));
            }
            Expr::Float(f) => {
                let c = self.add_const(Value::Float(*f));
                self.emit(Op::Const(c));
            }
            Expr::Bool(b) => {
                let c = self.add_const(Value::Bool(*b));
                self.emit(Op::Const(c));
            }
            Expr::Str(s) => {
                let c = self.add_const(Value::Str(s.clone()));
                self.emit(Op::Const(c));
            }
            Expr::None => {
                self.emit(Op::Nil);
            }
            // `obj::Trait.attr` の読み（#27）。レシーバの種別はコンパイル時に保証せず、
            // `trait_access_evaled` が実行時に検査する（ツリーウォークと同じ 1 実装）。
            Expr::TraitAccess { object, trait_name, attr } => {
                self.compile_expr(object)?;
                let ti = self.add_name(trait_name);
                let ai = self.add_name(attr);
                self.emit(Op::GetTraitAttr(ti, ai));
            }
            // `undefined` リテラル（#27）。`eval` の `Expr::Undefined => Value::Undefined` と同じ。
            Expr::Undefined => {
                let ci = self.add_const(Value::Undefined);
                self.emit(Op::Const(ci));
            }
            Expr::Ident { res: Resolution::Local(slot), .. } => {
                let s = u16::try_from(*slot).ok()?;
                self.emit(Op::LoadLocal(s));
            }
            // 解決済みグローバル参照（R2-b）。リゾルバが「最上位宣言かつ非シャドウ」と
            // 確定した読み取りなので、slots 走査も builtin 判定も要らず直接 LoadGlobal。
            Expr::Ident { name, res: Resolution::Global(_), .. } => {
                self.emit_load_global(name);
            }
            // メソッド本体の `Self` を値として読む（#27）。呼び出し以外の位置でも使える。
            Expr::Ident { name, .. }
                if name == "Self" && self.self_slot.is_some() && !self.slots.contains_key(name) =>
            {
                self.emit(Op::LoadSelfClass);
            }
            // 未解決 Ident はパラメータ名のときのみローカル読み（それ以外＝グローバル/組み込みは非対応）。
            // デバッグモードでは停止スコープからの名前引き（LoadName）。
            Expr::Ident { name, .. } => {
                if self.debug_mode {
                    let ni = self.add_name(name);
                    self.emit(Op::LoadName(ni));
                } else {
                    let slot = *self.slots.get(name)?;
                    self.emit(Op::LoadLocal(slot));
                }
            }
            Expr::UnaryOp { op, operand } => {
                self.compile_expr(operand)?;
                self.emit(Op::Un(op.clone()));
            }
            Expr::BinOp { op, left, right, node_id, .. } => match op {
                // 短絡評価: `a and b` / `a or b` は Python 意味論（値を返す）で書き下す。
                BinOp::And => {
                    self.compile_expr(left)?;
                    let j = self.emit(Op::JumpIfFalseOrPop(0));
                    self.compile_expr(right)?;
                    let end = self.here();
                    self.patch_jump(j, end);
                }
                BinOp::Or => {
                    self.compile_expr(left)?;
                    let j = self.emit(Op::JumpIfTrueOrPop(0));
                    self.compile_expr(right)?;
                    let end = self.here();
                    self.patch_jump(j, end);
                }
                _ => {
                    // 超命令融合（#2）＋型特化（plan A）: 単純オペランドなら LoadLocal…+Bin を1命令に。
                    if !self.try_emit_bin_fused(left, right, op, *node_id) {
                        use crate::type_check::BinOperandKind as K;
                        self.compile_expr(left)?;
                        self.compile_expr(right)?;
                        // 融合できない形（属性・添字・呼び出し結果など）でも、注釈が型を確定して
                        // いればスタック版の型特化 op に落とす（#16 段階(b)(iii)）。
                        match self.specialized_bin_kind(op, *node_id, left, right) {
                            Some(K::Int) => self.emit(Op::IntBinSS(op.clone())),
                            Some(K::Float) => self.emit(Op::FloatBinSS(op.clone())),
                            None => self.emit(Op::Bin(op.clone())),
                        };
                    }
                }
            },
            Expr::Attr { object, attr, .. } => {
                let name_idx = self.add_name(attr);
                // 超命令融合（#16 段階(b)(i)）: レシーバが局所変数なら frame から参照読みする
                // 専用 op に落とし、`Value` clone（Rc refcount 増減）と push/pop を省く。
                if let Some(slot) = self.as_local(object) {
                    self.emit(Op::GetAttrLocal(slot, name_idx, name_idx));
                } else {
                    self.compile_expr(object)?;
                    self.emit(Op::GetAttr(name_idx, name_idx));
                }
            }
            // 関数呼び出し `func(args)` / メソッド呼び出し `obj.method(args)`。
            Expr::Call { func, args, span, node_id, .. } => {
                if let Expr::Attr { object, attr, .. } = func.as_ref() {
                    // ── メソッド呼び出し ── object が Instance と保証できる（`self` または
                    // ユーザークラス型注釈の）識別子のときのみ対応。
                    if !self.object_is_instance(object) {
                        // レシーバが Instance と保証できない（グローバル受信者はここに来る）。
                        bail_expr("method-receiver", object);
                        return None;
                    }
                    // 超命令融合（#16 段階(b)(i)）: レシーバが局所変数なら push せず
                    // frame から直接読む op に落とす（属性読みの `GetAttrLocal` と同じ手）。
                    if let Some(slot) = self.as_local(object) {
                        let mask = self.compile_call_args(args)?;
                        let ni = self.add_name(attr);
                        self.emit(Op::CallMethodLocal(slot, ni, args.len() as u16, mask));
                    } else {
                        self.compile_expr(object)?; // receiver を push
                        let mask = self.compile_call_args(args)?;
                        let ni = self.add_name(attr);
                        self.emit(Op::CallMethod(ni, args.len() as u16, mask));
                    }
                    return Some(()); // メソッド呼び出しは span 不要
                }
                let site = self.add_span(span); // 関数呼び出しはトレースバック用の呼び出し位置を記録
                if let Expr::Ident { name, res: Resolution::Unresolved, .. } = func.as_ref() {
                    // ── VM 対応組み込み（print/range/len）── 評価済み引数で直接呼ぶ。
                    // ローカル slot に同名（シャドウ）がなければ組み込みとして扱う。
                    if is_vm_builtin(name) && !self.slots.contains_key(name) {
                        self.compile_call_args(args)?; // 組み込みは mut_mask 不要
                        let ni = self.add_name(name);
                        self.emit(Op::CallBuiltin(ni, args.len() as u16));
                    } else if self.debug_mode {
                        // デバッグモード: 呼び先を名前引きで取得（局所・グローバル両対応）。
                        let cn = self.add_name(name);
                        self.emit(Op::LoadName(cn));
                        let mask = self.compile_call_args(args)?;
                        self.emit(Op::Call(args.len() as u16, mask, cn, site, *node_id));
                    } else if let Some(&slot) = self.slots.get(name) {
                        // ローカル/パラメータが関数値を保持している場合は slot 読み。
                        self.emit(Op::LoadLocal(slot));
                        let mask = self.compile_call_args(args)?;
                        let ni = self.add_name(name);
                        self.emit(Op::Call(args.len() as u16, mask, ni, site, *node_id));
                    } else if name == "Self" && self.self_slot.is_some() {
                        // メソッド本体の `Self(...)`（#27）: レシーバのクラスを積んで通常の
                        // `Call` へ流す（`call_value_evaled` の `Value::Class` アーム＝
                        // ツリーウォークと同一のインスタンス化経路）。
                        self.emit(Op::LoadSelfClass);
                        let mask = self.compile_call_args(args)?;
                        let ni = self.add_name(name);
                        self.emit(Op::Call(args.len() as u16, mask, ni, site, *node_id));
                    } else if is_builtin_callee(name) || name == "Self" {
                        // その他の純粋 builtin・型コンストラクタ・`Self` は非対応。
                        // 呼び先名まで記録する（どれを `eval_builtin_evaled` へ足せば効くかを測るため・#27）。
                        if crate::interpreter::tw_stats::enabled() {
                            crate::interpreter::tw_stats::record_bail("callee-builtin", name);
                        }
                        return None;
                    } else {
                        // グローバル関数呼び出し（#11: 索引キャッシュ付き LoadGlobal）。
                        let ni = self.add_name(name);
                        let ci = self.global_caches.len() as u32;
                        self.global_caches.push(crate::ast::SlotCache::default());
                        self.emit(Op::LoadGlobal(ni, ci));
                        let mask = self.compile_call_args(args)?;
                        self.emit(Op::Call(args.len() as u16, mask, ni, site, *node_id));
                    }
                } else if let Expr::Ident { name, res: Resolution::Global(_), .. } = func.as_ref() {
                    // 解決済みグローバル関数呼び出し（R2-b）。
                    // 分類はリゾルバ済みなので builtin/slots の判定は不要。
                    // ただしデバッグモードは停止スコープの名前引きに合わせる。
                    let ni = self.add_name(name);
                    if self.debug_mode {
                        self.emit(Op::LoadName(ni));
                    } else {
                        self.emit_load_global(name);
                    }
                    let mask = self.compile_call_args(args)?;
                    self.emit(Op::Call(args.len() as u16, mask, ni, site, *node_id));
                } else if let Expr::Ident { name, res: Resolution::Local(slot), .. } = func.as_ref() {
                    // 解決済みローカル関数値の呼び出し。
                    let s = u16::try_from(*slot).ok()?;
                    self.emit(Op::LoadLocal(s));
                    let mask = self.compile_call_args(args)?;
                    let ni = self.add_name(name);
                    self.emit(Op::Call(args.len() as u16, mask, ni, site, *node_id));
                } else {
                    // その他の呼び先式（添字結果など）は非対応。
                    bail_expr("callee-expr", func);
                    return None;
                }
            }
            // ── 添字・コレクションリテラル（タスク #5） ──
            Expr::Subscript { object, index, .. } => {
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                self.emit(Op::Subscript);
            }
            Expr::List(items) => {
                let n = u16::try_from(items.len()).ok()?;
                for it in items {
                    self.compile_expr(it)?;
                }
                self.emit(Op::BuildList(n));
            }
            Expr::Tuple(items) => {
                let n = u16::try_from(items.len()).ok()?;
                for it in items {
                    self.compile_expr(it)?;
                }
                self.emit(Op::BuildTuple(n));
            }
            Expr::Set(items) => {
                let n = u16::try_from(items.len()).ok()?;
                for it in items {
                    self.compile_expr(it)?;
                }
                self.emit(Op::BuildSet(n));
            }
            Expr::Dict(pairs) => {
                let n = u16::try_from(pairs.len()).ok()?;
                for (k, v) in pairs {
                    self.compile_expr(k)?;
                    self.compile_expr(v)?;
                }
                self.emit(Op::BuildDict(n));
            }
            // ── ブロック式（値を産む制御構文, Phase V-C） ──
            Expr::Block { stmts, .. } => self.compile_block_expr(stmts)?,
            Expr::IfExpr { branches, else_body, .. } => {
                self.compile_if_expr(branches, else_body)?
            }
            Expr::MatchExpr { subject, arms, .. } => self.compile_match_expr(subject, arms)?,
            Expr::ForExpr { target, iter, body, .. } => {
                self.compile_for_expr(target, iter, body)?
            }
            Expr::WhileExpr { cond, body, .. } => self.compile_while_expr(cond, body)?,

            // ── 動的型検査（#16 段階(b)(ii)）──
            // 型検査が付けた `CheckBefore` 指示を消費して検査 op を出す。
            // 指示が無い（＝未採番ノード等）場合も**保守的に検査を出す**: 検査を落とす方向へは倒さない。
            Expr::IsType { expr, negated, type_name, .. } => {
                self.compile_expr(expr)?;
                let ni = self.add_name(type_name);
                self.emit(Op::IsType(ni));
                if *negated {
                    // `Op::Un(Not)` は Bool に対し `!b` を返す（eval の negated 分岐と同一）。
                    self.emit(Op::Un(crate::ast::UnaryOp::Not));
                }
            }
            Expr::MustBe { expr, guard_type, span, node_id } => {
                if !self.check_required(*node_id) {
                    bail("check-required-mustbe", None);
                    return None;
                }
                self.compile_expr(expr)?;
                let ni = self.add_name(guard_type);
                let si = self.add_span(span);
                self.emit(Op::MustBe(ni, si));
            }
            Expr::Cast { object, type_name, node_id, .. } => {
                if !self.check_required(*node_id) {
                    bail("check-required-cast", None);
                    return None;
                }
                self.compile_expr(object)?;
                let ni = self.add_name(type_name);
                self.emit(Op::Cast(ni));
            }

            // それ以外は非対応。
            _ => {
                bail_expr("expr", expr);
                return None;
            }
        }
        Some(())
    }

    /// `block: <stmts>` 式。block_return 値、なければ loop_yield 蓄積リスト、どちらもなければ None。
    fn compile_block_expr(&mut self, stmts: &[Stmt]) -> Option<()> {
        if block_body_bails(stmts, false, 0) {
            return None;
        }
        let yield_slot = self.alloc_temp()?;
        self.emit(Op::BuildEmptyList);
        self.emit(Op::StoreLocal(yield_slot));
        let result_slot = self.alloc_temp()?;
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot: Some(yield_slot),
        });
        for s in stmts {
            self.compile_stmt(s)?;
        }
        let ctx = self.block_ctxs.pop().unwrap();
        // 正常フォールスルー: 値 = 蓄積リスト or None。
        self.emit(Op::LoadLocal(yield_slot));
        self.emit(Op::ListOrNone);
        let after_normal = self.emit(Op::Jump(0)); // → EXPR_END
        // block_return 出口: 値 = result_slot。
        let br_end = self.here();
        for j in ctx.end_jumps {
            self.patch_jump(j, br_end);
        }
        self.emit(Op::LoadLocal(result_slot));
        let expr_end = self.here();
        self.patch_jump(after_normal, expr_end);
        self.free_temp(); // result_slot
        self.free_temp(); // yield_slot
        Some(())
    }

    /// `if cond -> T: ... [elif][else]` 式。マッチした分岐の block_return 値、なければ None。
    /// yield に対しては透過（yield_slot=None）。
    fn compile_if_expr(
        &mut self,
        branches: &[(Expr, Vec<Stmt>)],
        else_body: &Option<Vec<Stmt>>,
    ) -> Option<()> {
        for (_, b) in branches {
            if block_body_bails(b, false, 0) {
                return None;
            }
        }
        if let Some(eb) = else_body {
            if block_body_bails(eb, false, 0) {
                return None;
            }
        }
        let result_slot = self.alloc_temp()?;
        self.emit(Op::Nil);
        self.emit(Op::StoreLocal(result_slot)); // 既定 None
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot: None,
        });
        let mut branch_ends: Vec<usize> = Vec::new();
        for (cond, body) in branches {
            self.compile_expr(cond)?;
            let jf = self.emit(Op::JumpIfFalse(0));
            for s in body {
                self.compile_stmt(s)?;
            }
            branch_ends.push(self.emit(Op::Jump(0)));
            let next = self.here();
            self.patch_jump(jf, next);
        }
        if let Some(eb) = else_body {
            for s in eb {
                self.compile_stmt(s)?;
            }
        }
        let ctx = self.block_ctxs.pop().unwrap();
        let end = self.here();
        for j in branch_ends {
            self.patch_jump(j, end);
        }
        for j in ctx.end_jumps {
            self.patch_jump(j, end);
        }
        self.emit(Op::LoadLocal(result_slot)); // block_return 値 or None
        self.free_temp();
        Some(())
    }

    /// `match subj -> T: arms` 式。マッチしたアームの block_return 値、なければ None。
    fn compile_match_expr(&mut self, subject: &Expr, arms: &[MatchArm]) -> Option<()> {
        for arm in arms {
            if block_body_bails(&arm.body, false, 0) {
                return None;
            }
        }
        let subj_temp = self.alloc_temp()?;
        self.compile_expr(subject)?;
        self.emit(Op::StoreLocal(subj_temp));
        let result_slot = self.alloc_temp()?;
        self.emit(Op::Nil);
        self.emit(Op::StoreLocal(result_slot));
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot: None,
        });
        let mut arm_ends: Vec<usize> = Vec::new();
        for arm in arms {
            match &arm.pattern {
                MatchPattern::Case(Expr::Ident { name: n, .. }) if n == "_" => {
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    arm_ends.push(self.emit(Op::Jump(0)));
                }
                MatchPattern::Case(pat) => {
                    self.emit(Op::LoadLocal(subj_temp));
                    self.compile_expr(pat)?;
                    self.emit(Op::Bin(BinOp::Eq));
                    let jf = self.emit(Op::JumpIfFalse(0));
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    arm_ends.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
                MatchPattern::IsType(type_name) => {
                    self.emit(Op::LoadLocal(subj_temp));
                    let ni = self.add_name(type_name);
                    self.emit(Op::IsType(ni));
                    let jf = self.emit(Op::JumpIfFalse(0));
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    arm_ends.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
            }
        }
        let ctx = self.block_ctxs.pop().unwrap();
        let end = self.here();
        for j in arm_ends {
            self.patch_jump(j, end);
        }
        for j in ctx.end_jumps {
            self.patch_jump(j, end);
        }
        self.emit(Op::LoadLocal(result_slot));
        self.free_temp(); // result_slot
        self.free_temp(); // subj_temp
        Some(())
    }

    /// `for target in iter -> T: body` 式。block_return 値、なければ loop_yield 蓄積リスト（空なら None）。
    fn compile_for_expr(&mut self, target: &str, iter: &Expr, body: &[Stmt]) -> Option<()> {
        if block_body_bails(body, true, 0) {
            return None;
        }
        let target_slot = *self.slots.get(target)?;
        let yield_slot = self.alloc_temp()?;
        self.emit(Op::BuildEmptyList);
        self.emit(Op::StoreLocal(yield_slot));
        let result_slot = self.alloc_temp()?;
        let iter_temp = self.alloc_temp()?;
        self.compile_expr(iter)?;
        self.emit(Op::GetIter);
        self.emit(Op::StoreLocal(iter_temp));
        let loop_start = self.here();
        let fi = self.emit(Op::ForIter(iter_temp, target_slot, 0)); // exit → NORMAL_END
        self.loops.push(LoopCtx {
            continue_target: loop_start,
            break_jumps: Vec::new(),
        });
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot: Some(yield_slot),
        });
        for s in body {
            self.compile_stmt(s)?;
        }
        self.emit(Op::Jump(loop_start));
        // NORMAL_END: 反復終了 or break → 蓄積リスト or None。
        let normal_end = self.here();
        self.code[fi] = Op::ForIter(iter_temp, target_slot, normal_end);
        let loop_ctx = self.loops.pop().unwrap();
        for j in loop_ctx.break_jumps {
            self.patch_jump(j, normal_end);
        }
        let block_ctx = self.block_ctxs.pop().unwrap();
        self.emit(Op::LoadLocal(yield_slot));
        self.emit(Op::ListOrNone);
        let after_normal = self.emit(Op::Jump(0)); // → EXPR_END
        // BR_END: block_return → result_slot。
        let br_end = self.here();
        for j in block_ctx.end_jumps {
            self.patch_jump(j, br_end);
        }
        self.emit(Op::LoadLocal(result_slot));
        let expr_end = self.here();
        self.patch_jump(after_normal, expr_end);
        self.free_temp(); // iter_temp
        self.free_temp(); // result_slot
        self.free_temp(); // yield_slot
        Some(())
    }

    /// `while cond -> T: body` 式。block_return 値、なければ loop_yield 蓄積リスト（空なら None）。
    fn compile_while_expr(&mut self, cond: &Expr, body: &[Stmt]) -> Option<()> {
        if block_body_bails(body, true, 0) {
            return None;
        }
        let yield_slot = self.alloc_temp()?;
        self.emit(Op::BuildEmptyList);
        self.emit(Op::StoreLocal(yield_slot));
        let result_slot = self.alloc_temp()?;
        let loop_start = self.here();
        self.compile_expr(cond)?;
        let jf = self.emit(Op::JumpIfFalse(0)); // false → NORMAL_END
        self.loops.push(LoopCtx {
            continue_target: loop_start,
            break_jumps: Vec::new(),
        });
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot: Some(yield_slot),
        });
        for s in body {
            self.compile_stmt(s)?;
        }
        self.emit(Op::Jump(loop_start));
        let normal_end = self.here();
        self.patch_jump(jf, normal_end);
        let loop_ctx = self.loops.pop().unwrap();
        for j in loop_ctx.break_jumps {
            self.patch_jump(j, normal_end);
        }
        let block_ctx = self.block_ctxs.pop().unwrap();
        self.emit(Op::LoadLocal(yield_slot));
        self.emit(Op::ListOrNone);
        let after_normal = self.emit(Op::Jump(0));
        let br_end = self.here();
        for j in block_ctx.end_jumps {
            self.patch_jump(j, br_end);
        }
        self.emit(Op::LoadLocal(result_slot));
        let expr_end = self.here();
        self.patch_jump(after_normal, expr_end);
        self.free_temp(); // result_slot
        self.free_temp(); // yield_slot
        Some(())
    }
}
