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

use crate::ast::{BinOp, Expr, Param, Stmt};
use crate::interpreter::Value;

use super::chunk::Chunk;
use super::op::Op;

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
    n_locals: usize,
}

/// トップレベル関数本体を Chunk へコンパイルする。非対応構文があれば `None`。
///
/// - `params`: 仮引数（可変長があれば非対応）。
/// - `body`: 解決済み関数本体（リゾルバが `LocalRef` を付与済み）。
pub fn compile_fn(params: &[Param], body: &[Stmt]) -> Option<Chunk> {
    // base slot をリゾルバと同順で採番する: パラメータ → トップレベル let/mut/const。
    let mut slots: HashMap<String, u16> = HashMap::new();
    let mut slot_mut: Vec<bool> = Vec::new();
    let mut n: u16 = 0;
    for p in params {
        if p.variadic {
            return None;
        }
        slots.insert(p.name.clone(), n);
        slot_mut.push(p.mutable);
        n = n.checked_add(1)?;
    }
    // トップレベル宣言を事前採番。LetTuple/Static/入れ子定義など slot をずらす形は非対応。
    for stmt in body {
        match stmt {
            Stmt::Let(name, ..) | Stmt::Const(name, ..)
                if name != "_" && !slots.contains_key(name) =>
            {
                slots.insert(name.clone(), n);
                slot_mut.push(false);
                n = n.checked_add(1)?;
            }
            Stmt::Mut(name, ..) if name != "_" && !slots.contains_key(name) => {
                slots.insert(name.clone(), n);
                slot_mut.push(true);
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

    let mut c = Compiler {
        code: Vec::new(),
        consts: Vec::new(),
        names: Vec::new(),
        attr_caches: Vec::new(),
        slots,
        slot_mut,
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

impl Compiler {
    #[inline]
    fn emit(&mut self, op: Op) -> usize {
        self.code.push(op);
        self.code.len() - 1
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
                for s in body {
                    self.compile_stmt(s)?;
                }
                self.emit(Op::Jump(start));
                let end = self.here();
                self.patch_jump(jf, end);
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
            // それ以外（break/continue・for/match/block・例外・定義・import 等）は非対応。
            _ => return None,
        }
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
            // それ以外（呼び出し・添字・コレクション・キャスト・式ブロック等）は非対応。
            _ => return None,
        }
        Some(())
    }
}
