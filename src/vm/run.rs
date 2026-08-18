// vm/run.rs — バイトコード VM のディスパッチループ（Phase V, V-A）。
//
// 値スタックは呼び出し側が持つ単一バッファ（`buf`）を共有し、per-call 確保を避ける。
// フレームローカルは `buf[base .. base+n_locals]`、オペランドスタックはその上（`buf[base+n_locals ..]`）。
//
// 算術・比較は int/float の高速パスを VM ループ内にインライン展開し、それ以外
// （文字列・インスタンス演算子・混在型・ゼロ除算など）は既存の `apply_binop_dyn` へ委譲する。
// 高速パスは `apply_binop` の該当アームと**同一のセマンティクス**（`a + b` 等、オーバーフロー
// 挙動も含め）で書く。属性読み・真偽判定・単項も既存実装へ委譲するので結果はツリーウォークと一致する。

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::BinOp;
use crate::interpreter::{Interpreter, Value};

use super::chunk::Chunk;
use super::op::Op;

/// 1 命令実行後の制御フロー。
enum Flow {
    /// 次の命令へ（ip += 1）。
    Next,
    /// 次の命令へ（ip += 1）。**ただし直前に他のコードを実行した**（呼び出し系 op）。
    ///
    /// `Next` と分けているのは、呼び出しの中で `break_point` が発火して
    /// **デバッグセッションが始まっている可能性がある**のがここだけだから（#1）。
    /// このフレームは既に走り始めているので `run` 入口の判定を通過済みで、
    /// ここで拾わないと停止判定を持たないまま最後まで走ってしまう。
    /// 呼び出しは元々数百 ns かかるので、この再判定のコストは無視できる。
    NextAfterCall,
    /// 絶対 index へジャンプ。
    Jump(usize),
    /// 関数から値を返す。
    Return(Value),
}

/// フレームのセル表を作る（#27-d 段階 2b）。
///
/// - **可変キャプチャ**（`chunk.captured_cells`）は `captured_env` のセルを**そのまま入れる**
///   （`Rc` を clone するだけ＝外側と同じセルを指す）。ツリーウォークが `Var::Cell` を
///   共有するのと同じ効果。
/// - それ以外の index は**呼び出しごとに新しいセル**（入れ子 `fn` に可変キャプチャされる
///   自分のローカル用。呼び出しごとに別のセルになるのもツリーウォークと同じ）。
///
/// ⚠ `n_cells == 0`（大多数の関数）では `Vec::new()` を返すので**確保は起きない**。
#[inline]
fn build_cells(
    chunk: &Chunk,
    captured_env: Option<&std::collections::HashMap<String, crate::interpreter::CapturedVar>>,
) -> Vec<Rc<RefCell<Value>>> {
    if chunk.n_cells == 0 {
        return Vec::new();
    }
    let mut cells: Vec<Rc<RefCell<Value>>> = (0..chunk.n_cells)
        .map(|_| Rc::new(RefCell::new(Value::None)))
        .collect();
    if let Some(env) = captured_env {
        for (name, idx) in &chunk.captured_cells {
            if let Some(crate::interpreter::CapturedVar::Mutable(cell)) = env.get(name) {
                if let Some(slot) = cells.get_mut(*idx as usize) {
                    *slot = cell.clone(); // ⚠ 中身ではなく **Rc を共有**する
                }
            }
        }
    }
    cells
}

/// アクティブな例外ハンドラ（try 節ごとに VM のハンドラスタックに積む）。
struct Handler {
    /// 例外発生時に飛ぶ landing pad の ip。
    handler_ip: usize,
    /// try 進入時のオペランドスタック深さ（例外時にここまで巻き戻す）。
    stack_len: usize,
}

/// Chunk を実行して戻り値を返す。
/// `buf` の `base..base+n_locals` にパラメータが束縛済み。実行後 `buf` は base+n_locals..（オペランド）
/// を空にして返る（呼び出し側が `truncate(base)` する）。
pub fn run(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut Vec<Value>,
    base: usize,
    // クロージャの捕捉環境（#27-d 段階 2b）。可変キャプチャのセルを**共有**するために要る。
    // 捕捉を持たない呼び出し（大多数）は `None`。
    captured_env: Option<&std::collections::HashMap<String, crate::interpreter::CapturedVar>>,
) -> Result<Value, String> {
    let mut ip: usize = 0;
    let mut handlers: Vec<Handler> = Vec::new();
    // セル表（#27-d 段階 2b）。`n_cells == 0` の関数（大多数）は確保しない。
    let cells = build_cells(chunk, captured_env);

    // デバッグセッション中はステップ判定つきのループへ（#1）。
    // ⚠ **通常経路には何も足さない**のがこの分岐の目的。文境界ごとの停止判定を
    // このループに入れると、#12/#2b/#2a で削ったコストを毎命令ぶん戻すことになる。
    if crate::interpreter::debugger::dbg_active() {
        return run_stepping(interp, chunk, buf, base, ip, handlers, cells);
    }

    loop {
        match exec_op(interp, chunk, buf, base, ip, &mut handlers, &cells) {
            Ok(Flow::Next) => ip += 1,
            Ok(Flow::Jump(t)) => ip = t,
            Ok(Flow::Return(v)) => return Ok(v),
            // 呼び出しから戻った直後にデバッグセッションが始まっていることがある
            // （ネストした `break_point`）。**この再判定が無いと、既に走っている VM フレームが
            // 停止判定を持たないまま最後まで走り抜ける**（step-out が効かない実バグだった）。
            // 検査は「呼び出しから戻った直後」だけなので、通常のホット命令には影響しない。
            Ok(Flow::NextAfterCall) => {
                ip += 1;
                if crate::interpreter::debugger::dbg_active() {
                    return run_stepping(interp, chunk, buf, base, ip, handlers, cells);
                }
            }
            Err(e) => {
                // 例外: 最内ハンドラがあればオペランドを巻き戻して例外値を積み landing pad へ。
                // 変換できない（active exception なし等）・ハンドラなしなら伝播（Err を返す）。
                match handlers.pop() {
                    Some(h) => {
                        buf.truncate(h.stack_len);
                        match interp.vm_take_raised(&e) {
                            Some(exc_val) => {
                                buf.push(exc_val);
                                ip = h.handler_ip;
                            }
                            None => return Err(e),
                        }
                    }
                    None => return Err(e),
                }
            }
        }
    }
}

/// **文単位のステップ判定つき**ディスパッチループ（#1）。
///
/// 通常の `run` と同じ `exec_op` を回し、各命令の実行前に
/// 「その ip が文の先頭なら停止判定」を挟むだけ。位置は**ツリーウォークの `exec()` 冒頭と同じ**
/// （文を実行する前）なので、`--vm=off` と同じ順序で止まる。
///
/// このループへ入るのは 2 経路:
/// - `run` の入口でデバッグセッション中だったとき
/// - 呼び出しから戻ったらセッションが始まっていたとき（`Flow::NextAfterCall`）
///
/// ⚠ 通常経路（`run` のループ）には停止判定を**一切入れない**。入れると全命令に
/// 分岐が乗り、#12/#2b/#2a で削ったコストが戻る。
fn run_stepping(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut Vec<Value>,
    base: usize,
    mut ip: usize,
    mut handlers: Vec<Handler>,
    cells: Vec<Rc<RefCell<Value>>>, // #27-d 段階 2b: 通常ループから引き継ぐ
) -> Result<Value, String> {
    loop {
        // 文の先頭か？（行テーブルは code と 1:1 なので O(1)）
        if let Some(&span_idx) = chunk.stmt_spans.get(ip) {
            if span_idx != super::chunk::NOT_STMT {
                if let Some(span) = interp.vm_should_pause(chunk, span_idx) {
                    let declared = declared_slots(chunk, ip);
                    interp.vm_debug_pause(chunk, buf, base, &span, &declared)?;
                }
            }
        }
        match exec_op(interp, chunk, buf, base, ip, &mut handlers, &cells) {
            Ok(Flow::Next) | Ok(Flow::NextAfterCall) => ip += 1,
            Ok(Flow::Jump(t)) => ip = t,
            Ok(Flow::Return(v)) => return Ok(v),
            Err(e) => match handlers.pop() {
                Some(h) => {
                    buf.truncate(h.stack_len);
                    match interp.vm_take_raised(&e) {
                        Some(exc_val) => {
                            buf.push(exc_val);
                            ip = h.handler_ip;
                        }
                        None => return Err(e),
                    }
                }
                None => return Err(e),
            },
        }
    }
}

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
fn declared_slots(chunk: &Chunk, ip: usize) -> Vec<bool> {
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
fn store_global_miss(
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

/// `Op::MakeFn` の本体（#27）。入れ子 `fn` の関数値を作って slot へ書く。
///
/// ツリーウォークの `exec_fn_def`（デコレータ・テンプレートなし・キャプチャ空の経路）と同じ判断を、
/// **オーバーロード合成も含めて** `Interpreter::make_nested_fn_value` に集約して共有する。
/// ⚠ `#[inline(never)]`（`exec_op` は `#[inline(always)]` — #10-b の教訓）。
#[inline(never)]
fn make_fn(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut [Value],
    base: usize,
    idx: u32,
    cells: &[Rc<RefCell<Value>>],
) {
    let d = &chunk.fn_defs[idx as usize];
    // 不変キャプチャは**生成時点の値を複製**する（ツリーウォークの `capture_env` の不変分岐と同じ）。
    let captured: Vec<(String, Value)> = d
        .captures
        .iter()
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
    );
}

/// `Op::UnpackTuple` の本体（#27-c）。`for k, v in ...` の要素をスタックへ積む。
///
/// 検査とエラー文言はツリーウォーク（`exec_for_stmt` の複数ターゲット分岐）と一字一句同じにする。
/// ⚠ `#[inline(never)]`（`exec_op` は `#[inline(always)]` — #10-b の教訓）。
#[inline(never)]
fn unpack_tuple(buf: &mut Vec<Value>, base: usize, src: u16, n: u16) -> Result<(), String> {
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
fn let_tuple(
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
fn declare_global(
    interp: &mut Interpreter,
    chunk: &Chunk,
    ni: u32,
    kind: super::op::DeclKind,
    v: Value,
) -> Result<(), String> {
    let name = &chunk.names[ni as usize];
    interp.vm_declare_global(name, kind, &chunk.names, v)
}

/// `Op::GetTraitAttr` の本体（#27）。ツリーウォークと同じ `trait_access_evaled` へ委譲する。
/// ⚠ `#[inline(never)]`（`exec_op` は `#[inline(always)]` — #10-b の教訓）。
#[inline(never)]
fn get_trait_attr(
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
fn set_trait_attr(
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
fn breakpoint_op(
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

/// 単一命令を実行し、制御フローを返す。エラーは `Err` で返し、`run` がハンドラスタックへ回す。
///
/// ⚠ `#[inline(always)]`: 呼び出し元が `run` と `run_stepping` の **2 つ**になった時点で
/// `#[inline]` だけではインライン展開されなくなり、**通常経路が 3〜5% 退行した**（#1 で実測）。
/// バイトコードは byte-identical だったので、原因は生成コードではなくインライン判断だと特定できた。
///
/// ⚠ **重い op の本体はここに書かず `#[inline(never)]` の関数へ出す**（#10-b で実測）。
/// この関数は全呼び出し元へ展開されるので、その op を使わない Chunk まで巻き添えで遅くなる。
#[inline(always)]
fn exec_op(
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
                None => return Err(format!("NameError: '{name}' is not defined")),
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
        Op::StoreLocalDeepCopy(s) => {
            let v = Interpreter::deep_copy_value(buf.pop().unwrap());
            buf[base + *s as usize] = v;
        }
        Op::StoreLocalCopyFreeze(s) => {
            let v = Interpreter::deep_copy_value(buf.pop().unwrap());
            interp.apply_freeze_to_value(&v, true)?;
            buf[base + *s as usize] = v;
        }
        // `let x = <グローバル識別子>`（#27-c）。ソースの可変性は実行時にしか分からないので
        // `vm_let_value_from_ident`（`DeclKind::LetFromIdent` と**同一の実装**）へ委譲する。
        Op::StoreLocalFromIdent(s, ni) => {
            let v = buf.pop().unwrap();
            let src_mutable = interp.vm_global_is_mutable(&chunk.names[*ni as usize]);
            let v = interp.vm_let_value_from_ident(src_mutable, v)?;
            buf[base + *s as usize] = v;
        }
        Op::StoreLocalFreezeInstance(s) => {
            let v = buf.pop().unwrap();
            let v = if matches!(v, Value::Instance(_)) {
                let copied = Interpreter::deep_copy_value(v);
                interp.apply_freeze_to_value(&copied, true)?;
                copied
            } else {
                v
            };
            buf[base + *s as usize] = v;
        }
        Op::Pop => {
            buf.pop();
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
            let v = 'get: {
                if let Value::Instance(inst_rc) = &obj {
                    let class_id = inst_rc.borrow().class.class_id;
                    if let Some((idx, access)) = cache.get(class_id) {
                        if access == crate::ast::AttrCache::PUBLIC {
                            let inst = inst_rc.borrow();
                            debug_assert_eq!(
                                inst.class.field_index.get(&chunk.names[*name_idx as usize]).copied(),
                                Some(idx),
                                "VM GetAttr cache slot mismatch"
                            );
                            if let Some(fv) = inst.field_value(idx) {
                                break 'get fv;
                            }
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
                    if let Some((idx, access)) = cache.get(inst.class.class_id) {
                        if access == crate::ast::AttrCache::PUBLIC {
                            debug_assert_eq!(
                                inst.class.field_index.get(&chunk.names[*name_idx as usize]).copied(),
                                Some(idx),
                                "VM GetAttrLocal cache slot mismatch"
                            );
                            if let Some(fv) = inst.field_value(idx) {
                                break 'get fv;
                            }
                        }
                    }
                }
                let obj = buf[base + *slot as usize].clone();
                interp.get_attr_val(obj, &chunk.names[*name_idx as usize], Some(cache))?
            };
            buf.push(v);
        }
        Op::SetAttr(name_idx) => {
            let value = buf.pop().unwrap();
            let obj = buf.pop().unwrap();
            interp.attr_assign_evaled(obj, &chunk.names[*name_idx as usize], value)?;
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
            if interp.builtin_is_shadowed_global(name) {
                let callee = interp
                    .vm_load_name(name)
                    .ok_or_else(|| format!("NameError: '{name}' is not defined"))?;
                let evaled = args.into_iter().map(|v| (None, v, false)).collect();
                let name = name.clone(); // interp の可変借用のため名前を退避する
                buf.push(interp.call_value_evaled(callee, evaled, &name, None, 0)?);
            } else {
                match interp.eval_builtin_evaled(name, args) {
                    Some(r) => buf.push(r?),
                    // コンパイラは eval_builtin_evaled が扱う名前だけ発行するので到達しない。
                    None => return Err(format!("NameError: '{name}' is not defined")),
                }
            }
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
        Op::ForIter(iter_slot, target_slot, exit_ip) => {
            let iter_idx = base + *iter_slot as usize;
            // 高速パス: Generator（range/list/str/set/tuple/gen __iter__ の実体）は
            // index を直接進める（eval_method_call のディスパッチを丸ごと回避）。
            // メソッド呼び出しの Generator "next" アームと同一意味論。
            let next: Option<Option<Value>> =
                if let Value::Generator(state) = &buf[iter_idx] {
                    let mut s = state.borrow_mut();
                    if s.index < s.values.len() {
                        let val = s.values[s.index].clone();
                        s.index += 1;
                        Some(Some(val))
                    } else {
                        Some(None) // 枯渇
                    }
                } else {
                    None // 非 Generator（カスタムイテレータ）はフォールバック
                };
            match next {
                Some(Some(val)) => buf[base + *target_slot as usize] = val,
                Some(None) => return Ok(Flow::Jump(*exit_ip as usize)),
                None => {
                    // フォールバック: カスタムイテレータ等は .next() を呼ぶ。
                    let iter = buf[iter_idx].clone();
                    match interp.eval_method_call(iter, "next", &[], None) {
                        Ok(item) => buf[base + *target_slot as usize] = item,
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
            let arg_vals = buf.split_off(split); // arg0..argN-1（順序保持）
            let callee = buf.pop().unwrap();
            let evaled: Vec<(Option<String>, Value, bool)> = arg_vals
                .into_iter()
                .enumerate()
                .map(|(i, v)| (None, v, (mut_mask >> i) & 1 == 1))
                .collect();
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
            let arg_vals = buf.split_off(split);
            // レシーバは呼び先の `self` として所有権が要るので clone は避けられないが、
            // `LoadLocal` の op ディスパッチと push/pop は省ける。
            let obj = buf[base + *slot as usize].clone();
            let evaled: Vec<(Option<String>, Value, bool)> = arg_vals
                .into_iter()
                .enumerate()
                .map(|(i, v)| (None, v, (mut_mask >> i) & 1 == 1))
                .collect();
            let name = &chunk.names[*name_idx as usize];
            let r = if matches!(obj, Value::Instance(_)) {
                interp.call_instance_method_evaled(
                    obj, name, evaled,
                    Some(&chunk.attr_caches[*name_idx as usize]), None,
                )?
            } else {
                interp.vm_method_call_other(obj, name, evaled, *node_id, chunk)?
            };
            buf.push(r);
            return Ok(Flow::NextAfterCall); // #1
        }
        Op::CallMethod(name_idx, argc, mut_mask, node_id) => {
            let n = *argc as usize;
            let split = buf.len() - n;
            let arg_vals = buf.split_off(split);
            let obj = buf.pop().unwrap();
            let evaled: Vec<(Option<String>, Value, bool)> = arg_vals
                .into_iter()
                .enumerate()
                .map(|(i, v)| (None, v, (mut_mask >> i) & 1 == 1))
                .collect();
            // メソッドはツリーウォークと同じく call_span=None（degraded フレームで一致）。
            let name = &chunk.names[*name_idx as usize];
            let r = if matches!(obj, Value::Instance(_)) {
                interp.call_instance_method_evaled(
                    obj, name, evaled,
                    Some(&chunk.attr_caches[*name_idx as usize]), None,
                )?
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
            let evaled: Vec<(Option<String>, Value, bool)> = arg_vals
                .into_iter()
                .enumerate()
                .map(|(i, v)| (kw.arg_names[i].clone(), v, (kw.mut_mask >> i) & 1 == 1))
                .collect();
            let name = &chunk.names[kw.name_idx as usize];
            let r = if matches!(obj, Value::Instance(_)) {
                interp.call_instance_method_evaled(
                    obj,
                    name,
                    evaled,
                    Some(&chunk.attr_caches[kw.name_idx as usize]),
                    None,
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
                list.borrow_mut().push(v);
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
            let v = Interpreter::deep_copy_value(buf.pop().unwrap());
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
        Op::SetIndex => {
            let value = buf.pop().unwrap();
            let key = buf.pop().unwrap();
            let obj = buf.pop().unwrap();
            interp.eval_setitem(obj, key, value)?;
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
            buf.push(interp.vm_build_set(vals));
        }
        Op::BuildDict(n) => {
            let split = buf.len() - 2 * *n as usize;
            let flat = buf.split_off(split);
            buf.push(interp.vm_build_dict(flat));
        }
        Op::Yield => {
            let v = buf.pop().unwrap();
            interp.vm_yield_push(v);
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

/// int 型特化二項演算（plan A）。`apply_bin_fast` の (Int,Int) アームと同一意味論。
/// Add/Sub/Mul と比較のみ対応（Div/Mod/Pow/bit・ゼロ除算しうる op は `None`＝汎用へ委譲）。
#[inline]
fn int_binop_specialized(x: i64, y: i64, op: &BinOp) -> Option<Value> {
    Some(match op {
        BinOp::Add => Value::Int(x + y),
        BinOp::Sub => Value::Int(x - y),
        BinOp::Mul => Value::Int(x * y),
        // ゼロ除算は `None` を返して汎用パスへ委ねる。エラーメッセージ（`ZeroDivisionError: …` の
        // 3 種の文言）を `apply_binop` の一箇所に保つため、ここでは複製しない。
        BinOp::Div => {
            if y == 0 {
                return None;
            }
            Value::Float(x as f64 / y as f64)
        }
        BinOp::FloorDiv => {
            if y == 0 {
                return None;
            }
            Value::Int(x.div_euclid(y))
        }
        BinOp::Mod => {
            if y == 0 {
                return None;
            }
            Value::Int(x.rem_euclid(y))
        }
        // 指数が非負なら整数冪、負なら float（`apply_binop` の Int/Int アームと同一）。
        BinOp::Pow => {
            if y >= 0 {
                Value::Int(x.pow(y as u32))
            } else {
                Value::Float((x as f64).powi(y as i32))
            }
        }
        BinOp::BitAnd => Value::Int(x & y),
        BinOp::BitOr => Value::Int(x | y),
        BinOp::BitXor => Value::Int(x ^ y),
        BinOp::LShift => Value::Int(x << y),
        BinOp::RShift => Value::Int(x >> y),
        BinOp::Lt => Value::Bool(x < y),
        BinOp::Gt => Value::Bool(x > y),
        BinOp::LtEq => Value::Bool(x <= y),
        BinOp::GtEq => Value::Bool(x >= y),
        BinOp::Eq => Value::Bool(x == y),
        BinOp::NotEq => Value::Bool(x != y),
        _ => return None,
    })
}

/// float 型特化二項演算（plan A）。`apply_bin_fast` の (Float,Float) アームと同一意味論。
#[inline]
fn float_binop_specialized(x: f64, y: f64, op: &BinOp) -> Option<Value> {
    Some(match op {
        BinOp::Add => Value::Float(x + y),
        BinOp::Sub => Value::Float(x - y),
        BinOp::Mul => Value::Float(x * y),
        // float の除算はゼロ検査なし（inf/NaN を返す）＝ `apply_binop` の Float/Float アームと同一。
        // `//` と `%` は Float/Float のアームが存在しない（＝エラー）ため特化しない。
        BinOp::Div => Value::Float(x / y),
        BinOp::Pow => Value::Float(x.powf(y)),
        BinOp::Lt => Value::Bool(x < y),
        BinOp::Gt => Value::Bool(x > y),
        BinOp::LtEq => Value::Bool(x <= y),
        BinOp::GtEq => Value::Bool(x >= y),
        BinOp::Eq => Value::Bool(x == y),
        BinOp::NotEq => Value::Bool(x != y),
        _ => return None,
    })
}

/// int/float の算術・順序比較を高速パスで処理し、それ以外は `apply_binop_dyn` へ委譲する。
/// 高速パスは `apply_binop` の該当アームと同一セマンティクス。
#[inline]
fn apply_bin_fast(
    interp: &mut Interpreter,
    op: &BinOp,
    a: Value,
    b: Value,
) -> Result<Value, String> {
    use Value::{Float, Int};
    let r = match (op, &a, &b) {
        (BinOp::Add, Int(x), Int(y)) => Int(*x + *y),
        (BinOp::Add, Float(x), Float(y)) => Float(*x + *y),
        (BinOp::Add, Int(x), Float(y)) => Float(*x as f64 + *y),
        (BinOp::Add, Float(x), Int(y)) => Float(*x + *y as f64),
        (BinOp::Sub, Int(x), Int(y)) => Int(*x - *y),
        (BinOp::Sub, Float(x), Float(y)) => Float(*x - *y),
        (BinOp::Sub, Int(x), Float(y)) => Float(*x as f64 - *y),
        (BinOp::Sub, Float(x), Int(y)) => Float(*x - *y as f64),
        (BinOp::Mul, Int(x), Int(y)) => Int(*x * *y),
        (BinOp::Mul, Float(x), Float(y)) => Float(*x * *y),
        (BinOp::Mul, Int(x), Float(y)) => Float(*x as f64 * *y),
        (BinOp::Mul, Float(x), Int(y)) => Float(*x * *y as f64),
        (BinOp::Lt, Int(x), Int(y)) => Value::Bool(*x < *y),
        (BinOp::Lt, Float(x), Float(y)) => Value::Bool(*x < *y),
        (BinOp::Gt, Int(x), Int(y)) => Value::Bool(*x > *y),
        (BinOp::Gt, Float(x), Float(y)) => Value::Bool(*x > *y),
        (BinOp::LtEq, Int(x), Int(y)) => Value::Bool(*x <= *y),
        (BinOp::LtEq, Float(x), Float(y)) => Value::Bool(*x <= *y),
        (BinOp::GtEq, Int(x), Int(y)) => Value::Bool(*x >= *y),
        (BinOp::GtEq, Float(x), Float(y)) => Value::Bool(*x >= *y),
        // int/float 混在の順序比較（#16 段階(b)(iii)）。`apply_binop` の混在アームと同一意味論。
        // 以前はここが無く `apply_binop_dyn` → 巨大 match まで降りていた。
        // `LtEq`/`GtEq` は #18 で `apply_binop` 側にアームが入ったので、ここでも高速パスに載せる。
        // Eq/NotEq は `apply_binop` が `values_eq` の catch-all で処理するため特化しない。
        (BinOp::Lt, Int(x), Float(y)) => Value::Bool((*x as f64) < *y),
        (BinOp::Lt, Float(x), Int(y)) => Value::Bool(*x < *y as f64),
        (BinOp::Gt, Int(x), Float(y)) => Value::Bool((*x as f64) > *y),
        (BinOp::Gt, Float(x), Int(y)) => Value::Bool(*x > *y as f64),
        (BinOp::LtEq, Int(x), Float(y)) => Value::Bool((*x as f64) <= *y),
        (BinOp::LtEq, Float(x), Int(y)) => Value::Bool(*x <= *y as f64),
        (BinOp::GtEq, Int(x), Float(y)) => Value::Bool((*x as f64) >= *y),
        (BinOp::GtEq, Float(x), Int(y)) => Value::Bool(*x >= *y as f64),
        // int/float 混在の除算・冪（ゼロ検査不要なアームのみ）。
        (BinOp::Div, Int(x), Float(y)) => Float(*x as f64 / *y),
        (BinOp::Div, Float(x), Int(y)) => Float(*x / *y as f64),
        (BinOp::Div, Float(x), Float(y)) => Float(*x / *y),
        (BinOp::Pow, Float(x), Float(y)) => Float(x.powf(*y)),
        (BinOp::Pow, Int(x), Float(y)) => Float((*x as f64).powf(*y)),
        (BinOp::Pow, Float(x), Int(y)) => Float(x.powi(*y as i32)),
        // その他の型・ゼロ除算しうる int 演算はフルパスへ。
        _ => return interp.apply_binop_dyn(op, a, b),
    };
    Ok(r)
}

/// `Value::Bool` のみ直接判定し（比較演算子の結果＝ホットな条件）、それ以外は
/// セマンティクス一致のため既存 `eval_truthy` へ委譲する。
#[inline]
fn truthy_fast(interp: &mut Interpreter, v: &Value) -> Result<bool, String> {
    match v {
        Value::Bool(b) => Ok(*b),
        _ => interp.eval_truthy(v),
    }
}
