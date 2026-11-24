use super::*;

/// field 85（copy_alloc_speed_plan.md 7-16）: 数値のフィールドの値の番地。返すのは IC が当たり（public）、スロットが
/// 初期化済みで可変の印が立ち、種類どおりの `Int` / `Float` を持つときだけ。番地へ書いた値はフィールドの値になる。
/// `let` のフィールド・凍ったインスタンス・未初期化・種類の違い・private・空のセル・インスタンスでない・先に積まれた
/// 誤りは空（生成コードは今までの道へ回る）。
#[test]
fn num_field_ptr_only_for_initialized_mutable_numbers() {
    use crate::interpreter::value::{alloc_class_id, ClassValue, InstanceData};
    let cls = Rc::new(ClassValue {
        field_index: [("hits", 0usize), ("mass", 1), ("id", 2), ("unset", 3)]
            .into_iter()
            .map(|(n, i)| (n.to_string(), i))
            .collect(),
        field_count: 4,
        field_mutability_vec: vec![true, true, false, true],
        ..ClassValue::synthetic("Counter", alloc_class_id())
    });
    let new_counter = || {
        let mut inst = InstanceData::new_empty(cls.clone(), 0);
        assert!(inst.store_field(0, Value::Int(7), true));
        assert!(inst.store_field(1, Value::Float(1.5), true));
        assert!(inst.store_field(2, Value::Int(9), false));
        Rc::new(RefCell::new(inst))
    };
    let cell = |idx: usize, access: u8| {
        let c = crate::ast::AttrCache::default();
        c.fill(cls.class_id, idx, access);
        c.0.get()
    };
    let field = |rc: &Rc<RefCell<InstanceData>>, idx: usize| rc.borrow().field_value(idx);
    let ptr = |h: i64, name: &str, c: &mut u64, kind: i32| ar_num_field_ptr(h, name.as_ptr(), name.len() as i32, c, kind);
    let rc = new_counter();
    let b = push_handle(Value::Instance(rc.clone()));
    let (mut hits, mut mass, mut id, mut unset) = (cell(0, 0), cell(1, 0), cell(2, 0), cell(3, 0));

    // `int` / `float`: 番地へ書いた値がフィールドの値になる
    let p = ptr(b, "hits", &mut hits, 0);
    assert!(!p.is_null());
    unsafe { *p.cast::<i64>() += 5 };
    assert!(matches!(field(&rc, 0), Some(Value::Int(12))));
    let p = ptr(b, "mass", &mut mass, 1);
    assert!(!p.is_null());
    unsafe { *p.cast::<f64>() += 1.0 };
    assert!(matches!(field(&rc, 1), Some(Value::Float(f)) if f == 2.5));
    // 種類の違い・`let` のフィールド・未初期化
    assert!(ptr(b, "hits", &mut hits, 1).is_null());
    assert!(ptr(b, "mass", &mut mass, 0).is_null());
    assert!(ptr(b, "id", &mut id, 0).is_null());
    assert!(ptr(b, "unset", &mut unset, 0).is_null());
    // private のフィールド（IC の access が public でない）・空のセル・インスタンスでない
    let mut private = cell(0, 1);
    assert!(ptr(b, "hits", &mut private, 0).is_null());
    let mut empty = 0u64;
    assert!(ptr(b, "hits", &mut empty, 0).is_null());
    assert!(ptr(TL_NONE, "hits", &mut hits, 0).is_null());
    // 凍ったインスタンス（スロットの可変の印が倒れる）
    let frozen = new_counter();
    crate::interpreter::Interpreter::freeze_instance(&frozen);
    let fh = push_handle(Value::Instance(frozen.clone()));
    assert!(ptr(fh, "hits", &mut hits, 0).is_null());
    // 先に積まれた誤りがあれば空（誤りはそのまま）
    set_native_error("ValueError: earlier".to_string());
    assert!(ptr(b, "hits", &mut hits, 0).is_null());
    assert!(take_error().is_some_and(|e| e.contains("earlier")));
    assert!(matches!(field(&rc, 0), Some(Value::Int(12))));
}

/// field 84（copy_alloc_speed_plan.md 7-15）: 保存点まで巻き戻してから次の要素。反復の写しの要素は取り出して積み、
/// 元のリストは変わらない。保存点 -1 は巻き戻さない（field 17 と同じ）。終わり（`TL_STOP_ITER`）でも巻き戻す。
#[test]
fn iter_next_at_rewinds_and_takes_elements_from_the_copy() {
    let items = std::rc::Rc::new(RefCell::new(vec![Value::str("a"), Value::str("b")]));
    let lh = push_handle(Value::List(items.clone()));
    let it = ar_iter_from(lh);
    let len = || with_state(|s| s.borrow().arena.len());
    let text = |h: i64| match clone_value_at(h) {
        Value::Str(s) => s.to_string(),
        other => panic!("not a str: {other:?}"),
    };
    let save = len();
    // 1 周目: 巻き戻さない（-1）。周の中で一時値を 1 つ積む
    let h1 = ar_iter_next_at(it, -1);
    assert_eq!(text(h1), "a");
    push_handle(Value::str("round temporary"));
    assert_eq!(len(), save + 2);
    // 2 周目: 1 周目の値（要素と一時値）を捨ててから次の要素を積む
    let h2 = ar_iter_next_at(it, save as i64);
    assert_eq!(h2, save as i64, "the next element takes the first freed slot");
    assert_eq!(text(h2), "b");
    assert_eq!(len(), save + 1);
    // 終わり: 巻き戻して `TL_STOP_ITER`
    assert_eq!(ar_iter_next_at(it, save as i64), TL_STOP_ITER);
    assert_eq!(len(), save);
    // 元のリストは要素を持ったまま（取り出したのは写しから）
    let kept: Vec<String> = items.borrow().iter().map(|v| match v {
        Value::Str(s) => s.to_string(),
        other => panic!("not a str: {other:?}"),
    }).collect();
    assert_eq!(kept, ["a", "b"]);
    // field 17 は今までどおり巻き戻さない
    let it2 = ar_iter_from(lh);
    let before = len();
    let h = ar_iter_next(it2);
    assert_eq!(text(h), "a");
    assert_eq!(len(), before + 1);
    ar_iter_drop(it2);
    ar_iter_drop(it);
    assert!(take_error().is_none());
}

/// field 87（copy_alloc_speed_plan.md 7-20）: 写す型は要素を全部アリーナへ積み、ハンドルの並び（番地と個数）を返す。
/// 並びのハンドルは `push_value` と同じ（`None`・真偽値・小さい整数は決まったハンドル、ほかは順に新しい欄）で、要素は
/// 入口のときのもの（後でリストを変えても並びの値は変わらない）。並びを field 84 で進めるのはエラー。写さない型（集合）は
/// 個数 -1 で、field 84 が 1 つずつ取る。先に積まれた誤りがあれば `TL_NONE` と 0 個の並び。
#[test]
fn iter_begin_pushes_every_element_and_returns_the_handle_run() {
    let inner = std::rc::Rc::new(RefCell::new(vec![Value::Int(1)]));
    let items = std::rc::Rc::new(RefCell::new(vec![
        Value::None,
        Value::Bool(true),
        Value::Int(7),
        Value::Int(1_000_000),
        Value::str("s"),
        Value::List(inner.clone()),
    ]));
    let lh = push_handle(Value::List(items.clone()));
    let len = || with_state(|s| s.borrow().arena.len());
    let before = len() as i64;
    let mut out = [7i64, 7];
    let it = ar_iter_begin(lh, out.as_mut_ptr());
    assert!(it <= -2, "an iterator handle: {it}");
    assert_eq!(out[1], 6);
    // SAFETY: 並びは `iter_drop` まで動かない
    let run = unsafe { std::slice::from_raw_parts(out[0] as *const i64, 6) }.to_vec();
    assert_eq!(&run[..3], &[TL_NONE, TL_TRUE, int_cache_handle(7)]);
    assert_eq!(&run[3..], &[before, before + 1, before + 2], "the other elements take new slots in order");
    assert_eq!(len() as i64, before + 3);
    assert!(matches!(clone_value_at(run[3]), Value::Int(1_000_000)));
    assert!(matches!(clone_value_at(run[4]), Value::Str(ref s) if &**s == "s"));
    assert!(matches!(clone_value_at(run[5]), Value::List(ref l) if std::rc::Rc::ptr_eq(l, &inner)), "the element itself");
    // 後でリストを変えても並びの値は変わらない（入口で要素の参照を足してある）
    items.borrow_mut().clear();
    assert!(matches!(clone_value_at(run[4]), Value::Str(ref s) if &**s == "s"));
    // 並びを field 84 で進めない
    assert_eq!(ar_iter_next_at(it, -1), TL_STOP_ITER);
    assert!(take_error().is_some_and(|e| e.contains("handle run")));
    ar_iter_drop(it);
    assert!(take_error().is_none());
    // 空のリスト: 0 個の並び
    let eh = push_handle(Value::List(std::rc::Rc::new(RefCell::new(Vec::new()))));
    let it = ar_iter_begin(eh, out.as_mut_ptr());
    assert!(it <= -2 && out[1] == 0, "{it} {out:?}");
    ar_iter_drop(it);
    // 写さない型（集合）は 1 つずつ（反復プロトコルはインタープリタを呼ぶ）
    let set = Value::Set(std::rc::Rc::new(RefCell::new(vec![Value::Int(1), Value::Int(2), Value::Int(3)])));
    let mut interp = crate::interpreter::Interpreter::new();
    let outermost = super::enter_native_call(&mut interp as *mut _);
    let sh = push_handle(set);
    let it = ar_iter_begin(sh, out.as_mut_ptr());
    assert!(it <= -2 && out[1] == -1, "{it} {out:?}");
    let mut sum = 0;
    loop {
        let h = ar_iter_next_at(it, -1);
        if h == TL_STOP_ITER {
            break;
        }
        match clone_value_at(h) {
            Value::Int(n) => sum += n,
            other => panic!("not an int: {other:?}"),
        }
    }
    assert_eq!(sum, 6);
    ar_iter_drop(it);
    assert!(take_error().is_none());
    // 先に積まれた誤り: 何も作らない
    set_native_error("ValueError: earlier".to_string());
    out = [7, 7];
    assert_eq!(ar_iter_begin(lh, out.as_mut_ptr()), TL_NONE);
    assert_eq!(out, [0, 0]);
    assert!(take_error().is_some_and(|e| e.contains("earlier")));
    super::abort_native_call(outermost);
}

/// 並びの積み方（`push_run`・copy_alloc_speed_plan.md 7-21）: 空き欄へ直接書く種類（インスタンス・文字列・float・
/// 整数）も、`Value::clone` で写す種類（リスト）も、決まったハンドル（`None`・真偽値・小さい整数）を挟んで順に
/// 新しい欄へ入り、アリーナの長さは積んだ数だけ伸びる。要素は同じもの（`Rc` は参照の数が 1 つ増え、欄を捨てると戻る）。
/// 写しから積む型（タプル）は写しの要素を動かして積む。
#[test]
fn iter_begin_writes_each_kind_into_its_own_slot() {
    use crate::interpreter::value::{alloc_class_id, ClassValue, InstanceData};
    let cls = Rc::new(ClassValue::synthetic("Item", alloc_class_id()));
    let inst = Rc::new(RefCell::new(InstanceData::new_empty(cls, 0)));
    let s: Rc<str> = Rc::from("text");
    let inner = Rc::new(RefCell::new(vec![Value::Int(1)]));
    let items = vec![
        Value::Instance(inst.clone()),
        Value::None,
        Value::Str(s.clone()),
        Value::Bool(false),
        Value::Float(2.5),
        Value::Int(255),
        Value::Int(256),
        Value::Int(-1),
        Value::List(inner.clone()),
        Value::Bool(true),
    ];
    let lh = push_handle(Value::List(Rc::new(RefCell::new(items.clone()))));
    let len = || with_state(|s| s.borrow().arena.len());
    let (inst_rc, str_rc, list_rc) = (Rc::strong_count(&inst), Rc::strong_count(&s), Rc::strong_count(&inner));
    let before = len() as i64;
    let mut out = [7i64, 7];
    let it = ar_iter_begin(lh, out.as_mut_ptr());
    assert!(it <= -2 && out[1] == 10, "{it} {out:?}");
    // SAFETY: 並びは `iter_drop` まで動かない
    let run = unsafe { std::slice::from_raw_parts(out[0] as *const i64, 10) }.to_vec();
    let b = before;
    assert_eq!(
        run,
        [b, TL_NONE, b + 1, TL_FALSE, b + 2, int_cache_handle(255), b + 3, b + 4, b + 5, TL_TRUE],
        "fixed handles in between, the others in new slots in order"
    );
    assert_eq!(len() as i64, before + 6, "the arena grows by the pushed elements only");
    assert!(matches!(clone_value_at(run[0]), Value::Instance(ref r) if Rc::ptr_eq(r, &inst)));
    assert!(matches!(clone_value_at(run[2]), Value::Str(ref r) if Rc::ptr_eq(r, &s)));
    assert!(matches!(clone_value_at(run[4]), Value::Float(f) if f == 2.5));
    assert!(matches!(clone_value_at(run[6]), Value::Int(256)));
    assert!(matches!(clone_value_at(run[7]), Value::Int(-1)));
    assert!(matches!(clone_value_at(run[8]), Value::List(ref r) if Rc::ptr_eq(r, &inner)));
    // 参照の数は並びの分だけ 1 つ増える（`clone_value_at` の写しはもう捨ててある）
    assert_eq!(Rc::strong_count(&inst), inst_rc + 1);
    assert_eq!(Rc::strong_count(&s), str_rc + 1);
    assert_eq!(Rc::strong_count(&inner), list_rc + 1);
    ar_iter_drop(it);
    // 欄を捨てると戻る
    with_state(|st| st.borrow_mut().arena.truncate(before as usize));
    assert_eq!(Rc::strong_count(&inst), inst_rc);
    assert_eq!(Rc::strong_count(&s), str_rc);
    assert_eq!(Rc::strong_count(&inner), list_rc);
    // 写しから積む型（タプル）: 同じハンドルの並びになる
    let th = push_handle(Value::Tuple(Rc::new(crate::interpreter::TupleData::new(items))));
    let before = len() as i64;
    let it = ar_iter_begin(th, out.as_mut_ptr());
    assert!(it <= -2 && out[1] == 10, "{it} {out:?}");
    let run = unsafe { std::slice::from_raw_parts(out[0] as *const i64, 10) }.to_vec();
    let b = before;
    assert_eq!(run, [b, TL_NONE, b + 1, TL_FALSE, b + 2, int_cache_handle(255), b + 3, b + 4, b + 5, TL_TRUE]);
    assert_eq!(len() as i64, before + 6);
    assert!(matches!(clone_value_at(run[0]), Value::Instance(ref r) if Rc::ptr_eq(r, &inst)));
    assert!(matches!(clone_value_at(run[8]), Value::List(ref r) if Rc::ptr_eq(r, &inner)));
    ar_iter_drop(it);
    assert!(take_error().is_none());
}

/// ガードが外れたときの戻り値の開け方（field 80・7-5）: 宣言の型の値だけを開け、違う型は黙って変換せずに TypeError。
/// 先に積まれたエラー・例外は上書きしない。
#[test]
fn expect_ret_unboxes_only_the_declared_type() {
    let name = "Body.kinetic";
    let call = |h: i64, kind: i32| ar_expect_ret(h, kind, name.as_ptr(), name.len() as i32);
    // int: 小さい整数（番号で表す）・大きい整数
    assert_eq!(call(push_handle(Value::Int(7)), 0), 7);
    assert_eq!(call(push_handle(Value::Int(1 << 40)), 0), 1 << 40);
    assert_eq!(call(push_handle(Value::Int(-3)), 0), -3);
    // float: ビット列で返る
    assert_eq!(f64::from_bits(call(push_handle(Value::Float(2.5)), 1) as u64), 2.5);
    assert!(take_error().is_none());
    // 宣言と違う型: float に int・int に float・bool・None・文字列
    for (h, kind, got) in [
        (push_handle(Value::Int(3)), 1, "int"),
        (push_handle(Value::Float(3.0)), 0, "float"),
        (TL_TRUE, 0, "bool"),
        (TL_NONE, 1, "NoneType"),
        (ar_make_str(b"x".as_ptr(), 1), 1, "str"),
    ] {
        assert_eq!(call(h, kind), 0);
        let want = if kind == 1 { "float" } else { "int" };
        assert_eq!(
            take_error(),
            Some(format!("TypeError: method 'Body.kinetic' is declared to return '{want}', but the call returned '{got}'"))
        );
    }
    // 先に積まれたエラーは上書きしない
    set_native_error("ValueError: raised inside the method".to_string());
    assert_eq!(call(TL_NONE, 1), 0);
    assert_eq!(take_error().as_deref(), Some("ValueError: raised inside the method"));
}

/// 受け手のクラスのガード（field 64）: セルが埋まっていればクラスの名前を読まない（7-9）。セルが空のときだけ名前を
/// 検査し、正しくない UTF-8 ならエラー
#[test]
fn method_guard_reads_the_class_name_only_to_fill_the_cell() {
    use crate::interpreter::value::{alloc_class_id, ClassValue, InstanceData, INST_IMMUTABLE};
    use std::cell::RefCell;
    use std::rc::Rc;
    let cls = Rc::new(ClassValue::synthetic("Vec2", alloc_class_id()));
    let inst = |immutable: bool| {
        let mut d = InstanceData::new_empty(cls.clone(), 0);
        if immutable {
            d.set_flags(d.flags() | INST_IMMUTABLE);
        }
        push_handle(Value::Instance(Rc::new(RefCell::new(d))))
    };
    let bad = [0xFFu8, 0xFE];
    let mut cell = cls.class_id as u64;
    // セルが当たる: 名前（正しくない UTF-8）は読まず、エラーにもならない
    assert_eq!(ar_method_guard(inst(false), bad.as_ptr(), 2, &mut cell, 0), 1);
    assert_eq!(ar_method_guard(inst(false), bad.as_ptr(), 2, &mut cell, 1), 1);
    // `mut self` のメソッドに不変のインスタンス・別のクラスのセル・インスタンスでない値は外れ
    assert_eq!(ar_method_guard(inst(true), bad.as_ptr(), 2, &mut cell, 1), 0);
    let mut other = cell + 1;
    assert_eq!(ar_method_guard(inst(false), bad.as_ptr(), 2, &mut other, 0), 0);
    assert_eq!(ar_method_guard(push_handle(Value::Int(1 << 40)), bad.as_ptr(), 2, &mut cell, 0), 0);
    assert!(take_error().is_none());
    // セルが空: 名前を検査する（正しくない UTF-8 はエラー・セルは空のまま）
    let mut empty = 0u64;
    assert_eq!(ar_method_guard(inst(false), bad.as_ptr(), 2, &mut empty, 0), 0);
    assert_eq!(empty, 0);
    assert!(take_error().is_some_and(|e| e.contains("class name from native code is not valid UTF-8")));
}

/// 合成した `__init__` の生成（field 83・7-11）: 宣言から静的に求めた格納の順（`static_class_layout`）が実行時の計画と
/// 同じクラスにだけ、引数を格納する前のインスタンスを作って番地を返す。利用者が書いた `__init__`・違う順・違う表・
/// クラスでない名前・先に積まれたエラーは 0（何も作らない・エラーを積まない）。
#[test]
fn new_instance_makes_only_instances_of_the_described_synthesized_init() {
    use crate::interpreter::value::{block, direct_field_info, static_class_layout, DirectClassDesc, DirectFieldDesc};
    let src = "class V2:\n    mut x: float\n    let n: int32\n    const K: int = 3\n\n\
                   class U2:\n    mut x: float\n    let n: int32\n\n    \
                   fn __init__(mut self, let x: float, let n: int32) -> None:\n        self.x = x\n        self.n = n\n\n\
                   fn not_a_class() -> int:\n    return 1\n";
    let stmts = crate::parser::Parser::new(crate::lexer::Lexer::new(src, "t.ar").tokenize(), None)
        .parse_program()
        .expect("parse");
    let layout = |name: &str| {
        stmts.iter().find_map(|s| match s {
            crate::ast::Stmt::ClassDef { name: n, bases, body, .. } if n == name => static_class_layout(bases, body),
            _ => None,
        })
    };
    // 静的な順: 合成した `__init__` は `const` を受けない。利用者が書いた同じ形の `__init__` は合成したものでない
    let v2 = layout("V2").expect("V2 has a raw layout");
    assert_eq!(v2.auto_init, Some(vec![0, 1]));
    assert_eq!(layout("U2").expect("U2 has a raw layout").auto_init, None);

    let mut interp = crate::interpreter::Interpreter::new();
    for s in &stmts {
        interp.exec(s).expect("exec");
    }
    let outermost = super::enter_native_call(&mut interp as *mut _);
    // 生成コードの表（V2 の並び。U2 も同じ並び）
    #[repr(C)]
    struct Table(DirectClassDesc, [DirectFieldDesc; 2]);
    let table = |name: &'static str| {
        Table(
            DirectClassDesc { name: name.as_ptr(), name_len: name.len() as i64, n_fields: 2 },
            [
                DirectFieldDesc { name: "x".as_ptr(), name_len: 1, info: direct_field_info(v2.layout.fields[0], true) },
                DirectFieldDesc { name: "n".as_ptr(), name_len: 1, info: direct_field_info(v2.layout.fields[1], false) },
            ],
        )
    };
    let (tv, tu) = (table("V2"), table("U2"));
    // 1 つの箇所 = 1 組のセル・表・順（照合のセルは、その箇所の表と順で確かめたクラスを覚える）
    let call = |name: &str, t: &Table, order: &[i64], cells: &mut (u64, u64), out: &mut *mut u64| {
        let (cell, checked) = cells;
        ar_new_instance(cell, checked, name.as_ptr(), name.len() as i32, &t.0, order.as_ptr(), order.len() as i32, out)
    };

    // 表と順のとおり: インスタンスを積み、セルにクラスを覚え、先頭の塊の番地を返す。フィールドはまだ書いていない
    let (mut cells, mut out) = ((0u64, 0u64), std::ptr::null_mut());
    let h = call("V2", &tv, &[0, 1], &mut cells, &mut out);
    assert!(h >= 3 && !out.is_null());
    let Value::Instance(rc) = clone_value_at(h) else { panic!("not an instance") };
    let id1 = u64::from(rc.borrow().class.class_id) + 1;
    assert_eq!(cells, (id1, id1));
    assert_eq!(unsafe { (*rc.as_ptr()).raw.as_mut_ptr() }, out);
    let head = unsafe { *out };
    assert_eq!((head & block::CLASS_ID_MASK) as u32, rc.borrow().class.class_id);
    assert_eq!(head & (block::init_bit(0) | block::init_bit(1)), 0);
    assert!(rc.borrow().field_value(0).is_none());
    // 2 回目はセルから（名前は読まない: 正しくない UTF-8 でも作れる）
    let bad = [0xFFu8, 0xFE];
    let mut out2 = std::ptr::null_mut();
    let h2 = ar_new_instance(&mut cells.0, &mut cells.1, bad.as_ptr(), 2, &tv.0, [0i64, 1].as_ptr(), 2, &mut out2);
    assert!(h2 >= 3 && h2 != h && !out2.is_null() && out2 != out);

    // 外れ（それぞれ別の箇所）: 違う順・違う数・利用者が書いた `__init__`・表と違うクラス・クラスでない名前 → 0。
    // 何も作らず、照合のセルは空のまま、エラーも積まない
    let mut o = std::ptr::null_mut();
    for (name, t, order) in [
        ("V2", &tv, &[1i64, 0][..]),
        ("V2", &tv, &[0][..]),
        ("U2", &tu, &[0, 1][..]),
        ("V2", &tu, &[0, 1][..]),
        ("not_a_class", &tv, &[0, 1][..]),
    ] {
        let mut site = (0u64, 0u64);
        assert_eq!(call(name, t, order, &mut site, &mut o), 0, "{name} {order:?}");
        assert_eq!(site.1, 0);
    }
    assert!(o.is_null());
    assert!(take_error().is_none());
    // 先に積まれたエラーがあれば 0（エラーはそのまま残る）
    set_native_error("ValueError: earlier".to_string());
    assert_eq!(call("V2", &tv, &[0, 1], &mut cells, &mut o), 0);
    assert!(take_error().is_some_and(|e| e.contains("earlier")));
    super::abort_native_call(outermost);
}

/// 同じ親の入れ子の受け手の番地をまとめて取るコールバック（field 86・copy_alloc_speed_plan.md 7-18）: 1 件ごとの結果は
/// field 79（`ar_attr_block`）を同じ引数で呼んだのと同じ。表と違うクラスの子・インスタンスでない値・空の IC のセルの件だけ
/// 空で、ほかの件は番地。親がインスタンスでない・先に積まれた誤りがあれば全部空（誤りはそのまま）。
#[test]
fn attr_blocks_match_attr_block_for_each_request() {
    use crate::interpreter::value::{
        alloc_class_id, direct_field_info, ClassValue, DirectClassDesc, DirectFieldDesc, InstanceData, RawLayout,
    };
    let make_raw = |name: &str, fields: &[(&str, &str)]| -> Rc<ClassValue> {
        let own: Vec<(String, String)> = fields.iter().map(|(n, a)| (n.to_string(), a.to_string())).collect();
        Rc::new(ClassValue {
            field_index: fields.iter().enumerate().map(|(i, (n, _))| (n.to_string(), i)).collect(),
            field_count: fields.len(),
            field_mutability_vec: vec![true; fields.len()],
            raw_layout: RawLayout::for_class(true, &own, own.len()).map(Rc::new),
            ..ClassValue::synthetic(name, alloc_class_id())
        })
    };
    let new_inst = |cls: &Rc<ClassValue>| {
        let mut inst = InstanceData::new_empty(cls.clone(), 0);
        let _ = inst.store_field(0, Value::Float(1.5), true);
        let _ = inst.store_field(1, Value::Int(7), true);
        Rc::new(RefCell::new(inst))
    };
    let vec2 = make_raw("Vec2", &[("x", "float"), ("n", "int32")]);
    // 同じ名前で並びの違うクラス（表と照合すると外れる）
    let swapped = make_raw("Vec2", &[("n", "int32"), ("x", "float")]);
    // 親（raw レイアウトでない）: v / w / s は子のインスタンス、k は数
    let parent = Rc::new(ClassValue {
        field_index: [("v", 0usize), ("w", 1), ("s", 2), ("k", 3)].into_iter().map(|(n, i)| (n.to_string(), i)).collect(),
        field_count: 4,
        field_mutability_vec: vec![true; 4],
        ..ClassValue::synthetic("H", alloc_class_id())
    });
    let mut pinst = InstanceData::new_empty(parent.clone(), 0);
    let v_child = new_inst(&vec2);
    assert!(pinst.store_field(0, Value::Instance(v_child.clone()), true));
    assert!(pinst.store_field(1, Value::Instance(new_inst(&vec2)), true));
    assert!(pinst.store_field(2, Value::Instance(new_inst(&swapped)), true));
    assert!(pinst.store_field(3, Value::Int(3), true));
    let prc = Rc::new(RefCell::new(pinst));
    let ph = push_handle(Value::Instance(prc.clone()));

    #[repr(C)]
    struct Table(DirectClassDesc, [DirectFieldDesc; 2]);
    let layout = vec2.raw_layout.clone().unwrap();
    let table = Table(
        DirectClassDesc { name: "Vec2".as_ptr(), name_len: 4, n_fields: 2 },
        [
            DirectFieldDesc { name: "x".as_ptr(), name_len: 1, info: direct_field_info(layout.fields[0], true) },
            DirectFieldDesc { name: "n".as_ptr(), name_len: 1, info: direct_field_info(layout.fields[1], true) },
        ],
    );
    let desc = &table.0 as *const DirectClassDesc;
    let ic = |idx: usize| {
        let c = crate::ast::AttrCache::default();
        c.fill(parent.class_id, idx, 0);
        c.0.get()
    };
    // 件ごとの (名前, 読み出しの IC のセル, クラスのセル)。field 79 と同じ値を比べるため、セルは件ごとに別に持つ
    let names = ["v", "w", "s", "k"];
    let mut ics = [ic(0), ic(1), ic(2), ic(3), 0u64];
    let mut class_cells = [0u64; 5];
    let req = |name: &'static str, ic: *mut u64, cc: *mut u64| AttrBlockReq {
        name: name.as_ptr(),
        name_len: name.len() as i64,
        ic_cell: ic,
        desc,
        class_cell: cc,
    };
    let one = |name: &str, ic: &mut u64, cc: &mut u64| ar_attr_block(ph, name.as_ptr(), name.len() as i32, ic, desc, cc);
    let (mut ic79, mut cc79) = ([ic(0), ic(1), ic(2), ic(3)], [0u64; 4]);
    let want: Vec<*mut u64> = (0..4).map(|i| one(names[i], &mut ic79[i], &mut cc79[i])).collect();
    assert!(!want[0].is_null() && !want[1].is_null() && want[0] != want[1]);
    assert!(want[2].is_null() && want[3].is_null(), "a child of another layout / a number has no block");

    // 4 件 + IC のセルが空の件（v をもう 1 度・空のセル）
    let reqs = [
        req("v", &mut ics[0], &mut class_cells[0]),
        req("w", &mut ics[1], &mut class_cells[1]),
        req("s", &mut ics[2], &mut class_cells[2]),
        req("k", &mut ics[3], &mut class_cells[3]),
        req("v", &mut ics[4], &mut class_cells[4]),
    ];
    let mut out = [1usize as *mut u64; 5];
    ar_attr_blocks(ph, 5, reqs.as_ptr(), out.as_mut_ptr());
    assert_eq!(&out[..4], &want[..], "each request gets what field 79 returns");
    assert!(out[4].is_null(), "an empty IC cell gives null");
    // 親がインスタンスでない・先に積まれた誤り: 全部空（誤りはそのまま）
    for parent_h in [TL_NONE, push_handle(Value::Float(1.0))] {
        let mut out = [1usize as *mut u64; 2];
        ar_attr_blocks(parent_h, 2, reqs.as_ptr(), out.as_mut_ptr());
        assert!(out.iter().all(|p| p.is_null()));
    }
    // 借りられているインスタンス（7-22・借りずに読む）: 読むだけの借用は通る。可変の借用はその件だけ空（親なら全部空）で、
    // panic しない。field 79 も同じ。借用を返せば元どおり
    let blocks = || {
        let mut out = [1usize as *mut u64; 2];
        ar_attr_blocks(ph, 2, reqs.as_ptr(), out.as_mut_ptr());
        out
    };
    {
        let _shared = v_child.borrow();
        assert_eq!(&blocks()[..], &want[..2], "a shared borrow does not stop the read");
    }
    {
        let _held = v_child.borrow_mut();
        let out = blocks();
        assert!(out[0].is_null(), "a mutably borrowed child gives null");
        assert_eq!(out[1], want[1]);
        assert!(one("v", &mut ic79[0], &mut cc79[0]).is_null());
    }
    {
        let _held = prc.borrow_mut();
        assert!(blocks().iter().all(|p| p.is_null()), "a mutably borrowed parent gives nulls");
        assert!(one("w", &mut ic79[1], &mut cc79[1]).is_null());
    }
    assert_eq!(&blocks()[..], &want[..2]);
    assert_eq!(one("v", &mut ic79[0], &mut cc79[0]), want[0]);
    set_native_error("ValueError: earlier".to_string());
    let mut out = [1usize as *mut u64; 2];
    ar_attr_blocks(ph, 2, reqs.as_ptr(), out.as_mut_ptr());
    assert!(out.iter().all(|p| p.is_null()));
    assert!(take_error().is_some_and(|e| e.contains("earlier")));
}

/// 番地を返すコールバック（field 78・8-2）: クラスが生成コードの表のとおりのときだけ番地を返し、セルに覚える。
/// 表と 1 つでも違うクラス・インスタンスでない値・先に積まれたエラーがあるときは空。
#[test]
fn inst_block_returns_the_block_only_for_the_described_class() {
    use crate::interpreter::value::{
        alloc_class_id, block, direct_field_info, ClassValue, DirectClassDesc, DirectFieldDesc, InstanceData,
        RawLayout,
    };
    use std::cell::RefCell;
    use std::rc::Rc;
    let make_class = |name: &str, fields: &[(&str, &str, bool)]| -> Rc<ClassValue> {
        let own: Vec<(String, String)> = fields.iter().map(|(n, a, _)| (n.to_string(), a.to_string())).collect();
        Rc::new(ClassValue {
            field_index: fields.iter().enumerate().map(|(i, (n, _, _))| (n.to_string(), i)).collect(),
            field_count: fields.len(),
            field_mutability_vec: fields.iter().map(|(_, _, m)| *m).collect(),
            raw_layout: RawLayout::for_class(true, &own, own.len()).map(Rc::new),
            ..ClassValue::synthetic(name, alloc_class_id())
        })
    };
    let new_inst = |cls: &Rc<ClassValue>| {
        let mut inst = InstanceData::new_empty(cls.clone(), 0);
        // 並びの違うクラスでは型が合わず入らないものがある（番地が返らないことだけを見るので構わない）
        let _ = inst.store_field(0, Value::Float(1.5), true);
        let _ = inst.store_field(1, Value::Int(7), true);
        push_handle(Value::Instance(Rc::new(RefCell::new(inst))))
    };
    // 生成コードの表: 先頭 + フィールド 2 つ（`repr(C)` の並びのまま 1 つの定数にする）
    #[repr(C)]
    struct Table(DirectClassDesc, [DirectFieldDesc; 2]);
    let vec2 = make_class("Vec2", &[("x", "float", true), ("n", "int32", true)]);
    let layout = vec2.raw_layout.clone().unwrap();
    let table = Table(
        DirectClassDesc { name: "Vec2".as_ptr(), name_len: 4, n_fields: 2 },
        [
            DirectFieldDesc { name: "x".as_ptr(), name_len: 1, info: direct_field_info(layout.fields[0], true) },
            DirectFieldDesc { name: "n".as_ptr(), name_len: 1, info: direct_field_info(layout.fields[1], true) },
        ],
    );
    let desc = &table.0 as *const DirectClassDesc;
    let mut cell = 0u64;

    // 表のとおりのクラス: 番地が返り、セルが埋まる。番地の先頭の語は class_id、フィールドは表の位置にある
    let h = new_inst(&vec2);
    let p = ar_inst_block(h, desc, &mut cell);
    assert!(!p.is_null());
    assert_eq!(cell, direct_cell_value(vec2.class_id));
    let head = unsafe { *p };
    assert_eq!((head & block::CLASS_ID_MASK) as u32, vec2.class_id);
    assert_ne!(head & block::init_bit(0), 0);
    let x_at = (table.1[0].info & 0xFFFF) as usize;
    assert_eq!(unsafe { (p as *const u8).add(x_at).cast::<f64>().read_unaligned() }, 1.5);
    // 2 回目は同じ番地（セルが当たる）
    assert_eq!(ar_inst_block(h, desc, &mut cell), p);

    // 同じ名前で並びの違うクラス（フィールドの順・幅・`let`・private）は空。セルは変わらない
    let mut private = make_class("Vec2", &[("x", "float", true), ("n", "int32", true)]);
    Rc::get_mut(&mut private).unwrap().field_access.insert("n".to_string(), crate::ast::Accessibility::Private);
    for other in [
        make_class("Vec2", &[("n", "int32", true), ("x", "float", true)]),
        make_class("Vec2", &[("x", "float", true), ("n", "int", true)]),
        make_class("Vec2", &[("x", "float", true), ("n", "int32", false)]),
        make_class("Vec3", &[("x", "float", true), ("n", "int32", true)]),
        make_class("Vec2", &[("x", "float", true), ("n", "int32", true), ("z", "float", true)]),
        private,
    ] {
        let h = new_inst(&other);
        assert!(ar_inst_block(h, desc, &mut cell).is_null(), "class {:?} must not match", other.field_index);
        assert_eq!(cell, direct_cell_value(vec2.class_id));
    }

    // インスタンスでない値・小さい番号・空の引数
    assert!(ar_inst_block(push_handle(Value::Float(1.0)), desc, &mut cell).is_null());
    assert!(ar_inst_block(TL_NONE, desc, &mut cell).is_null());
    assert!(ar_inst_block(h, std::ptr::null(), &mut cell).is_null());
    assert!(ar_inst_block(h, desc, std::ptr::null_mut()).is_null());
    // 可変に借りられているインスタンスは空で、panic しない（借りずに読む・7-22）。借用を返せば同じ番地
    let rc = match clone_value_at(h) {
        Value::Instance(rc) => rc,
        other => panic!("not an instance: {other:?}"),
    };
    {
        let _held = rc.borrow_mut();
        assert!(ar_inst_block(h, desc, &mut cell).is_null(), "a mutably borrowed instance gives null");
    }
    assert_eq!(ar_inst_block(h, desc, &mut cell), p);
    // 先に積まれたエラーがあれば空（エラーはそのまま残る）
    set_native_error("ValueError: earlier".to_string());
    assert!(ar_inst_block(h, desc, &mut cell).is_null());
    assert!(take_error().is_some_and(|e| e.contains("earlier")));
    assert!(!ar_inst_block(h, desc, &mut cell).is_null());

    // ── 数値の書き込みの命中経路（8-8）: 当たれば 1 回の借用で検査と格納。エラーの文面は今までと同じ ──
    {
        use crate::vm::op::TypeTag;
        let body = Rc::new(ClassValue {
            field_index: [("name", 0usize), ("mass", 1), ("hits", 2), ("id", 3)]
                .into_iter()
                .map(|(n, i)| (n.to_string(), i))
                .collect(),
            field_count: 4,
            field_mutability_vec: vec![true, true, true, false],
            field_tags: vec![TypeTag::Str, TypeTag::Float, TypeTag::Int, TypeTag::Int],
            ..ClassValue::synthetic("Body", alloc_class_id())
        });
        let new_body = |flags: u32| {
            let mut inst = InstanceData::new_empty(body.clone(), 0);
            assert!(inst.store_field(0, Value::Str("b".into()), true));
            assert!(inst.store_field(1, Value::Float(1.0), true));
            assert!(inst.store_field(2, Value::Int(0), true));
            assert!(inst.store_field(3, Value::Int(9), false));
            inst.flags_or(flags);
            push_handle(Value::Instance(Rc::new(RefCell::new(inst))))
        };
        let field = |h: i64, idx: usize| match clone_value_at(h) {
            Value::Instance(rc) => rc.borrow().field_value(idx),
            other => panic!("{other:?}"),
        };
        let cell_for = |idx: usize| {
            let c = crate::ast::AttrCache::default();
            c.fill(body.class_id, idx, crate::ast::AttrCache::PUBLIC);
            c.0.get()
        };
        let (mut mass, mut hits, mut id) = (cell_for(1), cell_for(2), cell_for(3));
        let b = new_body(0);
        // 当たり: float・int・`int` を `float` のフィールドへ（昇格）
        ar_assign_float_field_ic(b, "mass".as_ptr(), 4, 2.5, &mut mass);
        ar_assign_int_field_ic(b, "hits".as_ptr(), 4, 7, &mut hits);
        assert!(take_error().is_none());
        assert!(matches!(field(b, 1), Some(Value::Float(f)) if f == 2.5));
        assert!(matches!(field(b, 2), Some(Value::Int(7))));
        ar_assign_int_field_ic(b, "mass".as_ptr(), 4, 3, &mut mass);
        assert!(take_error().is_none());
        assert!(matches!(field(b, 1), Some(Value::Float(f)) if f == 3.0));
        // 型の合わない値・`let` のフィールド・不変のインスタンス: 共有の検査（`store_field_checked`）と同じ文面で、値は変わらない
        ar_assign_float_field_ic(b, "hits".as_ptr(), 4, 1.5, &mut hits);
        assert_eq!(take_error().as_deref(), Some("TypeError: value does not match declared type of field 'hits'"));
        assert!(matches!(field(b, 2), Some(Value::Int(7))));
        ar_assign_int_field_ic(b, "id".as_ptr(), 2, 1, &mut id);
        assert_eq!(take_error().as_deref(), Some("TypeError: cannot assign to immutable field 'id'"));
        assert!(matches!(field(b, 3), Some(Value::Int(9))));
        let frozen = new_body(crate::interpreter::value::INST_IMMUTABLE);
        ar_assign_float_field_ic(frozen, "mass".as_ptr(), 4, 9.0, &mut mass);
        // 同じ形のもう 1 つのインスタンスに、共有の検査を直接かけた結果と比べる（エラーも、格納された値も）
        let twin = new_body(crate::interpreter::value::INST_IMMUTABLE);
        let direct = {
            let Value::Instance(rc) = clone_value_at(twin) else { panic!() };
            let mut inst = rc.borrow_mut();
            crate::interpreter::Interpreter::store_field_checked(&mut inst, 1, "mass", Value::Float(9.0), true)
        };
        assert_eq!(take_error(), direct.err(), "the same error as the shared check");
        assert!(matches!((field(frozen, 1), field(twin, 1)), (Some(Value::Float(a)), Some(Value::Float(b))) if a == b));
        // 先に積まれたエラーがあれば書かない（エラーはそのまま）
        set_native_error("ValueError: earlier".to_string());
        ar_assign_float_field_ic(b, "mass".as_ptr(), 4, 8.0, &mut mass);
        assert!(take_error().is_some_and(|e| e.contains("earlier")));
        assert!(matches!(field(b, 1), Some(Value::Float(f)) if f == 3.0));
        // 当たらない（別のクラスのセル・空のセル・インスタンスでない）ときは命中経路は何もしない
        let mut other = cell_for(1) ^ (1u64 << 40);
        assert!(!assign_number_ic_hit(b, "mass".as_ptr(), 4, Value::Float(8.0), &mut other));
        let mut empty = 0u64;
        assert!(!assign_number_ic_hit(b, "mass".as_ptr(), 4, Value::Float(8.0), &mut empty));
        assert!(!assign_number_ic_hit(TL_NONE, "mass".as_ptr(), 4, Value::Float(8.0), &mut mass));
        assert!(matches!(field(b, 1), Some(Value::Float(f)) if f == 3.0));
    }

    // ── 入れ子の受け手（field 79・8-7）: 親のフィールドが持っているインスタンスの番地。積まない ──
    let holder = Rc::new(ClassValue {
        field_index: [("p".to_string(), 0usize), ("tag".to_string(), 1)].into_iter().collect(),
        field_count: 2,
        field_mutability_vec: vec![true, true],
        ..ClassValue::synthetic("Holder", alloc_class_id())
    });
    let child = match clone_value_at(new_inst(&vec2)) {
        Value::Instance(rc) => rc,
        other => panic!("{other:?}"),
    };
    let mut hinst = InstanceData::new_empty(holder.clone(), 0);
    assert!(hinst.store_field(0, Value::Instance(child.clone()), true));
    assert!(hinst.store_field(1, Value::Int(3), true));
    let ph = push_handle(Value::Instance(Rc::new(RefCell::new(hinst))));
    let mut ic = 0u64;
    let mut ccell = 0u64;
    // 読み出しの IC のセルが空のうちは空（生成コードが field 58 で読んでセルを埋める）
    assert!(ar_attr_block(ph, "p".as_ptr(), 1, &mut ic, desc, &mut ccell).is_null());
    let cache = crate::ast::AttrCache::default();
    cache.fill(holder.class_id, 0, crate::ast::AttrCache::PUBLIC);
    ic = cache.0.get();
    let arena_len = with_state(|s| s.borrow().arena.len());
    let cp = ar_attr_block(ph, "p".as_ptr(), 1, &mut ic, desc, &mut ccell);
    assert_eq!(cp, unsafe { (*child.as_ptr()).raw.as_mut_ptr() }, "the block of the instance the field holds");
    assert_eq!(with_state(|s| s.borrow().arena.len()), arena_len, "nothing is pushed");
    // slot が持っている値がインスタンスでない・親が別のクラス（IC が外れる）・親がインスタンスでない
    cache.fill(holder.class_id, 1, crate::ast::AttrCache::PUBLIC);
    let mut ic_tag = cache.0.get();
    assert!(ar_attr_block(ph, "tag".as_ptr(), 3, &mut ic_tag, desc, &mut ccell).is_null());
    assert!(ar_attr_block(h, "p".as_ptr(), 1, &mut ic, desc, &mut ccell).is_null());
    assert!(ar_attr_block(TL_NONE, "p".as_ptr(), 1, &mut ic, desc, &mut ccell).is_null());
}

/// 正しい UTF-8 は `str` になる（日本語を含む）。
#[test]
fn make_str_accepts_valid_utf8() {
    let text = "日本語 abc";
    let h = ar_make_str(text.as_ptr(), text.len() as i32);
    assert!(take_error().is_none());
    assert!(matches!(clone_value_at(h), Value::Str(s) if &*s == text));
    // 長さ 0 は NULL でも空文字列（`from_raw_parts` に NULL を渡さない）
    let h = ar_make_str(std::ptr::null(), 0);
    assert!(take_error().is_none());
    assert!(matches!(clone_value_at(h), Value::Str(s) if s.is_empty()));
}

/// 不正な UTF-8・負の長さ・長さのある NULL は、黙って受け取らずにエラーになる（1-4）。
#[test]
fn make_str_rejects_invalid_input() {
    let cp932 = [0x93u8, 0xfa, 0x96, 0x7b]; // 「日本」の CP932
    assert_eq!(ar_make_str(cp932.as_ptr(), cp932.len() as i32), TL_NONE);
    assert!(take_error().is_some_and(|e| e.contains("not valid UTF-8")));

    assert_eq!(ar_make_str(b"abc".as_ptr(), -1), TL_NONE);
    assert!(take_error().is_some_and(|e| e.contains("invalid string")));

    assert_eq!(ar_make_str(std::ptr::null(), 3), TL_NONE);
    assert!(take_error().is_some_and(|e| e.contains("invalid string")));
}

/// C の文字列へは str だけ。None などや NUL を含む str は、黙って空文字列にせずエラーで NULL（1-2）。
#[test]
fn to_cstr_accepts_only_str_without_nul() {
    let h = ar_make_str(b"abc".as_ptr(), 3);
    let p = ar_to_cstr(h);
    assert!(!p.is_null());
    assert_eq!(unsafe { std::ffi::CStr::from_ptr(p as *const std::ffi::c_char) }.to_bytes(), b"abc");
    assert!(take_error().is_none());

    assert!(ar_to_cstr(TL_NONE).is_null());
    assert!(take_error().is_some_and(|e| e.contains("needs a str") && e.contains("NoneType")));

    let h = ar_make_str(b"a\0b".as_ptr(), 3);
    assert!(ar_to_cstr(h).is_null());
    assert!(take_error().is_some_and(|e| e.contains("NUL")));
}

/// 数値でない値は 0 にせずエラーにする。最初の誤りを残す。数値どうしは C の暗黙の変換のまま（7-12）。
#[test]
fn to_int_and_to_float_reject_non_numbers() {
    let s = ar_make_str(b"abc".as_ptr(), 3);
    assert_eq!(ar_pending_error(), 0);
    assert_eq!(ar_to_int(s), 0);
    assert_eq!(ar_pending_error(), 1);
    // 2 つ目の誤りで 1 つ目を上書きしない
    assert_eq!(ar_to_float(TL_NONE), 0.0);
    assert!(take_error().is_some_and(|e| e.contains("int argument must be a number") && e.contains("'str'")));
    assert_eq!(ar_pending_error(), 0);
    assert_eq!(ar_to_float(TL_NONE), 0.0);
    assert!(take_error().is_some_and(|e| e.contains("float argument") && e.contains("NoneType")));
    let xs = ar_make_list(std::ptr::null(), 0);
    assert_eq!(ar_to_int(xs), 0);
    assert!(take_error().is_some_and(|e| e.contains("'list'")));
    // 数値どうし
    assert_eq!(ar_to_int(push_handle(Value::Float(3.7))), 3);
    assert_eq!(ar_to_int(TL_TRUE), 1);
    assert_eq!(ar_to_int(push_handle(Value::Int(1 << 40))), 1 << 40);
    assert_eq!(ar_to_float(push_handle(Value::Int(5))), 5.0);
    assert!(take_error().is_none());
}

/// `bool` の引数は `True` / `False` だけ。以前は `True` 以外を黙って偽にしていた（7-12）。
#[test]
fn to_bool_accepts_only_bools() {
    assert_eq!(ar_to_bool(TL_TRUE), 1);
    assert_eq!(ar_to_bool(TL_FALSE), 0);
    assert!(take_error().is_none());
    // 小さい整数はアリーナに入らない（番号で表す）ので、型名を取り違えない
    assert_eq!(ar_to_bool(push_handle(Value::Int(1))), 0);
    assert!(take_error().is_some_and(|e| e.contains("must be a bool") && e.contains("'int'")));
    assert_eq!(ar_to_bool(TL_NONE), 0);
    assert!(take_error().is_some_and(|e| e.contains("NoneType")));
}

/// `char*` の戻り値は bytes、ほかのポインタは借用の Ptr、NULL は None（4-3）。
#[test]
fn raw_pointer_returns() {
    let text = b"hi\0";
    let h = ar_make_bytes(text.as_ptr(), 2);
    assert!(matches!(clone_value_at(h), Value::Bytes(b) if &*b == b"hi"));
    assert!(matches!(clone_value_at(ar_make_bytes(std::ptr::null(), 0)), Value::Bytes(b) if b.is_empty()));
    assert_eq!(ar_make_ptr(0, b"void".as_ptr(), 4), TL_NONE);
    let p = ar_make_ptr(0x1000, b"Db".as_ptr(), 2);
    match clone_value_at(p) {
        Value::Ptr(p) => {
            assert_eq!(p.elem_type(), "Db");
            assert!(p.release().unwrap_err().contains("borrowed"), "Arrow never frees a raw pointer");
        }
        other => panic!("{other:?}"),
    }
    assert!(take_error().is_none());
}

/// `const char*` の引数は bytes か None。str・NUL を含む bytes はエラー（4-3・D-13）。
#[test]
fn bytes_arg_takes_bytes_or_none() {
    let b = push_handle(Value::bytes(b"abc".to_vec()));
    let p = ar_bytes_arg(b);
    assert_eq!(unsafe { std::ffi::CStr::from_ptr(p as *const std::ffi::c_char) }.to_bytes(), b"abc");
    assert!(ar_bytes_arg(TL_NONE).is_null());
    assert!(take_error().is_none(), "None is NULL, not an error");
    let s = ar_make_str(b"abc".as_ptr(), 3);
    assert!(ar_bytes_arg(s).is_null());
    assert!(take_error().is_some_and(|e| e.contains("needs bytes, not 'str'") && e.contains("encode")));
    let nul = push_handle(Value::bytes(b"a\0b".to_vec()));
    assert!(ar_bytes_arg(nul).is_null());
    assert!(take_error().is_some_and(|e| e.contains("NUL")));
}

/// `void*` / 構造体ポインタの引数は Ptr（要素型を照合）か None（4-3）。
#[test]
fn ptr_arg_checks_the_element_type() {
    let db = ar_make_ptr(0x2000, b"Db".as_ptr(), 2);
    assert_eq!(ar_ptr_arg(db, b"Db".as_ptr(), 2), 0x2000);
    assert_eq!(ar_ptr_arg(db, b"void".as_ptr(), 4), 0x2000, "any Ptr goes to void*");
    assert_eq!(ar_ptr_arg(TL_NONE, b"Db".as_ptr(), 2), 0);
    assert!(take_error().is_none());
    assert_eq!(ar_ptr_arg(db, b"File".as_ptr(), 4), 0);
    assert!(take_error().is_some_and(|e| e.contains("Ptr[File]") && e.contains("Ptr[Db]")));
    assert_eq!(ar_ptr_arg(push_handle(Value::Int(1 << 40)), b"void".as_ptr(), 4), 0);
    assert!(take_error().is_some_and(|e| e.contains("needs a Ptr or None") && e.contains("'int'")));
}

/// 名前を受け取るコールバックも同じ検査を通る（不正なら呼び出しのエラー）。
#[test]
fn name_callbacks_reject_invalid_utf8() {
    let bad = [0xffu8, 0xfe];
    assert_eq!(ar_get_global(bad.as_ptr(), bad.len() as i32), TL_NONE);
    assert!(take_error().is_some_and(|e| e.contains("global name") && e.contains("not valid UTF-8")));
}
