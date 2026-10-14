// interpreter/ffi_boundary.rs — FFI 境界検査（#16 段階(b)(ii) の (A) 対応）。
//
// # 何を解いているか
//
// 動的型付け言語（Python / JavaScript）のスタブは、**向こう側の型注釈を信用して**
// Arrow の型に写しているだけで、実行時に守られる保証がない。
// たとえば Python の `def f() -> int: return "s"` は Python 自身が注釈を強制しないため、
// Arrow の静的型検査は素通りし、`int` と宣言したパラメータに str が入り込む。
// 実測ではエラーも警告も出ず `x * 2` が文字列反復になる（＝静かに誤った答えが出る）。
//
// そこで **外部の値が Arrow へ入る瞬間**（＝外部関数呼び出しの戻り値）に、
// スタブが宣言した型と実際の値を突き合わせる。スタブが宣言した型が検査の根拠になるので、
// **スタブを整備するほど検査が強くなる**（引数の静的型が動的なときだけ発火する
// 従来の `CheckBefore` ルールとは逆向きで、スタブ整備の方針と矛盾しない）。
//
// # 言語ごとの設計
//
// 値が `Value` に変換された後の突き合わせ自体は言語非依存だが、**境界の表現差**は言語ごとに違う。
// 最も分かりやすい例が数値で、JavaScript は数値がすべて f64 なので `-> int` と宣言された関数が
// `3` を返しても Arrow には `Value::Float(3.0)` として届く。ここで素朴に「int か？」と
// 検査すると**正しいコードが落ちる**。一方 Python は int と float を区別して届く。
//
// このため言語ごとに [`BoundaryChecker`] を実装し、共通判定 [`check_common`] を土台に
// 自言語固有の緩和だけを上書きする形にした。
//
// # 言語を追加するとき
//
// 1. `BoundaryChecker` を実装した struct を足す（共通判定で足りるなら `check_common` に委譲するだけ）
// 2. [`checker_for`] に 1 行足す
//
// これ以外の変更は要らない（呼び出し側・エラー生成・値の差し替えは共通実装）。

use crate::interpreter::Value;
use crate::type_check::InferredType;

/// 境界検査の判定結果。
#[derive(Debug)]
pub(crate) enum Verdict {
    /// 宣言型と矛盾しない。値はそのまま通す。
    Ok,
    /// 矛盾しないが、境界の表現差を埋めるため値を差し替えて通す。
    /// 例: JS の数値は常に f64 なので `int` 宣言なら整数値の `Float` を `Int` へ寄せる。
    Coerce(Value),
    /// 宣言型と矛盾する。`actual` は実際に届いた型の名前。
    Mismatch { actual: String },
    /// この宣言型は動的に検査できないので通す（`Any` / `Unresolved` / 関数型など）。
    Unverifiable,
}

/// 外部言語ごとの境界検査器。
pub(crate) trait BoundaryChecker {
    /// エラーメッセージに出す言語名。
    fn lang(&self) -> &'static str;

    /// 外部から届いた `value` が、スタブの宣言型 `declared` と矛盾しないかを判定する。
    fn check(&self, value: &Value, declared: &InferredType) -> Verdict;
}

/// `import[lang]` の言語タグから検査器を引く。**未対応の言語は `None`（＝無検査）**。
///
/// 静的型付け言語（C/C++・C#・Rust）は向こう側が型を守るため対象外。
/// また C/C++ の `void*` は Arrow の `int` に落ちるので、型タグでの動的検査では何も言えない
/// （出所・生存期間の追跡が要る別軸の課題）。
pub(crate) fn checker_for(lang: &str) -> Option<&'static dyn BoundaryChecker> {
    match lang {
        "py" | "py-int" => Some(&PythonChecker),
        "js-proc" => Some(&JavaScriptChecker),
        _ => None,
    }
}

// ── 共通判定 ─────────────────────────────────────────────────────────────────

/// 言語非依存の突き合わせ。各言語の実装はこれを土台にして固有の緩和だけを足す。
///
/// **保守的側**: 判定できない宣言型は [`Verdict::Unverifiable`] を返して通す。
/// 誤検知で正しいコードを落とすより、取りこぼしを許す方に倒す。
pub(crate) fn check_common(value: &Value, declared: &InferredType) -> Verdict {
    match declared {
        // 検査対象外（そもそも何でも入る宣言）。
        InferredType::Any
        | InferredType::Unresolved
        // ⚠ `Never`（⊥）は空コレクションリテラルの要素型にしか現れない推論の内部表現で、
        //    FFI 境界の宣言型には書けない。届いたら判定しようがないので通す（保守的側）。
        | InferredType::Never
        // ⚠ イテレータは FFI 境界の宣言型には書けない（注釈構文が無い）。
        | InferredType::IteratorOf(_)
        | InferredType::Undefined
        | InferredType::SelfType
        | InferredType::Protocol(_)
        | InferredType::Intersection(_)
        | InferredType::TypeVal
        | InferredType::TypeValOf(_)
        | InferredType::Namespace(..)
        | InferredType::PyNamespace(_)
        // ⚠ 具体化済みジェネリクスは Arrow のインスタンス。外部言語から届く値では
        //   ないので境界検査の対象にならない（`NamedInstance` と同じ扱い）。
        | InferredType::GenericInstance { .. }
        | InferredType::Function { .. }
        // ⚠ メタプログラミングの型は**展開時にしか存在しない**。FFI 境界の宣言型には
        //   書けない（書けても展開器が消す）ので、届いたら判定しようがない―保守的側に倒す。
        | InferredType::Code
        | InferredType::Meta(_) => Verdict::Unverifiable,

        InferredType::Int => prim(matches!(value, Value::Int(_) | Value::UInt(_)), value),
        InferredType::Float => prim(matches!(value, Value::Float(_)), value),
        InferredType::Str => prim(matches!(value, Value::Str(_)), value),
        InferredType::Bytes => prim(matches!(value, Value::Bytes(_)), value),
        // `Ptr[T]`（3-3）: 要素型まで照合する（`ptr_ann_matches` と同じ規則。`is` / `mustbe` と揃える）
        InferredType::Ptr(_) => prim(
            crate::interpreter::value::ptr_ann_matches(&declared.to_string(), value).unwrap_or(false),
            value,
        ),
        InferredType::Bool => prim(matches!(value, Value::Bool(_)), value),
        InferredType::None => prim(matches!(value, Value::None), value),
        InferredType::Complex => prim(matches!(value, Value::Complex(_, _)), value),

        // Union / Result: いずれかに適合すれば可。
        InferredType::Union(variants) => {
            if variants.iter().any(|t| matches!(check_common(value, t), Verdict::Ok)) {
                Verdict::Ok
            } else if variants.iter().any(|t| {
                matches!(check_common(value, t), Verdict::Unverifiable)
            }) {
                // 検査不能な選択肢を含むなら判定を放棄する（誤検知回避）。
                Verdict::Unverifiable
            } else {
                mismatch(value)
            }
        }
        InferredType::Result(ok, err) => {
            let as_union = InferredType::Union(vec![(**ok).clone(), (**err).clone()]);
            check_common(value, &as_union)
        }

        // コンテナ: 外側の形に加えて**要素型も検査する**。
        // `list[int]` と宣言されたのに `[1, "two"]` が返る、というのが実際に起きた失敗例。
        InferredType::List | InferredType::ListLike => {
            prim(matches!(value, Value::List(_) | Value::FrozenList { .. }), value)
        }
        InferredType::FixedList => prim(matches!(value, Value::FrozenList { .. }), value),
        InferredType::ListOf(elem) | InferredType::ListLikeOf(elem) => match value {
            Value::List(items) => check_elems(&items.borrow(), elem, value),
            Value::FrozenList { .. } => Verdict::Ok, // flat 表現は要素型が構築時に保証済み
            _ => mismatch(value),
        },
        InferredType::FixedListOf(_) => {
            prim(matches!(value, Value::FrozenList { .. }), value)
        }
        InferredType::Set => prim(matches!(value, Value::Set(_)), value),
        InferredType::SetOf(elem) => match value {
            Value::Set(items) => check_elems(&items.borrow(), elem, value),
            _ => mismatch(value),
        },
        InferredType::Dict => prim(matches!(value, Value::Dict(_)), value),
        // キーと値も照合する（5-2。⚠ 以前は外側の形だけ見ていた）
        InferredType::DictOf(kt, vt) => match value {
            Value::Dict(d) => {
                for (k, v) in d.borrow().all_pairs() {
                    for (x, t) in [(&k, &**kt), (&v, &**vt)] {
                        match check_common(x, t) {
                            Verdict::Ok | Verdict::Unverifiable => {}
                            _ => return mismatch(value),
                        }
                    }
                }
                Verdict::Ok
            }
            _ => mismatch(value),
        },
        InferredType::Tuple(types) => match value {
            Value::Tuple(td) => {
                let vals = td.all_values();
                if vals.len() != types.len() {
                    return mismatch(value);
                }
                for (v, t) in vals.iter().zip(types) {
                    match check_common(v, t) {
                        Verdict::Ok | Verdict::Unverifiable => {}
                        _ => return mismatch(value),
                    }
                }
                Verdict::Ok
            }
            _ => mismatch(value),
        },
        // ⚠ 素の `tuple` は**要素数も要素型も判らない**ので外側の形だけ見る
        //    （`Dict` / `Set` と同じ扱い・タスク 8.3）。
        InferredType::TupleAny => prim(matches!(value, Value::Tuple(_)), value),

        // ユーザー定義クラス: 外部から生の Instance が返ることは（現状の変換では）ない。
        // 誤検知を避けるため検査しない。
        InferredType::NamedInstance(_) => Verdict::Unverifiable,
    }
}

fn prim(ok: bool, value: &Value) -> Verdict {
    if ok {
        Verdict::Ok
    } else {
        mismatch(value)
    }
}

fn mismatch(value: &Value) -> Verdict {
    Verdict::Mismatch {
        actual: runtime_type_name(value).to_string(),
    }
}

/// エラーメッセージ用の実行時型名。`Interpreter::type_name_of` は `&self` を要するが、
/// ここは値だけで判定する純粋関数として保ちたいので最小版を持つ。
fn runtime_type_name(val: &Value) -> &'static str {
    match val {
        Value::Int(_) => "int",
        Value::UInt(_) => "uint",
        Value::Float(_) => "float",
        Value::Complex(_, _) => "complex",
        Value::Str(_) => "str",
        Value::Bool(_) => "bool",
        Value::None => "None",
        Value::Undefined => "Undefined",
        Value::List(_) => "list",
        Value::FrozenList { .. } => "fixed_list",
        Value::Dict(_) => "dict",
        Value::Set(_) => "set",
        Value::Tuple(_) => "tuple",
        Value::Instance(_) => "instance",
        Value::PyObject(_) => "python object",
        _ => "value",
    }
}

/// 要素型の検査。1 つでも矛盾したら不一致とする。
/// 走査コストは変換時（`py_to_tl` 等）に既に全要素を歩いているのと同オーダー。
fn check_elems(items: &[Value], elem: &InferredType, whole: &Value) -> Verdict {
    for v in items {
        match check_common(v, elem) {
            Verdict::Ok | Verdict::Unverifiable => {}
            _ => return mismatch(whole),
        }
    }
    Verdict::Ok
}

// ── Python ───────────────────────────────────────────────────────────────────

/// Python は int と float を区別して Arrow へ届く（`py_to_tl`）ため、共通判定をそのまま使える。
struct PythonChecker;

impl BoundaryChecker for PythonChecker {
    fn lang(&self) -> &'static str {
        "python"
    }

    fn check(&self, value: &Value, declared: &InferredType) -> Verdict {
        check_common(value, declared)
    }
}

// ── JavaScript ───────────────────────────────────────────────────────────────

/// JavaScript の数値はすべて f64 で、整数と小数の区別が無い。ブリッジ（`bridge/js_bridge.cjs`）は
/// 「int64 に入る整数値」を `Value::Int`、それ以外（小数・2^63 以上・NaN / ±Infinity）を `Value::Float` で届ける。
/// したがって宣言型で寄せる必要がある（ffi_value_exchange_plan.md 2-3）:
///   - `float` 宣言に `Int` が届いた（JS の `2.0` は整数値）→ `Float` へ寄せる。JS の値はもともと f64 なので正確に戻る
///   - `int` 宣言に `Float` が届いた → 整数値で int64 に入るなら `Int` へ寄せる。小数・範囲外・NaN は不一致
///   - コンテナ（`list[float]` / `list[list[int]]` / `Option[float]` …）は要素へ再帰して同じ規則を当てる
/// ⚠ 以前は「数値は常に `Float` で届く」前提で書かれていて、実際には整数値が `Int` で届くため、`-> float` の関数が
///   `2.0` を返すと誤って FfiTypeError になっていた。`list[float]` に整数値の要素が混ざっても寄せず、そもそも
///   コンテナの戻り値は型スタブの型が読まれず照合されていなかった（`collect_module_types`）。
struct JavaScriptChecker;

impl BoundaryChecker for JavaScriptChecker {
    fn lang(&self) -> &'static str {
        "javascript"
    }

    fn check(&self, value: &Value, declared: &InferredType) -> Verdict {
        match js_coerce(value, declared) {
            // ⚠ 寄せた値も照合し直す（5-2）。以前はそのまま通していたので、`list[float]` の `[1, "two"]` は `1` を寄せた
            //   時点で `"two"` が照合されずに通った
            Ok(Some(v)) => match check_common(&v, declared) {
                Verdict::Ok | Verdict::Unverifiable => Verdict::Coerce(v),
                _ => mismatch(value),
            },
            Ok(None) => check_common(value, declared),
            Err(()) => mismatch(value),
        }
    }
}

/// JS の数値の表現差を宣言型で埋めた値。`Ok(None)` は「寄せる必要が無い（共通判定に任せる）」、
/// `Err(())` は「数値として寄せられない不一致」。
fn js_coerce(value: &Value, declared: &InferredType) -> Result<Option<Value>, ()> {
    match (declared, value) {
        (InferredType::Float, Value::Int(n)) => Ok(Some(Value::Float(*n as f64))),
        (InferredType::Int, Value::Float(f)) => {
            // `i64::MAX as f64` は 2^63 に丸まるので、2^63 未満を `<` で確かめる
            if f.fract() == 0.0 && f.is_finite() && *f >= i64::MIN as f64 && *f < i64::MAX as f64 {
                Ok(Some(Value::Int(*f as i64)))
            } else {
                Err(())
            }
        }
        (InferredType::ListOf(elem) | InferredType::ListLikeOf(elem), Value::List(items)) => {
            let items = items.borrow();
            let mut changed = false;
            let mut out = Vec::with_capacity(items.len());
            for v in items.iter() {
                match js_coerce(v, elem)? {
                    Some(c) => {
                        changed = true;
                        out.push(c);
                    }
                    None => out.push(v.clone()),
                }
            }
            Ok(changed.then(|| Value::List(std::rc::Rc::new(std::cell::RefCell::new(out)))))
        }
        // dict の値・set の要素も寄せる（5-2）
        (InferredType::DictOf(kt, vt), Value::Dict(d)) => {
            let pairs = d.borrow().all_pairs();
            let mut changed = false;
            let mut out = Vec::with_capacity(pairs.len());
            for (k, v) in pairs {
                let (k2, v2) = (js_coerce(&k, kt)?, js_coerce(&v, vt)?);
                changed |= k2.is_some() || v2.is_some();
                out.push((k2.unwrap_or(k), v2.unwrap_or(v)));
            }
            if !changed {
                return Ok(None);
            }
            let mut dict = crate::interpreter::value::DictData::new("Any".to_string(), "Any".to_string());
            for (k, v) in out {
                crate::interpreter::Interpreter::dict_set_pure(&mut dict, k, v).map_err(|_| ())?;
            }
            Ok(Some(Value::Dict(std::rc::Rc::new(std::cell::RefCell::new(dict)))))
        }
        (InferredType::SetOf(elem), Value::Set(items)) => {
            let items = items.borrow();
            let mut changed = false;
            let mut out = Vec::with_capacity(items.len());
            for v in items.iter() {
                let c = js_coerce(v, elem)?;
                changed |= c.is_some();
                out.push(c.unwrap_or_else(|| v.clone()));
            }
            Ok(changed.then(|| Value::Set(std::rc::Rc::new(std::cell::RefCell::new(out)))))
        }
        // JS にタプルは無い（配列で返る）ので、宣言がタプルなら同じ長さの配列をタプルにする（5-2）
        (InferredType::Tuple(types), Value::List(items)) if items.borrow().len() == types.len() => {
            let items = items.borrow();
            let mut vals = Vec::with_capacity(items.len());
            let mut names = Vec::with_capacity(items.len());
            for (v, t) in items.iter().zip(types) {
                let v = js_coerce(v, t)?.unwrap_or_else(|| v.clone());
                names.push(crate::interpreter::ops::typecheck::element_type_name(&v));
                vals.push(v);
            }
            Ok(Some(Value::Tuple(std::rc::Rc::new(crate::interpreter::value::TupleData::new(vals, names)))))
        }
        // `Option[float]` など: 値に合う選択肢があればそのまま、寄せれば合う選択肢があれば寄せる
        (InferredType::Union(variants), _) => {
            if variants.iter().any(|t| matches!(check_common(value, t), Verdict::Ok)) {
                return Ok(None);
            }
            for t in variants {
                if let Ok(Some(v)) = js_coerce(value, t) {
                    return Ok(Some(v));
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn list(vals: Vec<Value>) -> Value {
        Value::List(Rc::new(RefCell::new(vals)))
    }

    fn is_ok(v: &Verdict) -> bool {
        matches!(v, Verdict::Ok)
    }
    fn is_mismatch(v: &Verdict) -> bool {
        matches!(v, Verdict::Mismatch { .. })
    }

    // ── Python: 共通判定をそのまま使う（int/float を区別して届く） ──

    #[test]
    fn python_accepts_matching_primitive() {
        let c = checker_for("py").unwrap();
        assert!(is_ok(&c.check(&Value::Int(1), &InferredType::Int)));
        assert!(is_ok(&c.check(&Value::Str("s".into()), &InferredType::Str)));
        assert!(is_ok(&c.check(&Value::None, &InferredType::None)));
    }

    #[test]
    fn python_rejects_lying_stub() {
        // `def f() -> int: return "s"` — Python は注釈を強制しないので実際に起きる。
        let c = checker_for("py").unwrap();
        assert!(is_mismatch(&c.check(&Value::Str("s".into()), &InferredType::Int)));
        assert!(is_mismatch(&c.check(&Value::None, &InferredType::Int)));
        // float は int ではない（Python は両者を区別して届けるため厳密に見る）。
        assert!(is_mismatch(&c.check(&Value::Float(3.0), &InferredType::Int)));
    }

    #[test]
    fn python_checks_list_elements() {
        let c = checker_for("py").unwrap();
        let declared = InferredType::ListOf(Box::new(InferredType::Int));
        assert!(is_ok(&c.check(&list(vec![Value::Int(1), Value::Int(2)]), &declared)));
        // 要素に別型が混ざる（`-> list[int]` が [1, "two"] を返す）。
        assert!(is_mismatch(&c.check(
            &list(vec![Value::Int(1), Value::Str("two".into())]),
            &declared
        )));
    }

    #[test]
    fn unverifiable_declarations_pass_through() {
        let c = checker_for("py").unwrap();
        // 宣言が無いに等しい型は検査しない（スタブ未整備の箇所の挙動を変えない）。
        assert!(matches!(
            c.check(&Value::Str("s".into()), &InferredType::Any),
            Verdict::Unverifiable
        ));
        assert!(matches!(
            c.check(&Value::Str("s".into()), &InferredType::Unresolved),
            Verdict::Unverifiable
        ));
    }

    #[test]
    fn union_accepts_any_variant() {
        let c = checker_for("py").unwrap();
        let opt_int = InferredType::Union(vec![InferredType::Int, InferredType::None]);
        assert!(is_ok(&c.check(&Value::Int(1), &opt_int)));
        assert!(is_ok(&c.check(&Value::None, &opt_int)));
        assert!(is_mismatch(&c.check(&Value::Str("s".into()), &opt_int)));
    }

    // ── JavaScript: 整数値は Int、それ以外は Float で届く（`bridge/js_bridge.cjs`）ので宣言型で寄せる ──

    #[test]
    fn javascript_coerces_integral_float_to_int() {
        // 2^63 以上の整数値は Float で届く。int64 に入るものだけ Int へ寄せる。
        let c = checker_for("js-proc").unwrap();
        match c.check(&Value::Float(3.0), &InferredType::Int) {
            Verdict::Coerce(Value::Int(3)) => {}
            other => panic!("expected Coerce(Int(3)), got {other:?}"),
        }
        assert!(is_mismatch(&c.check(&Value::Float(1e20), &InferredType::Int)));
        assert!(is_mismatch(&c.check(&Value::Float(f64::NAN), &InferredType::Int)));
    }

    #[test]
    fn javascript_rejects_fractional_float_for_int() {
        let c = checker_for("js-proc").unwrap();
        assert!(is_mismatch(&c.check(&Value::Float(3.5), &InferredType::Int)));
    }

    /// JS の `2.0` は整数値なので Int で届く。`float` 宣言では Float へ寄せる（2-3。以前は FfiTypeError）。
    #[test]
    fn javascript_coerces_int_to_float() {
        let c = checker_for("js-proc").unwrap();
        match c.check(&Value::Int(2), &InferredType::Float) {
            Verdict::Coerce(Value::Float(f)) if f == 2.0 => {}
            other => panic!("expected Coerce(Float(2.0)), got {other:?}"),
        }
        let opt = InferredType::Union(vec![InferredType::Float, InferredType::None]);
        assert!(matches!(c.check(&Value::Int(2), &opt), Verdict::Coerce(Value::Float(_))));
        assert!(is_ok(&c.check(&Value::None, &opt)));
    }

    #[test]
    fn javascript_coerces_list_elements_recursively() {
        let c = checker_for("js-proc").unwrap();
        let floats = InferredType::ListOf(Box::new(InferredType::Float));
        match c.check(&list(vec![Value::Int(1), Value::Float(2.5)]), &floats) {
            Verdict::Coerce(Value::List(items)) => {
                let got: Vec<Value> = items.borrow().clone();
                assert!(matches!(got.as_slice(), [Value::Float(a), Value::Float(b)] if *a == 1.0 && *b == 2.5));
            }
            other => panic!("expected coerced float list, got {other:?}"),
        }
        let ints = InferredType::ListOf(Box::new(InferredType::Int));
        assert!(is_ok(&c.check(&list(vec![Value::Int(1), Value::Int(2)]), &ints)));
        // 要素に別の型が混ざる（`-> list[int]` が [1, "two"] を返す）
        assert!(is_mismatch(&c.check(&list(vec![Value::Int(1), Value::Str("two".into())]), &ints)));
        assert!(is_mismatch(&c.check(&list(vec![Value::Float(1.5)]), &ints)));
        let nested = InferredType::ListOf(Box::new(InferredType::ListOf(Box::new(InferredType::Float))));
        assert!(matches!(c.check(&list(vec![list(vec![Value::Int(1)])]), &nested), Verdict::Coerce(_)));
    }

    /// 寄せた値も照合し直す・dict の値と set の要素も寄せる・配列を宣言のタプルにする（5-2）。
    #[test]
    fn javascript_coerces_containers_and_rechecks() {
        let floats = InferredType::ListOf(Box::new(InferredType::Float));
        assert!(is_mismatch(&JavaScriptChecker.check(&list(vec![Value::Int(1), Value::str("two")]), &floats)));
        let d = InferredType::DictOf(Box::new(InferredType::Str), Box::new(InferredType::Float));
        let mut dict = crate::interpreter::value::DictData::new("Any".to_string(), "Any".to_string());
        crate::interpreter::Interpreter::dict_set_pure(&mut dict, Value::str("a"), Value::Int(2)).unwrap();
        let dv = Value::Dict(Rc::new(RefCell::new(dict)));
        match JavaScriptChecker.check(&dv, &d) {
            Verdict::Coerce(Value::Dict(x)) => assert!(matches!(x.borrow().all_pairs()[0].1, Value::Float(f) if f == 2.0)),
            other => panic!("{other:?}"),
        }
        let wrong = InferredType::DictOf(Box::new(InferredType::Str), Box::new(InferredType::Str));
        assert!(is_mismatch(&JavaScriptChecker.check(&dv, &wrong)), "dict values are checked");
        let t = InferredType::Tuple(vec![InferredType::Int, InferredType::Float]);
        match JavaScriptChecker.check(&list(vec![Value::Int(1), Value::Int(2)]), &t) {
            Verdict::Coerce(Value::Tuple(td)) => assert!(matches!(td.all_values()[1], Value::Float(f) if f == 2.0)),
            other => panic!("{other:?}"),
        }
        assert!(is_mismatch(&JavaScriptChecker.check(&list(vec![Value::Int(1)]), &t)), "length");
    }

    #[test]
    fn javascript_falls_back_to_common_for_non_numeric() {
        let c = checker_for("js-proc").unwrap();
        assert!(is_ok(&c.check(&Value::Str("s".into()), &InferredType::Str)));
        assert!(is_mismatch(&c.check(&Value::Str("s".into()), &InferredType::Bool)));
    }

    // ── 言語の登録 ──

    #[test]
    fn only_dynamic_languages_are_checked() {
        assert!(checker_for("py").is_some());
        assert!(checker_for("py-int").is_some());
        assert!(checker_for("js-proc").is_some());
        // 静的型付け側は向こうが型を守るので対象外。
        // C/C++ の `void*` は Arrow の int に落ちるため型タグでは何も言えない（別軸の課題）。
        assert!(checker_for("cpp-lib").is_none());
        assert!(checker_for("cs-dll").is_none());
        assert!(checker_for("rs").is_none());
    }
}
