// vm/compiler/expr.rs — 式のコンパイル（`compile_expr`）。
//
// ⚠ `pending`（式が始まる深さ）は入口で `take()` するので、**直前に設定した親だけ**が値を渡せる。
// ⚠ 注釈（`res`・型）は**最適化ヒントであって意味論の根拠ではない**（#15e）。


use crate::ast::{
    BinOp, Expr, Resolution,
};
use crate::interpreter::Value;

use crate::vm::op::Op;
use super::*;


impl Compiler {
    pub(super) fn compile_expr(&mut self, expr: &Expr) -> Option<()> {
        // #34: 親が「この式が始まる深さ」を伝えていれば受け取る。ここで奪うので、
        // 明示的に伝え直さない限り子の式は `None`（＝ブロック式内 `break` が bail）になる。
        let pending = self.pending.take();
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
            // 虚数リテラル（#27-c）。`eval` の `ImaginaryLit(f) => Value::Complex(0.0, f)` と同じ。
            Expr::ImaginaryLit(f) => {
                let ci = self.add_const(Value::Complex(0.0, *f));
                self.emit(Op::Const(ci));
            }
            // `undefined` リテラル（#27）。`eval` の `Expr::Undefined => Value::Undefined` と同じ。
            Expr::Undefined => {
                let ci = self.add_const(Value::Undefined);
                self.emit(Op::Const(ci));
            }
            // **セル変数**の読み（#27-d 段階 2b）。slot を持たないので slot 系より先に判定する。
            Expr::Ident { name, .. } if self.cells.contains_key(name) => {
                let i = self.cells[name];
                self.emit(Op::LoadCell(i));
            }
            // `static mut` の読み（#27-d）。**slot 系より先に判定する**（slot を持たない名前）。
            // ⚠ `Resolution::Local` より先に置くこと。`static` を含む関数はリゾルバが
            // 解決を諦める（`collect_base_decls` が未対応の宣言文で false を返す）ので
            // 実際には `Local` は付かないが、順序で守っておく。
            Expr::Ident { name, .. } if self.statics.contains_key(name) => {
                let span = self.statics[name].clone();
                let si = self.add_span(&span);
                self.emit(Op::LoadStatic(si));
            }
            Expr::Ident { res: Resolution::Local(slot), .. } => {
                let s = u16::try_from(*slot).ok()?;
                // セル化された base slot（#27-d 段階 2b）。slot は穴なので読んではいけない。
                match self.cell_by_slot.get(&s) {
                    Some(&i) => self.emit(Op::LoadCell(i)),
                    None => self.emit(Op::LoadLocal(s)),
                };
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
            // 未解決 Ident は slot にあればローカル読み、無ければグローバル読み。
            // デバッグモードでは停止スコープからの名前引き（LoadName）。
            Expr::Ident { name, .. } => {
                if self.mode.is_debug_repl() {
                    let ni = self.add_name(name);
                    self.emit(Op::LoadName(ni));
                } else {
                    match self.slots.get(name) {
                        Some(&slot) => {
                            self.emit(Op::LoadLocal(slot));
                        }
                        // slot にもグローバル解決にも載らない識別子（組み込み型名 `Signal`/`dict`、
                        // リゾルバがシャドウ懸念で外した名前など）。
                        //
                        // ツリーウォークの `Resolution::Unresolved` は `get_val(name)` そのもので、
                        // `Op::LoadName`（= `vm_load_name` = `get_val`）と**エラー文言まで同一**。
                        // よって最上位ではそのまま置き換えられる（#27-c）。
                        //
                        // ⚠ **関数本体では `LoadName` は使えない**。スコープの隔離は `frame_floor`
                        // が担うが、VM フレームは `exec_fn_evaled` の `frame_floor` 前進より手前で
                        // 分岐するので、`get_val` が**呼び出し元のローカルまで見えてしまう**。最上位は
                        // `toplevel_vm_candidate` が `scopes.len() == 1` を保証するので安全。
                        None if self.reads_by_name() => {
                            let ni = self.add_name(name);
                            self.emit(Op::LoadName(ni));
                        }
                        // 関数本体側は `LoadGlobal`（**`scopes[0]` だけを見る**）で載せる（#27）。
                        //
                        // ここへ来る名前が**この関数のローカルではない**ことはコンパイル時に確定している:
                        // base slot の採番と `collect_nested_decls` が本体の全宣言（for ターゲット・
                        // 入れ子ブロックの宣言を含む）を**先に** `slots` へ入れるので、`slots` を引いて
                        // 外れた名前はどの束縛にも当たらない。よってツリーウォークの `get_val`
                        // （`scopes[frame_floor..]` を走査 → `scopes[0]`）と**結果が一致する**
                        // （前段の走査は必ず外れる）。`LoadName` と違い呼び出し元のローカルを覗かないので
                        // `frame_floor` の問題も起きない。未定義時の `NameError: '<name>' is not defined`
                        // も文言まで同一（`Op::LoadGlobal` のミス経路）。
                        None => {
                            self.emit_load_global(name);
                        }
                    }
                }
            }
            // 可変長引数の読み `local::args`（#27）。ツリーウォークは `get_val("local::args")`
            // だが、VM では `compile_fn_inner` が同名で slot を採番しているので slot 読みで足りる。
            // slot が無い＝可変長パラメータを持たない関数での参照＝ツリーウォークでも
            // `NameError` になる形なので、そちらへ委ねる。
            Expr::LocalVar(name) => {
                let key = format!("local::{name}");
                match self.slots.get(key.as_str()) {
                    Some(&slot) => self.emit(Op::LoadLocal(slot)),
                    None => {
                        bail_expr("localvar-unbound", expr);
                        return None;
                    }
                };
            }
            Expr::UnaryOp { op, operand } => {
                // 被演算子は親と同じ深さで始まる（#34）。`-block ->int: …` が該当。
                self.pending = pending;
                self.compile_expr(operand)?;
                self.emit(Op::Un(op.clone()));
            }
            Expr::BinOp { op, left, right, node_id, .. } => match op {
                // 短絡評価: `a and b` / `a or b` は Python 意味論（値を返す）で書き下す。
                BinOp::And => {
                    self.pending = pending;
                    self.compile_expr(left)?;
                    let j = self.emit(Op::JumpIfFalseOrPop(0));
                    // 右辺の評価中は左辺の値が 1 つ積まれている（`JumpIf*OrPop` は
                    // 短絡したときだけ残す＝右辺へ進む経路でも push 済み・#34）。
                    self.pending = pending.map(|d| d + 1);
                    self.compile_expr(right)?;
                    let end = self.here();
                    self.patch_jump(j, end);
                }
                BinOp::Or => {
                    self.pending = pending;
                    self.compile_expr(left)?;
                    let j = self.emit(Op::JumpIfTrueOrPop(0));
                    self.pending = pending.map(|d| d + 1);
                    self.compile_expr(right)?;
                    let end = self.here();
                    self.patch_jump(j, end);
                }
                _ => {
                    // 超命令融合（#2）＋型特化（plan A）: 単純オペランドなら LoadLocal…+Bin を1命令に。
                    // ⚠ 融合対象は単純オペランドだけなのでブロック式は来ない（深さ伝播は不要）。
                    if !self.try_emit_bin_fused(left, right, op, *node_id) {
                        use crate::type_check::BinOperandKind as K;
                        self.pending = pending;
                        self.compile_expr(left)?;
                        // 右辺の評価中は左辺の値が 1 つ積まれている（#34）。
                        self.pending = pending.map(|d| d + 1);
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
                    // ── メソッド呼び出し ──
                    // #27-b: **レシーバの型を問わない**。実行時の `vm_method_call_other` が
                    // ツリーウォークと同じ統一実装（`eval_method_call_full`）へ委ねるので、
                    // list/str/dict/CsObject/Signal… どれでも同じ結果になる。
                    // 以前は `Value::Instance` 専用経路しか無く `object_is_instance` で
                    // 弾いていた（最上位・関数あわせて 110 件が bail していた）。
                    //
                    // ⚠ `node_id` を必ず渡すこと。FFI 戻り値検査のキーで、落とすと
                    // 外部言語メソッドの検査が VM 経路だけ素通りする。
                    // FFI 境界検査のエラーメッセージ用（#27-b）。ツリーウォークは
                    // `callee_display_name(func)`（= `L.get_int`）と呼び出し位置を渡すので、
                    // 同じものをコンパイル時に作って副表へ置く（op は太らせない）。
                    self.record_ffi_call_info(*node_id, object, attr, span);
                    // ⚠ **レシーバを push するかは引数をコンパイルする前に決める**（#27-c）。
                    // 名前付き引数があると `CallMethodLocal`（frame 直読み融合）は使えないので、
                    // 引数の形を先に見て融合の可否を確定させる。
                    let fuse_slot = if has_named_args(args) { None } else { self.as_local(object) };
                    if let Some(slot) = fuse_slot {
                        // 超命令融合（#16 段階(b)(i)）: レシーバが局所変数なら push せず frame 直読み。
                        let (mask, _) = self.compile_call_args(args, Some(*node_id))?;
                        let ni = self.add_name(attr);
                        self.emit(Op::CallMethodLocal(slot, ni, args.len() as u16, mask, *node_id));
                    } else {
                        self.compile_expr(object)?; // receiver を push
                        let (mask, kw) = self.compile_call_args(args, Some(*node_id))?;
                        let ni = self.add_name(attr);
                        match kw {
                            None => {
                                self.emit(Op::CallMethod(ni, args.len() as u16, mask, *node_id));
                            }
                            // 名前付き／可変長引数（#27-c）。dispatcher は同じで、
                            // 引数名を `kw_calls` 経由で運ぶだけ。
                            Some(arg_names) => {
                                let i = u32::try_from(self.chunk.kw_calls.len()).ok()?;
                                self.chunk.kw_calls.push(crate::vm::chunk::KwCall {
                                    argc: u16::try_from(args.len()).ok()?,
                                    mut_mask: mask,
                                    name_idx: ni,
                                    // メソッドは call_span=None で呼ぶので span は使わない。
                                    span_idx: 0,
                                    node_id: *node_id,
                                    arg_names,
                                });
                                self.emit(Op::CallMethodKw(i));
                            }
                        }
                    }
                    return Some(()); // メソッド呼び出しは span 不要
                }
                let site = self.add_span(span); // 関数呼び出しはトレースバック用の呼び出し位置を記録
                if let Expr::Ident { name, res: Resolution::Unresolved, .. } = func.as_ref() {
                    // ── VM 対応組み込み（print/range/len）── 評価済み引数で直接呼ぶ。
                    // ローカル slot に同名（シャドウ）がなければ組み込みとして扱う。
                    if is_vm_builtin(name) && !self.slots.contains_key(name) {
                        // 組み込みは mut_mask 不要。
                        // 組み込みは `mut` ポインタ引数を取らないので書き戻し記録は不要（#48）。
                        let (_, kw) = self.compile_call_args(args, None)?;
                        let ni = self.add_name(name);
                        match kw {
                            None => {
                                self.emit(Op::CallBuiltin(ni, args.len() as u16));
                            }
                            // 名前付き引数（#27-c）。解釈を確認済みの組み込みだけ引数名ごと運ぶ。
                            // それ以外は `eval_builtin_evaled` が名前を受け取れないので bail。
                            Some(arg_names) if VM_BUILTIN_KW_NAMES.contains(&name.as_str()) => {
                                let i = u32::try_from(self.chunk.kw_calls.len()).ok()?;
                                self.chunk.kw_calls.push(crate::vm::chunk::KwCall {
                                    argc: u16::try_from(args.len()).ok()?,
                                    mut_mask: 0, // 組み込みは mut 引数を取らない
                                    name_idx: ni,
                                    span_idx: site,
                                    node_id: *node_id,
                                    arg_names,
                                });
                                self.emit(Op::CallBuiltinKw(i));
                            }
                            Some(_) => {
                                bail("call-arg", None);
                                return None;
                            }
                        }
                    } else if self.mode.is_debug_repl() {
                        // デバッグモード: 呼び先を名前引きで取得（局所・グローバル両対応）。
                        let cn = self.add_name(name);
                        self.emit(Op::LoadName(cn));
                        let (mask, kw) = self.compile_call_args(args, Some(*node_id))?;
                        self.emit_call(args.len(), mask, cn, site, *node_id, kw)?;
                    } else if let Some(&slot) = self.slots.get(name) {
                        // ローカル/パラメータが関数値を保持している場合は slot 読み。
                        self.emit(Op::LoadLocal(slot));
                        let (mask, kw) = self.compile_call_args(args, Some(*node_id))?;
                        let ni = self.add_name(name);
                        self.emit_call(args.len(), mask, ni, site, *node_id, kw)?;
                    } else if name == "Self" && self.self_slot.is_some() {
                        // メソッド本体の `Self(...)`（#27）: レシーバのクラスを積んで通常の
                        // `Call` へ流す（`call_value_evaled` の `Value::Class` アーム＝
                        // ツリーウォークと同一のインスタンス化経路）。
                        self.emit(Op::LoadSelfClass);
                        let (mask, kw) = self.compile_call_args(args, Some(*node_id))?;
                        let ni = self.add_name(name);
                        self.emit_call(args.len(), mask, ni, site, *node_id, kw)?;
                    } else if name == "Self" {
                        // メソッド本体の外の `Self(...)`。**そもそも不正なコード**だが、現状は
                        // bail するので `VmForceError` になる（本来は `NameError: 'Self' is not defined`）。
                        // ⚠ #34 の「必ず失敗する文は bail せず同じ文言を出す」に反している。#56 の
                        // 調査で見つけたが、#56 で削除した `is_builtin_callee` と違い**正しいコードは壊していない**
                        // ので分離した（別タスク）。
                        //
                        // ⚠⚠ **bail を足す前に「bail した先で何が起きるか」を確かめること。**
                        // #33 でツリーウォークへのフォールバックは消えたので **bail ＝ 停止**である。
                        // `is_builtin_callee` はこの取り違えで `parse_ar` を丸ごと殺していた（#55/#56）。
                        if crate::interpreter::tw_stats::enabled() {
                            crate::interpreter::tw_stats::record_bail("callee-builtin", name);
                        }
                        return None;
                    } else {
                        // グローバル関数呼び出し（#11: 索引キャッシュ付き LoadGlobal）。
                        let ni = self.add_name(name);
                        let ci = self.chunk.global_caches.len() as u32;
                        self.chunk.global_caches.push(crate::ast::SlotCache::default());
                        self.emit(Op::LoadGlobal(ni, ci));
                        let (mask, kw) = self.compile_call_args(args, Some(*node_id))?;
                        self.emit_call(args.len(), mask, ni, site, *node_id, kw)?;
                    }
                } else if let Expr::Ident { name, res: Resolution::Global(_), .. } = func.as_ref() {
                    // 解決済みグローバル関数呼び出し（R2-b）。
                    // 分類はリゾルバ済みなので builtin/slots の判定は不要。
                    // ただしデバッグモードは停止スコープの名前引きに合わせる。
                    let ni = self.add_name(name);
                    if self.mode.is_debug_repl() {
                        self.emit(Op::LoadName(ni));
                    } else {
                        self.emit_load_global(name);
                    }
                    let (mask, kw) = self.compile_call_args(args, Some(*node_id))?;
                    self.emit_call(args.len(), mask, ni, site, *node_id, kw)?;
                } else if let Expr::Ident { name, res: Resolution::Local(slot), .. } = func.as_ref() {
                    // 解決済みローカル関数値の呼び出し。
                    let s = u16::try_from(*slot).ok()?;
                    self.emit(Op::LoadLocal(s));
                    let (mask, kw) = self.compile_call_args(args, Some(*node_id))?;
                    let ni = self.add_name(name);
                    self.emit_call(args.len(), mask, ni, site, *node_id, kw)?;
                } else if let Expr::TemplateInstantiate { base, type_args } = func.as_ref() {
                    // テンプレート呼び出し `Tmpl[T](args)`（#27-c）。ツリーウォークの
                    // `eval_call` と同じく「base を評価 → `instantiate_template` 本体」。
                    self.compile_expr(base)?;
                    // テンプレート実体化は Arrow 関数なので native 書き戻しは無い（#48）。
                    let (mask, kw) = self.compile_call_args(args, None)?;
                    Self::no_kw(kw)?; // テンプレートの名前付き引数は未対応（#27-c 残り）
                    let ti = u32::try_from(self.chunk.type_arg_lists.len()).ok()?;
                    self.chunk.type_arg_lists.push(type_args.clone());
                    self.emit(Op::CallTemplate(ti, args.len() as u16, mask));
                } else {
                    // その他の呼び先式（`block:` 式・添字結果・属性以外の任意式）。
                    //
                    // ツリーウォークの `eval_call` も「呼び先式を評価 → `call_value_evaled`」なので、
                    // 素直に **[callee, args...] を積んで `Call`** すればよい（#27-c）。
                    // ⚠ トレースバック表示名は **`<anonymous>`**（`eval_call` の `call_name` が
                    // 識別子以外に付ける名前と揃える。ここを関数名にすると off/auto で出力が食い違う）。
                    self.compile_expr(func)?;
                    let (mask, kw) = self.compile_call_args(args, Some(*node_id))?;
                    let ni = self.add_name("<anonymous>");
                    self.emit_call(args.len(), mask, ni, site, *node_id, kw)?;
                }
            }
            // ── 添字・コレクションリテラル（タスク #5） ──
            Expr::Subscript { object, index, .. } => {
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                self.emit(Op::Subscript);
            }
            Expr::Slice { begin, end, step } => {
                // 省略された要素は `Op::Nil`（= `Value::None`）を積む。`slice_from_values`
                // が「無し」に畳むので、ツリーウォークの `None` と同じ意味になる。
                for part in [begin, end, step] {
                    match part {
                        Some(e) => self.compile_expr(e)?,
                        None => {
                            self.emit(Op::Nil);
                        }
                    }
                }
                self.emit(Op::BuildSlice);
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
            Expr::Block { stmts, return_type } => {
                let ann = self.add_return_type(return_type);
                self.compile_block_expr(stmts, pending, ann, true)?
            }
            Expr::IfExpr { branches, else_body, return_type } => {
                let ann = self.add_return_type(return_type);
                self.compile_if_expr(branches, else_body, pending, ann)?
            }
            Expr::MatchExpr { subject, arms, return_type } => {
                let ann = self.add_return_type(return_type);
                self.compile_match_expr(subject, arms, pending, ann)?
            }
            Expr::ForExpr { target, iter, body, return_type } => {
                let ann = self.add_return_type(return_type);
                self.compile_for_expr(target, iter, body, ann)?
            }
            Expr::WhileExpr { cond, body, return_type } => {
                let ann = self.add_return_type(return_type);
                self.compile_while_expr(cond, body, ann)?
            }

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
}
