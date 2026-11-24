use super::*;

/// 生成コードが出すのと同じ手順の読み出し（8-3 の IR はこの順）: 先頭の語の初期化の印を見て、立っていれば
/// 塊の先頭から `info` の位置を幅どおりに読む。立っていなければ `None`（生成コードは今までのコールバックへ回す）。
fn direct_read(blk: *const u64, idx: usize, info: u64) -> Option<Value> {
    let head = unsafe { *blk };
    if head & block::init_bit(idx) == 0 {
        return None;
    }
    let at = unsafe { (blk as *const u8).add((info & 0xFFFF) as usize) };
    macro_rules! rd {
        ($t:ty) => {
            unsafe { at.cast::<$t>().read_unaligned() }
        };
    }
    Some(match (info >> 16) & 0xF {
        1 => Value::Int(rd!(i8) as i64),
        2 => Value::Int(rd!(i16) as i64),
        3 => Value::Int(rd!(i32) as i64),
        4 => Value::Int(rd!(i64)),
        5 => Value::Int(rd!(u8) as i64),
        6 => Value::Int(rd!(u16) as i64),
        7 => Value::Int(rd!(u32) as i64),
        8 => Value::Int(rd!(u64) as i64),
        9 => Value::Float(rd!(f32) as f64),
        10 => Value::Float(rd!(f64)),
        other => panic!("unknown width code {other}"),
    })
}

/// 同じく書き込み: 不変の印が立っているか `mut` のフィールドでなければ書かない（`false`・今までのコールバックへ）。
/// 書いたら初期化の印を立てる。`int` の値を `float` のフィールドへ書くときは変換する。
fn direct_write(blk: *mut u64, idx: usize, info: u64, val: &Value) -> bool {
    let head = unsafe { *blk };
    if head & block::IMMUTABLE != 0 || info & (1 << 20) == 0 {
        return false;
    }
    let at = unsafe { (blk as *mut u8).add((info & 0xFFFF) as usize) };
    macro_rules! wr {
        ($t:ty, $v:expr) => {
            unsafe { at.cast::<$t>().write_unaligned($v) }
        };
    }
    match ((info >> 16) & 0xF, val) {
        (1 | 5, Value::Int(n)) => wr!(u8, *n as u8),
        (2 | 6, Value::Int(n)) => wr!(u16, *n as u16),
        (3 | 7, Value::Int(n)) => wr!(u32, *n as u32),
        (4 | 8, Value::Int(n)) => wr!(i64, *n),
        (9, Value::Float(f)) => wr!(f32, *f as f32),
        (9, Value::Int(n)) => wr!(f32, *n as f32),
        (10, Value::Float(f)) => wr!(f64, *f),
        (10, Value::Int(n)) => wr!(f64, *n as f64),
        _ => return false,
    }
    unsafe { *blk = head | block::init_bit(idx) };
    true
}

fn same(a: &Option<Value>, b: &Option<Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Int(x)), Some(Value::Int(y))) => x == y,
        (Some(Value::Float(x)), Some(Value::Float(y))) => x.to_bits() == y.to_bits(),
        _ => false,
    }
}

const ANNS: [&str; 12] = [
    "int8", "uint8", "int16", "uint16", "int32", "uint32", "int64", "uint64", "float32", "float64", "int", "float",
];

fn all_widths_class() -> Rc<ClassValue> {
    let own: Vec<(String, String)> = ANNS.iter().enumerate().map(|(i, a)| (format!("f{i}"), a.to_string())).collect();
    Rc::new(ClassValue {
        field_index: own.iter().enumerate().map(|(i, (n, _))| (n.clone(), i)).collect(),
        field_count: own.len(),
        field_mutability_vec: vec![true; own.len()],
        raw_layout: RawLayout::for_class(true, &own, own.len()).map(Rc::new),
        ..ClassValue::synthetic("AllWidths", alloc_class_id())
    })
}

/// 生成コードと同じ手順（塊の番地・`block` の定数・`direct_field_info`）の読み書きが、`field_value` / `store_field` と
/// 同じ値になる（8-2）: 幅ごと・未初期化・不変のインスタンス。⚠ 塊の並びか詰め方を変えるとここが落ちる。
#[test]
fn direct_access_matches_field_value() {
    let cls = all_widths_class();
    let layout = cls.raw_layout.clone().expect("all fields are numeric");
    let infos: Vec<u64> = layout.fields.iter().map(|d| direct_field_info(*d, true)).collect();
    // 幅ごとの値（符号・切り詰め・丸めが出るもの）
    let samples = [
        Value::Int(-1),
        Value::Int(0x1_23),
        Value::Int(i64::MIN + 5),
        Value::Int(0x7FFF_FFFF_FFFF),
        Value::Float(-2.75),
        Value::Float(1.0e-3),
    ];

    // 1. `store_field` で書いたものを直接読む
    for v in &samples {
        let mut inst = InstanceData::new_empty(cls.clone(), 0);
        for idx in 0..ANNS.len() {
            assert!(same(&direct_read(inst.raw.as_ptr(), idx, infos[idx]), &None), "{} starts uninitialized", ANNS[idx]);
            if !inst.store_field(idx, v.clone(), true) {
                continue; // `float` の値は整数のフィールドに入らない
            }
            let direct = direct_read(inst.raw.as_ptr(), idx, infos[idx]);
            assert!(same(&direct, &inst.field_value(idx)), "{} <- {v:?}: direct {direct:?}", ANNS[idx]);
        }
    }

    // 2. 直接書いたものを `field_value` で読む（`store_field` で書いたインスタンスと同じ値・同じ可否）
    for v in &samples {
        let mut a = InstanceData::new_empty(cls.clone(), 0);
        let mut b = InstanceData::new_empty(cls.clone(), 0);
        for idx in 0..ANNS.len() {
            let wrote = direct_write(a.raw.as_mut_ptr(), idx, infos[idx], v);
            assert_eq!(wrote, b.store_field(idx, v.clone(), true), "{} <- {v:?}", ANNS[idx]);
            assert!(same(&a.field_value(idx), &b.field_value(idx)), "{} <- {v:?}", ANNS[idx]);
            assert_eq!(a.slot_initialized(idx), wrote);
        }
        assert_eq!(a.raw, b.raw, "the whole block agrees after writing {v:?}");
    }

    // 3. 不変のインスタンス・`let` のフィールドへは直接書かない（今までのコールバックがエラーにする）
    let mut frozen = InstanceData::new_empty(cls.clone(), INST_IMMUTABLE);
    assert!(!direct_write(frozen.raw.as_mut_ptr(), 0, infos[0], &Value::Int(1)));
    assert!(!frozen.slot_initialized(0));
    let mut inst = InstanceData::new_empty(cls.clone(), 0);
    let let_info = direct_field_info(layout.fields[0], false);
    assert!(!direct_write(inst.raw.as_mut_ptr(), 0, let_info, &Value::Int(1)));
    // 不変でも読み出しは直接できる
    assert!(inst.store_field(3, Value::Int(9), true));
    inst.flags_or(INST_IMMUTABLE);
    assert!(same(&direct_read(inst.raw.as_ptr(), 3, infos[3]), &Some(Value::Int(9))));
}
