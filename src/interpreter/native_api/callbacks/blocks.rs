// native_api/callbacks/blocks.rs — フィールドの塊の番地を返すコールバックと、番地から直接読み書きする道。

use super::*;

/// field 86（copy_alloc_speed_plan.md 7-18）: 同じ親の `n` 個のフィールドが持つインスタンスの先頭の塊の番地を、親を 1 回だけ
/// たどって `out[i]` へ書く。1 件ごとの結果は field 79 を `reqs[i]` で呼んだのと同じ（読み出しの IC が当たり、slot が今持って
/// いる値が表のとおりのクラスのインスタンスのときだけ番地・ほかは空）。先に積まれた誤りがある・親がインスタンスでないなら
/// 全部空。アリーナには積まない。
/// ⚠ 外れた件をここで読み直さない（field 79 と同じ理由）。生成コードがその件の IC のセルで field 58 を呼んで埋める。
/// ⚠ 親も子も借りずに読む（`try_borrow_unguarded`・7-22）。可変に借りられているインスタンスはその件を空にする（今までの道へ）。
///   以前は `borrow()` で借りていた: 借用の印を書いて戻す 2 回の書き込みと、`instance_block` の呼び出し・`out.fill` の
///   `memset` が、1 回ごとの固定費だった（借りられていたら panic だった）
pub(super) extern "C" fn ar_attr_blocks(parent_h: i64, n: i32, reqs: *const AttrBlockReq, out: *mut *mut u64) {
    use std::sync::atomic::{AtomicU64, Ordering};
    if n <= 0 || reqs.is_null() || out.is_null() {
        return;
    }
    // SAFETY: 生成コードは `n` 件の表と `n` 個の `ptr` の置き場を渡す
    let reqs = unsafe { std::slice::from_raw_parts(reqs, n as usize) };
    let out = unsafe { std::slice::from_raw_parts_mut(out, n as usize) };
    let clear = |out: &mut [*mut u64]| {
        for o in out.iter_mut() {
            *o = std::ptr::null_mut();
        }
    };
    if parent_h < 3 {
        clear(out);
        return;
    }
    with_state(|s| {
        let st = s.borrow();
        if st.error.is_some() {
            clear(out);
            return;
        }
        let Some(Value::Instance(rc)) = st.arena.get(parent_h as usize) else {
            clear(out);
            return;
        };
        // SAFETY: 読むだけで、読み終わるまで誰も可変に借りない（コールバックの中で利用者のコードを呼ばない）
        let Ok(inst) = (unsafe { rc.try_borrow_unguarded() }) else {
            clear(out);
            return;
        };
        for (r, slot) in reqs.iter().zip(out.iter_mut()) {
            *slot = std::ptr::null_mut();
            if r.ic_cell.is_null() || r.class_cell.is_null() || r.desc.is_null() {
                continue;
            }
            // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（`read_ic_hit` と同じ）
            let packed = unsafe { &*(r.ic_cell as *const AtomicU64) }.load(Ordering::Relaxed);
            let Some((idx, _)) = inst.ic_slot(packed, true, || debug_field_name(r.name, r.name_len as i32)) else {
                continue;
            };
            if let Some(Some((Value::Instance(child), _))) = inst.boxed_fields.get(idx) {
                *slot = instance_block(child, r.desc, r.class_cell);
            }
        }
    })
}

/// field 85（copy_alloc_speed_plan.md 7-16）: `x.f op= v`（数値のフィールド）を生成コードが 1 回で行うための、フィールドの
/// 値の番地。返すのは、受け手が raw レイアウトでないクラスのインスタンスで、読み出しの IC が当たり（public のフィールド）、
/// スロットが初期化済みで可変の印が立ち（`let` のフィールド・凍ったインスタンスは印が倒れている）、`kind` どおりの
/// `Value::Int`（0）/ `Value::Float`（1）を持つときだけ。生成コードはそこへ `old op v` を直接書く: 値の型は変わらず（宣言の
/// 型の検査 `store_field` を通る値と同じ）、可変の印もそのまま。ほかは空を返し、生成コードは今までの道（同じ IC のセルで
/// 読む・`attr_assign_ic` で書く）へ回る。先に積まれた誤りがあれば空（今までの道が何もしない）。
/// ⚠ 番地は、次にコールバックを呼ぶまでの間だけ使える（スロットを差し替えるのはコールバックだけ）。
/// ⚠ 借りられない（誰かが借りたまま）なら空。
pub(super) extern "C" fn ar_num_field_ptr(obj_h: i64, name_ptr: *const u8, name_len: i32, cell: *mut u64, kind: i32) -> *mut u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    if cell.is_null() || obj_h < 3 || name_len <= 0 || name_ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `read_ic_hit` と同じ（生成コードが渡すのは DLL 内の `i64` の大域）
    let packed = unsafe { &*(cell as *const AtomicU64) }.load(Ordering::Relaxed);
    if packed == 0 {
        return std::ptr::null_mut();
    }
    with_state(|s| {
        let st = s.borrow();
        if st.error.is_some() {
            return std::ptr::null_mut();
        }
        let Some(Value::Instance(rc)) = st.arena.get(obj_h as usize) else { return std::ptr::null_mut() };
        let Ok(mut inst) = rc.try_borrow_mut() else { return std::ptr::null_mut() };
        if inst.has_raw_layout() {
            return std::ptr::null_mut();
        }
        let Some((idx, _)) = inst.ic_slot(packed, true, || debug_field_name(name_ptr, name_len)) else {
            return std::ptr::null_mut();
        };
        let p = match (kind, inst.boxed_fields.get_mut(idx)) {
            (0, Some(Some((Value::Int(n), true)))) => (n as *mut i64).cast::<u64>(),
            (1, Some(Some((Value::Float(f), true)))) => (f as *mut f64).cast::<u64>(),
            _ => return std::ptr::null_mut(),
        };
        // 格納の入口の数え方は今までの道（`assign_number_ic_hit`）と同じにする（数値は写さない・番地を返すときだけ数える）
        if let Some(Some((v, _))) = inst.boxed_fields.get(idx) {
            crate::interpreter::copy_stats::record(crate::interpreter::copy_stats::CopyKind::Store, v, true);
        }
        p
    })
}

/// field 80（7-5）: ガードが外れて名前で呼んだメソッドの戻り値を、宣言の型どおりに開ける。
///
/// ガード付きの呼び出し（受け手のクラスを型検査の注釈で決めた呼び出し）は、当たりの道で `_impl` が返す生の `int` / `float`
/// をそのまま使う。外れるのは注釈と違うクラスの値が来たとき（型検査の穴を通ったとき）だけで、そのとき呼んだ別のクラスの
/// メソッドが宣言と違う型を返したら、黙って変換せずに `TypeError` にする（以前は `int` は `to_int` で切り捨て、`float` は
/// 箱のまま続けていた・旧 known_problems.md #1）。
/// ⚠ 先に積まれたエラー・例外があれば何もしない（呼んだメソッドが投げた例外をこの誤りで上書きしない）。
/// ⚠ `int` に `bool` は通さない（Arrow の `bool` は `int` ではない）。`float` に `int` も通さない（VM はそのまま `int` で続ける）。
pub(super) extern "C" fn ar_expect_ret(h: i64, kind: i32, name_ptr: *const u8, name_len: i32) -> i64 {
    let (has_err, got) = with_state(|s| {
        let st = s.borrow();
        if st.error.is_some() || st.pending_raise.is_some() {
            return (true, None);
        }
        let got = match (kind, h) {
            (0, n) if n >= 3 && (n as usize) < INT_CACHE_BASE => Some(n - 3),
            (0, n) if n >= 3 => match st.arena.get(n as usize) {
                Some(Value::Int(v)) => Some(*v),
                Some(Value::UInt(v)) => Some(*v as i64),
                _ => None,
            },
            (1, n) if n >= INT_CACHE_BASE as i64 => match st.arena.get(n as usize) {
                Some(Value::Float(f)) => Some(f.to_bits() as i64),
                _ => None,
            },
            _ => None,
        };
        (false, got)
    });
    if has_err {
        return 0;
    }
    if let Some(v) = got {
        return v;
    }
    let want = if kind == 1 { "float" } else { "int" };
    let name = native_str(name_ptr, name_len, "method name").unwrap_or("?");
    set_argument_error(format!(
        "TypeError: method '{name}' is declared to return '{want}', but the call returned '{}'",
        handle_type_name(h)
    ));
    0
}

/// field 79（8-7）: 親の `name` のフィールドが持っているインスタンスの先頭の塊の番地。アリーナには積まない。
///
/// 返すのは、読み出しの IC が当たり（セルが空・クラスが違う・public でないなら当たらない）、その slot が**今持っている値**が
/// 表のとおりのクラスのインスタンスのときだけ。外れたときにここで読み直さない: 外れた道（`get_attr_val`）は一時値を返すことが
/// あり（ディスクリプタ・`import[rs]` のフィールド）、積まずに番地だけ返すと、持ち主のいない塊を指す。外れたら空を返し、
/// 生成コードが同じセルで field 58（`ar_get_attr_ic`）を呼ぶ（そちらがセルを埋め、値をアリーナに積む）。
pub(super) extern "C" fn ar_attr_block(
    parent_h: i64,
    name_ptr: *const u8,
    name_len: i32,
    ic_cell: *mut u64,
    desc: *const crate::interpreter::value::DirectClassDesc,
    class_cell: *mut u64,
) -> *mut u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    if parent_h < 3 || ic_cell.is_null() || class_cell.is_null() || desc.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（`read_ic_hit` と同じ）
    let packed = unsafe { &*(ic_cell as *const AtomicU64) }.load(Ordering::Relaxed);
    with_state(|s| {
        let st = s.borrow();
        if st.error.is_some() {
            return std::ptr::null_mut();
        }
        let Some(Value::Instance(rc)) = st.arena.get(parent_h as usize) else { return std::ptr::null_mut() };
        // SAFETY: 読むだけで、読み終わるまで誰も可変に借りない（可変に借りられていれば空・今までの道へ）
        let Ok(inst) = (unsafe { rc.try_borrow_unguarded() }) else { return std::ptr::null_mut() };
        let Some((idx, _)) = inst.ic_slot(packed, true, || debug_field_name(name_ptr, name_len)) else {
            return std::ptr::null_mut();
        };
        // インスタンスを持てるのは raw レイアウトでないクラスの slot だけ（raw レイアウトのフィールドは数値）
        match inst.boxed_fields.get(idx) {
            Some(Some((Value::Instance(child), _))) => instance_block(child, desc, class_cell),
            _ => std::ptr::null_mut(),
        }
    })
}

// ── 番地から直接読み書きする道（field 78・copy_alloc_speed_plan.md 8-2・D-13） ──────────────

/// セルに覚える値: 確かめたクラスの番号に印を付けたもの（0 は「まだ」なので、番号 0 のクラスと区別する）。
pub(super) fn direct_cell_value(class_id: u32) -> u64 {
    (1u64 << 32) | class_id as u64
}

/// 生成コードの表を読む: (クラスの名前, フィールドごとの (名前, `direct_field_info`))。UTF-8 でない名前・負の長さは `None`。
///
/// # Safety
/// `desc` は [`DirectClassDesc`] と、それに続く `n_fields` 個の [`DirectFieldDesc`]（生成コードが DLL の中に置く定数）を指すこと。
pub(super) unsafe fn read_direct_desc<'a>(
    desc: *const crate::interpreter::value::DirectClassDesc,
) -> Option<(&'a str, Vec<(&'a str, u64)>)> {
    use crate::interpreter::value::{DirectClassDesc, DirectFieldDesc};
    let text = |p: *const u8, len: i64| -> Option<&'a str> {
        if p.is_null() || len <= 0 {
            return None;
        }
        // SAFETY: 関数の契約（生成コードが `len` バイトの定数を渡す）
        std::str::from_utf8(unsafe { std::slice::from_raw_parts(p, len as usize) }).ok()
    };
    // SAFETY: 関数の契約
    let head: &DirectClassDesc = unsafe { &*desc };
    // raw レイアウトは 24 フィールドまで（`RawLayout::from_fields`）。それを超える表は読まない
    if !(1..=24).contains(&head.n_fields) {
        return None;
    }
    let name = text(head.name, head.name_len)?;
    // SAFETY: 表は先頭の直後に `n_fields` 個続く（並びは `repr(C)` で、どちらも 8 バイト境界）
    let fields = unsafe { std::slice::from_raw_parts(desc.add(1) as *const DirectFieldDesc, head.n_fields as usize) };
    let fields = fields.iter().map(|f| Some((text(f.name, f.name_len)?, f.info))).collect::<Option<Vec<_>>>()?;
    Some((name, fields))
}

/// field 78 の本体: インスタンスのクラスが生成コードの表のとおりなら、先頭の塊の番地。
#[inline(always)]
fn instance_block(
    rc: &std::rc::Rc<std::cell::RefCell<crate::interpreter::value::InstanceData>>,
    desc: *const crate::interpreter::value::DirectClassDesc,
    cell: *mut u64,
) -> *mut u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（8 バイト境界）で、`AtomicU64` と同じ並び。
    let cell = unsafe { &*(cell as *const AtomicU64) };
    // SAFETY: 読むだけで、読み終わるまで誰も可変に借りない
    let Ok(inst) = (unsafe { rc.try_borrow_unguarded() }) else { return std::ptr::null_mut() };
    // ⚠ クラスは `inst.class` の番号で見る。塊の先頭の語の番号で見る形を試すと k1 が 0.94x になった（`class` は
    //   どのインスタンスでも同じで速い記憶にあるが、塊はインスタンスごとの行で、ここでその行を待つことになる・7-22）
    let want = direct_cell_value(inst.class.class_id);
    if cell.load(Ordering::Relaxed) != want && !fill_direct_cell(inst, desc, cell, want) {
        return std::ptr::null_mut();
    }
    // SAFETY: 塊（`Box<[u64]>`）はインスタンスが生きている間動かない（作るのは生成と深いコピーだけ）。
    // 生成コードは、この番地を使う間そのインスタンスを借りている者がいない所でだけ読み書きする（D-13 ④）。
    unsafe { (*rc.as_ptr()).raw.as_mut_ptr() }
}

/// [`instance_block`] のセルが外れたとき: 生成コードの表とクラスを突き合わせ、合えばセルを埋めて `true`。
#[cold]
#[inline(never)]
fn fill_direct_cell(
    inst: &crate::interpreter::value::InstanceData,
    desc: *const crate::interpreter::value::DirectClassDesc,
    cell: &std::sync::atomic::AtomicU64,
    want: u64,
) -> bool {
    // SAFETY: `desc` は生成コードが DLL の中に置いた表
    let Some((name, fields)) = (unsafe { read_direct_desc(desc) }) else { return false };
    if !inst.class.matches_direct_desc(name, &fields) {
        return false;
    }
    cell.store(want, std::sync::atomic::Ordering::Relaxed);
    true
}

pub(super) extern "C" fn ar_inst_block(h: i64, desc: *const crate::interpreter::value::DirectClassDesc, cell: *mut u64) -> *mut u64 {
    if h < 3 || cell.is_null() || desc.is_null() {
        return std::ptr::null_mut();
    }
    with_state(|s| {
        let st = s.borrow();
        // 先に積まれたエラーがあれば直接の道に入れない（今までの道が何もせずに戻る）
        if st.error.is_some() {
            return std::ptr::null_mut();
        }
        match st.arena.get(h as usize) {
            Some(Value::Instance(rc)) => instance_block(rc, desc, cell),
            _ => std::ptr::null_mut(),
        }
    })
}
