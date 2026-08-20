// vm/compiler/stmt.rs — 文のコンパイル（`compile_stmt`）。
//
// ⚠ 文の入口はオペランドスタックが平衡（#34）。この文の**最初の**式にだけ深さを伝える。
// ⚠ **必ず失敗する文は bail せず `Op::Fail` で同じ文言を出す**（#34）。bail すると
// `VmForceError` になり、本来のエラー文言が消える。


use crate::ast::{
    Expr, Stmt, Resolution,
};

use crate::vm::op::{DeclKind, Op};
use super::*;


impl Compiler {
    pub(super) fn compile_stmt(&mut self, stmt: &Stmt) -> Option<()> {
        self.mark_stmt_start(stmt);
        // 文の入口はオペランドスタックが平衡（#34）。この文の**最初の**式にだけ深さを伝える。
        // `compile_expr` が `take()` するので 2 つ目以降の式は `None`（＝保守的に bail）になる。
        self.pending = self.stmt_base;
        match stmt {
            Stmt::Expr(e) => {
                self.compile_expr(e)?;
                self.emit(Op::Pop);
            }
            Stmt::Return(Some(e)) => {
                self.compile_expr(e)?;
                // #37: 開いている `finally` を**全部**走らせてから返す（内側から）。
                // ⚠ `try/except` の `PopTry` は不要（`run` から即復帰してハンドラごと捨てられる）。
                //    偽を渡すことで finally を持たない既存 Chunk は 1 命令も変わらない。
                // ⚠ 戻り値が 1 つ積まれた上で finally が走る（#40）。
                self.emit_unwind_tries(0, false, 1)?;
                self.emit(Op::Return);
            }
            Stmt::Return(None) => {
                self.emit_unwind_tries(0, false, 0)?;
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
                // モジュール本体の代入（#42）。名前でチェーンを探して書く。
                StoreTarget::Name(ni) => {
                    self.compile_expr(value)?;
                    self.emit(Op::StoreName(ni));
                }
                // `static mut` への代入（#27-d）。共有セルへ直接書く。
                StoreTarget::Static(si) => {
                    self.compile_expr(value)?;
                    self.emit(Op::StoreStatic(si));
                }
                // セル変数への代入（#27-d 段階 2b）。共有相手からも見える。
                StoreTarget::Cell(i) => {
                    self.compile_expr(value)?;
                    self.emit(Op::StoreCell(i));
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
                    // モジュール本体への複合代入（#42）。読みは `LoadName`、書きは `StoreName`。
                    StoreTarget::Name(ni) => {
                        let kind = self.annot_binop_kind(*node_id);
                        self.emit(Op::LoadName(ni));
                        self.compile_expr(value)?;
                        match kind {
                            Some(K::Int) => self.emit(Op::IntBinSS(op.clone())),
                            Some(K::Float) => self.emit(Op::FloatBinSS(op.clone())),
                            None => self.emit(Op::Bin(op.clone())),
                        };
                        self.emit(Op::StoreName(ni));
                    }
                    // `static mut` への複合代入（#27-d）。グローバル版と同じ形。
                    StoreTarget::Static(si) => {
                        let kind = self.annot_binop_kind(*node_id);
                        self.emit(Op::LoadStatic(si));
                        self.compile_expr(value)?;
                        match kind {
                            Some(K::Int) => self.emit(Op::IntBinSS(op.clone())),
                            Some(K::Float) => self.emit(Op::FloatBinSS(op.clone())),
                            None => self.emit(Op::Bin(op.clone())),
                        };
                        self.emit(Op::StoreStatic(si));
                    }
                    // セル変数への複合代入（#27-d 段階 2b）。`static` 版と同じ形。
                    StoreTarget::Cell(i) => {
                        let kind = self.annot_binop_kind(*node_id);
                        self.emit(Op::LoadCell(i));
                        self.compile_expr(value)?;
                        match kind {
                            Some(K::Int) => self.emit(Op::IntBinSS(op.clone())),
                            Some(K::Float) => self.emit(Op::FloatBinSS(op.clone())),
                            None => self.emit(Op::Bin(op.clone())),
                        };
                        self.emit(Op::StoreCell(i));
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
                    try_len: self.try_stack.len(),
                });
                // 本体はこのループ入口の深さで走る（#34）。
                let saved_base = self.stmt_base.replace(0);
                for s in body {
                    self.compile_stmt(s)?;
                }
                self.stmt_base = saved_base;
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
                if targets.is_empty() {
                    bail("for-no-target", None);
                    return None;
                }
                // 受け皿の temp が要るのは 2 通り（#27-c）:
                //  - 複数ターゲット `for k, v in ...` … 要素をいったん受けてから分解する
                //  - 捨てターゲット `for _ in ...`   … `_` は `add_decl` が slot を振らない
                //    （`let _ = e` は値を捨てるため）が、`ForIter` には**書き込み先が要る**
                // ⚠ **外側の同名束縛を覆うループ変数には専用 slot を割り当てる**（#27）。
                //
                // Arrow の `for` 変数はブロックスコープで、ツリーウォークは反復ごとに
                // スコープを push して宣言する（ループを抜けると外側の値が戻る）。
                // 名前ごとに 1 slot の flat モデルでこれを表現するため、**本体の間だけ**
                // `slots` の対応を temp slot へ差し替え、ループ後に元へ戻す。
                // これにより本体内の読みは temp を、ループ後の読みは元の slot を指す。
                //
                // 差し替えは `slot_of`（`target_slot` / `target_slots` の算出）より**前**に
                // 行う必要がある。temp は LIFO なので、解放は iter/sink より後（＝最後）。
                let mut shadow_saved: Vec<(String, u16)> = Vec::new();
                for t in targets {
                    if t != "_" && self.shadowed_for_targets.contains(t) {
                        let fresh = self.alloc_temp()?;
                        if let Some(old) = self.slots.insert(t.clone(), fresh) {
                            shadow_saved.push((t.clone(), old));
                        }
                    }
                }
                let unpack = targets.len() > 1;
                let sink_temp = if unpack || targets[0] == "_" {
                    Some(self.alloc_temp()?)
                } else {
                    None
                };
                let target_slot = match sink_temp {
                    Some(t) => t,
                    None => self.slot_of(&targets[0])?,
                };
                // 分解先 slot は**本体をコンパイルする前に**引いておく（`?` の早期 return で
                // temp の解放が漏れないようにするため）。`_` は捨てるので `None`。
                let mut target_slots: Vec<Option<u16>> = Vec::new();
                if unpack {
                    for t in targets {
                        if t == "_" {
                            target_slots.push(None);
                        } else {
                            target_slots.push(Some(self.slot_of(t)?));
                        }
                    }
                }
                // イテレータを取得して temp slot に格納。
                let iter_temp = self.alloc_temp()?;
                self.compile_expr(iter)?;
                self.emit(Op::GetIter);
                self.emit(Op::StoreLocal(iter_temp));
                // loop_start: ForIter で next。EndOfIteration なら exit へ、要素なら target へ束縛。
                let loop_start = self.here();
                let fi = self.emit(Op::ForIter(iter_temp, target_slot, 0)); // exit は後でパッチ
                // タプル分解: 要素を push して**逆順**に StoreLocal で受ける（pop は末尾から）。
                if unpack {
                    self.emit(Op::UnpackTuple(target_slot, targets.len() as u16));
                    for ts in target_slots.iter().rev() {
                        match ts {
                            Some(slot) => self.emit(Op::StoreLocal(*slot)),
                            None => self.emit(Op::Pop), // `for k, _ in ...` の捨て要素
                        };
                    }
                }
                self.loops.push(LoopCtx {
                    continue_target: loop_start, // continue は次の ForIter へ戻る
                    break_jumps: Vec::new(),
                    try_len: self.try_stack.len(),
                });
                // 本体はこのループ入口の深さで走る（#34）。
                let saved_base = self.stmt_base.replace(0);
                for s in body {
                    self.compile_stmt(s)?;
                }
                self.stmt_base = saved_base;
                self.emit(Op::Jump(loop_start));
                let exit = self.here();
                // ForIter の exit_ip をバックパッチ（patch_jump は Jump 系専用なので手動）。
                self.code[fi] = Op::ForIter(iter_temp, target_slot, exit);
                let ctx = self.loops.pop().unwrap();
                for j in ctx.break_jumps {
                    self.patch_jump(j, exit);
                }
                self.free_temp(); // iter_temp
                if sink_temp.is_some() {
                    self.free_temp(); // タプル分解／`_` 用の受け皿（#27-c）
                }
                // シャドウしていたループ変数の名前を外側の slot へ戻す（#27）。
                for (name, old) in shadow_saved.into_iter().rev() {
                    self.slots.insert(name, old);
                    self.free_temp();
                }
            }
            Stmt::Break => {
                // 最内ループの break_jumps に登録し、末尾へジャンプ（バックパッチ）。
                // ⚠ ブロック式の途中から跳ぶときは、その式が積んだオペランドを先に捨てる（#34）。
                if self.loops.is_empty() {
                    // 囲むループが無い＝実行時に必ず失敗する。**bail せず**ツリーウォークと
                    // 同じメッセージで落とす（bail すると `--vm=on` が `VmForceError` になり、
                    // 正しいエラー報告が off/on で食い違う・#34）。
                    let n = self.add_name("SyntaxError: 'break' outside for/while loop");
                    self.emit(Op::Fail(n));
                    return Some(());
                }
                self.emit_unwind_to_loop()?;
                let j = self.emit(Op::Jump(0));
                self.loops.last_mut()?.break_jumps.push(j);
            }
            Stmt::Continue => {
                let Some(target) = self.loops.last().map(|l| l.continue_target) else {
                    let n = self.add_name("SyntaxError: 'continue' outside for/while loop");
                    self.emit(Op::Fail(n));
                    return Some(());
                };
                self.emit_unwind_to_loop()?;
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
                    let slot = self.slot_of(name)?;
                    self.emit(Op::StoreLocal(slot)); // const は copy/freeze しない
                }
            }
            Stmt::Mut(name, _, e) => {
                self.compile_expr(e)?;
                if name == "_" {
                    self.emit(Op::Pop);
                } else if let Some(ni) = self.toplevel_decl_name(name) {
                    self.emit(Op::DeclareGlobal(ni, DeclKind::Mut));
                } else if let Some(&i) = self.cells.get(name) {
                    // 入れ子 `fn` に可変キャプチャされるローカル（#27-d 段階 2b）。
                    // deep_copy はセル版でも同じ（`Stmt::Mut` は常に複製する）。
                    self.emit(Op::StoreCellDeepCopy(i));
                } else {
                    let slot = self.slot_of(name)?;
                    self.emit(Op::StoreLocalDeepCopy(slot)); // mut は常に deep_copy
                }
            }
            Stmt::Let(name, _, e) if self.toplevel_decl_name(name).is_some() && name != "_" => {
                // 最上位の `let`（#10-c）。ソースが識別子のときの可変性は**コンパイル時に
                // 分からない**（`toplevel_globals` は名前の集合だけ）ので、予測せず
                // `LetFromIdent` でソース名を渡し、`exec_let` と同じ判断を実行時に行う（#27-c）。
                let kind = match e {
                    Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Str(_) | Expr::None => {
                        DeclKind::LetPlain
                    }
                    Expr::Ident { name: src, .. } => DeclKind::LetFromIdent(self.add_name(src)),
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
                    let slot = self.slot_of(name)?;
                    // ソースの種類で store op を選ぶ（exec_let のセマンティクスに一致）。
                    //
                    // ⚠ **`exec_let` は `Resolution` を見ない**。`Expr::Ident` なら何であれ
                    // `get_var(src)` の可変性で分岐する。よってここも**識別子は全て識別子として
                    // 扱う**こと（`Resolution::Global` を非識別子式の枝へ落とすと、可変グローバルを
                    // ソースにした `let` でコピー＆フリーズが漏れる）。
                    let store = match e {
                        // ident ソースのうち**可変性がコンパイル時に分かる**もの（＝slot にある）。
                        // 可変なら copy+freeze、不変ならそのまま。
                        Expr::Ident { res: Resolution::Local(s), .. } => {
                            if self.slot_mut.get(*s as usize).copied().unwrap_or(false) {
                                Op::StoreLocalCopyFreeze(slot)
                            } else {
                                Op::StoreLocal(slot)
                            }
                        }
                        Expr::Ident { name: nm, .. } if self.slots.contains_key(nm) => {
                            let s = self.slots[nm];
                            if self.slot_mut.get(s as usize).copied().unwrap_or(false) {
                                Op::StoreLocalCopyFreeze(slot)
                            } else {
                                Op::StoreLocal(slot)
                            }
                        }
                        // slot に無い ident（グローバル・未定義）は**実行時に**ソースの可変性を見る。
                        Expr::Ident { name: nm, .. } => {
                            let ni = self.add_name(nm);
                            Op::StoreLocalFromIdent(slot, ni)
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
            // `static mut x = e`（#27-d）。記憶域は `Interpreter::static_cells`（宣言位置がキー）。
            //
            // `exec_static_var` は **セルが既にあれば初期化子を評価しない**ので、
            // 「あればジャンプで飛び越す」形に落とす:
            //   StaticInit(span, after) ─ セルがあれば after へ
            //   <初期化子>
            //   StaticStore(span)       ─ セルを作って値を入れる
            //   after:
            // ⚠ ツリーウォークは毎回 `declare_var(name, Var::new_cell(cell))` するが、VM は
            // 名前の束縛を持たず読み書きを直接セルへ落とすので、宣言そのものは何も出さない。
            Stmt::Static(name, expr, span) => {
                // 採番済みの宣言位置と一致することを前提にする（prepass が入れたもの）。
                let Some(decl_span) = self.statics.get(name).cloned() else {
                    // 入れ子ブロックの中の `static`（prepass が見ていない）は非対応。
                    bail("static-nested", None);
                    return None;
                };
                debug_assert_eq!(
                    (decl_span.line, decl_span.col),
                    (span.line, span.col),
                    "static の宣言位置が prepass と compile でずれている"
                );
                let si = self.add_span(&decl_span);
                let guard = self.emit(Op::StaticInit(si, 0)); // 飛び先は後でパッチ
                self.compile_expr(expr)?;
                self.emit(Op::StaticStore(si));
                let after = self.here();
                self.code[guard] = Op::StaticInit(si, after);
            }
            // 属性代入 `obj.attr = value` / 添字代入 `obj[i] = value`。
            Stmt::AttrAssign { target, value } => match target {
                // `obj.attr = value`。obj を push → value を push → SetAttr。
                //
                // ⚠ レシーバの種類で絞らない（#27-c）。`attr_assign_evaled` が
                // ツリーウォークの `attr_assign` の**唯一の実装**になったので、
                // `Value::Instance` / `Value::Class` / それ以外のエラーまで一致する。
                // 以前は `object_is_instance` で絞っていたが、それは 2 実装の差を
                // 隠すためのもので、型注釈の無いグローバルが bail する原因だった。
                Expr::Attr { object, attr, .. } => {
                    self.compile_expr(object)?;
                    // #34: 右辺の評価中は obj が 1 つ積まれている。伝えないと
                    // `obj.x = 1 + block ->int: … break …` が bail する（実測で見つけた漏れ）。
                    self.pending = self.stmt_base.map(|d| d + 1);
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
                    self.pending = self.stmt_base.map(|d| d + 1); // 同上（#34）
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
            // 添字への複合代入 `obj[k] op= value`（#27-c）。
            //
            // ツリーウォークは `rhs = eval(value)` → `lhs = eval(target)` → 二項演算 →
            // `attr_assign(target, result)` の順で、**`object`/`index` を 2 回評価する**
            // （読みで 1 回、代入で 1 回）。副作用まで一致させるため、そのまま 2 回積む。
            Stmt::AttrCompoundAssign { target: target @ Expr::Subscript { .. }, op, value } => {
                let Expr::Subscript { object, index, .. } = target else {
                    unreachable!("matched above")
                };
                let rhs_tmp = self.alloc_temp()?;
                self.compile_expr(value)?; // 1. rhs を先に評価
                self.emit(Op::StoreLocal(rhs_tmp));
                self.compile_expr(object)?; // 2. 現在値の読み
                self.compile_expr(index)?;
                self.emit(Op::Subscript);
                self.emit(Op::LoadLocal(rhs_tmp));
                self.emit(Op::Bin(op.clone())); // 3. 二項演算
                let res_tmp = self.alloc_temp()?;
                self.emit(Op::StoreLocal(res_tmp));
                self.compile_expr(object)?; // 4. 代入（`attr_assign` と同じく再評価）
                self.compile_expr(index)?;
                self.emit(Op::LoadLocal(res_tmp));
                self.emit(Op::SetIndex);
                self.free_temp();
                self.free_temp();
            }
            // 属性複合代入 `obj.attr op= value`。
            //
            // ⚠ **レシーバの種類で絞らない**（#27）。読みは `GetAttr`（ツリーウォークの
            // `eval_attr` と同じ `get_attr_val`）、書きは `SetAttr`（`attr_assign` と**同一の**
            // `attr_assign_evaled`）なので、`Value::Class` の `static mut` まで意味論が一致する。
            // 以前あった `object_is_instance` の条件は「2 実装の差」ではなく、下の
            // **局所 slot 前提の最適化**（レシーバを 1 回しか評価しない融合）を守るためのもの。
            // 局所 slot でないレシーバはツリーウォークどおり 2 回評価する経路へ回す。
            Stmt::AttrCompoundAssign { target, op, value } => {
                let (object, attr) = match target {
                    Expr::Attr { object, attr, .. } => (object, attr),
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
                match self.as_local(object) {
                    // レシーバが局所 slot（`self`・ローカル変数）のとき。**再評価が副作用を
                    // 持たない**ので、`SetAttr` のベースを 1 回積むだけで読み書き両方に使える。
                    Some(obj_slot) => {
                        self.compile_expr(object)?; // SetAttr のベース

                        // 評価順（#2a）。ツリーウォークは **value を先に評価してから**現在値を読むので、
                        // 素直に組むと [value, cur] の順にスタックへ乗り `Swap` が要る。
                        // ただし value が**副作用を持たない**（局所変数読み or 定数リテラル）なら、
                        // 先に現在値を読んでも観測結果は同じなので `Swap` を丸ごと落とせる。
                        // レシーバ slot が value の評価中に再束縛されないことは `CallMethodLocal` と
                        // 同じ根拠（再束縛は文＝`StoreLocal` でしか起きず、クロージャ捕捉は VM 非対応
                        // で bail する）。
                        //
                        // 現在値の読み出しは `LoadLocal; GetAttr` の 2 命令を `GetAttrLocal` 1 命令へ
                        // 畳む（レシーバを **clone せず frame から参照で読む**ので `Rc` の refcount
                        // 増減も消える）。`Expr::Attr` の compile と同じ融合。
                        let value_pure =
                            self.as_local(value).is_some() || Self::as_const_lit(value).is_some();
                        if !value_pure {
                            self.compile_expr(value)?; // rhs を先に評価（順序保存）
                        }
                        self.emit(Op::GetAttrLocal(obj_slot, ni, ni));
                        if value_pure {
                            // [obj, cur, value] → Bin → [obj, new]（Swap 不要）
                            self.compile_expr(value)?;
                        } else {
                            // [obj, value, cur] → Swap → [obj, cur, value] → Bin → [obj, new]
                            self.emit(Op::Swap);
                        }
                        self.emit_bin_specialized(kind, op);
                        self.emit(Op::SetAttr(ni));
                    }
                    // 一般レシーバ（グローバル変数・クラス名・属性・呼び出し結果／`CompileMode::DebugRepl`）。
                    //
                    // ツリーウォークは `eval(value)` → `eval(target)`（**object 1 回目**）→ 二項演算
                    // → `attr_assign(target, ..)`（**object 2 回目**）の順で、`object` を 2 回評価する。
                    // 副作用まで一致させるため**そのまま 2 回積む**（添字複合代入 `d[k] op= v` と
                    // 同じ扱い・#27-c）。上の融合を使えるのは再評価が無害な局所 slot のときだけ。
                    None => {
                        let rhs_tmp = self.alloc_temp()?;
                        self.compile_expr(value)?; // 1. rhs を先に評価
                        self.emit(Op::StoreLocal(rhs_tmp));
                        self.compile_expr(object)?; // 2. 現在値の読み
                        self.emit(Op::GetAttr(ni, ni));
                        self.emit(Op::LoadLocal(rhs_tmp));
                        self.emit_bin_specialized(kind, op); // 3. 二項演算
                        let res_tmp = self.alloc_temp()?;
                        self.emit(Op::StoreLocal(res_tmp));
                        self.compile_expr(object)?; // 4. 代入（`attr_assign` と同じく再評価）
                        self.emit(Op::LoadLocal(res_tmp));
                        self.emit(Op::SetAttr(ni));
                        self.free_temp();
                        self.free_temp();
                    }
                }
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
                let ctx = self.block_ctxs.last()?;
                let result_slot = ctx.result_slot;
                let ann = ctx.return_type;
                let block_try_len = ctx.try_len;
                // #40: `finally` の複製の中から跳ぶときは、複製が載っている分を捨てる。
                let block_pops = match (self.stmt_base, ctx.entry_depth) {
                    (Some(now), Some(base)) => now.saturating_sub(base),
                    _ => 0,
                };
                self.compile_expr(e)?;
                // #35: `->T` があれば実行時検査（ツリーウォークの `check_block_return_type`）。
                // #43: 判定種別をアノテーション文字列から先に決める。`Any` は常に真なので
                // **op 自体を出さない**（検査を省くのではなく、検査が自明に成立する場合だけ）。
                if let Some(idx) = ann {
                    let tag = crate::vm::op::TypeTag::of(&self.names[idx as usize]);
                    if tag != crate::vm::op::TypeTag::Any {
                        self.emit(Op::CheckBlockReturn(idx, tag));
                    }
                }
                self.emit(Op::StoreLocal(result_slot));
                // #37: ブロック式入口までの try を巻き戻す（finally を走らせる）。
                // ⚠ 値は既に `result_slot` へ退避済みなので finally が何を積んでも安全。
                self.emit_unwind_tries(block_try_len, true, 0)?;
                for _ in 0..block_pops {
                    self.emit(Op::Pop);
                }
                let j = self.emit(Op::Jump(0));
                self.block_ctxs.last_mut().unwrap().end_jumps.push(j);
            }
            // loop_yield は最内の「yield 先を持つ」ブロック式（block:/for/while 式）の蓄積リストへ追加。
            // if/match 式は透過（yield_slot=None）なので飛ばして外側へ届く。
            Stmt::LoopYield(e) => {
                let Some(yield_slot) = self.block_ctxs.iter().rev().find_map(|c| c.yield_slot)
                else {
                    // for/while 式の外の `loop_yield`（#35）。**bail せず**ツリーウォークと
                    // 同じ文言で落とす（bail すると `--vm=on` だけ `VmForceError` になる）。
                    let n = self.add_name(
                        "SyntaxError: 'loop_yield' can only be used inside a for/while expression (with ->list[T] annotation)",
                    );
                    self.emit(Op::Fail(n));
                    return Some(());
                };
                // ⚠ 要素型は**最内ブロック式**のアノテーションから引く（`yield_slot` を持つ
                // ブロック式とは限らない）。ツリーウォークの `BLOCK_RETURN_EXPECTED_TYPE.last()` と同じ。
                let ann = self.block_ctxs.last().and_then(|c| c.return_type);
                self.compile_expr(e)?;
                if let Some(idx) = ann {
                    // #43: **要素型**から種別を決める（`list[T]` の `T`）。
                    // `list[T]` の形でなければ検査自体が無いので op を出さない
                    // （`check_loop_yield_type` が `Ok(())` を返すのと同じ）。
                    let ann_str = self.names[idx as usize].clone();
                    match crate::vm::op::elem_type_of_list_ann(&ann_str) {
                        Some(elem) => {
                            let tag = crate::vm::op::TypeTag::of(elem);
                            if tag != crate::vm::op::TypeTag::Any {
                                self.emit(Op::CheckLoopYield(idx, tag));
                            }
                        }
                        None => {}
                    }
                }
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
            // 入れ子 `fn` 定義（#27）。**外側フレームを一切参照しない場合に限り**載せる。
            //
            // ⚠ ここが健全性の要。ツリーウォークは `capture_env` で外側スコープを走査して
            // キャプチャを作るが、VM フレームは `scopes` に無いので走査しても見つからない。
            // 「自由変数 ∩ 外側の slot = ∅」を確かめれば、ツリーウォークでも
            // キャプチャは空になる＝両者一致する。**この検査を緩めると閉包変数が黙って消える**。
            //
            // デコレータ・テンプレートは非対応（`eval` と `TemplateFnValue` の再現が要るため）。
            Stmt::FnDef {
                name,
                template_params,
                params,
                body,
                decorators,
                return_type,
                ..
            } => {
                if !template_params.is_empty() {
                    bail("nested-fn-template", None);
                    return None;
                }
                if !decorators.is_empty() {
                    bail("nested-fn-decorator", None);
                    return None;
                }
                let Some((captures, cell_captures, static_captures)) =
                    self.nested_fn_captures(params, body)
                else {
                    // 事前解析（`nested_fn_free_names`）が拾えなかった可変キャプチャ。
                    bail("nested-fn-mutable-capture", None);
                    return None;
                };
                let slot = self.slot_of(name)?;
                let idx = u32::try_from(self.fn_defs.len()).ok()?;
                self.fn_defs.push(crate::vm::chunk::ChunkFnDef {
                    name: name.clone(),
                    params: params.clone(),
                    // #45: ここで 1 回だけ複製する。以降の実体は Rc を clone するだけ。
                    body: std::rc::Rc::from(&body[..]),
                    return_type: return_type.clone(),
                    slot,
                    captures,
                    cell_captures,
                    static_captures,
                    // #30: 実体ごとではなく**定義サイトごと**に本体をコンパイルする。
                    // 最初の呼び出しで埋まり、以降の実体は同じ `Rc<Chunk>` を共有する。
                    compiled: Default::default(),
                });
                self.emit(Op::MakeFn(idx));
            }
            // `block: <stmts>` 文（#27-c）。ツリーウォークの `exec_block_stmt`（#33 で削除）は
            // **`block_return` を吸収**して `Normal` を返し、break/continue/return/raise は外へ通す。
            // ブロック式のコンパイラをそのまま使い、値を捨てれば同じ意味論になる。
            // ⚠ 文なので入口はオペランドスタックが平衡＝深さは `stmt_base` そのもの（#34）。
            // 内部の `break`/`continue` は外側ループへ貫通する（以前は本体ごと bail していた）。
            Stmt::Block(body) => {
                let depth = self.stmt_base;
                // ⚠ `block:` **文**はツリーウォークで `BLOCK_RETURN_EXPECTED_TYPE` へ push しない
                // ので、中の `block_return` は**外側の式**のアノテーションで検査される（#35）。
                let ann = self.block_ctxs.last().and_then(|c| c.return_type);
                // ⚠ `block:` **文**は loop_yield に対して**透過**（#35）。
                self.compile_block_expr(body, depth, ann, false)?;
                self.emit(Op::Pop);
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
            // `let a, b = t`（#27-c）。束縛先は slot（制御フロー内の宣言）と
            // グローバル宣言（最上位の宣言文）の 2 通り。`collect_nested_decls` が
            // 入れ子のターゲットにだけ slot を割り当てるので、その有無で判別できる。
            //
            // ⚠ 混ぜると `for` の 2 周目で「already declared」になる（`built_in.ar` の `zx`）。
            Stmt::LetTuple { targets, value, .. } => {
                use crate::ast::TupleTarget;
                let name_of = |t: &TupleTarget| match t {
                    TupleTarget::Let(n) | TupleTarget::Bare(n) | TupleTarget::Mut(n) => {
                        Some(n.clone())
                    }
                    TupleTarget::Wildcard => None,
                };
                let slots: Vec<Option<u16>> = targets
                    .iter()
                    .map(|t| name_of(t).and_then(|n| self.slots.get(&n).copied()))
                    .collect();
                let any_slot = slots.iter().any(|s| s.is_some());
                if any_slot {
                    // 一部だけ slot に載る形は想定外（`collect_nested_decls` は全ターゲットを
                    // まとめて登録する）。取りこぼしを黙って混ぜないよう弾く。
                    if targets
                        .iter()
                        .zip(&slots)
                        .any(|(t, s)| name_of(t).is_some() && s.is_none())
                    {
                        bail("let-tuple-partial-slots", Some(stmt));
                        return None;
                    }
                } else if !self.writes_toplevel_globals() {
                    // 最上位モードでないのに slot も無い ＝ 束縛先が決まらない。
                    bail("let-tuple-no-target", Some(stmt));
                    return None;
                }
                self.compile_expr(value)?;
                let i = u32::try_from(self.tuple_decls.len()).ok()?;
                self.tuple_decls.push(crate::vm::chunk::TupleDecl {
                    targets: targets.to_vec(),
                    slots: if any_slot { slots } else { Vec::new() },
                });
                self.emit(Op::LetTuple(i));
            }
            // `freeze x`（#27-c）。値をスタックに載せずに `exec_freeze` を呼ぶだけ。
            Stmt::Freeze(name, span) => {
                let ni = self.add_name(name);
                let si = self.add_span(span);
                self.emit(Op::FreezeVar(ni, si));
            }
            // `src on/once handler` / `src off handler`（#27-c）。
            // 評価順（source → handler）はツリーウォークと同じ。
            Stmt::EventSubscribe { source, handler, is_once, is_async, .. } => {
                self.compile_expr(source)?;
                self.compile_expr(handler)?;
                self.emit(Op::EventSubscribe(*is_once, *is_async));
            }
            Stmt::EventUnsubscribe { source, handler, .. } => {
                self.compile_expr(source)?;
                self.compile_expr(handler)?;
                self.emit(Op::EventUnsubscribe);
            }
            // それ以外（定義・import 等）は非対応。
            _ => {
                bail("stmt", Some(stmt));
                return None;
            }
        }
        Some(())
    }
}
