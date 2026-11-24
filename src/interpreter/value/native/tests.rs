use super::*;

/// AbiTy::OutPtr のエンコード（初期値 → width 幅）とデコード（C 書き込み後 → Value）の
/// 幅変換規則（切り詰め・符号拡張・f32↔f64）を検証する。
#[test]
fn out_ptr_encode_decode_roundtrip() {
    use RawWidth::*;
    // f64: ビットパターンそのまま
    let enc = encode_out_ptr_init(&Value::Float(5.0), F64).unwrap();
    assert!(matches!(decode_out_ptr(enc, F64).unwrap(), Value::Float(f) if f == 5.0));
    // f32: 縮小格納 → 拡張読み出し（f32 で表現可能な値は往復不変）
    let enc = encode_out_ptr_init(&Value::Float(2.5), F32).unwrap();
    assert!(matches!(decode_out_ptr(enc, F32).unwrap(), Value::Float(f) if f == 2.5));
    // i32: 切り詰め格納・符号拡張読み出し
    let enc = encode_out_ptr_init(&Value::Int(-7), I32).unwrap();
    assert!(matches!(decode_out_ptr(enc, I32).unwrap(), Value::Int(-7)));
    // i64
    let enc = encode_out_ptr_init(&Value::Int(1 << 40), I64).unwrap();
    assert!(matches!(decode_out_ptr(enc, I64).unwrap(), Value::Int(n) if n == 1 << 40));
    // int → float 昇格（ハンドル経路の ar_to_float と同義）
    let enc = encode_out_ptr_init(&Value::Int(3), F64).unwrap();
    assert!(matches!(decode_out_ptr(enc, F64).unwrap(), Value::Float(f) if f == 3.0));
    // 型不一致（float を int* へ / 非数値）→ None（typed 経路を諦める）
    assert!(encode_out_ptr_init(&Value::Float(1.5), I32).is_none());
    assert!(encode_out_ptr_init(&Value::Bool(true), F64).is_none());
}

/// C の幅と符号どおりに広げる。符号なし 64 ビットの 2^63 以上は OverflowError（1-6）。
#[test]
fn out_ptr_decode_follows_c_width_and_sign() {
    use RawWidth::*;
    assert!(matches!(decode_out_ptr(0xFA, U8).unwrap(), Value::Int(250)));
    assert!(matches!(decode_out_ptr(0xFA, I8).unwrap(), Value::Int(-6)));
    assert!(matches!(decode_out_ptr(0xFFFE, I16).unwrap(), Value::Int(-2)));
    assert!(matches!(decode_out_ptr(0xFFFF_FFFF, U32).unwrap(), Value::Int(4294967295)));
    assert!(matches!(decode_out_ptr(42, U64).unwrap(), Value::Int(42)));
    assert!(matches!(decode_out_ptr(i64::MAX as u64, U64).unwrap(), Value::Int(i64::MAX)));
    let err = decode_out_ptr(u64::MAX, U64).unwrap_err();
    assert!(err.starts_with("OverflowError: "), "{err}");
    // 符号付き 64 ビットは同じビットでも負の値として読む（エラーではない）
    assert!(matches!(decode_out_ptr(u64::MAX, I64).unwrap(), Value::Int(-1)));
}
