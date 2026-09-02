// tests/hashing.rs — 既定ハッシュ（`Interpreter::default_hash`）のテスト。
//
// ⚠⚠ **このファイルが守っている不変条件はただ 1 つ**:
//
//     values_eq(a, b) == true  ⇒  default_hash(a) == default_hash(b)
//
// これが破れると辞書は「入れたのに引けない」壊れ方をする。しかも**エラーにならない**ので、
// 例題でもゲートでも映らない（B1 / B2 が全ゲートを素通りしていたのと同じ構図）。
// ⇒ `equality.rs` に腕を足したら、**必ずここにペアを足す**こと。

use super::*;
use crate::interpreter::ops::hash::HashError;

/// 文を実行し、**末尾の式**の値とインタプリタを返す。
///
/// ⚠ `prepare` を通すこと。素の `Interpreter::new()` に `exec` を直接投げると
/// 解決情報が無く `VmForceError` になる（`prepare` の doc を参照）。
fn run_last(src: &str) -> (Value, Interpreter) {
    let (stmts, mut interp) = prepare(src).expect("parse/wire");
    let mut last = Value::None;
    for stmt in &stmts {
        match stmt {
            Stmt::Expr(e) => last = interp.eval(e).expect("eval"),
            other => {
                interp.exec(other).expect("exec");
            }
        }
    }
    (last, interp)
}

/// 2 つの Arrow 式を評価して `(値, 値, インタプリタ)` を返す。
fn pair(a_src: &str, b_src: &str) -> (Value, Value, Interpreter) {
    (eval_expr(a_src), eval_expr(b_src), Interpreter::new())
}

/// 「等値なら同じハッシュ」を検査する。**等値であること自体も併せて確認**する
/// （`values_eq` が偽になっていると検査が空振りして、通ってしまうため）。
fn assert_eq_implies_same_hash(a_src: &str, b_src: &str) {
    let (a, b, interp) = pair(a_src, b_src);
    assert!(
        interp.values_eq(&a, &b).unwrap(),
        "前提が崩れている: `{a_src}` と `{b_src}` が values_eq で等値でない"
    );
    let ha = Interpreter::default_hash(&a).expect("hashable");
    let hb = Interpreter::default_hash(&b).expect("hashable");
    assert_eq!(ha, hb, "等値なのにハッシュが違う: `{a_src}` と `{b_src}`");
}

/// 「等値でない」ことだけを確認する（ハッシュは一致してもよい＝衝突は正しさを壊さない）。
fn assert_not_eq(a_src: &str, b_src: &str) {
    let (a, b, interp) = pair(a_src, b_src);
    assert!(
        !interp.values_eq(&a, &b).unwrap(),
        "等値でないはずが等値になっている: `{a_src}` と `{b_src}`"
    );
}

// ---------------------------------------------------------------------------
// 不変条件: 等値 ⇒ 同じハッシュ
// ---------------------------------------------------------------------------

/// hash_matches_eq_for_primitives のテスト。
#[test]
fn test_hash_matches_eq_for_primitives() {
    for (a, b) in [
        ("1", "1"),
        ("-7", "-7"),
        ("1.5", "1.5"),
        (r#""abc""#, r#""abc""#),
        ("True", "True"),
        ("False", "False"),
        ("None", "None"),
    ] {
        assert_eq_implies_same_hash(a, b);
    }
}

/// hash_matches_eq_for_containers のテスト。
#[test]
fn test_hash_matches_eq_for_containers() {
    for (a, b) in [
        ("[1, 2, 3]", "[1, 2, 3]"),
        ("[]", "[]"),
        ("(1, 2)", "(1, 2)"),
        (r#"{"a": 1, "b": 2}"#, r#"{"a": 1, "b": 2}"#),
        ("{1, 2, 3}", "{1, 2, 3}"),
        ("[[1], [2]]", "[[1], [2]]"),
        (r#"{"k": [1, 2]}"#, r#"{"k": [1, 2]}"#),
    ] {
        assert_eq_implies_same_hash(a, b);
    }
}

/// 順序なしのコレクションは**並び順が違っても等値**なので、ハッシュも一致しなければならない。
/// ⚠ 位置依存の畳み込みを使うとここで落ちる（`Set` / `Dict` を可換に畳んでいる理由）。
#[test]
fn test_hash_is_order_independent_for_set_and_dict() {
    assert_eq_implies_same_hash("{1, 2, 3}", "{3, 1, 2}");
    assert_eq_implies_same_hash(r#"{"a": 1, "b": 2}"#, r#"{"b": 2, "a": 1}"#);
}

/// ⚠ **`-0.0 == 0.0` は真**（IEEE-754）。ビットパターンをそのまま使うと落ちる。
#[test]
fn test_negative_zero_hashes_like_zero() {
    assert_eq_implies_same_hash("-0.0", "0.0");
}

// ---------------------------------------------------------------------------
// 型厳密性（B2-b）: 型が違えば等値でない
// ---------------------------------------------------------------------------

/// `values_eq` は**値の同一性**なので型をまたがない。
/// ⚠ 式としての `1 == 1.0` が真なのは `values_eq_expr` の昇格によるもので、別の規則。
#[test]
fn test_values_eq_is_type_strict() {
    assert_not_eq("1", "1.0");
    assert_not_eq("1", "True");
    assert_not_eq("[1]", "[1.0]");
    assert_not_eq("[1, 2]", "(1, 2)");
    assert_not_eq("1", r#""1""#);
}

/// 昇格は**式としての比較**にだけ掛かる（`values_eq_expr`）。
/// ここが `values_eq` と同じになってしまうと、ハッシュとの整合が崩れる。
#[test]
fn test_expr_equality_promotes_but_value_equality_does_not() {
    let (a, b, interp) = pair("1", "1.0");
    assert!(!interp.values_eq(&a, &b).unwrap(), "値の同一性は型厳密");
    assert!(interp.values_eq_expr(&a, &b).unwrap(), "式としての比較は昇格する");
    // bool は昇格ラティスに入っていない。
    let (x, y, interp2) = pair("1", "True");
    assert!(!interp2.values_eq_expr(&x, &y).unwrap(), "bool は昇格しない");
}

// ---------------------------------------------------------------------------
// 循環と深さ
// ---------------------------------------------------------------------------

/// 循環したリストでも**ハッシュは止まる**（深さで打ち切ってよい）。
/// ⚠ `values_eq` は打ち切れない（`RecursionError` を返す）。この非対称が設計の要点。
#[test]
fn test_hash_terminates_on_cycles() {
    let (cyclic, _interp) = run_last("mut a = [1]\na.append(a)\na");
    // 落ちずに値が返ることが検査したいこと（深さ上限で畳まれる）。
    let h = Interpreter::default_hash(&cyclic).expect("循環でも止まる");
    // 決定的であること（同じ値なら何度取っても同じ）。
    assert_eq!(h, Interpreter::default_hash(&cyclic).unwrap());
}

// ---------------------------------------------------------------------------
// インスタンス
// ---------------------------------------------------------------------------

/// `__hash__` を定義したクラスのインスタンスは `NeedsDispatch` になる。
///
/// ⚠ `default_hash` は `Value::deep_clone` や `extern "C"` コールバックからも呼ばれる
/// **純粋な経路**なので、ここでインタプリタを回すことはできない。実際のディスパッチは B1-c。
#[test]
fn test_instance_with_dunder_hash_needs_dispatch() {
    let src = concat!(
        "class H:\n",
        "    let x: int\n",
        "    fn __init__(mut self, let x: int) -> None:\n",
        "        self.x = x\n",
        "    fn __hash__(let self) -> int:\n",
        "        return self.x\n",
        "H(1)",
    );
    let (inst, _interp) = run_last(src);
    assert!(
        matches!(Interpreter::default_hash(&inst), Err(HashError::NeedsDispatch)),
        "__hash__ を持つクラスは呼び出し側に委ねる"
    );
}

/// `__hash__` を**持たない**クラスのインスタンスは構造ハッシュになる。
///
/// ⚠ `values_eq` がインスタンスを構造的に比べる（`Rc::ptr_eq` が外れても
/// 同じクラス名＋全フィールドが等値なら真）ので、**ポインタでハッシュしてはいけない**。
/// 構造ハッシュだからこそ `deep_clone` がハッシュを保存する（B1-c の前提）。
#[test]
fn test_instance_without_dunder_hash_is_structural() {
    let src = concat!(
        "class P:\n",
        "    let x: int\n",
        "    fn __init__(mut self, let x: int) -> None:\n",
        "        self.x = x\n",
        "mut a = P(1)\n",
        "mut b = P(1)\n",
        "[a, b]",
    );
    let (list, interp) = run_last(src);
    let Value::List(items) = list else { panic!("not a list") };
    let items = items.borrow();
    let (a, b) = (&items[0], &items[1]);
    assert!(interp.values_eq(a, b).unwrap(), "別オブジェクトだが構造的に等値");
    assert_eq!(
        Interpreter::default_hash(a).unwrap(),
        Interpreter::default_hash(b).unwrap(),
        "構造的に等値ならハッシュも一致しなければならない"
    );
}

/// `deep_clone` は構造を保つので、**ハッシュも保たれる**。
/// ⚠ これが「複製経路（`deep_copy_value` / `deep_clone`）は保存済みハッシュを
/// そのままコピーしてよい」根拠（B1-c）。ポインタでハッシュしていると成り立たない。
#[test]
fn test_deep_clone_preserves_hash() {
    for src in ["[1, [2, 3]]", r#"{"a": [1, 2]}"#, "(1, \"x\", True)", "{1, 2}"] {
        let v = eval_expr(src);
        let cloned = v.deep_clone();
        let interp = Interpreter::new();
        assert!(interp.values_eq(&v, &cloned).unwrap(), "deep_clone は等値: {src}");
        assert_eq!(
            Interpreter::default_hash(&v).unwrap(),
            Interpreter::default_hash(&cloned).unwrap(),
            "deep_clone がハッシュを保存していない: {src}"
        );
    }
}
