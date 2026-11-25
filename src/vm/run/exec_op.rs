// vm/run/exec_op.rs — 1 命令の実行（`exec_op`）と、そこから呼ぶグローバルの読み出し・引数の印。

use super::*;

/// 単一命令を実行し、制御フローを返す。エラーは `Err` で返し、`run` がハンドラスタックへ回す。
///
/// ⚠ `#[inline(always)]`: 呼び出し元が `run` と `run_stepping` の **2 つ**になった時点で
/// `#[inline]` だけではインライン展開されなくなり、**通常経路が 3〜5% 退行した**（#1 で実測）。
/// バイトコードは byte-identical だったので、原因は生成コードではなくインライン判断だと特定できた。
///
/// ⚠ **重い op の本体はここに書かず `#[inline(never)]` の関数へ出す**（#10-b で実測）。
/// この関数は全呼び出し元へ展開されるので、その op を使わない Chunk まで巻き添えで遅くなる。
/// `Chunk::global_refs[gi]` のグローバルを読む（#70）。
///
/// ⚠⚠ **`Op::LoadGlobal` と完全に同じ手順**でなければならない — 索引キャッシュのヒット判定、
/// 失効時の名前引きフォールバック、`NameError` の文言まで。違えると
/// 「融合したときだけ挙動が変わる」という最悪の形になる。
///
/// ⚠ `exec_op` は `#[inline(always)]` なので**アームに重い本体を書かない**（#10-b）。
/// ここは `#[inline]` に留めて、ミス経路が展開されないようにしてある。
/// 索引キャッシュが**当たっているとき限定**で、グローバルの `i64` を clone せずに読む（#70）。
///
/// ⚠ キャッシュミス時は解決を**しない**（`None` を返す）。解決とキャッシュ充填は
/// `load_global_ref` の 1 箇所に閉じておきたいため — 2 箇所で埋めると
/// 「どちらが埋めたか」で `NameError` の出方が変わりうる。
#[inline]
fn cached_global_int(interp: &Interpreter, chunk: &Chunk, gi: u32) -> Option<i64> {
    let (_, cache) = &chunk.global_refs[gi as usize];
    let idx = cache.get(interp.vm_slot_epoch())?;
    interp.vm_global_int_by_slot(idx)
}

#[inline]
fn load_global_ref(
    interp: &mut Interpreter,
    chunk: &Chunk,
    gi: u32,
) -> Result<Value, String> {
    let (ni, cache) = &chunk.global_refs[gi as usize];
    let epoch = interp.vm_slot_epoch();
    if let Some(idx) = cache.get(epoch) {
        if let Some(v) = interp.vm_global_by_slot(idx) {
            return Ok(v);
        }
        // 想定外（index 失効）は名前引きへフォールバック。
    }
    let name = &chunk.names[*ni as usize];
    match interp.vm_global_slot_of(name) {
        Some(idx) => {
            cache.fill(epoch, idx as u32);
            interp
                .vm_global_by_slot(idx)
                .ok_or_else(|| format!("NameError: '{name}' is not defined"))
        }
        // 束縛が無ければ組み込みの値（名前の探索の最後の段・タスク 1-7・`Op::LoadGlobal` と同じ）。
        None => interp
            .builtin_value(name)
            .ok_or_else(|| format!("NameError: '{name}' is not defined")),
    }
}

/// 呼び出しに「重ならない引数」の印があれば（[`crate::vm::op::MASK_DISJOINT_INFO`]）、束縛が見られるように
/// インタープリタへ置く（copy_alloc_speed_plan.md 4-6）。返すガードが落ちるとき（呼び出しの終わり・エラーでも）に消す。
/// ⚠ 印は**この呼び出しの評価済みの引数の並び**（番地）と対で置く。束縛は並びの番地が同じときだけ使うので、
///   途中で走るほかの呼び出し（特殊メソッド・組み込みが呼ぶ関数）には当たらない。
/// ⚠ `exec_op` は `#[inline(always)]` なので、ここは判定 1 つだけにして本体を外に出す。
#[inline(always)]
fn note_disjoint_args(
    interp: &mut Interpreter,
    mut_mask: u32,
    evaled: &[(Option<String>, Value, bool)],
    node_id: u32,
) -> Option<DisjointArgsGuard> {
    if mut_mask & crate::vm::op::MASK_DISJOINT_INFO == 0 {
        return None;
    }
    Some(note_disjoint_args_slow(interp, evaled, node_id))
}

#[inline(never)]
fn note_disjoint_args_slow(
    interp: &mut Interpreter,
    evaled: &[(Option<String>, Value, bool)],
    node_id: u32,
) -> DisjointArgsGuard {
    interp.pending_disjoint = (evaled.as_ptr() as usize, interp.annotations.disjoint_args(node_id));
    DisjointArgsGuard(interp as *mut Interpreter)
}

/// [`note_disjoint_args`] が置いた印を、呼び出しを抜けるときに消す。
struct DisjointArgsGuard(*mut Interpreter);

impl Drop for DisjointArgsGuard {
    fn drop(&mut self) {
        // SAFETY: ガードは `exec_op` の 1 つの命令の中だけで生き、その間 `interp` は生きている
        unsafe { (*self.0).pending_disjoint = (0, 0) };
    }
}

#[inline(always)]
pub(super) fn exec_op(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut Vec<Value>,
    base: usize,
    ip: usize,
    handlers: &mut Vec<Handler>,
    // #27-d 段階 2b: フレームのセル表（`n_cells == 0` の関数では空スライス）。
    cells: &[Rc<RefCell<Value>>],
) -> Result<Flow, String> {
    match &chunk.code[ip] {
        Op::Const(i) => buf.push(chunk.consts[*i as usize].clone()),
        Op::Nil => buf.push(Value::None),
        Op::LoadLocal(s) => {
            let v = buf[base + *s as usize].clone();
            buf.push(v);
        }
        Op::LoadGlobal(ni, ci) => {
            // #11: グローバル索引キャッシュ。ヒット時は名前ハッシュ引きを飛ばし slot 直読み。
            let cache = &chunk.global_caches[*ci as usize];
            let epoch = interp.vm_slot_epoch();
            if let Some(idx) = cache.get(epoch) {
                if let Some(v) = interp.vm_global_by_slot(idx) {
                    buf.push(v);
                    return Ok(Flow::Next);
                }
                // 想定外（index 失効）は名前引きへフォールバック。
            }
            let name = &chunk.names[*ni as usize];
            match interp.vm_global_slot_of(name) {
                Some(idx) => {
                    cache.fill(epoch, idx as u32);
                    // 直前に解決した index からそのまま読む（None は理論上起きない）。
                    match interp.vm_global_by_slot(idx) {
                        Some(v) => buf.push(v),
                        None => return Err(format!("NameError: '{name}' is not defined")),
                    }
                }
                // 束縛が無ければ組み込みの値（名前の探索の最後の段・タスク 1-7）。
                // ⚠ `load_global_ref` も同じ手順にすること（上の doc の「完全に同じ手順」）。
                None => match interp.builtin_value(name) {
                    Some(v) => buf.push(v),
                    None => return Err(format!("NameError: '{name}' is not defined")),
                },
            }
        }
        Op::StoreGlobal(ni, ci) => {
            // インラインキャッシュ（`LoadGlobal` と同じ形）。**ヒット経路だけをここに置き**、
            // 初回・失効時は `#[inline(never)]` の `store_global_miss` へ落とす。
            // `exec_op` は `#[inline(always)]` なので、ミス経路まで展開すると
            // この op を使わない Chunk のホットループを巻き添えにする（同関数のコメント参照）。
            let v = buf.pop().unwrap();
            let cache = &chunk.global_caches[*ci as usize];
            match cache.get(interp.vm_slot_epoch()) {
                Some(idx) => {
                    if let Some(v) = interp.vm_store_global_by_cell(idx, v) {
                        // 想定外（index 失効）は名前引きへフォールバック。
                        store_global_miss(interp, chunk, *ni, cache, v)?;
                    }
                }
                None => store_global_miss(interp, chunk, *ni, cache, v)?,
            }
        }
        Op::StoreLocal(s) => {
            let v = buf.pop().unwrap();
            buf[base + *s as usize] = v;
        }
        // 束縛の複製は `Interpreter::bind_copy` が唯一の実装（ツリーウォーク・ネイティブと共有・1-6）。
        Op::StoreLocalDeepCopy(s) => {
            let v = interp.bind_copy(BindCopy::DeepCopy, buf.pop().unwrap())?;
            buf[base + *s as usize] = v;
        }
        // `let x = <mut 変数>` と、関数の中の `freeze x`（`LoadLocal` + これ・タスク 1-9）。
        Op::StoreLocalCopyFreeze(s) => {
            let v = interp.bind_copy(BindCopy::CopyFreeze, buf.pop().unwrap())?;
            buf[base + *s as usize] = v;
        }
        // `let x = <グローバル識別子>`（#27-c）。ソースの可変性は実行時にしか分からないので
        // `vm_let_value_from_ident`（`DeclKind::LetFromIdent` と**同一の実装**）へ委譲する。
        Op::StoreLocalImplicit(s, ni) => {
            let v = buf.pop().unwrap();
            if matches!(v, Value::PyObject(_)) && !chunk.builtin_prelude {
                implicit_binding_check(interp, &chunk.names[*ni as usize], &v)?;
            }
            buf[base + *s as usize] = v;
        }
        Op::StoreLocalFromIdent(s, ni) => {
            let v = buf.pop().unwrap();
            let src_mutable = interp.vm_global_is_mutable(&chunk.names[*ni as usize]);
            let v = interp.vm_let_value_from_ident(src_mutable, v)?;
            buf[base + *s as usize] = v;
        }
        // ⚠ 名前に反して `Instance` 以外も複製する（L2）。以前は `Instance` だけで、関数の中の
        //   `let it = ys[0]` が `ys[0]` と共有された（ツリーウォーク・最上位とずれていた・1-6）。
        Op::StoreLocalFreezeInstance(s) => {
            let v = interp.bind_copy(BindCopy::CopyFreeze, buf.pop().unwrap())?;
            buf[base + *s as usize] = v;
        }
        Op::Pop => {
            buf.pop();
        }
        // 案 B（2026-09-08）: `float` 注釈の宣言でストア直前に置かれる。
        // ツリーウォークの `coerce_binding` と同一の判断（`Int` のときだけ昇格・他は素通し）。
        Op::CoerceFloat => {
            if let Some(Value::Int(n)) = buf.last() {
                let f = *n as f64;
                *buf.last_mut().unwrap() = Value::Float(f);
            }
        }
        Op::Bin(op) => {
            let b = buf.pop().unwrap();
            let a = buf.pop().unwrap();
            let r = apply_bin_fast(interp, op, a, b)?;
            buf.push(r);
        }
        // 超命令（#2）: オペランドを frame/const から直接読み、push/pop とディスパッチを省く。
        Op::BinLocalLocal(a, b, op) => {
            let lhs = buf[base + *a as usize].clone();
            let rhs = buf[base + *b as usize].clone();
            let r = apply_bin_fast(interp, op, lhs, rhs)?;
            buf.push(r);
        }
        Op::BinLocalConst(a, ci, op) => {
            let lhs = buf[base + *a as usize].clone();
            let rhs = chunk.consts[*ci as usize].clone();
            let r = apply_bin_fast(interp, op, lhs, rhs)?;
            buf.push(r);
        }
        // 型特化（plan A）: オペランドを参照で読み、int/float 直接算術。想定外型は汎用へ委譲。
        Op::IntBinLL(a, b, op) => {
            let r = match (&buf[base + *a as usize], &buf[base + *b as usize]) {
                (Value::Int(x), Value::Int(y)) => int_binop_specialized(*x, *y, op),
                _ => None,
            };
            match r {
                Some(v) => buf.push(v),
                None => {
                    let l = buf[base + *a as usize].clone();
                    let rr = buf[base + *b as usize].clone();
                    let v = apply_bin_fast(interp, op, l, rr)?;
                    buf.push(v);
                }
            }
        }
        // #70: 最上位ループ用のグローバル融合。`LoadGlobal` 相当の読みを**同じ順序で**行う。
        // ⚠ 重い本体はここに書かない（`exec_op` は `#[inline(always)]`・#10-b）。
        //    グローバル読みは `#[inline]` の `load_global_ref` へ出してある。
        Op::IntBinGG(ga, gb, op) => {
            // 速い経路（#70）: 索引キャッシュが当たっていて両方 int なら**clone せずに**計算する。
            // ⚠ ここを通らない場合（キャッシュミス・非 int・未定義）は下の通常経路へ落ちる
            //   — `NameError` の文言と `apply_bin_fast` の意味論を保つため。
            if let (Some(x), Some(y)) = (
                cached_global_int(interp, chunk, *ga),
                cached_global_int(interp, chunk, *gb),
            ) {
                if let Some(v) = int_binop_specialized(x, y, op) {
                    buf.push(v);
                    return Ok(Flow::Next);
                }
            }
            let a = load_global_ref(interp, chunk, *ga)?;
            let b = load_global_ref(interp, chunk, *gb)?;
            let r = match (&a, &b) {
                (Value::Int(x), Value::Int(y)) => int_binop_specialized(*x, *y, op),
                _ => None,
            };
            match r {
                Some(v) => buf.push(v),
                None => {
                    let v = apply_bin_fast(interp, op, a, b)?;
                    buf.push(v);
                }
            }
        }
        Op::IntBinGC(ga, ci, op) => {
            // 速い経路（#70）。`IntBinGG` と同じ理由。
            if let (Some(x), Value::Int(y)) = (
                cached_global_int(interp, chunk, *ga),
                &chunk.consts[*ci as usize],
            ) {
                if let Some(v) = int_binop_specialized(x, *y, op) {
                    buf.push(v);
                    return Ok(Flow::Next);
                }
            }
            let a = load_global_ref(interp, chunk, *ga)?;
            let r = match (&a, &chunk.consts[*ci as usize]) {
                (Value::Int(x), Value::Int(y)) => int_binop_specialized(*x, *y, op),
                _ => None,
            };
            match r {
                Some(v) => buf.push(v),
                None => {
                    let b = chunk.consts[*ci as usize].clone();
                    let v = apply_bin_fast(interp, op, a, b)?;
                    buf.push(v);
                }
            }
        }
        Op::IntBinLC(a, ci, op) => {
            let r = match (&buf[base + *a as usize], &chunk.consts[*ci as usize]) {
                (Value::Int(x), Value::Int(y)) => int_binop_specialized(*x, *y, op),
                _ => None,
            };
            match r {
                Some(v) => buf.push(v),
                None => {
                    let l = buf[base + *a as usize].clone();
                    let rr = chunk.consts[*ci as usize].clone();
                    let v = apply_bin_fast(interp, op, l, rr)?;
                    buf.push(v);
                }
            }
        }
        Op::FloatBinLL(a, b, op) => {
            let r = match (&buf[base + *a as usize], &buf[base + *b as usize]) {
                (Value::Float(x), Value::Float(y)) => float_binop_specialized(*x, *y, op),
                _ => None,
            };
            match r {
                Some(v) => buf.push(v),
                None => {
                    let l = buf[base + *a as usize].clone();
                    let rr = buf[base + *b as usize].clone();
                    let v = apply_bin_fast(interp, op, l, rr)?;
                    buf.push(v);
                }
            }
        }
        Op::FloatBinLC(a, ci, op) => {
            let r = match (&buf[base + *a as usize], &chunk.consts[*ci as usize]) {
                (Value::Float(x), Value::Float(y)) => float_binop_specialized(*x, *y, op),
                _ => None,
            };
            match r {
                Some(v) => buf.push(v),
                None => {
                    let l = buf[base + *a as usize].clone();
                    let rr = chunk.consts[*ci as usize].clone();
                    let v = apply_bin_fast(interp, op, l, rr)?;
                    buf.push(v);
                }
            }
        }
        // 型特化（#16 段階(b)(iii)）: オペランドの形を問わずスタック上の2値を参照で見る。
        // 属性・添字・呼び出し結果など `LL`/`LC` では拾えなかった式にも特化が乗る。
        Op::IntBinSS(op) => {
            let n = buf.len();
            let r = match (&buf[n - 2], &buf[n - 1]) {
                (Value::Int(x), Value::Int(y)) => int_binop_specialized(*x, *y, op),
                _ => None,
            };
            match r {
                Some(v) => {
                    buf.truncate(n - 2);
                    buf.push(v);
                }
                None => {
                    let b = buf.pop().unwrap();
                    let a = buf.pop().unwrap();
                    let v = apply_bin_fast(interp, op, a, b)?;
                    buf.push(v);
                }
            }
        }
        Op::FloatBinSS(op) => {
            let n = buf.len();
            let r = match (&buf[n - 2], &buf[n - 1]) {
                (Value::Float(x), Value::Float(y)) => float_binop_specialized(*x, *y, op),
                _ => None,
            };
            match r {
                Some(v) => {
                    buf.truncate(n - 2);
                    buf.push(v);
                }
                None => {
                    let b = buf.pop().unwrap();
                    let a = buf.pop().unwrap();
                    let v = apply_bin_fast(interp, op, a, b)?;
                    buf.push(v);
                }
            }
        }
        Op::Un(op) => {
            let a = buf.pop().unwrap();
            let r = interp.apply_unary_dyn(op, a)?;
            buf.push(r);
        }
        Op::GetAttr(name_idx, cache_idx) => {
            let obj = buf.pop().unwrap();
            let cache = &chunk.attr_caches[*cache_idx as usize];
            // R3 インラインキャッシュのヒット（public フィールド）を VM ループ内で直接処理し、
            // get_attr_val の関数呼び出しを避ける。ミス・非 public・非インスタンスはフルパスへ。
            // 命中判定は `InstanceData::ic_slot`（ツリーウォーク・ネイティブと共有・2-1）。
            let v = 'get: {
                if let Value::Instance(inst_rc) = &obj {
                    let inst = inst_rc.borrow();
                    #[cfg(feature = "prof")]
                    crate::prof::note_ic(cache.get(inst.class.class_id).is_some(), cache.is_empty());
                    if let Some((idx, _)) = inst.ic_slot(cache.0.get(), true, || &chunk.names[*name_idx as usize]) {
                        if let Some(fv) = inst.field_value(idx) {
                            break 'get fv;
                        }
                    }
                }
                interp.get_attr_val(obj, &chunk.names[*name_idx as usize], Some(cache))?
            };
            buf.push(v);
        }
        // 超命令（#16 段階(b)(i)）: レシーバを frame から**参照で**読む。`GetAttr` と同一意味論だが
        // `LoadLocal` の `Value` clone（＝`Rc` refcount 増減）と push/pop が消える。
        // IC ミス・非 public・非インスタンスのときだけ clone してフルパスへ回す。
        Op::GetAttrLocal(slot, name_idx, cache_idx) => {
            let cache = &chunk.attr_caches[*cache_idx as usize];
            let v = 'get: {
                if let Value::Instance(inst_rc) = &buf[base + *slot as usize] {
                    let inst = inst_rc.borrow();
                    #[cfg(feature = "prof")]
                    crate::prof::note_ic(cache.get(inst.class.class_id).is_some(), cache.is_empty());
                    if let Some((idx, _)) = inst.ic_slot(cache.0.get(), true, || &chunk.names[*name_idx as usize]) {
                        if let Some(fv) = inst.field_value(idx) {
                            break 'get fv;
                        }
                    }
                }
                let obj = buf[base + *slot as usize].clone();
                interp.get_attr_val(obj, &chunk.names[*name_idx as usize], Some(cache))?
            };
            buf.push(v);
        }
        // 書き込みの IC（2-3）。命中経路・ミス経路とも `attr_assign_ic`（ネイティブと共有）。
        Op::SetAttr(name_idx, cache_idx) => {
            let value = buf.pop().unwrap();
            let obj = buf.pop().unwrap();
            let cache = &chunk.attr_caches[*cache_idx as usize];
            interp.attr_assign_ic(obj, &chunk.names[*name_idx as usize], value, Some(cache))?;
        }
        Op::Swap => {
            let n = buf.len();
            buf.swap(n - 1, n - 2);
        }
        Op::IsType(name_idx) => {
            let v = buf.pop().unwrap();
            let r = interp.value_is_type(&v, &chunk.names[*name_idx as usize]);
            buf.push(Value::Bool(r));
        }
        // `CheckBefore` 指示の消費（#16 段階(b)(ii)）。
        // 検査本体はツリーウォークと**同一メソッド**（`mustbe_check`）を呼ぶので意味論が構造的に一致する。
        Op::MustBe(type_idx, span_idx) => {
            let v = buf.pop().unwrap();
            let r = interp.mustbe_check(
                v,
                &chunk.names[*type_idx as usize],
                &chunk.spans[*span_idx as usize],
            )?;
            buf.push(r);
        }
        Op::Cast(type_idx) => {
            let v = buf.pop().unwrap();
            let r = interp.eval_cast_evaled(v, &chunk.names[*type_idx as usize])?;
            buf.push(r);
        }
        Op::CallBuiltin(name_idx, argc) => {
            let n = *argc as usize;
            let split = buf.len() - n;
            let args = buf.split_off(split); // arg0..argN-1（順序保持）
            let name = &chunk.names[*name_idx as usize];
            // #15d: グローバルに同名のユーザー束縛があれば組み込みではなくそちらを呼ぶ。
            // ローカルのシャドウはコンパイル時に `slots.contains_key` で除外済みなので、
            // 実行時に見るのはグローバルだけでよい。ツリーウォーク側の
            // `builtin_is_shadowed` と同じ規則（`Value::Type` の自己名は組み込み登録なので除外）。
            // 振り分けの本体はネイティブコードの組み込み呼び出しと共有する（native_object_access_plan.md 1-3）。
            buf.push(interp.call_builtin_by_name_evaled(name, args)?);
        }
        // キーワード引数つきの組み込み呼び出し（#27-c）。`CallBuiltin` と同じ経路だが、
        // 引数名を一緒に渡して `eval_builtin_evaled_named` に解釈させる。
        Op::CallBuiltinKw(kw_idx) => {
            let kw = &chunk.kw_calls[*kw_idx as usize];
            let n = kw.argc as usize;
            let split = buf.len() - n;
            let args = buf.split_off(split); // arg0..argN-1（順序保持）
            let name = &chunk.names[kw.name_idx as usize];
            if interp.builtin_is_shadowed_global(name) {
                // グローバルにユーザー束縛があれば組み込みではなくそちらを呼ぶ（`CallBuiltin` と同じ）。
                let callee = interp
                    .vm_load_name(name)
                    .ok_or_else(|| format!("NameError: '{name}' is not defined"))?;
                let evaled = args
                    .into_iter()
                    .zip(kw.arg_names.iter())
                    .map(|(v, k)| (k.clone(), v, false))
                    .collect();
                let name = name.clone(); // interp の可変借用のため名前を退避する
                buf.push(interp.call_value_evaled(callee, evaled, &name, None, 0)?);
            } else {
                let named: Vec<(Option<String>, Value)> =
                    args.into_iter().zip(kw.arg_names.iter()).map(|(v, k)| (k.clone(), v)).collect();
                match interp.eval_builtin_evaled_named(name, named) {
                    Some(r) => buf.push(r?),
                    None => return Err(format!("NameError: '{name}' is not defined")),
                }
            }
        }
        Op::GetIter => {
            let iterable = buf.pop().unwrap();
            let iter = interp.make_for_iterator(iterable)?;
            buf.push(iter);
        }
        Op::GetIterChecked(node_id) => {
            let iterable = buf.pop().unwrap();
            let iter = interp.make_for_iterator_checked(iterable, Some(*node_id))?;
            buf.push(iter);
        }
        Op::ForIter(iter_slot, target_slot, exit_ip) => {
            let iter_idx = base + *iter_slot as usize;
            // 高速パス: Generator（range/list/str/set/tuple/gen __iter__ の実体）は
            // index を直接進める（eval_method_call のディスパッチを丸ごと回避）。
            // メソッド呼び出しの Generator "next" アームと同一意味論。
            // ⚠⚠ **借用を握ったまま再開しない**（B13）。以前は `state.borrow_mut()` を
            //    握ったまま値を取っていた。中断を入れた今、その中で本体を走らせると
            //    `RefCell already mutably borrowed` で**プロセスごと落ちる**。
            //    ⇒ `Rc` だけ複製して借用を落としてから `gen_next` へ。
            let gen_rc = match &buf[iter_idx] {
                Value::Generator(state) => Some(state.clone()),
                _ => None, // 非 Generator（カスタムイテレータ）はフォールバック
            };
            let next: Option<Option<Value>> = match gen_rc {
                Some(state) => Some(interp.gen_next(&state)?),
                None => None,
            };
            match next {
                Some(Some(val)) => {
                    // Python の物を `mut` でないループ変数で受けたら誤り（D-11・普通の値は判別子を見るだけ）
                    if matches!(val, Value::PyObject(_)) {
                        for_target_check(interp, chunk, ip, &val)?;
                    }
                    buf[base + *target_slot as usize] = val
                }
                Some(None) => return Ok(Flow::Jump(*exit_ip as usize)),
                None => {
                    // フォールバック: カスタムイテレータ等は .next() を呼ぶ。
                    let iter = buf[iter_idx].clone();
                    match interp.eval_method_call(iter, "next", &[], None) {
                        Ok(item) => {
                            if matches!(item, Value::PyObject(_)) {
                                for_target_check(interp, chunk, ip, &item)?;
                            }
                            buf[base + *target_slot as usize] = item
                        }
                        Err(ref e) if e.starts_with("EndOfIteration") => {
                            return Ok(Flow::Jump(*exit_ip as usize))
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        Op::Jump(t) => return Ok(Flow::Jump(*t as usize)),
        Op::JumpIfFalse(t) => {
            let c = buf.pop().unwrap();
            if !truthy_fast(interp, &c)? {
                return Ok(Flow::Jump(*t as usize));
            }
        }
        Op::JumpIfFalseOrPop(t) => {
            let truthy = truthy_fast(interp, buf.last().unwrap())?;
            if !truthy {
                return Ok(Flow::Jump(*t as usize));
            }
            buf.pop();
        }
        Op::JumpIfTrueOrPop(t) => {
            let truthy = truthy_fast(interp, buf.last().unwrap())?;
            if truthy {
                return Ok(Flow::Jump(*t as usize));
            }
            buf.pop();
        }
        Op::Call(argc, mut_mask, name_idx, span_idx, node_id) => {
            let n = *argc as usize;
            let split = buf.len() - n;
            // ⚠ `split_off` で一度 `Vec` に出すと、`evaled` と合わせて呼び出しのたびに 2 回確保する
            //   （`CallMethodLocal` と同じ・copy_alloc_speed_plan.md 6-3）。引数を取り出してから呼ぶ物を取る
            let mut evaled = interp.take_evaled_buf();
            evaled.extend(buf.drain(split..).enumerate().map(|(i, v)| (None, v, (mut_mask >> i) & 1 == 1)));
            let callee = buf.pop().unwrap();
            let _disjoint = note_disjoint_args(interp, *mut_mask, &evaled, *node_id);
            // native の `mut` ポインタ書き戻し（#48）。
            //
            // ⚠⚠ ここに置いてよいのは **discriminant 1 個の判定だけ**。`exec_op` は
            // `#[inline(always)]` なので、副表の `is_empty()`／ハッシュ引き／`has_writeback()`
            // （`ptr_params` の線形走査）をここへ展開すると、**native を 1 度も呼ばない
            // Chunk のホットループまで巻き添えで遅くなる**（実測: `fn_call` / `closure_call` /
            // `block_expr` が 0.91x。`f(mut x)` は普通に書かれるので副表はまず空にならない）。
            // 判定の残りと本体は `#[inline(never)]` の `native_call_with_wb` に置く。
            if let Value::NativeFunction(fn_ref) = &callee {
                // 計測（`--features prof`）: 呼び先本体を `Call` と別バケットにする。
                #[cfg(feature = "prof")]
                crate::prof::note_native_callee();
                let fn_ref = fn_ref.clone();
                let r = native_call_with_wb(
                    interp, chunk, buf, base, cells, &fn_ref, evaled, node_id,
                )?;
                buf.push(r);
                return Ok(Flow::NextAfterCall);
            }
            // 呼び出し元名・位置をトレースバック用に渡す（V-E）。
            // `node_id` は FFI 境界検査の宣言型キー（#22-b）。
            let r = interp.call_value_evaled(
                callee,
                evaled,
                &chunk.names[*name_idx as usize],
                Some(chunk.spans[*span_idx as usize].clone()),
                *node_id,
            )?;
            buf.push(r);
            return Ok(Flow::NextAfterCall); // #1: 呼び出し中に debug が始まった可能性を拾う
        }
        // 超命令（#16 段階(b)(i)）: レシーバを frame から直接取る。`CallMethod` と同一意味論。
        Op::CallMethodLocal(slot, name_idx, argc, mut_mask, node_id) => {
            let n = *argc as usize;
            let split = buf.len() - n;
            // ⚠ `split_off` で一度 `Vec` に出すと、`evaled` と合わせて呼び出しのたびに 2 回確保する（pandas_import_plan.md の 4-2）
            // 並びは置き場から取る（6-3）
            let mut evaled = interp.take_evaled_buf();
            evaled.extend(buf.drain(split..).enumerate().map(|(i, v)| (None, v, (mut_mask >> i) & 1 == 1)));
            let _disjoint = note_disjoint_args(interp, *mut_mask, &evaled, *node_id);
            // レシーバは呼び先の `self` として所有権が要るので clone は避けられないが、
            // `LoadLocal` の op ディスパッチと push/pop は省ける。
            let obj = buf[base + *slot as usize].clone();
            let name = &chunk.names[*name_idx as usize];
            let r = if matches!(obj, Value::Instance(_)) {
                interp.call_instance_method_evaled(
                    obj, name, evaled,
                    Some(&chunk.attr_caches[*name_idx as usize]), None,
                    crate::vm::op::recv_is_mut(*mut_mask, n),
                )?
            } else if let Some((fn_ref, wb)) = (matches!(obj, Value::Namespace(_)))
                .then(|| wb_native_method(chunk, &obj, name, node_id))
                .flatten()
            {
                // native の `mut` ポインタ書き戻し（#48）
                let (vals, flags) = interp.native_positional(&fn_ref, evaled)?;
                let mut out = Vec::new();
                let r = interp.dispatch_native_evaled_wb(&fn_ref, vals, &flags, wb.mask, &mut out)?;
                apply_writeback(interp, chunk, buf, base, cells, wb, out)?;
                r
            } else {
                interp.vm_method_call_other(obj, name, evaled, *node_id, chunk)?
            };
            buf.push(r);
            return Ok(Flow::NextAfterCall); // #1
        }
        Op::CallMethod(name_idx, argc, mut_mask, node_id) => {
            let n = *argc as usize;
            let split = buf.len() - n;
            let mut evaled = interp.take_evaled_buf();
            evaled.extend(buf.drain(split..).enumerate().map(|(i, v)| (None, v, (mut_mask >> i) & 1 == 1)));
            let _disjoint = note_disjoint_args(interp, *mut_mask, &evaled, *node_id);
            let obj = buf.pop().unwrap();
            // メソッドはツリーウォークと同じく call_span=None（degraded フレームで一致）。
            let name = &chunk.names[*name_idx as usize];
            let r = if matches!(obj, Value::Instance(_)) {
                interp.call_instance_method_evaled(
                    obj, name, evaled,
                    Some(&chunk.attr_caches[*name_idx as usize]), None,
                    crate::vm::op::recv_is_mut(*mut_mask, n),
                )?
            } else if let Some((fn_ref, wb)) = (matches!(obj, Value::Namespace(_)))
                .then(|| wb_native_method(chunk, &obj, name, node_id))
                .flatten()
            {
                // native の `mut` ポインタ書き戻し（#48）。`mod.func(mut x)` の形はここに来る。
                let (vals, flags) = interp.native_positional(&fn_ref, evaled)?;
                let mut out = Vec::new();
                let r = interp.dispatch_native_evaled_wb(&fn_ref, vals, &flags, wb.mask, &mut out)?;
                apply_writeback(interp, chunk, buf, base, cells, wb, out)?;
                r
            } else {
                interp.vm_method_call_other(obj, name, evaled, *node_id, chunk)?
            };
            buf.push(r);
            return Ok(Flow::NextAfterCall); // #1
        }
        // キーワード／可変長引数つきのメソッド呼び出し（#27-c）。
        // `CallMethod` との違いは**引数名を添えること**だけ（dispatcher は同一）。
        Op::CallMethodKw(kw_idx) => {
            let kw = &chunk.kw_calls[*kw_idx as usize];
            let n = kw.argc as usize;
            let split = buf.len() - n;
            let arg_vals = buf.split_off(split);
            let obj = buf.pop().unwrap();
            // 受け手の可変性は展開前の引数の個数で読む（6-1）
            let recv_mut = crate::vm::op::recv_is_mut(kw.mut_mask, n);
            let evaled: Vec<(Option<String>, Value, bool)> = arg_vals
                .into_iter()
                .enumerate()
                .map(|(i, v)| (kw.arg_names[i].clone(), v, (kw.mut_mask >> i) & 1 == 1))
                .collect();
            // ★ メソッド呼び出しの `o.m(*xs)` / `o.m(**d)` も同じ規則で広げる。
            let evaled = interp.expand_spread_args(evaled)?;
            let name = &chunk.names[kw.name_idx as usize];
            let r = if matches!(obj, Value::Instance(_)) {
                interp.call_instance_method_evaled(
                    obj,
                    name,
                    evaled,
                    Some(&chunk.attr_caches[kw.name_idx as usize]),
                    None,
                    recv_mut,
                )?
            } else {
                interp.vm_method_call_other(obj, name, evaled, kw.node_id, chunk)?
            };
            buf.push(r);
            return Ok(Flow::NextAfterCall); // #1
        }
        Op::Return => return Ok(Flow::Return(buf.pop().unwrap())),
        Op::ReturnNil => return Ok(Flow::Return(Value::None)),
        // ── 例外処理（Phase V-C） ──
        Op::SetupTry(handler_ip) => {
            handlers.push(Handler {
                handler_ip: *handler_ip as usize,
                stack_len: buf.len(),
            });
        }
        Op::PopTry => {
            handlers.pop();
        }
        Op::Raise(span_idx) => {
            let exc = buf.pop().unwrap();
            let sentinel = interp.vm_raise(exc, &chunk.spans[*span_idx as usize]);
            return Err(sentinel);
        }
        Op::Reraise => {
            let sentinel = interp.vm_reraise();
            return Err(sentinel);
        }
        Op::Dup => {
            let v = buf.last().unwrap().clone();
            buf.push(v);
        }
        Op::ExcMatch(name_idx) => {
            let v = buf.pop().unwrap();
            let r = interp.vm_exc_matches(&v, &chunk.names[*name_idx as usize]);
            buf.push(Value::Bool(r));
        }
        // #34: 実行時に必ず失敗する文（囲むループの無い break/continue）を、
        // ツリーウォークと**一字一句同じ**メッセージで落とす。
        Op::Fail(idx) => return Err(chunk.names[*idx as usize].clone()),
        // #35: block_return / loop_yield の実行時型検査。どちらも**値を消費しない**
        // （直後に StoreLocal / ListAppendLocal が使う）。判定はツリーウォークと同じ 1 実装へ委譲。
        // #42: モジュール本体の代入。`assign_var` はスコープチェーンを探すので
        // `StoreGlobal`（`scopes[0]` 限定）と違い push 済みスコープの名前にも当たる。
        Op::StoreName(idx) => {
            let v = buf.pop().unwrap();
            interp.vm_assign_by_name(&chunk.names[*idx as usize], v)?;
        }
        // #43: 種別が合えばここで終わり（文字列に触らない）。外れたときだけ一般判定へ落として
        // **同じ実装から同じ文言**のエラーを出す（`Other` は常に一般判定）。
        Op::CheckBlockReturn(idx, tag) => {
            let v = buf.last().unwrap();
            if !tag.matches(v) {
                interp.check_block_return_type(v, &chunk.names[*idx as usize])?;
            }
        }
        Op::CheckLoopYield(idx, tag) => {
            let v = buf.last().unwrap();
            if !tag.matches(v) {
                interp.check_loop_yield_type(v, &chunk.names[*idx as usize])?;
            }
        }
        Op::BuildEmptyList => buf.push(Value::List(Rc::new(RefCell::new(Vec::new())))),
        Op::ListAppendLocal(slot) => {
            let v = buf.pop().unwrap();
            if let Value::List(list) = &buf[base + *slot as usize] {
                // ⚠ 格納する値は複製する（L4）。ツリーウォークの `append` と同じ規則で
                //    ないと、VM 経路だけ共有が残る。Python のコードの中は写さない（known_bugs.md の #1）。
                let copied = interp.copy_for_store(v);
                list.borrow_mut().push(copied);
            }
        }
        Op::ListOrNone => {
            let list = buf.pop().unwrap();
            let empty = matches!(&list, Value::List(l) if l.borrow().is_empty());
            buf.push(if empty { Value::None } else { list });
        }
        Op::LoadName(name_idx) => {
            let name = &chunk.names[*name_idx as usize];
            match interp.vm_load_name(name) {
                Some(v) => buf.push(v),
                None => return Err(format!("NameError: '{name}' is not defined")),
            }
        }
        Op::DeclareName(name_idx) => {
            let v = buf.pop().unwrap();
            interp.vm_declare_debug(&chunk.names[*name_idx as usize], v)?;
        }
        Op::MakeFn(idx) => {
            // #27: 入れ子 `fn` 定義。本体は `#[inline(never)]`（`exec_op` を太らせない）。
            make_fn(interp, chunk, buf, base, *idx, cells);
        }
        Op::EnumDef(idx) => {
            // #68: 関数本体の `enum` 定義。本体は `#[inline(never)]`。
            enum_def_op(interp, chunk, buf, base, *idx)?;
        }
        // ── `static mut`（#27-d）。記憶域は `Interpreter::static_cells`（span キー） ──
        // `exec_static_var` と同じく、**セルが既にあれば初期化子を評価しない**。
        Op::StaticInit(span_idx, after_init) => {
            if interp.vm_static_cell(&chunk.spans[*span_idx as usize]).is_some() {
                return Ok(Flow::Jump(*after_init as usize));
            }
        }
        Op::StaticStore(span_idx) => {
            let v = buf.pop().unwrap();
            interp.vm_static_create(&chunk.spans[*span_idx as usize], v);
        }
        Op::LoadStatic(span_idx) => {
            let span = &chunk.spans[*span_idx as usize];
            match interp.vm_static_cell(span) {
                Some(cell) => {
                    let v = cell.borrow().clone();
                    buf.push(v);
                }
                // `StaticInit` を必ず先に通るので理論上起きない。
                None => return Err("NameError: static variable is not initialized".to_string()),
            }
        }
        Op::StoreStatic(span_idx) => {
            let v = buf.pop().unwrap();
            let span = &chunk.spans[*span_idx as usize];
            match interp.vm_static_cell(span) {
                Some(cell) => *cell.borrow_mut() = v,
                None => return Err("NameError: static variable is not initialized".to_string()),
            }
        }
        // ── セル変数（#27-d 段階 2b）。共有相手（外側フレーム／クロージャ）から見える ──
        Op::LoadCell(i) => {
            let v = cells[*i as usize].borrow().clone();
            buf.push(v);
        }
        Op::StoreCell(i) => {
            let v = buf.pop().unwrap();
            *cells[*i as usize].borrow_mut() = v;
        }
        Op::StoreCellDeepCopy(i) => {
            let v = interp.bind_copy(BindCopy::DeepCopy, buf.pop().unwrap())?;
            *cells[*i as usize].borrow_mut() = v;
        }
        Op::UnpackTuple(src, n) => {
            // #27-c: `for k, v in ...`。本体は `#[inline(never)]`（`exec_op` を太らせない）。
            unpack_tuple(buf, base, *src, *n)?;
        }
        Op::CallKw(i) => {
            // #27-c: キーワード/可変長引数つき呼び出し。スタック配置は `Op::Call` と同じで、
            // 引数名だけ副表から取る（`eval_call_args` が作る 3 つ組と同じ形にする）。
            let kc = &chunk.kw_calls[*i as usize];
            let n = kc.argc as usize;
            let split = buf.len() - n;
            let arg_vals = buf.split_off(split);
            let callee = buf.pop().unwrap();
            let evaled: Vec<(Option<String>, Value, bool)> = arg_vals
                .into_iter()
                .enumerate()
                .map(|(j, v)| (kc.arg_names[j].clone(), v, (kc.mut_mask >> j) & 1 == 1))
                .collect();
            // ★ `f(*xs)` / `f(**d)` の番兵を実際の引数列へ広げる（ツリーウォークと同じ規則）。
            let evaled = interp.expand_spread_args(evaled)?;
            let r = interp.call_value_evaled(
                callee,
                evaled,
                &chunk.names[kc.name_idx as usize],
                Some(chunk.spans[kc.span_idx as usize].clone()),
                kc.node_id,
            )?;
            buf.push(r);
            return Ok(Flow::NextAfterCall);
        }
        Op::CallTemplate(t, argc, mut_mask) => {
            // #27-c: `Tmpl[T](args)`。引数の作り方は `Op::Call` と同じ（位置引数のみ）。
            let n = *argc as usize;
            let split = buf.len() - n;
            let arg_vals = buf.split_off(split);
            let tmpl = buf.pop().unwrap();
            let evaled: Vec<(Option<String>, Value, bool)> = arg_vals
                .into_iter()
                .enumerate()
                .map(|(i, v)| (None, v, (mut_mask >> i) & 1 == 1))
                .collect();
            let r = interp.instantiate_template_evaled(
                tmpl,
                &chunk.type_arg_lists[*t as usize],
                evaled,
            )?;
            buf.push(r);
            return Ok(Flow::NextAfterCall); // 呼び出し中に debug が始まった可能性を拾う
        }
        Op::LetTuple(i) => {
            // #27-c: `let a, b = t`。本体は `#[inline(never)]`（`exec_op` を太らせない）。
            let v = buf.pop().unwrap();
            let_tuple(interp, chunk, buf, base, *i, v)?;
        }
        Op::FreezeVar(ni, si) => {
            // #27-c: `freeze x`。意味論はツリーウォークと同じ 1 実装。
            interp.exec_freeze(&chunk.names[*ni as usize], &chunk.spans[*si as usize])?;
        }
        Op::EventSubscribe(once, is_async) => {
            // #27-c: `src on/once handler`。
            let handler = buf.pop().unwrap();
            let source = buf.pop().unwrap();
            interp.event_subscribe_evaled(source, handler, *once, *is_async)?;
        }
        Op::EventUnsubscribe => {
            // #27-c: `src off handler`。
            let handler = buf.pop().unwrap();
            let source = buf.pop().unwrap();
            interp.event_unsubscribe_evaled(source, handler)?;
        }
        Op::BuildSlice => {
            // #27-c: `a[b:e:s]`。検査・エラー文言はツリーウォークと同じ 1 実装。
            let step = buf.pop().unwrap();
            let end = buf.pop().unwrap();
            let begin = buf.pop().unwrap();
            buf.push(interp.slice_from_values(begin, end, step)?);
        }
        Op::DeclareGlobal(ni, kind) => {
            // #10-c: 最上位の `let`/`mut`/`const`。本体は `#[inline(never)]`（`exec_op` を太らせない）。
            let v = buf.pop().unwrap();
            declare_global(interp, chunk, *ni, *kind, v)?;
        }
        Op::LoadSelfClass => {
            // #27: メソッド本体の `Self`。ツリーウォークが宣言する `Self` と同じ値
            // （`run_vm_method` が設定した `current_class`）。
            match interp.vm_self_class() {
                Some(v) => buf.push(v),
                None => return Err("NameError: 'Self' is not defined".to_string()),
            }
        }
        Op::GetTraitAttr(ti, ai) => {
            // #27: `obj::Trait.attr`。本体は `#[inline(never)]`（`exec_op` を太らせない）。
            let obj = buf.pop().unwrap();
            let v = get_trait_attr(interp, chunk, *ti, *ai, obj)?;
            buf.push(v);
        }
        Op::SetTraitAttr(ti, ai) => {
            // #27: `obj::Trait.attr = v`。スタックは `[obj, value]`（`SetAttr` と同順）。
            let v = buf.pop().unwrap();
            let obj = buf.pop().unwrap();
            set_trait_attr(interp, chunk, *ti, *ai, obj, v)?;
        }
        Op::BreakPoint(si) => {
            // #27: VM 内の `break_point`。
            // ⚠ **`exec_breakpoint` を直接呼んではいけない**。それだと REPL から
            //    このフレームのローカルが見えず `NameError` になる（off/auto 不一致・実際に踏んだ）。
            //    #1 の `vm_debug_pause` が「flat buffer の slot を一時スコープへ移す」処理を
            //    持っているので、停止は必ずそこを通す。
            // ⚠ `NextAfterCall` を返すこと。ここでデバッグセッションが始まるので、
            //    `Next` だとこのフレームが停止判定を持たないまま走り抜ける（#1 の既存バグと同型）。
            breakpoint_op(interp, chunk, buf, base, ip, *si)?;
            return Ok(Flow::NextAfterCall);
        }
        Op::Subscript => {
            let key = buf.pop().unwrap();
            let obj = buf.pop().unwrap();
            let v = interp.eval_subscript(obj, key)?;
            buf.push(v);
        }
        Op::CheckPyRead(node_id) => {
            let v = buf.pop().unwrap();
            let v = interp.check_python_read_at(v, *node_id)?;
            buf.push(v);
        }
        Op::SetIndex => {
            let value = buf.pop().unwrap();
            let key = buf.pop().unwrap();
            let obj = buf.pop().unwrap();
            interp.eval_setitem(obj, key, value)?;
        }
        Op::MetaInfo(n) => {
            let name = &chunk.names[*n as usize];
            match interp.meta_lookup(name) {
                Some(v) => buf.push(v),
                None => return Err(interp.meta_lookup_error(name)),
            }
        }
        Op::MakeCode(c) => {
            let template = match &chunk.consts[*c as usize] {
                Value::Code(lines) => std::rc::Rc::clone(lines),
                other => {
                    return Err(format!(
                        "MetaError: MAKE_CODE expects a `Code` template, got '{}'",
                        interp.type_name(other)
                    ))
                }
            };
            let holes = crate::meta_expand::splice_count(&template);
            let split = buf.len() - holes;
            let vals = buf.split_off(split);
            buf.push(Value::Code(std::rc::Rc::new(
                crate::meta_expand::fill_splices(&template, &vals)?,
            )));
        }
        Op::BuildList(n) => {
            let split = buf.len() - *n as usize;
            let vals = buf.split_off(split);
            buf.push(interp.vm_build_list(vals));
        }
        Op::BuildTuple(n) => {
            let split = buf.len() - *n as usize;
            let vals = buf.split_off(split);
            buf.push(interp.vm_build_tuple(vals));
        }
        Op::BuildSet(n) => {
            let split = buf.len() - *n as usize;
            let vals = buf.split_off(split);
            buf.push(interp.vm_build_set(vals)?);
        }
        Op::BuildDict(n) => {
            let split = buf.len() - 2 * *n as usize;
            let flat = buf.split_off(split);
            buf.push(interp.vm_build_dict(flat)?);
        }
        Op::SeqFinish(kind) => {
            let acc = buf.pop().expect("SeqFinish: 蓄積リストがスタックに無い");
            buf.push(interp.vm_seq_finish(acc, *kind)?);
        }
        Op::SeqExtend => {
            let src = buf.pop().expect("SeqExtend: 展開元がスタックに無い");
            let dest = buf.last().expect("SeqExtend: 追加先がスタックに無い").clone();
            interp.vm_seq_extend(&dest, src)?;
        }
        Op::DictMerge => {
            let src = buf.pop().expect("DictMerge: 展開元がスタックに無い");
            let dest = buf.last().expect("DictMerge: 合成先がスタックに無い").clone();
            interp.vm_dict_merge(&dest, src)?;
        }
        Op::Yield => {
            // ⚠⚠ **ここでディスパッチループを抜ける**（bug_fix.md B13）。
            //    以前は共有バッファへ積むだけで本体は最後まで走っていた（先行評価）。
            let v = buf.pop().unwrap();
            return Ok(Flow::Suspend(v));
        }
        Op::AsyncSubmit(idx) => {
            let mgr = buf.pop().unwrap();
            let block = &chunk.async_blocks[*idx as usize];
            // 捕捉変数を frame の slot から読み出す（capture_env と同じ mutable/immutable 規則は
            // vm_async_submit 内で Var 経由に適用する）。
            let captured: Vec<(String, Value, bool)> = block
                .captures
                .iter()
                .map(|(name, slot, is_mut)| (name.clone(), buf[base + *slot as usize].clone(), *is_mut))
                .collect();
            interp.vm_async_submit(mgr, &block.body, captured)?;
        }
    }
    Ok(Flow::Next)
}
