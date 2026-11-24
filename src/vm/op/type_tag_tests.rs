use super::*;
use crate::interpreter::ops::typecheck::primitive_ann_matches;
use crate::interpreter::Value;

/// **`TypeTag` と実行時の表がずれていないこと**（タスク 6.1）。
///
/// ⚠⚠ これがこのタスクの本体。`TypeTag::matches`（VM のホットパス・直書き）と
/// `primitive_ann_matches`（実行時の唯一の表）は**同じ答えを返さなければならない**が、
/// 別々に書いてある以上ずれうる。実際 `uint` では
/// 「`x is uint` は False・`block ->uint` は TypeError・フィールドは通る」という
/// **3 者不一致**が起きていた。
///
/// ⇒ 片方を直したらここが落ちる。**2 本あることを許す代わりに、ずれを検出する網**。
#[test]
fn tag_agrees_with_runtime_table() {
    let values = [
        Value::Int(1),
        Value::UInt(1),
        Value::Float(1.0),
        Value::Str("a".into()),
        Value::Bool(true),
        Value::None,
    ];
    // `TypeTag::of` が `Other` 以外を返す名前だけが対象（`Other` は
    // 呼び出し側が一般判定へ落とすので、表と一致する必要は無い）。
    for ann in ["Any", "int", "uint", "float", "str", "bool"] {
        let tag = TypeTag::of(ann);
        assert!(!matches!(tag, TypeTag::Other), "{ann} should have a tag");
        for v in &values {
            let from_table = primitive_ann_matches(ann, v)
                .unwrap_or_else(|| panic!("table does not know `{ann}`"));
            assert_eq!(
                tag.matches(v),
                from_table,
                "TypeTag::matches と primitive_ann_matches がずれている: ann={ann} value={v:?}"
            );
        }
    }
}
