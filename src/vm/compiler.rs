// vm/compiler.rs — 解決済み AST → Chunk のコンパイラ（Phase V, V-A）。
//
// 保守的コンパイル: 対応できない構文に出会ったら `None` を返し、呼び出し側は
// ツリーウォークにフォールバックする（デュアルモード, D2）。
//
// V-A の対応範囲（トップレベル関数のリーフ計算に限定）:
// - 文: `return` / `if` / `while` / 式文 / パラメータへの代入・複合代入。
// - 式: リテラル / `LocalRef`（パラメータ読み）/ 二項・単項演算 / 属性（フィールド）読み。
// - **非対応（=フォールバック）**: ローカル宣言（let/mut/const の freeze 意味論を避けるため）、
//   関数・メソッド呼び出し、クロージャ、for/match/block、例外、可変長引数、
//   グローバル/組み込み参照、添字、コレクションリテラル 等。

use std::collections::{HashMap, HashSet};

use crate::ast::{
    BinOp, CallArg, ExceptHandler, Expr, MatchArm, MatchPattern, Param, Stmt, TupleTarget,
};
use crate::interpreter::Value;

use super::chunk::Chunk;
use super::op::Op;

/// VM の `Call` op で解決できない呼び先名（純粋 builtin・型コンストラクタ）。
/// これらは `eval_builtin_ident_call` で特別扱いされるか、グローバル `Value::Type` として
/// 別セマンティクスで呼ばれるため、コンパイル時に弾いてツリーウォークへフォールバックする。
/// VM 内で評価済み引数から直接呼べる組み込み（`eval_builtin_evaled` が扱う集合）。
/// `for x in range(n)` や `print(...)` を含む関数を VM に載せられるようにする。
fn is_vm_builtin(name: &str) -> bool {
    matches!(name, "print" | "range" | "len")
}

fn is_builtin_callee(name: &str) -> bool {
    matches!(
        name,
        // eval_builtin_ident_call の各アーム（グローバルに存在しない純粋 builtin）
        "print" | "next" | "repr" | "range" | "len" | "create_flat_int_list" | "flat_get_int"
            | "flat_set_int" | "id" | "open" | "close" | "enumerate" | "zip" | "getenv" | "parse_ar"
            // 型コンストラクタ（Value::Type グローバル・別経路）
            | "int" | "uint" | "str" | "float" | "complex" | "bool" | "dict" | "set" | "tuple"
            | "list" | "function" | "slice" | "type" | "byte"
    )
}

struct Compiler {
    code: Vec<Op>,
    consts: Vec<Value>,
    names: Vec<String>,
    attr_caches: Vec<crate::ast::AttrCache>,
    spans: Vec<crate::token::Span>,
    /// 名前 → slot（base スコープ: パラメータ + トップレベル let/mut/const、宣言順）。
    /// リゾルバの base slot 採番と同順（パラメータ→宣言）なので `LocalRef` と一致する。
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
/// - `body`: 解決済み関数本体（リゾルバが `LocalRef` を付与済み）。
pub fn compile_fn(params: &[Param], body: &[Stmt]) -> Option<Chunk> {
    // `for` ループ変数が外側変数をシャドウする関数は flat-slot モデルで表現できないため諦める。
    if has_for_target_shadow(params, body) {
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
            | Stmt::FromImport { .. }
            | Stmt::AsyncAssign { .. } => return None,
            _ => {}
        }
    }
    // ネストしたブロック（if/while/match のボディ）内の Let/Const/Mut にも
    // フレーム内固定 slot を割り当てる（R0-B: 関数内の全ローカルが平坦 slot）。
    // トップレベル decl は上で採番済みなのでスキップされる。順序は問わない
    // （compile は slots 引きで参照する）。シャドウイング禁止＝同名は非同時生存なので
    // slot 再利用は健全。リゾルバは nested 名を解決しない（Ident のまま）ので衝突しない。
    collect_nested_decls(body, &mut slots, &mut slot_mut, &mut slot_type, &mut n)?;

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
    };

    for stmt in body {
        c.compile_stmt(stmt)?;
    }
    // 本体末尾までフォールオフしたら None を返す。
    c.emit(Op::ReturnNil);

    Some(Chunk {
        code: c.code,
        consts: c.consts,
        names: c.names,
        attr_caches: c.attr_caches,
        spans: c.spans,
        local_names,
        n_locals: c.n_locals,
    })
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
        local_names: Vec::new(),
        n_locals: c.n_locals,
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
        Expr::Subscript { object, index } => {
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
        Expr::Subscript { object, index } => {
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
        // リテラル・Ident・LocalRef 等は宣言を含まない。
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

    fn add_name(&mut self, name: &str) -> u32 {
        let idx = self.names.len() as u32;
        self.names.push(name.to_string());
        self.attr_caches.push(crate::ast::AttrCache::default());
        idx
    }

    fn add_span(&mut self, span: &crate::token::Span) -> u32 {
        let idx = self.spans.len() as u32;
        self.spans.push(span.clone());
        idx
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
    /// LocalRef/Ident を true とする。メソッド呼び出し・属性代入のレシーバ判定に使う。
    fn object_is_instance(&self, e: &Expr) -> bool {
        let slot = match e {
            Expr::LocalRef { slot, .. } => *slot as usize,
            Expr::Ident(name) => match self.slots.get(name) {
                Some(&s) => s as usize,
                None => return false,
            },
            _ => return false,
        };
        if Some(slot as u16) == self.self_slot {
            return true;
        }
        self.slot_type
            .get(slot)
            .and_then(|o| o.as_deref())
            .map(is_user_instance_type)
            .unwrap_or(false)
    }

    /// 呼び出し引数の `is_mutable`（`eval_call_args` と同じ判定: 変数 ident は変数の可変性、
    /// それ以外の式は保守的に true）。VM は base ローカルしか読まないので slot_mut で判定できる。
    fn arg_is_mutable(&self, e: &Expr) -> bool {
        match e {
            Expr::LocalRef { slot, .. } => {
                self.slot_mut.get(*slot as usize).copied().unwrap_or(true)
            }
            Expr::Ident(name) => self
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
                _ => return None,
            }
        }
        Some(mask)
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Option<()> {
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
            Stmt::Assign { name, value, .. } => {
                let slot = *self.slots.get(name)?;
                self.compile_expr(value)?;
                self.emit(Op::StoreLocal(slot));
            }
            Stmt::CompoundAssign { name, op, value, .. } => {
                let slot = *self.slots.get(name)?;
                self.emit(Op::LoadLocal(slot));
                self.compile_expr(value)?;
                self.emit(Op::Bin(op.clone()));
                self.emit(Op::StoreLocal(slot));
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
            Stmt::Const(name, _, e) => {
                self.compile_expr(e)?;
                if name == "_" {
                    self.emit(Op::Pop);
                } else {
                    let slot = *self.slots.get(name)?;
                    self.emit(Op::StoreLocal(slot)); // const は copy/freeze しない
                }
            }
            Stmt::Mut(name, _, e) => {
                self.compile_expr(e)?;
                if name == "_" {
                    self.emit(Op::Pop);
                } else {
                    let slot = *self.slots.get(name)?;
                    self.emit(Op::StoreLocalDeepCopy(slot)); // mut は常に deep_copy
                }
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
                        Expr::LocalRef { slot: s, .. } => {
                            if self.slot_mut.get(*s as usize).copied().unwrap_or(false) {
                                Op::StoreLocalCopyFreeze(slot)
                            } else {
                                Op::StoreLocal(slot)
                            }
                        }
                        Expr::Ident(nm) => {
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
                Expr::Subscript { object, index } => {
                    let vtmp = self.alloc_temp()?;
                    self.compile_expr(value)?; // value を先に評価
                    self.emit(Op::StoreLocal(vtmp));
                    self.compile_expr(object)?; // obj
                    self.compile_expr(index)?; // key
                    self.emit(Op::LoadLocal(vtmp)); // value
                    self.emit(Op::SetIndex);
                    self.free_temp();
                }
                _ => return None, // TraitAccess・非 instance 属性は非対応
            },
            // 属性複合代入 `obj.attr op= value`（obj が `self`/instance のときのみ）。
            Stmt::AttrCompoundAssign { target, op, value } => {
                let (object, attr) = match target {
                    Expr::Attr { object, attr, .. } if self.object_is_instance(object) => {
                        (object, attr)
                    }
                    _ => return None,
                };
                let ni = self.add_name(attr);
                // ツリーウォークの評価順（value を先に評価 → 現在値を get → op）に一致させる。
                // stack: [obj(set base), value, obj(get base)] → GetAttr → [obj, value, cur]
                //   → Swap → [obj, cur, value] → Bin(op) → [obj, new] → SetAttr。
                self.compile_expr(object)?; // SetAttr のベース
                self.compile_expr(value)?; // rhs を先に評価
                self.compile_expr(object)?; // GetAttr のベース
                self.emit(Op::GetAttr(ni, ni));
                self.emit(Op::Swap);
                self.emit(Op::Bin(op.clone()));
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
            // それ以外（定義・import 等）は非対応。
            _ => return None,
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
            return None;
        }
        for h in handlers {
            if has_escape(&h.body, false, 0) {
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
                MatchPattern::Case(Expr::Ident(n)) if n == "_" => {
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
            Expr::LocalRef { slot, .. } => {
                let s = u16::try_from(*slot).ok()?;
                self.emit(Op::LoadLocal(s));
            }
            // Ident はパラメータ名のときのみローカル読み（それ以外＝グローバル/組み込みは非対応）。
            // デバッグモードでは停止スコープからの名前引き（LoadName）。
            Expr::Ident(name) => {
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
            Expr::BinOp { op, left, right, .. } => match op {
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
                    self.compile_expr(left)?;
                    self.compile_expr(right)?;
                    self.emit(Op::Bin(op.clone()));
                }
            },
            Expr::Attr { object, attr, .. } => {
                self.compile_expr(object)?;
                let name_idx = self.add_name(attr);
                self.emit(Op::GetAttr(name_idx, name_idx));
            }
            // 関数呼び出し `func(args)` / メソッド呼び出し `obj.method(args)`。
            Expr::Call { func, args, span, .. } => {
                if let Expr::Attr { object, attr, .. } = func.as_ref() {
                    // ── メソッド呼び出し ── object が Instance と保証できる（`self` または
                    // ユーザークラス型注釈の）LocalRef/Ident のときのみ対応。
                    if !self.object_is_instance(object) {
                        return None;
                    }
                    self.compile_expr(object)?; // receiver を push
                    let mask = self.compile_call_args(args)?;
                    let ni = self.add_name(attr);
                    self.emit(Op::CallMethod(ni, args.len() as u16, mask));
                    return Some(()); // メソッド呼び出しは span 不要
                }
                let site = self.add_span(span); // 関数呼び出しはトレースバック用の呼び出し位置を記録
                if let Expr::Ident(name) = func.as_ref() {
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
                        self.emit(Op::Call(args.len() as u16, mask, cn, site));
                    } else if let Some(&slot) = self.slots.get(name) {
                        // ローカル/パラメータが関数値を保持している場合は slot 読み。
                        self.emit(Op::LoadLocal(slot));
                        let mask = self.compile_call_args(args)?;
                        let ni = self.add_name(name);
                        self.emit(Op::Call(args.len() as u16, mask, ni, site));
                    } else if is_builtin_callee(name) || name == "Self" {
                        // その他の純粋 builtin・型コンストラクタ・`Self` は非対応。
                        return None;
                    } else {
                        // グローバル関数呼び出し。
                        let ni = self.add_name(name);
                        self.emit(Op::LoadGlobal(ni));
                        let mask = self.compile_call_args(args)?;
                        self.emit(Op::Call(args.len() as u16, mask, ni, site));
                    }
                } else if let Expr::LocalRef { slot, name } = func.as_ref() {
                    // 解決済みローカル関数値の呼び出し。
                    let s = u16::try_from(*slot).ok()?;
                    self.emit(Op::LoadLocal(s));
                    let mask = self.compile_call_args(args)?;
                    let ni = self.add_name(name);
                    self.emit(Op::Call(args.len() as u16, mask, ni, site));
                } else {
                    // その他の呼び先式（添字結果など）は非対応。
                    return None;
                }
            }
            // ── 添字・コレクションリテラル（タスク #5） ──
            Expr::Subscript { object, index } => {
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
            // それ以外（添字・コレクション・キャスト等）は非対応。
            _ => return None,
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
                MatchPattern::Case(Expr::Ident(n)) if n == "_" => {
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
