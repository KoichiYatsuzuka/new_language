use super::Value;

/// `Value` は 32 バイトに収まっていること（§7.4-2 で 72→32B に縮めた成果を守る）。
/// #15 で `Str` を `String`(24B) → `Rc<str>`(16B) にしたが、最大変種は別なので不変。
#[test]
fn value_stays_32_bytes() {
    assert_eq!(std::mem::size_of::<Value>(), 32);
}
