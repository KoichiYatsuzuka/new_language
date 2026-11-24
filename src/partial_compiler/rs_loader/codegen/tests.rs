use super::*;

/// bytes / 数値の Vec・slice / list[str] / Option の変換を生成する（5-4）。16 進文字列はもう作らない。
#[test]
fn collection_conversions() {
    assert!(param_conversion(0, "xs", "&[i32]").contains("_num_vec_arg::<i32>(*args.add(0), \"xs\")"));
    assert!(param_conversion(1, "v", "Vec<u8>").contains("_bytes_arg(*args.add(1), \"v\")"));
    assert!(param_conversion(0, "ws", "&[&str]").contains("_str_vec_arg(*args.add(0), \"ws\")"));
    let opt = param_conversion(2, "s", "Option<String>");
    assert!(opt.contains("if *args.add(2) == TL_NONE { None }") && opt.contains("Some(_some_s)"), "{opt}");
    assert_eq!(return_conversion_expr("f()", Some("Vec<i32>")), "_ptr_from_vec::<i32>(f())");
    let b = return_conversion_expr("f()", Some("Vec<u8>"));
    assert!(b.contains("make_bytes") && !b.contains("02x"), "{b}");
    let o = return_conversion_expr("f()", Some("Option<i64>"));
    assert!(o.contains("Some(_o) => cb_make_int(_o)") && o.contains("None => TL_NONE"), "{o}");
}

/// 中継 DLL の `ArCallbacks` の写しは、本物（`native_api/mod.rs`）と同じ位置に欄を置く。`[usize; N]` は N 欄。
#[test]
fn callback_mirror_keeps_the_positions() {
    let body = ABI_HEADER.split("struct ArCallbacks {").nth(1).and_then(|b| b.split("
}").next()).expect("struct");
    let mut slot = 0usize;
    let mut at = std::collections::HashMap::new();
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//")) {
        let (name, ty) = line.split_once(':').expect("field");
        at.insert(name.trim().to_string(), slot);
        slot += match ty.trim().strip_prefix("[usize;").and_then(|n| n.trim().strip_suffix("],")) {
            Some(n) => n.trim().parse::<usize>().expect("count"),
            None => 1,
        };
    }
    assert_eq!(at["to_bool"], 38);
    assert_eq!(at["pending_error"], 39);
    assert_eq!(at["make_bytes"], 40);
    assert_eq!(at["protocol_arg"], 44);
    assert_eq!(at["protocol_ret"], 46);
    assert_eq!(at["bytes_buf_arg"], 49);
}
