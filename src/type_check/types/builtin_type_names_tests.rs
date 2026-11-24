use super::{InferredType, BUILTIN_TYPE_NAMES};

/// 一覧の名前はどれも `from_ann` が**組み込みとして**知っている（未知の名前の `NamedInstance` に落ちない）。
#[test]
fn builtin_type_names_resolve() {
    for t in BUILTIN_TYPE_NAMES {
        let ann = match t.name {
            "function" => "function[int]->int".to_string(),
            _ if t.arity == 0 => t.name.to_string(),
            _ => format!("{}[{}]", t.name, vec!["int"; t.arity].join(",")),
        };
        let ty = InferredType::from_ann(&ann);
        let unknown = match &ty {
            None => true,
            // `generator` は表で明示的に `NamedInstance("generator")` にしている（要素型が無いとき）
            Some(InferredType::NamedInstance(n)) => n != "generator" && ann.starts_with(n.as_str()),
            Some(_) => false,
        };
        assert!(!unknown, "`{ann}` is not a built-in type for from_ann: {ty:?}");
    }
}

/// 名前の重複が無い（補完に同じ型が 2 つ並ばない）。
#[test]
fn builtin_type_names_unique() {
    let mut seen = std::collections::HashSet::new();
    for t in BUILTIN_TYPE_NAMES {
        assert!(seen.insert(t.name), "duplicate built-in type name `{}`", t.name);
    }
    for (name, _) in crate::ast::C_ABI_TYPES {
        assert!(seen.insert(name), "C ABI type `{name}` collides with a built-in type name");
    }
}

/// `C_ABI_TYPES` と `c_abi_base_type` の中身が同じ。
#[test]
fn c_abi_types_match_base_type() {
    for (name, base) in crate::ast::C_ABI_TYPES {
        assert_eq!(crate::ast::c_abi_base_type(name), Some(*base), "{name}");
    }
    // 逆向き: `c_abi_base_type` が受ける綴りはどれも表にある
    for prefix in ["int", "uint", "float"] {
        for bits in ["8", "16", "32", "64", "128"] {
            let name = format!("{prefix}{bits}");
            if crate::ast::c_abi_base_type(&name).is_some() {
                assert!(crate::ast::C_ABI_TYPES.iter().any(|(n, _)| *n == name), "`{name}` missing from C_ABI_TYPES");
            }
        }
    }
}
