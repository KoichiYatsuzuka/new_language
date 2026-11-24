// native_api/callbacks/iter.rs — 反復のコールバック。

use super::*;

/// イテラブルなオブジェクトハンドルからイテレータハンドルを生成する。
/// イテレータハンドルは `-(idx+2)` の負値でエンコードされる。
pub(super) extern "C" fn ar_iter_from(obj_h: i64) -> i64 {
    let (has_err, obj) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h))
    });
    if has_err { return TL_NONE; }
    let entry = match snapshot_items(&obj) {
        Some(items) => NativeIter::Items(items, 0),
        None => match protocol_iter(obj, "iter_from") {
            Some(g) => NativeIter::Gen(g),
            None => return TL_NONE,
        },
    };
    push_iter(entry)
}

/// ① 実体化してよい型（VM の `make_for_iterator` も実体化する型）の要素の写し。ほかの型は `None`
/// （[`protocol_iter`] で 1 つずつ取る）。反復の途中のリストの変更は反復に効かない（VM と同じ）。
fn snapshot_items(obj: &Value) -> Option<Vec<Value>> {
    match obj {
        Value::List(l) => Some(l.borrow().clone()),
        Value::FrozenList { state, layout } => {
            let fst = state.borrow();
            Some((0..fst.len).map(|i| layout.reconstruct_item(&fst.data, i)).collect())
        }
        Value::Tuple(t) => Some(t.all_values().to_vec()),
        Value::Str(s) => Some(s.chars().map(|c| Value::str(c.to_string())).collect()),
        Value::Dict(d) => Some(d.borrow().all_keys()),
        _ => None,
    }
}

/// ② 写さない型は VM と同じ反復プロトコル（`make_for_iterator` → `gen_next`・1-3）。失敗したらエラーを積んで `None`。
/// ⚠ インタープリタへ呼び戻すので、`STATE` の借用を握ったまま呼ばない（`__iter__` がネイティブを呼ぶ）。
fn protocol_iter(obj: Value, what: &str) -> Option<std::rc::Rc<RefCell<crate::interpreter::GeneratorState>>> {
    let interp_ptr = with_state(|s| s.borrow().interp_ptr);
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set for {what}")));
        return None;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.make_for_iterator(obj) {
        Ok(Value::Generator(g)) => Some(g),
        Ok(other) => {
            let got = interp.type_name(&other);
            with_state(|s| s.borrow_mut().error = Some(format!(
                "TypeError: iterator protocol returned '{got}', not a generator"
            )));
            None
        }
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            None
        }
    }
}

/// イテレータの表へ足し、ハンドル `-(idx+2)` を返す。
fn push_iter(entry: NativeIter) -> i64 {
    with_state(|s| {
        let mut st = s.borrow_mut();
        let idx = st.iter_table.len();
        st.iter_table.push(entry);
        -((idx as i64) + 2)
    })
}

/// [`ar_iter_begin`] がアリーナを伸ばすとき、要素の分に足す欄の数（ループの本体が 1 周に積む一時値の分）。
const RUN_SLACK: usize = 1024;

/// field 87（copy_alloc_speed_plan.md 7-20）: `for` の入口。[`ar_iter_from`] と同じくイテレータのハンドルを返し、写す型なら
/// 要素を**全部アリーナへ積んで**、そのハンドルの並びの番地を `out[0]`・個数を `out[1]` へ書く。生成コードは周ごとに並びを
/// 読むだけで、要素を取るコールバックを呼ばない。写さない型は `out[1] = -1`（field 84 で 1 つずつ）。
/// 失敗したら `TL_NONE`（エラーは積んである）と 0 個の並び（生成コードのループは 1 周もしない）。
/// - 要素は `iter_from` の写しと同じ値（リストの要素の参照を 1 つずつ足す）で、ハンドルは `push_value` と同じ（`None`・
///   真偽値・小さい整数は決まったハンドル）。積み方は [`push_run`]（7-21）。⚠ 積んだ欄はループの保存点より下にある
///   （生成コードは保存点をこの呼び出しの後に取る）ので、周ごとの巻き戻しでは消えない。消えるのはループの外の巻き戻し
///   （呼ぶ側の生成コード・呼び出しの出口）
/// - 並びは表の `NativeIter::Run` が持ち、`iter_drop` まで書き換えない（表が伸びても並びの番地は動かない）
/// - ⚠ 以前（7-15）は入口で要素を `Vec` へ写し、周ごとに `iter_next_at` が写しから取り出して積んでいた（1 周に 1 回の
///   コールバック・k1 の約 2 割）
pub(super) extern "C" fn ar_iter_begin(obj_h: i64, out: *mut i64) -> i64 {
    // SAFETY: 生成コードは 2 個の `i64` の置き場を渡す
    let out = unsafe { std::slice::from_raw_parts_mut(out, 2) };
    out[0] = 0;
    out[1] = 0;
    let (has_err, obj) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h))
    });
    if has_err { return TL_NONE; }
    let hs = match &obj {
        // リストは写しの `Vec` を作らずに要素の参照を足して積む（ほかの型は `iter_from` と同じ写しから）
        Value::List(l) => {
            let l = l.borrow();
            with_state(|s| push_run(&mut s.borrow_mut(), l.len(), l.iter(), write_clone))
        }
        _ => match snapshot_items(&obj) {
            Some(items) => with_state(|s| {
                push_run(&mut s.borrow_mut(), items.len(), items.into_iter(), |v, slot| {
                    slot.write(v);
                })
            }),
            None => {
                return match protocol_iter(obj, "iter_begin") {
                    Some(g) => {
                        out[1] = -1;
                        push_iter(NativeIter::Gen(g))
                    }
                    None => TL_NONE,
                };
            }
        },
    };
    // 並びの番地は `Vec` を表へ移しても変わらない（中身の確保は動かない）
    out[0] = hs.as_ptr() as i64;
    out[1] = hs.len() as i64;
    push_iter(NativeIter::Run(hs))
}

/// [`ar_iter_begin`] の並びを作る: `n` 個の要素 `items` を積み、ハンドルの並びを返す（ハンドルは `push_value` と同じ・
/// [`fixed_handle`]）。要素はアリーナの空き欄へ直接書く（copy_alloc_speed_plan.md 7-21）。
/// - アリーナは 1 回だけ、ちょうど要る大きさ（と本体の一時値の分）に伸ばす。⚠ 1 つずつ積むと倍々に伸び、伸ばす途中の
///   確保と余りの分だけ最大の量が増えた（10 万要素の k5 で、写しの `Vec` だけだった以前より 約 6 MB・7-20）
/// - ⚠ `n` は `items` の個数（空き欄をこの数で確保する。超えて空き欄が尽きたら添字の検査で止まる）。`write` は
///   アリーナに触らないこと（空き欄を借りている）
/// - ⚠ 以前（7-20）は `&mut dyn Iterator` から `push_value` で 1 つずつ積んでいた。`next()` がメモリ経由で返した要素を
///   16 バイトずつ読み直す命令（要素の種類の 1 バイトとポインタの 8 バイトを別々に書いた直後なので、書いた値を読みへ
///   直接渡せない）で待ち、k1 の 18%（ほぼ全部がその 1 命令）を占めていた
#[inline(always)]
fn push_run<T: std::borrow::Borrow<Value>>(
    st: &mut NativeCallState,
    n: usize,
    items: impl Iterator<Item = T>,
    write: impl Fn(T, &mut std::mem::MaybeUninit<Value>),
) -> Vec<i64> {
    if st.arena.capacity() - st.arena.len() < n + RUN_SLACK {
        st.arena.reserve_exact(n + RUN_SLACK);
    }
    let base = st.arena.len();
    let spare = st.arena.spare_capacity_mut();
    let mut k = 0usize;
    let hs: Vec<i64> = items
        .map(|v| match fixed_handle(v.borrow()) {
            Some(h) => h,
            None => {
                write(v, &mut spare[k]);
                k += 1;
                (base + k - 1) as i64
            }
        })
        .collect();
    // SAFETY: 空き欄の先頭 `k` 個は上で書いた（途中で止まったら書いた値は捨てられずに残るだけで、未初期化は読まない）
    unsafe { st.arena.set_len(base + k) };
    hs
}

/// リストの要素の複製を空き欄 `slot` へ書く（[`ar_iter_begin`]）。よく出る種類はその場で複製して直接書く
/// （同じ値・`Value::clone` と同じ意味）。
/// ⚠ 複製を一時の値にしてから写すと、LLVM が一時の置き場へ 1〜8 バイトずつ書いて 16 バイトで読み直す形にし、
///   7-20 と同じ待ちが出た（試作で k1 の 11% がその 1 命令）。`Value::clone`（derive）も結果をメモリ経由で返す。
#[inline(always)]
fn write_clone(v: &Value, slot: &mut std::mem::MaybeUninit<Value>) {
    match v {
        Value::Instance(rc) => {
            slot.write(Value::Instance(rc.clone()));
        }
        Value::Str(s) => {
            slot.write(Value::Str(s.clone()));
        }
        Value::Float(f) => {
            slot.write(Value::Float(*f));
        }
        Value::Int(n) => {
            slot.write(Value::Int(*n));
        }
        _ => {
            slot.write(v.clone());
        }
    }
}

/// `for` のループを抜けたときにイテレータを捨てる（field 71・11-1・D-12）。表を `iter_h` の位置まで縮めるので、
/// このループの中で作り、`return` などで捨て損ねたイテレータも一緒に捨てる（ループは入れ子なので、後ろにあるのは
/// 内側のものだけ）。
/// ⚠ 表に無いハンドルは黙って無視しない。`ar_iter_from` が失敗した（`TL_NONE` を返した）ときは、エラーが既に
///   入っているのでそのまま返す。
pub(super) extern "C" fn ar_iter_drop(iter_h: i64) {
    with_state(|s| {
        let mut st = s.borrow_mut();
        let idx = (-(iter_h + 2)) as usize;
        if iter_h <= -2 && idx < st.iter_table.len() {
            st.iter_table.truncate(idx);
        } else if st.error.is_none() {
            st.error = Some(format!("NativeError: invalid iter handle {iter_h} (iter_drop)"));
        }
    })
}

/// イテレータハンドルから次の要素ハンドルを取得する。
pub(super) extern "C" fn ar_iter_next(iter_h: i64) -> i64 {
    iter_next_impl(iter_h, None)
}

/// field 84（copy_alloc_speed_plan.md 7-15）: アリーナを保存点 `save`（0 以上のとき）まで巻き戻してから次の要素。
/// 周の終わりに残す値が無いループの巻き戻しを、次の周の頭のこの呼び出しにまとめる。
pub(super) extern "C" fn ar_iter_next_at(iter_h: i64, save: i64) -> i64 {
    iter_next_impl(iter_h, usize::try_from(save).ok())
}

/// [`ar_iter_next`] / [`ar_iter_next_at`] の本体。リストなどの写し（`NativeIter::Items`）の要素は、複製せずに写しから
/// **取り出して**積む（写しの要素は 1 回しか使わない・7-15）。巻き戻し・取り出し・積むのを 1 回の借用で行う。
/// ⚠ 以前は要素を複製して積み（写しにも残る）、`push_handle` でもう 1 回状態を借りていた。要素はインスタンスの `Rc` で、
///   参照の数はばらばらの塊にあるので、触るたびに離れたメモリを読む（`Value::clone` が k2 の自己時間の 14%）。
/// ⚠ 巻き戻しはエラーがあってもする（周の終わりの `compact_many` と同じ）。
fn iter_next_impl(iter_h: i64, rewind: Option<usize>) -> i64 {
    enum Step { Done(i64), Gen(std::rc::Rc<RefCell<crate::interpreter::GeneratorState>>) }
    enum Next { Val(Value), Stop, Gen(std::rc::Rc<RefCell<crate::interpreter::GeneratorState>>), Bad }
    let (step, interp_ptr) = with_state(|s| {
        let mut st = s.borrow_mut();
        if let Some(len) = rewind {
            st.arena.truncate(len);
        }
        if st.error.is_some() { return (Step::Done(TL_STOP_ITER), st.interp_ptr); }
        let ip = st.interp_ptr;
        if iter_h > -2 {
            st.error = Some(format!("NativeError: invalid iter handle {iter_h}"));
            return (Step::Done(TL_STOP_ITER), ip);
        }
        let idx = (-(iter_h + 2)) as usize;
        let next = match st.iter_table.get_mut(idx) {
            Some(NativeIter::Items(items, pos)) => match items.get_mut(*pos) {
                Some(slot) => {
                    *pos += 1;
                    Next::Val(std::mem::replace(slot, Value::None))
                }
                None => Next::Stop,
            },
            Some(NativeIter::Gen(g)) => Next::Gen(g.clone()),
            // 並び（`ar_iter_begin`）は生成コードが番地から読む。ここで進めると生成コードの添字とずれるので止める
            Some(NativeIter::Run(_)) => {
                st.error = Some(format!("NativeError: iter handle {iter_h} is a handle run (iter_begin); read it directly"));
                return (Step::Done(TL_STOP_ITER), ip);
            }
            None => Next::Bad,
        };
        let step = match next {
            Next::Val(v) => Step::Done(st.push_value(v)),
            Next::Stop => Step::Done(TL_STOP_ITER),
            Next::Gen(g) => Step::Gen(g),
            // ⚠ 表に無いハンドル（最外の呼び出しを抜けて捨てた後など）を黙って「終わり」にしない
            Next::Bad => {
                st.error = Some(format!("NativeError: invalid iter handle {iter_h}"));
                Step::Done(TL_STOP_ITER)
            }
        };
        (step, ip)
    });
    match step {
        Step::Done(h) => h,
        // ⚠ `gen_next` は本体（ジェネレータ・`__next__`）を走らせうるので、`STATE` の借用を落としてから呼ぶ。
        Step::Gen(g) => {
            if interp_ptr.is_null() {
                with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for iter_next".to_string()));
                return TL_STOP_ITER;
            }
            let interp = unsafe { &mut *interp_ptr };
            match interp.gen_next(&g) {
                Ok(Some(v)) => push_handle(v),
                Ok(None) => TL_STOP_ITER,
                Err(e) => {
                    with_state(|s| s.borrow_mut().error = Some(e));
                    TL_STOP_ITER
                }
            }
        }
    }
}
