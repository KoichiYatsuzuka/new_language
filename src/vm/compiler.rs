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

use std::collections::HashMap;

use crate::ast::{BinOp, CallArg, Expr, MatchArm, MatchPattern, Param, Stmt};
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

    let mut c = Compiler {
        code: Vec::new(),
        consts: Vec::new(),
        names: Vec::new(),
        attr_caches: Vec::new(),
        slots,
        slot_mut,
        slot_type,
        self_slot,
        loops: Vec::new(),
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
        n_locals: c.n_locals,
    })
}

/// ネストしたブロック内の `let`/`const`/`mut` 宣言に平坦 slot を割り当てる（再帰）。
/// コンパイラが本体をコンパイルできる構文（if/while/match）にのみ踏み込む。
/// 既出名（トップレベル decl・別ブロックの同名）はスキップ（slot 再利用）。
fn collect_nested_decls(
    body: &[Stmt],
    slots: &mut HashMap<String, u16>,
    slot_mut: &mut Vec<bool>,
    slot_type: &mut Vec<Option<String>>,
    n: &mut u16,
) -> Option<()> {
    fn add(
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
    for stmt in body {
        match stmt {
            Stmt::Let(name, ty, _) | Stmt::Const(name, ty, _) => {
                add(name, ty, false, slots, slot_mut, slot_type, n)?
            }
            Stmt::Mut(name, ty, _) => add(name, ty, true, slots, slot_mut, slot_type, n)?,
            Stmt::If { branches, else_body } => {
                for (_, b) in branches {
                    collect_nested_decls(b, slots, slot_mut, slot_type, n)?;
                }
                if let Some(eb) = else_body {
                    collect_nested_decls(eb, slots, slot_mut, slot_type, n)?;
                }
            }
            Stmt::While { body, .. } => collect_nested_decls(body, slots, slot_mut, slot_type, n)?,
            Stmt::Match { arms, .. } => {
                for arm in arms {
                    collect_nested_decls(&arm.body, slots, slot_mut, slot_type, n)?;
                }
            }
            Stmt::For { targets, body, .. } => {
                // ループ変数は可変（tree-walk は Var::new(item, true)）。型注釈なし。
                for t in targets {
                    add(t, &None, true, slots, slot_mut, slot_type, n)?;
                }
                collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
            }
            // その他（未対応構文）には踏み込まない。compile_stmt が到達時に bail する。
            _ => {}
        }
    }
    Some(())
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
            // 属性代入 `obj.attr = value`（obj が `self`/instance で side-effect-free のときのみ）。
            Stmt::AttrAssign { target, value } => {
                let (object, attr) = match target {
                    Expr::Attr { object, attr, .. } if self.object_is_instance(object) => {
                        (object, attr)
                    }
                    _ => return None, // Subscript/TraitAccess/非 instance は非対応
                };
                // obj（SetAttr のベース）を push → value を push → SetAttr。
                // object は side-effect-free（self/base ローカル）なので先に push してよい。
                self.compile_expr(object)?;
                self.compile_expr(value)?;
                let ni = self.add_name(attr);
                self.emit(Op::SetAttr(ni));
            }
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
            // それ以外（break/continue・for/match/block・例外・定義・import 等）は非対応。
            _ => return None,
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
            Expr::Ident(name) => {
                let slot = *self.slots.get(name)?;
                self.emit(Op::LoadLocal(slot));
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
            Expr::Call { func, args, .. } => {
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
                } else if let Expr::Ident(name) = func.as_ref() {
                    // ── VM 対応組み込み（print/range/len）── 評価済み引数で直接呼ぶ。
                    // ローカル slot に同名（シャドウ）がなければ組み込みとして扱う。
                    if is_vm_builtin(name) && !self.slots.contains_key(name) {
                        self.compile_call_args(args)?; // 組み込みは mut_mask 不要
                        let ni = self.add_name(name);
                        self.emit(Op::CallBuiltin(ni, args.len() as u16));
                    } else if let Some(&slot) = self.slots.get(name) {
                        // ローカル/パラメータが関数値を保持している場合は slot 読み。
                        self.emit(Op::LoadLocal(slot));
                        let mask = self.compile_call_args(args)?;
                        self.emit(Op::Call(args.len() as u16, mask));
                    } else if is_builtin_callee(name) || name == "Self" {
                        // その他の純粋 builtin・型コンストラクタ・`Self` は非対応。
                        return None;
                    } else {
                        // グローバル関数呼び出し。
                        let ni = self.add_name(name);
                        self.emit(Op::LoadGlobal(ni));
                        let mask = self.compile_call_args(args)?;
                        self.emit(Op::Call(args.len() as u16, mask));
                    }
                } else if let Expr::LocalRef { slot, .. } = func.as_ref() {
                    // 解決済みローカル関数値の呼び出し。
                    let s = u16::try_from(*slot).ok()?;
                    self.emit(Op::LoadLocal(s));
                    let mask = self.compile_call_args(args)?;
                    self.emit(Op::Call(args.len() as u16, mask));
                } else {
                    // その他の呼び先式（添字結果など）は非対応。
                    return None;
                }
            }
            // それ以外（添字・コレクション・キャスト・式ブロック等）は非対応。
            _ => return None,
        }
        Some(())
    }
}
