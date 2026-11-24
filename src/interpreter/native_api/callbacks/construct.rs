// native_api/callbacks/construct.rs — インスタンスの生成・生成コードの直接呼び出しの文脈・新しい値の引き渡し。

use super::*;

// ── 生成コードの直接呼び出しの文脈（fields 62-63・native_object_access_plan.md 1-9） ──────────────
//
// 生成コードがモジュール内のメソッドを `_impl` で直接呼ぶとき、呼ばれるメソッドのクラスが呼ぶ側の文脈と違えば
// その前後で `current_class` を受け手のクラスにする（インタープリタのメソッド呼び出しと同じ）。
// 印は前の文脈そのもの（`Rc::into_raw`）で、`ar_leave_method_ctx` が受け取って戻す。生成コードは呼び出しと
// 呼び出しの間で抜けない（`raise` は呼ばれた側の関数から戻るだけ）ので、`ar_enter_method_ctx` と
// `ar_leave_method_ctx` は必ず対になる。
// ⚠ 先に積まれたエラーがあっても文脈は切り替える（対を崩さない）。

/// 印: 文脈を変えなかった（受け手がインスタンスでない。直接呼び出しは型検査の結果なので来ないが、
/// 来たら呼ばれた側の属性の読み書きがエラーにする）。
const CTX_UNCHANGED: i64 = -1;

/// 印: 前の文脈は無かった（`current_class` が `None`）。
const CTX_NONE: i64 = 0;

pub(super) extern "C" fn ar_enter_method_ctx(obj_h: i64) -> i64 {
    // private / protected のメンバーを持つクラスが 1 つも無い間は、文脈を読む者がいないので切り替えない（7-3）
    if !crate::interpreter::value::restricted_members_exist() {
        return CTX_UNCHANGED;
    }
    let (obj, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.clone_value(obj_h), st.interp_ptr)
    });
    let Value::Instance(rc) = obj else { return CTX_UNCHANGED };
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for enter_method_ctx".to_string()));
        return CTX_UNCHANGED;
    }
    let interp = unsafe { &mut *interp_ptr };
    let class = rc.borrow().class.clone();
    match interp.current_class.replace(class) {
        None => CTX_NONE,
        Some(prev) => std::rc::Rc::into_raw(prev) as i64,
    }
}

pub(super) extern "C" fn ar_leave_method_ctx(token: i64) {
    if token == CTX_UNCHANGED {
        return;
    }
    let interp_ptr = with_state(|s| s.borrow().interp_ptr);
    // SAFETY: `token` は `ar_enter_method_ctx` が `Rc::into_raw` で返したもの（0 は `None`）で、ここで 1 回だけ戻す。
    let prev = (token != CTX_NONE)
        .then(|| unsafe { std::rc::Rc::from_raw(token as *const crate::interpreter::ClassValue) });
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for leave_method_ctx".to_string()));
        return;
    }
    unsafe { &mut *interp_ptr }.current_class = prev;
}

/// 型検査の注釈で受け手のクラスを決めた直接呼び出しの前のガード（field 64・native_object_access_plan.md 3-2・D-2）。
///
/// 注釈はヒントであって意味論の根拠ではない（`list[T]` の要素・`new_type` など、実行時に保証されない型がある）
/// ので、受け手が本当にそのクラスのインスタンスか（class_id）と、`mut self` のメソッドなら不変インスタンスでないかを
/// 確かめる。外れたら 0 を返し、生成コードは名前による呼び出し（`ar_call_method_m`）へ回る（そちらが正しく
/// 振り分け、必要ならエラーを出す）。クラスの class_id は箇所ごとのセルに 1 回だけ名前で引いて覚える（0 = まだ）。
/// 名前で引けなければ（シャドウなど）0 のまま返す＝毎回名前による呼び出し（遅いが正しい）。
pub(super) extern "C" fn ar_method_guard(obj_h: i64, class_ptr: *const u8, class_len: i32, cell: *mut u64, need_mutable: i32) -> i32 {
    use std::sync::atomic::{AtomicU64, Ordering};
    // ⚠ クラスの名前（の UTF-8 の検査）はセルが空のときだけ使う（copy_alloc_speed_plan.md 7-9）。以前は毎回検査していて、
    //   それがコンパイル済みの k1 の 9%（このガードの 2/3）だった。セルを埋めるのは名前が正しいときだけなので、正しくない
    //   名前は今までどおり、名前を引くたびにエラーになる。
    if cell.is_null() || obj_h < 3 {
        return 0;
    }
    let probe = with_state(|s| {
        let st = s.borrow();
        match st.arena.get(obj_h as usize) {
            Some(Value::Instance(rc)) => {
                let inst = rc.borrow();
                Some((inst.class.class_id, inst.flags() & crate::interpreter::value::INST_IMMUTABLE != 0, st.interp_ptr))
            }
            _ => None,
        }
    });
    let Some((class_id, immutable, interp_ptr)) = probe else { return 0 };
    // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（8 バイト境界）で、`AtomicU64` と同じ並び。
    let cell = unsafe { &*(cell as *const AtomicU64) };
    let mut want = cell.load(Ordering::Relaxed);
    if want == 0 {
        let Some(class_name) = native_str(class_ptr, class_len, "class name") else { return 0 };
        if interp_ptr.is_null() {
            return 0;
        }
        let interp = unsafe { &*interp_ptr };
        // ⚠ 今の大域（このモジュールの大域・`enter_native_globals`）だけを引く（known_bugs.md #30・型の照合の
        //   `names_another_class` と同じ）。`get_var` はローカルの枠を先に見るので、非同期タスクの本体が見ていた
        //   本体の同じ名前のクラスを引いて、セルに別のクラスを焼きうる。
        match interp.global_class_id(class_name) {
            Some(id) => {
                want = id as u64;
                cell.store(want, Ordering::Relaxed);
            }
            None => return 0,
        }
    }
    i32::from(class_id as u64 == want && !(need_mutable != 0 && immutable))
}

/// モジュール内のクラスの生成（field 67・native_object_access_plan.md 5-1・D-4）。VM の `Call`（呼び先がクラス）と
/// 同じ `instantiate_evaled` を直接呼ぶ。引数の可変性（`let` 仮引数の複製）は `mut_mask` から VM と同じ規則で渡す。
/// ⚠ 以前の生成コードはクラスを `CB_GET_GLOBAL` で値として引き、`CB_CALL_FN` の遅い道（`call_value_with_args`）で
///   呼んでいた（呼び出しのたびにコールバック 2 回と値の振り分け）。
/// クラスは名前で引く（`CB_GET_GLOBAL` と同じ `get_var`。1-7 でモジュールの大域にいる）。クラスでない値に
/// 束縛されていたら（型検査が通さないはずだが）値として呼ぶ（`call_value_with_args`）。
pub(super) extern "C" fn ar_construct(name_ptr: *const u8, name_len: i32, args_ptr: *const i64, n_args: i32, mut_mask: u32) -> i64 {
    construct_by_name(std::ptr::null_mut(), name_ptr, name_len, args_ptr, n_args, mut_mask, 0)
}

/// `Class(args)` の呼び出し箇所ごとのセルつき版（field 70・native_object_access_plan.md 10-2・D-11）。セルに前回の
/// クラスの `class_id + 1` が入っていて、インタープリタの表（`native_classes`）にあれば、名前の検査も名前で引くことも
/// しない。無ければ（最初の 1 回・ワーカーの最初の 1 回）`ar_construct` と同じく名前で引き、表とセルを埋める。
/// ⚠ 以前（field 67）は生成のたびに名前を UTF-8 として検査し、`get_var` で引いていた（1 回 約 80 ns）。
/// 同じ箇所の名前はいつも同じクラスを指す（モジュールの最上位のクラスは束縛し直せない・ローカルに同名があれば
/// 生成コードはこの道を使わない）。
pub(super) extern "C" fn ar_construct_ic(
    cell: *mut u64,
    name_ptr: *const u8,
    name_len: i32,
    args_ptr: *const i64,
    n_args: i32,
    mut_mask: u32,
) -> i64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    // 作りたての印を読んで 0 に戻すのと、エラーの有無・インタープリタを読むのを、状態を 1 回借りて済ませる（7-10）
    let (fresh, has_err, interp_ptr) = with_state(|s| {
        let mut st = s.borrow_mut();
        (std::mem::take(&mut st.fresh_mask), st.error.is_some(), st.interp_ptr)
    });
    if !cell.is_null() {
        // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（IC のセルと同じ）
        let packed = unsafe { &*(cell as *const AtomicU64) }.load(Ordering::Relaxed);
        if packed != 0 {
            if has_err {
                return TL_NONE;
            }
            if !interp_ptr.is_null() {
                let interp = unsafe { &mut *interp_ptr };
                if let Some(cls) = interp.native_classes.get(&((packed - 1) as u32)).cloned() {
                    return construct_class(interp, cls, args_ptr, n_args, mut_mask, fresh);
                }
            }
        }
    }
    construct_by_name(cell, name_ptr, name_len, args_ptr, n_args, mut_mask, fresh)
}

/// クラスを名前で引いて生成する（field 67 と field 70 の外れた道）。`cell` があればクラスを表とセルに入れる。
/// 直後の生成・名前によるメソッド呼び出しの「作りたての引数」の印を入れる（field 75・2-4）。
pub(super) extern "C" fn ar_set_fresh(mask: u32) {
    with_state(|s| s.borrow_mut().fresh_mask = mask);
}

/// 印を読んで 0 に戻す。⚠ `ar_construct_ic` / `ar_call_method_ic` は**入口の最初に**読む（途中で抜けても、次の
/// 呼び出しに印が残らないように。`ar_construct_ic` はエラーの有無と一緒に、状態を借りる 1 回の中で読む・7-10）。
pub(super) fn take_fresh_mask() -> u32 {
    with_state(|s| std::mem::take(&mut s.borrow_mut().fresh_mask))
}

/// 引数 `i` の値。作りたての印（`fresh` の bit i+1）が付いていれば、アリーナの欄から**取り出す**（欄は `None` になる。
/// 生成コードはその欄を二度と読まない）。付いていなければ今までどおり欄の値の写し。
/// ⚠ 小さい整数・真偽値・`None` の共有の欄（`INT_CACHE_BASE` より前）は取り出さない。
pub(super) fn arg_value(args_ptr: *const i64, i: usize, fresh: u32) -> Value {
    with_state(|s| arg_value_in(&mut s.borrow_mut(), args_ptr, i, fresh))
}

/// [`arg_value`] の本体。スレッドの状態を借りたまま引数を続けて読む所（生成・7-10）が使う。
fn arg_value_in(st: &mut crate::interpreter::native_api::NativeCallState, args_ptr: *const i64, i: usize, fresh: u32) -> Value {
    let h = unsafe { *args_ptr.add(i) };
    if i < 31 && fresh & (1 << (i + 1)) != 0 && h >= crate::interpreter::native_api::INT_CACHE_BASE as i64 {
        return match st.arena.get_mut(h as usize) {
            Some(slot) => std::mem::replace(slot, Value::None),
            None => Value::None,
        };
    }
    st.clone_value(h)
}

/// 直接呼び出しの `let self` / `let` 仮引数へ渡す作りたての値（field 76・2-4）。`h` の欄の 1 個は数えずに、
/// 誰とも共有していなければ複製しない（VM の `copy_value_held` / 受け手の `deep_copy_value` と同じ結果）。
pub(super) extern "C" fn ar_copy_fresh(h: i64, is_self: i32) -> i64 {
    let (has_err, val, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for copy_fresh".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    if is_self != 0 {
        // `let self`: `deep_copy_value`（`__copy__` を呼ばない・印を保つ）。共有していなければそのまま
        let unshared = h >= crate::interpreter::native_api::INT_CACHE_BASE as i64 && crate::interpreter::Interpreter::is_unshared(&val, 2);
        crate::interpreter::copy_stats::record(crate::interpreter::copy_stats::CopyKind::LetSelf, &val, unshared);
        if unshared {
            return h;
        }
        return push_handle(crate::interpreter::Interpreter::deep_copy_value(val));
    }
    let held = usize::from(h >= crate::interpreter::native_api::INT_CACHE_BASE as i64);
    match interp.copy_value_held(val.clone(), held + 1) {
        // 写さなかった（同じ実体）なら欄をそのまま使う
        Ok(v) if crate::interpreter::Interpreter::same_heap_value(&v, &val) => h,
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

/// 直接呼び出しの `let self` / `let` 仮引数へ渡す `mut` の値（field 77・4-4）。呼ぶ関数が読むだけで、走っている
/// タスクが無ければ複製しない（VM の `try_fast_bind` と同じ判定・`Interpreter::may_borrow_for`）。
/// ⚠ セルはプロセスに 1 つだが、覚えるのは「名前が読むだけの集合にあるか」だけ。非同期タスクのワーカーは
///   自分が走っているタスクなので、セルが 1 でも必ず複製する（`no_live_tasks` が偽）。
///
/// `is_self` は種類: 0 = `let` 仮引数・1 = `let self`・2 = `let` 仮引数で、渡す値が受け手・ほかの引数と**重ならない**と
/// 生成コードが言えたもの（同じ変数の別のフィールド・4-6）。2 のときは、呼ぶ関数が自分の `mut` 仮引数だけを書き換える
/// 関数（`ReadonlyFns::is_confined_writer`）でも複製しない。
pub(super) extern "C" fn ar_borrow_or_copy(h: i64, is_self: i32, name_ptr: *const u8, name_len: i32, cell: *mut u64) -> i64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    if !cell.is_null() && h >= crate::interpreter::native_api::INT_CACHE_BASE as i64 && crate::interpreter::async_mgr::no_live_tasks() {
        // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（IC のセルと同じ）
        let cell = unsafe { &*(cell as *const AtomicU64) };
        let mut known = cell.load(Ordering::Relaxed);
        if known == 0 {
            let interp_ptr = with_state(|s| s.borrow().interp_ptr);
            if let (Some(name), false) = (native_str(name_ptr, name_len, "function name"), interp_ptr.is_null()) {
                let interp = unsafe { &*interp_ptr };
                let ok = if is_self == 2 {
                    interp.readonly_fns.is_confined_writer(name)
                } else {
                    interp.readonly_fns.contains(name)
                };
                known = if ok { 1 } else { 2 };
                cell.store(known, Ordering::Relaxed);
            }
        }
        if known == 1 {
            if is_self == 1 {
                return h;
            }
            // 仮引数: `__copy__` を持つクラスの値は今までどおり `__copy__` を呼ぶ
            let (val, interp_ptr) = with_state(|s| {
                let st = s.borrow();
                (st.clone_value(h), st.interp_ptr)
            });
            if !interp_ptr.is_null() && !unsafe { &*interp_ptr }.has_copy_hook(&val) {
                return h;
            }
        }
    }
    if is_self == 1 {
        ar_deep_copy(h)
    } else {
        ar_copy_arg(h)
    }
}

fn construct_by_name(
    cell: *mut u64,
    name_ptr: *const u8,
    name_len: i32,
    args_ptr: *const i64,
    n_args: i32,
    mut_mask: u32,
    fresh: u32,
) -> i64 {
    let Some(name) = native_str(name_ptr, name_len, "class name") else { return TL_NONE };
    let (has_err, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (constructing '{name}')")));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.get_var(name).map(|v| v.get_value()) {
        Some(Value::Class(cls)) => {
            remember_site_class(interp, cell, &cls);
            construct_class(interp, cls, args_ptr, n_args, mut_mask, fresh)
        }
        Some(other) => {
            let args: Vec<Value> = (0..n_args as usize).map(|i| clone_value_at(unsafe { *args_ptr.add(i) })).collect();
            finish_construct(interp.call_value_with_args(other, args))
        }
        None => finish_construct(Err(format!("NameError: '{name}' is not defined"))),
    }
}

/// クラスを VM の `Call` と同じ `instantiate_evaled` で生成する。引数の可変性は `mut_mask` から VM と同じ規則で渡す。
fn construct_class(
    interp: &mut crate::interpreter::Interpreter,
    cls: std::rc::Rc<crate::interpreter::ClassValue>,
    args_ptr: *const i64,
    n_args: i32,
    mut_mask: u32,
    fresh: u32,
) -> i64 {
    // 作りたての印の付いた引数は欄から取り出す（2-4・参照の数を VM と同じにして、束縛と格納の複製を省く）。
    // 並びは置き場から取り（`instantiate_evaled` が戻す）、引数は状態を 1 回借りて全部読む（7-10）
    let mut evaled = interp.take_evaled_buf();
    with_state(|s| {
        let mut st = s.borrow_mut();
        evaled.extend(
            (0..n_args.max(0) as usize).map(|i| (None, arg_value_in(&mut st, args_ptr, i, fresh), mask_arg_is_mut(mut_mask, i))),
        );
    });
    finish_construct(interp.instantiate_evaled(cls, evaled))
}

fn finish_construct(r: Result<Value, String>) -> i64 {
    match r {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

/// 生成の箇所のセルとインタープリタの表に、名前で引いたクラスを覚える（2 回目からは名前で引かない・10-2）。
fn remember_site_class(
    interp: &mut crate::interpreter::Interpreter,
    cell: *mut u64,
    cls: &std::rc::Rc<crate::interpreter::ClassValue>,
) {
    use std::sync::atomic::{AtomicU64, Ordering};
    if !cell.is_null() {
        interp.native_classes.insert(cls.class_id, cls.clone());
        // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（IC のセルと同じ）
        unsafe { &*(cell as *const AtomicU64) }.store(u64::from(cls.class_id) + 1, Ordering::Relaxed);
    }
}

/// 生成の箇所 `cell`（null でないこと）のクラス。`ar_construct_ic` と同じ引き方: セルが埋まっていて表にあればそれ、
/// 無ければ名前で引いて覚える。クラスでない値・見つからない名前は `None`（呼ぶ側は今までの道へ回り、そちらがエラーを出す）。
fn site_class(
    interp: &mut crate::interpreter::Interpreter,
    cell: *mut u64,
    name_ptr: *const u8,
    name_len: i32,
) -> Option<std::rc::Rc<crate::interpreter::ClassValue>> {
    use std::sync::atomic::{AtomicU64, Ordering};
    // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（IC のセルと同じ）
    let packed = unsafe { &*(cell as *const AtomicU64) }.load(Ordering::Relaxed);
    if packed != 0 {
        if let Some(cls) = interp.native_classes.get(&((packed - 1) as u32)) {
            return Some(cls.clone());
        }
    }
    let name = native_str(name_ptr, name_len, "class name")?;
    match interp.get_var(name).map(|v| v.get_value()) {
        Some(Value::Class(cls)) => {
            remember_site_class(interp, cell, &cls);
            Some(cls)
        }
        _ => None,
    }
}

/// 合成した `__init__` で作るインスタンスを、引数を格納する前の状態で作ってアリーナに積む（field 83・
/// copy_alloc_speed_plan.md 7-11）。生成コードはこのあと数値の引数を番地 `*out` へ直接書き、初期化の印を立てる。これは
/// `store_auto_init` が `store_field_checked` で行う格納と同じ結果になる: 作ったばかりのインスタンスは不変でなく、どの slot も
/// まだ初期化されていないので格納の検査は通り、値の変換（幅への切り詰め・`float32` への丸め）は生成コードが
/// `InstanceData::store_field` と同じ規則で行う。
/// クラスは `ar_construct_ic` と同じセル・同じ引き方（[`site_class`]）で決め、生成コードの表（`DirectClassDesc`）のとおりか
/// （並び・幅・`mut`・全部 public）と、合成した `__init__` の格納の順が `slots` と同じか（`Interpreter::auto_init_direct_ok`）を
/// 確かめる。確かめたクラスは照合のセル `checked` に番号 + 1 で覚え、2 回目からは照合しない（答えはクラスと、箇所ごとに
/// 決まった表と順だけで決まる。`ar_inst_block` のセルと同じ考え）。外れ・先に積まれたエラーは 0 を返して何もしない
/// （生成コードは今までの `ar_construct_ic` へ回り、結果もエラーも今までどおり）。
#[allow(clippy::too_many_arguments)]
pub(super) extern "C" fn ar_new_instance(
    cell: *mut u64,
    checked: *mut u64,
    name_ptr: *const u8,
    name_len: i32,
    desc: *const crate::interpreter::value::DirectClassDesc,
    slots: *const i64,
    n: i32,
    out: *mut *mut u64,
) -> i64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    if cell.is_null() || checked.is_null() || desc.is_null() || out.is_null() || n <= 0 || slots.is_null() {
        return 0;
    }
    let (has_err, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.interp_ptr)
    });
    if has_err || interp_ptr.is_null() {
        return 0;
    }
    let interp = unsafe { &mut *interp_ptr };
    let Some(cls) = site_class(interp, cell, name_ptr, name_len) else { return 0 };
    // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（IC のセルと同じ）
    let checked = unsafe { &*(checked as *const AtomicU64) };
    let id1 = u64::from(cls.class_id) + 1;
    if checked.load(Ordering::Relaxed) != id1 {
        // SAFETY: `desc` は生成コードが DLL の中に置いた表
        let Some((cname, fields)) = (unsafe { read_direct_desc(desc) }) else { return 0 };
        // SAFETY: 生成コードが渡す `n` 個の `i64` の定数（DLL の中）
        let want = unsafe { std::slice::from_raw_parts(slots, n as usize) };
        if !cls.matches_direct_desc(cname, &fields) || !interp.auto_init_direct_ok(&cls, want) {
            return 0;
        }
        checked.store(id1, Ordering::Relaxed);
    }
    let rc = std::rc::Rc::new(std::cell::RefCell::new(crate::interpreter::Interpreter::new_instance_data(&cls)));
    // SAFETY: 塊（`Box<[u64]>`）はインスタンスが生きている間動かない。インスタンスはすぐ下でアリーナの欄に置き、生成コードは
    // 次にコールバックを呼ぶ前に書き終える（その間インスタンスを借りている者はいない・D-13 ④）。
    let block = unsafe { (*rc.as_ptr()).raw.as_mut_ptr() };
    let h = push_handle(Value::Instance(rc));
    unsafe { *out = block };
    h
}
