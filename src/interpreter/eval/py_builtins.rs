// eval/py_builtins.rs — Python のコードを動かすために足した組み込み（python_builtins_plan.md のフェーズ 2〜）。
//
// 振り分けは `eval/builtins.rs` の `eval_builtin_evaled` の腕。ここには本体だけを置く。
// 名前は 3 つの表に載せること（`BUILTIN_VALUE_NAMES`・`VM_BUILTIN_NAMES`・型検査の `RUNTIME_BUILTIN_NAMES`）。

use crate::interpreter::{ClassValue, Interpreter, Value};

impl Interpreter {
    /// `isinstance(x, spec)`（タスク 2-1）。CPython の `object_recursive_isinstance` と同じ考え方:
    /// - 組は要素ごと（どれかに当たれば真）
    /// - クラスは同じクラスかその派生（[`Self::class_is_subclass`]）
    /// - 組み込みの型は値の種類。`bool` は `int` の派生（`isinstance(True, int)` は真）
    /// - trait / protocol は `is` と同じ判定
    /// ⚠ 型でない値（組み込み関数・数値など）は CPython と同じ `TypeError`。
    pub(crate) fn py_isinstance(&self, x: &Value, spec: &Value) -> Result<bool, String> {
        match spec {
            Value::Tuple(t) => {
                for s in t.all_values() {
                    if self.py_isinstance(x, s)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Value::Class(c) => Ok(match x {
                Value::Instance(inst) => Self::class_is_subclass(&inst.borrow().class, c),
                _ => false,
            }),
            Value::Type(name) if !super::is_builtin_function_name(name) => Ok(match name.as_str() {
                "int" => matches!(x, Value::Int(_) | Value::Bool(_)),
                "NoneType" => matches!(x, Value::None),
                "type" => matches!(x, Value::Class(_) | Value::Type(_)),
                _ => self.value_is_type(x, name),
            }),
            Value::Trait(name) | Value::Protocol(name) => Ok(self.value_is_type(x, name)),
            _ => Err("TypeError: isinstance() arg 2 must be a type, a tuple of types, or a union".to_string()),
        }
    }

    /// `type(x)`（1 引数・タスク 2-2）。インスタンスはそのクラスの値、ほかは組み込みの型の値（`<class 'int'>`）。
    ///
    /// 返した値はそのまま使える: 呼ぶと作り直せる（`type(x)(...)`）、`__name__` で名前、`===` / `==` で比べられる
    /// （`type(x) is C`・クラスは同一性、組み込みの型は名前で比べる）、`isinstance` の第 2 引数にできる。
    /// ⚠ 3 引数の `type(name, bases, dict)`（実行時にクラスを作る）は対象外（`call_type_by_name_evaled` が誤りにする）。
    pub(crate) fn py_type_of(&self, x: &Value) -> Value {
        let name = match x {
            Value::Instance(inst) => return Value::Class(inst.borrow().class.clone()),
            Value::None => "NoneType",
            Value::Function(_)
            | Value::OverloadedFn(_)
            | Value::NativeFunction(_)
            | Value::GeneratorFn(_)
            | Value::TemplateFn(_)
            | Value::TemplateGenFn(_)
            | Value::JsProcFn(_) => "function",
            Value::Class(_) | Value::Type(_) | Value::Trait(_) | Value::Protocol(_) | Value::TemplateClass(_) => "type",
            Value::Namespace(_) => "module",
            Value::Generator(_) => "generator",
            other => self.type_name(other),
        };
        Value::Type(name.to_string())
    }

    /// `getattr(o, name[, default])`（タスク 2-3）。読みは `o.name` と**同じ経路**（`get_attr_val`）なので、
    /// アクセス指定（`private:`）もそのまま効く。既定値があれば `AttributeError` のときだけそれを返す（CPython と同じ）。
    pub(crate) fn py_getattr(&mut self, args: Vec<Value>) -> Result<Value, String> {
        let n = args.len();
        if !(2..=3).contains(&n) {
            return Err(if n < 2 {
                format!("TypeError: getattr expected at least 2 arguments, got {n}")
            } else {
                format!("TypeError: getattr expected at most 3 arguments, got {n}")
            });
        }
        let mut it = args.into_iter();
        let (obj, name, default) = (it.next().expect("n >= 2"), it.next().expect("n >= 2"), it.next());
        let name = self.attr_name_arg(&name)?;
        match (self.get_attr_val(obj, &name, None), default) {
            (Err(e), Some(d)) if e.starts_with("AttributeError") => Ok(d),
            (r, _) => r,
        }
    }

    /// `hasattr(o, name)`（タスク 2-3）。読めれば真、`AttributeError` なら偽、ほかの誤りはそのまま上げる
    /// （CPython も `AttributeError` だけを飲み込む）。
    pub(crate) fn py_hasattr(&mut self, args: Vec<Value>) -> Result<Value, String> {
        let [obj, name]: [Value; 2] = args
            .try_into()
            .map_err(|a: Vec<Value>| format!("TypeError: hasattr expected 2 arguments, got {}", a.len()))?;
        let name = self.attr_name_arg(&name)?;
        match self.get_attr_val(obj, &name, None) {
            Ok(_) => Ok(Value::Bool(true)),
            Err(e) if e.starts_with("AttributeError") => Ok(Value::Bool(false)),
            Err(e) => Err(e),
        }
    }

    /// `setattr(o, name, v)`（タスク 2-3）。書きは `o.name = v` と同じ経路（`set_attr_val`）なので、
    /// アクセス指定・クラス変数（`const`）への代入の禁止・値の複製の規則がそのまま効く。
    /// ⚠ Arrow のクラスはフィールドを本体で宣言するので、宣言していない名前は作れない（`o.name = v` と同じ誤り）。
    pub(crate) fn py_setattr(&mut self, args: Vec<Value>) -> Result<Value, String> {
        let [obj, name, val]: [Value; 3] = args
            .try_into()
            .map_err(|a: Vec<Value>| format!("TypeError: setattr expected 3 arguments, got {}", a.len()))?;
        let name = self.attr_name_arg(&name)?;
        self.set_attr_val(obj, &name, val)?;
        Ok(Value::None)
    }

    /// 属性の名前の引数（`getattr` / `hasattr` / `setattr`）。文字列でなければ CPython と同じ `TypeError`。
    fn attr_name_arg(&self, v: &Value) -> Result<String, String> {
        match v {
            Value::Str(s) => Ok(s.to_string()),
            other => Err(format!("TypeError: attribute name must be string, not '{}'", self.type_name(other))),
        }
    }

    /// `issubclass(c, spec)`（タスク 2-4）。クラス同士は [`Self::class_is_subclass`]（`isinstance` と同じ判定）、
    /// 組み込みの型同士は名前（`bool` は `int` の派生）、trait は実装しているか。組は要素ごと。
    /// ⚠ 第 1 引数がクラス（型）でなければ CPython と同じ `TypeError`。
    pub(crate) fn py_issubclass(&self, c: &Value, spec: &Value) -> Result<bool, String> {
        if !matches!(c, Value::Class(_) | Value::Type(_)) {
            return Err("TypeError: issubclass() arg 1 must be a class".to_string());
        }
        match spec {
            Value::Tuple(t) => {
                for s in t.all_values() {
                    if self.py_issubclass(c, s)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Value::Class(base) => Ok(matches!(c, Value::Class(cc) if Self::class_is_subclass(cc, base))),
            Value::Type(name) if !super::is_builtin_function_name(name) => {
                Ok(matches!(c, Value::Type(cn) if cn == name || (cn == "bool" && name == "int")))
            }
            Value::Trait(name) | Value::Protocol(name) => Ok(matches!(c, Value::Class(cc) if cc.is_a(name))),
            _ => Err("TypeError: issubclass() arg 2 must be a class, a tuple of classes, or a union".to_string()),
        }
    }

    /// `callable(x)`（タスク 2-4）。関数・クラス・組み込みの型と関数の値・`__call__` を持つインスタンスは呼べる。
    /// ⚠ `x is function` と違い、`__call__` を持たないクラスも呼べる（作る）ので真（CPython と同じ）。
    pub(crate) fn py_callable(&self, x: &Value) -> bool {
        match x {
            Value::Function(_)
            | Value::OverloadedFn(_)
            | Value::NativeFunction(_)
            | Value::GeneratorFn(_)
            | Value::TemplateFn(_)
            | Value::TemplateGenFn(_)
            | Value::JsProcFn(_)
            | Value::Class(_)
            | Value::TemplateClass(_)
            | Value::Type(_) => true,
            Value::Instance(inst) => inst.borrow().class.methods.contains_key("__call__"),
            Value::PyObject(h) => crate::interpreter::py_interop::py_is_callable(h),
            _ => false,
        }
    }

    /// `dict(...)`（タスク 3-1）。CPython の `dict` の作り方と同じ:
    /// - `dict()` は空、`dict(mapping)` は辞書の写し、`dict(pairs)` は 2 要素の組（list / tuple）の列から
    /// - キーワード引数（`dict(a=1)`）は文字列のキーで足す（位置引数の後に上書き）
    /// ⚠ キーの格納は `dict_set`（`__hash__` / `__eq__` を尊重する）。誤りの文言は CPython と同じ。
    pub(crate) fn py_dict_ctor(&mut self, evaled: Vec<(Option<String>, Value, bool)>) -> Result<Value, String> {
        let (named, positional): (Vec<_>, Vec<_>) = evaled.into_iter().partition(|(k, _, _)| k.is_some());
        if positional.len() > 1 {
            return Err(format!("TypeError: dict expected at most 1 argument, got {}", positional.len()));
        }
        let d = std::rc::Rc::new(std::cell::RefCell::new(crate::interpreter::DictData::new(
            "Any".to_string(),
            "Any".to_string(),
        )));
        if let Some((_, src, _)) = positional.into_iter().next() {
            let pairs: Vec<(Value, Value)> = match src {
                Value::Dict(m) => m.borrow().all_pairs(),
                other => {
                    let mut out = Vec::new();
                    for (i, item) in self.drain_iterable(other)?.into_iter().enumerate() {
                        let elems = match &item {
                            Value::Tuple(t) => t.all_values().to_vec(),
                            Value::List(l) => l.borrow().clone(),
                            _ => {
                                return Err(format!(
                                    "TypeError: cannot convert dictionary update sequence element #{i} to a sequence"
                                ))
                            }
                        };
                        let [k, v]: [Value; 2] = elems.try_into().map_err(|e: Vec<Value>| {
                            format!(
                                "ValueError: dictionary update sequence element #{i} has length {}; 2 is required",
                                e.len()
                            )
                        })?;
                        out.push((k, v));
                    }
                    out
                }
            };
            for (k, v) in pairs {
                self.dict_set(&d, k, v)?;
            }
        }
        for (k, v, _) in named {
            self.dict_set(&d, Value::str(k.expect("partitioned by is_some").as_str()), v)?;
        }
        Ok(Value::Dict(d))
    }

    // ── 集計（タスク 3-2）──────────────────────────────────────────────────────

    /// 回す値をジェネレータの状態にする（`for` と同じ規則）。回せなければ CPython と同じ文言の `TypeError`。
    fn py_iter_state(&mut self, v: Value) -> Result<std::rc::Rc<std::cell::RefCell<crate::interpreter::GeneratorState>>, String> {
        let type_name = self.type_name(&v).to_string();
        match self.make_for_iterator(v) {
            Ok(Value::Generator(state)) => Ok(state),
            _ => Err(format!("TypeError: '{type_name}' object is not iterable")),
        }
    }

    /// `all(it)` / `any(it)`。**途中で決まったら止める**（無限のジェネレータでも決まれば返る・CPython と同じ）。
    pub(crate) fn py_all_any(&mut self, args: Vec<Value>, want_all: bool) -> Result<Value, String> {
        let fname = if want_all { "all" } else { "any" };
        let [it]: [Value; 1] = args.try_into().map_err(|a: Vec<Value>| {
            format!("TypeError: {fname}() takes exactly one argument ({} given)", a.len())
        })?;
        let state = self.py_iter_state(it)?;
        while let Some(v) = self.gen_next(&state)? {
            let t = self.eval_truthy(&v)?;
            if t != want_all {
                return Ok(Value::Bool(t));
            }
        }
        Ok(Value::Bool(want_all))
    }

    /// `key=` の関数を当てた値（無ければその値）。
    fn py_key_of(&mut self, key: &Option<Value>, item: &Value) -> Result<Value, String> {
        match key {
            Some(f) => self.call_value_evaled(f.clone(), vec![(None, item.clone(), false)], "key", None, 0),
            None => Ok(item.clone()),
        }
    }

    /// `a < b`（`__lt__` も使う）の真偽。並べ替えと `min` / `max` の比較の唯一の入口。
    fn py_less(&mut self, op: crate::ast::BinOp, a: &Value, b: &Value) -> Result<bool, String> {
        let r = self.apply_binop_dyn(&op, a.clone(), b.clone())?;
        self.eval_truthy(&r)
    }

    /// `min(...)` / `max(...)`（`key=` / `default=`）。CPython と同じく、引数が 1 つなら回す値、2 つ以上なら引数そのもの。
    /// 同じ大きさなら**最初のもの**（`max` は `item > best`、`min` は `item < best` のときだけ入れ替える）。
    pub(crate) fn py_min_max(&mut self, args: Vec<Value>, mut kw: Vec<(String, Value)>, is_max: bool) -> Result<Value, String> {
        let fname = if is_max { "max" } else { "min" };
        let key = take_kw(&mut kw, "key").filter(|k| !matches!(k, Value::None));
        let default = take_kw(&mut kw, "default");
        reject_extra_kw(fname, &kw)?;
        let items = match args.len() {
            0 => return Err(format!("TypeError: {fname} expected at least 1 argument, got 0")),
            1 => self.drain_iterable(args.into_iter().next().expect("len 1"))?,
            _ if default.is_some() => {
                return Err(format!(
                    "TypeError: Cannot specify a default for {fname}() with multiple positional arguments"
                ))
            }
            _ => args,
        };
        let op = if is_max { crate::ast::BinOp::Gt } else { crate::ast::BinOp::Lt };
        let mut best: Option<(Value, Value)> = None;
        for item in items {
            let k = self.py_key_of(&key, &item)?;
            let replace = match &best {
                None => true,
                Some((_, bk)) => self.py_less(op.clone(), &k, &bk.clone())?,
            };
            if replace {
                best = Some((item, k));
            }
        }
        match (best, default) {
            (Some((item, _)), _) => Ok(item),
            (None, Some(d)) => Ok(d),
            (None, None) => Err(format!("ValueError: {fname}() iterable argument is empty")),
        }
    }

    /// `sum(it, start=0)`。CPython 3.12 の `sum` と同じ 3 段:
    /// 1. 結果が int の間は int を足す（`bool` は `int`）
    /// 2. 結果が float になったら、float は **Neumaier の補償つき**で足す（int はそのまま足す）
    /// 3. それ以外は `+` で足す
    /// ⚠ 文字列は CPython と同じく `TypeError`（`''.join` を促す）。
    pub(crate) fn py_sum(&mut self, args: Vec<Value>, mut kw: Vec<(String, Value)>) -> Result<Value, String> {
        let n = args.len();
        let mut it = args.into_iter();
        let iterable = it
            .next()
            .ok_or_else(|| format!("TypeError: sum() takes at least 1 positional argument ({n} given)"))?;
        let start = match (it.next(), take_kw(&mut kw, "start")) {
            (Some(_), Some(_)) => return Err("TypeError: sum() got multiple values for argument 'start'".to_string()),
            (Some(s), None) | (None, Some(s)) => s,
            (None, None) => Value::Int(0),
        };
        if it.next().is_some() {
            return Err(format!("TypeError: sum() takes at most 2 arguments ({n} given)"));
        }
        reject_extra_kw("sum", &kw)?;
        if matches!(start, Value::Str(_)) {
            return Err("TypeError: sum() can't sum strings [use ''.join(seq) instead]".to_string());
        }
        let state = self.py_iter_state(iterable)?;
        let add = |me: &mut Self, a: Value, b: Value| -> Result<Value, String> {
            let b = if let Value::Bool(x) = b { Value::Int(x as i64) } else { b };
            me.apply_binop_dyn(&crate::ast::BinOp::Add, a, b)
        };
        let mut result = if let Value::Bool(x) = start { Value::Int(x as i64) } else { start };
        // 1. int の段
        if matches!(result, Value::Int(_)) {
            loop {
                match self.gen_next(&state)? {
                    None => return Ok(result),
                    Some(v @ (Value::Int(_) | Value::Bool(_))) => result = add(self, result, v)?,
                    Some(v) => {
                        result = add(self, result, v)?;
                        break;
                    }
                }
            }
        }
        // 2. float の段（Neumaier の補償つき・CPython 3.12）
        if let Value::Float(f0) = result {
            let (mut f, mut c) = (f0, 0.0f64);
            loop {
                match self.gen_next(&state)? {
                    None => {
                        if c != 0.0 && c.is_finite() {
                            f += c;
                        }
                        return Ok(Value::Float(f));
                    }
                    Some(Value::Float(x)) => {
                        let t = f + x;
                        if f.abs() >= x.abs() {
                            c += (f - t) + x;
                        } else {
                            c += (x - t) + f;
                        }
                        f = t;
                    }
                    Some(Value::Int(v)) => f += v as f64,
                    Some(Value::Bool(b)) => f += b as i64 as f64,
                    Some(v) => {
                        if c != 0.0 && c.is_finite() {
                            f += c;
                        }
                        result = add(self, Value::Float(f), v)?;
                        break;
                    }
                }
            }
        }
        // 3. ほかは `+`
        while let Some(v) = self.gen_next(&state)? {
            result = add(self, result, v)?;
        }
        Ok(result)
    }

    /// `reversed(seq)`。list / tuple / str / dict（キー）・`__reversed__` を持つインスタンス。
    /// ⚠ CPython と同じく、ジェネレータのような「長さの無い回すだけの値」は逆にできない（`TypeError`）。
    pub(crate) fn py_reversed(&mut self, args: Vec<Value>) -> Result<Value, String> {
        let [seq]: [Value; 1] = args
            .try_into()
            .map_err(|a: Vec<Value>| format!("TypeError: reversed expected 1 argument, got {}", a.len()))?;
        let items: Vec<Value> = match &seq {
            Value::List(l) => l.borrow().iter().rev().cloned().collect(),
            Value::Tuple(t) => t.all_values().iter().rev().cloned().collect(),
            Value::Str(s) => s.chars().rev().map(|c| Value::str(c.to_string())).collect(),
            Value::Dict(d) => d.borrow().all_keys().into_iter().rev().collect(),
            Value::FrozenList { .. } => self.drain_iterable(seq.clone())?.into_iter().rev().collect(),
            Value::Instance(inst) if inst.borrow().class.methods.contains_key("__reversed__") => {
                return self.eval_method_call_evaled(seq.clone(), "__reversed__", vec![]);
            }
            other => return Err(format!("TypeError: '{}' object is not reversible", self.type_name(other))),
        };
        Ok(Value::Generator(std::rc::Rc::new(std::cell::RefCell::new(
            crate::interpreter::GeneratorState::materialized(items),
        ))))
    }

    /// `abs(x)`。int / float / bool・complex は大きさ（`hypot`）・`__abs__` を持つインスタンス。
    pub(crate) fn py_abs(&mut self, args: Vec<Value>) -> Result<Value, String> {
        let [x]: [Value; 1] = args
            .try_into()
            .map_err(|a: Vec<Value>| format!("TypeError: abs() takes exactly one argument ({} given)", a.len()))?;
        match &x {
            Value::Int(n) => Ok(Value::Int(n.wrapping_abs())),
            Value::UInt(n) => Ok(Value::UInt(*n)),
            Value::Bool(b) => Ok(Value::Int(*b as i64)),
            Value::Float(f) => Ok(Value::Float(f.abs())),
            Value::Complex(re, im) => Ok(Value::Float(re.hypot(*im))),
            Value::Instance(inst) if inst.borrow().class.methods.contains_key("__abs__") => {
                self.eval_method_call_evaled(x.clone(), "__abs__", vec![])
            }
            other => Err(format!("TypeError: bad operand type for abs(): '{}'", self.type_name(other))),
        }
    }

    /// `round(x, ndigits=None)`。CPython と同じく**偶数への丸め**:
    /// - 桁が無ければ int（float は `round_ties_even`・NaN / 無限大は CPython と同じ誤り）
    /// - 桁があれば同じ型。float は**正確な 10 進の値**で丸める（CPython の `_Py_dg_dtoa` の mode 3 と同じ）ので、
    ///   `round(2.675, 2)` は `2.67`（2.675 の実際の値は 2.67499999…）
    /// - `__round__` を持つインスタンスはそれを呼ぶ
    pub(crate) fn py_round(&mut self, args: Vec<Value>, mut kw: Vec<(String, Value)>) -> Result<Value, String> {
        let n = args.len();
        let mut it = args.into_iter();
        let x = it
            .next()
            .or_else(|| take_kw(&mut kw, "number"))
            .ok_or_else(|| "TypeError: round() missing required argument 'number' (pos 1)".to_string())?;
        let nd = match (it.next(), take_kw(&mut kw, "ndigits")) {
            (Some(_), Some(_)) => return Err("TypeError: round() got multiple values for argument 'ndigits'".to_string()),
            (a, b) => a.or(b),
        };
        if it.next().is_some() {
            return Err(format!("TypeError: round() takes at most 2 arguments ({n} given)"));
        }
        reject_extra_kw("round", &kw)?;
        if let Value::Instance(inst) = &x {
            if inst.borrow().class.methods.contains_key("__round__") {
                let mut a = Vec::new();
                if let Some(d) = nd {
                    a.push((None, d, false));
                }
                return self.eval_method_call_evaled(x.clone(), "__round__", a);
            }
        }
        let nd: Option<i64> = match nd {
            None | Some(Value::None) => None,
            Some(Value::Int(i)) => Some(i),
            Some(Value::Bool(b)) => Some(b as i64),
            Some(other) => {
                return Err(format!(
                    "TypeError: '{}' object cannot be interpreted as an integer",
                    self.type_name(&other)
                ))
            }
        };
        match x {
            Value::Int(v) => Ok(Value::Int(match nd {
                Some(d) if d < 0 => round_int_half_even(v, -d),
                _ => v,
            })),
            Value::Bool(b) => Ok(Value::Int(b as i64)),
            Value::Float(f) => match nd {
                None => {
                    if f.is_nan() {
                        Err("ValueError: cannot convert float NaN to integer".to_string())
                    } else if f.is_infinite() {
                        Err("OverflowError: cannot convert float infinity to integer".to_string())
                    } else {
                        Ok(Value::Int(f.round_ties_even() as i64))
                    }
                }
                Some(d) => Ok(Value::Float(round_float_digits(f, d))),
            },
            other => Err(format!("TypeError: type {} doesn't define __round__ method", self.type_name(&other))),
        }
    }

    /// `divmod(a, b)`。int は `(a // b, a % b)`（`ops::py_floor_div` / `py_mod`）、float は CPython の `_float_div_mod`。
    pub(crate) fn py_divmod(&mut self, args: Vec<Value>) -> Result<Value, String> {
        let [a, b]: [Value; 2] = args
            .try_into()
            .map_err(|a: Vec<Value>| format!("TypeError: divmod expected 2 arguments, got {}", a.len()))?;
        let as_int = |v: &Value| match v {
            Value::Int(i) => Some(*i),
            Value::Bool(x) => Some(*x as i64),
            _ => None,
        };
        let as_float = |v: &Value| match v {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            Value::Bool(x) => Some(*x as i64 as f64),
            _ => None,
        };
        if let (Some(x), Some(y)) = (as_int(&a), as_int(&b)) {
            if y == 0 {
                return Err("ZeroDivisionError: integer division or modulo by zero".to_string());
            }
            let (q, r) = (crate::interpreter::ops::py_floor_div(x, y), crate::interpreter::ops::py_mod(x, y));
            return Ok(self.vm_build_tuple(vec![Value::Int(q), Value::Int(r)]));
        }
        if let (Some(x), Some(y)) = (as_float(&a), as_float(&b)) {
            if y == 0.0 {
                return Err("ZeroDivisionError: float divmod()".to_string());
            }
            let (q, r) = crate::interpreter::ops::py_float_div_mod(x, y);
            return Ok(self.vm_build_tuple(vec![Value::Float(q), Value::Float(r)]));
        }
        if let Value::Instance(inst) = &a {
            if inst.borrow().class.methods.contains_key("__divmod__") {
                return self.eval_method_call_evaled(a.clone(), "__divmod__", vec![(None, b, false)]);
            }
        }
        Err(format!(
            "TypeError: unsupported operand type(s) for divmod(): '{}' and '{}'",
            self.type_name(&a),
            self.type_name(&b)
        ))
    }

    /// `pow(base, exp, mod=None)`。2 引数は `**` と同じ。3 引数は整数の冪剰余（負の指数は逆元・CPython 3.8+）、
    /// 結果の符号は法と同じ（CPython の `%` と同じ）。
    pub(crate) fn py_pow(&mut self, args: Vec<Value>, mut kw: Vec<(String, Value)>) -> Result<Value, String> {
        let n = args.len();
        let mut it = args.into_iter();
        let base = it.next().or_else(|| take_kw(&mut kw, "base"));
        let exp = it.next().or_else(|| take_kw(&mut kw, "exp"));
        let modulus = it.next().or_else(|| take_kw(&mut kw, "mod"));
        if it.next().is_some() {
            return Err(format!("TypeError: pow() takes at most 3 arguments ({n} given)"));
        }
        reject_extra_kw("pow", &kw)?;
        let (Some(base), Some(exp)) = (base, exp) else {
            return Err(format!("TypeError: pow() missing required argument (pos {})", n + 1));
        };
        let modulus = match modulus {
            None | Some(Value::None) => return self.apply_binop_dyn(&crate::ast::BinOp::Pow, base, exp),
            Some(m) => m,
        };
        let as_int = |v: &Value| match v {
            Value::Int(i) => Some(*i),
            Value::Bool(x) => Some(*x as i64),
            _ => None,
        };
        let (Some(b), Some(e), Some(m)) = (as_int(&base), as_int(&exp), as_int(&modulus)) else {
            return Err("TypeError: pow() 3rd argument not allowed unless all arguments are integers".to_string());
        };
        if m == 0 {
            return Err("ValueError: pow() 3rd argument cannot be 0".to_string());
        }
        let am = (m as i128).abs();
        let mut b = (b as i128).rem_euclid(am);
        let mut e = e as i128;
        if e < 0 {
            b = mod_inverse(b, am).ok_or_else(|| "ValueError: base is not invertible for the given modulus".to_string())?;
            e = -e;
        }
        let mut r: i128 = 1 % am;
        while e > 0 {
            if e & 1 == 1 {
                r = r * b % am;
            }
            b = b * b % am;
            e >>= 1;
        }
        if m < 0 && r != 0 {
            r -= am;
        }
        Ok(Value::Int(r as i64))
    }

    /// 値を**安定に**並べ替える（`sorted` / `list.sort` の唯一の実装・タスク 3-2）。比較は `<` だけ（CPython と同じ）。
    /// `reverse=True` は CPython と同じく「逆にして並べ、また逆にする」ので、等しいものの順は保たれる。
    /// ⚠ 並べ方はボトムアップのマージソート（CPython は timsort）。全順序なら結果は同じ。比べられない値に
    ///   当たったら、その比較の `TypeError` で止まる。
    pub(crate) fn py_sort_values(&mut self, items: Vec<Value>, key: Option<Value>, reverse: bool) -> Result<Vec<Value>, String> {
        let mut pairs: Vec<(Value, Value)> = Vec::with_capacity(items.len());
        for item in items {
            let k = self.py_key_of(&key, &item)?;
            pairs.push((k, item));
        }
        if reverse {
            pairs.reverse();
        }
        let n = pairs.len();
        let mut width = 1;
        while width < n {
            let mut dst: Vec<(Value, Value)> = Vec::with_capacity(n);
            let mut lo = 0;
            while lo < n {
                let mid = (lo + width).min(n);
                let hi = (lo + 2 * width).min(n);
                let (mut a, mut b) = (lo, mid);
                while a < mid && b < hi {
                    // 右が左より**真に小さい**ときだけ右を先に出す（安定）。
                    if self.py_less(crate::ast::BinOp::Lt, &pairs[b].0, &pairs[a].0)? {
                        dst.push(pairs[b].clone());
                        b += 1;
                    } else {
                        dst.push(pairs[a].clone());
                        a += 1;
                    }
                }
                dst.extend_from_slice(&pairs[a..mid]);
                dst.extend_from_slice(&pairs[b..hi]);
                lo = hi;
            }
            pairs = dst;
            width *= 2;
        }
        if reverse {
            pairs.reverse();
        }
        Ok(pairs.into_iter().map(|(_, v)| v).collect())
    }

    /// `sorted(it, key=None, reverse=False)`。
    pub(crate) fn py_sorted(&mut self, args: Vec<Value>, mut kw: Vec<(String, Value)>) -> Result<Value, String> {
        let [it]: [Value; 1] = args
            .try_into()
            .map_err(|a: Vec<Value>| format!("TypeError: sorted expected 1 argument, got {}", a.len()))?;
        let key = take_kw(&mut kw, "key").filter(|k| !matches!(k, Value::None));
        let reverse = match take_kw(&mut kw, "reverse") {
            Some(r) => self.eval_truthy(&r)?,
            None => false,
        };
        reject_extra_kw("sorted", &kw)?;
        let items = self.drain_iterable(it)?;
        let out = self.py_sort_values(items, key, reverse)?;
        Ok(Value::List(std::rc::Rc::new(std::cell::RefCell::new(out))))
    }

    // ── 変換・遅延の組み込み（タスク 3-3）──────────────────────────────────────

    /// 遅延の組み込みの本体（`py_lazy_prelude`）を呼んでジェネレータを得る。
    fn py_call_lazy(&mut self, gen_name: &str, args: Vec<Value>, shown: &str) -> Result<Value, String> {
        let gen_fn = py_lazy_prelude(gen_name)?;
        let evaled = args.into_iter().map(|v| (None, v, false)).collect();
        self.call_value_evaled(gen_fn, evaled, shown, None, 0)
    }

    /// 回す値を**先に**イテレータにする（`map(f, 5)` を呼んだ時点で `TypeError`・CPython と同じ）。
    fn py_eager_iter(&mut self, v: Value) -> Result<Value, String> {
        Ok(Value::Generator(self.py_iter_state(v)?))
    }

    /// `map(f, *iterables)`。**遅延**（取り出すたびに `f` を呼ぶ・CPython と同じ）。
    /// ⚠ 回す値が 2 つ以上のときは `zip` で先にまとめる（短い方で止まるのは同じだが、無限の列は回り切らない）。
    pub(crate) fn py_map(&mut self, args: Vec<Value>) -> Result<Value, String> {
        if args.len() < 2 {
            return Err("TypeError: map() must have at least two arguments.".to_string());
        }
        let mut it = args.into_iter();
        let f = it.next().expect("len >= 2");
        let rest: Vec<Value> = it.collect();
        if rest.len() == 1 {
            let src = self.py_eager_iter(rest.into_iter().next().expect("len 1"))?;
            self.py_call_lazy("__py_map", vec![f, src], "map")
        } else {
            let zipped = self.zip_core(rest)?;
            self.py_call_lazy("__py_map_star", vec![f, zipped], "map")
        }
    }

    /// `filter(f, it)`。**遅延**。`f` が `None` なら値の真偽で選ぶ（CPython と同じ）。
    pub(crate) fn py_filter(&mut self, args: Vec<Value>) -> Result<Value, String> {
        let [f, it]: [Value; 2] = args
            .try_into()
            .map_err(|a: Vec<Value>| format!("TypeError: filter expected 2 arguments, got {}", a.len()))?;
        let src = self.py_eager_iter(it)?;
        match f {
            Value::None => self.py_call_lazy("__py_filter_truthy", vec![src], "filter"),
            f => self.py_call_lazy("__py_filter", vec![f, src], "filter"),
        }
    }

    /// `iter(x)` / `iter(f, sentinel)`。1 引数は `for` と同じイテレータ（ジェネレータは自分自身）。
    /// 2 引数は `f()` が `sentinel` と等しくなるまで呼ぶ**遅延**のイテレータ。
    pub(crate) fn py_iter(&mut self, args: Vec<Value>) -> Result<Value, String> {
        match <[Value; 2]>::try_from(args) {
            Ok([f, sentinel]) => {
                if !self.py_callable(&f) {
                    return Err("TypeError: iter(v, w): v must be callable".to_string());
                }
                self.py_call_lazy("__py_iter_call", vec![f, sentinel], "iter")
            }
            Err(args) => match <[Value; 1]>::try_from(args) {
                Ok([x]) => self.py_eager_iter(x),
                Err(a) => Err(format!("TypeError: iter expected at most 2 arguments, got {}", a.len())),
            },
        }
    }

    /// `hash(x)`。辞書・集合の鍵と同じ値（`hash_value`）。⚠ 値そのものは CPython と違う（文字列の乱数化なども無い）。
    pub(crate) fn py_hash(&mut self, args: Vec<Value>) -> Result<Value, String> {
        let [x]: [Value; 1] = args
            .try_into()
            .map_err(|a: Vec<Value>| format!("TypeError: hash() takes exactly one argument ({} given)", a.len()))?;
        Ok(Value::Int(self.hash_value(&x)? as i64))
    }

    /// `ord(c)` / `chr(i)`。文言は CPython と同じ。
    pub(crate) fn py_ord_chr(&mut self, args: Vec<Value>, is_ord: bool) -> Result<Value, String> {
        let fname = if is_ord { "ord" } else { "chr" };
        let [x]: [Value; 1] = args
            .try_into()
            .map_err(|a: Vec<Value>| format!("TypeError: {fname}() takes exactly one argument ({} given)", a.len()))?;
        if is_ord {
            return match &x {
                Value::Str(s) => {
                    let mut cs = s.chars();
                    match (cs.next(), cs.next()) {
                        (Some(c), None) => Ok(Value::Int(c as i64)),
                        _ => Err(format!(
                            "TypeError: ord() expected a character, but string of length {} found",
                            s.chars().count()
                        )),
                    }
                }
                other => Err(format!(
                    "TypeError: ord() expected string of length 1, but {} found",
                    self.type_name(other)
                )),
            };
        }
        let i = self.py_index_int(&x)?;
        u32::try_from(i)
            .ok()
            .and_then(char::from_u32)
            .map(|c| Value::str(c.to_string()))
            .ok_or_else(|| "ValueError: chr() arg not in range(0x110000)".to_string())
    }

    /// `hex(i)` / `oct(i)` / `bin(i)`（接頭辞つき・負数は `-0x..`・CPython と同じ）。
    pub(crate) fn py_int_to_base(&mut self, args: Vec<Value>, base: u32) -> Result<Value, String> {
        let fname = match base {
            16 => "hex",
            8 => "oct",
            _ => "bin",
        };
        let [x]: [Value; 1] = args
            .try_into()
            .map_err(|a: Vec<Value>| format!("TypeError: {fname}() takes exactly one argument ({} given)", a.len()))?;
        let i = self.py_index_int(&x)?;
        let mag = (i as i128).unsigned_abs();
        let body = match base {
            16 => format!("0x{mag:x}"),
            8 => format!("0o{mag:o}"),
            _ => format!("0b{mag:b}"),
        };
        Ok(Value::str(if i < 0 { format!("-{body}") } else { body }))
    }

    /// 整数として読める値（int / bool・`__index__` を持つインスタンス）。ほかは CPython と同じ `TypeError`。
    fn py_index_int(&mut self, x: &Value) -> Result<i64, String> {
        match x {
            Value::Int(i) => Ok(*i),
            Value::Bool(b) => Ok(*b as i64),
            Value::Instance(inst) if inst.borrow().class.methods.contains_key("__index__") => {
                match self.eval_method_call_evaled(x.clone(), "__index__", vec![])? {
                    Value::Int(i) => Ok(i),
                    other => Err(format!(
                        "TypeError: __index__ returned non-int (type {})",
                        self.type_name(&other)
                    )),
                }
            }
            other => Err(format!(
                "TypeError: '{}' object cannot be interpreted as an integer",
                self.type_name(other)
            )),
        }
    }

    /// `ascii(x)`。`repr(x)` の非 ASCII を `\xhh` / `\uhhhh` / `\Uhhhhhhhh` にする（CPython と同じ）。
    pub(crate) fn py_ascii(&mut self, args: Vec<Value>) -> Result<Value, String> {
        let [x]: [Value; 1] = args
            .try_into()
            .map_err(|a: Vec<Value>| format!("TypeError: ascii() takes exactly one argument ({} given)", a.len()))?;
        let r = self.display_repr(&x);
        let mut out = String::with_capacity(r.len());
        for c in r.chars() {
            let cp = c as u32;
            if cp < 0x80 {
                out.push(c);
            } else if cp < 0x100 {
                out.push_str(&format!("\\x{cp:02x}"));
            } else if cp < 0x10000 {
                out.push_str(&format!("\\u{cp:04x}"));
            } else {
                out.push_str(&format!("\\U{cp:08x}"));
            }
        }
        Ok(Value::str(out))
    }

    /// クラス `c` が `base` 自身か、その派生か（`isinstance` / `issubclass` の**唯一の**判定・タスク 2-1 / 2-4）。
    ///
    /// 同じクラスは `class_id` で見る。派生は祖先の名前（`bases`）で見る。モジュールで定義した基底は
    /// 修飾名（`zoo.Animal`）で載っている（`exec_class_def`）ので、別のモジュールの同名クラスと取り違えない。
    /// ⚠ メインのクラス・組み込みの例外（`ValueError` など）は素の名前。`Exception` はすべての例外の基底
    ///   （`ClassValue::is_a`・10-19）。
    pub(crate) fn class_is_subclass(c: &ClassValue, base: &ClassValue) -> bool {
        if c.class_id == base.class_id {
            return true;
        }
        match base.module_name.as_deref() {
            Some(m) => c.is_a(&format!("{m}.{}", base.name)),
            None => c.is_a(&base.name),
        }
    }
}

/// キーワード引数 `name` を取り出す（取り出したものは `kw` から消す）。
fn take_kw(kw: &mut Vec<(String, Value)>, name: &str) -> Option<Value> {
    let i = kw.iter().position(|(k, _)| k == name)?;
    Some(kw.remove(i).1)
}

/// 残ったキーワード引数は受けない（CPython と同じ文言）。
fn reject_extra_kw(fname: &str, kw: &[(String, Value)]) -> Result<(), String> {
    match kw.first() {
        Some((k, _)) => Err(format!("TypeError: '{k}' is an invalid keyword argument for {fname}()")),
        None => Ok(()),
    }
}

/// 位置引数とキーワード引数に分ける（キーワードつきの組み込みの入口・`eval_builtin_evaled_named`）。
pub(crate) fn split_named(args: Vec<(Option<String>, Value)>) -> (Vec<Value>, Vec<(String, Value)>) {
    let mut pos = Vec::new();
    let mut kw = Vec::new();
    for (k, v) in args {
        match k {
            Some(k) => kw.push((k, v)),
            None => pos.push(v),
        }
    }
    (pos, kw)
}

/// 整数を `10^k` の倍数へ**偶数への丸め**で寄せる（`round(1250, -2)` は `1200`・CPython と同じ）。
fn round_int_half_even(v: i64, k: i64) -> i64 {
    if k > 18 {
        return 0;
    }
    let p = 10i128.pow(k as u32);
    let a = (v as i128).abs();
    let (q, r) = (a / p, a % p);
    let q = if 2 * r > p || (2 * r == p && q % 2 == 1) { q + 1 } else { q };
    let out = q * p;
    (if v < 0 { -out } else { out }) as i64
}

/// float を小数点以下 `nd` 桁（負なら 10 の `-nd` 乗の位）へ、**正確な 10 進の値**で偶数への丸め（CPython の `double_round`）。
///
/// f64 の値は有限の 10 進の小数で表せる（小数部は最大 1074 桁）ので、まず全桁を出してから文字列の上で丸め、
/// 最後に読み直す（読み直しは正しく丸められる）。
fn round_float_digits(f: f64, nd: i64) -> f64 {
    if !f.is_finite() || f == 0.0 {
        return f;
    }
    // ⚠ 桁が多すぎれば値は変わらない（CPython も同じ）。少なすぎれば 0。
    if nd > 400 {
        return f;
    }
    let s = format!("{:.1100}", f.abs());
    let (int_part, frac_part) = s.split_once('.').unwrap_or((s.as_str(), ""));
    let digits: Vec<u8> = int_part.bytes().chain(frac_part.bytes()).map(|b| b - b'0').collect();
    let point = int_part.len() as i64;
    let keep = point + nd;
    let sign = if f < 0.0 { -1.0 } else { 1.0 };
    if keep < 0 {
        return 0.0 * sign;
    }
    let keep = keep as usize;
    if keep >= digits.len() {
        return f;
    }
    let mut kept: Vec<u8> = digits[..keep].to_vec();
    let first = digits[keep];
    let rest_nonzero = digits[keep + 1..].iter().any(|&d| d != 0);
    let last_odd = kept.last().is_some_and(|d| d % 2 == 1);
    let round_up = first > 5 || (first == 5 && (rest_nonzero || last_odd));
    let mut point = point;
    if round_up {
        let mut i = kept.len();
        loop {
            if i == 0 {
                kept.insert(0, 1);
                point += 1;
                break;
            }
            i -= 1;
            if kept[i] == 9 {
                kept[i] = 0;
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    // 小数点の位置へ戻す（`keep` 桁より先は 0）。
    let mut text = String::new();
    let digits_str: String = kept.iter().map(|d| (b'0' + d) as char).collect();
    if point <= 0 {
        text.push_str("0.");
        text.push_str(&"0".repeat((-point) as usize));
        text.push_str(&digits_str);
    } else if (point as usize) >= digits_str.len() {
        text.push_str(&digits_str);
        text.push_str(&"0".repeat(point as usize - digits_str.len()));
    } else {
        text.push_str(&digits_str[..point as usize]);
        text.push('.');
        text.push_str(&digits_str[point as usize..]);
    }
    if text.is_empty() {
        return 0.0 * sign;
    }
    sign * text.parse::<f64>().unwrap_or(0.0)
}

/// `a` の法 `m` での逆元（`pow(a, -e, m)`・拡張ユークリッドの互除法）。互いに素でなければ `None`。
fn mod_inverse(a: i128, m: i128) -> Option<i128> {
    let (mut old_r, mut r) = (a, m);
    let (mut old_s, mut s) = (1i128, 0i128);
    while r != 0 {
        let q = old_r / r;
        (old_r, r) = (r, old_r - q * r);
        (old_s, s) = (s, old_s - q * s);
    }
    (old_r == 1).then(|| old_s.rem_euclid(m))
}

/// 遅延の組み込み（`map` / `filter` / `iter(f, sentinel)`）の本体（タスク 3-3）。
///
/// CPython の `map` / `filter` は取り出すたびに `f` を呼ぶ**遅延**のイテレータ。同じ振る舞いを、
/// Arrow の `gen`（中断できる本体を持つジェネレータ・B13）で書いて既存の仕組みに載せる
/// （Rust で中断できるイテレータを足すとジェネレータの仕組みごと作り直しになる）。
/// ⚠ 型検査は通さない（仮引数に型が無い）。名前の解決は呼び出し側の大域（`GLOBALS_OF_CALLER`）だが、
///   本体は仮引数と `for` の変数しか読まないので、利用者の名前に左右されない。
const PY_LAZY_PRELUDE: &str = "\
gen __py_map(f, it) -> Any:
    for x in it:
        yield f(x)

gen __py_map_star(f, it) -> Any:
    for args in it:
        yield f(*args)

gen __py_filter(f, it) -> Any:
    for x in it:
        if f(x):
            yield x

gen __py_filter_truthy(it) -> Any:
    for x in it:
        if x:
            yield x

gen __py_iter_call(f, sentinel) -> Any:
    mut v = f()
    while v != sentinel:
        yield v
        v = f()
";

thread_local! {
    /// 読み込んだ前置きのジェネレータ関数（スレッドごとに 1 度だけ作る）。
    static PY_LAZY: std::cell::RefCell<Option<std::collections::HashMap<String, Value>>> =
        const { std::cell::RefCell::new(None) };
}

/// 前置きのジェネレータ関数 `name` の値（初回だけ読み込む）。
fn py_lazy_prelude(name: &str) -> Result<Value, String> {
    PY_LAZY.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let tokens = crate::lexer::Lexer::new(PY_LAZY_PRELUDE, "<py-prelude>").tokenize();
            let stmts = crate::parser::Parser::new(tokens, None)
                .parse_program()
                .map_err(|e| format!("RuntimeError: internal — the Python prelude does not parse: {e}"))?;
            let mut table = std::collections::HashMap::new();
            for st in stmts {
                if let crate::ast::Stmt::GenDef { name, params, body, .. } = st {
                    let value = Value::GeneratorFn(std::rc::Rc::new(crate::interpreter::GeneratorFnValue {
                        globals: crate::interpreter::value::GLOBALS_OF_CALLER,
                        owner_class: None,
                        name: name.clone(),
                        params,
                        body,
                        captured_env: std::collections::HashMap::new(),
                    }));
                    table.insert(name, value);
                }
            }
            *slot = Some(table);
        }
        slot.as_ref()
            .and_then(|t| t.get(name).cloned())
            .ok_or_else(|| format!("RuntimeError: internal — '{name}' is not in the Python prelude"))
    })
}
