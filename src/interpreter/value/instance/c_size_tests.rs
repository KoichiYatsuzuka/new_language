use super::*;

/// C の sizeof（配列の要素の間隔・4-6）。`total_bytes` は 8 の倍数に切り上げてあるので別物。
#[test]
fn c_size_is_sizeof() {
    let rgba = RawLayout::from_fields(&[
        ("r".into(), "uint8".into()),
        ("g".into(), "uint8".into()),
        ("b".into(), "uint8".into()),
        ("a".into(), "uint8".into()),
    ])
    .unwrap();
    assert_eq!((rgba.c_size(), rgba.total_bytes), (4, 8));
    let pair = RawLayout::from_fields(&[("x".into(), "int32".into()), ("y".into(), "float64".into())]).unwrap();
    assert_eq!(pair.c_size(), 16);
}
