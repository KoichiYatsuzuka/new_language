// vm/run/op_helpers.rs — `exec_op` から呼ぶ、まれな命令・遅い経路の本体。

use super::*;

/// 停止位置 `ip` の時点で「もう宣言されている」ローカル slot を返す（#1・デバッガ表示用）。
///
/// VM の flat buffer は**全 slot を `None` で初期化する**ので、値だけでは
/// 「未宣言」と「`None` を代入済み」を区別できない。ツリーウォークは宣言文を実行して初めて
/// 変数がスコープに入るので、合わせないと「まだ宣言していない変数が REPL から引ける」ことになり
/// off/auto がずれる。
///
/// **`code[..ip]` を静的に走査**して「そこまでに書き込み得た slot」を集める。
/// 実行時に追跡しないのは 2 つの理由から:
/// - 通常ループにも stepping ループにも per-op の仕事を足さずに済む。
/// - **セッション開始前から走っていたフレーム**（`Flow::NextAfterCall` で途中から stepping へ
///   切り替わったフレーム）では、それまでの代入を実行時には知りようがない。
///
/// 分岐で実行されなかった宣言も「宣言済み」と見なす過大近似だが、これは
/// 「このソース位置より前に宣言文がある」と同義で、ツリーウォークの見え方に十分近い。
/// 停止時にしか呼ばれないので O(ip) のコストは問題にならない。
pub(super) fn declared_slots(chunk: &Chunk, ip: usize) -> Vec<bool> {
    let mut declared = vec![false; chunk.n_locals];
    for d in declared.iter_mut().take(chunk.n_params) {
        *d = true; // パラメータは呼び出し側が束縛済み
    }
    for op in chunk.code.iter().take(ip) {
        let slot = match op {
            Op::StoreLocal(s)
            | Op::StoreLocalDeepCopy(s)
            | Op::StoreLocalCopyFreeze(s)
            | Op::StoreLocalFreezeInstance(s)
            | Op::StoreLocalFromIdent(s, _)
            | Op::StoreLocalImplicit(s, _)
            | Op::ListAppendLocal(s) => *s as usize,
            Op::ForIter(_, target, _) => *target as usize,
            _ => continue,
        };
        if let Some(d) = declared.get_mut(slot) {
            *d = true;
        }
    }
    declared
}

/// `Op::ForIter` の要素が Python の物のとき: このループが `mut` でない 1 つの変数の `for` なら誤り（D-11・2-10）。
/// ⚠ `#[inline(never)]`（`exec_op` は `#[inline(always)]`・遅い道を本体に入れない）。
#[inline(never)]
pub(super) fn for_target_check(interp: &Interpreter, chunk: &Chunk, ip: usize, value: &Value) -> Result<(), String> {
    if chunk.builtin_prelude {
        return Ok(());
    }
    match chunk.implicit_for_targets.binary_search_by_key(&(ip as u32), |(at, _)| *at) {
        Ok(k) => implicit_binding_check(interp, &chunk.names[chunk.implicit_for_targets[k].1 as usize], value),
        Err(_) => Ok(()),
    }
}

/// `mut` と書かない暗黙の束縛（`binding` は `for:x` / `except:e`）が Python の物 `value` を受けた誤り（D-11・2-10・
/// 本体は `Interpreter::implicit_py_binding_error`）。
#[inline(never)]
pub(super) fn implicit_binding_check(interp: &Interpreter, binding: &str, value: &Value) -> Result<(), String> {
    match interp.implicit_py_binding_error(binding, value) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// `Op::StoreGlobal` の**ミス経路**（#10-b）。初回・キャッシュ失効時だけ通る。
///
/// ツリーウォークの `Stmt::Assign` と同じ機構: `assign_var`（可変性検査・Cell 種別・
/// `NameError`）を通し、`try_fill_slot` が対象を `Var::SlotCell` へ昇格して
/// `global_slot_cells` の index を焼く。以後は `exec_op` 側のヒット経路が直接書き込む。
///
/// ⚠ **`#[inline(never)]` を外さないこと。** `exec_op` は `#[inline(always)]` なので、
/// ここを展開すると `StoreGlobal` を含まない Chunk のホットループまで巻き添えで遅くなる
/// （実測 4〜6%）。逆に**ヒット経路まで外へ出すと**、最上位ループが毎ストアで
/// 呼び出しを払って別のベンチが 7〜10% 落ちた。**分割する**のが両立点。
#[inline(never)]
pub(super) fn store_global_miss(
    interp: &mut Interpreter,
    chunk: &Chunk,
    ni: u32,
    cache: &crate::ast::SlotCache,
    v: Value,
) -> Result<(), String> {
    let name = &chunk.names[ni as usize];
    interp.vm_assign_global(name, v)?;
    interp.vm_fill_global_store_cache(name, cache);
    Ok(())
}

/// `Op::Call` の呼び先が `NativeFunction` だったときの本体（#48）。
///
/// `call_value_evaled` の `NativeFunction` アームと**同じ意味**（＝`dispatch_native_evaled`）に、
/// 書き戻しがある場合の副表引きと格納を足しただけ。
///
/// ⚠ `#[inline(never)]`。副表のハッシュ引きも `has_writeback()`（`ptr_params` の走査）も
/// ここに閉じ込める — `exec_op` は `#[inline(always)]` なので、展開すると native を
/// 使わない Chunk まで遅くなる（#10-b と同じ失敗を #48 で一度踏んだ）。
#[inline(never)]
pub(super) fn native_call_with_wb(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut [Value],
    base: usize,
    cells: &[Rc<RefCell<Value>>],
    fn_ref: &std::sync::Arc<crate::interpreter::value::NativeFnRef>,
    evaled: Vec<(Option<String>, Value, bool)>,
    node_id: &u32,
) -> Result<Value, String> {
    let wb = if fn_ref.has_writeback() {
        chunk.wb_targets.get(node_id)
    } else {
        None
    };
    let Some(wb) = wb else {
        return interp.dispatch_native_evaled_flags(fn_ref, evaled);
    };
    let (vals, flags) = interp.native_positional(fn_ref, evaled)?;
    let mut out = Vec::new();
    let r = interp.dispatch_native_evaled_wb(fn_ref, vals, &flags, wb.mask, &mut out)?;
    apply_writeback(interp, chunk, buf, base, cells, wb, out)?;
    Ok(r)
}

/// `mod.func(mut x)` が **書き戻しを持つ native 呼び出し**かを判定する（#48）。
///
/// `Some((呼び先, 書き戻し表))` を返したときだけ VM が `dispatch_native_evaled_wb` を
/// 直接呼ぶ。外れたら従来どおり `vm_method_call_other` に委ねる（挙動は変わらない）。
///
/// ⚠ **判定は「レシーバの形」から始める**（`Value::Namespace` か = discriminant 1 個）。
/// 副表のハッシュ引きを先に置くと、str/list/dict のメソッド呼び出し全部が毎回それを払う。
/// `Value::Instance` の通常のメソッド呼び出しはここに来ない
/// （呼び出し側が先に `matches!(obj, Value::Instance(_))` で分岐している）。
#[inline(never)]
pub(super) fn wb_native_method<'c>(
    chunk: &'c Chunk,
    obj: &Value,
    name: &str,
    node_id: &u32,
) -> Option<(
    std::sync::Arc<crate::interpreter::value::NativeFnRef>,
    &'c crate::vm::chunk::WbCall,
)> {
    // ⚠ 書き戻し先の表を先に見る（普通は空で、名前を引かずに済む）
    let wb = chunk.wb_targets.get(node_id)?;
    let fn_ref = Interpreter::vm_namespace_writeback_fn(obj, name)?;
    Some((fn_ref, wb))
}

/// native の `mut` ポインタ引数の**書き戻しを格納する**（#48）。
///
/// `dispatch_native_evaled_wb` が返した `(arg index, 値)` を、コンパイル時に確定した
/// 書き戻し先（[`WbStore`](crate::vm::chunk::WbStore)）へ書く。
/// ツリーウォークが `assign_var(名前)` でやっていたことの slot 版。
///
/// ⚠ **各アームは対応する `Op::Store*` と同じ意味でなければならない**
/// （`Local`→`StoreLocal` / `Cell`→`StoreCell` / `Global`→`StoreGlobal` /
/// `Name`→`StoreName`）。回帰検知は
/// `examples/interop/cpp_out_param_writeback.ar`（記憶域の種類ごとに 1 本ずつ置き、
/// **期待値と違えば raise する**ので exit code しか見ないゲートにも映る）。
/// ⚠ `#[inline(never)]`（`exec_op` は `#[inline(always)]` — #10-b の教訓）。
#[inline(never)]
pub(super) fn apply_writeback(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut [Value],
    base: usize,
    cells: &[Rc<RefCell<Value>>],
    wb: &crate::vm::chunk::WbCall,
    out: Vec<(u8, Value)>,
) -> Result<(), String> {
    use crate::vm::chunk::WbStore;
    for (arg_idx, v) in out {
        // `targets` は数個なので線形探索で足りる（呼び出し 1 回につき 1〜2 件）。
        let Some((_, store)) = wb.targets.iter().find(|(i, _)| *i == arg_idx) else {
            continue;
        };
        match *store {
            WbStore::Local(s) => buf[base + s as usize] = v,
            WbStore::Cell(i) => *cells[i as usize].borrow_mut() = v,
            // 書き戻しは稀なので `StoreGlobal` のヒット経路は使わず、
            // **ミス経路の実装をそのまま共有する**（可変性検査・`NameError`・
            // `Var::SlotCell` 昇格まで `Op::StoreGlobal` と同一になる）。
            WbStore::Global(ni, ci) => {
                store_global_miss(interp, chunk, ni, &chunk.global_caches[ci as usize], v)?
            }
            WbStore::Name(ni) => interp.vm_assign_by_name(&chunk.names[ni as usize], v)?,
        }
    }
    Ok(())
}

/// `Op::EnumDef` の本体（#68）。関数本体の `enum` 定義。
///
/// ツリーウォークの `exec_enum_def` と**同じ `build_enum_classes`** を呼ぶので、
/// バリアント値の採番・明示値の評価（`eval_definition_expr`）・エラー文言が構造的に一致する。
/// 違うのは**記憶域だけ**: `Name` はフレームの slot へ入れる（リゾルバが base slot を採番済み。
/// 名前で宣言すると読みの `LoadLocal` が別の変数を読む）。
/// メンバーのクラスはどちらの経路でも名前で宣言しない（`build_enum_classes`）。
///
/// ⚠ `#[inline(never)]`（`exec_op` は `#[inline(always)]` — #10-b の教訓）。
#[inline(never)]
pub(super) fn enum_def_op(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut [Value],
    base: usize,
    idx: u32,
) -> Result<(), String> {
    let d = &chunk.enum_defs[idx as usize];
    buf[base + d.slot as usize] = interp.vm_enum_def(&d.name, &d.variants)?;
    Ok(())
}

/// `Op::MakeFn` の本体（#27）。入れ子 `fn` の関数値を作って slot へ書く。
///
/// ツリーウォークの `exec_fn_def`（デコレータ・テンプレートなし・キャプチャ空の経路）と同じ判断を、
/// **オーバーロード合成も含めて** `Interpreter::make_nested_fn_value` に集約して共有する。
/// ⚠ `#[inline(never)]`（`exec_op` は `#[inline(always)]` — #10-b の教訓）。
#[inline(never)]
pub(super) fn make_fn(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut [Value],
    base: usize,
    idx: u32,
    cells: &[Rc<RefCell<Value>>],
) {
    let d = &chunk.fn_defs[idx as usize];
    // 自分の名前と組の相手は値を写さない（name_scope_plan.md の 3-3・`CapturedVar::SelfFn` / `Sibling`）。
    // ⚠ 写すと定義した時点の枠（まだ `None`）が入り、`'NoneType' object is not callable` になっていた。
    //   また自分の値を写すと `FnValue → captured_env → FnValue` の循環になる。
    let marker = |n: &str| -> Option<crate::interpreter::CapturedVar> {
        if n == d.name {
            Some(crate::interpreter::CapturedVar::SelfFn)
        } else {
            d.group.iter().position(|m| m == n).map(crate::interpreter::CapturedVar::Sibling)
        }
    };
    let markers: Vec<(String, crate::interpreter::CapturedVar)> =
        d.captures.iter().filter_map(|(n, _)| marker(n).map(|m| (n.clone(), m))).collect();
    // 不変キャプチャは**生成時点の値を複製**する（ツリーウォークの `capture_env` の不変分岐と同じ）。
    let captured: Vec<(String, Value)> = d
        .captures
        .iter()
        .filter(|(n, _)| marker(n).is_none())
        .map(|(n, s)| (n.clone(), buf[base + *s as usize].clone()))
        .collect();
    // 可変キャプチャは**セルを共有**する（#27-d 段階 2b）。`Rc` を clone するだけなので
    // 外側フレームの書き込みがクロージャから見え、その逆も見える（`capture_env` の可変分岐と同じ）。
    let mut cell_captured: Vec<(String, Rc<RefCell<Value>>)> = d
        .cell_captures
        .iter()
        .filter_map(|(n, i)| cells.get(*i as usize).map(|c| (n.clone(), c.clone())))
        .collect();
    // `static mut` のキャプチャはセルが `Interpreter::static_cells` にある（span がキー）。
    for (n, span_idx) in &d.static_captures {
        if let Some(cell) = interp.vm_static_cell(&chunk.spans[*span_idx as usize]) {
            cell_captured.push((n.clone(), cell));
        }
    }
    let slot = base + d.slot as usize;
    let existing = std::mem::replace(&mut buf[slot], Value::None);
    // 入れ子 `gen`（B13 段階 E）。捕捉の集め方はここまで完全に同じで、作る値だけが違う。
    if d.is_generator {
        buf[slot] = interp.make_nested_gen_value(
            &d.name,
            &d.params,
            &d.body,
            captured,
            cell_captured,
            d.qualname.clone(),
        );
        add_capture_markers(&mut buf[slot], markers);
        if let Some(first) = d.group_first {
            link_fn_group(chunk, buf, base, first, d.group.len());
        }
        return;
    }
    buf[slot] = interp.make_nested_fn_value(
        &d.name,
        &d.params,
        // #45: AST は複製せず定義サイトの `Rc` を共有する（参照カウント +1 だけ）。
        d.body.clone(),
        d.return_type.as_deref(),
        captured,
        cell_captured,
        existing,
        // #30: 実体ごとに再コンパイルせず、定義サイトの器を全実体で共有する。
        Some(d.compiled.clone()),
        d.qualname.clone(),
    );
    add_capture_markers(&mut buf[slot], markers);
    if let Some(first) = d.group_first {
        link_fn_group(chunk, buf, base, first, d.group.len());
    }
}

/// 作ったばかりの入れ子の関数の値の `captured_env` に、自分・組の相手の印を足す（name_scope_plan.md の 3-3）。
///
/// ⚠ 作ったばかりで他に持ち主がいない（`Rc::get_mut` が通る）。同じ名前の関数と合成されて多重定義
///   （`OverloadedFn`）になった値には足さない（組の相手・自己再帰は今までどおり見えない）。
fn add_capture_markers(v: &mut Value, markers: Vec<(String, crate::interpreter::CapturedVar)>) {
    if markers.is_empty() {
        return;
    }
    let env = match v {
        Value::Function(rc) => Rc::get_mut(rc).map(|f| &mut f.captured_env),
        Value::GeneratorFn(rc) => Rc::get_mut(rc).map(|g| &mut g.captured_env),
        _ => None,
    };
    if let Some(env) = env {
        env.extend(markers);
    }
}

/// 組の最後のメンバーを作った直後に、組を繋ぐ（name_scope_plan.md の 3-3）。
///
/// 組（`FnGroup`）には各メンバーの**組を持たない写し**を入れ、枠のメンバーの値に組を持たせる。
/// ⚠ 組は写しだけを持つので、組 → メンバー → 組 の循環にならない。
/// ⚠ メンバーが関数の値でない（多重定義に合成された）ときは繋がない（相手は今までどおり見えない）。
fn link_fn_group(chunk: &Chunk, buf: &mut [Value], base: usize, first: u32, len: usize) {
    use crate::interpreter::{FnGroup, GroupMember};
    let slots: Vec<usize> =
        (0..len).map(|k| base + chunk.fn_defs[first as usize + k].slot as usize).collect();
    let mut members = Vec::with_capacity(len);
    for &s in &slots {
        match &buf[s] {
            Value::Function(f) => members.push(GroupMember::Fn(Rc::new((**f).clone()))),
            Value::GeneratorFn(g) => members.push(GroupMember::Gen(Rc::new((**g).clone()))),
            _ => return,
        }
    }
    let group = Rc::new(FnGroup { members });
    for &s in &slots {
        match &mut buf[s] {
            Value::Function(f) => Rc::make_mut(f).group = Some(group.clone()),
            Value::GeneratorFn(g) => Rc::make_mut(g).group = Some(group.clone()),
            _ => {}
        }
    }
}

/// `Op::UnpackTuple` の本体（#27-c）。`for k, v in ...` の要素をスタックへ積む。
///
/// ⚠ 検査とエラー文言は #33 まで `exec_for_stmt`（削除済み）と揃えていたもの。**今は唯一の実装**。
/// 基準は参照実装の出力（`compare_python_impl.ps1`）。
/// ⚠ `#[inline(never)]`（`exec_op` は `#[inline(always)]` — #10-b の教訓）。
#[inline(never)]
pub(super) fn unpack_tuple(buf: &mut Vec<Value>, base: usize, src: u16, n: u16) -> Result<(), String> {
    let item = &buf[base + src as usize];
    let Value::Tuple(td) = item else {
        return Err("TypeError: cannot unpack non-tuple value in for loop".to_string());
    };
    if td.len() != n as usize {
        return Err(format!(
            "ValueError: not enough values to unpack (expected {}, got {})",
            n,
            td.len()
        ));
    }
    let elems = td.all_values().to_vec();
    buf.extend(elems);
    Ok(())
}

/// `Op::LetTuple` の本体（#27-c）。検査は `let_tuple_values`（ツリーウォークと同じ 1 実装）。
/// ⚠ `#[inline(never)]`（`exec_op` は `#[inline(always)]` — #10-b の教訓）。
#[inline(never)]
pub(super) fn let_tuple(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut [Value],
    base: usize,
    idx: u32,
    v: Value,
) -> Result<(), String> {
    let decl = &chunk.tuple_decls[idx as usize];
    if decl.slots.is_empty() {
        // 最上位の宣言文: スコープへ宣言する（「既に宣言済み」検査もそちらが行う）。
        interp.exec_let_tuple_evaled(&decl.targets, v)?;
        return Ok(());
    }
    for (i, val) in interp.let_tuple_values(&decl.targets, v)? {
        if let Some(slot) = decl.slots[i] {
            buf[base + slot as usize] = val;
        }
    }
    Ok(())
}

/// `Op::DeclareGlobal` の本体（#10-c）。ツリーウォークと同じ `vm_declare_global` へ委譲する。
/// ⚠ `#[inline(never)]`（`exec_op` は `#[inline(always)]` — #10-b の教訓）。
#[inline(never)]
pub(super) fn declare_global(
    interp: &mut Interpreter,
    chunk: &Chunk,
    ni: u32,
    kind: crate::vm::op::DeclKind,
    v: Value,
) -> Result<(), String> {
    let name = &chunk.names[ni as usize];
    interp.vm_declare_global(name, kind, &chunk.names, v)
}

/// `Op::GetTraitAttr` の本体（#27）。ツリーウォークと同じ `trait_access_evaled` へ委譲する。
/// ⚠ `#[inline(never)]`（`exec_op` は `#[inline(always)]` — #10-b の教訓）。
#[inline(never)]
pub(super) fn get_trait_attr(
    interp: &mut Interpreter,
    chunk: &Chunk,
    ti: u32,
    ai: u32,
    obj: Value,
) -> Result<Value, String> {
    let trait_name = &chunk.names[ti as usize];
    let attr = &chunk.names[ai as usize];
    interp.trait_access_evaled(obj, trait_name, attr)
}

/// `Op::SetTraitAttr` の本体（#27）。ツリーウォークと同じ `trait_assign_evaled` へ委譲する。
/// ⚠ `#[inline(never)]`（同上）。
#[inline(never)]
pub(super) fn set_trait_attr(
    interp: &mut Interpreter,
    chunk: &Chunk,
    ti: u32,
    ai: u32,
    obj: Value,
    v: Value,
) -> Result<(), String> {
    let trait_name = &chunk.names[ti as usize];
    let attr = &chunk.names[ai as usize];
    interp.trait_assign_evaled(obj, trait_name, attr, v)
}

/// `Op::BreakPoint` の本体（#27）。デバッガ REPL へ入る。
///
/// ⚠ **`#[inline(never)]`**。`exec_op` は `#[inline(always)]` なので、
/// デバッガを使わない Chunk のホットループを巻き添えにしないため外へ出す（#10-b の教訓）。
#[inline(never)]
pub(super) fn breakpoint_op(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &[Value],
    base: usize,
    ip: usize,
    si: u32,
) -> Result<(), String> {
    let span = chunk.spans[si as usize].clone();
    // `run_stepping` の停止と同じ経路（#1）。`declared_slots` は「この ip までに
    // 書き込み得た slot」＝ツリーウォークで宣言済みの変数に対応する。
    let declared = declared_slots(chunk, ip);
    interp.vm_debug_pause(chunk, buf, base, &span, &declared)
}
