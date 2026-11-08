// classes/builtin_member_methods.rs — 組み込みの値の特殊メソッド（`xs.__len__()` / `(5).__index__()` …）と `count` / `index`
// （pandas_import_plan.md の 6-6）。
//
// 組み込みの値が持つメンバーは型検査と共有する唯一の表（`type_check::builtin_members`）が決める。表にあるメンバーは
// protocol の照合で「持つ」とされるので（`list[int]` は `__len__` を持つ）、protocol の型の値として呼んでも（`h.__len__()`）、
// 直接呼んでも、実行時に動かなければならない。中身は組み込み関数・演算子と同じ本体（`len` / `obj[k]` / `in` / `hash` …）。
// ⚠ 以前は特殊メソッドのほとんどと、`list` / `tuple` の `count` / `index` が実行時に無かった（`[1, 2].count(1)` は型検査を
//   通るのに `AttributeError`）。

use {
    crate::interpreter::{Interpreter, Value},
    crate::type_check::builtin_members::{BuiltinKind, Member},
};

impl Interpreter {
    /// 表のメンバー `method_name` を組み込みの値 `obj` で呼ぶ。表に無い名前（`append` など、種類ごとの入口が持つもの）は
    /// `None`。`eval_method_call_full` が種類ごとの入口より先に呼ぶ。
    ///
    /// ⚠ 引くのは特殊メソッドと `count` / `index` だけ（ほかの名前は種類ごとの入口が持つ・メソッド呼び出しのたびに表を
    ///   引かないため）。`str` / `bytes` の `count` / `index` と `dict` の `keys` などは種類ごとの入口のまま（部分列を数える）。
    pub(super) fn call_builtin_member(
        &mut self,
        obj: &Value,
        method_name: &str,
        evaled: &[(Option<String>, Value, bool)],
    ) -> Option<Result<Value, String>> {
        if !(method_name.starts_with("__") || matches!(method_name, "count" | "index")) {
            return None;
        }
        let kind = Self::builtin_kind_of_value(obj)?;
        let member = crate::type_check::builtin_members::members_of(kind)
            .iter()
            .copied()
            .find(|m| m.name() == method_name)?;
        if matches!(member, Member::Count | Member::Index) && matches!(kind, BuiltinKind::Str | BuiltinKind::Bytes) {
            return None;
        }
        Some(self.call_builtin_member_of(obj.clone(), kind, member, evaled))
    }

    /// 表のメンバーの本体（`Member` を網羅する・`_` を書かない）。
    fn call_builtin_member_of(
        &mut self,
        obj: Value,
        kind: BuiltinKind,
        member: Member,
        evaled: &[(Option<String>, Value, bool)],
    ) -> Result<Value, String> {
        let type_name = self.type_name(&obj);
        let name = member.name();
        if let Some((Some(k), _, _)) = evaled.iter().find(|(k, _, _)| k.is_some()) {
            return Err(format!("TypeError: {type_name}.{name}() takes no keyword arguments ('{k}' given)"));
        }
        let args: Vec<Value> = evaled.iter().map(|(_, v, _)| v.clone()).collect();
        let arity = |min: usize, max: usize| -> Result<(), String> {
            if args.len() < min || args.len() > max {
                let expected = if min == max { format!("{min}") } else { format!("{min} to {max}") };
                return Err(format!(
                    "TypeError: {type_name}.{name}() takes {expected} argument(s) ({} given)",
                    args.len()
                ));
            }
            Ok(())
        };
        match member {
            Member::Iter => {
                arity(0, 0)?;
                self.py_iter(vec![obj])
            }
            Member::Next => {
                arity(0, 0)?;
                self.py_next(obj, None)
            }
            Member::Len => {
                arity(0, 0)?;
                self.len_of(obj)
            }
            Member::GetItem => {
                arity(1, 1)?;
                let key = args.into_iter().next().expect("one argument");
                self.eval_subscript(obj, key)
            }
            Member::Contains => {
                arity(1, 1)?;
                let item = args.into_iter().next().expect("one argument");
                self.apply_binop_dyn(&crate::ast::BinOp::In, item, obj)
            }
            Member::Reversed => {
                arity(0, 0)?;
                self.py_reversed(vec![obj])
            }
            Member::Count => {
                arity(1, 1)?;
                let items = self.membership_items(&obj).unwrap_or_default();
                let mut n = 0;
                for v in &items {
                    if self.values_eq_dyn(&args[0], v)? {
                        n += 1;
                    }
                }
                Ok(Value::Int(n))
            }
            // `index(x[, start[, stop]])`（CPython の `list.index` / `tuple.index`）。範囲は負の数も数える。
            Member::Index => {
                arity(1, 3)?;
                let items = self.membership_items(&obj).unwrap_or_default();
                let len = items.len() as i64;
                let bound = |v: Option<&Value>, default: i64| -> Result<usize, String> {
                    let n = match v {
                        None => default,
                        Some(Value::Int(n)) => *n,
                        Some(other) => {
                            return Err(format!(
                                "TypeError: slice indices must be integers, not '{}'",
                                self.type_name(other)
                            ))
                        }
                    };
                    let n = if n < 0 { (n + len).max(0) } else { n.min(len) };
                    Ok(n as usize)
                };
                let (start, stop) = (bound(args.get(1), 0)?, bound(args.get(2), len)?);
                for (i, v) in items.iter().enumerate().take(stop).skip(start) {
                    if self.values_eq_dyn(&args[0], v)? {
                        return Ok(Value::Int(i as i64));
                    }
                }
                Err(match kind {
                    BuiltinKind::Tuple => "ValueError: tuple.index(x): x not in tuple".to_string(),
                    _ => format!("ValueError: {} is not in list", self.repr_val(&args[0])?),
                })
            }
            Member::Hash => {
                arity(0, 0)?;
                self.py_hash(vec![obj])
            }
            Member::IndexDunder | Member::Int => {
                arity(0, 0)?;
                self.call_type_by_name_evaled("int", vec![obj])
            }
            Member::Float => {
                arity(0, 0)?;
                self.call_type_by_name_evaled("float", vec![obj])
            }
            // `complex` / `bytes` の値そのもの（CPython の `complex.__complex__` / `bytes.__bytes__`）
            Member::Complex | Member::Bytes => {
                arity(0, 0)?;
                Ok(obj)
            }
            Member::Abs => {
                arity(0, 0)?;
                self.py_abs(vec![obj])
            }
            Member::Round => {
                arity(0, 1)?;
                let mut all = vec![obj];
                all.extend(args);
                self.py_round(all, Vec::new())
            }
            // `dict` の `keys` などは特殊メソッドではないので `call_builtin_member` が引かない（種類ごとの入口が持つ）
            Member::Keys | Member::Values | Member::Items | Member::Get => {
                unreachable!("'{name}' is not looked up through call_builtin_member")
            }
        }
    }
}
